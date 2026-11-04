//! One trace through the Rust engine.

use super::backend::{answering_model, compact, Capabilities, Tape, TraceBackend, TraceReader};
use super::compare::{first_difference, same};
use super::entries::{self, Args, Outcome};
use flate2::read::GzDecoder;
use ks_backend::{FieldReader, InputBackend};
use ks_config::config::SettingsStore;
use ks_config::history::HistoryStore;
use ks_config::logsetup;
use ks_context::input_context::{Clock, InputContext};
use ks_context::{ContextModel, ContextPolicy, VersionedPrefixModel};
use ks_engine::engine::{CorrectionPlan, EngineCore, EngineParts, EngineSnapshot, KeySwitchEngine, LearningPrompt, PlanRef, Stroke};
use ks_lang::boundary_policy::BoundaryPolicy;
use ks_lang::detector::ProtectedTokens;
use ks_lang::early_switch::PrefixIndex;
use ks_lang::intent_model::{IntentModelStatus, LinearNgramModel};
use ks_lang::{LanguageModel, Resources, WordTable};
use serde_json::{json, Value};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

/// What a trace says in place of the tree it was recorded in.
const TREE_MARK: &str = "@TREE@";

/// The tree that holds the resources and the pinned lexicons: this workspace, or the one `KS_REPLAY_TREE` names.
fn reference_tree() -> std::path::PathBuf {
    std::env::var_os("KS_REPLAY_TREE").map_or_else(
        || {
            let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            workspace.canonicalize().unwrap_or(workspace)
        },
        std::path::PathBuf::from,
    )
}

#[derive(Clone, Copy)]
enum Choice {
    Packaged,
    Reference,
    Own,
}

/// One pair of language models and what is built on them.
struct Kind {
    models: Vec<Arc<LanguageModel>>,
    prefix_indexes: Vec<Arc<PrefixIndex>>,
    policy: Option<ContextPolicy>,
    model: Option<ContextModel>,
}

pub struct Worker {
    resources: Arc<Resources>,
    packaged: Option<Kind>,
    reference: Option<Kind>,
    /// The word lists a test wrote itself, built for that trace.
    own: Option<Kind>,
    intent_model: Option<Arc<LinearNgramModel>>,
    intent_model_status: IntentModelStatus,
    protected_tokens: Arc<ProtectedTokens>,
    boundary_model: Option<Arc<BoundaryPolicy>>,
    prefix_model: Option<Arc<VersionedPrefixModel>>,
    root: PathBuf,
    log: std::fs::File,
    serial: u64,
}

struct Difference {
    cause: String,
    detail: String,
}

fn differ(cause: impl Into<String>, detail: impl Into<String>) -> Option<Difference> {
    Some(Difference { cause: cause.into(), detail: detail.into() })
}

fn snapshot_value(snapshot: &EngineSnapshot) -> Value {
    json!({"running": snapshot.running, "enabled": snapshot.enabled, "backend": snapshot.backend, "current_group": snapshot.current_group,
           "current_word": snapshot.current_word, "correction_count": snapshot.correction_count, "last_action": snapshot.last_action,
           "last_error": snapshot.last_error, "context_action": snapshot.context_action, "context_model": snapshot.context_model})
}

fn plan_value(plan: &CorrectionPlan) -> Value {
    json!({"mode": plan.mode, "original": plan.original, "replacement": plan.replacement, "source_group": plan.source_group,
           "target_group": plan.target_group, "confidence": plan.confidence, "application": plan.application,
           "automatic": plan.automatic, "strokes": plan.strokes.len(), "boundary": plan.boundary.is_some(),
           "trailing": plan.trailing.len(), "kept_tail": plan.kept_tail})
}

fn plan_brief(plan: Option<&PlanRef>) -> Value {
    plan.map_or(Value::Null, |plan| {
        json!([plan.mode, plan.original, plan.replacement, plan.source_group, plan.target_group, plan.automatic, plan.strokes.len()])
    })
}

fn keys_text(strokes: &[Stroke]) -> Value {
    json!([strokes.iter().map(|stroke| stroke.character.as_str()).collect::<String>(), strokes.len()])
}

