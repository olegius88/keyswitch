//! The field replay gate: every stream of a battery typed through the engine over a model editor, every word written to
//! the report, and the report compared row by row with the reference the Python engine left.
//!
//!     ks-replay <battery.jsonl> <report.json> [--limit N] [--compare <reference.json>]
//!
//! `FIELD_DEBUG=1` writes the engine's technical log to `<report>.log`, to be set beside the Python's at the first
//! difference. The clock is one and fake: the engine's and the observed text's alike, as `time.monotonic` is patched on the
//! module both read it from.
#![forbid(unsafe_code)]

use ks_cast::To as _;
use ks_replay::harness::{key_of_char, Bundle, CaretReader, PhysicalKey, Session, UnreadField, KEYS, PAUSE_KEYCODE};
use ks_backend::constants::SHIFT_MASK;
use ks_backend::KeyEvent;
use ks_engine::engine::Update;
use ks_text::{pystr, Indent, JsonOptions};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const DEFAULT_PAUSE_DELAY_SECONDS: f64 = 0.7;
// The field reader reads up to about 512 characters before the caret: a count from 500 on may be cut.
const ANCHOR_LIMIT: i64 = 500;
// The release from which a word begun after a Backspace into text no word holds reads the field.
const INSIDE_READ_SINCE: [i64; 3] = [0, 46, 0];

/// `collections.Counter`, in the order of first counting.
#[derive(Default)]
struct Counts(Map<String, Value>);

impl Counts {
    fn add(&mut self, key: &str) {
        let slot = self.0.entry(key.to_owned()).or_insert(json!(0));
        *slot = json!(slot.as_i64().unwrap_or(0) + 1);
    }
}

fn vkey(version: &str) -> Vec<i64> {
    let parts: Option<Vec<i64>> = version.split('.').map(|part| ks_text::py_int(part).map(|value| value.to::<i64>())).collect();
    parts.unwrap_or_else(|| vec![0])
}

fn is_alpha_char(character: char) -> bool {
    ks_text::ucd::isalpha(character)
}

fn truthy(value: Option<&Value>) -> bool {
    value.is_some_and(ks_config::pycompat::truthy)
}

fn text_of(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or("").to_owned()
}

fn int_of(value: &Value) -> i64 {
    // `int(...)`: the battery holds numbers here, and a boolean counts.
    match value {
        Value::Bool(flag) => i64::from(*flag),
        other => other.as_i64().unwrap_or(0),
    }
}

/// `value or {}`.
fn mapping(value: Option<&Value>) -> &Value {
    static EMPTY: Value = Value::Null;
    match value {
        Some(found) if ks_config::pycompat::truthy(found) => found,
        _ => &EMPTY,
    }
}

/// The caret position nearest the end of `text` with `head` letters of a word before it and `tail` after it.
fn _place_inside(text: &[char], head: usize, tail: usize) -> Option<usize> {
    for position in (0..=text.len()).rev() {
        let (before, after) = text.split_at(position);
        if head != 0 && (before.len() < head || !before[before.len() - head..].iter().all(|c| is_alpha_char(*c))) {
            continue;
        }
        if before.len() > head && is_alpha_char(before[before.len() - head - 1]) {
            continue;
        }
        if tail != 0 && (after.len() < tail || !after[..tail].iter().all(|c| is_alpha_char(*c))) {
            continue;
        }
        if after.len() > tail && is_alpha_char(after[tail]) {
            continue;
        }
        return Some(position);
    }
    None
}

/// One field of one application: an editor and the engine typing into it, alive while the log shows typing going on in it.
struct Field {
    app: String,
    last_t: Option<String>,
    submitted: Option<String>,
    last_item: Option<Value>,
    last_span: Option<i64>,
    last_wrong: Option<(usize, usize, String)>,
    session: Session,
}

fn same_keys(shown: &[char], typed: &[char]) -> bool {
    shown.len() == typed.len()
        && shown.iter().zip(typed).all(|(a, b)| {
            let first = key_of_char(*a);
            first.is_some() && first == key_of_char(*b)
        })
}

