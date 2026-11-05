//! The independent strict evaluation of a layout-intent model (`evaluate_intent_model`): provenance against the frozen
//! inputs, every first-party sealed record recomputed through the runtime, the external corpora, and the real detector at
//! every reachable context extreme.
use ks_cast::To as _;
use super::dataset::{self, DatasetBundle, LexicalExample, Split};
use super::external::{self, ExternalCorpus, SealedSignatureIndex, Snapshot};
use super::fit::{self, Confusion, FitConfig, Selection, Selections, Veto, WILSON_95_Z, is_typo_kind, metrics_payload, wilson_upper};
use super::pipeline::{self, SafetyAudit};
use super::{probe, seal};
use crate::artifact::canonical;
use crate::physical::hex;
use ks_lang::detector::{DecideOptions, IntentClassifier, LanguageDetector, LanguageScorer, ProtectedTokens};
use ks_lang::intent_model::{CorrectionTrigger, IntentModelInput, LinearNgramModel, LinearPrediction, TRIGGERS};
use ks_lang::language_model::{LanguageModel, WordScore};
use ks_lang::{LoadError, constants as lang};
use ks_text::pynum::{float_hex, fsum, py_round_ndigits};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

pub const COMPARISON_SAMPLE: usize = 5_000;
pub const LATENCY_SAMPLE: usize = 5_000;
const LOAD_REPETITIONS: usize = 11;
const WARMUP_PREDICTIONS: usize = 100;
const LATENCY_DECIMALS: i64 = 6;
const LOAD_P95_MAX_MS: f64 = 500.0;
const INFERENCE_P95_MAX_MS: f64 = 10.0;
const RECALL_TOLERANCE: f64 = 0.005;
const RATE_BOUND_TOLERANCE: f64 = 1e-15;
const SNAPSHOT_READS_PER_FILE: i64 = 2;
const EXTERNAL_SCHEMA_VERSION: i64 = 2;
const MANIFEST_SCHEMA_VERSION: i64 = 2;
const CONTEXT_SCORE_MINIMUM: f64 = 0.0;
const CONTEXT_SCORE_MAXIMUM: f64 = 1.0;
const SOURCE_SENTINEL: &str = "\0keyswitch-context-source\0";
const TARGET_SENTINEL: &str = "\0keyswitch-context-target\0";
const CORPORA: [&str; 5] = ["sealed_test", "sealed_test_typos", "unknown_typo", "safety", "source_known"];
const MANIFEST_KEYS: [&str; 31] = ["schema_version", "model_id", "calibration_scope", "config_sha256", "toolchain", "dataset_sha256", "split_namespace", "sealed_evaluation", "source_package", "sources", "counts", "variant_quarantine_sha256", "sealed_variant_quarantine_sha256", "sealed_test_exclusion_signatures_sha256", "training_language_scorer", "gate_policy", "training", "quantization", "calibration", "veto", "thresholds", "threshold_selection_gate_breakdown", "sealed_test", "sealed_test_typos", "sealed_test_context_stress", "safety", "build_provenance_sha256", "quality_gate_breakdown", "quality_gates_passed", "artifact_sha256", "artifact_model_version"];
const MANIFEST_MAPPINGS: [&str; 17] = ["toolchain", "sealed_evaluation", "source_package", "counts", "training_language_scorer", "gate_policy", "training", "quantization", "calibration", "veto", "thresholds", "threshold_selection_gate_breakdown", "sealed_test", "sealed_test_typos", "sealed_test_context_stress", "safety", "quality_gate_breakdown"];
const MANIFEST_DIGESTS: [&str; 7] = ["config_sha256", "dataset_sha256", "variant_quarantine_sha256", "sealed_variant_quarantine_sha256", "sealed_test_exclusion_signatures_sha256", "build_provenance_sha256", "artifact_sha256"];
const LOCALES: [&str; 2] = ["en_US", "ru_RU"];
const BINARY_GATE_KEYS: [&str; 5] = ["passed", "checks", "actual", "false_positive_bound", "limits"];
const BINARY_CHECK_KEYS: [&str; 6] = ["positive_samples", "negative_samples", "precision", "recall", "specificity", "false_positive_rate_upper_bound"];
const METRIC_KEYS: [&str; 10] = ["true_positive", "false_negative", "true_negative", "false_positive", "precision", "recall", "specificity", "false_positive_rate", "false_positive_rate_upper_95", "negative_samples"];
const METRIC_COUNT_KEYS: [&str; 5] = ["true_positive", "false_negative", "true_negative", "false_positive", "negative_samples"];
const LIMIT_KEYS: [&str; 4] = ["minimum_precision", "minimum_recall", "minimum_specificity", "maximum_false_positive_rate_upper_bound"];
const BOUND_KEYS: [&str; 7] = ["method", "multiplicity_correction", "familywise_confidence", "comparisons", "per_comparison_confidence", "z_score", "upper"];
const DIRECTIONS: [&str; 2] = ["0>1", "1>0"];
pub const TRIGGER_NAMES: [&str; 6] = ["boundary_probe", "pause", "space", "enter", "tab", "punctuation"];

fn trigger_names() -> Vec<&'static str> {
    TRIGGERS.iter().map(|trigger| trigger.as_str()).collect()
}

/// `VerificationCheck`.
#[derive(Clone, Debug)]
pub struct Check {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

fn check(name: &str, passed: bool, detail: impl Into<String>) -> Check {
    Check { name: name.to_owned(), passed, detail: detail.into() }
}

impl Check {
    fn payload(&self) -> Value {
        json!({"name": self.name, "passed": self.passed, "detail": self.detail})
    }
}

fn checks_payload(checks: &[Check]) -> Value {
    Value::Array(checks.iter().map(Check::payload).collect())
}

fn is_sha256(value: &Value) -> bool {
    value.as_str().is_some_and(|text| text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
}

/// `_exact_mapping`: the object with exactly these keys, or nothing.
fn exact<'a>(value: Option<&'a Value>, keys: &[&str]) -> Option<&'a Map<String, Value>> {
    let object = value?.as_object()?;
    (object.len() == keys.len() && keys.iter().all(|key| object.contains_key(*key))).then_some(object)
}

fn finite(value: Option<&Value>) -> bool {
    value.and_then(Value::as_f64).is_some_and(f64::is_finite)
}

fn count(value: Option<&Value>) -> Option<i64> {
    value.and_then(Value::as_i64).filter(|number| *number >= 0)
}

fn is_true(value: Option<&Value>) -> bool {
    value == Some(&Value::Bool(true))
}

fn equals_float(value: Option<&Value>, expected: f64) -> bool {
    value.and_then(Value::as_f64) == Some(expected)
}

/// `_binary_gate_evidence_passed`.
fn binary_evidence(value: Option<&Value>, familywise: bool) -> Option<bool> {
    let gate = exact(value, &BINARY_GATE_KEYS)?;
    let checks = exact(gate.get("checks"), &BINARY_CHECK_KEYS)?;
    let actual = exact(gate.get("actual"), &METRIC_KEYS)?;
    let limits = exact(gate.get("limits"), &LIMIT_KEYS)?;
    let bound = exact(gate.get("false_positive_bound"), &BOUND_KEYS)?;
    let (comparisons, confidence, z) = if familywise { (fit::SELECTION_COMPARISONS, fit::SELECTION_PER_COMPARISON_CONFIDENCE, fit::SELECTION_Z) } else { (1, fit::WILSON_CONFIDENCE, WILSON_95_Z) };
    let expected = match (count(actual.get("false_positive")), count(actual.get("negative_samples"))) {
        (Some(positives), Some(negatives)) => wilson_upper(positives, negatives, z),
        _ => return Some(false),
    };
    let upper = bound.get("upper").and_then(Value::as_f64)?;
    Some(
        is_true(gate.get("passed"))
            && BINARY_CHECK_KEYS.iter().all(|key| is_true(checks.get(*key)))
            && METRIC_COUNT_KEYS.iter().all(|key| count(actual.get(*key)).is_some())
            && METRIC_KEYS.iter().filter(|key| !METRIC_COUNT_KEYS.contains(key)).all(|key| finite(actual.get(*key)))
            && LIMIT_KEYS.iter().all(|key| finite(limits.get(*key)))
            && bound.get("method") == Some(&json!("wilson_score_upper_endpoint"))
            && bound.get("multiplicity_correction") == Some(&json!(if familywise { "bonferroni" } else { "none" }))
            && equals_float(bound.get("familywise_confidence"), fit::WILSON_CONFIDENCE)
            && bound.get("comparisons").and_then(Value::as_i64) == Some(comparisons)
            && equals_float(bound.get("per_comparison_confidence"), confidence)
            && equals_float(bound.get("z_score"), z)
            && upper.is_finite()
            && (upper - expected).abs() <= RATE_BOUND_TOLERANCE
            && limits.get("maximum_false_positive_rate_upper_bound").and_then(Value::as_f64).is_some_and(|limit| expected <= limit),
    )
}

/// `_context_gate_evidence_passed`.
fn context_evidence(value: Option<&Value>, familywise: bool) -> Option<bool> {
    let root = exact(value, &["passed", "all_profiles_present", "expected_profiles", "profiles"])?;
    let mut expected: Vec<&str> = fit::STRESS_PROFILES.iter().map(|profile| profile.0).collect();
    expected.sort_unstable();
    if !is_true(root.get("passed")) || !is_true(root.get("all_profiles_present")) || root.get("expected_profiles") != Some(&json!(expected)) {
        return Some(false);
    }
    let profiles = exact(root.get("profiles"), &expected)?;
    for (name, delta, selector) in fit::STRESS_PROFILES {
        let profile = exact(profiles.get(name), &["passed", "delta", "group_selector", "all_triggers_present", "per_trigger"])?;
        if !is_true(profile.get("passed")) || !is_true(profile.get("all_triggers_present")) || !equals_float(profile.get("delta"), delta) || profile.get("group_selector") != Some(&json!(selector)) {
            return Some(false);
        }
        let per_trigger = exact(profile.get("per_trigger"), &TRIGGER_NAMES)?;
        for trigger in TRIGGER_NAMES {
            let gate = exact(per_trigger.get(trigger), &["passed", "overall", "typos"])?;
            if !is_true(gate.get("passed")) || !binary_evidence(gate.get("overall"), familywise)? || !binary_evidence(gate.get("typos"), familywise)? {
                return Some(false);
            }
        }
    }
    Some(true)
}

/// `_selection_trigger_evidence_passed`.
fn selection_trigger_evidence(value: Option<&Value>, maximum: i64) -> Option<bool> {
    let item = exact(value, &["passed", "logits", "overall", "typos", "false_positive_budget"])?;
    let logits = exact(item.get("logits"), &DIRECTIONS)?;
    let budget = exact(item.get("false_positive_budget"), &["passed", "checks", "actual", "maximum_false_positives_per_trigger"])?;
    let checks = exact(budget.get("checks"), &["overall", "typos"])?;
    let actual = exact(budget.get("actual"), &["overall_false_positives", "typo_false_positives"])?;
    let overall = exact(item.get("overall"), &BINARY_GATE_KEYS)?;
    let overall_actual = exact(overall.get("actual"), &METRIC_KEYS)?;
    let typos = exact(item.get("typos"), &BINARY_GATE_KEYS)?;
    let typo_actual = exact(typos.get("actual"), &METRIC_KEYS)?;
    let (overall_fp, typo_fp) = (count(actual.get("overall_false_positives")), count(actual.get("typo_false_positives")));
    Some(
        is_true(item.get("passed"))
            && DIRECTIONS.iter().all(|direction| finite(logits.get(*direction)))
            && binary_evidence(item.get("overall"), true)?
            && binary_evidence(item.get("typos"), true)?
            && is_true(budget.get("passed"))
            && is_true(checks.get("overall"))
            && is_true(checks.get("typos"))
            && overall_fp.is_some()
            && typo_fp.is_some()
            && actual.get("overall_false_positives") == overall_actual.get("false_positive")
            && actual.get("typo_false_positives") == typo_actual.get("false_positive")
            && budget.get("maximum_false_positives_per_trigger").and_then(Value::as_i64) == Some(maximum)
            && overall_fp? <= maximum
            && typo_fp? <= maximum,
    )
}

/// `_threshold_gate_evidence_passed`.
fn threshold_evidence(value: Option<&Value>, maximum: i64) -> bool {
    let walk = || -> Option<bool> {
        let root = exact(value, &["passed", "all_triggers_present", "per_trigger", "neutral", "context_stress"])?;
        let per_trigger = exact(root.get("per_trigger"), &TRIGGER_NAMES)?;
        let neutral = exact(root.get("neutral"), &["passed", "all_triggers_present", "per_trigger"])?;
        let neutral_triggers = exact(neutral.get("per_trigger"), &TRIGGER_NAMES)?;
        let mut every = is_true(root.get("passed")) && is_true(root.get("all_triggers_present")) && is_true(neutral.get("passed")) && is_true(neutral.get("all_triggers_present")) && per_trigger == neutral_triggers;
        for trigger in TRIGGER_NAMES {
            every = every && selection_trigger_evidence(per_trigger.get(trigger), maximum)?;
        }
        Some(every && context_evidence(root.get("context_stress"), true)?)
    };
    walk().unwrap_or(false)
}