/// The state both engines hold under one name.
fn internals(core: &EngineCore) -> Value {
    let shared = &core.shared;
    let sorted = |keys: &std::collections::HashSet<u32>| {
        let mut keys: Vec<u32> = keys.iter().copied().collect();
        keys.sort_unstable();
        keys
    };
    let mut pressed_since: Vec<(u32, f64)> = core._pressed_since.clone();
    pressed_since.sort_by_key(|entry| entry.0);
    let stream = &core.context_policy.stream;
    json!({
        "_source_group": core._source_group,
        "_manual_layout_group": core._manual_layout_group,
        "_pending_trigger_keycode": core._pending_trigger_keycode,
        "_pause_correction_pending": core._pause_correction_pending,
        "_last_committed_stale": core._last_committed_stale,
        "_caret_moved": core._caret_moved,
        "_position_unknown": core._position_unknown,
        "_early_switch_undone": core._early_switch_undone,
        "_early_switch_origin": core._early_switch_origin,
        "_manual_layout_word": core._manual_layout_word,
        "_manual_layout_source": core._manual_layout_source,
        "_last_key_character": core._last_key_character,
        "_character_before_key": core._character_before_key,
        "_untracked_token": core._untracked_token,
        "_foreign_input_active": core._foreign_input_active,
        "_own_layout_ignored": core._own_layout_ignored,
        "_convert_keycode": core._convert_keycode,
        "_convert_released": core._convert_released,
        "_personal_pending": core._personal_pending,
        "_late_refusal": core._late_refusal,
        "_correction_sequence": core._correction_sequence,
        "_context_wait_sequence": core._context_wait_sequence,
        "_last_word_input_at": core._last_word_input_at,
        "_action_deadline": core._action_deadline,
        "_last_correction_time": core._last_correction_time,
        "_manual_release_deadline": core._manual_release_deadline,
        "_convert_press_at": core._convert_press_at,
        "_engine_switch_at": core._engine_switch_at,
        "_engine_switch_group": core._engine_switch_group,
        "_early_switch_at": core._early_switch_at,
        "_manual_layout_observed_at": core._manual_layout_observed_at,
        "_input_delay_due": core._input_delay_due,
        "_pause_deferral_logged": core._pause_deferral_logged,
        "_wait_deferral_logged": core._wait_deferral_logged,
        "_pause_agreement_open": core._pause_agreement_open,
        "_focus_window": shared._focus_window(),
        "_sensitive_context_window": shared._sensitive_context_window(),
        "_prompt_key_deadline": shared._prompt_key_deadline(),
        "_input_suspended": shared._input_suspended(),
        "_test_field_focused": shared._test_field_focused(),
        "_typed_events": shared._typed_events.load(Ordering::Relaxed),
        "_typed_presses": shared._typed_presses.load(Ordering::Relaxed),
        "_pressed": sorted(&core._pressed),
        "_modifier_keycodes": sorted(&core._modifier_keycodes),
        "_pressed_since": pressed_since.iter().map(|entry| json!([entry.0, entry.1])).collect::<Vec<_>>(),
        "_strokes": keys_text(&core._strokes),
        "_symbol_strokes": keys_text(&core._symbol_strokes),
        "_kept": core._kept.len(),
        "_pending": plan_brief(core._pending.as_ref()),
        "_last_committed": plan_brief(core._last_committed.as_ref()),
        "_last_correction": plan_brief(core._last_correction.as_ref()),
        "_correction_origin": plan_brief(core._correction_origin.as_ref()),
        "_last_requested_plan": plan_brief(core._last_requested_plan.as_ref()),
        "_context_waiting": core._context_waiting.as_ref().map(|waiting| {
            json!([plan_brief(Some(&waiting.plan)), waiting.window, waiting.deadline, waiting.diagnostic_id, waiting.settles_on_pause, waiting.converted])
        }),
        "_provisional": core._provisional.is_some(),
        "_paused_conversion": core._paused_conversion.is_some(),
        "_field_wait": core._field_wait.is_some(),
        "_selection_pending": core._selection_pending.is_some(),
        "_learning_prompt_after": core._learning_prompt_after.is_some(),
        "_deferred_action": core._deferred_action.as_ref().map(|event| event.key_name.as_str()),
        "_mention_shown": core._mention_shown.as_ref().map(|event| event.key_name.as_str()),
        "_insertion": core._insertion.as_ref().map(|point| json!([point.head, point.tail])),
        "_contexts": core._contexts.iter().map(|(name, context)| {
            let words: serde_json::Map<String, Value> = context.words.iter().map(|(group, word)| (group.to_string(), json!(word))).collect();
            json!([name, context.group, words, context.updated_at])
        }).collect::<Vec<_>>(),
        "stream": [stream.application, stream.window, stream.text, stream.updated_at],
        "_input_overflow": shared._input_overflow.load(Ordering::Relaxed),
        "queue": shared.qsize(),
    })
}

