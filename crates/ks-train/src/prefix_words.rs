//! The word corpus of the prefix model (`prefix-corpus --config model/prefix_v3/config.json`): every word once, in three of
//! four situations, typed in the other layout and as meant (the paired classes: frequent words, rare forms, names, unknown
//! words, Russian words whose keys read like English), or only as meant (real tokens: terms, short terms, tokens that open
//! with a sign key, parts of addresses). The trainer makes the prefixes and their labels from the rows.
//!
//! Words come only from the training split of the frozen base corpus (the UD treebanks of `BASE_PINS`, names by their
//! PROPN tag, the Debian trixie commands, the addresses of UD_English-EWT) and from the CC0 Tatoeba snapshot, whose words
//! every holdout test already leaves out (`family-in-base-corpus`, `family-previously-exposed`); terms from the tree's
//! `resources/identifiers.json`, `resources/protected_tokens.txt` and the Latin column of
//! `resources/models/context-term-frequency.json`. A word whose physical aliases meet a held-out development sample is left
//! out, so that sample stays a fair measure. No private source and no test split is read.
use ks_cast::To as _;
use crate::corpus::checksum;
use crate::ortho_inputs::load_phrases;
use crate::physical::{expanded_aliases, hex, hex_digest};
use crate::prefix_corpus::{family, word_matches};
use crate::tools::reference_models;
use flate2::Compression;
use flate2::read::MultiGzDecoder;
use flate2::write::GzEncoder;
use ks_context::prefix_schema::prefix_conditionals;
use ks_lang::Resources;
use ks_lang::ortho_model::{OrthoModel, artifact_path};
use ks_text::{Indent, JsonOptions, pystr};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The splits of the word corpus. It has no test split: the pair's sealed test is the holdout freeze of the base corpus.
pub const WORD_SPLITS: [&str; 3] = ["train", "development", "calibration"];
/// The split of a corpus of schema three that no fit reads: the words engine acceptance samples are made of.
pub const ACCEPTANCE_SPLIT: &str = "acceptance";
/// The `kind` of a word corpus's config and receipt.
pub const WORD_CORPUS_KIND: &str = "prefix-words";
/// Schema two: the corpus as first drawn. Schema three adds the hard words by their first keys, the `low_keys` class, the
/// acceptance split and a calibration cell of each direction.
const WORD_CORPUS_SCHEMAS: [u32; 2] = [2, 3];
const HELD_OUT_FILE: &str = "held-out.json.gz";
const RECEIPT_FILE: &str = "corpus.json";
const CONFIG_FILE: &str = "config.json";
const GENERATOR: &str = "crates/ks-train/src/prefix_words.rs";
const TATOEBA_SNAPSHOT: &str = "model/context_v2/sources/tatoeba-cc0-en-ru.tsv.gz";
const IDENTIFIERS: &str = "resources/identifiers.json";
const PROTECTED_TOKENS: &str = "resources/protected_tokens.txt";
const TERM_TABLE: &str = "resources/models/context-term-frequency.json";
/// Hex digits of a draw: 64 bits of the SHA-256 of a name.
const DRAW_HEX_DIGITS: usize = 16;
/// Hex digits of a held-out alias kept in the held-out file: 64 bits, against some hundreds of thousands of aliases.
const HELD_OUT_ALIAS_HEX_DIGITS: usize = 16;
const PER_MILLE: u64 = 1000;
/// Keys of the US layout that are letters of the Russian one: the keys an address part may hold after its first letter.
const LAYOUT_LETTER_SIGNS: &str = ",.;'[]<>:\"{}`~";
/// The shortest prefix the orthotactic reading of a Russian word's keys is weighed at, as schema three buckets it.
const ENGLISH_KEYS_MIN_CHARACTERS: usize = 3;
const COMMENT_OPENING: &str = "// ";
/// The training split of the base corpus, its Russian locale in the Tatoeba snapshot, the treebank whose address tokens are
/// cut into parts, and the tag of a name.
const BASE_TRAINING_SPLIT: &str = "train";
const TATOEBA_RUSSIAN: &str = "rus";
const ADDRESS_TREEBANK: &str = "UD_English-EWT";
const NAME_TAG: &str = "PROPN";

/// What a word is, which decides whether it is also typed in the other layout and how it is counted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    /// One of the most frequent words of the Tatoeba snapshot in its language.
    Frequent,
    /// A form the onboard lexicon knows, not among the frequent ones.
    Rare,
    /// A name: tagged PROPN by its treebank, its first letter a capital.
    Name,
    /// A form the onboard lexicon does not know.
    Unknown,
    /// A Russian word whose keys the English orthotactic model reads better than the Russian one at some short prefix.
    EnglishKeys,
    /// A word, not a name, whose first keys its own layout's orthotactic channel reads worse than most words of its
    /// language: loanwords, rare stems (`config.low_keys`).
    LowKeys,
    /// A Latin technical token of five letters or more that is no English word of the corpus.
    Term,
    /// The same of three or four letters.
    ShortTerm,
    /// A Latin word after a key that is a letter of the Russian layout: `<`, `"`, `'`, `[`, `{`, `;`, `:`.
    SignLed,
    /// A part of an e-mail address or a link, between signs that are no letters in either layout.
    Address,
}

impl Class {
    /// A paired word is typed in the other layout too (to convert) as well as as meant (to keep).
    #[must_use]
    pub fn paired(self) -> bool {
        match self {
            Class::Frequent | Class::Rare | Class::Name | Class::Unknown | Class::EnglishKeys | Class::LowKeys => true,
            Class::Term | Class::ShortTerm | Class::SignLed | Class::Address => false,
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Class::Frequent => "frequent",
            Class::Rare => "rare",
            Class::Name => "name",
            Class::Unknown => "unknown",
            Class::EnglishKeys => "english_keys",
            Class::LowKeys => "low_keys",
            Class::Term => "term",
            Class::ShortTerm => "short_term",
            Class::SignLed => "sign_led",
            Class::Address => "address",
        }
    }

    /// The hard negatives: words typed as meant that a reading of their keys most easily takes for the other layout. Typed
    /// in Latin, every token no lexicon of words holds (terms long and short, sign-led tokens, address parts) and the
    /// names: the long terms are what held the threshold of earlier fits (`verifycsrftoken`, `vgcfgbackup`).
    #[must_use]
    pub fn hard(self, group: i64) -> bool {
        match self {
            Class::SignLed | Class::ShortTerm | Class::EnglishKeys | Class::LowKeys => true,
            Class::Name | Class::Term | Class::Address => group == 0,
            Class::Frequent | Class::Rare | Class::Unknown => false,
        }
    }
}

/// What the field holds before the word.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Situation {
    Empty,
    RussianText,
    EnglishText,
    Comment,
    /// The text before the token in its own sentence: for parts of addresses only.
    Natural,
}

/// Every word draws its situations from these, by hash.
const SITUATIONS: [Situation; 4] = [Situation::Empty, Situation::RussianText, Situation::EnglishText, Situation::Comment];

impl Situation {
    /// The role of the field: an empty field's is unknown, as the replay's editor reports it.
    #[must_use]
    pub fn role(self) -> &'static str {
        match self {
            Situation::Empty => "unknown",
            Situation::RussianText | Situation::EnglishText | Situation::Natural => "text",
            Situation::Comment => "code",
        }
    }
}

/// The words each class of a language takes in one split.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Quotas {
    #[serde(default)]
    pub en: BTreeMap<Class, usize>,
    #[serde(default)]
    pub ru: BTreeMap<Class, usize>,
}

impl Quotas {
    fn of(&self, group: i64, class: Class) -> usize {
        let table = if group == 0 { &self.en } else { &self.ru };
        table.get(&class).copied().unwrap_or(0)
    }
}

/// Which words are hard by their first keys: those whose first `characters` keys their own layout's orthotactic channel
/// gives the lowest mean log-probability, the lowest `per_mille` of the words (names included) of their language.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LowKeysRule {
    pub characters: usize,
    pub per_mille: u64,
}

