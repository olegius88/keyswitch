//! Key-space orthotactics v2, rejected: every candidate built on it was unsafe or no better than the installed model. A
//! replay rebuilds the key corpus, the frozen dictionary verdicts, the trusted labels, the candidate, its seal and the
//! one-shot test report and compares all of them with the frozen files; none of it can install anything.
use ks_cast::To as _;
use super::read_object;
use super::sources::ORTHO_V2;
use crate::artifact::canonical;
use crate::corpus::{checksum, read_json};
use crate::ortho::{Counts, Row, channel_payload, empty_counts, gz_text, other, quantise, read_rows, served, shape_channel, to_keys, visible, word_core};
use crate::ortho_inputs::{assign, load_phrases, read_lexicon, script_of, shape_of};
use crate::physical::{hex, hex_digest};
use ks_context::context_policy::word_shaped;
use ks_lang::ortho_model::{BOS, EOS, SCRIPTS};
use ks_lang::{LanguageModel, py_sum};
use ks_text::pynum::py_round_ndigits;
use ks_text::{normalize, pystr, ucd};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

const DIRECTORY: &str = "model/ortho_v2";
const BASELINE: &str = "model/ortho_v2/compatibility/generation-sources/model/ortho_v1/candidate.json";
const BASELINE_CORPUS: &str = "model/ortho_v2/compatibility/generation-sources/model/ortho_v1/tokens.jsonl.gz";
const NAMESPACE: &str = "keyswitch:ortho-v2:candidate1";
const CORPUS_NAMESPACE: &str = "keyswitch:ortho-v2:key-space-20260910b";
const GATE_NAMESPACE: &str = "keyswitch:ortho-v2:gate-20260910b";
const SEALED_BEFORE_TEST: &str = "sealed-before-test";
const GATE_SHARE_PERCENT: u64 = 16;
const PERCENT_SCALE: u64 = 100;
const CHOICE_DIGITS: usize = 8;
const MAXIMUM_LEXICON_WEIGHT: i128 = 32;
const MAXIMUM_TOKEN_CHARACTERS: usize = 32;
const VERIFIED_MIN_KNOWN_SHARE: f64 = 0.75;
const VERIFIED_MIN_TOKENS: usize = 5;
const FALSE_RATE_DECIMALS: i64 = 4;
const FALSE_RATE_SCALE: i64 = 10_000;
const RECALL_DECIMALS: i64 = 6;
const REPORTED_SCORE_DECIMALS: i64 = 4;
const REPORTED_WORST_FALSE: usize = 10;
const SEALED_THRESHOLD_DECIMALS: i64 = 6;
const VERSION_HASH_CHARACTERS: usize = 12;
const LEXICONS: [(&str, &str, &str); 2] = [
    ("en", "model/intent_v1/sources/en_US.lm", "017a513a23bb37947a3a0e73e307066fa8ec0642075172cdcad6ea204cae68da"),
    ("ru", "model/intent_v1/sources/ru_RU.lm", "e57c14eec2b78e52a2125dce2b38044d8dfe480f3964b9d118614527195ceda9"),
];
const CORPUS_PROVENANCE: [&str; 7] = [
    "tools/ortho_v2_corpus.py",
    "tools/context_corpus.py",
    "src/keyswitch/layouts.py",
    "model/context_v2/sources/tatoeba-cc0-en-ru.tsv.gz",
    "model/context_v2/corpus-receipt.json",
    "model/intent_v1/sources/en_US.lm",
    "model/intent_v1/sources/ru_RU.lm",
];
const SEAL_PROVENANCE: [&str; 12] = [
    "tools/train_ortho_v2.py",
    "tools/ortho_v2_corpus.py",
    "src/keyswitch/ortho_model.py",
    "model/ortho_v2/tokens.jsonl.gz",
    "model/ortho_v2/corpus-receipt.json",
    "model/ortho_v2/config.json",
    "model/ortho_v1/candidate.json",
    "model/ortho_v1/tokens.jsonl.gz",
    "model/ortho_v2/sources/known-evidence.json.gz",
    "model/ortho_v2/sources/known-receipt.json",
    "model/ortho_v2/sources/verified-labels.json.gz",
    "model/ortho_v2/sources/verified-receipt.json",
];
const ANCHORS: [(&str, &str); 3] = [
    ("seal.json", "7b9fddeba806c3b38f21bed33c550fde3a3ef4bae9107dc5e356816cedb441b0"),
    ("candidate.json", "9bde75a980f8a69b2b1f9bcf385cacaee10d35c2e4385ef85af0f431f1ba3348"),
    ("report.json", "0c08a8858cbb412e727e60f0007ac9f5b78001d7f79b0d26d59ec980c4b77173"),
];
const SCOPE: &str = "Historical rejected ortho-v2 evidence: anchored seal, report and candidate, source pins checked against archived copies; no new fit, new test or current-engine acceptance.";
const NOT_PROMOTED: &str = "no candidate built on this corpus has beaten the installed model while satisfying the rule; the weights separate 99.7% of wanted conversions at an ideal threshold, but the corpus cannot fix one: its negative tail is mislabelled wrong-layout text and unverifiable foreign strings, so the margin swallows the gain";
const EVIDENCE_SCOPE: &str = "synthetic wrong-layout renderings of frozen CC0 corpus tokens, shaped as the engine hands them over; the gate families contributed no character to any count; the lexical track is lowercased and therefore harsher than real typing";

fn gate_family(keys: &str) -> bool {
    let digest = hex_digest(&format!("{GATE_NAMESPACE}:{keys}"));
    u64::from_str_radix(&digest[..CHOICE_DIGITS], 16).expect("a digest") % PERCENT_SCALE < GATE_SHARE_PERCENT
}

