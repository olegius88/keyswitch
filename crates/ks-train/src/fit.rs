//! A fit of the context action model (`train_context_action_model.fit`): the curricula, the features, the epochs, the
//! threshold and the seal. Every value that reaches the candidate is the one the Python trainer computed, to the bit.
use crate::captured::{CapturedSource, captured_source_curriculum, captured_sources};
use crate::corpus::{CALIBRATION, CorpusRow, DEVELOPMENT, FITTING_SPLITS, TRAIN, checksum, load_split, read_json};
use crate::curricula::{self, Curriculum, Decide};
use crate::evidence::{evidence, frame_evidence, sorted_features};
use crate::heads::{HEAD_PREFIXES, OWN_EPOCH_HEADS, head_question, own_curriculum};
use crate::historical::historical_curriculum;
use crate::kernel::{self, Packed};
use crate::lookahead;
use crate::physical::{expanded_aliases, hex, reading, sha256};
use crate::rows::{ActionRow, action_rows, select_rows, training_order};
use crate::selection::{EpochSelection, Qualifying, ThresholdOptions, assess_epoch, choose_threshold, metrics, select_features, settled_epoch};
use crate::replay::Replayer;
use crate::spans::{SpanCurriculum, SpanFrame, build_span_curriculum};
use crate::tools::{Lexicons, Options, Tools, reference_models};
use crate::{ACTIONS, DETERMINISTIC_ROUNDING_DECIMALS, LOG_LOSS_PROBABILITY_FLOOR};
use ks_context::constants::{ALONE_FEATURE_PREFIX, KEPT_FEATURE_PREFIX};
use ks_context::context_action_features::{FeatureMap, extract_action_features};
use ks_context::context_model::{ACTIONS as ACTION_NAMES, ContextAction, ContextModel};
use ks_context::flat_table::WeightTable;
use ks_lang::Resources;
use ks_lang::detector::{IntentClassifier, LanguageDetector, LanguageScorer};
use ks_lang::intent_model::LinearNgramModel;
use ks_lang::ortho_model::{OrthoModel, artifact_path};
use ks_text::{Indent, JsonOptions, json_dumps, py_round_ndigits, pystr};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const FRAMES_PER_TASK: usize = 256;
const CONTEXT_ACTION_FEATURE_VERSION: i64 = 3;
const VERSION_HASH_CHARACTERS: usize = 12;
const REPORT_JSON_INDENT: usize = 2;
pub const ARTIFACT: &str = "context-action.json";
pub const SEAL: &str = "candidate-seal.json";
const SEALED_BEFORE_TEST: &str = "sealed-before-test";
const REJECTED_BEFORE_TEST: &str = "rejected-before-test";

/// A frame of one profile and split: a frame of a curriculum, or one captured from the engine.
#[derive(Clone, Debug, PartialEq)]
pub enum Frame {
    Row(ActionRow),
    Span(SpanFrame),
}

impl Frame {
    fn action(&self) -> ContextAction {
        match self {
            Frame::Row(row) => row.action,
            Frame::Span(span) => span.action,
        }
    }

    fn sample_weight(&self) -> f64 {
        match self {
            Frame::Row(row) => row.sample_weight,
            Frame::Span(span) => span.sample_weight,
        }
    }
}

/// `json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + "\n"`.
pub fn readable(value: &Value) -> Vec<u8> {
    let options = JsonOptions { ensure_ascii: false, sort_keys: true, indent: Some(Indent::Spaces(REPORT_JSON_INDENT)), ..JsonOptions::default() };
    (json_dumps(value, &options) + "\n").into_bytes()
}

/// What every stage of a fit reads: the recipe, the corpus splits, the natural frames and the models.
pub struct FitInputs {
    pub options: Value,
    pub source_rows: HashMap<&'static str, Vec<CorpusRow>>,
    pub frames: HashMap<&'static str, Vec<ActionRow>>,
    pub tools: Tools,
    pub ortho: OrthoModel,
    pub lexicons_spelling: Lexicons,
    pub profiles: Vec<String>,
    pub base: Option<ContextModel>,
    detectors: HashMap<String, LanguageDetector>,
    models_dir: PathBuf,
}

/// The refused aliases: this corpus's test and every accessed test (`refused_aliases`). The ledger of accessed tests
/// lives outside the repository; a fit that would refuse fewer than the corpus was frozen against stops.
fn refused_aliases(corpus: &Path, ledger: &Path) -> Result<BTreeSet<String>, String> {
    let membership = read_json(&corpus.join("test-membership.json"))?;
    let mut refused: BTreeSet<String> = membership.get("alias_sha256").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect();
    if ledger.is_dir() {
        let mut paths: Vec<PathBuf> = std::fs::read_dir(ledger).map_err(|error| error.to_string())?.filter_map(|entry| entry.ok().map(|entry| entry.path())).collect();
        paths.retain(|path| path.to_string_lossy().ends_with(".access.json"));
        paths.sort();
        for path in paths {
            let record = read_json(&path)?;
            if let Some(aliases) = record.get("test_membership").and_then(|value| value.get("alias_sha256")).and_then(Value::as_array) {
                refused.extend(aliases.iter().filter_map(Value::as_str).map(str::to_owned));
            }
        }
    }
    let manifest = read_json(&corpus.join("manifest.json"))?;
    if let Some(frozen_against) = manifest.get("metadata").and_then(|value| value.get("refused_test_aliases")).and_then(Value::as_u64) {
        if (refused.len() as u64) < frozen_against {
            return Err(format!("the fit would refuse {} test aliases and the corpus was frozen refusing {frozen_against}: the test ledger ({}) is missing or older than the corpus", refused.len(), ledger.display()));
        }
    }
    Ok(refused)
}

fn pinned_model(root: &Path, models_dir: &Path, spec: &Value, label: &str) -> Result<ContextModel, String> {
    let path = root.join(spec["artifact"].as_str().ok_or("a model without an artifact")?);
    if Value::String(checksum(&path)?) != spec["sha256"] {
        return Err(format!("{label} artifact differs from the recipe"));
    }
    let model = ContextModel::load_artifact(&path, models_dir).map_err(|error| format!("{}: {}", path.display(), error.name))?;
    if model.feature_version != CONTEXT_ACTION_FEATURE_VERSION {
        return Err(format!("a {label} is an action model"));
    }
    Ok(model)
}

fn carried_prefixes(model: &ContextModel) -> BTreeSet<String> {
    let mut result = BTreeSet::new();
    for prefix in std::iter::once(KEPT_FEATURE_PREFIX).chain(HEAD_PREFIXES.iter().map(|head| head.1)) {
        if model.weights.names().any(|name| name.starts_with(prefix)) {
            result.insert(prefix.to_owned());
        }
    }
    result
}

fn head_prefix(name: &str) -> Result<&'static str, String> {
    HEAD_PREFIXES.iter().find(|head| head.0 == name).map(|head| head.1).ok_or_else(|| format!("no head {name}"))
}

/// `frozen_heads`: the heads fitted onto a frozen base: feature prefix -> feature budget.
fn frozen_heads(options: &Value) -> Result<BTreeMap<String, usize>, String> {
    let mut heads = BTreeMap::new();
    if let Some(table) = options.get("frozen_base").filter(|value| !value.is_null()).and_then(|value| value.get("heads")).and_then(Value::as_object) {
        for (name, head) in table {
            heads.insert(head_prefix(name)?.to_owned(), head.int("maximum_features") as usize);
        }
    }
    Ok(heads)
}

fn named_base(root: &Path, models_dir: &Path, options: &Value, key: &str) -> Result<Option<ContextModel>, String> {
    let Some(spec) = options.get(key).filter(|value| !value.is_null()) else { return Ok(None) };
    let label = key.replace('_', " ");
    let model = pinned_model(root, models_dir, spec, &label)?;
    let carried = carried_prefixes(&model);
    if key == "warm_base" && !carried.is_empty() {
        return Err(format!("a {label} is an action model without kept-neighbour weights or heads"));
    }
    if key == "frozen_base" {
        if frozen_heads(options)?.keys().any(|prefix| carried.contains(prefix)) {
            return Err("a head the frozen base carries is frozen with it, not fitted again".to_owned());
        }
        for name in spec.get("carried").and_then(Value::as_object).into_iter().flatten().map(|entry| entry.0) {
            if !carried.contains(head_prefix(name)?) {
                return Err("the recipe names a carried head the frozen base has no weights for".to_owned());
            }
        }
    }
    Ok(Some(model))
}

