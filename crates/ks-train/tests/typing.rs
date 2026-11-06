//! The bench that types messages through the engine: the jobs of a recipe, each typing mode, the questions captured from a run and the chat test.
mod support;

use flate2::Compression;
use flate2::write::GzEncoder;
use ks_train::mixed::{DevchatRequest, capture, devchat, evaluate, jobs_for};
use serde_json::{Value, json};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use support::{chat_messages, digest, recipe, scratch, small_tree, trainer, tree};

/// Messages of the kinds a person types: Russian with terms, signs, digits, a word of capitals, a line with a mention.
const MIXED: [&str; 24] = [
    "привет как дела сегодня",
    "запусти git status и покажи вывод",
    "открой файл config.yaml и поправь порт",
    "Я думаю что это bug в docker-compose",
    "в python3 есть метод sorted, он быстрее",
    "мой адрес test@example.com, напиши",
    "сделай commit и push в main!",
    "почему API возвращает 500?",
    "если (x > 5) то выходим",
    "ссылка https://example.com/page тут",
    "npm install затем npm run build",
    "это \"тест\" и ещё 'цитата' здесь",
    "версия 2.5.1 вышла, обнови pip",
    "кто-то сломал CI, посмотри логи",
    "ФАЙЛ README лежит в docs/",
    "вот 42 строки кода на Rust",
    "ну и ладно, я ушёл",
    "дай мне minute, сейчас vernus",
    "она сказала: hello, world",
    "tot файл ещё не готов",
    "а что если set -e убрать",
    "в логе ERROR, потом WARN",
    "пятый шаг: make test",
    "итог: 3 из 5 прошли",
];
const RUSSIAN: [&str; 6] = ["просто обычный текст", "мы пошли домой вечером", "хорошая погода сегодня", "книга лежит на столе", "надо купить хлеба", "он читает новую книгу"];

fn gz(lines: &[String]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    for line in lines {
        encoder.write_all(line.as_bytes()).expect("in memory");
        encoder.write_all(b"\n").expect("in memory");
    }
    encoder.finish().expect("in memory")
}

struct Bench {
    folder: PathBuf,
    messages: PathBuf,
    code: PathBuf,
    resources: PathBuf,
    model: PathBuf,
}

static BENCH: LazyLock<Bench> = LazyLock::new(|| {
    let folder = scratch("typing");
    let mut lines: Vec<String> = MIXED.iter().enumerate().map(|(index, text)| json!({"kind": "comment", "text": text, "mix": "mixed", "qid": index}).to_string()).collect();
    lines.extend(RUSSIAN.iter().enumerate().map(|(index, text)| json!({"kind": "comment", "text": text, "mix": "russian", "qid": 100 + index}).to_string()));
    lines.push(json!({"kind": "other", "text": "не тот вид", "mix": "mixed", "qid": 999}).to_string());
    let messages = folder.join("messages.jsonl.gz");
    std::fs::write(&messages, gz(&lines)).expect("a file");
    let above: Vec<String> = ["строка сверху про проект", "ещё одна строка выше"].iter().enumerate().map(|(index, text)| json!({"kind": "note", "text": text, "mix": "russian", "qid": 500 + index}).to_string()).collect();
    std::fs::write(folder.join("above.jsonl.gz"), gz(&above)).expect("a file");
    let code = folder.join("code.txt.gz");
    std::fs::write(&code, gz(&["let x = 1;".to_owned(), "fn main() {}".to_owned()])).expect("a file");
    let root = tree();
    Bench { folder, messages, code, resources: root.join("resources"), model: root.join("resources/models/context_policy_v1.json") }
});

fn base_recipe() -> Value {
    json!({"seed": "typing-test", "kinds": ["comment"], "count": 24, "russian_share": 0.25})
}

fn typed(recipe: &Value, name: &str) -> Value {
    let bench = &*BENCH;
    let output = bench.folder.join(format!("{name}.json"));
    let summary = evaluate(&bench.resources, &bench.messages, recipe, Some(&bench.code), &bench.model, &output, 2).expect("typed");
    assert!(output.is_file());
    summary
}