/// What differs between runs for reasons that are no behaviour: the version of the program and the Python's own build.
fn normalized_line(line: &str) -> Option<Value> {
    let payload = line.strip_prefix("TECHNICAL ")?;
    let mut value: Value = serde_json::from_str(payload).ok()?;
    if let Some(object) = value.as_object_mut() {
        for field in ["keyswitch_version", "build_platform", "field_reader_details"] {
            object.remove(field);
        }
    }
    Some(value)
}

fn event_name(line: &str) -> String {
    normalized_line(line).and_then(|value| value["event"].as_str().map(str::to_owned)).unwrap_or_else(|| "?".to_owned())
}

/// A path without its indices, so that causes group.
pub fn general(path: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for character in path.chars() {
        match character {
            '[' => depth += 1,
            ']' => depth -= 1,
            _ if depth == 0 => out.push(character),
            _ => {}
        }
    }
    out
}

fn compare_logs(expected: &[Value], actual: &[String]) -> Option<Difference> {
    for (index, expected_line) in expected.iter().enumerate() {
        let expected_line = expected_line.as_str().unwrap_or("");
        let Some(actual_line) = actual.get(index) else {
            return differ(format!("log: missing {}", event_name(expected_line)), compact(&json!(expected_line)));
        };
        if expected_line == actual_line {
            continue;
        }
        let (left, right) = (normalized_line(expected_line), normalized_line(actual_line));
        let (Some(mut left), Some(mut right)) = (left, right) else {
            return differ("log: a line that is no event", format!("python {expected_line}; rust {actual_line}"));
        };
        // The platform reader's own status lives in that reader, which a trace answers for.
        if right["field_reader_status"] == "custom_reader" && left["field_reader_status"] != "custom_reader" {
            left.as_object_mut().map(|object| object.remove("field_reader_status"));
            right.as_object_mut().map(|object| object.remove("field_reader_status"));
        }
        // An exception's class is the Python's; the port has its own names for the same failure.
        if left["event"] == "history_write_failed" {
            left.as_object_mut().map(|object| object.remove("error"));
            right.as_object_mut().map(|object| object.remove("error"));
        }
        // The Python names each backend's exception class; the port has one failure type.
        if left["event"] == "selection_command" && right["error"] == "BackendError" && left["error"] != "LayoutSwitchUnconfirmed" {
            left.as_object_mut().map(|object| object.remove("error"));
            right.as_object_mut().map(|object| object.remove("error"));
        }
        if let Some((path, a, b)) = first_difference(&left, &right, "") {
            let (left_event, right_event) = (event_name(expected_line), event_name(actual_line));
            if left_event != right_event {
                return differ(
                    format!("log: {left_event} expected, {right_event} written"),
                    format!("python {}; rust {}", compact(&left), compact(&right)),
                );
            }
            return differ(format!("log: {left_event}{}", general(&path)), format!("python {}; rust {}; line {}", compact(&a), compact(&b), compact(&left)));
        }
    }
    if let Some(extra) = actual.get(expected.len()) {
        return differ(format!("log: extra {}", event_name(extra)), extra.clone());
    }
    None
}

