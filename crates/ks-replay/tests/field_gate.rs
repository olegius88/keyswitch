//! The field regression gate as the release runs it: over reports of a replay stub, every way it can stop.
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

const TIME: &str = "2026-10-15 10:00:00,000";

fn scratch() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!("ks-field-gate-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&path).expect("a folder");
    path
}

fn row(index: i64, shown: &str) -> Value {
    json!([0, index, TIME, "ghbdtn", "привет", "привет", shown, false, "0.66.2", false])
}

struct Gate {
    root: PathBuf,
    released: PathBuf,
    replay: PathBuf,
    manifest: PathBuf,
}

/// A tree with one battery, the report of the release and a replay that gives the rows it is told to.
fn gate() -> Gate {
    let root = scratch();
    let (released, answers) = (root.join("released"), root.join("answers"));
    std::fs::create_dir_all(&released).expect("a folder");
    std::fs::create_dir_all(&answers).expect("a folder");
    let replay = root.join("replay.sh");
    std::fs::write(&replay, format!("#!/bin/sh\nname=$(basename \"$2\")\n[ -z \"$KS_REPLAY_FAILS\" ] || exit 1\ncp {}/\"$name\" \"$2\"\n", answers.display())).expect("a stub");
    std::fs::set_permissions(&replay, std::fs::Permissions::from_mode(0o755)).expect("a mode");
    std::fs::write(root.join("pc.jsonl"), b"").expect("a battery");
    let manifest = root.join("manifest.json");
    std::fs::write(&manifest, json!({"batteries": {"pc": "pc.jsonl"}, "verdicts": "verdicts.json"}).to_string()).expect("a manifest");
    std::fs::write(root.join("verdicts.json"), b"{}").expect("no verdicts");
    Gate { root, released, replay, manifest }
}

impl Gate {
    fn rows(&self, released: &[Value], candidate: &[Value]) {
        std::fs::write(self.released.join("pc.json"), json!({"rows": released}).to_string()).expect("a report");
        std::fs::write(self.root.join("answers/pc.json"), json!({"rows": candidate}).to_string()).expect("a report");
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_field-gate"));
        command.arg("--root").arg(&self.root).arg("--released").arg(&self.released).arg("--manifest").arg(&self.manifest).arg("--replay").arg(&self.replay).args(["--jobs", "2"]);
        command
    }

    fn run(&self) -> Output {
        self.command().output().expect("it runs")
    }

    fn verdicts(&self, value: &Value) {
        std::fs::write(self.root.join("verdicts.json"), value.to_string()).expect("verdicts");
    }
}

