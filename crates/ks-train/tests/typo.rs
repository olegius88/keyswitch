//! The typo tools over small samples: the tables of word counts, the measurement of the corrector on texts, typos made on purpose and the owner's words.
mod support;

use flate2::Compression;
use flate2::write::GzEncoder;
use ks_train::typo_frequency::{Paths, command};
use serde_json::{Value, json};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::LazyLock;
use support::{digest, recipe, scratch, small_tree, trainer, tree};

const RUSSIAN: [&str; 30] = ["сегодня", "хорошая", "погода", "работа", "проект", "человек", "большой", "маленький", "говорить", "думать", "вопрос", "ответить", "время", "город", "страна", "дорога", "письмо", "машина", "компьютер", "программа", "система", "решение", "задача", "результат", "начать", "закончить", "первый", "второй", "последний", "новый"];
const ENGLISH: [&str; 30] = ["today", "weather", "people", "working", "project", "question", "answer", "system", "program", "computer", "number", "letter", "second", "little", "think", "should", "because", "through", "before", "after", "could", "would", "where", "there", "their", "other", "water", "house", "world", "place"];

fn gz(text: &str) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(text.as_bytes()).expect("in memory");
    encoder.finish().expect("in memory")
}

fn sentence(words: &[&str], seed: usize) -> String {
    let chosen: Vec<&str> = (0..6).map(|place| words[(seed * 7 + place * 11) % words.len()]).collect();
    let mut text = chosen.join(" ");
    if seed % 3 == 0 {
        let mut letters = text.chars();
        let first = letters.next().expect("a letter");
        text = first.to_uppercase().chain(letters).collect();
    }
    if seed % 5 == 0 {
        text.push('.');
    }
    if seed % 7 == 0 {
        text.push_str(" Новое Предложение КАПС Тутсмесь mix-слово 42");
    }
    text
}

fn message_file(path: &Path, seed: usize, count: usize) {
    let lines: Vec<String> = (0..count)
        .map(|index| {
            let words: &[&str] = if index % 2 == 0 { &RUSSIAN } else { &ENGLISH };
            let qid = if index % 5 == 0 { json!(format!("q{index}")) } else { json!(seed + index) };
            json!({"qid": qid, "text": sentence(words, seed + index), "kind": "x", "mix": "y"}).to_string()
        })
        .collect();
    std::fs::write(path, gz(&(lines.join("\n") + "\n"))).expect("a file");
}

struct Sample {
    folder: PathBuf,
    messages: PathBuf,
    manifest: PathBuf,
}

static SAMPLE: LazyLock<Sample> = LazyLock::new(|| {
    let folder = scratch("typo");
    let messages = folder.join("messages");
    std::fs::create_dir_all(&messages).expect("a folder");
    for (seed, name) in [(1, "tatoeba-dev"), (2000, "taiga-dev"), (5000, "so-dev")] {
        message_file(&messages.join(format!("{name}.jsonl.gz")), seed, 400);
    }
    let code: Vec<String> = (0..40).map(|index| format!("let weathr{index} = project.syste(); // comput qwerty")).collect();
    std::fs::write(messages.join("code-dev.txt.gz"), gz(&code.join("\n"))).expect("a file");
    // The owner's words, as a field battery holds them: two devices, a stream erased and typed again soon after.
    let private = folder.join(".t/field");
    std::fs::create_dir_all(&private).expect("a folder");
    let item = |original: &str, other: &str, group: i64, intended: i64, trigger: &str, excluded: Option<&str>| {
        let mut row = json!({"original": original, "other": other, "group": group, "intended_group": intended, "trigger": trigger, "t": "2026-11-01 10:00:05,000"});
        if let Some(reason) = excluded {
            row["excluded"] = json!(reason);
        }
        row
    };
    let stream = |time: &str, app: &str, deleted: i64, closed: Value, items: Vec<Value>| json!({"t": time, "app": app, "deleted_chars": deleted, "closed_ts": closed, "items": items}).to_string();
    let first = [
        stream("2026-11-01 10:00:00,000", "firefox", 5, json!(1_793_534_406.0), vec![item("сегодян", "cegjlyz", 1, 1, "space", Some("erased")), item("работа,", "hf,jnf", 1, 1, "space", None), item("прект", "ghtrn", 1, 1, "pause", None), item("!!!", "!!!", 0, 0, "space", None), item("вопрс", "djghc", 0, 1, "space", None), item("проект", "ghjtrn", 1, 1, "space", None), item("Новый", "Yjdsq", 1, 1, "space", None)]),
        stream("2026-11-01 10:00:07,000", "firefox", 0, json!(null), vec![item("сегодня", "ctujlyz", 1, 1, "space", None), item("сегодян", "cegjlyz", 1, 1, "space", None)]),
        stream("2026-11-01 10:00:07,000", "firefox", 0, json!(null), vec![item("дубль", "", 1, 1, "space", None)]),
        stream("2026-11-01 11:00:00,000", "code", 3, json!(1_793_538_000.0), vec![item("weathr", "цуфергк", 0, 0, "space", None)]),
        stream("2026-11-01 12:00:00,000", "code", 3, json!(1_793_538_000.0), vec![item("wether", "цуерук", 0, 0, "space", Some("erased"))]),
        stream("2026-11-02 12:00:00,000", "other", 3, json!(1), vec![]),
    ];
    std::fs::write(private.join("desktop1.jsonl"), first.join("\n") + "\n\n").expect("a file");
    std::fs::write(private.join("laptop.jsonl"), stream("2026-11-01 13:00:00,000", "term", 0, json!(null), vec![item("проектт", "ghjtrnn", 1, 1, "space", None)]) + "\n").expect("a file");
    let manifest = private.join("manifest.json");
    std::fs::write(&manifest, serde_json::to_vec(&json!({"batteries": {"desktop1": ".t/field/desktop1.jsonl", "laptop": ".t/field/laptop.jsonl"}})).expect("json")).expect("a manifest");
    Sample { folder, messages, manifest }
});

