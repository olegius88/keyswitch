//! The field regression gate: the owner's typing, replayed through this engine against the released one's reports. No
//! word that was right may come out wrong. The batteries and the reports name real words: they stay outside the repository,
//! and the output names files and counts, never a word.
//!
//!     field-gate --root <tree with the private data> --released <folder of reports> [--manifest <file>]
//!                [--replay <ks-replay binary>] [--jobs N] [--record <file>]
#![forbid(unsafe_code)]

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

const KEEP: &str = "keep";
const EXCLUDED: [&str; 2] = ["ambiguous", "partial"];
const OWNER: &str = "owner";
const AUTHORS: [&str; 2] = ["llm", OWNER];
const REVIEW_BEFORE_GATE: &str = "review-2026-10-15";
const REASON_CODES: [&str; 8] = ["garbage-in-prompt", "code-identifier", "retyped-elsewhere", "russian-typo", "mixed-script", "remote-echo", "owner-decision", REVIEW_BEFORE_GATE];
const ERROR_EXIT_CODE: i32 = 2;

/// One word of a battery as an engine typed it.
#[derive(Clone)]
struct Row {
    stream: i64,
    index: i64,
    time: String,
    typed: String,
    other: String,
    intended: String,
    shown: String,
    owner_labelled: bool,
    version: String,
    /// The word was asked about at a pause and typed on after it: left as typed it is right too.
    waits: bool,
}

type Rows = BTreeMap<(i64, i64), Row>;
type Verdicts = std::collections::HashMap<(String, i64, String), Value>;

fn fail(message: impl std::fmt::Display) -> ! {
    eprintln!("field gate: {message}");
    std::process::exit(ERROR_EXIT_CODE);
}

fn read_rows(report: &Path) -> Rows {
    let name = report.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let text = std::fs::read_to_string(report).unwrap_or_else(|_| fail(format!("{name} cannot be read")));
    let payload: Value = serde_json::from_str(&text).unwrap_or_else(|_| fail(format!("{name} is no replay report with rows")));
    let mut rows = Rows::new();
    for entry in payload["rows"].as_array().unwrap_or_else(|| fail(format!("{name} is no replay report with rows"))) {
        let text = |index: usize| entry[index].as_str().unwrap_or_default().to_owned();
        let row = Row {
            stream: entry[0].as_i64().unwrap_or(-1),
            index: entry[1].as_i64().unwrap_or(-1),
            time: text(2),
            typed: text(3),
            other: text(4),
            intended: text(5),
            shown: text(6),
            owner_labelled: entry[7].as_bool().unwrap_or(false),
            version: text(8),
            // A report of a replay script before waiting words had nine fields.
            waits: entry.get(9).and_then(Value::as_bool).unwrap_or(false),
        };
        if rows.insert((row.stream, row.index), row).is_some() {
            fail(format!("{name} holds one word twice"));
        }
    }
    rows
}

fn read_verdicts(path: &Path) -> Verdicts {
    let text = std::fs::read_to_string(path).unwrap_or_else(|_| fail("the verdicts cannot be read"));
    let payload: Value = serde_json::from_str(&text).unwrap_or_else(|_| fail("the verdicts are no JSON"));
    let mut verdicts = Verdicts::new();
    for (key, value) in payload.as_object().unwrap_or_else(|| fail("the verdicts are no table")) {
        let mut parts = key.splitn(4, '|');
        let (battery, stream, time) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        let verdict = value["verdict"].as_str().unwrap_or("");
        let reason = value["reason_code"].as_str().unwrap_or("");
        let valid = (verdict == KEEP || verdict == "convert" || EXCLUDED.contains(&verdict))
            && AUTHORS.contains(&value["by"].as_str().unwrap_or(""))
            && value["date"].as_str().is_some_and(|date| !date.is_empty())
            && REASON_CODES.contains(&reason)
            && (reason == REVIEW_BEFORE_GATE || value["reason"].as_str().is_some_and(|text| !text.is_empty()));
        if !valid {
            fail(format!("a verdict of battery {battery}, stream {stream} lacks its verdict, author, date or reason"));
        }
        verdicts.insert((battery.to_owned(), stream.parse().unwrap_or(-1), time.to_owned()), value.clone());
    }
    verdicts
}

