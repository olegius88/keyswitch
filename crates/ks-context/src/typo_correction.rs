//! Typo correction at a word's boundary: one keyboard slip, undone where a counted word list makes it clear.
//!
//! A word is corrected only when nothing reads it as a word: its form never occurs in the counted list of its language
//! (OpenSubtitles 2018 via FrequencyWords, every form down to one occurrence, so a rare inflected form survives), its
//! other layout's reading occurs in neither list, and the engine knows it from nowhere else (its lexicons, the term
//! tables, the identifier index, the user's exclusions). The candidates are the words one slip away: two neighbouring
//! letters swapped, a letter left out, a letter typed in excess, a letter typed with a neighbouring key. Each is scored as
//! a noisy channel, the logarithm of its count plus that of its slip, and the best one is taken only when it is frequent
//! enough and far more likely than every other candidate together. The decision reads nothing a platform provides (no
//! spell checker), so it is the same on every system.
//!
//! The correction types keys, not text: a swap or a deletion reuses the keys the user typed, with their Shift and Caps
//! Lock, and a letter not pressed in the word takes the key of the last press that typed it (KeyTemplates). A letter no
//! key has typed yet cannot be typed, and its candidate is not made.

use ks_cast::To as _;
use crate::constants::{
    KEYBOARD_ROW_LENGTHS, TYPO_ALPHABETS, TYPO_CANDIDATE_MIN_COUNT, TYPO_COUNT_LOG_SCALE, TYPO_DELETION_LOG_PROBABILITY, TYPO_FORM_SEPARATOR,
    TYPO_FREQUENCY_MAX_BYTES, TYPO_FREQUENCY_SCHEMA_VERSION, TYPO_INSERTION_LOG_PROBABILITY, TYPO_MIN_LETTERS, TYPO_OCCURRENCE_HASH_BYTES,
    TYPO_POSTERIOR_RATIO, TYPO_SECTIONS, TYPO_SUBSTITUTION_LOG_PROBABILITY, TYPO_TRANSPOSED_LETTERS, TYPO_TRANSPOSITION_LOG_PROBABILITY,
    VERSION_HASH_CHARACTERS,
};
use crate::context_model::TermTables;
use ks_backend::KeyEvent;
use ks_lang::detector::{is_protected_token, ProtectedTokens};
use ks_lang::identifier_lexicon::IdentifierLexicon;
use ks_lang::{py_sum, LanguageModel, LoadError, WordTable};
use ks_text::layouts::{RU_KEYS, US_KEYS};
use ks_text::russian_text::{quantity, LETTERS};
use ks_text::{pystr, ucd};
use indexmap::IndexMap;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// What both settings windows say of detection.correct_typos.
pub const CORRECT_TYPOS_TITLE: &str = "Исправлять опечатки";
pub const CORRECT_TYPOS_ENGLISH_TITLE: &str = "Исправлять опечатки и в английских словах";
pub const CORRECT_TYPOS_ENGLISH_DESCRIPTION: &str = "По умолчанию исправляются только русские слова: в технических текстах и коде незнакомое английское слово чаще оказывается именем переменной или термином, чем опечаткой";

/// `CORRECT_TYPOS_DESCRIPTION`.
pub fn correct_typos_description() -> String {
    format!(
        "Исправлять в слове одну опечатку: переставленные буквы, лишнюю, пропущенную или соседнюю клавишу. Только если ни один словарь не знает слово, а одно частое слово намного вероятнее других; слова, в которых меньше {}, и имена в середине предложения не трогаются. Pause или сочетание отмены сразу после исправления возвращает слово как набрано, и больше оно не исправляется; не исправляется и слово, которое не раз оставлено как набрано. Такие слова хранятся только как хеши и только при включённой истории",
        quantity(TYPO_MIN_LETTERS.to::<f64>(), LETTERS)
    )
}

/// The physical keys each locale's letters are typed with, in the order of layouts.US_KEYS.
pub fn locale_keys(locale: &str) -> Option<&'static str> {
    match locale {
        "en_US" => Some(US_KEYS),
        "ru_RU" => Some(RU_KEYS),
        _ => None,
    }
}

/// Russian prints `ё` as `е` as often as not: a form occurs if either spelling does.
const YO: char = 'ё';
const YE: char = 'е';

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Transposition,
    Deletion,
    Insertion,
    Substitution,
}

