//! Authored regressions and engine replays of the boundary and prefix models (`evaluate_boundary_engine`,
//! `evaluate_prefix_engine`): in-process keys over the model editor, not another independent quality test.
use ks_cast::To as _;
use crate::corpus::{checksum, read_json};
use crate::fit::{provenance as tree_provenance, readable};
use crate::physical::{hex_digest, pair};
use crate::prefix::role_named;
use crate::tools::{LOCALES, Lexicons, reference_models};
use flate2::read::MultiGzDecoder;
use ks_backend::{FieldContext, FieldReader, FieldRole, KeyEvent};
use ks_config::config::SettingsStore;
use ks_config::history::HistoryStore;
use ks_context::context_model::ContextModel;
use ks_context::context_policy::SharedOrtho;
use ks_context::input_context::InputContext;
use ks_context::{ContextPolicy, VersionedPrefixModel};
use ks_engine::engine::{EngineCore, EngineParts};
use ks_lang::boundary_model::BoundaryModel;
use ks_lang::boundary_policy::BoundaryPolicy;
use ks_lang::constants::{BOUNDARY_V1_FEATURE_VERSION, PREFIX_INDEX_DICTIONARY_MAX_BYTES};
use ks_lang::early_switch::{PrefixIndex, dictionary_stems};
use ks_lang::ortho_model::{OrthoModel, artifact_path};
use ks_lang::{LanguageModel, Resources};
use ks_replay::harness::{Backend, FakeClock, UnreadField};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const CLOCK_START_SECONDS: f64 = 1000.0;
const EDITOR_WINDOW: i64 = 1;
const FIRST_KEY_SERIAL: usize = 100;
const PREFIX_SEQUENCES_PER_CATEGORY: usize = 32;
const PREFIX_MAX_RECORDED_FAILURES: usize = 24;
const PREFIX_KEY_BASE: usize = 100;
const PREFIX_EARLY_RESTORED_MIN_FRACTION: f64 = 0.7;
const PROFILES: [&str; 2] = ["portable", "reference_hunspell"];
const SCENARIOS: [(&str, &str); 18] = [
    ("ghj,ktvf", "проблема"), ("ghtlkj;bk", "предложил"), (",b,kbjntrf", "библиотека"), ("rjnjhe.", "которую"), ("rjnjhe.,", "которую,"), ("ghbdtn,", "привет,"),
    ("ghbdtn.", "привет."), ("ghbdtn...", "привет..."), ("ghbdtn;", "привет;"), ("hello,", "hello,"), ("hello...", "hello..."), ("hello,world", "hello,world"),
    ("don't", "don't"), ("example.org", "example.org"), ("path/file.py", "path/file.py"), ("pm2", "pm2"), ("HTTP", "HTTP"), ("we’re", "we’re"),
];
const BOUNDARY_SCOPE: &str = "authored in-process regression; fixed physical keys/live layout; no native OS proof or independent model evaluation";
const PREFIX_SCOPE: &str = "in-process physical keys/live layout/exact final text/two spaces; separate observed and simulated field contexts; not native OS proof";

fn extras(root: &Path, recipe: &str, names: &[&Path]) -> Result<Value, String> {
    let mut table: Map<String, Value> = tree_provenance(root, &root.join(recipe))?.into_iter().map(|(name, hash)| (name, Value::String(hash))).collect();
    for name in names {
        let relative = name.strip_prefix(root).map_err(|_| "a provenance file outside the tree")?;
        table.insert(relative.to_string_lossy().into_owned(), Value::String(checksum(name)?));
    }
    Ok(Value::Object(table))
}

fn shared_of(resources: &Arc<Resources>, models: &Lexicons) -> SharedOrtho {
    let known = EngineCore::known_of(models);
    match OrthoModel::load(&artifact_path(resources.resource_dir()), Some(resources.packaged_identifier()), known, Some(resources.clone())) {
        Ok(model) => {
            let version = model.version.clone();
            (Some(Arc::new(model)), version)
        }
        Err(error) => (None, format!("unavailable: {}", error.name)),
    }
}

/// `installed_context`: the policy over the installed context model.
fn policy_of(root: &Path, resources: &Arc<Resources>, shared: SharedOrtho) -> Result<ContextPolicy, String> {
    let mut policy = ContextPolicy::with_ortho(resources, None, shared, None);
    policy.model = Some(ContextModel::load_artifact(&root.join("resources/models/context_policy_v1.json"), &resources.resource_dir().join("models")).map_err(|error| error.name.to_string())?);
    Ok(policy)
}

struct Engine {
    clock: FakeClock,
    core: EngineCore,
    backend: Arc<Backend>,
    folder: PathBuf,
}