/// Whether an `llm` verdict would change a word the owner labelled by hand: only the owner may.
fn owner_overruled(row: &Row, verdict: &Value) -> bool {
    let marked = verdict["typed"].as_str().unwrap_or("");
    if marked != row.typed && marked != row.other {
        fail(format!("a verdict of stream {} names neither reading of its word", row.stream));
    }
    if !row.owner_labelled || verdict["by"].as_str() == Some(OWNER) {
        return false;
    }
    let name = verdict["verdict"].as_str().unwrap_or("");
    if EXCLUDED.contains(&name) {
        return true;
    }
    let goal = if name == KEEP {
        marked
    } else if marked == row.typed {
        &row.other
    } else {
        &row.typed
    };
    goal != row.intended
}

/// The text the word should end as: the verdict's when there is one, else the reading the log implies; None when the
/// verdict takes the word out.
fn target(row: &Row, verdict: Option<&Value>) -> Option<String> {
    let Some(verdict) = verdict.filter(|verdict| !owner_overruled(row, verdict)) else {
        return Some(row.intended.clone());
    };
    let name = verdict["verdict"].as_str().unwrap_or("");
    if EXCLUDED.contains(&name) {
        return None;
    }
    let marked = verdict["typed"].as_str().unwrap_or("");
    Some(if name == KEEP {
        marked.to_owned()
    } else if marked == row.typed {
        row.other.clone()
    } else {
        row.typed.clone()
    })
}

fn reached(row: &Row, goal: &str) -> bool {
    row.shown == goal || (row.waits && row.shown == row.typed)
}

#[derive(Default)]
struct Comparison {
    rows: u64,
    excluded: u64,
    released_right: u64,
    candidate_right: u64,
    broken: Vec<Value>,
    fixed: Vec<Value>,
    verdicts_used: u64,
    needs_owner: u64,
}

fn compare(released: &BTreeMap<String, Rows>, candidate: &BTreeMap<String, Rows>, verdicts: &Verdicts) -> Comparison {
    let mut result = Comparison::default();
    for (battery, before) in released {
        let after = &candidate[battery];
        if !before.keys().eq(after.keys()) {
            fail(format!("the engines typed different words of battery {battery}"));
        }
        let mut diverged = HashSet::new();
        for (key, old) in before {
            let new = &after[key];
            let mut verdict = verdicts.get(&(battery.clone(), old.stream, old.time.clone()));
            let goal = target(old, verdict);
            if verdict.is_some_and(|verdict| owner_overruled(old, verdict)) {
                result.needs_owner += 1;
                verdict = None;
            }
            let first = old.shown != new.shown && !diverged.contains(&old.stream);
            if old.shown != new.shown {
                diverged.insert(old.stream);
            }
            let Some(goal) = goal else {
                result.excluded += 1;
                continue;
            };
            result.rows += 1;
            let right = (reached(old, &goal), reached(new, &goal));
            result.released_right += u64::from(right.0);
            result.candidate_right += u64::from(right.1);
            if verdict.is_some() && (reached(old, &old.intended) != right.0 || reached(new, &new.intended) != right.1) {
                result.verdicts_used += 1;
            }
            let word = || {
                json!({"battery": battery, "stream": old.stream, "index": old.index, "time": old.time, "typed": old.typed, "target": goal,
                       "released": old.shown, "candidate": new.shown, "owner_labelled": old.owner_labelled, "logged_by": old.version,
                       "first_divergence": first, "waits": old.waits})
            };
            match right {
                (true, false) => result.broken.push(word()),
                (false, true) => result.fixed.push(word()),
                _ => {}
            }
        }
    }
    result
}

