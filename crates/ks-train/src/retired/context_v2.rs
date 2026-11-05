//! Context-v2, rejected by its sealed test. A replay repeats the fit and the four numeric tracks of the test on the frozen
//! lexical evidence and compares candidate, seal and report byte for byte; it is a regression replay, never a new
//! evaluation, and nothing here may install the candidate.
use ks_cast::To as _;
use super::context_frames::{Frame, TECHNICAL_KEEP, all_frames, frames_sha256};
use super::read_object;
use super::sources::CONTEXT_V2;
use crate::artifact::canonical;
use crate::corpus::{checksum, read_json};
use crate::kernel::{self, Packed};
use crate::physical::{hex, hex_digest};
use flate2::read::MultiGzDecoder;
use ks_context::context_action_features::extract_baseline_features;
use ks_context::context_model::{ACTIONS, ContextAction, ContextEvidence, ContextModel, FieldContext};
use ks_context::input_context::FieldRole;
use ks_text::pyjson::json_string;
use ks_text::pynum::py_round_ndigits;
use ks_text::{JsonOptions, json_dumps};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Read;
use std::path::Path;

const DIRECTORY: &str = "model/context_v2";
const PROFILES: [&str; 2] = ["portable", "reference_hunspell"];
const SEALED_BEFORE_TEST: &str = "sealed-before-test";
const CORPUS_REVISION: i64 = 2;
const MAX_REPORTED_FALSE_CONVERSIONS: usize = 12;
const CHOICE_DIGITS: usize = 8;
const ROUNDING_DECIMALS: i64 = 9;
const VERSION_HASH_CHARACTERS: usize = 12;
const WIDTH: usize = ACTIONS.len();
const PROVENANCE: [&str; 15] = [
    "model/context_v2/config.json",
    "model/context_v2/baseline-context-v1.json",
    "model/context_v2/sources/lexical-evidence.json.gz",
    "model/context_v2/lexical-receipt.json",
    "model/context_v2/corpus-receipt.json",
    "model/context_v2/sources/tatoeba-cc0-en-ru.tsv.gz",
    "model/context_v2/safety.json",
    "tools/context_corpus.py",
    "tools/context_frames.py",
    "tools/context_evidence.py",
    "tools/train_context_v2.py",
    "tools/context_optimizer.py",
    "tools/context_optimizer.c",
    "src/keyswitch/context_model.py",
    "src/keyswitch/resources/models/layout_intent_v1.ksm",
];
const ANCHORS: [(&str, &str); 7] = [
    ("candidate.json", "55f0d735de8751f3f414d569b6e1eb2bbb294303d8fe01f2fde66c3fadecfeb1"),
    ("candidate-seal.json", "03878d1d8fcc0a79295b5f63acfa06f097a2b99931a1f98eb6060c7d0370a53a"),
    ("report.json", "be7ff355ddcbe0cb84e0fdbb53a62a6bafdbeb2509d589ded16ec1c82c38918e"),
    ("engine-report.json", "1849b11451ceda5b64da8a24eb37c7b6532d838273521ece25dab92afef618cb"),
    ("corpus-receipt.json", "cf88d7fe395d09ff6fafa8b3950cd18e1e7a07d781af9199e23e15fcff57cd5b"),
    ("lexical-receipt.json", "f0b89d49b707ac7163207ed273de0eb9d45fa04da2fb6735b430c18fcd058a94"),
    ("baseline-context-v1.json", "80a1b8c97a095413298e4287763bb675ea8dd9218dc6ca78d371ae7a1b9efd93"),
];
const SCOPE: &str = "Historical rejected context-v2 evidence and feature-2 numeric compatibility; no new fit, new holdout, native execution or current-engine acceptance.";
const REPORT_SCOPE: &str = "public CC0 phrases with synthetic layout/spelling interventions and authored safety cases; not observed user intent or full-engine quality";
const LEXICAL_SCOPE: &str = "unseen supervised focus family; surrounding words and external lexicons can contain it";

type EvidenceValues = (bool, bool, bool, f64);
type Cache = HashMap<String, EvidenceValues>;

