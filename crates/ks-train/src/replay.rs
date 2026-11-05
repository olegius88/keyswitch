//! A sequence of physical keys typed through the real engine over a model editor, on a fake clock
//! (`evaluate_context_action_sequences.replay`): how a candidate is measured and how span frames are captured.
use ks_cast::To as _;
use crate::physical::PhysicalKey;
use crate::tools::{Lexicons, Tools};
use ks_backend::KeyEvent;
use ks_backend::constants::SHIFT_MASK;
use ks_config::config::SettingsStore;
use ks_config::history::HistoryStore;
use ks_context::context_model::{ContextAction, ContextEvidence, ContextModel, ContextPrediction};
use ks_context::context_policy::SharedOrtho;
use ks_context::flat_table::WeightTable;
use ks_context::input_context::InputContext;
use ks_context::{ContextPolicy, VersionedPrefixModel};
use ks_engine::engine::{EngineCore, EngineParts};
use ks_lang::boundary_policy::BoundaryPolicy;
use ks_lang::detector::ProtectedTokens;
use ks_lang::early_switch::PrefixIndex;
use ks_lang::intent_model::{IntentModelStatus, LinearNgramModel};
use ks_replay::harness::{Backend, Correction, FakeClock, UnreadField};
use ks_text::pystr;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

const SEQUENCE_PROTOCOL_PAUSE_DELAY_SECONDS: f64 = 1.5;
const SIMULATED_KEY_DOWN_SECONDS: f64 = 0.05;
const SIMULATED_KEY_UP_SECONDS: f64 = 0.03;
const SIMULATED_WORD_IDLE_SECONDS: f64 = 1.7;
const MILLISECONDS_PER_SECOND: f64 = 1000.0;
const SPAN_CAPTURE_VERSION: &str = "context-v3-span-capture";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    EarlyOff,
    Default,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::EarlyOff => "early_off",
            Mode::Default => "default",
        }
    }
}

/// `SequencePlan`.
#[derive(Clone, Debug)]
pub struct Plan {
    pub identifier: String,
    pub document: String,
    pub group: Option<i64>,
    pub initially_wrong: bool,
    pub expected: String,
    pub keys: Vec<PhysicalKey>,
    pub unsupported: Vec<String>,
    pub trimmed_left: usize,
    pub trimmed_right: usize,
    pub expected_final_group: i64,
    pub intervention_event: Option<usize>,
}

impl Plan {
    /// A plan of keys chosen by the caller, with nothing trimmed and no intervention.
    pub fn typed(identifier: String, document: String, group: i64, expected: String, keys: Vec<PhysicalKey>, expected_final_group: i64) -> Plan {
        Plan { identifier, document, group: Some(group), initially_wrong: false, expected, keys, unsupported: Vec::new(), trimmed_left: 0, trimmed_right: 0, expected_final_group, intervention_event: None }
    }
}

/// What `replay` returns.
#[derive(Debug)]
pub struct Outcome {
    pub mode: Mode,
    pub actual: String,
    pub exact: bool,
    pub length_mismatch: bool,
    pub final_group: i64,
    pub expected_final_group: i64,
    pub final_layout_matches: bool,
    pub corrections: Vec<Correction>,
    pub intervention: Value,
    pub error: Option<String>,
}

impl Outcome {
    pub fn value(&self) -> Value {
        let corrections: Vec<Value> = self
            .corrections
            .iter()
            .map(|item| {
                json!({"event": item.event, "before": item.before, "after": item.after, "source_group": item.source_group, "target_group": item.target_group,
                    "actual_group": item.actual_group, "caret_before": item.caret_before, "caret_after": item.caret_after})
            })
            .collect();
        json!({"mode": self.mode.as_str(), "actual": self.actual, "exact": self.exact, "length_mismatch": self.length_mismatch, "final_group": self.final_group,
            "expected_final_group": self.expected_final_group, "final_layout_matches": self.final_layout_matches, "corrections": corrections,
            "intervention": self.intervention, "error": self.error})
    }
}

pub struct Capture {
    pub evidence: ContextEvidence,
    pub observed: String,
    pub event: usize,
}

#[derive(Default)]
struct Recording {
    backend: Option<Arc<Backend>>,
    serial: usize,
    records: Vec<Capture>,
}

/// One engine after another over the shared models.
pub struct Replayer {
    resource_dir: PathBuf,
    models: Lexicons,
    intent_model: Option<Arc<LinearNgramModel>>,
    intent_model_status: IntentModelStatus,
    protected_tokens: Arc<ProtectedTokens>,
    boundary_model: Option<Arc<BoundaryPolicy>>,
    prefix_model: Option<Arc<VersionedPrefixModel>>,
    prefix_indexes: Vec<Arc<PrefixIndex>>,
    context_policy: Option<ContextPolicy>,
    recording: Option<Arc<Mutex<Recording>>>,
    folder: PathBuf,
}

