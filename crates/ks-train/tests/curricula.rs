//! The curricula of a fit one by one, with every setting a recipe can give them, on rows of a small corpus.
mod support;

use ks_context::context_model::ContextAction;
use ks_lang::Resources;
use ks_train::corpus::{CorpusRow, load_rows};
use ks_train::curricula::{self as c};
use ks_train::heads::{HEAD_PREFIXES, OWN_EPOCH_HEADS, own_curriculum};
use ks_train::rows::ActionRow;
use ks_train::tools::{Tools, reference_models};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::{Arc, LazyLock};
use support::{recipe, row, scratch, tree, write_split};

/// Rows of a small corpus in the shapes the curricula look for.
const RUSSIAN: &[&[&str]] = &[
    &["сегодня", "хорошая", "погода", "и", "дом", "стоит", "на", "горе"],
    &["он", "а", "мы", "в", "лесу", "гуляем", "весело", "вечером"],
    &["я", "пишу", "письмо", "с", "другом", "у", "моря", "о", "жизни"],
    &["Москва", "большой", "город", "к", "нему", "едут", "люди"],
    &["работа", "программа", "система", "решение", "задача", "результат", "время", "дорога"],
];
const ENGLISH: &[&[&str]] = &[
    &["the", "weather", "is", "good", "today", "and", "people", "are", "working"],
    &["I", "think", "a", "house", "stands", "on", "the", "hill", "before", "water"],
    &["she", "said", "that", "letters", "should", "come", "through", "here"],
];

fn rows() -> Vec<CorpusRow> {
    let folder = scratch("curricula-corpus");
    let mut made = Vec::new();
    let mut index = 0;
    for (group, sentences) in [(1, RUSSIAN), (0, ENGLISH)] {
        for sentence in sentences {
            for at in 0..sentence.len() {
                made.push(row("train", index, group, sentence, at));
                index += 1;
            }
        }
    }
    // Those a curriculum has no use for: no group, no representation, words of the other script.
    let mut odd = row("train", index, 1, &["word", "слово"], 0);
    odd["layout_representable"] = json!(false);
    made.push(odd);
    let mut no_group = row("train", index + 1, 0, &["слово", "дело"], 0);
    no_group["group"] = Value::Null;
    made.push(no_group);
    made.push(row("train", index + 2, 1, &["word", "слово"], 0));
    made.push(row("train", index + 3, 1, &["word", "слово"], 1));
    made.push(row("train", index + 4, 1, &["word,", "сло-во"], 1));
    let mut manifest = json!({"splits": {}});
    write_split(&folder, "train", &made, &mut manifest);
    std::fs::write(folder.join("manifest.json"), serde_json::to_vec(&manifest).expect("json")).expect("a manifest");
    load_rows(&folder, "train").expect("the rows")
}

static TOOLS: LazyLock<Tools> = LazyLock::new(|| {
    let root = tree();
    let resources: Arc<Resources> = Resources::new(root.join("resources"));
    let portable = reference_models(&root, &resources, false).expect("the lexicons");
    let refused: BTreeSet<String> = ["work", "ьфьф", "ctujlyz"].iter().map(|word| (*word).to_owned()).collect();
    Tools::new(&root, resources, portable, refused).expect("the tools")
});

static ROWS: LazyLock<Vec<CorpusRow>> = LazyLock::new(rows);

fn options(name: &str) -> Value {
    recipe()[name].clone()
}

fn reported(curriculum: &(Vec<ActionRow>, Value)) -> usize {
    assert!(curriculum.1.is_object(), "{}", curriculum.1);
    curriculum.0.len()
}

fn with(mut value: Value, changes: &[(&str, Value)]) -> Value {
    for (name, change) in changes {
        value[*name] = change.clone();
    }
    value
}

const SPLITS: [&str; 3] = ["train", "development", "calibration"];

