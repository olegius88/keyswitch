//! The whole fit of the layout-intent model, in the order of `train_intent_model.main`, and the sections of its manifest.
use super::dataset::{self, DatasetBundle, DatasetConfig, LexicalExample, PreparedLexicon, Split};
use super::fit::{self, Confusion, FitConfig, Platt, Quantized, Scored, Selections, Veto, binary_gate, context_stress_gate, metrics_payload, selection_gate};
use super::seal;
use crate::corpus::read_json;
use crate::physical::hex;
use ks_lang::detector::{ProtectedTokens, is_protected_token};
use ks_lang::intent_model::{CorrectionTrigger, TRIGGERS};
use ks_text::pynum::float_hex;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const SAFETY_COLLISION_MIN_WORD_CHARACTERS: usize = 3;
use super::seal::SCOPE;
const MANIFEST_SCHEMA_VERSION: i64 = 2;
const DIAGNOSTIC_FREQUENCY_BOUNDS: [i64; 3] = [10, 100, 1_000];
const MIN_RUNTIME_TOKEN_CHARACTERS: i64 = 5;

/// `GuardedSafetyAudit`.
pub struct SafetyAudit {
    pub samples: usize,
    pub protected_samples: usize,
    pub collision_samples: usize,
    triggers: Vec<&'static str>,
    protected_triggers: Vec<&'static str>,
    collision_triggers: Vec<&'static str>,
    pub failures: Vec<String>,
}

impl SafetyAudit {
    /// `audit_guarded_safety_corpus`: the safety rows against the production guards that stop them before the model.
    pub fn of(examples: &[LexicalExample], protected: &ProtectedTokens) -> SafetyAudit {
        let (mut observed, mut guarded, mut colliding): (BTreeSet<&str>, BTreeSet<&str>, BTreeSet<&str>) = Default::default();
        let (mut protected_samples, mut collision_samples) = (0, 0);
        let mut failures = Vec::new();
        for (index, example) in examples.iter().enumerate() {
            observed.insert(example.trigger.as_str());
            let prefix = format!("row {index} ({}, {})", example.trigger.as_str(), example.variant_kind);
            if example.label || !example.safety {
                failures.push(format!("{prefix}: row is not a sealed safety negative"));
                continue;
            }
            if example.protected {
                protected_samples += 1;
                guarded.insert(example.trigger.as_str());
                if example.variant_kind != "protected" {
                    failures.push(format!("{prefix}: protected row has the wrong kind"));
                }
                if example.source_known || example.target_known {
                    failures.push(format!("{prefix}: protected row claims lexical knowledge"));
                }
                if !is_protected_token(&example.original, protected) {
                    failures.push(format!("{prefix}: production protected-token guard does not match"));
                }
                continue;
            }
            if example.variant_kind == "lexical_collision" {
                collision_samples += 1;
                colliding.insert(example.trigger.as_str());
                if !(example.source_known && example.target_known) {
                    failures.push(format!("{prefix}: collision must be known in both layouts"));
                }
                continue;
            }
            failures.push(format!("{prefix}: no production pre-model guard applies"));
        }
        let in_order = |set: &BTreeSet<&str>| -> Vec<&'static str> { TRIGGERS.iter().map(|trigger| trigger.as_str()).filter(|trigger| set.contains(trigger)).collect() };
        SafetyAudit { samples: examples.len(), protected_samples, collision_samples, triggers: in_order(&observed), protected_triggers: in_order(&guarded), collision_triggers: in_order(&colliding), failures }
    }

    pub fn passes(&self, maximum: i64) -> bool {
        let expected: Vec<&str> = TRIGGERS.iter().map(|trigger| trigger.as_str()).collect();
        maximum >= 0 && self.samples > 0 && self.protected_samples > 0 && self.collision_samples > 0 && self.triggers == expected && self.protected_triggers == expected && self.collision_triggers == expected && self.failures.len() as i64 <= maximum
    }

    pub fn payload(&self) -> Value {
        json!({"samples": self.samples, "protected_samples": self.protected_samples, "lexical_collision_samples": self.collision_samples, "triggers": self.triggers, "protected_triggers": self.protected_triggers, "lexical_collision_triggers": self.collision_triggers, "failures": self.failures})
    }
}

