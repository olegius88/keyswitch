//! V2 learned ranking of literal suffixes; no source words stored in weights.
//!
//! The old rejected boundary-v1 experiment remains separately loadable for regression. Only a separately verified v2
//! artifact enables this policy. An artifact carries the feature version it was trained on (2 or 3); the policy
//! extracts exactly those features, so an installed version-2 artifact decides as before while a version-3 one also
//! sees which readings are misspellings.

use ks_cast::To as _;
use crate::boundary_model::{features as legacy_features, BoundaryModel, Features, MAX_SUFFIX};
use crate::constants::*;
use crate::language_model::LanguageModel;
use crate::load_error::{json_loads, read_limited, LoadError};
use indexmap::IndexMap;
use ks_text::pystr;
use serde_json::Value;
use std::ops::Deref;
use std::path::{Path, PathBuf};

pub const FEATURE_VERSION: i64 = BOUNDARY_V3_FEATURE_VERSION;
pub const SUPPORTED_FEATURE_VERSIONS: [i64; 2] = BOUNDARY_SUPPORTED_FEATURE_VERSIONS;

/// `ARTIFACT` under the `resources` folder of the install.
pub fn artifact(resource_dir: &Path) -> PathBuf {
    resource_dir.join("models").join("boundary-v2.json")
}

/// A reading that is no word itself but a known word with one letter missing, not its last.
pub fn misspelled(text: &str, model: &LanguageModel) -> bool {
    let word = LanguageModel::normalize(text);
    if word.is_empty() || word != pystr::casefold(text) || !pystr::isalpha(&word) || model.frequencies.get_or_zero(&word) != 0 {
        return false;
    }
    let letters = BOUNDARY_MISSING_LETTERS.iter().find(|entry| entry.0 == model.locale).map_or("", |entry| entry.1);
    let mut candidate = String::with_capacity(word.len() + 2);
    word.char_indices().any(|(offset, _)| {
        letters.chars().any(|letter| {
            candidate.clear();
            candidate.push_str(&word[..offset]);
            candidate.push(letter);
            candidate.push_str(&word[offset..]);
            model.frequencies.get_or_zero(&candidate) != 0
        })
    })
}

/// `features(original, alternative, suffix, source, target, version)`; the default version is `FEATURE_VERSION`.
pub fn features(original: &str, alternative: &str, suffix: i64, source: &LanguageModel, target: &LanguageModel, version: i64) -> Features {
    let mut values = legacy_features(original, alternative, suffix, source, target);
    // The rejected v1 experiment's features alone: the engine replay compares it, and no release loads one.
    if version == BOUNDARY_V1_FEATURE_VERSION {
        return values;
    }
    // A candidate can be an English literal or a Russian word. Learning must compare both, not inherit the old
    // ranker's mostly-English calibration.
    let (source_known, target_known) = (values["source_word:known"], values["target_word:known"]);
    let (source_ngram, target_ngram) = (values["source_word:ngram"], values["target_word:ngram"]);
    let (source_invalid, target_invalid) = (values["source_word:invalid"], values["target_word:invalid"]);
    let known = if target_known > source_known { target_known } else { source_known };
    values.insert("word:known_either".into(), known);
    values.insert("word:known_both".into(), if target_known < source_known { target_known } else { source_known });
    values.insert("word:known_length".into(), known * values["length"]);
    values.insert("word:best_ngram".into(), if target_ngram > source_ngram { target_ngram } else { source_ngram });
    values.insert("word:least_invalid".into(), if target_invalid < source_invalid { target_invalid } else { source_invalid });
    let removed = if suffix != 0 { pystr::slice(original, Some(-suffix), None) } else { "" };
    for character in BOUNDARY_AMBIGUOUS_PUNCTUATION.chars() {
        values.insert(format!("tail:has:{character}"), f64::from(removed.contains(character)));
    }
    let length = pystr::len(original).to::<i64>();
    if version != BOUNDARY_V2_FEATURE_VERSION {
        // Misspellings: which reading could be a known word typed with one key missed.
        let end = length - suffix;
        let source_misspelled = f64::from(misspelled(pystr::slice(original, None, Some(end)), source));
        let target_misspelled = f64::from(misspelled(pystr::slice(alternative, None, Some(end)), target));
        values.insert("source_word:misspelling".into(), source_misspelled);
        values.insert("target_word:misspelling".into(), target_misspelled);
        values.insert("word:misspelling_either".into(), if target_misspelled > source_misspelled { target_misspelled } else { source_misspelled });
    }
    let tail = length - (pystr::len(pystr::rstrip_chars(original, BOUNDARY_AMBIGUOUS_PUNCTUATION))).to::<i64>();
    // `range(min(tail, MAX_SUFFIX, len(original) - 1) + 1)`: nothing for the empty token, whose bound is -1 + 1.
    let candidates = 0..tail.min(MAX_SUFFIX).min(length - 1) + 1;
    let mut known_candidates = 0;
    for candidate in candidates.clone() {
        let end = length - candidate;
        let (left, right) = (pystr::slice(original, None, Some(end)), pystr::slice(alternative, None, Some(end)));
        if (pystr::isalpha(left) && source.score(left).exact) || (pystr::isalpha(right) && target.score(right).exact) {
            known_candidates += 1;
        }
    }
    let competing = f64::from(known_candidates > 1);
    let base: Vec<(String, f64)> = values.iter().map(|(name, value)| (name.clone(), *value)).collect();
    // Learned interactions distinguish a strong sole candidate from several legitimate interpretations. No threshold
    // selects a spelling by itself.
    for (name, value) in &base {
        values.insert(format!("{name}:competition"), value * competing);
    }
    if version != BOUNDARY_V2_FEATURE_VERSION {
        let rivals = if known_candidates == 0 {
            // With no reading a known word, the legitimate interpretations are the misspellings of known words;
            // several of them are the same rivalry the known words have above, learned apart.
            let count = candidates
                .filter(|candidate| {
                    let end = length - candidate;
                    misspelled(pystr::slice(original, None, Some(end)), source) || misspelled(pystr::slice(alternative, None, Some(end)), target)
                })
                .count();
            Some(f64::from(count > 1))
        } else {
            None
        };
        for (name, value) in &base {
            values.insert(format!("{name}:misspelled_rivals"), rivals.map_or(0.0, |rivals| value * rivals));
        }
    }
    values
}