/// `lexicon_rows`: (script, keys) -> weight, compressed so that a frequent word cannot dominate. A dictionary is evidence
/// about which sequences a language admits, not about how often they are typed.
fn lexicon_rows(root: &Path) -> Result<BTreeMap<(String, String), i64>, String> {
    let mut counts = BTreeMap::new();
    for (script, path, expected) in LEXICONS {
        for (word, frequency) in read_lexicon(root, path, expected)? {
            if script_of(&word) != Some(script) {
                continue;
            }
            let keys = to_keys(&word)?;
            if keys.is_empty() || pystr::len(&keys) > MAXIMUM_TOKEN_CHARACTERS {
                continue;
            }
            let bits = i128::from(128 - frequency.unsigned_abs().leading_zeros());
            let weight = bits.min(MAXIMUM_LEXICON_WEIGHT).max(1).to::<i64>();
            let entry = counts.entry((script.to_owned(), keys)).or_insert(weight);
            *entry = (*entry).max(weight);
        }
    }
    Ok(counts)
}

/// `rows`: one row per (split, script, key sequence), with occurrence and shape counts; the word lists after them.
fn corpus_rows(root: &Path) -> Result<Vec<Row>, String> {
    let phrases = load_phrases(root)?;
    let mut counts: BTreeMap<(&str, &str, String), BTreeMap<&str, i64>> = BTreeMap::new();
    for assigned in assign(&phrases) {
        let text = normalize::nfc(&assigned.phrase.text);
        for (position, raw) in pystr::split(&text).into_iter().enumerate() {
            if pystr::len(raw) > MAXIMUM_TOKEN_CHARACTERS {
                continue;
            }
            let Some(script) = script_of(raw) else { continue };
            let keys = to_keys(raw)?;
            if keys.is_empty() {
                continue;
            }
            *counts.entry((assigned.split, script, keys)).or_default().entry(shape_of(raw, position)).or_insert(0) += 1;
        }
    }
    let mut rows: Vec<Row> = counts
        .into_iter()
        .map(|((split, script, keys), shape)| Row { split: split.to_owned(), script: script.to_owned(), keys, shapes: shape.into_iter().map(|(name, count)| (name.to_owned(), count)).collect() })
        .collect();
    rows.extend(lexicon_rows(root)?.into_iter().map(|((script, keys), weight)| Row { split: "lexicon".to_owned(), script, keys, shapes: vec![("lower".to_owned(), weight)] }));
    Ok(rows)
}

fn rows_text(rows: &[Row]) -> String {
    let mut text = String::new();
    for row in rows {
        let shapes: Map<String, Value> = row.shapes.iter().map(|(name, count)| (name.clone(), json!(count))).collect();
        let line = canonical(&json!({"split": row.split, "script": row.script, "keys": row.keys, "shapes": shapes}));
        text.push_str(std::str::from_utf8(&line).expect("canonical is UTF-8"));
    }
    text
}

/// Rebuilds the key corpus and its receipt.
fn verify_corpus(root: &Path) -> Result<Vec<Row>, String> {
    let directory = root.join(DIRECTORY);
    let tokens = directory.join("tokens.jsonl.gz");
    let rows = corpus_rows(root)?;
    if rows_text(&rows) != gz_text(&tokens)? {
        return Err("token corpus is not reproducible from the frozen source".to_owned());
    }
    let (mut by_split, mut by_script, mut occurrences, mut gated): (BTreeMap<String, i64>, BTreeMap<String, i64>, BTreeMap<String, i64>, BTreeMap<String, i64>) = Default::default();
    for row in &rows {
        let key = format!("{}:{}", row.split, row.script);
        *by_split.entry(row.split.clone()).or_default() += 1;
        *by_script.entry(key.clone()).or_default() += 1;
        *occurrences.entry(key.clone()).or_default() += row.shapes.iter().map(|shape| shape.1).sum::<i64>();
        if gate_family(&row.keys) {
            *gated.entry(key).or_default() += 1;
        }
    }
    let path = directory.join("corpus-receipt.json");
    let recorded = read_object(&path)?;
    let sources = ORTHO_V2.recorded(recorded.get("provenance").unwrap_or(&Value::Null), &CORPUS_PROVENANCE, root)?;
    let report = json!({
        "schema_version": 1, "namespace": CORPUS_NAMESPACE,
        "description": "Key-space token types from the frozen CC0 phrase snapshot and the frozen Onboard lexicons, stored whole. What the engine presents is the token trimmed for the layout being typed, which `presented` derives. Lexicon rows carry compressed frequency weight and are for character counts only; they are lowercased and must never be used as negatives.",
        "gate": {
            "namespace": GATE_NAMESPACE, "share_percent": GATE_SHARE_PERCENT,
            "rule": format!("sha256(namespace + ':' + keys)[:{CHOICE_DIGITS}] mod {PERCENT_SCALE} < share"),
            "meaning": "excluded from every count; scored once by the promotion test",
            "types_by_split_script": gated,
        },
        "lexicon_weight": format!("max(1, min({MAXIMUM_LEXICON_WEIGHT}, frequency.bit_length()))"),
        "types_by_split": by_split, "types_by_split_script": by_script, "occurrences_by_split_script": occurrences,
        "maximum_token_characters": MAXIMUM_TOKEN_CHARACTERS, "tokens_sha256": checksum(&tokens)?, "provenance": sources,
    });
    if crate::train_json(&report) != std::fs::read(&path).map_err(|error| error.to_string())? {
        return Err("corpus receipt is missing or changed; a frozen split is never overwritten".to_owned());
    }
    Ok(rows)
}

fn hunspell_block(root: &Path) -> Result<Value, String> {
    let config = read_json(&root.join("model/intent_v1/config.json"))?;
    let recorded = &config["external_evaluation"]["hunspell"];
    let table: Map<String, Value> = ["en_US", "ru_RU"].iter().filter_map(|locale| recorded.get(*locale).map(|value| ((*locale).to_owned(), value.clone()))).collect();
    Ok(Value::Object(table))
}

/// The runtime's meaning of `known`: the frozen frequency list or Hunspell, in the dictionaries the config pins.
fn spellers(root: &Path) -> Result<BTreeMap<&'static str, std::sync::Arc<LanguageModel>>, String> {
    let models = super::lexicons(root, true)?;
    Ok([("en", models[0].clone()), ("ru", models[1].clone())].into_iter().collect())
}

