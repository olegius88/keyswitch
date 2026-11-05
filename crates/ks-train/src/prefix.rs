//! The prefix candidate (`train_prefix_v2_model`, `prefix_v2_corpus`): a bounded counterfactual curriculum from already
//! exposed old splits. Application and role variants receive both intended-layout states; the candidate is chosen on
//! whole sequences, with no false conversion in either lexical profile.
use crate::corpus::{CALIBRATION, DEVELOPMENT, FITTING_SPLITS, TRAIN, checksum, read_json};
use crate::fit::{provenance, readable};
use crate::kernel::{self, Packed};
use crate::physical::{hex, pair};
use crate::tools::{Lexicons, Options, reference_models};
use crate::{ACTIONS, DETERMINISTIC_ROUNDING_DECIMALS, FEATURE_MASS_TOLERANCE, LOG_LOSS_PROBABILITY_FLOOR, PROBABILITY_SUM_TOLERANCE};
use flate2::read::MultiGzDecoder;
use ks_backend::{FieldContext, FieldRole};
use ks_context::prefix_model::{PrefixInput, PrefixModel, PrefixSide};
use ks_context::prefix_schema::features_for_version;
use ks_context::flat_table::WeightTable;
use ks_lang::early_switch::{PrefixIndex, _dictionary_stems};
use ks_lang::{Resources, LanguageModel};
use ks_text::{JsonOptions, fsum, json_dumps, py_round_ndigits, pystr};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const SELECTION_NAMESPACE: &str = "keyswitch:prefix-v2:exposed-parent-selection:1";
const SAFETY_CATEGORIES: [&str; 3] = ["identifier", "command", "mixed_english"];
const PREFIX_V2_AMBIGUOUS_POSITIVE_LABEL: u8 = 2;
const PREFIX_V2_LEGACY_DELETION_INDEX: usize = 3;
const PREFIX_V2_SHORT_PREFIX_CHARACTERS: usize = 4;
use ks_lang::constants::PREFIX_INDEX_DICTIONARY_MAX_BYTES;
const PREFIX_MIN_CHARACTERS: usize = 4;
const PREFIX_MAX_CHARACTERS: usize = 12;
const VERSION_HASH_CHARACTERS: usize = 12;
const PROFILES: [&str; 2] = ["portable", "reference_hunspell"];
const PREFIX_MIN_EARLY_RECALL_FLOOR: f64 = 0.70;
const WAIT_LABEL: u8 = 2;
const CONVERT_LABEL: usize = 1;

#[derive(Clone, Debug)]
pub struct ParentWord {
    pub original: String,
    pub group: i64,
    pub family: String,
    pub before: String,
    pub identifier: String,
    pub category: String,
    pub application: String,
    pub role: FieldRole,
}

struct PrefixContext {
    identifier: &'static str,
    before: String,
    application: String,
    role: FieldRole,
}

/// `json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":"))`.
fn compact(value: &Value) -> String {
    json_dumps(value, &JsonOptions { ensure_ascii: false, sort_keys: true, separators: Some((",".to_owned(), ":".to_owned())), ..JsonOptions::default() })
}

fn digest(value: &Value) -> String {
    hex(&Sha256::digest(compact(value).as_bytes()))
}

pub(crate) fn role_named(name: &str) -> Result<FieldRole, String> {
    match name {
        "unknown" => Ok(FieldRole::Unknown),
        "text" => Ok(FieldRole::Text),
        "code" => Ok(FieldRole::Code),
        "search" => Ok(FieldRole::Search),
        "terminal" => Ok(FieldRole::Terminal),
        _ => Err("invalid old prefix application metadata".to_owned()),
    }
}

