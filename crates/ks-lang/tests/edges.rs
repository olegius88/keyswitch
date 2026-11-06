//! What the language files and the decisions meet outside the shipped data.
#![expect(clippy::unwrap_used, reason = "a test stops on what it did not expect")]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use ks_lang::detector::{DecideOptions, LanguageDetector, LanguageScorer, ProtectedTokens};
use ks_lang::early_switch::{EarlySwitchPolicy, PrefixIndex, PrefixScorer, early_switch_decision};
use ks_lang::language_model::WordScore;
use ks_lang::lexicon_supplement::supplement_words;
use ks_lang::ortho_model::OrthoModel;
use ks_lang::short_words::trusted_short_word_decision;
use ks_lang::tables::WordTable;
use ks_lang::word_decision::opening_letter_decision;
use ks_lang::Resources;

fn folder(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!("ks-lang-edges-{name}-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn resources() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources"))
}

fn supplement_of(root: &Path, text: &str) -> Result<usize, String> {
    std::fs::write(root.join("lexicon-supplement-ru_RU.json"), text).unwrap();
    supplement_words(root, "ru_RU").map(|words| words.iter().count()).map_err(|error| error.message)
}

#[test]
fn a_supplement_holds_only_sorted_strings() {
    let root = folder("supplement");
    assert_eq!(supplement_of(&root, r#"{"schema_version": 1, "locale": "ru_RU", "words": ["абв", "абг"]}"#), Ok(2));
    for element in ["true", "1", "-1", "1.5", "null", "[1]", r#"{"a": 1}"#] {
        let text = format!(r#"{{"schema_version": 1, "locale": "ru_RU", "words": ["абв", {element}]}}"#);
        assert!(supplement_of(&root, &text).is_err(), "{element}");
    }
    assert!(supplement_of(&root, r#"{"schema_version": 1, "locale": "ru_RU", "words": ["абг", "абв"]}"#).is_err());
    let large = " ".repeat(8 * 1024 * 1024 + 1);
    assert!(supplement_of(&root, &large).is_err());
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn an_orthotactic_file_past_its_limit_is_unavailable() {
    let root = folder("ortho");
    let path = root.join("ortho.json");
    std::fs::write(&path, " ".repeat(24 * 1024 * 1024 + 1)).unwrap();
    assert!(OrthoModel::try_load(&path, None).0.is_none());
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn what_is_read_once_is_kept_and_a_lexicon_that_is_gone_knows_nothing() {
    let shipped = Resources::new(resources());
    let model = shipped.engine_language_model("en_US").unwrap();
    let first = shipped.prefix_index(&model);
    let second = shipped.prefix_index(&model);
    assert!(Arc::ptr_eq(&first, &second));
    let empty = folder("no-resources");
    let nothing = Resources::new(empty.clone());
    assert!(!(nothing.packaged_identifier())("htop"));
    std::fs::remove_dir_all(&empty).unwrap();
}

/// A language model that answers what the test tells it to.
struct Scripted(WordScore);

impl LanguageScorer for Scripted {
    fn score(&self, _word: &str) -> WordScore {
        self.0.clone()
    }
    fn context_score(&self, _previous: &str, _word: &str) -> f64 {
        0.0
    }
    fn best_single_deletion(&self, _word: &str) -> WordScore {
        self.0.clone()
    }
}

impl PrefixScorer for Scripted {
    fn score(&self, _word: &str) -> WordScore {
        self.0.clone()
    }
}

fn scored(known: bool, exact: bool, spell_known: bool, ngram_score: f64, value: f64) -> WordScore {
    WordScore { value, known, exact, spell_known, ngram_score, frequency: 1, gram_ratio: 0.5, invalid_ratio: 0.5, raw_ngram_score: ngram_score }
}

fn detector(source: WordScore, target: WordScore) -> LanguageDetector {
    let models: Vec<(i64, Arc<dyn LanguageScorer>)> = vec![(0, Arc::new(Scripted(source))), (1, Arc::new(Scripted(target)))];
    LanguageDetector::new(models, None, Arc::new(ProtectedTokens::default())).unwrap()
}

#[test]
fn a_rare_form_needs_context_to_convert() {
    let rare = detector(scored(false, false, false, -9.0, -9.0), scored(true, false, true, -99.0, 9.0));
    let decision = rare.decide("ghbdtn", &[(1, "привет".to_owned())], 0, &DecideOptions::default()).unwrap();
    assert!(!decision.should_convert);
    assert!(decision.reason.contains("редкая"), "{}", decision.reason);
    let with_context = rare.decide("ghbdtn", &[(1, "привет".to_owned())], 0, &DecideOptions { context_group: Some(1), ..DecideOptions::default() }).unwrap();
    assert!(with_context.reason.contains("морфологическим") || with_context.reason.contains("перевес"), "{}", with_context.reason);
}

#[test]
fn a_target_with_no_model_is_passed_over() {
    let plain = detector(scored(false, false, false, -9.0, -9.0), scored(true, true, false, 0.0, 9.0));
    assert!(trusted_short_word_decision(&plain, "d", &[(5, "в".to_owned())], 0, &[], &[], false, None).unwrap().is_none());
    assert!(opening_letter_decision(&plain, "d", &[(5, "в".to_owned())], 0, None, &[], &[], false, &[]).unwrap().is_none());
    assert!(opening_letter_decision(&plain, "d", &[(0, "в".to_owned())], 0, None, &[], &[], false, &[]).unwrap().is_none());
}

#[test]
fn an_early_switch_refuses_what_looks_like_an_abbreviation_or_a_word_of_the_source() {
    let frequencies = Arc::new(WordTable::from_pairs([("hello", 5), ("привет", 5)]));
    let index = PrefixIndex::new(["hello"], frequencies.clone());
    let other = PrefixIndex::new(["привет"], frequencies);
    let unknown = Scripted(scored(false, false, false, -9.0, -9.0));
    let known = Scripted(scored(true, true, false, 0.0, 9.0));
    let indexes: [(i64, &PrefixIndex); 2] = [(0, &index), (1, &other)];
    let decide = |source: &Scripted, original: &str, replacement: &str| {
        let scorers: [(i64, &dyn PrefixScorer); 2] = [(0, source), (1, &unknown)];
        early_switch_decision(&indexes, &scorers, original, &[(1, replacement)], 0, &EarlySwitchPolicy::default())
    };
    assert!(decide(&unknown, "ghbd", "ПрИв").reason.contains("сокращение"));
    assert!(decide(&known, "ghbd", "прив").reason.contains("словом исходного"));
}