/// `gate_policy_payload`: the complete, self-contained release policy.
pub fn gate_policy(raw: &Value, config: &FitConfig) -> Value {
    let external = &raw["external_evaluation"];
    let profiles: Vec<Value> = fit::STRESS_PROFILES.iter().map(|(name, delta, selector)| json!({"name": name, "delta": delta, "group_selector": selector})).collect();
    json!({
        "model_applicability": {"minimum_normalized_token_length": MIN_RUNTIME_TOKEN_CHARACTERS, "length_comparison": "maximum_of_original_and_replacement", "model_first_after_hard_guards": true, "post_guard_decision_rule": "trigger_direction_calibrated_logit_threshold_only", "membership_coverage_role": "diagnostic_only", "target_language_score_role": "diagnostic_only"},
        "sealed_evaluation": raw["sealed_evaluation"],
        "external_evaluation": {"schema_version": external["schema_version"], "minimum_words_per_group": external["minimum_words_per_group"], "trigger_expansion": external["trigger_expansion"], "hunspell": external["hunspell"], "lexical_disjoint_corpus_sha256": external["lexical_disjoint_corpus_sha256"], "unknown_typo_development_corpus_sha256": external["unknown_typo_development_corpus_sha256"], "unknown_typo_holdout_corpus_sha256": external["unknown_typo_holdout_corpus_sha256"]},
        "selection": {"context": "neutral_primary_plus_fixed_label_independent_stress", "context_stress_profiles": profiles, "minimum_precision": config.precision_floor, "minimum_recall": config.selection_recall, "minimum_pause_recall": config.selection_pause_recall, "minimum_typo_precision": config.precision_floor, "minimum_typo_recall": config.selection_typo_recall, "minimum_pause_typo_recall": config.selection_pause_typo_recall, "minimum_specificity": config.test_specificity, "maximum_false_positive_rate_familywise_upper_95": config.max_fpr, "pause_maximum_false_positive_rate_familywise_upper_95": config.pause_max_fpr, "typo_maximum_false_positive_rate_familywise_upper_95": config.max_fpr, "pause_typo_maximum_false_positive_rate_familywise_upper_95": config.pause_max_fpr, "maximum_false_positives_per_trigger": config.selection_max_false_positives, "threshold_logit_margin_policy": "maximum_feasible_not_above_signed_cap", "threshold_logit_margin_cap": config.margin_cap, "pause_logit_margin": config.pause_margin},
        "sealed_test": {"context": "neutral_primary_plus_fixed_label_independent_stress", "context_stress_profiles": profiles, "minimum_precision": config.test_precision, "minimum_recall": config.test_recall, "minimum_pause_recall": config.test_pause_recall, "minimum_typo_recall": config.test_typo_recall, "minimum_pause_typo_recall": config.test_pause_typo_recall, "minimum_specificity": config.test_specificity, "maximum_false_positive_rate_upper_95": config.max_fpr, "pause_maximum_false_positive_rate_upper_95": config.pause_max_fpr},
        "veto": {"maximum_false_negative_rate": config.veto_max_fnr},
        "safety": {"maximum_guard_failures": config.safety_max_failures},
        "statistical_bounds": {
            "selection": {"method": "wilson_score_upper_endpoint", "multiplicity_correction": "bonferroni", "familywise_confidence": fit::WILSON_CONFIDENCE, "comparisons": fit::SELECTION_COMPARISONS, "per_comparison_confidence": fit::SELECTION_PER_COMPARISON_CONFIDENCE, "z_score": fit::SELECTION_Z},
            "sealed_test": {"method": "wilson_score_upper_endpoint", "multiplicity_correction": "none", "familywise_confidence": fit::WILSON_CONFIDENCE, "comparisons": 1, "per_comparison_confidence": fit::WILSON_CONFIDENCE, "z_score": fit::WILSON_95_Z},
        },
    })
}

fn platt_payload(platt: &Platt) -> Value {
    json!({"slope": platt.slope, "intercept": platt.intercept, "sample_count": platt.sample_count, "positive_count": platt.positive_count, "provenance": SCOPE})
}

/// `DirectionalPlattCalibration.payload`.
fn calibration_payload(calibration: &[Platt; 2]) -> Value {
    json!({"schema_version": 1, "method": "independent-platt-by-layout-direction", "provenance": SCOPE, "sample_count": calibration[0].sample_count + calibration[1].sample_count, "positive_count": calibration[0].positive_count + calibration[1].positive_count, "by_direction": {"0>1": platt_payload(&calibration[0]), "1>0": platt_payload(&calibration[1])}})
}

fn stable_sigmoid(value: f64) -> f64 {
    ks_lang::intent_model::stable_sigmoid(value).expect("a finite logit")
}

