//! A frozen split read as the trainer reads it: what its manifest pins, each row it refuses, and the edits a word is given.
mod support;

use ks_train::corpus::{load_rows, load_split, typo_variants};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use support::{row, scratch, write_split};

fn split_of(name: &str, rows: &[Value]) -> (PathBuf, Value) {
    let folder = scratch(name);
    let mut manifest = json!({"splits": {}});
    write_split(&folder, "train", rows, &mut manifest);
    (folder, manifest)
}

fn read(folder: &Path, manifest: &Value) -> Result<usize, String> {
    std::fs::write(folder.join("manifest.json"), serde_json::to_vec(manifest).expect("json")).expect("a manifest");
    load_rows(folder, "train").map(|rows| rows.len())
}

#[test]
fn a_split_is_read_only_as_its_manifest_pins_it() {
    let good = [row("train", 0, 1, &["дом", "сад"], 0), row("train", 1, 0, &["word", "house"], 1)];
    let (folder, manifest) = split_of("corpus-pins", &good);
    assert_eq!(read(&folder, &manifest), Ok(2));
    // A row is written back as its canonical line: keys sorted, no spaces.
    let line = String::from_utf8(load_rows(&folder, "train").expect("the rows")[0].line()).expect("text");
    assert!(line.starts_with("{\"after\":") && line.ends_with("}\n") && line.contains("\"original\":\"дом\""), "{line}");
    assert_eq!(load_split(&folder, "train").map(|rows| rows.len()), Ok(2));
    assert!(load_split(&folder, "test").expect_err("no fitting split").contains("only a fitting split"));
    let refused = |change: &dyn Fn(&mut Value), words: &str| {
        let mut other = manifest.clone();
        change(&mut other);
        let said = read(&folder, &other).expect_err(words);
        assert!(said.contains(words), "{words}: {said}");
    };
    refused(&|value| value["splits"]["train"]["path"] = json!("other.jsonl.gz"), "unexpected split filename");
    refused(&|value| value["splits"]["train"]["sha256"] = json!("0"), "compressed checksum mismatch");
    refused(&|value| value["splits"]["train"]["rows"] = json!(3), "row count mismatch");
    refused(&|value| value["splits"]["train"]["content_sha256"] = json!("0"), "row count mismatch");
}

#[test]
fn a_row_out_of_its_bounds_or_its_split_stops_the_read() {
    let plain = || row("train", 0, 1, &["дом", "сад"], 0);
    let mut before = plain();
    before["before"] = json!("слово ".repeat(20));
    let mut after = plain();
    after["after"] = json!("слово ".repeat(12));
    let mut other = plain();
    other["split"] = json!("development");
    for (index, made) in [before, after, other].into_iter().enumerate() {
        let (folder, manifest) = split_of(&format!("corpus-bounds-{index}"), &[made]);
        let said = read(&folder, &manifest).expect_err("out of bounds");
        assert!(said.contains("invalid corpus row bounds or split"), "{said}");
    }
    let mut unnamed = plain();
    unnamed.as_object_mut().expect("an object").remove("lemma");
    let mut unflagged = plain();
    unflagged["layout_representable"] = json!("yes");
    for (index, (made, words)) in [(unnamed, "corpus row without lemma"), (unflagged, "without layout_representable")].into_iter().enumerate() {
        let (folder, manifest) = split_of(&format!("corpus-fields-{index}"), &[made]);
        let said = read(&folder, &manifest).expect_err(words);
        assert!(said.contains(words), "{words}: {said}");
    }
}

#[test]
fn an_edit_that_leaves_a_word_as_it_was_is_no_edit() {
    // Two equal letters swapped are the word itself.
    assert_eq!(typo_variants("мммм", "id"), ["ммм", "ммммм"]);
    assert_eq!(typo_variants("слово", "id").len(), 3);
}
