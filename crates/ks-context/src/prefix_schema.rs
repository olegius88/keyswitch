//! Prefix feature schemas beyond the frozen first one, and the loader that knows them.
//!
//! The installed prefix-v1 model, its frozen corpus and the lexical-compatibility gate pin `prefix_model` byte for byte,
//! so schema two (characters of the observed prefix) and the schema-aware artifact loader live here instead. The engine
//! loads the installed artifact through [`VersionedPrefixModel::default_model`], which reads either schema, so an accepted
//! schema-two pair can be installed without touching the frozen module.

use ks_cast::To as _;
use crate::constants::{
    CURRENT_PREFIX_FEATURE_VERSION, MAX_PREFIX_CHARACTER_NGRAM_ORDER, MAX_PREFIX_MODEL_BYTES, ORTHO_PREFIX_FEATURE_VERSION, PREFIX_CHARACTER_FEATURE_WEIGHT_CAP,
    PREFIX_MAX_CHARACTERS, PREFIX_ORTHO_LENGTH_BUCKETS, PREFIX_ORTHO_MAX_STEPS, PREFIX_ORTHO_MEAN_BOUND_NATS, PREFIX_ORTHO_STEP_NATS, PREFIX_ORTHO_SUM_BOUND_NATS,
    PREFIX_ORTHO_WORST_BOUND_NATS,
};
use crate::context_model::ContextPrediction;
use crate::flat_table::load_artifact;
use crate::prefix_model::{actions_match, features, finish_load, weight_spec, PrefixInput, PrefixModel, PrefixSide, ARTIFACT_FILE, PREFIX_FEATURE_VERSION};
use ks_lang::LoadError;
use ks_lang::ortho_model::{BOS, Channel, OrthoModel};
use ks_text::pystr;
use serde_json::Value;
use std::path::Path;

