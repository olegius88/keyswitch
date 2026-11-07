//! Prefix-v1, the first prefix model: fitted on frozen numeric rows, sealed, tested once. The installed prefix model has
//! its own schema and receipt; this keeps the old evidence checked, and a replay repeats the fit and the test on the
//! frozen rows and compares candidate, seal and report byte for byte.
use ks_cast::To as _;
use super::{read_object, unchanged};
use crate::artifact::canonical;
use crate::corpus::{checksum, read_json};
use crate::kernel::{self, Packed};
use crate::physical::hex;
use flate2::read::MultiGzDecoder;
use ks_context::context_model::{ACTIONS, ContextAction, first_greatest, softmax};
use ks_text::pynum::py_round_ndigits;
use ks_text::{JsonOptions, json_dumps, pystr};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::BufRead;
use std::path::Path;

const DIRECTORY: &str = "model/prefix_v1";
const SPLITS: [&str; 4] = ["train", "development", "calibration", "test"];
const PROFILES: [&str; 2] = ["portable", "reference_hunspell"];
const WIDTH: usize = ACTIONS.len();
const MIN_FEATURE_OCCURRENCES: i64 = 2;
const REPORT_MAX_EXAMPLES: usize = 15;
const MIN_CHARACTERS: i64 = 4;
const ROUNDING_DECIMALS: i64 = 9;
const VERSION_HASH_CHARACTERS: usize = 12;
const EARLY_RESTORED_MIN_FRACTION: f64 = 0.7;
const MIN_SELECTED_SEQUENCES: usize = 250;
const OPTIMIZER_SHA256: &str = "f523aa5652a70a897c1251985dfdc78676471ef338bfd9d8efe44f74efeb4b0c";
const TEST_SCOPE: &str = "First sealed synthetic prefix-sequence evaluation; repeats are regression, not a fresh test.";

