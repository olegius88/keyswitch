//! The chat test at its edges: a ledger not made yet, a folder holding other files, a batch that repeats members, members the
//! messages lack, a candidate much worse than the released model.
mod support;

use flate2::Compression;
use flate2::write::GzEncoder;
use ks_text::{JsonOptions, json_dumps};
use ks_train::mixed::{DevchatRequest, devchat};
use serde_json::{Value, json};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use support::{chat_messages, digest, recipe, scratch, small_tree};

/// Short messages of each shape the bench plans: Russian with a term, a term with Russian, words that open with Ё or ё, a
/// digit first, one-letter words, English. Each typing mode has a message to type: a table of the pairs holds every mode.
const TEXTS: [&str; 7] = ["запусти git status", "docker запускает новый контейнер", "Ёлка и ёж у окна", "42 строки на Rust", "а и в", "привет как дела", "simple english text"];
/// Plain Russian the released model restores when it is typed in the other layout, and a term before and inside it.
const PROSE: [&str; 10] = [
    "привет как дела сегодня",
    "мы пошли домой вечером",
    "хорошая погода сегодня утром",
    "книга лежит на столе",
    "надо купить хлеба и молока",
    "он читает новую книгу",
    "завтра будет дождь и ветер",
    "мама пишет длинное письмо",
    "docker запускает новый контейнер",
    "запусти docker сегодня вечером",
];

fn gz(lines: &[String]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    for line in lines {
        encoder.write_all(line.as_bytes()).expect("in memory");
        encoder.write_all(b"\n").expect("in memory");
    }
    encoder.finish().expect("in memory")
}

struct Chat {
    tree: PathBuf,
    directory: PathBuf,
    folder: PathBuf,
    ledger: PathBuf,
    sequences: PathBuf,
    candidate: PathBuf,
    baseline: PathBuf,
}

/// A chat test of these messages whose ledger folders are not made yet.
fn chat(name: &str, texts: &[&str], test_count: u64) -> Chat {
    let folder = scratch(name);
    let tree = small_tree(&format!("{name}-tree"), &recipe());
    let directory = folder.join("messages");
    chat_messages(&tree, &directory, texts, test_count);
    let candidate = folder.join("candidate.json");
    std::fs::copy(tree.join("resources/models/context_policy_v1.json"), &candidate).expect("a copy");
    let baseline = tree.join("model/context_v3/baseline-context-v1.json");
    let (ledger, sequences) = (folder.join("ledger"), folder.join("sequences"));
    Chat { tree, directory, folder, ledger, sequences, candidate, baseline }
}

fn run_chat(chat: &Chat, split: &str) -> Result<Value, String> {
    run_candidate(chat, split, &chat.candidate)
}

fn run_candidate(chat: &Chat, split: &str, candidate: &Path) -> Result<Value, String> {
    let resources = chat.tree.join("resources");
    let output = chat.folder.join(format!("{split}.json"));
    devchat(&DevchatRequest {
        root: &chat.tree,
        resource_dir: &resources,
        split,
        directory: &chat.directory,
        candidate,
        released: &chat.candidate,
        baseline: &chat.baseline,
        output: &output,
        ledger: &chat.ledger,
        sequences_ledger: &chat.sequences,
        workers: 2,
    })
}

/// The development result recorded with every floor holding, and a Tatoeba test the candidate passed.
fn allowed(chat: &Chat) {
    let development = chat.ledger.join(format!("{}.dev.json", ks_train::corpus::checksum(&chat.candidate).expect("a checksum")));
    let mut recorded: Value = serde_json::from_slice(&std::fs::read(&development).expect("the record")).expect("json");
    for floor in recorded["floors"].as_object_mut().expect("the floors").values_mut() {
        *floor = json!(true);
    }
    std::fs::write(&development, serde_json::to_vec(&recorded).expect("json")).expect("a record");
    std::fs::create_dir_all(&chat.sequences).expect("a folder");
    let identity = json!({"artifact_sha256": ks_train::corpus::checksum(&chat.candidate).expect("a checksum")});
    // A development read of the sequences first, then the test it passed.
    for (name, split) in [("a", "dev"), ("b", "test")] {
        std::fs::write(chat.sequences.join(format!("{name}.access.json")), serde_json::to_vec(&identity).expect("json")).expect("a record");
        std::fs::write(chat.sequences.join(format!("{name}.outcome.json")), serde_json::to_vec(&json!({"split": split, "promotion_passed": true})).expect("json")).expect("a record");
    }
}

fn members(texts: &[&str]) -> Vec<String> {
    texts
        .iter()
        .enumerate()
        .map(|(index, text)| json!({"text": digest(text.as_bytes()), "question": format!("question{index}"), "mix": if text.is_ascii() { "russian" } else { "mixed" }}).to_string())
        .collect()
}