impl Worker {
    pub fn new() -> Worker {
        let resources = Resources::new(reference_tree().join("resources"));
        let (intent_model, intent_model_status) = resources.intent_model();
        let root = std::env::temp_dir().join(format!("ks-trace-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("a folder for the stores");
        let log_path = root.join("technical.log");
        let handler = logsetup::RotatingFileHandler::new(&log_path, 1 << 40, 0).expect("the log");
        logsetup::install_handler(&handler);
        logsetup::set_root_level(logsetup::Level::Info);
        let log = std::fs::File::open(&log_path).expect("the log, to read");
        Worker {
            protected_tokens: resources.protected_tokens(),
            boundary_model: resources.boundary_policy(),
            prefix_model: VersionedPrefixModel::default_model(&resources.resource_dir().join("models")).map(Arc::new),
            intent_model,
            intent_model_status,
            resources,
            packaged: None,
            reference: None,
            own: None,
            root,
            log,
            serial: 0,
        }
    }

    pub fn clean(&self) {
        logsetup::uninstall_handler();
        let _ = std::fs::remove_dir_all(&self.root);
    }

    fn new_lines(&mut self) -> Vec<String> {
        let mut text = String::new();
        let _ = self.log.read_to_string(&mut text);
        text.lines().filter_map(|line| line.find("TECHNICAL ").map(|at| line[at..].to_owned())).collect()
    }

    fn slot(&mut self, choice: Choice) -> &mut Option<Kind> {
        match choice {
            Choice::Packaged => &mut self.packaged,
            Choice::Reference => &mut self.reference,
            Choice::Own => &mut self.own,
        }
    }

    fn kind(&mut self, wanted: &[Value]) -> Option<Choice> {
        if wanted.iter().all(|model| model.get("frequencies").is_some()) && !wanted.is_empty() {
            let models: Vec<Arc<LanguageModel>> = wanted
                .iter()
                .map(|model| {
                    let pairs = |field: &str| -> Vec<(String, i64)> {
                        model[field].as_object().map_or_else(Vec::new, |map| map.iter().map(|(word, count)| (word.clone(), count.as_i64().unwrap_or(0))).collect())
                    };
                    let frequencies = WordTable::from_pairs(pairs("frequencies"));
                    let bigrams = WordTable::from_pairs(pairs("bigrams"));
                    Arc::new(LanguageModel::new(model["locale"].as_str().unwrap_or(""), frequencies, model["source"].as_str().unwrap_or(""), bigrams, LanguageModel::disabled_speller()))
                })
                .collect();
            let prefix_indexes = models.iter().map(|model| Arc::new(PrefixIndex::from_lexicon(model.frequencies.clone(), &model.source))).collect();
            self.own = Some(self.kind_of(models, prefix_indexes));
            return Some(Choice::Own);
        }
        let source = |index: usize| wanted.get(index).and_then(|model| model["source"].as_str()).unwrap_or("");
        let words = |index: usize| wanted.get(index).and_then(|model| model["words"].as_u64()).unwrap_or(0) as usize;
        if self.packaged.is_none() {
            let models: Vec<Arc<LanguageModel>> = ["en_US", "ru_RU"].iter().filter_map(|locale| self.resources.engine_language_model(locale).ok()).collect();
            let prefix_indexes = models.iter().map(|model| self.resources.prefix_index(model)).collect();
            self.packaged = Some(self.kind_of(models, prefix_indexes));
        }
        let fits = |kind: &Kind| {
            kind.models.len() == wanted.len()
                && kind.models.iter().enumerate().all(|(index, model)| model.source == source(index) && model.frequencies.len() == words(index))
        };
        if self.packaged.as_ref().is_some_and(fits) {
            return Some(Choice::Packaged);
        }
        if self.reference.is_none() {
            let models: Vec<Arc<LanguageModel>> = ["en_US", "ru_RU"]
                .iter()
                .map(|locale| {
                    let supplement = self.resources.supplement_words(locale).expect("the packaged supplement");
                    let roots = vec![reference_tree().join("model/intent_v1/sources")];
                    Arc::new(LanguageModel::load(locale, supplement.iter(), &roots, LanguageModel::disabled_speller()))
                })
                .collect();
            let prefix_indexes = models.iter().map(|model| Arc::new(PrefixIndex::from_lexicon(model.frequencies.clone(), &model.source))).collect();
            self.reference = Some(self.kind_of(models, prefix_indexes));
        }
        self.reference.as_ref().is_some_and(fits).then_some(Choice::Reference)
    }

    fn kind_of(&self, models: Vec<Arc<LanguageModel>>, prefix_indexes: Vec<Arc<PrefixIndex>>) -> Kind {
        let known = EngineCore::known_of(&models);
        let mut policy = ContextPolicy::new(&self.resources, None, known, Some(Arc::new(|| 0.0)));
        let model = policy.model.take();
        Kind { models, prefix_indexes, policy: Some(policy), model }
    }

    pub fn run_file(&mut self, path: &Path, verbose: bool) -> Value {
        let file_name = path.to_string_lossy().into_owned();
        let read = std::fs::File::open(path).ok().and_then(|file| {
            let mut text = String::new();
            GzDecoder::new(file).read_to_string(&mut text).ok()?;
            let text = text.replace(TREE_MARK, &reference_tree().to_string_lossy());
            serde_json::from_str::<Value>(&text).ok()
        });
        let Some(trace) = read else {
            return json!({"file": file_name, "status": "not_run", "cause": "unreadable trace"});
        };
        let test = trace["test"].as_str().unwrap_or("?").to_owned();
        let empty = trace["steps"].as_array().is_none_or(Vec::is_empty);
        let mut result = if !trace["truncated"].is_null() && empty {
            // The test changed what the engine runs on before its first key: nothing here can be compared.
            json!({"status": "left_out", "steps_run": 0})
        } else {
            self.replay(&trace, &test, verbose)
        };
        result["file"] = json!(file_name);
        result["test"] = json!(test);
        if !trace["truncated"].is_null() {
            result["truncated"] = trace["truncated"].clone();
        }
        result
    }

    fn replay(&mut self, trace: &Value, test: &str, verbose: bool) -> Value {
        let wanted = trace["models"].as_array().cloned().unwrap_or_default();
        let Some(packaged) = self.kind(&wanted) else {
            let names: Vec<String> = wanted.iter().map(|model| format!("{}:{}", model["locale"].as_str().unwrap_or(""), model["words"])).collect();
            return json!({"status": "not_run", "cause": format!("language models {}", names.join(","))});
        };
        let policy_wanted = &trace["policy"];
        let steps = trace["steps"].as_array().cloned().unwrap_or_default();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.run_steps(trace, &steps, packaged, policy_wanted, verbose)));
        // A panic leaves no policy behind: the next trace builds it again.
        let kind_slot = self.slot(packaged);
        if kind_slot.as_ref().is_some_and(|kind| kind.policy.is_none()) {
            *kind_slot = None;
        }
        match outcome {
            Ok((count, None)) => json!({"status": "passed", "steps_run": count}),
            Ok((count, Some(found))) => {
                let status = if found.cause.starts_with("not run:") { "not_run" } else { "differed" };
                let at_test = steps.iter().take(count + 1).rev().find_map(|step| step["test"].as_str()).unwrap_or(test);
                json!({"status": status, "cause": found.cause.trim_start_matches("not run: "), "detail": found.detail, "steps_run": count,
                       "step": count, "entry": steps.get(count).map(|step| format!("{} {}", step["n"].as_str().unwrap_or(""), compact(&step["a"]))), "at_test": at_test})
            }
            Err(payload) => {
                let message = payload.downcast_ref::<String>().cloned().or_else(|| payload.downcast_ref::<&str>().map(|text| (*text).to_owned())).unwrap_or_default();
                let short: String = message.chars().take(160).collect();
                json!({"status": "differed", "cause": format!("panic: {short}"), "detail": message})
            }
        }
    }