pub const OPERATIONS: [Operation; 4] = [Operation::Transposition, Operation::Deletion, Operation::Insertion, Operation::Substitution];

impl Operation {
    pub fn as_str(self) -> &'static str {
        match self {
            Operation::Transposition => "transposition",
            Operation::Deletion => "deletion",
            Operation::Insertion => "insertion",
            Operation::Substitution => "substitution",
        }
    }
}

/// The decision's parameters; the shipped ones are the constants, and the measurement tool tries others.
#[derive(Clone, Debug, PartialEq)]
pub struct TypoPolicy {
    pub candidate_min_count: i64,
    pub posterior_ratio: f64,
    pub log_probabilities: [(Operation, f64); 4],
}

impl Default for TypoPolicy {
    fn default() -> Self {
        TypoPolicy {
            candidate_min_count: TYPO_CANDIDATE_MIN_COUNT.to::<i64>(),
            posterior_ratio: TYPO_POSTERIOR_RATIO,
            log_probabilities: [
                (Operation::Transposition, TYPO_TRANSPOSITION_LOG_PROBABILITY),
                (Operation::Deletion, TYPO_DELETION_LOG_PROBABILITY),
                (Operation::Insertion, TYPO_INSERTION_LOG_PROBABILITY),
                (Operation::Substitution, TYPO_SUBSTITUTION_LOG_PROBABILITY),
            ],
        }
    }
}

impl TypoPolicy {
    /// The stored weight of `candidate_min_count`, the least a corrected word may have.
    pub fn minimum_weight(&self) -> i64 {
        let scaled = ks_text::log(self.candidate_min_count.to::<f64>()).expect("a positive count") * TYPO_COUNT_LOG_SCALE.to::<f64>();
        ks_text::py_round_to_int(scaled).expect("a finite weight").to::<i64>()
    }

    fn log_probability(&self, operation: Operation) -> f64 {
        self.log_probabilities.iter().find(|entry| entry.0 == operation).map_or(0.0, |entry| entry.1)
    }
}

/// `typo_resource_path(locale)`.
pub fn typo_resource_path(resource_dir: &Path, locale: &str) -> PathBuf {
    resource_dir.join(format!("typo-frequency-{locale}.bin"))
}

/// The letters of the keys around each letter's key, in a layout whose keys `keys` lists (layouts.US_KEYS order).
///
/// Keys of one row touch their left and right neighbours. A lower row sits half a key to the right of the row above it,
/// so a key touches the key right above it and the one after that, and the key right below it and the one before that. The
/// key of the top row (left of `1`) touches no letter. ЙЦУКЕН and QWERTY are the same keys, so one table of positions
/// serves both layouts.
pub fn keyboard_neighbours(keys: &str) -> Result<HashMap<char, String>, LoadError> {
    let chars: Vec<char> = keys.chars().collect();
    let mut rows: Vec<&[char]> = Vec::new();
    let mut start = 0;
    for &length in KEYBOARD_ROW_LENGTHS {
        rows.push(&chars[start.min(chars.len())..(start + length).min(chars.len())]);
        start += length;
    }
    if start != chars.len() {
        return Err(LoadError::value("the key rows do not cover the layout's keys"));
    }
    let letter_rows = &rows[1..];
    let mut neighbours = HashMap::new();
    for (row_index, row) in letter_rows.iter().enumerate() {
        for (index, &key) in row.iter().enumerate() {
            let mut near: Vec<char> = Vec::new();
            for position in [index.to::<i64>() - 1, index.to::<i64>() + 1] {
                if 0 <= position && (position as usize) < row.len() {
                    near.push(row[position.to::<usize>()]);
                }
            }
            if row_index > 0 {
                let above = letter_rows[row_index - 1];
                near.extend([index, index + 1].into_iter().filter(|&position| position < above.len()).map(|position| above[position]));
            }
            if row_index + 1 < letter_rows.len() {
                let below = letter_rows[row_index + 1];
                near.extend([index.to::<i64>() - 1, index.to::<i64>()].into_iter().filter(|&position| 0 <= position && (position as usize) < below.len()).map(|position| below[position.to::<usize>()]));
            }
            if ucd::isalpha(key) {
                let mut letters: Vec<char> = near.into_iter().filter(|&c| ucd::isalpha(c)).collect();
                letters.sort_unstable();
                neighbours.insert(key, letters.into_iter().collect());
            }
        }
    }
    Ok(neighbours)
}