/// `thresholds_payload` of `main`.
fn thresholds_payload(selections: &Selections) -> Value {
    Value::Object(TRIGGERS.iter().map(|trigger| {
        let selection = &selections[trigger.as_str()].1;
        (trigger.as_str().to_owned(), json!({"global_logit_margin": selection.margin, "logits": {"0>1": selection.logits[0], "1>0": selection.logits[1]}, "confidences": {"0>1": stable_sigmoid(selection.logits[0]), "1>0": stable_sigmoid(selection.logits[1])}, "selection_metrics": metrics_payload(&selection.metrics), "selection_typo_metrics": metrics_payload(&selection.typo_metrics)}))
    }).collect())
}

pub fn by_trigger(metrics: &BTreeMap<&'static str, Confusion>) -> Value {
    Value::Object(TRIGGERS.iter().map(|trigger| (trigger.as_str().to_owned(), metrics_payload(&metrics[trigger.as_str()]))).collect())
}

fn trigger_metrics(items: &[Scored<'_>], selections: &Selections, typos_only: bool) -> BTreeMap<&'static str, Confusion> {
    TRIGGERS.iter().map(|trigger| {
        let logits = selections[trigger.as_str()].1.logits;
        (trigger.as_str(), fit::confusion_of(items, logits, |item| item.example.trigger == *trigger && (!typos_only || item.typo)))
    }).collect()
}

fn pause_or(trigger: CorrectionTrigger, pause: f64, other: f64) -> f64 {
    if trigger == CorrectionTrigger::Pause { pause } else { other }
}

/// `training_quality_gate_breakdown`.
pub fn quality_gate(config: &FitConfig, test: &BTreeMap<&'static str, Confusion>, typos: &BTreeMap<&'static str, Confusion>, audit: &SafetyAudit, veto: &Veto, test_veto: &Veto) -> Value {
    let (mut sealed, mut typo_gates) = (Map::new(), Map::new());
    let (mut sealed_passed, mut typo_passed) = (true, true);
    let mut pairs = BTreeMap::new();
    for trigger in TRIGGERS {
        let name = trigger.as_str();
        pairs.insert(name, (test[name], typos[name]));
        let fpr = pause_or(trigger, config.pause_max_fpr, config.max_fpr);
        let overall = binary_gate(&test[name], config.test_precision, pause_or(trigger, config.test_pause_recall, config.test_recall), config.test_specificity, fpr, false);
        let typo = binary_gate(&typos[name], config.test_precision, pause_or(trigger, config.test_pause_typo_recall, config.test_typo_recall), config.test_specificity, fpr, false);
        sealed_passed &= overall["passed"] == json!(true);
        typo_passed &= typo["passed"] == json!(true);
        sealed.insert(name.to_owned(), overall);
        typo_gates.insert(name.to_owned(), typo);
    }
    let safety_passed = audit.passes(config.safety_max_failures);
    let selection_veto = veto.positive_samples > 0 && veto.false_negative_rate <= config.veto_max_fnr;
    let test_veto_passed = test_veto.positive_samples > 0 && test_veto.false_negative_rate <= config.veto_max_fnr;
    let context = context_stress_gate(config, &pairs, false);
    let with_passed = |veto: &Veto, passed: bool| { let mut value = veto.payload(); value["passed"] = json!(passed); value };
    json!({
        "passed": sealed_passed && typo_passed && context["passed"] == json!(true) && safety_passed && selection_veto && test_veto_passed,
        "all_triggers_present": true,
        "sealed_test": {"passed": sealed_passed, "per_trigger": sealed},
        "sealed_test_typos": {"passed": typo_passed, "per_trigger": typo_gates},
        "sealed_test_context_stress": context,
        "safety": {"passed": safety_passed, "actual_guard_failures": audit.failures.len(), "maximum_guard_failures": config.safety_max_failures},
        "veto": {"passed": selection_veto && test_veto_passed, "selection": with_passed(veto, selection_veto), "sealed_test": with_passed(test_veto, test_veto_passed), "maximum_false_negative_rate": config.veto_max_fnr},
    })
}

/// `quantized_model_payload_sha256` and the bytes it hashes.
pub fn model_payload(quantized: &Quantized, fingerprints: &BTreeSet<u64>) -> Vec<u8> {
    let mut payload = Vec::with_capacity(quantized.values.len() * 2 + fingerprints.len() * 8);
    for value in &quantized.values {
        payload.extend_from_slice(&value.to_le_bytes());
    }
    for value in fingerprints {
        payload.extend_from_slice(&value.to_le_bytes());
    }
    payload
}

/// `training_candidate_model_parameters`: only what the final model file must contain.
fn candidate_parameters(config: &FitConfig, quantized: &Quantized, fingerprints: &BTreeSet<u64>, bias: f64, calibration: &[Platt; 2], selections: &Selections, veto: &Veto) -> Value {
    let thresholds: Map<String, Value> = TRIGGERS.iter().map(|trigger| { let logits = selections[trigger.as_str()].1.logits; (trigger.as_str().to_owned(), json!({"0>1": float_hex(logits[0]), "1>0": float_hex(logits[1])})) }).collect();
    json!({"dimension": config.dimension, "payload_sha256": hex(&Sha256::digest(model_payload(quantized, fingerprints))), "weight_scale_hex": float_hex(quantized.scale), "bias_hex": float_hex(bias),
        "platt_calibration_hex": {"0>1": {"scale": float_hex(calibration[0].slope), "bias": float_hex(calibration[0].intercept)}, "1>0": {"scale": float_hex(calibration[1].slope), "bias": float_hex(calibration[1].intercept)}},
        "threshold_logits_hex": thresholds, "veto_threshold_hex": float_hex(veto.raw_logit), "feature_hash_seed": config.feature_hash_seed, "membership_hash_seed": config.membership_hash_seed, "ngram_orders": [1, 2, 3, 4, 5]})
}

/// `load_onboard_unigrams` of both frozen sources: the words of each group, with the short ones the safety rows need.
pub fn onboard_lexicon(root: &Path, raw_config: &Value, dataset_config: &DatasetConfig) -> Result<[Vec<dataset::LexiconWord>; 2], String> {
    let languages = &raw_config["sources"]["languages"];
    let load = |locale: &str, group: i64| -> Result<Vec<dataset::LexiconWord>, String> { Ok(dataset::load_onboard_unigrams(&dataset::read_frozen(root, &languages[locale])?, group, SAFETY_COLLISION_MIN_WORD_CHARACTERS, dataset_config.maximum_word_length)) };
    Ok([load("en_US", 0)?, load("ru_RU", 1)?])
}

/// `dataset` of the config.
pub fn dataset_config(raw_config: &Value) -> Result<DatasetConfig, String> {
    let shape = &raw_config["dataset"];
    let number = |name: &str| shape[name].as_u64().map(|value| value as usize).ok_or_else(|| format!("dataset.{name}"));
    Ok(DatasetConfig { minimum_word_length: number("minimum_word_length")?, maximum_word_length: number("maximum_word_length")?, typo_augmentations: number("typo_augmentations")? })
}

/// What a run holds before anything is fitted: everything a fit reads, in the order `main` reads it.
pub struct Presealed {
    pub raw_config: Value,
    pub dataset_config: DatasetConfig,
    pub words: [Vec<dataset::LexiconWord>; 2],
    pub prepared: PreparedLexicon,
    pub base: DatasetBundle,
    pub dataset: DatasetBundle,
    pub hard: dataset::HardNegativeCorpus,
}

/// Load the frozen sources and build the presealed dataset.
pub fn presealed_dataset(root: &Path) -> Result<Presealed, String> {
    let raw_config = read_json(&root.join("model/intent_v1/config.json"))?;
    let dataset_config = dataset_config(&raw_config)?;
    let words = onboard_lexicon(root, &raw_config, &dataset_config)?;
    let prepared = dataset::prepare_lexicon(words.iter().flatten().cloned().collect(), dataset_config.minimum_word_length);
    let protected = ProtectedTokens::load(&root.join("resources"));
    let raw = dataset::read_frozen(root, &raw_config["hard_negative_development"]["source"])?;
    let hard = dataset::decode_hard_negative_corpus(&raw, &raw_config, &dataset_config, &protected)?;
    let base = dataset::build_dataset(&prepared, &dataset_config, &Split::PRESEALED);
    let built = dataset::merge_hard_negative_development(base.clone(), &hard);
    dataset::assert_no_split_leakage(&built, &Split::PRESEALED)?;
    Ok(Presealed { raw_config, dataset_config, words, prepared, base, dataset: built, hard })
}

fn references<'a>(rows: &'a [LexicalExample]) -> Vec<&'a LexicalExample> {
    rows.iter().collect()
}