    fn run_steps(&mut self, trace: &Value, steps: &[Value], packaged: Choice, _policy_wanted: &Value, verbose: bool) -> (usize, Option<Difference>) {
        self.serial += 1;
        let root = self.root.join(self.serial.to_string());
        std::fs::create_dir_all(&root).expect("a folder for the stores");
        let settings_path = root.join("settings.json");
        std::fs::write(&settings_path, trace["settings"].to_string()).expect("the settings file");
        let write = |name: &str, field: &str| {
            if let Some(text) = trace["files"][field].as_str() {
                std::fs::write(root.join(name), text).expect("a store's file");
            }
        };
        write("history.jsonl", "history");
        if trace["files"]["history_is_folder"] == true {
            std::fs::create_dir_all(root.join("history.jsonl")).expect("a folder in place of the history");
        }
        write("learning.json", "learning");
        write("personal-words.json", "personal");
        let settings = Arc::new(SettingsStore::new(Some(settings_path)));
        let snapshot = Value::Object(settings.snapshot());
        if let Some((path, a, b)) = first_difference(&trace["settings"], &snapshot, "") {
            let _ = std::fs::remove_dir_all(&root);
            return (0, differ(format!("not run: settings {path}"), format!("python {a}; rust {b}")));
        }
        let history = Arc::new(HistoryStore::new(Some(root.join("history.jsonl")), trace["history_limit"].as_i64().unwrap_or(200)));
        let clock_cell = Arc::new(AtomicU64::new(trace["clock"].as_f64().unwrap_or(0.0).to_bits()));
        let clock: Clock = {
            let cell = clock_cell.clone();
            Arc::new(move || f64::from_bits(cell.load(Ordering::Relaxed)))
        };
        let tape = Arc::new(Mutex::new(Tape::new(clock_cell.clone(), trace["status"].as_str().unwrap_or("not_requested").to_owned())));
        let caps = &trace["capabilities"];
        let capability = |name: &str| caps[name].as_bool().unwrap_or(false);
        let capabilities = Capabilities {
            take_unseen_input: capability("take_unseen_input"),
            take_input_delay: capability("take_input_delay"),
            take_hook_gap: capability("take_hook_gap"),
            input_anchor: capability("input_anchor"),
            restore_window: capability("restore_window"),
            keep_window_inactive: capability("keep_window_inactive"),
            selection: capability("selection"),
            erases_any_length: capability("erases_any_length"),
            swallows_hotkeys: capability("swallows_hotkeys"),
        };
        let backend: Arc<dyn InputBackend> = Arc::new(TraceBackend { tape: tape.clone(), capabilities });
        let reader: Arc<dyn FieldReader> = Arc::new(TraceReader { tape: tape.clone() });
        let (mut policy, models, prefix_indexes, context_model) = {
            let kind = self.slot(packaged).as_mut().expect("the kind chosen above");
            (kind.policy.take().expect("one trace at a time"), kind.models.clone(), kind.prefix_indexes.clone(), kind.model.take())
        };
        policy.stream = InputContext::with_clock(clock.clone());
        policy.reader = (!trace["reader_type"].as_str().is_some_and(|name| name == "NoneType")).then(|| reader.clone());
        policy.model = context_model;
        {
            let mut tape = tape.lock().unwrap_or_else(PoisonError::into_inner);
            tape.load(trace["init_io"].as_array().map_or(&[], Vec::as_slice));
            for step in steps {
                tape.look_ahead(step["io"].as_array().map_or(&[], Vec::as_slice));
            }
        }
        let _ = self.new_lines();
        let core = EngineCore::from_parts(EngineParts {
            settings,
            history,
            backend,
            learning: None,
            backend_label: trace["backend_label"].as_str().unwrap_or("").to_owned(),
            models,
            intent_model: self.intent_model.clone(),
            intent_model_status: self.intent_model_status.clone(),
            protected_tokens: self.protected_tokens.clone(),
            context_policy: policy,
            boundary_model: self.boundary_model.clone(),
            prefix_model: self.prefix_model.clone(),
            prefix_indexes,
            resource_dir: self.resources.resource_dir().to_path_buf(),
            clock,
            platform_reader: None,
        });
        let core = match core {
            Ok(core) => core,
            Err(error) => panic!("the engine was not built: {error}"),
        };
        tape.lock().unwrap_or_else(PoisonError::into_inner).shared = Arc::downgrade(&core.shared);
        let plans = Arc::new(Mutex::new(Vec::<Value>::new()));
        let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
        {
            let plans = plans.clone();
            core.shared.subscribe_corrections(Arc::new(move |plan: &CorrectionPlan| plans.lock().unwrap_or_else(PoisonError::into_inner).push(plan_value(plan))));
            let requests = requests.clone();
            core.shared.subscribe_rule_requests(Arc::new(move |prompt: &LearningPrompt| requests.lock().unwrap_or_else(PoisonError::into_inner).push(entries::prompt_value(prompt))));
        }
        let engine = KeySwitchEngine::of(core);
        let mut state = Expected { has_input_delay: caps["take_input_delay"] == true, internals: trace["internals"].clone(), snapshot: trace["snapshot"].clone(), history: trace["files"]["history"].is_null().then(|| json!([])), rules: None, prompt: Value::Null };
        let mut found = None;
        let init_lines = self.new_lines();
        let mut done = 0;
        if let Some(difference) = compare_logs(trace["init_logs"].as_array().map_or(&[], Vec::as_slice), &init_lines) {
            found = Some(difference);
        }
        let mut saved_model = None;
        if found.is_none() {
            for step in steps {
                if verbose {
                    eprintln!("step {done}: {} {}", step["n"].as_str().unwrap_or(""), compact(&step["a"]));
                }
                if let Some(difference) = self.run_step(step, &engine, &tape, &clock_cell, &reader, &plans, &requests, &mut state, &mut saved_model) {
                    found = Some(difference);
                    break;
                }
                done += 1;
            }
        }
        let core = engine.core().take().expect("the engine was never started");
        let mut policy = core.context_policy_out();
        policy.reader = None;
        let model = policy.model.take().filter(|model| model.answer.is_none()).or(saved_model);
        let kind = self.slot(packaged).as_mut().expect("the kind chosen above");
        kind.policy = Some(policy);
        kind.model = model;
        let _ = std::fs::remove_dir_all(&root);
        (done, found)
    }

