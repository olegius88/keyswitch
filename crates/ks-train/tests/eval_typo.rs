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
