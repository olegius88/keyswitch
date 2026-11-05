//! Early layout switching from an unambiguous word prefix.
//!
//! Ordinary correction waits for a word boundary or a typing pause. Many wrong layout words are, however, obvious
//! after a few letters: no English word starts with `ghbd` while hundreds of Russian words start with `прив`. This
//! module decides whether a typed prefix already proves the wrong layout so the engine can switch and rewrite the
//! prefix before the word is finished.
//!
//! The rule is deliberately conservative and was calibrated on the frozen EN/RU lexicons plus the Hunspell
//! dictionaries: the prefix must have no continuation at all in the source language (frequency lexicon and Hunspell
//! stems) and must not be a known word itself, while the alternative rendering must start many frequent words of the
//! other language.

use ks_cast::To as _;
use crate::constants::{DEFAULT_EARLY_SWITCH_MIN_LENGTH, EARLY_SWITCH_MIN_TARGET_COMPLETIONS, EARLY_SWITCH_MIN_TARGET_FREQUENCY, EARLY_SWITCH_DOMINANT_TARGET_FREQUENCY, PREFIX_INDEX_DICTIONARY_MAX_BYTES, PREFIX_INDEX_COMPLETION_SCAN_LIMIT};
use crate::language_model::{LanguageModel, WordScore};
use crate::tables::{WordList, WordTable};
use ks_text::{ucd, pystr};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The subset of `LanguageModel` needed to judge a prefix.
pub trait PrefixScorer {
    fn score(&self, word: &str) -> WordScore;
}

impl PrefixScorer for LanguageModel {
    fn score(&self, word: &str) -> WordScore {
        LanguageModel::score(self, word)
    }
}

/// What the lexicon of one language says about a prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrefixEvidence {
    pub completions: i64,
    pub maximum_frequency: i64,
    pub known: bool,
}

impl PrefixEvidence {
    #[must_use]
    pub fn as_dict(&self) -> Value {
        json!({"completions": self.completions, "maximum_frequency": self.maximum_frequency, "known": self.known})
    }
}

/// Thresholds; the defaults are the calibrated conservative values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EarlySwitchPolicy {
    pub minimum_length: usize,
    pub minimum_completions: i64,
    pub minimum_frequency: i64,
    pub dominant_frequency: i64,
}

impl Default for EarlySwitchPolicy {
    fn default() -> Self {
        EarlySwitchPolicy {
            minimum_length: DEFAULT_EARLY_SWITCH_MIN_LENGTH,
            minimum_completions: EARLY_SWITCH_MIN_TARGET_COMPLETIONS,
            minimum_frequency: EARLY_SWITCH_MIN_TARGET_FREQUENCY,
            dominant_frequency: EARLY_SWITCH_DOMINANT_TARGET_FREQUENCY,
        }
    }
}

