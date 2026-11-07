//! The seal of the layout-intent model (`train_intent_model`): the identity of a candidate, the one-shot ledgers of the
//! sealed test, the KSLM container and the transactional publication of a bundle of files.
//!
//! A manifest written by this trainer says so in `toolchain.implementation`, and its digests are of the Rust sources. A
//! manifest without that field was written by the Python trainer, whose sources are not in this tree: the numeric replay
//! (`intent --verify`) stands in for them until the first model sealed here replaces it.
use ks_cast::To as _;
use super::fit::{Platt, Quantized, Selections, Veto};
use crate::artifact::canonical;
use crate::physical::hex;
use ks_lang::constants::{INTENT_FEATURE_VERSION, KSLM_SCHEMA_VERSION};
use ks_lang::intent_model::{LinearNgramModel, TRIGGERS, crc32};
use ks_text::pynum::float_hex;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub const IMPLEMENTATION: &str = "rust";
pub const MODEL_ID: &str = "keyswitch-layout-intent-v1";
pub const SCOPE: &str = "lexical-synthetic-not-real-world-probability";
pub const VERSION_HASH_CHARACTERS: usize = 12;
const CANDIDATE_RECORD_SCHEMA_VERSION: i64 = 2;
const REGISTRY_MAX_BYTES: usize = 16 << 10;
const BACKUP_MAX_BYTES: usize = 64 << 20;
const QUANTIZED_LIMIT: f64 = 32767.0;
const HASH_ALGORITHM: &str = "fnv1a64-signed-v1";
const MEMBERSHIP_ALGORITHM: &str = "fnv1a64-unsigned-v1";
pub const OUTCOME_SECTIONS: [&str; 5] = ["sealed_test", "sealed_test_typos", "sealed_test_context_stress", "safety", "veto"];
pub const BUILD_PROVENANCE_FIELDS: [&str; 10] = ["config_sha256", "dataset_sha256", "split_namespace", "sealed_evaluation", "gate_policy", "source_package", "sources", "toolchain", "variant_quarantine_sha256", "training_language_scorer"];

/// The files behind each digest of the toolchain block. A digest of several files covers their names and their bytes.
pub const TOOLCHAIN_FILES: [(&str, &[&str]); 11] = [
    ("trainer_sha256", &["crates/ks-train/src/intent/dataset.rs", "crates/ks-train/src/intent/fit.rs", "crates/ks-train/src/intent/pipeline.rs", "crates/ks-train/src/intent/seal.rs", "crates/ks-train/src/intent/train.rs"]),
    ("runtime_sha256", &["crates/ks-lang/src/intent_model.rs"]),
    ("detector_sha256", &["crates/ks-lang/src/detector.rs"]),
    ("protected_tokens_sha256", &["resources/protected_tokens.txt"]),
    ("layouts_sha256", &["crates/ks-text/src/layouts.rs"]),
    ("language_model_sha256", &["crates/ks-lang/src/language_model.rs"]),
    ("evaluator_sha256", &["crates/ks-train/src/intent/evaluate.rs"]),
    ("environment_probe_sha256", &["crates/ks-train/src/intent/probe.rs"]),
    ("preseal_generator_sha256", &["crates/ks-train/src/intent/external.rs"]),
    ("development_freezer_sha256", &["crates/ks-train/src/intent/external.rs"]),
    ("spellcheck_sha256", &["crates/ks-lang/src/spellcheck.rs", "crates/ks-hunspell/src/lib.rs"]),
];
pub const CONSTANTS_FILES: [&str; 2] = ["crates/ks-lang/src/constants.rs", "crates/ks-text/src/constants/mod.rs"];

fn read(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))
}

pub fn sha256_file(path: &Path) -> Result<String, String> {
    Ok(hex(&Sha256::digest(read(path)?)))
}

/// One digest of the listed files of the tree.
pub fn digest_files(root: &Path, files: &[&str]) -> Result<String, String> {
    if let [only] = files {
        return sha256_file(&root.join(only));
    }
    let mut hasher = Sha256::new();
    for file in files {
        hasher.update(file.as_bytes());
        hasher.update(b"\0");
        hasher.update(read(&root.join(file))?);
        hasher.update(b"\n");
    }
    Ok(hex(&hasher.finalize()))
}