/// `_quality_gate_evidence_passed`.
fn quality_evidence(value: Option<&Value>) -> bool {
    let walk = || -> Option<bool> {
        let root = exact(value, &["passed", "all_triggers_present", "sealed_test", "sealed_test_typos", "sealed_test_context_stress", "safety", "veto"])?;
        let sealed_slice = |name: &str| -> Option<bool> {
            let section = exact(root.get(name), &["passed", "per_trigger"])?;
            let per_trigger = exact(section.get("per_trigger"), &TRIGGER_NAMES)?;
            let mut every = is_true(section.get("passed"));
            for trigger in TRIGGER_NAMES {
                every = every && binary_evidence(per_trigger.get(trigger), false)?;
            }
            Some(every)
        };
        let safety = exact(root.get("safety"), &["passed", "actual_guard_failures", "maximum_guard_failures"])?;
        let veto = exact(root.get("veto"), &["passed", "selection", "sealed_test", "maximum_false_negative_rate"])?;
        let veto_slice = |name: &str| -> Option<bool> {
            let section = exact(veto.get(name), &["passed", "raw_logit", "positive_samples", "vetoed_positive_samples", "false_negative_rate"])?;
            Some(is_true(section.get("passed")) && finite(section.get("raw_logit")) && count(section.get("positive_samples")).is_some_and(|samples| samples > 0) && count(section.get("vetoed_positive_samples")).is_some() && finite(section.get("false_negative_rate")))
        };
        Some(is_true(root.get("passed")) && is_true(root.get("all_triggers_present")) && sealed_slice("sealed_test")? && sealed_slice("sealed_test_typos")? && context_evidence(root.get("sealed_test_context_stress"), false)? && is_true(safety.get("passed")) && count(safety.get("actual_guard_failures")).is_some() && count(safety.get("maximum_guard_failures")).is_some() && is_true(veto.get("passed")) && finite(veto.get("maximum_false_negative_rate")) && veto_slice("selection")? && veto_slice("sealed_test")?)
    };
    walk().unwrap_or(false)
}

/// `_runtime_decision_parameters_match`: shipped thresholds and veto bound to every signed selection record.
fn runtime_parameters_match(model: &LinearNgramModel, manifest: &Value, config: &FitConfig) -> bool {
    let walk = || -> Option<bool> {
        let thresholds = exact(manifest.get("thresholds"), &TRIGGER_NAMES)?;
        let gate = exact(manifest.get("threshold_selection_gate_breakdown"), &["passed", "all_triggers_present", "per_trigger", "neutral", "context_stress"])?;
        let selections = exact(gate.get("per_trigger"), &TRIGGER_NAMES)?;
        let mut margin: Option<f64> = None;
        for (index, trigger) in TRIGGER_NAMES.iter().enumerate() {
            let threshold = exact(thresholds.get(*trigger), &["global_logit_margin", "logits", "confidences", "selection_metrics", "selection_typo_metrics"])?;
            let value = threshold.get("global_logit_margin").and_then(Value::as_f64).filter(|value| value.is_finite())?;
            match margin {
                Some(previous) if previous != value => return Some(false),
                _ => margin = Some(value),
            }
            let selection = exact(selections.get(*trigger), &["passed", "logits", "overall", "typos", "false_positive_budget"])?;
            let logits = exact(threshold.get("logits"), &DIRECTIONS)?;
            let confidences = exact(threshold.get("confidences"), &DIRECTIONS)?;
            let selection_logits = exact(selection.get("logits"), &DIRECTIONS)?;
            for (direction_index, direction) in DIRECTIONS.iter().enumerate() {
                let (Some(logit), Some(confidence)) = (logits.get(*direction).and_then(Value::as_f64), confidences.get(*direction).and_then(Value::as_f64)) else { return Some(false) };
                if !logit.is_finite() || !confidence.is_finite() || selection_logits.get(*direction).and_then(Value::as_f64) != Some(logit) || logit != model.threshold_logits[index][direction_index] || confidence != model.thresholds[index][direction_index] {
                    return Some(false);
                }
            }
        }
        let margin = margin?;
        if margin < 0.0 || margin > config.margin_cap {
            return Some(false);
        }
        let veto = exact(manifest.get("veto"), &["selection", "sealed_test"])?;
        let veto_selection = exact(veto.get("selection"), &["raw_logit", "positive_samples", "vetoed_positive_samples", "false_negative_rate"])?;
        let quality = exact(manifest.get("quality_gate_breakdown"), &["passed", "all_triggers_present", "sealed_test", "sealed_test_typos", "sealed_test_context_stress", "safety", "veto"])?;
        let quality_veto = exact(quality.get("veto"), &["passed", "selection", "sealed_test", "maximum_false_negative_rate"])?;
        let quality_selection = exact(quality_veto.get("selection"), &["passed", "raw_logit", "positive_samples", "vetoed_positive_samples", "false_negative_rate"])?;
        let raw = veto_selection.get("raw_logit").and_then(Value::as_f64)?;
        Some(raw.is_finite() && quality_selection.get("raw_logit").and_then(Value::as_f64) == Some(raw) && raw == model.veto_threshold)
    };
    walk().unwrap_or(false)
}

/// `_validate_manifest_schema`: every unknown schema is refused before provenance is evaluated.
fn validate_manifest(manifest: &Value) -> Result<(), String> {
    let object = exact(Some(manifest), &MANIFEST_KEYS).ok_or("the manifest does not have the keys of this schema")?;
    if object["schema_version"].as_i64() != Some(MANIFEST_SCHEMA_VERSION) {
        return Err("unsupported model manifest schema".to_owned());
    }
    if object["model_id"].as_str() != Some(seal::MODEL_ID) {
        return Err("manifest.model_id is unsupported".to_owned());
    }
    for field in MANIFEST_DIGESTS {
        if !is_sha256(&object[field]) {
            return Err(format!("manifest.{field} must be exact lowercase SHA-256"));
        }
    }
    for field in MANIFEST_MAPPINGS {
        if !object[field].is_object() {
            return Err(format!("manifest.{field} must be an object"));
        }
    }
    if !object["sources"].is_array() {
        return Err("manifest.sources must be an array".to_owned());
    }
    for field in ["calibration_scope", "split_namespace", "artifact_model_version"] {
        if object[field].as_str().is_none_or(str::is_empty) {
            return Err(format!("manifest.{field} must not be empty"));
        }
    }
    if !object["quality_gates_passed"].is_boolean() {
        return Err("manifest.quality_gates_passed must be a boolean".to_owned());
    }
    Ok(())
}

/// What the provenance checks read besides the manifest.
pub struct ProvenanceInputs<'a> {
    pub root: &'a Path,
    pub artifact: &'a Path,
    pub config_path: &'a Path,
    pub raw_config: &'a Value,
    pub config: &'a FitConfig,
    pub candidate: &'a DatasetBundle,
    pub dataset: Option<&'a DatasetBundle>,
    pub scorer: &'a Value,
    pub hard_negative: &'a Value,
    pub protected: &'a ProtectedTokens,
}

/// The digests of the toolchain block, against what is in the tree. A manifest of the Python trainer names sources that are
/// not here: its code digests can only be read, its data files still compared.
fn toolchain_checks(root: &Path, toolchain: &Value, registry: &str) -> Result<Vec<Check>, String> {
    let mut checks = Vec::new();
    let rust = toolchain["implementation"].as_str() == Some(seal::IMPLEMENTATION);
    let mut compare = |field: &str, current: Result<String, String>, data_file: bool| -> Result<(), String> {
        let recorded = &toolchain[field];
        if !is_sha256(recorded) {
            return Err(format!("manifest.toolchain.{field} must be exact lowercase SHA-256"));
        }
        let name = format!("toolchain_{field}");
        if rust || data_file {
            let current = current?;
            let recorded = recorded.as_str().expect("a digest");
            checks.push(check(&name, recorded == current, format!("current={current}, manifest={recorded}")));
        } else {
            checks.push(check(&name, true, format!("written by the Python trainer, its sources are not in this tree; manifest={}", recorded.as_str().expect("a digest"))));
        }
        Ok(())
    };
    for (field, files) in seal::TOOLCHAIN_FILES {
        if field == "spellcheck_sha256" {
            compare("preseal_receipt_sha256", seal::preseal_receipt_path(registry).and_then(|path| seal::sha256_file(&root.join(path))), true)?;
        }
        compare(field, seal::digest_files(root, files), field == "protected_tokens_sha256")?;
    }
    let constants: Vec<&str> = seal::CONSTANTS_FILES.iter().copied().filter(|file| root.join(file).is_file()).collect();
    compare("constants_sha256", seal::digest_files(root, &constants), false)?;
    Ok(checks)
}

/// `verify_provenance`.
pub fn verify_provenance(model: &LinearNgramModel, manifest: &Value, inputs: &ProvenanceInputs<'_>) -> Result<Vec<Check>, String> {
    validate_manifest(manifest)?;
    let ProvenanceInputs { root, artifact, config_path, raw_config, config, candidate, dataset, scorer, hard_negative, protected } = *inputs;
    let manifest_object = manifest.as_object().expect("validated");
    let artifact_digest = seal::sha256_file(artifact)?;
    let config_digest = seal::sha256_file(config_path)?;
    let candidate_digest = dataset::dataset_fingerprint(candidate);
    let dataset_digest = dataset.map(dataset::dataset_fingerprint);
    let namespace = raw_config["sealed_evaluation"]["split_namespace"].as_str().ok_or("a namespace")?;
    let registry = raw_config["sealed_evaluation"]["registry_path"].as_str().ok_or("a registry path")?;
    let source_hash = |group: i64| manifest["sources"].as_array().into_iter().flatten().find(|row| row["group"] == json!(group)).and_then(|row| row["sha256"].as_str()).unwrap_or("");
    let languages = &raw_config["sources"]["languages"];
    let english = seal::sha256_file(&root.join(languages["en_US"]["path"].as_str().ok_or("a path")?))?;
    let russian = seal::sha256_file(&root.join(languages["ru_RU"]["path"].as_str().ok_or("a path")?))?;
    let without_artifact: Map<String, Value> = manifest_object.iter().filter(|(key, _)| !matches!(key.as_str(), "artifact_sha256" | "artifact_model_version")).map(|(key, value)| (key.clone(), value.clone())).collect();
    let calculated = seal::build_provenance_sha256(manifest_object)?;
    let recorded = manifest["build_provenance_sha256"].as_str().ok_or("a provenance digest")?;
    let version = manifest["artifact_model_version"].as_str().ok_or("a version")?;
    let expected_version = format!("intent-v1-{}", &calculated[..seal::VERSION_HASH_CHARACTERS]);
    let parameters = seal::runtime_parameters(model);
    let audit = SafetyAudit::of(&candidate.safety, protected);
    let gate_policy = pipeline::gate_policy(raw_config, config);
    let examples: Map<String, Value> = Split::PRESEALED.iter().map(|split| (split.as_str().to_owned(), json!(candidate.split(*split).len()))).collect();
    let counts = json!({"examples": examples, "safety_examples": candidate.safety.len(), "quarantined_variant_occurrences": candidate.variant_quarantine.occurrences.len(), "quarantined_physical_signatures": candidate.variant_quarantine.signature_count()});
    let metadata = seal::metadata_projection(&config_digest, namespace, &manifest["toolchain"], &manifest["source_package"], &manifest["sources"], &counts, &candidate.variant_quarantine.sha256, scorer, &gate_policy, &manifest["training"], &manifest["quantization"], &manifest["calibration"], &manifest["veto"]["selection"], &manifest["thresholds"], &manifest["threshold_selection_gate_breakdown"], &audit.payload(), &parameters);
    let candidate_sha = seal::candidate_sha256(namespace, &config_digest, &candidate_digest, &metadata);
    let sealed_valid = seal::receipt_is_valid(root, &manifest["sealed_evaluation"], raw_config, &config_digest, &candidate_digest, &candidate_sha);
    let scope_ok = manifest["calibration_scope"].as_str() == Some(seal::SCOPE);
    let mut checks = vec![
        check("artifact_sha256", manifest["artifact_sha256"].as_str() == Some(artifact_digest.as_str()) && model.checksum == artifact_digest, artifact_digest.clone()),
        check("config_sha256", manifest["config_sha256"].as_str() == Some(config_digest.as_str()), config_digest.clone()),
        check("split_namespace", manifest["split_namespace"].as_str() == Some(dataset::SPLIT_NAMESPACE), dataset::SPLIT_NAMESPACE),
        check("sealed_evaluation", sealed_valid, "manifest receipt equals the immutable one-candidate registry"),
        check("sealed_candidate_sha256", sealed_valid, format!("loaded KSLM runtime parameters reproduce the registry candidate {candidate_sha}")),
        check("presealed_candidate_metadata", sealed_valid, "canonical training, quantization, calibration, source, count, threshold, and safety metadata reproduce the registry candidate"),
        check("gate_policy", manifest["gate_policy"] == gate_policy, "manifest gate policy equals the loaded training config"),
        check("threshold_selection_gate", threshold_evidence(manifest.get("threshold_selection_gate_breakdown"), config.selection_max_false_positives), "signed threshold-selection evidence passed"),
        check("training_quality_gate", quality_evidence(manifest.get("quality_gate_breakdown")) && manifest["quality_gates_passed"] == json!(true), "signed training quality evidence passed"),
        check("runtime_decision_parameters", runtime_parameters_match(model, manifest, config), "KSLM thresholds and veto equal every signed selection record"),
        check("training_language_scorer", &manifest["training_language_scorer"] == scorer, "manifest scorer provenance equals reconstructed train-only scorer"),
        check("hard_negative_development", &manifest["training"]["hard_negative_development"] == hard_negative, "manifest hard-negative provenance equals the verified frozen development corpus"),
        check("english_source_sha256", source_hash(0) == english, english.clone()),
        check("russian_source_sha256", source_hash(1) == russian, russian.clone()),
        check("embedded_manifest", model.metadata() == Value::Object(without_artifact), "embedded metadata equals the signed training manifest"),
        check("build_provenance_sha256", calculated == recorded, format!("calculated={calculated}, manifest={recorded}")),
        check("model_version", model.model_version == version && version == expected_version, format!("model={}, sidecar={version}, expected={expected_version}", model.model_version)),
        check("feature_schema", model.dimension == config.dimension.to::<i64>() && model.fnv_seed == config.feature_hash_seed, format!("dimension={}, fnv_seed={}", model.dimension, model.fnv_seed)),
        check("membership_schema", model.membership_seed == config.membership_hash_seed, format!("membership_seed={}, expected={}", model.membership_seed, config.membership_hash_seed)),
        check("calibration_scope", scope_ok, manifest["calibration_scope"].as_str().unwrap_or("").to_owned()),
    ];
    if let Some(digest) = dataset_digest {
        checks.insert(2, check("dataset_sha256", manifest["dataset_sha256"].as_str() == Some(digest.as_str()), digest.clone()));
    }
    checks.extend(toolchain_checks(root, &manifest["toolchain"], registry)?);
    Ok(checks)
}

