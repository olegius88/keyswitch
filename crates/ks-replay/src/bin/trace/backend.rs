//! The backend and the field reader of a trace: each call is checked against the next one the Python made, and answered
//! with what the Python got.

use ks_cast::To as _;
use ks_backend::input_delay::{HookGap, InputDelayReport};
use ks_backend::selected_text::{RefusalReason, SelectedText, SelectionError, SelectionRefused};
use ks_backend::{BackendError, BackendProbe, FieldContext, FieldReader, FieldRole, FocusInfo, Injection, InputBackend, KeyEvent, KeyFilter, KeyListener, ScreenAnchor};
use ks_context::context_model::ContextPrediction;
use ks_context::flat_table::WeightTable;
use ks_context::{BeforeOrigin, ContextAction, ContextEvidence, ContextModel, TermTables};
use ks_engine::engine::{QueueItem, Shared};
use ks_lang::WordScore;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

pub(crate) fn key_value(event: &KeyEvent) -> Value {
    json!({"$key": [event.pressed, event.keycode, event.key_name, event.character, event.characters, event.group, event.state,
                    event.timestamp, event.synthetic, event.deferred, event.foreign]})
}

pub(crate) fn key_from(value: &Value) -> Option<KeyEvent> {
    let fields = value.get("$key")?.as_array()?;
    Some(KeyEvent {
        pressed: fields[0].as_bool()?,
        keycode: u32::try_from(fields[1].as_i64()?).ok()?,
        key_name: fields[2].as_str()?.to_owned(),
        character: fields[3].as_str()?.to_owned(),
        characters: fields[4].as_array()?.iter().filter_map(|item| item.as_str().map(str::to_owned)).collect(),
        group: i32::try_from(fields[5].as_i64()?).ok()?,
        state: u32::try_from(fields[6].as_i64()?).ok()?,
        timestamp: fields[7].as_i64()?,
        synthetic: fields[8].as_bool()?,
        deferred: fields[9].as_bool()?,
        foreign: fields[10].as_bool()?,
    })
}

pub(crate) const REASONS: [RefusalReason; 20] = [
    RefusalReason::Disabled, RefusalReason::OwnWindow, RefusalReason::ExcludedApplication, RefusalReason::SensitiveField,
    RefusalReason::Unsupported, RefusalReason::NoSelection, RefusalReason::MultipleSelection, RefusalReason::ReadOnly,
    RefusalReason::TooLong, RefusalReason::LineBreaks, RefusalReason::Untypeable, RefusalReason::Terminal,
    RefusalReason::ClipboardBusy, RefusalReason::ClipboardUnrestorable, RefusalReason::NotSettable, RefusalReason::FocusChanged,
    RefusalReason::ReleaseTimeout, RefusalReason::NoLayout, RefusalReason::EmptyResult, RefusalReason::NoCaret,
];

/// What the entry in progress is answered from.
pub(crate) struct Tape {
    pub calls: Vec<Value>,
    pub at: usize,
    pub difference: Option<(String, String)>,
    pub status: String,
    pub clock: Arc<AtomicU64>,
    pub shared: Weak<Shared>,
    /// The last answer of each pure read.
    pub pure: Vec<(String, Value)>,
    /// The first answer of each pure read in the whole trace, for an engine that asks before the Python did.
    pub future: Vec<(String, Value)>,
}

impl Tape {
    pub(crate) fn new(clock: Arc<AtomicU64>, status: String) -> Tape {
        Tape { calls: Vec::new(), at: 0, difference: None, status, clock, shared: Weak::new(), pure: Vec::new(), future: Vec::new() }
    }

    /// Notes the first answer of each pure read in a list of entries.
    pub(crate) fn look_ahead(&mut self, calls: &[Value]) {
        for call in calls {
            let method = call["m"].as_str().unwrap_or("");
            if is_pure(method) && !self.future.iter().any(|(name, _)| name == method) {
                self.future.push((method.to_owned(), pure_answer(call)));
            }
        }
    }

    pub(crate) fn load(&mut self, calls: &[Value]) {
        self.calls = calls.to_vec();
        self.at = 0;
    }

    fn note(&mut self, cause: String, detail: String) {
        if self.difference.is_none() {
            self.difference = Some((cause, detail));
        }
    }

    /// Reader calls no reader trait has (`close`, the diagnostics) belong to the platform reader, which a trace does not run.
    fn skip_foreign(&mut self) {
        while let Some(call) = self.calls.get(self.at) {
            if call["o"] == "r" && call["m"] != "read" {
                self.at += 1;
            } else {
                break;
            }
        }
    }