/// Rebuilds the dictionary verdict of every token the engine can present and its receipt.
fn verify_known(root: &Path, rows: &[Row]) -> Result<(), String> {
    let directory = root.join(DIRECTORY).join("sources");
    let models = spellers(root)?;
    let mut forms: BTreeMap<&str, BTreeSet<String>> = SCRIPTS.iter().map(|script| (*script, BTreeSet::new())).collect();
    for row in rows {
        for direction in SCRIPTS {
            for token in served(&row.keys, direction) {
                forms.get_mut(direction).expect("a script").insert(visible(&token, direction));
            }
        }
    }
    let mut known_words = Map::new();
    let (mut known_by_script, mut forms_by_script) = (Map::new(), Map::new());
    for (script, words) in &forms {
        let kept: Vec<&String> = words.iter().filter(|word| models[script].score(word).known).collect();
        known_by_script.insert((*script).to_owned(), json!(kept.len()));
        forms_by_script.insert((*script).to_owned(), json!(words.len()));
        known_words.insert((*script).to_owned(), json!(kept));
    }
    let evidence = directory.join("known-evidence.json.gz");
    let text = String::from_utf8(canonical(&Value::Object(known_words))).map_err(|error| error.to_string())?;
    if text != gz_text(&evidence)? {
        return Err("dictionary evidence is not reproducible from these dictionaries".to_owned());
    }
    let path = directory.join("known-receipt.json");
    let recorded = read_object(&path)?;
    let generator = ORTHO_V2.recorded_digest("tools/ortho_v2_known.py", recorded.get("generator_sha256").unwrap_or(&Value::Null), root)?;
    let receipt = json!({
        "schema_version": 1,
        "description": "Whether the dictionary of each layout knows each token the engine can present, frozen so a replay needs no speller. Absent means not known.",
        "evidence_sha256": checksum(&evidence)?, "tokens_sha256": checksum(&root.join(DIRECTORY).join("tokens.jsonl.gz"))?,
        "generator_sha256": generator, "hunspell": hunspell_block(root)?,
        "known_by_script": known_by_script, "forms_by_script": forms_by_script,
        "scope": "the frequency list of model/intent_v1/sources plus Hunspell, which is what the runtime means by `known`; a machine whose speller differs will license more or fewer conversions than this evidence measured",
    });
    if crate::train_json(&receipt) != std::fs::read(&path).map_err(|error| error.to_string())? {
        return Err("dictionary receipt is missing or changed".to_owned());
    }
    Ok(())
}

/// Rebuilds the key sequences whose label a verifiably monolingual sentence vouches for, and their receipt.
fn verify_labels(root: &Path) -> Result<(), String> {
    let directory = root.join(DIRECTORY).join("sources");
    let models = spellers(root)?;
    let phrases = load_phrases(root)?;
    let mut trusted: BTreeMap<&str, BTreeSet<String>> = SCRIPTS.iter().map(|script| (*script, BTreeSet::new())).collect();
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    let mut cache: HashMap<(&str, String), bool> = HashMap::new();
    let mut knows = |script: &'static str, word: &str| -> bool { *cache.entry((script, word.to_owned())).or_insert_with(|| models[script].score(word).known) };
    for assigned in assign(&phrases) {
        let script: &'static str = if assigned.phrase.locale == "eng" { "en" } else { "ru" };
        let opposite = other(script);
        let text = normalize::nfc(&assigned.phrase.text);
        *counts.entry(format!("{script}:sentences")).or_default() += 1;
        let shaped: Vec<&str> = pystr::split(&text).into_iter().filter(|word| word.chars().any(ucd::isalpha)).collect();
        if shaped.len() < VERIFIED_MIN_TOKENS {
            *counts.entry(format!("{script}:too_short")).or_default() += 1;
            continue;
        }
        if shaped.iter().any(|word| script_of(word).is_some_and(|found| found != script)) {
            *counts.entry(format!("{script}:mixed_script")).or_default() += 1;
            continue;
        }
        let recognised = shaped.iter().filter(|word| knows(script, word)).count();
        if (recognised as f64) < VERIFIED_MIN_KNOWN_SHARE * shaped.len().to::<f64>() {
            *counts.entry(format!("{script}:too_unfamiliar")).or_default() += 1;
            continue;
        }
        let mut reads_otherwise = false;
        for word in &shaped {
            if !knows(script, word) && knows(opposite, &visible(&to_keys(word)?, opposite)) {
                reads_otherwise = true;
                break;
            }
        }
        if reads_otherwise {
            *counts.entry(format!("{script}:other_language_reading")).or_default() += 1;
            continue;
        }
        *counts.entry(format!("{script}:verified")).or_default() += 1;
        for word in &shaped {
            let keys = to_keys(word)?;
            trusted.get_mut(script).expect("a script").extend(served(&keys, script));
        }
    }
    let labels = directory.join("verified-labels.json.gz");
    let payload: Map<String, Value> = trusted.iter().map(|(script, words)| ((*script).to_owned(), json!(words))).collect();
    if String::from_utf8(canonical(&Value::Object(payload))).map_err(|error| error.to_string())? != gz_text(&labels)? {
        return Err("verified labels are not reproducible from these dictionaries".to_owned());
    }
    let path = directory.join("verified-receipt.json");
    let recorded = read_object(&path)?;
    let verified_sequences: BTreeMap<&str, usize> = trusted.iter().map(|(script, words)| (*script, words.len())).collect();
    let receipt = json!({
        "schema_version": 1,
        "description": "Key sequences whose language label is vouched for by a sentence that is verifiably monolingual, rather than inherited from one that is not.",
        "rule": {
            "minimum_tokens": VERIFIED_MIN_TOKENS, "minimum_known_share": VERIFIED_MIN_KNOWN_SHARE,
            "requires": ["every token written in the sentence's own alphabet", "at least the given share of tokens known to that language", "no unknown token that reads as a word of the other language"],
        },
        "labels_sha256": checksum(&labels)?, "source_sha256": checksum(&root.join("model/context_v2/sources/tatoeba-cc0-en-ru.tsv.gz"))?,
        "generator_sha256": ORTHO_V2.recorded_digest("tools/ortho_v2_verified.py", recorded.get("generator_sha256").unwrap_or(&Value::Null), root)?,
        "corpus_tool_sha256": ORTHO_V2.recorded_digest("tools/ortho_v2_corpus.py", recorded.get("corpus_tool_sha256").unwrap_or(&Value::Null), root)?,
        "hunspell": hunspell_block(root)?, "sentences": counts, "verified_sequences": verified_sequences,
        "scope": "a sentence-level verdict about a token-level label; it removes the inherited labels this corpus cannot support, and it cannot invent the ones it never had",
    });
    if crate::train_json(&receipt) != std::fs::read(&path).map_err(|error| error.to_string())? {
        return Err("verified-label receipt is missing or changed".to_owned());
    }
    Ok(())
}

