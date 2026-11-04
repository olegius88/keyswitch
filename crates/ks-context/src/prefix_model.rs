//! A separately trained action classifier for incremental EN/RU prefixes.
//!
//! Uses the validated sparse-weight container, not the completed-word extractor or its weights. Its scores describe
//! synthetic training evidence, not a calibrated probability that a real user's intent has been understood.

use crate::constants::{
    MAX_PREFIX_FEATURE_NAME_CHARACTERS, MAX_PREFIX_MODEL_BYTES, MAX_PREFIX_WEIGHTS, MIN_PREFIX_CONVERSION_THRESHOLD, PREFIX_V1_APPLICATION_NAME_CHARACTERS,
    PREFIX_V1_APPLICATION_TOKEN_COUNT, PREFIX_V1_BEFORE_CONTEXT_CHARACTERS, PREFIX_V1_BEFORE_WORD_COUNT, PREFIX_V1_COMPLETIONS_LOG_DIVISOR,
    PREFIX_V1_FEATURE_VERSION, PREFIX_V1_FREQUENCY_LOG_DIVISOR, PREFIX_V1_LENGTH_BUCKET_MAX_CHARACTERS, PREFIX_V1_LENGTH_SCALE_CHARACTERS,
    PREFIX_V1_NGRAM_SCORE_BOUND, PREFIX_V1_QUOTE_MARKS_PER_PAIR, VERSION_HASH_CHARACTERS,
};
use crate::context_model::{first_greatest, softmax, ContextAction, ContextPrediction, ACTIONS};
use crate::flat_table::{load_artifact, Artifact, WeightSpec, WeightTable, ACTION_COUNT};
use crate::input_context::FieldContext;
use ks_lang::early_switch::PrefixIndex;
use ks_lang::{LanguageModel, LoadError};
use ks_text::{pystr, ucd};
use serde_json::Value;
use std::path::Path;

/// The file name of the installed prefix artifact, in the `models` folder.
pub const ARTIFACT_FILE: &str = "prefix_policy_v1.json";
/// `PREFIX_FEATURE_VERSION`.
pub const PREFIX_FEATURE_VERSION: usize = PREFIX_V1_FEATURE_VERSION;

pub struct PrefixInput {
    pub original: String,
    pub alternative: String,
    pub source_group: i64,
    pub field: FieldContext,
}

/// `max(-bound, min(bound, value))`.
fn clamp(value: f64, bound: f64) -> f64 {
    let capped = if value < bound { value } else { bound };
    if capped > -bound { capped } else { -bound }
}

fn flag(value: bool) -> f64 {
    f64::from(u8::from(value))
}

/// The prefix index and the language model of each layout group, by group number (`indexes[group]`, `models[group]`).
pub struct PrefixSide<'a> {
    pub index: &'a PrefixIndex,
    pub model: &'a LanguageModel,
}