/// `model/prefix_v3/config.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WordConfig {
    pub schema_version: u32,
    pub kind: String,
    /// Where the frozen generation lives, relative to the tree.
    pub directory: String,
    pub namespace: String,
    pub minimum_word_length: usize,
    pub maximum_word_length: usize,
    pub frequent_words_per_language: usize,
    /// The share of physical families, in thousandths, that development and calibration take.
    pub development_per_mille: u64,
    pub calibration_per_mille: u64,
    pub situations_per_word: usize,
    pub sign_leaders: String,
    /// A Russian word reads like English when the English model explains its keys by at least this many nats more than the
    /// Russian one at some prefix from three keys to `english_keys_maximum_characters`.
    pub english_keys_minimum_nats: f64,
    pub english_keys_maximum_characters: usize,
    pub minimum_calibration_negatives: usize,
    pub minimum_calibration_hard_negatives: usize,
    pub quotas: BTreeMap<String, Quotas>,
    pub evidence_scope: String,
    /// Schema three: the hard words by their first keys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low_keys: Option<LowKeysRule>,
    /// Schema three: the thousandths of the families the acceptance split takes, after development and calibration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptance_per_mille: Option<u64>,
    /// Schema three: the fewest words typed as meant, and of them hard ones, development and calibration each hold in each
    /// direction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_words_per_direction: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_hard_words_per_direction: Option<usize>,
}

impl WordConfig {
    /// The splits this config writes.
    #[must_use]
    pub fn splits(&self) -> Vec<&'static str> {
        let mut splits = WORD_SPLITS.to_vec();
        if self.acceptance_per_mille.is_some_and(|share| share > 0) {
            splits.push(ACCEPTANCE_SPLIT);
        }
        splits
    }
}

/// A row of the corpus: one word in one situation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WordRow {
    pub before: String,
    pub class: Class,
    pub family: String,
    /// The layout group the word is meant in: 0 Latin, 1 Cyrillic.
    pub group: i64,
    pub paired: bool,
    pub role: String,
    pub situation: Situation,
    pub source: String,
    pub text: String,
    /// Typed as meant, its first keys read most easily as the other layout: a hard class, or a word whose first keys its
    /// own layout's orthotactic channel reads badly. Written only when true, as a corpus of schema two has no such field.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hard: bool,
}

#[expect(clippy::trivially_copy_pass_by_ref, reason = "serde hands a skipped field by reference")]
fn is_false(value: &bool) -> bool {
    !*value
}

/// The held-out development samples, by the aliases of their focus words.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeldOut {
    pub scope: String,
    /// The SHA-256 of each sample file and the rows it holds.
    pub files: Vec<HeldOutFile>,
    pub words: usize,
    /// The first sixteen hex digits of the SHA-256 of every physical alias of every focus word, sorted.
    pub aliases: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeldOutFile {
    pub sha256: String,
    pub rows: usize,
}

/// A pinned input of the corpus and what it gives.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRecord {
    pub name: String,
    pub pin: String,
    pub license: String,
    pub sha256: String,
    pub gives: String,
}

/// The counts of one split.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SplitCounts {
    pub rows: usize,
    pub words: usize,
    pub families: usize,
    /// Words by `<language>:<class>`.
    pub words_by_class: BTreeMap<String, usize>,
    /// Distinct texts typed in the other layout (paired words).
    pub convert_words: usize,
    /// Distinct texts typed as meant: every word.
    pub keep_words: usize,
    /// Of those, the hard ones (`Class::hard`, or by the first keys).
    pub hard_keep_words: usize,
    /// Schema three: the words typed as meant and the hard ones, by the layout group they are typed in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_words_by_group: Option<[usize; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hard_keep_words_by_group: Option<[usize; 2]>,
}

/// `corpus.json`: what the corpus was made of and what it holds.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WordReceipt {
    pub schema_version: u32,
    pub kind: String,
    pub config_sha256: String,
    pub generator_sha256: String,
    pub sources: Vec<SourceRecord>,
    pub held_out: HeldOutSummary,
    pub counts: BTreeMap<String, SplitCounts>,
    pub rows: BTreeMap<String, usize>,
    pub families: BTreeMap<String, usize>,
    pub family_overlap: usize,
    pub sha256: BTreeMap<String, String>,
    pub labels: String,
    pub situations: String,
    pub scope: String,
    /// Schema three: the mean log-probability, per language, at or under which a word's first keys make it hard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low_keys_cut_nats: Option<BTreeMap<String, f64>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeldOutSummary {
    pub file: String,
    pub sha256: String,
    pub files: usize,
    pub words: usize,
    pub aliases: usize,
}

/// A word before it is placed: its text as meant, its group, its class, where it came from and, for an address part, the
/// text before it in its sentence.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub text: String,
    pub group: i64,
    pub class: Class,
    pub source: String,
    pub natural: Option<String>,
    /// Typed as meant, its first keys read most easily as the other layout (`WordRow::hard`).
    pub hard: bool,
}

/// A row of the base corpus, the fields read here.
#[derive(Deserialize)]
struct BaseRow {
    original: String,
    upos: String,
    source: String,
    before: String,
    split: String,
}

#[derive(Deserialize)]
struct BaseManifest {
    namespace: String,
    splits: BTreeMap<String, BaseSplit>,
}

#[derive(Deserialize)]
struct BaseSplit {
    path: String,
    sha256: String,
}

#[derive(Deserialize)]
struct Identifiers {
    identifiers: Vec<String>,
}

#[derive(Deserialize)]
struct TermTable {
    latin: BTreeMap<String, i64>,
}

/// A row of a development sample: its focus word.
#[derive(Deserialize)]
struct SampleRow {
    original: String,
}

fn compact() -> JsonOptions {
    JsonOptions { ensure_ascii: false, separators: Some((",".to_owned(), ":".to_owned())), sort_keys: true, ..JsonOptions::default() }
}

fn readable() -> JsonOptions {
    JsonOptions { ensure_ascii: false, indent: Some(Indent::Spaces(1)), sort_keys: true, ..JsonOptions::default() }
}

/// A deterministic 64-bit draw named by a text.
#[must_use]
pub fn draw(text: &str) -> u64 {
    u64::from_str_radix(hex_digest(text).get(..DRAW_HEX_DIGITS).unwrap_or_default(), 16).expect("hex digits")
}

fn read_gzip_lines(path: &Path) -> Result<Vec<String>, String> {
    let file = std::fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    std::io::BufReader::new(MultiGzDecoder::new(file)).lines().collect::<Result<Vec<String>, _>>().map_err(|error| format!("{}: {error}", path.display()))
}

fn letters_of(word: &str, group: i64) -> bool {
    !word.is_empty() && word.chars().all(|character| if group == 0 { character.is_ascii_alphabetic() } else { ('а'..='я').contains(&character) || ('А'..='Я').contains(&character) || character == 'ё' || character == 'Ё' })
}

/// The group of a word all of whose letters are of one script, lower case.
fn lower_group(word: &str) -> Option<i64> {
    if letters_of(word, 0) && word.chars().all(|character| character.is_ascii_lowercase()) {
        Some(0)
    } else if letters_of(word, 1) && word.chars().all(|character| !pystr::isupper(character.encode_utf8(&mut [0_u8; 4]))) {
        Some(1)
    } else {
        None
    }
}

/// A name as typed: a capital and then small letters of one script.
#[must_use]
pub fn name_form(word: &str) -> Option<(String, i64)> {
    let mut characters = word.chars();
    let first = characters.next()?;
    let rest: String = characters.collect();
    let group = lower_group(&rest)?;
    let capital = first.to_uppercase().collect::<String>();
    (letters_of(&capital, group) && capital.chars().count() == 1 && capital != first.to_lowercase().collect::<String>()).then(|| (format!("{capital}{rest}"), group))
}