struct Parts<'a> {
    resources: &'a Arc<Resources>,
    models: &'a Lexicons,
    policy: ContextPolicy,
    reader: Arc<dyn FieldReader>,
    boundary: Option<Arc<BoundaryPolicy>>,
    prefix: Option<Arc<VersionedPrefixModel>>,
    indexes: Vec<Arc<PrefixIndex>>,
    settings: Vec<(&'a str, Value)>,
    backend: Arc<Backend>,
    folder: &'a Path,
}

fn engine(parts: Parts<'_>) -> Result<Engine, String> {
    let Parts { resources, models, mut policy, reader, boundary, prefix, indexes, settings, backend, folder } = parts;
    std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    let store = Arc::new(SettingsStore::new(Some(folder.join("settings.json"))));
    for (name, value) in settings {
        store.set(name, value, false).map_err(|error| format!("{error:?}"))?;
    }
    let clock = FakeClock::new();
    // Monotonic time is far from zero when a replay starts: text observed "at zero" would read as never observed.
    clock.advance(CLOCK_START_SECONDS);
    let (intent_model, intent_model_status) = resources.intent_model();
    policy.stream = InputContext::with_clock(clock.read());
    policy.reader = Some(reader);
    let core = EngineCore::from_parts(EngineParts {
        settings: store,
        history: Arc::new(HistoryStore::new(Some(folder.join("history.jsonl")), 200)),
        backend: backend.clone(),
        learning: None,
        backend_label: "editor".to_owned(),
        models: models.clone(),
        intent_model,
        intent_model_status,
        protected_tokens: resources.protected_tokens(),
        context_policy: policy,
        boundary_model: boundary,
        prefix_model: prefix,
        prefix_indexes: indexes,
        resource_dir: resources.resource_dir().to_path_buf(),
        clock: clock.read(),
        platform_reader: None,
    }).map_err(|error| format!("{error:?}"))?;
    Ok(Engine { clock, core, backend, folder: folder.to_path_buf() })
}

impl Engine {
    fn type_key(&mut self, character: char, characters: [char; 2], keycode: usize, timestamp: usize) -> Result<(), String> {
        let group = self.backend.group();
        let observed = characters[group.to::<usize>()];
        let pressed = KeyEvent {
            pressed: true,
            keycode: keycode.to::<u32>(),
            key_name: if observed == ' ' { "space".to_owned() } else { observed.to_string() },
            character: observed.to_string(),
            characters: characters.iter().map(char::to_string).collect(),
            group,
            state: 0,
            timestamp: timestamp.to::<i64>(),
            ..KeyEvent::default()
        };
        let _ = character;
        self.backend.type_key(&pressed);
        self.core.handle_inner(Arc::new(pressed.clone())).map_err(|error| format!("{error:?}"))?;
        self.core.handle_inner(Arc::new(KeyEvent { pressed: false, ..pressed })).map_err(|error| format!("{error:?}"))
    }

    /// The text typed, and the policy with its model back for the next replay: parsing the artifact took 97% of a replay.
    fn finish(self) -> (String, ContextPolicy) {
        let _ = std::fs::remove_dir_all(&self.folder);
        (self.backend.text().iter().collect(), self.core.context_policy)
    }
}

fn other_of(character: char, source: &str, target: &str) -> char {
    pair().translate(&character.to_string(), source, target).ok().and_then(|text| text.chars().next()).unwrap_or(character)
}

/// `replay` of the boundary regression: the text as typed and the injections before the hard boundary.
fn boundary_replay(original: &str, boundary: Option<Arc<BoundaryPolicy>>, policy: &mut Option<ContextPolicy>, resources: &Arc<Resources>, models: &Lexicons, folder: &Path) -> Result<(String, usize), String> {
    let backend = Arc::new(Backend::new("TestEditor", 0, Vec::new()));
    let settings = vec![("detection.context_policy", json!("off")), ("detection.early_switch", json!(false)), ("detection.learning", json!(false)), ("general.keep_history", json!(false))];
    let mut engine = engine(Parts { resources, models, policy: policy.take().ok_or("one engine at a time")?, reader: Arc::new(UnreadField), boundary, prefix: None, indexes: Vec::new(), settings, backend, folder })?;
    let mut early = 0;
    let typed: Vec<char> = original.chars().chain([' ']).collect();
    for (offset, character) in typed.iter().enumerate() {
        let serial = FIRST_KEY_SERIAL + offset;
        engine.type_key(*character, [*character, other_of(*character, "us", "ru")], serial, serial)?;
        if *character != ' ' {
            early = engine.backend.corrections().len();
        }
    }
    let (text, returned) = engine.finish();
    *policy = Some(returned);
    Ok((text, early))
}

