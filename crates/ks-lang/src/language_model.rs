//! Offline lexical and character language models for short EN/RU tokens.
//!
//! The Python keeps the lexicon in a dict and the gram counts in `Counter`s of strings. Here a lexicon is a sorted
//! `WordTable` and the gram counts are sorted arrays of integers: a gram's characters, translated to one byte each
//! over the alphabet of the lexicon, read as one key. Lookups answer what the dicts answer. Nothing outside iterates
//! the gram counts, and what iterates the frequencies at run time only counts or looks up, so insertion order is not kept.
//!
//! `@lru_cache` of `score` is not carried over: it is a pure function, cheap here, and the cache held up to 65 536 scores.

use ks_cast::To as _;
use crate::constants::{LANGUAGE_MODEL_UNSEEN_GRAM_VOCABULARY, LANGUAGE_MODEL_GRAM_SMOOTHING, LANGUAGE_MODEL_NGRAM_ORDERS, LANGUAGE_MODEL_MIN_WORD_CHARACTERS, PACKED_GRAM_LAST_CODE, LANGUAGE_MODEL_MAX_GRAM_WEIGHT, LANGUAGE_MODEL_DEFAULT_NGRAM_MEAN, LANGUAGE_MODEL_MIN_NGRAM_DEVIATION, LANGUAGE_MODEL_CALIBRATION_WORD_LIMIT, LANGUAGE_MODEL_CALIBRATION_MIN_CHARACTERS, SYNTHETIC_FREQUENCY_FLOOR, SYNTHETIC_FREQUENCY_DIVISOR, ARPA_BIGRAM_SECTION, LANGUAGE_MODEL_TRIGRAM_ORDER, LANGUAGE_MODEL_EMPTY_TOKEN_SCORE, LANGUAGE_MODEL_DELETION_POSITIONS, LANGUAGE_MODEL_DELETION_MIN_CHARACTERS, LANGUAGE_MODEL_NATURALNESS_MIN, LANGUAGE_MODEL_NATURALNESS_MAX, LANGUAGE_MODEL_EXACT_WORD_BASE_SCORE, LANGUAGE_MODEL_POPULARITY_WEIGHT, LANGUAGE_MODEL_SPELL_KNOWN_SCORE, LANGUAGE_MODEL_KNOWN_WORD_NATURALNESS_FLOOR, LANGUAGE_MODEL_NATURALNESS_WEIGHT, LANGUAGE_MODEL_INVALID_RATIO_WEIGHT};
use crate::pysum::py_sum;
use crate::spellcheck::{split_path_list, DisabledSpellChecker, SpellChecker};
use crate::tables::{TableBuilder, WordList, WordTable};
use ks_text::{fmean, max_by_key_first, pstdev, py_int, py_round, pystr, ucd};
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The two roots after the packaged one, as in `MODEL_ROOTS`.
pub const SYSTEM_MODEL_ROOTS: [&str; 2] = ["/usr/share/onboard/models", "/usr/local/share/onboard/models"];

/// `model_roots()` from the value of `KEYSWITCH_MODEL_PATH`: the override, then `models` of the resources folder of
/// the install, then the system roots.
pub fn model_roots_from(model_path: Option<&str>, resource_dir: &Path) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = split_path_list(model_path.unwrap_or("")).map(PathBuf::from).collect();
    roots.push(resource_dir.join("models"));
    roots.extend(SYSTEM_MODEL_ROOTS.iter().map(PathBuf::from));
    roots
}

/// `model_roots()`.
#[must_use]
pub fn model_roots(resource_dir: &Path) -> Vec<PathBuf> {
    let value = std::env::var_os("KEYSWITCH_MODEL_PATH").map(|value| value.to_string_lossy().into_owned());
    model_roots_from(value.as_deref(), resource_dir)
}

/// `LOCALE_FALLBACKS.get(locale, ())`.
#[must_use]
pub fn locale_fallbacks(locale: &str) -> &'static [&'static str] {
    match locale {
        "en_US" => &[
            "hello", "world", "please", "thanks", "thank", "good", "morning", "evening", "today", "tomorrow", "keyboard", "layout", "switch",
            "application", "program", "settings", "language", "english", "test", "text", "word", "ubuntu", "computer", "message", "correct",
            "wrong", "github", "docker", "linux", "python", "terminal", "browser", "email",
        ],
        "ru_RU" => &[
            "привет", "пока", "спасибо", "пожалуйста", "хорошо", "здравствуйте", "сегодня", "завтра", "клавиатура", "раскладка", "переключить",
            "приложение", "программа", "настройки", "язык", "русский", "тест", "текст", "слово", "убунту", "компьютер", "сообщение", "исправить",
            "ошибка", "работает", "работа", "можно", "нужно", "когда", "будет", "очень", "этот", "эта", "это", "мой", "моя", "для", "как", "что",
        ],
        _ => &[],
    }
}

