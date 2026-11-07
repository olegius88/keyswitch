//! The typo measurement of the owner's typing over streams written by hand: a stream typed again too late, one with
//! nothing after it, one whose close is not recorded and one whose next stream's time cannot be read.
mod support;

use serde_json::{Value, json};
use std::path::Path;
use support::{scratch, trainer, tree};

fn item(original: &str, other: &str, time: &str, excluded: Option<&str>) -> Value {
    let mut row = json!({"original": original, "other": other, "group": 1, "intended_group": 1, "trigger": "space", "t": time});
    if let Some(reason) = excluded {
        row["excluded"] = json!(reason);
    }
    row
}

fn stream(time: &str, deleted: i64, closed: Value, items: Vec<Value>) -> String {
    json!({"t": time, "app": "chat", "deleted_chars": deleted, "closed_ts": closed, "items": items}).to_string()
}

/// 2026-01-15 09:00:00 UTC, in seconds.
const CLOSED: f64 = 1_768_467_600.0;

fn manifest(folder: &Path) -> std::path::PathBuf {
    let private = folder.join(".t/field");
    std::fs::create_dir_all(&private).expect("a folder");
    let streams = [
        // Erased, and the next stream starts ten minutes after the close: not typed again.
        stream("2026-01-15 09:00:00,000", 4, json!(CLOSED), vec![item("сегодян", "cegjlyz", "2026-01-15 09:00:00,000", Some("erased")), item("abcж", "", "2026-01-15 09:00:00,000", None)]),
        stream("2026-01-15 09:00:01,000", 2, json!(null), vec![item("сегодня", "ctujlyz", "2026-01-15 09:10:00,000", None)]),
        // A close that is not recorded, then a stream of no items, then one whose time cannot be read.
        stream("2026-01-15 09:00:02,000", 2, json!(CLOSED), vec![item("работа", "hf,jnf", "2026-01-15 09:00:02,000", None)]),
        stream("2026-01-15 09:00:03,000", 1, json!(CLOSED), vec![]),
        stream("2026-01-15 09:00:04,000", 1, json!(CLOSED), vec![item("проект", "ghjtrn", "an hour ago", None)]),
    ];
    std::fs::write(private.join("phone.jsonl"), streams.join("\n") + "\n").expect("a battery");
    let path = private.join("manifest.json");
    std::fs::write(&path, serde_json::to_vec(&json!({"batteries": {"phone": ".t/field/phone.jsonl"}})).expect("json")).expect("a manifest");
    path
}

#[test]
fn the_owner_s_words_are_measured_whatever_follows_a_stream() {
    let folder = scratch("typo-streams");
    let manifest = manifest(&folder);
    let out = folder.join("out");
    let answer = trainer().arg("typo-evaluation").arg("field").arg("--tree").arg(tree()).arg("--messages").arg(&folder).arg("--manifest").arg(&manifest).arg("--out").arg(&out).output().expect("the trainer starts");
    assert!(answer.status.success(), "{}", String::from_utf8_lossy(&answer.stderr));
    let report: Value = serde_json::from_slice(&std::fs::read(out.join("report-field.json")).expect("a report")).expect("json");
    assert_eq!(report["field"]["words"], json!(5), "{}", report["field"]);
}

/// The most frequent lowercase words of at least five letters of a reference lexicon, most frequent first.
fn frequent(path: &Path, alphabet: &dyn Fn(char) -> bool) -> Vec<String> {
    let text = std::fs::read_to_string(path).expect("a lexicon");
    let mut rows: Vec<(i64, String)> = text
        .lines()
        .filter_map(|line| line.trim().split_once(' '))
        .filter_map(|(count, word)| count.parse::<i64>().ok().map(|count| (count, word.to_owned())))
        .filter(|(_, word)| word.chars().count() >= 5 && word.chars().all(alphabet))
        .collect();
    rows.sort_by(|a, b| b.cmp(a));
    rows.into_iter().take(400).map(|(_, word)| word).collect()
}

