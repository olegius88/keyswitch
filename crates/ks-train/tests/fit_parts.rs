//! The parts of a fit one by one: the numeric core's refusals, what a seal pins of a tree, the evidence of a frame.
#![expect(clippy::disallowed_types, clippy::disallowed_macros, clippy::disallowed_methods, reason = "temporary: these tests still build and read their JSON as untyped trees; step 7 of the strict-json plan types them and removes this")]
mod support;

use ks_backend::FieldRole;
use ks_context::context_model::ContextAction;
use ks_lang::Resources;
use ks_lang::detector::{IntentClassifier, LanguageDetector, LanguageScorer};
use ks_train::evidence::frame_evidence;
use ks_train::fit::provenance;
use ks_train::historical::{build_corpus, historical_curriculum};
use ks_train::kernel::{FeatureMass, Packed};
use ks_train::rows::{ActionRow, field_of};
use ks_train::tools::{Lexicons, Tools, reference_models};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use support::{scratch, tree};

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().expect("a folder")).expect("a folder");
    std::fs::write(path, text).expect("a file");
}

#[test]
fn packed_rows_refuse_a_target_or_an_importance_that_cannot_be() {
    let names = vec!["a".to_owned()];
    let row = vec![("a".to_owned(), 1.0)];
    for (label, importance) in [(4_u8, 1.0), (0, f64::NAN), (0, f64::INFINITY), (0, 0.0), (0, -1.0)] {
        assert_eq!(Packed::build([(row.as_slice(), label, importance)], &names), Err("invalid training target or importance".to_owned()), "{label} {importance}");
    }
    let odd = vec![("a".to_owned(), f64::NAN)];
    assert_eq!(Packed::build([(odd.as_slice(), 0, 1.0)], &names), Err("non-finite feature".to_owned()));
    // A feature outside the vocabulary is left out, and its row stays.
    let outside = vec![("b".to_owned(), 1.0), ("a".to_owned(), 2.0)];
    let packed = Packed::build([(outside.as_slice(), 3, 0.5)], &names).expect("a row");
    assert_eq!((packed.offsets, packed.indices, packed.values, packed.labels, packed.importance), (vec![0, 1], vec![0], vec![2.0], vec![3], vec![0.5]));
}

#[test]
fn a_feature_mass_refuses_a_weight_that_cannot_be_and_a_sum_that_overflows() {
    let mut mass = FeatureMass::default();
    for weight in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(mass.add(["a"], weight), Err("feature sample weight must be positive and finite".to_owned()), "{weight}");
    }
    mass.add(["a"], f64::MAX).expect("the largest weight");
    assert_eq!(mass.add(["a"], f64::MAX), Err("feature mass overflow".to_owned()));
}

const CRATES: [&str; 9] = ["ks-text", "ks-backend", "ks-config", "ks-lang", "ks-context", "ks-engine", "ks-replay", "ks-train", "ks-hunspell"];
const PINNED: [&str; 10] = [
    "model/context_v1/scenarios.json", "model/context_v1/captured/manifest.json", "model/context_v3/captured/manifest.json", "model/context_v3/baseline-context-v1.json",
    "model/intent_v1/config.json", "model/intent_v1/sources/en_US.lm", "model/intent_v1/sources/ru_RU.lm", "resources/protected_tokens.txt", "resources/identifiers.json",
    "resources/lexicon-supplement-ru_RU.json",
];

#[test]
fn a_seal_pins_the_sources_and_models_a_fit_reads_and_nothing_else() {
    let root = scratch("provenance");
    for name in CRATES {
        write(&root.join("crates").join(name).join("src/lib.rs"), name);
    }
    // Rust and TOML files at any depth are sources; other files are not.
    write(&root.join("crates/ks-train/src/inner/part.rs"), "part");
    write(&root.join("crates/ks-lang/src/table.toml"), "table");
    write(&root.join("crates/ks-lang/src/notes.txt"), "notes");
    write(&root.join("crates/ks-lang/src/README"), "readme");
    write(&root.join("Cargo.lock"), "lock");
    // The models are pinned but the shipped pair the fit replaces, and a folder among them is not a model.
    for name in ["boundary.json", "context_policy_v1.json", "prefix_policy_v1.json", "inner/model.json"] {
        write(&root.join("resources/models").join(name), name);
    }
    for name in PINNED {
        write(&root.join(name), name);
    }
    write(&root.join("recipe.json"), "{}");
    let pins = provenance(&root, &root.join("recipe.json")).expect("the pins");
    for name in ["crates/ks-train/src/lib.rs", "crates/ks-train/src/inner/part.rs", "crates/ks-lang/src/table.toml", "Cargo.lock", "resources/models/boundary.json", "recipe.json"] {
        assert!(pins.contains_key(name), "{name}: {pins:?}");
    }
    for name in ["crates/ks-lang/src/notes.txt", "crates/ks-lang/src/README", "resources/models/context_policy_v1.json", "resources/models/prefix_policy_v1.json", "resources/models/inner", "resources/models/inner/model.json"] {
        assert!(!pins.contains_key(name), "{name}: {pins:?}");
    }
    assert_eq!(pins.len(), CRATES.len() + 2 + 1 + 1 + PINNED.len() + 1);
    // A recipe outside the tree is pinned by its own path.
    let outside = scratch("provenance-recipe").join("recipe.json");
    write(&outside, "{}");
    assert!(provenance(&root, &outside).expect("the pins").contains_key(&outside.to_string_lossy().into_owned()));
    // What a fit reads has to be there.
    std::fs::remove_file(root.join(PINNED[0])).expect("a file");
    assert!(provenance(&root, &outside).is_err());
    std::fs::remove_dir_all(root.join("resources/models")).expect("a folder");
    assert!(provenance(&root, &outside).is_err());
    std::fs::remove_dir_all(root.join("crates/ks-hunspell")).expect("a folder");
    assert!(provenance(&root, &outside).is_err());
}