/// `holdout-vN-preseal.json` beside `seal-registry-vN.json`.
pub fn preseal_receipt_path(registry: &str) -> Result<String, String> {
    let name = registry.rsplit('/').next().unwrap_or(registry);
    let version = name.strip_prefix("seal-registry-v").and_then(|rest| rest.strip_suffix(".json")).ok_or_else(|| format!("unexpected registry path {registry:?}"))?;
    Ok(format!("model/intent_v1/holdout-v{version}-preseal.json"))
}

/// `capture_toolchain_snapshot`: code and data identity, before anything is fitted.
pub fn toolchain_snapshot(root: &Path, config_sha256: &str, registry: &str) -> Result<Value, String> {
    let mut block = Map::new();
    block.insert("implementation".to_owned(), json!(IMPLEMENTATION));
    block.insert("config_sha256".to_owned(), json!(config_sha256));
    for (field, files) in TOOLCHAIN_FILES {
        block.insert(field.to_owned(), json!(digest_files(root, files)?));
    }
    block.insert("preseal_receipt_sha256".to_owned(), json!(sha256_file(&root.join(preseal_receipt_path(registry)?))?));
    let constants: Vec<&str> = CONSTANTS_FILES.iter().copied().filter(|file| root.join(file).is_file()).collect();
    block.insert("constants_sha256".to_owned(), json!(digest_files(root, &constants)?));
    Ok(Value::Object(block))
}

/// `verify_toolchain_snapshot`: refuse publication when an input changed during training.
pub fn verify_toolchain(root: &Path, snapshot: &Value, config_path: &Path, registry: &str) -> Result<(), String> {
    let current = toolchain_snapshot(root, &sha256_file(config_path)?, registry)?;
    if &current == snapshot {
        return Ok(());
    }
    let changed: Vec<&String> = snapshot.as_object().into_iter().flatten().filter(|(name, value)| current.get(name.as_str()) != Some(value)).map(|(name, _)| name).collect();
    Err(format!("training inputs changed during execution: {changed:?}"))
}

/// Everything known before the sealed test: the exact projection of the manifest `presealed_candidate_metadata_projection` makes.
#[must_use]
pub fn metadata_projection(config_sha256: &str, namespace: &str, toolchain: &Value, source_package: &Value, sources: &Value, candidate_counts: &Value, variant_quarantine_sha256: &str, scorer: &Value, gate_policy: &Value, training: &Value, quantization: &Value, calibration: &Value, veto_selection: &Value, thresholds: &Value, selection_gate: &Value, safety_audit: &Value, parameters: &Value) -> Value {
    json!({
        "schema_version": 1,
        "model_id": MODEL_ID,
        "calibration_scope": SCOPE,
        "config_sha256": config_sha256,
        "split_namespace": namespace,
        "toolchain": toolchain,
        "source_package": source_package,
        "sources": sources,
        "candidate_counts": candidate_counts,
        "variant_quarantine_sha256": variant_quarantine_sha256,
        "training_language_scorer": scorer,
        "gate_policy": gate_policy,
        "training": training,
        "quantization": quantization,
        "calibration": calibration,
        "veto_selection": veto_selection,
        "thresholds": thresholds,
        "threshold_selection_gate_breakdown": selection_gate,
        "safety_guard_audit": safety_audit,
        "model_parameters": parameters,
    })
}

/// `sealed_candidate_sha256`: every candidate input fixed before the sealed test is exposed.
#[must_use]
pub fn candidate_sha256(namespace: &str, config_sha256: &str, candidate_dataset_sha256: &str, metadata: &Value) -> String {
    let payload = json!({
        "schema_version": CANDIDATE_RECORD_SCHEMA_VERSION,
        "split_namespace": namespace,
        "config_sha256": config_sha256,
        "candidate_dataset_sha256": candidate_dataset_sha256,
        "toolchain": metadata["toolchain"],
        "training_language_scorer": metadata["training_language_scorer"],
        "model": metadata["model_parameters"],
        "threshold_selection_gate_breakdown": metadata["threshold_selection_gate_breakdown"],
        "candidate_metadata": metadata,
    });
    hex(&Sha256::digest(canonical(&payload)))
}

