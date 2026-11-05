//! A sealed candidate measured against the frozen baseline pair by typing whole sequences through the engine
//! (`evaluate_context_action_sequences`). The test split is read here and nowhere else: only after the access is
//! claimed in the ledger, once for a test, and never for a test that shares a row, a family or a document with a test
//! read before.
use ks_cast::To as _;
use crate::corpus::{CorpusRow, DEVELOPMENT, checksum, load_rows, read_json};
use crate::curricula::Counts;
use crate::physical::{PhysicalKey, hex, hex_digest, key_of};
use crate::replay::{Mode, Plan, Replayer, prefix_indexes};
use crate::tools::{Lexicons, Tools, reference_models};
use ks_context::VersionedPrefixModel;
use ks_context::context_model::ContextModel;
use ks_engine::engine::EngineCore;
use ks_lang::Resources;
use ks_lang::ortho_model::{OrthoModel, artifact_path};
use ks_text::pystr;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const CORPUS_BEFORE_WINDOW_CHARACTERS: usize = 96;
const CORPUS_AFTER_WINDOW_CHARACTERS: usize = 64;
const SEQUENCE_DOCUMENT_CAP_PER_GROUP: usize = 512;
const MINIMUM_SEQUENCE_DOCUMENTS_PER_GROUP: i64 = 32;
pub const PROFILES: [&str; 2] = ["portable", "reference_hunspell"];
pub const BASELINE: &str = "model/context_v3/baseline-context-v1.json";
pub const PREFIX_BASELINE: &str = "model/prefix_v2/baseline-prefix-v1.json";
const SEALED_BEFORE_TEST: &str = "sealed-before-test";
/// The names of a pair in each settings mode, with its context and prefix models.
const PAIRS: [(Mode, &[(&str, &str, &str)]); 2] = [
    (Mode::EarlyOff, &[("baseline", "baseline", "baseline"), ("candidate", "candidate", "candidate")]),
    (Mode::Default, &[("baseline", "baseline", "baseline"), ("candidate", "candidate", "candidate"), ("candidate_context_baseline_prefix", "candidate", "baseline")]),
];
const MEMBERSHIP_IDENTITY_FIELDS: [&str; 3] = ["row_ids_sha256", "family_ids_sha256", "document_ids_sha256"];

/// `script_group`: `"a" <= character.lower() <= "z"`, and the same for the Russian letters.
pub(crate) fn script_group(character: char) -> Option<i64> {
    let lowered = pystr::lower(character.encode_utf8(&mut [0_u8; 4]));
    let text = lowered.as_str();
    if ("a"..="z").contains(&text) {
        return Some(0);
    }
    if ("а"..="я").contains(&text) || text == "ё" {
        return Some(1);
    }
    None
}

fn is_alpha(character: char) -> bool {
    crate::rows::is_alpha(character)
}

/// `trimmed_window`: at a full window clipped inside letters, only that outer letter fragment goes.
fn trimmed_window(row: &CorpusRow) -> (String, String, usize, usize) {
    let mut before: Vec<char> = row.before.chars().collect();
    let mut after: Vec<char> = row.after.chars().collect();
    let (mut left, mut right) = (0, 0);
    if before.len() == CORPUS_BEFORE_WINDOW_CHARACTERS && is_alpha(before[0]) {
        while left < before.len() && is_alpha(before[left]) {
            left += 1;
        }
        before.drain(..left);
    }
    if after.len() == CORPUS_AFTER_WINDOW_CHARACTERS && is_alpha(after[after.len() - 1]) {
        while right < after.len() && is_alpha(after[after.len() - 1 - right]) {
            right += 1;
        }
        after.truncate(after.len() - right);
    }
    (before.into_iter().collect(), after.into_iter().collect(), left, right)
}

