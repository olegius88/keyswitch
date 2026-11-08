//! What a row of a corpus gives the fit: the frames of each kind, and those rows a selection leaves out.
#![expect(clippy::disallowed_types, clippy::disallowed_macros, clippy::disallowed_methods, reason = "temporary: these tests still build and read their JSON as untyped trees; step 7 of the strict-json plan types them and removes this")]
mod support;

use ks_lang::Resources;
use ks_train::corpus::{CorpusRow, load_rows};
use ks_train::physical::reading;
use ks_train::rows::{action_rows, capital_citation, cyrillic, curated_letter, select_rows};
use ks_train::tools::{Tools, reference_models};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::{Arc, LazyLock};
use support::{row, scratch, tree, write_split};

static TOOLS: LazyLock<Tools> = LazyLock::new(|| {
    let root = tree();
    let resources: Arc<Resources> = Resources::new(root.join("resources"));
    let portable = reference_models(&root, &resources, false).expect("the lexicons");
    Tools::new(&root, resources, portable, BTreeSet::new()).expect("the tools")
});

fn loaded(name: &str, made: &[Value]) -> Vec<CorpusRow> {
    let folder = scratch(name);
    let mut manifest = json!({"splits": {}});
    write_split(&folder, "train", made, &mut manifest);
    std::fs::write(folder.join("manifest.json"), serde_json::to_vec(&manifest).expect("json")).expect("a manifest");
    load_rows(&folder, "train").expect("the rows")
}

/// An abbreviation Russian technical text uses, whose letters are letters in the other layout too.
fn abbreviation() -> String {
    TOOLS.tables.term("cyrillic").words().find(|word| word.chars().count() == 3 && word.chars().all(|letter| "йцукенгшщзфывапролдячсмить".contains(letter))).map(ks_text::pystr::upper).expect("an abbreviation")
}

/// Words of both languages in each shape `action_rows` tells apart, each after a text and at the start of a field.
fn every_shape() -> Vec<Value> {
    let keys = reading(&abbreviation(), 1);
    let english: Vec<&str> = vec!["word", "house", "House", "HTTP", "ZIP", "RTX", "the", "on", "a", "I", "z", "qzxv", "Qzxv", "123", &keys, "ghbdtn", "ujl", "don't", "lfkmit"];
    let russian = ["слово", "дом", "Дом", "ДОМ", "на", "я", "а", "и", "ъыъ", "кто-то", "бюро", "ьщтпщ"];
    let mut made = Vec::new();
    let mut index = 0;
    let lines: [(i64, &[&str]); 2] = [(0, &english), (1, &russian)];
    for (group, words) in lines {
        let lefts: [&[&str]; 6] = if group == 0 {
            [&[], &["the", "house", "stands"], &["don't"], &["lfkmit"], &["on"], &["house", "("]]
        } else {
            [&[], &["это", "дом", "стоит"], &["бюро"], &["дальше"], &["на"], &["дом", "("]]
        };
        for word in words {
            for left in lefts {
                let sentence: Vec<&str> = left.iter().copied().chain([*word, "сад"]).collect();
                let at = left.len();
                // Each shape under several names: a name decides the trigger of its frames.
                for copy in 0..4 {
                    let mut value = row("train", index, group, &sentence, at);
                    value["after"] = json!(if copy % 2 == 0 { "" } else { " сад" });
                    value["spacing"] = json!(if copy == 3 { "" } else { " " });
                    value["literal_tail"] = json!(if copy == 2 { "," } else { "" });
                    made.push(value);
                    index += 1;
                }
            }
        }
    }
    made
}

