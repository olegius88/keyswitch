//! Conservative runtime exceptions for frequent two-letter function words.
//!
//! This policy intentionally lives outside the frozen intent-model detector. It does not change the certified
//! classifier toolchain and only admits reviewed, exact lexicon hits with a large corpus-frequency advantage.

use crate::constants::*;
use crate::detector::{DetectionDecision, DetectorError, GroupTexts, LanguageDetector};
use crate::language_model::LanguageModel;
use crate::tables::WordTable;
use ks_text::{max_by_key_first, pystr};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub use crate::constants::{NATURAL_SOURCE_MAX_LENGTH, SINGLE_LETTER_CONFIDENCE, TRUSTED_SHORT_WORD_MAX_LENGTH, TRUSTED_SHORT_WORD_MINIMUM_FREQUENCY, TRUSTED_SHORT_WORD_MINIMUM_RATIO, TRUSTED_SHORT_WORD_CONTEXT_RATIO};

// Words a person types in the wrong layout every day and has to fix by hand. The list is curated, not generated:
// every entry is a frequent function word whose other-layout rendering is not a word anyone types on purpose, and
// the frequency and ratio gates below still have to agree.
pub const TRUSTED_SINGLE_LETTER_WORDS: [&str; 8] = ["а", "и", "с", "в", "к", "у", "о", "я"];

/// `TRUSTED_SHORT_WORDS[0]`: only the reported, manually reviewed collision. Broad lists of two-letter words would
/// make legitimate abbreviations such as RU `шт` candidates for EN `in` and would defeat the precision-first policy.
const TRUSTED_SHORT_WORDS_EN: [&str; 1] = ["if"];
const TRUSTED_TWO_LETTER_WORDS_RU: [&str; 28] = [
    "не", "ли", "на", "по", "то", "мы", "вы", "ты", "он", "но", "же", "за", "до", "из", "от", "об", "ну", "их", "им", "да", "ни", "во", "ко", "со", "бы",
    "уж", "ей", "её",
];

/// `TRUSTED_SHORT_WORDS.get(group, frozenset())`.
pub fn trusted_short_words(group: i64) -> Vec<&'static str> {
    match group {
        0 => TRUSTED_SHORT_WORDS_EN.to_vec(),
        1 => TRUSTED_TWO_LETTER_WORDS_RU.iter().chain(TRUSTED_SINGLE_LETTER_WORDS.iter()).copied().collect(),
        _ => Vec::new(),
    }
}

// Short Russian words that open sentences, a fact the frequency lexicons do not carry: they are encyclopedic, and
// `ты` stands there at 5 599 next to the English token `ns` at 2 430. In the train parts of UD Taiga (119 490
// sentences) 14% of sentences open with a curated letter and 10.1% with a curated two-letter word (но 3 149, он 1 708,
// на 1 296, не 1 028, мы 818, по 590, да 421, ты 372, ну 363), while the Latin readings of these words open at most one
// English sentence each in UD EWT (12 544 sentences; ns and vs none); `же` and `бы` open none, so they are not listed,
// and `ли` opens one, as its reading `kb` does in English, so it is taught with the rest. The context model reads it
// as evidence (feature schema 6); it decides nothing on its own.
const OPENING_WORDS_RU: [&str; 26] = [
    "не", "ли", "на", "по", "то", "мы", "вы", "ты", "он", "но", "за", "до", "из", "от", "об", "ну", "их", "им", "да", "ни", "во", "ко", "со", "уж", "ей",
    "её",
];

/// `OPENING_WORDS.get(group, frozenset())` holds `text`.
fn opening_word(text: &str, group: i64) -> bool {
    group == 1 && (OPENING_WORDS_RU.contains(&text) || TRUSTED_SINGLE_LETTER_WORDS.contains(&text))
}

/// Whether this reading is a word that opens sentences of its language.
pub fn opens_sentences(text: &str, group: i64) -> bool {
    opening_word(&LanguageModel::normalize(text), group)
}

// A short token found only in the other language's frequency list is thin evidence: "дев" reads as ordinary Russian
// (n-gram z-score -0.7) yet "ltd" is a frequent English token, and the intent model never sees tokens this short. Up to
// NATURAL_SOURCE_MAX_LENGTH the source must also read unnaturally - the bar the detector's unknown-word branch already
// applies, HEURISTIC_UNLIKELY_SOURCE_NGRAM_MAX - unless the recent context favours the target language. This lives
// here, not in the detector, because the detector is part of the certified model toolchain.
pub const DICTIONARY_ONLY_REASONS: [&str; 2] = ["слово найдено только в целевом частотном словаре", "словоформа подтверждена морфологическим словарём"];
pub const NATURAL_SOURCE_REASON: &str = "короткое слово читается естественно в исходном языке";