/// `sequence_plan`.
pub fn sequence_plan(row: &CorpusRow, initially_wrong: bool) -> Plan {
    let declared = row.group.filter(|group| *group == 0 || *group == 1);
    assert!(!initially_wrong || declared.is_some(), "mixed or unclassified focus has no unique wrong layout");
    let focus_letter = declared.and_then(|group| row.original.chars().position(|character| script_group(character) == Some(group)));
    assert!(!initially_wrong || focus_letter.is_some(), "wrong intervention requires a focus letter in the declared group");
    let (before, after, left, right) = trimmed_window(row);
    let mut intended = format!("{before}{}", row.original).chars().find_map(script_group).or(row.group).filter(|group| *group != 0).unwrap_or(0);
    let mut keys: Vec<PhysicalKey> = Vec::new();
    let mut unsupported: BTreeSet<String> = BTreeSet::new();
    let mut first = true;
    let mut intervention_event = None;
    for (phase, text) in [("before", before.as_str()), ("focus", row.original.as_str()), ("after", after.as_str())] {
        for (index, character) in text.chars().enumerate() {
            let mut explicit: Option<i64> = None;
            let group = script_group(character);
            if first || group.is_some_and(|value| value != intended) {
                intended = group.unwrap_or(intended);
                explicit = Some(intended);
                first = false;
            }
            if let Some(declared) = declared.filter(|_| phase == "focus") {
                if index == 0 {
                    intended = declared;
                    explicit = Some(declared);
                }
                if Some(index) == focus_letter {
                    intended = declared;
                    explicit = Some(if initially_wrong { 1 - declared } else { declared });
                }
            }
            let mut candidate = key_of(character, intended.to::<usize>());
            if candidate.is_none() {
                candidate = key_of(character, (1 - intended).to::<usize>());
                if candidate.is_some() {
                    intended = 1 - intended;
                    explicit = Some(intended);
                }
            }
            let Some(key) = candidate else {
                unsupported.insert(format!("U+{:04X}", u32::from(character)));
                continue;
            };
            if phase == "focus" && Some(index) == focus_letter && initially_wrong {
                intervention_event = Some(keys.len() + 1);
            }
            let mut chosen = key.clone();
            chosen.explicit_group = explicit.map(ks_cast::To::to::<i32>);
            keys.push(chosen);
        }
    }
    let expected = format!("{before}{}{after}", row.original);
    Plan { identifier: row.identifier.clone(), document: row.document.clone(), group: row.group, initially_wrong, expected, keys, unsupported: unsupported.into_iter().collect(), trimmed_left: left, trimmed_right: right, expected_final_group: intended, intervention_event }
}

/// `select_rows`: hash-ranked documents and one hash-ranked focus per document and group.
#[must_use]
pub fn select_rows(rows: &[CorpusRow]) -> Vec<&CorpusRow> {
    let mut selected = Vec::new();
    for group in [Some(0), Some(1), None] {
        let mut documents: BTreeMap<&str, Vec<&CorpusRow>> = BTreeMap::new();
        for row in rows {
            if row.group == group {
                documents.entry(&row.document).or_default().push(row);
            }
        }
        let mut ranked: Vec<&str> = documents.keys().copied().collect();
        ranked.sort_by_cached_key(|name| (hex_digest(&format!("context-sequence:document:{name}")), (*name).to_owned()));
        for document in ranked.into_iter().take(SEQUENCE_DOCUMENT_CAP_PER_GROUP) {
            let focus = documents[document].iter().min_by_key(|row| (hex_digest(&format!("context-sequence:focus:{}", row.identifier)), row.identifier.clone())).expect("a document holds rows");
            selected.push(*focus);
        }
    }
    selected
}

fn zeroed_counts() -> Counts {
    let mut counts = Counts::default();
    for key in ["rows", "initially_correct", "initially_wrong", "preserved_correct", "exactly_restored", "correct_text_corruptions", "length_mismatches", "injections", "execution_errors", "correction_layout_mismatches", "final_layout_mismatches"] {
        counts.add_by(key, 0);
    }
    counts
}

