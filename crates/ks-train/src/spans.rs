//! Bounded non-test frames captured from physical engine boundaries (`context_action_spans`): a wrong focus after a
//! correct anchor of the same split, typed through the real engine, whose every question to the model is recorded.
use crate::corpus::{CorpusRow, TRAIN, DEVELOPMENT, CALIBRATION};
use crate::curricula::Counts;
use crate::physical::{PhysicalKey, hex_digest, key_of, sha256};
use crate::rows::is_alpha;
use crate::tools::{Lexicons, Tools};
use ks_backend::KeyEvent;
use ks_backend::constants::SHIFT_MASK;
use ks_config::config::SettingsStore;
use ks_config::history::HistoryStore;
use ks_context::ContextPolicy;
use ks_context::context_model::{ContextAction, ContextEvidence, ContextModel, ContextPrediction};
use ks_context::flat_table::WeightTable;
use ks_context::input_context::InputContext;
use ks_engine::engine::{EngineCore, EngineParts};
use ks_lang::detector::ProtectedTokens;
use ks_lang::intent_model::{IntentModelStatus, LinearNgramModel};
use ks_replay::harness::{Backend, FakeClock};
use ks_text::pystr;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

const VARIANTS: [(&str, &str); 6] = [("", " "), (".", " "), (",", "  "), (";", "   "), ("...", "  "), ("!", "   ")];
const SPAN_ANCHOR_MAX_CHARACTERS: usize = 24;
const SPAN_ANCHOR_MIN_CHARACTERS: usize = 3;
const SPAN_MAXIMUM_FAMILIES: i64 = 128;
const SPAN_ORIGINAL_MAX_CHARACTERS: usize = 64;
const LAYOUT_GROUP_COUNT: usize = 2;
const SEQUENCE_PROTOCOL_PAUSE_DELAY_SECONDS: f64 = 1.5;
const SIMULATED_KEY_DOWN_SECONDS: f64 = 0.05;
const SIMULATED_KEY_UP_SECONDS: f64 = 0.03;
const SIMULATED_WORD_IDLE_SECONDS: f64 = 1.7;
const MILLISECONDS_PER_SECOND: f64 = 1000.0;

#[derive(Clone, Debug, PartialEq)]
pub struct SpanFrame {
    pub evidence: ContextEvidence,
    pub action: ContextAction,
    pub parent_family: String,
    pub source_identifier: String,
    pub sequence_id: String,
    pub sample_weight: f64,
}

#[derive(Default)]
pub struct SpanCurriculum {
    pub frames: Vec<SpanFrame>,
    pub counts: Option<Counts>,
}

fn rank(purpose: &str, identity: &str) -> [u8; 32] {
    sha256(&format!("context-action-spans-v1:{purpose}:{identity}"))
}

/// `_keys`: the first key that types each character in this layout.
fn keys_of(text: &str, group: i64) -> Option<Vec<PhysicalKey>> {
    text.chars().map(|character| key_of(character, group as usize).cloned()).collect()
}

fn body(text: &str, group: i64) -> Vec<u32> {
    keys_of(text, group).expect("unrepresentable span sequence").iter().map(|key| key.keycode).collect()
}

fn select(rows: &[CorpusRow], maximum: usize) -> (Vec<&CorpusRow>, [Vec<&CorpusRow>; 2]) {
    let mut ordered: Vec<&CorpusRow> = rows.iter().collect();
    ordered.sort_by_cached_key(|row| (rank("source", &row.identifier), row.identifier.clone()));
    let mut families: HashMap<&str, &CorpusRow> = HashMap::new();
    let mut anchors: [Vec<&CorpusRow>; 2] = [Vec::new(), Vec::new()];
    for row in ordered {
        let Some(group) = row.group.filter(|group| *group == 0 || *group == 1) else { continue };
        let length = pystr::len(&row.original);
        if !row.layout_representable || !(1..=SPAN_ORIGINAL_MAX_CHARACTERS).contains(&length) || !row.original.chars().any(is_alpha) || keys_of(&row.original, group).is_none() {
            continue;
        }
        families.entry(&row.family).or_insert(row);
        if pystr::isalpha(&row.original) && (SPAN_ANCHOR_MIN_CHARACTERS..=SPAN_ANCHOR_MAX_CHARACTERS).contains(&length) {
            anchors[group as usize].push(row);
        }
    }
    let mut ranked: Vec<&CorpusRow> = families.into_values().collect();
    ranked.sort_by_cached_key(|row| (rank("family", &row.family), row.family.clone()));
    let mut selected = Vec::new();
    let mut per_group = [0_usize; 2];
    for row in ranked {
        let group = row.group.expect("a selected row has a group") as usize;
        if anchors[group].is_empty() || per_group[group] >= maximum / LAYOUT_GROUP_COUNT {
            continue;
        }
        per_group[group] += 1;
        selected.push(row);
    }
    (selected, anchors)
}

