//! Streams that go on in the field the stream before left, typed through the replay: every way the log says the next stream
//! began (Backspaces into the last word, a reopened word, a click, a word begun inside text, text pasted over) must lead the
//! replay to the same place in its own text, and each is counted in the report.
#![cfg(any(not(debug_assertions), coverage))]

use ks_replay::harness::KEYS;
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("ks-replay-openings-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("a folder");
    path
}

fn said(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

/// The same keys read in the other layout.
fn other(text: &str, group: usize) -> String {
    text.chars()
        .map(|character| KEYS.iter().find(|key| key.characters[group] == character).map(|key| key.characters[1 - group]).expect("a key of the keyboard"))
        .collect()
}

/// A battery written stream by stream, its times one second apart.
#[derive(Default)]
struct Battery {
    seconds: u32,
    streams: Vec<Value>,
}

impl Battery {
    fn time(&mut self) -> String {
        self.seconds += 1;
        format!("2026-11-07T10:{:02}:{:02}", self.seconds / 60, self.seconds % 60)
    }

    /// A word typed in `group`, meant in `intended`, closed by a space and shown as typed unless `extra` says otherwise.
    fn item(&mut self, original: &str, group: usize, intended: usize, extra: &Value) -> Value {
        let mut item = json!({
            "t": self.time(), "original": original, "group": group, "other": other(original, group), "trigger": "space",
            "shown_group": group, "intended_group": intended, "events": [], "labelled_by_user": false, "engine_convert": false,
            "engine_reason": "", "manual_before": null,
        });
        for (key, value) in extra.as_object().expect("fields") {
            item[key] = value.clone();
        }
        item
    }

    fn word(&mut self, original: &str, intended: usize) -> Value {
        self.item(original, 0, intended, &json!({}))
    }

    /// A stream of these words in `app`; its place in the battery.
    fn stream(&mut self, app: &str, items: Vec<Value>, opening: Option<Value>, closed_by: &str) -> usize {
        let mut stream = Map::new();
        stream.insert("app".to_owned(), json!(app));
        stream.insert("v".to_owned(), json!("0.66.2"));
        stream.insert("t".to_owned(), items[0]["t"].clone());
        stream.insert("items".to_owned(), Value::Array(items));
        stream.insert("closed_by".to_owned(), json!(closed_by));
        stream.insert("closing_gap_s".to_owned(), json!(1.2));
        if let Some(opening) = opening {
            stream.insert("opening".to_owned(), opening);
        }
        self.streams.push(Value::Object(stream));
        self.streams.len() - 1
    }

    /// The opening of a stream that goes on after `previous`: the characters the log counts before the first word, the
    /// ones it counted before the previous word, and the edits between.
    fn after(&self, previous: usize, decision: Value, prior: Value, prior_role: &str, edits: Value) -> Value {
        let last = self.streams[previous]["items"].as_array().expect("items").last().expect("an item")["t"].clone();
        json!({"decision": decision, "previous": {"t": last, "before": prior, "role": prior_role}, "edits": edits})
    }

    fn carry(&self, previous: usize, before: i64, edits: Value) -> Value {
        self.after(previous, json!({"before": before, "role": "text"}), json!(0), "text", edits)
    }

    fn write(&self, path: &Path) {
        let lines: Vec<String> = self.streams.iter().map(Value::to_string).collect();
        std::fs::write(path, lines.join("\n") + "\n").expect("the battery");
    }
}

fn openings() -> Battery {
    let mut battery = Battery::default();
    let editor = "gedit";
    // The engine converts what the log shows left as typed: the next stream's Pause turns it back, and the log's counts
    // place the next word inside the text, where the replay's own caret is, below its start, and past its end.
    let word = battery.word("ghbdtn", 1);
    let first = battery.stream(editor, vec![word], Some(json!({})), "focus");
    let word = battery.word("vbh", 1);
    let anchored = battery.stream(editor, vec![word], Some(battery.carry(first, 7, json!([["bs", 0, 7]]))), "focus");
    let word = battery.word("ljv", 1);
    let agrees = battery.stream(editor, vec![word], Some(battery.carry(anchored, 3, json!([["bs", 0, 3]]))), "focus");
    let word = battery.word("ntcn", 1);
    let below = battery.stream(editor, vec![word], Some(battery.after(agrees, json!({"before": 1, "role": "text"}), json!(400), "text", json!([["bs", 0, 3]]))), "focus");
    let word = battery.word("ckjdj", 1);
    let past = battery.stream(editor, vec![word], Some(battery.carry(below, 50, json!([["bs", 0, 3]]))), "focus");
    // Counts the replay cannot place: too far before, below one, too far after, or of another kind of field.
    let mut previous = past;
    for (decision, prior, prior_role) in [
        (json!({"before": 5, "role": "text"}), json!(600), "text"),
        (json!({"before": -3, "role": "text"}), json!(0), "text"),
        (json!({"before": 600, "role": "text"}), json!(0), "text"),
        (json!({"before": 5, "role": "text"}), json!(0), "code"),
    ] {
        let word = battery.word("rjn", 1);
        previous = battery.stream(editor, vec![word], Some(battery.after(previous, decision, prior, prior_role, json!([["bs", 0, 3]]))), "focus");
    }
    // No decision logged; the message sent with Return, and the next stream in the field it emptied.
    let words = vec![battery.word("ldf", 1), battery.item("ghbdtn", 0, 1, &json!({"trigger": "enter"}))];
    let sent = battery.stream(editor, words, Some(battery.after(previous, Value::Null, json!(0), "text", json!([]))), "enter");
    let word = battery.word("nhb", 1);
    previous = battery.stream(editor, vec![word], Some(battery.carry(sent, 3, json!([["click"]]))), "focus");
    // A click before a word, and words begun inside text with no letters on one side.
    for edits in [json!([["inside", 0, 3], ["click"]]), json!([["inside", 2, 0]]), json!([["inside", 0, 2]])] {
        let word = battery.word("xtnshtcf", 1);
        previous = battery.stream(editor, vec![word], Some(battery.carry(previous, 3, edits)), "focus");
    }
    // A word the engine corrected at its boundary: the first Backspaces count down its letters.
    let word = battery.item("rjirf", 0, 1, &json!({"events": ["auto:context"]}));
    let corrected = battery.stream(editor, vec![word], Some(battery.carry(previous, 3, json!([["click"]]))), "focus");
    let word = battery.word("rjirb", 1);
    previous = battery.stream(editor, vec![word], Some(battery.carry(corrected, 3, json!([["bs", 5, 9], ["bs", 4, 8]]))), "focus");
    // A first Backspace after a corrected word that finds fewer letters than the word has: a letter typed after it.
    let word = battery.item("rjirf", 0, 1, &json!({"events": ["auto:context"]}));
    let corrected = battery.stream(editor, vec![word], Some(battery.carry(previous, 3, json!([["click"]]))), "focus");
    let word = battery.word("rjirb", 1);
    previous = battery.stream(editor, vec![word], Some(battery.carry(corrected, 3, json!([["bs", 2, 5], ["bs", 0, 4], ["bs", 3, 3]]))), "focus");
    // Backspaces past a reopened word, of letters the replay never typed, at a level already seen, and at none.
    for edits in [json!([["reopen", 3], ["bs", 5, 6], ["bs", 0, 5]]), json!([["bs", 0, 5], ["bs", 0, 5], ["bs", 0]])] {
        let word = battery.word("ujh", 1);
        previous = battery.stream(editor, vec![word], Some(battery.carry(previous, 3, edits)), "focus");
    }
    // A stream that ends with a word judged at a pause, the word discarded with more after it, and other text discarded.
    let word = battery.item("cdtn", 0, 1, &json!({"trigger": "pause"}));
    let paused = battery.stream(editor, vec![word], Some(battery.carry(previous, 3, json!([["click"]]))), "focus");
    let word = battery.word("gjkt", 1);
    previous = battery.stream(editor, vec![word], Some(battery.carry(paused, 3, json!([["discard", "cdtnxy"]]))), "focus");
    let word = battery.item("rhfq", 0, 1, &json!({"trigger": "pause"}));
    previous = battery.stream(editor, vec![word], Some(battery.carry(previous, 3, json!([["discard", "other"]]))), "focus");
    let word = battery.word("ytn", 1);
    previous = battery.stream(editor, vec![word], Some(battery.carry(previous, 3, json!([["discard", "other"]]))), "focus");
    // A capital word converted, reopened and typed on in the layout the log names; then reopened longer than it is.
    let word = battery.item("Ghbdtn", 0, 1, &json!({"shown_group": 1}));
    let capital = battery.stream(editor, vec![word], Some(battery.carry(previous, 3, json!([["click"]]))), "focus");
    let word = battery.word("Ghbdtnf", 1);
    let reopened = battery.stream(editor, vec![word], Some(battery.carry(capital, 3, json!([["bs", 0, 7], ["reopen", 6]]))), "focus");
    let word = battery.word("ab", 0);
    battery.stream(editor, vec![word], Some(battery.carry(reopened, 3, json!([["bs", 0, 7], ["reopen", 6]]))), "focus");
    // Another program: a word reopened longer than the text, over a sign no key types, after a click, and as the engine holds it.
    let notes = "notes";
    let word = battery.word("ab", 0);
    previous = battery.stream(notes, vec![word], None, "focus");
    for (typed, edits) in [
        ("abcdefghij", json!([["bs", 0, 3], ["reopen", 8]])),
        ("ok", json!([["discard", "€"], ["reopen", 1]])),
        ("hello", json!([["click"]])),
        ("lower", json!([["bs", 0, 3], ["click"], ["reopen", 2]])),
        ("world", json!([["click"]])),
        ("worlds", json!([["bs", 0, 3], ["reopen", 5]])),
    ] {
        let word = battery.word(typed, 0);
        previous = battery.stream(notes, vec![word], Some(battery.carry(previous, 3, edits)), "focus");
    }
    // A word the log shows in a reading no Pause can give.
    let word = battery.item("ghbdtn", 0, 1, &json!({"shown_group": 1, "other": "nowhere"}));
    previous = battery.stream(notes, vec![word], Some(battery.carry(previous, 3, json!([["click"]]))), "focus");
    let word = battery.word("vbh", 1);
    battery.stream(notes, vec![word], Some(battery.carry(previous, 3, json!([["click"]]))), "focus");
    // Layouts picked by hand, words left out of the count and a word labelled by the lexicon, in the field as left.
    let words = vec![
        battery.item("vbh", 0, 1, &json!({"manual_before": 1, "manual_kind": "pause", "manual_wait_s": 0})),
        battery.item("ckjdj", 0, 1, &json!({"manual_before": 0, "manual_kind": "hotkey"})),
        battery.item("ljv", 0, 1, &json!({"excluded": true})),
        battery.item("rjn", 0, 1, &json!({"lexical_label": true})),
        battery.item("ntcn", 0, 1, &json!({"excluded": true})),
    ];
    battery.stream(notes, words, Some(json!({"decision": {"before": true, "role": "text"}, "previous": {}})), "focus");
    battery
}

fn replay(battery: &Path, report: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ks-replay"));
    command.arg(battery).arg(report);
    command
}

#[test]
fn every_way_a_stream_can_go_on_in_the_field_is_replayed_and_counted() {
    let folder = scratch("streams");
    let battery = folder.join("battery.jsonl");
    openings().write(&battery);
    let report = folder.join("report.json");
    let output = replay(&battery, &report).output().expect("it runs");
    assert!(output.status.success(), "{}", said(&output));
    let written: Value = serde_json::from_slice(&std::fs::read(&report).expect("a report")).expect("json");
    let counts = &written["counts"];
    for kind in [
        "opening:fresh", "opening:carry", "opening:restore", "opening:anchored", "opening:anchor_agrees", "opening:separated",
        "opening:separated_by_log", "opening:inside_unplaced", "opening:kept_mismatch", "opening:kept_realigned", "opening:owner_fix",
        "opening:owner_fix_failed", "opening:layout_as_logged", "exec:manual:manual", "excluded",
    ] {
        assert!(counts[kind].as_i64().is_some_and(|count| count > 0), "{kind}: {counts}");
    }
    // The same report as the reference but for one count: only that count is named.
    let mut reference = written.clone();
    reference["counts"]["opening:carry"] = json!(0);
    let reference_path = folder.join("reference.json");
    std::fs::write(&reference_path, reference.to_string()).expect("a reference");
    let compared = replay(&battery, &report).arg("--compare").arg(&reference_path).output().expect("it runs");
    let shown = said(&compared);
    assert!(compared.status.success() && shown.contains("count opening:carry") && !shown.contains("count opening:fresh"), "{shown}");
    std::fs::remove_dir_all(folder).expect("cleaned");
}