/// `load_parents`: hash-selected natural parents and matching existing typo forms, never test.
pub fn load_parents(directory: &Path, split: &str, max_families: i64, max_words_per_family: i64) -> Result<Vec<ParentWord>, String> {
    if !FITTING_SPLITS.contains(&split) {
        return Err("prefix-v2 may read only exposed train/development/calibration splits".to_owned());
    }
    let path = directory.join(format!("{split}.jsonl.gz"));
    let receipt = read_json(&directory.join("corpus.json"))?;
    if Value::String(checksum(&path)?) != receipt["sha256"][split] {
        return Err("old prefix split checksum mismatch".to_owned());
    }
    let mut natural: BTreeMap<String, Vec<ParentWord>> = BTreeMap::new();
    let mut typos: HashMap<(String, i64, String, String), ParentWord> = HashMap::new();
    let mut safety: HashMap<(String, String, String, String, String, &'static str), ParentWord> = HashMap::new();
    let file = std::fs::File::open(&path).map_err(|error| error.to_string())?;
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for line in std::io::BufReader::new(MultiGzDecoder::new(file)).lines() {
        let line = line.map_err(|error| error.to_string())?;
        let row: Value = serde_json::from_str(&line).map_err(|error| error.to_string())?;
        let category = row["category"].as_str().unwrap_or("");
        if row["length"].as_i64() != Some(1) || row["profile"].as_str() != Some("portable") || !(category == "correct" || category == "typo" || SAFETY_CATEGORIES.contains(&category)) {
            continue;
        }
        let sequence = row["sequence"].as_i64().ok_or("invalid old prefix parent metadata")?;
        let parent = ParentWord {
            original: row["text"].as_str().ok_or("invalid old prefix parent metadata")?.to_owned(),
            group: row["source"].as_i64().ok_or("invalid old prefix parent metadata")?,
            family: row["family"].as_str().ok_or("invalid old prefix parent metadata")?.to_owned(),
            before: row["before"].as_str().ok_or("invalid old prefix parent metadata")?.to_owned(),
            identifier: format!("prefix-v1:{split}:{sequence}"),
            category: category.to_owned(),
            application: row.get("application").and_then(Value::as_str).unwrap_or("").to_owned(),
            role: role_named(row.get("role").and_then(Value::as_str).unwrap_or("unknown"))?,
        };
        if !seen.insert(parent.identifier.clone()) {
            return Err("duplicate old prefix parent identifier".to_owned());
        }
        if category == "correct" {
            natural.entry(parent.family.clone()).or_default().push(parent);
        } else if category == "typo" {
            let key = (parent.family.clone(), parent.group, parent.before.clone(), parent.original.clone());
            if typos.get(&key).is_none_or(|previous| parent.identifier < previous.identifier) {
                typos.insert(key, parent);
            }
        } else {
            let key = (parent.family.clone(), parent.category.clone(), parent.original.clone(), parent.before.clone(), parent.application.clone(), parent.role.as_str());
            if parent.group == 0 && safety.get(&key).is_none_or(|previous| parent.identifier < previous.identifier) {
                safety.insert(key, parent);
            }
        }
    }
    let mut families: Vec<&String> = natural.keys().collect();
    families.sort_by_cached_key(|name| (digest(&json!([SELECTION_NAMESPACE, name])), (*name).clone()));
    families.truncate(max_families as usize);
    let mut result = Vec::new();
    for family in families {
        let mut candidates: Vec<&ParentWord> = natural[family].iter().collect();
        candidates.sort_by_cached_key(|item| (digest(&json!([SELECTION_NAMESPACE, family, item.original, item.group])), item.identifier.clone()));
        let mut selected: BTreeSet<(String, i64)> = BTreeSet::new();
        for parent in candidates {
            if !selected.insert((parent.original.clone(), parent.group)) {
                continue;
            }
            result.push(parent.clone());
            // Match the legacy deletion surface; do not synthesize a new row.
            let letters: Vec<char> = parent.original.chars().collect();
            let deleted: String = letters.iter().take(PREFIX_V2_LEGACY_DELETION_INDEX).chain(letters.iter().skip(PREFIX_V2_LEGACY_DELETION_INDEX + 1)).collect();
            if let Some(typo) = typos.get(&(parent.family.clone(), parent.group, parent.before.clone(), deleted)) {
                let mut row = typo.clone();
                row.identifier = format!("{}:parent:{}", typo.identifier, parent.identifier);
                result.push(row);
            }
            let physical = if parent.group != 0 { pair().translate(&parent.original, "ru", "us").expect("the pair of layouts") } else { parent.original.clone() };
            let mut source_contexts = vec![
                ("identifier", format!("{physical}_value"), "const value = ", "Code", FieldRole::Code),
                ("command", physical.clone(), "$ ", "WindowsTerminal", FieldRole::Terminal),
            ];
            if parent.group == 0 {
                source_contexts.push(("mixed_english", parent.original.clone(), "в сообщении написано ", "Telegram", FieldRole::Text));
            }
            for (category, text, before, application, role) in source_contexts {
                let key = (parent.family.clone(), category.to_owned(), text, before.to_owned(), application.to_owned(), role.as_str());
                if let Some(existing) = safety.get(&key) {
                    let mut row = existing.clone();
                    row.identifier = format!("{}:parent:{}", existing.identifier, parent.identifier);
                    result.push(row);
                }
            }
            if selected.len() == max_words_per_family as usize {
                break;
            }
        }
    }
    Ok(result)
}

/// `contexts_for`, sorted and without repeats as `_contexts` leaves them.
fn contexts_of(parent: &ParentWord) -> Vec<PrefixContext> {
    let values: Vec<PrefixContext> = if SAFETY_CATEGORIES.contains(&parent.category.as_str()) {
        vec![PrefixContext { identifier: "existing", before: parent.before.clone(), application: parent.application.clone(), role: parent.role }]
    } else {
        let comment = if parent.group != 0 { "// пояснение " } else { "// explanation " };
        let shell = if parent.group != 0 { "# описание " } else { "# description " };
        let context = |identifier: &'static str, before: &str, application: &str, role: FieldRole| PrefixContext { identifier, before: before.to_owned(), application: application.to_owned(), role };
        vec![
            context("natural", &parent.before, "Telegram", FieldRole::Text),
            context("empty", "", "TestEditor", FieldRole::Unknown),
            context("comment-code-unknown", comment, "Code", FieldRole::Unknown),
            context("comment-code", comment, "Code", FieldRole::Code),
            context("shell-unknown", shell, "WindowsTerminal", FieldRole::Unknown),
            context("shell-terminal", shell, "WindowsTerminal", FieldRole::Terminal),
            context("comment-telegram", comment, "Telegram", FieldRole::Text),
            context("chat-code-unknown", &parent.before, "Code", FieldRole::Unknown),
            context("chat-code-text", &parent.before, "Code", FieldRole::Text),
        ]
    };
    let mut sorted = values;
    sorted.sort_by_key(|item| item.identifier);
    let mut unique: BTreeMap<(String, String, &'static str), PrefixContext> = BTreeMap::new();
    for item in sorted {
        unique.entry((item.before.clone(), item.application.clone(), item.role.as_str())).or_insert(item);
    }
    unique.into_values().collect()
}

pub struct Indexes {
    portable: Vec<PrefixIndex>,
    hunspell: Vec<PrefixIndex>,
}

/// `_indexes`: the supplied frozen models, with the Hunspell stems for the reference profile.
fn indexes_of(portable: &Lexicons, spelling: &Lexicons) -> Indexes {
    let plain = portable.iter().map(|model| PrefixIndex::new(model.frequencies.words().iter(), model.frequencies.clone())).collect();
    let stemmed = spelling
        .iter()
        .map(|model| {
            let stems = _dictionary_stems(Path::new(model.speller.source()), PREFIX_INDEX_DICTIONARY_MAX_BYTES);
            PrefixIndex::new(model.frequencies.words().iter().chain(stems.iter()), model.frequencies.clone())
        })
        .collect();
    Indexes { portable: plain, hunspell: stemmed }
}

/// A frame of the curriculum, with what the epochs and the metrics read.
pub struct PrefixFrame {
    pub label: u8,
    pub parent_family: u32,
    pub sequence: u32,
    pub length: u32,
    pub full_length: u32,
    pub desired: bool,
    pub profile: u8,
    pub sample_weight: f64,
    pub supported: bool,
    pub pair: u32,
    pub features: Vec<(u32, f64)>,
}

/// The frames of one split and the names their features use.
pub struct SplitFrames {
    pub frames: Vec<PrefixFrame>,
    pub names: Vec<String>,
    pub families: Vec<String>,
    pub pairs: usize,
}

struct ParentFrames {
    frames: Vec<PrefixFrame>,
    names: Vec<String>,
    pairs: Vec<String>,
    sequences: Vec<String>,
}

/// The frames of one parent. Sequences and pairs are numbered inside the parent and renumbered by the caller.
fn parent_frames(parent: &ParentWord, family_count: u64, family: u32, models: [&Lexicons; 2], indexes: &Indexes, maximum_prefix_length: usize, feature_version: i64) -> ParentFrames {
    let variants = contexts_of(parent);
    let categories_correct = parent.category == "correct";
    let states: &[bool] = if categories_correct { &[false, true] } else { &[false] };
    let parent_length = pystr::len(&parent.original);
    let length_count = parent_length.min(maximum_prefix_length);
    let weight = 1.0 / (family_count as f64 * variants.len() as f64 * PROFILES.len() as f64 * states.len() as f64 * length_count as f64);
    let mut names: HashMap<String, u32> = HashMap::new();
    let mut out = ParentFrames { frames: Vec::new(), names: Vec::new(), pairs: Vec::new(), sequences: Vec::new() };
    let (from, to) = if parent.group != 0 { ("ru", "us") } else { ("us", "ru") };
    for context in &variants {
        let context_key = json!([context.before, context.application, context.role.as_str()]);
        for (profile_number, profile) in PROFILES.iter().enumerate() {
            let pair_id = format!("prefix-v2-pair:{}", digest(&json!([parent.identifier, context_key, profile])));
            let pair_number = out.pairs.len() as u32;
            out.pairs.push(pair_id.clone());
            let table = if profile_number == 0 { &indexes.portable } else { &indexes.hunspell };
            let model_of = |group: usize| -> &LanguageModel { &models[profile_number][group] };
            let sides = [PrefixSide { index: &table[0], model: model_of(0) }, PrefixSide { index: &table[1], model: model_of(1) }];
            for desired in states {
                let source = if *desired { 1 - parent.group } else { parent.group };
                let full = if *desired { pair().translate(&parent.original, from, to).expect("the pair of layouts") } else { parent.original.clone() };
                assert_eq!(pystr::len(&full), parent_length, "prefix translation changed the parent length");
                let sequence_id = format!("prefix-v2-sequence:{}", digest(&json!([pair_id, desired])));
                let sequence_number = out.sequences.len() as u32;
                out.sequences.push(sequence_id);
                for length in 1..=length_count {
                    let original = pystr::slice(&full, None, Some(length as i64)).to_owned();
                    let (source_layout, target_layout) = if source != 0 { ("ru", "us") } else { ("us", "ru") };
                    let alternate = pair().translate(&original, source_layout, target_layout).expect("the pair of layouts");
                    let ambiguous = indexes.portable[source as usize].completions(&original).completions > 0;
                    let label = if *desired {
                        if length < PREFIX_V2_SHORT_PREFIX_CHARACTERS || ambiguous { PREFIX_V2_AMBIGUOUS_POSITIVE_LABEL } else { 1 }
                    } else {
                        0
                    };
                    let mut field = FieldContext::new(context.application.as_str(), pair_id.as_str());
                    field.before.clone_from(&context.before);
                    field.role = context.role;
                    let item = PrefixInput { original: original.clone(), alternative: alternate.clone(), source_group: source, field };
                    let values = features_for_version(&item, [&sides[0], &sides[1]], feature_version).expect("the schema of the recipe");
                    let features = values
                        .into_iter()
                        .map(|(name, value)| {
                            let next = out.names.len() as u32;
                            let id = *names.entry(name.clone()).or_insert_with(|| {
                                out.names.push(name);
                                next
                            });
                            (id, value)
                        })
                        .collect();
                    // The prefix decision requires four to twelve letters and no interior capitals.
                    let supported = (PREFIX_MIN_CHARACTERS..=PREFIX_MAX_CHARACTERS).contains(&length)
                        && pystr::len(&original) == length
                        && pystr::len(&alternate) == length
                        && pystr::isalpha(&original)
                        && pystr::isalpha(&alternate)
                        && (source == 0 || source == 1)
                        && !original.chars().skip(1).chain(alternate.chars().skip(1)).any(|character| pystr::isupper(character.encode_utf8(&mut [0_u8; 4])));
                    out.frames.push(PrefixFrame {
                        label,
                        parent_family: family,
                        sequence: sequence_number,
                        length: length as u32,
                        full_length: parent_length as u32,
                        desired: *desired,
                        profile: profile_number as u8,
                        sample_weight: weight,
                        supported,
                        pair: pair_number,
                        features,
                    });
                }
            }
        }
    }
    out
}

/// `generate_frames`: paired snapshots with unit total sampling mass per physical family, by `jobs` threads.
fn generate_frames(parents: &[ParentWord], models: [&Lexicons; 2], indexes: &Indexes, maximum_prefix_length: usize, feature_version: i64, jobs: usize) -> SplitFrames {
    let mut family_counts: BTreeMap<&str, u64> = BTreeMap::new();
    for parent in parents {
        *family_counts.entry(&parent.family).or_insert(0) += 1;
    }
    let families: Vec<String> = family_counts.keys().map(|name| (*name).to_owned()).collect();
    let family_number: HashMap<&str, u32> = families.iter().enumerate().map(|(index, name)| (name.as_str(), index as u32)).collect();
    let next = AtomicUsize::new(0);
    let done: std::sync::Mutex<Vec<Option<ParentFrames>>> = std::sync::Mutex::new((0..parents.len()).map(|_| None).collect());
    std::thread::scope(|scope| {
        for _ in 0..jobs.max(1) {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(parent) = parents.get(index) else { break };
                    let built = parent_frames(parent, family_counts[parent.family.as_str()], family_number[parent.family.as_str()], models, indexes, maximum_prefix_length, feature_version);
                    done.lock().expect("a worker finished")[index] = Some(built);
                }
            });
        }
    });
    let mut names: Vec<String> = Vec::new();
    let mut numbers: HashMap<String, u32> = HashMap::new();
    let mut frames = Vec::new();
    let (mut sequences, mut pairs) = (0_u32, 0_u32);
    for part in done.into_inner().expect("the workers finished").into_iter().flatten() {
        let mapping: Vec<u32> = part
            .names
            .iter()
            .map(|name| {
                *numbers.entry(name.clone()).or_insert_with(|| {
                    names.push(name.clone());
                    (names.len() - 1) as u32
                })
            })
            .collect();
        for mut frame in part.frames {
            frame.sequence += sequences;
            frame.pair += pairs;
            for feature in &mut frame.features {
                feature.0 = mapping[feature.0 as usize];
            }
            frames.push(frame);
        }
        sequences += part.sequences.len() as u32;
        pairs += part.pairs.len() as u32;
    }
    SplitFrames { frames, names, families, pairs: pairs as usize }
}

