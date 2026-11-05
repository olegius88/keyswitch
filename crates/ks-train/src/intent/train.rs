//! The real fit of the layout-intent model (`main` of `train_intent_model`): the toolchain fixed first, the candidate
//! sealed before a held-out row is built, the answer of the test fixed before it is written anywhere, then a transactional
//! publication. And the receipt of a candidate the strict evaluation rejected (`write_intent_rejection`).
use super::pipeline::{self, Candidate};
use super::{dataset, probe, seal};
use crate::physical::hex;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const REQUIRED_LICENSE_LINES: [&str; 3] = ["Files: models/*", "Copyright: 2013, 2014, marmuta <marmvta@gmail.com>", "  2011, 2012, Francesco Fumanti <francesco.fumanti@gmx.net>"];
const FAILURE_SECTIONS: [(&str, &str); 10] = [("fallback_regression", "model_vs_fallback"), ("unknown_typo_false_positives", "lexical_disjoint_unknown_typos"), ("unknown_typo_recall", "lexical_disjoint_unknown_typos"), ("sealed_test", "sealed_test"), ("sealed_test_context_stress", "sealed_test_context_stress"), ("production_context_ensemble", "production_context_ensemble"), ("safety", "safety"), ("veto", "veto"), ("load_latency", "performance"), ("inference_latency", "performance")];
const COMPACT_MAX_DEPTH: usize = 3;
const COMPACT_MAX_INLINE_ITEMS: usize = 16;

pub fn pretty_sorted(value: &Value) -> String {
    ks_text::json_dumps(value, &ks_text::JsonOptions { ensure_ascii: false, indent: Some(ks_text::Indent::Spaces(2)), sort_keys: true, ..ks_text::JsonOptions::default() }) + "\n"
}

/// `verify_training_sources`: both corpora and the verbatim license evidence.
pub fn verify_sources(root: &Path, raw_config: &Value) -> Result<(), String> {
    let sources = &raw_config["sources"];
    dataset::read_frozen(root, &sources["languages"]["en_US"])?;
    dataset::read_frozen(root, &sources["languages"]["ru_RU"])?;
    let evidence = dataset::read_frozen(root, &sources["license_evidence"])?;
    let text = String::from_utf8(evidence).map_err(|_| "Onboard license evidence is not valid UTF-8")?;
    let lines: Vec<&str> = ks_text::pystr::splitlines(&text, false);
    let declaration = format!("License: {}", sources["license_declaration"].as_str().unwrap_or(""));
    let missing: Vec<&str> = REQUIRED_LICENSE_LINES.iter().copied().chain([declaration.as_str()]).filter(|line| !lines.contains(line)).collect();
    if !missing.is_empty() {
        return Err(format!("Onboard license evidence is missing required lines: {missing:?}"));
    }
    Ok(())
}

/// The whole manifest, once the sealed sections are in the candidate.
pub fn manifest_of(built: &Candidate, config_sha256: &str, toolchain: &Value, sealed_evaluation: &Value) -> Result<Map<String, Value>, String> {
    let mut manifest = built.sections.clone();
    manifest.insert("config_sha256".to_owned(), json!(config_sha256));
    manifest.insert("toolchain".to_owned(), toolchain.clone());
    manifest.insert("sealed_evaluation".to_owned(), sealed_evaluation.clone());
    let provenance = seal::build_provenance_sha256(&manifest)?;
    manifest.insert("build_provenance_sha256".to_owned(), json!(provenance));
    Ok(manifest)
}

/// The test report: the manifest's sealed sections, without what only the sidecar has to carry.
pub fn report_of(manifest: &Map<String, Value>) -> Value {
    let mut report = Map::new();
    for name in ["model_id", "calibration_scope", "config_sha256", "build_provenance_sha256", "split_namespace", "sealed_evaluation", "gate_policy", "thresholds", "threshold_selection_gate_breakdown", "sealed_test", "sealed_test_typos", "sealed_test_context_stress", "safety", "veto", "quality_gate_breakdown", "quality_gates_passed", "artifact_sha256", "artifact_model_version"] {
        if let Some(value) = manifest.get(name) {
            report.insert(name.to_owned(), value.clone());
        }
    }
    Value::Object(report)
}