struct Capture {
    evidence: ContextEvidence,
    observed: String,
    event: usize,
}

#[derive(Default)]
struct Recording {
    backend: Option<Arc<Backend>>,
    serial: usize,
    records: Vec<Capture>,
}

/// One engine after another over the shared models, every question recorded and answered `keep`.
pub struct Replayer {
    tools_root: PathBuf,
    models: Lexicons,
    intent_model: Option<Arc<LinearNgramModel>>,
    intent_model_status: IntentModelStatus,
    protected_tokens: Arc<ProtectedTokens>,
    boundary_model: Option<Arc<ks_lang::boundary_policy::BoundaryPolicy>>,
    prefix_indexes: Vec<Arc<ks_lang::early_switch::PrefixIndex>>,
    context_policy: Option<ContextPolicy>,
    recording: Arc<Mutex<Recording>>,
    folder: PathBuf,
}

impl Replayer {
    /// `shared` is the orthotactic model every replay of a fit binds to the first profile's dictionaries.
    pub fn new(tools: &Tools, models: &Lexicons, shared: ks_context::context_policy::SharedOrtho, folder: PathBuf) -> Replayer {
        let resources = &tools.resources;
        let (intent_model, intent_model_status) = resources.intent_model();
        let recording = Arc::new(Mutex::new(Recording::default()));
        let mut context_policy = ContextPolicy::with_ortho(resources, None, shared, None);
        let mut recorder = ContextModel::new(WeightTable::empty(), "context-v3-span-capture", 0.99, tools.tables.clone());
        let handle = recording.clone();
        recorder.answer = Some(Arc::new(move |item: &ContextEvidence| {
            let mut state = handle.lock().unwrap_or_else(PoisonError::into_inner);
            let observed: String = state.backend.as_ref().expect("a replay is running").text().iter().collect();
            let event = state.serial;
            state.records.push(Capture { evidence: item.clone(), observed, event });
            ContextPrediction { action: ContextAction::Keep, probability: 1.0, probabilities: [1.0, 0.0, 0.0, 0.0], model_version: "context-v3-span-capture".to_owned(), supported: true }
        }));
        context_policy.model = Some(recorder);
        std::fs::create_dir_all(&folder).expect("a folder for the stores");
        Replayer {
            tools_root: resources.resource_dir().to_path_buf(),
            models: models.clone(),
            intent_model,
            intent_model_status,
            protected_tokens: resources.protected_tokens(),
            boundary_model: resources.boundary_policy(),
            prefix_indexes: Vec::new(),
            context_policy: Some(context_policy),
            recording,
            folder,
        }
    }

