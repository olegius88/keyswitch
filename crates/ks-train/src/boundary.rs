//! The boundary-v2 model: balanced segmentation examples frozen before any held-out evaluation (`boundary_v2_corpus`),
//! pairwise learning, development selection, calibration and the sealed test (`train_boundary_v2`).
use ks_cast::To as _;
use crate::artifact::canonical;
use crate::corpus::{checksum, read_json};
use crate::kernel::{self, Packed};
use crate::physical::{hex, pair};
use crate::tools::reference_models;
use crate::{ACTIONS, DETERMINISTIC_ROUNDING_DECIMALS, LOG_LOSS_PROBABILITY_FLOOR};
use flate2::Compression;
use flate2::read::MultiGzDecoder;
use flate2::write::GzEncoder;
use indexmap::IndexMap;
use ks_lang::boundary_model::Features;
use ks_lang::boundary_policy::{BoundaryPolicy, FEATURE_VERSION, features, misspelled};
use ks_lang::constants::{BOUNDARY_AMBIGUOUS_PUNCTUATION, BOUNDARY_MAX_LITERAL_SUFFIX_CHARACTERS, BOUNDARY_V2_VERSION_PREFIX};
use ks_lang::language_model::locale_fallbacks;
use ks_lang::{LanguageModel, Resources, py_sum};
use ks_text::pynum::py_round_ndigits;
use ks_text::pystr;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const BOUNDARY_MAX_SUFFIX: usize = ks_cast::const_usize(BOUNDARY_MAX_LITERAL_SUFFIX_CHARACTERS);
const FAMILY_KEY_CHARACTERS: usize = 4;
const SPLIT_BUCKET_COUNT: u64 = 10;
const TRAIN_SPLIT_BUCKETS: u64 = 6;
const WORD_MIN_CHARACTERS: usize = 3;
const WORD_MAX_CHARACTERS: usize = 24;
const LITERAL_SUFFIXES: [&str; 8] = [",", ".", ";", "[", "]", "...", "'", "`"];
const TYPO_MIN_CHARACTERS: usize = 7;
const TYPO_CHOICE_HEX_DIGITS: usize = 2;
const TYPO_SAMPLE_MODULUS: u64 = 7;
const CHOICE_HEX_DIGITS: usize = 8;
const WEIGHT_DECIMALS: i64 = 12;
const VERSION_HASH_CHARACTERS: usize = 12;
const PROGRESS_EPOCHS: usize = 10;
const SPLITS: [&str; 4] = ["train", "development", "calibration", "test"];
const FITTING: [&str; 3] = ["train", "development", "calibration"];
const LOCALES: [&str; 2] = ["en_US", "ru_RU"];
const CONFIG: &str = "model/boundary_v2/config.json";
const SOURCE: &str = "model/context_v2/sources/tatoeba-cc0-en-ru.tsv.gz";
const SOURCE_SHA256: &str = "1f7df948b946f1d1aed5efd2a83a9a5a09d2b8abcb9224045b40c9ec6720e793";
const TEST_SCOPE: &str = "First sealed synthetic segmentation evaluation of this candidate, on supervision families it was not fitted, selected or calibrated on; subsequent replays are regression only.";

fn digest(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
}

/// `int(digest[:digits], 16)`.
fn choice(hexadecimal: &str, digits: usize) -> u64 {
    u64::from_str_radix(&hexadecimal[..digits], 16).expect("a hexadecimal digest")
}

fn is_cyrillic(character: char) -> bool {
    ('а'..='я').contains(&character) || character == 'ё'
}

fn translated(text: &str, source: &str, target: &str) -> String {
    pair().translate(text, source, target).expect("known layouts")
}

/// `canonical_tokens`: the words of a phrase, in `[^\W_]+(?:['’\-][^\W_]+)*`.
pub fn canonical_tokens(text: &str) -> Vec<String> {
    let normalized = pystr::casefold(&ks_text::normalize::nfc(text)).replace('ё', "е").replace('’', "'");
    let characters: Vec<char> = normalized.chars().collect();
    let letter = |character: char| character != '_' && pystr::isalnum(character.encode_utf8(&mut [0; 4]));
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < characters.len() {
        if !letter(characters[index]) {
            index += 1;
            continue;
        }
        let start = index;
        while index < characters.len() && letter(characters[index]) {
            index += 1;
        }
        while index + 1 < characters.len() && matches!(characters[index], '\'' | '’' | '-') && letter(characters[index + 1]) {
            index += 1;
            while index < characters.len() && letter(characters[index]) {
                index += 1;
            }
        }
        tokens.push(characters[start..index].iter().collect());
    }
    tokens
}