fn evaluation(measurement: &str, out: &Path, extra: &[&str]) -> Output {
    trainer().arg("typo-evaluation").arg(measurement).arg("--tree").arg(tree()).arg("--messages").arg(&SAMPLE.messages).arg("--manifest").arg(&SAMPLE.manifest).arg("--out").arg(out).args(extra).output().expect("the trainer starts")
}

fn read(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).expect("a file")).expect("json")
}

#[test]
fn the_corrector_is_measured_on_typos_made_on_purpose_and_on_clean_texts() {
    let out = SAMPLE.folder.join("all");
    let answer = evaluation("all", &out, &["--utc-offset", "18000"]);
    assert!(answer.status.success(), "{}", String::from_utf8_lossy(&answer.stderr));
    let report = read(&out.join("report-all.json"));
    for section in ["synthetic_tuning_half", "synthetic_report_half", "clean", "field"] {
        assert!(report.get(section).is_some(), "{section}");
    }
    assert!(report["field"]["words"].as_i64().unwrap_or(0) > 0, "{}", report["field"]);
    for (measurement, section) in [("synthetic", "synthetic_report_half"), ("clean", "clean"), ("field", "field")] {
        let out = SAMPLE.folder.join(measurement);
        let answer = evaluation(measurement, &out, &[]);
        assert!(answer.status.success(), "{}", String::from_utf8_lossy(&answer.stderr));
        assert!(read(&out.join(format!("report-{measurement}.json"))).get(section).is_some());
    }
    // A small sample gives a policy nothing to choose from.
    let answer = evaluation("synthetic", &SAMPLE.folder.join("tuned"), &["--tune"]);
    assert!(!answer.status.success() && String::from_utf8_lossy(&answer.stderr).contains("no policy"), "{}", String::from_utf8_lossy(&answer.stderr));
}

#[test]
fn inputs_that_are_not_there_stop_a_measurement() {
    let out = SAMPLE.folder.join("failures");
    assert!(!trainer().arg("typo-evaluation").arg("synthetic").arg("--tree").arg(tree()).arg("--messages").arg("/nonexistent").arg("--out").arg(&out).output().expect("the trainer starts").status.success());
    assert!(!evaluation("vocabulary", &out, &[]).status.success());
    let bad = SAMPLE.folder.join("bad.json");
    for manifest in [json!({}), json!({"batteries": {"a": 1}}), json!({"batteries": {"a": "gone.jsonl"}})] {
        std::fs::write(&bad, serde_json::to_vec(&manifest).expect("json")).expect("a file");
        let answer = trainer().arg("typo-evaluation").arg("field").arg("--tree").arg(tree()).arg("--messages").arg(&SAMPLE.messages).arg("--manifest").arg(&bad).arg("--out").arg(&out).output().expect("the trainer starts");
        assert!(!answer.status.success());
    }
}

/// A word list of the shape the tool reads, and the training texts that give the vocabulary.
fn sources(folder: &Path) -> Paths {
    std::fs::create_dir_all(folder).expect("a folder");
    let list = |words: &[&str]| -> String { words.iter().enumerate().map(|(index, word)| format!("{word} {}", 5000 - index * 100)).chain(["редкоеслово 3".to_owned(), "rareword 3".to_owned(), "смесьmix 10".to_owned(), "без_числа".to_owned(), " 5".to_owned()]).collect::<Vec<_>>().join("\n") + "\n" };
    let (ru, en, messages, code) = (folder.join("ru.txt"), folder.join("en.txt"), folder.join("so-train.jsonl.gz"), folder.join("code-train.txt.gz"));
    std::fs::write(&ru, list(&RUSSIAN)).expect("a file");
    std::fs::write(&en, list(&ENGLISH)).expect("a file");
    let lines: Vec<String> = (0..60).map(|index| json!({"qid": index, "text": "вебсокет через docker контейнер запустил rareword редкоеслово", "kind": "x", "mix": "y"}).to_string()).collect();
    std::fs::write(&messages, gz(&(lines.join("\n") + "\n"))).expect("a file");
    std::fs::write(&code, gz(&"docker kubectl вебсокет\n".repeat(30))).expect("a file");
    let pin = |path: &Path| digest(&std::fs::read(path).expect("a file"));
    let pins = [pin(&ru), pin(&en), pin(&messages), pin(&code)];
    Paths { ru, en, messages, code, pins }
}