/// Withhold a dictionary-only conversion of a short, natural-looking word.
pub fn natural_short_source_veto(decision: DetectionDecision, context_group: Option<i64>) -> DetectionDecision {
    if !decision.should_convert || !DICTIONARY_ONLY_REASONS.contains(&decision.reason.as_str()) {
        return decision;
    }
    let length = pystr::len(&LanguageModel::normalize(&decision.original)).max(pystr::len(&LanguageModel::normalize(&decision.replacement)));
    if length > NATURAL_SOURCE_MAX_LENGTH
        || decision.source_score.ngram_score <= HEURISTIC_UNLIKELY_SOURCE_NGRAM_MAX
        || context_group == Some(decision.target_group)
    {
        return decision;
    }
    DetectionDecision { should_convert: false, reason: NATURAL_SOURCE_REASON.to_string(), ..decision }
}

// With the previous word already in the target language, a two-letter entry only has to be at least as frequent as the
// token it replaces (TRUSTED_SHORT_WORD_CONTEXT_RATIO): `kb` and `nj` are real English tokens, but after a Russian word
// they are `ли` and `то`. The frequency lists hold no single letters at all, so a one-letter entry relies on the
// curated list and that context alone.
pub const CONTEXT_SHORT_WORD_REASON: &str = "короткое слово из безопасного списка после слова на целевом языке";
/// The reason of the message-start exception applied in `word_decision`; it is listed here so the context policies
/// treat it as a curated short-word rule.
pub const ISOLATED_SHORT_WORD_REASON: &str = "однобуквенное слово из безопасного списка в начале сообщения";
pub const FREQUENT_SHORT_WORD_REASON: &str = "частотное короткое слово из безопасного списка";

pub const SHORT_WORD_REASONS: [&str; 3] = [FREQUENT_SHORT_WORD_REASON, CONTEXT_SHORT_WORD_REASON, ISOLATED_SHORT_WORD_REASON];

/// `Counter` of the first `PAUSE_WORD_START_LETTERS` letters of the words longer than that.
#[derive(Debug, Default)]
pub struct WordStarts {
    counts: HashMap<String, u32>,
}

impl WordStarts {
    /// `counter[start]`: zero for a start nobody has.
    pub fn count(&self, start: &str) -> u32 {
        self.counts.get(start).copied().unwrap_or(0)
    }

    pub fn len(&self) -> usize {
        self.counts.len()
    }

    /// The counted starts and their counts, sorted by start.
    pub fn items(&self) -> Vec<(&str, u32)> {
        let mut items: Vec<(&str, u32)> = self.counts.iter().map(|item| (item.0.as_str(), *item.1)).collect();
        items.sort_unstable();
        items
    }

    pub fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }
}

/// The tables built, most recently used last: `lru_cache(maxsize=LAYOUT_GROUP_COUNT)` keyed by the model. The key is
/// the lexicon (the entry holds it, so its address stays this lexicon's).
type WordStartsCache = Vec<(Arc<WordTable>, Arc<WordStarts>)>;
static WORD_STARTS: Mutex<WordStartsCache> = Mutex::new(Vec::new());

/// How many words of a lexicon begin with each PAUSE_WORD_START_LETTERS letters, counted once per model.
pub fn word_starts(model: &LanguageModel) -> Arc<WordStarts> {
    let mut cache = WORD_STARTS.lock().expect("the lock is not poisoned");
    if let Some(at) = cache.iter().position(|entry| Arc::ptr_eq(&entry.0, &model.frequencies)) {
        let entry = cache.remove(at);
        let starts = entry.1.clone();
        cache.push(entry);
        return starts;
    }
    let mut counts: HashMap<String, u32> = HashMap::new();
    for (word, _) in model.frequencies.iter() {
        if pystr::len(word) > PAUSE_WORD_START_LETTERS {
            *counts.entry(pystr::slice(word, None, Some(PAUSE_WORD_START_LETTERS as i64)).to_string()).or_insert(0) += 1;
        }
    }
    let starts = Arc::new(WordStarts { counts });
    if cache.len() >= LAYOUT_GROUP_COUNT {
        cache.remove(0);
    }
    cache.push((model.frequencies.clone(), starts.clone()));
    starts
}

