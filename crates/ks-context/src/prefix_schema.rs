//! Prefix feature schemas beyond the frozen first one, and the loader that knows them.
//!
//! The installed prefix-v1 model, its frozen corpus and the lexical-compatibility gate pin `prefix_model` byte for byte,
//! so schema two (characters of the observed prefix) and the schema-aware artifact loader live here instead. The engine
//! loads the installed artifact through [`VersionedPrefixModel::default_model`], which reads either schema, so an accepted
//! schema-two pair can be installed without touching the frozen module.

use ks_cast::To as _;
use crate::constants::{
    CURRENT_PREFIX_FEATURE_VERSION, MAX_PREFIX_CHARACTER_NGRAM_ORDER, MAX_PREFIX_MODEL_BYTES, PREFIX_CHARACTER_FEATURE_WEIGHT_CAP, PREFIX_MAX_CHARACTERS,
};
use crate::context_model::ContextPrediction;
use crate::flat_table::load_artifact;
use crate::prefix_model::{actions_match, features, finish_load, weight_spec, PrefixInput, PrefixModel, PrefixSide, ARTIFACT_FILE, PREFIX_FEATURE_VERSION};
use ks_lang::LoadError;
use ks_text::pystr;
use serde_json::Value;
use std::path::Path;

/// Schema one stays frozen; schema two learns characters of observed prefixes.
///
/// The start marker describes an observed boundary. There is no end marker: an incremental prefix is not evidence that
/// the word has finished.
pub fn features_for_version(item: &PrefixInput, sides: [&PrefixSide<'_>; 2], feature_version: i64) -> Result<indexmap::IndexMap<String, f64>, LoadError> {
    if feature_version != 1 && feature_version != CURRENT_PREFIX_FEATURE_VERSION.to::<i64>() {
        return Err(LoadError::value("unsupported prefix features"));
    }
    let mut result = features(item, sides);
    if feature_version == 1 {
        return Ok(result);
    }
    for (side, text, group) in [("source", &item.original, item.source_group), ("target", &item.alternative, 1 - item.source_group)] {
        let normalized = ks_text::normalize::nfc(pystr::slice(text, None, Some(PREFIX_MAX_CHARACTERS.to::<i64>())));
        let observed: Vec<char> = std::iter::once('^').chain(pystr::casefold(&normalized).chars()).collect();
        for order in 1..=MAX_PREFIX_CHARACTER_NGRAM_ORDER {
            for start in 0..(observed.len() + 1).saturating_sub(order) {
                let gram: String = observed[start..start + order].iter().collect();
                let name = format!("{side}:prefix_char:{group}:{order}:{gram}");
                let found = result.get(&name).copied().unwrap_or(0.0);
                let total = found + 1.0;
                result.insert(name, if PREFIX_CHARACTER_FEATURE_WEIGHT_CAP < total { PREFIX_CHARACTER_FEATURE_WEIGHT_CAP } else { total });
            }
        }
    }
    Ok(result)
}

/// A prefix model that carries the feature schema its weights were fitted on.
pub struct VersionedPrefixModel {
    pub model: PrefixModel,
    pub feature_version: i64,
}

impl VersionedPrefixModel {
    pub fn version(&self) -> &str {
        &self.model.version
    }

    pub fn predict(&self, item: &PrefixInput, sides: [&PrefixSide<'_>; 2]) -> Result<ContextPrediction, LoadError> {
        Ok(self.model.predict_features(&features_for_version(item, sides, self.feature_version)?))
    }

    pub fn load(path: &Path) -> Result<VersionedPrefixModel, LoadError> {
        let (artifact, _) = load_artifact(path, MAX_PREFIX_MODEL_BYTES, "oversized prefix model", &weight_spec())?;
        let unsupported = || LoadError::value("unsupported prefix model");
        let artifact = artifact.ok_or_else(unsupported)?;
        let payload = &artifact.payload;
        let feature_version = payload.get("feature_version").and_then(Value::as_i64).filter(|&found| found == PREFIX_FEATURE_VERSION.to::<i64>() || found == CURRENT_PREFIX_FEATURE_VERSION.to::<i64>());
        let (true, Some(feature_version)) = (payload.get("kind").and_then(Value::as_str) == Some("keyswitch.prefix-policy") && actions_match(payload), feature_version) else {
            return Err(unsupported());
        };
        // `payload.get("prefix_feature_version", 1 if feature_version == 1 else None)`.
        let declared = match payload.get("prefix_feature_version") {
            Some(value) => value.as_i64(),
            None if feature_version == 1 => Some(1),
            None => None,
        };
        if declared != Some(feature_version) {
            return Err(LoadError::value("inconsistent prefix features"));
        }
        let loaded = finish_load(artifact, feature_version, &format!("prefix-v{feature_version}-"))?;
        Ok(VersionedPrefixModel { model: loaded.model, feature_version: loaded.feature_version })
    }

    /// `VersionedPrefixModel.default()`: the installed prefix artifact in whichever schema it was accepted, or none.
    pub fn default_model(models_dir: &Path) -> Option<VersionedPrefixModel> {
        VersionedPrefixModel::load(&models_dir.join(ARTIFACT_FILE)).ok()
    }
}

/// The installed prefix model for a support report: the third model, beside the intent model of the baseline and the
/// context model, the one that switches the layout while a word is typed. Whether it is available, and its version.
pub fn prefix_diagnostics(models_dir: &Path) -> (bool, String) {
    match VersionedPrefixModel::default_model(models_dir) {
        Some(model) => (true, model.version().to_string()),
        None => (false, String::new()),
    }
}
