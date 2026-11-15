//! The common-reading head: its class (the rest of the short-token head's shape), where a token stands on its line, and
//! the evidence it reads there: the counts of the short-token table's two sections and how Russian the word before reads.

use ks_backend::{FieldContext, FieldRole};
use ks_context::constants::{COMMON_FEATURE_PREFIX, SHORT_FEATURE_PREFIX};
use ks_context::context_action_features::{LinePlace, common_question, common_shape, extract_action_features, line_place, short_question};
use ks_context::context_model::{AfterOrigin, BeforeOrigin, ContextEvidence, TermTables, SHORT_ALPHABETS, SHORT_FREQUENCY_FILE};
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
    "space".clone_into(&mut item.trigger);
    item.target_known = true;
    item
}

/// The head's features of a question, without their prefix.
fn head(item: &ContextEvidence, tables: &TermTables) -> BTreeSet<String> {
    let features = extract_action_features(item, tables).expect("features");
    features.keys().filter_map(|name| name.strip_prefix(COMMON_FEATURE_PREFIX)).map(str::to_owned).collect()
}

#[test]
fn the_table_counts_what_follows_a_cyrillic_word() {
    assert_eq!(SHORT_ALPHABETS, ["after_cyrillic", "cyrillic"]);
    let tables = tables();
    let short = tables.short().expect("the short-token table");
    let after = short.alphabet("after_cyrillic");
    // `ру` follows a Russian word far more often than `he` does; `if` far more often than `ша`.
    assert!(after.count("ру") > 16 * after.count("he"));
    assert!(after.count("if") > 1000 * after.count("ша").max(1));
    assert_eq!(after.count("длинное"), 0);
    // The short-token head's own counts are where they were.
    assert_eq!(short.alphabet("cyrillic").count("ру"), 2842);
}

#[test]
fn the_class_is_the_rest_of_the_short_token_shape() {
    let tables = tables();
    // Both readings common, a letter before it: after a Russian word, after Latin text, first on a line.
    for (original, before) in [("ру", "сайт майл "), ("ща", "у нас "), ("ру", "and then "), ("Ру", "Вот что он написал.\n")] {
        assert_eq!(common_question(original, before, "space", &tables), Ok(true), "{original} after {before:?}");
        assert_eq!(common_shape(original, before, "space", &tables), Ok(true), "{original} after {before:?}");
    }
    // Of the shape, but the Latin reading follows a Russian word as often or more often: the frozen model's.
    for (original, before) in [("ин", "and then "), ("сс", "запусти ")] {
        assert_eq!(common_shape(original, before, "space", &tables), Ok(true), "{original}");
        assert_eq!(common_question(original, before, "space", &tables), Ok(false), "{original}");
    }
    // Nothing before it; capitals; a Latin token; a Latin reading that is no word (`ё`); too long or too short; Russian text
    // that hardly uses it (`гы`); a Latin reading too rare (the short-token head's `кз`); a term (`сш`).
    for (original, before) in [("ру", ""), ("ру", "  "), ("РУ", "сайт "), ("he", "сайт "), ("ёж", "сайт "), ("руку", "сайт "), ("р", "сайт "), ("гы", "он такой "), ("кз", "говорит он "), ("сш", "запусти ")] {
        assert_eq!(common_question(original, before, "space", &tables), Ok(false), "{original} after {before:?}");
    }
    // At a sign as at a space; at a pause or Enter the frozen model decides.
    assert_eq!(common_question("ру", "сайт майл ", "punctuation", &tables), Ok(true));
    for trigger in ["pause", "enter", "tab"] {
        assert_eq!(common_question("ру", "сайт майл ", trigger, &tables), Ok(false), "{trigger}");
    }
    // The two classes do not meet.
    assert_eq!(short_question("кз", "говорит он ", &tables), Ok(true));
    assert_eq!(short_question("ру", "сайт майл ", &tables), Ok(false));
    // A table that does not load is no answer.
    let folder = std::env::temp_dir().join(format!("ks-common-head-{}", std::process::id()));
    std::fs::create_dir_all(&folder).expect("a folder");
    std::fs::copy(models().join("context-term-frequency.json"), folder.join("context-term-frequency.json")).expect("the term table");
    let without = TermTables::load(&folder).expect("the term table loads");
    assert!(common_question("ру", "сайт майл ", "space", &without).is_err());
    std::fs::write(folder.join(SHORT_FREQUENCY_FILE), "{\"cyrillic\": {\"ру\": 2842}}").expect("a table of one section");
    let one_section = TermTables::load(&folder).expect("the term table loads");
    assert!(common_question("ру", "сайт майл ", "space", &one_section).is_err());
    assert!(extract_action_features(&evidence("ру", "he", "сайт майл "), &one_section).is_err());
    std::fs::remove_dir_all(&folder).expect("cleaned");
}

