//! The fast, fail-closed gate a package is built behind: every installed model has its evidence, and no rejected
//! candidate is installed. Reads no test row. Replays are separate (`boundary --verify`, `ortho --verify`, `intent
//! --verify`, `retired <family> --replay`).
use crate::corpus::{checksum, read_json};
use crate::engine_reports::verify_boundary_report;
use crate::retired;
use serde_json::{Map, Value, json};
use std::path::Path;

const SEALED_BEFORE_TEST: &str = "sealed-before-test";
const ORTHO_DIRECTORY: &str = "model/ortho_v1";
const ORTHO_EVIDENCE_SCHEMA: i64 = 2;
/// A pin of a resource under the path it had in the Python tree.
const OLD_RESOURCE_PREFIX: &str = "src/keyswitch/resources/";

fn object_with_schema(path: &Path, schema: i64) -> Result<Map<String, Value>, String> {
    let value = retired::read_object(path)?;
    if value.get("schema_version").and_then(Value::as_i64) != Some(schema) {
        return Err(format!("invalid metadata: {}", path.display()));
    }
    Ok(value)
}

fn digest_of(table: &Map<String, Value>, field: &str) -> Option<String> {
    table.get(field).and_then(Value::as_str).map(str::to_owned)
}

/// The accepted boundary model: the candidate its report accepted is the one installed, and it restores the regressions.
pub fn boundary(root: &Path) -> Result<Value, String> {
    let directory = root.join("model/boundary_v2");
    let candidate = checksum(&directory.join("candidate.json"))?;
    let (seal, report) = (read_json(&directory.join("seal.json"))?, read_json(&directory.join("report.json"))?);
    if seal["candidate_sha256"].as_str() != Some(candidate.as_str()) || report["candidate_sha256"].as_str() != Some(candidate.as_str()) || report["seal_sha256"].as_str() != Some(checksum(&directory.join("seal.json"))?.as_str()) {
        return Err("boundary-v2 sealed quality evidence changed".to_owned());
    }
    if report["accepted"] != json!(true) {
        return Err("boundary-v2 was rejected by its test".to_owned());
    }
    if checksum(&root.join("resources/models/boundary-v2.json"))? != candidate {
        return Err("approved boundary-v2 model missing or changed".to_owned());
    }
    verify_boundary_report(&read_json(&root.join("model/boundary_v1/engine-regression.json"))?)?;
    Ok(json!({"model_version": read_json(&directory.join("candidate.json"))?["version"], "accepted": true, "test": report["test"]}))
}

