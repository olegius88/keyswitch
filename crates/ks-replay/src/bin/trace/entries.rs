//! One entry into the engine from outside: the Python's name and arguments, called on the Rust engine.

use ks_cast::To as _;
use super::backend::key_from;
use super::compare::disposition_value;
use ks_config::history::HistoryEntry;
use ks_config::learning::{LearnedRule, RuleAction, RuleMatch};
use ks_config::selection_commands::SelectionCommand;
use ks_engine::engine::{CorrectionPlan, EngineCore, KeySwitchEngine, LearningPrompt, QueueItem, Update};
use serde_json::{json, Value};
use std::sync::Arc;

pub(crate) enum Outcome {
    /// What the entry returned or raised, for the comparison; None where the two languages have no common answer.
    Done { returned: Option<Value>, error: Option<String> },
    /// A question of the Python's alone, which leaves nothing behind: its calls are passed over.
    Query,
    /// Nothing here runs this.
    Unknown(String),
}

fn done() -> Outcome {
    Outcome::Done { returned: None, error: None }
}

fn result_of<E: std::fmt::Display>(result: Result<(), E>) -> Outcome {
    Outcome::Done { returned: None, error: result.err().map(|error| error.to_string()) }
}

fn answer(value: Value) -> Outcome {
    Outcome::Done { returned: Some(value), error: None }
}

pub(crate) struct Args<'a> {
    pub positional: &'a [Value],
    pub named: Option<&'a serde_json::Map<String, Value>>,
}

impl Args<'_> {
    pub(crate) fn get(&self, index: usize, name: &str) -> Option<&Value> {
        self.positional.get(index).or_else(|| self.named.and_then(|named| named.get(name)))
    }

    fn text(&self, index: usize, name: &str) -> Option<&str> {
        self.get(index, name).and_then(Value::as_str)
    }

    fn int(&self, index: usize, name: &str) -> Option<i64> {
        self.get(index, name).and_then(Value::as_i64)
    }

    fn float(&self, index: usize, name: &str) -> Option<f64> {
        self.get(index, name).and_then(Value::as_f64)
    }

    fn flag(&self, index: usize, name: &str) -> Option<bool> {
        self.get(index, name).and_then(Value::as_bool)
    }
}

pub(crate) fn rule_from(value: &Value) -> Option<LearnedRule> {
    let text = |name: &str| value[name].as_str().map(str::to_owned);
    Some(LearnedRule {
        pattern: text("pattern")?,
        source_group: value["source_group"].as_i64()?,
        target_group: value["target_group"].as_i64()?,
        action: match value["action"].as_str()? {
            "keep" => RuleAction::Keep,
            _ => RuleAction::Convert,
        },
        r#match: match value["match"].as_str()? {
            "contains" => RuleMatch::Contains,
            "prefix" => RuleMatch::Prefix,
            _ => RuleMatch::Exact,
        },
        case_sensitive: value["case_sensitive"].as_bool()?,
    })
}