/// `provenance_checks_pass`: the exact gate set before any sealed-test access. The caller asks for the count it expects.
fn checks_pass(checks: &[Check], expected_count: usize) -> bool {
    let names: BTreeSet<&str> = checks.iter().map(|check| check.name.as_str()).collect();
    checks.len() == expected_count && names.len() == expected_count && checks.iter().all(|check| check.passed)
}

fn map_rows<T: Sync, R: Send>(rows: &[T], workers: usize, work: impl Fn(&T) -> Result<R, String> + Sync) -> Result<Vec<R>, String> {
    let ranges = fit::balanced_ranges(rows.len(), workers.max(1));
    let chunks: Vec<Result<Vec<R>, String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = ranges
            .iter()
            .map(|range| {
                let (range, work) = (range.clone(), &work);
                scope.spawn(move || rows[range].iter().map(work).collect::<Result<Vec<R>, String>>())
            })
            .collect();
        handles.into_iter().map(|handle| handle.join().expect("a row worker")).collect()
    });
    let mut results = Vec::with_capacity(rows.len());
    for chunk in chunks {
        results.extend(chunk?);
    }
    Ok(results)
}

/// `ModelPredictionRow`.
#[derive(Clone, Copy)]
struct PredictionRow<'a> {
    example: &'a LexicalExample,
    should_switch: bool,
    raw_logit: f64,
    coverage: f64,
}

fn load_error(error: LoadError) -> String {
    error.name.to_string()
}

fn predict(model: &LinearNgramModel, example: &LexicalExample) -> Result<LinearPrediction, String> {
    // The feature schema reads no word score (`target_language_score_role` is diagnostic only), so none is computed.
    let score = fit::ignored_score();
    model.predict(&IntentModelInput { original: &example.original, alternative: &example.alternative, source_group: example.source_group, target_group: example.target_group, trigger: example.trigger, source_score: &score, target_score: &score, context_delta: example.context_delta, context_group: example.context_group }).map_err(load_error)
}

fn predict_rows<'a>(model: &LinearNgramModel, examples: &'a [LexicalExample], workers: usize) -> Result<Vec<PredictionRow<'a>>, String> {
    let references: Vec<&'a LexicalExample> = examples.iter().collect();
    map_rows(&references, workers, |example| {
        let prediction = predict(model, example)?;
        Ok(PredictionRow { example, should_switch: prediction.should_switch, raw_logit: prediction.logit, coverage: prediction.coverage })
    })
}

fn confusion(rows: impl Iterator<Item = (bool, bool)>) -> Confusion {
    let mut matrix = Confusion::default();
    for (label, predicted) in rows {
        match (label, predicted) {
            (true, true) => matrix.true_positive += 1,
            (true, false) => matrix.false_negative += 1,
            (false, true) => matrix.false_positive += 1,
            (false, false) => matrix.true_negative += 1,
        }
    }
    matrix
}

/// `prediction_metrics`: every trigger, also those without a row.
fn prediction_metrics<'a, 'b>(rows: impl Iterator<Item = &'a PredictionRow<'b>> + Clone) -> BTreeMap<&'static str, Confusion>
where
    'b: 'a,
{
    TRIGGERS.iter().map(|trigger| (trigger.as_str(), confusion(rows.clone().filter(|row| row.example.trigger == *trigger).map(|row| (row.example.label, row.should_switch))))).collect()
}

fn veto_from(rows: &[PredictionRow<'_>], threshold: f64) -> Result<Veto, String> {
    let positives: Vec<&PredictionRow<'_>> = rows.iter().filter(|row| row.example.label).collect();
    if positives.is_empty() {
        return Err("veto evaluation requires positive examples".to_owned());
    }
    let vetoed = positives.iter().filter(|row| row.raw_logit < threshold).count().to::<i64>();
    Ok(Veto { raw_logit: threshold, positive_samples: positives.len().to::<i64>(), vetoed_positive_samples: vetoed, false_negative_rate: vetoed.to::<f64>() / positives.len().to::<f64>() })
}

fn typo_rows<'a, 'b>(rows: &'a [PredictionRow<'b>]) -> impl Iterator<Item = &'a PredictionRow<'b>> + Clone {
    rows.iter().filter(|row| is_typo_kind(&row.example.variant_kind))
}

/// `evaluate_runtime_threshold_selection`: the pre-sealed selection evidence through the runtime.
fn runtime_threshold_selection(model: &LinearNgramModel, examples: &[LexicalExample], config: &FitConfig, workers: usize) -> Result<Value, String> {
    let rows = predict_rows(model, examples, workers)?;
    let overall = prediction_metrics(rows.iter());
    let typos = prediction_metrics(typo_rows(&rows));
    let selections: Selections = TRIGGERS.iter().enumerate().map(|(index, trigger)| { let logits = model.threshold_logits[index]; (trigger.as_str(), (*trigger, Selection { logits, metrics: overall[trigger.as_str()], typo_metrics: typos[trigger.as_str()], margin: logits[0].max(logits[1]) })) }).collect();
    Ok(fit::selection_gate(config, &selections))
}

struct InternalEvidence<'a> {
    test_rows: Vec<PredictionRow<'a>>,
    test_metrics: BTreeMap<&'static str, Confusion>,
    typo_metrics: BTreeMap<&'static str, Confusion>,
    safety_rows: Vec<PredictionRow<'a>>,
    safety_metrics: BTreeMap<&'static str, Confusion>,
    selection_gate: Value,
    checks: Vec<Check>,
}

/// `recompute_internal_sealed_evidence`: every first-party sealed record, recomputed and compared exactly.
fn internal_evidence<'a>(model: &LinearNgramModel, manifest: &Value, config: &FitConfig, prepared: &dataset::PreparedLexicon, whole: &'a DatasetBundle, protected: &ProtectedTokens, workers: usize) -> Result<InternalEvidence<'a>, String> {
    let selection_gate = runtime_threshold_selection(model, whole.split(Split::Threshold), config, workers)?;
    let calibration = predict_rows(model, whole.split(Split::Calibration), workers)?;
    let selection_veto = veto_from(&calibration, model.veto_threshold)?;
    drop(calibration);
    fit::verify_context_invariance(config)?;
    let test_rows = predict_rows(model, whole.split(Split::Test), workers)?;
    let test_metrics = prediction_metrics(test_rows.iter());
    let typo_metrics = prediction_metrics(typo_rows(&test_rows));
    let audit = SafetyAudit::of(&whole.safety, protected);
    let safety_rows = predict_rows(model, &whole.safety, workers)?;
    let safety_metrics = prediction_metrics(safety_rows.iter());
    let test_veto = veto_from(&test_rows, model.veto_threshold)?;
    let quality = pipeline::quality_gate(config, &test_metrics, &typo_metrics, &audit, &selection_veto, &test_veto);
    let (overall, typos) = (pipeline::by_trigger(&test_metrics), pipeline::by_trigger(&typo_metrics));
    let context: Map<String, Value> = fit::STRESS_PROFILES.iter().map(|profile| (profile.0.to_owned(), json!({"overall": overall, "typos": typos}))).collect();
    let exclusions = hex(&Sha256::digest(whole.sealed_test_exclusion_signatures.join("\n").as_bytes()));
    let manifest_veto = &manifest["veto"];
    let manifest_safety = &manifest["safety"];
    let veto_shape = manifest_veto.as_object().is_some_and(|object| object.len() == 2 && object.contains_key("selection") && object.contains_key("sealed_test"));
    let safety_shape = manifest_safety.as_object().is_some_and(|object| object.len() == 2 && object.contains_key("guard_audit") && object.contains_key("raw_model_diagnostics"));
    let checks = vec![
        check("runtime_threshold_selection", selection_gate == manifest["threshold_selection_gate_breakdown"], "runtime threshold split exactly reproduces signed evidence"),
        check("sealed_test", overall == manifest["sealed_test"], "runtime test predictions exactly reproduce signed metrics"),
        check("sealed_test_typos", typos == manifest["sealed_test_typos"], "runtime typo slice exactly reproduces signed metrics"),
        check("sealed_test_context_stress", Value::Object(context) == manifest["sealed_test_context_stress"], "runtime context profiles exactly reproduce signed metrics"),
        check("veto_selection", veto_shape && selection_veto.payload() == manifest_veto["selection"], "runtime calibration split exactly reproduces selected veto"),
        check("veto_sealed_test", veto_shape && test_veto.payload() == manifest_veto["sealed_test"], "runtime test logits exactly reproduce signed veto evidence"),
        check("safety_guard_audit", safety_shape && canonical(&audit.payload()) == canonical(&manifest_safety["guard_audit"]), "production static guards exactly reproduce signed safety audit"),
        check("safety_raw_model_diagnostics", safety_shape && pipeline::by_trigger(&safety_metrics) == manifest_safety["raw_model_diagnostics"], "runtime raw safety predictions exactly reproduce diagnostics"),
        check("quality_gate_breakdown", quality == manifest["quality_gate_breakdown"] && manifest["quality_gates_passed"] == quality["passed"], "recomputed inputs exactly reproduce the complete quality gate"),
        check("sealed_dataset_metadata", pipeline::sealed_counts(prepared, whole) == manifest["counts"] && manifest["variant_quarantine_sha256"].as_str() == Some(whole.variant_quarantine.sha256.as_str()) && manifest["sealed_variant_quarantine_sha256"].as_str() == Some(whole.sealed_variant_quarantine.sha256.as_str()) && manifest["sealed_test_exclusion_signatures_sha256"].as_str() == Some(exclusions.as_str()), "reconstructed sealed counts and quarantine hashes match"),
    ];
    Ok(InternalEvidence { test_rows, test_metrics, typo_metrics, safety_rows, safety_metrics, selection_gate, checks })
}

/// `PredictionComparison`.
#[derive(Clone, Copy, Default)]
struct Comparison {
    fallback: Confusion,
    ensemble: Confusion,
    rescued: i64,
    vetoed: i64,
    prevented: i64,
    introduced: i64,
    samples: i64,
    model_evaluated: i64,
    negative_model_evaluated: i64,
}

