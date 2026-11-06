//! The key-space orthotactic model, by counting (`train_ortho_model`, `ortho_corpus`): character n-gram channels per
//! language, case channels, and feature weights fitted out of fold by a logistic regression. Every step iterates in a
//! fixed order, so `--verify` replays the artifact byte for byte.
use ks_cast::To as _;
use crate::artifact::canonical;
use crate::corpus::{checksum, read_json};
use crate::physical::{hex, hex_digest, pair};
use flate2::read::MultiGzDecoder;
use ks_context::context_policy::word_shaped;
use ks_engine::unicode_category::category;
use ks_lang::identifier_lexicon::IdentifierLexicon;
use ks_lang::ortho_model::{CONTINUOUS_FEATURE, FEATURES, OrthoEvidence, OrthoModel, SCRIPTS, TARGET_FEATURE, collapse_runs, schema_features, token_features, BOS, EOS, Identifier, KnownWord};
use ks_lang::py_sum;
use ks_text::pynum::{fsum, py_round_ndigits};
use ks_text::{py_round, pystr};
use num_bigint::{BigInt, BigUint, Sign};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

const DIRECTORY: &str = "model/ortho_v1";
const NAMESPACE: &str = "keyswitch:ortho-v1:candidate15";
const SYNTHESIS_NAMESPACE: &str = "keyswitch:ortho-v1:extra-key-synthesis";
pub(crate) const AUTHORED_WORDS: [&str; 25] = ["htop", "webjs", "nginx", "bild", ".dist", "я-то", "фпривет", "привет", "клаиватура", "повторнно", "превет", "репост", "севодня", "созвон", "уведмление", "чатик", "пересборку", "healthcheck", "websocket", "hotfix", "npm", "kubectl", "const", "фнафер", "йцукен"];
pub(crate) const AUTHORED_KEYS: [&str; 25] = [".dist", "aghbdtn", "ayfath", "bild", "cjpdjy", "const", "ctdjlyz", "edtlvktybt", "ghbdtn", "ghtdtn", "gjdnjhyyj", "gthtc,jhre", "healthcheck", "hotfix", "htgjcn", "htop", "kubectl", "nginx", "npm", "qwerty", "rkfbdfnehf", "webjs", "websocket", "xfnbr", "z-nj"];
const TRAIN: &str = "train";
const TEST: &str = "test";
pub(crate) const LEXICON: &str = "lexicon";
const EVIDENCE_SCHEMA: i64 = 2;
const VERSION_HASH_CHARACTERS: usize = 12;
const CHOICE_HEX_DIGITS: usize = 8;
const UNIGRAM_PSEUDO_COUNT: f64 = 0.5;
const SHAPE_PSEUDO_COUNT: f64 = 0.5;
const SEALED_DECIMALS: i64 = 6;
const RECALL_DECIMALS: i64 = 6;
const REPORTED_WORST_FALSE: usize = 10;
const REPORTED_SCORE_DECIMALS: i64 = 4;
const EXTRA_KEY_SCHEMA: i64 = 3;
const STRETCH_SCHEMA: i64 = 4;
const REPLACEMENT_SCHEMA: i64 = 5;
const PLAUSIBILITY_SCHEMA: i64 = 6;
const EXTRA_KEY_SHARE_PERCENT: u64 = 10;
const PERCENT_SCALE: u64 = 100;
const LOGISTIC_ITERATIONS: usize = 25;
const LOGISTIC_TOLERANCE: f64 = 1e-7;
const LOGIT_BOUND: f64 = 30.0;
const FIRTH_CENTRE: f64 = 0.5;
const TYPO_MIN_CHARACTERS: usize = 5;
const WEB_TOP_PER_BILLION: i64 = 50_000_000;
const MAXIMUM_LEXICON_WEIGHT: i64 = 32;
const SHARED_TIE: &str = "shared by both directions";
const FEATURE_PARENTS: [(&str, &str); 3] = [("dot_word", "parts_target"), ("extra_key", "parts_target"), ("source_word", "parts_source")];
const SHAPES: [&str; 4] = ["lower", "initial", "inner", "upper"];
const BOUNDARY_PUNCTUATION: &str = ".,!?;:()[]{}—–-…\"«»";
const BOUNDARY_EXEMPT: &str = "_/\\@";
const WORD_JOINERS: &str = "'’-‐‑_@/\\=";

#[derive(Clone)]
pub struct Row {
    pub split: String,
    pub script: String,
    pub keys: String,
    pub shapes: Vec<(String, i64)>,
}

impl Row {
    fn weight(&self) -> i64 {
        self.shapes.iter().map(|shape| shape.1).sum()
    }
}

pub(crate) type Label = (String, String, String);

fn is_alpha(character: char) -> bool {
    pystr::isalpha(character.encode_utf8(&mut [0; 4]))
}

pub(crate) static KEYS: LazyLock<HashSet<char>> = LazyLock::new(|| (33..127_u8).map(char::from).collect());
static LETTERS_EN: LazyLock<HashSet<char>> = LazyLock::new(|| KEYS.iter().copied().filter(|key| is_alpha(*key)).collect());
static LETTERS_RU: LazyLock<HashSet<char>> = LazyLock::new(|| KEYS.iter().copied().filter(|key| pystr::isalpha(&pair().translate(&key.to_string(), "us", "ru").expect("layouts"))).collect());

fn letter_keys(layout: &str) -> &'static HashSet<char> {
    if layout == "en" { &LETTERS_EN } else { &LETTERS_RU }
}

pub(crate) fn other(script: &str) -> &'static str {
    if script == "en" { "ru" } else { "en" }
}

/// `visible` of one key.
fn visible_key(key: char, layout: &str) -> String {
    if layout == "en" {
        return key.to_string();
    }
    if key == '/' {
        return ".".to_owned();
    }
    pair().translate(&key.to_string(), "us", "ru").expect("layouts")
}

pub(crate) fn visible(keys: &str, layout: &str) -> String {
    keys.chars().map(|key| visible_key(key, layout)).collect()
}

/// `ends_a_word`: would the engine commit the word at this character?
fn ends_a_word(character: &str) -> bool {
    let first = character.chars().next().unwrap_or(' ');
    pystr::isspace(character) || BOUNDARY_PUNCTUATION.contains(character) || (!BOUNDARY_EXEMPT.contains(character) && category(first).starts_with('P'))
}