/// A counted word list of one language: forms with their counts, and a membership filter for the rarer forms.
///
/// `forms` holds the stored forms sorted, each coded one byte per letter (its place in `alphabet`, from one up) and ended
/// by TYPO_FORM_SEPARATOR; `starts` the offset of each, with one more for the end; `weights` each form's count as
/// `round(ln(count) * TYPO_COUNT_LOG_SCALE)`. `seen` is a Bloom filter of `seen_bits` bits and `seen_hashes` positions over
/// the forms too rare to be stored: it may hold a form it was not given, never miss one it was.
pub struct TypoLexicon {
    pub locale: String,
    pub alphabet: String,
    forms: Vec<u8>,
    starts: Vec<u32>,
    weights: Vec<u8>,
    seen: Vec<u8>,
    seen_bits: u64,
    seen_hashes: u64,
    pub version: String,
}

fn int_member(value: Option<&Value>) -> Option<i64> {
    match value {
        Some(Value::Number(number)) => number.as_i64(),
        _ => None,
    }
}

/// `header.get(name) == number` as Python compares an int with a JSON number or flag.
fn equals_int(value: Option<&Value>, wanted: i64) -> bool {
    match value {
        Some(Value::Number(number)) => number.as_i64().map_or_else(|| number.as_f64() == Some(wanted.to::<f64>()), |found| found == wanted),
        Some(Value::Bool(flag)) => i64::from(*flag) == wanted,
        _ => false,
    }
}

impl TypoLexicon {
    pub fn size(&self) -> usize {
        self.weights.len()
    }

    /// The form as stored: one byte per letter, or None when a character cannot be coded.
    fn coded(&self, form: &str) -> Option<Vec<u8>> {
        let mut coded = Vec::with_capacity(form.len());
        for c in form.chars() {
            match self.alphabet.chars().position(|letter| letter == c) {
                Some(place) => coded.push((place + 1).to::<u8>()),
                None if (c as u32) < 256 => coded.push((c.to::<u32>()).to::<u8>()),
                None => return None,
            }
        }
        // `not coded or max(coded) > len(self.alphabet)`
        if coded.is_empty() || coded.iter().copied().max().is_some_and(|byte| usize::from(byte) > self.alphabet.chars().count()) {
            return None;
        }
        Some(coded)
    }

    /// The stored count of a lowercase form as its scaled logarithm, or None when it is not stored.
    pub fn weight(&self, form: &str) -> Option<i64> {
        let coded = self.coded(form)?;
        let (mut low, mut high) = (0, self.size());
        while low < high {
            let middle = (low + high) >> 1;
            let stored = &self.forms[self.starts[middle].to::<usize>()..(self.starts[middle + 1]).to::<usize>() - 1];
            match stored.cmp(coded.as_slice()) {
                std::cmp::Ordering::Equal => return Some(i64::from(self.weights[middle])),
                std::cmp::Ordering::Less => low = middle + 1,
                std::cmp::Ordering::Greater => high = middle,
            }
        }
        None
    }

    /// Whether a lowercase form occurs in the list at all, written with `ё` or with `е`.
    pub fn occurs(&self, form: &str) -> bool {
        let plain: String = form.chars().map(|c| if c == YO { YE } else { c }).collect();
        self.occurs_spelling(form) || (plain != form && self.occurs_spelling(&plain))
    }

    fn occurs_spelling(&self, form: &str) -> bool {
        if self.weight(form).is_some() {
            return true;
        }
        if self.coded(form).is_none() {
            return false;
        }
        occurrence_positions(form, self.seen_bits, self.seen_hashes).all(|position| self.seen[(position / 8).to::<usize>()] & (1 << (position % 8)) != 0)
    }