impl Comparison {
    fn payload(&self) -> Value {
        json!({"samples": self.samples, "fallback": metrics_payload(&self.fallback), "ensemble": metrics_payload(&self.ensemble), "rescued_true_positives": self.rescued, "vetoed_true_positives": self.vetoed, "prevented_false_positives": self.prevented, "introduced_false_positives": self.introduced, "model_evaluated_samples": self.model_evaluated, "negative_model_evaluated": self.negative_model_evaluated})
    }
}

/// `ProductionPolicyRow`.
#[derive(Clone, Copy)]
struct PolicyRow<'a> {
    example: &'a LexicalExample,
    fallback: bool,
    ensemble: bool,
    model_evaluated: bool,
    observed_delta: f64,
}

fn comparison_of<'a, 'b>(rows: impl Iterator<Item = &'a PolicyRow<'b>>) -> Comparison
where
    'b: 'a,
{
    let mut result = Comparison::default();
    let (mut fallback, mut ensemble) = (Vec::new(), Vec::new());
    for row in rows {
        let label = row.example.label;
        fallback.push((label, row.fallback));
        ensemble.push((label, row.ensemble));
        result.samples += 1;
        if row.model_evaluated {
            result.model_evaluated += 1;
            result.negative_model_evaluated += i64::from(!label);
        }
        match (label, row.fallback, row.ensemble) {
            (true, false, true) => result.rescued += 1,
            (true, true, false) => result.vetoed += 1,
            (false, true, false) => result.prevented += 1,
            (false, false, true) => result.introduced += 1,
            _ => {}
        }
    }
    result.fallback = confusion(fallback.into_iter());
    result.ensemble = confusion(ensemble.into_iter());
    result
}

fn detector_error(error: ks_lang::detector::DetectorError) -> String {
    format!("{error:?}")
}

fn policy_row<'a>(detector: &LanguageDetector, example: &'a LexicalExample, profile: Option<&Profile>) -> Result<PolicyRow<'a>, String> {
    let alternatives = [(example.target_group, example.alternative.clone())];
    let previous: Vec<(i64, String)> = if profile.is_some() { vec![(example.source_group, SOURCE_SENTINEL.to_owned()), (example.target_group, TARGET_SENTINEL.to_owned())] } else { Vec::new() };
    let context_group = profile.and_then(|profile| profile.context_group(example.source_group, example.target_group));
    let observed_delta = if profile.is_some() { detector.context_delta(example.source_group, example.target_group, &example.original, &example.alternative, &previous, context_group).map_err(detector_error)? } else { 0.0 };
    let options = |use_intent_model: bool| DecideOptions { previous_words: &previous, context_group, trigger: example.trigger, use_intent_model, ..DecideOptions::default() };
    let fallback = detector.decide(&example.original, &alternatives, example.source_group, &options(false)).map_err(detector_error)?;
    let ensemble = detector.decide(&example.original, &alternatives, example.source_group, &options(true)).map_err(detector_error)?;
    Ok(PolicyRow { example, fallback: fallback.should_convert, ensemble: ensemble.should_convert, model_evaluated: ensemble.model_probability.is_some(), observed_delta })
}

/// `compare_with_fallback`.
fn compare_with_fallback(detector: &LanguageDetector, examples: &[LexicalExample], requested: usize, workers: usize) -> Result<Comparison, String> {
    let all: Vec<&LexicalExample> = examples.iter().collect();
    let sample = external::deterministic_sample(&all, requested);
    let rows = map_rows(&sample, workers, |example| policy_row(detector, example, None))?;
    Ok(comparison_of(rows.iter()))
}

/// `ProductionContextProfile`: one role-relative reachable endpoint of the detector's context arithmetic.
#[derive(Clone, Copy, Debug)]
struct Profile {
    name: &'static str,
    source_context: f64,
    target_context: f64,
    selector: &'static str,
}

impl Profile {
    fn expected_delta(&self) -> f64 {
        let delta = lang::CONTEXT_DELTA_MULTIPLIER * (self.target_context - self.source_context);
        match self.selector {
            "target" => delta + lang::CONTEXT_TARGET_GROUP_BONUS,
            "source" => delta - lang::CONTEXT_SOURCE_GROUP_PENALTY,
            _ => delta,
        }
    }

    fn context_group(&self, source: i64, target: i64) -> Option<i64> {
        match self.selector {
            "source" => Some(source),
            "target" => Some(target),
            _ => None,
        }
    }
}

const PROFILES: [Profile; 7] = [
    Profile { name: "neutral", source_context: 0.0, target_context: 0.0, selector: "none" },
    Profile { name: "none_min", source_context: CONTEXT_SCORE_MAXIMUM, target_context: CONTEXT_SCORE_MINIMUM, selector: "none" },
    Profile { name: "none_max", source_context: CONTEXT_SCORE_MINIMUM, target_context: CONTEXT_SCORE_MAXIMUM, selector: "none" },
    Profile { name: "source_min", source_context: CONTEXT_SCORE_MAXIMUM, target_context: CONTEXT_SCORE_MINIMUM, selector: "source" },
    Profile { name: "source_max", source_context: CONTEXT_SCORE_MINIMUM, target_context: CONTEXT_SCORE_MAXIMUM, selector: "source" },
    Profile { name: "target_min", source_context: CONTEXT_SCORE_MAXIMUM, target_context: CONTEXT_SCORE_MINIMUM, selector: "target" },
    Profile { name: "target_max", source_context: CONTEXT_SCORE_MINIMUM, target_context: CONTEXT_SCORE_MAXIMUM, selector: "target" },
];

/// `_FixedContextLanguageScorer`: lexical evidence of the delegate, one reachable context profile.
struct FixedContext {
    delegate: Arc<dyn LanguageScorer>,
    profile: Profile,
}

impl LanguageScorer for FixedContext {
    fn score(&self, word: &str) -> WordScore {
        self.delegate.score(word)
    }

    fn context_score(&self, previous: &str, _word: &str) -> f64 {
        match previous {
            SOURCE_SENTINEL => self.profile.source_context,
            TARGET_SENTINEL => self.profile.target_context,
            _ => panic!("production context scorer received an unknown sentinel"),
        }
    }

    fn best_single_deletion(&self, word: &str) -> WordScore {
        self.delegate.best_single_deletion(word)
    }
}

/// `_CorpusLanguageScorerCache`.
struct CorpusCache {
    delegate: Arc<dyn LanguageScorer>,
    scores: RwLock<HashMap<String, WordScore>>,
    deletions: RwLock<HashMap<String, WordScore>>,
}

impl CorpusCache {
    fn new(delegate: Arc<dyn LanguageScorer>) -> CorpusCache {
        CorpusCache { delegate, scores: RwLock::default(), deletions: RwLock::default() }
    }
}

fn cached(table: &RwLock<HashMap<String, WordScore>>, word: &str, compute: impl FnOnce() -> WordScore) -> WordScore {
    if let Some(found) = table.read().expect("a score cache").get(word) {
        return found.clone();
    }
    let value = compute();
    table.write().expect("a score cache").insert(word.to_owned(), value.clone());
    value
}

impl LanguageScorer for CorpusCache {
    fn score(&self, word: &str) -> WordScore {
        cached(&self.scores, word, || self.delegate.score(word))
    }

    fn context_score(&self, previous: &str, word: &str) -> f64 {
        self.delegate.context_score(previous, word)
    }

    fn best_single_deletion(&self, word: &str) -> WordScore {
        cached(&self.deletions, word, || self.delegate.best_single_deletion(word))
    }
}

/// `_ContextInvariantIntentModel`: exact predictions reused, after the features were proved context invariant. A call that
/// finds its key is a hit, whichever worker made the first one, so the counts are those of a sequential run.
struct InvariantModel {
    delegate: Arc<LinearNgramModel>,
    cache: Mutex<HashMap<(String, String, i64, CorrectionTrigger), LinearPrediction>>,
    hits: AtomicU64,
}

impl IntentClassifier for InvariantModel {
    fn veto_threshold(&self) -> f64 {
        self.delegate.veto_threshold
    }

    fn predict(&self, item: &IntentModelInput<'_>) -> Result<LinearPrediction, LoadError> {
        let key = (item.original.to_owned(), item.alternative.to_owned(), item.source_group * 2 + item.target_group, item.trigger);
        if let Some(found) = self.cache.lock().expect("a prediction cache").get(&key) {
            self.hits.fetch_add(1, Ordering::Relaxed);
            return Ok(found.clone());
        }
        let neutral = IntentModelInput { context_delta: 0.0, context_group: None, ..*item };
        let prediction = self.delegate.predict(&neutral)?;
        match self.cache.lock().expect("a prediction cache").entry(key) {
            std::collections::hash_map::Entry::Occupied(_) => {
                self.hits.fetch_add(1, Ordering::Relaxed);
            }
            std::collections::hash_map::Entry::Vacant(place) => {
                place.insert(prediction.clone());
            }
        }
        Ok(prediction)
    }
}

/// `ProductionContextCorpusEvaluation`.
struct CorpusEvaluation<'a> {
    examples: Vec<&'a LexicalExample>,
    negative_decisions: Vec<bool>,
    overall: Comparison,
    per_trigger: BTreeMap<&'static str, Comparison>,
}

fn corpus_evaluation<'a>(rows: &[&PolicyRow<'a>]) -> CorpusEvaluation<'a> {
    let per_trigger = TRIGGERS.iter().filter(|trigger| rows.iter().any(|row| row.example.trigger == **trigger)).map(|trigger| (trigger.as_str(), comparison_of(rows.iter().copied().filter(|row| row.example.trigger == *trigger)))).collect();
    CorpusEvaluation { examples: rows.iter().map(|row| row.example).collect(), negative_decisions: rows.iter().filter(|row| !row.example.label).map(|row| row.ensemble).collect(), overall: comparison_of(rows.iter().copied()), per_trigger }
}

struct ProfileEvaluation<'a> {
    profile: Profile,
    observed: BTreeMap<String, Vec<f64>>,
    corpora: BTreeMap<&'static str, CorpusEvaluation<'a>>,
}

struct ContextEvaluation<'a> {
    profiles: Vec<ProfileEvaluation<'a>>,
    unique_predictions: u64,
    cache_hits: u64,
}

/// `evaluate_production_context_ensemble`: the real detector at every role-relative context endpoint.
#[expect(clippy::too_many_arguments)]
fn production_context<'a>(model: &Arc<LinearNgramModel>, sealed_test: &'a [LexicalExample], unknown_typo: &'a [LexicalExample], safety: &'a [LexicalExample], source_known: &'a [LexicalExample], models: &[Arc<LanguageModel>; 2], protected: &Arc<ProtectedTokens>, config: &FitConfig, workers: usize) -> Result<ContextEvaluation<'a>, String> {
    if safety.iter().any(|example| example.label || !example.safety) {
        return Err("production context safety rows must be safety negatives".to_owned());
    }
    if source_known.iter().any(|example| example.label || !models[example.source_group.to::<usize>()].score(&example.original).known) {
        return Err("production context source-known rows must be source-known negatives".to_owned());
    }
    fit::verify_context_invariance(config)?;
    let primary: [(&'static str, &'a [LexicalExample]); 4] = [("sealed_test", sealed_test), ("unknown_typo", unknown_typo), ("safety", safety), ("source_known", source_known)];
    let mut profiles: Vec<ProfileEvaluation<'a>> = PROFILES.iter().map(|profile| ProfileEvaluation { profile: *profile, observed: BTreeMap::new(), corpora: BTreeMap::new() }).collect();
    let (mut unique, mut hits) = (0, 0);
    for (name, examples) in primary {
        if examples.is_empty() {
            return Err(format!("production context corpus {name:?} must not be empty"));
        }
        let cache = Arc::new(InvariantModel { delegate: model.clone(), cache: Mutex::default(), hits: AtomicU64::new(0) });
        let corpus_models: [Arc<CorpusCache>; 2] = [Arc::new(CorpusCache::new(models[0].clone())), Arc::new(CorpusCache::new(models[1].clone()))];
        let references: Vec<&'a LexicalExample> = examples.iter().collect();
        for evaluation in &mut profiles {
            let profile = evaluation.profile;
            let fixed: Vec<(i64, Arc<dyn LanguageScorer>)> = (0..2).map(|group| -> (i64, Arc<dyn LanguageScorer>) { (group.to::<i64>(), Arc::new(FixedContext { delegate: corpus_models[group].clone(), profile })) }).collect();
            let detector = LanguageDetector::new(fixed, Some(cache.clone()), protected.clone()).map_err(load_error)?;
            let rows = map_rows(&references, workers, |example| policy_row(&detector, example, Some(&profile)))?;
            let all: Vec<&PolicyRow<'a>> = rows.iter().collect();
            evaluation.corpora.insert(name, corpus_evaluation(&all));
            for row in &rows {
                let direction = format!("{}_to_{}", row.example.source_group, row.example.target_group);
                let values = evaluation.observed.entry(direction).or_default();
                if !values.contains(&row.observed_delta) {
                    values.push(row.observed_delta);
                }
            }
            if name == "sealed_test" {
                let typos: Vec<&PolicyRow<'a>> = rows.iter().filter(|row| is_typo_kind(&row.example.variant_kind)).collect();
                evaluation.corpora.insert("sealed_test_typos", corpus_evaluation(&typos));
            }
        }
        unique += cache.cache.lock().expect("a prediction cache").len().to::<u64>();
        hits += cache.hits.load(Ordering::Relaxed);
    }
    for evaluation in &mut profiles {
        for values in evaluation.observed.values_mut() {
            values.sort_by(|a, b| a.partial_cmp(b).expect("finite deltas"));
        }
    }
    Ok(ContextEvaluation { profiles, unique_predictions: unique, cache_hits: hits })
}