/// The log's edits between the previous word and the stream's first word: the letters of the first word the reopened word
/// already holds, and whether the caret moved (a click, navigation, a shortcut, a focus change).
fn apply_edits(field: &mut Field, edits: &[Value], first: &Value, previous: Option<&Value>, counts: &mut Counts) -> (usize, bool) {
    let mut reopened: i64 = 0;
    let mut countdown: i64 = 0;
    let mut beyond = false;
    let mut moved = false;
    // Each Backspace logs the characters before the caret it found. One that goes no lower than an earlier one erased a
    // letter typed after it (a word begun, taken back, typed anew), not the text the stream found: the replay never
    // typed that letter, so it does not erase one either.
    let mut lowest: Option<i64> = None;
    // A word the engine corrected at its boundary stays its word for a moment (Pause turns it back): the first Backspaces
    // then count down that word's letters, as for a reopened word.
    let corrected = previous.is_some_and(|previous| {
        previous.get("events").and_then(Value::as_array).is_some_and(|events| events.iter().any(|event| event.as_str().is_some_and(|event| event.starts_with("auto:"))))
    });
    let mut first_bs = true;
    let previous_original = previous.map_or_else(String::new, |previous| text_of(previous, "original"));
    for edit in edits {
        let kind = edit[0].as_str().unwrap_or("");
        match kind {
            "bs" => {
                let tracked = int_of(&edit[1]);
                let level = edit.get(2).and_then(Value::as_i64).filter(|level| *level > 0); // 0: the engine lost count
                let opening_bs = first_bs;
                first_bs = false;
                if let (Some(level), Some(low)) = (level, lowest)
                    && level >= low
                {
                    continue;
                }
                if level.is_some() {
                    lowest = level;
                }
                if countdown != 0 && tracked == countdown {
                    countdown -= 1;
                } else if tracked != 0 && opening_bs && corrected && tracked == pystr::len(&previous_original).to::<i64>() {
                    reopened = tracked;
                    countdown = tracked - 1;
                } else if tracked != 0 {
                    continue; // a letter of a word the replay never typed (begun after the boundary, taken back)
                } else if reopened != 0 {
                    beyond = true;
                }
                field.session.tap_named("BackSpace");
            }
            "reopen" => {
                reopened = int_of(&edit[1]);
                countdown = reopened;
                beyond = false;
            }
            "discard" => {
                let mut text = edit[1].as_str().unwrap_or("").to_owned();
                if let Some(previous) = previous
                    && previous.get("trigger").and_then(Value::as_str) == Some("pause")
                    && text.starts_with(&previous_original)
                {
                    // A word a pause judged stays the engine's word until a click drops it: its letters are typed.
                    text = text[previous_original.len()..].to_owned();
                }
                field.session.backend.insert_at_caret(&text);
                field.session.pointer();
                reopened = 0;
                countdown = 0;
                moved = true;
            }
            "inside" => {
                let position = _place_inside(&field.session.backend.text(), int_of(&edit[1]).to::<usize>(), int_of(&edit[2]).to::<usize>());
                match position {
                    None => {
                        counts.add("opening:inside_unplaced");
                        moved = true; // no such word in the replayed text: the word begins apart, as after a click
                    }
                    Some(position) => {
                        field.session.backend.set_caret(position);
                        moved = false;
                    }
                }
                field.session.pointer();
                reopened = 0;
                countdown = 0;
            }
            "delete" => field.session.tap_named("Delete"),
            _ => {
                // A click, navigation, a shortcut, a focus change, foreign input: the engine loses its word.
                field.session.pointer();
                reopened = 0;
                countdown = 0;
                moved = true;
            }
        }
    }
    let mut kept = if reopened != 0 && !beyond { countdown.to::<usize>() } else { 0 };
    if kept != 0 {
        let backend = &field.session.backend;
        let (text, caret) = (backend.text(), backend.caret());
        let first_chars: Vec<char> = text_of(first, "original").chars().collect();
        if kept > first_chars.len() || caret < kept || !same_keys(&text[caret - kept..caret], &first_chars[..kept]) {
            counts.add("opening:kept_mismatch");
            kept = 0;
        } else if text[caret - kept..caret] != first_chars[..kept] {
            let mut realigned = text[..caret - kept].to_vec();
            realigned.extend_from_slice(&first_chars[..kept]);
            realigned.extend_from_slice(&text[caret..]);
            backend.set_text(realigned);
            counts.add("opening:kept_realigned");
        }
    }
    (kept, moved)
}