/// Every row of a frozen split, read in order.
fn each_row(root: &Path, split: &str, mut visit: impl FnMut(&Value) -> Result<(), String>) -> Result<(), String> {
    let directory = root.join(DIRECTORY);
    let path = directory.join(format!("{split}.jsonl.gz"));
    if read_json(&directory.join("corpus.json"))?["sha256"][split].as_str() != Some(checksum(&path)?.as_str()) {
        return Err("prefix corpus checksum mismatch".to_owned());
    }
    let reader = std::io::BufReader::new(MultiGzDecoder::new(std::fs::File::open(&path).map_err(|error| error.to_string())?));
    for line in reader.lines() {
        let row: Value = serde_json::from_str(&line.map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
        visit(&row)?;
    }
    Ok(())
}

fn features_of(row: &Value) -> Result<&Map<String, Value>, String> {
    row["features"].as_object().ok_or_else(|| "a prefix row without features".to_owned())
}

struct SequenceResult {
    text: String,
    category: String,
    desired: bool,
    profile: String,
    candidates: Vec<(i64, f64)>,
    legacy_at: Option<i64>,
}

type Weights = HashMap<String, Vec<f64>>;

fn sequence_results(root: &Path, weights: &Weights, split: &str) -> Result<Vec<SequenceResult>, String> {
    let mut positions: HashMap<(i64, String), usize> = HashMap::new();
    let mut results: Vec<SequenceResult> = Vec::new();
    each_row(root, split, |row| {
        let profile = row["profile"].as_str().ok_or("a row without a profile")?.to_owned();
        let key = (row["sequence"].as_i64().ok_or("a row without a sequence")?, profile.clone());
        let place = *positions.entry(key).or_insert_with(|| {
            results.push(SequenceResult {
                text: row["text"].as_str().unwrap_or("").to_owned(),
                category: row["category"].as_str().unwrap_or("").to_owned(),
                desired: row["desired"].as_bool().unwrap_or(false),
                profile,
                candidates: Vec::new(),
                legacy_at: None,
            });
            results.len() - 1
        });
        let length = row["length"].as_i64().ok_or("a row without a length")?;
        if length < MIN_CHARACTERS {
            return Ok(());
        }
        let mut scores = [0.0_f64; WIDTH];
        for (name, value) in features_of(row)? {
            if let Some(table) = weights.get(name) {
                let value = value.as_f64().ok_or("a feature without a value")?;
                for (index, weight) in table.iter().enumerate() {
                    scores[index] += weight * value;
                }
            }
        }
        let probabilities = softmax(&scores);
        let best = first_greatest(&probabilities);
        let result = &mut results[place];
        if ACTIONS[best] == ContextAction::Convert {
            result.candidates.push((length, probabilities[best]));
        }
        if row["legacy_convert"].as_bool().unwrap_or(false) && result.legacy_at.is_none() {
            result.legacy_at = Some(length);
        }
        Ok(())
    })?;
    Ok(results)
}

fn metrics(data: &[SequenceResult], threshold: f64) -> Value {
    let mut counts: BTreeMap<&str, i64> = BTreeMap::new();
    let mut profiles: BTreeMap<&str, BTreeMap<&str, i64>> = BTreeMap::new();
    let mut examples = Vec::new();
    for result in data {
        let first = result.candidates.iter().find(|(_, probability)| *probability >= threshold).map(|(length, _)| *length);
        let is_false = first.is_some() && !result.desired;
        let mut local = vec![
            ("sequences", 1),
            ("desired", i64::from(result.desired)),
            ("converted", i64::from(first.is_some() && result.desired)),
            ("converted_before_end", i64::from(first.is_some_and(|length| length < pystr::len(&result.text).to::<i64>()) && result.desired)),
            ("false", i64::from(is_false)),
            ("technical_false", i64::from(is_false && matches!(result.category.as_str(), "identifier" | "command"))),
            ("legacy_converted", i64::from(result.legacy_at.is_some() && result.desired)),
            ("legacy_false", i64::from(result.legacy_at.is_some() && !result.desired)),
        ];
        if let Some(length) = first.filter(|_| result.desired) {
            local.push(("characters_before_conversion", length));
        }
        if examples.len() < REPORT_MAX_EXAMPLES && (is_false || (result.desired && first.is_none())) {
            examples.push(json!({"text": result.text, "category": result.category, "profile": result.profile, "desired": result.desired, "first_conversion": first}));
        }
        for (name, value) in local {
            *counts.entry(name).or_insert(0) += value;
            *profiles.entry(result.profile.as_str()).or_default().entry(name).or_insert(0) += value;
        }
    }
    json!({"counts": counts, "profiles": profiles, "examples": examples})
}

fn train(root: &Path, pins: &Value) -> Result<(Vec<u8>, Vec<u8>), String> {
    let cfg = read_json(&root.join(DIRECTORY).join("config.json"))?;
    let number = |name: &str| cfg[name].as_f64().ok_or_else(|| format!("the config has no {name}"));
    let (mut counts, mut labels): (HashMap<String, i64>, BTreeMap<usize, i64>) = Default::default();
    each_row(root, "train", |row| {
        for name in features_of(row)?.keys() {
            *counts.entry(name.clone()).or_insert(0) += 1;
        }
        *labels.entry((row["label"].as_u64().ok_or("a row without a label")?).to::<usize>()).or_insert(0) += 1;
        Ok(())
    })?;
    let mut ranked: Vec<(i64, String)> = counts.into_iter().map(|(name, count)| (-count, name)).collect();
    ranked.sort();
    ranked.truncate((number("maximum_features")?).to::<usize>());
    let mut names: Vec<String> = ranked.into_iter().filter(|(count, _)| -*count >= MIN_FEATURE_OCCURRENCES).map(|(_, name)| name).collect();
    names.sort();
    let factors = [number("keep_importance")?, 1.0, number("wait_importance")?, 1.0];
    let total: i64 = labels.values().sum();
    let importance: BTreeMap<usize, f64> = labels.iter().map(|(label, count)| (*label, total.to::<f64>() / (labels.len().to::<i64>() * count).to::<f64>() * factors[*label])).collect();
    let vocabulary: HashMap<&str, u32> = names.iter().enumerate().map(|(index, name)| (name.as_str(), index.to::<u32>())).collect();
    let pack = |split: &str| -> Result<Packed, String> {
        let mut packed = Packed { offsets: vec![0], ..Packed::default() };
        each_row(root, split, |row| {
            let label = (row["label"].as_u64().ok_or("a row without a label")?).to::<usize>();
            for (name, value) in features_of(row)? {
                if let Some(index) = vocabulary.get(name.as_str()) {
                    packed.indices.push(*index);
                    packed.values.push(value.as_f64().ok_or("a feature without a value")?);
                }
            }
            packed.offsets.push(packed.indices.len().to::<u64>());
            packed.labels.push(label.to::<u8>());
            packed.importance.push(importance[&label]);
            Ok(())
        })?;
        Ok(packed)
    };
    let (trained, developed) = (pack("train")?, pack("development")?);
    let (mut weights, mut accumulators) = (vec![0.0_f64; names.len() * WIDTH], vec![1.0_f64; names.len() * WIDTH]);
    let (mut best, mut best_loss, mut selected_epoch) = (weights.clone(), f64::INFINITY, 0_i64);
    let rate = number("learning_rate")?;
    for epoch in 0..(number("epochs")?).to::<i64>() {
        kernel::epoch(&trained, &mut weights, &mut accumulators, rate);
        let loss = kernel::log_loss(&kernel::predict(&developed, &weights), &developed) / developed.rows().to::<f64>();
        if loss < best_loss {
            best.clone_from(&weights);
            best_loss = loss;
            selected_epoch = epoch + 1;
        }
    }
    let learned: Map<String, Value> = names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), json!((0..WIDTH).map(|action| py_round_ndigits(best[index * WIDTH + action], ROUNDING_DECIMALS).expect("a finite weight")).collect::<Vec<f64>>())))
        .collect();
    let table: Weights = learned.iter().map(|(name, row)| (name.clone(), row.as_array().into_iter().flatten().filter_map(Value::as_f64).collect())).collect();
    let learned = Value::Object(learned);
    let compact = JsonOptions { ensure_ascii: false, separators: Some((",".to_owned(), ":".to_owned())), sort_keys: true, ..JsonOptions::default() };
    let digest = hex(&Sha256::digest(json_dumps(&learned, &compact).as_bytes()));
    let version = format!("prefix-v1-{}", digest.get(..VERSION_HASH_CHARACTERS).unwrap_or_default());
    let calibration = sequence_results(root, &table, "calibration")?;
    let mut threshold = 1.0;
    for candidate in cfg["thresholds"].as_array().ok_or("no thresholds")?.iter().filter_map(Value::as_f64) {
        if metrics(&calibration, candidate)["counts"]["false"] == json!(0) {
            threshold = candidate;
            break;
        }
    }
    let actions: Vec<&str> = ACTIONS.iter().map(|action| action.as_str()).collect();
    let raw = canonical(&json!({"feature_version": 1, "actions": actions, "version": version, "conversion_threshold": threshold, "weights_sha256": digest, "weights": learned, "kind": "keyswitch.prefix-policy", "prefix_feature_version": 1}));
    let seal = json!({
        "schema_version": 1, "candidate_sha256": hex(&Sha256::digest(&raw)), "provenance": pins, "config": cfg, "features": names.len(), "epoch": selected_epoch,
        "development_loss": py_round_ndigits(best_loss, ROUNDING_DECIMALS).expect("a finite loss"), "training_rows": trained.rows(),
        "development": metrics(&sequence_results(root, &table, "development")?, threshold),
        "calibration": metrics(&calibration, threshold), "threshold": threshold,
    });
    Ok((raw, canonical(&seal)))
}