fn shape_is_valid(value: &CorpusEvaluation<'_>) -> bool {
    let positives = value.examples.iter().filter(|example| example.label).count().to::<i64>();
    let negatives = value.examples.len().to::<i64>() - positives;
    let valid = |comparison: &Comparison, positives: i64, negatives: i64| comparison.fallback.true_positive + comparison.fallback.false_negative == positives && comparison.fallback.true_negative + comparison.fallback.false_positive == negatives && comparison.ensemble.true_positive + comparison.ensemble.false_negative == positives && comparison.ensemble.true_negative + comparison.ensemble.false_positive == negatives && 0 <= comparison.negative_model_evaluated && comparison.negative_model_evaluated <= comparison.model_evaluated && comparison.model_evaluated <= comparison.samples;
    let aggregate = value.overall.samples == value.examples.len().to::<i64>() && valid(&value.overall, positives, negatives) && value.negative_decisions.len().to::<i64>() == negatives && value.negative_decisions.iter().filter(|decision| **decision).count().to::<i64>() == value.overall.ensemble.false_positive;
    let mut samples = 0;
    let mut per_trigger = true;
    for (trigger, comparison) in &value.per_trigger {
        let rows: Vec<&&LexicalExample> = value.examples.iter().filter(|example| example.trigger.as_str() == *trigger).collect();
        let positive = rows.iter().filter(|example| example.label).count().to::<i64>();
        samples += comparison.samples;
        per_trigger = per_trigger && comparison.samples == rows.len().to::<i64>() && valid(comparison, positive, rows.len().to::<i64>() - positive);
    }
    aggregate && per_trigger && samples == value.examples.len().to::<i64>()
}

/// `_context_profile_corpus_gate`.
fn context_profile_corpus_gate(config: &FitConfig, profile: &Profile, name: &str, corpus: &CorpusEvaluation<'_>, neutral: &CorpusEvaluation<'_>) -> Value {
    let expected: Vec<&'static str> = if name == "source_known" { vec!["space"] } else { trigger_names() };
    let both_labels = matches!(name, "sealed_test" | "sealed_test_typos" | "unknown_typo");
    let typo_recall = matches!(name, "sealed_test_typos" | "unknown_typo");
    let (corpus_shape, neutral_shape) = (shape_is_valid(corpus), shape_is_valid(neutral));
    let aligned = corpus.examples == neutral.examples;
    let newly_converted = if aligned && corpus.negative_decisions.len() == neutral.negative_decisions.len() { corpus.negative_decisions.iter().zip(&neutral.negative_decisions).filter(|(current, baseline)| **current && !**baseline).count() } else { corpus.examples.len() + 1 };
    let is_pause = |trigger: &str| trigger == "pause";
    let strictest = expected.iter().map(|trigger| if is_pause(trigger) { config.pause_max_fpr } else { config.max_fpr }).fold(f64::INFINITY, f64::min);
    let overall = &corpus.overall;
    let overall_negatives = overall.ensemble.true_negative + overall.ensemble.false_positive;
    let overall_checks = json!({
        "samples_nonempty": overall.samples > 0,
        "corpus_shape_valid": corpus_shape,
        "neutral_corpus_shape_valid": neutral_shape,
        "rows_aligned_with_neutral": aligned,
        "absolute_precision": overall.ensemble.precision() >= config.test_precision,
        "absolute_specificity": overall.ensemble.specificity() >= config.test_specificity,
        "false_positive_policy": overall_negatives > 0 && (if both_labels { wilson_upper(overall.ensemble.false_positive, overall_negatives, WILSON_95_Z) <= strictest } else { overall.ensemble.false_positive == 0 }),
        "introduced_false_positive_policy": if both_labels { overall.ensemble.false_positive <= overall.fallback.false_positive } else { overall.introduced == 0 },
        "ensemble_false_positives_no_more_than_fallback": overall.ensemble.false_positive <= overall.fallback.false_positive,
        "no_new_negative_vs_neutral": newly_converted == 0,
        "ensemble_false_positives_no_more_than_neutral": overall.ensemble.false_positive <= neutral.overall.ensemble.false_positive,
    });
    let has_exactly = |table: &BTreeMap<&'static str, Comparison>| table.len() == expected.len() && expected.iter().all(|trigger| table.contains_key(trigger));
    let (keys_exact, neutral_keys_exact) = (has_exactly(&corpus.per_trigger), has_exactly(&neutral.per_trigger));
    let mut sorted = expected.clone();
    sorted.sort_unstable();
    let mut trigger_gates = Map::new();
    for trigger in &sorted {
        let (Some(comparison), Some(baseline)) = (corpus.per_trigger.get(trigger), neutral.per_trigger.get(trigger)) else {
            trigger_gates.insert((*trigger).to_owned(), json!({"passed": false, "missing": true}));
            continue;
        };
        let ensemble = &comparison.ensemble;
        let positive = ensemble.true_positive + ensemble.false_negative;
        let negative = ensemble.true_negative + ensemble.false_positive;
        let maximum_fpr = if is_pause(trigger) { config.pause_max_fpr } else { config.max_fpr };
        let minimum_recall = match (typo_recall, is_pause(trigger)) {
            (true, true) => config.test_pause_typo_recall,
            (true, false) => config.test_typo_recall,
            (false, true) => config.test_pause_recall,
            (false, false) => config.test_recall,
        };
        let (recall_passed, recall_policy) = if both_labels {
            if profile.expected_delta() < 0.0 { (ensemble.recall() + RECALL_TOLERANCE >= comparison.fallback.recall(), format!("contextual-fallback-minus-{RECALL_TOLERANCE}")) } else { (ensemble.recall() >= minimum_recall, "absolute-floor".to_owned()) }
        } else {
            (positive == 0, "negative-only".to_owned())
        };
        let checks = json!({
            "label_support": if both_labels { positive > 0 && negative > 0 } else { positive == 0 && negative > 0 },
            "absolute_precision": ensemble.precision() >= config.test_precision,
            "absolute_specificity": ensemble.specificity() >= config.test_specificity,
            "false_positive_policy": negative > 0 && (if both_labels { wilson_upper(ensemble.false_positive, negative, WILSON_95_Z) <= maximum_fpr } else { ensemble.false_positive == 0 }),
            "recall": recall_passed,
            "introduced_false_positive_policy": if both_labels { ensemble.false_positive <= comparison.fallback.false_positive } else { comparison.introduced == 0 },
            "ensemble_false_positives_no_more_than_fallback": ensemble.false_positive <= comparison.fallback.false_positive,
            "ensemble_false_positives_no_more_than_neutral": ensemble.false_positive <= baseline.ensemble.false_positive,
        });
        let passed = all_true(&checks);
        trigger_gates.insert((*trigger).to_owned(), json!({"passed": passed, "recall_policy": recall_policy, "minimum_recall": minimum_recall, "maximum_false_positive_rate": maximum_fpr, "checks": checks, "comparison": comparison.payload()}));
    }
    let reachability = match name {
        "unknown_typo" => json!({"all_rows_reach_model": overall.model_evaluated == overall.samples, "all_negative_rows_reach_model": overall.negative_model_evaluated * external::PAIRED_ROWS_PER_TRIGGER.to::<i64>() == overall.samples}),
        "safety" | "source_known" => json!({"pre_model_guard_blocks_all_rows": overall.model_evaluated == 0 && overall.negative_model_evaluated == 0}),
        _ => json!({}),
    };
    let passed = keys_exact && neutral_keys_exact && all_true(&overall_checks) && all_true(&reachability) && trigger_gates.values().all(|gate| gate["passed"] == json!(true));
    json!({"passed": passed, "expected_triggers": sorted, "all_triggers_present": keys_exact && neutral_keys_exact, "newly_converted_negative_vs_neutral": newly_converted, "overall_checks": overall_checks, "reachability_checks": reachability, "overall_comparison": overall.payload(), "per_trigger": trigger_gates})
}

fn all_true(checks: &Value) -> bool {
    checks.as_object().is_some_and(|object| object.values().all(|value| value == &json!(true)))
}

fn observed_payload(observed: &BTreeMap<String, Vec<f64>>) -> Value {
    Value::Object(observed.iter().map(|(direction, values)| (direction.clone(), json!(values))).collect())
}

/// `production_context_ensemble_gate_breakdown`.
fn production_context_gate(config: &FitConfig, evaluation: &ContextEvaluation<'_>) -> Value {
    let neutral = evaluation.profiles.iter().find(|profile| profile.profile.name == "neutral");
    let mut gates = Map::new();
    for observed in &evaluation.profiles {
        let profile = &observed.profile;
        let corpora_exact = observed.corpora.len() == CORPORA.len() && CORPORA.iter().all(|name| observed.corpora.contains_key(name));
        let directions_exact = observed.observed.len() == 2 && observed.observed.contains_key("0_to_1") && observed.observed.contains_key("1_to_0");
        let extrema_exact = directions_exact && observed.observed.values().all(|values| values.as_slice() == [profile.expected_delta()]);
        let mut corpus_gates = Map::new();
        if let Some(neutral) = neutral {
            for name in CORPORA {
                match (observed.corpora.get(name), neutral.corpora.get(name)) {
                    (Some(corpus), Some(baseline)) => {
                        corpus_gates.insert(name.to_owned(), context_profile_corpus_gate(config, profile, name, corpus, baseline));
                    }
                    _ => {
                        corpus_gates.insert(name.to_owned(), json!({"passed": false, "missing": true}));
                    }
                }
            }
        }
        let passed = corpora_exact && extrema_exact && corpus_gates.len() == CORPORA.len() && corpus_gates.values().all(|gate| gate["passed"] == json!(true));
        gates.insert(profile.name.to_owned(), json!({"passed": passed, "profile_matches_contract": true, "all_corpora_present": corpora_exact, "reachable_extrema_exact": extrema_exact, "expected_delta": profile.expected_delta(), "expected_delta_hex": float_hex(profile.expected_delta()), "group_selector": profile.selector, "observed_deltas_by_direction": observed_payload(&observed.observed), "corpora": corpus_gates}));
    }
    let mut expected: Vec<&str> = PROFILES.iter().map(|profile| profile.name).collect();
    expected.sort_unstable();
    let all_present = evaluation.profiles.len() == PROFILES.len();
    json!({"passed": all_present && gates.values().all(|gate| gate["passed"] == json!(true)), "schema_version": 1, "all_profiles_present": all_present, "expected_profiles": expected, "profiles": gates})
}

/// `_production_context_payload`.
fn production_context_payload(evaluation: &ContextEvaluation<'_>, gate: &Value) -> Value {
    let profiles: Map<String, Value> = evaluation
        .profiles
        .iter()
        .map(|result| {
            let corpora: Map<String, Value> = result.corpora.iter().map(|(name, corpus)| ((*name).to_owned(), json!({"overall": corpus.overall.payload(), "per_trigger": Value::Object(corpus.per_trigger.iter().map(|(trigger, comparison)| ((*trigger).to_owned(), comparison.payload())).collect())}))).collect();
            (result.profile.name.to_owned(), json!({"source_context": result.profile.source_context, "target_context": result.profile.target_context, "group_selector": result.profile.selector, "expected_delta": result.profile.expected_delta(), "expected_delta_hex": float_hex(result.profile.expected_delta()), "observed_deltas_by_direction": observed_payload(&result.observed), "corpora": corpora}))
        })
        .collect();
    json!({
        "schema_version": 1,
        "bounds": {"context_score_minimum": CONTEXT_SCORE_MINIMUM, "context_score_maximum": CONTEXT_SCORE_MAXIMUM, "delta_multiplier": lang::CONTEXT_DELTA_MULTIPLIER, "target_group_bonus": lang::CONTEXT_TARGET_GROUP_BONUS, "source_group_penalty": lang::CONTEXT_SOURCE_GROUP_PENALTY},
        "prediction_cache": {"context_invariance_proved_before_use": true, "unique_model_predictions": evaluation.unique_predictions, "cache_hits": evaluation.cache_hits},
        "gate_breakdown": gate,
        "profiles": profiles,
    })
}