#[test]
fn the_tables_of_word_counts_are_built_and_verified() {
    let small = small_tree("typo-tables", &recipe());
    let folder = scratch("typo-sources");
    let paths = sources(&folder);
    assert!(command(&small, &paths, false).expect("built"));
    assert!(command(&small, &paths, true).expect("verified"), "the same sources give the same tables");
    // A table or a receipt that differs.
    let receipt = small.join("resources/typo-frequency.receipt.json");
    let good = read(&receipt);
    let mut changed = good.clone();
    changed["locales"]["ru_RU"]["sections_sha256"] = json!({});
    std::fs::write(&receipt, serde_json::to_vec(&changed).expect("json")).expect("a file");
    assert!(!command(&small, &paths, true).expect("compared"));
    std::fs::write(&receipt, serde_json::to_vec(&good).expect("json")).expect("a file");
    let table = small.join("resources/typo-frequency-en_US.bin");
    let bytes = std::fs::read(&table).expect("a table");
    let cut = bytes.iter().position(|byte| *byte == b'\n').expect("a header");
    let mut other = bytes.clone();
    let last = other.len() - 1;
    other[last] ^= 0xff;
    std::fs::write(&table, &other).expect("a file");
    assert!(!command(&small, &paths, true).expect("compared"));
    std::fs::write(&table, &bytes[..cut]).expect("a file");
    assert!(command(&small, &paths, true).is_err());
    std::fs::write(&table, b"no header").expect("a file");
    assert!(command(&small, &paths, true).is_err());
    // Sources that are not those pinned.
    let mut wrong = sources(&folder.join("wrong"));
    wrong.pins[2] = "0".to_owned();
    assert!(command(&small, &wrong, true).unwrap_err().contains("differs from the pinned"));
    let mut gone = sources(&folder.join("gone"));
    gone.messages = PathBuf::from("/nonexistent");
    assert!(command(&small, &gone, true).is_err());
    let mut unpacked = sources(&folder.join("unpacked"));
    std::fs::write(&unpacked.code, b"not gzip").expect("a file");
    unpacked.pins[3] = digest(b"not gzip");
    assert!(command(&small, &unpacked, true).is_err());
    let mut bad_list = sources(&folder.join("bad-list"));
    std::fs::write(&bad_list.ru, [0xff_u8, 0xfe]).expect("a file");
    bad_list.pins[0] = digest(&[0xff, 0xfe]);
    assert!(command(&small, &bad_list, true).is_err());
    let mut bad_messages = sources(&folder.join("bad-messages"));
    let packed = gz("not json\n");
    std::fs::write(&bad_messages.messages, &packed).expect("a file");
    bad_messages.pins[2] = digest(&packed);
    assert!(command(&small, &bad_messages, true).is_err());
    // As a person runs it, with the sources of the repository not there.
    let answer = trainer().arg("typo-frequency").arg("--tree").arg(&small).arg("--private").arg(&folder).arg("--verify").output().expect("the trainer starts");
    assert!(!answer.status.success());
    let answer = trainer().arg("typo-frequency").arg("--tree").arg(&small).output().expect("the trainer starts");
    assert!(!answer.status.success());
}

#[test]
fn the_count_of_the_technical_vocabulary_is_chosen_on_the_development_texts() {
    let folder = scratch("typo-vocabulary");
    let paths = sources(&folder);
    // What a clean measurement lists: a word it would correct, in both languages, one of them a word the vocabulary knows.
    let clean = folder.join("clean.tsv");
    std::fs::write(&clean, "corpus\tkind\tlocale\tqid\tword\tchoice\toperation\tcount\ntatoeba-dev\tprose\tru_RU\t1\tсегодян\tсегодня\tswapped\t5\nso-dev\tforum\ten_US\t2\tdocker\tdocked\tneighbour\t5\nso-dev\tforum\ten_US\t3\tweathr\tweather\tmissed\t5\n").expect("a file");
    let vocabulary = SAMPLE.folder.join("vocabulary");
    let answer = trainer()
        .arg("typo-evaluation")
        .arg("vocabulary")
        .arg("--tree")
        .arg(tree())
        .arg("--messages")
        .arg(&SAMPLE.messages)
        .arg("--out")
        .arg(&vocabulary)
        .arg("--clean")
        .arg(&clean)
        .arg("--ru")
        .arg(&paths.ru)
        .arg("--en")
        .arg(&paths.en)
        .arg("--train-messages")
        .arg(&paths.messages)
        .arg("--train-code")
        .arg(&paths.code)
        .output()
        .expect("the trainer starts");
    assert!(answer.status.success(), "{}", String::from_utf8_lossy(&answer.stderr));
    assert!(read(&vocabulary.join("report-vocabulary.json")).get("vocabulary").is_some());
}