const EMERGENCY_SOURCE: &str = "встроенный аварийный словарь";

/// Evidence that a decoded token belongs to one language.
#[derive(Clone, Debug, PartialEq)]
pub struct WordScore {
    pub value: f64,
    pub known: bool,
    pub frequency: i64,
    pub gram_ratio: f64,
    pub exact: bool,
    pub spell_known: bool,
    pub ngram_score: f64,
    pub invalid_ratio: f64,
    pub raw_ngram_score: f64,
}

/// The multiplicative hash of the counting maps: the keys are integers, and millions of them are counted at start-up.
#[derive(Default)]
struct MixHasher(u64);

impl Hasher for MixHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    fn write_u32(&mut self, value: u32) {
        let mixed = u64::from(value).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        self.0 = mixed ^ (mixed >> 29);
    }
    fn write_u128(&mut self, value: u128) {
        let mixed = (value.to::<u64>() ^ (value >> 64).to::<u64>().rotate_left(23)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        self.0 = mixed ^ (mixed >> 29);
    }
}

/// A gram read as one integer. `u32` holds four one-byte codes of an alphabet of at most 255 characters; a lexicon
/// with more distinct letters falls back to raw code points, 21 bits each, in a `u128`.
trait GramKey: Copy + Eq + Hash + Ord {
    const ZERO: Self;
    fn pushed(self, code: u32) -> Self;
    fn into_keys(keys: Vec<Self>) -> GramKeys;
}

impl GramKey for u32 {
    const ZERO: Self = 0;
    fn pushed(self, code: u32) -> Self {
        (self << 8) | code
    }
    fn into_keys(keys: Vec<Self>) -> GramKeys {
        GramKeys::Narrow(keys)
    }
}

impl GramKey for u128 {
    const ZERO: Self = 0;
    fn pushed(self, code: u32) -> Self {
        (self << 21) | u128::from(code)
    }
    fn into_keys(keys: Vec<Self>) -> GramKeys {
        GramKeys::Wide(keys)
    }
}

enum GramKeys {
    Narrow(Vec<u32>),
    Wide(Vec<u128>),
}

/// One `Counter` of grams of an order: sorted keys and their counts.
struct GramCounter {
    keys: GramKeys,
    counts: Vec<u32>,
    total: u64,
    /// `total + alpha * vocabulary` of `raw_ngram_score`.
    denominator: f64,
}

impl GramCounter {
    fn new<K: GramKey>(counted: HashMap<K, u32, BuildHasherDefault<MixHasher>>) -> Self {
        let mut entries: Vec<(K, u32)> = counted.into_iter().collect();
        entries.sort_unstable();
        let total: u64 = entries.iter().map(|entry| u64::from(entry.1)).sum();
        let vocabulary = entries.len() + LANGUAGE_MODEL_UNSEEN_GRAM_VOCABULARY;
        GramCounter {
            keys: K::into_keys(entries.iter().map(|entry| entry.0).collect()),
            counts: entries.iter().map(|entry| entry.1).collect(),
            total,
            denominator: total.to::<f64>() + LANGUAGE_MODEL_GRAM_SMOOTHING * vocabulary.to::<f64>(),
        }
    }

    fn len(&self) -> usize {
        self.counts.len()
    }

    /// `counter.get(gram, 0)` of the gram with these codes.
    fn get(&self, codes: &[u32]) -> u32 {
        let found = match &self.keys {
            GramKeys::Narrow(keys) => {
                if codes.contains(&0) {
                    return 0;
                }
                keys.binary_search(&codes.iter().fold(0u32, |key, &code| key.pushed(code)))
            }
            GramKeys::Wide(keys) => keys.binary_search(&codes.iter().fold(0u128, |key, &code| key.pushed(code))),
        };
        found.map_or(0, |index| self.counts[index])
    }
}

/// `gram_counts`: the counters of the orders 2, 3 and 4 and the alphabet their narrow keys are written in.
struct GramCounts {
    /// Sorted; a character's code is its position plus one. `None` where the keys hold code points.
    alphabet: Option<Vec<char>>,
    counters: [GramCounter; 3],
}