/// `counts`: a gate-family sequence is skipped here and nowhere else, which is what makes its later evaluation a statement
/// about sequences the model never saw.
fn counts(rows: &[Row], order: usize) -> (Counts, Counts, Counts) {
    let (mut grams, mut shapes, mut acronyms) = (empty_counts(), empty_counts(), empty_counts());
    let mut upper_keys: HashMap<&str, HashSet<&str>> = SCRIPTS.iter().map(|script| (*script, HashSet::new())).collect();
    for row in rows.iter().filter(|row| row.split == "train" && !gate_family(&row.keys)) {
        if row.shapes.iter().any(|(name, count)| name == "upper" && *count != 0) {
            upper_keys.get_mut(row.script.as_str()).expect("a script").insert(row.keys.as_str());
        }
    }
    let padding: String = std::iter::repeat_n(BOS, order - 1).collect();
    for row in rows.iter().filter(|row| (row.split == "train" || row.split == "lexicon") && !gate_family(&row.keys)) {
        let weight: i64 = row.shapes.iter().map(|shape| shape.1).sum();
        let sequences: BTreeSet<String> = SCRIPTS.iter().map(|layout| word_core(&row.keys, layout)).filter(|sequence| !sequence.is_empty()).collect();
        let table = grams.get_mut(&row.script).expect("a script");
        for sequence in sequences {
            let padded: Vec<char> = padding.chars().chain(sequence.chars()).chain([EOS]).collect();
            for index in order - 1..padded.len() {
                for size in 1..=order {
                    *table.entry(padded[index + 1 - size..=index].iter().collect()).or_insert(0) += weight;
                }
            }
        }
        if row.split != "train" {
            continue;
        }
        let target = if upper_keys[row.script.as_str()].contains(row.keys.as_str()) { &mut acronyms } else { &mut shapes };
        let table = target.get_mut(&row.script).expect("a script");
        for (name, value) in &row.shapes {
            *table.entry(name.clone()).or_insert(0) += value;
        }
    }
    (grams, shapes, acronyms)
}

fn build(rows: &[Row], settings: &Value) -> Result<Map<String, Value>, String> {
    let order = (settings["order"].as_u64().ok_or("no order")?).to::<usize>();
    let discount = settings["discount"].as_f64().ok_or("no discount")?;
    let scale = settings["scale"].as_i64().ok_or("no scale")?;
    let (grams, prose, acronym) = counts(rows, order);
    let models: Map<String, Value> = SCRIPTS.iter().map(|script| ((*script).to_owned(), channel_payload(&grams[*script], order, discount, scale))).collect();
    let mut payload = Map::new();
    payload.insert("schema_version".to_owned(), json!(1));
    payload.insert("order".to_owned(), json!(order));
    payload.insert("scale".to_owned(), json!(scale));
    payload.insert("models".to_owned(), Value::Object(models));
    payload.insert("prose_shape".to_owned(), shape_channel(&prose, scale));
    payload.insert("acronym_shape".to_owned(), shape_channel(&acronym, scale));
    let digest = hex(&Sha256::digest(canonical(&Value::Object(payload.clone()))));
    payload.insert("version".to_owned(), json!(format!("ortho-v2-{}", &digest[..VERSION_HASH_CHARACTERS])));
    payload.insert("weights_sha256".to_owned(), json!(digest));
    Ok(payload)
}

/// A counted artifact, scored here rather than through the runtime loader: the installed model and every candidate are
/// read by these few lines, so the two are compared by the same code.
struct CountedModel {
    order: usize,
    version: String,
    logprob: BTreeMap<&'static str, HashMap<String, f64>>,
    backoff: BTreeMap<&'static str, HashMap<String, f64>>,
    uniform: BTreeMap<&'static str, f64>,
    shape: BTreeMap<&'static str, BTreeMap<&'static str, BTreeMap<String, f64>>>,
    thresholds: BTreeMap<&'static str, f64>,
}

impl CountedModel {
    fn new(payload: &Value) -> Result<CountedModel, String> {
        if payload["schema_version"] != json!(1) {
            return Err("unexpected counted artifact".to_owned());
        }
        let scale = (payload["scale"].as_i64().ok_or("no scale")?).to::<f64>();
        let (mut logprob, mut backoff, mut uniform) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
        for script in SCRIPTS {
            let channel = &payload["models"][script];
            let names = channel["grams"].as_str().ok_or("no grams")?.split('\n');
            let values = channel["logprob"].as_array().ok_or("no logprob")?;
            logprob.insert(script, names.zip(values).map(|(name, value)| (name.to_owned(), value.as_i64().unwrap_or(0).to::<f64>() / scale)).collect());
            let table = channel["backoff"].as_array().ok_or("no backoff")?;
            backoff.insert(script, table.iter().map(|entry| (entry[0].as_str().unwrap_or("").to_owned(), entry[1].as_i64().unwrap_or(0).to::<f64>() / scale)).collect());
            uniform.insert(script, (channel["uniform"].as_i64().ok_or("no uniform")?).to::<f64>() / scale);
        }
        let mut shape = BTreeMap::new();
        for name in ["prose_shape", "acronym_shape"] {
            let mut tables = BTreeMap::new();
            for script in SCRIPTS {
                let table = payload[name][script].as_object().ok_or("no shape table")?;
                tables.insert(script, table.iter().map(|(shape, value)| (shape.clone(), value.as_i64().unwrap_or(0).to::<f64>() / scale)).collect());
            }
            shape.insert(name, tables);
        }
        let thresholds = SCRIPTS.iter().map(|script| (*script, (payload["thresholds"][script].as_i64().unwrap_or(0)).to::<f64>() / scale)).collect();
        Ok(CountedModel { order: (payload["order"].as_u64().ok_or("no order")?).to::<usize>(), version: payload["version"].as_str().unwrap_or("").to_owned(), logprob, backoff, uniform, shape, thresholds })
    }