#[test]
fn a_token_stands_somewhere_on_its_line() {
    assert_eq!(line_place("сайт майл "), (LinePlace::Cyrillic, "майл"));
    assert_eq!(line_place("купил 5 "), (LinePlace::Cyrillic, "купил"));
    assert_eq!(line_place("Сегодня тепло.\nand then "), (LinePlace::Latin, "then"));
    assert_eq!(line_place("Сегодня тепло.\n"), (LinePlace::Line, ""));
    assert_eq!(line_place("Сегодня тепло.\n12 "), (LinePlace::Line, ""));
    assert_eq!(line_place("testтест "), (LinePlace::Mixed, "testтест"));
    let names: Vec<&str> = [LinePlace::Line, LinePlace::Cyrillic, LinePlace::Latin, LinePlace::Mixed].iter().map(|place| place.as_str()).collect();
    assert_eq!(names, ["line", "cyrillic", "latin", "mixed"]);
}

#[test]
fn the_head_reads_where_the_token_stands_and_what_follows_a_russian_word() {
    let tables = tables();
    // After a Russian word: `ру` follows one 276 times and `he` 15 (binary orders 9 and 4), `майл` reads Russian (33 times
    // in Russian prose, its keys never in English: order 6).
    let russian = head(&evidence("ру", "he", "сайт майл "), &tables);
    for name in ["place:cyrillic", "follows:5:place:cyrillic", "follows:cyrillic:9:place:cyrillic", "follows:latin:4:place:cyrillic", "previous:6:place:cyrillic", "follows:ru:previous:ru:place:cyrillic", "ratio:-4:place:cyrillic", "known:1:identifier:0:place:cyrillic", "boundary:space:place:cyrillic", "after:none:place:cyrillic", "letters:2:place:cyrillic"] {
        assert!(russian.contains(name), "{name}: {russian:?}");
    }
    assert!(!russian.contains("bias"), "no weight is shared by every place: {russian:?}");
    // After English text: the word before reads English.
    let english = head(&evidence("ру", "he", "and then "), &tables);
    assert!(english.contains("previous:-8:place:latin"), "{english:?}");
    assert!(english.contains("follows:ru:previous:en:place:latin"), "{english:?}");
    // After the keys of an English word typed in the Russian layout (`шы`, `is`).
    let keys = head(&evidence("ру", "he", "Вот.\nшы "), &tables);
    assert!(keys.iter().any(|name| name.starts_with("previous:-") && name.ends_with(":place:cyrillic")), "{keys:?}");
    // First on a line under a Russian line, and after a word of both scripts: no word before to read.
    let opening = head(&evidence("Ру", "He", "Вот что он написал.\n"), &tables);
    assert!(opening.contains("previous:na:place:line") && opening.contains("follows:ru:previous:na:place:line"), "{opening:?}");
    let mixed = head(&evidence("ру", "he", "testтест "), &tables);
    assert!(mixed.contains("previous:na:place:mixed"), "{mixed:?}");
    // At a sign, not at a pause, and asked again with the next word planned for conversion.
    let mut paused = evidence("ру", "he", "сайт майл ");
    "punctuation".clone_into(&mut paused.trigger);
    assert!(head(&paused, &tables).contains("boundary:end:place:cyrillic"));
    "pause".clone_into(&mut paused.trigger);
    assert!(head(&paused, &tables).is_empty());
    let mut planned = evidence("ру", "he", "сайт майл ");
    planned.after_origin = AfterOrigin::PlannedNextConversion;
    "is".clone_into(&mut planned.field.after);
    let planned = head(&planned, &tables);
    assert!(planned.contains("follows:ru:previous:ru:after:planned_next_conversion:place:cyrillic"), "{planned:?}");
}

#[test]
fn the_head_leaves_alone_what_it_does_not_answer() {
    let tables = tables();
    // A word the lexicon knows, the short-token head's class, a question about a hypothesis, and one beside a kept word.
    let mut known = evidence("ру", "he", "сайт майл ");
    known.source_known = true;
    assert!(head(&known, &tables).is_empty());
    let short = evidence("кз", "rp", "говорит он ");
    assert!(head(&short, &tables).is_empty());
    assert!(extract_action_features(&short, &tables).expect("features").keys().any(|name| name.starts_with(SHORT_FEATURE_PREFIX)));
    let mut hypothesis = evidence("ру", "he", "tot ");
    hypothesis.before_origin = BeforeOrigin::PlannedPreviousReading;
    assert!(head(&hypothesis, &tables).is_empty());
    let mut kept = evidence("ру", "he", "сайт майл ");
    "работает".clone_into(&mut kept.field.after);
    kept.after_origin = AfterOrigin::KeptNextWord;
    assert!(head(&kept, &tables).is_empty());
    assert!(head(&evidence("ру", "he", ""), &tables).is_empty());
}