    /// `TypoLexicon.load(path, locale)`.
    pub fn load(path: &Path, locale: &str) -> Result<TypoLexicon, LoadError> {
        use std::io::Read;
        let file = std::fs::File::open(path).map_err(|error| LoadError::os(&error, path))?;
        let mut raw = Vec::new();
        file.take(TYPO_FREQUENCY_MAX_BYTES.to::<u64>() + 1).read_to_end(&mut raw).map_err(|error| LoadError::os(&error, path))?;
        if raw.len() > TYPO_FREQUENCY_MAX_BYTES {
            return Err(LoadError::value("typo frequency file is too large"));
        }
        let Some(split) = raw.iter().position(|&byte| byte == b'\n') else {
            return Err(LoadError::value("typo frequency file has no header"));
        };
        let (header_line, payload) = (&raw[..split], &raw[split + 1..]);
        let header: Value = serde_json::from_slice(header_line).map_err(|error| LoadError { name: "JSONDecodeError", message: error.to_string() })?;
        let invalid_header = || LoadError::value("invalid typo frequency header");
        let header = header.as_object().ok_or_else(invalid_header)?;
        let alphabet = TYPO_ALPHABETS.iter().find(|entry| entry.0 == locale).map(|entry| entry.1);
        let digest: String = Sha256::digest(payload).iter().map(|byte| format!("{byte:02x}")).collect();
        if !equals_int(header.get("schema_version"), TYPO_FREQUENCY_SCHEMA_VERSION.to::<i64>())
            || header.get("locale").and_then(Value::as_str) != Some(locale)
            || alphabet.is_none()
            || header.get("alphabet").and_then(Value::as_str) != alphabet
            || !equals_int(header.get("count_log_scale"), TYPO_COUNT_LOG_SCALE.to::<i64>())
            || header.get("payload_sha256").and_then(Value::as_str) != Some(digest.as_str())
        {
            return Err(invalid_header());
        }
        let (Some(sections), Some(seen)) = (header.get("sections").and_then(Value::as_object), header.get("seen").and_then(Value::as_object)) else {
            return Err(invalid_header());
        };
        let sizes: Vec<i64> = TYPO_SECTIONS.iter().filter_map(|name| int_member(sections.get(*name)).filter(|&size| size >= 0)).collect();
        let (bits, hashes) = (int_member(seen.get("bits")), int_member(seen.get("hashes")));
        let (Some(bits), Some(hashes)) = (bits, hashes) else {
            return Err(LoadError::value("invalid typo frequency sections"));
        };
        if sizes.len() != TYPO_SECTIONS.len() || sizes.iter().sum::<i64>() != payload.len().to::<i64>() || bits < 1 || hashes < 1 {
            return Err(LoadError::value("invalid typo frequency sections"));
        }
        let ends: Vec<usize> = sizes.iter().scan(0, |total, &size| { *total += size.to::<usize>(); Some(*total) }).collect();
        let inflate = |bytes: &[u8]| miniz_oxide::inflate::decompress_to_vec_zlib(bytes).map_err(|error| LoadError::value(format!("Error -3 while decompressing data: {error:?}")));
        let forms = inflate(&payload[..ends[0]])?;
        let weights = inflate(&payload[ends[0]..ends[1]])?;
        let filter_bits = payload[ends[1]..].to_vec();
        let mut starts: Vec<u32> = vec![0];
        starts.extend(forms.iter().enumerate().filter(|(_, byte)| usize::from(**byte) == TYPO_FORM_SEPARATOR).map(|(at, _)| (at + 1).to::<u32>()));
        if starts.len() - 1 != weights.len() || (!forms.is_empty() && usize::from(forms[forms.len() - 1]) != TYPO_FORM_SEPARATOR) || (filter_bits.len() as i64) * 8 < bits {
            return Err(LoadError::value("invalid typo frequency payload"));
        }
        let version_digest: String = Sha256::digest(&raw).iter().map(|byte| format!("{byte:02x}")).collect();
        Ok(TypoLexicon {
            locale: locale.to_string(),
            alphabet: header.get("alphabet").and_then(Value::as_str).unwrap_or_default().to_string(),
            forms,
            starts,
            weights,
            seen: filter_bits,
            seen_bits: bits.to::<u64>(),
            seen_hashes: hashes.to::<u64>(),
            version: format!("typo-frequency-{}", &version_digest[..VERSION_HASH_CHARACTERS]),
        })
    }
}