    fn conditional(&self, script: &'static str, gram: &[char], buffer: &mut String) -> f64 {
        let mut accumulated = 0.0;
        let mut gram = gram;
        loop {
            buffer.clear();
            buffer.extend(gram);
            if let Some(found) = self.logprob[script].get(buffer.as_str()) {
                return accumulated + found;
            }
            if gram.len() == 1 {
                return accumulated + self.uniform[script];
            }
            buffer.pop();
            accumulated += self.backoff[script].get(buffer.as_str()).copied().unwrap_or(0.0);
            gram = &gram[1..];
        }
    }

    fn sequence(&self, script: &'static str, keys: &str) -> f64 {
        let padded: Vec<char> = std::iter::repeat_n(BOS, self.order - 1).chain(keys.chars()).chain([EOS]).collect();
        let mut buffer = String::new();
        let values: Vec<f64> = (self.order - 1..padded.len()).map(|index| self.conditional(script, &padded[index + 1 - self.order..=index], &mut buffer)).collect();
        py_sum(values.into_iter())
    }

    /// Nats in favour of reading these keys in the other layout.
    fn score(&self, keys: &str, shape: &str, source: &'static str) -> f64 {
        let target: &'static str = if source == "en" { "ru" } else { "en" };
        let shape = if matches!(shape, "lower" | "initial" | "inner" | "upper") { shape } else { "lower" };
        self.sequence(target, keys) - self.sequence(source, keys) - (self.shape["acronym_shape"][source][shape] - self.shape["prose_shape"][source][shape])
    }

    fn converts(&self, keys: &str, shape: &str, source: &'static str) -> bool {
        self.score(keys, shape, source) > self.thresholds[source]
    }
}

type Sample = (String, String, String, i64);

fn evidence_rows(rows: &[Row], split: Option<&str>, gate: bool, seen: Option<&HashSet<String>>) -> Vec<Sample> {
    let mut result = Vec::new();
    for row in rows {
        if row.split == "lexicon" || gate_family(&row.keys) != gate || split.is_some_and(|wanted| row.split != wanted) || seen.is_some_and(|seen| seen.contains(&row.keys)) {
            continue;
        }
        for (shape, weight) in &row.shapes {
            result.push((row.script.clone(), row.keys.clone(), shape.clone(), *weight));
        }
    }
    result
}

fn code_like(keys: &str) -> bool {
    keys.chars().any(|character| ucd::isdigit(character) || "_/\\=:@".contains(character))
}

/// Only what the engine actually hands the model reaches an evaluation.
fn governed(keys: &str, shape: &str, minimum: usize, direction: &str) -> bool {
    if pystr::len(keys) < minimum || shape == "upper" || !word_shaped(&visible(keys, direction)) {
        return false;
    }
    !code_like(keys)
}

type Known = HashMap<String, HashSet<String>>;

fn dictionary_known(known: &Known, script: &str, keys: &str) -> bool {
    known[script].contains(&visible(keys, script))
}

struct Scored {
    script: String,
    direction: &'static str,
    total: f64,
    weight: i64,
    keys: String,
    shape: String,
}

/// `measure`: each token in both directions. The score depends only on the keys; the label decides whether it is a
/// negative for that direction or a positive.
fn measure(model: &CountedModel, samples: &[Sample], minimum: usize, known: &Known) -> Vec<Scored> {
    let mut scored = Vec::new();
    for (script, whole, shape, weight) in samples {
        for direction in SCRIPTS {
            for keys in served(whole, direction) {
                if !governed(&keys, shape, minimum, direction) || dictionary_known(known, direction, &keys) {
                    continue;
                }
                let total = model.score(&keys, shape, direction);
                scored.push(Scored { script: script.clone(), direction, total, weight: *weight, keys, shape: shape.clone() });
            }
        }
    }
    scored
}

fn bucket_of(keys: &str, buckets: &[i64]) -> i64 {
    let length = pystr::len(keys).to::<i64>();
    buckets.iter().copied().find(|value| length <= *value).unwrap_or(buckets[buckets.len() - 1])
}

fn outcome(scored: &[Scored], thresholds: &BTreeMap<&str, f64>, buckets: &[i64], attested: &HashSet<String>) -> Value {
    let mut counted: BTreeMap<String, i64> = BTreeMap::new();
    let mut worst: BTreeMap<&str, Vec<(f64, &str, &str)>> = SCRIPTS.iter().map(|script| (*script, Vec::new())).collect();
    for item in scored {
        let direction = item.direction;
        let bucket = bucket_of(&item.keys, buckets);
        let converts = item.total > thresholds[direction];
        let mut add = |name: String, value: i64| *counted.entry(format!("{direction}:{name}")).or_insert(0) += value;
        if item.script == direction {
            add("negative_types".to_owned(), 1);
            add("negative_occurrences".to_owned(), item.weight);
            if converts {
                add("false_types".to_owned(), 1);
                add("false_occurrences".to_owned(), item.weight);
                if item.weight > 1 {
                    add("false_repeated_types".to_owned(), 1);
                }
                if attested.contains(&item.keys) {
                    add("false_attested_types".to_owned(), 1);
                }
                worst.get_mut(direction).expect("a script").push((item.total, &item.keys, &item.shape));
            }
        } else {
            add("positive_types".to_owned(), 1);
            add("positive_occurrences".to_owned(), item.weight);
            add(format!("positive_types_len{bucket}"), 1);
            if converts {
                add("recalled_types".to_owned(), 1);
                add("recalled_occurrences".to_owned(), item.weight);
                add(format!("recalled_types_len{bucket}"), 1);
            }
        }
    }
    let mut listed = Map::new();
    for (script, mut items) in worst {
        items.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| b.1.cmp(a.1)).then_with(|| b.2.cmp(a.2)));
        let top: Vec<Value> = items.iter().take(REPORTED_WORST_FALSE).map(|(value, keys, shape)| json!({"score": py_round_ndigits(*value, REPORTED_SCORE_DECIMALS).expect("a finite score"), "keys": keys, "shape": shape})).collect();
        listed.insert(script.to_owned(), json!(top));
    }
    json!({"counts": counted, "worst_false": listed})
}