/// `score_pair`.
fn score_pair(plans: &[Plan], replayer: &mut Replayer, mode: Mode) -> (Counts, Vec<Value>) {
    let mut counts = zeroed_counts();
    let mut results = Vec::new();
    for plan in plans {
        if !plan.unsupported.is_empty() {
            continue;
        }
        let outcome = replayer.replay(plan, mode);
        counts.add("rows");
        counts.add(if plan.initially_wrong { "initially_wrong" } else { "initially_correct" });
        counts.add_by(if plan.initially_wrong { "exactly_restored" } else { "preserved_correct" }, i64::from(outcome.exact));
        counts.add_by("correct_text_corruptions", i64::from(!plan.initially_wrong && !outcome.exact));
        counts.add_by("length_mismatches", i64::from(outcome.length_mismatch));
        counts.add_by("execution_errors", i64::from(outcome.error.is_some()));
        counts.add_by("injections", outcome.corrections.len().to::<i64>());
        counts.add_by("correction_layout_mismatches", outcome.corrections.iter().filter(|item| item.actual_group != item.target_group).count().to::<i64>());
        counts.add_by("final_layout_mismatches", i64::from(!outcome.final_layout_matches));
        let mut case = json!({"identifier": plan.identifier, "document": plan.document, "group": plan.group, "initially_wrong": plan.initially_wrong, "expected": plan.expected, "trimmed_left": plan.trimmed_left, "trimmed_right": plan.trimmed_right});
        if let (Some(table), Value::Object(outcome)) = (case.as_object_mut(), outcome.value()) {
            table.extend(outcome);
        }
        results.push(case);
    }
    (counts, results)
}

fn profile_gates(candidate: &Counts, baseline: &Counts, documents: &BTreeMap<String, i64>) -> Value {
    let enough = |group: &str| documents.get(group).copied().unwrap_or(0) >= MINIMUM_SEQUENCE_DOCUMENTS_PER_GROUP;
    json!({
        "correct_text_preserved": candidate.get("correct_text_corruptions") <= baseline.get("correct_text_corruptions"),
        "length_preserved": candidate.get("length_mismatches") == 0,
        "net_restorations_at_least_baseline": candidate.get("exactly_restored") - candidate.get("correct_text_corruptions") >= baseline.get("exactly_restored") - baseline.get("correct_text_corruptions"),
        "enough_documents_us": enough("0"),
        "enough_documents_ru": enough("1"),
        "execution_succeeded": candidate.get("execution_errors") == 0 && baseline.get("execution_errors") == 0,
        "correction_layouts_match": candidate.get("correction_layout_mismatches") == 0,
    })
}

/// The models of an evaluation.
pub struct Pairs {
    pub candidate: PathBuf,
    pub baseline: PathBuf,
    pub prefix_candidate: PathBuf,
    pub prefix_baseline: PathBuf,
}