/// The prefix index of each lexicon, as a replay hands the engine its reference models.
pub fn prefix_indexes(models: &Lexicons) -> Vec<Arc<PrefixIndex>> {
    models.iter().map(|model| Arc::new(PrefixIndex::from_lexicon(model.frequencies.clone(), &model.source))).collect()
}

impl Replayer {
    /// Every replay of a fit or an evaluation shares `shared`: the orthotactic model bound to the first profile's
    /// dictionaries, as the first replay of a process binds it.
    pub fn new(tools: &Tools, models: &Lexicons, shared: SharedOrtho, folder: PathBuf, model: ContextModel, prefix: Option<Arc<VersionedPrefixModel>>, indexes: Vec<Arc<PrefixIndex>>) -> Replayer {
        let resources = &tools.resources;
        let (intent_model, intent_model_status) = resources.intent_model();
        let mut context_policy = ContextPolicy::with_ortho(resources, None, shared, None);
        context_policy.model = Some(model);
        std::fs::create_dir_all(&folder).expect("a folder for the stores");
        Replayer {
            resource_dir: resources.resource_dir().to_path_buf(),
            models: models.clone(),
            intent_model,
            intent_model_status,
            protected_tokens: resources.protected_tokens(),
            boundary_model: resources.boundary_policy(),
            prefix_model: prefix,
            prefix_indexes: indexes,
            context_policy: Some(context_policy),
            recording: None,
            folder,
        }
    }

    /// `_Recorder`: every question recorded and answered `keep`.
    pub fn recording(tools: &Tools, models: &Lexicons, shared: SharedOrtho, folder: PathBuf) -> Replayer {
        let recording = Arc::new(Mutex::new(Recording::default()));
        let mut recorder = ContextModel::new(WeightTable::empty(), SPAN_CAPTURE_VERSION, 0.99, tools.tables.clone());
        let handle = recording.clone();
        recorder.answer = Some(Arc::new(move |item: &ContextEvidence| {
            let mut state = handle.lock().unwrap_or_else(PoisonError::into_inner);
            let observed: String = state.backend.as_ref().expect("a replay is running").text().iter().collect();
            let event = state.serial;
            state.records.push(Capture { evidence: item.clone(), observed, event });
            ContextPrediction { action: ContextAction::Keep, probability: 1.0, probabilities: [1.0, 0.0, 0.0, 0.0], model_version: SPAN_CAPTURE_VERSION.to_owned(), supported: true }
        }));
        let mut replayer = Replayer::new(tools, models, shared, folder, recorder, None, Vec::new());
        replayer.recording = Some(recording);
        replayer
    }

    pub fn take_records(&self) -> Vec<Capture> {
        let recording = self.recording.as_ref().expect("a recording replayer");
        std::mem::take(&mut recording.lock().unwrap_or_else(PoisonError::into_inner).records)
    }

    /// `replay`.
    pub fn replay(&mut self, plan: &Plan, mode: Mode) -> Outcome {
        assert!(plan.unsupported.is_empty(), "unsupported physical sequence cannot be replayed");
        let settings = Arc::new(SettingsStore::new(Some(self.folder.join("settings.json"))));
        // The sequences measure the layout decision; the typo correction is measured apart.
        let mut changes = vec![("detection.correct_typos", json!(false)), ("detection.pause_delay_seconds", json!(SEQUENCE_PROTOCOL_PAUSE_DELAY_SECONDS))];
        match mode {
            Mode::Default => changes.push(("detection.early_switch", json!(true))),
            Mode::EarlyOff => changes.extend([
                ("detection.context_policy", json!("assist")),
                ("detection.context_aware", json!(true)),
                ("detection.context_read_field", json!(false)),
                ("detection.early_switch", json!(false)),
                ("detection.respect_manual_layout", json!(false)),
                ("detection.learning", json!(false)),
                ("general.keep_history", json!(false)),
                ("diagnostics.technical_logging", json!(false)),
            ]),
        }
        for (name, value) in changes {
            settings.set(name, value, false).expect("in memory");
        }
        let backend = Arc::new(Backend::new("TestEditor", 0, Vec::new()));
        let clock = FakeClock::new();
        let mut context_policy = self.context_policy.take().expect("one engine at a time");
        context_policy.stream = InputContext::with_clock(clock.read());
        context_policy.reader = Some(Arc::new(UnreadField));
        if let Some(recording) = &self.recording {
            let mut state = recording.lock().unwrap_or_else(PoisonError::into_inner);
            state.backend = Some(backend.clone());
            state.records.clear();
        }
        let mut core = EngineCore::from_parts(EngineParts {
            settings,
            history: Arc::new(HistoryStore::new(Some(self.folder.join("history.jsonl")), 200)),
            backend: backend.clone(),
            learning: None,
            backend_label: "editor".to_owned(),
            models: self.models.clone(),
            intent_model: self.intent_model.clone(),
            intent_model_status: self.intent_model_status.clone(),
            protected_tokens: self.protected_tokens.clone(),
            context_policy,
            boundary_model: self.boundary_model.clone(),
            prefix_model: self.prefix_model.clone(),
            prefix_indexes: self.prefix_indexes.clone(),
            resource_dir: self.resource_dir.clone(),
            clock: clock.read(),
            platform_reader: None,
        })
        .expect("an engine over the shared models");
        let expected: Vec<char> = plan.expected.chars().collect();
        let mut error = None;
        if mode == Mode::Default && plan.keys.len() != expected.len() {
            error = Some("ValueError: default settings replay requires one key per expected character".to_owned());
        }
        let mut intervention = Value::Null;
        if error.is_none() {
            error = self.type_keys(&mut core, &backend, &clock, plan, &expected, &mut intervention).err();
        }
        self.context_policy = Some(core.context_policy);
        let _ = std::fs::remove_file(self.folder.join("history.jsonl"));
        let actual: String = backend.text().iter().collect();
        let final_group = i64::from(backend.group());
        Outcome {
            mode,
            exact: error.is_none() && actual == plan.expected,
            length_mismatch: actual.chars().count() != expected.len(),
            actual,
            final_group,
            expected_final_group: plan.expected_final_group,
            final_layout_matches: final_group == plan.expected_final_group,
            corrections: backend.corrections(),
            intervention,
            error,
        }
    }