/// Schema one stays frozen; schema two learns characters of observed prefixes.
///
/// The start marker describes an observed boundary. There is no end marker: an incremental prefix is not evidence that
/// the word has finished.
pub fn features_for_version(item: &PrefixInput, sides: [&PrefixSide<'_>; 2], feature_version: i64) -> Result<indexmap::IndexMap<String, f64>, LoadError> {
    features_with_ortho(item, sides, feature_version, None)
}

/// The features of any schema; schema three reads the keys with the orthotactic models and refuses without them.
pub fn features_with_ortho(item: &PrefixInput, sides: [&PrefixSide<'_>; 2], feature_version: i64, ortho: Option<&OrthoModel>) -> Result<indexmap::IndexMap<String, f64>, LoadError> {
    if feature_version == ORTHO_PREFIX_FEATURE_VERSION.to::<i64>() {
        let ortho = ortho.ok_or_else(|| LoadError::value("prefix schema three needs the orthotactic model"))?;
        return Ok(schema_three(item, sides, ortho));
    }
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

/// log P(key | the keys before it) of every key, a word's start before them and no end after them: a prefix is not
/// evidence that the word has finished. `Channel::score` walks the same backoff chain with an end marker.
#[must_use]
pub fn prefix_conditionals(channel: &Channel, keys: &str) -> Vec<f64> {
    let mut padded: Vec<char> = vec![BOS; channel.order.saturating_sub(1)];
    padded.extend(keys.chars());
    padded
        .windows(channel.order)
        .map(|window| {
            let mut gram = window;
            let mut accumulated = 0.0;
            loop {
                if let Some(found) = channel.logprob.get_chars(gram) {
                    break accumulated + found;
                }
                if gram.len() == 1 {
                    break accumulated + channel.uniform;
                }
                accumulated += channel.backoff.get_chars(&gram[..gram.len() - 1]).unwrap_or(0.0);
                gram = &gram[1..];
            }
        })
        .collect()
}

fn bounded(value: f64, bound: f64) -> f64 {
    (value / bound).clamp(-1.0, 1.0)
}

/// Schema three: schema one without the application's name and the words before the token, which a model learns by
/// heart rather than as evidence; the characters of the typed prefix on its own side only, since the other side's are
/// the same keys read in the other layout; and the orthotactic models' reading of the keys. The last is what tells a
/// rare word or a name typed in the wrong layout from a word typed as meant when no lexicon holds either reading:
/// how much better the other layout's language explains the keys than the typed one's, summed, per key and at the
/// worst key of each reading, by length.
fn schema_three(item: &PrefixInput, sides: [&PrefixSide<'_>; 2], ortho: &OrthoModel) -> indexmap::IndexMap<String, f64> {
    let mut result = features(item, sides);
    result.retain(|name, _| !name.starts_with("app:") && !name.starts_with("before:word:"));
    let normalized = ks_text::normalize::nfc(pystr::slice(&item.original, None, Some(PREFIX_MAX_CHARACTERS.to::<i64>())));
    let observed: Vec<char> = std::iter::once('^').chain(pystr::casefold(&normalized).chars()).collect();
    for order in 1..=MAX_PREFIX_CHARACTER_NGRAM_ORDER {
        for start in 0..(observed.len() + 1).saturating_sub(order) {
            let gram: String = observed[start..start + order].iter().collect();
            let name = format!("source:prefix_char:{}:{order}:{gram}", item.source_group);
            let total = result.get(&name).copied().unwrap_or(0.0) + 1.0;
            result.insert(name, total.min(PREFIX_CHARACTER_FEATURE_WEIGHT_CAP));
        }
    }
    // The channels read keys of the US layout: the typed text itself, or its reading there.
    let keys = pystr::casefold(&ks_text::normalize::nfc(if item.source_group == 0 { &item.original } else { &item.alternative }));
    let (typed, other) = if item.source_group == 0 { (&ortho.channels.en, &ortho.channels.ru) } else { (&ortho.channels.ru, &ortho.channels.en) };
    let (typed, other) = (prefix_conditionals(typed, &keys), prefix_conditionals(other, &keys));
    let count = typed.len();
    if count == 0 {
        return result;
    }
    let bucket = PREFIX_ORTHO_LENGTH_BUCKETS.iter().rev().find(|bound| count >= **bound).copied().unwrap_or(PREFIX_ORTHO_LENGTH_BUCKETS[0]);
    let difference = ks_lang::pysum::py_sum(other.iter().copied()) - ks_lang::pysum::py_sum(typed.iter().copied());
    let worst = |values: &[f64]| values.iter().copied().fold(f64::INFINITY, f64::min);
    let step = (difference / PREFIX_ORTHO_STEP_NATS).floor().clamp(-PREFIX_ORTHO_MAX_STEPS.to::<f64>(), PREFIX_ORTHO_MAX_STEPS.to::<f64>());
    result.insert(format!("ortho:sum:{bucket}"), bounded(difference, PREFIX_ORTHO_SUM_BOUND_NATS));
    result.insert(format!("ortho:mean:{bucket}"), bounded(difference / count.to::<f64>(), PREFIX_ORTHO_MEAN_BOUND_NATS));
    result.insert(format!("ortho:step:{bucket}:{}", step.to::<i64>()), 1.0);
    result.insert("ortho:worst_typed".to_owned(), bounded(worst(&typed), PREFIX_ORTHO_WORST_BOUND_NATS));
    result.insert("ortho:worst_other".to_owned(), bounded(worst(&other), PREFIX_ORTHO_WORST_BOUND_NATS));
    result
}

/// A prefix model that carries the feature schema its weights were fitted on.
pub struct VersionedPrefixModel {
    pub model: PrefixModel,
    pub feature_version: i64,
}

impl VersionedPrefixModel {
    #[must_use]
    pub fn version(&self) -> &str {
        &self.model.version
    }

    pub fn predict(&self, item: &PrefixInput, sides: [&PrefixSide<'_>; 2]) -> Result<ContextPrediction, LoadError> {
        self.predict_with(item, sides, None)
    }

    /// The prediction with the orthotactic models at hand, which schema three reads.
    pub fn predict_with(&self, item: &PrefixInput, sides: [&PrefixSide<'_>; 2], ortho: Option<&OrthoModel>) -> Result<ContextPrediction, LoadError> {
        Ok(self.model.predict_features(&features_with_ortho(item, sides, self.feature_version, ortho)?))
    }

    /// Whether the schema reads the keys with the orthotactic models, so the model answers only beside them.
    #[must_use]
    pub fn needs_ortho(&self) -> bool {
        self.feature_version == ORTHO_PREFIX_FEATURE_VERSION.to::<i64>()
    }

    pub fn load(path: &Path) -> Result<VersionedPrefixModel, LoadError> {
        let (artifact, _) = load_artifact(path, MAX_PREFIX_MODEL_BYTES, "oversized prefix model", &weight_spec())?;
        let unsupported = || LoadError::value("unsupported prefix model");
        let artifact = artifact.ok_or_else(unsupported)?;
        let payload = &artifact.payload;
        let feature_version = payload.get("feature_version").and_then(Value::as_i64).filter(|&found| [PREFIX_FEATURE_VERSION, CURRENT_PREFIX_FEATURE_VERSION, ORTHO_PREFIX_FEATURE_VERSION].iter().any(|known| found == known.to::<i64>()));
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
    #[must_use]
    pub fn default_model(models_dir: &Path) -> Option<VersionedPrefixModel> {
        VersionedPrefixModel::load(&models_dir.join(ARTIFACT_FILE)).ok()
    }
}

/// The installed prefix model for a support report: the third model, beside the intent model of the baseline and the
/// context model, the one that switches the layout while a word is typed. Whether it is available, and its version.
#[must_use]
pub fn prefix_diagnostics(models_dir: &Path) -> (bool, String) {
    match VersionedPrefixModel::default_model(models_dir) {
        Some(model) => (true, model.version().to_string()),
        None => (false, String::new()),
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use serde_json::json;

    use super::*;

    fn real() -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/models/prefix_policy_v1.json");
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn loaded(value: &Value) -> Result<VersionedPrefixModel, LoadError> {
        let path = std::env::temp_dir().join(format!("ks-prefix-{}-{:?}.json", std::process::id(), std::thread::current().id()));
        std::fs::write(&path, value.to_string()).unwrap();
        let result = VersionedPrefixModel::load(&path);
        std::fs::remove_file(&path).unwrap();
        result
    }

    #[test]
    fn the_shipped_model_loads_and_every_changed_field_is_refused() {
        assert!(loaded(&real()).is_ok());
        let models = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/models");
        assert!(VersionedPrefixModel::default_model(&models).is_some());
        assert!(VersionedPrefixModel::default_model(Path::new("/nonexistent")).is_none());
        let changes: [(&str, Value); 8] = [
            ("kind", json!("other")),
            ("feature_version", json!(99)),
            ("prefix_feature_version", json!(99)),
            ("version", json!("other")),
            ("conversion_threshold", json!(0.0)),
            ("weights_sha256", json!("0")),
            ("actions", json!(["a"])),
            ("weights", json!({})),
        ];
        for (name, value) in changes {
            let mut changed = real();
            changed[name] = value;
            assert!(loaded(&changed).is_err(), "{name}");
        }
        let mut no_version = real();
        no_version.as_object_mut().unwrap().remove("prefix_feature_version");
        let _ = loaded(&no_version);
        let mut no_kind = real();
        no_kind.as_object_mut().unwrap().remove("kind");
        assert!(loaded(&no_kind).is_err());
        assert!(loaded(&json!([1])).is_err());
    }
}