/// `load_cache`: the frozen lexical inputs of both dictionary profiles.
fn load_cache(root: &Path) -> Result<Cache, String> {
    let directory = root.join(DIRECTORY);
    let (path, receipt) = (directory.join("sources/lexical-evidence.json.gz"), read_object(&directory.join("lexical-receipt.json"))?);
    if receipt.get("sha256").and_then(Value::as_str) != Some(checksum(&path)?.as_str()) {
        return Err("lexical cache checksum mismatch".to_owned());
    }
    let mut raw = Vec::new();
    MultiGzDecoder::new(std::fs::File::open(&path).map_err(|error| error.to_string())?).read_to_end(&mut raw).map_err(|error| error.to_string())?;
    serde_json::from_slice(&raw).map_err(|error| format!("invalid lexical cache: {error}"))
}

fn key(row: &Frame, profile: &str) -> String {
    format!("[{},{},{},{}]", json_string(profile, false), json_string(&row.original, false), row.group, json_string(row.trigger, false))
}

fn evidence(row: &Frame, profile: &str, cache: &Cache) -> Result<ContextEvidence, String> {
    let (baseline, source, target, delta) = *cache.get(&key(row, profile)).ok_or("a frame without lexical evidence")?;
    let mut field = FieldContext::new(row.application, "public-corpus");
    field.before.clone_from(&row.before);
    field.after.clone_from(&row.after);
    field.role = if row.role == "text" { FieldRole::Text } else { FieldRole::Unknown };
    let mut item = ContextEvidence::new(&row.original, &row.alternative, row.group, field);
    row.trigger.clone_into(&mut item.trigger);
    item.baseline_convert = baseline;
    item.source_known = source;
    item.target_known = target;
    item.score_delta = delta;
    Ok(item.finished())
}

fn count_by<'a>(values: impl Iterator<Item = &'a str>) -> BTreeMap<&'a str, i64> {
    let mut counts = BTreeMap::new();
    for value in values {
        *counts.entry(value).or_insert(0) += 1;
    }
    counts
}

fn audit(frames: &[Frame]) -> Result<Value, String> {
    let partitions = ["train", "development", "calibration", "test", "lexical_test"];
    let of = |split: &str| frames.iter().filter(|row| row.split == split).map(|row| row.cluster.as_str()).collect::<BTreeSet<&str>>();
    let groups: BTreeMap<&str, BTreeSet<&str>> = partitions.iter().map(|split| (*split, of(split))).collect();
    let disjoint = |left: &str, right: &str| groups[left].is_disjoint(&groups[right]);
    let leak = ["train", "development", "calibration"].iter().any(|fitted| !disjoint(fitted, "test") || !disjoint(fitted, "lexical_test"))
        || !disjoint("train", "development")
        || !disjoint("train", "calibration")
        || !disjoint("development", "calibration");
    if leak {
        return Err("source-group leakage".to_owned());
    }
    let fit_families: BTreeSet<&str> = frames.iter().filter(|row| matches!(row.split, "train" | "development" | "calibration")).map(|row| row.focus_family.as_str()).collect();
    if frames.iter().any(|row| row.split == "lexical_test" && fit_families.contains(row.focus_family.as_str())) {
        return Err("focus-family leakage".to_owned());
    }
    let groups_by_split: BTreeMap<&str, usize> = groups.iter().map(|(split, value)| (*split, value.len())).collect();
    Ok(json!({
        "rows": frames.len(), "rows_by_split": count_by(frames.iter().map(|row| row.split)),
        "groups_by_split": groups_by_split, "actions": count_by(frames.iter().map(|row| row.action)),
        "source_group_overlap": 0, "focus_lexical_overlap": 0, "lexical_scope": LEXICAL_SCOPE,
        "frames_sha256": frames_sha256(frames),
    }))
}

fn samples<'a>(frames: &'a [Frame], split: &str) -> Vec<(&'a Frame, &'static str)> {
    let mut items = Vec::new();
    for row in frames.iter().filter(|row| row.split == split) {
        if split == "train" {
            let draw = u64::from_str_radix(hex_digest(&row.identifier).get(..CHOICE_DIGITS).unwrap_or_default(), 16).expect("a digest");
            items.push((row, PROFILES[(draw % PROFILES.len().to::<u64>()).to::<usize>()]));
        } else {
            items.extend(PROFILES.iter().map(|profile| (row, *profile)));
        }
    }
    items.sort_by_cached_key(|(row, profile)| hex_digest(&format!("optimizer-order:{}:{profile}", row.identifier)));
    items
}

