//! The field regression gate: the owner's typing, replayed through this engine against the released one's reports. No
//! word that was right may come out wrong. The batteries and the reports name real words: they stay outside the repository,
//! and the output names files and counts, never a word.
//!
//!     field-gate --root <tree with the private data> --released <folder of reports> [--manifest <file>]
//!                [--replay <ks-replay binary>] [--jobs N] [--record <file>] [--speller]
//!
//! `--speller` replays the candidate with the Hunspell dictionaries the installed program uses (`ks-replay --speller`).
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
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

/// A folder only this user can open: the replays write the owner's typing into it, and the temporary folder is shared.
fn private_folder(path: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(path)
}

/// A file only this user can read, for the record of the words that changed.
fn private_write(path: &Path, text: &str) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    std::io::Write::write_all(&mut options.open(path)?, text.as_bytes())
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
    let speller = arguments.iter().any(|argument| argument == "--speller");
    if speller && let Err(error) = ks_replay::harness::spellers_ready(&ks_hunspell::hunspell_speller(ks_config::history::data_dir())) {
        fail(format!("--speller: {error}"));
    }
    let speller = speller.then_some("--speller");
    let text = std::fs::read_to_string(&manifest_path).unwrap_or_else(|_| fail(format!("no field manifest at {}: the batteries are not on this machine", manifest_path.display())));
    let manifest: Value = serde_json::from_str(&text).unwrap_or_else(|_| fail("the manifest is no JSON"));
    let batteries: Vec<(String, PathBuf)> = manifest["batteries"].as_object().map_or_else(Vec::new, |table| {
        table.iter().map(|(name, path)| (name.clone(), root.join(path.as_str().unwrap_or_default()))).collect()
    });
    if batteries.is_empty() {
        fail("the field manifest names no battery");
    }
    let scratch = std::env::temp_dir().join(format!("field-gate-{}", std::process::id()));
    private_folder(&scratch).unwrap_or_else(|_| fail("no scratch folder"));
    let (next, failures) = (AtomicUsize::new(0), Mutex::new(Vec::new()));
    std::thread::scope(|scope| {
        for _ in 0..jobs.max(1) {
            scope.spawn(|| {
                while let Some((name, battery)) = batteries.get(next.fetch_add(1, Ordering::SeqCst)) {
                    let status = Command::new(&replay).arg(battery).arg(scratch.join(format!("{name}.json"))).args(speller).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
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
        let _ = private_write(Path::new(&record), &ks_text::json_dumps(&payload, &ks_text::JsonOptions { ensure_ascii: false, indent: Some(ks_text::Indent::Spaces(1)), ..ks_text::JsonOptions::default() }));
    }
    println!(
        "field regression gate: rows={}, excluded={}, released_right={}, candidate_right={}, fixed={}, broken={}, verdicts_used={}, needs_owner={}",
        result.rows, result.excluded, result.released_right, result.candidate_right, result.fixed.len(), result.broken.len(), result.verdicts_used, result.needs_owner,
    );
    let passed = result.broken.is_empty() && result.needs_owner == 0;
    println!("field regression gate: {}", if passed { "passed" } else { "NOT passed" });
    std::process::exit(i32::from(!passed));
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    const TIME: &str = "2026-10-15 10:00:00,000";
    const LATER: &str = "2026-10-15 10:00:01,000";

    fn row(index: i64, shown: &str) -> Row {
        Row { stream: 0, index, time: TIME.to_owned(), typed: "ghbdtn".to_owned(), other: "привет".to_owned(), intended: "привет".to_owned(), shown: shown.to_owned(), owner_labelled: false, version: "0.0.0".to_owned(), waits: false }
    }

    fn owned() -> Row {
        Row { owner_labelled: true, ..row(0, "x") }
    }

    fn verdict(name: &str, by: &str) -> Value {
        json!({"verdict": name, "typed": "ghbdtn", "by": by, "date": "2026-10-16", "reason_code": "garbage-in-prompt", "reason": "a reason"})
    }

    fn battery(rows: Vec<Row>) -> BTreeMap<String, Rows> {
        BTreeMap::from([("pc".to_owned(), rows.into_iter().map(|row| ((row.stream, row.index), row)).collect())])
    }

    #[test]
    fn the_reading_the_log_implies_holds_without_a_verdict() {
        assert_eq!(target(&row(0, "ghbdtn"), None).as_deref(), Some("привет"));
    }

    #[test]
    fn a_verdict_reads_the_word_in_either_layout_it_was_typed() {
        assert_eq!(target(&row(0, "x"), Some(&verdict(KEEP, "llm"))).as_deref(), Some("ghbdtn"));
        assert_eq!(target(&row(0, "x"), Some(&verdict("convert", "llm"))).as_deref(), Some("привет"));
        // The same word typed in the other layout by another tree: the verdict names the same text.
        let swapped = Row { typed: "привет".to_owned(), other: "ghbdtn".to_owned(), ..row(0, "x") };
        assert_eq!(target(&swapped, Some(&verdict(KEEP, "llm"))).as_deref(), Some("ghbdtn"));
        assert_eq!(target(&swapped, Some(&verdict("convert", "llm"))).as_deref(), Some("привет"));
        assert_eq!(target(&row(0, "x"), Some(&verdict("ambiguous", "llm"))), None);
    }

    #[test]
    fn only_the_owner_reads_a_word_the_owner_labelled_otherwise() {
        // A verdict of a model against the owner's own label, or taking the word out, counts for nothing.
        assert!(owner_overruled(&owned(), &verdict(KEEP, "llm")));
        assert_eq!(target(&owned(), Some(&verdict(KEEP, "llm"))).as_deref(), Some("привет"));
        assert!(owner_overruled(&owned(), &verdict("ambiguous", "llm")));
        assert_eq!(target(&owned(), Some(&verdict("ambiguous", "llm"))).as_deref(), Some("привет"));
        assert_eq!(target(&owned(), Some(&verdict(KEEP, OWNER))).as_deref(), Some("ghbdtn"));
        assert_eq!(target(&owned(), Some(&verdict("ambiguous", OWNER))), None);
        // A verdict that agrees with the owner's label needs nobody.
        assert_eq!(target(&owned(), Some(&verdict("convert", "llm"))).as_deref(), Some("привет"));
        assert!(!owner_overruled(&owned(), &verdict("convert", "llm")));
    }

    #[test]
    fn a_word_the_owner_labelled_and_a_verdict_overrules_is_listed_for_the_owner() {
        let released = battery(vec![Row { owner_labelled: true, ..row(0, "привет") }]);
        let verdicts = Verdicts::from([(("pc".to_owned(), 0, TIME.to_owned()), verdict(KEEP, "llm"))]);
        let result = compare(&released, &released, &verdicts);
        assert_eq!((result.needs_owner, result.released_right, result.verdicts_used), (1, 1, 0));
    }

    #[test]
    fn a_word_the_release_got_right_and_the_candidate_breaks_is_counted_and_listed() {
        let later = |shown: &str| Row { time: LATER.to_owned(), ..row(1, shown) };
        let other_stream = Row { stream: 1, ..row(0, "привет") };
        let released = battery(vec![row(0, "привет"), later("ghbdtn"), other_stream.clone()]);
        let candidate = battery(vec![row(0, "ghbdtn"), later("привет"), other_stream]);
        let result = compare(&released, &candidate, &Verdicts::new());
        assert_eq!((result.rows, result.excluded, result.released_right, result.candidate_right, result.fixed.len(), result.broken.len()), (3, 0, 2, 2, 1, 1));
        assert_eq!(result.broken[0]["candidate"], json!("ghbdtn"));
        // The first word of its stream the trees typed differently; the next one may only follow from it.
        assert_eq!(result.broken[0]["first_divergence"], json!(true));
        assert_eq!(result.fixed[0]["first_divergence"], json!(false));
    }

    #[test]
    fn a_verdict_that_decides_a_word_is_counted() {
        let later = Row { time: LATER.to_owned(), ..row(1, "ghbdtn") };
        let released = battery(vec![row(0, "ghbdtn"), later]);
        let verdicts = Verdicts::from([(("pc".to_owned(), 0, TIME.to_owned()), verdict(KEEP, "llm")), (("pc".to_owned(), 0, LATER.to_owned()), verdict("ambiguous", "llm"))]);
        let result = compare(&released, &released, &verdicts);
        assert_eq!((result.rows, result.excluded, result.released_right, result.verdicts_used), (1, 1, 1, 1));
    }

    #[test]
    fn a_word_asked_about_at_a_pause_and_typed_on_is_right_as_typed_or_in_the_layout_it_was_finished_in() {
        let waiting = |index: i64, time: &str, shown: &str, typed: &str, other: &str, intended: &str| Row { index, time: time.to_owned(), shown: shown.to_owned(), typed: typed.to_owned(), other: other.to_owned(), intended: intended.to_owned(), waits: true, ..row(0, "") };
        let released = battery(vec![waiting(0, TIME, "Ф", "Ф", "A", "A"), waiting(1, LATER, "сд", "сд", "cl", "сд")]);
        let candidate = battery(vec![waiting(0, TIME, "A", "Ф", "A", "A"), waiting(1, LATER, "cl", "сд", "cl", "сд")]);
        let result = compare(&released, &candidate, &Verdicts::new());
        assert_eq!((result.released_right, result.candidate_right, result.broken.len(), result.fixed.len()), (2, 1, 1, 0));
        assert_eq!((&result.broken[0]["candidate"], &result.broken[0]["waits"]), (&json!("cl"), &json!(true)));
    }

    #[test]
    fn a_model_converting_an_owner_labelled_word_from_its_other_reading_overrules_the_owner() {
        let other = json!({"verdict": "convert", "typed": "привет", "by": "llm", "date": "2026-10-16", "reason_code": "garbage-in-prompt", "reason": "a reason"});
        assert!(owner_overruled(&owned(), &other), "converting from the other reading ends as the typed one, not as labelled");
    }

    #[test]
    fn a_verdict_counts_where_the_candidate_alone_reads_it_otherwise_than_its_own_label() {
        let released = battery(vec![row(0, "привет")]);
        let candidate = battery(vec![Row { intended: "ghbdtn".to_owned(), ..row(0, "привет") }]);
        let verdicts = Verdicts::from([(("pc".to_owned(), 0, TIME.to_owned()), verdict("convert", "llm"))]);
        let result = compare(&released, &candidate, &verdicts);
        assert_eq!((result.released_right, result.candidate_right, result.verdicts_used), (1, 1, 1));
    }
}