/// The parts of an address or a link the engine reads as tokens: runs of Latin letters and of keys that are letters of
/// the Russian layout, cut at every other sign, without the signs that end them, each opening with a letter. Each part
/// comes with the text of the token before it.
#[must_use]
pub fn address_parts(token: &str) -> Vec<(String, String)> {
    let characters: Vec<char> = token.chars().collect();
    let inside = |character: char| character.is_ascii_alphabetic() || LAYOUT_LETTER_SIGNS.contains(character);
    let mut parts = Vec::new();
    let mut index = 0;
    while index < characters.len() {
        if !characters[index].is_ascii_alphabetic() {
            index += 1;
            continue;
        }
        let start = index;
        while index < characters.len() && inside(characters[index]) {
            index += 1;
        }
        let mut end = index;
        while end > start && !characters[end - 1].is_ascii_alphabetic() {
            end -= 1;
        }
        parts.push((characters[start..end].iter().collect(), characters[..start].iter().collect()));
    }
    parts
}

/// An address or a link, as a treebank token: an `@` between letters, or a scheme, `www.` or a domain ending.
#[must_use]
pub fn is_address(token: &str) -> bool {
    let lower = pystr::casefold(token);
    let at = token.find('@').is_some_and(|place| place > 0 && place + 1 < token.len());
    at || lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("www.") || [".com", ".org", ".net", ".gov", ".edu"].iter().any(|ending| lower.contains(ending))
}

/// The largest margin, in nats, by which the English channel explains the keys of a word's prefix better than the Russian
/// one, over prefixes of `ENGLISH_KEYS_MIN_CHARACTERS` to `maximum` keys: the two channels' conditionals of the keys.
#[must_use]
pub fn english_margin(english: &[f64], russian: &[f64], maximum: usize) -> Option<f64> {
    let mut best: Option<f64> = None;
    let (mut sum_english, mut sum_russian) = (0.0, 0.0);
    for (index, (left, right)) in english.iter().zip(russian).enumerate() {
        sum_english += left;
        sum_russian += right;
        let length = index + 1;
        if (ENGLISH_KEYS_MIN_CHARACTERS..=maximum).contains(&length) {
            let margin = sum_english - sum_russian;
            best = Some(best.map_or(margin, |known: f64| known.max(margin)));
        }
    }
    best
}

/// What the corpus is made of, read once.
struct Pools {
    /// Words of the Tatoeba snapshot by (form, group), lower case, and how often each occurs.
    tatoeba: BTreeMap<(String, i64), i64>,
    /// The snapshot's phrases by group, the texts the situations put before a word.
    phrases: [Vec<String>; 2],
    /// Words of the base corpus's training split: (form, group) to its treebank, lower case.
    treebank_words: BTreeMap<(String, i64), String>,
    /// Names of the base corpus's training split: (form as typed, group) to its treebank.
    names: BTreeMap<(String, i64), String>,
    /// Latin tokens the tree knows as terms, lower case, with where each came from.
    terms: BTreeMap<String, String>,
    /// Address tokens of the base training split with the text before each.
    addresses: Vec<(String, String, String)>,
}

fn in_bounds(config: &WordConfig, text: &str) -> bool {
    (config.minimum_word_length..=config.maximum_word_length).contains(&text.chars().count())
}

fn base_rows(base: &Path) -> Result<(Vec<BaseRow>, String, String), String> {
    let manifest: BaseManifest = ks_text::json::loads(std::fs::read(base.join("manifest.json")).map_err(|error| error.to_string())?).map_err(|error| format!("the base manifest: {error}"))?;
    let record = manifest.splits.get("train").ok_or("a base without a training split")?;
    let path = base.join(&record.path);
    let digest = checksum(&path)?;
    if digest != record.sha256 {
        return Err("the base training split differs from its manifest".to_owned());
    }
    let mut rows = Vec::new();
    for line in read_gzip_lines(&path)? {
        let row: BaseRow = ks_text::json::loads(&line).map_err(|error| format!("a base row: {error}"))?;
        if row.split != BASE_TRAINING_SPLIT {
            return Err("a base training split holds a row of another split".to_owned());
        }
        rows.push(row);
    }
    Ok((rows, manifest.namespace, digest))
}

fn pools(root: &Path, config: &WordConfig, base: &[BaseRow]) -> Result<Pools, String> {
    let mut tatoeba: BTreeMap<(String, i64), i64> = BTreeMap::new();
    let mut phrases: [Vec<String>; 2] = [Vec::new(), Vec::new()];
    for phrase in load_phrases(root)? {
        let group = i64::from(phrase.locale == TATOEBA_RUSSIAN);
        phrases[group.to::<usize>()].push(phrase.text.clone());
        let characters: Vec<char> = phrase.text.chars().collect();
        for (_, found) in word_matches(&characters) {
            let word = pystr::casefold(&found);
            if in_bounds(config, &word)
                && let Some(found_group) = lower_group(&word) {
                    *tatoeba.entry((word, found_group)).or_insert(0) += 1;
                }
        }
    }
    let (mut treebank_words, mut names, mut terms) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
    let mut addresses = Vec::new();
    for row in base {
        if row.source.starts_with("Debian") {
            let command = pystr::casefold(&row.original);
            if lower_group(&command) == Some(0) {
                terms.entry(command).or_insert_with(|| row.source.clone());
            }
            continue;
        }
        if !row.source.starts_with("UD_") {
            continue;
        }
        if row.source == ADDRESS_TREEBANK && is_address(&row.original) {
            addresses.push((row.original.clone(), row.before.clone(), row.source.clone()));
            continue;
        }
        if !in_bounds(config, &row.original) {
            continue;
        }
        if row.upos == NAME_TAG {
            if let Some(found) = name_form(&row.original).filter(|(form, _)| *form == row.original) {
                names.entry(found).or_insert_with(|| row.source.clone());
            }
            continue;
        }
        let word = pystr::casefold(&row.original);
        if let Some(group) = lower_group(&word) {
            treebank_words.entry((word, group)).or_insert_with(|| row.source.clone());
        }
    }
    let identifiers: Identifiers = ks_text::json::loads(std::fs::read(root.join(IDENTIFIERS)).map_err(|error| error.to_string())?).map_err(|error| format!("{IDENTIFIERS}: {error}"))?;
    for identifier in identifiers.identifiers {
        terms.entry(pystr::casefold(&identifier)).or_insert_with(|| IDENTIFIERS.to_owned());
    }
    let protected = std::fs::read_to_string(root.join(PROTECTED_TOKENS)).map_err(|error| error.to_string())?;
    for line in protected.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#')) {
        terms.entry(pystr::casefold(line)).or_insert_with(|| PROTECTED_TOKENS.to_owned());
    }
    let table: TermTable = ks_text::json::loads(std::fs::read(root.join(TERM_TABLE)).map_err(|error| error.to_string())?).map_err(|error| format!("{TERM_TABLE}: {error}"))?;
    for token in table.latin.into_keys() {
        terms.entry(pystr::casefold(&token)).or_insert_with(|| TERM_TABLE.to_owned());
    }
    terms.retain(|term, _| lower_group(term) == Some(0) && in_bounds(config, term));
    Ok(Pools { tatoeba, phrases, treebank_words, names, terms, addresses })
}

/// The class of a paired lower-case word: Russian words that read like English first, then words whose first keys their
/// own layout reads badly, then the frequent ones, then by whether the onboard lexicon knows the form.
fn word_class(word: &str, group: i64, frequent: &BTreeSet<(String, i64)>, known: bool, english_like: bool, low_keys: bool) -> Class {
    if group == 1 && english_like {
        Class::EnglishKeys
    } else if low_keys {
        Class::LowKeys
    } else if frequent.contains(&(word.to_owned(), group)) {
        Class::Frequent
    } else if known {
        Class::Rare
    } else {
        Class::Unknown
    }
}

/// The keys of the US layout a word is typed with.
fn keys_of(text: &str, group: i64) -> String {
    pystr::casefold(&if group == 1 { crate::physical::pair().translate(text, "ru", "us").expect("the pair of layouts") } else { text.to_owned() })
}