/// `evaluate_boundary_engine`: the report's bytes.
pub fn boundary_report(root: &Path, resources: &Arc<Resources>, scratch: &Path) -> Result<Vec<u8>, String> {
    let candidate = root.join("model/boundary_v1/candidate.json");
    let runtime = extras(root, "model/context_v3/recipe.json", &[&candidate, &root.join("resources/models/boundary-v2.json"), &root.join("model/boundary_v2/corpus.json"), &root.join("model/boundary_v2/seal.json"), &root.join("resources/lexicon-supplement-ru_RU.json")])?;
    let models = reference_models(root, resources, false)?;
    let mut policy = Some(policy_of(root, resources, shared_of(resources, &models))?);
    let rejected = BoundaryModel::load(&candidate).map_err(|error| error.name.to_string())?;
    let rejected = BoundaryPolicy { model: rejected, feature_version: BOUNDARY_V1_FEATURE_VERSION };
    let active = BoundaryPolicy::load(&root.join("resources/models/boundary-v2.json")).map_err(|error| error.name.to_string())?;
    let mut results = Map::new();
    for (label, model) in [("legacy_no_segmentation", None), ("rejected_v1", Some(Arc::new(rejected))), ("active_v2", Some(Arc::new(active)))] {
        let (mut exact, mut changed, mut premature, mut mismatched) = (0, 0, 0, 0);
        let mut rows = Vec::new();
        for (original, expected) in SCENARIOS {
            let (actual, early) = boundary_replay(original, model.clone(), &mut policy, resources, &models, scratch)?;
            let wanted = format!("{expected} ");
            let matches = actual == wanted;
            exact += i64::from(matches);
            changed += i64::from(original == expected && !matches);
            premature += early;
            mismatched += i64::from(actual.chars().count() != wanted.chars().count());
            rows.push(json!({"original": original, "expected": wanted, "actual": actual, "before_hard_boundary": early}));
        }
        results.insert(label.to_owned(), json!({"rows": rows, "exact": exact, "changed_correct": changed, "injections_before_hard_boundary": premature, "length_mismatches": mismatched}));
    }
    Ok(readable(&json!({"schema_version": 1, "scope": BOUNDARY_SCOPE, "provenance": runtime, "results": results})))
}

/// `verify_boundary_v2`: the exact-text gate of the active model over the regression's report.
pub fn verify_boundary_report(report: &Value) -> Result<(), String> {
    let results = report["results"].as_object().filter(|table| table.len() == 3 && ["legacy_no_segmentation", "rejected_v1", "active_v2"].iter().all(|name| table.contains_key(*name))).ok_or("boundary-v2 engine comparisons missing")?;
    let active = &results["active_v2"];
    if active["exact"].as_u64() != Some(SCENARIOS.len().to::<u64>()) || ["changed_correct", "length_mismatches", "injections_before_hard_boundary"].iter().any(|key| active[*key] != json!(0)) {
        return Err("boundary-v2 exact-text gate failed".to_owned());
    }
    Ok(())
}

/// What a rerun of the prefix replay reads of a frozen row.
struct PrefixRow {
    sequence: i64,
    category: String,
    text: String,
    before: String,
    application: String,
    source: i32,
    desired: bool,
    role: FieldRole,
}

fn prefix_rows(root: &Path, split: &str) -> Result<Vec<PrefixRow>, String> {
    let directory = root.join("model/prefix_v1");
    let receipt = read_json(&directory.join("corpus.json"))?;
    let path = directory.join(format!("{split}.jsonl.gz"));
    if Some(checksum(&path)?.as_str()) != receipt["sha256"][split].as_str() {
        return Err("prefix corpus checksum mismatch".to_owned());
    }
    let mut text = String::new();
    MultiGzDecoder::new(std::fs::File::open(&path).map_err(|error| error.to_string())?).read_to_string(&mut text).map_err(|error| error.to_string())?;
    let mut unique: BTreeMap<i64, Value> = BTreeMap::new();
    for line in text.lines() {
        let row: Value = serde_json::from_str(line).map_err(|error| error.to_string())?;
        if row["length"] == json!(1) && row["profile"] == json!("portable") {
            unique.insert(row["sequence"].as_i64().ok_or("a sequence")?, row);
        }
    }
    let mut by_category: BTreeMap<String, Vec<(String, Value)>> = BTreeMap::new();
    for (sequence, row) in unique {
        by_category.entry(row["category"].as_str().ok_or("a category")?.to_owned()).or_default().push((hex_digest(&format!("prefix-engine:{sequence}")), row));
    }
    let mut selected = Vec::new();
    for (category, mut rows) in by_category {
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        for (_, row) in rows.into_iter().take(PREFIX_SEQUENCES_PER_CATEGORY) {
            selected.push(PrefixRow {
                sequence: row["sequence"].as_i64().ok_or("a sequence")?,
                category: category.clone(),
                text: row["text"].as_str().ok_or("a text")?.to_owned(),
                before: row["before"].as_str().ok_or("a field")?.to_owned(),
                application: row["application"].as_str().ok_or("an application")?.to_owned(),
                source: (row["source"].as_i64().ok_or("a source")?).to::<i32>(),
                desired: row["desired"].as_bool().ok_or("desired")?,
                role: role_named(row["role"].as_str().ok_or("a role")?)?,
            });
        }
    }
    Ok(selected)
}