#[test]
fn a_curriculum_set_to_nothing_or_given_nothing_gives_no_frames() {
    let (tools, rows) = (&*TOOLS, &*ROWS);
    let models = &tools.portable;
    let none: &[CorpusRow] = &[];
    assert_eq!(reported(&c::capital_citation_curriculum(tools, none, &options("capital_citation_curriculum"), models)), 0);
    assert_eq!(reported(&c::english_capital_curriculum(tools, rows, &with(options("english_capital_curriculum"), &[("frames_per_word", json!(0))]), models)), 0);
    assert_eq!(reported(&c::abbreviation_curriculum(tools, "train", rows, &with(options("abbreviation_curriculum"), &[("frames_per_form", json!(0))]))), 0);
    assert_eq!(reported(&c::latin_abbreviation_curriculum(tools, "train", rows, &with(options("latin_abbreviation_curriculum"), &[("frames_per_form", json!(0))]))), 0);
    assert_eq!(reported(&c::unknown_token_curriculum(tools, "train", rows, &with(options("unknown_token_curriculum"), &[("item_modulus", json!(0))]))), 0);
    assert_eq!(reported(&c::joined_token_curriculum(tools, "train", rows, &with(options("joined_token_curriculum"), &[("item_modulus", json!(0))]))), 0);
    assert_eq!(reported(&c::term_curriculum(tools, "train", rows, &with(options("term_curriculum"), &[("item_modulus", json!(0))]))), 0);
    assert_eq!(reported(&c::first_curriculum(tools, "train", &with(options("first_curriculum"), &[("enabled", json!(false))]))), 0);
    assert_eq!(reported(&c::first_curriculum(tools, "train", &json!({}))), 0);
    assert_eq!(reported(&c::brief_curriculum(tools, "train", &with(options("brief_curriculum"), &[("minimum_count", json!(0))]))), 0);
    assert_eq!(reported(&c::kept_neighbour_curriculum(tools, rows, &with(options("kept_neighbour_curriculum"), &[("natural_frames", json!(0)), ("term_frames", json!(0)), ("abbreviation_frames", json!(0)), ("typo_frames", json!(0))]), models)), 0);
    assert_eq!(reported(&c::counted_token_curriculum(tools, none, &with(options("counted_token_curriculum"), &[("term_frames", json!(5))]))), 0);
    assert_eq!(reported(&c::counted_token_curriculum(tools, rows, &with(options("counted_token_curriculum"), &[("term_frames", json!(0)), ("abbreviation_frames", json!(0))]))), 0);
    assert_eq!(reported(&c::letter_curriculum(tools, rows, &with(options("letter_curriculum"), &[("term_sentences", json!(0)), ("english_sentences", json!(0))]))), 0);
    for (name, runs) in [
        ("calibration", c::abbreviation_curriculum(tools, "calibration", rows, &options("abbreviation_curriculum"))),
        ("calibration", c::latin_abbreviation_curriculum(tools, "calibration", rows, &options("latin_abbreviation_curriculum"))),
        ("calibration", c::unknown_token_curriculum(tools, "calibration", rows, &options("unknown_token_curriculum"))),
        ("calibration", c::joined_token_curriculum(tools, "calibration", rows, &options("joined_token_curriculum"))),
        ("calibration", c::term_curriculum(tools, "calibration", rows, &options("term_curriculum"))),
        ("calibration", c::first_curriculum(tools, "calibration", &options("first_curriculum"))),
        ("calibration", c::brief_curriculum(tools, "calibration", &options("brief_curriculum"))),
        ("no contexts", c::abbreviation_curriculum(tools, "train", none, &options("abbreviation_curriculum"))),
        ("no contexts", c::latin_abbreviation_curriculum(tools, "train", none, &options("latin_abbreviation_curriculum"))),
        ("no contexts", c::unknown_token_curriculum(tools, "train", none, &options("unknown_token_curriculum"))),
        ("no contexts", c::term_curriculum(tools, "train", none, &options("term_curriculum"))),
    ] {
        assert_eq!(reported(&runs), 0, "{name}");
    }
}