fn order_slot(order: usize) -> usize {
    order - LANGUAGE_MODEL_NGRAM_ORDERS[0]
}

impl GramCounts {
    /// `_build_gram_counts`.
    fn build(frequencies: &WordTable) -> Self {
        let eligible = |word: &str| pystr::len(word) >= LANGUAGE_MODEL_MIN_WORD_CHARACTERS && pystr::isalpha(word);
        let mut seen = vec![0u64; 0x11_0000 / 64];
        let mut mark = |c: char| seen[c.to::<usize>() / 64] |= 1 << (c.to::<usize>() % 64);
        mark('^');
        mark('$');
        for (word, _frequency) in frequencies.iter() {
            if eligible(word) {
                word.chars().for_each(&mut mark);
            }
        }
        let alphabet: Vec<char> =
            (0..0x11_0000u32).filter(|&cp| seen[cp.to::<usize>() / 64] >> (cp % 64) & 1 == 1).filter_map(char::from_u32).collect();
        drop(seen);
        if alphabet.len() <= PACKED_GRAM_LAST_CODE { Self::counted::<u32>(frequencies, Some(alphabet)) } else { Self::counted::<u128>(frequencies, None) }
    }

    fn counted<K: GramKey>(frequencies: &WordTable, alphabet: Option<Vec<char>>) -> Self {
        let encode = |c: char| encode_with(alphabet.as_deref(), c);
        let mut maps: [HashMap<K, u32, BuildHasherDefault<MixHasher>>; 3] = Default::default();
        let mut codes: Vec<u32> = Vec::new();
        for (word, frequency) in frequencies.iter() {
            if pystr::len(word) < LANGUAGE_MODEL_MIN_WORD_CHARACTERS || !pystr::isalpha(word) {
                continue;
            }
            // The source counts are highly skewed. Logarithmic weighting keeps frequent words important without
            // erasing legitimate rare forms.
            let log = frequency.max(1).to::<f64>().log2();
            let weight = ((log.to::<i64>() + 1).clamp(1, LANGUAGE_MODEL_MAX_GRAM_WEIGHT)).to::<u32>();
            codes.clear();
            codes.push(encode('^'));
            codes.extend(word.chars().map(encode));
            codes.push(encode('$'));
            for (slot, &order) in LANGUAGE_MODEL_NGRAM_ORDERS.iter().enumerate() {
                for window in codes.windows(order) {
                    let key = window.iter().fold(K::ZERO, |key, &code| key.pushed(code));
                    let count = maps[slot].entry(key).or_insert(0);
                    *count = count.saturating_add(weight);
                }
            }
        }
        let [second, third, fourth] = maps;
        let counters = [GramCounter::new(second), GramCounter::new(third), GramCounter::new(fourth)];
        GramCounts { alphabet, counters }
    }

    fn padded_codes(&self, normalized: &str) -> Vec<u32> {
        let encode = |c: char| encode_with(self.alphabet.as_deref(), c);
        let mut codes = Vec::with_capacity(normalized.len() + 2);
        codes.push(encode('^'));
        codes.extend(normalized.chars().map(encode));
        codes.push(encode('$'));
        codes
    }
}

fn encode_with(alphabet: Option<&[char]>, c: char) -> u32 {
    match alphabet {
        Some(alphabet) => alphabet.binary_search(&c).map_or(0, |index| index.to::<u32>() + 1),
        None => c.to::<u32>(),
    }
}

/// The key of a bigram in its table: normalised words hold letters, `'` and `-` only.
fn bigram_key(left: &str, right: &str) -> String {
    let mut key = String::with_capacity(left.len() + right.len() + 1);
    key.push_str(left);
    key.push('\0');
    key.push_str(right);
    key
}

/// `dict[tuple[str, str], int]` as a table, for `LanguageModel::new`.
pub fn bigram_table<L: AsRef<str>, R: AsRef<str>, I: IntoIterator<Item = (L, R, i64)>>(pairs: I) -> WordTable {
    WordTable::from_pairs(pairs.into_iter().map(|(left, right, count)| (bigram_key(left.as_ref(), right.as_ref()), count)))
}

/// The lines a file read in text mode yields: `\n`, `\r\n` and a lone `\r` end a line, nothing else does.
fn universal_lines(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        if let Some(end) = rest.find(['\n', '\r']) {
            let line = rest.get(..end).unwrap_or_default();
            let skip = if rest.get(end..).unwrap_or_default().starts_with("\r\n") { 2 } else { 1 };
            rest = rest.get(end + skip..).unwrap_or_default();
            Some(line)
        } else {
            let line = rest;
            rest = "";
            Some(line)
        }
    })
}