/// `Reader`: the text before the caret as a field reader's, under the row's role.
struct Reader {
    backend: Arc<Backend>,
    role: FieldRole,
}

impl FieldReader for Reader {
    fn read(&self, application: &str, window: i64) -> Option<FieldContext> {
        let caret = self.backend.caret();
        let mut field = FieldContext::new(application, window.to_string());
        field.before = self.backend.slice(0, caret).iter().collect();
        field.role = self.role;
        "fixture-field".clone_into(&mut field.source);
        Some(field)
    }
}

struct PrefixLexicon {
    models: Lexicons,
    indexes: Vec<Arc<PrefixIndex>>,
    shared: SharedOrtho,
}

/// `prefix_corpus.lexicon`: the engine's models; indexes of the onboard lexicon and its fallback words without the
/// supplement, plus the stems of the Hunspell dictionary the serving model uses.
fn lexicon(root: &Path, resources: &Arc<Resources>, profile: &str) -> Result<PrefixLexicon, String> {
    let spelling = profile == "reference_hunspell";
    let models = reference_models(root, resources, spelling)?;
    let config = read_json(&root.join("model/intent_v1/config.json"))?;
    let mut indexes = Vec::new();
    for (group, locale) in LOCALES.iter().enumerate() {
        let path = root.join(config["sources"]["languages"][locale]["path"].as_str().ok_or("a lexicon without a path")?);
        let directory = path.parent().ok_or("a lexicon outside a folder")?.to_path_buf();
        let onboard = LanguageModel::load(locale, std::iter::empty::<&str>(), &[directory], LanguageModel::disabled_speller());
        let index = if spelling {
            let stems = dictionary_stems(Path::new(models[group].speller.source()), PREFIX_INDEX_DICTIONARY_MAX_BYTES);
            PrefixIndex::new(onboard.frequencies.words().iter().chain(stems.iter()), onboard.frequencies.clone())
        } else {
            PrefixIndex::new(onboard.frequencies.words().iter(), onboard.frequencies.clone())
        };
        indexes.push(Arc::new(index));
    }
    let shared = shared_of(resources, &models);
    Ok(PrefixLexicon { models, indexes, shared })
}

struct PrefixTask<'a> {
    resources: &'a Arc<Resources>,
    lexicon: &'a PrefixLexicon,
    prefix: Option<Arc<VersionedPrefixModel>>,
    native: bool,
}

/// `replay` of a prefix row: the text expected, as typed, the key of the early switch, the injections.
fn prefix_replay(row: &PrefixRow, task: &PrefixTask<'_>, policy: &mut Option<ContextPolicy>, folder: &Path) -> Result<(String, String, Option<usize>, usize), String> {
    let (source, target) = if row.source == 1 { ("ru", "us") } else { ("us", "ru") };
    let expected = if row.desired { pair().translate(&row.text, source, target).map_err(|error| format!("{error:?}"))? } else { row.text.clone() };
    let before: Vec<char> = row.before.chars().collect();
    let backend = Arc::new(Backend::new(&row.application, row.source, before));
    let settings = vec![("detection.context_policy", json!("assist")), ("detection.early_switch", json!(true)), ("detection.context_read_field", json!(task.native)), ("detection.learning", json!(false)), ("detection.respect_manual_layout", json!(false)), ("general.keep_history", json!(false))];
    let reader = Arc::new(Reader { backend: backend.clone(), role: row.role });
    let mut engine = engine(Parts { resources: task.resources, models: &task.lexicon.models, policy: policy.take().ok_or("one engine at a time")?, reader, boundary: task.resources.boundary_policy(), prefix: task.prefix.clone(), indexes: task.lexicon.indexes.clone(), settings, backend: backend.clone(), folder })?;
    engine.core.shared.set_focus_window(EDITOR_WINDOW);
    engine.core.context_policy.stream.focus(&row.application, EDITOR_WINDOW);
    engine.core.context_policy.stream.text.clone_from(&row.before);
    engine.core.context_policy.stream.updated_at = engine.clock.now();
    let mut early = None;
    let typed: Vec<char> = row.text.chars().chain([' ', ' ']).collect();
    for (index, character) in typed.iter().enumerate() {
        let other = other_of(*character, source, target);
        let characters = if row.source == 1 { [other, *character] } else { [*character, other] };
        engine.type_key(*character, characters, index + PREFIX_KEY_BASE, index + PREFIX_KEY_BASE)?;
        if early.is_none() && engine.core.early_switch_origin.is_some() {
            early = Some(index + 1);
        }
    }
    let injections = engine.backend.corrections().len();
    let (text, returned) = engine.finish();
    *policy = Some(returned);
    Ok((format!("{}{expected}  ", row.before), text, early, injections))
}

