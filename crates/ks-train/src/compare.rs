//! Artifacts scored on a corpus's calibration and development frames, as the trainer does (`compare_context_candidates`):
//! the measurement of a candidate against the installed pair before its sealed test is read. The test split is never read.
use crate::corpus::{CALIBRATION, DEVELOPMENT};
use crate::fit::{Columns, FeatureSet, FitInputs, build_features_of_splits};
use crate::fit::NoStages;
use crate::{ACTIONS};
use ks_context::constants::{ABBREVIATION_FEATURE_PREFIX, LATIN_ABBREVIATION_FEATURE_PREFIX, LETTER_FEATURE_PREFIX, ALONE_FEATURE_PREFIX, START_FEATURE_PREFIX, CAPITALS_FEATURE_PREFIX, KEPT_FEATURE_PREFIX, UNKNOWN_FEATURE_PREFIX, JOINED_FEATURE_PREFIX, FIRST_FEATURE_PREFIX, TERM_FEATURE_PREFIX, BRIEF_FEATURE_PREFIX};
use ks_context::context_action_features::FeatureMap;
use ks_context::context_model::{ContextModel, softmax};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

const HEAD_CLASSES: [(&str, &str); 12] = [
    ("abbreviation", ABBREVIATION_FEATURE_PREFIX), ("latin_abbreviation", LATIN_ABBREVIATION_FEATURE_PREFIX), ("letter", LETTER_FEATURE_PREFIX), ("alone", ALONE_FEATURE_PREFIX),
    ("start", START_FEATURE_PREFIX), ("capitals", CAPITALS_FEATURE_PREFIX), ("kept", KEPT_FEATURE_PREFIX), ("unknown", UNKNOWN_FEATURE_PREFIX), ("joined", JOINED_FEATURE_PREFIX),
    ("first", FIRST_FEATURE_PREFIX), ("term", TERM_FEATURE_PREFIX), ("brief", BRIEF_FEATURE_PREFIX),
];
const BASE_CLASS: &str = "base";
const ALL_FRAMES: &str = "all";
const CONVERT: usize = 1;

/// `head_class`: the head whose question a frame asks, by the prefixes of its feature names.
fn head_class(features: &FeatureMap) -> &'static str {
    for (name, prefix) in HEAD_CLASSES {
        if features.keys().any(|feature| feature.starts_with(prefix)) {
            return name;
        }
    }
    BASE_CLASS
}

/// What the runtime converts on these features: convert selected, at the serving threshold, after the support check.
fn converts(model: &ContextModel, features: &FeatureMap) -> bool {
    let mut scores = [0.0_f64; ACTIONS];
    for (name, value) in features {
        if let Some(weights) = model.weights.get(name) {
            for (index, weight) in weights.iter().enumerate() {
                scores[index] += weight * value;
            }
        }
    }
    let probabilities = softmax(&scores);
    let mut selected = 0;
    for index in 1..ACTIONS {
        if probabilities[index] > probabilities[selected] {
            selected = index;
        }
    }
    if selected < 2 && !model.supports_features(features) {
        return false;
    }
    selected == CONVERT && probabilities[CONVERT] >= model.conversion_threshold
}

#[derive(Default, Clone, Copy)]
struct Tally {
    rows: i64,
    convert_rows: i64,
    right: i64,
    wrong: i64,
}

fn tally_value(table: &BTreeMap<String, Tally>) -> Value {
    let mut report = Map::new();
    for (name, counts) in table {
        report.insert(name.clone(), json!({"rows": counts.rows, "convert_rows": counts.convert_rows, "right": counts.right, "false": counts.wrong, "net": counts.right - counts.wrong}));
    }
    Value::Object(report)
}

fn score_split(models: &[(String, ContextModel)], part: &Columns, names: &[String]) -> Value {
    let rows: Vec<(FeatureMap, u8)> = part.feature_rows(names).map(|(features, label, _)| (features, label)).collect();
    let classes: Vec<&str> = rows.iter().map(|(features, _)| head_class(features)).collect();
    let decisions: Vec<Vec<bool>> = models.iter().map(|(_, model)| rows.iter().map(|(features, _)| converts(model, features)).collect()).collect();
    let mut report = Map::new();
    for (position, (name, model)) in models.iter().enumerate() {
        let mut tallies: BTreeMap<String, Tally> = BTreeMap::new();
        let mut differences: BTreeMap<String, BTreeMap<String, i64>> = BTreeMap::new();
        for (index, (_, label)) in rows.iter().enumerate() {
            let conversion = decisions[position][index];
            let possible = usize::from(*label) == CONVERT;
            for key in [ALL_FRAMES, classes[index]] {
                let counts = tallies.entry(key.to_owned()).or_default();
                counts.rows += 1;
                counts.convert_rows += i64::from(possible);
                counts.right += i64::from(conversion && possible);
                counts.wrong += i64::from(conversion && !possible);
                if position > 0 && decisions[0][index] != conversion {
                    let kind = format!("{}{}", if conversion { "gained" } else { "lost" }, if possible { "_right" } else { "_false" });
                    *differences.entry(key.to_owned()).or_default().entry(kind).or_insert(0) += 1;
                }
            }
        }
        report.insert(name.clone(), json!({"model_version": model.version, "threshold": model.conversion_threshold, "by_class": tally_value(&tallies), "differences": differences, "examples": []}));
    }
    Value::Object(report)
}

/// `compare`: the corpus loaded as the trainer does, its frames built once and every artifact scored on them.
pub fn compare(inputs: &FitInputs, artifacts: &[(String, std::path::PathBuf)], jobs: usize) -> Result<Value, String> {
    let models_dir = inputs.tools.resources.resource_dir().join("models");
    let mut models = Vec::new();
    for (name, path) in artifacts {
        models.push((name.clone(), ContextModel::load_artifact(path, &models_dir).map_err(|error| format!("{}: {}", path.display(), error.name))?));
    }
    let features: FeatureSet = build_features_of_splits(inputs, jobs, &mut NoStages, &[CALIBRATION, DEVELOPMENT])?;
    let mut report = Map::new();
    for split in [CALIBRATION, DEVELOPMENT] {
        let mut by_profile = Map::new();
        for profile in &inputs.profiles {
            by_profile.insert(profile.clone(), score_split(&models, features.part_of(profile, split), features.feature_names()));
        }
        report.insert(split.to_owned(), Value::Object(by_profile));
    }
    Ok(Value::Object(report))
}