    /// `_capture`: what the model was asked while the keys of one sequence were typed.
    fn capture(&mut self, keys: &[PhysicalKey], expected: &[char]) -> Vec<Capture> {
        let settings = Arc::new(SettingsStore::new(Some(self.folder.join("settings.json"))));
        for (name, value) in [
            ("detection.correct_typos", json!(false)),
            ("detection.pause_delay_seconds", json!(SEQUENCE_PROTOCOL_PAUSE_DELAY_SECONDS)),
            ("detection.context_policy", json!("assist")),
            ("detection.context_aware", json!(true)),
            ("detection.context_read_field", json!(false)),
            ("detection.early_switch", json!(false)),
            ("detection.respect_manual_layout", json!(false)),
            ("detection.learning", json!(false)),
            ("general.keep_history", json!(false)),
            ("diagnostics.technical_logging", json!(false)),
        ] {
            settings.set(name, value, false).expect("in memory");
        }
        let backend = Arc::new(Backend::new("TestEditor", 0, Vec::new()));
        let clock = FakeClock::new();
        let mut context_policy = self.context_policy.take().expect("one engine at a time");
        context_policy.stream = InputContext::with_clock(clock.read());
        {
            let mut state = self.recording.lock().unwrap_or_else(PoisonError::into_inner);
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
            prefix_model: None,
            prefix_indexes: self.prefix_indexes.clone(),
            resource_dir: self.tools_root.clone(),
            clock: clock.read(),
            platform_reader: None,
        })
        .expect("an engine over the shared models");
        let timers = |core: &mut EngineCore| {
            core._expire_deferred_action().expect("a timer of the replay");
            core._expire_manual_correction();
            core._poll_current_group().expect("a timer of the replay");
            core._maybe_correct_after_pause(None).expect("a timer of the replay");
            core._expire_learning_prompt(None);
        };
        for (index, key) in keys.iter().enumerate() {
            let serial = index + 1;
            self.recording.lock().unwrap_or_else(PoisonError::into_inner).serial = serial;
            if let Some(group) = key.explicit_group {
                backend.set_group(group);
            }
            let group = backend.group();
            let observed = key.characters[group as usize];
            clock.advance(SIMULATED_KEY_DOWN_SECONDS);
            let pressed = KeyEvent {
                pressed: true,
                keycode: key.keycode,
                key_name: if observed == ' ' { "space".to_owned() } else { observed.to_string() },
                character: observed.to_string(),
                characters: key.characters.iter().map(char::to_string).collect(),
                group,
                state: if key.shift { SHIFT_MASK } else { 0 },
                timestamp: ks_text::py_round(clock.now() * MILLISECONDS_PER_SECOND) as i64,
                ..KeyEvent::default()
            };
            backend.type_key(&pressed);
            core._handle(Arc::new(pressed.clone())).expect("a key the engine handles");
            clock.advance(SIMULATED_KEY_UP_SECONDS);
            let released = KeyEvent { pressed: false, timestamp: ks_text::py_round(clock.now() * MILLISECONDS_PER_SECOND) as i64, ..pressed };
            backend.type_key(&released);
            core._handle(Arc::new(released)).expect("a key the engine handles");
            timers(&mut core);
            let character = expected[index];
            let following = expected.get(index + 1);
            let space = |value: char| pystr::isspace(value.encode_utf8(&mut [0_u8; 4]));
            if !space(character) && following.is_none_or(|value| space(*value)) {
                clock.advance(SIMULATED_WORD_IDLE_SECONDS);
                timers(&mut core);
            }
        }
        assert_eq!(backend.corrections(), 0, "span capture must complete without corrections");
        self.context_policy = Some(core.context_policy);
        let _ = std::fs::remove_file(self.folder.join("history.jsonl"));
        std::mem::take(&mut self.recording.lock().unwrap_or_else(PoisonError::into_inner).records)
    }
}

/// `_label`: what a captured question's answer is, given the text that should stand by then.
fn label(capture: &Capture, expected: &str) -> Option<ContextAction> {
    if capture.observed == expected {
        return Some(ContextAction::Keep);
    }
    let item = &capture.evidence;
    let suffix = format!("{}{}", item.literal_tail, item.boundary_text);
    let anchor = format!("{}{suffix}", item.original);
    if anchor.is_empty() || !capture.observed.ends_with(&anchor) {
        return None;
    }
    let converted = format!("{}{}{suffix}", &capture.observed[..capture.observed.len() - anchor.len()], item.alternative);
    (converted == expected).then_some(ContextAction::Convert)
}