/// `score_sequences`: every pair replayed in both settings modes; acceptance needs every gate in every mode and profile.
pub fn score_sequences(root: &Path, resources: &Arc<Resources>, tools: &Tools, spelling: &Lexicons, rows: &[CorpusRow], pairs: &Pairs, jobs: usize) -> Result<Value, String> {
    let selected = select_rows(rows);
    let mut plans = Vec::new();
    for row in &selected {
        plans.push(sequence_plan(row, false));
        if matches!(row.group, Some(0 | 1)) {
            plans.push(sequence_plan(row, true));
        }
    }
    let mut documents: BTreeMap<String, i64> = BTreeMap::new();
    for (name, group) in [("0", Some(0)), ("1", Some(1)), ("None", None)] {
        let distinct: BTreeSet<&str> = plans.iter().filter(|plan| plan.group == group && plan.unsupported.is_empty()).map(|plan| plan.document.as_str()).collect();
        documents.insert(name.to_owned(), distinct.len().to::<i64>());
    }
    let unsupported: Vec<Value> = plans.iter().filter(|plan| !plan.unsupported.is_empty() && !plan.initially_wrong).map(|plan| json!({"identifier": plan.identifier, "codepoints": plan.unsupported})).collect();
    let selection = json!({"source_ids": selected.iter().map(|row| row.identifier.as_str()).collect::<Vec<_>>(), "selected_rows": selected.len(), "replayed_documents": documents,
        "trimmed_rows": plans.iter().filter(|plan| !plan.initially_wrong && (plan.trimmed_left != 0 || plan.trimmed_right != 0)).count(), "unsupported": unsupported});
    let first = &tools.portable;
    let known = EngineCore::known_of(first);
    let shared = match OrthoModel::load(&artifact_path(resources.resource_dir()), Some(resources.packaged_identifier()), known, Some(resources.clone())) {
        Ok(model) => {
            let version = model.version.clone();
            (Some(Arc::new(model)), version)
        }
        Err(error) => (None, format!("unavailable: {}", error.name)),
    };
    let models_dir = resources.resource_dir().join("models");
    let load_context = |path: &Path| ContextModel::load_artifact(path, &models_dir).map_err(|error| format!("{}: {}", path.display(), error.name));
    let load_prefix = |path: &Path| VersionedPrefixModel::load(path).map(Arc::new).map_err(|error| format!("{}: {}", path.display(), error.name));
    let (prefix_candidate, prefix_baseline) = (load_prefix(&pairs.prefix_candidate)?, load_prefix(&pairs.prefix_baseline)?);
    let indexes = [prefix_indexes(first), prefix_indexes(spelling)];
    let mut tasks: Vec<(usize, Mode, &str, &str, &str)> = Vec::new();
    for profile in 0..PROFILES.len() {
        for mode in [Mode::EarlyOff, Mode::Default] {
            let listed = PAIRS.iter().find(|entry| entry.0 == mode).expect("a mode").1;
            for (name, context, prefix) in listed {
                tasks.push((profile, mode, name, context, prefix));
            }
        }
    }
    let next = AtomicUsize::new(0);
    let done: Mutex<Vec<Option<(Counts, Vec<Value>)>>> = Mutex::new((0..tasks.len()).map(|_| None).collect());
    let failure: Mutex<Option<String>> = Mutex::new(None);
    std::thread::scope(|scope| {
        for worker in 0..jobs.max(1).min(tasks.len()) {
            let (tasks, next, done, failure, plans, shared, indexes, prefix_candidate, prefix_baseline, load_context) = (&tasks, &next, &done, &failure, &plans, &shared, &indexes, &prefix_candidate, &prefix_baseline, &load_context);
            scope.spawn(move || {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some((profile, mode, _name, context, prefix)) = tasks.get(index).copied() else { break };
                    let path = if context == "candidate" { &pairs.candidate } else { &pairs.baseline };
                    let model = match load_context(path) {
                        Ok(model) => model,
                        Err(error) => {
                            *failure.lock().expect("a worker finished") = Some(error);
                            break;
                        }
                    };
                    let prefix_model = if prefix == "candidate" { prefix_candidate.clone() } else { prefix_baseline.clone() };
                    let models = if profile == 0 { first } else { spelling };
                    let folder = std::env::temp_dir().join(format!("ks-evaluate-{}-{worker}", std::process::id()));
                    let mut replayer = Replayer::new(tools, models, shared.clone(), folder.clone(), model, Some(prefix_model), indexes[profile].clone());
                    let scored = score_pair(plans, &mut replayer, mode);
                    let _ = std::fs::remove_dir_all(folder);
                    done.lock().expect("a worker finished")[index] = Some(scored);
                }
            });
        }
    });
    if let Some(error) = failure.into_inner().expect("the workers finished") {
        return Err(error);
    }
    let mut scored = done.into_inner().expect("the workers finished").into_iter();
    let mut profiles = Map::new();
    let mut all_passed = true;
    for profile in PROFILES {
        let mut blocks: BTreeMap<&str, Value> = BTreeMap::new();
        for (mode, listed) in PAIRS {
            let mut totals: BTreeMap<&str, Counts> = BTreeMap::new();
            let mut cases: BTreeMap<&str, Vec<Value>> = BTreeMap::new();
            for (name, _, _) in listed {
                let (counts, results) = scored.next().flatten().ok_or("a task was not scored")?;
                totals.insert(name, counts);
                cases.insert(name, results);
            }
            let gates = profile_gates(&totals["candidate"], &totals["baseline"], &documents);
            all_passed = all_passed && gates.as_object().is_some_and(|table| table.values().all(|value| value == &json!(true)));
            let mut block = json!({"counts": {"baseline": totals["baseline"].report(), "candidate": totals["candidate"].report()},
                "cases": {"baseline": cases["baseline"], "candidate": cases["candidate"]}, "gates": gates});
            let mut ablation = Map::new();
            for (name, counts) in &totals {
                if *name != "baseline" && *name != "candidate" {
                    ablation.insert((*name).to_owned(), json!({"counts": counts.report(), "cases": cases[name]}));
                }
            }
            if !ablation.is_empty() {
                block["ablation"] = Value::Object(ablation);
            }
            blocks.insert(mode.as_str(), block);
        }
        let mut block = blocks.remove("default").ok_or("the default mode")?;
        block["early_off"] = blocks.remove("early_off").ok_or("the early-off mode")?;
        profiles.insert(profile.to_owned(), block);
    }
    let _ = root;
    Ok(json!({"selection": selection, "profiles": profiles, "promotion_passed": all_passed}))
}