/// `sealed_outcome_sha256`: what the sealed test said, over the sections that say it.
pub fn outcome_sha256(manifest: &Map<String, Value>) -> Result<String, String> {
    let mut payload = Map::new();
    for name in OUTCOME_SECTIONS {
        payload.insert(name.to_owned(), manifest.get(name).cloned().ok_or_else(|| format!("the manifest is missing the sealed-outcome section {name}"))?);
    }
    Ok(hex(&Sha256::digest(canonical(&Value::Object(payload)))))
}

/// `json.dumps(payload, ensure_ascii=False, sort_keys=True, separators=(",", ":"))` of the provenance fields.
pub fn build_provenance_sha256(manifest: &Map<String, Value>) -> Result<String, String> {
    let mut payload = Map::new();
    for name in BUILD_PROVENANCE_FIELDS {
        payload.insert(name.to_owned(), manifest.get(name).cloned().ok_or_else(|| format!("the manifest is missing {name}"))?);
    }
    let bytes = canonical(&Value::Object(payload));
    Ok(hex(&Sha256::digest(&bytes[..bytes.len() - 1])))
}

fn read_snapshot(path: &Path) -> Result<Vec<u8>, String> {
    if path.is_symlink() {
        return Err(format!("sealed evaluation registry cannot be a symlink: {}", path.display()));
    }
    let data = std::fs::read(path).map_err(|error| format!("sealed evaluation registry is unavailable: {}: {error}", path.display()))?;
    if data.len() > REGISTRY_MAX_BYTES {
        return Err("sealed evaluation registry exceeds its size limit".to_owned());
    }
    Ok(data)
}

/// `_stage_bytes`: durably beside the destination, without replacing it.
fn stage(destination: &Path, data: &[u8]) -> Result<PathBuf, String> {
    use std::io::Write as _;
    let parent = destination.parent().ok_or("a destination has a folder")?;
    std::fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    let name = destination.file_name().ok_or("a destination has a name")?.to_string_lossy().into_owned();
    let staged = parent.join(format!(".{name}.{}.staged", std::process::id()));
    let mut file = std::fs::File::create(&staged).map_err(|error| format!("{}: {error}", staged.display()))?;
    file.write_all(data).and_then(|()| file.sync_all()).map_err(|error| format!("{}: {error}", staged.display()))?;
    Ok(staged)
}

fn sync_folder(path: &Path) -> Result<(), String> {
    std::fs::File::open(path).map_or_else(|error| unopened_folder(path, &error), |folder| folder.sync_all().map_err(|error| format!("{}: {error}", path.display())))
}

/// Windows opens no folder as a file: there is nothing to sync, and nothing failed.
#[cfg(windows)]
fn unopened_folder(_path: &Path, _error: &std::io::Error) -> Result<(), String> {
    Ok(())
}

/// Elsewhere a folder that does not open is a failure of the publication.
#[cfg(not(windows))]
fn unopened_folder(path: &Path, error: &std::io::Error) -> Result<(), String> {
    Err(format!("{}: {error}", path.display()))
}

/// Create a record exactly once: nothing else can have been there when this returns `Ok(None)`; otherwise the bytes found.
fn create_once(path: &Path, expected: &[u8]) -> Result<Option<Vec<u8>>, String> {
    let staged = stage(path, expected)?;
    let linked = std::fs::hard_link(&staged, path);
    let _ = std::fs::remove_file(&staged);
    match linked {
        Ok(()) => {
            sync_folder(path.parent().expect("a folder"))?;
            Ok(None)
        }
        Err(error) => existing_record(path, &error),
    }
}