/// `load_source`: the phrases of the frozen CC0 source, by identifier.
fn load_source(root: &Path) -> Result<Vec<String>, String> {
    let path = root.join(SOURCE);
    if checksum(&path)? != SOURCE_SHA256 {
        return Err("frozen CC0 source checksum mismatch".to_owned());
    }
    let mut raw = String::new();
    MultiGzDecoder::new(std::fs::File::open(&path).map_err(|error| error.to_string())?).read_to_string(&mut raw).map_err(|error| error.to_string())?;
    let mut phrases = BTreeMap::new();
    for line in pystr::splitlines(&raw, true) {
        let fields: Vec<&str> = line.strip_suffix('\n').unwrap_or(line).split('\t').collect();
        let [identifier, language, text, _modified] = fields.as_slice() else { return Err("invalid CC0 TSV row".to_owned()) };
        if *language != "eng" && *language != "rus" {
            continue;
        }
        let number: i64 = identifier.parse().map_err(|_| "invalid CC0 source ID")?;
        if number <= 0 || phrases.insert(number, (*text).to_owned()).is_some() {
            return Err("nonpositive or duplicate source ID".to_owned());
        }
    }
    Ok(phrases.into_values().collect())
}

/// `onboard_vocabulary`: the frozen onboard lexicons the packages ship, and the built-in fallback words; the packaged
/// supplement is served but not sampled.
fn onboard_vocabulary(root: &Path) -> Result<Vec<Vec<String>>, String> {
    let config = read_json(&root.join("model/intent_v1/config.json"))?;
    let languages = &config["sources"]["languages"];
    let mut vocabulary = Vec::new();
    for locale in LOCALES {
        let path = root.join(languages[locale]["path"].as_str().ok_or("a lexicon without a path")?);
        if Some(checksum(&path)?.as_str()) != languages[locale]["sha256"].as_str() {
            return Err("onboard lexicon checksum mismatch".to_owned());
        }
        let (frequencies, _) = LanguageModel::_read_arpa(&path);
        let mut words: HashSet<String> = frequencies.iter().map(|(word, _)| word.to_owned()).collect();
        words.extend(locale_fallbacks(locale).iter().map(|word| (*word).to_owned()));
        let mut ordered: Vec<(Vec<u8>, String)> = words.into_iter().map(|word| (Sha256::digest(word.as_bytes()).to_vec(), word)).collect();
        ordered.sort();
        vocabulary.push(ordered.into_iter().map(|entry| entry.1).collect());
    }
    Ok(vocabulary)
}

fn family(word: &str, group: usize) -> String {
    let physical = if group == 1 { translated(word, "ru", "us") } else { word.to_owned() };
    pystr::casefold(&physical).replace('`', "t").chars().take(FAMILY_KEY_CHARACTERS).collect()
}

fn split_for(signature: &str, namespace: &str) -> &'static str {
    let bucket = choice(&digest(&format!("{namespace}{signature}")), CHOICE_HEX_DIGITS) % SPLIT_BUCKET_COUNT;
    match bucket {
        bucket if bucket < TRAIN_SPLIT_BUCKETS => "train",
        bucket if bucket == TRAIN_SPLIT_BUCKETS => "development",
        bucket if bucket == TRAIN_SPLIT_BUCKETS + 1 => "calibration",
        _ => "test",
    }
}

fn typo_sampled(word: &str) -> bool {
    choice(&digest(word), TYPO_CHOICE_HEX_DIGITS).is_multiple_of(TYPO_SAMPLE_MODULUS)
}

/// The key a misspelling drops, chosen per word: any key but the last, the one this model segments.
fn typo_position(word: &str, keys: usize) -> usize {
    (choice(&digest(&format!("typo:{word}")), CHOICE_HEX_DIGITS) % (keys.to::<u64>() - 1)).to::<usize>()
}