    fn type_keys(&self, core: &mut EngineCore, backend: &Arc<Backend>, clock: &FakeClock, plan: &Plan, expected: &[char], intervention: &mut Value) -> Result<(), String> {
        let timers = |core: &mut EngineCore| -> Result<(), String> {
            // The engine's idle loop runs exactly these callbacks when no key arrives.
            core.expire_deferred_action().map_err(|error| format!("{error:?}"))?;
            core.expire_manual_correction();
            core.poll_current_group().map_err(|error| format!("{error:?}"))?;
            core.maybe_correct_after_pause(None).map_err(|error| format!("{error:?}"))?;
            core.expire_learning_prompt(None);
            Ok(())
        };
        for (index, key) in plan.keys.iter().enumerate() {
            let serial = index + 1;
            backend.set_serial(serial);
            if let Some(recording) = &self.recording {
                recording.lock().unwrap_or_else(PoisonError::into_inner).serial = serial;
            }
            if let Some(group) = key.explicit_group {
                backend.set_group(group);
            }
            let group = backend.group();
            let observed = key.characters[group.to::<usize>()];
            if plan.intervention_event == Some(serial) {
                let target = plan.group.expect("an intervention has a group");
                let intended = key.characters[target.to::<usize>()];
                *intervention = json!({"event": serial, "expected": intended.to_string(), "observed": observed.to_string(), "source_group": group, "target_group": target});
                if observed == intended {
                    return Err("ValueError: wrong intervention did not change the focus letter".to_owned());
                }
            }
            clock.advance(SIMULATED_KEY_DOWN_SECONDS);
            let pressed = KeyEvent {
                pressed: true,
                keycode: key.keycode,
                key_name: if observed == ' ' { "space".to_owned() } else { observed.to_string() },
                character: observed.to_string(),
                characters: key.characters.iter().map(char::to_string).collect(),
                group,
                state: if key.shift { SHIFT_MASK } else { 0 },
                timestamp: (ks_text::py_round(clock.now() * MILLISECONDS_PER_SECOND)).to::<i64>(),
                ..KeyEvent::default()
            };
            backend.type_key(&pressed);
            core.handle_inner(Arc::new(pressed.clone())).map_err(|error| format!("{error:?}"))?;
            clock.advance(SIMULATED_KEY_UP_SECONDS);
            let released = KeyEvent { pressed: false, timestamp: (ks_text::py_round(clock.now() * MILLISECONDS_PER_SECOND)).to::<i64>(), ..pressed };
            backend.type_key(&released);
            core.handle_inner(Arc::new(released)).map_err(|error| format!("{error:?}"))?;
            timers(core)?;
            let space = |value: char| pystr::isspace(value.encode_utf8(&mut [0_u8; 4]));
            // Both modes run the idle callbacks: a document that ends without a boundary key still reaches its decision.
            if !space(expected[index]) && expected.get(index + 1).is_none_or(|value| space(*value)) {
                clock.advance(SIMULATED_WORD_IDLE_SECONDS);
                timers(core)?;
            }
        }
        Ok(())
    }
}