fn hash_list(value: &Value, name: &str) -> Result<Vec<String>, String> {
    let items = value.as_array().filter(|items| !items.is_empty()).ok_or_else(|| format!("invalid test membership: {name}"))?;
    let mut entries = Vec::new();
    for item in items {
        let text = item.as_str().filter(|text| text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))).ok_or_else(|| format!("invalid test membership: {name}"))?;
        entries.push(text.to_owned());
    }
    if entries.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err("test membership must be sorted and unique".to_owned());
    }
    Ok(entries)
}

/// `test_membership`.
fn test_membership(corpus: &Path) -> Result<Value, String> {
    let manifest = read_json(&corpus.join("manifest.json"))?;
    let path = corpus.join("test-membership.json");
    if manifest.get("test_membership_sha256") != Some(&Value::String(checksum(&path)?)) {
        return Err("test membership differs from manifest".to_owned());
    }
    let membership = read_json(&path)?;
    let mut result = Map::new();
    for field in MEMBERSHIP_IDENTITY_FIELDS.iter().copied().chain(membership.get("alias_sha256").map(|_| "alias_sha256")) {
        result.insert(field.to_owned(), json!(hash_list(&membership[field], field)?));
    }
    Ok(Value::Object(result))
}

/// What must not change between the seal and the end of a replay.
pub struct Identity {
    pub value: Value,
    pub key: String,
}

pub fn evaluation_identity(request: &EvaluationRequest<'_>) -> Result<Identity, String> {
    let root = request.root;
    let membership = test_membership(request.corpus)?;
    let key = hex(&Sha256::digest(crate::artifact::canonical(&membership)));
    let runtime = crate::fit::provenance(root, &root.join("model/context_v3/recipe.json"))?;
    let value = json!({"schema_version": 1, "artifact_sha256": checksum(request.artifact)?, "candidate_seal_sha256": checksum(request.seal)?, "corpus_manifest_sha256": checksum(&request.corpus.join("manifest.json"))?,
        "test_membership": membership, "baseline_sha256": checksum(&root.join(BASELINE))?, "prefix_candidate_sha256": checksum(request.prefix)?, "prefix_candidate_seal_sha256": checksum(request.prefix_seal)?,
        "prefix_baseline_path": PREFIX_BASELINE, "prefix_baseline_sha256": checksum(&root.join(PREFIX_BASELINE))?, "settings_modes": ["early_off", "default"], "runtime": runtime, "protocol": protocol(root)?});
    Ok(Identity { value, key })
}

/// The protocol a report states, in the words of the audited protocol, whose bytes its hash pins.
pub fn protocol(root: &Path) -> Result<Value, String> {
    let protocols = read_json(&root.join("model/context_v3/protocols.json"))?;
    let sequence = protocols["sequence"].clone();
    if protocols["audited_sequence_protocol_sha256"].as_str() != Some(hex(&Sha256::digest(crate::artifact::canonical(&sequence))).as_str()) {
        return Err("sequence protocol changed; public receipt protocol must be reviewed".to_owned());
    }
    Ok(sequence)
}