/// What a link that did not take found: the record already there, or a failure.
fn existing_record(path: &Path, error: &std::io::Error) -> Result<Option<Vec<u8>>, String> {
    if error.kind() == std::io::ErrorKind::AlreadyExists { Ok(Some(read_snapshot(path)?)) } else { Err(format!("cannot create the ledger {}: {error}", path.display())) }
}

/// A ledger read again is the one that was read: anything else wrote it meanwhile.
fn still(same: bool, what: &str) -> Result<(), String> {
    if same { Ok(()) } else { Err(what.to_owned()) }
}

/// `SealedEvaluationReceipt`.
pub struct Receipt {
    pub namespace: String,
    pub candidate_sha256: String,
    pub config_sha256: String,
    pub candidate_dataset_sha256: String,
    pub registry: String,
    pub registry_sha256: String,
    path: PathBuf,
    bytes: Vec<u8>,
}

impl Receipt {
    #[must_use]
    pub fn payload(&self) -> Value {
        json!({"schema_version": 1, "split_namespace": self.namespace, "candidate_sha256": self.candidate_sha256, "config_sha256": self.config_sha256, "candidate_dataset_sha256": self.candidate_dataset_sha256, "registry_path": self.registry, "registry_sha256": self.registry_sha256})
    }

    /// `verify_sealed_evaluation_receipt`: the one-shot ledger has not moved since test access was granted.
    pub fn verify(&self) -> Result<(), String> {
        still(read_snapshot(&self.path)? == self.bytes, "sealed evaluation registry changed after claim")
    }
}

fn refusal(existing: &[u8], record: &Value) -> String {
    let Ok(previous) = serde_json::from_slice::<Value>(existing) else {
        return "the sealed evaluation registry is unreadable; it must be restored from version control before this candidate is evaluated".to_owned();
    };
    if previous["split_namespace"] != record["split_namespace"] {
        return format!("the sealed evaluation registry belongs to split namespace {}, not {}; the wrong registry file is in place", previous["split_namespace"], record["split_namespace"]);
    }
    let moved: Vec<&str> = ["config_sha256", "candidate_dataset_sha256"].into_iter().filter(|field| previous[*field] != record[*field]).collect();
    if !moved.is_empty() {
        return format!("the sealed test namespace is already consumed by another candidate ({} differ); rotate split_namespace and registry_path before evaluating a changed candidate", moved.join(", "));
    }
    format!("the sealed test namespace is already consumed, and this candidate has the same config and the same dataset but a different candidate hash ({} became {}). Either the certified toolchain changed - rotate split_namespace and registry_path - or this machine computed different weights from identical inputs: compare the probe of the recorded build to find which primitive moved.", previous["candidate_sha256"], record["candidate_sha256"])
}

/// `claim_sealed_evaluation`: atomically consume a split namespace for exactly one candidate. An identical rerun is allowed.
pub fn claim_evaluation(root: &Path, registry: &str, namespace: &str, candidate: &str, config_sha256: &str, candidate_dataset_sha256: &str) -> Result<Receipt, String> {
    let record = json!({"schema_version": 1, "split_namespace": namespace, "candidate_sha256": candidate, "config_sha256": config_sha256, "candidate_dataset_sha256": candidate_dataset_sha256});
    let expected = canonical(&record);
    let path = root.join(registry);
    if let Some(existing) = create_once(&path, &expected)?
        && existing != expected {
            return Err(refusal(&existing, &record));
        }
    let bytes = read_snapshot(&path)?;
    still(bytes == expected, "sealed evaluation registry changed during claim")?;
    Ok(Receipt { namespace: namespace.to_owned(), candidate_sha256: candidate.to_owned(), config_sha256: config_sha256.to_owned(), candidate_dataset_sha256: candidate_dataset_sha256.to_owned(), registry: registry.to_owned(), registry_sha256: hex(&Sha256::digest(&bytes)), path, bytes })
}

/// `seal-outcome-vN.json` beside `seal-registry-vN.json`.
pub fn outcome_path(registry: &str) -> Result<String, String> {
    if !registry.contains("seal-registry-") {
        return Err(format!("unexpected registry path {registry:?}"));
    }
    Ok(registry.replace("seal-registry-", "seal-outcome-"))
}