/// The model file and the fields of the sidecar that name it.
pub fn seal_artifact(built: &Candidate, manifest: &mut Map<String, Value>) -> Result<Vec<u8>, String> {
    let provenance = manifest["build_provenance_sha256"].as_str().ok_or("a provenance digest")?.to_owned();
    let version = format!("intent-v1-{}", &provenance[..seal::VERSION_HASH_CHARACTERS]);
    let metadata = Value::Object(manifest.clone());
    let encoded = seal::encode_model(&seal::ModelFile { version: &version, quantized: &built.quantized, fingerprints: &built.fingerprints, selections: &built.selections, veto: &built.veto, bias: built.bias, calibration: &built.calibration, fnv_seed: built.config.feature_hash_seed, membership_seed: built.config.membership_hash_seed, metadata: Some(&metadata) })?;
    let loaded = ks_lang::intent_model::LinearNgramModel::from_bytes(encoded.clone(), None).map_err(|error| error.name.to_string())?;
    if seal::runtime_parameters(&loaded) != built.parameters {
        return Err("serialized KSLM parameters differ from the sealed candidate".to_owned());
    }
    manifest.insert("artifact_sha256".to_owned(), json!(hex(&Sha256::digest(&encoded))));
    manifest.insert("artifact_model_version".to_owned(), json!(version));
    Ok(encoded)
}

/// Where the fit writes.
pub struct Outputs {
    pub artifact: PathBuf,
    pub manifest: PathBuf,
    pub report: PathBuf,
    pub environment: PathBuf,
    pub diagnostic: Option<PathBuf>,
    pub dry_run: bool,
}

impl Outputs {
    pub fn defaults(root: &Path) -> Outputs {
        Outputs { artifact: root.join("resources/models/layout_intent_v1.ksm"), manifest: root.join("model/intent_v1/manifest.json"), report: root.join("model/intent_v1/test-report.json"), environment: root.join("model/intent_v1/build-environment.json"), diagnostic: None, dry_run: false }
    }

    /// `validate_training_paths`: outputs must not alias each other, the ledgers or an input.
    fn validate(&self, root: &Path, raw_config: &Value, registry: &str) -> Result<(), String> {
        let mut seen: Vec<PathBuf> = Vec::new();
        let mut mutable = vec![self.artifact.clone(), self.manifest.clone(), self.report.clone(), self.environment.clone(), root.join(registry), root.join(seal::outcome_path(registry)?)];
        mutable.extend(self.diagnostic.clone());
        for path in &mutable {
            if seen.contains(path) {
                return Err("artifact, manifest, report, build environment and the two seal ledgers must be distinct paths".to_owned());
            }
            seen.push(path.clone());
        }
        let sources = &raw_config["sources"];
        let inputs = [root.join("model/intent_v1/config.json"), root.join(sources["languages"]["en_US"]["path"].as_str().ok_or("a path")?), root.join(sources["languages"]["ru_RU"]["path"].as_str().ok_or("a path")?), root.join(sources["license_evidence"]["path"].as_str().ok_or("a path")?), root.join(raw_config["hard_negative_development"]["source"]["path"].as_str().ok_or("a path")?), root.join(seal::preseal_receipt_path(registry)?)];
        if let Some(path) = mutable.iter().find(|path| inputs.contains(path)) {
            return Err(format!("output path aliases an immutable training input: {}", path.display()));
        }
        Ok(())
    }
}

fn compiler_identity() -> String {
    match std::process::Command::new("rustc").arg("--version").output() {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        _ => "absent".to_owned(),
    }
}

/// `TrainingEnvironmentProvenance`: the machine, for the reader rather than for the seal.
fn environment_provenance() -> Map<String, Value> {
    let mut block = Map::new();
    block.insert("schema_version".to_owned(), json!(1));
    block.insert("implementation".to_owned(), json!(seal::IMPLEMENTATION));
    block.insert("trainer_version".to_owned(), json!(env!("CARGO_PKG_VERSION")));
    block.insert("system".to_owned(), json!(std::env::consts::OS));
    block.insert("machine".to_owned(), json!(std::env::consts::ARCH));
    block.insert("byteorder".to_owned(), json!(if cfg!(target_endian = "little") { "little" } else { "big" }));
    block.insert("compiler".to_owned(), json!(compiler_identity()));
    block.insert("environment_probe".to_owned(), probe::measure());
    block
}