#[test]
fn every_mode_types_every_message() {
    let summary = typed(&base_recipe(), "modes");
    for mode in ["correct", "forget_ru", "forget_en", "start_wrong"] {
        assert!(summary["summary"][mode]["n"].as_i64().expect("a count") > 0, "{mode}");
    }
    let mut more = base_recipe();
    more["modes"] = json!(["slip", "slip_letter", "slip_letter_back", "unprotected", "us_signs"]);
    let summary = typed(&more, "more-modes");
    assert!(summary["summary"].as_object().expect("a table").len() >= 4);
    let mut edit = base_recipe();
    edit["edit"] = json!(true);
    let summary = typed(&edit, "edit");
    assert!(summary["summary"]["insert"]["n"].as_i64().expect("a count") > 0);
}

#[test]
fn a_recipe_puts_typos_and_lines_above_into_the_messages() {
    let mut recipe = base_recipe();
    recipe["typo_rate"] = json!(0.6);
    recipe["typo_kinds"] = json!(["missed", "swapped", "doubled", "neighbour", "extra"]);
    recipe["typo_min_letters"] = json!(2);
    recipe["prefix_rate"] = json!(1.0);
    assert!(typed(&recipe, "typos")["summary"].is_object());
    // The line above comes from the code, from another pool or from the messages themselves.
    recipe["lines_above"] = json!({"messages": "above.jsonl.gz", "kinds": ["note"], "mix": "russian", "seed": "above", "count": 2});
    let jobs = jobs_for(&BENCH.messages, &recipe, Some(&BENCH.code)).expect("jobs");
    let prefixes: Vec<&str> = jobs.iter().map(|job| job.prefix.as_str()).collect();
    assert!(prefixes.iter().any(|line| line.contains("fn main") || line.contains("let x")), "{prefixes:?}");
    assert!(prefixes.iter().any(|line| line.contains("строка")), "{prefixes:?}");
    recipe.as_object_mut().expect("an object").remove("lines_above");
    let jobs = jobs_for(&BENCH.messages, &recipe, None).expect("jobs");
    assert!(jobs.iter().all(|job| job.prefix.ends_with('\n')));
    recipe["prefix_rate"] = json!(0.0);
    assert!(jobs_for(&BENCH.messages, &recipe, Some(&BENCH.code)).expect("jobs").iter().all(|job| job.prefix.is_empty()));
}

#[test]
fn a_recipe_that_cannot_be_is_refused() {
    let refused = |recipe: &Value, words: &str| {
        let said = jobs_for(&BENCH.messages, recipe, None).map(|_| ()).expect_err(words);
        assert!(said.contains(words), "{words}: {said}");
    };
    refused(&json!({}), "without a seed");
    let mut typos = base_recipe();
    typos["typo_kinds"] = json!(["unheard of"]);
    refused(&typos, "unknown typo kinds");
    let mut least = base_recipe();
    least["typo_min_letters"] = json!(0);
    refused(&least, "unknown typo kinds");
    let mut modes = base_recipe();
    modes["modes"] = json!(["unheard of"]);
    refused(&modes, "unknown typing modes");
    assert!(jobs_for(Path::new("/nonexistent/messages.gz"), &base_recipe(), None).is_err());
    let garbage = BENCH.folder.join("garbage.gz");
    std::fs::write(&garbage, gz(&["not json".to_owned()])).expect("a file");
    assert!(jobs_for(&garbage, &base_recipe(), None).is_err());
    let mut pool = base_recipe();
    pool["lines_above"] = json!({"messages": "gone.gz", "kinds": [], "mix": "", "seed": "", "count": 1});
    assert!(jobs_for(&BENCH.messages, &pool, None).is_err());
    let unreadable = base_recipe();
    let output = BENCH.folder.join("no-model.json");
    assert!(evaluate(&BENCH.resources, &BENCH.messages, &unreadable, None, Path::new("/nonexistent/model.json"), &output, 1).is_err());
}

/// One capture source per class of questions.
fn manifest(folder: &Path, schema: i64, classes: &[Option<&str>]) -> PathBuf {
    let sources: Vec<Value> = classes
        .iter()
        .enumerate()
        .map(|(index, question)| {
            let mut recipe = base_recipe();
            recipe["messages"] = json!("messages.jsonl.gz");
            recipe["count"] = json!(8);
            if let Some(name) = question {
                recipe["questions"] = json!(name);
            }
            if index % 2 == 1 {
                recipe["labels"] = json!(["convert", "keep"]);
            }
            json!({"file": format!("source{index}.jsonl.xz"), "recipe": recipe})
        })
        .collect();
    let path = folder.join("manifest.json");
    std::fs::write(&path, serde_json::to_vec(&json!({"schema_version": schema, "sources": sources})).expect("json")).expect("a manifest");
    path
}