struct Totals {
    negative_types: i64,
    negative_occurrences: i64,
    false_types: i64,
    false_occurrences: i64,
    false_repeated_types: i64,
    false_attested_types: i64,
    recall: f64,
}

impl Totals {
    fn false_per_10000(&self) -> f64 {
        (FALSE_RATE_SCALE * self.false_occurrences).to::<f64>() / self.negative_occurrences.max(1).to::<f64>()
    }

    fn rate_json(&self) -> f64 {
        py_round_ndigits(self.false_per_10000(), FALSE_RATE_DECIMALS).expect("a finite rate")
    }

    fn recall_json(&self) -> f64 {
        py_round_ndigits(self.recall, RECALL_DECIMALS).expect("a finite recall")
    }
}

fn totals(counts: &Value) -> Totals {
    let summed = |name: &str| SCRIPTS.iter().map(|script| counts.get(format!("{script}:{name}")).and_then(Value::as_i64).unwrap_or(0)).sum::<i64>();
    let positives = summed("positive_types");
    Totals {
        negative_types: summed("negative_types"),
        negative_occurrences: summed("negative_occurrences"),
        false_types: summed("false_types"),
        false_occurrences: summed("false_occurrences"),
        false_repeated_types: summed("false_repeated_types"),
        false_attested_types: summed("false_attested_types"),
        recall: summed("recalled_types").to::<f64>() / positives.max(1).to::<f64>(),
    }
}

fn promoted(result: &Totals, baseline: &Totals, promotion: &Value, prefix: &str) -> bool {
    result.negative_types >= promotion[format!("minimum_{prefix}negatives")].as_i64().unwrap_or(i64::MAX) && result.false_types == 0 && result.recall >= baseline.recall + promotion["minimum_recall_gain"].as_f64().unwrap_or(f64::INFINITY)
}

/// The installed model's numbers on the rows the candidate saw; with `guards`, behind this release's two refusals.
fn baseline_outcome(model: &CountedModel, samples: &[Sample], minimum: usize, guards: bool, known: &Known) -> Totals {
    let mut counted: BTreeMap<String, i64> = BTreeMap::new();
    for (script, whole, shape, weight) in samples {
        for direction in SCRIPTS {
            for keys in served(whole, direction) {
                if pystr::len(&keys) < minimum || shape == "upper" || code_like(&keys) {
                    continue;
                }
                if guards && (!governed(&keys, shape, minimum, direction) || dictionary_known(known, direction, &keys)) {
                    continue;
                }
                let converts = model.converts(&keys, shape, direction);
                let mut add = |name: &str, value: i64| *counted.entry(format!("{direction}:{name}")).or_insert(0) += value;
                if script == direction {
                    add("negative_types", 1);
                    add("negative_occurrences", *weight);
                    if converts {
                        add("false_types", 1);
                        add("false_occurrences", *weight);
                        if *weight > 1 {
                            add("false_repeated_types", 1);
                        }
                    }
                } else {
                    add("positive_types", 1);
                    add("positive_occurrences", *weight);
                    if converts {
                        add("recalled_types", 1);
                        add("recalled_occurrences", *weight);
                    }
                }
            }
        }
    }
    totals(&json!(counted))
}

/// `attested_keys`: key sequences the snapshot shows more than once.
fn attested_keys(rows: &[Row]) -> HashSet<String> {
    let mut occurrences: HashMap<String, i64> = HashMap::new();
    for row in rows.iter().filter(|row| row.split != "lexicon") {
        let weight: i64 = row.shapes.iter().map(|shape| shape.1).sum();
        let tokens: BTreeSet<String> = SCRIPTS.iter().flat_map(|direction| served(&row.keys, direction)).collect();
        for keys in tokens {
            *occurrences.entry(keys).or_insert(0) += weight;
        }
    }
    occurrences.into_iter().filter(|(_, count)| *count > 1).map(|(keys, _)| keys).collect()
}

fn load_known(root: &Path) -> Result<Known, String> {
    let payload: Value = serde_json::from_str(&gz_text(&root.join(DIRECTORY).join("sources/known-evidence.json.gz"))?).map_err(|error| error.to_string())?;
    let table = payload.as_object().ok_or("invalid dictionary evidence")?;
    Ok(table.iter().map(|(script, words)| (script.clone(), words.as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect())).collect())
}

fn seal_of(payload: &Map<String, Value>, thresholds: &BTreeMap<&str, f64>, settings: &Value, pins: &Map<String, Value>, calibration_rows: usize, candidate: &[u8]) -> Value {
    let rounded: BTreeMap<&str, f64> = thresholds.iter().map(|(script, value)| (*script, py_round_ndigits(*value, SEALED_THRESHOLD_DECIMALS).expect("a finite threshold"))).collect();
    json!({
        "schema_version": 1, "stage": SEALED_BEFORE_TEST, "namespace": NAMESPACE, "model_version": payload["version"],
        "candidate_sha256": hex(&Sha256::digest(candidate)), "thresholds": rounded, "config": settings, "provenance": pins,
        "calibration_rows": calibration_rows,
    })
}