/// Send the message: Return as the engine sees it (a deferred action), the text it submits.
fn _finish_with_enter(session: &mut Session) -> String {
    let backend = session.backend.clone();
    let action = KeyEvent {
        pressed: true,
        keycode: 36,
        key_name: "Return".to_owned(),
        character: "\n".to_owned(),
        characters: vec!["\n".to_owned(), "\n".to_owned()],
        group: backend.group(),
        deferred: true,
        ..KeyEvent::default()
    };
    backend.watch_enter(action.clone());
    session.handle(action.clone());
    session.handle(KeyEvent { pressed: false, ..action });
    let delivered = backend.stop_watching_enter();
    delivered.unwrap_or_else(|| backend.text().iter().collect())
}

fn keys_of(item: &Value) -> Vec<&'static PhysicalKey> {
    let group = int_of(&item["group"]).to::<usize>();
    text_of(item, "original")
        .chars()
        .map(|char| KEYS.iter().find(|key| key.characters[group] == char).expect("every key of a battery is on the keyboard"))
        .collect()
}

fn replay(battery: &Path, report: &Path, limit: Option<usize>) {
    let content = std::fs::read_to_string(battery).expect("the battery is readable");
    let mut streams: Vec<Value> = content.lines().filter(|line| !line.is_empty()).map(|line| serde_json::from_str(line).expect("a line of the battery")).collect();
    if let Some(limit) = limit.filter(|limit| *limit != 0) {
        streams.truncate(limit);
    }
    drop(content);
    let debug_log = (std::env::var("FIELD_DEBUG").as_deref() == Ok("1")).then(|| PathBuf::from(format!("{}.log", report.display())));
    let mut bundle = Bundle::load(debug_log.as_deref());
    let mut counts = Counts::default();
    let mut errors: Vec<Value> = Vec::new();
    // Every word as the tree typed it: stream, the item's place in it and time, the reading the keys gave, the other reading,
    // the reading the log implies, what stood in the field, whether the word was labelled by hand and the version that logged it.
    let mut rows: Vec<Value> = Vec::new();
    // How each stream's field began: the mode, the field's characters before the first word in the log and in the replay,
    // whether letters stood before it, and the letters of a reopened word it continued.
    let mut openings: Vec<Value> = Vec::new();
    let mut field: Option<Field> = None;
    let mut memory: HashMap<String, Vec<char>> = HashMap::new();
    let space = KEYS.iter().find(|key| key.characters[0] == ' ').expect("a space bar");
    for (number, stream) in streams.iter().enumerate() {
        let items = stream["items"].as_array().expect("a stream holds items");
        let first = &items[0];
        let app = text_of(stream, "app");
        let first_group = int_of(&first["group"]).to::<i32>();
        let opening = mapping(stream.get("opening"));
        let decision = opening.get("decision").filter(|decision| !decision.is_null());
        let opening_previous = mapping(opening.get("previous"));
        let previous = opening_previous.get("t").and_then(Value::as_str);
        let logged_before: Option<i64> = decision.map(|decision| decision.get("before").filter(|value| ks_config::pycompat::truthy(value)).map_or(0, int_of));
        let adjacent = field.as_ref().is_some_and(|field| field.app == app && field.last_t.is_some() && field.last_t.as_deref() == previous);
        let mode = if logged_before == Some(0) || (logged_before.is_none() && !adjacent) {
            "fresh"
        } else if adjacent {
            "carry"
        } else if memory.contains_key(&app) {
            "restore"
        } else {
            "unknown_text"
        };
        if mode != "carry" {
            if let Some(old) = field.take() {
                bundle.give_back(old.session);
            }
            let text = if mode == "restore" { memory[&app].clone() } else { Vec::new() };
            field = Some(Field {
                app: app.clone(),
                last_t: None,
                submitted: None,
                last_item: None,
                last_span: None,
                last_wrong: None,
                session: bundle.session(&app, first_group, text),
            });
        } else if let Some(submitted) = field.as_mut().and_then(|field| field.submitted.take()) {
            let mut text: Vec<char> = submitted.chars().collect();
            text.push('\n');
            let backend = &field.as_ref().expect("carried").session.backend;
            let caret = text.len();
            backend.set_text(text);
            backend.set_caret(caret);
        }
        let field_ref = field.as_mut().expect("a field for every stream");
        // A field the log shows unread (role unknown: the engine had only the keys it observed) is not read here either.
        let unread = decision.is_some_and(|decision| decision.get("role").and_then(Value::as_str) == Some("unknown"));
        field_ref.session.core.context_policy.reader =
            Some(if unread { Arc::new(UnreadField) } else { Arc::new(CaretReader { backend: field_ref.session.backend.clone() }) });
        counts.add(if unread { "opening:unread" } else { "opening:read" });
        let mut kept = 0;
        if mode == "carry"
            && let Some((start, length, intended)) = field_ref.last_wrong.clone()
        {
            // The last word of the stream before stands otherwise than it stood on the logged screen: Pause turns it,
            // through the engine, into the reading seen there. The next stream begins from that text, not from the
            // tree's own decision there, which the stream before already counts.
            let group = field_ref.session.backend.group();
            field_ref.session.tap(KeyEvent {
                pressed: true,
                keycode: PAUSE_KEYCODE,
                key_name: "Pause".to_owned(),
                characters: vec![String::new(), String::new()],
                group,
                ..KeyEvent::default()
            });
            field_ref.session.idle(0.3);
            let fixed = field_ref.session.backend.slice(start, start + length) == intended.chars().collect::<Vec<_>>();
            counts.add(if fixed { "opening:owner_fix" } else { "opening:owner_fix_failed" });
        }
        if mode == "carry" || mode == "restore" {
            let pre_text = field_ref.session.backend.text();
            let edits: Vec<Value> = opening.get("edits").and_then(Value::as_array).cloned().unwrap_or_default();
            let last_item = field_ref.last_item.clone();
            let (found, moved) = apply_edits(field_ref, &edits, first, if mode == "carry" { last_item.as_ref() } else { None }, &mut counts);
            kept = found;
            let backend = field_ref.session.backend.clone();
            let mut prior = opening_previous.get("before").and_then(Value::as_i64);
            if opening_previous.get("role") != mapping(decision).get("role") {
                prior = None; // an observed count and a read one count different things
            }
            let edits_kinds: HashSet<&str> = edits.iter().filter_map(|edit| edit[0].as_str()).collect();
            let only_backspaces = edits_kinds.iter().all(|kind| *kind == "bs" || *kind == "reopen");
            if let (true, true, Some(last_span), Some(prior), Some(logged)) = (mode == "carry" && kept == 0, only_backspaces, field_ref.last_span, prior, logged_before)
                && prior < ANCHOR_LIMIT
                && 0 < logged
                && logged < ANCHOR_LIMIT
            {
                // The log counts the field's characters before the previous word and before this one: the first word
                // begins that far from where the previous word began, whatever boundary signs the replay typed between.
                let target = last_span + logged - prior;
                let caret = backend.caret().to::<i64>();
                if 0 <= target && target != caret {
                    let target = target.to::<usize>();
                    let mut head: Vec<char> = if target <= pre_text.len() {
                        pre_text[..target].to_vec()
                    } else {
                        let mut head = pre_text.clone();
                        head.resize(target, ' ');
                        head
                    };
                    let length = head.len();
                    head.extend_from_slice(&backend.text()[backend.caret()..]);
                    backend.set_text(head);
                    backend.set_caret(length);
                    counts.add("opening:anchored");
                } else if target == caret {
                    counts.add("opening:anchor_agrees");
                }
            } else if moved && kept == 0 {
                // After a click the log shows no word begun inside text: nothing but a separator stood next to the caret.
                let text = backend.text();
                let caret = backend.caret();
                let (before, after) = text.split_at(caret);
                let letter_before = before.last().is_some_and(|c| is_alpha_char(*c));
                let letter_after = after.first().is_some_and(|c| is_alpha_char(*c));
                if letter_before || letter_after {
                    let mut joined = before.to_vec();
                    if letter_before {
                        joined.push(' ');
                    }
                    let position = joined.len();
                    if letter_after {
                        joined.push(' ');
                    }
                    joined.extend_from_slice(after);
                    backend.set_text(joined);
                    backend.set_caret(position);
                    counts.add("opening:separated");
                }
            }
            if kept == 0
                && !edits_kinds.contains("inside")
                && vkey(&text_of(stream, "v")) >= INSIDE_READ_SINCE.to_vec()
                && decision.is_some_and(|decision| matches!(decision.get("role").and_then(Value::as_str), Some("text" | "search")))
            {
                // From 0.46.0 a word begun after a Backspace into untracked text reads the field and logs letters standing
                // against it (word_started_inside_text); this log shows none, so none stood there.
                let text = backend.text();
                let caret = backend.caret();
                if text[..caret].last().is_some_and(|c| is_alpha_char(*c)) {
                    let mut joined = text[..caret].to_vec();
                    joined.push(' ');
                    joined.extend_from_slice(&text[caret..]);
                    backend.set_text(joined);
                    backend.set_caret(caret + 1);
                    counts.add("opening:separated_by_log");
                }
            }
        }
        let carried_group = field_ref.session.backend.group();
        if carried_group != first_group {
            // The first word was typed in the layout the log names, as a fresh field started it: a layout the tree left
            // differently in the stream before is the tree's own doing there, no choice of the typist's here.
            field_ref.session.backend.set_group(first_group);
            field_ref.session.core._update(Update { current_group: Some(i64::from(first_group)), ..Update::default() });
            counts.add("opening:layout_as_logged");
        }
        if kept != 0 && (carried_group != first_group || field_ref.session.core._strokes.len() != kept) {
            let first_keys = keys_of(first);
            field_ref.session.core._strokes = first_keys[..kept]
                .iter()
                .map(|key| {
                    let character = key.characters[first_group.to::<usize>()].to_string();
                    Arc::new(KeyEvent {
                        pressed: true,
                        keycode: key.keycode,
                        key_name: character.clone(),
                        character,
                        characters: key.strings(),
                        group: first_group,
                        state: if key.shift { SHIFT_MASK } else { 0 },
                        ..KeyEvent::default()
                    })
                })
                .collect();
            field_ref.session.core._source_group = i64::from(first_group);
        }
        counts.add(&format!("opening:{mode}"));
        let backend = field_ref.session.backend.clone();
        let session = &mut field_ref.session;
        let start_before: Vec<char> = backend.text()[..backend.caret() - kept].to_vec();
        openings.push(json!([number, mode, logged_before, start_before.len(), start_before.iter().any(|c| is_alpha_char(*c)), kept, carried_group, first_group]));
        let mut spans: Vec<(usize, usize, usize, i32)> = Vec::new();
        // A word judged at a pause is judged again at its boundary, or after an edit, as the next item of its stream: no
        // boundary follows the pause, and the next item goes on with the same word (the letters they share stay, the rest is
        // erased and typed). What the pause left is the pause's outcome. A stream that ends with a word judged at a pause
        // ended without a boundary: the word stood until a click, a Backspace or another window.
        let mut open_word: Option<(usize, Vec<&'static PhysicalKey>)> = None;
        let mut at_pause: HashMap<usize, String> = HashMap::new();
        for (index, item) in items.iter().enumerate() {
            let manual_before = item.get("manual_before").and_then(Value::as_i64).filter(|group| *group == 0 || *group == 1);
            if let Some(manual_before) = manual_before {
                // The layout was picked by hand: where the tree already stands in it, the pick is still a choice and protects
                // the next word as it did live.
                if i64::from(backend.group()) == manual_before {
                    session.select_layout((1 - manual_before).to::<i32>());
                }
                if item.get("manual_kind").and_then(Value::as_str) == Some("pause") {
                    // Pause with nothing to convert toggled the layout: each tree's own engine decides whether that holds
                    // the next word. As long after the last word as live: whether the Pause undid a fresh correction depends on it.
                    if let Some(wait) = item.get("manual_wait_s").filter(|wait| ks_config::pycompat::truthy(wait)).and_then(Value::as_f64) {
                        session.idle(wait);
                    }
                    session.core._switch_layout_only(i64::from(PAUSE_KEYCODE));
                } else {
                    session.select_layout(manual_before.to::<i32>());
                }
            }
            let typed_group = backend.group();
            let keys = keys_of(item);
            let skip;
            let start;
            if let Some((word_start, earlier)) = open_word.take() {
                start = word_start;
                // A key and its Shift are two characters (`_` is not `-`): Backspaces erased them as such.
                let shared = earlier
                    .iter()
                    .zip(&keys)
                    .position(|(a, b)| (a.keycode, a.shift) != (b.keycode, b.shift))
                    .unwrap_or(earlier.len().min(keys.len()));
                for _ in 0..earlier.len() - shared {
                    session.tap_named("BackSpace");
                }
                skip = shared;
                counts.add("pause_word:continued");
            } else {
                skip = if index == 0 { kept } else { 0 };
                start = backend.caret() - skip;
            }
            for key in &keys[skip..] {
                let group = backend.group();
                let observed = key.characters[group.to::<usize>()].to_string();
                session.tap(KeyEvent {
                    pressed: true,
                    keycode: key.keycode,
                    key_name: observed.clone(),
                    character: observed,
                    characters: key.strings(),
                    group,
                    state: if key.shift { SHIFT_MASK } else { 0 },
                    ..KeyEvent::default()
                });
            }
            spans.push((start, keys.len(), index, typed_group));
            let trigger = item.get("trigger").and_then(Value::as_str);
            if trigger == Some("pause") {
                session.idle(2.0);
                at_pause.insert(index, backend.slice(start, start + keys.len()).iter().collect());
                if index + 1 != items.len() {
                    open_word = Some((start, keys));
                }
                continue;
            }
            let enter_last = stream.get("closed_by").and_then(Value::as_str) == Some("enter") && index + 1 == items.len() && trigger == Some("enter");
            if !enter_last {
                let group = backend.group();
                session.tap(KeyEvent {
                    pressed: true,
                    keycode: space.keycode,
                    key_name: "space".to_owned(),
                    character: " ".to_owned(),
                    characters: space.strings(),
                    group,
                    ..KeyEvent::default()
                });
            }
        }
        let last = items.last().expect("a stream holds items");
        let mut submitted = None;
        if stream.get("closed_by").and_then(Value::as_str) == Some("enter") && last.get("trigger").and_then(Value::as_str) == Some("enter") {
            submitted = Some(_finish_with_enter(session));
        } else if stream.get("closing_gap_s").is_none_or(|gap| gap.is_null() || gap.as_f64().is_some_and(|gap| gap >= DEFAULT_PAUSE_DELAY_SECONDS)) {
            // Only where the log shows the typist really stopped: a stream that ended with a Backspace or a click a moment
            // later had no pause to settle anything.
            session.idle(2.0);
        }
        let text: Vec<char> = match &submitted {
            Some(submitted) => submitted.chars().collect(),
            None => backend.text(),
        };
        field_ref.last_t = Some(text_of(last, "t"));
        field_ref.submitted = submitted.clone();
        let &(last_start, last_length, last_index, _) = spans.last().expect("a span per item");
        let last_item = &items[last_index];
        field_ref.last_item = Some(last_item.clone());
        field_ref.last_span = Some(last_start.to::<i64>());
        field_ref.last_wrong = None;
        // What really stood there: the reading the log shows after every correction of the word while its stream lasted (an
        // implied target can differ: a word left as a typo is labelled by its other reading).
        let reading = |item: &Value, group_key: &str| -> String {
            if item[group_key] == item["group"] { text_of(item, "original") } else { text_of(item, "other") }
        };
        let window = |text: &[char], start: usize, length: usize| -> String {
            text.iter().skip(start).take(length).collect()
        };
        let last_stood = reading(last_item, "shown_group");
        if submitted.is_none() && !truthy(last_item.get("excluded")) && window(&text, last_start, last_length) != last_stood {
            field_ref.last_wrong = Some((last_start, last_length, last_stood));
        }
        let mut remembered = text.clone();
        if submitted.is_some() {
            remembered.push('\n');
        }
        memory.insert(app.clone(), remembered);
        let executed = std::mem::take(session.core.executed.as_mut().expect("the replay records corrections"));
        for plan in &executed {
            counts.add(&format!("exec:{}:{}", plan.mode, if plan.automatic { "auto" } else { "manual" }));
        }
        let early_words: HashSet<&str> = executed.iter().filter(|plan| plan.mode == "early").map(|plan| plan.original.as_str()).collect();
        let context = |items: &[Value]| -> Vec<String> { items.iter().map(|item| reading(item, "intended_group")).take(20).collect() };
        for &(start, length, index, typed_group) in &spans {
            let item = &items[index];
            let typed_reading = if i64::from(typed_group) == int_of(&item["group"]) { text_of(item, "original") } else { text_of(item, "other") };
            if truthy(item.get("excluded")) {
                continue;
            }
            if early_words.iter().any(|prefix| !prefix.is_empty() && typed_reading.starts_with(prefix)) {
                let shown_now = window(&text, start, length);
                let intended_now = reading(item, "intended_group");
                counts.add("early:word");
                let early_outcome = if shown_now == intended_now {
                    "early:right"
                } else if shown_now == typed_reading {
                    "early:reverted_wrong"
                } else {
                    "early:wrong"
                };
                counts.add(early_outcome);
                if early_outcome != "early:right" {
                    errors.push(json!({
                        "stream": number, "app": stream["app"], "v": stream["v"], "t": item["t"], "outcome": early_outcome,
                        "label": if truthy(item.get("labelled_by_user")) { "user" } else { "implicit" }, "typed": typed_reading,
                        "intended": intended_now, "shown": shown_now, "field_engine": {"events": item["events"]},
                        "context": context(items),
                    }));
                }
            }
        }
        for (index, &(start, length, item_index, typed_group)) in spans.iter().enumerate() {
            let item = &items[item_index];
            if truthy(item.get("excluded")) {
                counts.add("excluded");
                continue;
            }
            let shown = at_pause.get(&index).cloned().unwrap_or_else(|| window(&text, start, length));
            // A word typed on after a pause: its target is its reading in the layout it was finished in, and left as typed it
            // is right too.
            let waits = item.get("pause_word_group").is_some();
            let group = if waits { &item["pause_word_group"] } else { &item["intended_group"] };
            let intended = if *group == item["group"] { text_of(item, "original") } else { text_of(item, "other") };
            let typed_reading = if i64::from(typed_group) == int_of(&item["group"]) { text_of(item, "original") } else { text_of(item, "other") };
            let other_reading = if typed_reading == text_of(item, "original") { text_of(item, "other") } else { text_of(item, "original") };
            rows.push(json!([
                number, index, item["t"], typed_reading, other_reading, intended, shown,
                truthy(item.get("labelled_by_user")), stream["v"], waits,
            ]));
            let should_convert = typed_reading != intended;
            let outcome = if shown == intended {
                if should_convert { "right_converted" } else { "right_kept" }
            } else if waits && shown == typed_reading {
                "right_waited"
            } else if shown == typed_reading {
                "miss"
            } else if !should_convert {
                "false_conversion"
            } else {
                "wrong_other"
            };
            let weight = if truthy(item.get("labelled_by_user")) {
                "user"
            } else if truthy(item.get("lexical_label")) {
                "lexical"
            } else {
                "implicit"
            };
            counts.add(outcome);
            counts.add(&format!("{outcome}:{weight}"));
            if matches!(outcome, "miss" | "false_conversion" | "wrong_other") {
                errors.push(json!({
                    "stream": number, "app": stream["app"], "v": stream["v"], "t": item["t"], "outcome": outcome,
                    "label": weight, "typed": typed_reading, "intended": intended, "shown": shown,
                    "field_engine": {"convert": item["engine_convert"], "reason": item["engine_reason"], "events": item["events"]},
                    "context": context(items),
                }));
            }
        }
    }
    if let Some(old) = field.take() {
        bundle.give_back(old.session);
    }
    bundle.clean();
    let document = json!({"counts": Value::Object(counts.0.clone()), "errors": errors, "rows": rows, "openings": openings});
    let options = JsonOptions { ensure_ascii: false, indent: Some(Indent::Spaces(1)), separators: None, sort_keys: false };
    std::fs::write(report, ks_text::json_dumps(&document, &options)).expect("the report is writable");
    println!("{}", ks_text::json_dumps(&Value::Object(counts.0), &JsonOptions { ensure_ascii: false, ..JsonOptions::default() }));
}

/// The differences of a report from the reference, row by row; the first few name their stream and word.
fn compare(report: &Path, reference: &Path) -> bool {
    let read = |path: &Path| -> Value { serde_json::from_slice(&std::fs::read(path).expect("a report to compare")).expect("a report holds JSON") };
    let (ours, theirs) = (read(report), read(reference));
    let empty = Vec::new();
    let rows = |document: &'_ Value, key: &str| -> usize { document[key].as_array().map_or(0, Vec::len) };
    let (our_rows, their_rows) = (ours["rows"].as_array().unwrap_or(&empty), theirs["rows"].as_array().unwrap_or(&empty));
    let differing: Vec<(&Value, &Value)> = our_rows.iter().zip(their_rows).filter(|(a, b)| a != b).collect();
    println!(
        "rows {} vs {}, differing {}; errors {} vs {} {}; counts {}; openings {}",
        our_rows.len(), their_rows.len(), differing.len(), rows(&ours, "errors"), rows(&theirs, "errors"),
        if ours["errors"] == theirs["errors"] { "equal" } else { "DIFFER" },
        if ours["counts"] == theirs["counts"] { "equal" } else { "DIFFER" },
        if ours["openings"] == theirs["openings"] { "equal" } else { "DIFFER" },
    );
    for (ours, theirs) in differing.iter().take(8) {
        println!("  stream {} index {}: ours {} | python {}", ours[0], ours[1], ours, theirs);
    }
    if ours["counts"] != theirs["counts"]
        && let (Some(a), Some(b)) = (ours["counts"].as_object(), theirs["counts"].as_object())
    {
        for key in a.keys().chain(b.keys().filter(|key| !a.contains_key(*key))) {
            if a.get(key) != b.get(key) {
                println!("  count {key}: ours {} | python {}", a.get(key).unwrap_or(&Value::Null), b.get(key).unwrap_or(&Value::Null));
            }
        }
    }
    differing.is_empty() && our_rows.len() == their_rows.len() && ours["errors"] == theirs["errors"]
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let option = |name: &str| arguments.iter().position(|argument| argument == name).and_then(|index| arguments.get(index + 1)).cloned();
    let (Some(battery), Some(report)) = (arguments.first(), arguments.get(1)) else {
        eprintln!("ks-replay <battery.jsonl> <report.json> [--limit N] [--compare <reference.json>]");
        std::process::exit(2);
    };
    replay(Path::new(battery), Path::new(report), option("--limit").and_then(|limit| limit.parse().ok()));
    if let Some(reference) = option("--compare")
        && !compare(Path::new(report), Path::new(&reference))
    {
        std::process::exit(1);
    }
}