fn label_of(row: &Frame) -> Result<u8, String> {
    ACTIONS.iter().position(|action| action.as_str() == row.action).map(ks_cast::To::to::<u8>).ok_or_else(|| "an unknown action".to_owned())
}

fn pack(items: &[(&Frame, &str)], names: &[String], cache: &Cache, importance: &[f64; WIDTH], technical: f64) -> Result<Packed, String> {
    let vocabulary: HashMap<&str, u32> = names.iter().enumerate().map(|(index, name)| (name.as_str(), index.to::<u32>())).collect();
    let mut packed = Packed { offsets: vec![0], ..Packed::default() };
    for (row, profile) in items {
        let label = label_of(row)?;
        let multiplier = if row.category == TECHNICAL_KEEP { technical } else { 1.0 };
        let weight = importance[usize::from(label)] * multiplier;
        if !weight.is_finite() || weight <= 0.0 {
            return Err("invalid training target or importance".to_owned());
        }
        for (name, value) in extract_baseline_features(&evidence(row, profile, cache)?) {
            if !value.is_finite() {
                return Err("non-finite feature".to_owned());
            }
            if let Some(index) = vocabulary.get(name.as_str()) {
                packed.indices.push(*index);
                packed.values.push(value);
            }
        }
        packed.offsets.push(packed.indices.len().to::<u64>());
        packed.labels.push(label);
        packed.importance.push(weight);
    }
    Ok(packed)
}

fn select_threshold(predictions: &[f64], labels: &[u8], candidates: &[f64], maximum_false: i64) -> Result<(f64, Value), String> {
    let convert = 1;
    let mut sorted = candidates.to_vec();
    sorted.sort_by(f64::total_cmp);
    for threshold in sorted {
        let (mut false_conversions, mut true_conversions) = (0_i64, 0_i64);
        for (row, label) in labels.iter().enumerate() {
            let probabilities = &predictions[row * WIDTH..(row + 1) * WIDTH];
            let selected = (0..WIDTH).fold(0, |best, index| if probabilities[index] > probabilities[best] { index } else { best });
            let converted = selected == convert && probabilities[convert] >= threshold;
            false_conversions += i64::from(converted && usize::from(*label) != convert);
            true_conversions += i64::from(converted && usize::from(*label) == convert);
        }
        if false_conversions <= maximum_false {
            return Ok((threshold, json!({"rows": labels.len(), "false_conversions": false_conversions, "converted_correctly": true_conversions})));
        }
    }
    Err("no calibration threshold meets the declared safety budget".to_owned())
}

fn config(root: &Path) -> Result<Value, String> {
    let value = read_json(&root.join(DIRECTORY).join("config.json"))?;
    if value.get("schema_version").and_then(Value::as_i64) != Some(1) || value.get("feature_version").and_then(Value::as_i64) != Some(2) {
        return Err("incompatible experiment configuration".to_owned());
    }
    Ok(value)
}