/// A count as the tables hold it: the frozen files have small ones, and Python's are unbounded.
fn count_value(count: i128) -> i64 {
    i64::try_from(count).unwrap_or(if count < 0 { i64::MIN } else { i64::MAX })
}

/// What a language model is built from: the lexicon, its bigrams and where they came from.
pub struct LexiconTables {
    pub frequencies: WordTable,
    pub bigrams: WordTable,
    pub source: String,
}

/// Frequency lexicon + Hunspell morphology + smoothed character n-grams.
pub struct LanguageModel {
    pub locale: String,
    pub frequencies: Arc<WordTable>,
    /// Keys are `left\0right`; ask `bigram`.
    pub bigrams: WordTable,
    pub maximum: i64,
    pub maximum_bigram: i64,
    pub speller: Box<dyn SpellChecker>,
    pub source: String,
    gram_counts: GramCounts,
    pub ngram_mean: f64,
    pub ngram_deviation: f64,
}

impl LanguageModel {
    /// `LanguageModel(locale, frequencies, source, bigrams)`. `enable_spellcheck=False` is `DisabledSpellChecker`.
    pub fn new(locale: &str, frequencies: impl Into<Arc<WordTable>>, source: &str, bigrams: WordTable, speller: Box<dyn SpellChecker>) -> Self {
        let frequencies: Arc<WordTable> = frequencies.into();
        let maximum = frequencies.max_value().unwrap_or(1);
        let maximum_bigram = bigrams.max_value().unwrap_or(1);
        let source = if speller.available() { format!("{source}; Hunspell: {}", speller.source()) } else { source.to_string() };
        let mut model = LanguageModel {
            locale: locale.to_string(),
            maximum,
            maximum_bigram,
            speller,
            source,
            gram_counts: GramCounts::build(&frequencies),
            frequencies,
            bigrams,
            ngram_mean: LANGUAGE_MODEL_DEFAULT_NGRAM_MEAN,
            ngram_deviation: 1.0,
        };
        let calibration = model.calibration();
        if !calibration.is_empty() {
            model.ngram_mean = fmean(&calibration).unwrap_or(LANGUAGE_MODEL_DEFAULT_NGRAM_MEAN);
        }
        if calibration.len() > 1 {
            model.ngram_deviation = pstdev(&calibration).unwrap_or(1.0);
        }
        if model.ngram_deviation < LANGUAGE_MODEL_MIN_NGRAM_DEVIATION {
            model.ngram_deviation = 1.0;
        }
        model
    }

    /// The raw scores of the most frequent words, most frequent first and then by word.
    fn calibration(&self) -> Vec<f64> {
        let mut order: Vec<u32> = (0..self.frequencies.len().to::<u32>()).collect();
        // The table is sorted by word and the sort is stable: (-frequency, word).
        order.sort_by_key(|&index| std::cmp::Reverse(self.frequencies.value_at(index.to::<usize>())));
        order.truncate(LANGUAGE_MODEL_CALIBRATION_WORD_LIMIT);
        order
            .iter()
            .map(|&index| self.frequencies.words().get(index.to::<usize>()))
            .filter(|word| pystr::len(word) >= LANGUAGE_MODEL_CALIBRATION_MIN_CHARACTERS && pystr::isalpha(word))
            .map(|word| self.raw_ngram_score(word))
            .collect()
    }

    /// `LanguageModel.load(locale, extra_words)` over given roots, without the Python's cache (see `Resources`).
    pub fn load<S: AsRef<str>, I: IntoIterator<Item = S>>(locale: &str, extra_words: I, roots: &[PathBuf], speller: Box<dyn SpellChecker>) -> Self {
        let tables = Self::load_tables(locale, extra_words, roots);
        LanguageModel::new(locale, tables.frequencies, &tables.source, tables.bigrams, speller)
    }

