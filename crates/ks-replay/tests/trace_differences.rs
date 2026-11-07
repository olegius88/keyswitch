//! Traces made to differ from what the engine does, one way each: the trace tool must find each difference, name it, and
//! stop that trace there; and the tool itself, run over folders it cannot read, with workers that die or cannot start.
#![cfg(any(not(debug_assertions), coverage))]

use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BASE: &str = "test_selection_commands/test_selection_commands.EngineSelectionTests.test_the_engine_refuses_where_no_correction_is_made--1.json.gz";
const FAILING_SELECTION: &str = "test_selection_commands/test_selection_commands.EngineSelectionTests.test_a_failure_while_typing_is_an_error_and_an_unconfirmed_switch_is_still_the_engines--1.json.gz";

fn scratch(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("ks-trace-differences-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("a folder");
    path
}

fn said(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

fn read(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("traces").join(name);
    let mut text = String::new();
    flate2::read::GzDecoder::new(std::fs::File::open(path).expect("the trace")).read_to_string(&mut text).expect("gzip");
    serde_json::from_str(&text).expect("json")
}

fn write(folder: &Path, name: &str, trace: &Value) {
    let file = std::fs::File::create(folder.join(format!("{name}.json.gz"))).expect("a file");
    let mut packed = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
    packed.write_all(trace.to_string().as_bytes()).expect("written");
    packed.finish().expect("finished");
}

fn trace_tool() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ks-trace"))
}

/// One entry that changes nothing, with what is seen after it.
fn quiet(clock: &Value, observed: Value) -> Value {
    json!({"k": "call", "n": "subscribe", "a": [], "io": [], "c": clock, "r": null, "o": observed})
}

fn technical(payload: &Value) -> Value {
    json!(format!("TECHNICAL {payload}"))
}

/// The traces, each made from the recorded one with no entries by one change, with the words its result must name.
fn cases() -> Vec<(&'static str, Value, &'static str, &'static str)> {
    let base = read(BASE);
    let clock = base["clock"].clone();
    let with = |edit: &dyn Fn(&mut Value)| {
        let mut trace = base.clone();
        edit(&mut trace);
        trace
    };
    let steps = |steps: Value| with(&|trace: &mut Value| trace["steps"] = steps.clone());
    let quietly = |observed: Value| steps(json!([quiet(&clock, observed)]));
    let initialized: Value = serde_json::from_str(base["init_logs"][0].as_str().expect("a line").trim_start_matches("TECHNICAL ")).expect("an event");
    let mut other_backend = initialized.clone();
    other_backend["backend"] = json!("another backend");
    let mut snapshot = base["snapshot"].clone();
    snapshot["last_action"] = json!("something else");
    let mut own_words = base["snapshot"].clone();
    own_words["last_error"] = json!("История не записана: диск полон");
    let key = json!({"$key": [true, 38, "a", "a", ["a", "ф"], 0, 0, 1, false, false, false]});
    let mut selection = read(FAILING_SELECTION);
    let lines = selection["steps"][3]["o"]["l"].as_array_mut().expect("the logged lines");
    for line in lines.iter_mut() {
        let text = line.as_str().expect("a line").to_owned();
        if text.contains("\"selection_command\"") {
            let mut event: Value = serde_json::from_str(text.trim_start_matches("TECHNICAL ")).expect("an event");
            event["error"] = json!("LayoutSwitchUnconfirmed");
            *line = technical(&event);
        }
    }
    vec![
        ("log-no-object", with(&|trace| trace["init_logs"][0] = json!("TECHNICAL 5")), "differed", "log: ? expected, engine_initialized written"),
        ("log-missing", with(&|trace| trace["init_logs"].as_array_mut().expect("lines").push(technical(&json!({"event": "gone"})))), "differed", "log: missing gone"),
        ("log-no-event", with(&|trace| trace["init_logs"][0] = json!("plain words")), "differed", "log: a line that is no event"),
        ("log-history", with(&|trace| trace["init_logs"][0] = technical(&json!({"event": "history_write_failed", "error": "OSError"}))), "differed", "log: history_write_failed expected, engine_initialized written"),
        ("log-selection", with(&|trace| trace["init_logs"][0] = technical(&json!({"event": "selection_command"}))), "differed", "log: selection_command expected, engine_initialized written"),
        ("log-extra", with(&|trace| trace["init_logs"] = json!([])), "differed", "log: extra engine_initialized"),
        ("log-field", with(&|trace| trace["init_logs"][0] = technical(&other_backend)), "differed", "log: engine_initialized.backend"),
        ("log-error-class", selection, "differed", "log: selection_command.error"),
        ("models-none", with(&|trace| trace["models"] = json!([])), "not_run", "language models "),
        ("settings-missing", with(&|trace| {
            trace["settings"].as_object_mut().expect("settings").remove("enabled");
        }), "not_run", "settings .enabled"),
        ("engine-not-built", with(&|trace| trace["files"]["learning"] = json!(r#"{"rules": {"²:word": {"target_group": 1, "confirmations": 5}}}"#)), "differed", "panic: the engine was not built"),
        ("panic", steps(json!([{"k": "policy", "n": "model", "a": [], "io": [], "c": clock, "o": {}}])), "differed", "panic: index out of bounds"),
        ("step-kind", steps(json!([{"k": "strange", "n": "x", "a": [], "io": [], "c": clock, "o": {}}])), "not_run", "step kind strange"),
        ("call-unrecorded", steps(json!([{"k": "call", "n": "_handle", "a": [key], "io": [], "c": clock, "o": {}}])), "differed", "read where the Python never read it"),
        ("call-left-over", steps(json!([{"k": "call", "n": "subscribe", "a": [], "io": [{"o": "b", "m": "stop", "a": []}], "c": clock, "o": {}}])), "differed", "call: python made stop more"),
        ("raises-another", steps(json!([{"k": "queue", "n": "get", "a": [], "io": [], "c": clock, "r": null, "x": {"message": "another"}, "o": {}}])), "differed", "raises another error"),
        ("rust-raises", steps(json!([{"k": "queue", "n": "get", "a": [], "io": [], "c": clock, "r": null, "o": {}}])), "differed", "rust raises, python does not"),
        ("python-raises", steps(json!([{"k": "call", "n": "subscribe", "a": [], "io": [], "c": clock, "x": {"message": "boom"}, "o": {}}])), "differed", "python raises, rust does not"),
        ("result", steps(json!([{"k": "call", "n": "consumes_key", "a": [key], "io": [], "c": clock, "r": "no such answer", "o": {}}])), "differed", "result of consumes_key"),
        ("step-log", quietly(json!({"l": [technical(&json!({"event": "gone"}))]})), "differed", "log: missing gone"),
        ("snapshot", quietly(json!({"s": snapshot})), "differed", "snapshot.last_action"),
        ("snapshot-own-words", quietly(json!({"s": own_words})), "differed", "snapshot.last_error"),
        ("state", quietly(json!({"i": {"_strokes": ["", 1]}})), "differed", "state._strokes"),
        ("history", quietly(json!({"h": [["ghbdtn", "привет", "app", 0.9]]})), "differed", "history"),
        ("rules", quietly(json!({"L": [{"pattern": "ghbdtn"}]})), "differed", "learning rules"),
        ("personal-count", quietly(json!({"p": {"n": 3, "w": []}})), "differed", "personal words"),
        ("personal-word", quietly(json!({"p": {"n": 0, "w": ["слово"]}})), "differed", "personal words"),
        ("prompt", quietly(json!({"q": {"$dc": "LearningPrompt"}})), "differed", "learning prompt"),
        ("plan", quietly(json!({"P": [{"mode": "automatic"}]})), "differed", "plan.len"),
        ("rule-request", quietly(json!({"R": [{"pattern": "ghbdtn"}]})), "differed", "rule request.len"),
        // What runs and leaves nothing to compare.
        ("model-gone", steps(json!([
            {"k": "policy", "n": "model", "a": [null], "io": [], "c": clock, "o": {}},
            {"k": "policy", "n": "model", "a": [null], "io": [], "c": clock, "o": {}},
            {"k": "call", "n": "subscribe", "a": [], "io": [], "c": clock, "model_attributes": {"answers_letters": true}, "o": {}},
        ])), "passed", ""),
        ("history-file", with(&|trace| {
            trace["files"]["history"] = json!("");
            trace["steps"] = json!([quiet(&clock, json!({}))]);
        }), "passed", ""),
        ("history-step", steps(json!([{"k": "history", "n": "clear", "a": [], "io": [], "c": clock, "o": {}}])), "", ""),
        ("personal-step", steps(json!([{"k": "personal", "n": "add", "a": ["слово"], "io": [], "c": clock, "o": {}}])), "", ""),
        ("engine-prefix", steps(json!([{"k": "engine", "n": "prefix_model", "a": [], "io": [], "c": clock, "o": {}}])), "", ""),
        ("own-models", with(&|trace| trace["models"] = json!([
            {"locale": "en_US", "source": "authored", "frequencies": {"hello": 30, "world": 20}, "bigrams": {"hello world": 3}},
            {"locale": "ru_RU", "source": "authored", "frequencies": {"привет": 30}, "bigrams": {}},
        ])), "", ""),
    ]
}

#[test]
fn every_way_a_trace_can_differ_is_found_and_named() {
    let folder = scratch("cases");
    let cases = cases();
    for (name, trace, _, _) in &cases {
        write(&folder, name, trace);
    }
    let report = folder.join("report.json");
    let output = trace_tool().arg("--all").arg(&folder).args(["--jobs", "1", "--report"]).arg(&report).output().expect("it runs");
    assert_eq!(output.status.code(), Some(1), "a trace differed: {}", said(&output));
    let written: Value = serde_json::from_slice(&std::fs::read(&report).expect("the report")).expect("json");
    let results: HashMap<String, Value> = written["results"]
        .as_array()
        .expect("results")
        .iter()
        .map(|result| {
            let file = result["file"].as_str().expect("a file");
            (Path::new(file).file_name().expect("a name").to_string_lossy().trim_end_matches(".json.gz").to_owned(), result.clone())
        })
        .collect();
    for (name, _, status, cause) in &cases {
        let result = &results[*name];
        if !status.is_empty() {
            assert_eq!(result["status"], json!(status), "{name}: {result}");
        }
        assert!(result["cause"].as_str().unwrap_or("").contains(cause), "{name}: {result}");
    }
    // Strict, a trace recorded over models that are not here fails the run; not strict, it is only counted.
    let models = scratch("models");
    write(&models, "models-none", &cases.iter().find(|case| case.0 == "models-none").expect("the case").1);
    let strict = trace_tool().arg("--all").arg(&models).args(["--jobs", "1", "--strict"]).output().expect("it runs");
    assert_eq!(strict.status.code(), Some(1), "{}", said(&strict));
    let lenient = trace_tool().arg("--all").arg(&models).args(["--jobs", "1"]).output().expect("it runs");
    assert_eq!(lenient.status.code(), Some(0), "{}", said(&lenient));
    // One trace alone, entry by entry.
    let alone = trace_tool().arg(folder.join("model-gone.json.gz")).arg("--verbose").output().expect("it runs");
    assert!(alone.status.success() && said(&alone).contains("step 2"), "{}", said(&alone));
    std::fs::remove_dir_all(folder).expect("cleaned");
    std::fs::remove_dir_all(models).expect("cleaned");
}

#[test]
fn the_tool_over_what_it_cannot_read_or_run() {
    let folder = scratch("tool");
    write(&folder, "quiet", &read(BASE));
    let traces_in = |output: &Output| said(output).lines().next().unwrap_or("").to_owned();
    // A folder that is not there holds no trace.
    let nowhere = trace_tool().args(["--all", "/nonexistent/traces", "--jobs", "1"]).output().expect("it runs");
    assert!(nowhere.status.success() && traces_in(&nowhere).starts_with("traces 0 "), "{}", said(&nowhere));
    // No folder to write the list of a worker into: no worker runs.
    let unlisted = trace_tool().arg("--all").arg(&folder).args(["--jobs", "1"]).env("TMPDIR", "/nonexistent/tmp").output().expect("it runs");
    assert!(unlisted.status.success() && traces_in(&unlisted).starts_with("traces 0 "), "{}", said(&unlisted));
    // No descriptors left for a worker's pipes: it cannot be started.
    let starved = Command::new("sh").arg("-c").arg("ulimit -n 5 && exec \"$0\" \"$@\"").arg(env!("CARGO_BIN_EXE_ks-trace")).arg("--all").arg(&folder).args(["--jobs", "1"]).output().expect("it runs");
    assert!(starved.status.success() && traces_in(&starved).starts_with("traces 0 "), "{}", said(&starved));
    // A worker that gives no answer for its trace: run through the dynamic loader, the tool starts the loader as its
    // worker, which answers nothing.
    let loader = ["/lib64/ld-linux-x86-64.so.2", "/lib/ld-linux-aarch64.so.1"].into_iter().find(|path| Path::new(path).exists()).expect("the dynamic loader");
    let report = folder.join("report.json");
    let dead = Command::new(loader).arg(env!("CARGO_BIN_EXE_ks-trace")).arg("--all").arg(&folder).args(["--jobs", "1", "--report"]).arg(&report).output().expect("it runs");
    assert!(dead.status.success(), "{}", said(&dead));
    let written: Value = serde_json::from_slice(&std::fs::read(&report).expect("the report")).expect("json");
    assert_eq!(written["results"][0]["cause"], json!("the worker died on this trace"), "{written}");
    std::fs::remove_dir_all(folder).expect("cleaned");
}