/// The features of schema one, frozen. `sides` are the groups 0 and 1.
pub fn features(item: &PrefixInput, sides: [&PrefixSide<'_>; 2]) -> indexmap::IndexMap<String, f64> {
    let original = pystr::casefold(&item.original);
    let alternate = pystr::casefold(&item.alternative);
    let mut result = indexmap::IndexMap::new();
    let mut put = |name: String, value: f64| {
        result.insert(name, value);
    };
    put("bias".to_string(), 1.0);
    put(format!("direction:{}", item.source_group), 1.0);
    put(format!("length:{}", pystr::len(&original).min(PREFIX_V1_LENGTH_BUCKET_MAX_CHARACTERS)), 1.0);
    put("length".to_string(), pystr::len(&original).min(PREFIX_V1_LENGTH_SCALE_CHARACTERS) as f64 / PREFIX_V1_LENGTH_SCALE_CHARACTERS as f64);
    put("source:letters".to_string(), flag(pystr::isalpha(&original)));
    put("target:letters".to_string(), flag(pystr::isalpha(&alternate)));
    put("token:capitals".to_string(), flag(pystr::slice(&item.original, Some(1), None).chars().any(ucd::isupper)));
    put("token:digits".to_string(), flag(original.chars().any(ucd::isdigit)));
    for (name, text, group) in [("source", &original, item.source_group), ("target", &alternate, 1 - item.source_group)] {
        let side = sides[usize::try_from(group).expect("a layout group is 0 or 1")];
        let evidence = side.index.completions(text);
        let score = side.model.score(text);
        put(format!("{name}:empty"), flag(evidence.completions == 0));
        put(format!("{name}:completions"), (evidence.completions as f64).ln_1p() / PREFIX_V1_COMPLETIONS_LOG_DIVISOR as f64);
        put(format!("{name}:frequency"), (evidence.maximum_frequency as f64).ln_1p() / PREFIX_V1_FREQUENCY_LOG_DIVISOR as f64);
        put(format!("{name}:known"), flag(evidence.known || score.known));
        put(format!("{name}:invalid"), score.invalid_ratio);
        put(format!("{name}:ngram"), clamp(score.ngram_score, PREFIX_V1_NGRAM_SCORE_BOUND) / PREFIX_V1_NGRAM_SCORE_BOUND);
    }
    let before = pystr::casefold(pystr::slice(&item.field.before, Some(-(PREFIX_V1_BEFORE_CONTEXT_CHARACTERS as i64)), None));
    let line = before.rsplit('\n').next().unwrap_or("");
    let comment = line.contains("//") || line.contains("/*") || line.contains('#');
    let syntax = line.chars().any(|c| "=;{}()[]$".contains(c));
    let quote = !line.matches('"').count().is_multiple_of(PREFIX_V1_QUOTE_MARKS_PER_PAIR) || !line.matches('\'').count().is_multiple_of(PREFIX_V1_QUOTE_MARKS_PER_PAIR);
    let ru = before.chars().filter(|&c| ('а'..='я').contains(&c) || c == 'ё').count();
    let en = before.chars().filter(|c| c.is_ascii_lowercase()).count();
    let total = (ru + en).max(1) as f64;
    let context = [
        ("comment", flag(comment)),
        ("syntax", flag(syntax)),
        ("quote", flag(quote)),
        ("empty", flag(pystr::strip(&before).is_empty())),
        ("ru", ru as f64 / total),
        ("en", en as f64 / total),
        ("technical", flag(syntax && !comment && !quote)),
    ];
    let source_empty = result["source:empty"];
    for (name, value) in context {
        result.insert(format!("before:{name}"), value);
        result.insert(format!("source_empty:before:{name}"), value * source_empty);
    }
    result.insert(format!("role:{}", item.field.role.as_str()), 1.0);
    let application = pystr::casefold(&item.field.application);
    let application = pystr::slice(&application, None, Some(PREFIX_V1_APPLICATION_NAME_CHARACTERS as i64));
    for part in application.split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit())).filter(|part| !part.is_empty()).take(PREFIX_V1_APPLICATION_TOKEN_COUNT) {
        result.insert(format!("app:{part}"), 1.0);
    }
    let words: Vec<&str> = before.split(|c: char| !ucd::re_letter(c)).filter(|part| !part.is_empty()).collect();
    for word in &words[words.len().saturating_sub(PREFIX_V1_BEFORE_WORD_COUNT)..] {
        result.insert(format!("before:word:{word}"), 1.0);
    }
    result
}

pub struct PrefixModel {
    pub weights: WeightTable,
    pub version: String,
    pub conversion_threshold: f64,
}

/// What `PrefixModel.load` and `VersionedPrefixModel.load` read of an artifact, which differ in the schemas they accept.
pub(crate) struct Loaded {
    pub model: PrefixModel,
    pub feature_version: i64,
}

pub(crate) fn weight_spec() -> WeightSpec {
    WeightSpec {
        max_entries: MAX_PREFIX_WEIGHTS,
        max_name_characters: MAX_PREFIX_FEATURE_NAME_CHARACTERS,
        name_required: true,
        max_magnitude: None,
        entry_error: "invalid prefix coefficient",
        weight_error: "invalid prefix coefficient",
        table_error: "invalid prefix weights",
    }
}