#[test]
fn a_capture_keeps_the_questions_a_recipe_names_and_verifies_against_its_files() {
    let bench = &*BENCH;
    let folder = scratch("capture");
    let classes = [None, Some("variable"), Some("dotted"), Some("alnum"), Some("hypothesis"), Some("short"), Some("slip"), Some("sign"), Some("resume"), Some("cyrletter"), Some("dash"), Some("digitdot"), Some("quote")];
    let path = manifest(&folder, 2, &classes);
    let results = capture(&bench.resources, &path, &bench.folder, &bench.model, None, false, 2).expect("captured");
    assert_eq!(results.as_object().expect("a table").len(), classes.len());
    let written = serde_json::from_slice::<Value>(&std::fs::read(&path).expect("the manifest")).expect("json");
    assert!(written["sources"][0]["sha256"].is_string());
    // The same run reads the same questions.
    let again = capture(&bench.resources, &path, &bench.folder, &bench.model, None, true, 2).expect("verified");
    assert_eq!(again, results);
    // One source only, and a file that differs.
    let only = capture(&bench.resources, &path, &bench.folder, &bench.model, Some("source0.jsonl.xz"), true, 1).expect("verified");
    assert_eq!(only.as_object().expect("a table").len(), 1);
    let other = folder.join("source0.jsonl.xz");
    let mut encoder = xz2::write::XzEncoder::new(Vec::new(), 1);
    encoder.write_all(b"other").expect("in memory");
    std::fs::write(&other, encoder.finish().expect("in memory")).expect("a file");
    assert!(capture(&bench.resources, &path, &bench.folder, &bench.model, Some("source0.jsonl.xz"), true, 1).unwrap_err().contains("differ"));
    std::fs::remove_file(&other).expect("the file goes");
    assert!(capture(&bench.resources, &path, &bench.folder, &bench.model, Some("source0.jsonl.xz"), true, 1).is_err());
    // The older schema, and what no schema allows.
    let old = scratch("capture-old");
    let path = manifest(&old, 1, &[Some("hypothesis"), Some("short"), Some("slip")]);
    capture(&bench.resources, &path, &bench.folder, &bench.model, None, false, 1).expect("captured");
    let refused = |value: &Value, words: &str| {
        let path = old.join("refused.json");
        std::fs::write(&path, serde_json::to_vec(value).expect("json")).expect("a manifest");
        let said = capture(&bench.resources, &path, &bench.folder, &bench.model, None, false, 1).expect_err(words);
        assert!(said.contains(words), "{words}: {said}");
    };
    refused(&json!({"schema_version": 3, "sources": []}), "invalid captured question manifest");
    refused(&json!({"schema_version": 2}), "without sources");
    refused(&json!({"schema_version": 2, "sources": [{"file": "x", "recipe": {}}]}), "without messages");
    let mut unknown = base_recipe();
    unknown["messages"] = json!("messages.jsonl.gz");
    unknown["questions"] = json!("unheard of");
    refused(&json!({"schema_version": 2, "sources": [{"file": "x", "recipe": unknown}]}), "no class of questions");
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

fn chat(name: &str, test_count: u64) -> Chat {
    let folder = scratch(name);
    let tree = small_tree(&format!("{name}-tree"), &recipe());
    let directory = folder.join("messages");
    let texts: Vec<&str> = MIXED.iter().chain(&RUSSIAN).copied().collect();
    chat_messages(&tree, &directory, &texts, test_count);
    let candidate = folder.join("candidate.json");
    std::fs::copy(tree.join("resources/models/context_policy_v1.json"), &candidate).expect("a copy");
    let baseline = tree.join("model/context_v3/baseline-context-v1.json");
    let (ledger, sequences) = (folder.join("ledger"), folder.join("sequences"));
    std::fs::create_dir_all(&ledger).expect("a folder");
    std::fs::create_dir_all(&sequences).expect("a folder");
    Chat { tree, directory, folder, ledger, sequences, candidate, baseline }
}

fn run_chat(chat: &Chat, split: &str, output: &Path, candidate: &Path, baseline: &Path) -> Result<Value, String> {
    let resources = chat.tree.join("resources");
    devchat(&DevchatRequest {
        root: &chat.tree,
        resource_dir: &resources,
        split,
        directory: &chat.directory,
        candidate,
        released: &chat.candidate,
        baseline,
        output,
        ledger: &chat.ledger,
        sequences_ledger: &chat.sequences,
        workers: 2,
    })
}

/// A sealed test the sequence ledger holds as passed, for a candidate.
fn passed_sequences(chat: &Chat, candidate: &Path, passed: bool) {
    let identity = json!({"artifact_sha256": ks_train::corpus::checksum(candidate).expect("a checksum")});
    std::fs::write(chat.sequences.join("a.access.json"), serde_json::to_vec(&identity).expect("json")).expect("a record");
    std::fs::write(chat.sequences.join("a.outcome.json"), serde_json::to_vec(&json!({"split": "test", "promotion_passed": passed})).expect("json")).expect("a record");
}

#[test]
fn a_chat_test_is_read_after_a_development_result_that_holds() {
    let run = chat("chat", 4);
    let output = run.folder.join("chat.json");
    // The baseline against itself gains nothing, so the floors do not hold.
    let same = run_chat(&run, "dev", &output, &run.baseline, &run.baseline).expect("a result");
    assert_eq!(same["floors"]["f1_better_than_baseline"], json!(false));
    // No development result of this candidate.
    let refused = |words: &str, candidate: &Path, baseline: &Path| {
        let said = run_chat(&run, "test", &output, candidate, baseline).expect_err(words);
        assert!(said.contains(words), "{words}: {said}");
    };
    refused("no development result", &run.candidate, &run.baseline);
    refused("holds no gain", &run.baseline, &run.baseline);
    // Development, for the candidate.
    let result = run_chat(&run, "dev", &output, &run.candidate, &run.baseline).expect("a result");
    assert!(output.is_file());
    let development = run.ledger.join(format!("{}.dev.json", ks_train::corpus::checksum(&run.candidate).expect("a checksum")));
    let mut recorded: Value = serde_json::from_slice(&std::fs::read(&development).expect("the record")).expect("json");
    assert_eq!(recorded["split"], json!("dev"));
    assert!(result["results"]["candidate"]["correct"]["n"].as_i64().expect("a count") > 0);
    // Whatever it held, a test is read only after floors that hold, the same runtime and pairs.
    for floor in recorded["floors"].as_object_mut().expect("the floors").values_mut() {
        *floor = json!(true);
    }
    let holds = serde_json::to_vec(&recorded).expect("json");
    std::fs::write(&development, &holds).expect("a record");
    let mut other_runtime = recorded.clone();
    other_runtime["runtime"] = json!({});
    std::fs::write(&development, serde_json::to_vec(&other_runtime).expect("json")).expect("a record");
    refused("differ from the candidate's development", &run.candidate, &run.baseline);
    let mut other_pairs = recorded.clone();
    other_pairs["pairs"]["baseline"]["sha256"] = json!("0");
    std::fs::write(&development, serde_json::to_vec(&other_pairs).expect("json")).expect("a record");
    refused("differ from the candidate's development", &run.candidate, &run.baseline);
    std::fs::write(&development, &holds).expect("a record");
    // No Tatoeba test passed: none at all, one that failed, one of another model.
    refused("passed no Tatoeba test", &run.candidate, &run.baseline);
    passed_sequences(&run, &run.candidate, false);
    refused("passed no Tatoeba test", &run.candidate, &run.baseline);
    passed_sequences(&run, &run.baseline, true);
    refused("passed no Tatoeba test", &run.candidate, &run.baseline);
    passed_sequences(&run, &run.candidate, true);
    // Somebody else is reading.
    std::fs::write(run.ledger.join(".access-lock"), b"").expect("a lock");
    refused("locked", &run.candidate, &run.baseline);
    std::fs::remove_file(run.ledger.join(".access-lock")).expect("the lock goes");
    // A draw that shares messages with a read before.
    let accepted: Vec<String> = MIXED.iter().map(|text| digest(text.as_bytes())).collect();
    std::fs::write(run.ledger.join("earlier.access.json"), serde_json::to_vec(&json!({"pairs": {"candidate": {"sha256": "0"}}, "test_membership": {"messages": [accepted[0]], "questions": []}})).expect("json")).expect("a record");
    let read = run_chat(&run, "test", &output, &run.candidate, &run.baseline).expect("what is left is read");
    assert!(read["messages"].as_i64().expect("a count") > 0);
    assert_eq!(read["split"], json!("test"));
    // Read again it comes from the ledger.
    assert_eq!(run_chat(&run, "test", &output, &run.candidate, &run.baseline).expect("recorded"), read);
    let access: Vec<PathBuf> = std::fs::read_dir(&run.ledger).expect("the ledger").flatten().map(|entry| entry.path()).filter(|path| path.to_string_lossy().ends_with(".result.json")).collect();
    for path in access {
        std::fs::remove_file(path).expect("the record goes");
    }
    refused("no result was recorded", &run.candidate, &run.baseline);
}

#[test]
fn a_chat_test_of_too_few_messages_or_with_other_files_reads_nothing() {
    let run = chat("chat-few", 1000);
    let output = run.folder.join("few.json");
    run_chat(&run, "dev", &output, &run.candidate, &run.baseline).expect("a result");
    let development = run.ledger.join(format!("{}.dev.json", ks_train::corpus::checksum(&run.candidate).expect("a checksum")));
    let mut recorded: Value = serde_json::from_slice(&std::fs::read(&development).expect("the record")).expect("json");
    for floor in recorded["floors"].as_object_mut().expect("the floors").values_mut() {
        *floor = json!(true);
    }
    std::fs::write(&development, serde_json::to_vec(&recorded).expect("json")).expect("a record");
    passed_sequences(&run, &run.candidate, true);
    let said = run_chat(&run, "test", &output, &run.candidate, &run.baseline).expect_err("too few");
    assert!(said.contains("fewer fresh messages"), "{said}");
    // Files that are not those the tree pins.
    std::fs::write(run.directory.join("devchat-dev.messages.jsonl.gz"), gz(&["{\"text\": \"другой\"}".to_owned()])).expect("a file");
    let said = run_chat(&run, "dev", &output, &run.candidate, &run.baseline).expect_err("other files");
    assert!(said.contains("differ from the pinned"), "{said}");
    std::fs::remove_file(run.directory.join("devchat-dev.messages.jsonl.gz")).expect("the file goes");
    assert!(run_chat(&run, "dev", &output, &run.candidate, &run.baseline).is_err());
    // A batch the tree lists, read with the rest of a test.
    assert!(run_chat(&run, "dev", &output, Path::new("/nonexistent"), &run.baseline).is_err());
}

/// The commands as a person runs them.
#[test]
fn the_commands_type_and_capture() {
    let bench = &*BENCH;
    let root = tree();
    let output = bench.folder.join("cli.json");
    let run = trainer()
        .arg("mixed")
        .arg("--tree")
        .arg(&root)
        .arg("--messages")
        .arg(&bench.messages)
        .args(["--count", "4", "--kinds", "comment", "--seed", "cli", "--russian-share", "0.5", "--typo-rate", "0.1", "--prefix-rate", "0.5", "--jobs", "1"])
        .arg("--code")
        .arg(&bench.code)
        .arg("--output")
        .arg(&output)
        .output()
        .expect("the trainer starts");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let edit = trainer().arg("mixed").arg("--tree").arg(&root).arg("--messages").arg(&bench.messages).args(["--count", "2", "--edit", "--jobs", "1"]).arg("--output").arg(&output).arg("--model").arg(&bench.model).output().expect("the trainer starts");
    assert!(edit.status.success(), "{}", String::from_utf8_lossy(&edit.stderr));
    let folder = scratch("capture-cli");
    let path = manifest(&folder, 2, &[None]);
    let captured = trainer().arg("capture").arg("--tree").arg(&root).arg("--manifest").arg(&path).arg("--messages").arg(&bench.folder).arg("--model").arg(&bench.model).args(["--jobs", "1"]).output().expect("the trainer starts");
    assert!(captured.status.success(), "{}", String::from_utf8_lossy(&captured.stderr));
    let verified = trainer().arg("capture").arg("--tree").arg(&root).arg("--manifest").arg(&path).arg("--messages").arg(&bench.folder).arg("--model").arg(&bench.model).args(["--verify", "--only", "source0.jsonl.xz", "--jobs", "1"]).output().expect("the trainer starts");
    assert!(verified.status.success(), "{}", String::from_utf8_lossy(&verified.stderr));
    let refused = trainer().arg("devchat").arg("--tree").arg(&root).args(["--split", "dev"]).arg("--directory").arg(&folder).arg("--candidate").arg(&bench.model).arg("--output").arg(&output).args(["--jobs", "1"]).output().expect("the trainer starts");
    assert!(!refused.status.success());
}