/// `fit`, with the seal's pins standing in for the live source hashes. Returns the artifact and the seal.
fn fit(root: &Path, frames: &[Frame], cache: &Cache, pins: &Map<String, Value>) -> Result<(Vec<u8>, Vec<u8>), String> {
    let options = config(root)?;
    let number = |name: &str| options[name].as_f64().ok_or_else(|| format!("the config has no {name}"));
    let rows = samples(frames, "train");
    let mut counts = [0_usize; WIDTH];
    for (row, _) in &rows {
        counts[usize::from(label_of(row)?)] += 1;
    }
    if counts.contains(&0) {
        return Err("training needs all four actions".to_owned());
    }
    let mut importance = [0.0; WIDTH];
    for label in 0..WIDTH {
        importance[label] = rows.len().to::<f64>() / (WIDTH * counts[label]).to::<f64>() * if label == 0 { number("keep_importance")? } else { 1.0 };
    }
    let technical = number("technical_importance")?;
    let mut frequencies: HashMap<String, i64> = HashMap::new();
    for (row, profile) in &rows {
        for name in extract_baseline_features(&evidence(row, profile, cache)?).into_keys() {
            *frequencies.entry(name).or_insert(0) += 1;
        }
    }
    let minimum = (number("minimum_feature_rows")?).to::<i64>();
    let mut kept: Vec<(i64, String)> = frequencies.into_iter().filter(|(_, count)| *count >= minimum).map(|(name, count)| (-count, name)).collect();
    kept.sort();
    kept.truncate((number("maximum_features")?).to::<usize>());
    let mut names: Vec<String> = kept.into_iter().map(|(_, name)| name).collect();
    names.sort();
    let train = pack(&rows, &names, cache, &importance, technical)?;
    let development = pack(&samples(frames, "development"), &names, cache, &importance, technical)?;
    let calibration = pack(&samples(frames, "calibration"), &names, cache, &importance, technical)?;
    if development.rows() == 0 || calibration.rows() == 0 {
        return Err("empty development or calibration split".to_owned());
    }
    println!("packed: train={}, development={}, calibration={}, features={}", train.rows(), development.rows(), calibration.rows(), names.len());
    let (mut weights, mut accumulators) = (vec![0.0_f64; names.len() * WIDTH], vec![1.0_f64; names.len() * WIDTH]);
    let (mut best, mut best_epoch, mut best_loss) = (Vec::new(), 0_i64, f64::INFINITY);
    let rate = number("learning_rate")?;
    for epoch in 0..(number("epochs")?).to::<i64>() {
        kernel::epoch(&train, &mut weights, &mut accumulators, rate);
        let loss = kernel::log_loss(&kernel::predict(&development, &weights), &development) / development.rows().to::<f64>();
        if loss < best_loss {
            best = weights.iter().map(|value| py_round_ndigits(*value, ROUNDING_DECIMALS).expect("a finite weight")).collect();
            best_epoch = epoch + 1;
            best_loss = loss;
        }
    }
    let candidates: Vec<f64> = options["threshold_candidates"].as_array().ok_or("no threshold candidates")?.iter().filter_map(Value::as_f64).collect();
    let (threshold, calibration_metrics) = select_threshold(&kernel::predict(&calibration, &best), &calibration.labels, &candidates, (number("calibration_max_false_conversions")?).to::<i64>())?;
    let mapping: Map<String, Value> = names.iter().enumerate().map(|(index, name)| (name.clone(), json!(best[index * WIDTH..(index + 1) * WIDTH]))).collect();
    let mapping = Value::Object(mapping);
    let compact = JsonOptions { ensure_ascii: false, separators: Some((",".to_owned(), ":".to_owned())), sort_keys: true, ..JsonOptions::default() };
    let weight_hash = hex(&Sha256::digest(json_dumps(&mapping, &compact).as_bytes()));
    let version = format!("context-v1-{}", weight_hash.get(..VERSION_HASH_CHARACTERS).unwrap_or_default());
    let actions: Vec<&str> = ACTIONS.iter().map(|action| action.as_str()).collect();
    let artifact = canonical(&json!({"actions": actions, "feature_version": 2, "weights": mapping, "weights_sha256": weight_hash, "version": version, "conversion_threshold": threshold}));
    let seal = json!({
        "schema_version": 1, "stage": SEALED_BEFORE_TEST, "corpus_revision": CORPUS_REVISION, "selected_epoch": best_epoch,
        "development_loss": py_round_ndigits(best_loss, ROUNDING_DECIMALS).expect("a finite loss"),
        "calibration": calibration_metrics, "conversion_threshold": threshold, "feature_count": names.len(),
        "audit": audit(frames)?, "provenance": pins, "artifact_sha256": hex(&Sha256::digest(&artifact)), "model_version": version,
    });
    Ok((artifact, canonical(&seal)))
}