/// The mean log-probability its own layout's orthotactic channel gives the first `characters` keys of a word typed as
/// meant; none for a word without keys.
#[must_use]
pub fn own_keys_mean(ortho: &OrthoModel, text: &str, group: i64, characters: usize) -> Option<f64> {
    let channel = if group == 1 { &ortho.channels.ru } else { &ortho.channels.en };
    let conditionals: Vec<f64> = prefix_conditionals(channel, &keys_of(text, group)).into_iter().take(characters).collect();
    (!conditionals.is_empty()).then(|| ks_lang::pysum::py_sum(conditionals.iter().copied()) / conditionals.len().to::<f64>())
}

/// The mean at the rule's thousandth from the bottom among these means: a word at or under it is hard by its first keys.
#[must_use]
pub fn low_keys_cut(means: &mut [f64], per_mille: u64) -> Option<f64> {
    means.sort_by(f64::total_cmp);
    let place = (means.len().to::<u64>() * per_mille / PER_MILLE).to::<usize>();
    means.get(place.min(means.len().saturating_sub(1))).copied()
}

/// Which words are hard by their first keys: the cut of each language, over its names and words, and the test.
struct LowKeys<'a> {
    ortho: &'a OrthoModel,
    rule: Option<LowKeysRule>,
    cuts: [Option<f64>; 2],
}

impl LowKeys<'_> {
    fn is_low(&self, text: &str, group: i64) -> bool {
        let (Some(rule), Some(cut)) = (self.rule, self.cuts[usize::from(group == 1)]) else {
            return false;
        };
        own_keys_mean(self.ortho, text, group, rule.characters).is_some_and(|mean| mean <= cut)
    }
}

/// Every candidate once, held-out words left out. Names come first, then words, then tokens typed only as meant; a
/// term that is a word of the corpus stays a word.
fn candidates(root: &Path, resources: &Arc<Resources>, config: &WordConfig, pools: &Pools, held_out: &HashSet<String>) -> Result<(Vec<Candidate>, [Option<f64>; 2]), String> {
    let models = reference_models(root, resources, false)?;
    let ortho = OrthoModel::load(&artifact_path(resources.resource_dir()), None, None, Some(resources.clone())).map_err(|error| format!("the orthotactic model: {}", error.name))?;
    let mut low = LowKeys { ortho: &ortho, rule: config.low_keys, cuts: [None, None] };
    if let Some(rule) = config.low_keys {
        let mut means: [Vec<f64>; 2] = [Vec::new(), Vec::new()];
        let named = pools.names.keys().map(|(text, group)| (text.as_str(), *group));
        for (text, group) in named.chain(pools.tatoeba.keys().chain(pools.treebank_words.keys()).map(|(text, group)| (text.as_str(), *group))) {
            if let Some(mean) = own_keys_mean(&ortho, text, group, rule.characters) {
                means[usize::from(group == 1)].push(mean);
            }
        }
        low.cuts = means.map(|mut found| low_keys_cut(&mut found, rule.per_mille));
    }
    let mut by_count: Vec<(&(String, i64), &i64)> = pools.tatoeba.iter().collect();
    by_count.sort_by(|left, right| right.1.cmp(left.1).then_with(|| left.0.cmp(right.0)));
    let mut frequent: BTreeSet<(String, i64)> = BTreeSet::new();
    let mut per_group = [0_usize; 2];
    for (key, _) in by_count {
        let taken = &mut per_group[key.1.to::<usize>()];
        if *taken < config.frequent_words_per_language {
            *taken += 1;
            frequent.insert(key.clone());
        }
    }
    let leaders: Vec<char> = config.sign_leaders.chars().collect();
    if leaders.is_empty() {
        return Err("a word corpus needs sign leaders".to_owned());
    }
    let held = |text: &str| expanded_aliases(text).iter().any(|alias| held_out.contains(alias.get(..HELD_OUT_ALIAS_HEX_DIGITS).unwrap_or_default()));
    let mut chosen: BTreeMap<(String, i64), Candidate> = BTreeMap::new();
    let add = |candidate: Candidate, chosen: &mut BTreeMap<(String, i64), Candidate>| {
        let key = (candidate.text.clone(), candidate.group);
        if !chosen.contains_key(&key) && !held(&candidate.text) {
            chosen.insert(key, candidate);
        }
    };
    for ((name, group), source) in &pools.names {
        add(Candidate { text: name.clone(), group: *group, class: Class::Name, source: source.clone(), natural: None, hard: Class::Name.hard(*group) || low.is_low(name, *group) }, &mut chosen);
    }
    let mut words: BTreeMap<(String, i64), String> = pools.tatoeba.keys().map(|key| (key.clone(), TATOEBA_SNAPSHOT.to_owned())).collect();
    for (key, source) in &pools.treebank_words {
        words.entry(key.clone()).or_insert_with(|| source.clone());
    }
    for ((word, group), source) in &words {
        let known = models[group.to::<usize>()].frequencies.contains(word);
        let english_like = *group == 1 && {
            let keys = pystr::casefold(&crate::physical::pair().translate(word, "ru", "us").expect("the pair of layouts"));
            let (english, russian) = (prefix_conditionals(&ortho.channels.en, &keys), prefix_conditionals(&ortho.channels.ru, &keys));
            english_margin(&english, &russian, config.english_keys_maximum_characters).is_some_and(|margin| margin >= config.english_keys_minimum_nats)
        };
        let class = word_class(word, *group, &frequent, known, english_like, low.is_low(word, *group));
        add(Candidate { text: word.clone(), group: *group, class, source: source.clone(), natural: None, hard: class.hard(*group) }, &mut chosen);
    }
    let english: Vec<Candidate> = chosen.values().filter(|candidate| candidate.group == 0).cloned().collect();
    for word in english {
        let leader = leaders[(draw(&format!("{}:sign:{}", config.namespace, word.text)) % leaders.len().to::<u64>()).to::<usize>()];
        let text = format!("{leader}{}", word.text);
        if in_bounds(config, &text) {
            add(Candidate { text, group: 0, class: Class::SignLed, source: word.source.clone(), natural: None, hard: true }, &mut chosen);
        }
    }
    let known_words: BTreeSet<String> = chosen.values().filter(|candidate| candidate.class.paired()).map(|candidate| pystr::casefold(&candidate.text)).collect();
    for (term, source) in &pools.terms {
        // A token whose keys read as a Russian word is that word typed in the wrong layout as often as a term: it would
        // be kept here and converted as the Russian word.
        let reading = crate::physical::pair().translate(term, "us", "ru").expect("the pair of layouts");
        if known_words.contains(term) || known_words.contains(&reading) || models[1].frequencies.contains(&reading) {
            continue;
        }
        let class = if term.chars().count() <= 4 { Class::ShortTerm } else { Class::Term };
        add(Candidate { text: term.clone(), group: 0, class, source: source.clone(), natural: None, hard: class.hard(0) || low.is_low(term, 0) }, &mut chosen);
    }
    for (token, before, source) in &pools.addresses {
        for (part, inside) in address_parts(token) {
            if in_bounds(config, &part) {
                let hard = low.is_low(&part, 0);
                add(Candidate { text: part, group: 0, class: Class::Address, source: source.clone(), natural: Some(format!("{before}{inside}")), hard }, &mut chosen);
            }
        }
    }
    Ok((chosen.into_values().collect(), low.cuts))
}

/// The split of a physical family: development, calibration and acceptance take their thousandths by hash, training the
/// rest.
#[must_use]
pub fn split_of(config: &WordConfig, family: &str) -> &'static str {
    let bucket = draw(&format!("{}:split:{family}", config.namespace)) % PER_MILLE;
    let calibration = config.development_per_mille + config.calibration_per_mille;
    if bucket < config.development_per_mille {
        "development"
    } else if bucket < calibration {
        "calibration"
    } else if bucket < calibration + config.acceptance_per_mille.unwrap_or(0) {
        ACCEPTANCE_SPLIT
    } else {
        "train"
    }
}