/// `main` of the trainer. The manifest goes to standard output; the exit status says whether the gates passed.
pub fn train(root: &Path, workers: usize, outputs: &Outputs) -> Result<i32, String> {
    let config_path = root.join("model/intent_v1/config.json");
    let raw_config = crate::corpus::read_json(&config_path)?;
    let config_sha256 = seal::sha256_file(&config_path)?;
    let registry = raw_config["sealed_evaluation"]["registry_path"].as_str().ok_or("a registry path")?.to_owned();
    let namespace = raw_config["sealed_evaluation"]["split_namespace"].as_str().ok_or("a namespace")?.to_owned();
    if namespace != dataset::SPLIT_NAMESPACE {
        return Err("the config and the trainer name different split namespaces".to_owned());
    }
    let toolchain = seal::toolchain_snapshot(root, &config_sha256, &registry)?;
    // Measured before a single row is featurised and checked again before publication: the probe has to describe the
    // machine that did the work.
    let environment = environment_provenance();
    verify_sources(root, &raw_config)?;
    outputs.validate(root, &raw_config, &registry)?;
    let recheck = || -> Result<(), String> {
        verify_sources(root, &raw_config)?;
        seal::verify_toolchain(root, &toolchain, &config_path, &registry)
    };
    let candidate = pipeline::fit_candidate(root, workers)?;
    if candidate.presealed_gate["passed"] != json!(true) {
        recheck()?;
        let mut diagnostic = candidate.sections.clone();
        diagnostic.insert("schema_version".to_owned(), json!(1));
        diagnostic.insert("phase".to_owned(), json!("presealed_candidate"));
        diagnostic.insert("quality_gates_passed".to_owned(), json!(false));
        diagnostic.insert("config_sha256".to_owned(), json!(config_sha256));
        diagnostic.insert("toolchain".to_owned(), toolchain.clone());
        diagnostic.insert("candidate_dataset_sha256".to_owned(), json!(candidate.candidate_dataset_sha256));
        diagnostic.insert("veto".to_owned(), json!({"selection": candidate.veto.payload()}));
        diagnostic.insert("selection_tail_diagnostics".to_owned(), candidate.diagnostic.clone().unwrap_or(Value::Null));
        diagnostic.insert("presealed_candidate_gate_breakdown".to_owned(), candidate.presealed_gate.clone());
        diagnostic.insert("sealed_test_evaluated".to_owned(), json!(false));
        diagnostic.insert("safety".to_owned(), json!({"guard_audit": candidate.audit.payload()}));
        for name in ["calibration_scope", "source_package", "sources"] {
            diagnostic.remove(name);
        }
        let text = pretty_sorted(&Value::Object(diagnostic));
        if let Some(path) = &outputs.diagnostic {
            seal::publish_bundle(&[(path.clone(), text.clone().into_bytes())])?;
        }
        print!("{text}");
        return Ok(1);
    }
    recheck()?;
    let metadata = candidate.metadata(&config_sha256, &toolchain);
    let candidate_sha256 = seal::candidate_sha256(&namespace, &config_sha256, &candidate.candidate_dataset_sha256, &metadata);
    let receipt = seal::claim_evaluation(root, &registry, &namespace, &candidate_sha256, &config_sha256, &candidate.candidate_dataset_sha256)?;
    receipt.verify()?;
    let (built, gates_passed) = pipeline::open_sealed_test(candidate, root, workers)?;
    recheck()?;
    receipt.verify()?;
    let mut manifest = manifest_of(&built, &config_sha256, &toolchain, &receipt.payload())?;
    // Before the artifact is staged and before a single metric is written anywhere a reader could see it: a rerun that
    // computes a different answer stops here.
    seal::claim_outcome(root, &registry, &namespace, &seal::outcome_sha256(&manifest)?, &candidate_sha256)?;
    if !outputs.dry_run && gates_passed {
        let encoded = seal_artifact(&built, &mut manifest)?;
        recheck()?;
        if probe::measure()["probe_sha256"] != environment["environment_probe"]["probe_sha256"] {
            return Err("the environment changed while this run was in progress; nothing produced across that boundary can be trusted".to_owned());
        }
        receipt.verify()?;
        let mut sidecar = environment.clone();
        sidecar.insert("artifact_model_version".to_owned(), manifest["artifact_model_version"].clone());
        sidecar.insert("build_provenance_sha256".to_owned(), manifest["build_provenance_sha256"].clone());
        sidecar.insert("artifact_sha256".to_owned(), manifest["artifact_sha256"].clone());
        let manifest_bytes = pretty_sorted(&Value::Object(manifest.clone())).into_bytes();
        let report_bytes = pretty_sorted(&report_of(&manifest)).into_bytes();
        seal::publish_bundle(&[(outputs.report.clone(), report_bytes), (outputs.artifact.clone(), encoded), (outputs.manifest.clone(), manifest_bytes), (outputs.environment.clone(), pretty_sorted(&Value::Object(sidecar)).into_bytes())])?;
    }
    print!("{}", pretty_sorted(&Value::Object(manifest)));
    Ok(i32::from(!gates_passed))
}