pub(crate) fn rule_value(rule: &LearnedRule) -> Value {
    json!({"pattern": rule.pattern, "source_group": rule.source_group, "target_group": rule.target_group,
           "action": rule.action.as_str(), "match": rule.r#match.as_str(), "case_sensitive": rule.case_sensitive})
}

/// A rule as a call returns it, a dataclass.
fn rule_returned(rule: &LearnedRule) -> Value {
    let mut value = rule_value(rule);
    value["$dc"] = json!("LearnedRule");
    value
}

pub(crate) fn prompt_from(value: &Value) -> Option<LearningPrompt> {
    value.get("$dc")?;
    let text = |name: &str| value[name].as_str().map(str::to_owned);
    Some(LearningPrompt {
        source_group: value["source_group"].as_i64()?,
        target_group: value["target_group"].as_i64()?,
        original: text("original")?,
        replacement: text("replacement")?,
        application: text("application")?,
        action: if value["action"] == "keep" { RuleAction::Keep } else { RuleAction::Convert },
    })
}

pub(crate) fn prompt_value(prompt: &LearningPrompt) -> Value {
    json!({"$dc": "LearningPrompt", "source_group": prompt.source_group, "target_group": prompt.target_group,
           "original": prompt.original, "replacement": prompt.replacement, "application": prompt.application,
           "action": prompt.action.as_str()})
}

/// A plan a test made or took from the engine. Its `revisit` holds a decision, which only the engine's own flow fills.
fn plan_from(value: &Value) -> Option<CorrectionPlan> {
    if value["$dc"] != "CorrectionPlan" || !value["revisit"].is_null() {
        return None;
    }
    let boundary = value.get("boundary").and_then(key_from).map(Arc::new);
    let text = |name: &str| value[name].as_str().unwrap_or("").to_owned();
    let mut plan = CorrectionPlan::new(
        strokes_from(value.get("strokes")), boundary, value["source_group"].as_i64()?, value["target_group"].as_i64()?, text("original"),
        text("replacement"), value["confidence"].as_f64()?, &text("application"),
    );
    plan.automatic = value["automatic"].as_bool()?;
    plan.mode = Box::leak(text("mode").into_boxed_str());
    plan.context_field = text("context_field");
    plan.trailing = strokes_from(value.get("trailing"));
    plan.head = strokes_from(value.get("head"));
    plan.sign_replayed = value["sign_replayed"].as_bool()?;
    plan.typed = text("typed");
    plan.kept_tail = (value["kept_tail"].as_u64()?).to::<usize>();
    plan.replaced_strokes = strokes_from(value.get("replaced_strokes"));
    plan.replaced = strokes_from(value.get("replaced"));
    Some(plan)
}

/// The fields of a plan that say what it does, for a plan a call returned.
fn plan_digest(plan: &CorrectionPlan) -> Value {
    json!({"mode": plan.mode, "original": plan.original, "replacement": plan.replacement, "source_group": plan.source_group,
           "target_group": plan.target_group, "confidence": plan.confidence, "application": plan.application, "automatic": plan.automatic,
           "strokes": plan.strokes.iter().map(|stroke| super::backend::key_value(stroke)).collect::<Vec<_>>(),
           "replaced": plan.replaced.iter().map(|stroke| super::backend::key_value(stroke)).collect::<Vec<_>>()})
}

fn strokes_from(value: Option<&Value>) -> Vec<Arc<ks_backend::KeyEvent>> {
    value.and_then(Value::as_array).map_or_else(Vec::new, |items| items.iter().filter_map(key_from).map(Arc::new).collect())
}

fn update_from(args: &Args<'_>) -> Update {
    let text = |name: &str| args.named.and_then(|named| named.get(name)).and_then(Value::as_str).map(str::to_owned);
    let number = |name: &str| args.named.and_then(|named| named.get(name)).and_then(Value::as_i64);
    let flag = |name: &str| args.named.and_then(|named| named.get(name)).and_then(Value::as_bool);
    Update {
        running: flag("running"),
        enabled: flag("enabled"),
        backend: text("backend"),
        current_group: number("current_group"),
        current_word: text("current_word"),
        correction_count: number("correction_count"),
        last_action: text("last_action"),
        last_error: text("last_error"),
        context_action: text("context_action"),
        context_model: text("context_model"),
    }
}

/// Methods a test calls only to read the answer: they leave the state as it was, or the trace would show it next.
const QUERIES: &[&str] = &[
    "_pause_delay", "_loop_timeout", "_application_excluded", "_typed_ahead", "_typed_after", "_context_for", "_layout_name",
    "_layout_names", "_keys_lean_to_target_language", "_replayed_signs", "_unshifted", "_forced_target_group",
    "_early_switch_policy", "_planned_baseline", "_completed_word", "_word_begun", "_rejected_targets", "_word_protected",
    "_learning_diagnostics", "_rule_offer", "_confidence_threshold", "_minimum_word_length", "_trigger_for_boundary",
    "_insertion_holds", "_mention_head", "_field_reader_status", "_field_reader_details", "_boundary_enabled", "_field_catch_up_seconds", "_layout_letters", "_empty_rule_offer", "_decide_prefix", "_decide_inside_word",
];

pub(crate) fn call(engine: &KeySwitchEngine, name: &str, args: &Args<'_>, recorded: &Value) -> Outcome {
    let mut guard = engine.core();
    let Some(core) = guard.as_mut() else {
        return Outcome::Unknown("the worker holds the engine".to_owned());
    };
    let shared = core.shared.clone();
    match name {
        "_handle" => match args.get(0, "event").and_then(key_from) {
            Some(event) => result_of(core.handle_inner(Arc::new(event))),
            None => Outcome::Unknown("_handle of something that is no key".to_owned()),
        },
        "enqueue" => match args.get(0, "event").and_then(key_from) {
            Some(event) => {
                shared.enqueue(event);
                done()
            }
            None => Outcome::Unknown("enqueue of something that is no key".to_owned()),
        },
        "consumes_key" => match args.get(0, "event").and_then(key_from) {
            Some(event) => answer(disposition_value(shared.consumes_key(&event))),
            None => Outcome::Unknown("consumes_key of something that is no key".to_owned()),
        },
        "_maybe_correct_after_pause" => result_of(core.maybe_correct_after_pause(args.float(usize::MAX, "now"))),
        "_remember_context" => {
            let strokes = strokes_from(args.get(2, "strokes"));
            core.remember_context(args.text(0, "application").unwrap_or(""), args.int(1, "group").unwrap_or(0), &strokes);
            done()
        }
        "_commit_word" => match args.get(0, "boundary").and_then(key_from) {
            Some(event) => result_of(core.commit_word(&Arc::new(event))),
            None => Outcome::Unknown("_commit_word of something else".to_owned()),
        },
        "_execute_correction" => match args.get(0, "plan").and_then(plan_from) {
            Some(plan) => answer(Value::Bool(core.execute_correction(Arc::new(plan), args.get(1, "learning_prompt").and_then(prompt_from)))),
            None => Outcome::Unknown("_execute_correction of a plan with a revisit".to_owned()),
        },
        "_fix_double_capitals" => match (args.get(0, "plan").and_then(plan_from), args.get(1, "boundary").and_then(key_from)) {
            (Some(plan), Some(boundary)) => {
                let fixed = core.fix_double_capitals(&Arc::new(plan), &Arc::new(boundary));
                let digest = plan_digest(&fixed);
                let expected = plan_from(recorded).map(|plan| plan_digest(&plan));
                if expected.as_ref() == Some(&digest) { answer(recorded.clone()) } else { answer(digest) }
            }
            _ => Outcome::Unknown("_fix_double_capitals of a plan with a revisit".to_owned()),
        },
        "_early_prefix_protection" => answer(json!(core.early_prefix_protection(args.text(0, "original").unwrap_or(""), args.int(1, "source").unwrap_or(0), args.int(2, "target").unwrap_or(0)))),
        "_early_prefix_field" => {
            let (field, protection) = core.early_prefix_field(args.text(0, "original").unwrap_or(""), args.text(1, "application").unwrap_or(""));
            answer(json!([super::backend::field_value(&field), protection]))
        }
        "_read_insertion" => match args.get(0, "event").and_then(key_from) {
            Some(event) => answer(core.read_insertion(&Arc::new(event)).map_or(Value::Null, |point| json!({"$dc": "InsertionPoint", "head": point.head, "tail": point.tail}))),
            None => Outcome::Unknown("_read_insertion of something else".to_owned()),
        },
        "_ambiguous_key_is_boundary" => match args.get(0, "event").and_then(key_from) {
            Some(event) => match core.ambiguous_key_is_boundary(&event) {
                Ok(boundary) => answer(Value::Bool(boundary)),
                Err(error) => Outcome::Done { returned: None, error: Some(error.to_string()) },
            },
            None => Outcome::Unknown("_ambiguous_key_is_boundary of something else".to_owned()),
        },
        "_log_input_edit" => match args.get(0, "event").and_then(key_from) {
            Some(event) => {
                core.log_input_edit(&event, args.text(1, "application").unwrap_or(""));
                done()
            }
            None => Outcome::Unknown("_log_input_edit of something else".to_owned()),
        },
        "_advance_selection_command" => {
            core.advance_selection_command();
            done()
        }
        "_run_selection_command" => {
            let command = match args.text(0, "command") {
                Some("layout") => SelectionCommand::Layout,
                Some("case") => SelectionCommand::Case,
                _ => SelectionCommand::Transliterate,
            };
            core.run_selection_command(command);
            done()
        }
        "_is_boundary" => match args.get(0, "event").and_then(key_from) {
            Some(event) => answer(Value::Bool(EngineCore::_is_boundary(&event))),
            None => Outcome::Unknown("_is_boundary of something else".to_owned()),
        },
        "_poll_current_group" => result_of(core.poll_current_group()),
        "_expire_deferred_action" => result_of(core.expire_deferred_action()),
        "_expire_manual_correction" => {
            core.expire_manual_correction();
            done()
        }
        "_expire_learning_prompt" => answer(Value::Bool(core.expire_learning_prompt(args.float(usize::MAX, "now")))),
        "_retry_field_wait" => {
            core.retry_field_wait();
            done()
        }
        "_report_unseen_input" => result_of(core.report_unseen_input()),
        "_report_input_delay" => {
            core.report_input_delay();
            done()
        }
        "_run_timers" => result_of(core.run_timers()),
        "_schedule_manual_conversion" => {
            core.schedule_manual_conversion(args.int(0, "trigger_keycode").unwrap_or(-1));
            done()
        }
        "_schedule_undo" => {
            core.schedule_undo(args.int(0, "trigger_keycode").unwrap_or(-1));
            done()
        }
        "_schedule_undo_hotkey" => {
            core.schedule_undo_hotkey(args.int(0, "trigger_keycode").unwrap_or(-1));
            done()
        }
        "_clear_word" => {
            let action = args.text(0, "action");
            let reason = args.text(usize::MAX, "reason").unwrap_or("");
            core.clear_word(action, reason);
            done()
        }
        "_observe_group" => {
            core.observe_group(args.int(0, "group").unwrap_or(0), args.text(usize::MAX, "source").unwrap_or("poll"));
            done()
        }
        "_apply_layout_selection" => {
            core.apply_layout_selection(args.int(0, "group").unwrap_or(0));
            done()
        }
        "_settings_changed" => {
            let value = args.get(1, "value").cloned().unwrap_or(Value::Null);
            core.settings_changed(args.text(0, "path").unwrap_or(""), &value);
            done()
        }
        "_update" => {
            core.update(update_from(args));
            done()
        }
        "_show_learning_prompt" => match args.get(0, "prompt").and_then(prompt_from) {
            Some(prompt) => {
                shared.show_learning_prompt(prompt);
                done()
            }
            None => Outcome::Unknown("_show_learning_prompt of something else".to_owned()),
        },
        "_technical_session_event" => {
            core.technical_session_event(args.text(0, "event").unwrap_or(""));
            done()
        }
        "_maybe_execute_pending" => match args.get(0, "event").and_then(key_from) {
            Some(event) => result_of(core.maybe_execute_pending(&event)),
            None => Outcome::Unknown("_maybe_execute_pending of something else".to_owned()),
        },
        "confirm_learning_prompt" => {
            let prompt = args.get(0, "prompt").and_then(prompt_from);
            drop(guard);
            answer(Value::Bool(engine.confirm_learning_prompt(prompt.as_ref())))
        }
        "dismiss_learning_prompt" => {
            let prompt = args.get(0, "prompt").and_then(prompt_from);
            let reason = args.text(usize::MAX, "reason").unwrap_or("dismissed");
            drop(guard);
            answer(Value::Bool(shared.dismiss_learning_prompt(prompt.as_ref(), reason)))
        }
        "set_rule_editor_open" => {
            shared.set_rule_editor_open(args.flag(0, "opened").unwrap_or(false));
            done()
        }
        "set_test_field_focused" => {
            shared.set_test_field_focused(args.flag(0, "focused").unwrap_or(false));
            done()
        }
        "select_alternate_group" => {
            drop(guard);
            answer(Value::Bool(engine.select_alternate_group()))
        }
        "add_learning_rule" => {
            let Some(rule) = args.get(0, "rule").and_then(rule_from) else {
                return Outcome::Unknown("add_learning_rule of something else".to_owned());
            };
            let replacing = args.get(usize::MAX, "replacing").and_then(rule_from);
            let application = args.text(usize::MAX, "application").unwrap_or("").to_owned();
            drop(guard);
            match engine.add_learning_rule(rule, replacing.as_ref(), &application) {
                Ok(stored) => answer(rule_returned(&stored)),
                Err(error) => Outcome::Done { returned: None, error: Some(error.to_string()) },
            }
        }
        "clear_personal_words" => {
            drop(guard);
            engine.clear_personal_words();
            done()
        }
        "stop" => {
            drop(guard);
            engine.stop();
            done()
        }
        "subscribe" | "subscribe_corrections" | "subscribe_learning_prompts" | "subscribe_rule_requests" => done(),
        _ if QUERIES.contains(&name) => Outcome::Query,
        _ => Outcome::Unknown(format!("no dispatch for {name}")),
    }
}

pub(crate) fn settings_step(core: &mut EngineCore, name: &str, args: &Args<'_>) -> Outcome {
    let settings = core.settings.clone();
    let outcome = match name {
        "set" => {
            let persist = args.flag(usize::MAX, "persist").unwrap_or(true);
            let value = args.get(1, "value").cloned().unwrap_or(Value::Null);
            result_of(settings.set(args.text(0, "dotted_path").unwrap_or(""), value, persist))
        }
        "reset" => result_of(settings.reset()),
        "restore_default" => match settings.restore_default(args.text(0, "dotted_path").unwrap_or("")) {
            Ok(changed) => answer(Value::Bool(changed)),
            Err(error) => Outcome::Done { returned: None, error: Some(error.to_string()) },
        },
        _ => return Outcome::Unknown(format!("settings.{name}")),
    };
    core.apply_commands();
    outcome
}

pub(crate) fn history_step(core: &mut EngineCore, name: &str, args: &Args<'_>) -> Outcome {
    match name {
        "clear" => result_of(core.history.clear()),
        "append" => {
            let Some(entry) = args.get(0, "entry").filter(|entry| entry.get("$dc").is_some()) else {
                return Outcome::Unknown("history.append of something else".to_owned());
            };
            let text = |field: &str| entry[field].as_str().unwrap_or("").to_owned();
            let entry = HistoryEntry::with_timestamp(text("timestamp"), &text("original"), &text("replacement"), &text("application"), entry["confidence"].as_f64().unwrap_or(0.0));
            result_of(core.history.append(&entry))
        }
        _ => Outcome::Unknown(format!("history.{name}")),
    }
}

pub(crate) fn learning_step(core: &mut EngineCore, name: &str, args: &Args<'_>) -> Outcome {
    let store = core.learning.clone();
    match name {
        "add_rule" => match args.get(0, "rule").and_then(rule_from) {
            Some(rule) => match store.add_rule(rule) {
                Ok(stored) => answer(rule_returned(&stored)),
                Err(error) => Outcome::Done { returned: None, error: Some(error.to_string()) },
            },
            None => Outcome::Unknown("add_rule of something else".to_owned()),
        },
        "replace_rule" => match (args.get(0, "old").and_then(rule_from), args.get(1, "new").and_then(rule_from)) {
            (Some(old), Some(new)) => match store.replace_rule(&old, new) {
                Ok(stored) => answer(rule_returned(&stored)),
                Err(error) => Outcome::Done { returned: None, error: Some(error.to_string()) },
            },
            _ => Outcome::Unknown("replace_rule of something else".to_owned()),
        },
        "remove_rule" => match args.get(0, "rule").and_then(rule_from) {
            Some(rule) => match store.remove_rule(&rule) {
                Ok(removed) => answer(Value::Bool(removed)),
                Err(error) => Outcome::Done { returned: None, error: Some(error.to_string()) },
            },
            None => Outcome::Unknown("remove_rule of something else".to_owned()),
        },
        "clear" => result_of(store.clear()),
        _ => Outcome::Unknown(format!("learning.{name}")),
    }
}

pub(crate) fn personal_step(core: &mut EngineCore, name: &str, args: &Args<'_>) -> Outcome {
    match name {
        "clear" => {
            core.personal.clear();
            done()
        }
        "add" => {
            core.personal.add(args.text(0, "word").unwrap_or(""));
            done()
        }
        "keep" => answer(Value::Bool(core.personal.keep(args.text(0, "word").unwrap_or("")))),
        _ => Outcome::Unknown(format!("personal.{name}")),
    }
}

pub(crate) fn queue_step(core: &mut EngineCore, name: &str, args: &Args<'_>, recorded: &Value) -> Outcome {
    match name {
        "put" => {
            let item = match args.get(0, "item") {
                Some(Value::Null) => QueueItem::Stop,
                Some(value) if value.get("$key").is_some() => match key_from(value) {
                    Some(event) => QueueItem::Key(Arc::new(event)),
                    None => return Outcome::Unknown("a queue item".to_owned()),
                },
                Some(value) if value["$dc"] == "_LayoutSelection" => QueueItem::LayoutSelection(value["group"].as_i64().unwrap_or(0)),
                _ => return Outcome::Unknown("a queue item".to_owned()),
            };
            core.shared.put_nowait(item);
            done()
        }
        "get" => {
            let taken = core.shared.get_nowait();
            if taken.is_none() && recorded.is_null() {
                // `queue.Empty`, or a queue that held None.
                return Outcome::Done { returned: None, error: Some(String::new()) };
            }
            let same = match (&taken, key_from(recorded)) {
                (Some(QueueItem::Key(event)), Some(expected)) => **event == expected,
                (Some(QueueItem::Stop), None) => recorded.is_null(),
                (Some(QueueItem::LayoutSelection(group)), None) => recorded["group"].as_i64() == Some(*group),
                _ => false,
            };
            if same { done() } else { Outcome::Done { returned: Some(json!({"queue_get": format!("{taken:?}")})), error: None } }
        }
        _ => Outcome::Unknown(format!("queue.{name}")),
    }
}
