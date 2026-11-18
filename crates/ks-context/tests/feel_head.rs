//! The feel head: its class (a word of the message-start head's class that no lexicon and no identifier index knows in
//! either reading, scored by the orthotactic model) and the evidence it reads there: how much better a key the other
//! reading fits its language than the reading as typed fits its own, and the typed reading's own fit.

use ks_backend::{FieldContext, FieldRole};
use ks_context::constants::{FEEL_FEATURE_PREFIX, START_FEATURE_PREFIX};
use ks_context::context_action_features::{extract_action_features, feel_question};
use ks_context::context_model::{ContextEvidence, TermTables};
use std::collections::BTreeSet;
use std::path::PathBuf;

fn tables() -> TermTables {
    TermTables::load(&PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources/models"))).expect("the shipped tables")
}

/// `uegieg` typed alone in the English layout, both readings unknown, with the orthotactic means given.
fn evidence(original: &str, alternative: &str, before: &str, means: (Option<f64>, Option<f64>)) -> ContextEvidence {
    let mut field = FieldContext::new("firefox", "1");
    before.clone_into(&mut field.before);
    field.role = FieldRole::Text;
    let mut item = ContextEvidence::new(original, alternative, i64::from(!original.is_ascii()), field);
    " ".clone_into(&mut item.boundary_text);
    "space".clone_into(&mut item.trigger);
    item.ortho_score = Some(-3.0);
    item.ortho_threshold = Some(13.3);
    (item.ortho_typed_mean, item.ortho_other_mean) = means;
    item
}

/// The head's features of a question, without their prefix.
fn head(item: &ContextEvidence, tables: &TermTables) -> BTreeSet<String> {
    let features = extract_action_features(item, tables).expect("features");
    features.keys().filter_map(|name| name.strip_prefix(FEEL_FEATURE_PREFIX)).map(str::to_owned).collect()
}

#[test]
fn the_class_is_an_unknown_word_alone_with_both_readings_scored() {
    let means = (Some(-8.2), Some(-4.1));
    let member = evidence("uegieg", "гупшуп", "", means);
    assert_eq!(feel_question(&member), Some((-8.2, -4.1)));
    // Text before it, or a letter after it: not the first word of a message.
    assert_eq!(feel_question(&evidence("uegieg", "гупшуп", "привет ", means)), None);
    let mut after = evidence("uegieg", "гупшуп", "", means);
    "потом".clone_into(&mut after.field.after);
    assert_eq!(feel_question(&after.finished()), None);
    // Three letters, or signs in the other reading: the start head's class does not hold them.
    assert_eq!(feel_question(&evidence("ufc", "гас", "", means)), None);
    assert_eq!(feel_question(&evidence("[kt,", "хлеб", "", means)), None);
    // A reading a lexicon or the identifier index knows.
    for known in 0..4 {
        let mut item = evidence("uegieg", "гупшуп", "", means);
        match known {
            0 => item.source_known = true,
            1 => item.target_known = true,
            2 => item.source_identifier = true,
            _ => item.target_identifier = true,
        }
        assert_eq!(feel_question(&item), None, "{known}");
    }
    // No orthotactic model, or a token it does not score.
    assert_eq!(feel_question(&evidence("uegieg", "гупшуп", "", (None, None))), None);
    assert_eq!(feel_question(&evidence("uegieg", "гупшуп", "", (Some(-8.2), None))), None);
}

#[test]
fn the_head_reads_the_gap_a_key_and_the_typed_fit_by_direction() {
    let tables = tables();
    // The other reading fits its language 4.1 better a key: the band of 3 to 4.5; the typed one -8.2, the band of -10 to -8.
    let features = head(&evidence("uegieg", "гупшуп", "", (Some(-8.2), Some(-4.1))), &tables);
    assert_eq!(features, BTreeSet::from(["bias:direction:0".to_owned(), "gap:2:direction:0".to_owned(), "typed:-5:direction:0".to_owned(), "gap:2:typed:-5:direction:0".to_owned()]));
    // Typed as meant in the Russian layout: the gap goes the other way, and the direction is the other one.
    let features = head(&evidence("гупшуп", "uegieg", "", (Some(-4.1), Some(-8.2))), &tables);
    assert!(features.contains("gap:-3:direction:1") && features.contains("typed:-3:direction:1"), "{features:?}");
    // The bands stop at their bounds either way.
    let features = head(&evidence("uegieg", "гупшуп", "", (Some(-40.0), Some(-1.0))), &tables);
    assert!(features.contains("gap:4:direction:0") && features.contains("typed:-6:direction:0"), "{features:?}");
    let features = head(&evidence("uegieg", "гупшуп", "", (Some(-1.0), Some(-40.0))), &tables);
    assert!(features.contains("gap:-4:direction:0") && features.contains("typed:-1:direction:0"), "{features:?}");
    // Outside the class the head reads nothing, and the start head still answers the word.
    let mut known = evidence("uegieg", "гупшуп", "", (Some(-8.2), Some(-4.1)));
    known.target_known = true;
    assert!(head(&known, &tables).is_empty());
    let features = extract_action_features(&known, &tables).expect("features");
    assert!(features.keys().any(|name| name.starts_with(START_FEATURE_PREFIX)));
}