/// `claim_sealed_outcome`: the answer of the sealed test can be given once. It is claimed before the metrics are written
/// anywhere, unconditionally, and holds digests only.
pub fn claim_outcome(root: &Path, registry: &str, namespace: &str, outcome: &str, candidate: &str) -> Result<(), String> {
    let expected = canonical(&json!({"schema_version": 1, "split_namespace": namespace, "candidate_sha256": candidate, "sealed_outcome_sha256": outcome}));
    let Some(existing) = create_once(&root.join(outcome_path(registry)?), &expected)? else { return Ok(()) };
    if existing == expected {
        return Ok(());
    }
    let previous: Value = serde_json::from_slice(&existing).map_err(|_| "the sealed outcome ledger is unreadable")?;
    if previous["candidate_sha256"].as_str() != Some(candidate) {
        return Err("the sealed outcome ledger belongs to a different candidate; rotate split_namespace before evaluating a changed candidate".to_owned());
    }
    Err(format!("this run computed a different answer from the sealed test than the run that consumed it: the inputs match but {} became {outcome}. Something outside the sealed inputs changed the result; compare the environment probe before trusting either number.", previous["sealed_outcome_sha256"]))
}

/// `sealed_evaluation_evidence_is_valid`: a manifest's receipt against the ledger of the tree.
#[must_use]
pub fn receipt_is_valid(root: &Path, evidence: &Value, config: &Value, config_sha256: &str, candidate_dataset_sha256: &str, candidate: &str) -> bool {
    let expected_keys = ["schema_version", "split_namespace", "candidate_sha256", "config_sha256", "candidate_dataset_sha256", "registry_path", "registry_sha256"];
    let Some(object) = evidence.as_object() else { return false };
    if object.len() != expected_keys.len() || !expected_keys.iter().all(|key| object.contains_key(*key)) {
        return false;
    }
    let sealed = &config["sealed_evaluation"];
    let (Some(namespace), Some(registry)) = (sealed["split_namespace"].as_str(), sealed["registry_path"].as_str()) else { return false };
    if evidence["schema_version"] != json!(1) || evidence["split_namespace"].as_str() != Some(namespace) || evidence["candidate_sha256"].as_str() != Some(candidate) || evidence["registry_path"].as_str() != Some(registry) || evidence["config_sha256"].as_str() != Some(config_sha256) || evidence["candidate_dataset_sha256"].as_str() != Some(candidate_dataset_sha256) {
        return false;
    }
    let record = json!({"schema_version": 1, "split_namespace": namespace, "candidate_sha256": candidate, "config_sha256": config_sha256, "candidate_dataset_sha256": candidate_dataset_sha256});
    match read_snapshot(&root.join(registry)) {
        Ok(snapshot) => snapshot == canonical(&record) && evidence["registry_sha256"].as_str() == Some(hex(&Sha256::digest(&snapshot)).as_str()),
        Err(_) => false,
    }
}

/// `runtime_candidate_model_parameters`: the candidate's identity out of a validated KSLM.
#[must_use]
pub fn runtime_parameters(model: &LinearNgramModel) -> Value {
    let directions = |values: [f64; 2]| json!({"0>1": float_hex(values[0]), "1>0": float_hex(values[1])});
    let thresholds: Map<String, Value> = TRIGGERS.iter().enumerate().map(|(index, trigger)| (trigger.as_str().to_owned(), directions(model.threshold_logits[index]))).collect();
    let platt = |index: usize| json!({"scale": float_hex(model.platt_calibration[index].scale), "bias": float_hex(model.platt_calibration[index].bias)});
    json!({"dimension": model.dimension, "payload_sha256": model.payload_sha256, "weight_scale_hex": float_hex(model.weight_scale), "bias_hex": float_hex(model.bias), "platt_calibration_hex": {"0>1": platt(0), "1>0": platt(1)}, "threshold_logits_hex": thresholds, "veto_threshold_hex": float_hex(model.veto_threshold), "feature_hash_seed": model.fnv_seed, "membership_hash_seed": model.membership_seed, "ngram_orders": model.ngram_orders})
}

