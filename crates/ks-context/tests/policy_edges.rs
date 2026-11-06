//! The context policy asked what only unusual callers ask: another layout, no words, a model that is not there.

use std::path::PathBuf;
use std::sync::Arc;

use ks_backend::{FieldContext, FieldReader, FieldRole};
use ks_context::context_policy::{ContextPolicy, DecideOptions, EvidenceOptions, evidence_for_decision};
use ks_context::input_context::InputContext;
use ks_lang::Resources;
use ks_lang::detector::{DetectionDecision, LanguageDetector, LanguageScorer, ProtectedTokens};

fn resources() -> Arc<Resources> {
    Resources::new(&PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources")))
}

struct Shows(FieldContext);

impl FieldReader for Shows {
    fn read(&self, _application: &str, _window: i64) -> Option<FieldContext> {
        Some(self.0.clone())
    }

    fn status(&self) -> String {
        "available".to_owned()
    }
}

fn field(before: &str) -> FieldContext {
    let mut field = FieldContext::new("firefox", "1");
    before.clone_into(&mut field.before);
    field.role = FieldRole::Text;
    field
}

fn baseline(detector: &LanguageDetector, should_convert: bool, original: &str, replacement: &str, source_group: i64) -> DetectionDecision {
    let score = detector.model_for(0).expect("the first model").score(original);
    DetectionDecision::new(should_convert, original, replacement, source_group, 1 - source_group, 0.9, "test", score.clone(), score)
}

#[test]
fn a_layout_with_no_orthotactic_side_is_read_by_no_orthotactic_model() {
    let resources = resources();
    let en = resources.engine_language_model("en_US").expect("en");
    let models: Vec<(i64, Arc<dyn LanguageScorer>)> = vec![(0, en.clone()), (1, resources.engine_language_model("ru_RU").expect("ru")), (2, en)];
    let detector = LanguageDetector::new(models, None, Arc::new(ProtectedTokens::default())).expect("three models");
    let policy = ContextPolicy::new(&resources, None, None, None);
    let ortho = policy.ortho.clone();
    assert!(ortho.is_some());
    let third = baseline(&detector, false, "ghbdtn", "привет", 2);
    // Without an orthotactic model at hand, and with one, a third layout has no reading of its keys.
    let shown = field("x ");
    let plain = evidence_for_decision(&third, "привет", 0, &detector, &shown, "space", &EvidenceOptions::default(), None).expect("evidence");
    assert!(plain.ortho_score.is_none());
    let read = evidence_for_decision(&third, "привет", 0, &detector, &shown, "space", &EvidenceOptions { ortho: ortho.as_deref(), ..EvidenceOptions::default() }, None).expect("evidence");
    assert!(read.ortho_score.is_none());
    assert!(policy.orthotactic(&third, "привет", 0, &shown, Some(&detector)).is_none());
    // Asked with no detector, for a word of the layouts it knows, and for a shape it does not score.
    let first = baseline(&detector, false, "ghbdtn", "привет", 0);
    let mut unknown = first.clone();
    unknown.source_score.known = false;
    let _ = policy.orthotactic(&unknown, "привет", 1, &shown, None);
    let mut shapeless = (*ortho.expect("the model")).clone();
    shapeless.unscored_shapes = vec!["lower".to_owned(), "initial".to_owned(), "inner".to_owned(), "upper".to_owned()];
    let mut other = ContextPolicy::new(&resources, None, None, None);
    other.ortho = Some(Arc::new(shapeless));
    assert!(other.orthotactic(&unknown, "привет", 1, &shown, Some(&detector)).is_none());
}

#[test]
fn a_policy_with_a_model_and_no_orthotactic_file_still_starts() {
    let resources = resources();
    let policy = ContextPolicy::with_ortho(&resources, None, (None, "unavailable".to_owned()), None);
    assert!(policy.model.is_some() && policy.ortho.is_none());
}

#[test]
fn a_word_of_no_letters_is_not_looked_for_in_the_field() {
    let resources = resources();
    let detector = resources.language_detector(&["en_US", "ru_RU"]).expect("the detector");
    let reader: Arc<dyn FieldReader> = Arc::new(Shows(field("текст ")));
    let policy = ContextPolicy::new(&resources, Some(reader), None, None);
    let nothing = baseline(&detector, false, "", "", 0);
    let options = DecideOptions { read_field: true, ..DecideOptions::default() };
    assert!(policy.decide(&nothing, "", 1, &detector, "space", "assist", &options).is_ok());
    let stream = InputContext::new();
    assert_eq!(stream.before_word(""), "");
}

#[test]
fn a_short_word_rule_is_kept_by_a_model_of_another_generation() {
    let resources = resources();
    let detector = resources.language_detector(&["en_US", "ru_RU"]).expect("the detector");
    let mut policy = ContextPolicy::new(&resources, None, None, None);
    policy.model.as_mut().expect("the model").feature_version = 2;
    let mut rule = baseline(&detector, true, "z", "я", 0);
    rule.reason = ks_lang::short_words::SHORT_WORD_REASONS[0].to_owned();
    let options = DecideOptions { field_override: Some(field("начало ")), ..DecideOptions::default() };
    assert!(policy.decide(&rule, "я", 1, &detector, "space", "assist", &options).is_ok());
}