/// `immutable_record`: create a record, or verify the exact existing bytes.
fn immutable_record(path: &Path, value: &Value) -> Result<(), String> {
    let encoded = crate::artifact::canonical(value);
    if path.exists() {
        if std::fs::read(path).map_err(|error| error.to_string())? != encoded {
            return Err(format!("immutable test record differs: {}", path.display()));
        }
        return Ok(());
    }
    std::fs::create_dir_all(path.parent().ok_or("a record outside a folder")?).map_err(|error| error.to_string())?;
    let pending = path.with_extension("pending");
    std::fs::write(&pending, &encoded).map_err(|error| error.to_string())?;
    let linked = std::fs::hard_link(&pending, path);
    let _ = std::fs::remove_file(&pending);
    linked.map_err(|error| error.to_string())
}

/// `claim_test_access`: a test is read once, and never when it shares anything with a test read before.
fn claim_test_access(ledger: &Path, identity: &Identity) -> Result<(), String> {
    std::fs::create_dir_all(ledger).map_err(|error| error.to_string())?;
    let lock = ledger.join(".access-lock");
    std::fs::OpenOptions::new().write(true).create_new(true).open(&lock).map_err(|_| "test access ledger is locked; no test was read".to_owned())?;
    let claimed = (|| -> Result<(), String> {
        let membership = &identity.value["test_membership"];
        let mine = format!("{}.access.json", identity.key);
        let mut paths: Vec<PathBuf> = std::fs::read_dir(ledger).map_err(|error| error.to_string())?.filter_map(|entry| entry.ok().map(|entry| entry.path())).collect();
        paths.sort();
        for path in paths {
            let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
            if !name.ends_with(".access.json") || name == mine {
                continue;
            }
            let record = read_json(&path)?;
            for field in MEMBERSHIP_IDENTITY_FIELDS {
                let earlier: BTreeSet<&str> = record["test_membership"][field].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                if membership[field].as_array().into_iter().flatten().filter_map(Value::as_str).any(|entry| earlier.contains(entry)) {
                    return Err("test overlaps previously accessed rows, families or documents".to_owned());
                }
            }
        }
        immutable_record(&ledger.join(mine), &identity.value)
    })();
    let _ = std::fs::remove_file(&lock);
    claimed
}

/// The calibration a candidate carries must show a balance, not an absence of error (`validate_calibration`).
fn validate_calibration(value: &Value) -> Result<(), String> {
    let profiles = value.get("by_profile").and_then(Value::as_object).ok_or("calibration must contain both declared lexical profiles")?;
    if profiles.len() != PROFILES.len() || PROFILES.iter().any(|name| !profiles.contains_key(*name)) {
        return Err("calibration must contain both declared lexical profiles".to_owned());
    }
    let count = |row: &Value, field: &str| row.get(field).and_then(Value::as_i64).filter(|number| *number >= 0).ok_or_else(|| format!("calibration.{field} must be a nonnegative integer"));
    let mut sums = [0_i64; 4];
    let fields = ["rows", "false_conversions", "converted_correctly", "convert_rows"];
    for (name, row) in std::iter::once(("aggregate", value)).chain(PROFILES.iter().map(|name| (*name, &profiles[*name]))) {
        let numbers: Vec<i64> = fields.iter().map(|field| count(row, field)).collect::<Result<_, _>>()?;
        let (rows, wrong, right, possible) = (numbers[0], numbers[1], numbers[2], numbers[3]);
        let recall = row.get("conversion_recall").and_then(Value::as_f64).unwrap_or(f64::NAN);
        if possible == 0 || rows < possible || right > possible || recall != right.to::<f64>() / possible.to::<f64>() || wrong > right || right - wrong < 1 {
            return Err(format!("calibration gate failed: {name}"));
        }
        if name == "aggregate" {
            sums = [0; 4];
        } else {
            for (sum, number) in sums.iter_mut().zip(&numbers) {
                *sum += number;
            }
        }
    }
    for (field, sum) in fields.iter().zip(sums) {
        if count(value, field)? != sum {
            return Err("calibration aggregate does not match its profiles".to_owned());
        }
    }
    Ok(())
}