#[test]
#[should_panic(expected = "no trigger nothing")]
fn a_trigger_the_engine_does_not_have_is_no_trigger() {
    let _ = ks_train::evidence::trigger_of("nothing");
}

/// The lexicons and the detector a fit's reference profile decides with.
static LEXICONS: LazyLock<(Arc<Resources>, Lexicons, LanguageDetector)> = LazyLock::new(|| {
    let resources: Arc<Resources> = Resources::new(tree().join("resources"));
    let portable = reference_models(&tree(), &resources, false).expect("the lexicons");
    let (intent, _) = resources.intent_model();
    let intent: Option<Arc<dyn IntentClassifier>> = intent.map(|model| -> Arc<dyn IntentClassifier> { model });
    let scorers: Vec<(i64, Arc<dyn LanguageScorer>)> = portable.iter().enumerate().map(|(group, model)| -> (i64, Arc<dyn LanguageScorer>) { (i64::try_from(group).expect("a group"), model.clone()) }).collect();
    let detector = LanguageDetector::new(scorers, intent, resources.protected_tokens()).expect("a detector");
    (resources, portable, detector)
});

/// The tools of a tree whose authored scenarios are these.
fn with_scenarios(scenarios: &Value) -> Tools {
    let root: PathBuf = scratch("scenarios");
    write(&root.join("model/context_v1/scenarios.json"), &scenarios.to_string());
    let (resources, portable, _) = &*LEXICONS;
    Tools::new(&root, resources.clone(), portable.clone(), BTreeSet::new()).expect("the tools")
}

#[test]
fn authored_scenarios_frame_what_their_words_allow() {
    let (_, portable, detector) = &*LEXICONS;
    let refused = build_corpus(&with_scenarios(&json!({"schema_version": 2})), detector, portable);
    assert!(refused.is_err_and(|error| error.contains("invalid scenarios")));
    // Russian words, one of them an English word in the other layout (руки, herb); every trusted short word as one that
    // never opens a message; short unknown tokens, one of them (if) a word the detector converts to on its own; an inside
    // context with a tail of no word after it.
    let trusted: Vec<&str> = ks_lang::short_words::trusted_short_words(1);
    let scenarios = json!({
        "schema_version": 1, "russian": ["руки", "работа", "погода", "дорога", "машина"], "short_russian_not_opening": trusted,
        "technical_terms": ["ab", "xq", "js", "go", "qz"], "english_unknown": ["ok", "zz", "hi", "wq", "if"], "russian_unknown": ["ыв"],
        "inside_contexts": ["", "я думаю что "], "inside_after": {"russian": "!!", "english": "?"},
    });
    let tools = with_scenarios(&scenarios);
    let rows = build_corpus(&tools, detector, portable).expect("the scenarios");
    assert!(!rows.is_empty());
    let frames = historical_curriculum(&tools, detector, portable).expect("the curriculum");
    let actions: BTreeSet<&str> = frames.iter().filter(|frame| frame.category == "legacy_trusted_short_wrong").map(|frame| frame.action.as_str()).collect();
    assert!(actions.contains(ContextAction::Convert.as_str()), "{actions:?}");
    assert!(frames.iter().any(|frame| frame.category == "legacy_inside_wrong" && frame.field.after == "!!"));
    assert!(!frames.iter().any(|frame| frame.original == "herb" && frame.category == "legacy_revisit_stays"));
    let unknown: BTreeSet<&str> = frames.iter().filter(|frame| frame.category.ends_with("_unknown_wrong") || frame.category == "legacy_technical_terms_wrong").map(|frame| frame.action.as_str()).collect();
    assert!(unknown.contains(ContextAction::Convert.as_str()), "{unknown:?}");
}

#[test]
fn the_evidence_of_a_frame_drops_what_training_drops_for_some_families() {
    let (resources, portable, detector) = &*LEXICONS;
    let tools = Tools::new(&tree(), resources.clone(), portable.clone(), BTreeSet::new()).expect("the tools");
    let frame = |identifier: &str, original: &str, group: i64, before: &str| ActionRow::new(identifier, original, group, field_of("Telegram", "field", before, "", FieldRole::Text), "space", ContextAction::Convert, "test", " ");
    // A word before the caret that is no pair of keys in either layout gives the frame no context of its own.
    let mixed = frame_evidence(&tools, &frame("mixed:1", "ghbdtn", 0, "привет словоabc "), "train", detector, None);
    let plain = frame_evidence(&tools, &frame("mixed:1", "ghbdtn", 0, ""), "train", detector, None);
    assert_eq!(mixed.original, plain.original);
    // Commands of one family share the identifier dropout; a few families of unknown tokens train without their counts.
    for index in 0..12 {
        let _ = frame_evidence(&tools, &frame(&format!("legacy:command:{index}:x"), "ghbdtn", 0, ""), "train", detector, None);
    }
    let dropped: Vec<bool> = (0..24).map(|index| frame_evidence(&tools, &frame(&format!("unknown-{index}:0"), "rjvgbkznjh", 0, ""), "train", detector, None).term_counts_dropped).collect();
    assert!(dropped.contains(&true) && dropped.contains(&false), "{dropped:?}");
    assert!(!frame_evidence(&tools, &frame("unknown-0:0", "rjvgbkznjh", 0, ""), "development", detector, None).term_counts_dropped);
    assert!(!frame_evidence(&tools, &frame("unknown-0:0", "rjv", 0, ""), "train", detector, None).term_counts_dropped);
}