impl EarlySwitchPolicy {
    #[must_use]
    pub fn as_dict(&self) -> Value {
        json!({
            "minimum_length": self.minimum_length,
            "minimum_completions": self.minimum_completions,
            "minimum_frequency": self.minimum_frequency,
            "dominant_frequency": self.dominant_frequency,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EarlySwitchDecision {
    pub should_switch: bool,
    pub source_group: i64,
    pub target_group: i64,
    pub original: String,
    pub replacement: String,
    pub reason: String,
    pub source_evidence: Option<PrefixEvidence>,
    pub target_evidence: Option<PrefixEvidence>,
}

impl EarlySwitchDecision {
    pub fn as_dict(&self) -> Value {
        json!({
            "should_switch": self.should_switch,
            "source_group": self.source_group,
            "target_group": self.target_group,
            "replacement": self.replacement,
            "reason": self.reason,
            "source_evidence": self.source_evidence.as_ref().map(PrefixEvidence::as_dict),
            "target_evidence": self.target_evidence.as_ref().map(PrefixEvidence::as_dict),
        })
    }
}

/// Sorted case-folded words of one language with lexicon frequencies.
pub struct PrefixIndex {
    /// `None`: the words are the keys of `frequencies`, as on a machine without Hunspell stems.
    words: Option<WordList>,
    frequencies: Arc<WordTable>,
}

impl PrefixIndex {
    /// `PrefixIndex(words, frequencies)`.
    pub fn new<S: AsRef<str>, I: IntoIterator<Item = S>>(words: I, frequencies: impl Into<Arc<WordTable>>) -> Self {
        let mut list = WordList::new();
        for word in words {
            if !word.as_ref().is_empty() {
                list.push(word.as_ref());
            }
        }
        PrefixIndex { words: Some(list.sorted_unique()), frequencies: frequencies.into() }
    }

    /// The body of `_cached_index` on the frequencies it found: the words of a lexicon plus the stems of the Hunspell
    /// dictionary `source` names, if it names one.
    ///
    /// `PrefixIndex.for_language_model` in the Python builds on `LanguageModel.load(locale)`, which holds no supplement,
    /// whatever the model passed: `Resources::prefix_index` does the same. A replay patches that `load` to return the
    /// reference model with the supplement; it passes that model's frequencies here.
    #[must_use]
    pub fn from_lexicon(frequencies: Arc<WordTable>, source: &str) -> Self {
        let stems = hunspell_dictionary_path(source).map(|path| dictionary_stems(&path, PREFIX_INDEX_DICTIONARY_MAX_BYTES));
        match stems {
            Some(stems) if !stems.is_empty() => PrefixIndex { words: Some(frequencies.words().merged(&stems)), frequencies },
            _ => PrefixIndex { words: None, frequencies },
        }
    }

    fn words(&self) -> &WordList {
        self.words.as_ref().unwrap_or_else(|| self.frequencies.words())
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.words().len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Count words starting with `prefix` and their best frequency.
    #[must_use]
    pub fn completions(&self, prefix: &str) -> PrefixEvidence {
        self.completions_limit(prefix, PREFIX_INDEX_COMPLETION_SCAN_LIMIT)
    }

    /// `completions(prefix, limit=limit)`.
    #[must_use]
    pub fn completions_limit(&self, prefix: &str, limit: usize) -> PrefixEvidence {
        let normalized = pystr::casefold(prefix);
        if normalized.is_empty() {
            return PrefixEvidence { completions: 0, maximum_frequency: 0, known: false };
        }
        let words = self.words();
        let start = words.lower_bound(&normalized);
        let stop = words.lower_bound(&format!("{normalized}\u{ffff}"));
        let last = stop.min(start.saturating_add(limit));
        let maximum = match &self.words {
            None => (start..last).map(|index| self.frequencies.value_at(index)).max(),
            Some(words) => (start..last).map(|index| self.frequencies.get_or_zero(words.get(index))).max(),
        };
        PrefixEvidence {
            completions: (stop - start).to::<i64>(),
            maximum_frequency: maximum.unwrap_or(0),
            known: self.frequencies.contains(&normalized) || (start < words.len() && words.get(start) == normalized),
        }
    }
}

#[must_use]
pub fn hunspell_dictionary_path(source: &str) -> Option<PathBuf> {
    for part in source.split(';') {
        let part = pystr::strip(part);
        if let Some(rest) = part.strip_prefix("Hunspell: ") {
            let candidate = PathBuf::from(pystr::strip(rest));
            if candidate.extension().is_some_and(|extension| extension == "dic") && candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Read the alphabetic stems of a Hunspell `.dic` file (bounded), sorted.
#[must_use]
pub fn dictionary_stems(path: &Path, limit: usize) -> WordList {
    let Ok(payload) = crate::load_error::read_limited(path, limit) else {
        return WordList::new();
    };
    if payload.len() > limit {
        return WordList::new();
    }
    let text = String::from_utf8_lossy(&payload);
    let mut stems = WordList::new();
    for line in pystr::splitlines(&text, false).into_iter().skip(1) {
        let head = line.split('/').next().unwrap_or("");
        let head = head.split('\t').next().unwrap_or("");
        let stem = pystr::casefold(pystr::strip(head));
        if pystr::isalpha(&stem) {
            stems.push(&stem);
        }
    }
    stems.sorted_unique()
}

/// ALL-CAPS, camelCase and similar shapes are never switched early.
pub fn looks_like_abbreviation(token: &str) -> bool {
    token.chars().skip(1).any(ucd::isupper)
}

fn decision(should_switch: bool, source_group: i64, target_group: i64, original: &str, replacement: &str, reason: &str) -> EarlySwitchDecision {
    EarlySwitchDecision {
        should_switch,
        source_group,
        target_group,
        original: original.to_string(),
        replacement: replacement.to_string(),
        reason: reason.to_string(),
        source_evidence: None,
        target_evidence: None,
    }
}

/// Decide whether the typed prefix already proves the wrong layout. The maps of the Python are lists of pairs;
/// `alternatives` keeps its order, as the first candidate in it is the one judged.
pub fn early_switch_decision(
    indexes: &[(i64, &PrefixIndex)],
    scorers: &[(i64, &dyn PrefixScorer)],
    original: &str,
    alternatives: &[(i64, &str)],
    source_group: i64,
    policy: &EarlySwitchPolicy,
) -> EarlySwitchDecision {
    let index_of = |group: i64| indexes.iter().find(|entry| entry.0 == group).map(|entry| entry.1);
    let scorer_of = |group: i64| scorers.iter().find(|entry| entry.0 == group).map(|entry| entry.1);
    let candidate = alternatives.iter().find(|entry| entry.0 != source_group && index_of(entry.0).is_some() && scorer_of(entry.0).is_some());
    let (Some(source_index), Some(source_scorer), Some(&(target_group, replacement))) = (index_of(source_group), scorer_of(source_group), candidate) else {
        return decision(false, source_group, source_group, original, original, "нет другой раскладки");
    };
    let target_index = index_of(target_group).expect("a candidate has an index");
    let mut rejected = decision(false, source_group, target_group, original, replacement, "");
    let with_reason = |rejected: &EarlySwitchDecision, reason: &str| EarlySwitchDecision { reason: reason.to_string(), ..rejected.clone() };
    if pystr::len(original) < policy.minimum_length {
        return with_reason(&rejected, "слишком короткий префикс");
    }
    // Only the rendering in the other layout has to be letters: the comma key is the letter "б" there, so `nt,z`
    // (тебя) is a valid prefix, while a digit or a real punctuation mark is not a letter in either layout.
    if !pystr::isalpha(replacement) {
        return with_reason(&rejected, "префикс содержит не буквы");
    }
    if looks_like_abbreviation(original) || looks_like_abbreviation(replacement) {
        return with_reason(&rejected, "похоже на сокращение или camelCase");
    }
    let source_evidence = source_index.completions(original);
    rejected.source_evidence = Some(source_evidence);
    if source_evidence.completions > 0 {
        return with_reason(&rejected, "префикс продолжается в исходном языке");
    }
    if source_evidence.known || source_scorer.score(original).known {
        return with_reason(&rejected, "префикс сам является словом исходного языка");
    }
    let target_evidence = target_index.completions(replacement);
    rejected.target_evidence = Some(target_evidence);
    let strong = target_evidence.completions >= policy.minimum_completions && target_evidence.maximum_frequency >= policy.minimum_frequency;
    let dominant = target_evidence.maximum_frequency >= policy.dominant_frequency;
    if !strong && !dominant {
        return with_reason(&rejected, "целевой префикс не начинает частотных слов");
    }
    let reason = if dominant { "целевой префикс доминирует по частоте" } else { "исходный префикс невозможен, целевой начинает частотные слова" };
    EarlySwitchDecision { should_switch: true, reason: reason.to_string(), ..rejected }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    fn word_score(known: bool) -> WordScore {
        WordScore { value: 0.0, known, frequency: 0, gram_ratio: 0.0, exact: false, spell_known: false, ngram_score: 0.0, invalid_ratio: 0.0, raw_ngram_score: 0.0 }
    }

    struct Scorer(bool);
    impl PrefixScorer for Scorer {
        fn score(&self, _word: &str) -> WordScore {
            word_score(self.0)
        }
    }

    #[test]
    fn completions_count_and_scan() {
        let table = WordTable::from_pairs([("прив", 5), ("привет", 100), ("привык", 200), ("пока", 1)]);
        let index = PrefixIndex::new(["прив", "привет", "привык", "пока", "приз"], table);
        assert_eq!(index.len(), 5);
        assert_eq!(index.completions("ПРИВ"), PrefixEvidence { completions: 3, maximum_frequency: 200, known: true });
        assert_eq!(index.completions_limit("прив", 2).maximum_frequency, 100);
        assert_eq!(index.completions("").completions, 0);
        assert_eq!(index.completions("приз"), PrefixEvidence { completions: 1, maximum_frequency: 0, known: true });
        assert_eq!(index.completions("я"), PrefixEvidence { completions: 0, maximum_frequency: 0, known: false });
    }

    #[test]
    fn decides_like_the_rule() {
        let russian: Vec<(String, i64)> = (0..12).map(|i| (format!("прив{}", char::from_u32(0x430 + i).unwrap()), 3000)).collect();
        let source = PrefixIndex::from_lexicon(Arc::new(WordTable::from_pairs([("hello", 10)])), "lm");
        let target = PrefixIndex::from_lexicon(Arc::new(WordTable::from_pairs(russian)), "lm");
        let indexes = [(0, &source), (1, &target)];
        let (unknown, known) = (Scorer(false), Scorer(true));
        let scorers: [(i64, &dyn PrefixScorer); 2] = [(0, &unknown), (1, &unknown)];
        let policy = EarlySwitchPolicy::default();
        let run = |original: &str, replacement: &str, scorers: &[(i64, &dyn PrefixScorer)]| early_switch_decision(&indexes, scorers, original, &[(1, replacement)], 0, &policy);
        assert!(run("ghbd", "прив", &scorers).should_switch);
        assert_eq!(run("ghbd", "прив", &scorers).reason, "исходный префикс невозможен, целевой начинает частотные слова");
        assert_eq!(run("gh", "пр", &scorers).reason, "слишком короткий префикс");
        assert_eq!(run("hell", "рудд", &scorers).reason, "префикс продолжается в исходном языке");
        assert_eq!(run("gh1b", "пр1и", &scorers).reason, "префикс содержит не буквы");
        assert_eq!(run("gHbd", "пРив", &scorers).reason, "похоже на сокращение или camelCase");
        assert_eq!(run("ghbd", "зззз", &scorers).reason, "целевой префикс не начинает частотных слов");
        let knowing: [(i64, &dyn PrefixScorer); 2] = [(0, &known), (1, &unknown)];
        assert_eq!(run("ghbd", "прив", &knowing).reason, "префикс сам является словом исходного языка");
        let nothing = early_switch_decision(&indexes, &scorers, "ghbd", &[], 0, &policy);
        assert_eq!((nothing.reason.as_str(), nothing.target_group, nothing.replacement.as_str()), ("нет другой раскладки", 0, "ghbd"));
        let dict = run("ghbd", "прив", &scorers).as_dict();
        assert_eq!(dict["target_evidence"]["completions"], 12);
        assert_eq!(dict.as_object().unwrap().keys().next().unwrap(), "should_switch");
    }

    #[test]
    fn dictionary_paths_and_abbreviations() {
        assert_eq!(hunspell_dictionary_path("x; Hunspell: /nonexistent-ks-lang/a.dic"), None);
        assert_eq!(hunspell_dictionary_path("x"), None);
        assert!(looks_like_abbreviation("aB") && !looks_like_abbreviation("Ab") && !looks_like_abbreviation(""));
        let path = std::env::temp_dir().join(format!("ks-lang-stems-{}.dic", std::process::id()));
        std::fs::write(&path, "3\nСлово/ABC\nabc\tpo:x\n123\n слово \n").unwrap();
        let stems = dictionary_stems(&path, 1000);
        assert_eq!(stems.iter().collect::<Vec<_>>(), ["abc", "слово"]);
        assert!(dictionary_stems(&path, 3).is_empty());
        assert_eq!(hunspell_dictionary_path(&format!("x; Hunspell: {}", path.display())), Some(path.clone()));
        std::fs::remove_file(&path).unwrap();
    }
}