fn option(arguments: &[String], name: &str) -> Option<String> {
    arguments.iter().position(|argument| argument == name).and_then(|index| arguments.get(index + 1)).cloned()
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let root = PathBuf::from(option(&arguments, "--root").unwrap_or_else(|| fail("--root is required")));
    let released_folder = PathBuf::from(option(&arguments, "--released").unwrap_or_else(|| fail("--released is required")));
    let manifest_path = option(&arguments, "--manifest").map_or_else(|| root.join(".t/field/manifest.json"), PathBuf::from);
    let replay = option(&arguments, "--replay").map_or_else(
        || std::env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join("ks-replay"))).unwrap_or_else(|| PathBuf::from("ks-replay")),
        PathBuf::from,
    );
    let jobs: usize = option(&arguments, "--jobs").and_then(|text| text.parse().ok()).unwrap_or(4);
    let text = std::fs::read_to_string(&manifest_path).unwrap_or_else(|_| fail(format!("no field manifest at {}: the batteries are not on this machine", manifest_path.display())));
    let manifest: Value = serde_json::from_str(&text).unwrap_or_else(|_| fail("the manifest is no JSON"));
    let batteries: Vec<(String, PathBuf)> = manifest["batteries"].as_object().map_or_else(Vec::new, |table| {
        table.iter().map(|(name, path)| (name.clone(), root.join(path.as_str().unwrap_or_default()))).collect()
    });
    if batteries.is_empty() {
        fail("the field manifest names no battery");
    }
    let scratch = std::env::temp_dir().join(format!("field-gate-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap_or_else(|_| fail("no scratch folder"));
    let (next, failures) = (AtomicUsize::new(0), Mutex::new(Vec::new()));
    std::thread::scope(|scope| {
        for _ in 0..jobs.max(1) {
            scope.spawn(|| {
                while let Some((name, battery)) = batteries.get(next.fetch_add(1, Ordering::SeqCst)) {
                    let status = Command::new(&replay).arg(battery).arg(scratch.join(format!("{name}.json"))).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
                    if !status.is_ok_and(|status| status.success()) {
                        failures.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(name.clone());
                    }
                }
            });
        }
    });
    let failed = failures.into_inner().unwrap_or_default();
    if !failed.is_empty() {
        let _ = std::fs::remove_dir_all(&scratch);
        fail(format!("the replay of batteries {failed:?} failed"));
    }
    let (mut released, mut candidate) = (BTreeMap::new(), BTreeMap::new());
    for (name, _) in &batteries {
        released.insert(name.clone(), read_rows(&released_folder.join(format!("{name}.json"))));
        candidate.insert(name.clone(), read_rows(&scratch.join(format!("{name}.json"))));
    }
    let _ = std::fs::remove_dir_all(&scratch);
    let verdicts = read_verdicts(&root.join(manifest["verdicts"].as_str().unwrap_or_default()));
    let result = compare(&released, &candidate, &verdicts);
    if let Some(record) = option(&arguments, "--record") {
        let payload = json!({"broken": result.broken, "fixed": result.fixed});
        let _ = std::fs::write(record, ks_text::json_dumps(&payload, &ks_text::JsonOptions { ensure_ascii: false, indent: Some(ks_text::Indent::Spaces(1)), ..ks_text::JsonOptions::default() }));
    }
    println!(
        "field regression gate: rows={}, excluded={}, released_right={}, candidate_right={}, fixed={}, broken={}, verdicts_used={}, needs_owner={}",
        result.rows, result.excluded, result.released_right, result.candidate_right, result.fixed.len(), result.broken.len(), result.verdicts_used, result.needs_owner,
    );
    let passed = result.broken.is_empty() && result.needs_owner == 0;
    println!("field regression gate: {}", if passed { "passed" } else { "NOT passed" });
    std::process::exit(i32::from(!passed));
}