fn accepted(result: &Value, gates: &Value) -> bool {
    let Some(profiles) = result["profiles"].as_object() else { return false };
    if profiles.keys().map(String::as_str).collect::<HashSet<&str>>() != PROFILES.iter().copied().collect() {
        return false;
    }
    profiles.values().all(|counts| {
        let count = |name: &str| counts[name].as_i64().filter(|value| *value >= 0);
        let (Some(sequences), Some(desired), Some(converted), Some(before_end), Some(false_count), Some(technical), Some(legacy_false)) =
            (count("sequences"), count("desired"), count("converted"), count("converted_before_end"), count("false"), count("technical_false"), count("legacy_false"))
        else {
            return false;
        };
        let gate = |name: &str| gates[name].as_f64().unwrap_or(f64::NAN);
        before_end <= converted
            && converted <= desired
            && desired <= sequences
            && technical <= false_count
            && false_count <= sequences - desired
            && sequences.to::<f64>() >= gate("minimum_test_sequences")
            && desired > 0
            && false_count.to::<f64>() / sequences.to::<f64>() <= gate("maximum_false_sequence_rate")
            && technical.to::<f64>() <= gate("maximum_technical_false_sequences")
            && before_end.to::<f64>() / desired.to::<f64>() >= gate("minimum_wrong_layout_sequence_recall")
            && (false_count - legacy_false).to::<f64>() <= gate("maximum_false_sequences_over_legacy")
    })
}