/// Whether PAUSE_WORD_START_LETTERS letters begin at least PAUSE_WORD_START_MIN_WORDS words of the lexicon: at a pause
/// in typing they are as likely the start of a word still being typed as a word. A lone letter is not asked: typed and
/// left at a pause it was meant converted ten times against twice in the owner's typing (field batteries,
/// 30.10.2026), and the next key is often a click, after which no boundary decides it.
pub fn begins_words(model: &LanguageModel, token: &str) -> bool {
    pystr::isalpha(token) && pystr::len(token) == PAUSE_WORD_START_LETTERS && word_starts(model).count(&pystr::casefold(token)) >= PAUSE_WORD_START_MIN_WORDS
}

/// Whether the decision came from the trusted short-word list.
pub fn is_short_word_override(decision: &DetectionDecision) -> bool {
    decision.should_convert && SHORT_WORD_REASONS.contains(&decision.reason.as_str())
}

/// Return a high-precision short-word override, or leave policy unchanged.
///
/// A single letter is a word only in a sentence: `f` alone may be a variable name, so one-letter entries need the
/// previous word to be in the target language (`context_group`); two-letter entries stand on their own. The
/// message-start exception for a lone letter lives in `word_decision::automatic_word_decision`, outside this table, so
/// the frozen context-v1 training input stays what it was.
#[allow(clippy::too_many_arguments)]
pub fn trusted_short_word_decision(
    detector: &LanguageDetector,
    original: &str,
    alternatives: GroupTexts<'_>,
    source_group: i64,
    ignored_words: &[String],
    rejected_targets: &[i64],
    protect_code: bool,
    context_group: Option<i64>,
) -> Result<Option<DetectionDecision>, DetectorError> {
    let original_key = LanguageDetector::token_key(original);
    if ignored_words.iter().any(|word| LanguageDetector::token_key(word) == original_key) {
        return Ok(None);
    }
    if protect_code && detector.is_protected_token(original) {
        return Ok(None);
    }

    let source_model = detector.model_for(source_group)?;
    let source_score = source_model.score(original);
    let normalized_original = LanguageModel::normalize(original);
    let mut candidates: Vec<DetectionDecision> = Vec::new();
    for (target_group, replacement) in alternatives {
        let target_group = *target_group;
        let Some(target_model) = detector.model(target_group) else {
            continue;
        };
        if target_group == source_group || replacement == original || rejected_targets.contains(&target_group) {
            continue;
        }
        let normalized_replacement = LanguageModel::normalize(replacement);
        if pystr::len(&normalized_original).max(pystr::len(&normalized_replacement)) > TRUSTED_SHORT_WORD_MAX_LENGTH
            || !trusted_short_words(target_group).contains(&normalized_replacement.as_str())
        {
            continue;
        }
        let supported = context_group == Some(target_group);
        let target_score = target_model.score(replacement);
        if TRUSTED_SINGLE_LETTER_WORDS.contains(&normalized_replacement.as_str()) {
            if !supported {
                continue;
            }
            candidates.push(DetectionDecision::new(
                true,
                original,
                replacement,
                source_group,
                target_group,
                SINGLE_LETTER_CONFIDENCE,
                CONTEXT_SHORT_WORD_REASON,
                source_score.clone(),
                target_score,
            ));
            continue;
        }
        if !target_score.exact || target_score.frequency < TRUSTED_SHORT_WORD_MINIMUM_FREQUENCY {
            continue;
        }
        let ratio = (target_score.frequency + 1) as f64 / (source_score.frequency + 1) as f64;
        if ratio < TRUSTED_SHORT_WORD_MINIMUM_RATIO && !(supported && ratio >= TRUSTED_SHORT_WORD_CONTEXT_RATIO) {
            continue;
        }
        candidates.push(DetectionDecision::new(
            true,
            original,
            replacement,
            source_group,
            target_group,
            ratio.log10(),
            if ratio >= TRUSTED_SHORT_WORD_MINIMUM_RATIO { FREQUENT_SHORT_WORD_REASON } else { CONTEXT_SHORT_WORD_REASON },
            source_score.clone(),
            target_score,
        ));
    }
    Ok(max_by_key_first(candidates, |item| item.confidence))
}