/// The opening of a phrase of the snapshot, to a word boundary and a space: what a field holds before the word.
fn phrase_opening(phrases: &[String], name: &str) -> String {
    if phrases.is_empty() {
        return String::new();
    }
    let phrase = &phrases[(draw(&format!("{name}:phrase")) % phrases.len().to::<u64>()).to::<usize>()];
    let words: Vec<&str> = phrase.split_whitespace().collect();
    let count = 1 + (draw(&format!("{name}:words")) % words.len().saturating_sub(1).max(1).to::<u64>()).to::<usize>();
    format!("{} ", words[..count.min(words.len())].join(" "))
}

/// The situations of a candidate: its own sentence first when it has one, then the rest drawn from the four by hash.
fn situations_of(config: &WordConfig, candidate: &Candidate) -> Vec<Situation> {
    let mut drawn = SITUATIONS.to_vec();
    drawn.sort_by_key(|situation| (draw(&format!("{}:situation:{}:{}:{situation:?}", config.namespace, candidate.group, candidate.text)), *situation));
    let mut result = Vec::new();
    if candidate.natural.is_some() {
        result.push(Situation::Natural);
    }
    result.extend(drawn.into_iter().take(config.situations_per_word.saturating_sub(result.len())));
    result
}

fn before_of(config: &WordConfig, pools: &Pools, candidate: &Candidate, situation: Situation) -> String {
    let name = format!("{}:before:{}:{}:{situation:?}", config.namespace, candidate.group, candidate.text);
    match situation {
        Situation::Empty => String::new(),
        Situation::RussianText => phrase_opening(&pools.phrases[1], &name),
        Situation::EnglishText => phrase_opening(&pools.phrases[0], &name),
        Situation::Comment => format!("{COMMENT_OPENING}{}", phrase_opening(&pools.phrases[(draw(&format!("{name}:language")) % 2).to::<usize>()], &name)),
        Situation::Natural => candidate.natural.clone().unwrap_or_default(),
    }
}

/// The rows of every split: each split's candidates of each language and class in hash order, up to the quota.
fn rows(config: &WordConfig, pools: &Pools, candidates: &[Candidate]) -> Result<BTreeMap<&'static str, Vec<WordRow>>, String> {
    let mut grouped: BTreeMap<(&'static str, i64, Class), Vec<&Candidate>> = BTreeMap::new();
    for candidate in candidates {
        let split = split_of(config, &family(&candidate.text, candidate.group));
        grouped.entry((split, candidate.group, candidate.class)).or_default().push(candidate);
    }
    let mut result: BTreeMap<&'static str, Vec<WordRow>> = config.splits().into_iter().map(|split| (split, Vec::new())).collect();
    for ((split, group, class), mut members) in grouped {
        let quota = config.quotas.get(split).ok_or_else(|| format!("no quotas for {split}"))?.of(group, class);
        members.sort_by_cached_key(|candidate| (draw(&format!("{}:select:{}:{}", config.namespace, class.as_str(), candidate.text)), candidate.text.clone()));
        for candidate in members.into_iter().take(quota) {
            for situation in situations_of(config, candidate) {
                result.get_mut(split).expect("a split").push(WordRow {
                    before: before_of(config, pools, candidate, situation),
                    class,
                    family: family(&candidate.text, group),
                    group,
                    paired: class.paired(),
                    role: situation.role().to_owned(),
                    situation,
                    source: candidate.source.clone(),
                    text: candidate.text.clone(),
                    // A corpus of schema two has no hard words by their first keys and writes none.
                    hard: config.low_keys.is_some() && candidate.hard,
                });
            }
        }
    }
    for rows in result.values_mut() {
        rows.sort_by(|left, right| (&left.family, &left.text, left.group, left.situation).cmp(&(&right.family, &right.text, right.group, right.situation)));
    }
    Ok(result)
}

/// What a split holds.
#[must_use]
pub fn split_counts(rows: &[WordRow]) -> SplitCounts {
    let mut words: BTreeMap<(String, i64), (Class, bool, bool)> = BTreeMap::new();
    let families: BTreeSet<&str> = rows.iter().map(|row| row.family.as_str()).collect();
    for row in rows {
        words.insert((row.text.clone(), row.group), (row.class, row.paired, row.hard));
    }
    let mut counts = SplitCounts { rows: rows.len(), words: words.len(), families: families.len(), ..SplitCounts::default() };
    let (mut by_group, mut hard_by_group) = ([0_usize; 2], [0_usize; 2]);
    for ((_, group), (class, paired, hard)) in &words {
        *counts.words_by_class.entry(format!("{}:{}", if *group == 0 { "en" } else { "ru" }, class.as_str())).or_insert(0) += 1;
        counts.convert_words += usize::from(*paired);
        counts.keep_words += 1;
        let hard = class.hard(*group) || *hard;
        counts.hard_keep_words += usize::from(hard);
        by_group[usize::from(*group == 1)] += 1;
        hard_by_group[usize::from(*group == 1)] += usize::from(hard);
    }
    // Rows of schema three say which words are hard by their first keys: the cells of each direction are counted then.
    if rows.iter().any(|row| row.hard) {
        (counts.keep_words_by_group, counts.hard_keep_words_by_group) = (Some(by_group), Some(hard_by_group));
    }
    counts
}

/// The held-out aliases of the focus words of development samples.
pub fn held_out_of(samples: &[PathBuf]) -> Result<HeldOut, String> {
    let (mut words, mut aliases, mut files) = (BTreeSet::new(), BTreeSet::new(), Vec::new());
    for path in samples {
        // A sample is the rows of a development corpus, or a list of words, one a line.
        let originals: Vec<String> = if path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("txt")) {
            let text = std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
            text.lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_owned).collect()
        } else {
            read_gzip_lines(path)?.iter().map(|line| ks_text::json::loads::<SampleRow>(line).map(|row| row.original).map_err(|error| format!("{}: {error}", path.display()))).collect::<Result<_, _>>()?
        };
        for original in &originals {
            if words.insert(pystr::casefold(original)) {
                aliases.extend(expanded_aliases(original).into_iter().map(|alias| alias.get(..HELD_OUT_ALIAS_HEX_DIGITS).unwrap_or_default().to_owned()));
            }
        }
        files.push(HeldOutFile { sha256: checksum(path)?, rows: originals.len() });
    }
    files.sort_by(|left, right| left.sha256.cmp(&right.sha256));
    Ok(HeldOut {
        scope: "focus words of development samples left out of the corpus, by the first sixteen hex digits of the SHA-256 of each physical alias".to_owned(),
        files,
        words: words.len(),
        aliases: aliases.into_iter().collect(),
    })
}

fn gzip(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(bytes).map_err(|error| error.to_string())?;
    encoder.finish().map_err(|error| error.to_string())
}

fn gunzip(path: &Path) -> Result<Vec<u8>, String> {
    let mut raw = Vec::new();
    MultiGzDecoder::new(std::fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?).read_to_end(&mut raw).map_err(|error| error.to_string())?;
    Ok(raw)
}

/// A calibration smaller than the config asks for is no corpus: its words typed as meant and the hard ones, in all and in
/// each direction; and the development split holds as many in each direction, since an epoch is chosen on it.
fn cells_hold(config: &WordConfig, development: &SplitCounts, calibration: &SplitCounts) -> Result<(), String> {
    if calibration.keep_words < config.minimum_calibration_negatives || calibration.hard_keep_words < config.minimum_calibration_hard_negatives {
        return Err(format!("calibration holds {} negative words, {} hard: the config asks for {} and {}", calibration.keep_words, calibration.hard_keep_words, config.minimum_calibration_negatives, config.minimum_calibration_hard_negatives));
    }
    let (words, hard) = (config.minimum_words_per_direction.unwrap_or(0), config.minimum_hard_words_per_direction.unwrap_or(0));
    for (split, counts) in [("development", development), ("calibration", calibration)] {
        let (by_group, hard_by_group) = (counts.keep_words_by_group.unwrap_or_default(), counts.hard_keep_words_by_group.unwrap_or_default());
        for group in 0..2 {
            if by_group[group] < words || hard_by_group[group] < hard {
                return Err(format!("{split} typed in layout group {group} holds {} words, {} hard: the config asks for {words} and {hard}", by_group[group], hard_by_group[group]));
            }
        }
    }
    Ok(())
}