/// `SafetyPolicyEvaluation`.
#[derive(Default)]
struct SafetyPolicy {
    per_trigger: BTreeMap<&'static str, Confusion>,
    samples: i64,
    protected_samples: i64,
    lexical_collision_samples: i64,
    pre_model_guarded_samples: i64,
    expected_guard_samples: i64,
    expected_guarded_samples: i64,
    model_evaluated_samples: i64,
    guard_failure_samples: i64,
    reason_counts: BTreeMap<String, i64>,
}

/// `evaluate_safety_policy`: the safety rows through the same guards and ensemble as production.
fn safety_policy(detector: &LanguageDetector, examples: &[LexicalExample], protected: &ProtectedTokens) -> Result<SafetyPolicy, String> {
    let mut policy = SafetyPolicy { samples: examples.len().to::<i64>(), per_trigger: TRIGGERS.iter().map(|trigger| (trigger.as_str(), Confusion::default())).collect(), ..SafetyPolicy::default() };
    for (index, example) in examples.iter().enumerate() {
        if example.label || !example.safety {
            return Err(format!("safety policy corpus must contain only labelled-negative safety rows: index={index}, base={:?}", example.base_signature));
        }
        let guard_matches = match example.variant_kind.as_str() {
            "protected" => {
                if !example.protected {
                    return Err(format!("protected safety row is missing its protected marker: index={index}, base={:?}", example.base_signature));
                }
                policy.protected_samples += 1;
                ks_lang::detector::is_protected_token(&example.original, protected)
            }
            "lexical_collision" => {
                if example.protected {
                    return Err(format!("lexical-collision safety row cannot be marked protected: index={index}, base={:?}", example.base_signature));
                }
                policy.lexical_collision_samples += 1;
                true
            }
            other => return Err(format!("unsupported safety policy row kind: index={index}, variant={other:?}")),
        };
        let decision = detector.decide(&example.original, &[(example.target_group, example.alternative.clone())], example.source_group, &DecideOptions { trigger: example.trigger, context_group: example.context_group, use_intent_model: true, ..DecideOptions::default() }).map_err(detector_error)?;
        let evaluated = decision.model_probability.is_some();
        if evaluated {
            policy.model_evaluated_samples += 1;
        } else {
            policy.pre_model_guarded_samples += 1;
            policy.expected_guarded_samples += 1;
        }
        policy.expected_guard_samples += 1;
        let collision_matches = example.variant_kind != "lexical_collision" || matches!(decision.reason.as_str(), "обе раскладки дают допустимое слово" | "исходное слово допустимо");
        if decision.should_convert || evaluated || !guard_matches || !collision_matches {
            policy.guard_failure_samples += 1;
        }
        *policy.reason_counts.entry(decision.reason.clone()).or_default() += 1;
        let matrix = policy.per_trigger.get_mut(example.trigger.as_str()).expect("every trigger");
        if decision.should_convert {
            matrix.false_positive += 1;
        } else {
            matrix.true_negative += 1;
        }
    }
    Ok(policy)
}

fn percentile(samples: &[f64], quantile: f64) -> Result<f64, String> {
    if samples.is_empty() {
        return Err("percentile requires samples".to_owned());
    }
    let mut ordered = samples.to_vec();
    ordered.sort_by(|a, b| a.partial_cmp(b).expect("finite samples"));
    let index = (ordered.len() - 1).min((ks_text::py_round((ordered.len() - 1).to::<f64>() * quantile)).to::<usize>());
    Ok(ordered[index])
}

fn median(samples: &[f64]) -> f64 {
    let mut ordered = samples.to_vec();
    ordered.sort_by(|a, b| a.partial_cmp(b).expect("finite samples"));
    let middle = ordered.len() / 2;
    if ordered.len() % 2 == 1 { ordered[middle] } else { f64::midpoint(ordered[middle - 1], ordered[middle]) }
}

fn coverage_statistics(rows: &[&PredictionRow<'_>]) -> Result<Value, String> {
    let values: Vec<f64> = rows.iter().map(|row| row.coverage).collect();
    if values.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) {
        return Err("prediction coverage must be finite and in [0, 1]".to_owned());
    }
    let total = fsum(&values).map_err(|_| "the sum of the coverage overflowed")?;
    Ok(json!({"samples": values.len(), "minimum": values.iter().copied().fold(f64::INFINITY, f64::min), "p05": percentile(&values, 0.05)?, "p25": percentile(&values, 0.25)?, "median": median(&values), "p75": percentile(&values, 0.75)?, "p95": percentile(&values, 0.95)?, "maximum": values.iter().copied().fold(f64::NEG_INFINITY, f64::max), "mean": total / values.len().to::<f64>(), "zero_coverage_samples": values.iter().filter(|value| **value == 0.0).count(), "full_coverage_samples": values.iter().filter(|value| **value == 1.0).count()}))
}

/// `coverage_diagnostics`: OOD membership coverage, never a gate.
fn coverage_diagnostics(rows: &[PredictionRow<'_>]) -> Result<Value, String> {
    let mut slices: Vec<(String, Vec<&PredictionRow<'_>>)> = vec![
        ("overall".to_owned(), rows.iter().collect()),
        ("label_negative".to_owned(), rows.iter().filter(|row| !row.example.label).collect()),
        ("label_positive".to_owned(), rows.iter().filter(|row| row.example.label).collect()),
        ("direction_0_to_1".to_owned(), rows.iter().filter(|row| row.example.source_group == 0 && row.example.target_group == 1).collect()),
        ("direction_1_to_0".to_owned(), rows.iter().filter(|row| row.example.source_group == 1 && row.example.target_group == 0).collect()),
    ];
    for trigger in TRIGGERS {
        slices.push((format!("trigger_{}", trigger.as_str()), rows.iter().filter(|row| row.example.trigger == trigger).collect()));
    }
    let mut result = Map::new();
    for (name, selected) in slices {
        if !selected.is_empty() {
            result.insert(name, coverage_statistics(&selected)?);
        }
    }
    Ok(Value::Object(result))
}

/// `latency_report`.
fn latency_report(artifact: &Path, model: &LinearNgramModel, examples: &[LexicalExample], requested: usize) -> Result<Value, String> {
    let mut load = Vec::new();
    for _ in 0..LOAD_REPETITIONS {
        let started = Instant::now();
        let loaded = LinearNgramModel::load(artifact).map_err(load_error)?;
        load.push(started.elapsed().as_secs_f64() * 1000.0);
        if loaded.checksum != model.checksum {
            return Err("model checksum changed between repeated loads".to_owned());
        }
    }
    let all: Vec<&LexicalExample> = examples.iter().collect();
    let sample = external::deterministic_sample(&all, requested.min(all.len()));
    for example in sample.iter().take(WARMUP_PREDICTIONS) {
        predict(model, example)?;
    }
    let mut inference = Vec::new();
    let mut first = Vec::new();
    for example in &sample {
        let started = Instant::now();
        let prediction = predict(model, example)?;
        inference.push(started.elapsed().as_secs_f64() * 1000.0);
        first.push(prediction);
    }
    let mut deterministic = true;
    for (example, earlier) in sample.iter().zip(&first) {
        let again = predict(model, example)?;
        deterministic &= again.logit.to_bits() == earlier.logit.to_bits() && again.probability.to_bits() == earlier.probability.to_bits() && again.coverage.to_bits() == earlier.coverage.to_bits() && again.should_switch == earlier.should_switch;
    }
    let bytes = std::fs::metadata(artifact).map_err(|error| format!("{}: {error}", artifact.display()))?.len();
    let summary = |samples: &[f64]| -> Result<Value, String> { Ok(json!({"median": py_round_ndigits(median(samples), LATENCY_DECIMALS), "p95": py_round_ndigits(percentile(samples, 0.95)?, LATENCY_DECIMALS), "samples": samples.len()})) };
    Ok(json!({"artifact_bytes": bytes, "load_ms": summary(&load)?, "inference_ms": summary(&inference)?, "deterministic_predictions": deterministic}))
}

/// `_hunspell_provenance_matches`.
fn dictionaries_match_policy(provenance: &[external::DictionaryProvenance; 2], hunspell: &Value) -> bool {
    LOCALES.iter().enumerate().all(|(group, locale)| {
        let (observed, pinned) = (&provenance[group], &hunspell[*locale]);
        observed.locale == *locale && pinned["dictionary_sha256"].as_str() == Some(observed.dictionary.sha256.as_str()) && pinned["dictionary_bytes"].as_u64() == Some(observed.dictionary.bytes.to::<u64>()) && pinned["affix_sha256"].as_str() == Some(observed.affix.sha256.as_str()) && pinned["affix_bytes"].as_u64() == Some(observed.affix.bytes.to::<u64>())
    })
}

/// `_unknown_typo_trigger_expansion_matches`.
fn trigger_expansion_matches(corpus: &ExternalCorpus, expected: &Value) -> bool {
    if expected != &json!(TRIGGER_NAMES) {
        return false;
    }
    let mut rows: BTreeMap<&str, BTreeSet<(&str, bool)>> = BTreeMap::new();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for example in &corpus.examples {
        rows.entry(&example.base_signature).or_default().insert((example.trigger.as_str(), example.label));
        *counts.entry(&example.base_signature).or_default() += 1;
    }
    let wanted = TRIGGERS.len() * 2;
    !rows.is_empty() && rows.values().all(|pairs| pairs.len() == wanted) && counts.values().all(|count| *count == wanted)
}

/// `external_corpus_provenance_gate_breakdown`: every frozen external input, before model metrics are computed.
fn external_provenance_gates(policy: &Value, stable: bool, lexical: &ExternalCorpus, index: &SealedSignatureIndex, development: &ExternalCorpus, holdout: &ExternalCorpus) -> Value {
    let signatures = external::unknown_typo_physical_signatures(development).and_then(|first| Ok((first, external::unknown_typo_physical_signatures(holdout)?)));
    let shape_valid = signatures.is_ok();
    let (development_signatures, holdout_signatures) = signatures.unwrap_or_default();
    let exclusions: BTreeSet<String> = index.signatures.union(&development_signatures).cloned().collect();
    let sources_match = |corpus: &ExternalCorpus| corpus.dictionary_sources[0] == corpus.dictionary_provenance[0].dictionary.path && corpus.dictionary_sources[1] == corpus.dictionary_provenance[1].dictionary.path;
    let provenance_matches = lexical.dictionary_provenance == development.dictionary_provenance && development.dictionary_provenance == holdout.dictionary_provenance && sources_match(lexical) && sources_match(holdout) && sources_match(development) && dictionaries_match_policy(&lexical.dictionary_provenance, &policy["hunspell"]);
    let minimum = (policy["minimum_words_per_group"].as_u64().unwrap_or(0)).to::<usize>();
    let pinned = |name: &str| policy[name].as_str().unwrap_or("").to_owned();
    json!({
        "external_policy_schema": policy["schema_version"].as_i64() == Some(EXTERNAL_SCHEMA_VERSION),
        "external_minimum_corpus_policy": lexical.minimum_words_per_group == minimum && development.minimum_words_per_group == minimum && holdout.minimum_words_per_group == minimum,
        "external_trigger_expansion_policy": trigger_expansion_matches(holdout, &policy["trigger_expansion"]),
        "external_hunspell_provenance": provenance_matches,
        "hunspell_handle_snapshot_stability": stable,
        "lexical_disjoint_size": lexical.words_by_group.iter().all(|count| *count >= lexical.minimum_words_per_group),
        "lexical_disjoint_corpus_provenance": lexical.corpus_sha256 == dataset::external_corpus_fingerprint(&lexical.examples) && lexical.corpus_sha256 == pinned("lexical_disjoint_corpus_sha256"),
        "unknown_typo_development_provenance": shape_valid && development.corpus_sha256 == dataset::external_corpus_fingerprint(&development.examples) && development.corpus_sha256 == pinned("unknown_typo_development_corpus_sha256") && development.rank_namespace == external::DEVELOPMENT_RANK_NAMESPACE && development.choice_namespace == external::DEVELOPMENT_CHOICE_NAMESPACE && development.exclusion_signature_count == index.signatures.len() && development.exclusion_signature_sha256 == index.sha256 && development.selected_sealed_overlap_count == 0,
        "unknown_typo_disjoint_size": holdout.words_by_group.iter().all(|count| *count >= holdout.minimum_words_per_group),
        "unknown_typo_holdout_provenance": holdout.corpus_sha256 == dataset::external_corpus_fingerprint(&holdout.examples) && holdout.corpus_sha256 == pinned("unknown_typo_holdout_corpus_sha256") && holdout.rank_namespace == external::HOLDOUT_RANK_NAMESPACE && holdout.choice_namespace == external::HOLDOUT_CHOICE_NAMESPACE,
        "unknown_typo_holdout_disjointness": shape_valid && holdout.exclusion_signature_count == exclusions.len() && holdout.exclusion_signature_sha256 == dataset::signature_set_fingerprint(exclusions.iter().map(String::as_str)) && holdout.selected_sealed_overlap_count == 0 && development_signatures.is_disjoint(&holdout_signatures) && index.signatures.is_disjoint(&holdout_signatures),
    })
}

fn slice_passes(config: &FitConfig, metrics: &Confusion, minimum_recall: f64, maximum_fpr: f64) -> bool {
    let positive = metrics.true_positive + metrics.false_negative;
    let negative = metrics.true_negative + metrics.false_positive;
    positive > 0 && negative > 0 && metrics.precision() >= config.test_precision && metrics.recall() >= minimum_recall && metrics.specificity() >= config.test_specificity && wilson_upper(metrics.false_positive, negative, WILSON_95_Z) <= maximum_fpr
}

/// What the strict report is built from.
pub struct Options<'a> {
    pub root: &'a Path,
    pub artifact: &'a Path,
    pub manifest: &'a Path,
    pub comparison_sample: usize,
    pub latency_sample: usize,
    pub strict: bool,
    pub provenance_only: bool,
    pub workers: usize,
}

