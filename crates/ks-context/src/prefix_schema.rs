//! Prefix feature schemas beyond the frozen first one, and the loader that knows them.
//!
//! The installed prefix-v1 model, its frozen corpus and the lexical-compatibility gate pin `prefix_model` byte for byte,
//! so schema two (characters of the observed prefix) and the schema-aware artifact loader live here instead. The engine
//! loads the installed artifact through [`VersionedPrefixModel::default_model`], which reads either schema, so an accepted
//! schema-two pair can be installed without touching the frozen module.

use ks_cast::To as _;
use crate::constants::{
    CURRENT_PREFIX_FEATURE_VERSION, MAX_PREFIX_CHARACTER_NGRAM_ORDER, MAX_PREFIX_MODEL_BYTES, ORTHO_PREFIX_FEATURE_VERSION, PREFIX_CHARACTER_FEATURE_WEIGHT_CAP,
    MIN_PREFIX_CONVERSION_THRESHOLD, PREFIX_HEAD_MAX_LENGTH_BUCKET, PREFIX_HEAD_MIN_CHARACTERS, PREFIX_MAX_CHARACTERS, PREFIX_ORTHO_ABSOLUTE_MAX_STEPS,
    PREFIX_ORTHO_ABSOLUTE_STEP_NATS, PREFIX_ORTHO_BOTH_MAX_STEPS, PREFIX_ORTHO_BOTH_STEP_NATS, PREFIX_ORTHO_LENGTH_BUCKETS, PREFIX_ORTHO_MAX_STEPS,
    PREFIX_TAIL_MAX_LENGTH_BUCKET, PREFIX_TERM_COUNT_LOG_DIVISOR, TERMS_PREFIX_FEATURE_VERSION, PREFIX_ORTHO_MEAN_BOUND_NATS, PREFIX_ORTHO_STEP_NATS, PREFIX_ORTHO_SUM_BOUND_NATS,
    PREFIX_ORTHO_WORST_BOUND_NATS, SUPPLEMENTED_PREFIX_FEATURE_VERSION,
};
use ks_lang::early_switch::PrefixLexicon;
use crate::context_model::{first_greatest, ContextAction, ContextPrediction, ACTIONS};
use crate::flat_table::load_artifact;
use crate::prefix_model::{actions_match, features, finish_load, weight_spec, PrefixInput, PrefixModel, PrefixSide, ARTIFACT_FILE, PREFIX_FEATURE_VERSION};
use crate::prefix_terms::{PrefixTerms, TermEvidence};
use ks_lang::LoadError;
use ks_lang::ortho_model::{BOS, Channel, OrthoModel};
use ks_text::pystr;
use serde_json::Value;
use std::path::Path;

/// The words the prefix index of a schema holds (`VersionedPrefixModel::prefix_lexicon`), for whoever builds frames of that
/// schema before any model of it exists: the trainer.
#[must_use]
pub fn prefix_lexicon_of(feature_version: i64) -> PrefixLexicon {
    if feature_version >= SUPPLEMENTED_PREFIX_FEATURE_VERSION.to::<i64>() { PrefixLexicon::Held } else { PrefixLexicon::Plain }
}

/// Schema one stays frozen; schema two learns characters of observed prefixes.
///
/// The start marker describes an observed boundary. There is no end marker: an incremental prefix is not evidence that
/// the word has finished.
pub fn features_for_version(item: &PrefixInput, sides: [&PrefixSide<'_>; 2], feature_version: i64) -> Result<indexmap::IndexMap<String, f64>, LoadError> {
    features_with_ortho(item, sides, feature_version, None)
}

/// What a schema beyond the second reads besides the lexicons: the orthotactic models (schema three) and the term lists
/// (schema four).
#[derive(Clone, Copy, Default)]
pub struct PrefixReaders<'a> {
    pub ortho: Option<&'a OrthoModel>,
    pub terms: Option<&'a PrefixTerms>,
    /// The lists the reading as typed is looked up in, when not `terms`: a trainer's frames of a word typed as meant
    /// whose lists are closed to that reading alone. The program never sets it.
    pub typed_terms: Option<&'a PrefixTerms>,
}