/// The filter positions of a form: two hashes from its SHA-256, combined (Kirsch and Mitzenmacher).
pub fn occurrence_positions(form: &str, bits: u64, hashes: u64) -> impl Iterator<Item = u64> {
    let digest = Sha256::digest(form.as_bytes());
    let width = TYPO_OCCURRENCE_HASH_BYTES;
    let read = |bytes: &[u8]| bytes.iter().rev().fold(0_u128, |total, &byte| (total << 8) | u128::from(byte));
    let first = read(&digest[..width]);
    let step = read(&digest[width..2 * width]) | 1;
    (0..hashes).map(move |index| ((first + u128::from(index) * step) % u128::from(bits)).to::<u64>())
}

/// `packaged_lexicon(locale)`: the shipped list of a locale; None when none is shipped or it does not load.
pub fn packaged_lexicon(resource_dir: &Path, locale: &str) -> Option<TypoLexicon> {
    let path = typo_resource_path(resource_dir, locale);
    if !TYPO_ALPHABETS.iter().any(|entry| entry.0 == locale) || !path.is_file() {
        return None;
    }
    TypoLexicon::load(&path, locale).ok()
}

/// One slip undone: the operation, where it applies in the typed word, and the letter it brings, if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edit {
    pub operation: Operation,
    pub position: usize,
    pub letter: Option<char>,
}

impl Edit {
    pub fn apply(&self, word: &str) -> String {
        let chars: Vec<char> = word.chars().collect();
        let position = self.position;
        let mut result: String = chars[..position].iter().collect();
        match self.operation {
            Operation::Transposition => {
                result.push(chars[position + 1]);
                result.push(chars[position]);
                result.extend(&chars[(position + TYPO_TRANSPOSED_LETTERS).min(chars.len())..]);
            }
            Operation::Deletion => result.extend(&chars[position + 1..]),
            Operation::Insertion => {
                result.extend(self.letter);
                result.extend(&chars[position..]);
            }
            Operation::Substitution => {
                result.extend(self.letter);
                result.extend(&chars[position + 1..]);
            }
        }
        result
    }

    /// For each letter of the corrected word, the typed letter it is, or None for the letter the edit brings.
    pub fn sources(&self, length: usize) -> Vec<Option<usize>> {
        let mut order: Vec<Option<usize>> = (0..length).map(Some).collect();
        let position = self.position;
        match self.operation {
            Operation::Transposition => order.swap(position, position + 1),
            Operation::Deletion => {
                order.remove(position);
            }
            Operation::Insertion => order.insert(position, None),
            Operation::Substitution => order[position] = None,
        }
        order
    }
}

/// Every single slip that could have turned another word into `word` (lowercase), each undone once.
///
/// A letter typed in excess is a key pressed twice or one brushed beside the key meant: it repeats a letter next to it or
/// lies on a key next to that letter's. Any other extra letter is no slip of the fingers but part of the word: an
/// identifier's prefix (`nlevel`, `bweight`) or a rare inflection (`эстафетах`), which the development texts showed
/// corrected away. A letter left out may be any letter, and a letter typed for another lies on a neighbouring key.
pub fn edits(word: &str, alphabet: &str, neighbours: &HashMap<char, String>) -> Vec<Edit> {
    let chars: Vec<char> = word.chars().collect();
    let mut found = Vec::new();
    for position in 0..chars.len().saturating_sub(1) {
        if chars[position] != chars[position + 1] {
            found.push(Edit { operation: Operation::Transposition, position, letter: None });
        }
    }
    for (position, &extra) in chars.iter().enumerate() {
        let around = [position.to::<i64>() - 1, position.to::<i64>() + 1].into_iter().filter(|&index| 0 <= index && (index as usize) < chars.len()).map(|index| chars[index.to::<usize>()]);
        if around.into_iter().any(|letter| extra == letter || neighbours.get(&letter).is_some_and(|near| near.contains(extra))) {
            found.push(Edit { operation: Operation::Deletion, position, letter: None });
        }
    }
    for position in 0..=chars.len() {
        for letter in alphabet.chars() {
            found.push(Edit { operation: Operation::Insertion, position, letter: Some(letter) });
        }
    }
    for (position, typed) in chars.iter().enumerate() {
        for letter in neighbours.get(typed).map_or("", String::as_str).chars() {
            found.push(Edit { operation: Operation::Substitution, position, letter: Some(letter) });
        }
    }
    found
}