/// Fits the candidate and sets its thresholds on the development splits.
fn fit(root: &Path, rows: &[Row], settings: &Value, known: &Known, pins: &Map<String, Value>) -> Result<(Vec<u8>, Vec<u8>), String> {
    let minimum = (settings["minimum_length"].as_u64().ok_or("no minimum length")?).to::<usize>();
    let scale = settings["scale"].as_i64().ok_or("no scale")?;
    let mut payload = build(rows, settings)?;
    payload.insert("minimum_length".to_owned(), json!(minimum));
    payload.insert("thresholds".to_owned(), json!({"en": 0, "ru": 0}));
    let model = CountedModel::new(&Value::Object(payload.clone()))?;
    let mut samples = evidence_rows(rows, Some("calibration"), false, None);
    samples.extend(evidence_rows(rows, Some("development"), false, None));
    let calibration = measure(&model, &samples, minimum, known);
    let margin = settings["margin_nats"].as_f64().ok_or("no margin")?;
    let mut thresholds = BTreeMap::new();
    for script in SCRIPTS {
        let negatives = calibration.iter().filter(|item| item.script == script && item.direction == script).map(|item| item.total);
        let worst = ks_text::max_first(negatives).ok_or_else(|| format!("calibration split has no {script} negatives"))?;
        thresholds.insert(script, worst + margin);
    }
    let quantised: BTreeMap<&str, i64> = thresholds.iter().map(|(script, value)| (*script, quantise(*value, scale))).collect();
    payload.insert("thresholds".to_owned(), json!(quantised));
    let _ = root;
    let candidate = canonical(&Value::Object(payload.clone()));
    let seal = seal_of(&payload, &thresholds, settings, pins, calibration.len(), &candidate);
    Ok((candidate, canonical(&seal)))
}

/// The one-shot test, repeated as a regression: the report's every number.
fn test_report(root: &Path, rows: &[Row], settings: &Value, known: &Known) -> Result<Vec<u8>, String> {
    let directory = root.join(DIRECTORY);
    let minimum = (settings["minimum_length"].as_u64().ok_or("no minimum length")?).to::<usize>();
    let buckets: Vec<i64> = settings["length_buckets"].as_array().ok_or("no buckets")?.iter().filter_map(Value::as_i64).collect();
    let seal = read_object(&directory.join("seal.json"))?;
    let table = seal.get("thresholds").and_then(Value::as_object).ok_or("no thresholds")?;
    let thresholds: BTreeMap<&str, f64> = SCRIPTS.iter().map(|script| (*script, table[*script].as_f64().unwrap_or(f64::INFINITY))).collect();
    let model = CountedModel::new(&read_json(&directory.join("candidate.json"))?)?;
    let legacy_path: PathBuf = root.join(BASELINE);
    let legacy = CountedModel::new(&read_json(&legacy_path)?)?;
    let mut seen: HashSet<String> = read_rows(&gz_text(&root.join(BASELINE_CORPUS))?)?.into_iter().filter(|row| row.split == "train").map(|row| row.keys).collect();
    seen.extend(rows.iter().filter(|row| (row.split == "train" || row.split == "lexicon") && !gate_family(&row.keys)).map(|row| row.keys.clone()));
    let attested = attested_keys(rows);
    let gate_rows = evidence_rows(rows, None, true, None);
    let unseen_rows = evidence_rows(rows, None, true, Some(&seen));
    let test_rows = evidence_rows(rows, Some("test"), false, None);
    let gate = outcome(&measure(&model, &gate_rows, minimum, known), &thresholds, &buckets, &attested);
    let unseen = outcome(&measure(&model, &unseen_rows, minimum, known), &thresholds, &buckets, &attested);
    let held = outcome(&measure(&model, &test_rows, minimum, known), &thresholds, &buckets, &attested);
    let mut stress = Vec::new();
    for ((script, keys), _) in lexicon_rows(root)? {
        if gate_family(&keys) && governed(&keys, "lower", minimum, &script) {
            stress.push((script, keys, "lower".to_owned(), 1));
        }
    }
    let lexical = outcome(&measure(&model, &stress, minimum, known), &thresholds, &buckets, &HashSet::new());
    let promotion = &settings["promotion"];
    let (gate_totals, unseen_totals, held_totals) = (totals(&gate["counts"]), totals(&unseen["counts"]), totals(&held["counts"]));
    let gate_baseline = baseline_outcome(&legacy, &gate_rows, minimum, false, known);
    let unseen_baseline = baseline_outcome(&legacy, &unseen_rows, minimum, false, known);
    let test_baseline = baseline_outcome(&legacy, &test_rows, minimum, false, known);
    let gate_guarded = baseline_outcome(&legacy, &gate_rows, minimum, true, known);
    let test_guarded = baseline_outcome(&legacy, &test_rows, minimum, true, known);
    let passed = promoted(&unseen_totals, &unseen_baseline, promotion, "unseen_") && promoted(&gate_totals, &gate_baseline, promotion, "gate_") && promoted(&held_totals, &test_baseline, promotion, "test_");
    let plain = |totals: &Totals| json!({"false_per_10000": totals.rate_json(), "false_repeated_types": totals.false_repeated_types, "false_attested_types": totals.false_attested_types, "recall": totals.recall_json()});
    let report = json!({
        "schema_version": 1, "model_version": seal["model_version"], "seal_sha256": checksum(&directory.join("seal.json"))?, "candidate_sha256": checksum(&directory.join("candidate.json"))?,
        "thresholds": seal["thresholds"], "gate_families": gate, "gate_recall": gate_totals.recall_json(), "gate_false_per_10000": gate_totals.rate_json(),
        "mutually_unseen": unseen, "mutually_unseen_recall": unseen_totals.recall_json(), "mutually_unseen_false_per_10000": unseen_totals.rate_json(),
        "mutually_unseen_scope": "gate families the installed model did not count either; the only population where neither model can be answering from memory",
        "corpus_test": held, "recall": held_totals.recall_json(), "test_false_per_10000": held_totals.rate_json(), "lexical_stress_track": lexical,
        "baseline": {"model_version": legacy.version, "scope": "the installed model as users run it today, on the same rows", "gate": plain(&gate_baseline), "test": plain(&test_baseline), "mutually_unseen": plain(&unseen_baseline)},
        "baseline_behind_new_guards": {
            "scope": "the same weights with this release's two refusals applied, reported so the weights and the guards can be told apart",
            "gate": {"false_per_10000": gate_guarded.rate_json(), "recall": gate_guarded.recall_json()},
            "test": {"false_per_10000": test_guarded.rate_json(), "recall": test_guarded.recall_json()},
        },
        "promotion_passed": passed, "not_promoted_because": if passed { Value::Null } else { json!(NOT_PROMOTED) }, "evidence_scope": EVIDENCE_SCOPE,
    });
    Ok(canonical(&report))
}