fn said(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

fn stops(output: &Output, code: i32, words: &str) {
    assert!(output.status.code() == Some(code) && said(output).contains(words), "{code} {words}: {}", said(output));
}

#[test]
fn a_word_that_stays_right_passes_and_a_word_that_breaks_does_not() {
    let gate = gate();
    gate.rows(&[row(0, "привет")], &[row(0, "привет")]);
    stops(&gate.run(), 0, "passed");
    gate.rows(&[row(0, "привет")], &[row(0, "ghbdtn")]);
    stops(&gate.run(), 1, "NOT passed");
    // What it fixed and what it broke go to the record.
    let record = gate.root.join("record.json");
    let answer = gate.command().arg("--record").arg(&record).output().expect("it runs");
    stops(&answer, 1, "broken=1");
    let written: Value = serde_json::from_slice(&std::fs::read(&record).expect("a record")).expect("json");
    assert_eq!(written["broken"][0]["candidate"], json!("ghbdtn"));
}

#[test]
fn the_verdicts_are_checked_before_they_count() {
    let gate = gate();
    gate.rows(&[row(0, "ghbdtn")], &[row(0, "ghbdtn")]);
    let key = format!("pc|0|{TIME}|x");
    let good = json!({"verdict": "keep", "typed": "ghbdtn", "by": "llm", "date": "2026-10-16", "reason_code": "garbage-in-prompt", "reason": "because"});
    gate.verdicts(&json!({&key: good}));
    stops(&gate.run(), 0, "verdicts_used=1");
    for (field, value) in [("verdict", json!("maybe")), ("by", json!("nobody")), ("date", json!("")), ("reason_code", json!("other")), ("reason", json!(""))] {
        let mut changed = good.clone();
        changed[field] = value;
        gate.verdicts(&json!({&key: changed}));
        stops(&gate.run(), 2, "lacks its verdict");
    }
    // The reason of the first review is its code.
    let mut review = good.clone();
    review["reason_code"] = json!("review-2026-10-15");
    review["reason"] = json!("");
    gate.verdicts(&json!({&key: review}));
    stops(&gate.run(), 0, "passed");
    let mut neither = good.clone();
    neither["typed"] = json!("other");
    gate.verdicts(&json!({&key: neither}));
    stops(&gate.run(), 2, "names neither reading");
    gate.verdicts(&json!(1));
    stops(&gate.run(), 2, "no table");
    std::fs::write(gate.root.join("verdicts.json"), b"not json").expect("a file");
    stops(&gate.run(), 2, "no JSON");
    std::fs::remove_file(gate.root.join("verdicts.json")).expect("the file goes");
    stops(&gate.run(), 2, "cannot be read");
}

#[test]
fn what_it_reads_has_to_be_there() {
    let gate = gate();
    gate.rows(&[row(0, "привет")], &[row(0, "привет")]);
    stops(&Command::new(env!("CARGO_BIN_EXE_field-gate")).output().expect("it runs"), 2, "--root is required");
    stops(&Command::new(env!("CARGO_BIN_EXE_field-gate")).arg("--root").arg(&gate.root).output().expect("it runs"), 2, "--released is required");
    stops(&Command::new(env!("CARGO_BIN_EXE_field-gate")).arg("--root").arg(&gate.root).arg("--released").arg(&gate.released).output().expect("it runs"), 2, "no field manifest");
    std::fs::write(&gate.manifest, b"not json").expect("a file");
    stops(&gate.run(), 2, "no JSON");
    std::fs::write(&gate.manifest, b"{}").expect("a file");
    stops(&gate.run(), 2, "names no battery");
    std::fs::write(&gate.manifest, json!({"batteries": {"pc": "pc.jsonl"}, "verdicts": "verdicts.json"}).to_string()).expect("a file");
    stops(&gate.command().env("KS_REPLAY_FAILS", "1").output().expect("it runs"), 2, "failed");
    // Reports that are no reports, that say a word twice, or that hold other words.
    std::fs::write(gate.released.join("pc.json"), b"not json").expect("a file");
    stops(&gate.run(), 2, "no replay report");
    std::fs::write(gate.released.join("pc.json"), b"{}").expect("a file");
    stops(&gate.run(), 2, "no replay report");
    gate.rows(&[row(0, "привет"), row(0, "привет")], &[row(0, "привет")]);
    stops(&gate.run(), 2, "one word twice");
    gate.rows(&[row(0, "привет")], &[row(1, "привет")]);
    stops(&gate.run(), 2, "different words");
    std::fs::remove_file(gate.released.join("pc.json")).expect("the file goes");
    stops(&gate.run(), 2, "cannot be read");
    // A report of the older shape, with no field for waiting words.
    let old = json!([0, 0, TIME, "ghbdtn", "привет", "привет", "привет", false, "0.40.0"]);
    gate.rows(std::slice::from_ref(&old), std::slice::from_ref(&old));
    stops(&gate.run(), 0, "passed");
}

#[test]
fn the_replay_program_beside_it_is_the_one_it_runs_by_default() {
    let gate = gate();
    gate.rows(&[row(0, "привет")], &[row(0, "привет")]);
    let default = Command::new(env!("CARGO_BIN_EXE_field-gate")).arg("--root").arg(&gate.root).arg("--released").arg(&gate.released).arg("--manifest").arg(&gate.manifest).output().expect("it runs");
    assert!(default.status.code() == Some(2) || default.status.code() == Some(0), "{}", said(&default));
}

#[test]
fn a_verdict_converts_a_word_or_takes_it_out() {
    let gate = gate();
    gate.rows(&[row(0, "ghbdtn")], &[row(0, "привет")]);
    let key = format!("pc|0|{TIME}|x");
    let verdict = |name: &str| json!({&key: {"verdict": name, "typed": "ghbdtn", "by": "llm", "date": "2026-10-16", "reason_code": "russian-typo", "reason": "because"}});
    gate.verdicts(&verdict("convert"));
    stops(&gate.run(), 0, "fixed=1");
    gate.verdicts(&verdict("ambiguous"));
    stops(&gate.run(), 0, "excluded=1");
    gate.verdicts(&verdict("partial"));
    stops(&gate.run(), 0, "excluded=1");
    gate.verdicts(&verdict("keep"));
    stops(&gate.run(), 1, "broken=1");
}

#[test]
fn without_a_scratch_folder_nothing_is_replayed() {
    let gate = gate();
    gate.rows(&[row(0, "привет")], &[row(0, "привет")]);
    let file = gate.root.join("a file");
    std::fs::write(&file, b"").expect("a file");
    stops(&gate.command().env("TMPDIR", &file).output().expect("it runs"), 2, "no scratch folder");
}

/// A folder of files named as the Hunspell library that are no libraries: a process started with it on its library path
/// finds no Hunspell, as on a machine without it.
fn no_hunspell(root: &std::path::Path) -> PathBuf {
    let folder = root.join("no-hunspell");
    std::fs::create_dir_all(&folder).expect("a folder");
    for name in ["libhunspell-1.7.so.0", "libhunspell-1.7.so", "libhunspell.so.0", "libhunspell.so"] {
        std::fs::write(folder.join(name), b"no library").expect("a file");
    }
    folder
}

#[test]
fn the_speller_is_asked_of_the_replay_and_has_to_be_there() {
    let gate = gate();
    gate.rows(&[row(0, "привет")], &[row(0, "привет")]);
    let asked = gate.root.join("asked.txt");
    std::fs::write(&gate.replay, format!("#!/bin/sh\necho \"$@\" >> {}\nname=$(basename \"$2\")\ncp {}/\"$name\" \"$2\"\n", asked.display(), gate.root.join("answers").display())).expect("a stub");
    stops(&gate.run(), 0, "passed");
    let plain = std::fs::read_to_string(&asked).expect("the arguments");
    assert!(!plain.contains("--speller"), "{plain}");
    std::fs::remove_file(&asked).expect("the file goes");
    let spelling = gate.command().arg("--speller").output().expect("it runs");
    let ready = ks_replay::harness::spellers_ready(&ks_hunspell::hunspell_speller(ks_config::history::data_dir()));
    if ready.is_ok() {
        stops(&spelling, 0, "passed");
        let asked = std::fs::read_to_string(&asked).expect("the arguments");
        assert!(asked.trim_end().ends_with("--speller"), "{asked}");
    } else {
        stops(&spelling, 2, "--speller: no Hunspell speller");
    }
    // Where there is no Hunspell, the gate says so before it replays anything.
    let missing = gate.command().arg("--speller").env("LD_LIBRARY_PATH", no_hunspell(&gate.root)).output().expect("it runs");
    stops(&missing, 2, "--speller: no Hunspell speller for en_US");
}