fn typo_suffix(word: &str) -> &'static str {
    LITERAL_SUFFIXES[(choice(&digest(&format!("literal:{word}")), CHOICE_HEX_DIGITS) % LITERAL_SUFFIXES.len().to::<u64>()).to::<usize>()]
}

fn hex_of(path: &Path) -> Result<Value, String> {
    Ok(Value::String(checksum(path)?))
}

/// `provenance`: the bytes of every input the rows are computed from.
pub fn corpus_provenance(root: &Path) -> Result<Value, String> {
    let paths = [
        "crates/ks-train/src/boundary.rs", CONFIG, SOURCE, "crates/ks-lang/src/boundary_policy.rs", "crates/ks-lang/src/boundary_model.rs", "crates/ks-lang/src/language_model.rs",
        "crates/ks-text/src/layouts.rs", "crates/ks-lang/src/lexicon_supplement.rs", "resources/lexicon-supplement-ru_RU.json", "crates/ks-train/src/tools.rs",
    ];
    let mut table = Map::new();
    for path in paths {
        table.insert(path.to_owned(), hex_of(&root.join(path))?);
    }
    table.insert("constants_sha256".to_owned(), hex_of(&root.join("crates/ks-lang/src/constants.rs"))?);
    let config = read_json(&root.join("model/intent_v1/config.json"))?;
    let contract = json!({"languages": config["sources"]["languages"], "hunspell": config["external_evaluation"]["hunspell"]});
    let raw = canonical(&contract);
    table.insert("lexical_contract_sha256".to_owned(), Value::String(hex(&Sha256::digest(&raw[..raw.len() - 1]))));
    Ok(Value::Object(table))
}

struct Row {
    family: String,
    split: &'static str,
    original: String,
    intended_word: String,
    labels: Vec<usize>,
    category: String,
    features: Vec<Features>,
}

fn table(values: &IndexMap<String, f64>) -> Value {
    Value::Object(values.iter().map(|(name, value)| (name.clone(), json!(value))).collect())
}

fn row_json(row: &Row) -> Value {
    let features: Vec<Value> = row.features.iter().map(table).collect();
    json!({"family": row.family, "split": row.split, "original": row.original, "intended_word": row.intended_word, "labels": row.labels, "category": row.category, "features": features})
}

struct Sample<'a> {
    word: &'a str,
    physical: &'a str,
    signature: &'a str,
    split: &'static str,
    group: usize,
    suffix: &'a str,
    typo: bool,
}

fn add(rows: &mut BTreeMap<String, Row>, models: &[Arc<LanguageModel>], sample: &Sample<'_>) {
    let Sample { word, physical, signature, split, group, suffix, typo } = *sample;
    let original = format!("{physical}{suffix}");
    let length = original.chars().count();
    let tail = length - pystr::rstrip_chars(&original, BOUNDARY_AMBIGUOUS_PUNCTUATION).chars().count();
    if tail == 0 || tail > BOUNDARY_MAX_SUFFIX || tail >= length {
        return;
    }
    let label = suffix.chars().count();
    // A misspelled word is categorised exactly like a correct one, with the prefix typo_, so the trainer's per-category
    // balance weighs its ending against its literals as it does for correct words.
    let category = format!("{}{}", if typo { "typo_" } else { "" }, if suffix.is_empty() { "word_ending" } else if group == 1 { "literal_ru" } else { "literal_en" });
    let key = format!("{split}:{original}");
    if let Some(row) = rows.get_mut(&key) {
        if typo && row.features.iter().any(|values| values["word:known_either"] != 0.0) {
            return;
        }
        if !row.labels.contains(&label) {
            row.labels.push(label);
            row.labels.sort_unstable();
            row.category = "ambiguous".to_owned();
        }
        return;
    }
    let alternative = translated(&original, "us", "ru");
    let values: Vec<Features> = (0..=tail).map(|count| features(&original, &alternative, count.to::<i64>(), &models[0], &models[1], FEATURE_VERSION)).collect();
    if typo && values.iter().any(|value| value["word:known_either"] != 0.0) {
        return;
    }
    rows.insert(key, Row { family: signature.to_owned(), split, original, intended_word: word.to_owned(), labels: vec![label], category, features: values });
}