/// `feature_vocabulary`: Kahan feature mass; family weight precedes class multipliers.
fn feature_vocabulary(split: &SplitFrames, minimum: f64, maximum: usize) -> Vec<String> {
    let mut totals = vec![0.0_f64; split.names.len()];
    let mut errors = vec![0.0_f64; split.names.len()];
    let mut seen = vec![false; split.names.len()];
    for frame in &split.frames {
        for (id, value) in &frame.features {
            assert!(value.is_finite(), "invalid feature value");
            let id = *id as usize;
            let previous = totals[id];
            let corrected = frame.sample_weight - errors[id];
            let total = previous + corrected;
            errors[id] = (total - previous) - corrected;
            totals[id] = total;
            seen[id] = true;
        }
    }
    // Quantised ordering and a 1e-9 cutoff tolerate divided-parent roundoff.
    let mut ranked: Vec<(f64, &String)> = (0..split.names.len())
        .filter(|id| seen[*id] && totals[*id] + FEATURE_MASS_TOLERANCE >= minimum)
        .map(|id| (py_round_ndigits(totals[id], DETERMINISTIC_ROUNDING_DECIMALS).expect("a finite mass"), &split.names[id]))
        .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    ranked.truncate(maximum);
    let mut selected: Vec<String> = ranked.into_iter().map(|entry| entry.1.clone()).collect();
    selected.sort();
    selected
}