    #[allow(clippy::too_many_arguments)]
    fn run_step(
        &mut self, step: &Value, engine: &KeySwitchEngine, tape: &Arc<Mutex<Tape>>, clock: &Arc<AtomicU64>, reader: &Arc<dyn FieldReader>,
        plans: &Arc<Mutex<Vec<Value>>>, requests: &Arc<Mutex<Vec<Value>>>, state: &mut Expected, saved_model: &mut Option<ContextModel>,
    ) -> Option<Difference> {
        clock.store(step["c"].as_f64().unwrap_or(0.0).to_bits(), Ordering::Relaxed);
        {
            let mut tape = tape.lock().unwrap_or_else(PoisonError::into_inner);
            tape.load(step["io"].as_array().map_or(&[], Vec::as_slice));
            if let Some(status) = step["status"].as_str() {
                tape.status = status.to_owned();
            }
        }
        if let Some(attributes) = step.get("model_attributes") {
            let mut guard = engine.core();
            if let Some(model) = guard.as_mut().and_then(|core| core.context_policy.model.as_mut()) {
                let flag = |name: &str| attributes[name].as_bool().unwrap_or(false);
                model.answers_letters = flag("answers_letters");
                model.answers_variables = flag("answers_variables");
                model.answers_alnum = flag("answers_alnum");
                model.answers_resume = flag("answers_resume");
                model.answers_cyrletter = flag("answers_cyrletter");
                model.answers_digit_dot = flag("answers_digit_dot");
                model.conversion_threshold = attributes["conversion_threshold"].as_f64().unwrap_or(model.conversion_threshold);
            }
        }
        let name = step["n"].as_str().unwrap_or("");
        let positional = step["a"].as_array().map_or(&[][..], Vec::as_slice);
        let args = Args { positional, named: step["kw"].as_object() };
        let outcome = match step["k"].as_str().unwrap_or("") {
            "call" => entries::call(engine, name, &args, &step["r"]),
            kind => {
                let mut guard = engine.core();
                let core = guard.as_mut().expect("no worker in a replay");
                match kind {
                    "settings" => entries::settings_step(core, name, &args),
                    "history" => entries::history_step(core, name, &args),
                    "learning" => entries::learning_step(core, name, &args),
                    "personal" => entries::personal_step(core, name, &args),
                    "queue" => entries::queue_step(core, name, &args, &step["r"]),
                    "policy" => {
                        match name {
                            "reader" => core.context_policy.reader = (!args.positional[0].is_null()).then(|| reader.clone()),
                            _ => {
                                let old = core.context_policy.model.take();
                                if let Some(old) = old
                                    && old.answer.is_none()
                                {
                                    *saved_model = Some(old);
                                }
                                if let (Some(attributes), Some(real)) = (args.positional[0].as_object(), saved_model.as_ref()) {
                                    let attributes = Value::Object(attributes.clone());
                                    core.context_policy.model = Some(answering_model(tape.clone(), &attributes, real.tables.clone()));
                                }
                            }
                        }
                        Outcome::Done { returned: None, error: None }
                    }
                    "engine" => {
                        match name {
                            "boundary_model" => core.boundary_model = None,
                            _ => core.prefix_model = None,
                        }
                        Outcome::Done { returned: None, error: None }
                    }
                    other => Outcome::Unknown(format!("step kind {other}")),
                }
            }
        };
        let mut tape_guard = tape.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((cause, detail)) = tape_guard.difference.take() {
            return differ(format!("call: {cause}"), detail);
        }
        match outcome {
            Outcome::Unknown(what) => return differ(format!("not run: {what}"), compact(step)),
            Outcome::Query => {
                drop(tape_guard);
                let _ = self.new_lines();
                return None;
            }
            Outcome::Done { returned, error } => {
                if let Some(left) = tape_guard.leftover() {
                    return differ(format!("call: python made {} more", left.split(' ').next().unwrap_or("")), left);
                }
                match (&step.get("x"), &error) {
                    (Some(raised), Some(message)) => {
                        if raised["message"].as_str() != Some(message) {
                            return differ("raises another error", format!("python {}; rust {message}", compact(raised)));
                        }
                    }
                    (Some(raised), None) => return differ("python raises, rust does not", compact(raised)),
                    (None, Some(message)) => return differ("rust raises, python does not", message.clone()),
                    (None, None) => {}
                }
                if let Some(value) = returned
                    && step.get("x").is_none()
                    && !same(&step["r"], &value)
                {
                    return differ(format!("result of {name}"), format!("python {}; rust {}", compact(&step["r"]), compact(&value)));
                }
            }
        }
        drop(tape_guard);
        let observed = &step["o"];
        let lines = self.new_lines();
        let mut guard = engine.core();
        let core = guard.as_mut()?;
        if let Some(found) = compare_logs(observed["l"].as_array().map_or(&[], Vec::as_slice), &lines) {
            return Some(found);
        }
        if let Some(snapshot) = observed.get("s") {
            state.snapshot = snapshot.clone();
        }
        if let Some((path, a, b)) = first_difference(&state.snapshot, &snapshot_value(&core.shared.snapshot()), "") {
            // The text of an operating system error is each language's own after this prefix.
            let own_words = |value: &Value| value.as_str().is_some_and(|text| text.starts_with("История не записана: "));
            if !(own_words(&a) && own_words(&b)) {
                return differ(format!("snapshot{path}"), format!("python {a}; rust {b}"));
            }
        }
        if let Some(changed) = observed.get("i").and_then(Value::as_object) {
            for (name, value) in changed {
                state.internals[name] = value.clone();
            }
        }
        let mut held = internals(core);
        // Without `take_input_delay` the Python returns before moving the deadline; a trait's default cannot be told from an answer.
        if !state.has_input_delay {
            state.internals.as_object_mut().map(|object| object.remove("_input_delay_due"));
            held.as_object_mut().map(|object| object.remove("_input_delay_due"));
        }
        if let Some((path, a, b)) = first_difference(&state.internals, &held, "") {
            return differ(format!("state{}", general(&path)), format!("{path}: python {}; rust {}", compact(&a), compact(&b)));
        }
        if let Some(history) = observed.get("h") {
            state.history = Some(history.clone());
        }
        if let Some(expected) = &state.history {
            let actual: Vec<Value> = core.history.read(None).unwrap_or_default().iter().map(|entry| json!([entry.original, entry.replacement, entry.application, entry.confidence])).collect();
            if !same(expected, &Value::Array(actual.clone())) {
                return differ("history", format!("python {}; rust {}", compact(expected), compact(&Value::Array(actual))));
            }
        }
        if let Some(rules) = observed.get("L") {
            state.rules = Some(rules.clone());
        }
        if let Some(expected) = &state.rules {
            let actual: Vec<Value> = core.learning.rules().iter().map(entries::rule_value).collect();
            if !same(expected, &Value::Array(actual.clone())) {
                return differ("learning rules", format!("python {}; rust {}", compact(expected), compact(&Value::Array(actual))));
            }
        }
        if let Some(personal) = observed.get("p") {
            let count = core._personal.len() as i64;
            if personal["n"].as_i64() != Some(count) {
                return differ("personal words", format!("python {}; rust {count}", compact(personal)));
            }
            for word in personal["w"].as_array().map_or(&[][..], Vec::as_slice) {
                let word = word.as_str().unwrap_or("");
                if !core._personal.contains(word) {
                    return differ("personal words", format!("the Rust store lacks {word}"));
                }
            }
        }
        if let Some(prompt) = observed.get("q") {
            state.prompt = prompt.clone();
        }
        let prompt = core.shared.learning_prompt().map_or(Value::Null, |prompt| entries::prompt_value(&prompt));
        if !same(&state.prompt, &prompt) {
            return differ("learning prompt", format!("python {}; rust {}", compact(&state.prompt), compact(&prompt)));
        }
        let taken = std::mem::take(&mut *plans.lock().unwrap_or_else(PoisonError::into_inner));
        let expected = observed.get("P").cloned().unwrap_or_else(|| json!([]));
        if let Some((path, a, b)) = first_difference(&expected, &Value::Array(taken), "") {
            return differ(format!("plan{}", general(&path)), format!("python {a}; rust {b}"));
        }
        let taken = std::mem::take(&mut *requests.lock().unwrap_or_else(PoisonError::into_inner));
        let expected = observed.get("R").cloned().unwrap_or_else(|| json!([]));
        if let Some((path, a, b)) = first_difference(&expected, &Value::Array(taken), "") {
            return differ(format!("rule request{path}"), format!("python {a}; rust {b}"));
        }
        None
    }
}

struct Expected {
    has_input_delay: bool,
    internals: Value,
    snapshot: Value,
    history: Option<Value>,
    rules: Option<Value>,
    prompt: Value,
}