/// `evaluate_prefix_engine.evaluate`: the report, every profile, field context and variant over the selected rows.
pub fn prefix_report(root: &Path, resources: &Arc<Resources>, split: &str, scratch: &Path, jobs: usize) -> Result<Value, String> {
    let directory = root.join("model/prefix_v1");
    let (candidate, seal) = (directory.join("candidate.json"), directory.join("seal.json"));
    let runtime = extras(root, "model/prefix_v2/recipe.json", &[&candidate, &seal, &directory.join("corpus.json"), &root.join("resources/lexicon-supplement-ru_RU.json")])?;
    let selected = prefix_rows(root, split)?;
    let model = Arc::new(VersionedPrefixModel::load(&candidate).map_err(|error| error.name.to_string())?);
    let lexicons: BTreeMap<&str, PrefixLexicon> = PROFILES.iter().map(|profile| Ok((*profile, lexicon(root, resources, profile)?))).collect::<Result<_, String>>()?;
    let tasks: Vec<(&str, bool, &str)> = PROFILES.iter().flat_map(|profile| [false, true].into_iter().flat_map(move |native| ["shipping_no_prefix", "candidate"].into_iter().map(move |name| (*profile, native, name)))).collect();
    let outcome: Mutex<Vec<Option<Result<(BTreeMap<&str, i64>, Vec<Value>), String>>>> = Mutex::new((0..tasks.len()).map(|_| None).collect());
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for worker in 0..jobs.clamp(1, tasks.len()) {
            let (tasks, selected, lexicons, model, outcome, next) = (&tasks, &selected, &lexicons, &model, &outcome, &next);
            let folder = scratch.join(format!("worker-{worker}"));
            scope.spawn(move || {
                loop {
                    let number = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some((profile, native, name)) = tasks.get(number).copied() else { break };
                    let task = PrefixTask { resources, lexicon: &lexicons[profile], prefix: (name == "candidate").then(|| model.clone()), native };
                    let result = policy_of(root, resources, lexicons[profile].shared.clone()).and_then(|policy| prefix_task(selected, &task, policy, profile, name, &folder));
                    outcome.lock().expect("a report in progress")[number] = Some(result);
                }
            });
        }
    });
    let mut results: BTreeMap<String, BTreeMap<&str, BTreeMap<&str, i64>>> = BTreeMap::new();
    let mut failures: Vec<Value> = Vec::new();
    for (task, done) in tasks.iter().zip(outcome.into_inner().expect("finished")) {
        let (counts, failed) = done.ok_or("a task did not run")??;
        let (profile, native, name) = *task;
        let room = PREFIX_MAX_RECORDED_FAILURES.saturating_sub(failures.len());
        failures.extend(failed.into_iter().take(room));
        let label = format!("{profile}/{}", if native { "field" } else { "observed" });
        println!("{label} {name} {}", json!(counts));
        results.entry(label).or_default().insert(name, counts);
    }
    let passed = results.values().all(prefix_gates_hold);
    let identifiers: Vec<i64> = selected.iter().map(|row| row.sequence).collect();
    Ok(json!({"schema_version": 1, "split": split, "selection": format!("{PREFIX_SEQUENCES_PER_CATEGORY} hash-ranked situations per category, selected before scoring"), "sequence_ids": identifiers, "provenance": runtime, "scope": PREFIX_SCOPE, "results": results, "examples": failures, "passed": passed}))
}

/// The gates of one field-context variant: the candidate keeps every length, changes no more correct text and restores
/// no fewer words than the engine without a prefix model, and most of its restorations come before the word ends.
fn prefix_gates_hold(variant: &BTreeMap<&str, BTreeMap<&str, i64>>) -> bool {
    let (candidate, shipping) = (&variant["candidate"], &variant["shipping_no_prefix"]);
    candidate["length_mismatches"] == 0 && candidate["changed_correct"] <= shipping["changed_correct"] && candidate["restored"] >= shipping["restored"] && (candidate["early_restored"]).to::<f64>() >= PREFIX_EARLY_RESTORED_MIN_FRACTION * (candidate["desired"]).to::<f64>()
}

