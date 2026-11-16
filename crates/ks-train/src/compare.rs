//! Artifacts scored on a corpus's calibration and development frames, as the trainer does (`compare_context_candidates`):
//! the measurement of a candidate against the installed pair before its sealed test is read, with every frame a model
//! converts against its label spelled back (`false_frames`) and every frame it decides otherwise than the first model
//! (`changed_frames`), so that a class of false conversions and every regression show on data a fit may read. The test
//! split is never read.
use crate::corpus::{CALIBRATION, DEVELOPMENT};
use crate::fit::{Columns, FeatureSet, FitInputs, Frame, Stages, build_features_of_splits};
use crate::ACTIONS;
use ks_context::constants::{ABBREVIATION_FEATURE_PREFIX, LATIN_ABBREVIATION_FEATURE_PREFIX, LETTER_FEATURE_PREFIX, ALONE_FEATURE_PREFIX, START_FEATURE_PREFIX, CAPITALS_FEATURE_PREFIX, KEPT_FEATURE_PREFIX, UNKNOWN_FEATURE_PREFIX, JOINED_FEATURE_PREFIX, FIRST_FEATURE_PREFIX, TERM_FEATURE_PREFIX, BRIEF_FEATURE_PREFIX, UPPER_FEATURE_PREFIX, COMMON_FEATURE_PREFIX};
use ks_context::context_action_features::FeatureMap;
use ks_context::context_model::{ACTIONS as ACTION_NAMES, ContextModel, softmax};
use serde_json::{Map, Value, json};
use ks_text::pystr;
use std::collections::{BTreeMap, HashMap};

/// The classes a frame is counted in, by the first head whose prefix its features hold: a token in capitals is counted
/// with the upper-case head whatever other head answers it too.
const HEAD_CLASSES: [(&str, &str); 14] = [
    ("upper", UPPER_FEATURE_PREFIX),
    ("common", COMMON_FEATURE_PREFIX),
    ("abbreviation", ABBREVIATION_FEATURE_PREFIX), ("latin_abbreviation", LATIN_ABBREVIATION_FEATURE_PREFIX), ("letter", LETTER_FEATURE_PREFIX), ("alone", ALONE_FEATURE_PREFIX),
    ("start", START_FEATURE_PREFIX), ("capitals", CAPITALS_FEATURE_PREFIX), ("kept", KEPT_FEATURE_PREFIX), ("unknown", UNKNOWN_FEATURE_PREFIX), ("joined", JOINED_FEATURE_PREFIX),
    ("first", FIRST_FEATURE_PREFIX), ("term", TERM_FEATURE_PREFIX), ("brief", BRIEF_FEATURE_PREFIX),
];
const BASE_CLASS: &str = "base";
const ALL_FRAMES: &str = "all";
const CONVERT: usize = 1;
/// The text around a falsely converted frame that its listing keeps: the end of the text before it and the start of the
/// text after it.
const FALSE_FRAME_BEFORE_CHARACTERS: i64 = 60;
const FALSE_FRAME_AFTER_CHARACTERS: i64 = 30;

/// A frame as its listing spells it back.
#[derive(Clone)]
struct FrameText {
    original: String,
    alternative: String,
    before: String,
    after: String,
    trigger: String,
    origin: &'static str,
}

/// The frames of each profile and split as the build hands them on, in the order their features are scored.
#[derive(Default)]
struct FrameTexts(HashMap<(String, String), Vec<FrameText>>);

impl Stages for FrameTexts {
    fn frames(&mut self, name: &str, frames: &[Frame]) {
        let Some((profile, split)) = name.strip_prefix("entries-").and_then(|rest| rest.rsplit_once('-')) else { return };
        let texts = frames
            .iter()
            .map(|frame| match frame {
                Frame::Row(row) => FrameText {
                    original: row.original.clone(),
                    alternative: crate::physical::translated(&row.original, row.group).unwrap_or_default(),
                    before: row.field.before.clone(),
                    after: row.field.after.clone(),
                    trigger: row.trigger.to_owned(),
                    origin: row.after_origin.as_str(),
                },
                Frame::Span(span) => FrameText {
                    original: span.evidence.original.clone(),
                    alternative: span.evidence.alternative.clone(),
                    before: span.evidence.field.before.clone(),
                    after: span.evidence.field.after.clone(),
                    trigger: span.evidence.trigger.clone(),
                    origin: span.evidence.after_origin.as_str(),
                },
            })
            .collect();
        self.0.insert((profile.to_owned(), split.to_owned()), texts);
    }
}

/// `head_class`: the head whose question a frame asks, by the prefixes of its feature names.
fn head_class(features: &FeatureMap) -> &'static str {
    for (name, prefix) in HEAD_CLASSES {
        if features.keys().any(|feature| feature.starts_with(prefix)) {
            return name;
        }
    }
    BASE_CLASS
}