/// The orthotactic model: its corpus, dictionary verdict, word lists and independent test still describe the installed
/// candidate. Pins of the tools that wrote it are not in this tree; pins of data are compared.
pub fn ortho(root: &Path) -> Result<Value, String> {
    let directory = root.join(ORTHO_DIRECTORY);
    let tokens = checksum(&directory.join("tokens.jsonl.gz"))?;
    let receipt = object_with_schema(&directory.join("corpus-receipt.json"), ORTHO_EVIDENCE_SCHEMA)?;
    if digest_of(&receipt, "tokens_sha256").as_deref() != Some(tokens.as_str()) {
        return Err("key-space corpus does not match its receipt".to_owned());
    }
    let known = object_with_schema(&directory.join("known-receipt.json"), 1)?;
    if digest_of(&known, "evidence_sha256").as_deref() != Some(checksum(&directory.join("known-evidence.json.gz"))?.as_str()) || digest_of(&known, "tokens_sha256").as_deref() != Some(tokens.as_str()) {
        return Err("dictionary evidence changed".to_owned());
    }
    for (receipt, forms, schema) in [("web-lexicon-receipt.json", "web-lexicon-ru.json.gz", 1), ("prose-lexicon-receipt.json", "prose-lexicon-ru.json.gz", 1)] {
        if digest_of(&object_with_schema(&directory.join(receipt), schema)?, "forms_sha256").as_deref() != Some(checksum(&directory.join(forms))?.as_str()) {
            return Err(format!("{forms} does not match its receipt"));
        }
    }
    let candidate = checksum(&directory.join("candidate.json"))?;
    let seal = object_with_schema(&directory.join("seal.json"), ORTHO_EVIDENCE_SCHEMA)?;
    if seal.get("stage").and_then(Value::as_str) != Some(SEALED_BEFORE_TEST) || digest_of(&seal, "candidate_sha256").as_deref() != Some(candidate.as_str()) {
        return Err("orthotactic seal changed".to_owned());
    }
    let pins = seal.get("provenance").and_then(Value::as_object).ok_or("a seal without provenance")?;
    for (name, digest) in pins {
        let path = if let Some(rest) = name.strip_prefix(OLD_RESOURCE_PREFIX) {
            root.join("resources").join(rest)
        } else if name.starts_with("model/") {
            root.join(name)
        } else {
            continue;
        };
        if digest.as_str() != Some(checksum(&path)?.as_str()) {
            return Err(format!("orthotactic provenance changed: {name}"));
        }
    }
    let config = read_json(&directory.join("config.json"))?;
    if seal.get("config") != Some(&config) {
        return Err("orthotactic configuration changed after the seal".to_owned());
    }
    let report = object_with_schema(&directory.join("report.json"), ORTHO_EVIDENCE_SCHEMA)?;
    if digest_of(&report, "seal_sha256").as_deref() != Some(checksum(&directory.join("seal.json"))?.as_str()) || digest_of(&report, "candidate_sha256").as_deref() != Some(candidate.as_str()) || report.get("thresholds") != seal.get("thresholds") {
        return Err("orthotactic report does not describe this candidate".to_owned());
    }
    if report.get("promotion_passed") != Some(&json!(true)) {
        return Err("orthotactic candidate did not pass its promotion gates".to_owned());
    }
    if let Some(independent) = report.get("independent_test").filter(|value| !value.is_null()) {
        let folder = directory.join("independent");
        let recorded = read_json(&folder.join("receipt.json"))?;
        if recorded["tokens_sha256"].as_str() != Some(checksum(&folder.join("tokens.jsonl.gz"))?.as_str()) || recorded["evidence_sha256"].as_str() != Some(checksum(&folder.join("known-evidence.json.gz"))?.as_str()) || recorded["snapshot_tokens_sha256"].as_str() != Some(tokens.as_str()) {
            return Err("independent test files do not match their receipt".to_owned());
        }
        if independent["set"] != recorded["set"] || independent["receipt_sha256"].as_str() != Some(checksum(&folder.join("receipt.json"))?.as_str()) || independent["tokens_sha256"] != recorded["tokens_sha256"] || independent["evidence_sha256"] != recorded["evidence_sha256"] || independent["source"] != recorded["source"] {
            return Err("orthotactic report does not describe the frozen independent test".to_owned());
        }
        let counts = report.get("corpus_test").and_then(|test| test.get("counts")).and_then(Value::as_object).ok_or("no test counts")?;
        let population: Map<String, Value> = counts.iter().filter(|(name, _)| name.ends_with("negative_types") || name.ends_with("positive_types")).map(|(name, value)| (name.clone(), value.clone())).collect();
        if recorded.get("types") != Some(&Value::Object(population)) {
            return Err("orthotactic report does not describe the frozen independent test".to_owned());
        }
    }
    let counts = report.get("corpus_test").and_then(|test| test.get("counts")).ok_or("no test counts")?;
    let summed = |field: &str| ["ru", "en"].iter().map(|script| counts.get(format!("{script}:{field}")).and_then(Value::as_i64).unwrap_or(0)).sum::<i64>();
    let (negatives, false_total) = (summed("negative_types"), summed("false_types"));
    let promotion = &config["promotion"];
    if negatives < promotion["minimum_test_negatives"].as_i64().unwrap_or(i64::MAX) {
        return Err("orthotactic test population is too small to bound the risk".to_owned());
    }
    if false_total > promotion["maximum_test_false_conversions"].as_i64().unwrap_or(-1) {
        return Err("orthotactic test reports false conversions".to_owned());
    }
    if report.get("recall").and_then(Value::as_f64).unwrap_or(0.0) < promotion["minimum_recall"].as_f64().unwrap_or(1.0) {
        return Err("orthotactic recall is below the promotion gate".to_owned());
    }
    let active = root.join("resources/models/ortho_v1.json");
    if checksum(&active)? != candidate {
        return Err("installed orthotactic model is not the evaluated candidate".to_owned());
    }
    if read_json(&active)?["version"] != seal.get("model_version").cloned().unwrap_or(Value::Null) {
        return Err("installed orthotactic identity differs from the seal".to_owned());
    }
    Ok(json!({"model_version": seal["model_version"], "artifact_sha256": candidate, "thresholds": seal["thresholds"], "recall": report["recall"], "test_negatives": negatives, "test_false_conversions": false_total}))
}

/// The layout-intent model: the sealed file is the one its manifest names, and the packaged file is the sealed one without
/// its table of fingerprints, to the byte, with the record that names both (`intent::pack`).
pub fn intent(root: &Path) -> Result<Value, String> {
    let (sealed, packaged) = crate::intent::pack::packaged_from_sealed(root)?;
    let manifest = read_json(&root.join("model/intent_v1/manifest.json"))?;
    if manifest["artifact_sha256"].as_str() != Some(sealed.as_str()) {
        return Err("the sealed layout-intent model is not the one its manifest names".to_owned());
    }
    Ok(json!({"model_version": manifest["artifact_model_version"], "sealed_sha256": sealed, "packaged_sha256": packaged}))
}

pub fn run(root: &Path) -> Result<Value, String> {
    let resources = root.join("resources/models");
    let receipt = read_json(&root.join("model/context_v3/release-receipt.json"))?;
    let (context, prefix) = (resources.join("context_policy_v1.json"), resources.join("prefix_policy_v1.json"));
    // A pair sealed by the Python trainer pins sources this tree no longer has: its numbers and evidence are still checked.
    if receipt["provenance"].get("Cargo.lock").is_some() {
        crate::receipt::validate_receipt(&receipt, &context, root, &prefix, &resources)?;
    } else {
        crate::receipt::validate_evidence(&receipt, &context, root, &prefix, &resources)?;
    }
    Ok(json!({
        "context": {"model_version": receipt["model_version"], "scope": receipt["scope"]},
        "boundary": boundary(root)?,
        "orthotactic": ortho(root)?,
        "intent": intent(root)?,
        "rejected": {
            "context-v2": retired::context_v2::verify(root)?["candidate"],
            "boundary-v1": retired::boundary_v1::verify(root)?["accepted"],
            "ortho-v2": retired::ortho_v2::verify(root)?["model_version"],
        },
        "prefix-v1": retired::prefix_v1::verify(root)?["model_version"],
    }))
}