/// `compact`: scalar evidence and small mappings stay, bulky per-row payloads go.
fn compact(value: &Value, depth: usize) -> Value {
    match value {
        Value::Object(object) if depth >= COMPACT_MAX_DEPTH => {
            let mut keys: Vec<&String> = object.keys().collect();
            keys.sort();
            json!({"omitted_keys": keys})
        }
        Value::Object(object) => Value::Object(object.iter().map(|(key, item)| (key.clone(), compact(item, depth + 1))).collect()),
        Value::Array(items) if items.len() > COMPACT_MAX_INLINE_ITEMS => json!({"omitted_items": items.len()}),
        other => other.clone(),
    }
}

/// `write_intent_rejection`: a candidate whose sealed test was opened but whose strict evaluation failed leaves an
/// immutable, fact-only receipt. Every value is read from the evidence; nothing is typed in by hand.
pub fn rejection_receipt(root: &Path, version: u32, strict_report: &Path, remediation: &str) -> Result<Value, String> {
    let directory = root.join("model/intent_v1");
    let manifest_path = directory.join("manifest.json");
    let registry_path = directory.join(format!("seal-registry-v{version}.json"));
    let artifact_path = root.join("resources/models/layout_intent_v1.ksm");
    let test_report_path = directory.join("test-report.json");
    let (manifest, registry, report) = (crate::corpus::read_json(&manifest_path)?, crate::corpus::read_json(&registry_path)?, crate::corpus::read_json(strict_report)?);
    let gates = report["strict_gates"].as_object().ok_or("strict report lacks strict_gates")?;
    let mut failed: Vec<&String> = gates.iter().filter(|(_, value)| **value != json!(true)).map(|(name, _)| name).collect();
    failed.sort();
    if failed.is_empty() {
        return Err("strict report passed every gate; nothing to reject".to_owned());
    }
    if report["strict_passed"] != json!(false) {
        return Err("strict report does not record strict_passed=false".to_owned());
    }
    let namespace = manifest["split_namespace"].as_str().ok_or("split_namespace must be a string")?;
    if !namespace.contains(&format!(":intent-v{version}:")) {
        return Err(format!("manifest namespace {namespace:?} is not v{version}"));
    }
    let mut failure = Map::new();
    failure.insert("failed_strict_gates".to_owned(), json!(failed));
    for gate in &failed {
        if let Some((_, section)) = FAILURE_SECTIONS.iter().find(|(name, _)| name == gate) {
            if let Some(value) = report.get(*section) {
                failure.insert((*section).to_owned(), compact(value, 0));
            }
        }
    }
    let text = |value: &Value, label: &str| -> Result<String, String> { value.as_str().map(str::to_owned).ok_or_else(|| format!("{label} must be a string")) };
    Ok(json!({
        "schema_version": 1,
        "phase": format!("strict_{}", failed[0]),
        "decision": "rejected",
        "reason": failed.iter().map(|name| name.as_str()).collect::<Vec<_>>().join(","),
        "artifact_published": true,
        "internal_quality_gates_passed": manifest["quality_gates_passed"] == json!(true),
        "independent_external_holdout_evaluated": report.get("lexical_disjoint_unknown_typos").is_some(),
        "model_version": text(&manifest["artifact_model_version"], "model version")?,
        "artifact_sha256": seal::sha256_file(&artifact_path)?,
        "config_sha256": text(&manifest["config_sha256"], "config sha")?,
        "candidate_sha256": text(&registry["candidate_sha256"], "candidate sha")?,
        "candidate_dataset_sha256": text(&registry["candidate_dataset_sha256"], "candidate dataset sha")?,
        "seal_registry_sha256": seal::sha256_file(&registry_path)?,
        "manifest_sha256": seal::sha256_file(&manifest_path)?,
        "internal_report_sha256": seal::sha256_file(&test_report_path)?,
        "evaluator_sha256": seal::digest_files(root, &["crates/ks-train/src/intent/evaluate.rs"])?,
        "strict_report_sha256": seal::sha256_file(strict_report)?,
        "failure": failure,
        "remediation": remediation,
    }))
}