/// What a freeze or a verification needs.
pub struct WordCorpusRequest<'a> {
    pub root: &'a Path,
    pub resources: &'a Arc<Resources>,
    pub config: &'a Path,
    pub base: &'a Path,
    /// The development samples to leave out, for a freeze; a verification reads the frozen held-out file.
    pub hold_out: &'a [PathBuf],
    pub output: Option<&'a Path>,
    /// A freeze into `output` that makes the tree's corpus again from its sources: its receipt must be the tree's, byte
    /// for byte, as the files are not tracked and the receipt is.
    pub rebuild: bool,
}

/// The bytes of a generation: each split's content and the receipt, from the held-out aliases it is given.
fn generation(request: &WordCorpusRequest<'_>, config: &WordConfig, held_out: &HeldOut, held_out_bytes: &[u8]) -> Result<(BTreeMap<&'static str, Vec<u8>>, Vec<u8>, BTreeMap<String, SplitCounts>), String> {
    if !WORD_CORPUS_SCHEMAS.contains(&config.schema_version) || config.kind != WORD_CORPUS_KIND {
        return Err("not a word corpus config".to_owned());
    }
    let three = config.schema_version == WORD_CORPUS_SCHEMAS[1];
    if three != (config.low_keys.is_some() && config.acceptance_per_mille.is_some()) {
        return Err("a word corpus of schema three, and only one, names its hard words by their first keys and its acceptance split".to_owned());
    }
    let (base, namespace, base_digest) = base_rows(request.base)?;
    let pools = pools(request.root, config, &base)?;
    let aliases: HashSet<String> = held_out.aliases.iter().cloned().collect();
    let (candidates, cuts) = candidates(request.root, request.resources, config, &pools, &aliases)?;
    let rows = rows(config, &pools, &candidates)?;
    let mut families: BTreeMap<String, usize> = BTreeMap::new();
    let mut seen: BTreeMap<&str, &str> = BTreeMap::new();
    let mut overlap = 0;
    for (split, split_rows) in &rows {
        for row in split_rows {
            let held = *seen.entry(row.family.as_str()).or_insert(*split);
            overlap += usize::from(held != *split);
        }
    }
    if overlap != 0 {
        return Err("physical families overlap between splits".to_owned());
    }
    let mut content = BTreeMap::new();
    let mut counts = BTreeMap::new();
    let mut digests = BTreeMap::new();
    let mut row_counts = BTreeMap::new();
    for (split, split_rows) in &rows {
        let bytes: Vec<u8> = split_rows.iter().flat_map(|row| (ks_text::json::dumps(row, &compact()) + "\n").into_bytes()).collect();
        digests.insert((*split).to_owned(), hex(&Sha256::digest(gzip(&bytes)?)));
        content.insert(*split, bytes);
        let split_count = split_counts(split_rows);
        families.insert((*split).to_owned(), split_count.families);
        row_counts.insert((*split).to_owned(), split_count.rows);
        counts.insert((*split).to_owned(), split_count);
    }
    cells_hold(config, &counts.get("development").cloned().unwrap_or_default(), &counts.get("calibration").cloned().unwrap_or_default())?;
    let tree_file = |name: &str| checksum(&request.root.join(name));
    let ud = crate::base_corpus::BASE_PINS.iter().map(|(name, commit)| format!("{name}@{commit}")).collect::<Vec<_>>().join(", ");
    let sources = vec![
        SourceRecord { name: TATOEBA_SNAPSHOT.to_owned(), pin: "Tatoeba CC0 export of 2026-08-29 (model/context_v2/sources/README.md)".to_owned(), license: "CC0 1.0".to_owned(), sha256: tree_file(TATOEBA_SNAPSHOT)?, gives: "word counts (frequent, rare and unknown words) and the phrases put before a word".to_owned() },
        SourceRecord { name: format!("base corpus {namespace}, training split"), pin: format!("train.jsonl.gz of the frozen base; treebanks {ud}; Debian trixie main amd64 Contents"), license: "UD_English-EWT and UD_Russian-Taiga CC BY-SA 4.0, UD_Russian-SynTagRus CC BY-NC-SA 4.0 (their LICENSE.txt at the pinned commits); Debian file names: the Contents index states no license".to_owned(), sha256: base_digest, gives: "treebank words and names (PROPN), Debian commands, address parts of UD_English-EWT".to_owned() },
        SourceRecord { name: IDENTIFIERS.to_owned(), pin: "Debian trixie main amd64 Contents (its source block)".to_owned(), license: "the Contents index states no license".to_owned(), sha256: tree_file(IDENTIFIERS)?, gives: "terms and short terms".to_owned() },
        SourceRecord { name: PROTECTED_TOKENS.to_owned(), pin: "this tree".to_owned(), license: "the project's".to_owned(), sha256: tree_file(PROTECTED_TOKENS)?, gives: "terms and short terms".to_owned() },
        SourceRecord { name: format!("{TERM_TABLE} (latin)"), pin: "ru.stackoverflow.com via Hugging Face IlyaGusev/ru_stackoverflow@3bfd79e5, training split".to_owned(), license: "CC BY-SA 4.0".to_owned(), sha256: tree_file(TERM_TABLE)?, gives: "Latin tokens of Russian technical text: terms and short terms".to_owned() },
        SourceRecord { name: "model/intent_v1/sources/*.lm, resources/lexicon-supplement-ru_RU.json".to_owned(), pin: "model/intent_v1/config.json".to_owned(), license: "model/intent_v1/sources/COPYRIGHT.onboard-data".to_owned(), sha256: tree_file("model/intent_v1/config.json")?, gives: "whether the onboard lexicon knows a form (rare or unknown)".to_owned() },
        SourceRecord { name: "resources/models/ortho_v1.json".to_owned(), pin: "this tree".to_owned(), license: "the project's".to_owned(), sha256: tree_file("resources/models/ortho_v1.json")?, gives: "which Russian words read like English by their keys".to_owned() },
    ];
    let receipt = WordReceipt {
        schema_version: config.schema_version,
        kind: WORD_CORPUS_KIND.to_owned(),
        config_sha256: checksum(request.config)?,
        generator_sha256: tree_file(GENERATOR)?,
        sources,
        held_out: HeldOutSummary { file: HELD_OUT_FILE.to_owned(), sha256: hex(&Sha256::digest(held_out_bytes)), files: held_out.files.len(), words: held_out.words, aliases: held_out.aliases.len() },
        counts: counts.clone(),
        rows: row_counts,
        families,
        family_overlap: overlap,
        sha256: digests,
        labels: "made by the trainer from each row: a paired word typed as meant is kept at every length; typed in the other layout it converts from the recipe's number of letters, and waits before it, while the typed prefix continues a word of the typed layout's lexicon or a known term; every other row is kept".to_owned(),
        situations: "each word in three of four situations drawn by hash (an empty field of unknown role, Russian text, English text, a code comment); an address part in its own sentence and two of the four; one application for every row".to_owned(),
        scope: config.evidence_scope.clone(),
        low_keys_cut_nats: three.then(|| ["en", "ru"].iter().zip(cuts).filter_map(|(language, cut)| Some(((*language).to_owned(), cut?))).collect()),
    };
    let receipt_bytes = (ks_text::json::dumps(&receipt, &readable()) + "\n").into_bytes();
    Ok((content, receipt_bytes, counts))
}