fn check_pins(root: &Path, hashes: &Value, label: &str) -> Result<(), String> {
    for (name, expected) in hashes.as_object().ok_or("the seal names no provenance")? {
        let path = Path::new(name);
        if path.is_absolute() || name.contains("..") {
            return Err(format!("{label} provenance must use repository-relative paths"));
        }
        if Some(checksum(&root.join(path))?.as_str()) != expected.as_str() {
            return Err(format!("{label} provenance changed: {name}"));
        }
    }
    Ok(())
}

/// `validate_candidate_seal`: done before any test byte is read.
pub fn validate_candidate_seal(root: &Path, artifact: &Path, seal_path: &Path, corpus: &Path, models_dir: &Path) -> Result<Value, String> {
    let seal = read_json(seal_path)?;
    if seal.get("schema_version").and_then(Value::as_i64) != Some(1) {
        return Err("unsupported candidate seal".to_owned());
    }
    if seal.get("stage").and_then(Value::as_str) != Some(SEALED_BEFORE_TEST) || seal.get("test_accessed").and_then(Value::as_bool) != Some(false) {
        return Err("candidate is not sealed before test".to_owned());
    }
    if seal.get("artifact_sha256") != Some(&Value::String(checksum(artifact)?)) {
        return Err("candidate artifact differs from seal".to_owned());
    }
    if seal.get("corpus_manifest_sha256") != Some(&Value::String(checksum(&corpus.join("manifest.json"))?)) {
        return Err("corpus differs from candidate seal".to_owned());
    }
    let recipe = read_json(&root.join("model/context_v3/recipe.json"))?;
    if crate::artifact::canonical(&seal["recipe"]) != crate::artifact::canonical(&recipe) {
        return Err("recipe differs from candidate seal".to_owned());
    }
    if crate::artifact::canonical(&seal["gate_policy"]) != crate::artifact::canonical(&recipe["gate_policy"]) || recipe["profiles"] != json!(PROFILES) {
        return Err("candidate gate policy differs from required policy".to_owned());
    }
    check_pins(root, &seal["provenance"], "candidate")?;
    validate_calibration(&seal["calibration"])?;
    let model = ContextModel::load_artifact(artifact, models_dir).map_err(|error| error.name.to_string())?;
    if seal.get("model_version").and_then(Value::as_str) != Some(model.version.as_str()) || seal.get("conversion_threshold").and_then(Value::as_f64) != Some(model.conversion_threshold) {
        return Err("candidate model identity differs from seal".to_owned());
    }
    Ok(seal)
}

/// `validate_prefix_seal`: calibration is mandatory before a sealed test.
pub fn validate_prefix_seal(root: &Path, artifact: &Path, seal_path: &Path, require_calibration: bool) -> Result<Value, String> {
    let seal = read_json(seal_path)?;
    if seal.get("schema_version").and_then(Value::as_i64) != Some(1) {
        return Err("unsupported prefix seal".to_owned());
    }
    if seal.get("promotion_accepted").and_then(Value::as_bool) != Some(false) || seal.get("independent_test_evaluated").and_then(Value::as_bool) != Some(false) {
        return Err("prefix seal must not claim promotion or an evaluated test".to_owned());
    }
    if require_calibration && seal.get("calibration_passed").and_then(Value::as_bool) != Some(true) {
        return Err("prefix candidate did not pass calibration".to_owned());
    }
    if seal.get("candidate_sha256") != Some(&Value::String(checksum(artifact)?)) {
        return Err("prefix artifact differs from seal".to_owned());
    }
    check_pins(root, &seal["provenance"], "prefix")?;
    let model = VersionedPrefixModel::load(artifact).map_err(|error| error.name.to_string())?;
    if seal.get("model_version").and_then(Value::as_str) != Some(model.version()) || seal.get("threshold").and_then(Value::as_f64) != Some(model.model.conversion_threshold) {
        return Err("prefix model identity differs from seal".to_owned());
    }
    Ok(seal)
}

