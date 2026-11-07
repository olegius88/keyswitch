//! Questions captured while letters taken out of a word are typed back into it after a click, in the other layout.
mod support;

use flate2::Compression;
use flate2::write::GzEncoder;
use ks_train::mixed::capture;
use serde_json::{Value, json};
use std::io::Write as _;
use support::{scratch, tree};

const TEXTS: [&str; 8] = [
    "привет как дела сегодня",
    "открой файл и поправь порт",
    "почему сервер возвращает ошибку",
    "запусти тесты перед слиянием",
    "please restart the server",
    "книга лежит на столе",
    "мы пошли домой вечером",
    "напиши письмо завтра утром",
];

fn gz(lines: &[String]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    for line in lines {
        encoder.write_all(line.as_bytes()).expect("in memory");
        encoder.write_all(b"\n").expect("in memory");
    }
    encoder.finish().expect("in memory")
}

#[test]
fn a_capture_of_edits_keeps_the_questions_asked_inside_a_word() {
    let folder = scratch("capture-edit");
    let lines: Vec<String> = TEXTS.iter().enumerate().map(|(index, text)| json!({"kind": "comment", "text": text, "mix": "mixed", "qid": index}).to_string()).collect();
    std::fs::write(folder.join("messages.jsonl.gz"), gz(&lines)).expect("a file");
    let recipe = json!({"seed": "edit", "kinds": ["comment"], "count": TEXTS.len(), "russian_share": 0.0, "messages": "messages.jsonl.gz", "edit": true});
    let manifest = folder.join("manifest.json");
    std::fs::write(&manifest, serde_json::to_vec(&json!({"schema_version": 2, "sources": [{"file": "edit.jsonl.xz", "recipe": recipe}]})).expect("json")).expect("a manifest");
    let root = tree();
    let results = capture(&root.join("resources"), &manifest, &folder, &root.join("resources/models/context_policy_v1.json"), None, false, 1).expect("captured");
    let rows = results["edit.jsonl.xz"]["rows"].as_i64().expect("a count");
    assert!(rows > 0, "{results}");
    let written: Value = serde_json::from_slice(&std::fs::read(&manifest).expect("the manifest")).expect("json");
    assert_eq!(written["sources"][0]["rows"].as_i64(), Some(rows));
}
