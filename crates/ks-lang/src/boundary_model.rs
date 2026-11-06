//! Learned segmentation of a completed token with a layout-ambiguous suffix.
//!
//! This is not an intent classifier for unfinished prefixes. The observer keeps physical keys until a hard
//! boundary/idle; this model then compares the whole word with a word followed by literal punctuation. An uncertain
//! result must not authorize an automatic rewrite.

use ks_cast::To as _;
use crate::constants::{BOUNDARY_V1_FEATURE_VERSION, BOUNDARY_MAX_LITERAL_SUFFIX_CHARACTERS, BOUNDARY_FEATURE_LENGTH_SCALE_CHARACTERS, BOUNDARY_FEATURE_FREQUENCY_LOG_DIVISOR, BOUNDARY_FEATURE_NGRAM_SCORE_BOUND, BOUNDARY_THRESHOLD_EXCLUSIVE_MIN};
use crate::language_model::LanguageModel;
use crate::load_error::{json_loads, py_equals_int, read_all, LoadError};
use crate::pysum::py_sum;
use indexmap::IndexMap;
use ks_text::{max_by_key_first, max_first, pystr};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub const FEATURE_VERSION: i64 = BOUNDARY_V1_FEATURE_VERSION;
pub const MAX_SUFFIX: i64 = BOUNDARY_MAX_LITERAL_SUFFIX_CHARACTERS;

/// Feature dicts add up in their order, which is the order of insertion.
pub type Features = IndexMap<String, f64>;

/// `ARTIFACT` under the `resources` folder of the install.
#[must_use]
pub fn artifact(resource_dir: &Path) -> PathBuf {
    resource_dir.join("models").join("boundary-v1.json")
}

/// `max(low, min(high, value))` of Python: the first of equals.
fn clamped(value: f64, low: f64, high: f64) -> f64 {
    let limited = if value < high { value } else { high };
    if limited > low { limited } else { low }
}

/// Numeric evidence only: no word IDs, apps or private context memorized.
#[must_use]
pub fn features(original: &str, alternative: &str, suffix_length: i64, source: &LanguageModel, target: &LanguageModel) -> Features {
    let end = pystr::len(original).to::<i64>() - suffix_length;
    let scale = BOUNDARY_FEATURE_LENGTH_SCALE_CHARACTERS;
    let mut result = Features::with_capacity(16);
    result.insert("bias".into(), 1.0);
    result.insert("suffix".into(), suffix_length.min(MAX_SUFFIX).to::<f64>() / MAX_SUFFIX.to::<f64>());
    result.insert("length".into(), end.min(scale).to::<f64>() / scale.to::<f64>());
    result.insert("whole".into(), f64::from(suffix_length == 0));
    for (name, text, model) in [("source_word", pystr::slice(original, None, Some(end)), source), ("target_word", pystr::slice(alternative, None, Some(end)), target)] {
        let score = model.score(text);
        let letters = pystr::isalpha(text);
        result.insert(format!("{name}:letters"), f64::from(letters));
        result.insert(format!("{name}:known"), f64::from(letters && score.exact));
        result.insert(format!("{name}:frequency"), score.frequency.to::<f64>().ln_1p() / BOUNDARY_FEATURE_FREQUENCY_LOG_DIVISOR);
        let bound = BOUNDARY_FEATURE_NGRAM_SCORE_BOUND;
        result.insert(format!("{name}:ngram"), clamped(score.ngram_score, -bound, bound) / bound);
        result.insert(format!("{name}:invalid"), score.invalid_ratio);
    }
    result
}

#[derive(Clone, Debug, PartialEq)]
pub struct BoundaryPrediction {
    /// `None` = abstain, 0 = whole word.
    pub suffix_length: Option<usize>,
    /// Softmax score, not a real-user correctness probability.
    pub probability: f64,
    pub version: String,
}

#[derive(Clone, Debug)]
pub struct BoundaryModel {
    pub weights: IndexMap<String, f64>,
    pub threshold: f64,
    pub version: String,
}

impl BoundaryModel {
    pub fn new(weights: IndexMap<String, f64>, threshold: f64, version: impl Into<String>) -> Self {
        BoundaryModel { weights, threshold, version: version.into() }
    }

