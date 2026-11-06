//! A small corpus and a small recipe, of plain words of both languages: the whole of a training run in a minute or two.
#![allow(dead_code, reason = "each test program uses what it needs of this")]
use flate2::Compression;
use flate2::write::GzEncoder;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) fn tree() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("the repository")
}

pub(crate) fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!("ks-train-{name}-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&path).expect("a folder");
    path
}

/// The trainer, with the reference dictionaries of the repository.
pub(crate) fn trainer() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ks-train"));
    command.env("KEYSWITCH_HUNSPELL_PATH", tree().join("model/intent_v1/sources/hunspell"));
    command
}

/// What a command said, when it was to succeed.
pub(crate) fn succeeded(output: &Output) -> String {
    assert!(output.status.success(), "{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// A command that was to stop, and with these words.
pub(crate) fn refused(output: &Output, words: &str) {
    let said = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success() && said.contains(words), "{words}: {said}");
}

/// The files a cycle writes or reads as its own; everything else of a small tree is the repository's, through links.
const OWN_FILES: [&str; 37] = [
    "model/ortho_v1/web-lexicon-ru.json.gz",
    "model/ortho_v1/tokens.jsonl.gz",
    "model/ortho_v1/corpus-receipt.json",
    "model/ortho_v1/config.json",
    "model/ortho_v1/known-receipt.json",
    "model/ortho_v1/web-lexicon-receipt.json",
    "model/ortho_v1/prose-lexicon-receipt.json",
    "model/ortho_v1/prose-lexicon-ru.json.gz",
    "resources/models/boundary-v2.json",
    "resources/models/ortho_v1.json",
    "model/boundary_v1/engine-regression.json",
    "model/prefix_v1/engine-report.json",
    "model/archive-sha256.json",
    "packaging/third-party-licenses/linux.txt",
    "packaging/third-party-licenses/macos.txt",
    "packaging/third-party-licenses/windows.txt",
    "model/prefix_v1/corpus.json",
    "model/prefix_v1/config.json",
    "packaging/keyswitch-archive-keyring.asc",
    "Cargo.toml",
    "packaging/debian/keyswitch.sources",
    "resources/typo-frequency.receipt.json",
    "resources/typo-frequency-ru_RU.bin",
    "resources/typo-frequency-en_US.bin",
    "model/intent_v1/config.json",
    "model/intent_v1/manifest.json",
    "model/intent_v1/test-report.json",
    "model/intent_v1/build-environment.json",
    "resources/models/layout_intent_v1.ksm",
    "model/context_v3/devchat-test-batches.json",
    "model/prefix_v2/recipe.json",
    "model/context_v3/recipe.json",
    "model/context_v3/candidate-seal.json",
    "model/context_v3/release-receipt.json",
    "model/prefix_v2/candidate-seal.json",
    "resources/models/context_policy_v1.json",
    "resources/models/prefix_policy_v1.json",
];

/// Folders a cycle writes into: empty in a small tree, never links.
const OWN_FOLDERS: [&str; 2] = ["model/boundary_v1/engine-history", "model/prefix_v1/engine-history"];
/// Folders a cycle makes itself: not in a small tree at all.
const LEFT_OUT: [&str; 1] = ["model/ortho_v1/independent"];

fn mirror(source: &Path, target: &Path, relative: &str) {
    std::fs::create_dir_all(target).expect("a folder");
    for entry in std::fs::read_dir(source).expect("a folder of the repository") {
        let entry = entry.expect("an entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        if matches!(name.as_str(), ".git" | ".t" | "build" | "target" | "dist") {
            continue;
        }
        let path = if relative.is_empty() { name.clone() } else { format!("{relative}/{name}") };
        if LEFT_OUT.contains(&path.as_str()) {
            continue;
        }
        if OWN_FILES.contains(&path.as_str()) {
            std::fs::copy(entry.path(), target.join(&name)).expect("a copy");
        } else if OWN_FOLDERS.contains(&path.as_str()) {
            std::fs::create_dir_all(target.join(&name)).expect("a folder");
        } else if OWN_FILES.iter().any(|file| file.starts_with(&format!("{path}/"))) {
            mirror(&entry.path(), &target.join(&name), &path);
        } else {
            std::os::unix::fs::symlink(entry.path(), target.join(&name)).expect("a link");
        }
    }
}

/// A tree whose recipe is this one: a seal fitted over it holds against it.
pub(crate) fn small_tree(name: &str, options: &Value) -> PathBuf {
    let folder = scratch(name);
    mirror(&tree(), &folder, "");
    write_recipe(&folder.join("model/context_v3/recipe.json"), options);
    folder
}

fn gz(lines: &[String]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    for line in lines {
        encoder.write_all(line.as_bytes()).expect("in memory");
        encoder.write_all(b"\n").expect("in memory");
    }
    encoder.finish().expect("in memory")
}

/// The message files of a chat test, for both splits, each message a member of its own; the small tree pins them.
pub(crate) fn chat_messages(small: &Path, directory: &Path, texts: &[&str], test_count: u64) {
    std::fs::create_dir_all(directory).expect("a folder");
    let messages: Vec<String> = texts.iter().map(|text| json!({"text": text}).to_string()).collect();
    let members: Vec<String> = texts
        .iter()
        .enumerate()
        .map(|(index, text)| json!({"text": digest(text.as_bytes()), "question": format!("question{index}"), "mix": if text.is_ascii() { "russian" } else { "mixed" }}).to_string())
        .collect();
    for split in ["dev", "test"] {
        std::fs::write(directory.join(format!("devchat-{split}.messages.jsonl.gz")), gz(&messages)).expect("a file");
        std::fs::write(directory.join(format!("devchat-{split}.members.jsonl.gz")), gz(&members)).expect("a file");
    }
    let pin = digest((messages.join("\n") + "\n").as_bytes());
    let batches = json!({"development_content_sha256": pin, "test_content_sha256": pin, "test_count": test_count, "batches": []});
    std::fs::write(small.join("model/context_v3/devchat-test-batches.json"), serde_json::to_vec(&batches).expect("json")).expect("a file");
}

pub(crate) fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

const RUSSIAN: &[&str] = &[
    "привет", "дела", "сегодня", "хорошая", "погода", "домой", "читаем", "новую", "книгу", "пишет", "длинное", "письмо", "делает", "работу", "этом", "городе", "много", "людей",
    "машин", "есть", "время", "отдыха", "кто-то", "сказал", "завтра", "будет", "дождь",
];
const ENGLISH: &[&str] = &[
    "hello", "today", "weather", "good", "read", "book", "writes", "long", "letters", "does", "work", "this", "town", "many", "people", "cars", "have", "time", "rest", "someone",
    "said", "that", "tomorrow", "will", "rain",
];

/// A small generator with a fixed start: the same corpus on every machine.
struct Dice(u64);

impl Dice {
    fn below(&mut self, limit: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        usize::try_from(self.0 % u64::try_from(limit).expect("small")).expect("small")
    }
}

/// One row of a split, as a frozen corpus holds it.
pub(crate) fn row(split: &str, index: usize, group: i64, words: &[&str], at: usize) -> Value {
    let before = if at == 0 { String::new() } else { format!("{} ", words[..at].join(" ")) };
    let after = if at + 1 < words.len() { format!(" {}", words[at + 1..].join(" ")) } else { String::new() };
    json!({
        "identifier": format!("{split}:{index}"), "original": words[at], "group": group, "before": before, "after": after, "family": format!("family{}", index % 37),
        "document": format!("document{}", index % 41), "language": if group == 0 { "en" } else { "ru" }, "source": "test", "source_file": "file", "source_sentence": index.to_string(),
        "source_token": at.to_string(), "spacing": " ", "space_before": " ", "literal_tail": "", "upos": "NOUN", "features": "", "misc": "", "alignment": "exact",
        "layout_representable": true, "split": split, "quarantine_reasons": [], "lemma": words[at],
    })
}

pub(crate) fn write_split(directory: &Path, split: &str, rows: &[Value], manifest: &mut Value) {
    let mut raw = Vec::new();
    for row in rows {
        raw.extend_from_slice(serde_json::to_string(row).expect("json").as_bytes());
        raw.push(b'\n');
    }
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(&raw).expect("in memory");
    let packed = encoder.finish().expect("in memory");
    std::fs::write(directory.join(format!("{split}.jsonl.gz")), &packed).expect("a split");
    manifest["splits"][split] = json!({"path": format!("{split}.jsonl.gz"), "sha256": digest(&packed), "rows": rows.len(), "content_sha256": digest(&raw)});
}

/// A corpus of the four splits.
pub(crate) fn corpus(directory: &Path) {
    corpus_of(directory, 1);
}

/// The same, each split so many times as large.
pub(crate) fn corpus_of(directory: &Path, scale: usize) {
    std::fs::create_dir_all(directory).expect("a folder");
    let mut dice = Dice(0x9e37_79b9_7f4a_7c15);
    let mut manifest = json!({"splits": {}});
    let mut members: [Vec<String>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for (split, size) in [("train", 400 * scale), ("development", 150 * scale), ("calibration", 150 * scale), ("test", 200 * scale)] {
        let mut rows = Vec::new();
        for index in 0..size {
            let group = i64::try_from(index % 2).expect("small");
            let source = if group == 0 { ENGLISH } else { RUSSIAN };
            let words: Vec<&str> = (0..4 + dice.below(6)).map(|_| source[dice.below(source.len())]).collect();
            let at = dice.below(words.len());
            let new = row(split, index, group, &words, at);
            if split == "test" {
                for (list, field) in members.iter_mut().zip(["identifier", "family", "document"]) {
                    list.push(digest(new[field].as_str().expect("text").as_bytes()));
                }
            }
            rows.push(new);
        }
        write_split(directory, split, &rows, &mut manifest);
    }
    for list in &mut members {
        list.sort();
        list.dedup();
    }
    let [row_ids, family_ids, document_ids] = members;
    let membership = serde_json::to_vec(&json!({"row_ids_sha256": row_ids, "family_ids_sha256": family_ids, "document_ids_sha256": document_ids, "namespace": "small"})).expect("json");
    std::fs::write(directory.join("test-membership.json"), &membership).expect("a file");
    manifest["test_membership_sha256"] = json!(digest(&membership));
    std::fs::write(directory.join("manifest.json"), serde_json::to_vec(&manifest).expect("json")).expect("a file");
}

/// The recipe of the shipped model with its curricula cut down to a few frames each.
pub(crate) fn recipe() -> Value {
    let mut recipe: Value = serde_json::from_slice(&std::fs::read(tree().join("model/context_v3/recipe.json")).expect("the recipe")).expect("json");
    recipe["epochs"] = json!(2);
    recipe["maximum_source_rows"] = json!({"train": 400, "development": 150, "calibration": 150});
    recipe["maximum_features"] = json!(2000);
    recipe["span_maximum_families"] = json!({"train": 8, "development": 4, "calibration": 4});
    for (name, families, seeds) in [("lookahead_curriculum", 8, 4), ("natural_lookahead_curriculum", 8, 2), ("lexical_short_pair_curriculum", 8, 2)] {
        recipe[name]["maximum_families"] = json!(families);
        recipe[name]["seeds_per_family"] = json!(seeds);
    }
    for (name, frames) in [("natural_frames", 40), ("term_frames", 30), ("abbreviation_frames", 20), ("typo_frames", 20), ("maximum_features", 500)] {
        recipe["kept_neighbour_curriculum"][name] = json!(frames);
    }
    recipe["letter_curriculum"]["term_sentences"] = json!(20);
    recipe["letter_curriculum"]["english_sentences"] = json!(20);
    let names: Vec<String> = recipe.as_object().expect("an object").keys().filter(|name| name.ends_with("_curriculum")).cloned().collect();
    for name in names {
        let part = &mut recipe[&name];
        for (field, value) in [
            ("token_cap", 4), ("per_token", 1), ("per_piece", 1), ("repeat_words", 4), ("frames_per_form", 1), ("minimum_term_count", 2000), ("minimum_count", 5000), ("minimum_latin_count", 5000),
            ("minimum_cyrillic_count", 5000),
        ] {
            if part.get(field).is_some() {
                part[field] = json!(value);
            }
        }
        if part.get("item_modulus").is_some() {
            part["item_modulus"] = json!(97);
        }
    }
    recipe
}

pub(crate) fn write_recipe(path: &Path, recipe: &Value) {
    std::fs::write(path, serde_json::to_vec(recipe).expect("json")).expect("a recipe");
}