fn evaluate(root: &Path) -> Result<Vec<u8>, String> {
    let directory = root.join(DIRECTORY);
    let (candidate, seal_path) = (directory.join("candidate.json"), directory.join("seal.json"));
    let seal = read_object(&seal_path)?;
    let cfg = read_json(&directory.join("config.json"))?;
    let artifact = read_json(&candidate)?;
    let table: Weights = artifact["weights"].as_object().ok_or("a model without weights")?.iter().map(|(name, row)| (name.clone(), row.as_array().into_iter().flatten().filter_map(Value::as_f64).collect())).collect();
    let threshold = seal.get("threshold").and_then(Value::as_f64).ok_or("a seal without a threshold")?;
    let result = metrics(&sequence_results(root, &table, "test")?, threshold);
    let passed = accepted(&result, &cfg["promotion"]);
    Ok(canonical(&json!({"schema_version": 1, "candidate_sha256": checksum(&candidate)?, "seal_sha256": checksum(&seal_path)?, "test": result, "accepted": passed, "scope": TEST_SCOPE})))
}

/// The frozen examples are the receipt's, the seal and the report describe the candidate, and the engine's exact-text
/// evidence holds together. The prefix model installed today is checked by its pair's release receipt (`ks-train verify`).
pub fn verify(root: &Path) -> Result<Value, String> {
    let directory = root.join(DIRECTORY);
    let receipt = read_object(&directory.join("corpus.json"))?;
    if receipt.get("family_overlap") != Some(&json!(0)) || receipt.get("profiles") != Some(&json!(PROFILES)) {
        return Err("prefix corpus provenance changed".to_owned());
    }
    for split in SPLITS {
        if receipt.get("sha256").and_then(|table| table.get(split)).and_then(Value::as_str) != Some(checksum(&directory.join(format!("{split}.jsonl.gz")))?.as_str()) {
            return Err("prefix frozen examples changed".to_owned());
        }
    }
    let (candidate, seal_path) = (directory.join("candidate.json"), directory.join("seal.json"));
    let (seal, report, engine) = (read_object(&seal_path)?, read_object(&directory.join("report.json"))?, read_object(&directory.join("engine-report.json"))?);
    let cfg = read_json(&directory.join("config.json"))?;
    let pins = seal.get("provenance").ok_or("a seal without provenance")?;
    if pins.get("corpus").and_then(Value::as_str) != Some(checksum(&directory.join("corpus.json"))?.as_str()) || pins.get("optimizer").and_then(Value::as_str) != Some(OPTIMIZER_SHA256) || seal.get("config") != Some(&cfg) || seal.get("candidate_sha256").and_then(Value::as_str) != Some(checksum(&candidate)?.as_str()) {
        return Err("prefix seal changed".to_owned());
    }
    let artifact = read_json(&candidate)?;
    if seal.get("threshold") != artifact.get("conversion_threshold") {
        return Err("prefix threshold changed after calibration".to_owned());
    }
    if report.get("candidate_sha256") != seal.get("candidate_sha256") || report.get("seal_sha256").and_then(Value::as_str) != Some(checksum(&seal_path)?.as_str()) {
        return Err("prefix test identity changed".to_owned());
    }
    let test = report.get("test").filter(|test| test.is_object()).ok_or("prefix test missing")?;
    if report.get("accepted") != Some(&json!(true)) || !accepted(test, &cfg["promotion"]) {
        return Err("prefix model fails sequence quality gates".to_owned());
    }
    let results = engine.get("results").and_then(Value::as_object).ok_or("prefix engine quality evidence missing")?;
    let expected: HashSet<String> = PROFILES.iter().flat_map(|profile| ["observed", "field"].iter().map(move |context| format!("{profile}/{context}"))).collect();
    if engine.get("passed") != Some(&json!(true)) || engine.get("split") != Some(&json!("test")) || results.keys().cloned().collect::<HashSet<String>>() != expected {
        return Err("prefix engine quality evidence missing".to_owned());
    }
    let ids = engine.get("sequence_ids").and_then(Value::as_array).ok_or("prefix engine selection incomplete")?;
    let distinct: HashSet<String> = ids.iter().map(Value::to_string).collect();
    if ids.len() < MIN_SELECTED_SEQUENCES || distinct.len() != ids.len() {
        return Err("prefix engine selection incomplete".to_owned());
    }
    for variants in results.values() {
        let table = variants.as_object().filter(|table| table.keys().map(String::as_str).collect::<HashSet<&str>>() == HashSet::from(["candidate", "shipping_no_prefix"])).ok_or("prefix engine baseline missing")?;
        for counts in table.values() {
            let count = |name: &str| counts[name].as_i64().filter(|value| *value >= 0).ok_or("invalid prefix engine counts");
            let (sequences, desired, exact, restored, changed, early) = (count("sequences")?, count("desired")?, count("exact")?, count("restored")?, count("changed_correct")?, count("early_restored")?);
            count("length_mismatches")?;
            count("injections")?;
            if !(sequences == ids.len().to::<i64>() && 0 < desired && desired <= sequences && early <= restored && restored <= desired && exact == restored + sequences - desired - changed) {
                return Err("inconsistent prefix engine counts".to_owned());
            }
        }
        let (current, previous) = (&table["candidate"], &table["shipping_no_prefix"]);
        let number = |value: &Value, name: &str| value[name].as_i64().unwrap_or(0);
        if number(current, "length_mismatches") != 0 || number(current, "changed_correct") > number(previous, "changed_correct") || number(current, "restored") < number(previous, "restored") || number(current, "early_restored").to::<f64>() < EARLY_RESTORED_MIN_FRACTION * number(current, "desired").to::<f64>() {
            return Err("prefix engine quality gates failed".to_owned());
        }
    }
    if let Some(regression) = engine.get("runtime_regression").filter(|value| !value.is_null()) {
        let previous = regression.get("previous_report").and_then(Value::as_str).ok_or("invalid prefix runtime history")?;
        let inside = Path::new(previous).parent().is_some_and(|folder| folder == Path::new("model/prefix_v1/engine-history"));
        if !inside || regression.get("previous_sha256").and_then(Value::as_str) != Some(checksum(&root.join(previous))?.as_str()) || read_object(&root.join(previous))?.get("sequence_ids") != engine.get("sequence_ids") {
            return Err("prefix runtime history changed".to_owned());
        }
    }
    Ok(json!({"model_version": artifact["version"], "feature_version": 1, "accepted": true, "artifact_sha256": checksum(&candidate)?, "test": test, "engine_passed": true}))
}