    /// Remembers a pure read's answer; a changed one the Rust engine would not have looked at is a difference.
    fn pass_pure(&mut self) {
        let call = self.calls[self.at].clone();
        let method = call["m"].as_str().unwrap_or("").to_owned();
        let answer = pure_answer(&call);
        if let Some(known) = self.pure.iter().find(|(name, _)| *name == method)
            && known.1 != answer
            && self.difference.is_none()
        {
            self.note(format!("{method} changed and the Rust engine did not read it"), format!("was {}; now {}", compact(&known.1), compact(&answer)));
        }
        self.remember(&method, answer);
        self.follow_clock(&call);
        self.at += 1;
    }

    /// A fake may move the test's clock while it answers.
    fn follow_clock(&self, call: &Value) {
        if let Some(clock) = call.get("c").and_then(Value::as_f64) {
            self.clock.store(clock.to_bits(), Ordering::Relaxed);
        }
    }

    fn remember(&mut self, method: &str, answer: Value) {
        match self.pure.iter_mut().find(|(name, _)| name == method) {
            Some(known) => known.1 = answer,
            None => self.pure.push((method.to_owned(), answer)),
        }
    }

    /// The Python calls the Rust engine did not make, when the entry is over.
    pub(crate) fn leftover(&mut self) -> Option<String> {
        self.skip_foreign();
        while self.calls.get(self.at).is_some_and(|call| is_pure(call["m"].as_str().unwrap_or(""))) {
            self.pass_pure();
            self.skip_foreign();
        }
        let call = self.calls.get(self.at)?;
        Some(format!("{} {}", call["m"].as_str().unwrap_or("?"), compact(&call["a"])))
    }

    fn take(&mut self, origin: &str, method: &str, args: Value) -> Option<Value> {
        if self.difference.is_some() {
            return None;
        }
        self.skip_foreign();
        if is_pure(method) {
            // A pure read has no effect: what counts is the answer, not how many times either engine asked.
            let mut ahead = self.at;
            while self.calls.get(ahead).is_some_and(|call| call["m"] != method && is_pure(call["m"].as_str().unwrap_or(""))) {
                ahead += 1;
            }
            if self.calls.get(ahead).is_none_or(|call| call["m"] != method) {
                let known = self.pure.iter().find(|(name, _)| name == method).map(|(_, answer)| answer.clone());
                return if let Some(answer) = known.or_else(|| self.first_read(method)) { Some(answer) } else {
                    self.note(format!("{method} read where the Python never read it"), String::new());
                    None
                };
            }
            while self.at < ahead {
                self.pass_pure();
            }
            let call = self.calls[self.at].clone();
            self.remember(method, pure_answer(&call));
            self.follow_clock(&call);
            self.at += 1;
            return Some(call);
        }
        while self.calls.get(self.at).is_some_and(|call| is_pure(call["m"].as_str().unwrap_or(""))) {
            self.pass_pure();
            self.skip_foreign();
            if self.difference.is_some() {
                return None;
            }
        }
        let Some(call) = self.calls.get(self.at).cloned() else {
            self.note(format!("extra call {method}"), format!("the Rust engine called {method} {}, the Python made no more calls", compact(&args)));
            return None;
        };
        if call["m"] != method || call["o"] != origin {
            self.note(
                format!("call {} expected, {method} made", call["m"].as_str().unwrap_or("?")),
                format!("python {} {}; rust {method} {}", call["m"].as_str().unwrap_or("?"), compact(&call["a"]), compact(&args)),
            );
            return None;
        }
        let mut recorded = call["a"].clone();
        if method == "inject_correction" {
            // The arguments the Python leaves at their defaults.
            for (name, default) in [("source_group", json!(null)), ("late", json!([])), ("trailing", json!([])), ("kept_tail", json!(0)), ("erased", json!(null))] {
                if recorded.get(name).is_none() {
                    recorded[name] = default;
                }
            }
        }
        if let Some((path, a, b)) = super::compare::first_difference(&recorded, &args, "") {
            self.note(format!("arguments of {method}{}", super::run::general(&path)), format!("at {path}: python {}; rust {}; whole call: python {}; rust {}", compact(&a), compact(&b), compact(&recorded), compact(&args)));
            return None;
        }
        self.at += 1;
        self.follow_clock(&call);
        if let Some(status) = call.get("status").and_then(Value::as_str) {
            status.clone_into(&mut self.status);
        }
        Some(call)
    }