pub struct EvaluationRequest<'a> {
    pub root: &'a Path,
    pub resource_dir: &'a Path,
    pub artifact: &'a Path,
    pub seal: &'a Path,
    pub prefix: &'a Path,
    pub prefix_seal: &'a Path,
    pub corpus: &'a Path,
    pub split: &'a str,
    pub output: &'a Path,
    pub ledger: &'a Path,
    pub jobs: usize,
    /// A development diagnostic of models that were not fitted by this tree: the seals are not read.
    pub unsealed: bool,
}

/// `evaluate`: the report of a development diagnostic, or of the sealed test, read once.
pub fn evaluate(request: &EvaluationRequest<'_>) -> Result<Value, String> {
    let split = request.split;
    if split != DEVELOPMENT && split != "test" {
        return Err("sequence evaluator accepts development or sealed test only".to_owned());
    }
    if request.unsealed && split == "test" {
        return Err("a sealed test needs seals".to_owned());
    }
    let (root, models_dir) = (request.root, request.resource_dir.join("models"));
    if !request.unsealed {
        validate_candidate_seal(root, request.artifact, request.seal, request.corpus, &models_dir)?;
        validate_prefix_seal(root, request.prefix, request.prefix_seal, split == "test")?;
    }
    let identity = if request.unsealed {
        Identity { value: json!({"unsealed": true}), key: String::new() }
    } else {
        evaluation_identity(request)?
    };
    if split == "test" {
        claim_test_access(request.ledger, &identity)?;
    }
    // No test file, decompression, model scoring or row selection occurs above the access claim.
    let resources: Arc<Resources> = Resources::with_speller(request.resource_dir.to_path_buf(), ks_hunspell::hunspell_speller(ks_config::history::data_dir()));
    let portable = reference_models(root, &resources, false)?;
    let spelling = reference_models(root, &resources, true)?;
    let tools = Tools::new(root, resources.clone(), portable, BTreeSet::new())?;
    let rows = load_rows(request.corpus, split)?;
    let pairs = Pairs { candidate: request.artifact.to_path_buf(), baseline: root.join(BASELINE), prefix_candidate: request.prefix.to_path_buf(), prefix_baseline: root.join(PREFIX_BASELINE) };
    let gate_policy = read_json(&root.join("model/context_v3/recipe.json"))?["gate_policy"].clone();
    let mut report = json!({"schema_version": 1, "split": split, "identity": identity.value, "protocol": protocol(root)?, "gate_policy": gate_policy, "promotion_passed": false});
    let scored = score_sequences(root, &resources, &tools, &spelling, &rows, &pairs, request.jobs);
    let write_report = |report: &Value| -> Result<(), String> {
        std::fs::create_dir_all(request.output.parent().ok_or("a report outside a folder")?).map_err(|error| error.to_string())?;
        std::fs::write(request.output, crate::artifact::canonical(report)).map_err(|error| error.to_string())
    };
    match scored {
        Ok(Value::Object(table)) => {
            if let Some(object) = report.as_object_mut() {
                object.extend(table);
            }
        }
        Ok(_) => return Err("a scored report is an object".to_owned()),
        Err(error) => {
            report["execution_error"] = json!(error);
            report["promotion_passed"] = json!(false);
            if split == "test" {
                immutable_record(&request.ledger.join(format!("{}.outcome.json", identity.key)), &report)?;
            }
            write_report(&report)?;
            return Err(error);
        }
    }
    if !request.unsealed && evaluation_identity(request)?.value != identity.value {
        return Err("evaluation inputs changed during sequence replay".to_owned());
    }
    if split == "test" {
        immutable_record(&request.ledger.join(format!("{}.outcome.json", identity.key)), &report)?;
    } else {
        report["promotion_passed"] = json!(false);
        report["scope"] = json!("development diagnostic; cannot promote a model");
    }
    write_report(&report)?;
    Ok(report)
}
