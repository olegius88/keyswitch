//! The replay tools as a person runs them: what they write, what they compare, and how they stop.
#![cfg(any(not(debug_assertions), coverage))]

use serde_json::{Value, json};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

fn scratch() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!("ks-replay-commands-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&path).expect("a folder");
    path
}

fn said(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

fn battery(folder: &Path) -> PathBuf {
    let packed = std::fs::File::open(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/battery-public.jsonl.gz")).expect("the fixture");
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(packed).read_to_end(&mut bytes).expect("gzip");
    let path = folder.join("battery.jsonl");
    std::fs::write(&path, bytes).expect("the copy");
    path
}

fn replay() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ks-replay"))
}

#[test]
fn a_replay_writes_its_report_and_compares_it_with_another() {
    let folder = scratch();
    let battery = battery(&folder);
    let report = folder.join("report.json");
    let run = |reference: Option<&Path>| {
        let mut command = replay();
        command.arg(&battery).arg(&report).args(["--limit", "4"]);
        if let Some(path) = reference {
            command.arg("--compare").arg(path);
        }
        command.output().expect("it runs")
    };
    assert!(run(None).status.success());
    let written: Value = serde_json::from_slice(&std::fs::read(&report).expect("a report")).expect("json");
    assert!(!written["rows"].as_array().expect("rows").is_empty());
    // The same words again, then a reference that differs in each part it holds.
    let reference = folder.join("reference.json");
    std::fs::write(&reference, serde_json::to_vec(&written).expect("json")).expect("a file");
    let same = run(Some(&reference));
    assert!(same.status.success() && said(&same).contains("differing 0"), "{}", said(&same));
    let differs = |edit: &dyn Fn(&mut Value), words: &str| {
        let mut changed = written.clone();
        edit(&mut changed);
        std::fs::write(&reference, serde_json::to_vec(&changed).expect("json")).expect("a file");
        let answer = run(Some(&reference));
        assert!(said(&answer).contains(words), "{words}: {}", said(&answer));
        answer.status.success()
    };
    assert!(!differs(&|value| value["rows"][0][6] = json!("другое"), "differing 1"));
    assert!(!differs(&|value| value["errors"] = json!(["ошибка"]), "DIFFER"));
    assert!(!differs(&|value| value["rows"].as_array_mut().expect("rows").truncate(1), "vs 1,"));
    assert!(differs(&|value| value["counts"] = json!({"other": 1}), "count other"));
    assert!(differs(&|value| value["counts"] = json!(1), "counts DIFFER"));
    assert!(differs(&|value| value["openings"] = json!([]), "openings DIFFER"));
    // The technical log.
    let logged = replay().arg(&battery).arg(&report).args(["--limit", "1"]).env("FIELD_DEBUG", "1").output().expect("it runs");
    assert!(logged.status.success(), "{}", said(&logged));
    let log = folder.join("report.json.log");
    assert!(log.is_file() || folder.join("report.log").is_file());
    // How it is asked.
    assert_eq!(replay().output().expect("it runs").status.code(), Some(2));
    assert_eq!(replay().arg(&battery).output().expect("it runs").status.code(), Some(2));
    // A limit of nothing is no limit, and an empty battery is fine.
    std::fs::write(&battery, "").expect("a file");
    assert!(replay().arg(&battery).arg(&report).args(["--limit", "0"]).output().expect("it runs").status.success());
}

fn trace() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ks-trace"))
}

fn traces() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("traces")
}

fn first_trace() -> PathBuf {
    let folder = traces().join("test_action_boundary");
    std::fs::read_dir(folder).expect("the traces").flatten().map(|entry| entry.path()).min().expect("a trace")
}

#[test]
fn a_trace_runs_alone_with_many_or_as_a_worker() {
    assert_eq!(trace().output().expect("it runs").status.code(), Some(2));
    let one = first_trace();
    let answer = trace().arg(&one).output().expect("it runs");
    let result: Value = serde_json::from_slice(&answer.stdout).expect("json");
    assert!(result["status"].is_string(), "{result}");
    assert!(trace().arg(&one).arg("--verbose").output().expect("it runs").status.code().is_some());
    // Some of them, with a report.
    let folder = scratch();
    let report = folder.join("report.json");
    let some = trace().args(["--all"]).arg(traces()).args(["--jobs", "2", "--filter", "test_action_boundary", "--strict"]).arg("--report").arg(&report).output().expect("it runs");
    assert!(some.status.success(), "{}", said(&some));
    assert!(serde_json::from_slice::<Value>(&std::fs::read(&report).expect("a report")).expect("json")["results"].is_array());
    assert!(trace().args(["--all"]).arg(traces()).args(["--filter", "no such trace"]).output().expect("it runs").status.success());
    // What it cannot read is said so, and counts against it.
    let empty_folder = folder.join("own");
    std::fs::create_dir_all(empty_folder.join("deeper")).expect("a folder");
    std::fs::write(empty_folder.join("deeper/broken.json.gz"), b"not gzip").expect("a file");
    std::fs::write(empty_folder.join("notes.txt"), b"").expect("a file");
    let broken = trace().arg(empty_folder.join("deeper/broken.json.gz")).output().expect("it runs");
    assert_ne!(broken.status.code(), Some(0), "{}", said(&broken));
    let all = trace().args(["--all"]).arg(&empty_folder).args(["--jobs", "1"]).output().expect("it runs");
    println!("{}", said(&all));
    // The worker, as the others start it.
    let list = folder.join("list.txt");
    std::fs::write(&list, format!("{}\n", one.display())).expect("a file");
    let worker = trace().arg("--worker").arg(&list).output().expect("it runs");
    assert!(worker.status.success(), "{}", said(&worker));
    assert!(serde_json::from_slice::<Value>(worker.stdout.split(|byte| *byte == b'\n').next().expect("a line")).expect("json")["status"].is_string());
}