fn prefix_task<'a>(selected: &[PrefixRow], task: &PrefixTask<'_>, policy: ContextPolicy, profile: &str, name: &str, folder: &Path) -> Result<(BTreeMap<&'a str, i64>, Vec<Value>), String> {
    let mut policy = Some(policy);
    let context = format!("{profile}/{}", if task.native { "field" } else { "observed" });
    let mut counts: BTreeMap<&str, i64> = BTreeMap::new();
    let mut failures = Vec::new();
    for row in selected {
        let (expected, actual, early_at, injections) = prefix_replay(row, task, &mut policy, folder)?;
        let exact = actual == expected;
        let early = early_at.is_some_and(|key| key < row.text.chars().count());
        for (key, value) in [
            ("sequences", 1), ("desired", i64::from(row.desired)), ("exact", i64::from(exact)), ("restored", i64::from(row.desired && exact)), ("changed_correct", i64::from(!row.desired && !exact)),
            ("early_restored", i64::from(row.desired && exact && early)), ("length_mismatches", i64::from(actual.chars().count() != expected.chars().count())), ("injections", injections.to::<i64>()),
        ] {
            *counts.entry(key).or_insert(0) += value;
        }
        if !exact {
            failures.push(json!({"variant": name, "profile": context, "sequence": row.sequence, "category": row.category, "desired": row.desired, "expected": expected, "actual": actual, "early_at": early_at, "injections": injections}));
        }
    }
    // The first failures, in the order of the rows.
    failures.truncate(PREFIX_MAX_RECORDED_FAILURES);
    Ok((counts, failures))
}

/// The commands: `boundary` or `prefix`, with `--verify` against the report kept, `--refresh-runtime` to replace it and
/// keep the old one in the history, `--output` to write somewhere else.
pub fn command(root: &Path, resources: &Arc<Resources>, which: &str, verify: bool, refresh: bool, output: Option<&Path>, jobs: usize) -> Result<(), String> {
    let scratch = std::env::temp_dir().join(format!("keyswitch-engine-report-{}", std::process::id()));
    let (directory, report, content) = match which {
        "boundary" => {
            let directory = root.join("model/boundary_v1");
            let content = boundary_report(root, resources, &scratch)?;
            (directory.clone(), directory.join("engine-regression.json"), content)
        }
        "prefix" => {
            let directory = root.join("model/prefix_v1");
            let report = directory.join("engine-report.json");
            let mut result = prefix_report(root, resources, "test", &scratch, jobs)?;
            let result_object = result.as_object_mut().ok_or("a report")?;
            if report.exists() {
                let previous = read_json(&report)?;
                if refresh {
                    if result_object["sequence_ids"] != previous["sequence_ids"] {
                        return Err("runtime refresh cannot change the observed selection or candidate".to_owned());
                    }
                    let archive = directory.join("engine-history").join(format!("{}.json", checksum(&report)?));
                    result_object.insert("runtime_regression".to_owned(), json!({"previous_report": archive.strip_prefix(root).map_err(|_| "a report outside the tree")?.to_string_lossy(), "previous_sha256": checksum(&report)?}));
                } else if let Some(earlier) = previous.get("runtime_regression") {
                    result_object.insert("runtime_regression".to_owned(), earlier.clone());
                }
            }
            (directory.clone(), report, readable(&result))
        }
        other => return Err(format!("unknown engine report: {other}")),
    };
    let _ = std::fs::remove_dir_all(&scratch);
    if let Some(path) = output {
        std::fs::write(path, &content).map_err(|error| error.to_string())?;
    } else if verify {
        if std::fs::read(&report).map_err(|error| error.to_string())? != content {
            return Err(format!("{which} engine report changed"));
        }
    } else if report.exists() && !refresh {
        return Err("regression report already exists; use --verify or --refresh-runtime".to_owned());
    } else {
        if report.exists() {
            let history = directory.join("engine-history");
            std::fs::create_dir_all(&history).map_err(|error| error.to_string())?;
            std::fs::copy(&report, history.join(format!("{}.json", checksum(&report)?))).map_err(|error| error.to_string())?;
        }
        std::fs::write(&report, &content).map_err(|error| error.to_string())?;
    }
    let result: Value = serde_json::from_slice(&content).map_err(|error| error.to_string())?;
    if which == "boundary" {
        verify_boundary_report(&result)?;
    } else if result["passed"] != json!(true) {
        return Err("prefix engine gates failed".to_owned());
    }
    println!("{which} engine gates hold");
    Ok(())
}

const CONTEXT_V2_ROWS_PER_LOCALE: usize = 64;
const CONTEXT_V2_MAX_REPORTED_FAILURES: usize = 12;
const CONTEXT_V2_PHRASE_MIN: usize = 8;
const CONTEXT_V2_PHRASE_MAX: usize = 96;
const CONTEXT_V2_PUNCTUATION: &str = ".,!?'’-:;()";