/// A stored word one slip away from the typed one: its weight and every slip that turns it into the typed word.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub word: String,
    pub weight: i64,
    pub edits: Vec<Edit>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Shape,
    Capital,
    Occurs,
    OtherReading,
    Known,
    NoCandidate,
    Rare,
    Ambiguous,
    Corrected,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::Shape => "shape",
            Reason::Capital => "capital",
            Reason::Occurs => "occurs",
            Reason::OtherReading => "other_reading",
            Reason::Known => "known",
            Reason::NoCandidate => "no_candidate",
            Reason::Rare => "rare",
            Reason::Ambiguous => "ambiguous",
            Reason::Corrected => "corrected",
        }
    }
}

/// The candidate a word is corrected to (cased as the word was), how, and how clearly.
///
/// `weight` is the stored count's scaled logarithm, `candidates` how many words one slip away the list holds, and `margin`
/// the natural logarithm of the best candidate's posterior over the sum of all the others' (infinite when there are none).
#[derive(Clone, Debug, PartialEq)]
pub struct TypoChoice {
    pub word: String,
    pub edit: Edit,
    pub weight: i64,
    pub candidates: usize,
    pub margin: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TypoJudgement {
    pub reason: Reason,
    pub choice: Option<TypoChoice>,
    pub candidates: usize,
}

impl TypoJudgement {
    fn bare(reason: Reason) -> Self {
        TypoJudgement { reason, choice: None, candidates: 0 }
    }

    /// The decimal order of magnitude of the chosen candidate's count, for the log; -1 without one.
    pub fn count_order(&self) -> i64 {
        match &self.choice {
            None => -1,
            Some(choice) => ((choice.weight.to::<f64>() / TYPO_COUNT_LOG_SCALE.to::<f64>()).exp().log10().floor()).to::<i64>(),
        }
    }
}

/// Letters of the alphabet only, at least TYPO_MIN_LETTERS of them, lowercase or with one leading capital.
pub fn word_shape(word: &str, alphabet: &str) -> bool {
    let lower = pystr::casefold(word);
    pystr::len(word) >= TYPO_MIN_LETTERS
        && lower.chars().all(|c| alphabet.contains(c))
        && pystr::slice(word, Some(1), None) == pystr::slice(&lower, Some(1), None)
        && pystr::len(&lower) == pystr::len(word)
}

/// Judges words of the languages it has lists for; the engine asks it at a word's boundary.
pub struct TypoCorrector {
    pub lexicons: Vec<(String, TypoLexicon)>,
    pub policy: TypoPolicy,
    neighbours: Vec<(String, HashMap<char, String>)>,
}

impl TypoCorrector {
    pub fn new(lexicons: Vec<(String, TypoLexicon)>, policy: TypoPolicy) -> Result<Self, LoadError> {
        let mut neighbours = Vec::new();
        for (locale, _) in &lexicons {
            let keys = locale_keys(locale).ok_or_else(|| LoadError { name: "KeyError", message: locale.clone() })?;
            neighbours.push((locale.clone(), keyboard_neighbours(keys)?));
        }
        Ok(TypoCorrector { lexicons, policy, neighbours })
    }

    /// `TypoCorrector.packaged(locales)`.
    pub fn packaged(resource_dir: &Path, locales: &[&str]) -> Result<Self, LoadError> {
        let lexicons = locales.iter().filter_map(|locale| packaged_lexicon(resource_dir, locale).map(|lexicon| ((*locale).to_string(), lexicon))).collect();
        Self::new(lexicons, TypoPolicy::default())
    }

    /// The keys beside each key of a locale's layout.
    pub fn neighbours_of(&self, locale: &str) -> &HashMap<char, String> {
        &self.neighbours.iter().find(|entry| entry.0 == locale).expect("neighbours of every list").1
    }

    pub fn lexicon(&self, locale: &str) -> Option<&TypoLexicon> {
        self.lexicons.iter().find(|entry| entry.0 == locale).map(|entry| &entry.1)
    }