/// What goes into a KSLM file.
pub struct ModelFile<'a> {
    pub version: &'a str,
    pub quantized: &'a Quantized,
    pub fingerprints: &'a BTreeSet<u64>,
    pub selections: &'a Selections,
    pub veto: &'a Veto,
    pub bias: f64,
    pub calibration: &'a [Platt; 2],
    pub fnv_seed: u64,
    pub membership_seed: u64,
    pub metadata: &'a Value,
}

/// `encode_model`, called with the weights the quantization gives back, as `main` calls it: the quantization is done
/// again here and has to land on the same values.
pub fn encode_model(file: &ModelFile<'_>) -> Result<Vec<u8>, String> {
    let weights: Vec<f64> = file.quantized.values.iter().map(|value| f64::from(*value) * file.quantized.scale).collect();
    let largest = weights.iter().fold(0.0_f64, |largest, value| largest.max(value.abs()));
    let scale = if largest == 0.0 { 1.0 } else { largest / QUANTIZED_LIMIT };
    let mut payload = Vec::with_capacity(weights.len() * 2 + file.fingerprints.len() * 8);
    for weight in &weights {
        let value = ks_text::py_round(weight / scale).clamp(-QUANTIZED_LIMIT, QUANTIZED_LIMIT).to::<i16>();
        payload.extend_from_slice(&value.to_le_bytes());
    }
    for fingerprint in file.fingerprints {
        payload.extend_from_slice(&fingerprint.to_le_bytes());
    }
    let thresholds: Map<String, Value> = TRIGGERS.iter().map(|trigger| { let logits = file.selections[trigger.as_str()].1.logits; (trigger.as_str().to_owned(), json!({"0>1": logits[0], "1>0": logits[1]})) }).collect();
    let platt = |index: usize| json!({"bias": file.calibration[index].intercept, "scale": file.calibration[index].slope});
    let mut manifest = json!({
        "bias": file.bias,
        "dimension": file.quantized.dimension,
        "feature_version": INTENT_FEATURE_VERSION,
        "fnv_seed": file.fnv_seed,
        "format": "KSLM",
        "hash_algorithm": HASH_ALGORITHM,
        "membership_algorithm": MEMBERSHIP_ALGORITHM,
        "membership_seed": file.membership_seed,
        "model_version": file.version,
        "ngram_orders": [1, 2, 3, 4, 5],
        "payload_sha256": hex(&Sha256::digest(&payload)),
        "platt_calibration": {"0>1": platt(0), "1>0": platt(1)},
        "schema": KSLM_SCHEMA_VERSION,
        "supported_fingerprint_count": file.fingerprints.len(),
        "threshold_logits": thresholds,
        "veto_threshold": file.veto.raw_logit,
        "weight_scale": scale,
    });
    manifest["metadata"] = file.metadata.clone();
    let mut manifest_bytes = canonical(&manifest);
    manifest_bytes.pop();
    let mut encoded = Vec::with_capacity(52 + manifest_bytes.len() + payload.len());
    encoded.extend_from_slice(b"KSLM");
    encoded.extend_from_slice(&KSLM_SCHEMA_VERSION.to::<u16>().to_le_bytes());
    encoded.extend_from_slice(&0_u16.to_le_bytes());
    encoded.extend_from_slice(&manifest_bytes.len().to::<u32>().to_le_bytes());
    encoded.extend_from_slice(&payload.len().to::<u32>().to_le_bytes());
    encoded.extend_from_slice(&crc32(&payload).to_le_bytes());
    encoded.extend_from_slice(&Sha256::digest(&manifest_bytes));
    encoded.extend_from_slice(&manifest_bytes);
    encoded.extend_from_slice(&payload);
    LinearNgramModel::from_bytes(encoded.clone(), None).map_err(|error| format!("the encoded model does not load: {}", error.name))?;
    Ok(encoded)
}