/// Repeats the fit and the test on the frozen rows; writes nothing.
pub fn replay(root: &Path) -> Result<Value, String> {
    let mut result = verify(root)?;
    let directory = root.join(DIRECTORY);
    let seal = read_object(&directory.join("seal.json"))?;
    let (candidate, sealed) = train(root, seal.get("provenance").unwrap_or(&Value::Null))?;
    let read = |name: &str| std::fs::read(directory.join(name)).map_err(|error| error.to_string());
    unchanged(&candidate, &read("candidate.json")?, "prefix-v1 candidate is not reproducible")?;
    unchanged(&sealed, &read("seal.json")?, "prefix-v1 seal is not reproducible")?;
    unchanged(&evaluate(root)?, &read("report.json")?, "prefix-v1 report is not reproducible")?;
    result["frozen_numeric_regression"] = json!(true);
    Ok(result)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::super::testing::{Tree, refused, repository};
    use super::*;

    fn passing() -> (Value, Value) {
        let counts = json!({"sequences": 100, "desired": 50, "converted": 40, "converted_before_end": 30, "false": 2, "technical_false": 1, "legacy_false": 2});
        let gates = json!({"minimum_test_sequences": 10, "maximum_false_sequence_rate": 0.1, "maximum_technical_false_sequences": 1, "minimum_wrong_layout_sequence_recall": 0.5, "maximum_false_sequences_over_legacy": 1});
        (json!({"profiles": {"portable": counts, "reference_hunspell": counts}}), gates)
    }

    #[test]
    fn a_test_is_accepted_when_both_profiles_hold_every_gate() {
        let (result, gates) = passing();
        assert!(accepted(&result, &gates));
        assert!(!accepted(&json!({}), &gates));
        assert!(!accepted(&json!({"profiles": {"portable": result["profiles"]["portable"]}}), &gates));
        let with_count = |name: &str, value: Value| {
            let mut changed = result.clone();
            changed["profiles"]["reference_hunspell"][name] = value;
            accepted(&changed, &gates)
        };
        assert!(!with_count("legacy_false", json!(-1)));
        assert!(!with_count("legacy_false", Value::Null));
        for (name, value) in [("converted_before_end", 41), ("converted", 51), ("desired", 101), ("technical_false", 3), ("false", 60), ("legacy_false", 0)] {
            assert!(!with_count(name, json!(value)), "{name}");
        }
        let mut nothing_wanted = result.clone();
        for name in ["desired", "converted", "converted_before_end"] {
            nothing_wanted["profiles"]["portable"][name] = json!(0);
        }
        assert!(!accepted(&nothing_wanted, &gates));
        for (name, value) in [("minimum_test_sequences", 1000.0), ("maximum_false_sequence_rate", 0.01), ("maximum_technical_false_sequences", 0.0), ("minimum_wrong_layout_sequence_recall", 0.7)] {
            let mut stricter = gates.clone();
            stricter[name] = json!(value);
            assert!(!accepted(&result, &stricter), "{name}");
        }
    }

    #[test]
    fn a_split_is_read_only_as_its_receipt_pins_it() {
        let tree = Tree::new("prefix-rows");
        tree.edit("model/prefix_v1/corpus.json", |receipt| receipt["sha256"]["test"] = json!("0"));
        refused(each_row(&tree.root, "test", |_| Ok(())), "prefix corpus checksum mismatch");
    }

    const FILES: [&str; 6] = ["corpus.json", "seal.json", "report.json", "engine-report.json", "config.json", "candidate.json"];

    /// The tree's evidence back to the repository's, then changed by `edit`.
    fn evidence(tree: &Tree, edit: impl FnOnce(&Tree)) -> Result<Value, String> {
        for name in FILES {
            tree.write(&format!("{DIRECTORY}/{name}"), &std::fs::read(repository().join(DIRECTORY).join(name)).expect("a file"));
        }
        edit(tree);
        verify(&tree.root)
    }

    fn edit(tree: &Tree, name: &str, change: impl FnOnce(&mut Value)) {
        tree.edit(&format!("{DIRECTORY}/{name}"), change);
    }

    /// Every variant of every engine track changed alike.
    fn engine(tree: &Tree, change: impl Fn(&mut Map<String, Value>, &str)) {
        edit(tree, "engine-report.json", |report| {
            for variants in report["results"].as_object_mut().expect("tracks").values_mut() {
                for (name, counts) in variants.as_object_mut().expect("variants") {
                    change(counts.as_object_mut().expect("counts"), name);
                }
            }
        });
    }

    #[test]
    fn the_receipt_the_seal_and_the_report_describe_one_candidate() {
        let tree = Tree::new("prefix-verify");
        assert_eq!(evidence(&tree, |_| ()).expect("the kept evidence")["accepted"], json!(true));
        refused(evidence(&tree, |tree| edit(tree, "corpus.json", |receipt| receipt["family_overlap"] = json!(1))), "prefix corpus provenance changed");
        refused(evidence(&tree, |tree| edit(tree, "corpus.json", |receipt| receipt["profiles"] = json!(["portable"]))), "prefix corpus provenance changed");
        refused(evidence(&tree, |tree| edit(tree, "corpus.json", |receipt| receipt["sha256"]["train"] = json!("0"))), "prefix frozen examples changed");
        let seal_changed = "prefix seal changed";
        refused(evidence(&tree, |tree| edit(tree, "seal.json", |seal| drop(seal.as_object_mut().expect("a seal").remove("provenance")))), "a seal without provenance");
        refused(evidence(&tree, |tree| edit(tree, "seal.json", |seal| seal["provenance"]["corpus"] = json!("0"))), seal_changed);
        refused(evidence(&tree, |tree| edit(tree, "seal.json", |seal| seal["provenance"]["optimizer"] = json!("0"))), seal_changed);
        refused(evidence(&tree, |tree| edit(tree, "config.json", |config| config["epochs"] = json!(0))), seal_changed);
        refused(evidence(&tree, |tree| edit(tree, "seal.json", |seal| seal["candidate_sha256"] = json!("0"))), seal_changed);
        let moved_threshold = |tree: &Tree| {
            edit(tree, "candidate.json", |candidate| candidate["conversion_threshold"] = json!(0.5));
            let digest = checksum(&tree.path(&format!("{DIRECTORY}/candidate.json"))).expect("a digest");
            edit(tree, "seal.json", |seal| seal["candidate_sha256"] = json!(digest));
        };
        refused(evidence(&tree, moved_threshold), "prefix threshold changed after calibration");
        let identity = "prefix test identity changed";
        refused(evidence(&tree, |tree| edit(tree, "report.json", |report| report["candidate_sha256"] = json!("0"))), identity);
        refused(evidence(&tree, |tree| edit(tree, "report.json", |report| report["seal_sha256"] = json!("0"))), identity);
        refused(evidence(&tree, |tree| edit(tree, "report.json", |report| report["test"] = json!(1))), "prefix test missing");
        let gates = "prefix model fails sequence quality gates";
        refused(evidence(&tree, |tree| edit(tree, "report.json", |report| report["accepted"] = json!(false))), gates);
        refused(evidence(&tree, |tree| edit(tree, "report.json", |report| report["test"]["profiles"] = json!({}))), gates);
    }

    #[test]
    fn the_engine_evidence_holds_together_and_beats_the_shipped_model() {
        let tree = Tree::new("prefix-engine");
        let missing = "prefix engine quality evidence missing";
        refused(evidence(&tree, |tree| edit(tree, "engine-report.json", |report| report["results"] = json!([]))), missing);
        refused(evidence(&tree, |tree| edit(tree, "engine-report.json", |report| report["passed"] = json!(false))), missing);
        refused(evidence(&tree, |tree| edit(tree, "engine-report.json", |report| report["split"] = json!("development"))), missing);
        refused(evidence(&tree, |tree| edit(tree, "engine-report.json", |report| drop(report["results"].as_object_mut().expect("tracks").remove("portable/field")))), missing);
        let incomplete = "prefix engine selection incomplete";
        refused(evidence(&tree, |tree| edit(tree, "engine-report.json", |report| report["sequence_ids"] = Value::Null)), incomplete);
        refused(evidence(&tree, |tree| edit(tree, "engine-report.json", |report| report["sequence_ids"].as_array_mut().expect("ids").truncate(10))), incomplete);
        refused(evidence(&tree, |tree| edit(tree, "engine-report.json", |report| report["sequence_ids"][1] = report["sequence_ids"][0].clone())), incomplete);
        refused(evidence(&tree, |tree| edit(tree, "engine-report.json", |report| report["results"]["portable/field"] = json!({"candidate": {}}))), "prefix engine baseline missing");
        refused(evidence(&tree, |tree| edit(tree, "engine-report.json", |report| report["results"]["portable/field"] = json!(1))), "prefix engine baseline missing");
        for (name, value) in [("sequences", json!(-1)), ("injections", Value::Null), ("length_mismatches", Value::Null)] {
            refused(evidence(&tree, |tree| engine(tree, |counts, _| drop(counts.insert(name.to_owned(), value.clone())))), "invalid prefix engine counts");
        }
        for (name, value) in [("sequences", 287), ("desired", 0), ("desired", 289), ("early_restored", 129), ("restored", 129), ("exact", 0)] {
            refused(evidence(&tree, |tree| engine(tree, |counts, _| drop(counts.insert(name.to_owned(), json!(value))))), "inconsistent prefix engine counts");
        }
        let failed = "prefix engine quality gates failed";
        let candidate = |changes: &'static [(&'static str, i64)]| {
            move |tree: &Tree| {
                engine(tree, |counts, variant| {
                    if variant == "candidate" {
                        for (name, value) in changes {
                            counts.insert((*name).to_owned(), json!(value));
                        }
                    }
                });
            }
        };
        refused(evidence(&tree, candidate(&[("length_mismatches", 1)])), failed);
        refused(evidence(&tree, candidate(&[("changed_correct", 21), ("exact", 267)])), failed);
        refused(evidence(&tree, candidate(&[("restored", 125), ("exact", 265)])), failed);
        refused(evidence(&tree, candidate(&[("early_restored", 80)])), failed);
    }

    #[test]
    fn the_runtime_history_is_the_one_the_engine_report_names() {
        let tree = Tree::new("prefix-history");
        assert!(evidence(&tree, |tree| edit(tree, "engine-report.json", |report| report["runtime_regression"] = Value::Null)).is_ok());
        refused(evidence(&tree, |tree| edit(tree, "engine-report.json", |report| report["runtime_regression"] = json!({}))), "invalid prefix runtime history");
        let changed = "prefix runtime history changed";
        refused(evidence(&tree, |tree| edit(tree, "engine-report.json", |report| report["runtime_regression"]["previous_report"] = json!("model/elsewhere.json"))), changed);
        refused(evidence(&tree, |tree| edit(tree, "engine-report.json", |report| report["runtime_regression"]["previous_sha256"] = json!("0"))), changed);
        let other_selection = |tree: &Tree| {
            let previous = format!("{DIRECTORY}/engine-history/other.json");
            tree.write(&previous, br#"{"sequence_ids": []}"#);
            let digest = checksum(&tree.path(&previous)).expect("a digest");
            edit(tree, "engine-report.json", |report| report["runtime_regression"] = json!({"previous_report": previous, "previous_sha256": digest}));
        };
        refused(evidence(&tree, other_selection), changed);
    }

    /// A frozen split of these rows in the tree, pinned by the tree's receipt.
    fn split_of(tree: &Tree, split: &str, rows: &[Value]) {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        for row in rows {
            std::io::Write::write_all(&mut encoder, format!("{row}\n").as_bytes()).expect("in memory");
        }
        let path = format!("{DIRECTORY}/{split}.jsonl.gz");
        tree.write(&path, &encoder.finish().expect("in memory"));
        let digest = checksum(&tree.path(&path)).expect("a digest");
        edit(tree, "corpus.json", |receipt| receipt["sha256"][split] = json!(digest));
    }

    #[test]
    fn the_epoch_and_the_threshold_kept_are_the_first_the_held_out_rows_allow() {
        let tree = Tree::new("prefix-train");
        let row = |sequence: i64, feature: &str, label: u64, desired: bool| {
            json!({"profile": "portable", "sequence": sequence, "text": "ghbdtn", "category": "wrong", "desired": desired, "length": 6, "features": {feature: 100.0}, "label": label, "legacy_convert": false})
        };
        let trained: Vec<Value> = (0..8).map(|sequence| if sequence % 2 == 0 { row(sequence, "a", 1, true) } else { row(sequence, "b", 0, false) }).collect();
        split_of(&tree, "train", &trained);
        // Development rows answer the other way: the more the fit learns, the worse it does on them.
        split_of(&tree, "development", &(0..4).map(|sequence| row(sequence, "a", 0, false)).collect::<Vec<Value>>());
        // Calibration rows that the learned weights convert, against what they want.
        split_of(&tree, "calibration", &(0..4).map(|sequence| row(sequence, "a", 0, false)).collect::<Vec<Value>>());
        let (_, seal) = train(&tree.root, &Value::Null).expect("a fit");
        let seal: Value = serde_json::from_slice(&seal).expect("json");
        assert_eq!(seal["epoch"], json!(1), "{seal}");
        assert!(seal["threshold"].as_f64().expect("a threshold") > 0.985, "{seal}");
    }
}