    /// What `_load_cached` reads and adds, before the model is built on it: a prefix index needs no more than this.
    pub fn load_tables<S: AsRef<str>, I: IntoIterator<Item = S>>(locale: &str, extra_words: I, roots: &[PathBuf]) -> LexiconTables {
        let mut normalized_extra = WordList::new();
        for word in extra_words {
            let normalized = Self::normalize(word.as_ref());
            if !normalized.is_empty() {
                normalized_extra.push(&normalized);
            }
        }
        let normalized_extra = normalized_extra.sorted_unique();
        let path = roots.iter().map(|root| root.join(format!("{locale}.lm"))).find(|path| path.is_file());
        let (frequencies, bigrams, source) = match path {
            Some(path) => {
                let (frequencies, bigrams) = Self::_read_arpa(&path);
                (frequencies, bigrams, path.display().to_string())
            }
            None => (WordTable::default(), WordTable::default(), EMERGENCY_SOURCE.to_string()),
        };
        let synthetic_frequency =
            frequencies.max_value().unwrap_or(SYNTHETIC_FREQUENCY_FLOOR).div_euclid(SYNTHETIC_FREQUENCY_DIVISOR).max(SYNTHETIC_FREQUENCY_FLOOR);
        // The Python normalises the extra words once more here; casefolding twice is not always the same as once.
        let mut additions = WordList::with_capacity(normalized_extra.len(), normalized_extra.arena_bytes());
        for word in locale_fallbacks(locale).iter().copied().chain(normalized_extra.iter()) {
            let normalized = Self::normalize(word);
            if !normalized.is_empty() {
                additions.push(&normalized);
            }
        }
        drop(normalized_extra);
        let additions = additions.sorted_unique();
        LexiconTables { frequencies: frequencies.with_floor(&additions, synthetic_frequency), bigrams, source }
    }

    /// The same model without Hunspell: `enable_spellcheck=False`.
    #[must_use]
    pub fn disabled_speller() -> Box<dyn SpellChecker> {
        Box::new(DisabledSpellChecker)
    }

    #[must_use]
    pub fn normalize(word: &str) -> String {
        pystr::casefold(word).chars().filter(|&c| ucd::isalpha(c) || c == '\'' || c == '-').collect()
    }

    /// `_read_arpa`: the unigrams and bigrams of an ARPA file; both empty when it cannot be read.
    pub fn _read_arpa(path: &Path) -> (WordTable, WordTable) {
        let Ok(content) = std::fs::read(path) else {
            return (WordTable::default(), WordTable::default());
        };
        let text = String::from_utf8_lossy(&content);
        let (mut unigrams, mut bigrams) = (TableBuilder::new(), TableBuilder::new());
        let mut section = 0;
        for line in universal_lines(&text) {
            if line == r"\1-grams:" {
                section = 1;
                continue;
            }
            if line == r"\2-grams:" {
                section = ARPA_BIGRAM_SECTION;
                continue;
            }
            if line.starts_with('\\') {
                section = 0;
                continue;
            }
            if section == 0 || line.is_empty() {
                continue;
            }
            let Some((count_text, payload)) = line.split_once(' ') else {
                continue;
            };
            if payload.starts_with('<') {
                continue;
            }
            let Some(count) = py_int(count_text).map(count_value) else {
                continue;
            };
            if section == 1 {
                let token = Self::normalize(payload);
                if pystr::len(&token) >= LANGUAGE_MODEL_MIN_WORD_CHARACTERS {
                    unigrams.push(&token, count);
                }
            } else {
                // Section zero was skipped above and section one was handled by the preceding branch, so only
                // bigrams remain here.
                let Some((left, right)) = payload.split_once(' ') else {
                    continue;
                };
                let (left, right) = (Self::normalize(left), Self::normalize(right));
                if !left.is_empty() && !right.is_empty() {
                    bigrams.push(&bigram_key(&left, &right), count);
                }
            }
        }
        (unigrams.finish(i64::saturating_add), bigrams.finish(i64::saturating_add))
    }

    /// `self.bigrams.get((left, right), 0)`.
    #[must_use]
    pub fn bigram(&self, left: &str, right: &str) -> i64 {
        self.bigrams.get_or_zero(&bigram_key(left, right))
    }

    /// `len(gram_counts[order])`, `_gram_totals[order]`.
    #[must_use]
    pub fn gram_size(&self, order: usize) -> (usize, u64) {
        let counter = &self.gram_counts.counters[order_slot(order)];
        (counter.len(), counter.total)
    }

    /// `gram_counts[order].get(gram, 0)`; the gram must have `order` characters.
    #[must_use]
    pub fn gram_count(&self, order: usize, gram: &str) -> u32 {
        let codes: Vec<u32> = gram.chars().map(|c| encode_with(self.gram_counts.alphabet.as_deref(), c)).collect();
        self.gram_counts.counters[order_slot(order)].get(&codes)
    }

    /// `gram in self._grams`.
    #[must_use]
    pub fn has_gram(&self, gram: &str) -> bool {
        pystr::len(gram) == LANGUAGE_MODEL_TRIGRAM_ORDER && self.gram_count(LANGUAGE_MODEL_TRIGRAM_ORDER, gram) != 0
    }