    /// Whether to correct `word`, typed in the layout of `locale`, and to what.
    ///
    /// `readings` are the same keys read in the other layouts, by locale; `capital_allowed` says whether a capital may open
    /// the word here (the start of a message or of a sentence; a capital in the middle of one is a name); `known` says
    /// whether the engine knows a reading from elsewhere.
    pub fn judge(&self, word: &str, locale: &str, readings: &[(&str, &str)], capital_allowed: bool, known: &dyn Fn(&str) -> bool) -> TypoJudgement {
        let Some(lexicon) = self.lexicon(locale).filter(|lexicon| word_shape(word, &lexicon.alphabet)) else {
            return TypoJudgement::bare(Reason::Shape);
        };
        let lower = pystr::casefold(word);
        let capital = lower != word;
        if capital && !capital_allowed {
            return TypoJudgement::bare(Reason::Capital);
        }
        if lexicon.occurs(&lower) {
            return TypoJudgement::bare(Reason::Occurs);
        }
        for (other, reading) in readings {
            if self.lexicon(other).is_some_and(|other_lexicon| other_lexicon.occurs(&pystr::casefold(reading))) {
                return TypoJudgement::bare(Reason::OtherReading);
            }
        }
        if known(word) || readings.iter().any(|(_, reading)| known(reading)) {
            return TypoJudgement::bare(Reason::Known);
        }
        self.decide(&self.candidates(&lower, locale), capital)
    }

    /// The stored words one slip away from a lowercase word, each with every slip that leads to it.
    ///
    /// They do not depend on the policy, so the measurement tool scores one list under many policies.
    pub fn candidates(&self, lower: &str, locale: &str) -> Vec<Candidate> {
        let lexicon = self.lexicon(locale).expect("a locale the corrector has a list for");
        let neighbours = &self.neighbours.iter().find(|entry| entry.0 == locale).expect("neighbours of every list").1;
        let mut found: IndexMap<String, Vec<Edit>> = IndexMap::new();
        let mut looked: IndexMap<String, Option<i64>> = IndexMap::new();
        for edit in edits(lower, &lexicon.alphabet, neighbours) {
            let candidate = edit.apply(lower);
            let weight = match looked.get(&candidate) {
                Some(&weight) => weight,
                None => {
                    let weight = lexicon.weight(&candidate);
                    looked.insert(candidate.clone(), weight);
                    weight
                }
            };
            if weight.is_some() {
                found.entry(candidate).or_default().push(edit);
            }
        }
        looked
            .into_iter()
            .filter_map(|(word, weight)| weight.map(|weight| Candidate { edits: found.swap_remove(&word).unwrap_or_default(), word, weight }))
            .collect()
    }