/// `alone_evidence`: the counts that label lone two-letter frames for the lone-word head, or None without that head.
fn alone_evidence(options: &Value) -> Option<Value> {
    let base = options.get("frozen_base").filter(|value| !value.is_null())?;
    base.get("heads").and_then(|heads| heads.get("alone")).or_else(|| base.get("carried").and_then(|carried| carried.get("alone"))).cloned()
}

/// `load_inputs`.
pub fn load_inputs(root: &Path, resource_dir: &Path, corpus: &Path, options: Value, ledger: &Path) -> Result<FitInputs, String> {
    let data_dir = ks_config::history::data_dir();
    let resources: Arc<Resources> = Resources::with_speller(resource_dir.to_path_buf(), ks_hunspell::hunspell_speller(data_dir));
    let models_dir = resource_dir.join("models");
    let portable = reference_models(root, &resources, false)?;
    let refused = refused_aliases(corpus, ledger)?;
    let tools = Tools::new(root, resources.clone(), portable.clone(), refused)?;
    let lexicons_spelling = reference_models(root, &resources, true)?;
    let mut source_rows = HashMap::new();
    let mut frames = HashMap::new();
    for split in FITTING_SPLITS {
        let rows = load_split(corpus, split)?;
        let chosen = select_rows(&rows, options["maximum_source_rows"].int(split) as usize);
        let natural = action_rows(&tools, &chosen);
        if natural.is_empty() {
            return Err("empty fitting split".to_owned());
        }
        frames.insert(split, natural);
        source_rows.insert(split, rows);
    }
    let (intent, _) = resources.intent_model();
    let intent: Option<Arc<dyn IntentClassifier>> = intent.map(|model: Arc<LinearNgramModel>| model as _);
    let profiles: Vec<String> = options["profiles"].as_array().ok_or("profiles")?.iter().filter_map(Value::as_str).map(str::to_owned).collect();
    let mut detectors = HashMap::new();
    for profile in &profiles {
        let models = if profile == "reference_hunspell" { &lexicons_spelling } else { &portable };
        let scorers: Vec<(i64, Arc<dyn LanguageScorer>)> = models.iter().enumerate().map(|(group, model)| (group as i64, model.clone() as Arc<dyn LanguageScorer>)).collect();
        detectors.insert(profile.clone(), LanguageDetector::new(scorers, intent.clone(), resources.protected_tokens()).map_err(|error| error.name.to_string())?);
    }
    let ortho = OrthoModel::load(&artifact_path(resource_dir), None, None, Some(resources)).map_err(|error| error.name.to_string())?;
    let base = if frozen_heads(&options)?.is_empty() { named_base(root, &models_dir, &options, "frozen_base")? } else { None };
    Ok(FitInputs { options, source_rows, frames, tools, ortho, lexicons_spelling, profiles, base, detectors, models_dir })
}

impl FitInputs {
    fn detector(&self, profile: &str) -> &LanguageDetector {
        &self.detectors[profile]
    }

    fn lexical_models(&self, profile: &str) -> &Lexicons {
        if profile == "reference_hunspell" { &self.lexicons_spelling } else { &self.tools.portable }
    }

    fn recipe_part(&self, key: &str) -> Option<&Value> {
        self.options.get(key)
    }

    /// `base_action`: the labelling base's own action on a frame, or None when the profiles' answers differ.
    fn base_action(&self, labelling: &ContextModel, row: &ActionRow) -> Option<ContextAction> {
        let mut answers = self.profiles.iter().map(|profile| labelling.predict(&evidence(&self.tools, row, self.detector(profile), Some(&self.ortho), None)).action);
        let first = answers.next()?;
        answers.all(|action| action == first).then_some(first)
    }
}

fn curriculum_or_absent(part: Option<&Value>, build: impl FnOnce(&Value) -> Curriculum) -> Curriculum {
    part.map_or_else(|| (Vec::new(), json!({"frames": 0, "scope": "not used"})), build)
}

/// `tail_curricula`: the TRAIN frames that join after the lookahead curricula, with their reports by curriculum.
fn tail_curricula(inputs: &FitInputs) -> Result<Curriculum, String> {
    let tools = &inputs.tools;
    let rows = &inputs.source_rows[TRAIN];
    let portable = &tools.portable;
    let mut frames: Vec<ActionRow> = Vec::new();
    let mut reports = Map::new();
    let mut add = |key: &str, (mut new, report): Curriculum| {
        frames.append(&mut new);
        reports.insert(key.to_owned(), report);
    };
    add("kept_neighbour_curriculum", curricula::kept_neighbour_curriculum(tools, rows, &inputs.options["kept_neighbour_curriculum"], portable));
    add("counted_token_curriculum", curricula::counted_token_curriculum(tools, rows, &inputs.options["counted_token_curriculum"]));
    add("letter_curriculum", curriculum_or_absent(inputs.recipe_part("letter_curriculum"), |options| curricula::letter_curriculum(tools, rows, options)));
    add("abbreviation_curriculum", curriculum_or_absent(inputs.recipe_part("abbreviation_curriculum"), |options| curricula::abbreviation_curriculum(tools, TRAIN, rows, options)));
    add("latin_abbreviation_curriculum", curriculum_or_absent(inputs.recipe_part("latin_abbreviation_curriculum"), |options| curricula::latin_abbreviation_curriculum(tools, TRAIN, rows, options)));
    add("unknown_token_curriculum", curriculum_or_absent(inputs.recipe_part("unknown_token_curriculum"), |options| curricula::unknown_token_curriculum(tools, TRAIN, rows, options)));
    for (key, label) in [
        ("joined_token_curriculum", "joined"), ("term_curriculum", "term"), ("brief_curriculum", "brief"), ("first_curriculum", "first"), ("variable_curriculum", "variable"),
        ("dotted_curriculum", "dotted"), ("alnum_curriculum", "alnum"), ("hypothesis_curriculum", "hypothesis"), ("short_curriculum", "short"), ("slip_curriculum", "slip"),
        ("sign_curriculum", "sign"), ("resume_curriculum", "resume"), ("cyrletter_curriculum", "cyrletter"), ("dash_curriculum", "dash"), ("digitdot_curriculum", "digitdot"),
        ("quote_curriculum", "quote"),
    ] {
        add(key, own_curriculum(tools, label, TRAIN, rows, &inputs.options)?);
    }
    Ok((frames, Value::Object(reports)))
}