/// `freeze`: the frozen rows and their receipt, in `directory`.
pub fn freeze(root: &Path, resources: &Arc<Resources>, directory: &Path) -> Result<Value, String> {
    if directory.join("corpus.json").exists() || SPLITS.iter().any(|split| directory.join(format!("{split}.jsonl.gz")).exists()) {
        return Err("refusing to overwrite frozen boundary examples".to_owned());
    }
    let config = read_json(&root.join(CONFIG))?;
    let namespace = config["namespace"].as_str().ok_or("a boundary namespace")?;
    let (maximum_stratum, maximum_family) = (config["maximum_words_per_stratum"].as_i64().ok_or("a stratum limit")?, config["maximum_words_per_family"].as_i64().ok_or("a family limit")?);
    let pool = onboard_vocabulary(root)?;
    let models = reference_models(root, resources, false)?;
    let mut consumed = HashSet::new();
    for phrase in load_source(root)? {
        for word in canonical_tokens(&phrase) {
            if pystr::isalpha(&word) {
                consumed.insert(family(&word, usize::from(word.chars().any(is_cyrillic))));
            }
        }
    }
    let mut rows: BTreeMap<String, Row> = BTreeMap::new();
    let mut strata: HashMap<&str, i64> = HashMap::new();
    let mut families: HashMap<String, i64> = HashMap::new();
    for (group, vocabulary) in pool.iter().enumerate() {
        for word in vocabulary {
            let length = word.chars().count();
            if !(WORD_MIN_CHARACTERS..=WORD_MAX_CHARACTERS).contains(&length) || !word.chars().all(|character| if group == 1 { is_cyrillic(character) } else { character.is_ascii_lowercase() }) {
                continue;
            }
            let physical = if group == 1 { translated(word, "ru", "us") } else { word.clone() };
            let signature = family(word, group);
            let ends_ambiguous = physical.chars().last().is_some_and(|last| BOUNDARY_AMBIGUOUS_PUNCTUATION.contains(last));
            let stratum = if group == 0 { "en" } else if ends_ambiguous { "ru_ending" } else { "ru_other" };
            let split = split_for(&signature, namespace);
            // Test families are fresh for all earlier phrase-derived tasks, not a newly shuffled version of the rejected test.
            if split == "test" && consumed.contains(&signature) {
                continue;
            }
            if strata.get(stratum).copied().unwrap_or(0) >= maximum_stratum || families.get(&signature).copied().unwrap_or(0) >= maximum_family {
                continue;
            }
            *strata.entry(stratum).or_insert(0) += 1;
            *families.entry(signature.clone()).or_insert(0) += 1;
            let sample = |physical: &'_ str, suffix: &'_ str, typo: bool, rows: &mut BTreeMap<String, Row>| {
                add(rows, &models, &Sample { word, physical, signature: &signature, split, group, suffix, typo });
            };
            if group == 1 && ends_ambiguous {
                sample(&physical, "", false, &mut rows);
            }
            for suffix in LITERAL_SUFFIXES {
                sample(&physical, suffix, false, &mut rows);
            }
            let keys: Vec<char> = physical.chars().collect();
            if keys.len() >= TYPO_MIN_CHARACTERS && typo_sampled(word) {
                let position = typo_position(word, keys.len());
                let changed: String = keys.iter().enumerate().filter(|(index, _)| *index != position).map(|(_, key)| *key).collect();
                for suffix in ["", typo_suffix(word)] {
                    sample(&changed, suffix, true, &mut rows);
                }
            }
        }
    }
    for row in rows.values_mut() {
        // Counterfactual, valid lexicon words for every span: the same physical input with two intended outputs is
        // ambiguous without context, and neither label may be forced.
        let mut labels: BTreeSet<usize> = row.labels.iter().copied().collect();
        let known: Vec<usize> = row.features.iter().enumerate().filter(|(_, value)| value["word:known_either"] != 0.0).map(|(index, _)| index).collect();
        if known.is_empty() {
            let alternative = translated(&row.original, "us", "ru");
            let prefix = |text: &str, index: usize| -> String { let count = text.chars().count(); text.chars().take(count - index).collect() };
            labels.extend((0..row.features.len()).filter(|index| misspelled(&prefix(&row.original, *index), &models[0]) || misspelled(&prefix(&alternative, *index), &models[1])));
        } else {
            labels.extend(known);
        }
        if labels.len() > 1 {
            row.labels = labels.into_iter().collect();
            row.category = "ambiguous".to_owned();
        }
    }
    let mut counts = Map::new();
    let (mut categories, mut family_counts, mut hashes) = (Map::new(), Map::new(), Map::new());
    let mut groups: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    for split in SPLITS {
        let mut data = Vec::new();
        let mut number = 0;
        let mut local: BTreeMap<String, i64> = BTreeMap::new();
        let members = groups.entry(split).or_default();
        for row in rows.values().filter(|row| row.split == split) {
            data.extend(canonical(&row_json(row)));
            number += 1;
            *local.entry(row.category.clone()).or_insert(0) += 1;
            members.insert(row.family.clone());
        }
        let mut encoder = GzEncoder::new(Vec::new(), Compression::new(9));
        encoder.write_all(&data).map_err(|error| error.to_string())?;
        let path = directory.join(format!("{split}.jsonl.gz"));
        std::fs::write(&path, encoder.finish().map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
        hashes.insert(split.to_owned(), hex_of(&path)?);
        counts.insert(split.to_owned(), json!(number));
        categories.insert(split.to_owned(), json!(local));
        family_counts.insert(split.to_owned(), json!(members.len()));
    }
    for a in SPLITS {
        for b in SPLITS {
            if a != b && !groups[a].is_disjoint(&groups[b]) {
                return Err("boundary family contamination".to_owned());
            }
        }
    }
    if groups["test"].iter().any(|member| consumed.contains(member)) {
        return Err("boundary family contamination".to_owned());
    }
    let selected: BTreeMap<&str, i64> = strata.into_iter().collect();
    let receipt = json!({"schema_version": 1, "provenance": corpus_provenance(root)?, "sha256": hashes, "rows": counts, "categories": categories, "families": family_counts, "family_overlap": 0, "prior_phrase_test_family_overlap": 0, "selected_words": selected, "scope": config["evidence_scope"]});
    std::fs::write(directory.join("corpus.json"), canonical(&receipt)).map_err(|error| error.to_string())?;
    Ok(receipt)
}

struct Example {
    category: String,
    labels: Vec<usize>,
    features: Vec<Features>,
}

/// `rows`: a split of the frozen examples, once its bytes are the receipt's.
fn examples(directory: &Path, split: &str) -> Result<Vec<Example>, String> {
    let receipt = read_json(&directory.join("corpus.json"))?;
    let path = directory.join(format!("{split}.jsonl.gz"));
    if Some(checksum(&path)?.as_str()) != receipt["sha256"][split].as_str() {
        return Err("boundary corpus checksum mismatch".to_owned());
    }
    let mut text = String::new();
    MultiGzDecoder::new(std::fs::File::open(&path).map_err(|error| error.to_string())?).read_to_string(&mut text).map_err(|error| error.to_string())?;
    text.lines()
        .map(|line| {
            let row: Value = serde_json::from_str(line).map_err(|error| error.to_string())?;
            let labels = row["labels"].as_array().ok_or("labels")?.iter().filter_map(Value::as_u64).map(|label| label.to::<usize>()).collect();
            let values = row["features"].as_array().ok_or("features")?.iter().map(|table| table.as_object().map_or_else(IndexMap::new, |table| table.iter().map(|(name, value)| (name.clone(), value.as_f64().unwrap_or(0.0))).collect())).collect();
            Ok(Example { category: row["category"].as_str().ok_or("category")?.to_owned(), labels, features: values })
        })
        .collect()
}

fn metrics(model: &BoundaryPolicy, data: &[Example]) -> Value {
    let mut counts: BTreeMap<&str, i64> = BTreeMap::new();
    let mut categories: BTreeMap<&str, BTreeMap<&str, i64>> = BTreeMap::new();
    for row in data {
        let prediction = model.predict(&row.features).expect("a row with candidates");
        let outcome = match prediction.suffix_length {
            None => "abstained",
            Some(length) if row.labels.len() != 1 || length != row.labels[0] => "errors",
            Some(_) => "correct",
        };
        let local = categories.entry(&row.category).or_default();
        for name in ["rows", outcome] {
            *counts.entry(name).or_insert(0) += 1;
            *local.entry(name).or_insert(0) += 1;
        }
    }
    json!({"counts": counts, "categories": categories})
}

fn acceptable(root: &Path, result: &Value, test: bool) -> Result<bool, String> {
    let gates = read_json(&root.join(CONFIG))?["promotion"].clone();
    let counts = &result["counts"];
    let categories = &result["categories"];
    if counts["errors"].as_i64().unwrap_or(0) > gates["maximum_errors"].as_i64().unwrap_or(0) {
        return Ok(false);
    }
    for category in gates["required_recall_strata"].as_array().ok_or("recall strata")?.iter().filter_map(Value::as_str) {
        let local = &categories[category];
        let share = (local["correct"].as_i64().unwrap_or(0)).to::<f64>() / (local["rows"].as_i64().unwrap_or(0).max(1)).to::<f64>();
        if share < gates["minimum_decided_fraction_per_stratum"].as_f64().unwrap_or(1.0) {
            return Ok(false);
        }
    }
    Ok(!test || (counts["rows"].as_i64().unwrap_or(0) >= gates["minimum_test_rows"].as_i64().unwrap_or(0) && categories["word_ending"]["rows"].as_i64().unwrap_or(0) >= gates["minimum_test_word_endings"].as_i64().unwrap_or(0)))
}

/// `comparisons`: this candidate wins over that one, and the other way round.
fn packed(data: &[Example], names: &[String]) -> Result<Packed, String> {
    let mut sizes: HashMap<&str, usize> = HashMap::new();
    for row in data {
        *sizes.entry(&row.category).or_insert(0) += 1;
    }
    let mut rows: Vec<(Vec<(String, f64)>, u8, f64)> = Vec::new();
    for row in data {
        let values = &row.features;
        let denominator = sizes.len() * sizes[row.category.as_str()] * row.labels.len() * (values.len() - 1);
        let importance = data.len().to::<f64>() / denominator.to::<f64>();
        for correct in &row.labels {
            for other in 0..values.len() {
                if *correct == other {
                    continue;
                }
                let mut features: Vec<&String> = values[*correct].keys().chain(values[other].keys()).collect();
                features.sort_by(|a, b| pystr::cmp(a, b));
                features.dedup();
                let difference: Vec<(String, f64)> = features.iter().map(|name| ((*name).clone(), values[*correct].get(*name).copied().unwrap_or(0.0) - values[other].get(*name).copied().unwrap_or(0.0))).collect();
                let opposite: Vec<(String, f64)> = difference.iter().map(|(name, value)| (name.clone(), -value)).collect();
                rows.push((difference, 1, importance));
                rows.push((opposite, 0, importance));
            }
        }
    }
    Packed::build(rows.iter().map(|(features, label, importance)| (features.as_slice(), *label, *importance)), names)
}

/// What the seal pins: the trainer's own sources and the frozen examples.
pub fn provenance(root: &Path, directory: &Path) -> Result<Value, String> {
    Ok(json!({"trainer": hex_of(&root.join("crates/ks-train/src/boundary.rs"))?, "optimizer": hex_of(&root.join("crates/ks-train/src/kernel.rs"))?, "corpus": hex_of(&directory.join("corpus.json"))?, "constants_sha256": hex_of(&root.join("crates/ks-lang/src/constants.rs"))?}))
}

/// A receipt the old program wrote names its files, whose bytes are gone from this tree: its rows are checked by their
/// hashes, and the next freeze writes a receipt of this program's.
fn check_receipt(root: &Path, directory: &Path) -> Result<(), String> {
    let receipt = read_json(&directory.join("corpus.json"))?;
    let frozen_before = receipt["provenance"].get("tools/boundary_v2_corpus.py").is_some();
    if !frozen_before && receipt["provenance"] != corpus_provenance(root)? {
        return Err("boundary corpus provenance changed".to_owned());
    }
    Ok(())
}

/// `train`: the candidate's bytes and its seal's.
pub fn train(root: &Path, directory: &Path) -> Result<(Vec<u8>, Vec<u8>), String> {
    check_receipt(root, directory)?;
    let config = read_json(&root.join(CONFIG))?;
    let data: BTreeMap<&str, Vec<Example>> = FITTING.iter().map(|split| Ok((*split, examples(directory, split)?))).collect::<Result<_, String>>()?;
    let mut names: Vec<String> = data["train"].iter().flat_map(|row| row.features.iter().flat_map(|values| values.keys().cloned())).collect::<HashSet<_>>().into_iter().collect();
    names.sort_by(|a, b| pystr::cmp(a, b));
    let train_rows = packed(&data["train"], &names)?;
    let development = packed(&data["development"], &names)?;
    // The optimizer keeps one weight per feature and context action; a boundary comparison uses the first two.
    let mut weights = vec![0.0; names.len() * ACTIONS];
    let mut accumulators = vec![1.0; names.len() * ACTIONS];
    let (mut best, mut best_loss, mut selected) = (weights.clone(), f64::INFINITY, 0);
    let rate = config["learning_rate"].as_f64().ok_or("a learning rate")?;
    for epoch in 0..(config["epochs"].as_u64().ok_or("epochs")?).to::<usize>() {
        kernel::epoch(&train_rows, &mut weights, &mut accumulators, rate);
        let probabilities = kernel::predict(&development, &weights);
        let terms: Vec<f64> = development.labels.iter().enumerate().map(|(index, label)| -development.importance[index] * probabilities[index * ACTIONS + usize::from(*label)].max(LOG_LOSS_PROBABILITY_FLOOR).ln()).collect();
        let loss = py_sum(terms) / development.labels.len().to::<f64>();
        if loss < best_loss {
            (best, best_loss, selected) = (weights.clone(), loss, epoch + 1);
        }
        if epoch % PROGRESS_EPOCHS == 0 {
            println!("boundary-v2 epoch {}: development loss {loss:.6}", epoch + 1);
        }
    }
    let learned: IndexMap<String, f64> = names.iter().enumerate().map(|(index, name)| (name.clone(), py_round_ndigits(best[index * ACTIONS + 1] - best[index * ACTIONS], WEIGHT_DECIMALS).unwrap_or(0.0))).collect();
    let digest_of_weights = hex(&Sha256::digest(canonical(&table(&learned))));
    let version = format!("{BOUNDARY_V2_VERSION_PREFIX}{}", &digest_of_weights[..VERSION_HASH_CHARACTERS]);
    let mut search = Vec::new();
    let mut threshold = 1.0;
    for candidate in config["thresholds"].as_array().ok_or("thresholds")?.iter().filter_map(Value::as_f64) {
        let result = metrics(&BoundaryPolicy::new(learned.clone(), candidate, version.clone(), FEATURE_VERSION), &data["calibration"]);
        let accepted = acceptable(root, &result, false)?;
        search.push(json!({"threshold": candidate, "result": result}));
        if accepted {
            threshold = candidate;
            break;
        }
    }
    let model = BoundaryPolicy::new(learned.clone(), threshold, version.clone(), FEATURE_VERSION);
    let raw = canonical(&json!({"feature_version": FEATURE_VERSION, "version": version, "weights": table(&learned), "threshold": threshold}));
    let seal = json!({"schema_version": 1, "candidate_sha256": hex(&Sha256::digest(&raw)), "provenance": provenance(root, directory)?, "epoch": selected, "training_rows": data["train"].len(),
        "development_loss": py_round_ndigits(best_loss, DETERMINISTIC_ROUNDING_DECIMALS), "development": metrics(&model, &data["development"]), "calibration": metrics(&model, &data["calibration"]), "threshold_search": search, "config": config, "scope": config["evidence_scope"]});
    Ok((raw, canonical(&seal)))
}

/// `evaluate`: the sealed test's report, after the seal and the calibration gates hold.
pub fn evaluate(root: &Path, directory: &Path, frozen_provenance: bool) -> Result<Vec<u8>, String> {
    let (candidate, seal_path) = (directory.join("candidate.json"), directory.join("seal.json"));
    let seal = read_json(&seal_path)?;
    check_receipt(root, directory)?;
    let changed = seal["candidate_sha256"].as_str() != Some(checksum(&candidate)?.as_str()) || (!frozen_provenance && seal["provenance"] != provenance(root, directory)?) || seal["config"] != read_json(&root.join(CONFIG))?;
    if changed {
        return Err("boundary candidate/provenance changed after seal".to_owned());
    }
    if !acceptable(root, &seal["calibration"], false)? {
        return Err("calibration gates failed; do not open the held-out test".to_owned());
    }
    let model = BoundaryPolicy::load(&candidate).map_err(|error| error.name.to_string())?;
    let result = metrics(&model, &examples(directory, "test")?);
    let accepted = acceptable(root, &result, true)?;
    Ok(canonical(&json!({"schema_version": 1, "seal_sha256": checksum(&seal_path)?, "candidate_sha256": checksum(&candidate)?, "test": result, "accepted": accepted, "scope": TEST_SCOPE})))
}

/// The commands: `--freeze`, `--train`, `--test`, `--verify`, `--promote`.
pub fn command(root: &Path, resources: &Arc<Resources>, action: &str, directory: &Path, output: &Path) -> Result<(), String> {
    let (candidate, seal, report): (PathBuf, PathBuf, PathBuf) = (output.join("candidate.json"), output.join("seal.json"), output.join("report.json"));
    let write = |path: &Path, bytes: &[u8]| std::fs::write(path, bytes).map_err(|error| error.to_string());
    match action {
        "freeze" => {
            println!("{}", ks_text::json_dumps(&freeze(root, resources, directory)?, &ks_text::JsonOptions { indent: Some(ks_text::Indent::Spaces(2)), ..ks_text::JsonOptions::default() }));
        }
        "train" => {
            if report.exists() {
                return Err("test already observed; use a new experiment".to_owned());
            }
            if seal.exists() {
                let history = output.join("development-history");
                std::fs::create_dir_all(&history).map_err(|error| error.to_string())?;
                write(&history.join(format!("{}.json", checksum(&seal)?)), &canonical(&json!({"candidate": read_json(&candidate)?, "seal": read_json(&seal)?, "scope": "superseded before opening test"})))?;
            }
            let (raw, sealed) = train(root, directory)?;
            std::fs::create_dir_all(output).map_err(|error| error.to_string())?;
            write(&candidate, &raw)?;
            write(&seal, &sealed)?;
        }
        "test" => {
            if report.exists() {
                return Err("test already observed; use --verify".to_owned());
            }
            write(&report, &evaluate(root, directory, false)?)?;
        }
        "verify" => {
            let (raw, sealed) = train(root, directory)?;
            let sealed_value: Value = serde_json::from_slice(&sealed).map_err(|error| error.to_string())?;
            let known = read_json(&seal)?;
            let without_provenance = |value: &Value| { let mut copy = value.clone(); copy["provenance"] = Value::Null; copy };
            if raw != std::fs::read(&candidate).map_err(|error| error.to_string())? || without_provenance(&sealed_value) != without_provenance(&known) {
                return Err("boundary-v2 experiment is not reproducible".to_owned());
            }
            if evaluate(root, directory, true)? != std::fs::read(&report).map_err(|error| error.to_string())? {
                return Err("boundary-v2 test report is not reproducible".to_owned());
            }
            println!("{} reproduces", read_json(&candidate)?["version"].as_str().unwrap_or(""));
        }
        "promote" => {
            let result = evaluate(root, directory, false)?;
            let accepted = serde_json::from_slice::<Value>(&result).map_err(|error| error.to_string())?["accepted"] == json!(true);
            if result != std::fs::read(&report).map_err(|error| error.to_string())? || !accepted {
                return Err("boundary-v2 failed promotion gates".to_owned());
            }
            write(&root.join("resources/models/boundary-v2.json"), &std::fs::read(&candidate).map_err(|error| error.to_string())?)?;
        }
        other => return Err(format!("unknown boundary command: {other}")),
    }
    Ok(())
}