fn pretty(value: &Value) -> String {
    ks_text::json_dumps(value, &ks_text::JsonOptions { ensure_ascii: false, indent: Some(ks_text::Indent::Spaces(2)), sort_keys: true, ..ks_text::JsonOptions::default() })
}

fn environment_section(root: &Path) -> Result<Value, String> {
    let frozen = root.join("model/intent_v1/sources/hunspell");
    let mut dictionaries = Map::new();
    for name in ["en_US.aff", "en_US.dic", "ru_RU.aff", "ru_RU.dic"] {
        let digest = seal::sha256_file(&frozen.join(name))?;
        let system = Path::new("/usr/share/hunspell").join(name);
        let entry = if system.is_file() {
            let installed = seal::sha256_file(&system)?;
            json!({"frozen_sha256": digest, "system_sha256": installed, "matches_system": installed == digest})
        } else {
            json!({"frozen_sha256": digest, "system_sha256": null, "matches_system": null})
        };
        dictionaries.insert(name.to_owned(), entry);
    }
    Ok(json!({"schema_version": 1, "probe": probe::measure(), "hunspell_dictionaries": dictionaries, "hunspell_scope": "the evaluation reads the frozen dictionaries; the system ones are reported only so that a drift between them is visible"}))
}

fn corpus_dictionaries(corpus: &ExternalCorpus) -> Value {
    json!({"0": corpus.dictionary_provenance[0].payload(), "1": corpus.dictionary_provenance[1].payload()})
}

fn pair_of(values: &[usize; 2]) -> Value {
    json!({"0": values[0], "1": values[1]})
}

fn sources_of(corpus: &ExternalCorpus) -> Value {
    json!({"0": corpus.dictionary_sources[0], "1": corpus.dictionary_sources[1]})
}