/// `frame_chain`: the curricula one profile adds to the frames of one split, and their reports.
fn frame_chain(inputs: &FitInputs, labelling: Option<&ContextModel>, profile: &str, split: &'static str, rows: Vec<ActionRow>, tail: &[ActionRow]) -> Result<Curriculum, String> {
    if split == TRAIN && inputs.base.is_some() {
        // A frozen base keeps every weight of the word decided at its own boundary: nothing else is framed for TRAIN.
        return Ok((tail.to_vec(), json!({})));
    }
    let options = &inputs.options;
    let tools = &inputs.tools;
    let detector = inputs.detector(profile);
    let ortho = Some(&inputs.ortho);
    let source_rows = &inputs.source_rows[split];
    let mut reports = Map::new();
    let mut result = rows;
    if split == TRAIN {
        let budget = &options["lookahead_curriculum"];
        let (frames, report) = lookahead::legacy_lookahead_rows(tools, result, detector, ortho, budget.int("maximum_families"), budget.int("seeds_per_family"));
        result = frames;
        reports.insert("lookahead".to_owned(), report);
    }
    let budget = &options["natural_lookahead_curriculum"];
    let (frames, report) = lookahead::natural_lookahead_rows(tools, result, source_rows, detector, ortho, budget.int("maximum_families"), budget.int("seeds_per_family"));
    result = frames;
    reports.insert("natural".to_owned(), report);
    if split == TRAIN {
        let budget = &options["lexical_short_pair_curriculum"];
        let (frames, report) = lookahead::lexical_short_pair_rows(tools, result, source_rows, detector, ortho, budget.int("maximum_families"), budget.int("seeds_per_family"), &tools.portable)?;
        result = frames;
        reports.insert("lexical".to_owned(), report);
        let (frames, report) = lookahead::balance_planned_mass(tools, result);
        result = frames;
        reports.insert("balance".to_owned(), report);
        result.extend_from_slice(tail);
        if options.get("first_curriculum").and_then(|value| value.get("enabled")).and_then(Value::as_bool) == Some(true) {
            // The first-term head is fitted on its own curriculum alone.
            let before = result.len();
            result.retain(|row| !(!row.identifier.starts_with("first:") && row.field.role != ks_backend::FieldRole::Unknown && ks_context::context_action_features::first_question(&row.original, &row.field.before, &row.field.after, &tools.tables)));
            reports.insert("first_class_elsewhere".to_owned(), json!(before - result.len()));
        }
    }
    if split == DEVELOPMENT {
        // The heads' epoch is chosen on the same curricula drawn from DEVELOPMENT's own rows.
        let (mut new, report) = curriculum_or_absent(inputs.recipe_part("letter_curriculum"), |part| curricula::letter_curriculum(tools, source_rows, part));
        result.append(&mut new);
        reports.insert("letter".to_owned(), report);
        let (mut new, report) = curriculum_or_absent(inputs.recipe_part("abbreviation_curriculum"), |part| curricula::abbreviation_curriculum(tools, split, source_rows, part));
        result.append(&mut new);
        reports.insert("abbreviation".to_owned(), report);
        let (mut new, report) = curriculum_or_absent(inputs.recipe_part("latin_abbreviation_curriculum"), |part| curricula::latin_abbreviation_curriculum(tools, split, source_rows, part));
        result.append(&mut new);
        reports.insert("latin_abbreviation".to_owned(), report);
        let (mut new, report) = curriculum_or_absent(inputs.recipe_part("unknown_token_curriculum"), |part| curricula::unknown_token_curriculum(tools, split, source_rows, part));
        result.append(&mut new);
        reports.insert("unknown_token".to_owned(), report);
    }
    if let Some(alone) = alone_evidence(options) {
        let labelling = labelling.ok_or("the lone-word head needs its labelling base")?;
        let decide = |row: &ActionRow| inputs.base_action(labelling, row);
        let decide: Decide<'_> = &decide;
        let (lone, report) = curricula::lone_word_curriculum(tools, split, &alone, Some(decide));
        reports.insert("lone_word".to_owned(), report);
        let mut labelled = curricula::alone_labels(tools, result, Some(&alone), Some(decide));
        labelled.extend(lone);
        result = labelled;
    }
    Ok((result, Value::Object(reports)))
}

/// `prepared_frames`: the frames of one profile and split in the order the optimizer reads them.
fn prepared_frames(rows: Vec<ActionRow>, spans: Vec<SpanFrame>, split: &str) -> Vec<Frame> {
    let mut prepared: Vec<(String, Frame)> = rows.into_iter().map(|row| (row.identifier.clone(), Frame::Row(row))).collect();
    prepared.extend(spans.into_iter().enumerate().map(|(index, span)| (format!("span:{}:{index}", span.sequence_id), Frame::Span(span))));
    if split == TRAIN {
        prepared.sort_by_cached_key(|entry| (sha256(&entry.0), entry.0.clone()));
    }
    prepared.into_iter().map(|entry| entry.1).collect()
}

fn head_frame(tools: &Tools, frame: &Frame, prefixes: &BTreeSet<String>) -> bool {
    match frame {
        Frame::Row(row) => head_question(tools, &row.original, &reading(&row.original, row.group), &row.field.before, &row.field.after, row.trigger, row.after_origin, prefixes, row.before_origin),
        Frame::Span(span) => {
            let item = &span.evidence;
            head_question(tools, &item.original, &item.alternative, &item.field.before, &item.field.after, &item.trigger, item.after_origin, prefixes, item.before_origin)
        }
    }
}

/// The featurised frames of one profile and split, in frame order.
#[derive(Default)]
pub struct Columns {
    pub ids: Vec<u32>,
    pub values: Vec<f64>,
    pub counts: Vec<u32>,
    pub labels: Vec<u8>,
    pub importance: Vec<f64>,
    pub weights: Vec<f64>,
}

impl Columns {
    pub fn feature_rows<'a>(&'a self, names: &'a [String]) -> impl Iterator<Item = (FeatureMap, u8, f64)> + 'a {
        let mut position = 0_usize;
        self.counts.iter().zip(self.labels.iter()).zip(self.importance.iter()).map(move |((count, label), importance)| {
            let end = position + *count as usize;
            let features: FeatureMap = (position..end).map(|entry| (names[self.ids[entry] as usize].clone(), self.values[entry])).collect();
            position = end;
            (features, *label, *importance)
        })
    }
}

/// The global feature ids, in the order the names arrive.
#[derive(Default)]
struct Vocabulary {
    names: Vec<String>,
    index: HashMap<String, u32>,
}

impl Vocabulary {
    fn id(&mut self, name: &str) -> u32 {
        if let Some(found) = self.index.get(name) {
            return *found;
        }
        let id = self.names.len() as u32;
        self.names.push(name.to_owned());
        self.index.insert(name.to_owned(), id);
        id
    }
}

struct Chunk {
    names: Vec<String>,
    ids: Vec<u32>,
    values: Vec<f64>,
    counts: Vec<u32>,
}

fn featurize(inputs: &FitInputs, profile: &str, split: &str, frames: &[Frame]) -> Chunk {
    let detector = inputs.detector(profile);
    let mut local: HashMap<String, u32> = HashMap::new();
    let mut chunk = Chunk { names: Vec::new(), ids: Vec::new(), values: Vec::new(), counts: Vec::with_capacity(frames.len()) };
    for frame in frames {
        let features = match frame {
            Frame::Row(row) => sorted_features(&inputs.tools, &frame_evidence(&inputs.tools, row, split, detector, Some(&inputs.ortho))),
            Frame::Span(span) => sorted_features(&inputs.tools, &span.evidence),
        };
        chunk.counts.push(features.len() as u32);
        for (name, value) in features {
            let next = chunk.names.len() as u32;
            let id = *local.entry(name.clone()).or_insert_with(|| {
                chunk.names.push(name);
                next
            });
            chunk.ids.push(id);
            chunk.values.push(value);
        }
    }
    chunk
}

/// The features of frames, in their order, by `jobs` threads.
fn columns_of(inputs: &FitInputs, vocabulary: &mut Vocabulary, profile: &str, split: &str, frames: &[Frame], keep_importance: f64, jobs: usize) -> Columns {
    let started = std::time::Instant::now();
    let batches: Vec<&[Frame]> = frames.chunks(FRAMES_PER_TASK).collect();
    let next = AtomicUsize::new(0);
    let done: Mutex<Vec<Option<Chunk>>> = Mutex::new((0..batches.len()).map(|_| None).collect());
    std::thread::scope(|scope| {
        for _ in 0..jobs.max(1) {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(batch) = batches.get(index) else { break };
                    let chunk = featurize(inputs, profile, split, batch);
                    done.lock().expect("a worker finished")[index] = Some(chunk);
                }
            });
        }
    });
    eprintln!("{profile}/{split}: features of {} frames in {:.1} s on {} threads", frames.len(), started.elapsed().as_secs_f64(), jobs.max(1));
    let mut columns = Columns::default();
    for chunk in done.into_inner().expect("the workers finished").into_iter().flatten() {
        let mapping: Vec<u32> = chunk.names.iter().map(|name| vocabulary.id(name)).collect();
        columns.ids.extend(chunk.ids.iter().map(|id| mapping[*id as usize]));
        columns.values.extend_from_slice(&chunk.values);
        columns.counts.extend_from_slice(&chunk.counts);
    }
    for frame in frames {
        let label = ACTION_NAMES.iter().position(|action| *action == frame.action()).expect("an action") as u8;
        let weight = frame.sample_weight();
        columns.labels.push(label);
        columns.weights.push(weight);
        columns.importance.push(weight * if label == 0 { keep_importance } else { 1.0 });
    }
    assert!(columns.importance.iter().all(|value| value.is_finite() && *value > 0.0), "invalid training target or importance");
    assert!(columns.values.iter().all(|value| value.is_finite()), "non-finite feature");
    columns
}