fn packed_rows(split: &SplitFrames, names: &[String], cfg: &Value) -> Packed {
    let vocabulary: HashMap<&str, u32> = names.iter().enumerate().map(|(index, name)| (name.as_str(), index as u32)).collect();
    let column: Vec<Option<u32>> = split.names.iter().map(|name| vocabulary.get(name.as_str()).copied()).collect();
    let (keep, wait) = (cfg.float("keep_importance"), cfg.float("wait_importance"));
    let mut packed = Packed { offsets: vec![0], ..Packed::default() };
    for frame in &split.frames {
        for (id, value) in &frame.features {
            if let Some(target) = column[*id as usize] {
                packed.indices.push(target);
                packed.values.push(*value);
            }
        }
        packed.offsets.push(packed.indices.len() as u64);
        packed.labels.push(frame.label);
        let factor = if frame.label == 0 {
            keep
        } else if frame.label == WAIT_LABEL {
            wait
        } else {
            1.0
        };
        packed.importance.push(frame.sample_weight * factor);
    }
    packed
}

#[derive(Clone, Default)]
struct Sequence {
    profile: u8,
    desired: bool,
    full_length: u32,
    candidates: Vec<(u32, f64)>,
}

fn best_action(scores: &[f64]) -> usize {
    let mut best = 0;
    for (index, score) in scores.iter().enumerate().skip(1) {
        if *score > scores[best] {
            best = index;
        }
    }
    best
}