/// The evidence is whole and describes the sealed candidate, and nothing of it is installed. Reads no token.
pub fn verify(root: &Path) -> Result<Value, String> {
    let directory = root.join(DIRECTORY);
    for (name, expected) in ANCHORS {
        if checksum(&directory.join(name))? != expected {
            return Err(format!("historical ortho-v2 anchor changed: {name}"));
        }
    }
    let table = Value::Object(ORTHO_V2.sources.iter().map(|(name, digest)| ((*name).to_owned(), json!(digest))).collect());
    ORTHO_V2.verify(&table, root)?;
    let receipt = read_object(&directory.join("corpus-receipt.json"))?;
    if receipt.get("tokens_sha256").and_then(Value::as_str) != Some(checksum(&directory.join("tokens.jsonl.gz"))?.as_str()) {
        return Err("key-space corpus does not match its receipt".to_owned());
    }
    ORTHO_V2.recorded(receipt.get("provenance").unwrap_or(&Value::Null), &CORPUS_PROVENANCE, root)?;
    let dictionary = read_object(&directory.join("sources/known-receipt.json"))?;
    if dictionary.get("evidence_sha256").and_then(Value::as_str) != Some(checksum(&directory.join("sources/known-evidence.json.gz"))?.as_str()) {
        return Err("dictionary evidence does not match its receipt".to_owned());
    }
    ORTHO_V2.recorded_digest("tools/ortho_v2_known.py", dictionary.get("generator_sha256").unwrap_or(&Value::Null), root)?;
    let labels = read_object(&directory.join("sources/verified-receipt.json"))?;
    if labels.get("labels_sha256").and_then(Value::as_str) != Some(checksum(&directory.join("sources/verified-labels.json.gz"))?.as_str()) {
        return Err("verified labels do not match their receipt".to_owned());
    }
    ORTHO_V2.recorded_digest("tools/ortho_v2_verified.py", labels.get("generator_sha256").unwrap_or(&Value::Null), root)?;
    ORTHO_V2.recorded_digest("tools/ortho_v2_corpus.py", labels.get("corpus_tool_sha256").unwrap_or(&Value::Null), root)?;
    let candidate = checksum(&directory.join("candidate.json"))?;
    let seal = read_object(&directory.join("seal.json"))?;
    if seal.get("stage").and_then(Value::as_str) != Some(SEALED_BEFORE_TEST) || seal.get("candidate_sha256").and_then(Value::as_str) != Some(candidate.as_str()) {
        return Err("orthotactic seal or provenance changed".to_owned());
    }
    ORTHO_V2.recorded(seal.get("provenance").unwrap_or(&Value::Null), &SEAL_PROVENANCE, root)?;
    let config_path = directory.join("config.json");
    if seal.get("config") != Some(&read_json(&config_path)?) {
        return Err("orthotactic configuration changed after the seal".to_owned());
    }
    let report = read_object(&directory.join("report.json"))?;
    if report.get("seal_sha256").and_then(Value::as_str) != Some(checksum(&directory.join("seal.json"))?.as_str()) || report.get("candidate_sha256").and_then(Value::as_str) != Some(candidate.as_str()) || report.get("thresholds") != seal.get("thresholds") {
        return Err("orthotactic report does not describe this candidate".to_owned());
    }
    if report.get("promotion_passed") != Some(&json!(false)) {
        return Err("a v2 candidate now passes its promotion rule; publish it deliberately and replace this verifier rather than letting it ship unnoticed".to_owned());
    }
    if root.join("resources/models/ortho_v2.json").exists() {
        return Err("a v2 artifact is installed although no candidate was promoted".to_owned());
    }
    let mut measured = Map::new();
    for (name, section) in [("mutually_unseen", "mutually_unseen"), ("gate", "gate_families"), ("test", "corpus_test")] {
        let counts = &report[section]["counts"];
        let summed = |field: &str| SCRIPTS.iter().map(|script| counts.get(format!("{script}:{field}")).and_then(Value::as_i64).unwrap_or(0)).sum::<i64>();
        let recall = summed("recalled_types").to::<f64>() / summed("positive_types").max(1).to::<f64>();
        measured.insert(name.to_owned(), json!({"negatives": summed("negative_types"), "false_types": summed("false_types"), "recall": py_round_ndigits(recall, RECALL_DECIMALS).expect("a finite recall")}));
    }
    Ok(json!({
        "schema_version": 1, "model_version": seal["model_version"], "historical_evidence_verified": true, "current_runtime_verified": false, "scope": SCOPE,
        "promoted": false, "installed_artifact": null, "candidate_sha256": candidate, "configuration_sha256": checksum(&config_path)?, "measured": measured,
        "why_not_promoted": report["not_promoted_because"],
    }))
}

/// Rebuilds everything from the frozen sources; writes nothing.
pub fn replay(root: &Path) -> Result<Value, String> {
    let mut result = verify(root)?;
    let directory = root.join(DIRECTORY);
    let rows = verify_corpus(root)?;
    verify_known(root, &rows)?;
    verify_labels(root)?;
    let settings = read_json(&directory.join("config.json"))?;
    let known = load_known(root)?;
    let seal = read_object(&directory.join("seal.json"))?;
    let pins = ORTHO_V2.recorded(seal.get("provenance").unwrap_or(&Value::Null), &SEAL_PROVENANCE, root)?;
    let (candidate, sealed) = fit(root, &rows, &settings, &known, &pins)?;
    let read = |name: &str| std::fs::read(directory.join(name)).map_err(|error| error.to_string());
    if candidate != read("candidate.json")? {
        return Err("orthotactic candidate is not reproducible".to_owned());
    }
    if sealed != read("seal.json")? {
        return Err("orthotactic seal is not reproducible".to_owned());
    }
    if test_report(root, &rows, &settings, &known)? != read("report.json")? {
        return Err("orthotactic test report is not reproducible".to_owned());
    }
    result["frozen_replay"] = json!(true);
    Ok(result)
}