fn diagnostic_length_bucket(example: &LexicalExample) -> String {
    let length = example.original.chars().count().max(example.alternative.chars().count());
    let (exact, short, medium, long) = (ks_lang::constants::INTENT_LENGTH_EXACT_MAX_CHARACTERS, ks_lang::constants::INTENT_LENGTH_SHORT_MAX_CHARACTERS, ks_lang::constants::INTENT_LENGTH_MEDIUM_MAX_CHARACTERS, ks_lang::constants::INTENT_LENGTH_LONG_MAX_CHARACTERS);
    if length <= exact {
        return format!("1-{exact}");
    }
    if length <= short {
        return format!("{}-{short}", exact + 1);
    }
    if length <= medium {
        return format!("{}-{medium}", short + 1);
    }
    if length <= long {
        return format!("{}-{long}", medium + 1);
    }
    format!("{}+", long + 1)
}

fn diagnostic_frequency_bucket(frequency: i64) -> String {
    let (first, second, third) = (DIAGNOSTIC_FREQUENCY_BOUNDS[0], DIAGNOSTIC_FREQUENCY_BOUNDS[1], DIAGNOSTIC_FREQUENCY_BOUNDS[2]);
    if frequency <= 0 {
        "0".to_owned()
    } else if frequency < first {
        format!("1-{}", first - 1)
    } else if frequency < second {
        format!("{first}-{}", second - 1)
    } else if frequency < third {
        format!("{second}-{}", third - 1)
    } else {
        format!("{third}+")
    }
}