#[test]
fn each_kind_of_frame_comes_from_the_rows_it_is_for() {
    let rows = loaded("rows-shapes", &every_shape());
    let frames = action_rows(&TOOLS, &rows);
    let named = |part: &str| frames.iter().filter(|frame| frame.identifier.contains(part)).count();
    for part in [":observed:keep", ":empty:wrong", ":mixed", ":mixed:wrong", ":mixed-natural", ":mixed-natural:wrong", ":mixed-natural:head", ":stranded-previous", ":opening", ":quote-tail"] {
        assert!(named(part) > 0, "no frame {part}");
    }
    // The keys of an abbreviation stand after Russian prose: no frame says they are wrong there.
    let keys = reading(&abbreviation(), 1);
    assert!(frames.iter().any(|frame| frame.original == keys && frame.identifier.ends_with(":mixed")));
    assert!(!frames.iter().any(|frame| frame.original == keys && frame.identifier.ends_with(":mixed:wrong")));
    // With no text of the other language to stand in.
    let english_only: Vec<CorpusRow> = rows.iter().filter(|row| row.group == Some(0)).cloned().collect();
    assert!(!action_rows(&TOOLS, &english_only).iter().any(|frame| frame.identifier.contains(":mixed-natural")));
    // A Latin abbreviation in capitals whose other reading is a rare word.
    let capital = ks_train::curricula::capital_citation_curriculum(&TOOLS, &rows, &{
        let mut wanted = support::recipe()["capital_citation_curriculum"].clone();
        wanted["words_by_length"] = json!({"3": 8, "4": 8});
        wanted
    }, &TOOLS.portable);
    let original = capital.0.iter().find(|frame| frame.group == 0 && frame.original.chars().all(|letter| letter.is_ascii_uppercase())).expect("a capital citation").original.clone();
    let mut with_capital = every_shape();
    with_capital.push(row("train", 900_000, 0, &["the", &original, "сад"], 1));
    with_capital.push(row("train", 900_001, 0, &[&original, "сад"], 0));
    let capitals = action_rows(&TOOLS, &loaded("rows-capital", &with_capital));
    assert!(capitals.iter().any(|frame| frame.identifier.contains(":mixed-capital")));
}

#[test]
fn a_capital_citation_is_latin_capitals_whose_other_reading_is_a_rare_word() {
    for (original, alternate) in [("ÄB", "дом"), ("A1", "дом"), ("Ab", "дом"), ("ABCDEFGHIJKLMNOPQRSTUV", "дом"), ("QZX", "ййй")] {
        assert!(!capital_citation(&TOOLS, original, alternate), "{original}");
    }
}

#[test]
fn a_curated_letter_is_one_letter_that_is_a_word_in_one_layout() {
    assert!(curated_letter("z", "я"));
    assert!(curated_letter("я", "z"));
    assert!(!curated_letter("q", "й"));
    assert!(!curated_letter("zz", "я"));
    assert!(!curated_letter("z", "яя"));
    assert!(!cyrillic("ñ"));
    assert!(!cyrillic("я1"));
    assert!(cyrillic("ЁЖ"));
}

#[test]
fn a_selection_takes_a_share_of_each_language_and_of_each_family() {
    let mut made = Vec::new();
    for index in 0..40 {
        made.push(row("train", index, 1, &["дом", "сад"], 0));
        made.push(row("train", 100 + index, 0, &["word", "house"], 0));
    }
    let mut no_group = row("train", 500, 0, &["word"], 0);
    no_group["group"] = Value::Null;
    made.push(no_group);
    let mut other_group = row("train", 501, 0, &["word"], 0);
    other_group["group"] = json!(5);
    made.push(other_group);
    let mut unrepresentable = row("train", 502, 0, &["word"], 0);
    unrepresentable["layout_representable"] = json!(false);
    made.push(unrepresentable);
    made.push(row("train", 503, 0, &["ñandu"], 0));
    for value in &mut made[..80] {
        value["family"] = json!("one family");
    }
    let rows = loaded("rows-selection", &made);
    let all = select_rows(&rows, 10_000);
    assert!(!all.is_empty() && all.len() < 40, "{}", all.len());
    assert!(!all.iter().any(|row| matches!(row.identifier.as_str(), "train:500" | "train:501" | "train:502" | "train:503")));
    assert!(select_rows(&rows, 2).len() <= 2);
    assert!(select_rows(&rows, 0).is_empty());
}

#[test]
fn a_one_letter_word_before_is_no_phrase_typed_in_the_other_layout() {
    let rows = loaded("rows-letter", &[row("train", 0, 1, &["это", "я", "дом"], 2)]);
    let frames = action_rows(&TOOLS, &rows);
    assert!(!frames.is_empty());
    assert!(!frames.iter().any(|frame| frame.identifier.contains(":stranded-previous")));
}