/// Every featurised frame of a fit, and what the seal reports about the curricula.
pub struct FeatureSet {
    names: Vec<String>,
    columns: HashMap<(String, &'static str), Columns>,
    chains: HashMap<(String, &'static str), Value>,
    spans: HashMap<(String, &'static str), Value>,
    captured: Value,
    capitals: Value,
    english: Value,
    tail: Value,
}

/// A hook that sees each stage of a build: its name and the frames or features of that stage.
pub trait Stages {
    fn rows(&mut self, _name: &str, _rows: &[ActionRow]) {}
    fn frames(&mut self, _name: &str, _frames: &[Frame]) {}
    fn features(&mut self, _name: &str, _columns: &Columns, _names: &[String]) {}
}

pub struct NoStages;
impl Stages for NoStages {}

fn captured_all(inputs: &FitInputs, sources: &[CapturedSource], options: &Value) -> Result<Vec<Curriculum>, String> {
    let next = AtomicUsize::new(0);
    let done: Mutex<Vec<Option<Result<Curriculum, String>>>> = Mutex::new((0..sources.len()).map(|_| None).collect());
    std::thread::scope(|scope| {
        for _ in 0..sources.len().min(4) {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(source) = sources.get(index) else { break };
                    let result = captured_source_curriculum(&inputs.tools, source, options);
                    done.lock().expect("a worker finished")[index] = Some(result);
                }
            });
        }
    });
    done.into_inner().expect("the workers finished").into_iter().map(|entry| entry.expect("every source was read")).collect()
}

/// `Build.run`: curricula and features of one fit.
pub fn build_features(inputs: &FitInputs, jobs: usize, stages: &mut dyn Stages) -> Result<FeatureSet, String> {
    build_features_of_splits(inputs, jobs, stages, &FITTING_SPLITS)
}