/// `scores_from_probabilities` and `sequence_scores`: each sequence's candidate conversions.
fn sequence_scores(split: &SplitFrames, probabilities: &[f64]) -> Vec<Sequence> {
    assert_eq!(probabilities.len(), split.frames.len() * ACTIONS, "invalid prefix probability dimensions");
    let count = split.frames.iter().map(|frame| frame.sequence as usize + 1).max().unwrap_or(0);
    let mut sequences = vec![Sequence::default(); count];
    for (index, frame) in split.frames.iter().enumerate() {
        let scores = &probabilities[index * ACTIONS..(index + 1) * ACTIONS];
        assert!(
            scores.iter().all(|value| value.is_finite() && (0.0..=1.0).contains(value)) && (ks_lang::py_sum(scores.iter().copied()) - 1.0).abs() <= PROBABILITY_SUM_TOLERANCE,
            "invalid prefix probabilities"
        );
        let value = &mut sequences[frame.sequence as usize];
        value.profile = frame.profile;
        value.desired = frame.desired;
        value.full_length = frame.full_length;
        if frame.supported && best_action(scores) == CONVERT_LABEL {
            value.candidates.push((frame.length, scores[CONVERT_LABEL]));
        }
    }
    sequences
}

#[derive(Clone, Copy, Default)]
struct Metrics {
    sequences: i64,
    desired: i64,
    converted: i64,
    converted_before_end: i64,
    false_conversions: i64,
}

impl Metrics {
    fn value(&self) -> Value {
        json!({"sequences": self.sequences, "desired": self.desired, "converted": self.converted, "converted_before_end": self.converted_before_end, "false": self.false_conversions})
    }
}

fn sequence_metrics(scores: &[Sequence], threshold: f64) -> [Metrics; 2] {
    let mut results = [Metrics::default(); 2];
    for value in scores {
        let counts = &mut results[value.profile as usize];
        let first = value.candidates.iter().filter(|candidate| candidate.1 >= threshold).map(|candidate| candidate.0).min();
        counts.sequences += 1;
        counts.desired += i64::from(value.desired);
        counts.converted += i64::from(value.desired && first.is_some());
        counts.converted_before_end += i64::from(value.desired && first.is_some_and(|length| length < value.full_length));
        counts.false_conversions += i64::from(!value.desired && first.is_some());
    }
    results
}