    /// Whether the next call the Python made, past its pure reads, is this one: a test may give its fake the method
    /// after the engine was built.
    pub(crate) fn next_is(&self, method: &str) -> bool {
        self.calls[self.at.min(self.calls.len())..]
            .iter()
            .find(|call| !is_pure(call["m"].as_str().unwrap_or("")) && (call["o"] != "r" || call["m"] == "read"))
            .is_some_and(|call| call["m"] == method)
    }

    fn first_read(&self, method: &str) -> Option<Value> {
        self.future.iter().find(|(name, _)| name == method).map(|(_, answer)| answer.clone())
    }
}

const PURE_READS: [&str; 3] = ["active_application", "focused_window", "current_group"];

fn is_pure(method: &str) -> bool {
    PURE_READS.contains(&method)
}

/// A recorded read with only what the engine gets from it.
fn pure_answer(call: &Value) -> Value {
    match call.get("x") {
        Some(error) => json!({"x": error}),
        None => json!({"r": call["r"]}),
    }
}

pub(crate) fn compact(value: &Value) -> String {
    let text = value.to_string();
    if text.chars().count() > 600 { format!("{}…", text.chars().take(600).collect::<String>()) } else { text }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Capabilities {
    pub take_unseen_input: bool,
    pub take_input_delay: bool,
    pub take_hook_gap: bool,
    pub input_anchor: bool,
    pub restore_window: bool,
    pub keep_window_inactive: bool,
    pub selection: bool,
    pub erases_any_length: bool,
    pub swallows_hotkeys: bool,
}

pub(crate) struct TraceBackend {
    pub tape: Arc<Mutex<Tape>>,
    pub capabilities: Capabilities,
}

fn error_of(call: &Value) -> Option<BackendError> {
    let error = call.get("x")?;
    let message = error["message"].as_str().unwrap_or("").to_owned();
    Some(match (error["type"].as_str(), error.get("group").and_then(Value::as_i64)) {
        (Some("LayoutSwitchUnconfirmed"), Some(group)) => BackendError::LayoutSwitchUnconfirmed { message, group: group.to::<i32>() },
        _ => BackendError::Failed(message),
    })
}

fn selection_error_of(call: &Value) -> Option<SelectionError> {
    let error = call.get("x")?;
    if let Some(reason) = error.get("reason").and_then(Value::as_str)
        && let Some(reason) = REASONS.iter().find(|item| item.as_str() == reason)
    {
        return Some(SelectionRefused::new(*reason).into());
    }
    error_of(call).map(SelectionError::Failed)
}

impl TraceBackend {
    fn lock(&self) -> MutexGuard<'_, Tape> {
        self.tape.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The next call, with what the fake backend did to the engine while the engine waited for it.
    fn call(&self, method: &str, args: Value) -> Option<Value> {
        let (call, shared) = {
            let mut tape = self.lock();
            (tape.take("b", method, args), tape.shared.upgrade())
        };
        let call = call?;
        if let (Some(during), Some(shared)) = (call.get("d").and_then(Value::as_array), shared) {
            for what in during {
                if let Some(event) = what.get("enqueue").and_then(key_from) {
                    shared.enqueue(event);
                } else if let Some(event) = what.get("put").and_then(key_from) {
                    shared.put_nowait(QueueItem::Key(Arc::new(event)));
                } else if let Some(event) = what.get("consumes").and_then(key_from) {
                    let answer = super::compare::disposition_value(shared.consumes_key(&event));
                    if what["r"] != answer {
                        self.lock().note("consumes_key during a call".to_owned(), format!("python {}; rust {answer}", compact(&what["r"])));
                    }
                }
            }
        }
        Some(call)
    }

    fn int_answer(&self, method: &str, args: Value) -> Result<i32, BackendError> {
        let Some(call) = self.call(method, args) else { return Ok(0) };
        if let Some(error) = error_of(&call) {
            return Err(error);
        }
        Ok((call["r"].as_i64().unwrap_or(0)).to::<i32>())
    }

    /// A call the Python may raise from while the trait says it cannot: the difference is the port's.
    fn no_failure(&self, method: &str, call: &Value) {
        if let Some(error) = call.get("x") {
            self.lock().note(format!("{method} cannot fail in the port"), compact(error));
        }
    }
}

fn keys_value(keys: &[KeyEvent]) -> Value {
    Value::Array(keys.iter().map(key_value).collect())
}

impl InputBackend for TraceBackend {
    fn start(&self, _listener: KeyListener) -> Result<(), BackendError> {
        let Some(call) = self.call("start", json!([{"$callable": "enqueue"}])) else { return Ok(()) };
        error_of(&call).map_or(Ok(()), Err)
    }

    fn set_key_filter(&self, predicate: Option<KeyFilter>) {
        let args = if predicate.is_some() { json!([{"$callable": "consumes_key"}]) } else { json!([null]) };
        self.call("set_key_filter", args);
    }

    fn hold_input(&self) {
        if let Some(call) = self.call("hold_input", json!([])) {
            self.no_failure("hold_input", &call);
        }
    }

    fn try_hold_input(&self) -> Result<(), BackendError> {
        let Some(call) = self.call("hold_input", json!([])) else { return Ok(()) };
        error_of(&call).map_or(Ok(()), Err)
    }

    fn release_input(&self) -> Result<i32, BackendError> {
        self.int_answer("release_input", json!([]))
    }

    fn complete_action(&self, deliver: bool) -> Result<i32, BackendError> {
        self.int_answer("complete_action", json!([deliver]))
    }

    fn stop(&self) {
        if let Some(call) = self.call("stop", json!([])) {
            self.no_failure("stop", &call);
        }
    }

    fn close(&self) {
        if let Some(call) = self.call("close", json!([])) {
            self.no_failure("close", &call);
        }
    }

    fn current_group(&self) -> Result<i32, BackendError> {
        self.int_answer("current_group", json!([]))
    }

    fn switch_group(&self, group: i32) -> Result<(), BackendError> {
        let Some(call) = self.call("switch_group", json!([group])) else { return Ok(()) };
        error_of(&call).map_or(Ok(()), Err)
    }

    fn active_application(&self) -> String {
        let Some(call) = self.call("active_application", json!([])) else { return String::new() };
        self.no_failure("active_application", &call);
        call["r"].as_str().unwrap_or("").to_owned()
    }

    fn focused_window(&self) -> Option<FocusInfo> {
        let call = self.call("focused_window", json!([]))?;
        self.no_failure("focused_window", &call);
        let answer = &call["r"];
        Some(FocusInfo {
            window: answer.get("window")?.as_i64()?,
            own: answer["own"].as_bool().unwrap_or(false),
            isolated_layout: answer["isolated_layout"].as_bool().unwrap_or(false),
        })
    }

    fn probe(&self) -> BackendProbe {
        let Some(call) = self.call("probe", json!([])) else { return BackendProbe::default() };
        let answer = &call["r"];
        let text = |name: &str| answer[name].as_str().unwrap_or("").to_owned();
        BackendProbe {
            available: answer["available"].as_bool().unwrap_or(false),
            session_type: text("session_type"),
            display: text("display"),
            record_version: text("record_version"),
            xtest_version: text("xtest_version"),
            xkb_version: text("xkb_version"),
            current_group: (answer["current_group"].as_i64().unwrap_or(0)).to::<i32>(),
            error: text("error"),
        }
    }

    fn inject_correction(&self, injection: Injection<'_>) -> Result<i32, BackendError> {
        let args = json!({
            "strokes": keys_value(injection.strokes),
            "target_group": injection.target_group,
            "boundary": injection.boundary.map(key_value),
            "source_group": injection.source_group,
            "late": keys_value(injection.late),
            "trailing": keys_value(injection.trailing),
            "kept_tail": injection.kept_tail,
            "erased": injection.erased,
        });
        self.int_answer("inject_correction", args)
    }

    fn erases_any_length(&self) -> bool {
        self.capabilities.erases_any_length
    }

    fn swallows_hotkeys(&self) -> bool {
        self.capabilities.swallows_hotkeys
    }

    fn take_unseen_input(&self) -> Result<Option<i64>, BackendError> {
        if !self.capabilities.take_unseen_input && !self.lock().next_is("take_unseen_input") {
            return Ok(None);
        }
        let Some(call) = self.call("take_unseen_input", json!([])) else { return Ok(None) };
        if let Some(error) = error_of(&call) {
            return Err(error);
        }
        Ok(call["r"].as_i64())
    }

    fn take_input_delay(&self) -> Option<InputDelayReport> {
        if !self.capabilities.take_input_delay && !self.lock().next_is("take_input_delay") {
            return None;
        }
        let call = self.call("take_input_delay", json!([]))?;
        let answer = &call["r"];
        answer.get("keys")?;
        let number = |name: &str| answer[name].as_i64().unwrap_or(0);
        Some(InputDelayReport {
            keys: number("keys"),
            late_keys: number("late_keys"),
            worst_late_ms: number("worst_late_ms"),
            slow_callbacks: number("slow_callbacks"),
            worst_callback_ms: number("worst_callback_ms"),
            unstamped_keys: number("unstamped_keys"),
            foreign_keys: number("foreign_keys"),
        })
    }

    fn take_hook_gap(&self) -> Option<HookGap> {
        if !self.capabilities.take_hook_gap && !self.lock().next_is("take_hook_gap") {
            return None;
        }
        let call = self.call("take_hook_gap", json!([]))?;
        let answer = &call["r"];
        Some(HookGap { late_ms: answer.get("late_ms")?.as_i64()?, callback_ms: answer["callback_ms"].as_i64().unwrap_or(0) })
    }

    fn supports_selection(&self) -> bool {
        self.capabilities.selection
    }

    fn read_selection(&self, application: &str, window: i64) -> Result<SelectedText, SelectionError> {
        let Some(call) = self.call("read_selection", json!([application, window])) else {
            return Err(SelectionRefused::new(RefusalReason::Unsupported).into());
        };
        if let Some(error) = selection_error_of(&call) {
            return Err(error);
        }
        let answer = &call["r"];
        Ok(SelectedText { text: answer["text"].as_str().unwrap_or("").to_owned(), source: answer["source"].as_str().unwrap_or("").to_owned() })
    }

    fn replace_selection(&self, selected: &SelectedText, replacement: &str, final_group: Option<i32>) -> Result<bool, SelectionError> {
        let args = json!([{"$dc": "SelectedText", "text": selected.text, "source": selected.source}, replacement, final_group]);
        let Some(call) = self.call("replace_selection", args) else { return Ok(false) };
        if let Some(error) = selection_error_of(&call) {
            return Err(error);
        }
        Ok(call["r"].as_bool().unwrap_or(false))
    }

    fn input_anchor(&self) -> Option<ScreenAnchor> {
        if !self.capabilities.input_anchor && !self.lock().next_is("input_anchor") {
            return None;
        }
        let call = self.call("input_anchor", json!([]))?;
        let answer = &call["r"];
        Some(ScreenAnchor { x: (answer.get("x")?.as_i64()?).to::<i32>(), y: (answer["y"].as_i64().unwrap_or(0)).to::<i32>(), window: answer["window"].as_i64() })
    }

    fn restore_window(&self, window: Option<i64>) -> bool {
        if !self.capabilities.restore_window && !self.lock().next_is("restore_window") {
            return false;
        }
        self.call("restore_window", json!([window])).is_some_and(|call| call["r"].as_bool().unwrap_or(false))
    }

    fn keep_window_inactive(&self, window: i64) -> bool {
        if !self.capabilities.keep_window_inactive && !self.lock().next_is("keep_window_inactive") {
            return false;
        }
        self.call("keep_window_inactive", json!([window])).is_some_and(|call| call["r"].as_bool().unwrap_or(false))
    }
}

pub(crate) struct TraceReader {
    pub tape: Arc<Mutex<Tape>>,
}

fn word_score_value(score: Option<&WordScore>) -> Value {
    score.map_or(Value::Null, |score| {
        json!({"$dc": "WordScore", "value": score.value, "known": score.known, "frequency": score.frequency, "gram_ratio": score.gram_ratio,
               "exact": score.exact, "spell_known": score.spell_known, "ngram_score": score.ngram_score, "invalid_ratio": score.invalid_ratio,
               "raw_ngram_score": score.raw_ngram_score})
    })
}

pub(crate) fn field_value(field: &FieldContext) -> Value {
    json!({"$dc": "FieldContext", "application": field.application, "field_id": field.field_id, "before": field.before, "after": field.after,
           "role": field.role.as_str(), "selection": field.selection, "sensitive": field.sensitive, "source": field.source})
}

fn evidence_value(item: &ContextEvidence) -> Value {
    json!([{"$dc": "ContextEvidence", "original": item.original, "alternative": item.alternative, "source_group": item.source_group,
            "field": field_value(&item.field), "trigger": item.trigger, "baseline_convert": item.baseline_convert,
            "source_known": item.source_known, "target_known": item.target_known, "score_delta": item.score_delta,
            "source_score": word_score_value(item.source_score.as_ref()), "target_score": word_score_value(item.target_score.as_ref()),
            "model_probability": item.model_probability, "model_threshold": item.model_threshold, "literal_tail": item.literal_tail,
            "ortho_score": item.ortho_score, "ortho_threshold": item.ortho_threshold, "boundary_text": item.boundary_text,
            "after_origin": item.after_origin.as_str(), "before_origin": if item.before_origin == BeforeOrigin::Field { "field" } else { "planned_previous_reading" },
            "source_identifier": item.source_identifier, "target_identifier": item.target_identifier, "source_typo": item.source_typo,
            "target_typo": item.target_typo, "source_opening": item.source_opening, "target_opening": item.target_opening,
            "inside": item.inside, "term_counts_dropped": item.term_counts_dropped}])
}

/// A context model that asks the tape: the evidence is compared with what the Python's model was asked.
pub(crate) fn answering_model(tape: Arc<Mutex<Tape>>, attributes: &Value, tables: Arc<TermTables>) -> ContextModel {
    let mut model = ContextModel::new(WeightTable::empty(), attributes["version"].as_str().unwrap_or(""), attributes["conversion_threshold"].as_f64().unwrap_or(0.5), tables);
    let flag = |name: &str| attributes[name].as_bool().unwrap_or(false);
    model.feature_version = attributes["feature_version"].as_i64().unwrap_or(0);
    model.answers_letters = flag("answers_letters");
    model.answers_variables = flag("answers_variables");
    model.answers_alnum = flag("answers_alnum");
    model.answers_resume = flag("answers_resume");
    model.answers_cyrletter = flag("answers_cyrletter");
    model.answers_digit_dot = flag("answers_digit_dot");
    let version = model.version.clone();
    model.answer = Some(Arc::new(move |item: &ContextEvidence| {
        let call = tape.lock().unwrap_or_else(PoisonError::into_inner).take("m", "predict", evidence_value(item));
        let unsupported = || ContextPrediction { action: ContextAction::Suggest, probability: 0.0, probabilities: [0.0, 0.0, 0.0, 1.0], model_version: version.clone(), supported: false };
        let Some(call) = call else { return unsupported() };
        let answer = &call["r"];
        let action = match answer["action"].as_str() {
            Some("keep") => ContextAction::Keep,
            Some("convert") => ContextAction::Convert,
            Some("wait") => ContextAction::Wait,
            _ => ContextAction::Suggest,
        };
        let mut probabilities = [0.0; 4];
        for (slot, value) in probabilities.iter_mut().zip(answer["probabilities"].as_array().map_or(&[][..], Vec::as_slice)) {
            *slot = value.as_f64().unwrap_or(0.0);
        }
        ContextPrediction {
            action,
            probability: answer["probability"].as_f64().unwrap_or(0.0),
            probabilities,
            model_version: answer["model_version"].as_str().unwrap_or("").to_owned(),
            supported: answer["supported"].as_bool().unwrap_or(false),
        }
    }));
    model
}

pub(crate) fn role_from(name: &str) -> FieldRole {
    match name {
        "text" => FieldRole::Text,
        "code" => FieldRole::Code,
        "search" => FieldRole::Search,
        "terminal" => FieldRole::Terminal,
        "password" => FieldRole::Password,
        _ => FieldRole::Unknown,
    }
}

impl FieldReader for TraceReader {
    fn read(&self, application: &str, window: i64) -> Option<FieldContext> {
        let mut tape = self.tape.lock().unwrap_or_else(PoisonError::into_inner);
        let call = tape.take("r", "read", json!([application, window]))?;
        if call.get("x").is_some() {
            tape.note("reader raises in the Python".to_owned(), compact(&call["x"]));
            return None;
        }
        let answer = &call["r"];
        let text = |name: &str| answer[name].as_str().unwrap_or("").to_owned();
        answer.get("application")?;
        Some(FieldContext {
            application: text("application"),
            field_id: text("field_id"),
            before: text("before"),
            after: text("after"),
            role: role_from(answer["role"].as_str().unwrap_or("unknown")),
            selection: answer["selection"].as_bool().unwrap_or(false),
            sensitive: answer["sensitive"].as_bool().unwrap_or(false),
            source: text("source"),
        })
    }

    fn status(&self) -> String {
        self.tape.lock().unwrap_or_else(PoisonError::into_inner).status.clone()
    }
}