/// The context-v2 candidate against the frozen v1: every length kept, no more correct text changed, no fewer restored.
fn context_v2_passed(results: &Map<String, Value>) -> bool {
    let number = |variant: &str, field: &str| results[variant][field].as_i64().unwrap_or(0);
    number("candidate", "length_mismatches") == 0 && number("candidate", "changed_correct") <= number("v1", "changed_correct") && number("candidate", "exactly_restored") >= number("v1", "exactly_restored")
}

/// `select_phrases` of the context-v2 replay: hash-ranked distinct groups of the old test partition, before any scoring.
fn context_v2_phrases<'a>(rows: &'a [crate::ortho_inputs::Assigned<'a>]) -> Vec<&'a crate::ortho_inputs::Assigned<'a>> {
    let fits = |row: &crate::ortho_inputs::Assigned<'_>| {
        let english = row.phrase.locale == "eng";
        row.phrase.text.chars().all(|character| {
            let letter = if english { character.is_ascii_alphabetic() } else { matches!(character, 'А'..='Я' | 'а'..='я' | 'Ё' | 'ё') };
            letter || ks_text::ucd::isspace(character) || CONTEXT_V2_PUNCTUATION.contains(character)
        })
    };
    let mut eligible: Vec<&crate::ortho_inputs::Assigned<'a>> = rows
        .iter()
        .filter(|row| row.split == "test" && (CONTEXT_V2_PHRASE_MIN..=CONTEXT_V2_PHRASE_MAX).contains(&row.phrase.text.chars().count()) && fits(row))
        .collect();
    eligible.sort_by_cached_key(|row| hex_digest(&format!("engine-replay:{}", row.phrase.identifier)));
    let mut selected = Vec::new();
    for locale in ["eng", "rus"] {
        let mut seen = std::collections::HashSet::new();
        let unique = eligible.iter().filter(|row| row.phrase.locale == locale && seen.insert(row.group.as_str())).copied();
        selected.extend(unique.take(CONTEXT_V2_ROWS_PER_LOCALE));
    }
    selected
}

/// `replay`: the keys of a phrase over the model editor, the glyphs following the layout the engine leaves.
fn context_v2_replay(text: &str, target: i32, initial: i32, setting: &str, policy: &mut Option<ContextPolicy>, resources: &Arc<Resources>, models: &Lexicons, boundary: Arc<BoundaryPolicy>, folder: &Path) -> Result<(String, usize), String> {
    let backend = Arc::new(Backend::new("TestEditor", initial, Vec::new()));
    let settings = vec![("detection.context_policy", json!(setting)), ("detection.early_switch", json!(false)), ("detection.learning", json!(false)), ("general.keep_history", json!(false)), ("diagnostics.technical_logging", json!(false))];
    let mut engine = engine(Parts { resources, models, policy: policy.take().ok_or("one engine at a time")?, reader: Arc::new(UnreadField), boundary: Some(boundary), prefix: None, indexes: Vec::new(), settings, backend: backend.clone(), folder })?;
    let (own, other) = if target == 0 { ("us", "ru") } else { ("ru", "us") };
    for (offset, desired) in text.chars().enumerate() {
        let serial = FIRST_KEY_SERIAL + offset;
        let alternate = other_of(desired, own, other);
        let characters = if target == 0 { [desired, alternate] } else { [alternate, desired] };
        let group = backend.group();
        let observed = characters[group.to::<usize>()];
        let pressed = KeyEvent {
            pressed: true,
            keycode: serial.to::<u32>(),
            key_name: if observed == ' ' { "space".to_owned() } else { observed.to_string() },
            character: observed.to_string(),
            characters: characters.iter().map(char::to_string).collect(),
            group,
            state: if ks_text::ucd::isupper(desired) { ks_backend::constants::SHIFT_MASK } else { 0 },
            timestamp: serial.to::<i64>(),
            ..KeyEvent::default()
        };
        backend.type_key(&pressed);
        engine.core.handle_inner(Arc::new(pressed.clone())).map_err(|error| format!("{error:?}"))?;
        engine.core.handle_inner(Arc::new(KeyEvent { pressed: false, ..pressed })).map_err(|error| format!("{error:?}"))?;
    }
    let injections = backend.corrections().len();
    let (typed, returned) = engine.finish();
    *policy = Some(returned);
    Ok((typed, injections))
}