/// What the runtime converts on these features: convert selected, at the serving threshold, after the support check; and
/// the probability of the conversion.
fn converts(model: &ContextModel, features: &FeatureMap) -> (bool, f64) {
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
        return (false, probabilities[CONVERT]);
    }
    (selected == CONVERT && probabilities[CONVERT] >= model.conversion_threshold, probabilities[CONVERT])
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

/// A frame spelled back, with its class, its label and the probability of the conversion.
#[derive(serde::Serialize)]
struct FalseFrame {
    class: &'static str,
    label: &'static str,
    original: String,
    alternative: String,
    before: String,
    after: String,
    trigger: String,
    after_origin: &'static str,
    p: f64,
}

impl FalseFrame {
    fn new(text: &FrameText, class: &'static str, label: u8, probability: f64) -> Self {
        Self {
            class,
            label: ACTION_NAMES[usize::from(label)].as_str(),
            original: text.original.clone(),
            alternative: text.alternative.clone(),
            before: pystr::slice(&text.before, Some(-FALSE_FRAME_BEFORE_CHARACTERS), None).to_owned(),
            after: pystr::slice(&text.after, None, Some(FALSE_FRAME_AFTER_CHARACTERS)).to_owned(),
            trigger: text.trigger.clone(),
            after_origin: text.origin,
            p: probability,
        }
    }
}

/// A frame a model decides otherwise than the first model: which way (`gained_right`, `lost_false`…) and the first
/// model's probability of the conversion.
#[derive(serde::Serialize)]
struct ChangedFrame {
    #[serde(flatten)]
    frame: FalseFrame,
    kind: String,
    p_first: f64,
}

fn score_split(models: &[(String, ContextModel)], part: &Columns, names: &[String], texts: &[FrameText]) -> Value {
    let rows: Vec<(FeatureMap, u8)> = part.feature_rows(names).map(|(features, label, _)| (features, label)).collect();
    let classes: Vec<&'static str> = rows.iter().map(|(features, _)| head_class(features)).collect();
    let decisions: Vec<Vec<(bool, f64)>> = models.iter().map(|(_, model)| rows.iter().map(|(features, _)| converts(model, features)).collect()).collect();
    let mut report = Map::new();
    for (position, (name, model)) in models.iter().enumerate() {
        let mut tallies: BTreeMap<String, Tally> = BTreeMap::new();
        let mut differences: BTreeMap<String, BTreeMap<String, i64>> = BTreeMap::new();
        let mut falsely: Vec<FalseFrame> = Vec::new();
        let mut changed: Vec<ChangedFrame> = Vec::new();
        for (index, (_, label)) in rows.iter().enumerate() {
            let (conversion, probability) = decisions[position][index];
            let possible = usize::from(*label) == CONVERT;
            if conversion && !possible {
                falsely.push(FalseFrame::new(&texts[index], classes[index], *label, probability));
            }
            for key in [ALL_FRAMES, classes[index]] {
                let counts = tallies.entry(key.to_owned()).or_default();
                counts.rows += 1;
                counts.convert_rows += i64::from(possible);
                counts.right += i64::from(conversion && possible);
                counts.wrong += i64::from(conversion && !possible);
                if position > 0 && decisions[0][index].0 != conversion {
                    let kind = format!("{}{}", if conversion { "gained" } else { "lost" }, if possible { "_right" } else { "_false" });
                    if key == ALL_FRAMES {
                        changed.push(ChangedFrame { frame: FalseFrame::new(&texts[index], classes[index], *label, probability), kind: kind.clone(), p_first: decisions[0][index].1 });
                    }
                    *differences.entry(key.to_owned()).or_default().entry(kind).or_insert(0) += 1;
                }
            }
        }
        report.insert(name.clone(), json!({"model_version": model.version, "threshold": model.conversion_threshold, "by_class": tally_value(&tallies), "differences": differences, "examples": [], "false_frames": falsely, "changed_frames": changed}));
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
    let mut texts = FrameTexts::default();
    let features: FeatureSet = build_features_of_splits(inputs, jobs, &mut texts, &[CALIBRATION, DEVELOPMENT])?;
    let mut report = Map::new();
    for split in [CALIBRATION, DEVELOPMENT] {
        let mut by_profile = Map::new();
        for profile in &inputs.profiles {
            // The build hands on the frames of every profile and split it scores, one for each row of its features.
            let frames = &texts.0[&(profile.clone(), split.to_owned())];
            by_profile.insert(profile.clone(), score_split(&models, features.part_of(profile, split), features.feature_names(), frames));
        }
        report.insert(split.to_owned(), Value::Object(by_profile));
    }
    Ok(Value::Object(report))
}