/// `publish_bytes_bundle`: every payload staged before the first destination changes; on failure every destination
/// already replaced gets its exact old bytes back. The commit marker goes last.
pub fn publish_bundle(outputs: &[(PathBuf, Vec<u8>)]) -> Result<(), String> {
    let mut previous: Vec<Option<Vec<u8>>> = Vec::new();
    let mut staged: Vec<PathBuf> = Vec::new();
    let clean = |staged: &[PathBuf]| {
        for path in staged {
            let _ = std::fs::remove_file(path);
        }
    };
    for (destination, data) in outputs {
        match std::fs::read(destination) {
            Ok(existing) if existing.len() > BACKUP_MAX_BYTES => {
                clean(&staged);
                return Err(format!("existing publication target exceeds the rollback limit: {}", destination.display()));
            }
            Ok(existing) => previous.push(Some(existing)),
            Err(_) => previous.push(None),
        }
        match stage(destination, data) {
            Ok(path) => staged.push(path),
            Err(error) => {
                clean(&staged);
                return Err(error);
            }
        }
    }
    for (index, (destination, _)) in outputs.iter().enumerate() {
        let replaced = std::fs::rename(&staged[index], destination).map_err(|error| error.to_string()).and_then(|()| sync_folder(destination.parent().expect("a folder")));
        if let Err(error) = replaced {
            let mut failures = Vec::new();
            for back in (0..index).rev() {
                let target = &outputs[back].0;
                let restored = match &previous[back] {
                    None => std::fs::remove_file(target).map_err(|error| error.to_string()),
                    Some(old) => stage(target, old).and_then(|path| std::fs::rename(path, target).map_err(|error| error.to_string())),
                };
                failures.extend(restored.err().map(|failure| format!("; and {} was not put back: {failure}", target.display())));
            }
            clean(&staged);
            return Err(format!("{}: {error}{}", destination.display(), failures.concat()));
        }
    }
    Ok(())
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::intent::fit::{Confusion, Selection};

    fn folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("ks-train-seal-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("a folder");
        folder
    }

    #[test]
    fn a_ledger_that_cannot_be_linked_or_read_again_says_why() {
        let path = folder("ledger").join("seal-registry-v1.json");
        std::fs::write(&path, b"{}").expect("a file");
        assert_eq!(existing_record(&path, &std::io::Error::from(std::io::ErrorKind::AlreadyExists)).expect("the record"), Some(b"{}".to_vec()));
        assert!(existing_record(&path, &std::io::Error::other("no links here")).unwrap_err().contains("cannot create the ledger"));
        still(true, "moved").expect("the same");
        assert_eq!(still(false, "moved").unwrap_err(), "moved");
    }

    #[cfg(not(windows))]
    #[test]
    fn a_folder_that_does_not_open_stops_a_publication() {
        assert!(unopened_folder(&folder("unopened"), &std::io::Error::other("gone")).unwrap_err().contains("gone"));
    }

    #[test]
    fn a_model_of_no_weights_is_encoded_with_the_unit_scale() {
        let quantized = Quantized { dimension: 8, scale: 0.5, values: vec![0; 8], support: vec![0], maximum_absolute_error: 0.0 };
        let selection = Selection { logits: [0.5, 0.5], metrics: Confusion::default(), typo_metrics: Confusion::default(), margin: 0.0 };
        let selections: Selections = TRIGGERS.iter().map(|trigger| (trigger.as_str(), (*trigger, selection))).collect();
        let platt = Platt { slope: 1.0, intercept: 0.0, sample_count: 2, positive_count: 1 };
        let veto = Veto { raw_logit: -1.0, positive_samples: 1, vetoed_positive_samples: 0, false_negative_rate: 0.0 };
        let metadata = json!({"note": "small"});
        let file = ModelFile { version: "intent-v1-small", quantized: &quantized, fingerprints: &BTreeSet::new(), selections: &selections, veto: &veto, bias: 0.0, calibration: &[platt, platt], fnv_seed: 1, membership_seed: 2, metadata: &metadata };
        let model = LinearNgramModel::from_bytes(encode_model(&file).expect("a model"), None).expect("it loads");
        assert!((model.weight_scale - 1.0).abs() < f64::EPSILON);
    }
}