    /// The noisy-channel choice among the candidates: each scores the logarithm of its count plus that of the sum of its
    /// slips' probabilities; the best is taken when it is frequent enough and clear enough of the rest.
    pub fn decide(&self, candidates: &[Candidate], capital: bool) -> TypoJudgement {
        decide_under(&self.policy, candidates, capital)
    }
}

/// `TypoCorrector.decide` under any policy: the candidates do not depend on it, so the measurement tool scores one list under many.
pub fn decide_under(policy: &TypoPolicy, candidates: &[Candidate], capital: bool) -> TypoJudgement {
    if candidates.is_empty() {
        return TypoJudgement::bare(Reason::NoCandidate);
    }
    let totals: Vec<f64> = candidates
        .iter()
        .map(|candidate| {
            let slips: Vec<f64> = candidate.edits.iter().map(|edit| policy.log_probability(edit.operation)).collect();
            candidate.weight.to::<f64>() / TYPO_COUNT_LOG_SCALE.to::<f64>() + log_sum(&slips)
        })
        .collect();
    // `max(range(n), key=lambda index: (totals[index], candidates[index].word))`: the first of equal keys.
    let mut best = 0;
    for index in 1..candidates.len() {
        let better = totals[index] > totals[best] || (totals[index] == totals[best] && candidates[index].word > candidates[best].word);
        if better {
            best = index;
        }
    }
    let others: Vec<f64> = totals.iter().enumerate().filter(|(index, _)| *index != best).map(|(_, &total)| total).collect();
    let margin = if others.is_empty() { f64::INFINITY } else { totals[best] - log_sum(&others) };
    let chosen = &candidates[best];
    let edit = ks_text::max_by_key_first(chosen.edits.iter(), |edit| policy.log_probability(edit.operation)).copied().expect("a candidate has an edit");
    let word = if capital { format!("{}{}", pystr::upper(pystr::slice(&chosen.word, None, Some(1))), pystr::slice(&chosen.word, Some(1), None)) } else { chosen.word.clone() };
    let choice = TypoChoice { word, edit, weight: chosen.weight, candidates: candidates.len(), margin };
    let reason = if chosen.weight < policy.minimum_weight() {
        Reason::Rare
    } else if margin < ks_text::log(policy.posterior_ratio).expect("a positive ratio") {
        Reason::Ambiguous
    } else {
        Reason::Corrected
    };
    TypoJudgement { reason, choice: Some(choice), candidates: candidates.len() }
}

fn log_sum(values: &[f64]) -> f64 {
    let high = ks_text::max_first(values.iter().copied()).expect("values");
    high + py_sum(values.iter().map(|&value| (value - high).exp())).ln()
}

/// Whether the engine knows a reading from anything but the counted lists: a word of its lexicons (spelt with `ё` or `е`,
/// no spell checker), a word the term tables count in any alphabet, a name of the identifier index, a token the code guard
/// protects, or a word the user excluded.
pub fn known_elsewhere(
    text: &str, lexicons: &[&WordTable], terms: &TermTables, identifiers: Option<&IdentifierLexicon>, protected: &ProtectedTokens,
    excluded: &dyn Fn(&str) -> bool,
) -> bool {
    let lower = pystr::casefold(text);
    let plain: String = text.chars().map(|c| if c == YO { YE } else { c }).collect();
    let spellings = [LanguageModel::normalize(text), LanguageModel::normalize(&plain)];
    lexicons.iter().any(|lexicon| spellings.iter().any(|spelling| lexicon.contains(spelling)))
        || crate::context_model::TERM_ALPHABETS.iter().any(|alphabet| terms.term(alphabet).contains(&lower))
        || identifiers.is_some_and(|lexicon| lexicon.contains(text))
        || is_protected_token(text, protected)
        || excluded(&lower)
}

/// The last key press that typed each letter, by layout: the key a correction types for a letter not in the word.
///
/// Every press carries its character in every layout (KeyEvent.characters), so one press teaches a letter of each.
#[derive(Default)]
pub struct KeyTemplates {
    keys: HashMap<(i32, char), KeyEvent>,
}

impl KeyTemplates {
    pub fn new() -> Self {
        KeyTemplates::default()
    }

    pub fn observe(&mut self, event: &KeyEvent) {
        if !event.pressed || event.synthetic || event.control() || event.alt() || event.super_key() {
            return;
        }
        for (group, character) in event.characters.iter().enumerate() {
            let mut chars = character.chars();
            if let (Some(c), None) = (chars.next(), chars.next())
                && ucd::isalpha(c)
            {
                self.keys.insert((group.to::<i32>(), c), event.clone());
            }
        }
    }

    /// A press of the key that types `character` in `group`, as if typed in `typed_group` like the word's keys.
    pub fn key(&self, group: i32, character: char, typed_group: i32) -> Option<KeyEvent> {
        let template = self.keys.get(&(group, character))?;
        let mut key = template.clone();
        key.pressed = true;
        key.group = typed_group;
        key.character = template.character_for(typed_group).to_string();
        key.synthetic = false;
        key.deferred = false;
        key.foreign = false;
        Some(key)
    }
}

/// The keys that type `choice.word` in `group`, from the keys typed: each letter the user typed is reused as it was pressed
/// when it still types the right letter (case included), and any other letter takes a template; None when a letter has
/// none.
pub fn corrected_keys(typed: &[KeyEvent], group: i32, choice: &TypoChoice, templates: &KeyTemplates) -> Option<Vec<KeyEvent>> {
    let mut keys = Vec::new();
    let typed_group = typed.first().map_or(group, |first| first.group);
    let sources = choice.edit.sources(typed.len());
    let wanted: Vec<char> = choice.word.chars().collect();
    assert_eq!(wanted.len(), sources.len(), "zip() argument 2 is shorter than argument 1");
    for (wanted, source) in wanted.into_iter().zip(sources) {
        if let Some(source) = source
            && typed[source].character_for(group) == wanted.to_string()
        {
            keys.push(typed[source].clone());
            continue;
        }
        keys.push(templates.key(group, wanted, typed_group)?);
    }
    Some(keys)
}