/// The features of `splits` alone: without TRAIN none of its curricula is built and no TRAIN frame is held in memory.
#[allow(clippy::too_many_lines)]
pub fn build_features_of_splits(inputs: &FitInputs, jobs: usize, stages: &mut dyn Stages, splits: &[&'static str]) -> Result<FeatureSet, String> {
    let train_built = splits.contains(&TRAIN);
    let root = inputs.tools.root.clone();
    let options = &inputs.options;
    let keep_importance = options.float("keep_importance");
    let frozen = inputs.base.is_some();
    let sources = captured_sources(&root.join(options["captured_curriculum"]["manifest"].as_str().ok_or("captured_curriculum.manifest")?))?;
    let heads = trainable_prefixes(&inputs.tools.root, &inputs.models_dir, options)?;
    let labelling = labelling_base(&root, &inputs.models_dir, options)?;
    let mut captured_report = Map::new();
    let file_name = |source: &CapturedSource| source.path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let mut extra: Vec<ActionRow> = Vec::new();
    let (capitals_report, english_report);
    let tools = &inputs.tools;
    for (split, rows) in &inputs.frames {
        stages.rows(&format!("natural-{split}"), rows);
    }
    if !train_built {
        let skipped = json!({"scope": "not built: TRAIN is not featurised"});
        for source in &sources {
            captured_report.insert(file_name(source), skipped.clone());
        }
        capitals_report = skipped.clone();
        english_report = skipped;
    } else if frozen {
        // Nothing but the tail is framed for TRAIN: the other TRAIN curricula are not built.
        let unused = json!({"scope": "not used: a frozen base keeps the word decided at its own boundary"});
        for source in &sources {
            captured_report.insert(file_name(source), unused.clone());
        }
        capitals_report = unused.clone();
        english_report = unused;
    } else {
        let detector_models = &inputs.lexicons_spelling;
        let detector = inputs.detector("reference_hunspell");
        let historical = historical_curriculum(tools, detector, detector_models)?;
        stages.rows("historical", &historical);
        eprintln!("historical: {} frames", historical.len());
        extra = historical;
        let captured = captured_all(inputs, &sources, &options["captured_curriculum"])?;
        for (index, (source, (rows, report))) in sources.iter().zip(captured).enumerate() {
            stages.rows(&format!("captured-{index}"), &rows);
            captured_report.insert(file_name(source), report);
            extra.extend(rows);
        }
        let (rows, report) = curricula::capital_citation_curriculum(tools, &inputs.source_rows[TRAIN], &options["capital_citation_curriculum"], &tools.portable);
        stages.rows("capitals", &rows);
        extra.extend(rows);
        capitals_report = report;
        let (rows, report) = curricula::english_capital_curriculum(tools, &inputs.source_rows[TRAIN], &options["english_capital_curriculum"], &tools.portable);
        stages.rows("english", &rows);
        extra.extend(rows);
        english_report = report;
    }
    let (tail, tail_report) = if train_built { tail_curricula(inputs)? } else { (Vec::new(), json!({"scope": "not built: TRAIN is not featurised"})) };
    stages.rows("tail", &tail);
    eprintln!("tail: {} frames; extra: {} frames", tail.len(), extra.len());
    let first = inputs.lexical_models(&inputs.profiles[0]).clone();
    // The engine's orthotactic model binds its dictionaries once, to the first profile's lexicons, for every replay.
    let known = ks_engine::engine::EngineCore::known_of(&first);
    let shared = match OrthoModel::load(&artifact_path(tools.resources.resource_dir()), Some(tools.resources.packaged_identifier()), known, Some(tools.resources.clone())) {
        Ok(model) => {
            let version = model.version.clone();
            (Some(Arc::new(model)), version)
        }
        Err(error) => (None, format!("unavailable: {}", error.name)),
    };
    let budgets = &options["span_maximum_families"];
    let mut vocabulary = Vocabulary::default();
    let mut columns = HashMap::new();
    let (mut chains, mut spans) = (HashMap::new(), HashMap::new());
    for profile in &inputs.profiles {
        for split in splits.iter().copied() {
            let natural = inputs.frames[split].clone();
            let rows = if split == TRAIN && !frozen {
                let mut all = natural;
                all.extend_from_slice(&extra);
                training_order(&mut all);
                all
            } else {
                natural
            };
            let (chain_rows, chain_report) = frame_chain(inputs, labelling.as_ref(), profile, split, rows, if split == TRAIN { &tail } else { &[] })?;
            stages.rows(&format!("chain-{profile}-{split}"), &chain_rows);
            let curriculum = if frozen && split == TRAIN {
                SpanCurriculum::default()
            } else {
                let folder = std::env::temp_dir().join(format!("ks-train-{}", std::process::id()));
                let mut replayer = Replayer::recording(tools, inputs.lexical_models(profile), shared.clone(), folder.clone());
                let built = build_span_curriculum(&mut replayer, &inputs.source_rows[split], budgets.int(split), split);
                let _ = std::fs::remove_dir_all(folder);
                built
            };
            if !(frozen && split == TRAIN) {
                spans.insert((profile.clone(), split), curriculum.report());
            }
            stages.frames(&format!("spans-{profile}-{split}"), &curriculum.frames.iter().cloned().map(Frame::Span).collect::<Vec<_>>());
            let mut entries = prepared_frames(chain_rows, curriculum.frames, split);
            eprintln!("{profile}/{split}: {} frames", entries.len());
            if split == TRAIN {
                if let Some(prefixes) = &heads {
                    entries.retain(|frame| head_frame(tools, frame, prefixes));
                    eprintln!("{profile}/{split}: {} frames ask a question of a head", entries.len());
                }
            }
            stages.frames(&format!("entries-{profile}-{split}"), &entries);
            let part = columns_of(inputs, &mut vocabulary, profile, split, &entries, keep_importance, jobs);
            stages.features(&format!("features-{profile}-{split}"), &part, &vocabulary.names);
            chains.insert((profile.clone(), split), chain_report);
            columns.insert((profile.clone(), split), part);
        }
    }
    Ok(FeatureSet { names: vocabulary.names, columns, chains, spans, captured: Value::Object(captured_report), capitals: capitals_report, english: english_report, tail: tail_report })
}

/// `trainable_prefixes`: the feature prefixes a fit onto a frozen base with heads can move, or None for any other fit.
fn trainable_prefixes(root: &Path, models_dir: &Path, options: &Value) -> Result<Option<BTreeSet<String>>, String> {
    let heads = frozen_heads(options)?;
    if heads.is_empty() {
        return Ok(None);
    }
    let base = named_base(root, models_dir, options, "frozen_base")?.ok_or("heads need a frozen base")?;
    let mut prefixes: BTreeSet<String> = heads.keys().cloned().collect();
    if !carried_prefixes(&base).contains(KEPT_FEATURE_PREFIX) {
        prefixes.insert(KEPT_FEATURE_PREFIX.to_owned());
    }
    Ok(Some(prefixes))
}

/// `labelling_base`: the model whose answers label the lone-word frames the counts leave open.
fn labelling_base(root: &Path, models_dir: &Path, options: &Value) -> Result<Option<ContextModel>, String> {
    if alone_evidence(options).is_none() {
        return Ok(None);
    }
    let spec = &options["frozen_base"];
    match spec.get("labels").filter(|value| !value.is_null()) {
        Some(labels) => pinned_model(root, models_dir, labels, "labelling base").map(Some),
        None => named_base(root, models_dir, options, "frozen_base"),
    }
}

impl FeatureSet {
    /// `masses`: FeatureMass over the TRAIN frames of every profile, in the order the serial fit adds them.
    fn masses(&self, profiles: &[String]) -> HashMap<String, f64> {
        let mut values = vec![0.0_f64; self.names.len()];
        let mut errors = vec![0.0_f64; self.names.len()];
        let mut present = vec![false; self.names.len()];
        for profile in profiles {
            let part = &self.columns[&(profile.clone(), TRAIN)];
            let mut position = 0;
            for (count, weight) in part.counts.iter().zip(part.weights.iter()) {
                let end = position + *count as usize;
                for id in &part.ids[position..end] {
                    let id = *id as usize;
                    let previous = values[id];
                    let increment = weight - errors[id];
                    let total = previous + increment;
                    assert!(total.is_finite(), "feature mass overflow");
                    errors[id] = (total - previous) - increment;
                    values[id] = total;
                    present[id] = true;
                }
                position = end;
            }
        }
        self.names.iter().enumerate().filter(|(id, _)| present[*id]).map(|(id, name)| (name.clone(), values[id])).collect()
    }

    fn packed(&self, parts: &[&Columns], column: &[i64]) -> Packed {
        let mut result = Packed { offsets: vec![0], ..Packed::default() };
        for part in parts {
            let mut position = 0;
            for (row, count) in part.counts.iter().enumerate() {
                let end = position + *count as usize;
                let start_of_row = result.indices.len();
                for entry in position..end {
                    let target = column[part.ids[entry] as usize];
                    if target >= 0 {
                        result.indices.push(target as u32);
                        result.values.push(part.values[entry]);
                    }
                }
                debug_assert!(result.indices[start_of_row..].windows(2).all(|pair| pair[0] < pair[1]));
                result.offsets.push(result.indices.len() as u64);
                result.labels.push(part.labels[row]);
                result.importance.push(part.importance[row]);
                position = end;
            }
        }
        result
    }

    fn part(&self, profile: &str, split: &'static str) -> &Columns {
        &self.columns[&(profile.to_owned(), split)]
    }

    pub fn part_of(&self, profile: &str, split: &'static str) -> &Columns {
        self.part(profile, split)
    }

    pub fn feature_names(&self) -> &[String] {
        &self.names
    }
}

fn base_weights(names: &[String], base: &ContextModel) -> Vec<f64> {
    let mut weights = vec![0.0; names.len() * ACTIONS];
    for (index, name) in names.iter().enumerate() {
        if let Some(values) = base.weights.get(name) {
            weights[index * ACTIONS..(index + 1) * ACTIONS].copy_from_slice(values);
        }
    }
    weights
}

/// `trainable_frames`: the frames that hold a feature whose AdaGrad accumulator is finite, in their order.
fn trainable_frames(data: &Packed, accumulators: &[f64]) -> Packed {
    let mut result = Packed { offsets: vec![0], ..Packed::default() };
    for row in 0..data.rows() {
        let range = data.offsets[row] as usize..data.offsets[row + 1] as usize;
        if data.indices[range.clone()].iter().any(|index| accumulators[*index as usize * ACTIONS].is_finite()) {
            result.indices.extend_from_slice(&data.indices[range.clone()]);
            result.values.extend_from_slice(&data.values[range]);
            result.offsets.push(result.indices.len() as u64);
            result.labels.push(data.labels[row]);
            result.importance.push(data.importance[row]);
        }
    }
    result
}

fn best_of(scores: &[f64]) -> usize {
    let mut best = 0;
    for (index, score) in scores.iter().enumerate().skip(1) {
        if *score > scores[best] {
            best = index;
        }
    }
    best
}

/// `apply_support_mask`: an unsupported row may neither keep nor convert; both become suggestions.
fn apply_support_mask(probabilities: &[f64], supported: &[bool]) -> Vec<f64> {
    let mut result = probabilities.to_vec();
    for (index, available) in supported.iter().enumerate() {
        let range = index * ACTIONS..(index + 1) * ACTIONS;
        let selected = best_of(&result[range.clone()]);
        if !*available && selected < 2 {
            result[range].copy_from_slice(&[0.0, 0.0, 0.0, 1.0]);
        }
    }
    result
}

fn with_runtime_support(model: &ContextModel, part: &Columns, names: &[String]) -> Vec<bool> {
    part.feature_rows(names).map(|(features, _, _)| model.supports_features(&features)).collect()
}

/// `outside_lone_word`: calibration at the serving threshold on the frames outside the lone-word head's class.
fn outside_lone_word(predictions: &[(String, Vec<f64>, Vec<u8>)], parts: &[&Columns], names: &[String], threshold: f64) -> Value {
    let mut by_profile = Map::new();
    let (mut rows, mut wrong, mut right, mut possible) = (0_i64, 0_i64, 0_i64, 0_i64);
    for ((name, values, labels), part) in predictions.iter().zip(parts) {
        let mut position = 0;
        let (mut subset, mut kept_labels) = (Vec::new(), Vec::new());
        for (index, count) in part.counts.iter().enumerate() {
            let end = position + *count as usize;
            if !part.ids[position..end].iter().any(|id| names[*id as usize].starts_with(ALONE_FEATURE_PREFIX)) {
                subset.extend_from_slice(&values[index * ACTIONS..(index + 1) * ACTIONS]);
                kept_labels.push(labels[index]);
            }
            position = end;
        }
        let row = metrics(&subset, &kept_labels, threshold);
        rows += row["rows"].as_i64().unwrap_or(0);
        wrong += row["false_conversions"].as_i64().unwrap_or(0);
        right += row["converted_correctly"].as_i64().unwrap_or(0);
        possible += row["convert_rows"].as_i64().unwrap_or(0);
        by_profile.insert(name.clone(), row);
    }
    json!({"rows": rows, "false_conversions": wrong, "converted_correctly": right, "convert_rows": possible,
        "conversion_recall": if possible > 0 { right as f64 / possible as f64 } else { 0.0 }, "by_profile": by_profile})
}

fn selection_value(selection: &EpochSelection) -> Value {
    let by_profile: Map<String, Value> = selection.by_profile.iter().cloned().collect();
    json!({"threshold": selection.threshold, "minimum_recall": selection.minimum_recall, "pooled_recall": selection.pooled_recall, "loss": selection.loss,
        "by_profile": by_profile, "net_benefit": selection.net_benefit, "minimum_net_benefit": selection.minimum_net_benefit})
}

fn rounded(weights: &[f64]) -> Vec<f64> {
    weights.iter().map(|value| py_round_ndigits(*value, DETERMINISTIC_ROUNDING_DECIMALS).expect("a finite weight")).collect()
}

fn own_rows(inputs: &FitInputs, frames: &[ActionRow], profile: &str, keep_importance: f64, names: &HashMap<&str, u32>) -> Packed {
    // In name order, as the serial fit read them back from its feature files.
    let mut packed = Packed { offsets: vec![0], ..Packed::default() };
    for row in frames {
        let item = frame_evidence(&inputs.tools, row, DEVELOPMENT, inputs.detector(profile), Some(&inputs.ortho));
        for (name, value) in sorted_features(&inputs.tools, &item) {
            if let Some(index) = names.get(name.as_str()) {
                packed.indices.push(*index);
                packed.values.push(value);
            }
        }
        packed.offsets.push(packed.indices.len() as u64);
        let label = ACTION_NAMES.iter().position(|action| *action == row.action).expect("an action") as u8;
        packed.labels.push(label);
        packed.importance.push(row.sample_weight * if label == 0 { keep_importance } else { 1.0 });
    }
    packed
}

fn total_mass(data: &[(String, Packed)]) -> f64 {
    ks_lang::py_sum(data.iter().map(|(_, packed)| ks_lang::py_sum(packed.importance.iter().copied())))
}

fn loss_over(checks: &[(String, Packed)], weights: &[f64]) -> f64 {
    ks_lang::py_sum(checks.iter().map(|(_, data)| kernel::log_loss(&kernel::predict(data, weights), data)))
}

fn write_all(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = std::fs::File::create(path).map_err(|error| error.to_string())?;
    file.write_all(bytes).map_err(|error| error.to_string())
}

/// What a seal pins: the bytes of everything a fit reads (`provenance`).
pub fn provenance(root: &Path, recipe_path: &Path) -> Result<BTreeMap<String, String>, String> {
    fn walk(folder: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
        for entry in std::fs::read_dir(folder).map_err(|error| error.to_string())? {
            let path = entry.map_err(|error| error.to_string())?.path();
            if path.is_dir() {
                walk(&path, out)?;
            } else if path.extension().is_some_and(|extension| extension == "rs" || extension == "toml") {
                out.push(path);
            }
        }
        Ok(())
    }
    let mut paths: Vec<PathBuf> = Vec::new();
    for crate_name in ["ks-text", "ks-backend", "ks-config", "ks-lang", "ks-context", "ks-engine", "ks-replay", "ks-train", "ks-hunspell"] {
        walk(&root.join("crates").join(crate_name).join("src"), &mut paths)?;
    }
    paths.push(root.join("Cargo.lock"));
    for folder in ["resources/models"] {
        for entry in std::fs::read_dir(root.join(folder)).map_err(|error| error.to_string())? {
            let path = entry.map_err(|error| error.to_string())?.path();
            if path.is_file() && !matches!(path.file_name().and_then(|name| name.to_str()), Some("context_policy_v1.json" | "prefix_policy_v1.json")) {
                paths.push(path);
            }
        }
    }
    for file in [
        "model/context_v1/scenarios.json", "model/context_v1/captured/manifest.json", "model/context_v3/captured/manifest.json", "model/context_v3/baseline-context-v1.json",
        "model/intent_v1/config.json", "model/intent_v1/sources/en_US.lm", "model/intent_v1/sources/ru_RU.lm", "resources/protected_tokens.txt", "resources/identifiers.json",
        "resources/lexicon-supplement-ru_RU.json",
    ] {
        paths.push(root.join(file));
    }
    paths.push(recipe_path.to_path_buf());
    let mut result = BTreeMap::new();
    for path in paths {
        let name = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().into_owned();
        result.insert(name, checksum(&path)?);
    }
    Ok(result)
}

/// The inputs of a fit.
pub struct FitRequest<'a> {
    pub root: &'a Path,
    pub resource_dir: &'a Path,
    pub recipe: &'a Path,
    pub corpus: &'a Path,
    pub output: &'a Path,
    pub ledger: &'a Path,
    pub jobs: usize,
}