/// `evaluate_context_engine`: the three variants of the rejected context-v2 on today's engine: the detector alone, the
/// frozen v1 and the candidate. A regression over an observed test, never a promotion.
pub fn context_v2_report(root: &Path, resources: &Arc<Resources>, scratch: &Path) -> Result<Value, String> {
    let phrases = crate::ortho_inputs::load_phrases(root)?;
    let assigned = crate::ortho_inputs::assign(&phrases);
    let selected = context_v2_phrases(&assigned);
    // The phrases are the frozen source's, whose bytes `load_phrases` checks against their pin: the selection is always
    // the same, and full.
    assert_eq!(selected.len(), CONTEXT_V2_ROWS_PER_LOCALE * 2, "insufficient independent phrase groups");
    let models = crate::retired::reference_models(root)?;
    let directory = root.join("model/context_v2");
    let boundary = Arc::new(BoundaryPolicy::load(&root.join("resources/models/boundary-v2.json")).map_err(|error| error.name.to_string())?);
    let (mut results, mut examples) = (Map::new(), Map::new());
    for (name, artifact) in [("detector", None), ("v1", Some("baseline-context-v1.json")), ("candidate", Some("candidate.json"))] {
        let mut policy = ContextPolicy::with_ortho(resources, None, shared_of(resources, &models), None);
        policy.model = match artifact {
            Some(file) => Some(ContextModel::load_artifact(&directory.join(file), &resources.resource_dir().join("models")).map_err(|error| error.name.to_string())?),
            None => None,
        };
        let mut policy = Some(policy);
        let setting = if artifact.is_some() { "assist" } else { "off" };
        let mut counts: BTreeMap<&str, i64> = BTreeMap::new();
        let mut failures = Vec::new();
        for row in &selected {
            let target = i32::from(row.phrase.locale != "eng");
            let text = format!("{} ", row.phrase.text);
            for initial in [target, 1 - target] {
                let (actual, injections) = context_v2_replay(&text, target, initial, setting, &mut policy, resources, &models, boundary.clone(), scratch)?;
                let correct = initial == target;
                let same = actual == text;
                *counts.entry("rows").or_insert(0) += 1;
                *counts.entry(if correct { "initially_correct" } else { "initially_wrong" }).or_insert(0) += 1;
                *counts.entry(if correct { "preserved_correct" } else { "exactly_restored" }).or_insert(0) += i64::from(same);
                *counts.entry("changed_correct").or_insert(0) += i64::from(correct && !same);
                *counts.entry("length_mismatches").or_insert(0) += i64::from(actual.chars().count() != text.chars().count());
                *counts.entry("injections").or_insert(0) += injections.to::<i64>();
                if !same {
                    failures.push(json!({"source_id": row.phrase.identifier, "initially_correct": correct, "expected": text, "actual": actual}));
                }
            }
        }
        results.insert(name.to_owned(), json!(counts));
        failures.truncate(CONTEXT_V2_MAX_REPORTED_FAILURES);
        examples.insert(name.to_owned(), json!(failures));
    }
    let passed = context_v2_passed(&results);
    let ids: Vec<i64> = selected.iter().map(|row| row.phrase.identifier).collect();
    Ok(json!({"schema_version": 1, "source_ids": ids, "results": results, "examples": examples, "promotion_passed": passed}))
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    fn variant(candidate: [i64; 5], shipping: [i64; 5]) -> BTreeMap<&'static str, BTreeMap<&'static str, i64>> {
        let counts = |values: [i64; 5]| ["length_mismatches", "changed_correct", "restored", "early_restored", "desired"].into_iter().zip(values).collect();
        BTreeMap::from([("candidate", counts(candidate)), ("shipping_no_prefix", counts(shipping))])
    }

    #[test]
    fn the_context_v2_candidate_passes_only_where_it_does_no_worse_than_v1() {
        let results = |candidate: [i64; 3]| -> Map<String, Value> {
            let counts = |values: [i64; 3]| json!({"length_mismatches": values[0], "changed_correct": values[1], "exactly_restored": values[2]});
            Map::from_iter([("candidate".to_owned(), counts(candidate)), ("v1".to_owned(), counts([0, 2, 10]))])
        };
        assert!(context_v2_passed(&results([0, 2, 10])));
        for candidate in [[1, 2, 10], [0, 3, 10], [0, 2, 9]] {
            assert!(!context_v2_passed(&results(candidate)), "{candidate:?}");
        }
    }

    #[test]
    fn each_gate_of_a_prefix_variant_can_fail_alone() {
        let shipping = [0, 2, 10, 0, 12];
        assert!(prefix_gates_hold(&variant([0, 2, 10, 9, 12], shipping)));
        for candidate in [[1, 2, 10, 9, 12], [0, 3, 10, 9, 12], [0, 2, 9, 9, 12], [0, 2, 10, 8, 12]] {
            assert!(!prefix_gates_hold(&variant(candidate, shipping)), "{candidate:?}");
        }
    }
}