    #[must_use]
    pub fn raw_ngram_score(&self, word: &str) -> f64 {
        let normalized = Self::normalize(word);
        if normalized.is_empty() {
            return LANGUAGE_MODEL_EMPTY_TOKEN_SCORE;
        }
        let codes = self.gram_counts.padded_codes(&normalized);
        let mut order_scores = [0.0; 3];
        let mut scored = 0;
        for (slot, &order) in LANGUAGE_MODEL_NGRAM_ORDERS.iter().enumerate() {
            // A token too short for an order has no grams of it, and the order is left out.
            if codes.len() < order {
                continue;
            }
            let counter = &self.gram_counts.counters[slot];
            let grams = codes.windows(order);
            let count = grams.len();
            order_scores[scored] = py_sum(grams.map(|gram| ((f64::from(counter.get(gram)) + LANGUAGE_MODEL_GRAM_SMOOTHING) / counter.denominator).ln())) / count.to::<f64>();
            scored += 1;
        }
        fmean(&order_scores[..scored]).unwrap_or(LANGUAGE_MODEL_EMPTY_TOKEN_SCORE)
    }

    #[must_use]
    pub fn ngram_score(&self, word: &str) -> f64 {
        (self.raw_ngram_score(word) - self.ngram_mean) / self.ngram_deviation
    }

    #[must_use]
    pub fn context_score(&self, previous_word: &str, word: &str) -> f64 {
        let frequency = self.bigram(&Self::normalize(previous_word), &Self::normalize(word));
        if frequency == 0 {
            return 0.0;
        }
        frequency.to::<f64>().ln_1p() / self.maximum_bigram.to::<f64>().ln_1p()
    }

    /// Return the strongest score after dropping one likely typo character (`limit` at its default: no caller changes it).
    #[must_use]
    pub fn best_single_deletion(&self, word: &str) -> WordScore {
        let chars: Vec<char> = word.chars().collect();
        let (length, limit) = (chars.len(), LANGUAGE_MODEL_DELETION_POSITIONS);
        if length < LANGUAGE_MODEL_DELETION_MIN_CHARACTERS {
            return self.score("");
        }
        let mut indices: Vec<usize> = (0..length).collect();
        if length > limit {
            indices = (0..limit).map(|index| (py_round((index * (length - 1)).to::<f64>() / (limit - 1).to::<f64>())).to::<usize>()).collect();
            indices.dedup();
        }
        let scores = indices.into_iter().map(|index| {
            let shorter: String = chars.iter().enumerate().filter(|(position, _)| *position != index).map(|(_, &c)| c).collect();
            self.score(&shorter)
        });
        max_by_key_first(scores, |item| item.value).unwrap_or_else(|| self.score(""))
    }