/// `served`: every token the engine hands to a model for these keys typed in this layout.
#[must_use]
pub fn served(keys: &str, layout: &str) -> Vec<String> {
    let (other_letters, keep) = (letter_keys(other(layout)), letter_keys(layout));
    let mut segments: Vec<Vec<char>> = Vec::new();
    let mut current: Vec<char> = Vec::new();
    for key in keys.chars() {
        if keep.contains(&key) || other_letters.contains(&key) {
            current.push(key);
            continue;
        }
        let shown = visible_key(key, layout);
        if !current.is_empty() && WORD_JOINERS.contains(shown.as_str()) {
            current.push(key);
            continue;
        }
        if ends_a_word(&shown) {
            if !current.is_empty() {
                segments.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(key);
    }
    if !current.is_empty() {
        segments.push(current);
    }
    let mut result = Vec::new();
    for segment in segments {
        let core: &[char] = segment.iter().rposition(|key| *key == '/').map_or(&segment, |head| &segment[head + 1..]);
        let mut end = core.len();
        while end > 0 && !keep.contains(&core[end - 1]) && other_letters.contains(&core[end - 1]) {
            end -= 1;
        }
        if end > 0 {
            result.push(core[..end].iter().collect());
        }
    }
    result
}

/// `word_core`: the letters of the token in this layout, for counting rather than for serving.
pub(crate) fn word_core(keys: &str, layout: &str) -> String {
    let characters: Vec<char> = keys.chars().collect();
    let keep = letter_keys(layout);
    let (mut start, mut end) = (0, characters.len());
    while start < end && !keep.contains(&characters[start]) {
        start += 1;
    }
    while end > start && !keep.contains(&characters[end - 1]) {
        end -= 1;
    }
    characters[start..end].iter().collect()
}

/// `extra_key_variant`: the keys of a train word typed in the other layout with one stray key.
pub(crate) fn extra_key_variant(script: &str, keys: &str) -> Option<String> {
    let base = word_core(keys, script);
    let length = base.chars().count();
    if length + 1 < TYPO_MIN_CHARACTERS || !pystr::isalpha(&visible(&base, script)) || AUTHORED_KEYS.contains(&base.as_str()) {
        return None;
    }
    let digest = hex_digest(&format!("{SYNTHESIS_NAMESPACE}:{script}:{base}"));
    let part = |index: usize| u64::from_str_radix(digest.get(index * CHOICE_HEX_DIGITS..(index + 1) * CHOICE_HEX_DIGITS).unwrap_or_default(), 16).expect("a digest");
    if part(0) % PERCENT_SCALE >= EXTRA_KEY_SHARE_PERCENT {
        return None;
    }
    let mut letters: Vec<char> = letter_keys(other(script)).iter().copied().filter(|letter| pystr::lower(&letter.to_string()) == letter.to_string()).collect();
    letters.sort_unstable();
    let position = (part(1) % (length.to::<u64>() + 1)).to::<usize>();
    let mut output: String = base.chars().take(position).collect();
    output.push(letters[(part(2) % letters.len().to::<u64>()).to::<usize>()]);
    output.extend(base.chars().skip(position));
    Some(output)
}

pub(crate) type Counts = HashMap<String, HashMap<String, i64>>;

pub(crate) fn empty_counts() -> Counts {
    SCRIPTS.iter().map(|script| ((*script).to_owned(), HashMap::new())).collect()
}

/// `counts`: character n-gram counts per script, and the case shapes of prose and of abbreviations.
fn counts(rows: &[&Row], order: usize, collapse: Option<i64>) -> (Counts, Counts, Counts) {
    let mut counted: Vec<(&str, String, &[(String, i64)], bool)> = Vec::new();
    for row in rows {
        if row.split == TRAIN {
            let sequences: BTreeSet<String> = SCRIPTS.iter().map(|layout| word_core(&row.keys, layout)).filter(|sequence| !sequence.is_empty()).collect();
            counted.extend(sequences.into_iter().map(|sequence| (row.script.as_str(), sequence, row.shapes.as_slice(), true)));
        } else if row.split == LEXICON {
            counted.push((row.script.as_str(), row.keys.clone(), row.shapes.as_slice(), false));
        }
    }
    if let Some(limit) = collapse {
        for entry in &mut counted {
            entry.1 = collapse_runs(&entry.1, limit);
        }
    }
    let (mut grams, mut shapes, mut acronyms) = (empty_counts(), empty_counts(), empty_counts());
    let mut upper: HashMap<&str, HashSet<String>> = SCRIPTS.iter().map(|script| (*script, HashSet::new())).collect();
    for (script, sequence, shape_counts, cased) in &counted {
        if *cased && shape_counts.iter().any(|(name, value)| name == "upper" && *value != 0) {
            upper.get_mut(script).expect("a script").insert(sequence.clone());
        }
    }
    let padding: String = std::iter::repeat_n(BOS, order - 1).collect();
    for (script, sequence, shape_counts, cased) in &counted {
        let weight: i64 = shape_counts.iter().map(|shape| shape.1).sum();
        let padded: Vec<char> = padding.chars().chain(sequence.chars()).chain([EOS]).collect();
        let table = grams.get_mut(*script).expect("a script");
        for index in order - 1..padded.len() {
            for size in 1..=order {
                let gram: String = padded[index + 1 - size..=index].iter().collect();
                *table.entry(gram).or_insert(0) += weight;
            }
        }
        if !cased {
            continue;
        }
        let target = if upper[script].contains(sequence) { &mut acronyms } else { &mut shapes };
        let table = target.get_mut(*script).expect("a script");
        for (name, value) in *shape_counts {
            *table.entry(name.clone()).or_insert(0) += value;
        }
    }
    (grams, shapes, acronyms)
}

fn drop_last(gram: &str) -> &str {
    gram.char_indices().next_back().map_or("", |(place, _)| gram.get(..place).unwrap_or_default())
}

fn drop_first(gram: &str) -> &str {
    let mut characters = gram.chars();
    characters.next();
    characters.as_str()
}

pub(crate) fn quantise(value: f64, scale: i64) -> i64 {
    (py_round(value * scale.to::<f64>())).to::<i64>()
}

/// `channel_payload` over `fit_channel`: interpolated absolute discounting, quantised as the artifact stores it.
pub(crate) fn channel_payload(gram_counts: &HashMap<String, i64>, order: usize, discount: f64, scale: i64) -> Value {
    let alphabet = gram_counts.keys().filter(|gram| gram.chars().count() == 1).count();
    let (mut context_total, mut context_types): (HashMap<&str, i64>, HashMap<&str, i64>) = Default::default();
    for (gram, count) in gram_counts {
        if gram.chars().count() > 1 {
            *context_total.entry(drop_last(gram)).or_insert(0) += count;
            *context_types.entry(drop_last(gram)).or_insert(0) += 1;
        }
    }
    let uniform = -alphabet.to::<f64>().ln();
    let unigram_total: i64 = gram_counts.iter().filter(|(gram, _)| gram.chars().count() == 1).map(|(_, count)| *count).sum();
    let mut logprob: HashMap<&str, f64> = HashMap::with_capacity(gram_counts.len());
    let mut backoff: BTreeMap<&str, f64> = BTreeMap::new();
    let mut by_size: Vec<Vec<&String>> = vec![Vec::new(); order + 1];
    for gram in gram_counts.keys() {
        by_size[gram.chars().count()].push(gram);
    }
    for (size, grams) in by_size.iter_mut().enumerate().skip(1) {
        grams.sort_unstable();
        for gram in grams.iter() {
            let count = gram_counts[*gram].to::<f64>();
            if size == 1 {
                let probability = (count + UNIGRAM_PSEUDO_COUNT) / (unigram_total.to::<f64>() + UNIGRAM_PSEUDO_COUNT * alphabet.to::<f64>());
                logprob.insert(gram, probability.ln());
                continue;
            }
            let context = drop_last(gram);
            let total = context_total[context].to::<f64>();
            let weight = discount * context_types[context].to::<f64>() / total;
            let lower = logprob.get(drop_first(gram)).map_or(uniform.exp(), |value| value.exp());
            let probability = (count - discount).max(0.0) / total + weight * lower;
            logprob.insert(gram, probability.ln());
            backoff.insert(context, weight.ln());
        }
    }
    let mut names: Vec<&str> = logprob.keys().copied().collect();
    names.sort_unstable();
    json!({
        "grams": names.join("\n"),
        "logprob": names.iter().map(|name| quantise(logprob[name], scale)).collect::<Vec<i64>>(),
        "backoff": backoff.iter().map(|(name, value)| json!([name, quantise(*value, scale)])).collect::<Vec<Value>>(),
        "uniform": quantise(uniform, scale),
    })
}

pub(crate) fn shape_channel(observed: &Counts, scale: i64) -> Value {
    let mut result = Map::new();
    for script in SCRIPTS {
        let counter = &observed[script];
        let total = (counter.values().sum::<i64>()).to::<f64>() + SHAPE_PSEUDO_COUNT * SHAPES.len().to::<f64>();
        let table: Map<String, Value> = SHAPES.iter().map(|shape| ((*shape).to_owned(), json!(quantise(((counter.get(*shape).copied().unwrap_or(0).to::<f64>() + SHAPE_PSEUDO_COUNT) / total).ln(), scale)))).collect();
        result.insert(script.to_owned(), Value::Object(table));
    }
    Value::Object(result)
}

/// `source_rows`: the train tokens, and the word lists merged with the web forms, a key weighing the larger of its two.
fn source_rows(rows: &[&Row], web: &[Row]) -> Vec<Row> {
    let scripts: BTreeSet<&str> = web.iter().map(|row| row.script.as_str()).collect();
    let mut lists: BTreeMap<(String, String), i64> = BTreeMap::new();
    let listed = rows.iter().copied().filter(|row| row.split == LEXICON && scripts.contains(row.script.as_str()));
    for row in listed.chain(web.iter()) {
        let weight = row.weight();
        let entry = lists.entry((row.script.clone(), row.keys.clone())).or_insert(0);
        *entry = (*entry).max(weight);
    }
    let mut output: Vec<Row> = rows.iter().filter(|row| row.split == TRAIN).map(|row| (*row).clone()).collect();
    output.extend(lists.into_iter().map(|((script, keys), weight)| Row { split: LEXICON.to_owned(), script, keys, shapes: vec![("lower".to_owned(), weight)] }));
    output
}

struct Settings {
    value: Value,
    order: usize,
    scale: i64,
    discount: f64,
    collapse: Option<i64>,
}

impl Settings {
    fn list(&self, name: &str) -> Vec<String> {
        self.value[name].as_array().map_or_else(Vec::new, |items| items.iter().filter_map(Value::as_str).map(str::to_owned).collect())
    }
}

/// `char_payload`: the counted weights: character models and case channels, no features, no thresholds.
fn char_payload(rows: &[&Row], settings: &Settings, web: &[Row]) -> Map<String, Value> {
    let (grams, prose, acronym) = counts(rows, settings.order, settings.collapse);
    let channel = |table: &HashMap<String, i64>| channel_payload(table, settings.order, settings.discount, settings.scale);
    let models: Map<String, Value> = SCRIPTS.iter().map(|script| ((*script).to_owned(), channel(&grams[*script]))).collect();
    let mut payload = Map::new();
    payload.insert("schema_version".to_owned(), json!(1));
    payload.insert("order".to_owned(), json!(settings.order));
    payload.insert("scale".to_owned(), json!(settings.scale));
    payload.insert("models".to_owned(), Value::Object(models));
    payload.insert("prose_shape".to_owned(), shape_channel(&prose, settings.scale));
    payload.insert("acronym_shape".to_owned(), shape_channel(&acronym, settings.scale));
    if !web.is_empty() {
        let merged = source_rows(rows, web);
        let references: Vec<&Row> = merged.iter().collect();
        let (sourced, _, _) = counts(&references, settings.order, settings.collapse);
        let scripts: BTreeSet<&str> = web.iter().map(|row| row.script.as_str()).collect();
        payload.insert("source_models".to_owned(), Value::Object(scripts.into_iter().map(|script| (script.to_owned(), channel(&sourced[script]))).collect()));
    }
    payload
}

/// `scope`: the artifact's run collapse, its scope and schema, from the configuration.
fn scope(settings: &Settings) -> Result<Map<String, Value>, String> {
    let value = &settings.value;
    let mut result = Map::new();
    result.insert("collapse_runs".to_owned(), value["collapse_runs"].clone());
    result.insert("unscored_shapes".to_owned(), value["unscored_shapes"].clone());
    let mut schema = EXTRA_KEY_SCHEMA;
    if !value["stretch_runs"].is_null() {
        schema = if value["hyphen_stretch"].as_bool() == Some(true) { REPLACEMENT_SCHEMA } else { STRETCH_SCHEMA };
        result.insert("stretch_runs".to_owned(), value["stretch_runs"].clone());
    }
    if value["target_plausibility"].as_bool() == Some(true) {
        if schema != REPLACEMENT_SCHEMA {
            return Err("the plausibility of the replacement needs the schema 5 readings".to_owned());
        }
        schema = PLAUSIBILITY_SCHEMA;
    } else if value["feature_gated"].as_array().is_some_and(|items| !items.is_empty()) {
        return Err("only a schema 6 artifact gates features".to_owned());
    }
    if value["compound_min_letters"].as_object().is_some_and(|table| !table.is_empty()) {
        if schema < REPLACEMENT_SCHEMA {
            return Err("only a schema 5 artifact requires a length of a compound's half".to_owned());
        }
        result.insert("compound_min_letters".to_owned(), value["compound_min_letters"].clone());
    }
    result.insert("schema_version".to_owned(), json!(schema));
    Ok(result)
}

/// `char_model`: counted weights as a scoring model: its scope, no feature weight, thresholds unused.
fn char_model(payload: &Map<String, Value>, settings: &Settings, scratch: &Path) -> Result<OrthoModel, String> {
    let scoped = scope(settings)?;
    let schema = scoped["schema_version"].as_i64().unwrap_or(0);
    let names = schema_features(schema).ok_or("an unknown artifact schema")?;
    let mut scored = payload.clone();
    scored.extend(scoped);
    let zero_per_script = json!({"en": 0, "ru": 0});
    if schema >= REPLACEMENT_SCHEMA {
        scored.insert("feature_centres".to_owned(), zero_per_script.clone());
    }
    if schema == PLAUSIBILITY_SCHEMA {
        scored.insert("target_centres".to_owned(), zero_per_script.clone());
    }
    let zeros: Map<String, Value> = names.iter().map(|name| ((*name).to_owned(), json!(0))).collect();
    scored.insert("features".to_owned(), json!({"en": zeros, "ru": zeros}));
    scored.insert("version".to_owned(), json!(format!("ortho-v1-{}", "0".repeat(VERSION_HASH_CHARACTERS))));
    scored.insert("minimum_length".to_owned(), settings.value["minimum_length"].clone());
    scored.insert("thresholds".to_owned(), zero_per_script);
    let path = scratch.join("counts.json");
    std::fs::write(&path, canonical(&Value::Object(scored))).map_err(|error| error.to_string())?;
    OrthoModel::load(&path, None, None, None).map_err(|error| error.name.to_string())
}

fn fold_of(row: &Row, folds: u64) -> u64 {
    let core = word_core(&row.keys, &row.script);
    let digest = hex_digest(&format!("{NAMESPACE}:fold:{}:{core}", row.script));
    u64::from_str_radix(digest.get(..CHOICE_HEX_DIGITS).unwrap_or_default(), 16).expect("a digest") % folds
}

/// `solve`: Gaussian elimination with partial pivoting, in a fixed order.
fn solve(matrix: &[Vec<f64>], vector: &[f64]) -> Vec<f64> {
    let size = vector.len();
    let mut rows: Vec<Vec<f64>> = matrix.iter().enumerate().map(|(index, row)| row.iter().copied().chain([vector[index]]).collect()).collect();
    for column in 0..size {
        let mut pivot = column;
        for index in column + 1..size {
            if rows[index][column].abs() > rows[pivot][column].abs() {
                pivot = index;
            }
        }
        rows.swap(column, pivot);
        for index in 0..size {
            if index != column && rows[column][column] != 0.0 {
                let factor = rows[index][column] / rows[column][column];
                for position in column..=size {
                    let change = factor * rows[column][position];
                    rows[index][position] -= change;
                }
            }
        }
    }
    (0..size).map(|index| rows[index][size] / rows[index][index]).collect()
}

fn probability_of(logit: f64) -> f64 {
    let bounded = (-LOGIT_BOUND).max(LOGIT_BOUND.min(logit));
    1.0 / (1.0 + (-bounded).exp())
}

fn dyadic(value: f64) -> (BigInt, i64) {
    if value == 0.0 {
        return (BigInt::from(0), 0);
    }
    let bits = value.to_bits();
    let exponent = ((bits >> 52) & 0x7ff).to::<i64>();
    let fraction = bits & ((1_u64 << 52) - 1);
    let (mantissa, power) = if exponent == 0 { (fraction, -1074) } else { (fraction | (1_u64 << 52), exponent - 1075) };
    (BigInt::from_biguint(if bits >> 63 == 1 { Sign::Minus } else { Sign::Plus }, BigUint::from(mantissa)), power)
}

/// `_float_sqrt_of_frac`: the square root of n/m, correctly rounded.
fn sqrt_of_fraction(numerator: &BigUint, denominator: &BigUint, extra_power: i64) -> f64 {
    const WIDTH: i64 = 109;
    let q = (numerator.bits().to::<i64>() - denominator.bits().to::<i64>() - WIDTH).div_euclid(2);
    let round_to_odd = |n: BigUint, m: BigUint| -> BigUint {
        let a = (&n / &m).sqrt();
        if &a * &a * &m == n { a } else { a | BigUint::from(1_u8) }
    };
    let (value, power) = if q >= 0 {
        (round_to_odd(numerator.clone(), denominator << (2 * q).to::<u64>()), q)
    } else {
        (round_to_odd(numerator << (-2 * q).to::<u64>(), denominator.clone()), q)
    };
    // `numerator / denominator` of two integers: the nearest double, ties to even.
    let width = value.bits().to::<i64>();
    let drop = width - 53;
    let kept: BigUint = &value >> drop.to::<u64>();
    let mut mantissa = kept.iter_u64_digits().next().unwrap_or(0);
    let half = BigUint::from(1_u8) << (drop - 1).to::<u64>();
    let rest = &value - (&kept << drop.to::<u64>());
    if rest > half || (rest == half && mantissa & 1 == 1) {
        mantissa += 1;
    }
    // In two steps: the power alone may be too small for a double while the result is not.
    let exponent = (drop + power + extra_power).to::<i32>();
    mantissa.to::<f64>() * 2_f64.powi(exponent / 2) * 2_f64.powi(exponent - exponent / 2)
}

/// `statistics.pstdev`: the exact variance as a fraction, its square root correctly rounded.
fn pstdev(values: &[f64]) -> f64 {
    let parts: Vec<(BigInt, i64)> = values.iter().map(|value| dyadic(*value)).collect();
    let least = parts.iter().filter(|(mantissa, _)| mantissa.sign() != Sign::NoSign).map(|(_, power)| *power).min().unwrap_or(0);
    let (mut sum, mut squares) = (BigInt::from(0), BigInt::from(0));
    for (mantissa, power) in parts {
        if mantissa.sign() == Sign::NoSign {
            continue;
        }
        let scaled = mantissa << (power - least).to::<u64>();
        squares += &scaled * &scaled;
        sum += scaled;
    }
    let count = BigInt::from(values.len());
    let numerator = &count * &squares - &sum * &sum;
    let (_, numerator) = numerator.into_parts();
    if numerator.bits() == 0 {
        return 0.0;
    }
    let denominator = (&count * &count).into_parts().1;
    // The variance is numerator / count^2 * 2^(2 * least); its square root carries 2^least.
    sqrt_of_fraction(&numerator, &denominator, least)
}

fn fmean(values: &[f64]) -> f64 {
    fsum(values).expect("finite samples") / values.len().to::<f64>()
}

/// `logistic`: coefficients on the raw scale of a logistic regression with an L2 penalty, by Newton steps.
fn logistic(samples: &[Vec<f64>], labels: &[i64], l2: f64, firth: bool, separated: &[usize]) -> Vec<f64> {
    let width = samples[0].len();
    let column_of = |column: usize| -> Vec<f64> { samples.iter().map(|sample| sample[column]).collect() };
    let mean: Vec<f64> = (0..width).map(|column| fmean(&column_of(column))).collect();
    let spread: Vec<f64> = (0..width).map(|column| { let value = pstdev(&column_of(column)); if value == 0.0 { 1.0 } else { value } }).collect();
    let standard: Vec<Vec<f64>> = samples.iter().map(|sample| std::iter::once(1.0).chain((0..width).map(|column| (sample[column] - mean[column]) / spread[column])).collect()).collect();
    let mut weights = vec![0.0; width + 1];
    for _ in 0..LOGISTIC_ITERATIONS {
        let mut gradient: Vec<f64> = weights.iter().enumerate().map(|(index, weight)| if index != 0 { l2 * weight } else { 0.0 }).collect();
        let mut hessian: Vec<Vec<f64>> = (0..=width).map(|row| (0..=width).map(|column| if row == column && row != 0 { l2 } else { 0.0 }).collect()).collect();
        let probabilities: Vec<f64> = standard.iter().map(|row| probability_of(py_sum(weights.iter().zip(row).map(|(w, x)| w * x)))).collect();
        for ((row, label), probability) in standard.iter().zip(labels).zip(&probabilities) {
            let (residual, curvature) = (probability - (*label).to::<f64>(), probability * (1.0 - probability));
            for first in 0..=width {
                gradient[first] += residual * row[first];
                for second in first..=width {
                    hessian[first][second] += curvature * row[first] * row[second];
                }
            }
        }
        for first in 0..=width {
            for second in 0..first {
                hessian[first][second] = hessian[second][first];
            }
        }
        for column in separated {
            let prior = probability_of(weights[column + 1] / spread[*column]);
            gradient[column + 1] += (prior - FIRTH_CENTRE) / spread[*column];
            hessian[column + 1][column + 1] += prior * (1.0 - prior) / (spread[*column] * spread[*column]);
        }
        if firth {
            let inverse: Vec<Vec<f64>> = (0..=width).map(|column| solve(&hessian, &(0..=width).map(|index| if index == column { 1.0 } else { 0.0 }).collect::<Vec<f64>>())).collect();
            for (row, probability) in standard.iter().zip(&probabilities) {
                let spread_row: Vec<f64> = (0..=width).map(|first| py_sum((0..=width).map(|second| inverse[first][second] * row[second]))).collect();
                let leverage = probability * (1.0 - probability) * py_sum(row.iter().zip(&spread_row).map(|(x, y)| x * y));
                for first in 0..=width {
                    gradient[first] += leverage * (probability - FIRTH_CENTRE) * row[first];
                }
            }
        }
        let step = solve(&hessian, &gradient);
        for (weight, change) in weights.iter_mut().zip(&step) {
            *weight -= change;
        }
        if step.iter().map(|change| change.abs()).fold(f64::NEG_INFINITY, f64::max) < LOGISTIC_TOLERANCE {
            break;
        }
    }
    (0..width).map(|column| weights[column + 1] / spread[column]).collect()
}

type Parts = BTreeMap<&'static str, Vec<(f64, f64, i64)>>;

/// `shared_weight`: one weight in nats for a feature both directions share, from both directions' samples at once.
fn shared_weight(parts: &Parts, firth: bool, prior: Option<&BTreeMap<&'static str, f64>>) -> f64 {
    let directions: Vec<&&str> = parts.keys().collect();
    let size = directions.len() + 1;
    let mut estimate = vec![0.0; size];
    for _ in 0..LOGISTIC_ITERATIONS {
        let mut gradient = vec![0.0; size];
        let mut hessian = vec![vec![0.0; size]; size];
        let mut seen: Vec<(Vec<f64>, f64)> = Vec::new();
        for (position, direction) in directions.iter().enumerate() {
            for (predictor, column, label) in &parts[**direction] {
                let row: Vec<f64> = (0..size - 1).map(|index| if index == position { 1.0 } else { 0.0 }).chain([*column]).collect();
                let probability = probability_of(estimate[position] + predictor + estimate[size - 1] * column);
                let (residual, curvature) = (probability - (*label).to::<f64>(), probability * (1.0 - probability));
                for first in 0..size {
                    gradient[first] += residual * row[first];
                    for second in 0..size {
                        hessian[first][second] += curvature * row[first] * row[second];
                    }
                }
                seen.push((row, probability));
            }
        }
        if hessian[size - 1][size - 1] == 0.0 {
            return 0.0;
        }
        if let Some(scales) = prior {
            let share = 1.0 / scales.len().to::<f64>();
            for scale in scales.values() {
                let pseudo = probability_of(estimate[size - 1] * scale);
                gradient[size - 1] += share * (pseudo - FIRTH_CENTRE) * scale;
                hessian[size - 1][size - 1] += share * pseudo * (1.0 - pseudo) * scale * scale;
            }
        }
        if firth {
            let inverse: Vec<Vec<f64>> = (0..size).map(|column| solve(&hessian, &(0..size).map(|index| if index == column { 1.0 } else { 0.0 }).collect::<Vec<f64>>())).collect();
            for (row, probability) in &seen {
                let inner = py_sum((0..size).flat_map(|first| { let (row, inverse) = (&row, &inverse); (0..size).map(move |second| row[first] * inverse[first][second] * row[second]) }));
                let leverage = probability * (1.0 - probability) * inner;
                for first in 0..size {
                    gradient[first] += leverage * (probability - FIRTH_CENTRE) * row[first];
                }
            }
        }
        let step = solve(&hessian, &gradient);
        for (value, change) in estimate.iter_mut().zip(&step) {
            *value -= change;
        }
        if step.iter().map(|change| change.abs()).fold(f64::NEG_INFINITY, f64::max) < LOGISTIC_TOLERANCE {
            break;
        }
    }
    estimate[size - 1]
}

type Ties = BTreeMap<&'static str, BTreeMap<String, String>>;
type Weights = BTreeMap<&'static str, BTreeMap<String, f64>>;

/// `fitted_weights`: each feature's weight in nats per direction from the out-of-fold samples [char score, *names].
fn fitted_weights(samples: &BTreeMap<&'static str, Vec<Vec<f64>>>, labels: &BTreeMap<&'static str, Vec<i64>>, settings: &Settings, names: &[&'static str], ties: &mut Ties) -> Result<Weights, String> {
    let minimum = (settings.value["feature_min_types"].as_i64().unwrap_or(0)).to::<usize>();
    let copied_names = settings.list("feature_copied");
    let always_tied = settings.list("feature_tied");
    let omitted = settings.list("feature_omitted");
    let shared: Vec<&'static str> = settings.list("feature_shared").iter().filter_map(|name| names.iter().copied().find(|known| known == name)).filter(|name| !omitted.iter().any(|left| left == name)).collect();
    let firth = settings.value["feature_prior"].as_str() == Some("firth");
    let separated = &settings.value["feature_separated"];
    let l2 = settings.value["feature_l2"].as_f64().ok_or("feature_l2")?;
    let has = |name: &str| names.contains(&name);
    let index_of = |name: &str| names.iter().position(|known| *known == name).expect("a known feature");
    let mut result: Weights = BTreeMap::new();
    let mut scales: BTreeMap<&'static str, f64> = BTreeMap::new();
    let mut parts: BTreeMap<&'static str, Parts> = shared.iter().map(|name| (*name, Parts::new())).collect();
    for direction in SCRIPTS {
        let rows = &samples[direction];
        let in_shared = |name: &str| shared.contains(&name);
        let copied: BTreeMap<&str, &str> = FEATURE_PARENTS.iter().filter(|(name, parent)| copied_names.iter().any(|known| known == name) && has(name) && has(parent) && !in_shared(name)).map(|(name, parent)| (*name, *parent)).collect();
        let tied: Vec<(&str, &str)> = FEATURE_PARENTS.iter().filter(|(rare, parent)| has(rare) && has(parent) && !copied.contains_key(rare) && !in_shared(rare) && (always_tied.iter().any(|known| known == rare) || rows.iter().filter(|row| row[1 + index_of(rare)] != 0.0).count() < minimum)).map(|(rare, parent)| (*rare, *parent)).collect();
        let kept: Vec<&str> = names.iter().copied().filter(|name| !tied.iter().any(|(rare, _)| rare == name) && !copied.contains_key(name) && !in_shared(name) && !omitted.iter().any(|left| left == name)).collect();
        let columns: Vec<Vec<usize>> = kept.iter().map(|name| std::iter::once(index_of(name)).chain(tied.iter().filter(|(_, parent)| parent == name).map(|(rare, _)| index_of(rare))).collect()).collect();
        let matrix: Vec<Vec<f64>> = rows.iter().map(|row| std::iter::once(row[0]).chain(columns.iter().map(|column| py_sum(column.iter().map(|index| row[1 + index])))).collect()).collect();
        let listed: Vec<String> = separated[direction].as_array().map_or_else(Vec::new, |items| items.iter().filter_map(Value::as_str).map(str::to_owned).collect()).into_iter().filter(|name: &String| !in_shared(name)).collect();
        if listed.iter().any(|name| !kept.contains(&name.as_str())) {
            return Err("a separated column must be a feature fitted on its own column".to_owned());
        }
        let positions: Vec<usize> = listed.iter().map(|name| 1 + kept.iter().position(|known| known == name).expect("kept")).collect();
        let coefficients = logistic(&matrix, &labels[direction], l2, firth, &positions);
        scales.insert(direction, coefficients[0]);
        if coefficients[0] <= 0.0 {
            return Err("the character score does not predict a wrong layout; refusing the feature fit".to_owned());
        }
        let mut fitted: BTreeMap<String, f64> = kept.iter().enumerate().map(|(index, name)| ((*name).to_owned(), coefficients[index + 1] / coefficients[0])).collect();
        for (rare, parent) in tied.iter().copied().chain(copied.iter().map(|(name, parent)| (*name, *parent))) {
            let weight = fitted[parent];
            fitted.insert(rare.to_owned(), weight);
        }
        result.insert(direction, FEATURES.iter().map(|name| ((*name).to_owned(), fitted.get(*name).copied().unwrap_or(0.0))).collect());
        for name in &shared {
            let list: Vec<(f64, f64, i64)> = matrix.iter().zip(rows).zip(&labels[direction]).map(|((entry, row), label)| (py_sum(coefficients.iter().zip(entry).map(|(coefficient, value)| coefficient * value)), coefficients[0] * row[1 + index_of(name)], *label)).collect();
            parts.get_mut(name).expect("a shared feature").insert(direction, list);
        }
        let mut table: BTreeMap<String, String> = tied.iter().map(|(rare, parent)| ((*rare).to_owned(), (*parent).to_owned())).collect();
        table.extend(copied.iter().map(|(name, parent)| ((*name).to_owned(), (*parent).to_owned())));
        table.extend(shared.iter().map(|name| ((*name).to_owned(), SHARED_TIE.to_owned())));
        ties.insert(direction, table);
    }
    for name in &shared {
        let listed_in = SCRIPTS.iter().filter(|direction| separated[**direction].as_array().is_some_and(|items| items.iter().any(|item| item.as_str() == Some(name)))).count();
        if listed_in != 0 && listed_in != SCRIPTS.len() {
            return Err("a shared feature is separated in both directions or in none".to_owned());
        }
        let weight = shared_weight(&parts[name], firth, (listed_in != 0).then_some(&scales));
        for direction in SCRIPTS {
            result.get_mut(direction).expect("a direction").insert((*name).to_owned(), weight);
        }
    }
    Ok(result)
}

/// `population`: (direction, served keys, shape) -> is it a negative, for what the engine serves the model.
pub(crate) fn population(rows: &[&Row], split: &str, known: &HashMap<String, HashSet<String>>, minimum: usize) -> BTreeMap<Label, bool> {
    let mut labels = BTreeMap::new();
    for row in rows {
        if row.split != split {
            continue;
        }
        let mut shapes: Vec<&str> = row.shapes.iter().map(|shape| shape.0.as_str()).collect();
        shapes.sort_unstable();
        for direction in SCRIPTS {
            for token in served(&row.keys, direction) {
                if token.chars().count() < minimum || !token.chars().all(|key| KEYS.contains(&key)) {
                    continue;
                }
                let original = visible(&token, direction);
                if !word_shaped(&original) || known[direction].contains(&original) {
                    continue;
                }
                let replaced = visible(&token, other(direction));
                let replacement = replaced.strip_prefix('.').unwrap_or(&replaced);
                if pystr::isalpha(&original) && !(pystr::isalpha(replacement) || word_shaped(replacement)) {
                    continue;
                }
                let negative = row.script == direction;
                for shape in &shapes {
                    let entry = labels.entry((direction.to_owned(), token.clone(), (*shape).to_owned())).or_insert(false);
                    *entry = *entry || negative;
                }
            }
        }
    }
    labels
}

/// `synthetic_population`: positive types for the feature fit only: train words typed in the other layout with a stray key.
fn synthetic_population(rows: &[&Row], known: &HashMap<String, HashSet<String>>, minimum: usize) -> BTreeMap<Label, bool> {
    let mut labels = BTreeMap::new();
    for row in rows {
        let Some(synthetic) = extra_key_variant(&row.script, &row.keys) else { continue };
        let typed = Row { keys: synthetic, ..(*row).clone() };
        for (label, _) in population(&[&typed], TRAIN, known, minimum) {
            if label.0 != row.script {
                labels.insert(label, false);
            }
        }
    }
    labels
}

struct Inputs {
    rows: Vec<Row>,
    known: HashMap<String, HashSet<String>>,
    identifier: Arc<IdentifierLexicon>,
    web: Vec<Row>,
    settings: Settings,
}

/// `feature_weights`: each feature's weight in nats per direction, fitted out of fold on train; the centres are measured here.
fn feature_weights(inputs: &Inputs, names: &[&'static str], ties: &mut Ties, centres: &mut BTreeMap<&'static str, f64>, target_centres: &mut BTreeMap<&'static str, f64>, scratch: &Path) -> Result<Weights, String> {
    let settings = &inputs.settings;
    let folds = settings.value["feature_folds"].as_u64().ok_or("feature_folds")?;
    let minimum = (settings.value["minimum_length"].as_u64().ok_or("minimum_length")?).to::<usize>();
    let mut samples: BTreeMap<&'static str, Vec<Vec<f64>>> = SCRIPTS.iter().map(|script| (*script, Vec::new())).collect();
    let mut labels: BTreeMap<&'static str, Vec<i64>> = SCRIPTS.iter().map(|script| (*script, Vec::new())).collect();
    let train: Vec<&Row> = inputs.rows.iter().filter(|row| row.split == TRAIN).collect();
    let lexicon: Vec<&Row> = inputs.rows.iter().filter(|row| row.split == LEXICON).collect();
    let known = |script: &str, word: &str| inputs.known[script].contains(word);
    let identifier = |token: &str| inputs.identifier.contains(token);
    let folds_of: Vec<u64> = train.iter().map(|row| fold_of(row, folds)).collect();
    for fold in 0..folds {
        let counted: Vec<&Row> = train.iter().zip(&folds_of).filter(|(_, number)| **number != fold).map(|(row, _)| *row).chain(lexicon.iter().copied()).collect();
        let model = char_model(&char_payload(&counted, settings, &inputs.web), settings, scratch)?;
        let held: Vec<&Row> = train.iter().zip(&folds_of).filter(|(_, number)| **number == fold).map(|(row, _)| *row).collect();
        let real = population(&held, TRAIN, &inputs.known, minimum);
        let synthetic = synthetic_population(&held, &inputs.known, minimum);
        let synthetic = synthetic.iter().filter(|(label, _)| !real.contains_key(*label));
        for (label, negative) in real.iter().chain(synthetic) {
            let (direction, keys, shape) = label;
            let scored = model.score(&OrthoEvidence { keys, shape, source_script: direction }, None);
            if !scored.supported {
                continue;
            }
            let features = token_features(&scored.reading, direction, &identifier, &known, shape, model.hyphens, &model.compound_min_letters);
            let mut values: HashMap<&str, f64> = features.as_pairs().iter().map(|(name, value)| (*name, (*value).to::<f64>())).collect();
            let length = scored.reading.chars().count().to::<f64>() + 1.0;
            values.insert(CONTINUOUS_FEATURE, scored.source_logprob / length);
            values.insert(TARGET_FEATURE, scored.target_logprob / length);
            let script: &'static str = if direction == "en" { "en" } else { "ru" };
            samples.get_mut(script).expect("a script").push(std::iter::once(scored.total).chain(names.iter().map(|name| values[name])).collect());
            labels.get_mut(script).expect("a script").push(i64::from(!*negative));
        }
    }
    let gated = settings.list("feature_gated");
    let index_of = |name: &str| names.iter().position(|known| *known == name);
    if let Some(position) = index_of(CONTINUOUS_FEATURE) {
        for (direction, drawn) in &samples {
            centres.insert(direction, fmean(&drawn.iter().map(|sample| sample[1 + position]).collect::<Vec<f64>>()));
        }
        for name in &gated {
            let gate = 1 + index_of(name).ok_or("a gated feature")?;
            for (direction, drawn) in &mut samples {
                for sample in drawn.iter_mut() {
                    if sample[1 + position] >= centres[direction] {
                        sample[gate] = 0.0;
                    }
                }
            }
        }
    }
    if let Some(position) = index_of(TARGET_FEATURE) {
        for (direction, drawn) in &samples {
            target_centres.insert(direction, fmean(&drawn.iter().map(|sample| sample[1 + position]).collect::<Vec<f64>>()));
        }
    }
    fitted_weights(&samples, &labels, settings, names, ties)
}

/// `build`: the complete artifact: counted weights, feature weights, then the fixed thresholds.
fn build(inputs: &Inputs, ties: &mut Ties, scratch: &Path) -> Result<Value, String> {
    let settings = &inputs.settings;
    let references: Vec<&Row> = inputs.rows.iter().collect();
    let mut payload = char_payload(&references, settings, &inputs.web);
    payload.extend(scope(settings)?);
    let schema = payload["schema_version"].as_i64().unwrap_or(0);
    let names = schema_features(schema).ok_or("an unknown artifact schema")?;
    let (mut centres, mut target_centres) = (BTreeMap::new(), BTreeMap::new());
    let weights = feature_weights(inputs, names, ties, &mut centres, &mut target_centres, scratch)?;
    let features: Map<String, Value> = weights.iter().map(|(direction, table)| ((*direction).to_owned(), Value::Object(names.iter().map(|name| ((*name).to_owned(), json!(quantise(table[*name], settings.scale)))).collect()))).collect();
    payload.insert("features".to_owned(), Value::Object(features));
    let per_script = |table: &BTreeMap<&'static str, f64>| -> Value { Value::Object(SCRIPTS.iter().map(|script| ((*script).to_owned(), json!(quantise(table[script], settings.scale)))).collect()) };
    if schema >= REPLACEMENT_SCHEMA {
        payload.insert("feature_centres".to_owned(), per_script(&centres));
    }
    if schema == PLAUSIBILITY_SCHEMA {
        payload.insert("target_centres".to_owned(), per_script(&target_centres));
        let mut gated = settings.list("feature_gated");
        if !gated.is_empty() {
            gated.sort();
            payload.insert("gated_features".to_owned(), json!(gated));
        }
    }
    let digest = hex(&Sha256::digest(canonical(&Value::Object(payload.clone()))));
    payload.insert("version".to_owned(), json!(format!("ortho-v1-{}", digest.get(..VERSION_HASH_CHARACTERS).unwrap_or_default())));
    payload.insert("weights_sha256".to_owned(), json!(digest));
    payload.insert("minimum_length".to_owned(), settings.value["minimum_length"].clone());
    let thresholds = &settings.value["thresholds_nats"];
    payload.insert("thresholds".to_owned(), Value::Object(SCRIPTS.iter().map(|script| ((*script).to_owned(), json!(quantise(thresholds[*script].as_f64().unwrap_or(0.0), settings.scale)))).collect()));
    Ok(Value::Object(payload))
}

pub(crate) fn gz_text(path: &Path) -> Result<String, String> {
    let mut text = String::new();
    MultiGzDecoder::new(std::fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?).read_to_string(&mut text).map_err(|error| error.to_string())?;
    Ok(text)
}

/// `web_weight`: an Onboard-like weight from a rank: the bit length of the count per billion words (Zipf).
fn web_weight(rank: i64) -> i64 {
    let per_billion = (py_round(WEB_TOP_PER_BILLION.to::<f64>() / (rank + 1).to::<f64>())).to::<i64>();
    (per_billion.cast_unsigned().bit_width().min(MAXIMUM_LEXICON_WEIGHT.to::<u32>()).max(1)).to::<i64>()
}

/// `to_keys`: the physical keys a typist pressed to produce this token.
pub(crate) fn to_keys(token: &str) -> Result<String, String> {
    let lowered = pystr::casefold(&ks_text::normalize::nfc(token));
    let cyrillic = lowered.chars().any(|character| "абвгдеёжзийклмнопрстуфхцчшщъыьэюя".contains(character));
    let latin = lowered.chars().any(|character| character.is_ascii_lowercase());
    if !cyrillic || latin {
        return Ok(lowered);
    }
    let mut keys = String::new();
    for character in lowered.chars() {
        if character == '.' || character == ',' {
            keys.push('/');
        } else {
            keys.push_str(&pair().translate(&character.to_string(), "ru", "us").map_err(|error| format!("{error:?}"))?);
        }
    }
    Ok(keys)
}

/// `web_rows`: word-list rows of the frozen web forms and of the frozen public-domain prose forms.
fn web_rows(root: &Path, settings: &Settings) -> Result<Vec<Row>, String> {
    let directory = root.join(DIRECTORY);
    let mut rows = Vec::new();
    for (key, forms, receipt, name) in [("web_source_scripts", "web-lexicon-ru.json.gz", "web-lexicon-receipt.json", "web"), ("prose_source_scripts", "prose-lexicon-ru.json.gz", "prose-lexicon-receipt.json", "prose")] {
        let scripts = settings.list(key);
        if scripts.is_empty() {
            continue;
        }
        if scripts != ["ru"] {
            return Err(format!("the {name} lexicon holds Russian forms only"));
        }
        if read_json(&directory.join(receipt))?["forms_sha256"].as_str() != Some(checksum(&directory.join(forms))?.as_str()) {
            return Err(format!("{name} lexicon does not match its receipt"));
        }
        let payload: Value = serde_json::from_str(&gz_text(&directory.join(forms))?).map_err(|error| error.to_string())?;
        // Web forms are listed with their rank in the source, prose forms with their weight.
        let table: BTreeMap<String, i64> = match &payload["forms"] {
            Value::Array(items) => items.iter().filter_map(|item| Some((item[0].as_str()?.to_owned(), web_weight(item[1].as_i64()?)))).collect(),
            Value::Object(items) => items.iter().filter_map(|(form, weight)| Some((form.clone(), weight.as_i64()?))).collect(),
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => return Err("a lexicon without forms".to_owned()),
        };
        for (form, weight) in table {
            rows.push(Row { split: LEXICON.to_owned(), script: "ru".to_owned(), keys: to_keys(&form)?, shapes: vec![("lower".to_owned(), weight)] });
        }
    }
    Ok(rows)
}

fn load_inputs(root: &Path) -> Result<Inputs, String> {
    let directory = root.join(DIRECTORY);
    let value = read_json(&directory.join("config.json"))?;
    let settings = Settings { order: (value["order"].as_u64().ok_or("order")?).to::<usize>(), scale: value["scale"].as_i64().ok_or("scale")?, discount: value["discount"].as_f64().ok_or("discount")?, collapse: value["collapse_runs"].as_i64(), value };
    if read_json(&directory.join("corpus-receipt.json"))?["tokens_sha256"].as_str() != Some(checksum(&directory.join("tokens.jsonl.gz"))?.as_str()) {
        return Err("key-space corpus does not match its receipt".to_owned());
    }
    if read_json(&directory.join("known-receipt.json"))?["evidence_sha256"].as_str() != Some(checksum(&directory.join("known-evidence.json.gz"))?.as_str()) {
        return Err("dictionary evidence does not match its receipt".to_owned());
    }
    let rows = read_rows(&gz_text(&directory.join("tokens.jsonl.gz"))?)?;
    let evidence: Value = serde_json::from_str(&gz_text(&directory.join("known-evidence.json.gz"))?).map_err(|error| error.to_string())?;
    let known = evidence.as_object().ok_or("dictionary evidence")?.iter().map(|(script, words)| (script.clone(), words.as_array().map_or_else(HashSet::new, |words| words.iter().filter_map(Value::as_str).map(str::to_owned).collect()))).collect();
    let identifier = Arc::new(IdentifierLexicon::load(&root.join("resources/identifiers.json")).map_err(|error| error.name.to_string())?);
    let web = web_rows(root, &settings)?;
    Ok(Inputs { rows, known, identifier, web, settings })
}

/// The rows of a token file.
pub(crate) fn read_rows(text: &str) -> Result<Vec<Row>, String> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let row: Value = serde_json::from_str(line).map_err(|error| error.to_string())?;
        let field = |name: &str| row[name].as_str().map(str::to_owned).ok_or_else(|| format!("a token row without {name}"));
        let shapes = row["shapes"].as_object().ok_or("a token row without shapes")?.iter().map(|(name, value)| (name.clone(), value.as_i64().unwrap_or(0))).collect();
        rows.push(Row { split: field("split")?, script: field("script")?, keys: field("keys")?, shapes });
    }
    Ok(rows)
}

/// The installed model on an independent set, with the verdict of that set's own dictionary evidence.
pub(crate) fn independent_outcome(root: &Path, labels: &BTreeMap<Label, bool>, known: &HashMap<String, HashSet<String>>) -> Result<Value, String> {
    let lexicon = Arc::new(IdentifierLexicon::load(&root.join("resources/identifiers.json")).map_err(|error| error.name.to_string())?);
    let identifier: Identifier = Arc::new(move |token: &str| lexicon.contains(token));
    let table = known.clone();
    let word: KnownWord = Arc::new(move |script: &str, text: &str| table[script].contains(text));
    let model = OrthoModel::load(&root.join(DIRECTORY).join("candidate.json"), Some(identifier), Some(word), None).map_err(|error| error.name.to_string())?;
    Ok(outcome(&model, labels))
}

/// `outcome`: false conversions and recall, counted by type.
fn outcome(model: &OrthoModel, labels: &BTreeMap<Label, bool>) -> Value {
    let mut counted: BTreeMap<String, i64> = BTreeMap::new();
    let mut worst: BTreeMap<&str, Vec<(f64, &str, &str)>> = SCRIPTS.iter().map(|script| (*script, Vec::new())).collect();
    for ((direction, keys, shape), negative) in labels {
        let scored = model.score(&OrthoEvidence { keys, shape, source_script: direction }, None);
        let converts = scored.supported && scored.total > *model.thresholds.get(direction);
        let mut add = |name: &str| *counted.entry(format!("{direction}:{name}")).or_insert(0) += 1;
        if *negative {
            add("negative_types");
            if converts {
                add("false_types");
                worst.get_mut(direction.as_str()).expect("a script").push((scored.total, keys, shape));
            }
        } else {
            add("positive_types");
            if converts {
                add("recalled_types");
            }
        }
    }
    let shown: Map<String, Value> = worst.into_iter().map(|(script, mut items)| {
        items.sort_by(|a, b| b.partial_cmp(a).expect("finite scores"));
        (script.to_owned(), Value::Array(items.into_iter().take(REPORTED_WORST_FALSE).map(|(value, keys, shape)| json!({"score": py_round_ndigits(value, REPORTED_SCORE_DECIMALS), "keys": keys, "shape": shape})).collect()))
    }).collect();
    json!({"counts": counted, "worst_false": shown})
}

fn hash_of(path: &Path) -> Result<Value, String> {
    Ok(Value::String(checksum(path)?))
}

/// What a seal of this program pins: the code that counts, the frozen data, the configuration.
fn provenance(root: &Path) -> Result<Value, String> {
    let names = ["crates/ks-train/src/ortho.rs", "crates/ks-lang/src/ortho_model.rs", "crates/ks-lang/src/constants.rs", "model/ortho_v1/tokens.jsonl.gz", "model/ortho_v1/corpus-receipt.json", "model/ortho_v1/known-evidence.json.gz", "model/ortho_v1/known-receipt.json", "model/ortho_v1/web-lexicon-ru.json.gz", "model/ortho_v1/web-lexicon-receipt.json", "model/ortho_v1/prose-lexicon-ru.json.gz", "model/ortho_v1/prose-lexicon-receipt.json", "model/ortho_v1/config.json", "resources/identifiers.json"];
    names.iter().map(|name| Ok(((*name).to_owned(), hash_of(&root.join(name))?))).collect::<Result<Map<String, Value>, String>>().map(Value::Object)
}

fn serving_model(path: &Path, inputs: &Inputs) -> Result<OrthoModel, String> {
    let lexicon = inputs.identifier.clone();
    let identifier: Identifier = Arc::new(move |token: &str| lexicon.contains(token));
    let table = inputs.known.clone();
    let known: KnownWord = Arc::new(move |script: &str, word: &str| table[script].contains(word));
    OrthoModel::load(path, Some(identifier), Some(known), None).map_err(|error| error.name.to_string())
}

/// What `command` does: `Fit` into a folder, `Verify` the artifact byte for byte and the recorded test, `Test`, `Promote`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Fit,
    Verify,
    Test,
    Promote,
}

/// Runs one `Action` of the orthotactic model.
pub fn command(root: &Path, action: Action, output: Option<&Path>) -> Result<(), String> {
    let inputs = load_inputs(root)?;
    let directory: PathBuf = output.map_or_else(|| root.join(DIRECTORY), Path::to_path_buf);
    let (candidate, seal_path, report_path) = (directory.join("candidate.json"), directory.join("seal.json"), directory.join("report.json"));
    let scratch = std::env::temp_dir().join(format!("keyswitch-ortho-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).map_err(|error| error.to_string())?;
    let mut ties = Ties::new();
    let result = (|| -> Result<(), String> {
        match action {
            Action::Fit => {
                if candidate.exists() || seal_path.exists() {
                    return Err("candidate already exists; do not refit over sealed evidence".to_owned());
                }
                std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
                let payload = build(&inputs, &mut ties, &scratch)?;
                std::fs::write(&candidate, canonical(&payload)).map_err(|error| error.to_string())?;
                let model = serving_model(&candidate, &inputs)?;
                let thresholds: Map<String, Value> = SCRIPTS.iter().map(|script| ((*script).to_owned(), json!(py_round_ndigits(*model.thresholds.get(script), SEALED_DECIMALS)))).collect();
                let per_script = model.features.as_ref().ok_or("an artifact without features")?;
                let features: Map<String, Value> = SCRIPTS.iter().map(|script| ((*script).to_owned(), Value::Object(per_script.get(script).iter().map(|(name, weight)| ((*name).to_owned(), json!(py_round_ndigits(*weight, SEALED_DECIMALS)))).collect()))).collect();
                let seal = json!({"schema_version": EVIDENCE_SCHEMA, "stage": "sealed-before-test", "namespace": NAMESPACE, "model_version": payload["version"], "candidate_sha256": checksum(&candidate)?, "thresholds": thresholds, "feature_weights_nats": features, "feature_ties": ties, "config": inputs.settings.value, "provenance": provenance(root)?});
                std::fs::write(&seal_path, canonical(&seal)).map_err(|error| error.to_string())?;
                println!("{}", payload["version"].as_str().unwrap_or(""));
            }
            Action::Verify => {
                let payload = build(&inputs, &mut ties, &scratch)?;
                if canonical(&payload) != std::fs::read(&candidate).map_err(|error| error.to_string())? {
                    return Err("orthotactic candidate is not reproducible".to_owned());
                }
                println!("{} reproduces", payload["version"].as_str().unwrap_or(""));
            }
            Action::Test => {
                if report_path.exists() {
                    return Err("test already observed; re-scoring it would not be an independent test".to_owned());
                }
                let seal = read_json(&seal_path)?;
                if seal["candidate_sha256"].as_str() != Some(checksum(&candidate)?.as_str()) || seal["provenance"] != provenance(root)? {
                    return Err("candidate or provenance changed after the seal".to_owned());
                }
                let references: Vec<&Row> = inputs.rows.iter().collect();
                let minimum = (inputs.settings.value["minimum_length"].as_u64().ok_or("minimum_length")?).to::<usize>();
                let model = serving_model(&candidate, &inputs)?;
                let corpus = outcome(&model, &population(&references, TEST, &inputs.known, minimum));
                let summed = |name: &str| SCRIPTS.iter().map(|script| corpus["counts"][format!("{script}:{name}")].as_i64().unwrap_or(0)).sum::<i64>();
                let recall = summed("recalled_types").to::<f64>() / summed("positive_types").max(1).to::<f64>();
                let promotion = &inputs.settings.value["promotion"];
                let passed = summed("negative_types") >= promotion["minimum_test_negatives"].as_i64().unwrap_or(0) && summed("false_types") <= promotion["maximum_test_false_conversions"].as_i64().unwrap_or(0) && recall >= promotion["minimum_recall"].as_f64().unwrap_or(1.0);
                let report = json!({"schema_version": EVIDENCE_SCHEMA, "model_version": seal["model_version"], "seal_sha256": checksum(&seal_path)?, "candidate_sha256": checksum(&candidate)?, "thresholds": seal["thresholds"], "corpus_test": corpus, "recall": py_round_ndigits(recall, RECALL_DECIMALS), "promotion_passed": passed,
                    "evidence_scope": "synthetic wrong-layout renderings of frozen CC0 corpus tokens, cut as the engine cuts them and limited to what it serves the model: keys of the us/ru pair, word-shaped, unknown to the layout's dictionary, with a word as the replacement; one type is (direction, keys, case shape)"});
                std::fs::write(&report_path, canonical(&report)).map_err(|error| error.to_string())?;
                println!("promotion passed: {passed}");
            }
            Action::Promote => {
                let report = read_json(&report_path)?;
                if report["promotion_passed"] != json!(true) || report["candidate_sha256"].as_str() != Some(checksum(&candidate)?.as_str()) {
                    return Err("refusing to promote a candidate without a passing report".to_owned());
                }
                std::fs::copy(&candidate, root.join("resources/models/ortho_v1.json")).map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn a_word_ends_where_the_engine_ends_it() {
        assert!(ends_a_word(" "));
        assert!(ends_a_word("!"));
        assert!(!ends_a_word("a"));
    }

    #[test]
    fn a_matrix_with_an_empty_column_is_solved_without_dividing_by_it() {
        let answer = solve(&[vec![0.0, 0.0], vec![0.0, 2.0]], &[0.0, 4.0]);
        assert!(answer[0].is_nan());
        assert!((answer[1] - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_spread_is_the_exact_one_rounded_to_the_nearest_even() {
        // Each value checked against Python's statistics.pstdev.
        let exact = |values: &[f64], wanted: f64| assert_eq!(pstdev(values).to_bits(), wanted.to_bits(), "{values:?}");
        exact(&[2.0, 2.0], 0.0);
        exact(&[0.0, 0.0], 0.0);
        exact(&[1.0, 3.0], 1.0);
        exact(&[f64::from_bits(1), f64::from_bits(3)], f64::from_bits(1));
        exact(&[0.0, 1.0, 2.0], 0.816_496_580_927_726);
        // Halfway between two doubles: to the even one, down and then up.
        exact(&[-9_007_199_254_740_992.0, 9_007_199_254_740_994.0], 9_007_199_254_740_992.0);
        exact(&[-9_007_199_254_740_992.0, 9_007_199_254_740_998.0], 9_007_199_254_740_996.0);
        assert_eq!(dyadic(-1.0).0, BigInt::from(-4_503_599_627_370_496_i64));
    }

    #[test]
    fn a_column_that_never_changes_and_the_firth_prior_leave_the_fit_on_the_score() {
        let samples: Vec<Vec<f64>> = [(1.0, 5.0, 0_i64), (2.0, 5.0, 0), (3.0, 5.0, 1), (4.0, 5.0, 0), (5.0, 5.0, 1), (6.0, 5.0, 1)].iter().map(|row| vec![row.0, row.1]).collect();
        let labels = [0, 0, 1, 0, 1, 1];
        let plain = logistic(&samples, &labels, 0.1, false, &[]);
        let firth = logistic(&samples, &labels, 0.1, true, &[]);
        assert!(plain[0] > 0.0 && firth[0] > 0.0 && (plain[0] - firth[0]).abs() > 0.0);
    }

    #[test]
    fn a_shared_weight_with_nothing_seen_is_nothing_and_firth_moves_the_rest() {
        let nothing: Parts = BTreeMap::new();
        assert_eq!(shared_weight(&nothing, false, None).to_bits(), 0.0_f64.to_bits());
        let parts: Parts = [("en", vec![(-1.0, 1.0, 0_i64), (1.0, 1.0, 1), (0.5, 1.0, 0), (-0.5, 0.0, 1)])].into_iter().collect();
        assert_ne!(shared_weight(&parts, true, None).to_bits(), shared_weight(&parts, false, None).to_bits());
    }

    fn settings_of(value: Value) -> Settings {
        Settings { value, order: 5, scale: 512, discount: 0.75, collapse: None }
    }

    #[test]
    fn a_score_that_points_the_wrong_way_is_refused_and_copied_features_follow_their_parent() {
        let settings = |extra: &[(&str, Value)]| {
            let mut value = json!({"feature_l2": 0.1, "feature_min_types": 0, "feature_shared": [], "feature_tied": [], "feature_omitted": [], "feature_copied": [], "feature_separated": {}});
            for (name, item) in extra {
                value[*name] = item.clone();
            }
            settings_of(value)
        };
        let row = |score: f64, parent: f64, rare: f64| vec![score, parent, rare];
        let upward: Vec<Vec<f64>> = vec![row(1.0, 0.0, 0.0), row(2.0, 1.0, 0.0), row(3.0, 0.0, 1.0), row(4.0, 1.0, 0.0), row(5.0, 0.0, 1.0), row(6.0, 1.0, 1.0)];
        let downward: Vec<Vec<f64>> = upward.iter().rev().cloned().collect();
        let labels: BTreeMap<&'static str, Vec<i64>> = SCRIPTS.iter().map(|script| (*script, vec![0, 0, 1, 0, 1, 1])).collect();
        let samples_of = |rows: &[Vec<f64>]| -> BTreeMap<&'static str, Vec<Vec<f64>>> { SCRIPTS.iter().map(|script| (*script, rows.to_vec())).collect() };
        let (rare, parent) = FEATURE_PARENTS[0];
        let names = [parent, rare];
        let mut ties = Ties::new();
        let refused = fitted_weights(&samples_of(&downward), &labels, &settings(&[]), &names, &mut ties).unwrap_err();
        assert!(refused.contains("does not predict"), "{refused}");
        let copied = fitted_weights(&samples_of(&upward), &labels, &settings(&[("feature_copied", json!([rare]))]), &names, &mut ties).expect("a fit");
        assert_eq!(copied["en"][rare].to_bits(), copied["en"][parent].to_bits());
        // A feature named copied whose parent is not in the model stands on its own.
        let alone = fitted_weights(&samples_of(&upward.iter().map(|sample| vec![sample[0], sample[2]]).collect::<Vec<_>>()), &labels, &settings(&[("feature_copied", json!([rare]))]), &[rare], &mut ties).expect("a fit");
        assert!(alone["en"].contains_key(rare));
        let other: Vec<&'static str> = FEATURE_PARENTS.iter().map(|pair| pair.1).filter(|name| *name != rare).take(1).collect();
        let without = fitted_weights(&samples_of(&upward.iter().map(|sample| vec![sample[0], sample[1]]).collect::<Vec<_>>()), &labels, &settings(&[("feature_copied", json!([rare]))]), &other, &mut ties).expect("a fit");
        assert!(without["en"].get(rare).is_none_or(|weight| weight.to_bits() == 0.0_f64.to_bits()));
    }
}