/// `re.fullmatch(namespace + r"[a-f0-9]{12}", version)`.
fn versioned(version: &str, namespace: &str) -> bool {
    version.strip_prefix(namespace).is_some_and(|hash| hash.len() == VERSION_HASH_CHARACTERS && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

/// The checks both loaders share, after the schema was accepted: version, threshold, weights, checksum.
pub(crate) fn finish_load(artifact: Artifact, feature_version: i64, namespace: &str) -> Result<Loaded, LoadError> {
    let payload = &artifact.payload;
    let version = payload.get("version").and_then(Value::as_str);
    let version = match version {
        Some(version) if versioned(version, namespace) => version,
        _ => return Err(LoadError::value("invalid prefix version")),
    };
    let threshold = match payload.get("conversion_threshold") {
        Some(Value::Number(number)) => number.as_f64(),
        _ => None,
    };
    let threshold = match threshold {
        Some(threshold) if (MIN_PREFIX_CONVERSION_THRESHOLD..=1.0).contains(&threshold) => threshold,
        _ => return Err(LoadError::value("unsafe prefix threshold")),
    };
    let spec = weight_spec();
    let weights = match artifact.weights {
        Some(weights) if weights.entries > 0 && weights.entries <= spec.max_entries => weights,
        _ => return Err(LoadError::value(spec.table_error)),
    };
    if let Some(message) = weights.refusal {
        return Err(LoadError::value(message));
    }
    let digest = weights.digest.as_str();
    if payload.get("weights_sha256").and_then(Value::as_str) != Some(digest) || version != format!("{namespace}{}", &digest[..VERSION_HASH_CHARACTERS]) {
        return Err(LoadError::value("prefix checksum mismatch"));
    }
    Ok(Loaded { model: PrefixModel { version: version.to_string(), conversion_threshold: threshold, weights: weights.table }, feature_version })
}

pub(crate) fn actions_match(payload: &serde_json::Map<String, Value>) -> bool {
    payload.get("actions").and_then(Value::as_array).is_some_and(|actions| {
        actions.len() == ACTIONS.len() && actions.iter().zip(ACTIONS).all(|(found, wanted)| found.as_str() == Some(wanted.as_str()))
    })
}

impl PrefixModel {
    pub fn predict_features(&self, values: &indexmap::IndexMap<String, f64>) -> ContextPrediction {
        let mut scores = [0.0_f64; ACTION_COUNT];
        for (name, &value) in values {
            if let Some(weights) = self.weights.get(name) {
                for (index, &weight) in weights.iter().enumerate() {
                    scores[index] += weight * value;
                }
            }
        }
        let probabilities = softmax(&scores);
        let best = first_greatest(&probabilities);
        let mut action = ACTIONS[best];
        let probability = probabilities[best];
        if action == ContextAction::Convert && probability < self.conversion_threshold {
            action = ContextAction::Wait;
        }
        ContextPrediction { action, probability, probabilities, model_version: self.version.clone(), supported: true }
    }

    pub fn predict(&self, item: &PrefixInput, sides: [&PrefixSide<'_>; 2]) -> ContextPrediction {
        self.predict_features(&features(item, sides))
    }

    /// `PrefixModel.load(path)`: the frozen schema one only.
    pub fn load(path: &Path) -> Result<PrefixModel, LoadError> {
        let (artifact, _) = load_artifact(path, MAX_PREFIX_MODEL_BYTES, "oversized prefix model", &weight_spec())?;
        let unsupported = || LoadError::value("unsupported prefix model");
        let artifact = artifact.ok_or_else(unsupported)?;
        let payload = &artifact.payload;
        if payload.get("kind").and_then(Value::as_str) != Some("keyswitch.prefix-policy")
            || payload.get("feature_version").and_then(Value::as_i64) != Some(PREFIX_FEATURE_VERSION as i64)
            || !actions_match(payload)
        {
            return Err(unsupported());
        }
        Ok(finish_load(artifact, PREFIX_FEATURE_VERSION as i64, "prefix-v1-")?.model)
    }

    /// `PrefixModel.default()`: the installed artifact, or nothing.
    pub fn default_model(models_dir: &Path) -> Option<PrefixModel> {
        PrefixModel::load(&models_dir.join(ARTIFACT_FILE)).ok()
    }
}