#[test]
fn a_development_read_makes_its_ledger_and_passes_over_other_files() {
    let run = chat("chat-fresh", &TEXTS, 6);
    // Files of the folder that are no messages of a split: neither is pinned.
    std::fs::write(run.directory.join("notes.txt"), b"notes").expect("a file");
    std::fs::write(run.directory.join("devchat-notes.txt"), b"notes").expect("a file");
    assert!(!run.ledger.exists());
    let result = run_chat(&run, "dev").expect("a result");
    assert_eq!(result["messages"].as_u64(), Some(u64::try_from(TEXTS.len()).expect("small")));
    let development = run.ledger.join(format!("{}.dev.json", ks_train::corpus::checksum(&run.candidate).expect("a checksum")));
    let recorded: Value = serde_json::from_slice(&std::fs::read(&development).expect("the record")).expect("json");
    let pinned: Vec<&String> = recorded["runtime"].as_object().expect("the pins").keys().filter(|name| name.starts_with("messages/")).collect();
    assert_eq!(pinned.len(), 4, "{pinned:?}");
}

#[test]
fn a_member_the_messages_do_not_hold_stops_the_read() {
    let run = chat("chat-lacking", &TEXTS, 6);
    let mut listed = members(&TEXTS);
    listed.push(json!({"text": digest("нет такого сообщения".as_bytes()), "question": "lost", "mix": "mixed"}).to_string());
    std::fs::write(run.directory.join("devchat-dev.members.jsonl.gz"), gz(&listed)).expect("a file");
    let said = run_chat(&run, "dev").expect_err("a member without its message");
    assert!(said.contains("lacks members"), "{said}");
}

#[test]
fn a_batch_that_repeats_members_counts_each_once() {
    let run = chat("chat-batch", &TEXTS, 6);
    let messages: Vec<String> = TEXTS.iter().map(|text| json!({"text": text}).to_string()).collect();
    std::fs::write(run.directory.join("devchat-batch1.messages.jsonl.gz"), gz(&messages)).expect("a file");
    // The batch's members name their questions by number, as the comments' posts do.
    let numbered: Vec<String> = TEXTS.iter().enumerate().map(|(index, text)| json!({"text": digest(text.as_bytes()), "question": index, "mix": "mixed"}).to_string()).collect();
    std::fs::write(run.directory.join("devchat-batch1.members.jsonl.gz"), gz(&numbered)).expect("a file");
    let path = run.tree.join("model/context_v3/devchat-test-batches.json");
    let mut batches: Value = serde_json::from_slice(&std::fs::read(&path).expect("the batches")).expect("json");
    batches["batches"] = json!([{"name": "devchat-batch1", "content_sha256": digest((messages.join("\n") + "\n").as_bytes())}]);
    std::fs::write(&path, serde_json::to_vec(&batches).expect("json")).expect("a file");
    run_chat(&run, "dev").expect("a development result");
    allowed(&run);
    // A read of this candidate that took no message holds no claim.
    let nothing = json!({"pairs": {"candidate": {"sha256": ks_train::corpus::checksum(&run.candidate).expect("a checksum")}}, "test_membership": {"messages": [], "questions": []}});
    std::fs::write(run.ledger.join("0-nothing.access.json"), serde_json::to_vec(&nothing).expect("json")).expect("a record");
    let read = run_chat(&run, "test").expect("the test is read");
    // Six mixed messages and a quarter as many Russian ones, each counted once though two files hold it.
    assert_eq!(read["messages"].as_u64(), Some(7));
    let access: Vec<Value> = std::fs::read_dir(&run.ledger)
        .expect("the ledger")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.to_string_lossy().ends_with(".access.json"))
        .map(|path| serde_json::from_slice(&std::fs::read(path).expect("a record")).expect("json"))
        .collect();
    assert_eq!(access.len(), 2);
    assert!(access.iter().any(|record| record["messages_sha256"].as_array().map(Vec::len) == Some(2)));
}

#[test]
fn a_candidate_that_keeps_what_the_released_model_restores_is_worse() {
    let run = chat("chat-worse", &PROSE, 2);
    // The released model with every answer pushed to keeping the word as typed, its weights pinned again.
    let mut model: Value = serde_json::from_slice(&std::fs::read(&run.candidate).expect("the model")).expect("json");
    model["weights"]["bias"] = json!([500, -500, -500, -500]);
    let compact = JsonOptions { ensure_ascii: false, sort_keys: true, separators: Some((",".to_owned(), ":".to_owned())), ..JsonOptions::default() };
    model["weights_sha256"] = json!(digest(json_dumps(&model["weights"], &compact).as_bytes()));
    let worse = run.folder.join("worse.json");
    std::fs::write(&worse, serde_json::to_vec(&model).expect("json")).expect("a model");
    let result = run_candidate(&run, "dev", &worse).expect("a result");
    let losses = result["discordance"].as_object().expect("the pairs").values().filter_map(|mode| mode["losses"].as_i64()).max();
    assert!(losses >= Some(5), "{}", result["discordance"]);
    assert_eq!(result["floors"]["f3_not_worse_than_released"], json!(false), "{}", result["discordance"]);
}
