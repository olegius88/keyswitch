//! The upper-case head: its class, its evidence at a word's own boundary and beside a next word kept as typed, and the
//! classes of capitals it took from the lone-word, short-message and known-reply heads.

use ks_backend::{FieldContext, FieldRole};
use ks_context::constants::UPPER_FEATURE_PREFIX;
use ks_context::context_action_features::{alone_question, brief_question, extract_action_features, known_reply_question, upper_question, upper_shape};
use ks_context::context_model::{AfterOrigin, ContextEvidence, TermTables};
use std::collections::BTreeSet;
use std::path::PathBuf;

fn models() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources/models"))
}

fn tables() -> TermTables {
    TermTables::load(&models()).expect("the shipped tables")
}

fn evidence(original: &str, alternative: &str, before: &str) -> ContextEvidence {
    let mut field = FieldContext::new("firefox", "1");
    before.clone_into(&mut field.before);
    field.role = FieldRole::Text;
    let mut item = ContextEvidence::new(original, alternative, i64::from(!original.is_ascii()), field);
    " ".clone_into(&mut item.boundary_text);
    item
}

/// The head's features of a question, without their prefix.
fn head(item: &ContextEvidence, tables: &TermTables) -> BTreeSet<String> {
    let features = extract_action_features(item, tables).expect("features");
    features.keys().filter_map(|name| name.strip_prefix(UPPER_FEATURE_PREFIX)).map(str::to_owned).collect()
}

#[test]
fn the_shape_is_a_token_in_capitals_alone_or_after_russian_text() {
    assert!(upper_shape("USD", "ГЫВ", ""));
    assert!(upper_shape("ГЫВ", "USD", "курс "));
    assert!(upper_shape("CCCH", "СССР", "12 "));
    assert!(upper_shape("ЛЯЕ", "KZT", "в Kotlin и курс валют "));
    assert!(!upper_shape("USD", "ГЫВ", "the price in "));
    assert!(!upper_shape("Usd", "Гыв", ""));
    assert!(!upper_shape("U", "Г", ""));
    assert!(!upper_shape("WINDOWS", "ЦШТВЩЦЫ", ""));
    assert!(!upper_shape("ΑΒΓ", "ΑΒΓ", ""));
    assert!(!upper_shape("AC<", "ФСБ", ""));
    assert!(!upper_shape("ФСБ", "AC<", ""));
    assert!(!upper_shape("U2", "Г2", ""));
}

#[test]
fn the_class_is_a_token_some_source_spells_in_capitals() {
    let tables = tables();
    // Counted in capitals in either reading, a currency code, a form the lexicon writes in capitals.
    for (original, alternative) in [("USD", "ГЫВ"), ("CCCH", "СССР"), ("API", "ФЗШ"), ("FYI", "АНШ"), ("ЛЯЕ", "KZT"), ("UAH", "ГФР"), ("XTS", "ЧЕЫ"), ("ВЛКСМ", "DKRCV")] {
        assert_eq!(upper_question(original, alternative, "", &tables), Ok(true), "{original}");
    }
    // No source spells it so, or no token of the shape.
    for (original, alternative) in [("LGTM", "ДПЕЬ"), ("ЗОР", "PJH"), ("usd", "гыв")] {
        assert_eq!(upper_question(original, alternative, "", &tables), Ok(false), "{original}");
    }
    // A table that does not load is no answer.
    let folder = std::env::temp_dir().join(format!("ks-upper-head-{}", std::process::id()));
    std::fs::create_dir_all(&folder).expect("a folder");
    std::fs::copy(models().join("context-term-frequency.json"), folder.join("context-term-frequency.json")).expect("the term table");
    let without = TermTables::load(&folder).expect("the term table loads");
    assert!(upper_question("USD", "ГЫВ", "", &without).is_err());
    assert!(extract_action_features(&evidence("USD", "ГЫВ", ""), &without).is_err());
    let mut kept = evidence("ФШ", "AI", "");
    "курс".clone_into(&mut kept.field.after);
    kept.after_origin = AfterOrigin::KeptNextWord;
    assert!(extract_action_features(&kept, &without).is_err());
    std::fs::remove_dir_all(&folder).expect("cleaned");
}

#[test]
fn the_head_reads_how_often_each_reading_is_spelled_in_capitals_where_it_stands() {
    let tables = tables();
    let typed_russian = head(&evidence("ЛЯЕ", "KZT", "курс "), &tables);
    assert!(typed_russian.contains("iso:1:lexicon:0:before:ru:direction:1"), "{typed_russian:?}");
    assert!(typed_russian.contains("before:ru:direction:1"));
    assert!(typed_russian.contains("ratio:3:before:ru:direction:1"), "{typed_russian:?}");
    let typed_latin = head(&evidence("CCCH", "СССР", ""), &tables);
    assert!(typed_latin.contains("iso:0:lexicon:1:before:none:direction:0"), "{typed_latin:?}");
    assert!(typed_latin.contains("before:none:direction:0"));
    assert!(typed_latin.contains("ratio:-8:before:none:direction:0"), "{typed_latin:?}");
    assert!(head(&evidence("ЫЫР", "SSH", ""), &tables).contains("non_initial:1:before:none:direction:1"));
    let caps_lock = head(&evidence("YTN", "НЕТ", "я "), &tables);
    assert!(caps_lock.iter().any(|name| name.starts_with("prose:") && !name.starts_with("prose:0")));
    assert!(caps_lock.contains("caseless:8:before:ru:direction:0"), "{caps_lock:?}");
    // Outside the class the head says nothing.
    assert!(head(&evidence("usd", "гыв", ""), &tables).is_empty());
    assert!(head(&evidence("LGTM", "ДПЕЬ", ""), &tables).is_empty());
}

#[test]
fn the_head_answers_beside_a_next_word_kept_as_typed() {
    let tables = tables();
    let mut item = evidence("ФШ", "AI", "");
    "курс".clone_into(&mut item.field.after);
    item.after_origin = AfterOrigin::KeptNextWord;
    let features = extract_action_features(&item, &tables).expect("features");
    assert!(features.keys().any(|name| name == &format!("{UPPER_FEATURE_PREFIX}after:kept_next_word:before:none:direction:1")));
    assert!(features.keys().all(|name| name.starts_with(UPPER_FEATURE_PREFIX) || name.starts_with("kept|")));
    let mut lower = evidence("фш", "ai", "");
    "курс".clone_into(&mut lower.field.after);
    lower.after_origin = AfterOrigin::KeptNextWord;
    assert!(!extract_action_features(&lower, &tables).expect("features").keys().any(|name| name.starts_with(UPPER_FEATURE_PREFIX)));
}

#[test]
fn capitals_left_the_lone_word_and_short_message_heads() {
    let tables = tables();
    assert!(!alone_question("ЦА", "WF", "", "", "pause"));
    assert!(alone_question("ца", "wf", "", "", "pause"));
    assert!(!brief_question("BYN", "", ""));
    assert!(brief_question("byn", "", ""));
    assert!(!known_reply_question("JR", "ОК", "", "", &tables));
    assert!(known_reply_question("jr", "ок", "", "", &tables));
}