/// `_selection_metric_slices`.
fn selection_metric_slices(items: &[Scored<'_>], logits: [f64; 2]) -> Value {
    let mut grouped: BTreeMap<&str, BTreeMap<String, Vec<Scored<'_>>>> = BTreeMap::new();
    for item in items {
        let example = item.example;
        let intended = if example.label { example.target_group } else { example.source_group };
        let categories = [("direction", format!("{}>{}", example.source_group, example.target_group)), ("intended_group", intended.to_string()), ("variant_kind", example.variant_kind.clone()), ("length", diagnostic_length_bucket(example)), ("frequency", diagnostic_frequency_bucket(example.frequency))];
        for (dimension, category) in categories {
            grouped.entry(dimension).or_default().entry(category).or_default().push(*item);
        }
    }
    Value::Object(grouped.into_iter().map(|(dimension, categories)| (dimension.to_owned(), Value::Object(categories.into_iter().map(|(category, rows)| (category, metrics_payload(&fit::confusion_of(&rows, logits, |_| true)))).collect()))).collect())
}

/// `_recall_target_threshold`.
fn recall_target_threshold(items: &[Scored<'_>], target: f64, typos_only: bool) -> Result<f64, String> {
    let mut scores: Vec<f64> = items.iter().filter(|item| item.example.label && (!typos_only || item.typo)).map(|item| item.calibrated).collect();
    if scores.is_empty() {
        return Err(format!("selection diagnostics require {} examples", if typos_only { "typo-positive" } else { "positive" }));
    }
    scores.sort_by(|a, b| b.partial_cmp(a).expect("finite logits"));
    let required = scores.len().min((target * scores.len() as f64).ceil().max(1.0) as usize);
    Ok(scores[required - 1])
}

/// `_selection_point_payload`.
fn selection_point(items: &[Scored<'_>], threshold: f64, trigger: CorrectionTrigger, config: &FitConfig) -> Value {
    let fpr = pause_or(trigger, config.pause_max_fpr, config.max_fpr);
    let overall = fit::confusion_of(items, [threshold, threshold], |_| true);
    let typos = fit::confusion_of(items, [threshold, threshold], |item| item.typo);
    json!({"logit": threshold, "overall": binary_gate(&overall, config.precision_floor, pause_or(trigger, config.selection_pause_recall, config.selection_recall), config.test_specificity, fpr, true), "typos": binary_gate(&typos, config.precision_floor, pause_or(trigger, config.selection_pause_typo_recall, config.selection_typo_recall), config.test_specificity, fpr, true)})
}

/// `selection_tail_diagnostics`: a pre-sealed tail failure explained without lexical tokens.
fn selection_tail_diagnostics(scored: &[Scored<'_>], selections: &Selections, config: &FitConfig) -> Result<Value, String> {
    let mut per_trigger = Map::new();
    for trigger in TRIGGERS {
        let selected: Vec<Scored<'_>> = scored.iter().filter(|item| item.example.trigger == trigger).copied().collect();
        if selected.is_empty() {
            return Err(format!("selection diagnostics require examples for {}", trigger.as_str()));
        }
        let (ordinary_target, typo_target) = (pause_or(trigger, config.selection_pause_recall, config.selection_recall), pause_or(trigger, config.selection_pause_typo_recall, config.selection_typo_recall));
        let ordinary = recall_target_threshold(&selected, ordinary_target, false)?;
        let typo = recall_target_threshold(&selected, typo_target, true)?;
        let logits = selections[trigger.as_str()].1.logits;
        per_trigger.insert(trigger.as_str().to_owned(), json!({"selected_logits": {"0>1": logits[0], "1>0": logits[1]}, "selected_metric_slices": selection_metric_slices(&selected, logits), "ordinary_recall_target": ordinary_target, "ordinary_target_point": selection_point(&selected, ordinary, trigger, config), "typo_recall_target": typo_target, "typo_target_point": selection_point(&selected, typo, trigger, config)}));
    }
    Ok(json!({"schema_version": 1, "source_split": "threshold", "contains_lexical_tokens": false, "per_trigger": per_trigger}))
}

/// `presealed_candidate_gate_breakdown`: every already-known release failure, before the sealed test is opened.
fn presealed_gate(config: &FitConfig, selection: &Value, audit: &SafetyAudit, veto: &Veto) -> Value {
    let selection_passed = selection["passed"] == json!(true);
    let safety_passed = audit.passes(config.safety_max_failures);
    let veto_passed = veto.positive_samples > 0 && veto.false_negative_rate <= config.veto_max_fnr;
    let mut veto_gate = veto.payload();
    veto_gate["passed"] = json!(veto_passed);
    veto_gate["maximum_false_negative_rate"] = json!(config.veto_max_fnr);
    json!({"passed": selection_passed && safety_passed && veto_passed, "threshold_selection": selection, "safety": {"passed": safety_passed, "actual_guard_failures": audit.failures.len(), "maximum_guard_failures": config.safety_max_failures}, "veto_selection": veto_gate})
}

/// A candidate: everything fixed before the sealed test is exposed.
pub struct Candidate {
    pub raw_config: Value,
    pub config: FitConfig,
    pub dataset_config: DatasetConfig,
    pub prepared: PreparedLexicon,
    pub dataset: DatasetBundle,
    pub candidate_dataset_sha256: String,
    pub quantized: Quantized,
    pub fingerprints: BTreeSet<u64>,
    pub bias: f64,
    pub calibration: [Platt; 2],
    pub selections: Selections,
    pub veto: Veto,
    pub parameters: Value,
    pub audit: SafetyAudit,
    /// The sections of the manifest known so far.
    pub sections: Map<String, Value>,
    pub presealed_gate: Value,
    pub diagnostic: Option<Value>,
}

impl Candidate {
    /// `presealed_candidate_counts`: only what can be counted before the held-out split is built.
    pub fn counts(&self) -> Value {
        let examples: Map<String, Value> = Split::PRESEALED.iter().map(|split| (split.as_str().to_owned(), json!(self.dataset.split(*split).len()))).collect();
        json!({"examples": examples, "safety_examples": self.dataset.safety.len(), "quarantined_variant_occurrences": self.dataset.variant_quarantine.occurrences.len(), "quarantined_physical_signatures": self.dataset.variant_quarantine.signature_count()})
    }

    /// `presealed_candidate_metadata_projection`.
    pub fn metadata(&self, config_sha256: &str, toolchain: &Value) -> Value {
        let section = |name: &str| &self.sections[name];
        seal::metadata_projection(config_sha256, dataset::SPLIT_NAMESPACE, toolchain, section("source_package"), section("sources"), &self.counts(), &self.dataset.variant_quarantine.sha256, section("training_language_scorer"), section("gate_policy"), section("training"), section("quantization"), section("calibration"), &self.veto.payload(), section("thresholds"), section("threshold_selection_gate_breakdown"), &self.audit.payload(), &self.parameters)
    }
}

/// `sources` of the manifest: each frozen language source and what declares its license.
pub fn source_rows(raw_config: &Value) -> Value {
    let sources = &raw_config["sources"];
    let row = |locale: &str| { let language = &sources["languages"][locale]; json!({"locale": locale, "group": language["group"], "path": language["path"], "sha256": language["sha256"], "bytes": language["bytes"], "license_declaration": sources["license_declaration"], "license_evidence": sources["license_evidence"]["path"]}) };
    json!([row("en_US"), row("ru_RU")])
}

/// The whole fit up to the candidate. Nothing is written and the held-out rows are not built.
pub fn fit_candidate(root: &Path, workers: usize) -> Result<Candidate, String> {
    let Presealed { raw_config, dataset_config, prepared, dataset: dataset_candidate, hard, .. } = presealed_dataset(root)?;
    let config = FitConfig::read(&raw_config)?;
    fit::verify_context_invariance(&config)?;
    let protected = ProtectedTokens::load(&root.join("resources"));
    let candidate_dataset_sha256 = dataset::dataset_fingerprint(&dataset_candidate);
    let audit = SafetyAudit::of(&dataset_candidate.safety, &protected);
    let mut fingerprints = BTreeSet::new();
    eprintln!("intent: features of train");
    let training_rows = references(dataset_candidate.split(Split::Train));
    let training = fit::featurize(&training_rows, &config, workers, Some(&mut fingerprints))?;
    let development_rows = references(dataset_candidate.split(Split::Development));
    let development = fit::featurize(&development_rows, &config, workers, None)?;
    eprintln!("intent: FTRL over {} rows", training.examples.len());
    let result = fit::fit_ftrl(&training, &development, &config, workers)?;
    drop(training);
    drop(development);
    let bias = result.model.bias();
    let weights = result.model.sparse_weights();
    let quantized = fit::quantize(&weights, config.dimension);
    let calibration_rows = references(dataset_candidate.split(Split::Calibration));
    let calibration_featured = fit::featurize(&calibration_rows, &config, workers, None)?;
    let calibration_raw = fit::score_quantized(&quantized, bias, &calibration_featured, workers);
    let calibration = fit::fit_calibration(&calibration_featured, &calibration_raw, &config)?;
    if calibration.iter().any(|platt| platt.slope <= 0.0) {
        return Err("calibration reversed the model score orientation".to_owned());
    }
    let calibration_scored = fit::calibrate(&calibration_featured, &calibration_raw, &calibration);
    let veto = fit::choose_veto(&calibration_scored, &config);
    drop(calibration_scored);
    drop(calibration_featured);
    let threshold_rows = references(dataset_candidate.split(Split::Threshold));
    let threshold_featured = fit::featurize(&threshold_rows, &config, workers, None)?;
    let threshold_raw = fit::score_quantized(&quantized, bias, &threshold_featured, workers);
    let threshold_scored = fit::calibrate(&threshold_featured, &threshold_raw, &calibration);
    let selections = fit::choose_thresholds(&threshold_scored, &config)?;
    let selection = selection_gate(&config, &selections);
    let presealed = presealed_gate(&config, &selection, &audit, &veto);
    let diagnostic = if presealed["passed"] == json!(true) { None } else { Some(selection_tail_diagnostics(&threshold_scored, &selections, &config)?) };
    drop(threshold_scored);
    drop(threshold_featured);
    let parameters = candidate_parameters(&config, &quantized, &fingerprints, bias, &calibration, &selections, &veto);
    let source_package = json!({"name": raw_config["sources"]["package"], "version": raw_config["sources"]["package_version"], "license_declaration": raw_config["sources"]["license_declaration"], "license_evidence": raw_config["sources"]["license_evidence"], "license_evidence_verified_path": raw_config["sources"]["license_evidence"]["path"]});
    let scorer = dataset::train_only_scorer_provenance(&prepared, &dataset_candidate.variant_quarantine);
    let training_payload = json!({"seed": config.seed, "best_epoch": result.best_epoch, "history": result.history, "nonzero_weights": weights.len(), "bias": bias, "supported_character_fingerprints": fingerprints.len(), "hard_negative_development": hard.provenance()});
    let support_bits: u32 = quantized.support.iter().map(|byte| byte.count_ones()).sum();
    let quantization = json!({"format": "signed-int16", "scale": quantized.scale, "maximum_absolute_error": quantized.maximum_absolute_error, "quantized_nonzero_weights": support_bits});
    let mut sections = Map::new();
    sections.insert("schema_version".to_owned(), json!(MANIFEST_SCHEMA_VERSION));
    sections.insert("model_id".to_owned(), json!(seal::MODEL_ID));
    sections.insert("calibration_scope".to_owned(), json!(SCOPE));
    sections.insert("split_namespace".to_owned(), json!(dataset::SPLIT_NAMESPACE));
    sections.insert("source_package".to_owned(), source_package);
    sections.insert("sources".to_owned(), source_rows(&raw_config));
    sections.insert("training_language_scorer".to_owned(), scorer);
    sections.insert("gate_policy".to_owned(), gate_policy(&raw_config, &config));
    sections.insert("training".to_owned(), training_payload);
    sections.insert("quantization".to_owned(), quantization);
    sections.insert("calibration".to_owned(), calibration_payload(&calibration));
    sections.insert("thresholds".to_owned(), thresholds_payload(&selections));
    sections.insert("threshold_selection_gate_breakdown".to_owned(), selection);
    sections.insert("variant_quarantine_sha256".to_owned(), json!(dataset_candidate.variant_quarantine.sha256));
    Ok(Candidate { raw_config, config, dataset_config, prepared, dataset: dataset_candidate, candidate_dataset_sha256, quantized, fingerprints, bias, calibration, selections, veto, parameters, audit, sections, presealed_gate: presealed, diagnostic })
}

/// `counts` of the manifest, over the dataset with the held-out rows.
pub fn sealed_counts(prepared: &PreparedLexicon, whole: &DatasetBundle) -> Value {
    let lexicon_words: usize = prepared.words_by_split.iter().map(Vec::len).sum();
    let examples: Map<String, Value> = Split::ALL.iter().map(|split| (split.as_str().to_owned(), json!(whole.split(*split).len()))).collect();
    json!({"lexicon_words": lexicon_words, "collisions": prepared.collisions.len(), "examples": examples, "safety_examples": whole.safety.len(), "quarantined_variant_occurrences": whole.variant_quarantine.occurrences.len(), "quarantined_physical_signatures": whole.variant_quarantine.signature_count(), "sealed_quarantined_variant_occurrences": whole.sealed_variant_quarantine.occurrences.len(), "sealed_quarantined_physical_signatures": whole.sealed_variant_quarantine.signature_count(), "sealed_test_exclusion_signatures": whole.sealed_test_exclusion_signatures.len()})
}

/// `quality_gate_breakdown` and the sealed sections of the manifest: the held-out rows, opened. `main` does this only after
/// the claim of the seal, which is the caller's to make.
pub fn open_sealed_test(mut candidate: Candidate, root: &Path, workers: usize) -> Result<(Candidate, bool), String> {
    let protected = ProtectedTokens::load(&root.join("resources"));
    let config = candidate.config.clone();
    let sealed = dataset::build_dataset(&candidate.prepared, &candidate.dataset_config, &[Split::Test]);
    dataset::assert_no_split_leakage(&sealed, &[Split::Test])?;
    let whole = dataset::merge_sealed_test_dataset(std::mem::take(&mut candidate.dataset), sealed)?;
    dataset::assert_no_split_leakage(&whole, &Split::PRESEALED)?;
    let dataset_sha256 = dataset::dataset_fingerprint(&whole);
    let audit = SafetyAudit::of(&whole.safety, &protected);
    let (gates_passed, sealed_sections) = {
        let test_rows = references(whole.split(Split::Test));
        let test_featured = fit::featurize(&test_rows, &config, workers, None)?;
        let test_raw = fit::score_quantized(&candidate.quantized, candidate.bias, &test_featured, workers);
        let test_scored = fit::calibrate(&test_featured, &test_raw, &candidate.calibration);
        let safety_rows = references(&whole.safety);
        let safety_featured = fit::featurize(&safety_rows, &config, workers, None)?;
        let safety_raw = fit::score_quantized(&candidate.quantized, candidate.bias, &safety_featured, workers);
        let safety_scored = fit::calibrate(&safety_featured, &safety_raw, &candidate.calibration);
        let test_veto = fit::veto_metrics(&test_scored, candidate.veto.raw_logit);
        let test_metrics = trigger_metrics(&test_scored, &candidate.selections, false);
        let typo_metrics = trigger_metrics(&test_scored, &candidate.selections, true);
        let safety_metrics = trigger_metrics(&safety_scored, &candidate.selections, false);
        let (overall, typos) = (by_trigger(&test_metrics), by_trigger(&typo_metrics));
        let context: Map<String, Value> = fit::STRESS_PROFILES.iter().map(|profile| (profile.0.to_owned(), json!({"overall": overall, "typos": typos}))).collect();
        let quality = quality_gate(&config, &test_metrics, &typo_metrics, &audit, &candidate.veto, &test_veto);
        let mut sections = Map::new();
        sections.insert("veto".to_owned(), json!({"selection": candidate.veto.payload(), "sealed_test": test_veto.payload()}));
        sections.insert("sealed_test".to_owned(), overall);
        sections.insert("sealed_test_typos".to_owned(), typos);
        sections.insert("sealed_test_context_stress".to_owned(), Value::Object(context));
        sections.insert("safety".to_owned(), json!({"guard_audit": audit.payload(), "raw_model_diagnostics": by_trigger(&safety_metrics)}));
        let passed = quality["passed"] == json!(true);
        sections.insert("quality_gate_breakdown".to_owned(), quality);
        sections.insert("quality_gates_passed".to_owned(), json!(passed));
        sections.insert("counts".to_owned(), sealed_counts(&candidate.prepared, &whole));
        sections.insert("sealed_variant_quarantine_sha256".to_owned(), json!(whole.sealed_variant_quarantine.sha256));
        sections.insert("sealed_test_exclusion_signatures_sha256".to_owned(), json!(hex(&Sha256::digest(whole.sealed_test_exclusion_signatures.join("\n").as_bytes()))));
        sections.insert("dataset_sha256".to_owned(), json!(dataset_sha256));
        (passed, sections)
    };
    candidate.sections.extend(sealed_sections);
    candidate.dataset = whole;
    Ok((candidate, gates_passed))
}