fn gz(text: &str) -> Vec<u8> {
    use std::io::Write as _;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(text.as_bytes()).expect("in memory");
    encoder.finish().expect("in memory")
}

/// Development texts of frequent words, eight to a message, and lines whose draws were searched so that the slip made in
/// them is the one a case needs: a substitution of `ё`, which has no neighbour on the keyboard; transpositions whose
/// slips the training texts hold; a rare word sampled; words no list holds sampled.
fn development_texts(folder: &Path) -> std::path::PathBuf {
    let sources = tree().join("model/intent_v1/sources");
    let russian = frequent(&sources.join("ru_RU.lm"), &|character| ('а'..='я').contains(&character) || character == 'ё');
    let english = frequent(&sources.join("en_US.lm"), &|character| character.is_ascii_lowercase());
    let mut lines: Vec<String> = (0..800)
        .map(|index| {
            let words = if index % 2 == 0 { &russian } else { &english };
            let text: Vec<&str> = (0..8).map(|place| words[(index * 7 + place * 13) % words.len()].as_str()).collect();
            json!({"qid": index, "text": text.join(" ")}).to_string()
        })
        .collect();
    let shared = lines.clone();
    for (qid, text) in [
        ("q164", "да да да ребёнок"), ("q10051", "да зелёный"), ("q20027", "да да твёрдый"), ("q30268", "да да да лёгкий"), ("q40016", "да да да да да человек"), ("q50042", "да который"),
        ("q60001", "да обычно"), ("q70004", "qzxwvbnm"), ("q80002", "да да да да да zxqwvbtk"), ("mixed", "McDonald and Смесьmix are here."), ("slips", "это чеолвек и коотрый\tгород"),
    ] {
        lines.push(json!({"qid": qid, "text": text}).to_string());
    }
    let messages = folder.join("messages");
    std::fs::create_dir_all(&messages).expect("a folder");
    std::fs::write(messages.join("tatoeba-dev.jsonl.gz"), gz(&(lines.join("\n") + "\n"))).expect("a file");
    for name in ["taiga-dev", "so-dev"] {
        std::fs::write(messages.join(format!("{name}.jsonl.gz")), gz(&(shared.join("\n") + "\n"))).expect("a file");
    }
    std::fs::write(messages.join("code-dev.txt.gz"), gz("let x = 1\nfn чеолвек() {}\n")).expect("a file");
    messages
}

/// The owner's streams of one device: corrections erased and typed again soon after, as typed or otherwise, erased with
/// nothing after them, and kept; a word kept often enough to become the owner's; a word too short to judge.
fn field_streams(folder: &Path) -> std::path::PathBuf {
    let private = folder.join(".t/field");
    std::fs::create_dir_all(&private).expect("a folder");
    let at = |second: u32| format!("2026-01-15 09:00:{second:02},000");
    let closed = |second: u32| json!(CLOSED + f64::from(second));
    let keep = |word: &str, second: u32| item(word, "", &at(second), None);
    let erase = |word: &str, second: u32| item(word, "", &at(second), Some("erased"));
    let streams = [
        stream(&at(0), 3, closed(1), vec![erase("чеолвек", 0)]),
        stream(&at(2), 3, closed(3), vec![keep("человек", 2)]),
        stream(&at(4), 3, closed(5), vec![erase("коотрый", 4)]),
        stream(&at(6), 0, closed(7), vec![keep("привет", 6), keep("гоорда", 6), keep("гоорда", 6), keep("гоорда", 6), keep("да", 6), keep("abc1", 6)]),
        stream(&at(8), 3, closed(9), vec![keep("работа", 8)]),
        json!({"t": at(10), "app": "mail", "deleted_chars": 3, "closed_ts": closed(11), "items": [erase("обалсти", 10)]}).to_string(),
    ];
    std::fs::write(private.join("tablet.jsonl"), streams.join("\n") + "\n").expect("a battery");
    let path = private.join("manifest.json");
    std::fs::write(&path, serde_json::to_vec(&json!({"batteries": {"tablet": ".t/field/tablet.jsonl"}})).expect("json")).expect("a manifest");
    path
}