/// `fit`: every back end of the Python built the same candidate; so does this one.
#[allow(clippy::too_many_lines)]
pub fn fit(request: &FitRequest<'_>, stages: &mut dyn Stages) -> Result<Value, String> {
    let (root, corpus, output) = (request.root, request.corpus, request.output);
    if output.exists() {
        return Err("candidate directory already exists; never overwrite an experiment".to_owned());
    }
    let options = read_json(request.recipe)?;
    if options.get("schema_version").and_then(Value::as_i64) != Some(1) || options.get("feature_version").and_then(Value::as_i64) != Some(CONTEXT_ACTION_FEATURE_VERSION) {
        return Err("invalid action recipe".to_owned());
    }
    let before_provenance = provenance(root, request.recipe)?;
    let corpus_hash = checksum(&corpus.join("manifest.json"))?;
    let inputs = load_inputs(root, request.resource_dir, corpus, options.clone(), request.ledger)?;
    let models_dir = inputs.models_dir.clone();
    let base = named_base(root, &models_dir, &options, "frozen_base")?;
    let heads = frozen_heads(&options)?;
    let warm = named_base(root, &models_dir, &options, "warm_base")?;
    if base.is_some() && warm.is_some() {
        return Err("a recipe names a frozen base or a warm base, not both".to_owned());
    }
    let kept_options = &options["kept_neighbour_curriculum"];
    if base.is_some() && heads.is_empty() {
        if options["english_capital_curriculum"]["words_by_length"].as_object().is_some_and(|table| table.values().any(|value| value.as_i64() != Some(0))) {
            return Err("English-capital frames fit the word decided at its own boundary, which a frozen base keeps".to_owned());
        }
        if ["term_frames", "abbreviation_frames"].iter().any(|name| options["counted_token_curriculum"].int(name) != 0) {
            return Err("counted-token frames fit the word decided at its own boundary, which a frozen base keeps".to_owned());
        }
    }
    std::fs::create_dir_all(output).map_err(|error| error.to_string())?;
    write_all(&output.join("recipe.json"), &crate::artifact::canonical(&options))?;
    let features = build_features(&inputs, request.jobs, stages)?;
    let keep_importance = options.float("keep_importance");
    let tools = &inputs.tools;
    let profiles = inputs.profiles.clone();

    let minimum = options.float("minimum_feature_mass");
    let masses = features.masses(&profiles);
    // The kept-neighbour question and every head have feature names of their own and a budget of their own.
    let start = base.as_ref().or(warm.as_ref());
    let own: Vec<&str> = std::iter::once(KEPT_FEATURE_PREFIX).chain(HEAD_PREFIXES.iter().map(|head| head.1)).collect();
    let carried = base.as_ref().map(carried_prefixes).unwrap_or_default();
    let mut budgets: Vec<(String, usize)> = vec![(KEPT_FEATURE_PREFIX.to_owned(), kept_options.int("maximum_features") as usize)];
    budgets.extend(heads.iter().map(|(prefix, budget)| (prefix.clone(), *budget)));
    budgets.retain(|(prefix, _)| !carried.contains(prefix));
    let mut names: Vec<String> = match start {
        Some(model) => model.weights.names().map(str::to_owned).collect(),
        None => {
            let plain: HashMap<String, f64> = masses.iter().filter(|(name, _)| !own.iter().any(|prefix| name.starts_with(prefix))).map(|(name, mass)| (name.clone(), *mass)).collect();
            select_features(&plain, minimum, options.int("maximum_features") as usize)?
        }
    };
    for (prefix, budget) in &budgets {
        let part: HashMap<String, f64> = masses.iter().filter(|(name, _)| name.starts_with(prefix.as_str())).map(|(name, mass)| (name.clone(), *mass)).collect();
        names.extend(select_features(&part, minimum, *budget)?);
    }
    names.sort();
    let index: HashMap<&str, u32> = names.iter().enumerate().map(|(position, name)| (name.as_str(), position as u32)).collect();
    let column: Vec<i64> = features.names.iter().map(|name| index.get(name.as_str()).map_or(-1, |position| i64::from(*position))).collect();
    let train_parts: Vec<&Columns> = profiles.iter().map(|profile| features.part(profile, TRAIN)).collect();
    let mut train = features.packed(&train_parts, &column);
    let development: Vec<(String, Packed)> = profiles.iter().map(|profile| (profile.clone(), features.packed(&[features.part(profile, DEVELOPMENT)], &column))).collect();
    let calibration: Vec<(String, Packed)> = profiles.iter().map(|profile| (profile.clone(), features.packed(&[features.part(profile, CALIBRATION)], &column))).collect();
    let entries: Vec<(&str, [f64; ACTIONS])> = names.iter().map(|name| (name.as_str(), [0.0; ACTIONS])).collect();
    let support_model = ContextModel::new(WeightTable::from_entries(entries), "context-v3-vocabulary", 0.99, tools.tables.clone());
    let development_masks: Vec<Vec<bool>> = profiles.iter().map(|profile| with_runtime_support(&support_model, features.part(profile, DEVELOPMENT), &features.names)).collect();
    let development_mass = total_mass(&development);
    let mut weights = match start {
        Some(model) => base_weights(&names, model),
        None => vec![0.0; names.len() * ACTIONS],
    };
    let mut accumulators = vec![1.0; names.len() * ACTIONS];
    if let Some(model) = &base {
        for (position, name) in names.iter().enumerate() {
            if model.weights.get(name).is_some() {
                accumulators[position * ACTIONS..(position + 1) * ACTIONS].fill(f64::INFINITY);
            }
        }
        // The epochs read only the frames that hold a weight the fit can move: the others step nothing.
        let frames = train.rows();
        train = trainable_frames(&train, &accumulators);
        println!("frames with a trainable feature: {} of {frames}", train.rows());
    }
    let rate = if warm.is_some() { options["warm_base"].float("learning_rate") } else { options.float("learning_rate") };
    let epochs = options.int("epochs") as usize;

    // The heads' own frames of DEVELOPMENT choose their epochs, as the kept-neighbour frames choose the kept-neighbour question's.
    let mut kept_check: Vec<(String, Packed)> = Vec::new();
    if base.is_some() {
        let (kept_frames, _) = curricula::kept_neighbour_curriculum(tools, &inputs.source_rows[DEVELOPMENT], kept_options, &tools.portable);
        println!("features kept-development: {} per profile", kept_frames.len());
        kept_check = profiles.iter().map(|profile| (profile.clone(), own_rows(&inputs, &kept_frames, profile, keep_importance, &index))).collect();
    }
    let kept_mass = total_mass(&kept_check);
    let mut own_checks: Vec<(&str, &str, Vec<(String, Packed)>, f64, Vec<usize>)> = Vec::new();
    for (prefix, label, _key) in OWN_EPOCH_HEADS {
        if base.is_none() || !heads.contains_key(prefix) {
            continue;
        }
        let (frames, _) = own_curriculum(tools, label, DEVELOPMENT, &inputs.source_rows[DEVELOPMENT], &options)?;
        println!("features {label}-development: {} per profile", frames.len());
        if frames.is_empty() {
            continue;
        }
        let checks: Vec<(String, Packed)> = profiles.iter().map(|profile| (profile.clone(), own_rows(&inputs, &frames, profile, keep_importance, &index))).collect();
        let mass = total_mass(&checks);
        let slots: Vec<usize> = names.iter().enumerate().filter(|(_, name)| name.starts_with(prefix)).flat_map(|(position, _)| (0..ACTIONS).map(move |action| position * ACTIONS + action)).collect();
        own_checks.push((prefix, label, checks, mass, slots));
    }
    let mut own_best: Vec<(Vec<f64>, usize, f64)> = own_checks.iter().map(|_| (Vec::new(), 0, f64::INFINITY)).collect();
    let (mut kept_best, mut kept_epoch, mut kept_loss) = (Vec::new(), 0, f64::INFINITY);
    let mut qualifying: Vec<Qualifying> = Vec::new();
    let mut history: Vec<Value> = Vec::new();
    let thresholds = development_thresholds(&options)?;
    kernel::epoch(&train, &mut weights, &mut accumulators, rate);
    for epoch in 0..epochs {
        let mut current = rounded(&weights);
        let mut record = Map::new();
        record.insert("epoch".to_owned(), json!(epoch + 1));
        std::thread::scope(|scope| -> Result<(), String> {
            // The next epoch trains in a thread while this one is scored.
            if epoch + 1 < epochs {
                let (train, weights, accumulators) = (&train, &mut weights, &mut accumulators);
                scope.spawn(move || kernel::epoch(train, weights, accumulators, rate));
            }
            if base.is_some() {
                let loss = loss_over(&kept_check, &current) / kept_mass;
                if loss < kept_loss {
                    kept_best.clone_from(&current);
                    kept_epoch = epoch + 1;
                    kept_loss = loss;
                }
                record.insert("kept_development_loss".to_owned(), json!(loss));
                println!("epoch {}: kept_development_loss={loss:.9}, best={kept_epoch}", epoch + 1);
            }
            for (slot, (_, label, checks, mass, _)) in own_checks.iter().enumerate() {
                let loss = loss_over(checks, &current) / mass;
                if loss < own_best[slot].2 {
                    own_best[slot] = (current.clone(), epoch + 1, loss);
                }
                record.insert(format!("{label}_development_loss"), json!(loss));
                println!("epoch {}: {label}_development_loss={loss:.9}, best={}", epoch + 1, own_best[slot].1);
            }
            // The other heads are ranked without the heads of their own epoch.
            for (_, _, _, _, slots) in &own_checks {
                for slot in slots {
                    current[*slot] = 0.0;
                }
            }
            if base.is_some() && heads.is_empty() {
                history.push(Value::Object(record.clone()));
                return Ok(());
            }
            let predictions: Vec<Vec<f64>> = development.iter().map(|(_, data)| kernel::predict(data, &current)).collect();
            let loss = ks_lang::py_sum(development.iter().zip(&predictions).flat_map(|((_, data), values)| {
                (0..data.rows()).map(move |row| -data.importance[row] * values[row * ACTIONS + usize::from(data.labels[row])].max(LOG_LOSS_PROBABILITY_FLOOR).ln())
            })) / development_mass;
            let masked: Vec<Vec<f64>> = predictions.iter().zip(&development_masks).map(|(values, mask)| apply_support_mask(values, mask)).collect();
            let table: Vec<(String, &[f64], &[u8])> = development.iter().zip(&masked).map(|((name, data), values)| (name.clone(), values.as_slice(), data.labels.as_slice())).collect();
            let selection = assess_epoch(&table, &thresholds, loss, options["epoch_selection"].int("minimum_net_benefit_per_profile"))?;
            let mut entry = record.clone();
            entry.insert("development_loss".to_owned(), json!(loss));
            entry.insert("selection".to_owned(), selection.as_ref().map_or(Value::Null, selection_value));
            history.push(Value::Object(entry));
            if let Some(selection) = selection {
                qualifying.push(Qualifying { epoch: epoch + 1, selection, weights: current.clone(), loss });
            }
            println!("epoch {}: development_loss={loss:.9}", epoch + 1);
            Ok(())
        })?;
    }
    let settled = settled_epoch(&qualifying).map(|entry| (entry.epoch, entry.selection.clone(), entry.weights.clone(), entry.loss));
    if (base.is_none() || !heads.is_empty()) && settled.is_none() {
        return Err("no epoch repaired more than it broke on development".to_owned());
    }
    let (mut best, mut best_epoch, mut best_loss, best_selection) = match settled {
        Some((epoch, selection, weights, loss)) => (weights, epoch, loss, Some(selection)),
        None => (Vec::new(), 0, f64::INFINITY, None),
    };
    if base.is_some() && heads.is_empty() {
        best.clone_from(&kept_best);
        best_epoch = kept_epoch;
        best_loss = kept_loss;
    } else if base.is_some() {
        // The heads at the epoch development ranks best, the kept-neighbour head at its own.
        for (position, name) in names.iter().enumerate() {
            let range = position * ACTIONS..(position + 1) * ACTIONS;
            if name.starts_with(KEPT_FEATURE_PREFIX) {
                best[range.clone()].copy_from_slice(&kept_best[range]);
            } else if let Some(slot) = own_checks.iter().enumerate().position(|(slot, check)| name.starts_with(check.0) && own_best[slot].1 != 0) {
                best[range.clone()].copy_from_slice(&own_best[slot].0[range]);
            }
        }
    }
    let gates = &options["gate_policy"];
    let mut mapping: BTreeMap<String, Vec<f64>> = names.iter().enumerate().map(|(position, name)| (name.clone(), best[position * ACTIONS..(position + 1) * ACTIONS].to_vec())).collect();
    let entries: Vec<(&str, [f64; ACTIONS])> = names.iter().enumerate().map(|(position, name)| {
        let mut row = [0.0; ACTIONS];
        row.copy_from_slice(&best[position * ACTIONS..(position + 1) * ACTIONS]);
        (name.as_str(), row)
    }).collect();
    let candidate = ContextModel::new(WeightTable::from_entries(entries), "context-v3-fitting", 0.99, tools.tables.clone());
    let calibration_predictions: Vec<(String, Vec<f64>, Vec<u8>)> = calibration
        .iter()
        .map(|(name, data)| {
            let supported = with_runtime_support(&candidate, features.part(name, CALIBRATION), &features.names);
            (name.clone(), apply_support_mask(&kernel::predict(data, &best), &supported), data.labels.clone())
        })
        .collect();
    if let Some(model) = &base {
        for name in model.weights.names() {
            let values = model.weights.get(name).expect("a name of the table");
            let same = mapping.get(name).is_some_and(|row| row.as_slice() == values.as_slice());
            if !same {
                return Err("a weight of the frozen base moved".to_owned());
            }
            // Equal, and now the very values of the base: a zero step may leave -0.0 as 0.0.
            mapping.insert(name.to_owned(), values.to_vec());
        }
    }
    let band = &options["threshold_selection"];
    let candidates: Vec<f64> = match &base {
        Some(model) => vec![model.conversion_threshold],
        None => options["threshold_candidates"].as_array().ok_or("threshold_candidates")?.iter().filter_map(Value::as_f64).collect(),
    };
    let table: Vec<(String, &[f64], &[u8])> = calibration_predictions.iter().map(|(name, values, labels)| (name.clone(), values.as_slice(), labels.as_slice())).collect();
    let (threshold, mut calibration_report, passed) = choose_threshold(&table, &candidates, &ThresholdOptions {
        minimum_net_benefit: gates.int("minimum_calibration_net_benefit"),
        minimum_recall: gates.float("minimum_calibration_conversion_recall"),
        minimum_threshold: match (&base, &best_selection) {
            (Some(model), _) => model.conversion_threshold,
            (None, Some(selection)) => selection.threshold,
            (None, None) => 0.0,
        },
        net_benefit_tolerance: band.float("net_benefit_tolerance"),
        maximum_threshold: band.float("maximum_threshold"),
        authored_floor: band.float("authored_floor"),
    })?;
    let calibration_parts: Vec<&Columns> = profiles.iter().map(|profile| features.part(profile, CALIBRATION)).collect();
    calibration_report["outside_lone_word"] = outside_lone_word(&calibration_predictions, &calibration_parts, &features.names, threshold);
    let weight_table: Map<String, Value> = mapping.iter().map(|(name, row)| (name.clone(), json!(row))).collect();
    let weights_value = Value::Object(weight_table);
    let weight_hash = hex(&Sha256::digest(ks_text::json_dumps(&weights_value, &JsonOptions { ensure_ascii: false, sort_keys: true, separators: Some((",".to_owned(), ":".to_owned())), ..JsonOptions::default() }).as_bytes()));
    let version = format!("context-v3-{}", &weight_hash[..VERSION_HASH_CHARACTERS]);
    let actions: Vec<&str> = ACTION_NAMES.iter().map(|action| action.as_str()).collect();
    let payload = json!({"actions": actions, "feature_version": CONTEXT_ACTION_FEATURE_VERSION, "weights": weights_value, "weights_sha256": weight_hash, "version": version, "conversion_threshold": threshold});
    write_all(&output.join(ARTIFACT), &crate::artifact::artifact_bytes(&payload))?;
    ContextModel::load_artifact(&output.join(ARTIFACT), &models_dir).map_err(|error| format!("the artifact written does not load: {}", error.name))?;
    if provenance(root, request.recipe)? != before_provenance || checksum(&corpus.join("manifest.json"))? != corpus_hash {
        return Err("training inputs changed while fitting; candidate cannot be sealed".to_owned());
    }
    let mut development_record = json!({"selected_epoch": best_epoch, "loss": best_loss, "history": history, "selection": best_selection.as_ref().map_or(Value::Null, selection_value)});
    if base.is_some() && !heads.is_empty() {
        development_record["kept_selected_epoch"] = json!(kept_epoch);
        development_record["kept_loss"] = json!(kept_loss);
    }
    for (slot, (_, label, _, _, _)) in own_checks.iter().enumerate() {
        if own_best[slot].1 != 0 {
            development_record[format!("{label}_selected_epoch")] = json!(own_best[slot].1);
            development_record[format!("{label}_loss")] = json!(own_best[slot].2);
        }
    }
    let by_profile = |make: &dyn Fn(&str) -> Option<Value>| -> Value {
        let mut table = Map::new();
        for profile in &profiles {
            if let Some(value) = make(profile) {
                table.insert(profile.clone(), value);
            }
        }
        Value::Object(table)
    };
    let chain_of = |profile: &str, split: &'static str| features.chains.get(&(profile.to_owned(), split)).cloned().unwrap_or(json!({}));
    let per_split = |name: &'static str, profile: &str| -> Value {
        let mut table = Map::new();
        for split in FITTING_SPLITS {
            if let Some(value) = chain_of(profile, split).get(name) {
                table.insert(split.to_owned(), value.clone());
            }
        }
        Value::Object(table)
    };
    let mut seal = json!({
        "schema_version": 1,
        "stage": if passed { SEALED_BEFORE_TEST } else { REJECTED_BEFORE_TEST },
        "artifact_sha256": checksum(&output.join(ARTIFACT))?,
        "model_version": version,
        "provenance": before_provenance,
        "corpus_manifest_sha256": corpus_hash,
        "recipe": options,
        "gate_policy": gates,
        "calibration": calibration_report,
        "development": development_record,
        "frozen_base": Value::Null,
        "warm_base": Value::Null,
        "conversion_threshold": threshold,
        "feature_count": names.len(),
        "span_curriculum": by_profile(&|profile| {
            let mut table = Map::new();
            for split in FITTING_SPLITS {
                if let Some(value) = features.spans.get(&(profile.to_owned(), split)) {
                    table.insert(split.to_owned(), value.clone());
                }
            }
            Some(Value::Object(table))
        }),
        "lookahead_curriculum": by_profile(&|profile| chain_of(profile, TRAIN).get("lookahead").cloned()),
        "natural_lookahead_curriculum": by_profile(&|profile| Some(per_split("natural", profile))),
        "lexical_short_pair_curriculum": by_profile(&|profile| chain_of(profile, TRAIN).get("lexical").cloned()),
        "captured_curriculum": features.captured,
        "capital_citation_curriculum": features.capitals,
        "english_capital_curriculum": features.english,
        "planned_mass_balance": by_profile(&|profile| chain_of(profile, TRAIN).get("balance").cloned()),
        "test_accessed": false,
        "scope": "natural KEEP plus declared layout/mixed-context interventions; sequence evaluation required",
    });
    if let Some(model) = &base {
        let spec = &options["frozen_base"];
        let mut record = json!({"artifact": spec["artifact"], "sha256": spec["sha256"], "model_version": model.version, "conversion_threshold": model.conversion_threshold});
        if let Some(labels) = spec.get("labels").filter(|value| !value.is_null()) {
            let labelling = pinned_model(root, &models_dir, labels, "labelling base")?;
            let mut entry = labels.clone();
            entry["model_version"] = json!(labelling.version);
            record["labels"] = entry;
        }
        seal["frozen_base"] = record;
    }
    if let Some(model) = &warm {
        let spec = &options["warm_base"];
        seal["warm_base"] = json!({"artifact": spec["artifact"], "sha256": spec["sha256"], "model_version": model.version, "learning_rate": rate});
    }
    if alone_evidence(&options).is_some() {
        seal["lone_word_curriculum"] = by_profile(&|profile| Some(per_split("lone_word", profile)));
    }
    if let Some(table) = features.tail.as_object() {
        for (key, value) in table {
            seal[key.as_str()] = value.clone();
        }
    }
    write_all(&output.join(SEAL), &readable(&seal))?;
    println!("{}: {version}; threshold={threshold}; test not read", seal["stage"].as_str().unwrap_or(""));
    let _ = expanded_aliases;
    let _ = extract_action_features;
    let _ = pystr::len;
    Ok(seal)
}

/// `development_thresholds`: the operating points an epoch may be selected for: those the serving band can hold.
fn development_thresholds(options: &Value) -> Result<Vec<f64>, String> {
    let ceiling = options["threshold_selection"].float("maximum_threshold");
    let candidates: Vec<f64> = options["threshold_candidates"].as_array().ok_or("threshold_candidates")?.iter().filter_map(Value::as_f64).filter(|value| *value <= ceiling).collect();
    if candidates.is_empty() {
        return Err("no threshold candidate lies inside the serving band".to_owned());
    }
    Ok(candidates)
}