#[derive(Default)]
struct Tally(BTreeMap<&'static str, i64>);

impl Tally {
    fn add(&mut self, updates: &[(&'static str, bool)]) {
        for (name, value) in updates {
            *self.0.entry(name).or_insert(0) += i64::from(*value);
        }
    }
}

fn metrics(model: &ContextModel, frames: &[&Frame], profile: &str, cache: &Cache) -> Result<Value, String> {
    let (mut counts, mut categories, mut examples) = (Tally::default(), BTreeMap::<&str, Tally>::new(), Vec::new());
    for row in frames {
        let item = evidence(row, profile, cache)?;
        let prediction = model.predict(&item);
        let converts = prediction.action == ContextAction::Convert;
        let effective = if !prediction.supported && matches!(prediction.action, ContextAction::Keep | ContextAction::Convert) { item.baseline_convert } else { converts };
        let desired = row.action == "convert";
        let updates = [
            ("rows", true),
            ("desired_conversions", desired),
            ("correct_actions", prediction.action.as_str() == row.action),
            ("converted_correctly", effective && desired),
            ("false_conversions", effective && !desired),
            ("raw_converted_correctly", converts && desired),
            ("raw_false_conversions", converts && !desired),
            ("baseline_converted_correctly", item.baseline_convert && desired),
            ("baseline_false_conversions", item.baseline_convert && !desired),
        ];
        counts.add(&updates);
        categories.entry(row.category).or_default().add(&updates);
        if effective && !desired && examples.len() < MAX_REPORTED_FALSE_CONVERSIONS {
            examples.push(json!({"source_id": row.identifier, "original": row.original, "alternative": row.alternative, "expected": row.action, "actual": prediction.action.as_str()}));
        }
    }
    let categories: BTreeMap<&str, &BTreeMap<&str, i64>> = categories.iter().map(|(name, tally)| (*name, &tally.0)).collect();
    Ok(json!({"counts": counts.0, "categories": categories, "false_conversion_examples": examples}))
}

fn promotion_failures(candidate: &Value, baseline: &Value, gate: &Value) -> Vec<&'static str> {
    let count = |value: &Value, name: &str| value["counts"][name].as_i64().unwrap_or(0);
    let (rows, desired) = (count(candidate, "rows"), count(candidate, "desired_conversions"));
    let negatives = rows - desired;
    if negatives == 0 || desired == 0 {
        return vec!["empty evaluation class"];
    }
    let mut failures = Vec::new();
    if (count(candidate, "converted_correctly").to::<f64>() / desired.to::<f64>()) < gate["minimum_recall"].as_f64().unwrap_or(1.0) {
        failures.push("recall below declared minimum");
    }
    if count(candidate, "false_conversions").to::<f64>() / negatives.to::<f64>() > gate["maximum_false_conversion_rate"].as_f64().unwrap_or(0.0) {
        failures.push("false conversion rate above budget");
    }
    if count(candidate, "false_conversions") > count(baseline, "false_conversions") {
        failures.push("more false conversions than v1");
    }
    if count(candidate, "converted_correctly") < count(baseline, "converted_correctly") {
        failures.push("fewer correct conversions than v1");
    }
    if count(candidate, "false_conversions") > count(candidate, "baseline_false_conversions") {
        failures.push("more false conversions than detector");
    }
    if candidate["categories"][TECHNICAL_KEEP]["false_conversions"].as_i64().unwrap_or(0) > gate["maximum_technical_false_conversions"].as_i64().unwrap_or(0) {
        failures.push("technical text converted");
    }
    failures
}

fn load_model(root: &Path, name: &str) -> Result<ContextModel, String> {
    ContextModel::load_artifact(&root.join(DIRECTORY).join(name), &root.join("resources/models")).map_err(|error| format!("{name}: {error}"))
}

/// The report's four numeric tracks.
fn evaluate(root: &Path, frames: &[Frame], cache: &Cache) -> Result<Vec<u8>, String> {
    let directory = root.join(DIRECTORY);
    let expected = read_object(&directory.join("report.json"))?;
    let (candidate, baseline) = (load_model(root, "candidate.json")?, load_model(root, "baseline-context-v1.json")?);
    if candidate.feature_version != 2 || baseline.feature_version != 2 {
        return Err("historical numeric replay requires feature-2 models".to_owned());
    }
    let gate = config(root)?["promotion"].clone();
    let (mut results, mut failures) = (Map::new(), Map::new());
    for split in ["test", "lexical_test"] {
        let selected: Vec<&Frame> = frames.iter().filter(|row| row.split == split).collect();
        for profile in PROFILES {
            let name = format!("{split}:{profile}");
            let current = metrics(&candidate, &selected, profile, cache)?;
            let previous = metrics(&baseline, &selected, profile, cache)?;
            failures.insert(name.clone(), json!(promotion_failures(&current, &previous, &gate)));
            results.insert(name, json!({"candidate": current, "v1": previous}));
        }
    }
    let passed = !failures.values().any(|value| value.as_array().is_some_and(|list| !list.is_empty()));
    let mut report = expected;
    report.insert("audit".to_owned(), audit(frames)?);
    report.insert("results".to_owned(), Value::Object(results));
    report.insert("promotion_failures".to_owned(), Value::Object(failures));
    report.insert("promotion_passed".to_owned(), json!(passed));
    report.insert("scope".to_owned(), json!(REPORT_SCOPE));
    report.insert("threshold_selected_on".to_owned(), json!("calibration only, before test"));
    report.insert("reserve_used".to_owned(), json!(false));
    report.insert("schema_version".to_owned(), json!(1));
    Ok(canonical(&Value::Object(report)))
}

/// The evidence is whole, every pin holds, the rejected candidate is not installed. Reads no phrase.
pub fn verify(root: &Path) -> Result<Value, String> {
    let directory = root.join(DIRECTORY);
    for (name, expected) in ANCHORS {
        if checksum(&directory.join(name))? != expected {
            return Err(format!("historical context-v2 anchor changed: {name}"));
        }
    }
    let seal = read_object(&directory.join("candidate-seal.json"))?;
    let pins = seal.get("provenance").cloned().unwrap_or(Value::Null);
    CONTEXT_V2.recorded(&pins, &PROVENANCE, root).or_else(|_| CONTEXT_V2.verify(&pins, root))?;
    let lexical = read_object(&directory.join("lexical-receipt.json"))?;
    let engine = read_object(&directory.join("engine-report.json"))?;
    let corpus = read_object(&directory.join("corpus-receipt.json"))?;
    CONTEXT_V2.verify(lexical.get("source_hashes").unwrap_or(&Value::Null), root)?;
    CONTEXT_V2.verify(engine.get("provenance").unwrap_or(&Value::Null), root)?;
    CONTEXT_V2.recorded_digest("tools/context_corpus.py", corpus.get("builder_sha256").unwrap_or(&Value::Null), root)?;
    let artifact = checksum(&directory.join("candidate.json"))?;
    if seal.get("stage").and_then(Value::as_str) != Some(SEALED_BEFORE_TEST) || seal.get("artifact_sha256").and_then(Value::as_str) != Some(artifact.as_str()) {
        return Err("candidate seal or provenance changed".to_owned());
    }
    let model = load_model(root, "candidate.json")?;
    if model.feature_version != 2 || seal.get("model_version").and_then(Value::as_str) != Some(model.version.as_str()) || seal.get("conversion_threshold").and_then(Value::as_f64) != Some(model.conversion_threshold) {
        return Err("candidate identity changed".to_owned());
    }
    if let Some(regression) = engine.get("runtime_regression").filter(|value| !value.is_null()) {
        let previous = regression.get("previous_report").and_then(Value::as_str).ok_or("invalid runtime regression history")?;
        let inside = Path::new(previous).parent().is_some_and(|folder| folder == Path::new("model/context_v2/engine-history"));
        if !inside || regression.get("previous_sha256").and_then(Value::as_str) != Some(checksum(&root.join(previous))?.as_str()) {
            return Err("runtime regression history mismatch".to_owned());
        }
        if read_object(&root.join(previous))?.get("source_ids") != engine.get("source_ids") {
            return Err("runtime regression selection changed".to_owned());
        }
    }
    let report = read_object(&directory.join("report.json"))?;
    if report.get("seal_sha256").and_then(Value::as_str) != Some(checksum(&directory.join("candidate-seal.json"))?.as_str())
        || report.get("artifact_sha256").and_then(Value::as_str) != Some(artifact.as_str())
        || report.get("audit") != seal.get("audit")
        || report.get("reserve_used") != Some(&json!(false))
    {
        return Err("candidate evaluation identity changed".to_owned());
    }
    let results = report.get("results").and_then(Value::as_object).ok_or("missing independent evaluation track")?;
    let expected: BTreeSet<String> = ["test", "lexical_test"].iter().flat_map(|split| PROFILES.iter().map(move |profile| format!("{split}:{profile}"))).collect();
    if results.keys().cloned().collect::<BTreeSet<String>>() != expected {
        return Err("missing independent evaluation track".to_owned());
    }
    let gate = config(root)?["promotion"].clone();
    let mut failures = Map::new();
    for (track, value) in results {
        let (current, prior) = (value.get("candidate").ok_or("missing candidate metrics")?, value.get("v1").ok_or("missing candidate metrics")?);
        for metrics in [current, prior] {
            let count = |name: &str| metrics["counts"][name].as_i64().filter(|number| *number >= 0);
            let fields = ["rows", "desired_conversions", "converted_correctly", "false_conversions", "baseline_false_conversions"];
            if fields.iter().any(|field| count(field).is_none()) {
                return Err("invalid candidate counts".to_owned());
            }
            let (rows, desired, correct) = (count("rows").unwrap_or(0), count("desired_conversions").unwrap_or(0), count("converted_correctly").unwrap_or(0));
            if !(0 < desired && desired < rows) || correct > desired {
                return Err("impossible candidate counts".to_owned());
            }
        }
        failures.insert(track.clone(), json!(promotion_failures(current, prior, &gate)));
    }
    let any_failed = failures.values().any(|value| value.as_array().is_some_and(|list| !list.is_empty()));
    if report.get("promotion_failures") != Some(&Value::Object(failures)) || report.get("promotion_passed") != Some(&json!(false)) || !any_failed || engine.get("promotion_passed") != Some(&json!(false)) {
        return Err("candidate promotion result contradicts the metrics".to_owned());
    }
    let active = root.join("resources/models/context_policy_v1.json");
    let active_digest = checksum(&active)?;
    if active_digest == artifact {
        return Err("research candidate must not replace the shipping model".to_owned());
    }
    super::feature_two::verify(root)?;
    Ok(json!({
        "historical_evidence_verified": true, "current_runtime_verified": false, "feature_two_compatible": true, "scope": SCOPE, "generation_ref": "80534ec",
        "schema_version": 1, "candidate": seal["model_version"], "promotion_passed": false, "engine_promotion_passed": false,
        "rejected_candidate_not_active": true, "active_artifact_sha256": active_digest, "artifact_sha256": seal["artifact_sha256"],
        "corpus_rows": seal["audit"]["rows"],
    }))
}

/// Repeats the fit and the test; writes nothing.
pub fn replay(root: &Path) -> Result<Value, String> {
    let mut result = verify(root)?;
    let directory = root.join(DIRECTORY);
    let (frames, cache) = (all_frames(root)?, load_cache(root)?);
    let seal = read_object(&directory.join("candidate-seal.json"))?;
    let pins = CONTEXT_V2.verify(seal.get("provenance").unwrap_or(&Value::Null), root)?;
    let (artifact, sealed) = fit(root, &frames, &cache, &pins)?;
    let read = |name: &str| std::fs::read(directory.join(name)).map_err(|error| error.to_string());
    if artifact != read("candidate.json")? {
        return Err("context-v2 candidate changed".to_owned());
    }
    if sealed != read("candidate-seal.json")? {
        return Err("context-v2 seal changed".to_owned());
    }
    if evaluate(root, &frames, &cache)? != read("report.json")? {
        return Err("context-v2 numeric report changed".to_owned());
    }
    result["frozen_training_replay"] = json!(true);
    result["frozen_numeric_regression"] = json!(true);
    Ok(result)
}

/// Rebuilds the corpus receipt from the snapshot and compares it with the frozen one.
pub fn verify_corpus_receipt(root: &Path) -> Result<Value, String> {
    let phrases = crate::ortho_inputs::load_phrases(root)?;
    let report = crate::ortho_inputs::assignment_report(&phrases);
    let path = root.join(DIRECTORY).join("corpus-receipt.json");
    let recorded = read_object(&path)?;
    let builder = CONTEXT_V2.recorded_digest("tools/context_corpus.py", recorded.get("builder_sha256").unwrap_or(&Value::Null), root)?;
    let mut content = report;
    content.insert("schema_version".to_owned(), json!(1));
    content.insert("source_sha256".to_owned(), json!("1f7df948b946f1d1aed5efd2a83a9a5a09d2b8abcb9224045b40c9ec6720e793"));
    content.insert("archive_sha256".to_owned(), json!("0b4b58b84239c4c194096040258deececa4f6a1ac3b93da16b64ae477b70f98e"));
    content.insert("source_url".to_owned(), json!("https://downloads.tatoeba.org/exports/sentences_CC0.tar.bz2"));
    content.insert("license".to_owned(), json!("CC0-1.0"));
    content.insert("license_evidence".to_owned(), json!("https://tatoeba.org/en/downloads"));
    content.insert("builder_sha256".to_owned(), json!(builder));
    let bytes = crate::train_json(&Value::Object(content.clone()));
    if bytes != std::fs::read(&path).map_err(|error| error.to_string())? {
        return Err("corpus receipt is missing or changed; a frozen split is never overwritten".to_owned());
    }
    Ok(Value::Object(content))
}