#[test]
fn every_measurement_runs_over_texts_whose_slips_were_chosen() {
    let folder = scratch("typo-chosen");
    let messages = development_texts(&folder);
    let manifest = field_streams(&folder);
    let out = folder.join("all");
    let answer = trainer().arg("typo-evaluation").arg("all").arg("--tree").arg(tree()).arg("--messages").arg(&messages).arg("--manifest").arg(&manifest).arg("--out").arg(&out).output().expect("the trainer starts");
    assert!(answer.status.success(), "{}", String::from_utf8_lossy(&answer.stderr));
    let report: Value = serde_json::from_slice(&std::fs::read(out.join("report-all.json")).expect("a report")).expect("json");
    assert!(report["synthetic_tuning_half"]["all"]["wrong"].as_i64().expect("a count") + report["synthetic_report_half"]["all"]["wrong"].as_i64().expect("a count") > 0, "{report}");
    let field = &report["field"];
    for mark in ["erased_retyped_as_correction", "erased_retyped_otherwise", "erased", "kept", "personal"] {
        assert!(field[mark].as_i64().unwrap_or(0) > 0, "{mark}: {field}");
    }
    assert!(field["corrections_of_words_that_became_the_owner_s"].as_i64().expect("a count") > 0, "{field}");
    assert!(std::fs::read_to_string(out.join("clean-corrections.tsv")).expect("the corrections").lines().count() > 2);
    // The count of the technical vocabulary, over lists of the words the draws chose and training texts that hold two of
    // their slips and the other reading of a clean correction, each three times: the least count that keeps every slip.
    let lists = folder.join("lists");
    std::fs::create_dir_all(&lists).expect("a folder");
    let (ru, en, clean) = (lists.join("ru.txt"), lists.join("en.txt"), lists.join("clean.tsv"));
    std::fs::write(&ru, "ребёнок 5000\nзелёный 5000\nтвёрдый 5000\nлёгкий 5000\nчеловек 5000\nкоторый 5000\nобычно 3\nзарплата 100\n").expect("a list");
    std::fs::write(&en, "window 5000\ngarden 100\n").expect("a list");
    std::fs::write(&clean, "corpus\tkind\tlocale\tqid\tword\tcorrection\toperation\ntatoeba-dev\tprose\tru_RU\t1\tприве\tпривет\tinsertion\n").expect("a list");
    let train_messages: Vec<String> = (0..3).map(|_| json!({"qid": 1, "text": "челвоек и ещё"}).to_string()).collect();
    let (messages_file, code_file) = (lists.join("so-train.jsonl.gz"), lists.join("code-train.txt.gz"));
    std::fs::write(&messages_file, gz(&(train_messages.join("\n") + "\n"))).expect("a file");
    std::fs::write(&code_file, gz(&"rjnjshq ghbdt\n".repeat(3))).expect("a file");
    let vocabulary = folder.join("vocabulary");
    let answer = trainer()
        .arg("typo-evaluation")
        .arg("vocabulary")
        .arg("--tree")
        .arg(tree())
        .arg("--messages")
        .arg(&messages)
        .arg("--out")
        .arg(&vocabulary)
        .arg("--clean")
        .arg(&clean)
        .arg("--ru")
        .arg(&ru)
        .arg("--en")
        .arg(&en)
        .arg("--train-messages")
        .arg(&messages_file)
        .arg("--train-code")
        .arg(&code_file)
        .output()
        .expect("the trainer starts");
    assert!(answer.status.success(), "{}", String::from_utf8_lossy(&answer.stderr));
    let report: Value = serde_json::from_slice(&std::fs::read(vocabulary.join("report-vocabulary.json")).expect("a report")).expect("json");
    assert_eq!(report["vocabulary"]["chosen"], json!(5), "{}", report["vocabulary"]);
}