    /// Score a token.
    #[must_use]
    pub fn score(&self, word: &str) -> WordScore {
        let normalized = Self::normalize(word);
        if normalized.is_empty() {
            return WordScore {
                value: LANGUAGE_MODEL_EMPTY_TOKEN_SCORE,
                known: false,
                frequency: 0,
                gram_ratio: 0.0,
                exact: false,
                spell_known: false,
                ngram_score: LANGUAGE_MODEL_NATURALNESS_MIN,
                invalid_ratio: 1.0,
                raw_ngram_score: LANGUAGE_MODEL_NATURALNESS_MIN,
            };
        }
        let lexical_eligible = word.chars().all(|c| ucd::isalpha(c) || c == '\'' || c == '-');
        let frequency = if lexical_eligible { self.frequencies.get_or_zero(&normalized) } else { 0 };
        let exact = frequency != 0;
        let spell_known = lexical_eligible && pystr::isalpha(&normalized) && self.speller.check(&normalized);
        let known = exact || spell_known;
        let codes = self.gram_counts.padded_codes(&normalized);
        let counter = &self.gram_counts.counters[order_slot(LANGUAGE_MODEL_TRIGRAM_ORDER)];
        let trigrams = codes.windows(LANGUAGE_MODEL_TRIGRAM_ORDER);
        let count = trigrams.len();
        let hits = trigrams.filter(|gram| counter.get(gram) != 0).count();
        let ratio = hits.to::<f64>() / count.to::<f64>();
        let invalid_ratio = 1.0 - ratio;
        let score = self.ngram_score(&normalized);
        let limited = if score < LANGUAGE_MODEL_NATURALNESS_MAX { score } else { LANGUAGE_MODEL_NATURALNESS_MAX };
        let raw_naturalness = if limited > LANGUAGE_MODEL_NATURALNESS_MIN { limited } else { LANGUAGE_MODEL_NATURALNESS_MIN };
        let mut naturalness = raw_naturalness;
        let mut lexical = 0.0;
        if exact {
            let popularity = frequency.to::<f64>().ln_1p() / self.maximum.to::<f64>().ln_1p();
            lexical = LANGUAGE_MODEL_EXACT_WORD_BASE_SCORE + LANGUAGE_MODEL_POPULARITY_WEIGHT * popularity;
        } else if spell_known {
            lexical = LANGUAGE_MODEL_SPELL_KNOWN_SCORE;
        }
        if known && LANGUAGE_MODEL_KNOWN_WORD_NATURALNESS_FLOOR > naturalness {
            naturalness = LANGUAGE_MODEL_KNOWN_WORD_NATURALNESS_FLOOR;
        }
        let value = lexical + LANGUAGE_MODEL_NATURALNESS_WEIGHT * naturalness - LANGUAGE_MODEL_INVALID_RATIO_WEIGHT * invalid_ratio;
        WordScore { value, known, frequency, gram_ratio: ratio, exact, spell_known, ngram_score: naturalness, invalid_ratio, raw_ngram_score: raw_naturalness }
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::spellcheck::FnSpellChecker;

    #[test]
    fn a_table_with_a_very_large_alphabet_and_the_hashes_of_its_keys() {
        let words: Vec<(String, i64)> = (0x4e00..0x4e00 + 300_u32).map(|code| (char::from_u32(code).unwrap().to_string(), 5)).collect();
        let frequencies = WordTable::from_pairs(words.iter().map(|(word, count)| (word.as_str(), *count)));
        let model = LanguageModel::new("en_US", frequencies, "test", WordTable::default(), LanguageModel::disabled_speller());
        assert!(model.raw_ngram_score("\u{4e00}") > f64::NEG_INFINITY);
        let mut hasher = MixHasher(0);
        hasher.write(b"ab");
        let first = hasher.finish();
        hasher.write_u128(1);
        assert_ne!(first, hasher.finish());
    }

    #[test]
    fn counts_outside_the_range_and_models_with_nothing_to_measure() {
        assert_eq!((count_value(-1), count_value(i128::MIN), count_value(i128::MAX)), (-1, i64::MIN, i64::MAX));
        let empty = LanguageModel::new("en_US", WordTable::default(), "test", WordTable::default(), LanguageModel::disabled_speller());
        assert_eq!(empty.ngram_deviation.to_bits(), 1.0_f64.to_bits());
        let one = LanguageModel::new("en_US", WordTable::from_pairs([("a", 5)]), "test", WordTable::default(), LanguageModel::disabled_speller());
        assert_eq!(one.ngram_deviation.to_bits(), 1.0_f64.to_bits());
        let equal = LanguageModel::new("en_US", WordTable::from_pairs([("a", 5), ("b", 5)]), "test", WordTable::default(), LanguageModel::disabled_speller());
        assert_eq!(equal.ngram_deviation.to_bits(), 1.0_f64.to_bits());
        assert!(!tiny().has_gram("ab"));
    }

    #[test]
    fn an_arpa_file_that_is_gone_or_names_no_word() {
        let (unigrams, bigrams) = LanguageModel::_read_arpa(Path::new("/nonexistent.arpa"));
        assert!(unigrams.is_empty() && bigrams.is_empty());
        let file = std::env::temp_dir().join(format!("ks-arpa-{}.arpa", std::process::id()));
        std::fs::write(&file, "\\\\data\\\\\n\\\\1-grams:\n-1 a\n\\\\2-grams:\n-1 1 b\n-1 a 2\n-1 a b\n").unwrap();
        let (_, bigrams) = LanguageModel::_read_arpa(&file);
        assert!(bigrams.len() <= 1);
        std::fs::remove_file(file).unwrap();
        let tables = LanguageModel::load_tables("en_US", ["123"], &[]);
        assert!(!tables.frequencies.is_empty());
    }

    fn tiny() -> LanguageModel {
        let frequencies = WordTable::from_pairs([("hello", 1000), ("help", 500), ("world", 2000), ("it's", 7), ("a", 9)]);
        let bigrams = bigram_table([("hello", "world", 9), ("help", "me", 3)]);
        LanguageModel::new("en_US", frequencies, "test", bigrams, LanguageModel::disabled_speller())
    }

    #[test]
    fn normalizes_like_python() {
        assert_eq!(LanguageModel::normalize("Straße-1'x"), "strasse-'x");
        assert_eq!(LanguageModel::normalize("123"), "");
        assert_eq!(LanguageModel::normalize("ПРИВЕТ"), "привет");
    }

    #[test]
    fn universal_newlines() {
        let lines: Vec<&str> = universal_lines("a\r\nb\rc\n\nd").collect();
        assert_eq!(lines, ["a", "b", "c", "", "d"]);
        assert_eq!(universal_lines("").count(), 0);
        assert_eq!(universal_lines("a\u{2028}b\x0c").count(), 1);
    }

    #[test]
    #[expect(clippy::float_cmp, reason = "the exact double is the meaning here, as it is in the Python this was checked against")]
    fn scores_a_small_lexicon() {
        let model = tiny();
        assert_eq!(model.maximum, 2000);
        assert_eq!(model.source, "test");
        let empty = model.score("123");
        assert_eq!((empty.value, empty.known, empty.ngram_score, empty.invalid_ratio), (-30.0, false, -15.0, 1.0));
        let hello = model.score("Hello");
        assert!(hello.exact && hello.known && hello.frequency == 1000 && hello.gram_ratio == 1.0);
        let symbol = model.score("hello1");
        assert_eq!(symbol.frequency, 0);
        assert!(model.has_gram("^he") && !model.has_gram("xyz"));
        assert_eq!(model.gram_count(2, "he"), 10 + 9);
        assert!(model.context_score("Hello", "WORLD") > 0.99);
        assert_eq!(model.context_score("world", "hello"), 0.0);
        assert_eq!(model.best_single_deletion("hi").value, -30.0);
        assert_eq!(model.best_single_deletion("hellno").frequency, 1000);
        assert_eq!(model.score("x").raw_ngram_score, model.ngram_score("x").clamp(-15.0, 4.0));
    }

    #[test]
    fn the_speller_counts_for_alphabetic_forms_only() {
        let frequencies = WordTable::from_pairs([("hello", 1000), ("world", 2000), ("again", 300)]);
        let speller = FnSpellChecker::new("/d.dic", |word: &str| word == "hellos" || word == "a-b");
        let model = LanguageModel::new("en_US", frequencies, "test", WordTable::default(), Box::new(speller));
        assert_eq!(model.source, "test; Hunspell: /d.dic");
        let known = model.score("HELLOS");
        assert!(known.spell_known && known.known && !known.exact);
        assert!(!model.score("a-b").spell_known);
    }

    #[test]
    fn no_file_leaves_the_fallback_words() {
        let tables = LanguageModel::load_tables("ru_RU", ["Абв", "12"], &[PathBuf::from("/nonexistent-ks-lang")]);
        assert_eq!(tables.source, EMERGENCY_SOURCE);
        assert_eq!(tables.frequencies.get("абв"), Some(SYNTHETIC_FREQUENCY_FLOOR));
        assert_eq!(tables.frequencies.get("привет"), Some(SYNTHETIC_FREQUENCY_FLOOR));
        assert_eq!(tables.frequencies.len(), locale_fallbacks("ru_RU").len() + 1);
        assert!(LanguageModel::load_tables("de_DE", [""; 0], &[]).frequencies.is_empty());
    }

    #[test]
    fn reads_an_arpa_file() {
        let path = std::env::temp_dir().join(format!("ks-lang-arpa-{}.lm", std::process::id()));
        std::fs::write(&path, "\\data\\\r\nngram 1=3\n\n\\1-grams:\n5 AB\n3 ab\n1 <s>\nx yz\n٣ cd\n7 e\n9\n\\2-grams:\n2 AB cd\n1 <s> ab\n4 ab\n\\end\\\n9 zz\n".as_bytes()).unwrap();
        let (unigrams, bigrams) = LanguageModel::_read_arpa(&path);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(unigrams.iter().collect::<Vec<_>>(), [("ab", 8), ("cd", 3)]);
        assert_eq!(bigrams.get(&bigram_key("ab", "cd")), Some(2));
        assert_eq!(bigrams.len(), 1);
    }

    #[test]
    fn roots_put_the_override_first() {
        let roots = model_roots_from(Some(if cfg!(windows) { "a;;b" } else { "a::b" }), Path::new("res"));
        assert_eq!(roots[..3], [PathBuf::from("a"), PathBuf::from("b"), Path::new("res").join("models")]);
        assert_eq!(roots.len(), 5);
    }
}