#[test]
fn each_curriculum_makes_its_frames_in_each_split() {
    let (tools, rows) = (&*TOOLS, &*ROWS);
    let models = &tools.portable;
    assert!(reported(&c::capital_citation_curriculum(tools, rows, &with(options("capital_citation_curriculum"), &[("words_by_length", json!({"3": 5, "4": 5, "5": 2}))]), models)) > 0);
    for (contrast, russian) in [(true, true), (false, false)] {
        let english = with(options("english_capital_curriculum"), &[("words_by_length", json!({"2": 2, "3": 4, "4": 3, "5": 1})), ("lowercase_contrast", json!(contrast)), ("russian_contrast", json!(russian))]);
        assert!(reported(&c::english_capital_curriculum(tools, rows, &english, models)) > 0);
    }
    for split in &SPLITS[..2] {
        assert!(reported(&c::abbreviation_curriculum(tools, split, rows, &options("abbreviation_curriculum"))) > 0, "{split}");
        assert!(reported(&c::latin_abbreviation_curriculum(tools, split, rows, &options("latin_abbreviation_curriculum"))) > 0, "{split}");
        let mut unknown = options("unknown_token_curriculum");
        unknown["item_modulus"] = json!(1);
        unknown["echo_modulus"] = json!(1);
        let _ = reported(&c::unknown_token_curriculum(tools, split, rows, &unknown));
        let mut joined = options("joined_token_curriculum");
        joined["repeat_words"] = json!(30);
        let _ = reported(&c::joined_token_curriculum(tools, split, rows, &joined));
        let mut term = options("term_curriculum");
        term["item_modulus"] = json!(1);
        let _ = reported(&c::term_curriculum(tools, split, rows, &term));
        let _ = reported(&c::first_curriculum(tools, split, &options("first_curriculum")));
        let _ = reported(&c::brief_curriculum(tools, split, &options("brief_curriculum")));
    }
    for capitals in [false, true] {
        let counted = with(options("counted_token_curriculum"), &[("term_frames", json!(8)), ("abbreviation_frames", json!(8)), ("capitals", json!(capitals))]);
        assert!(reported(&c::counted_token_curriculum(tools, rows, &counted)) > 0);
    }
    let letter = with(options("letter_curriculum"), &[("term_sentences", json!(30)), ("english_sentences", json!(30))]);
    assert!(reported(&c::letter_curriculum(tools, rows, &letter)) > 0);
    let kept = with(options("kept_neighbour_curriculum"), &[("natural_frames", json!(40)), ("term_frames", json!(30)), ("abbreviation_frames", json!(20)), ("typo_frames", json!(20))]);
    assert!(reported(&c::kept_neighbour_curriculum(tools, rows, &kept, models)) > 0);
}

#[test]
fn a_lone_word_is_labelled_by_its_counts_or_by_what_was_asked() {
    let tools = &*TOOLS;
    let evidence = json!({"minimum_count": 5, "other_maximum_count": 1, "dominance": 3.0, "minimum_ratio": 3.0, "sample_weight": 1.0});
    let plain = c::lone_word_curriculum(tools, "train", &evidence, None);
    assert!(reported(&plain) > 0);
    let decide = |row: &ActionRow| -> Option<ContextAction> { if row.original.starts_with('z') { Some(ContextAction::Keep) } else { None } };
    let asked = c::lone_word_curriculum(tools, "development", &evidence, Some(&decide));
    assert!(reported(&asked) > 0);
    let same = c::alone_labels(tools, plain.0.clone(), None, None);
    assert_eq!(same.len(), plain.0.len());
    assert!(c::curated("z", "я"));
    assert!(!c::curated("ghbdtn", "привет"));
}

#[test]
fn a_head_finds_its_curriculum_in_a_recipe() {
    let (tools, rows) = (&*TOOLS, &*ROWS);
    let recipe = recipe();
    let mut labels: Vec<&str> = OWN_EPOCH_HEADS.iter().map(|head| head.1).collect();
    labels.extend(HEAD_PREFIXES.iter().map(|head| head.0));
    labels.extend(["joined", "term", "brief", "first", "variable", "dotted", "alnum", "hypothesis", "short", "slip", "sign", "resume", "cyrletter", "dash", "digitdot", "quote", "knownreply"]);
    for label in labels {
        for split in SPLITS {
            let _ = own_curriculum(tools, label, split, rows, &recipe);
        }
    }
    // A head no recipe knows, and one this recipe has no part for.
    assert_eq!(reported(&own_curriculum(tools, "nothing", "train", rows, &json!({})).expect("absent")), 0);
    assert!(own_curriculum(tools, "nothing", "train", rows, &json!({"nothing": {}})).unwrap_err().contains("no curriculum"));
}