/// `prefix-corpus --config`: verify the frozen generation the config names, or freeze a new one into `output`.
pub fn command(request: &WordCorpusRequest<'_>) -> Result<BTreeMap<String, SplitCounts>, String> {
    let config: WordConfig = ks_text::json::loads(std::fs::read(request.config).map_err(|error| format!("{}: {error}", request.config.display()))?).map_err(|error| format!("{}: {error}", request.config.display()))?;
    let Some(output) = request.output else {
        let directory = request.root.join(&config.directory);
        let held_out_bytes = std::fs::read(directory.join(HELD_OUT_FILE)).map_err(|error| error.to_string())?;
        let held_out: HeldOut = ks_text::json::loads(gunzip(&directory.join(HELD_OUT_FILE))?).map_err(|error| format!("{HELD_OUT_FILE}: {error}"))?;
        let (content, receipt, counts) = generation(request, &config, &held_out, &held_out_bytes)?;
        let mut differences = 0;
        for split in config.splits() {
            let same = gunzip(&directory.join(format!("{split}.jsonl.gz")))? == content[split];
            println!("{split}: {}", if same { "same" } else { "DIFFERS" });
            differences += usize::from(!same);
        }
        let same = std::fs::read(directory.join(RECEIPT_FILE)).map_err(|error| error.to_string())? == receipt;
        println!("{RECEIPT_FILE}: {}", if same { "same" } else { "DIFFERS" });
        differences += usize::from(!same);
        return if differences == 0 { Ok(counts) } else { Err(format!("{differences} files of the word corpus differ")) };
    };
    if request.hold_out.is_empty() {
        return Err("a freeze names the development samples to leave out (--hold-out)".to_owned());
    }
    for name in config.splits().iter().map(|split| format!("{split}.jsonl.gz")).chain([RECEIPT_FILE.to_owned(), HELD_OUT_FILE.to_owned()]) {
        if output.join(&name).exists() {
            return Err("refusing to overwrite a frozen word corpus".to_owned());
        }
    }
    let held_out = held_out_of(request.hold_out)?;
    let held_out_bytes = gzip((ks_text::json::dumps(&held_out, &compact()) + "\n").as_bytes())?;
    let (content, receipt, counts) = generation(request, &config, &held_out, &held_out_bytes)?;
    std::fs::create_dir_all(output).map_err(|error| error.to_string())?;
    std::fs::write(output.join(HELD_OUT_FILE), &held_out_bytes).map_err(|error| error.to_string())?;
    for split in config.splits() {
        std::fs::write(output.join(format!("{split}.jsonl.gz")), gzip(&content[split])?).map_err(|error| error.to_string())?;
    }
    if !output.join(CONFIG_FILE).exists() {
        std::fs::copy(request.config, output.join(CONFIG_FILE)).map_err(|error| error.to_string())?;
    }
    std::fs::write(output.join(RECEIPT_FILE), &receipt).map_err(|error| error.to_string())?;
    if request.rebuild {
        let tracked = request.root.join(&config.directory).join(RECEIPT_FILE);
        if std::fs::read(&tracked).map_err(|error| format!("{}: {error}", tracked.display()))? != receipt {
            return Err(format!("the rebuilt word corpus differs from its receipt {}", tracked.display()));
        }
        println!("{RECEIPT_FILE}: same as the tree's");
    }
    Ok(counts)
}

/// A frozen split of a word corpus, its checksum checked against the receipt.
pub fn read_split(directory: &Path, split: &str) -> Result<Vec<WordRow>, String> {
    if !WORD_SPLITS.contains(&split) && split != ACCEPTANCE_SPLIT {
        return Err("a word corpus has no such split".to_owned());
    }
    /// What a reader of the splits needs of the receipt: the checksum of each.
    #[derive(Deserialize)]
    struct Pins {
        sha256: BTreeMap<String, String>,
    }
    let receipt: Pins = ks_text::json::loads(std::fs::read(directory.join(RECEIPT_FILE)).map_err(|error| error.to_string())?).map_err(|error| format!("{RECEIPT_FILE}: {error}"))?;
    let path = directory.join(format!("{split}.jsonl.gz"));
    if receipt.sha256.get(split) != Some(&checksum(&path)?) {
        return Err("word corpus split checksum mismatch".to_owned());
    }
    read_gzip_lines(&path)?.iter().map(|line| ks_text::json::loads(line).map_err(|error| format!("a word row: {error}"))).collect()
}