/// `build_span_curriculum`: attainable labels captured without dictionary or candidate predictions.
pub fn build_span_curriculum(replayer: &mut Replayer, rows: &[CorpusRow], maximum_families: i64, expected_split: &str) -> SpanCurriculum {
    assert!(
        (LAYOUT_GROUP_COUNT as i64..=SPAN_MAXIMUM_FAMILIES).contains(&maximum_families) && maximum_families % LAYOUT_GROUP_COUNT as i64 == 0,
        "span family budget"
    );
    assert!([TRAIN, DEVELOPMENT, CALIBRATION].contains(&expected_split), "span curriculum never accepts the test split");
    assert!(rows.iter().all(|row| row.split == expected_split && row.quarantine_reasons.is_empty()), "span curriculum accepts only rows from the declared split");
    let (selected, anchors) = select(rows, maximum_families as usize);
    let mut counts = Counts::default();
    counts.add_by("input_rows", rows.len() as i64);
    counts.add_by("selected_families", selected.len() as i64);
    let mut frames: Vec<SpanFrame> = Vec::new();
    for row in selected {
        let group = row.group.expect("a selected row has a group");
        counts.add(format!("selected_group_{group}"));
        let purpose = format!("anchor:{}", row.family);
        let anchor = anchors[group as usize].iter().min_by_key(|other| (rank(&purpose, &other.identifier), other.identifier.clone())).expect("an anchor");
        let parent_body = body(&row.original, group);
        for (variant, (tail, spacing)) in VARIANTS.into_iter().enumerate() {
            let before = format!("{}{spacing}", anchor.original);
            for wrong in [false, true] {
                let source = if wrong { 1 - group } else { group };
                let prefix_keys = keys_of(&before, group).expect("unrepresentable span sequence");
                let focus_keys = keys_of(&row.original, group).expect("unrepresentable span sequence");
                let tail_keys = keys_of(&format!("{tail} "), source).expect("unrepresentable span sequence");
                let prefix_length = prefix_keys.len();
                let mut keys = prefix_keys;
                keys.extend(focus_keys);
                keys.extend(tail_keys);
                keys[0].explicit_group = Some(group as i32);
                keys[prefix_length].explicit_group = Some(source as i32);
                let expected = format!("{before}{}{tail} ", row.original);
                let expected_characters: Vec<char> = expected.chars().collect();
                let identity = hex_digest(&format!("{}:{variant}:{}", row.identifier, i32::from(wrong)));
                let records = replayer.capture(&keys, &expected_characters);
                counts.add("sequences");
                let focus: Vec<Capture> = records.into_iter().filter(|record| record.event > prefix_length).collect();
                counts.add_by("no_focus_call", i64::from(focus.is_empty()));
                for record in focus {
                    counts.add("focus_calls");
                    let item = &record.evidence;
                    if body(&item.original, item.source_group) != parent_body {
                        counts.add("skipped_nonparent_body");
                        continue;
                    }
                    let so_far: String = expected_characters[..record.event].iter().collect();
                    let Some(action) = label(&record, &so_far) else {
                        counts.add("skipped_inexpressible");
                        continue;
                    };
                    counts.add(format!("action_{}", action.as_str()));
                    counts.add_by("tail_nonempty", i64::from(!item.literal_tail.is_empty()));
                    counts.add_by("repeated_space_before", i64::from(item.field.before.ends_with("  ")));
                    frames.push(SpanFrame { evidence: record.evidence, action, parent_family: row.family.clone(), source_identifier: row.identifier.clone(), sequence_id: identity.clone(), sample_weight: 0.0 });
                }
            }
        }
    }
    let mut family_counts: BTreeMap<String, u64> = BTreeMap::new();
    for frame in &frames {
        *family_counts.entry(frame.parent_family.clone()).or_insert(0) += 1;
    }
    for frame in &mut frames {
        frame.sample_weight = 1.0 / family_counts[&frame.parent_family] as f64;
    }
    counts.add_by("retained_families", family_counts.len() as i64);
    counts.add_by("frames", frames.len() as i64);
    SpanCurriculum { frames, counts: Some(counts) }
}

impl SpanCurriculum {
    pub fn report(&self) -> Value {
        self.counts.as_ref().map_or(json!({}), Counts::report)
    }
}