/// `main` of the evaluator: the report on standard output (or returned), the exit status as a number.
pub fn evaluate(options: &Options<'_>) -> Result<(String, i32), String> {
    let root = options.root;
    let workers = options.workers.max(1);
    let config_path = root.join("model/intent_v1/config.json");
    let presealed = pipeline::presealed_dataset(root)?;
    let pipeline::Presealed { raw_config, dataset_config, words, prepared, base, dataset: candidate, hard } = presealed;
    let config = FitConfig::read(&raw_config)?;
    let policy = raw_config["external_evaluation"].clone();
    let protected_list = Arc::new(ProtectedTokens::load(&root.join("resources")));
    let scorer = dataset::train_only_scorer_provenance(&prepared, &candidate.variant_quarantine);
    let hard_provenance = hard.provenance();
    let manifest = crate::corpus::read_json(options.manifest)?;
    let model = Arc::new(LinearNgramModel::load(options.artifact).map_err(load_error)?);
    let presealed_inputs = ProvenanceInputs { root, artifact: options.artifact, config_path: &config_path, raw_config: &raw_config, config: &config, candidate: &candidate, dataset: None, scorer: &scorer, hard_negative: &hard_provenance, protected: &protected_list };
    let presealed_checks = verify_provenance(&model, &manifest, &presealed_inputs)?;
    let provenance_count = presealed_checks.len();
    if !checks_pass(&presealed_checks, provenance_count) {
        return Ok((pretty(&json!({"schema_version": 1, "phase": "presealed_provenance", "sealed_test_evaluated": false, "provenance": checks_payload(&presealed_checks)})), 1));
    }
    let sealed = dataset::build_dataset(&prepared, &dataset_config, &[Split::Test]);
    dataset::assert_no_split_leakage(&sealed, &[Split::Test])?;
    let whole = dataset::merge_sealed_test_dataset(candidate.clone(), sealed.clone())?;
    dataset::assert_no_split_leakage(&whole, &Split::PRESEALED)?;
    let base_sealed = dataset::merge_sealed_test_dataset(base, sealed)?;
    dataset::assert_no_split_leakage(&base_sealed, &Split::PRESEALED)?;
    let full_inputs = ProvenanceInputs { dataset: Some(&whole), ..presealed_inputs };
    let provenance = verify_provenance(&model, &manifest, &full_inputs)?;
    if !checks_pass(&provenance, provenance_count + 1) {
        return Ok((pretty(&json!({"schema_version": 1, "phase": "full_provenance", "sealed_test_evaluated": false, "provenance": checks_payload(&provenance)})), 1));
    }
    let evidence = internal_evidence(&model, &manifest, &config, &prepared, &whole, &protected_list, workers)?;
    if !checks_pass(&evidence.checks, 10) {
        return Ok((pretty(&json!({"schema_version": 1, "phase": "internal_sealed_evidence", "sealed_test_evaluated": true, "provenance": checks_payload(&provenance), "internal_sealed_evidence": checks_payload(&evidence.checks)})), 1));
    }
    if options.provenance_only {
        return Ok((pretty(&json!({"schema_version": 1, "phase": "internal_sealed_evidence", "provenance_passed": true, "internal_sealed_evidence_passed": true, "sealed_test_evaluated": true, "provenance": checks_payload(&provenance), "internal_sealed_evidence": checks_payload(&evidence.checks)})), 0));
    }
    let frozen = super::frozen_dictionaries(root)?;
    let before: [Snapshot; 2] = [external::discover_snapshot(LOCALES[0], &frozen)?, external::discover_snapshot(LOCALES[1], &frozen)?];
    let (resources, models) = super::runtime_models(root)?;
    let protected = resources.protected_tokens();
    let latency = latency_report(options.artifact, &model, whole.split(Split::Test), options.latency_sample)?;
    let snapshots: [Snapshot; 2] = [external::snapshot_of_model(&models[0], LOCALES[0])?, external::snapshot_of_model(&models[1], LOCALES[1])?];
    let stable = before == snapshots && (0..2).all(|group| models[group].speller.available() && models[group].speller.source() == snapshots[group].provenance.dictionary.path);
    let index = external::build_sealed_signature_index(&base_sealed)?;
    let scorers: Vec<(i64, Arc<dyn LanguageScorer>)> = vec![(0, models[0].clone()), (1, models[1].clone())];
    let detector = LanguageDetector::new(scorers, Some(model.clone()), protected.clone()).map_err(load_error)?;
    let safety = safety_policy(&detector, &whole.safety, &protected)?;
    let rows = &evidence.test_rows;
    let slice = |keep: &dyn Fn(&LexicalExample) -> bool| -> Vec<PredictionRow<'_>> { rows.iter().filter(|row| keep(row.example)).copied().collect() };
    let slices: Vec<(&str, Vec<PredictionRow<'_>>)> = vec![
        ("identity", slice(&|example| example.variant_kind == "identity")),
        ("typo_unknown", slice(&|example| is_typo_kind(&example.variant_kind))),
        ("direction_0_to_1", slice(&|example| example.target_group == 1)),
        ("direction_1_to_0", slice(&|example| example.target_group == 0)),
        ("context_none", slice(&|example| example.context_group.is_none())),
        ("context_present", slice(&|example| example.context_group.is_some())),
        ("context_misleading", slice(&|example| example.context_group.is_some() && example.context_group == Some(if example.label { example.source_group } else { example.target_group }))),
    ];
    let slice_metrics: BTreeMap<&str, BTreeMap<&'static str, Confusion>> = slices.iter().map(|(name, selected)| (*name, prediction_metrics(selected.iter()))).collect();
    let veto = veto_from(rows, model.veto_threshold)?;
    let comparison = compare_with_fallback(&detector, whole.split(Split::Test), options.comparison_sample, workers)?;
    let onboard = external::onboard_words(&words, dataset_config.minimum_word_length);
    let minimum = (policy["minimum_words_per_group"].as_u64().ok_or("minimum_words_per_group")?).to::<usize>();
    let references = [&*models[0], &*models[1]];
    let lexical = external::build_lexical_disjoint_corpus(&onboard, minimum, &snapshots)?;
    let development = external::build_unknown_typo_disjoint_corpus(&onboard, &index.signatures, minimum, &snapshots, &references, &protected, external::DEVELOPMENT_RANK_NAMESPACE, external::DEVELOPMENT_CHOICE_NAMESPACE)?;
    let development_signatures = external::unknown_typo_physical_signatures(&development)?;
    let combined: BTreeSet<String> = index.signatures.union(&development_signatures).cloned().collect();
    let holdout = external::build_unknown_typo_disjoint_corpus(&onboard, &combined, minimum, &snapshots, &references, &protected, external::HOLDOUT_RANK_NAMESPACE, external::HOLDOUT_CHOICE_NAMESPACE)?;
    let provenance_gates = external_provenance_gates(&policy, stable, &lexical, &index, &development, &holdout);
    if !all_true(&provenance_gates) {
        return Ok((pretty(&json!({"schema_version": 1, "phase": "external_corpus_provenance", "external_model_metrics_evaluated": false, "external_provenance_gates": provenance_gates})), 1));
    }
    let lexical_comparison = compare_with_fallback(&detector, &lexical.examples, 0, workers)?;
    let unknown_comparison = compare_with_fallback(&detector, &holdout.examples, 0, workers)?;
    let unknown_rows = predict_rows(&model, &holdout.examples, workers)?;
    let unknown_metrics = prediction_metrics(unknown_rows.iter());
    let source_known: Vec<LexicalExample> = lexical.examples.iter().filter(|example| !example.label && models[example.source_group.to::<usize>()].score(&example.original).known).map(|example| LexicalExample { source_known: true, ..example.clone() }).collect();
    if !(0..2).all(|group| source_known.iter().any(|example| example.source_group == group)) {
        return Err("source-known runtime subset must cover groups 0 and 1".to_owned());
    }
    let source_known_comparison = compare_with_fallback(&detector, &source_known, 0, workers)?;
    let context = production_context(&model, whole.split(Split::Test), &holdout.examples, &whole.safety, &source_known, &models, &protected, &config, workers)?;
    let context_gate = production_context_gate(&config, &context);
    let load_p95 = latency["load_ms"]["p95"].as_f64().ok_or("load_ms.p95")?;
    let inference_p95 = latency["inference_ms"]["p95"].as_f64().ok_or("inference_ms.p95")?;
    let safety_false_positives: i64 = safety.per_trigger.values().map(|matrix| matrix.false_positive).sum();
    let bypasses = safety.expected_guard_samples - safety.expected_guarded_samples;
    let expected_unknown = external::PAIRED_ROWS_PER_TRIGGER.to::<i64>() * (holdout.words_by_group[0] + holdout.words_by_group[1]).to::<i64>() * TRIGGERS.len().to::<i64>();
    let mut neutral_metrics = BTreeMap::new();
    for trigger in TRIGGERS {
        neutral_metrics.insert(trigger.as_str(), (evidence.test_metrics[trigger.as_str()], evidence.typo_metrics[trigger.as_str()]));
    }
    let context_stress = fit::context_stress_gate(&config, &neutral_metrics, false);
    let typo_unknown = &slice_metrics["typo_unknown"];
    let per_trigger_fpr = |trigger: CorrectionTrigger| if trigger == CorrectionTrigger::Pause { config.pause_max_fpr } else { config.max_fpr };
    let mut gates = provenance_gates.as_object().expect("an object").clone();
    let mut gate = |name: &str, value: bool| {
        gates.insert(name.to_owned(), json!(value));
    };
    gate("provenance", provenance.iter().all(|check| check.passed));
    gate("runtime_threshold_selection_evidence", true);
    gate("sealed_test", TRIGGERS.iter().all(|trigger| slice_passes(&config, &evidence.test_metrics[trigger.as_str()], if *trigger == CorrectionTrigger::Pause { config.test_pause_recall } else { config.test_recall }, per_trigger_fpr(*trigger))));
    gate("sealed_test_context_stress", context_stress["passed"] == json!(true));
    gate("safety", safety.samples > 0 && safety.protected_samples > 0 && safety.lexical_collision_samples > 0 && safety.protected_samples + safety.lexical_collision_samples == safety.samples && safety.pre_model_guarded_samples + safety.model_evaluated_samples == safety.samples && safety.expected_guard_samples > 0 && safety.expected_guard_samples == safety.samples && 0 <= safety.expected_guarded_samples && safety.expected_guarded_samples <= safety.expected_guard_samples && safety_false_positives.max(bypasses) <= safety.guard_failure_samples && safety.guard_failure_samples <= safety_false_positives + bypasses && safety.guard_failure_samples <= config.safety_max_failures && safety.reason_counts.values().sum::<i64>() == safety.samples && safety.per_trigger.len() == TRIGGERS.len() && safety.per_trigger.values().all(|matrix| matrix.true_positive + matrix.false_negative == 0 && matrix.true_negative + matrix.false_positive > 0) && safety.per_trigger.values().map(|matrix| matrix.true_negative + matrix.false_positive).sum::<i64>() == safety.samples);
    gate("typo_unknown_recall", TRIGGERS.iter().all(|trigger| slice_passes(&config, &typo_unknown[trigger.as_str()], if *trigger == CorrectionTrigger::Pause { config.test_pause_typo_recall } else { config.test_typo_recall }, per_trigger_fpr(*trigger))));
    gate("veto", veto.false_negative_rate <= config.veto_max_fnr);
    gate("fallback_regression", comparison.introduced == 0 && comparison.ensemble.false_positive <= comparison.fallback.false_positive);
    gate("hunspell_hard_guard_regression", source_known_comparison.samples > 0 && source_known_comparison.model_evaluated == 0 && source_known_comparison.negative_model_evaluated == 0 && source_known_comparison.ensemble.false_positive == 0 && source_known_comparison.introduced == 0);
    gate("lexical_disjoint_recall", lexical_comparison.ensemble.recall() + RECALL_TOLERANCE >= lexical_comparison.fallback.recall());
    gate("unknown_typo_model_evaluated", unknown_comparison.samples == expected_unknown && unknown_comparison.model_evaluated == unknown_comparison.samples && unknown_comparison.negative_model_evaluated * external::PAIRED_ROWS_PER_TRIGGER.to::<i64>() == unknown_comparison.samples);
    gate("unknown_typo_false_positives", unknown_comparison.ensemble.false_positive <= unknown_comparison.fallback.false_positive && unknown_comparison.ensemble.specificity() >= unknown_comparison.fallback.specificity());
    gate("unknown_typo_recall", unknown_comparison.ensemble.recall() + RECALL_TOLERANCE >= unknown_comparison.fallback.recall());
    gate("unknown_typo_raw_model_integrity", unknown_metrics.values().all(|matrix| matrix.true_positive + matrix.false_negative > 0 && matrix.true_negative + matrix.false_positive > 0));
    gate("production_context_ensemble", context_gate["passed"] == json!(true));
    gate("artifact_size", latency["artifact_bytes"].as_u64().is_some_and(|bytes| bytes.to::<usize>() <= lang::KSLM_MAX_CONTAINER_BYTES));
    gate("load_latency", load_p95 <= LOAD_P95_MAX_MS);
    gate("inference_latency", inference_p95 <= INFERENCE_P95_MAX_MS);
    gate("deterministic_inference", latency["deterministic_predictions"] == json!(true));
    let strict_passed = gates.len() == STRICT_GATES.len() && STRICT_GATES.iter().all(|name| gates.get(*name) == Some(&json!(true)));
    let (overall, typos) = (pipeline::by_trigger(&evidence.test_metrics), pipeline::by_trigger(&evidence.typo_metrics));
    let context_profiles: Map<String, Value> = fit::STRESS_PROFILES.iter().map(|profile| (profile.0.to_owned(), json!({"overall": overall, "typos": typos}))).collect();
    let hunspell_policy: Map<String, Value> = LOCALES.iter().map(|locale| { let pinned = &policy["hunspell"][*locale]; ((*locale).to_owned(), json!({"dictionary_sha256": pinned["dictionary_sha256"], "dictionary_bytes": pinned["dictionary_bytes"], "affix_sha256": pinned["affix_sha256"], "affix_bytes": pinned["affix_bytes"]})) }).collect();
    let actual: Map<String, Value> = snapshots.iter().map(|snapshot| (snapshot.provenance.locale.clone(), snapshot.provenance.payload())).collect();
    let earlier: Map<String, Value> = before.iter().map(|snapshot| (snapshot.provenance.locale.clone(), snapshot.provenance.payload())).collect();
    let loaded: Map<String, Value> = snapshots.iter().map(|snapshot| (snapshot.provenance.locale.clone(), json!(snapshot.words.len()))).collect();
    let sealed_slices: Map<String, Value> = slice_metrics.iter().map(|(name, metrics)| ((*name).to_owned(), pipeline::by_trigger(metrics))).collect();
    let policy_rows: Map<String, Value> = safety.per_trigger.iter().map(|(trigger, matrix)| ((*trigger).to_owned(), metrics_payload(matrix))).collect();
    let toolchain_files: Vec<&str> = seal::TOOLCHAIN_FILES.iter().flat_map(|entry| entry.1.iter().copied()).collect();
    let constants: Vec<&str> = seal::CONSTANTS_FILES.iter().copied().filter(|file| root.join(file).is_file()).collect();
    let payload = json!({
        "model": {"version": model.model_version, "checksum": model.checksum, "dimension": model.dimension, "calibration_scope": seal::SCOPE, "training_language_scorer": scorer},
        "provenance": checks_payload(&provenance),
        "toolchain_constants": {"sha256": seal::digest_files(root, &constants)?, "files": constants, "toolchain_files": toolchain_files},
        "environment": environment_section(root)?,
        "runtime_threshold_selection": {"matches_signed_training_evidence": true, "gate_breakdown": evidence.selection_gate},
        "external_evaluation_provenance": {
            "policy": {"schema_version": policy["schema_version"], "minimum_words_per_group": policy["minimum_words_per_group"], "trigger_expansion": policy["trigger_expansion"], "hunspell": hunspell_policy, "lexical_disjoint_corpus_sha256": policy["lexical_disjoint_corpus_sha256"], "unknown_typo_development_corpus_sha256": policy["unknown_typo_development_corpus_sha256"], "unknown_typo_holdout_corpus_sha256": policy["unknown_typo_holdout_corpus_sha256"]},
            "actual_hunspell": actual,
            "before_handle_hunspell": earlier,
            "handle_snapshot_stable": stable,
            "bounded_snapshot_reads_per_file": SNAPSHOT_READS_PER_FILE,
            "words_loaded_by_group": loaded,
        },
        "reproducibility": {"scope": "input provenance, sealed dataset fingerprint, repeated load and inference", "byte_identical_retraining_executed": false, "note": "Run the deterministic trainer twice to audit byte-identical retraining."},
        "sealed_test": overall,
        "sealed_test_context_stress": {"gate_breakdown": context_stress, "profiles": context_profiles},
        "safety": {
            "qualification": "Strict safety gates use the production detector ensemble and its pre-model structural/source-known guards. Direct intent-model predictions are diagnostic only.",
            "guard_audit": {"samples": safety.samples, "protected_samples": safety.protected_samples, "lexical_collision_samples": safety.lexical_collision_samples, "pre_model_guarded_samples": safety.pre_model_guarded_samples, "expected_pre_model_guard_samples": safety.expected_guard_samples, "expected_pre_model_guarded_samples": safety.expected_guarded_samples, "model_evaluated_samples": safety.model_evaluated_samples, "guard_failure_samples": safety.guard_failure_samples, "reason_counts": safety.reason_counts, "production_policy_per_trigger": policy_rows},
            "raw_model_diagnostics": {"per_trigger": pipeline::by_trigger(&evidence.safety_metrics), "membership_coverage": coverage_diagnostics(&evidence.safety_rows)?, "is_a_gate": false},
        },
        "sealed_slices": sealed_slices,
        "veto": veto.payload(),
        "model_vs_fallback": comparison.payload(),
        "lexical_disjoint_hunspell": {
            "qualification": "Lexically disjoint from Onboard unigrams; not fully source-independent because Hunspell is also an input to the runtime language scorer. Its correct-layout negatives validate the source-known hard guard, not model false-positive behavior.",
            "words_by_group": pair_of(&lexical.words_by_group),
            "dictionary_sources": sources_of(&lexical),
            "dictionary_provenance": corpus_dictionaries(&lexical),
            "corpus_sha256": lexical.corpus_sha256,
            "comparison": lexical_comparison.payload(),
            "runtime_source_known_guard_subset": {"samples": source_known.len(), "comparison": source_known_comparison.payload()},
        },
        "lexical_disjoint_unknown_typos_development": {
            "qualification": "Policy-development corpus retained byte-for-byte from v5. Its observed metrics were used to freeze the v6/v7 serving policy, so it is never accepted as independent release evidence.",
            "is_release_gate": false,
            "words_by_group": pair_of(&development.words_by_group),
            "corpus_sha256": development.corpus_sha256,
            "rank_namespace": development.rank_namespace,
            "choice_namespace": development.choice_namespace,
            "selected_physical_signature_count": development_signatures.len(),
        },
        "lexical_disjoint_unknown_typos": {
            "qualification": "Previously unseen release holdout of deterministic physical-key typos of Hunspell-not-Onboard words; both decodings are verified unknown by the runtime scorers before the symmetric pair enters evaluation. Every physical signature emitted by a sealed train/development/calibration/threshold/test or safety row, plus every v5 development signature, is excluded before selection.",
            "role": "independent-release-holdout",
            "rank_namespace": holdout.rank_namespace,
            "choice_namespace": holdout.choice_namespace,
            "serving_policy": {"model_first_after_hard_guards": true, "post_guard_decision_rule": "trigger_direction_calibrated_logit_threshold_only", "membership_coverage_role": "diagnostic_only", "target_language_score_role": "diagnostic_only"},
            "words_by_group": pair_of(&holdout.words_by_group),
            "dictionary_sources": sources_of(&holdout),
            "dictionary_provenance": corpus_dictionaries(&holdout),
            "corpus_sha256": holdout.corpus_sha256,
            "sealed_signature_exclusion_provenance": {"schema": "keyswitch-intent-v3-holdout-exclusions", "signature_count": holdout.exclusion_signature_count, "sha256": holdout.exclusion_signature_sha256, "sealed_signature_count": index.signatures.len(), "sealed_signature_sha256": index.sha256, "development_signature_count": development_signatures.len(), "development_signature_sha256": dataset::signature_set_fingerprint(development_signatures.iter().map(String::as_str)), "split_lexical_signature_count": index.split_lexical_signature_count, "safety_lexical_signature_count": index.safety_lexical_signature_count, "protected_exact_signature_count": index.protected_exact_signature_count},
            "rejected_sealed_overlaps_by_group": pair_of(&holdout.rejected_sealed_overlaps_by_group),
            "rejected_sealed_overlaps_total": holdout.rejected_sealed_overlaps_by_group.iter().sum::<usize>(),
            "selected_sealed_overlap_count": holdout.selected_sealed_overlap_count,
            "comparison": unknown_comparison.payload(),
            "raw_model_per_trigger": pipeline::by_trigger(&unknown_metrics),
            "membership_coverage_diagnostics": coverage_diagnostics(&unknown_rows)?,
            "membership_coverage_is_a_gate": false,
        },
        "production_context_ensemble": production_context_payload(&context, &context_gate),
        "performance": latency,
        "strict_gates": gates,
        "strict_passed": strict_passed,
        "statistical_limitation": "Observed false-positive rates and Wilson upper bounds describe only the sealed synthetic lexical set, not real-world user traffic.",
    });
    Ok((pretty(&payload), i32::from(options.strict && !strict_passed)))
}

/// `STRICT_GATE_NAMES`.
pub const STRICT_GATES: [&str; 30] = ["provenance", "external_policy_schema", "external_minimum_corpus_policy", "external_trigger_expansion_policy", "external_hunspell_provenance", "hunspell_handle_snapshot_stability", "runtime_threshold_selection_evidence", "sealed_test", "sealed_test_context_stress", "safety", "typo_unknown_recall", "veto", "fallback_regression", "lexical_disjoint_size", "lexical_disjoint_corpus_provenance", "hunspell_hard_guard_regression", "lexical_disjoint_recall", "unknown_typo_development_provenance", "unknown_typo_disjoint_size", "unknown_typo_holdout_provenance", "unknown_typo_holdout_disjointness", "unknown_typo_model_evaluated", "unknown_typo_false_positives", "unknown_typo_recall", "unknown_typo_raw_model_integrity", "production_context_ensemble", "artifact_size", "load_latency", "inference_latency", "deterministic_inference"];