/// `BoundaryPolicy(BoundaryModel)`: `probabilities`, `predict` and the fields come through `Deref`.
#[derive(Clone, Debug)]
pub struct BoundaryPolicy {
    pub model: BoundaryModel,
    pub feature_version: i64,
}

impl Deref for BoundaryPolicy {
    type Target = BoundaryModel;
    fn deref(&self) -> &BoundaryModel {
        &self.model
    }
}

impl BoundaryPolicy {
    pub fn new(weights: IndexMap<String, f64>, threshold: f64, version: impl Into<String>, feature_version: i64) -> Self {
        BoundaryPolicy { model: BoundaryModel::new(weights, threshold, version), feature_version }
    }

    /// The features of the version this artifact was trained on.
    pub fn extract(&self, original: &str, alternative: &str, suffix: i64, source: &LanguageModel, target: &LanguageModel) -> Features {
        features(original, alternative, suffix, source, target, self.feature_version)
    }

    pub fn load(path: &Path) -> Result<BoundaryPolicy, LoadError> {
        let raw = read_limited(path, BOUNDARY_POLICY_MAX_BYTES)?;
        if raw.len() > BOUNDARY_POLICY_MAX_BYTES {
            return Err(LoadError::value("oversized boundary policy"));
        }
        let value = json_loads(&raw)?;
        // `value in (2, 3)`: 2.0 is 2, and `true` is 1, which is no version here.
        let feature_version = match value.get("feature_version") {
            Some(Value::Number(number)) if value.is_object() => number.as_f64().map(|version| version.to::<i64>()).filter(|version| {
                SUPPORTED_FEATURE_VERSIONS.contains(version) && number.as_f64() == Some((*version).to::<f64>())
            }),
            _ => None,
        };
        let Some(feature_version) = feature_version else {
            return Err(LoadError::value("unsupported boundary policy"));
        };
        let (weights, threshold, version) = BoundaryModel::parse_fields(&value, "invalid boundary policy")?;
        if !version.starts_with(BOUNDARY_V2_VERSION_PREFIX) {
            return Err(LoadError::value("invalid boundary policy"));
        }
        Ok(BoundaryPolicy::new(weights, threshold, version, feature_version))
    }
}