fn metrics_value(metrics: &[Metrics; 2]) -> Value {
    json!({PROFILES[0]: metrics[0].value(), PROFILES[1]: metrics[1].value()})
}

#[derive(Clone)]
struct Selection {
    threshold: f64,
    minimum_recall: f64,
    pooled_recall: f64,
    loss: f64,
    by_profile: Value,
}

impl Selection {
    fn rank(&self) -> [f64; 4] {
        [self.minimum_recall, self.pooled_recall, -self.loss, -self.threshold]
    }

    fn value(&self) -> Value {
        json!({"threshold": self.threshold, "minimum_recall": self.minimum_recall, "pooled_recall": self.pooled_recall, "loss": self.loss, "by_profile": self.by_profile, "net_benefit": 0, "minimum_net_benefit": 0})
    }
}

fn better(candidate: &Selection, known: &Selection) -> bool {
    candidate.rank().partial_cmp(&known.rank()) == Some(std::cmp::Ordering::Greater)
}

/// `assess_prefix_epoch`: a development frontier on whole sequences, requiring zero false in both profiles.
fn assess(scores: &[Sequence], thresholds: &[f64], loss: f64) -> Option<Selection> {
    let mut best: Option<Selection> = None;
    let mut grid = thresholds.to_vec();
    grid.sort_by(f64::total_cmp);
    grid.dedup();
    for threshold in grid {
        let metrics = sequence_metrics(scores, threshold);
        assert!(metrics.iter().all(|row| row.desired != 0 && row.desired != row.sequences), "prefix development requires both classes in both lexical profiles");
        if metrics.iter().any(|row| row.false_conversions != 0) {
            continue;
        }
        let recall = |row: &Metrics| row.converted_before_end as f64 / row.desired as f64;
        let mut by_profile = Map::new();
        for (name, row) in PROFILES.iter().zip(&metrics) {
            let mut value = row.value();
            value["conversion_recall"] = json!(recall(row));
            by_profile.insert((*name).to_owned(), value);
        }
        let candidate = Selection {
            threshold,
            minimum_recall: recall(&metrics[0]).min(recall(&metrics[1])),
            pooled_recall: (metrics[0].converted_before_end + metrics[1].converted_before_end) as f64 / (metrics[0].desired + metrics[1].desired) as f64,
            loss,
            by_profile: Value::Object(by_profile),
        };
        if best.as_ref().is_none_or(|known| better(&candidate, known)) {
            best = Some(candidate);
        }
    }
    best
}

fn calibration_passed(metrics: &[Metrics; 2], minimum_recall: f64) -> bool {
    if !(PREFIX_MIN_EARLY_RECALL_FLOOR..=1.0).contains(&minimum_recall) {
        return false;
    }
    metrics.iter().all(|counts| counts.desired != 0 && counts.sequences != counts.desired && counts.false_conversions == 0 && counts.converted_before_end as f64 / counts.desired as f64 >= minimum_recall)
}

/// The inputs of a fit; what a seal pins.
fn prefix_provenance(root: &Path, recipe_path: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut result = provenance(root, recipe_path)?;
    for name in ["model/prefix_v1/corpus.json", "model/prefix_v1/config.json", "model/prefix_v1/train.jsonl.gz", "model/prefix_v1/development.jsonl.gz", "model/prefix_v1/calibration.jsonl.gz"] {
        result.insert(name.to_owned(), checksum(&root.join(name))?);
    }
    Ok(result)
}

pub struct PrefixRequest<'a> {
    pub root: &'a Path,
    pub resource_dir: &'a Path,
    pub recipe: &'a Path,
    pub output: &'a Path,
    pub jobs: usize,
    pub reuse_installed: bool,
}

/// `reuse_installed`: the installed candidate and its seal as the output of a fit, when the seal pins this tree's inputs.
fn reuse_installed(request: &PrefixRequest<'_>) -> Result<Option<Value>, String> {
    let seal = read_json(&request.root.join("model/prefix_v2/candidate-seal.json"))?;
    let artifact = request.resource_dir.join("models/prefix_policy_v1.json");
    let pinned = serde_json::to_value(prefix_provenance(request.root, request.recipe)?).map_err(|error| error.to_string())?;
    if seal.get("provenance") != Some(&pinned) || seal.get("candidate_sha256") != Some(&Value::String(checksum(&artifact)?)) || seal.get("calibration_passed").and_then(Value::as_bool) != Some(true) {
        return Ok(None);
    }
    std::fs::create_dir_all(request.output).map_err(|error| error.to_string())?;
    let copy = |from: &Path, name: &str| std::fs::copy(from, request.output.join(name)).map_err(|error| error.to_string());
    copy(&artifact, "prefix-candidate.json")?;
    copy(&request.root.join("model/prefix_v2/candidate-seal.json"), "seal.json")?;
    copy(request.recipe, "recipe.json")?;
    Ok(Some(seal))
}