/// Whether a folder holds a word corpus: its receipt says so.
#[must_use]
pub fn is_word_corpus(directory: &Path) -> bool {
    #[derive(Deserialize)]
    struct Kind {
        kind: Option<String>,
    }
    std::fs::read(directory.join(RECEIPT_FILE)).ok().and_then(|bytes| ks_text::json::loads_lenient::<Kind>(bytes).ok()).and_then(|found| found.kind).is_some_and(|kind| kind == WORD_CORPUS_KIND)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    fn config() -> WordConfig {
        WordConfig {
            schema_version: WORD_CORPUS_SCHEMAS[0],
            kind: WORD_CORPUS_KIND.to_owned(),
            directory: "model/prefix_v3".to_owned(),
            namespace: "test".to_owned(),
            minimum_word_length: 3,
            maximum_word_length: 16,
            frequent_words_per_language: 2,
            development_per_mille: 100,
            calibration_per_mille: 100,
            situations_per_word: 3,
            sign_leaders: "<\"'[{;:".to_owned(),
            english_keys_minimum_nats: 0.0,
            english_keys_maximum_characters: 6,
            minimum_calibration_negatives: 0,
            minimum_calibration_hard_negatives: 0,
            quotas: BTreeMap::new(),
            evidence_scope: String::new(),
            low_keys: None,
            acceptance_per_mille: None,
            minimum_words_per_direction: None,
            minimum_hard_words_per_direction: None,
        }
    }

    #[test]
    fn a_name_is_a_capital_and_small_letters_of_one_script() {
        assert_eq!(name_form("Kaplan"), Some(("Kaplan".to_owned(), 0)));
        assert_eq!(name_form("Тимчука"), Some(("Тимчука".to_owned(), 1)));
        assert_eq!(name_form("McDonald"), None);
        assert_eq!(name_form("Kаплан"), None);
        assert_eq!(name_form("kaplan"), Some(("Kaplan".to_owned(), 0)));
        assert_eq!(name_form(""), None);
        assert_eq!(name_form("1abc"), None);
    }

    #[test]
    fn an_address_is_cut_where_neither_layout_has_a_letter_and_each_part_keeps_what_came_before_it() {
        assert_eq!(address_parts("<kaplan@iepa.com>"), [("kaplan".to_owned(), "<".to_owned()), ("iepa.com".to_owned(), "<kaplan@".to_owned())]);
        assert_eq!(address_parts("http://www.google.com/search?aq=0"), [("http".to_owned(), String::new()), ("www.google.com".to_owned(), "http://".to_owned()), ("search".to_owned(), "http://www.google.com/".to_owned()), ("aq".to_owned(), "http://www.google.com/search?".to_owned())]);
        assert!(address_parts("@@").is_empty());
        assert!(is_address("spahnn@hnks.com") && is_address("www.igourmet.com") && is_address("Goldstar.com") && is_address("https://x"));
        assert!(!is_address("@home") && !is_address("plain") && !is_address("home@"));
    }

    #[test]
    fn the_english_margin_is_the_best_over_the_prefixes_it_weighs() {
        // Sums over three keys: -3 against -6; over four: -4 against -6.5.
        assert_eq!(english_margin(&[-1.0, -1.0, -1.0, -1.0], &[-2.0, -2.0, -2.0, -0.5], 6), Some(3.0));
        assert_eq!(english_margin(&[-1.0, -1.0], &[-2.0, -2.0], 6), None);
        assert_eq!(english_margin(&[-1.0, -1.0, -1.0, -9.0], &[-1.0, -1.0, -2.0, -1.0], 3), Some(1.0));
    }

    #[test]
    fn a_word_is_classed_by_its_reading_its_frequency_and_the_lexicon() {
        let frequent: BTreeSet<(String, i64)> = [("дом".to_owned(), 1)].into_iter().collect();
        assert_eq!(word_class("дом", 1, &frequent, true, true, true), Class::EnglishKeys);
        assert_eq!(word_class("дом", 1, &frequent, true, false, true), Class::LowKeys);
        assert_eq!(word_class("дом", 1, &frequent, true, false, false), Class::Frequent);
        assert_eq!(word_class("house", 0, &frequent, true, true, false), Class::Rare);
        assert_eq!(word_class("xyzzy", 0, &frequent, false, false, false), Class::Unknown);
        assert!(Class::LowKeys.hard(0) && Class::LowKeys.hard(1) && Class::LowKeys.paired());
        assert!(Class::Name.hard(0) && !Class::Name.hard(1) && Class::SignLed.hard(1) && Class::Term.hard(0) && Class::Address.hard(0) && !Class::Term.hard(1) && !Class::Rare.hard(0));
        assert!(Class::EnglishKeys.paired() && !Class::Address.paired());
        for class in [Class::Frequent, Class::Rare, Class::Name, Class::Unknown, Class::EnglishKeys, Class::LowKeys, Class::Term, Class::ShortTerm, Class::SignLed, Class::Address] {
            assert_eq!(ks_text::json::dumps(&class, &compact()), format!("\"{}\"", class.as_str()));
        }
    }

    #[test]
    fn a_word_takes_three_situations_and_an_address_part_its_own_sentence_and_two() {
        let word = Candidate { text: "kaplan".to_owned(), group: 0, class: Class::Name, source: "s".to_owned(), natural: None, hard: true };
        let situations = situations_of(&config(), &word);
        assert_eq!(situations.len(), 3);
        assert!(!situations.contains(&Situation::Natural));
        let part = Candidate { natural: Some("Write to <".to_owned()), class: Class::Address, ..word };
        let situations = situations_of(&config(), &part);
        assert_eq!((situations.len(), situations[0]), (3, Situation::Natural));
        assert_eq!([Situation::Empty, Situation::RussianText, Situation::Comment, Situation::Natural].map(Situation::role), ["unknown", "text", "code", "text"]);
        let pools = Pools { tatoeba: BTreeMap::new(), phrases: [vec!["One two three.".to_owned()], Vec::new()], treebank_words: BTreeMap::new(), names: BTreeMap::new(), terms: BTreeMap::new(), addresses: Vec::new() };
        assert_eq!(before_of(&config(), &pools, &part, Situation::Natural), "Write to <");
        assert_eq!(before_of(&config(), &pools, &part, Situation::Empty), "");
        assert_eq!(before_of(&config(), &pools, &part, Situation::RussianText), "");
        let english = before_of(&config(), &pools, &part, Situation::EnglishText);
        assert!(english == "One " || english == "One two ", "{english}");
    }

    #[test]
    fn a_split_holds_its_words_once_and_counts_the_hard_ones() {
        let row = |text: &str, group: i64, class: Class, situation: Situation| WordRow { before: String::new(), class, family: family(text, group), group, paired: class.paired(), role: situation.role().to_owned(), situation, source: "s".to_owned(), text: text.to_owned(), hard: false };
        let rows = [row("kaplan", 0, Class::Name, Situation::Empty), row("kaplan", 0, Class::Name, Situation::Comment), row("<kaplan", 0, Class::SignLed, Situation::Empty), row("дом", 1, Class::Frequent, Situation::Empty)];
        let counts = split_counts(&rows);
        assert_eq!((counts.rows, counts.words, counts.families, counts.convert_words, counts.keep_words, counts.hard_keep_words), (4, 3, 3, 2, 3, 2));
        assert_eq!(counts.words_by_class["ru:frequent"], 1);
        let shares = config();
        let splits: BTreeSet<&str> = (0..200).map(|index| split_of(&shares, &format!("f{index}"))).collect();
        assert_eq!(splits.len(), 3);
        assert_eq!(shares.splits(), WORD_SPLITS);
        let with_acceptance = WordConfig { acceptance_per_mille: Some(100), ..config() };
        let splits: BTreeSet<&str> = (0..200).map(|index| split_of(&with_acceptance, &format!("f{index}"))).collect();
        assert_eq!((splits.len(), with_acceptance.splits().last().copied()), (4, Some(ACCEPTANCE_SPLIT)));
        // A row hard by its first keys makes the split count each direction's cell.
        let hard = WordRow { hard: true, ..row("погода", 1, Class::LowKeys, Situation::Empty) };
        let counts = split_counts(&[hard, row("house", 0, Class::Rare, Situation::Empty), row("<house", 0, Class::SignLed, Situation::Empty)]);
        assert_eq!((counts.keep_words_by_group, counts.hard_keep_words_by_group, counts.hard_keep_words), (Some([2, 1]), Some([1, 1]), 2));
        assert_eq!(split_counts(&rows).keep_words_by_group, None);
    }

    #[test]
    fn a_word_is_hard_by_its_first_keys_when_its_own_channel_reads_them_among_the_worst() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let resources = Resources::new(root.join("resources"));
        let ortho = OrthoModel::load(&artifact_path(resources.resource_dir()), None, None, Some(resources.clone())).expect("the orthotactic model");
        let mean = |text: &str, group: i64| own_keys_mean(&ortho, text, group, 4).expect("keys");
        // A loanword and a name start worse for their own channel than a common word.
        assert!(mean("офсет", 1) < mean("погода", 1) && mean("Кыргыз", 1) < mean("погода", 1));
        assert!(mean("cntk", 0) < mean("house", 0));
        assert_eq!(own_keys_mean(&ortho, "", 0, 4), None);
        assert_eq!(low_keys_cut(&mut [3.0, -1.0, -5.0, 0.0], 250), Some(-1.0));
        assert_eq!(low_keys_cut(&mut [3.0, -1.0], 1000), Some(3.0));
        assert_eq!(low_keys_cut(&mut [], 250), None);
        let low = LowKeys { ortho: &ortho, rule: Some(LowKeysRule { characters: 4, per_mille: 200 }), cuts: [Some(mean("cntk", 0)), Some(mean("офсет", 1))] };
        assert!(low.is_low("cntk", 0) && !low.is_low("house", 0) && low.is_low("офсет", 1) && !low.is_low("погода", 1));
        assert!(!LowKeys { rule: None, ..low }.is_low("cntk", 0));
        assert!(!LowKeys { cuts: [None, None], ..low }.is_low("cntk", 0));
    }

    #[test]
    fn a_list_of_words_is_held_out_as_the_rows_of_a_sample_are() {
        let folder = std::env::temp_dir().join(format!("ks-train-held-out-{}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("a folder");
        let list = folder.join("words.txt");
        std::fs::write(&list, "гупшуп\n\n  тыгыдык \n").expect("a list");
        let held = held_out_of(&[list]).expect("the held-out words");
        assert_eq!((held.words, held.files[0].rows), (2, 2));
        assert!(held.aliases.len() >= 2);
        assert!(held_out_of(&[folder.join("missing.txt")]).is_err());
    }

    #[test]
    fn development_and_calibration_hold_their_words_in_all_and_in_each_direction() {
        let cells = SplitCounts { keep_words: 10, hard_keep_words: 4, keep_words_by_group: Some([6, 4]), hard_keep_words_by_group: Some([3, 1]), ..SplitCounts::default() };
        let smaller = SplitCounts { keep_words_by_group: Some([6, 3]), ..cells.clone() };
        let asking = |all: usize, words: Option<usize>, hard: Option<usize>| WordConfig { minimum_calibration_negatives: all, minimum_words_per_direction: words, minimum_hard_words_per_direction: hard, ..config() };
        assert!(cells_hold(&asking(10, Some(4), Some(1)), &cells, &cells).is_ok());
        assert!(cells_hold(&asking(11, None, None), &cells, &cells).unwrap_err().contains("10 negative words, 4 hard"));
        assert!(cells_hold(&asking(0, Some(5), None), &cells, &cells).unwrap_err().contains("development typed in layout group 1 holds 4 words, 1 hard: the config asks for 5 and 0"));
        assert!(cells_hold(&asking(0, None, Some(2)), &cells, &cells).unwrap_err().contains("layout group 1 holds 4 words, 1 hard"));
        assert!(cells_hold(&asking(0, Some(4), None), &cells, &smaller).unwrap_err().contains("calibration typed in layout group 1 holds 3 words"));
        // A split of schema two counts no direction: a config of schema two asks for none.
        assert!(cells_hold(&asking(0, None, None), &SplitCounts::default(), &SplitCounts::default()).is_ok());
    }
}