/// The features of any schema; schema three reads the keys with the orthotactic models and refuses without them.
pub fn features_with_ortho(item: &PrefixInput, sides: [&PrefixSide<'_>; 2], feature_version: i64, ortho: Option<&OrthoModel>) -> Result<indexmap::IndexMap<String, f64>, LoadError> {
    features_with_readers(item, sides, feature_version, PrefixReaders { ortho, terms: None, typed_terms: None })
}

/// The features of any schema, with what schemas three and four read; each refuses without its readers.
pub fn features_with_readers(item: &PrefixInput, sides: [&PrefixSide<'_>; 2], feature_version: i64, readers: PrefixReaders<'_>) -> Result<indexmap::IndexMap<String, f64>, LoadError> {
    if feature_version == ORTHO_PREFIX_FEATURE_VERSION.to::<i64>() || feature_version == TERMS_PREFIX_FEATURE_VERSION.to::<i64>() {
        let ortho = readers.ortho.ok_or_else(|| LoadError::value("prefix schemas three and four need the orthotactic model"))?;
        if feature_version == ORTHO_PREFIX_FEATURE_VERSION.to::<i64>() {
            return Ok(schema_three(item, sides, ortho));
        }
        let terms = readers.terms.ok_or_else(|| LoadError::value("prefix schema four needs the term lists"))?;
        return Ok(schema_four(item, sides, ortho, [readers.typed_terms.unwrap_or(terms), terms]));
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

/// The step of a mean log-probability per key, counted down from zero and bounded.
fn absolute_step(mean: f64, step: f64, maximum: i64) -> i64 {
    (-mean / step).floor().clamp(0.0, maximum.to::<f64>()).to::<i64>()
}

/// What the term lists say of a reading, as features under a name: the term it is and the terms it begins, each list
/// apart, a count as its logarithm.
fn term_features(result: &mut indexmap::IndexMap<String, f64>, name: &str, kind: &str, evidence: Option<TermEvidence>) {
    let Some(evidence) = evidence else {
        return;
    };
    result.insert(format!("term:{name}:{kind}"), 1.0);
    if evidence.count > 0 {
        result.insert(format!("term:{name}:{kind}:count"), (evidence.count.to::<f64>().ln_1p() / PREFIX_TERM_COUNT_LOG_DIVISOR).min(1.0));
    }
    if evidence.identifier {
        result.insert(format!("term:{name}:{kind}:identifier"), 1.0);
    }
    if evidence.protected {
        result.insert(format!("term:{name}:{kind}:protected"), 1.0);
    }
}

/// Schema four: schema three, and three readings it lacks. The term lists the program loads, for each reading of the
/// keys, with how often Russian technical text writes a term: `cntk` is a term of the table where `стел…` is a word.
/// Each reading's own score by its own language's channel, apart and together: a compound token or a random
/// abbreviation (`vgcfgb`, `мпсапи`) reads badly both ways, where a word typed in the wrong layout reads well the other
/// way. And the longest whole word or term the typed keys start with, with what follows it: `verify|csrf`, `git|rfgrep`.
/// `terms` are the lists of the reading as typed (its head and tail as well) and of the other reading.
fn schema_four(item: &PrefixInput, sides: [&PrefixSide<'_>; 2], ortho: &OrthoModel, terms: [&PrefixTerms; 2]) -> indexmap::IndexMap<String, f64> {
    let mut result = schema_three(item, sides, ortho);
    let other_group = 1 - item.source_group;
    let [typed_terms, other_terms] = terms;
    for (name, text, group, lists) in [("typed", &item.original, item.source_group, typed_terms), ("other", &item.alternative, other_group, other_terms)] {
        let found = lists.lookup(text, group);
        term_features(&mut result, name, "exact", found.exact);
        term_features(&mut result, name, "begins", found.begins);
    }
    let keys = pystr::casefold(&ks_text::normalize::nfc(if item.source_group == 0 { &item.original } else { &item.alternative }));
    let (typed_channel, other_channel) = if item.source_group == 0 { (&ortho.channels.en, &ortho.channels.ru) } else { (&ortho.channels.ru, &ortho.channels.en) };
    let (typed, other) = (prefix_conditionals(typed_channel, &keys), prefix_conditionals(other_channel, &keys));
    if !typed.is_empty() {
        let count = typed.len();
        let bucket = PREFIX_ORTHO_LENGTH_BUCKETS.iter().rev().find(|bound| count >= **bound).copied().unwrap_or(PREFIX_ORTHO_LENGTH_BUCKETS[0]);
        let mean = |values: &[f64]| ks_lang::pysum::py_sum(values.iter().copied()) / count.to::<f64>();
        let (typed_mean, other_mean) = (mean(&typed), mean(&other));
        result.insert("ortho:typed_mean".to_owned(), bounded(typed_mean, PREFIX_ORTHO_MEAN_BOUND_NATS));
        result.insert("ortho:other_mean".to_owned(), bounded(other_mean, PREFIX_ORTHO_MEAN_BOUND_NATS));
        result.insert(format!("ortho:typed_step:{bucket}:{}", absolute_step(typed_mean, PREFIX_ORTHO_ABSOLUTE_STEP_NATS, PREFIX_ORTHO_ABSOLUTE_MAX_STEPS)), 1.0);
        result.insert(format!("ortho:other_step:{bucket}:{}", absolute_step(other_mean, PREFIX_ORTHO_ABSOLUTE_STEP_NATS, PREFIX_ORTHO_ABSOLUTE_MAX_STEPS)), 1.0);
        let both = |mean: f64| absolute_step(mean, PREFIX_ORTHO_BOTH_STEP_NATS, PREFIX_ORTHO_BOTH_MAX_STEPS);
        result.insert(format!("ortho:both:{}:{}", both(typed_mean), both(other_mean)), 1.0);
    }
    // The longest whole word of the typed layout's lexicon, or term, the typed keys begin with, shorter than the keys.
    let typed_text = pystr::casefold(&item.original);
    let letters: Vec<char> = typed_text.chars().collect();
    let side = sides[usize::from(item.source_group == 1)];
    let head = (PREFIX_HEAD_MIN_CHARACTERS..letters.len()).rev().find_map(|length| {
        let head: String = letters[..length].iter().collect();
        if side.model.frequencies.contains(&head) {
            Some(("word", length))
        } else {
            typed_terms.lookup(&head, item.source_group).exact.map(|_| ("term", length))
        }
    });
    if let Some((kind, length)) = head {
        let tail: String = letters[length..].iter().collect();
        result.insert(format!("head:{kind}:{}", length.min(PREFIX_HEAD_MAX_LENGTH_BUCKET)), 1.0);
        result.insert(format!("head:tail:{}", (letters.len() - length).min(PREFIX_TAIL_MAX_LENGTH_BUCKET)), 1.0);
        if side.index.completions(&tail).completions > 0 {
            result.insert("head:tail_continues".to_owned(), 1.0);
        }
        let found = typed_terms.lookup(&tail, item.source_group);
        if found.exact.is_some() || found.begins.is_some() {
            result.insert("head:tail_term".to_owned(), 1.0);
        }
    }
    result
}

/// A prefix model that carries the feature schema its weights were fitted on.
pub struct VersionedPrefixModel {
    pub model: PrefixModel,
    pub feature_version: i64,
    /// The threshold of each direction of typing, by the layout group of the keys (`conversion_thresholds`), when the
    /// artifact has them; the scalar `conversion_threshold` is then the larger of the two.
    pub thresholds: Option<[f64; 2]>,
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
        self.predict_with_readers(item, sides, PrefixReaders { ortho, terms: None, typed_terms: None })
    }

    /// The prediction with what schemas three and four read. A model with a threshold for each direction converts at the
    /// threshold of the direction the keys were typed in.
    pub fn predict_with_readers(&self, item: &PrefixInput, sides: [&PrefixSide<'_>; 2], readers: PrefixReaders<'_>) -> Result<ContextPrediction, LoadError> {
        let mut prediction = self.model.predict_features(&features_with_readers(item, sides, self.feature_version, readers)?);
        if self.thresholds.is_some() && ACTIONS[first_greatest(&prediction.probabilities)] == ContextAction::Convert {
            prediction.action = if prediction.probability >= self.threshold_for(item.source_group) { ContextAction::Convert } else { ContextAction::Wait };
        }
        Ok(prediction)
    }

    /// The thresholds of the two directions; a model with one threshold has it in both.
    #[must_use]
    pub fn thresholds(&self) -> [f64; 2] {
        self.thresholds.unwrap_or([self.model.conversion_threshold; 2])
    }

    /// The threshold a prefix typed in this layout group converts at.
    #[must_use]
    pub fn threshold_for(&self, source_group: i64) -> f64 {
        self.thresholds()[usize::from(source_group == 1)]
    }

    /// Whether the schema reads the keys with the orthotactic models, so the model answers only beside them.
    #[must_use]
    pub fn needs_ortho(&self) -> bool {
        self.feature_version == ORTHO_PREFIX_FEATURE_VERSION.to::<i64>() || self.needs_terms()
    }

    /// Whether the schema reads the term lists, so the model answers only beside them.
    #[must_use]
    pub fn needs_terms(&self) -> bool {
        self.feature_version == TERMS_PREFIX_FEATURE_VERSION.to::<i64>()
    }

    /// The words its prefix index holds (`Resources::prefix_index`). A model of schema four is trained, measured and run
    /// with every word of the language model, the supplement's among them. An older one runs with the index the program
    /// has always given it, without the supplement: prefix_v2 was trained with the supplement, but users have run it
    /// without since 0.37.0, and it stays as they know it.
    #[must_use]
    pub fn prefix_lexicon(&self) -> PrefixLexicon {
        prefix_lexicon_of(self.feature_version)
    }

    pub fn load(path: &Path) -> Result<VersionedPrefixModel, LoadError> {
        let (artifact, _) = load_artifact(path, MAX_PREFIX_MODEL_BYTES, "oversized prefix model", &weight_spec())?;
        let unsupported = || LoadError::value("unsupported prefix model");
        let artifact = artifact.ok_or_else(unsupported)?;
        let payload = &artifact.payload;
        let feature_version = payload
            .get("feature_version")
            .and_then(Value::as_i64)
            .filter(|&found| [PREFIX_FEATURE_VERSION, CURRENT_PREFIX_FEATURE_VERSION, ORTHO_PREFIX_FEATURE_VERSION, TERMS_PREFIX_FEATURE_VERSION].iter().any(|known| found == known.to::<i64>()));
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
        let thresholds = direction_thresholds(payload.get("conversion_thresholds"))?;
        let loaded = finish_load(artifact, feature_version, &format!("prefix-v{feature_version}-"))?;
        if thresholds.is_some_and(|both| both[0].max(both[1]).total_cmp(&loaded.model.conversion_threshold).is_ne()) {
            return Err(LoadError::value("inconsistent prefix thresholds"));
        }
        Ok(VersionedPrefixModel { model: loaded.model, feature_version: loaded.feature_version, thresholds })
    }

    /// `VersionedPrefixModel.default()`: the installed prefix artifact in whichever schema it was accepted, or none.
    #[must_use]
    pub fn default_model(models_dir: &Path) -> Option<VersionedPrefixModel> {
        VersionedPrefixModel::load(&models_dir.join(ARTIFACT_FILE)).ok()
    }
}

/// `conversion_thresholds`, when the artifact has them: two numbers, each a threshold the scalar one may be.
fn direction_thresholds<'a, D: serde::Deserializer<'a>>(found: Option<D>) -> Result<Option<[f64; 2]>, LoadError> {
    let Some(found) = found else {
        return Ok(None);
    };
    match <[f64; 2] as serde::Deserialize>::deserialize(found) {
        Ok(both) if both.iter().all(|value| (MIN_PREFIX_CONVERSION_THRESHOLD..=1.0).contains(value)) => Ok(Some(both)),
        _ => Err(LoadError::value("unsafe prefix threshold")),
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