/// `fit`: an explicit candidate fit, preserving all existing model directories.
#[allow(clippy::too_many_lines)]
pub fn fit(request: &PrefixRequest<'_>) -> Result<Value, String> {
    let output = request.output;
    if output.exists() {
        return Err("refusing to overwrite a prefix experiment".to_owned());
    }
    if request.reuse_installed {
        if let Some(seal) = reuse_installed(request)? {
            return Ok(seal);
        }
    }
    let cfg = read_json(request.recipe)?;
    let feature_version = cfg.int("runtime_feature_version");
    let frozen = prefix_provenance(request.root, request.recipe)?;
    std::fs::create_dir_all(output).map_err(|error| error.to_string())?;
    let directory: PathBuf = request.root.join("model/prefix_v1");
    let limits = &cfg["maximum_families"];
    let mut parents: HashMap<&str, Vec<ParentWord>> = HashMap::new();
    for split in FITTING_SPLITS {
        parents.insert(split, load_parents(&directory, split, limits.int(split), cfg.int("maximum_words_per_family"))?);
    }
    let family_sets: Vec<BTreeSet<&str>> = FITTING_SPLITS.iter().map(|split| parents[split].iter().map(|row| row.family.as_str()).collect()).collect();
    for left in 0..family_sets.len() {
        for right in left + 1..family_sets.len() {
            if family_sets[left].intersection(&family_sets[right]).next().is_some() {
                return Err("source physical families overlap".to_owned());
            }
        }
    }
    let resources: Arc<Resources> = Resources::with_speller(request.resource_dir.to_path_buf(), ks_hunspell::hunspell_speller(ks_config::history::data_dir()));
    let portable = reference_models(request.root, &resources, false)?;
    let spelling = reference_models(request.root, &resources, true)?;
    let indexes = indexes_of(&portable, &spelling);
    let maximum_prefix_length = cfg.int("maximum_prefix_length") as usize;
    let frames_of = |split: &str| generate_frames(&parents[split], [&portable, &spelling], &indexes, maximum_prefix_length, feature_version, request.jobs);
    let train_frames = frames_of(TRAIN);
    println!("train: {} frames", train_frames.frames.len());
    let names = feature_vocabulary(&train_frames, cfg.float("minimum_feature_mass"), cfg.int("maximum_features") as usize);
    if names.is_empty() {
        return Err("no prefix features survive the family-mass cutoff".to_owned());
    }
    let train = packed_rows(&train_frames, &names, &cfg);
    let development_frames = frames_of(DEVELOPMENT);
    let development = packed_rows(&development_frames, &names, &cfg);
    if train.rows() == 0 || development.rows() == 0 {
        return Err("empty training or development split".to_owned());
    }
    let mut weights = vec![0.0; names.len() * ACTIONS];
    let mut accumulator = vec![1.0; names.len() * ACTIONS];
    let mut best = weights.clone();
    let mut selected_epoch = 0;
    let mut selected: Option<Selection> = None;
    let mut history = Vec::new();
    let denominator = fsum(&development.importance).expect("finite importance");
    let thresholds: Vec<f64> = cfg["thresholds"].as_array().ok_or("thresholds")?.iter().filter_map(Value::as_f64).collect();
    let learning_rate = cfg.float("learning_rate");
    for epoch in 0..cfg.int("epochs") {
        kernel::epoch(&train, &mut weights, &mut accumulator, learning_rate);
        let rounded: Vec<f64> = weights.iter().map(|value| py_round_ndigits(*value, DETERMINISTIC_ROUNDING_DECIMALS).expect("a finite weight")).collect();
        let probabilities = kernel::predict(&development, &rounded);
        let terms: Vec<f64> = (0..development.rows()).map(|index| -development.importance[index] * probabilities[index * ACTIONS + usize::from(development.labels[index])].max(LOG_LOSS_PROBABILITY_FLOOR).ln()).collect();
        let loss = fsum(&terms).expect("finite losses") / denominator;
        assert!(loss.is_finite(), "prefix optimisation diverged");
        let candidate = assess(&sequence_scores(&development_frames, &probabilities), &thresholds, loss);
        if let Some(found) = &candidate {
            if selected.as_ref().is_none_or(|known| better(found, known)) {
                best.clone_from(&rounded);
                selected = Some(found.clone());
                selected_epoch = epoch + 1;
            }
        }
        history.push(json!({"epoch": epoch + 1, "development_loss": loss, "selection": candidate.as_ref().map_or(Value::Null, Selection::value)}));
        println!("prefix-v2 epoch {}: development loss {loss:.9}, early recall {:?}", epoch + 1, candidate.as_ref().map(|found| found.minimum_recall));
    }
    let selected = selected.ok_or("no zero-false development epoch frontier")?;
    let row_of = |index: usize| best[index * ACTIONS..(index + 1) * ACTIONS].to_vec();
    let table: Map<String, Value> = names.iter().enumerate().map(|(index, name)| (name.clone(), json!(row_of(index)))).collect();
    let learned = Value::Object(table);
    let weight_digest = hex(&Sha256::digest(compact(&learned).as_bytes()));
    let version = format!("prefix-v{feature_version}-{}", &weight_digest[..VERSION_HASH_CHARACTERS]);
    let entries: Vec<(&str, [f64; ACTIONS])> = names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let mut row = [0.0; ACTIONS];
            row.copy_from_slice(&best[index * ACTIONS..(index + 1) * ACTIONS]);
            (name.as_str(), row)
        })
        .collect();
    let model = PrefixModel { weights: WeightTable::from_entries(entries), version: version.clone(), conversion_threshold: 1.0 };
    let calibration_frames = frames_of(CALIBRATION);
    let mut probabilities = Vec::with_capacity(calibration_frames.frames.len() * ACTIONS);
    for frame in &calibration_frames.frames {
        // The runtime's own softmax: what the installed candidate answers.
        let values: indexmap_like::Map = frame.features.iter().map(|(id, value)| (calibration_frames.names[*id as usize].clone(), *value)).collect();
        probabilities.extend_from_slice(&model.predict_features(&values).probabilities);
    }
    let scores = sequence_scores(&calibration_frames, &probabilities);
    let minimum_recall = cfg.float("minimum_early_recall");
    let mut threshold = 1.0;
    let mut passed = false;
    for candidate in &thresholds {
        if calibration_passed(&sequence_metrics(&scores, *candidate), minimum_recall) {
            threshold = *candidate;
            passed = true;
            break;
        }
    }
    let actions: Vec<&str> = ks_context::context_model::ACTIONS.iter().map(|action| action.as_str()).collect();
    let payload = json!({"kind": "keyswitch.prefix-policy", "feature_version": feature_version, "prefix_feature_version": feature_version, "actions": actions,
        "version": version, "conversion_threshold": threshold, "weights_sha256": weight_digest, "weights": learned});
    let raw = crate::artifact::canonical(&payload);
    let mut budgets = Map::new();
    let mut pair_counts = Map::new();
    for (split, frames) in [(TRAIN, &train_frames), (DEVELOPMENT, &development_frames), (CALIBRATION, &calibration_frames)] {
        let mut amounts = vec![0.0_f64; frames.families.len()];
        let mut errors = vec![0.0_f64; frames.families.len()];
        for frame in &frames.frames {
            let family = frame.parent_family as usize;
            let previous = amounts[family];
            let corrected = frame.sample_weight - errors[family];
            let total = previous + corrected;
            errors[family] = (total - previous) - corrected;
            amounts[family] = total;
        }
        if amounts.iter().any(|value| (value - 1.0).abs() > FEATURE_MASS_TOLERANCE) {
            return Err("physical-family budget drift".to_owned());
        }
        let table: Map<String, Value> = frames.families.iter().cloned().zip(amounts.iter().map(|value| json!(value))).collect();
        budgets.insert(split.to_owned(), Value::Object(table));
        pair_counts.insert(split.to_owned(), json!(frames.pairs));
    }
    let selected_parents: Map<String, Value> = FITTING_SPLITS.iter().map(|split| ((*split).to_owned(), json!(parents[split].iter().map(|row| row.identifier.as_str()).collect::<Vec<_>>()))).collect();
    let final_development = sequence_scores(&development_frames, &kernel::predict(&development, &best));
    let seal = json!({
        "schema_version": 1, "candidate_sha256": hex(&Sha256::digest(&raw)), "provenance": frozen, "recipe": cfg, "model_version": version, "features": names.len(),
        "epoch": selected_epoch, "development_loss": selected.loss, "threshold": threshold, "epoch_selection": selected.value(), "epoch_history": history,
        "training_rows": train.rows(), "development_rows": development.rows(), "family_budgets": budgets, "pair_counts": pair_counts, "selected_parents": selected_parents,
        "development": metrics_value(&sequence_metrics(&final_development, threshold)), "calibration": metrics_value(&sequence_metrics(&scores, threshold)), "calibration_passed": passed,
        "promotion_accepted": false, "independent_test_evaluated": false, "scope": cfg["scope"],
    });
    if prefix_provenance(request.root, request.recipe)? != frozen {
        return Err("prefix fit inputs changed".to_owned());
    }
    let write = |name: &str, bytes: &[u8]| std::fs::write(output.join(name), bytes).map_err(|error| error.to_string());
    write("prefix-candidate.json", &raw)?;
    ks_context::prefix_schema::VersionedPrefixModel::load(&output.join("prefix-candidate.json")).map_err(|error| format!("prefix export does not load: {}", error.name))?;
    write("seal.json", &readable(&seal))?;
    write("recipe.json", &std::fs::read(request.recipe).map_err(|error| error.to_string())?)?;
    Ok(seal)
}

mod indexmap_like {
    pub type Map = ks_context::context_action_features::FeatureMap;
}