    /// `None` for no candidates, where the Python raises `ValueError`.
    #[must_use]
    pub fn probabilities(&self, candidates: &[Features]) -> Option<Vec<f64>> {
        let scores: Vec<f64> =
            candidates.iter().map(|values| py_sum(values.iter().map(|(name, value)| self.weights.get(name).copied().unwrap_or(0.0) * value))).collect();
        let maximum = max_first(scores.iter().copied())?;
        let exp: Vec<f64> = scores.iter().map(|value| (value - maximum).exp()).collect();
        let total = py_sum(exp.iter().copied());
        Some(exp.iter().map(|value| value / total).collect())
    }

    /// `None` for no candidates, where the Python raises `ValueError`.
    #[must_use]
    pub fn predict(&self, candidates: &[Features]) -> Option<BoundaryPrediction> {
        let probabilities = self.probabilities(candidates)?;
        let best = max_by_key_first(0..probabilities.len(), |&index| probabilities[index])?;
        let probability = probabilities[best];
        Some(BoundaryPrediction { suffix_length: (probability >= self.threshold).then_some(best), probability, version: self.version.clone() })
    }

    /// The fields both boundary artifacts share, checked alike; the check of the version differs and comes after.
    pub(crate) fn parse_fields(value: &Value, invalid: &str) -> Result<(IndexMap<String, f64>, f64, String), LoadError> {
        let fail = || LoadError::value(invalid);
        let weights = value.get("weights").and_then(Value::as_object).filter(|weights| !weights.is_empty()).ok_or_else(fail)?;
        let mut parsed = IndexMap::with_capacity(weights.len());
        for (key, weight) in weights {
            // `type(weight) in (int, float)`: a boolean is no weight, and JSON holds no infinite number.
            let Value::Number(number) = weight else {
                return Err(fail());
            };
            parsed.insert(key.clone(), number.as_f64().filter(|weight| weight.is_finite()).ok_or_else(fail)?);
        }
        let threshold = match value.get("threshold") {
            Some(Value::Number(number)) => number.as_f64().ok_or_else(fail)?,
            _ => return Err(fail()),
        };
        if !(BOUNDARY_THRESHOLD_EXCLUSIVE_MIN < threshold && threshold <= 1.0) {
            return Err(fail());
        }
        let version = value.get("version").and_then(Value::as_str).ok_or_else(fail)?.to_string();
        Ok((parsed, threshold, version))
    }

    pub fn load(path: &Path) -> Result<BoundaryModel, LoadError> {
        let value = json_loads(&read_all(path)?)?;
        if !value.is_object() || !value.get("feature_version").is_some_and(|version| py_equals_int(version, FEATURE_VERSION)) {
            return Err(LoadError::value("unsupported boundary model"));
        }
        let (weights, threshold, version) = Self::parse_fields(&value, "invalid boundary model")?;
        if version.is_empty() {
            return Err(LoadError::value("invalid boundary model"));
        }
        Ok(BoundaryModel::new(weights, threshold, version))
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn a_value_is_held_between_its_bounds() {
        assert_eq!((clamped(5.0, 0.0, 1.0), clamped(-5.0, 0.0, 1.0), clamped(0.5, 0.0, 1.0)), (1.0, 0.0, 0.5));
    }

    fn candidate(pairs: &[(&str, f64)]) -> Features {
        pairs.iter().map(|&(name, value)| (name.to_string(), value)).collect()
    }

    #[test]
    fn predicts_the_first_of_equals() {
        let weights: IndexMap<String, f64> = [("a".to_string(), 1.0), ("b".to_string(), -1.0)].into_iter().collect();
        let model = BoundaryModel::new(weights, 0.9, "v");
        let same = [candidate(&[("a", 1.0)]), candidate(&[("a", 1.0)])];
        let predicted = model.predict(&same).unwrap();
        assert_eq!((predicted.suffix_length, predicted.probability), (None, 0.5));
        let clear = [candidate(&[("b", 9.0)]), candidate(&[("a", 9.0), ("z", 5.0)])];
        assert_eq!(model.predict(&clear).unwrap().suffix_length, Some(1));
        assert_eq!(model.predict(&[]), None);
        assert_eq!(model.probabilities(&[candidate(&[])]).unwrap(), [1.0]);
    }
}
