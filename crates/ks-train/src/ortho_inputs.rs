//! What the orthotactic model counts, built from frozen sources (`context_corpus`, `ortho_corpus`, `ortho_known`,
//! `ortho_web_lexicon`, `ortho_independent`, `wikisource_text`): tokens of the CC0 phrase snapshot in key space, the verdict
//! of the reference dictionaries on every word a token can ask about, the web word forms, and the independent test sets.
//! `--inputs` rebuilds each and compares it with the frozen file; `--freeze-inputs` writes a missing generation.
use ks_cast::To as _;
use crate::corpus::{checksum, read_json};
use crate::ortho::{self, KEYS, LEXICON, Row, other, read_rows, served, to_keys, visible, word_core};
use crate::physical::{hex, hex_digest};
use flate2::Compression;
use flate2::read::MultiGzDecoder;
use flate2::write::GzEncoder;
use ks_lang::LanguageModel;
use ks_lang::constants::ACRONYM_MIN_LETTERS;
use ks_lang::ortho_model::{read_once, readings, stretch_readings};
use ks_text::{normalize, pystr};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::Path;

const SOURCE: &str = "model/context_v2/sources/tatoeba-cc0-en-ru.tsv.gz";
const SOURCE_SHA256: &str = "1f7df948b946f1d1aed5efd2a83a9a5a09d2b8abcb9224045b40c9ec6720e793";
const PHRASE_NAMESPACE: &str = "keyswitch:context-v2:phrases-20260905";
const PHRASE_SPLIT_BUCKETS: u64 = 100;
const SPLIT_NAMESPACE: &str = "keyswitch:ortho-v1:served-split-g15";
const DIRECTORY: &str = "model/ortho_v1";
const PHRASES_PER_GROUP: usize = 4;
const PHRASE_MIN_CHARACTERS: usize = 4;
const PHRASE_MAX_CHARACTERS: usize = 512;
const PHRASE_MIN_WORDS: usize = 2;
const NEAR_DUPLICATE_MIN_WORDS: usize = 4;
const NEAR_DUPLICATE_MAX_WORDS: usize = 40;
const BUCKET_COUNT: u64 = 100;
const TEST_SPLIT_CEILING: u64 = 55;
const CHOICE_HEX_DIGITS: usize = 8;
const MAXIMUM_TOKEN_CHARACTERS: usize = 32;
const LEXICON_MIN_CHARACTERS: usize = 2;
const MAXIMUM_LEXICON_WEIGHT: u32 = 32;
const EVIDENCE_SCHEMA: i64 = 2;
const CONTEXT_TYPO_MIN_CHARACTERS: usize = 5;
const MIN_COLLAPSED_RUN: i64 = 2;
const MIN_STRETCH_RUN: i64 = 2;
const SCRIPTS: [&str; 2] = ["en", "ru"];
const LEXICONS: [(&str, &str, &str, &str); 2] = [("en", "model/intent_v1/sources/en_US.lm", "017a513a23bb37947a3a0e73e307066fa8ec0642075172cdcad6ea204cae68da", "en_US"), ("ru", "model/intent_v1/sources/ru_RU.lm", "e57c14eec2b78e52a2125dce2b38044d8dfe480f3964b9d118614527195ceda9", "ru_RU")];
const CYRILLIC: &str = "абвгдеёжзийклмнопрстуфхцчшщъыьэюя";
const CORPUS_CODE: [&str; 3] = ["crates/ks-train/src/ortho_inputs.rs", "crates/ks-train/src/ortho.rs", "crates/ks-text/src/layouts.rs"];
const KNOWN_CODE: [&str; 1] = ["crates/ks-train/src/ortho_inputs.rs"];
const WEB_SOURCE_SHA256: &str = "02bd7a202103a6b90c8f97617144ed230e3b4d51a08c28d2e029e1f0027917fa";
const WEB_MIN_CHARACTERS: usize = 4;
const WEB_SCHEMA: i64 = 1;
const INDEPENDENT_SCHEMA: i64 = 2;
const SET_NAME: &str = "wikipedia-en-averchenko-leskov-ru";
const HISTORY: [(&str, &str); 2] = [("development-history/independent-common-voice-sentence-collector", "development-history/report-ortho-v1-71e34529bdd4.json"), ("development-history/independent-wikipedia-en-chekhov-ru", "development-history/report-ortho-v1-8723bff1344e.json")];
const DEVELOPMENT_SETS: [&str; 1] = ["development-history/class-development-gorky-bulgakov-kuzmin"];
const LOOKED_AT_WORDS: [&str; 43] = ["каеффф", "рсфср", "гыф", "фыва", "гифку", "тыща", "лдпр", "мацуи", "москва", "мда-а", "у-у", "а-б", "лут", "флуд", "фнаф", "ctrl", "oled", "gtk", "hfcs", "-тв", "тв-", "gtk-ключ", "а-бв", "ааа-ба", "абв", "квазимодо", "мцуис", "ну-ну", "тпште", "ура-а-а", "ф-привет", "фпревет", "сборки", "abab", "ahtop", "q-nj", "xyzzy", "zorblax", "ab-bb", "htop-", ".xyz", "dist", "гыр-гыр"];

fn gz_write(raw: &[u8]) -> Result<Vec<u8>, String> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(raw).map_err(|error| error.to_string())?;
    encoder.finish().map_err(|error| error.to_string())
}

fn gz_read(path: &Path) -> Result<Vec<u8>, String> {
    let mut raw = Vec::new();
    MultiGzDecoder::new(std::fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?).read_to_end(&mut raw).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(raw)
}

/// `Phrase` and `AssignedPhrase`, as far as the corpus reads them.
pub struct Phrase {
    pub identifier: i64,
    pub locale: String,
    pub text: String,
}

pub struct Assigned<'a> {
    pub phrase: &'a Phrase,
    pub group: String,
    pub split: &'static str,
}

/// `load_source`.
pub fn load_phrases(root: &Path) -> Result<Vec<Phrase>, String> {
    let path = root.join(SOURCE);
    if checksum(&path)? != SOURCE_SHA256 {
        return Err("frozen CC0 source checksum mismatch".to_owned());
    }
    let raw = String::from_utf8(gz_read(&path)?).map_err(|error| error.to_string())?;
    let mut phrases: BTreeMap<i64, Phrase> = BTreeMap::new();
    for line in pystr::splitlines(&raw, true) {
        let fields: Vec<&str> = line.strip_suffix('\n').unwrap_or(line).split('\t').collect();
        let [identifier, language, text, _modified] = fields.as_slice() else { return Err("invalid CC0 TSV row".to_owned()) };
        if *language != "eng" && *language != "rus" {
            continue;
        }
        let number: i64 = identifier.parse().map_err(|_| "invalid CC0 source ID")?;
        if number <= 0 || phrases.insert(number, Phrase { identifier: number, locale: (*language).to_owned(), text: (*text).to_owned() }).is_some() {
            return Err("nonpositive or duplicate source ID".to_owned());
        }
    }
    Ok(phrases.into_values().collect())
}

fn phrase_is_eligible(item: &Phrase) -> bool {
    let length = item.text.chars().count();
    (PHRASE_MIN_CHARACTERS..=PHRASE_MAX_CHARACTERS).contains(&length) && crate::boundary::canonical_tokens(&item.text).len() >= PHRASE_MIN_WORDS && !item.text.contains('\0')
}

/// Identical and bounded single-token edits grouped: each group under its lowest phrase, members in order of identifier.
fn phrase_groups(phrases: &[Phrase]) -> BTreeMap<usize, Vec<&Phrase>> {
    let eligible: Vec<&Phrase> = phrases.iter().filter(|item| phrase_is_eligible(item)).collect();
    let mut parents: Vec<usize> = (0..eligible.len()).collect();
    fn root(parents: &mut [usize], mut index: usize) -> usize {
        while index != parents[index] {
            parents[index] = parents[parents[index]];
            index = parents[index];
        }
        index
    }
    let mut signatures: HashMap<[u8; 32], usize> = HashMap::new();
    for (index, item) in eligible.iter().enumerate() {
        let words = crate::boundary::canonical_tokens(&item.text);
        let mut variants = vec![words.clone()];
        if (NEAR_DUPLICATE_MIN_WORDS..=NEAR_DUPLICATE_MAX_WORDS).contains(&words.len()) {
            for offset in 0..words.len() {
                let mut variant = words.clone();
                variant.remove(offset);
                variants.push(variant);
            }
        }
        for variant in variants {
            let key: [u8; 32] = Sha256::digest(hex_digest(&variant.join("\u{1f}")).as_bytes()).into();
            match signatures.get(&key) {
                Some(other) => {
                    let (a, b) = (root(&mut parents, index), root(&mut parents, *other));
                    if a != b {
                        parents[a.max(b)] = a.min(b);
                    }
                }
                None => {
                    signatures.insert(key, index);
                }
            }
        }
    }
    let mut grouped: BTreeMap<usize, Vec<&Phrase>> = BTreeMap::new();
    for (index, item) in eligible.iter().enumerate() {
        grouped.entry(root(&mut parents, index)).or_default().push(item);
    }
    grouped
}

/// `assign`: each group capped, by a hash, at a few phrases.
#[must_use]
pub fn assign(phrases: &[Phrase]) -> Vec<Assigned<'_>> {
    let mut output = Vec::new();
    for (_, mut members) in phrase_groups(phrases) {
        let identifier = format!("tatoeba:{}", members[0].identifier);
        let split = phrase_split(&identifier);
        members.sort_by_cached_key(|item| hex_digest(&format!("{PHRASE_NAMESPACE}:sample:{}", item.identifier)));
        output.extend(members.into_iter().take(PHRASES_PER_GROUP).map(|phrase| Assigned { phrase, group: identifier.clone(), split }));
    }
    output.sort_by_key(|item| item.phrase.identifier);
    output
}

/// The split of a phrase family, decided by its name alone before anything looks at a model.
fn phrase_split(group: &str) -> &'static str {
    let digest = hex_digest(&format!("{PHRASE_NAMESPACE}:{group}"));
    let bucket = u64::from_str_radix(digest.get(..CHOICE_HEX_DIGITS).unwrap_or_default(), 16).expect("a digest") % PHRASE_SPLIT_BUCKETS;
    match bucket {
        0..60 => "train",
        60..70 => "development",
        70..80 => "calibration",
        80..90 => "test",
        _ => "reserve",
    }
}

/// The receipt of the phrase split: what was kept of the snapshot and how it fell, without reading any model.
pub fn assignment_report(phrases: &[Phrase]) -> Map<String, Value> {
    let assigned = assign(phrases);
    let eligible = phrases.iter().filter(|item| phrase_is_eligible(item)).count();
    let groups = phrase_groups(phrases);
    let mut groups_by_split: BTreeMap<&str, usize> = BTreeMap::new();
    for members in groups.values() {
        *groups_by_split.entry(phrase_split(&format!("tatoeba:{}", members[0].identifier))).or_default() += 1;
    }
    let (mut rows_by_split, mut locale_rows): (BTreeMap<&str, usize>, BTreeMap<&str, usize>) = (BTreeMap::new(), BTreeMap::new());
    let mut lines = Vec::with_capacity(assigned.len());
    for item in &assigned {
        *rows_by_split.entry(item.split).or_default() += 1;
        *locale_rows.entry(item.phrase.locale.as_str()).or_default() += 1;
        lines.push(format!("{}\t{}\t{}", item.phrase.identifier, item.group, item.split));
    }
    let report = serde_json::json!({
        "source_rows": phrases.len(), "eligible_rows": eligible, "retained_rows": assigned.len(), "groups": groups.len(),
        "maximum_group_size": groups.values().map(Vec::len).max().unwrap_or(0), "group_cap": PHRASES_PER_GROUP,
        "groups_by_split": groups_by_split, "rows_by_split": rows_by_split, "locale_rows": locale_rows,
        "assignment_sha256": hex_digest(&lines.join("\n")), "split_namespace": PHRASE_NAMESPACE,
        "model_loaded": false, "metrics_evaluated": false,
        "grouping_scope": "normalized lexical text and bounded single-token edits; not all paraphrases",
    });
    report.as_object().cloned().unwrap_or_default()
}

/// `script_of`.
pub(crate) fn script_of(token: &str) -> Option<&'static str> {
    let lowered = pystr::casefold(&normalize::nfc(token));
    let cyrillic = lowered.chars().any(|character| CYRILLIC.contains(character));
    let latin = lowered.chars().any(|character| character.is_ascii_lowercase());
    match (cyrillic, latin) {
        (true, false) => Some("ru"),
        (false, true) => Some("en"),
        _ => None,
    }
}

/// `shape_of`: case shape, which is what makes the abbreviation reading available.
pub(crate) fn shape_of(token: &str, position: usize) -> &'static str {
    let letters: Vec<char> = token.chars().filter(|character| pystr::isalpha(character.encode_utf8(&mut [0; 4]))).collect();
    if letters.len() >= ACRONYM_MIN_LETTERS && letters.iter().all(|character| pystr::isupper(character.encode_utf8(&mut [0; 4]))) {
        return "upper";
    }
    if token.chars().next().is_some_and(|first| pystr::isupper(first.encode_utf8(&mut [0; 4]))) {
        return if position == 0 { "initial" } else { "inner" };
    }
    "lower"
}

fn split_of(group: &str) -> &'static str {
    let digest = hex_digest(&format!("{SPLIT_NAMESPACE}:{group}"));
    let bucket = u64::from_str_radix(digest.get(..CHOICE_HEX_DIGITS).unwrap_or_default(), 16).expect("a digest") % BUCKET_COUNT;
    if bucket < TEST_SPLIT_CEILING { "test" } else { "train" }
}

/// `read_lexicon`: unigrams of a frozen ARPA word list, refusing anything but the reviewed bytes.
pub(crate) fn read_lexicon(root: &Path, relative: &str, expected: &str) -> Result<BTreeMap<String, i128>, String> {
    let content = std::fs::read(root.join(relative)).map_err(|error| format!("{relative}: {error}"))?;
    if hex(&Sha256::digest(&content)) != expected {
        return Err(format!("frozen word list {relative} checksum mismatch"));
    }
    let text = String::from_utf8_lossy(&content);
    let mut words = BTreeMap::new();
    let mut section = false;
    for line in pystr::splitlines(&text, false) {
        if line == "\\1-grams:" {
            section = true;
            continue;
        }
        if line.starts_with('\\') {
            section = false;
            continue;
        }
        if !section || line.is_empty() {
            continue;
        }
        let Some((count_text, payload)) = line.split_once(' ') else { continue };
        if payload.starts_with('<') {
            continue;
        }
        let Some(count) = ks_text::pynum::py_int(count_text) else { continue };
        let token = pystr::casefold(payload);
        if token.chars().count() >= LEXICON_MIN_CHARACTERS {
            *words.entry(token).or_insert(0) += count;
        }
    }
    Ok(words)
}

type WordLists = HashMap<&'static str, HashSet<String>>;

fn word_lists(root: &Path) -> Result<WordLists, String> {
    let mut lists = HashMap::new();
    for (script, path, expected, _) in LEXICONS {
        lists.insert(script, read_lexicon(root, path, expected)?.into_keys().collect());
    }
    Ok(lists)
}

/// `label`: the language a token was written in: its own alphabet, unless it is the sentence's language in the wrong layout.
fn label(raw: &str, locale: &str, words: &WordLists) -> Result<Option<&'static str>, String> {
    let Some(script) = script_of(raw) else { return Ok(None) };
    let sentence = if locale == "rus" { "ru" } else { "en" };
    if script == sentence {
        return Ok(Some(script));
    }
    let keys = to_keys(raw)?;
    let core = word_core(&keys, script);
    let read = |text: &str| words[sentence].contains(&visible(text, sentence));
    if (read(&keys) || read(&core)) && !words[script].contains(&visible(&core, script)) {
        return Ok(Some(sentence));
    }
    Ok(Some(script))
}

fn lexicon_rows(root: &Path) -> Result<BTreeMap<(String, String), i64>, String> {
    let mut counts: BTreeMap<(String, String), i64> = BTreeMap::new();
    for (script, path, expected, _) in LEXICONS {
        for (word, frequency) in read_lexicon(root, path, expected)? {
            if script_of(&word) != Some(script) {
                continue;
            }
            let keys = to_keys(&word)?;
            if keys.is_empty() || keys.chars().count() > MAXIMUM_TOKEN_CHARACTERS {
                continue;
            }
            let bits = 128 - (frequency.unsigned_abs()).leading_zeros();
            let weight = i64::from(bits.clamp(1, MAXIMUM_LEXICON_WEIGHT));
            let entry = counts.entry((script.to_owned(), keys)).or_insert(0);
            *entry = (*entry).max(weight);
        }
    }
    Ok(counts)
}

type TokenRow = (String, String, String, BTreeMap<String, i64>);

/// `rows`: one per (split, script, key sequence) with shape counts, then the word-list rows.
fn corpus_rows(root: &Path) -> Result<Vec<TokenRow>, String> {
    let phrases = load_phrases(root)?;
    let lists = word_lists(root)?;
    let mut counts: BTreeMap<(String, String, String), BTreeMap<String, i64>> = BTreeMap::new();
    for assigned in assign(&phrases) {
        let split = split_of(&assigned.group);
        let text = normalize::nfc(&assigned.phrase.text);
        for (position, raw) in pystr::split(&text).into_iter().enumerate() {
            if raw.chars().count() > MAXIMUM_TOKEN_CHARACTERS {
                continue;
            }
            let Some(script) = label(raw, &assigned.phrase.locale, &lists)? else { continue };
            let keys = to_keys(raw)?;
            if keys.is_empty() {
                continue;
            }
            *counts.entry((split.to_owned(), script.to_owned(), keys)).or_default().entry(shape_of(raw, position).to_owned()).or_insert(0) += 1;
        }
    }
    let mut rows: Vec<TokenRow> = counts.into_iter().map(|((split, script, keys), shapes)| (split, script, keys, shapes)).collect();
    for ((script, keys), weight) in lexicon_rows(root)? {
        rows.push((LEXICON.to_owned(), script, keys, BTreeMap::from([("lower".to_owned(), weight)])));
    }
    Ok(rows)
}

fn tokens_content(rows: &[TokenRow]) -> Vec<u8> {
    let mut raw = Vec::new();
    for (split, script, keys, shapes) in rows {
        raw.extend(crate::artifact::canonical(&json!({"split": split, "script": script, "keys": keys, "shapes": shapes})));
    }
    raw
}

fn pretty(value: &Value) -> Vec<u8> {
    (ks_text::json_dumps(value, &ks_text::JsonOptions { ensure_ascii: false, indent: Some(ks_text::Indent::Spaces(2)), sort_keys: true, ..ks_text::JsonOptions::default() }) + "\n").into_bytes()
}

fn pins(root: &Path, files: &[&str]) -> Result<Map<String, Value>, String> {
    files.iter().map(|file| Ok(((*file).to_owned(), json!(checksum(&root.join(file))?)))).collect()
}

/// `report` of `ortho_corpus`.
fn corpus_receipt(root: &Path, rows: &[TokenRow], tokens_sha: &str) -> Result<Value, String> {
    let (mut by_split, mut by_script, mut occurrences): (BTreeMap<&str, i64>, BTreeMap<String, i64>, BTreeMap<String, i64>) = Default::default();
    for (split, script, _, shapes) in rows {
        *by_split.entry(split).or_default() += 1;
        *by_script.entry(format!("{split}:{script}")).or_default() += 1;
        *occurrences.entry(format!("{split}:{script}")).or_default() += shapes.values().sum::<i64>();
    }
    let mut provenance = pins(root, &CORPUS_CODE)?;
    provenance.extend(pins(root, &[SOURCE, "model/context_v2/corpus-receipt.json", LEXICONS[0].1, LEXICONS[1].1])?);
    Ok(json!({
        "schema_version": EVIDENCE_SCHEMA,
        "namespace": SPLIT_NAMESPACE,
        "description": "Key-space token types from the frozen CC0 phrase snapshot, stored whole with their punctuation, Russian commas and full stops on their own key; the frozen Onboard word lists follow as rows that only add characters to the counts. What the engine serves the model is derived per layout by `served`.",
        "label_rule": "the alphabet of the token itself, except that a token in the other alphabet inside a sentence of a language, whose reading in that language is a word of its frozen word list while the token is no word of its own alphabet's list, is that language typed in the wrong layout; the reading is taken of the whole token and of its letter core in the layout it was typed in, and the token's own word is that core",
        "split_rule": format!("context-v2 phrase groups (tools/context_corpus.py); bucket = int(sha256(namespace + ':' + group)[:{CHOICE_HEX_DIGITS}], 16) mod {BUCKET_COUNT}; test below {TEST_SPLIT_CEILING}, train otherwise"),
        "lexicon_weight": format!("max(1, min({MAXIMUM_LEXICON_WEIGHT}, frequency.bit_length()))"),
        "types_by_split": by_split,
        "types_by_split_script": by_script,
        "occurrences_by_split_script": occurrences,
        "maximum_token_characters": MAXIMUM_TOKEN_CHARACTERS,
        "tokens_sha256": tokens_sha,
        "provenance": provenance,
    }))
}

fn asked(keys: &str, direction: &str, result: &mut HashMap<&'static str, HashSet<String>>) {
    let target_script = other(direction);
    let (source, target) = readings(keys, direction);
    let script_key = |name: &str| if name == "en" { "en" } else { "ru" };
    result.get_mut(script_key(direction)).expect("a script").insert(visible(keys, direction));
    for (text, script) in [(&source, direction), (&target, target_script)] {
        if text.contains('-') {
            result.get_mut(script_key(script)).expect("a script").extend(text.split('-').filter(|part| !part.is_empty()).map(str::to_owned));
        }
    }
    if direction == "ru" && keys.chars().count() > 1 && keys.starts_with('.') {
        result.get_mut("en").expect("a script").insert(keys.get(1..).unwrap_or_default().to_owned());
    }
    if pystr::isalpha(&target) {
        let set = result.get_mut(script_key(target_script)).expect("a script");
        set.insert(target.clone());
        let letters: Vec<char> = target.chars().collect();
        if letters.len() >= CONTEXT_TYPO_MIN_CHARACTERS {
            for index in 0..letters.len() {
                let mut variant = letters.clone();
                variant.remove(index);
                set.insert(variant.into_iter().collect());
            }
        }
    }
}

/// `queried`: every word the population filter and the token features ask a dictionary about for one served token.
fn queried(keys: &str, direction: &str, result: &mut HashMap<&'static str, HashSet<String>>) {
    let characters: Vec<char> = keys.chars().collect();
    let (mut longest, mut run) = (0_i64, 0_i64);
    for (index, character) in characters.iter().enumerate() {
        run = if index > 0 && characters[index - 1] == *character { run + 1 } else { 1 };
        longest = longest.max(run);
    }
    let mut variants: BTreeSet<String> = BTreeSet::from([keys.to_owned()]);
    for minimum in MIN_STRETCH_RUN..=longest.max(MIN_STRETCH_RUN) {
        for hyphens in [false, true] {
            variants.extend(stretch_readings(keys, Some(MIN_COLLAPSED_RUN), Some(minimum), hyphens));
        }
    }
    for variant in &variants {
        asked(variant, direction, result);
    }
}

fn empty_forms() -> HashMap<&'static str, HashSet<String>> {
    HashMap::from([("en", HashSet::new()), ("ru", HashSet::new())])
}

/// `forms` of `ortho_known`.
fn known_forms(rows: &[Row]) -> HashMap<&'static str, HashSet<String>> {
    let mut result = empty_forms();
    for row in rows {
        if row.split == LEXICON {
            continue;
        }
        let mut typed: Vec<(String, &str)> = SCRIPTS.iter().map(|direction| (row.keys.clone(), *direction)).collect();
        if let Some(synthetic) = ortho::extra_key_variant(&row.script, &row.keys) {
            typed.push((synthetic, other(&row.script)));
        }
        for (text, direction) in typed {
            for token in served(&text, direction) {
                if token.chars().all(|key| KEYS.contains(&key)) {
                    queried(&token, direction, &mut result);
                }
            }
        }
    }
    result
}

/// `reference_models`: the engine's models over the reference word lists and dictionaries of Layout Intent v1.
fn reference_models(root: &Path) -> Result<HashMap<&'static str, LanguageModel>, String> {
    let frozen = crate::intent::frozen_dictionaries(root)?;
    let word_roots = [root.join("model/intent_v1/sources")];
    let mut models = HashMap::new();
    for (script, _, _, locale) in LEXICONS {
        let supplement = ks_lang::lexicon_supplement::supplement_words(&root.join("resources"), locale).map_err(|error| error.name.to_string())?;
        let speller = ks_hunspell::HunspellDictionary::in_roots(locale, std::slice::from_ref(&frozen));
        let model = LanguageModel::load(locale, supplement.iter(), &word_roots, Box::new(speller));
        if !model.speller.available() {
            return Err(format!("no {script} speller: refusing to freeze a partial verdict"));
        }
        models.insert(script, model);
    }
    Ok(models)
}

/// `build` and `verdict`: only the known forms are kept: the answer for anything absent is "not known".
fn verdict(models: &HashMap<&'static str, LanguageModel>, forms: HashMap<&'static str, HashSet<String>>) -> Value {
    let mut payload = Map::new();
    for script in SCRIPTS {
        let mut known: Vec<&String> = forms[script].iter().filter(|word| models[script].score(word).known).collect();
        known.sort();
        payload.insert(script.to_owned(), json!(known));
    }
    Value::Object(payload)
}

fn known_receipt(root: &Path, payload: &Value, evidence_sha: &str, tokens: &Path) -> Result<Value, String> {
    let config = read_json(&root.join("model/intent_v1/config.json"))?;
    let hunspell = &config["external_evaluation"]["hunspell"];
    let mut provenance = pins(root, &KNOWN_CODE)?;
    provenance.insert("model/ortho_v1/tokens.jsonl.gz".to_owned(), json!(checksum(tokens)?));
    for locale in ["en_US", "ru_RU"] {
        let path = format!("resources/lexicon-supplement-{locale}.json");
        if root.join(&path).is_file() {
            provenance.insert(path.clone(), json!(checksum(&root.join(&path))?));
        }
    }
    Ok(json!({
        "schema_version": 1,
        "description": "Whether the dictionary of each layout knows each token the engine can serve the orthotactic model, each half of a hyphenated reading and the word after a leading dot, frozen so a replay needs no speller. Absent means not known.",
        "evidence_sha256": evidence_sha,
        "tokens_sha256": checksum(tokens)?,
        "hunspell": {"en_US": hunspell["en_US"], "ru_RU": hunspell["ru_RU"]},
        "word_lists": {"en": LEXICONS[0].1, "ru": LEXICONS[1].1},
        "known_by_script": {"en": payload["en"].as_array().map_or(0, Vec::len), "ru": payload["ru"].as_array().map_or(0, Vec::len)},
        "scope": "LanguageModel.load(locale, supplement_words(locale)).score(token).known with the reference word lists and Hunspell dictionaries of Layout Intent v1, which is what the runtime means by `known`; a machine whose speller differs licenses more or fewer conversions than this evidence measured",
        "provenance": provenance,
    }))
}

/// `derive` of `ortho_web_lexicon`: `[form, rank]` of every lowercase Cyrillic word with enough letters, and of every compound.
fn derive_web_forms(source: &Path) -> Result<Vec<u8>, String> {
    if checksum(source)? != WEB_SOURCE_SHA256 {
        return Err("web vocabulary source is not the pinned file".to_owned());
    }
    let raw = String::from_utf8(gz_read(source)?).map_err(|error| error.to_string())?;
    let word = |text: &str| !text.is_empty() && text.chars().all(|character| ('а'..='я').contains(&character) || character == 'ё');
    let mut forms = Vec::new();
    for (rank, line) in raw.split('\n').skip(1).filter(|line| !line.is_empty()).enumerate() {
        let form = line.split(' ').next().unwrap_or("");
        let compound = form.contains('-') && form.split('-').all(word);
        if (form.chars().count() >= WEB_MIN_CHARACTERS && word(form)) || compound {
            forms.push(json!([form, rank]));
        }
    }
    let mut text = ks_text::json_dumps(&json!({"schema_version": WEB_SCHEMA, "forms": forms}), &ks_text::JsonOptions { ensure_ascii: false, separators: Some((",".to_owned(), ":".to_owned())), ..ks_text::JsonOptions::default() });
    text.push('\n');
    Ok(text.into_bytes())
}

fn read_set_rows(path: &Path) -> Result<Vec<Row>, String> {
    read_rows(&String::from_utf8(gz_read(path)?).map_err(|error| error.to_string())?)
}

fn read_known(path: &Path) -> Result<HashMap<String, HashSet<String>>, String> {
    let evidence: Value = serde_json::from_slice(&gz_read(path)?).map_err(|error| error.to_string())?;
    Ok(evidence.as_object().ok_or("dictionary evidence")?.iter().map(|(script, words)| (script.clone(), words.as_array().map_or_else(HashSet::new, |words| words.iter().filter_map(Value::as_str).map(str::to_owned).collect()))).collect())
}

/// `seen`: served keys, every run read once, of every row of the snapshot and of every opened set, and of hand-written cases.
fn seen(root: &Path) -> Result<HashSet<String>, String> {
    let directory = root.join(DIRECTORY);
    let mut sequences: Vec<String> = read_set_rows(&directory.join("tokens.jsonl.gz"))?.into_iter().filter(|row| row.split != LEXICON).map(|row| row.keys).collect();
    for set in HISTORY.iter().map(|entry| entry.0).chain(DEVELOPMENT_SETS) {
        sequences.extend(read_set_rows(&directory.join(set).join("tokens.jsonl.gz"))?.into_iter().map(|row| row.keys));
    }
    let mut written: BTreeSet<String> = ortho::AUTHORED_KEYS.iter().map(|keys| (*keys).to_owned()).collect();
    for word in LOOKED_AT_WORDS {
        written.insert(to_keys(word)?);
    }
    sequences.extend(written);
    let mut result = HashSet::new();
    for keys in &sequences {
        for direction in SCRIPTS {
            result.extend(served(keys, direction).iter().map(|token| read_once(token, MIN_STRETCH_RUN)));
        }
    }
    Ok(result)
}

fn type_counts(labels: &BTreeMap<ortho::Label, bool>) -> BTreeMap<String, i64> {
    let mut counted = BTreeMap::new();
    for (label, negative) in labels {
        *counted.entry(format!("{}:{}_types", label.0, if *negative { "negative" } else { "positive" })).or_insert(0) += 1;
    }
    counted
}

/// `population` of `ortho_independent`: the independent types and the count of those the development saw.
fn independent_population(root: &Path, rows: &[Row], known: &HashMap<String, HashSet<String>>, minimum: usize) -> Result<(BTreeMap<ortho::Label, bool>, BTreeMap<String, i64>), String> {
    let references: Vec<&Row> = rows.iter().collect();
    let labels = ortho::population(&references, "test", known, minimum);
    let excluded = seen(root)?;
    let (mut kept, mut dropped) = (BTreeMap::new(), BTreeMap::new());
    for (label, negative) in labels {
        if excluded.contains(&read_once(&label.1, MIN_STRETCH_RUN)) {
            *dropped.entry(format!("{}:{}_types", label.0, if negative { "negative" } else { "positive" })).or_insert(0) += 1;
        } else {
            kept.insert(label, negative);
        }
    }
    Ok((kept, dropped))
}

fn report_line(differences: &mut usize, name: &str, equal: bool) {
    println!("{name}: {}", if equal { "same" } else { "DIFFERS" });
    *differences += usize::from(!equal);
}

/// Everything rebuilt from the frozen sources and compared with the frozen files. A digest of the code in a receipt written
/// by the Python tools names files that are not in this tree, so the receipts are compared without their provenance.
pub fn verify(root: &Path) -> Result<(), String> {
    let directory = root.join(DIRECTORY);
    let mut differences = 0;
    let rows = corpus_rows(root)?;
    let content = tokens_content(&rows);
    report_line(&mut differences, "tokens", content == gz_read(&directory.join("tokens.jsonl.gz"))?);
    let mut receipt = corpus_receipt(root, &rows, &checksum(&directory.join("tokens.jsonl.gz"))?)?;
    let mut recorded = read_json(&directory.join("corpus-receipt.json"))?;
    receipt["provenance"] = Value::Null;
    recorded["provenance"] = Value::Null;
    report_line(&mut differences, "corpus receipt", receipt == recorded);
    let token_rows = read_set_rows(&directory.join("tokens.jsonl.gz"))?;
    let models = reference_models(root)?;
    let evidence = verdict(&models, known_forms(&token_rows));
    report_line(&mut differences, "known evidence", evidence == serde_json::from_slice::<Value>(&gz_read(&directory.join("known-evidence.json.gz"))?).map_err(|error| error.to_string())?);
    let mut known = known_receipt(root, &evidence, &checksum(&directory.join("known-evidence.json.gz"))?, &directory.join("tokens.jsonl.gz"))?;
    let mut recorded_known = read_json(&directory.join("known-receipt.json"))?;
    known["provenance"] = Value::Null;
    recorded_known["provenance"] = Value::Null;
    report_line(&mut differences, "known receipt", known == recorded_known);
    report_line(&mut differences, "web lexicon receipt", read_json(&directory.join("web-lexicon-receipt.json"))?["forms_sha256"].as_str() == Some(&checksum(&directory.join("web-lexicon-ru.json.gz"))?));
    report_line(&mut differences, "prose lexicon receipt", read_json(&directory.join("prose-lexicon-receipt.json"))?["forms_sha256"].as_str() == Some(&checksum(&directory.join("prose-lexicon-ru.json.gz"))?));
    for (set, report_path) in HISTORY {
        let (folder, report) = (directory.join(set), read_json(&directory.join(report_path))?);
        let receipt = read_json(&folder.join("receipt.json"))?;
        let described = &report["independent_test"];
        let counts: BTreeMap<String, Value> = report["corpus_test"]["counts"].as_object().ok_or("counts")?.iter().filter(|(name, _)| name.ends_with("negative_types") || name.ends_with("positive_types")).map(|(name, value)| (name.clone(), value.clone())).collect();
        report_line(&mut differences, set, described["receipt_sha256"].as_str() == Some(&checksum(&folder.join("receipt.json"))?) && receipt["tokens_sha256"].as_str() == Some(&checksum(&folder.join("tokens.jsonl.gz"))?) && receipt["evidence_sha256"].as_str() == Some(&checksum(&folder.join("known-evidence.json.gz"))?) && described["source"] == receipt["source"] && json!(counts) == receipt["types"]);
    }
    let active = directory.join("independent");
    let active_rows = read_set_rows(&active.join("tokens.jsonl.gz"))?;
    let active_evidence = verdict(&models, {
        let mut forms = empty_forms();
        for row in &active_rows {
            for direction in SCRIPTS {
                for token in served(&row.keys, direction) {
                    if token.chars().all(|key| KEYS.contains(&key)) {
                        queried(&token, direction, &mut forms);
                    }
                }
            }
        }
        forms
    });
    report_line(&mut differences, "independent evidence", active_evidence == serde_json::from_slice::<Value>(&gz_read(&active.join("known-evidence.json.gz"))?).map_err(|error| error.to_string())?);
    let known_active = read_known(&active.join("known-evidence.json.gz"))?;
    let minimum = (read_json(&directory.join("config.json"))?["minimum_length"].as_u64().ok_or("minimum_length")?).to::<usize>();
    let (kept, dropped) = independent_population(root, &active_rows, &known_active, minimum)?;
    let recorded_set = read_json(&active.join("receipt.json"))?;
    report_line(&mut differences, "independent set", recorded_set["set"].as_str() == Some(SET_NAME) && recorded_set["schema_version"].as_i64().is_some() && json!(type_counts(&kept)) == recorded_set["types"] && json!(dropped) == recorded_set["excluded_as_seen"] && recorded_set["tokens_sha256"].as_str() == Some(&checksum(&active.join("tokens.jsonl.gz"))?) && recorded_set["evidence_sha256"].as_str() == Some(&checksum(&active.join("known-evidence.json.gz"))?) && recorded_set["snapshot_tokens_sha256"].as_str() == Some(&checksum(&directory.join("tokens.jsonl.gz"))?));
    let report = read_json(&directory.join("report.json"))?;
    let outcome = ortho::independent_outcome(root, &kept, &known_active)?;
    report_line(&mut differences, "independent test report", outcome == report["corpus_test"]);
    if differences != 0 {
        return Err(format!("{differences} inputs of the orthotactic model differ from the frozen files"));
    }
    println!("the inputs of the orthotactic model reproduce");
    Ok(())
}

/// A new generation: writes what is not there yet; a file that exists is never overwritten.
pub fn freeze(root: &Path, output: &Path) -> Result<(), String> {
    std::fs::create_dir_all(output).map_err(|error| format!("{}: {error}", output.display()))?;
    let (tokens, receipt) = (output.join("tokens.jsonl.gz"), output.join("corpus-receipt.json"));
    if tokens.exists() || receipt.exists() {
        return Err("do not overwrite a frozen split".to_owned());
    }
    let rows = corpus_rows(root)?;
    std::fs::write(&tokens, gz_write(&tokens_content(&rows))?).map_err(|error| error.to_string())?;
    std::fs::write(&receipt, pretty(&corpus_receipt(root, &rows, &checksum(&tokens)?)?)).map_err(|error| error.to_string())?;
    let token_rows = read_set_rows(&tokens)?;
    let models = reference_models(root)?;
    let evidence = verdict(&models, known_forms(&token_rows));
    let (evidence_path, known_path) = (output.join("known-evidence.json.gz"), output.join("known-receipt.json"));
    std::fs::write(&evidence_path, gz_write(&crate::artifact::canonical(&evidence))?).map_err(|error| error.to_string())?;
    std::fs::write(&known_path, pretty(&known_receipt(root, &evidence, &checksum(&evidence_path)?, &tokens)?)).map_err(|error| error.to_string())?;
    Ok(())
}

const PROSE_LEADING: &str = "—–-«»\"'„“” ";
const PROSE_DUMP: &str = "ruwikisource-20260901";
const PROSE_FILES: [(&str, &str); 4] = [("pages-articles.xml.bz2", "900444fd4dea093a918908390ec93130575fa530dddbec327c89313a65f7fc81"), ("page.sql.gz", "4fc8ee33447f4c864086afbbbb492e46486224dbd176e37f22933dac6a2b5b5d"), ("categorylinks.sql.gz", "356b33624d80ca13f82ce3dc50fbab53229d42bae96ed70019e792e564150a7e"), ("linktarget.sql.gz", "5927a112017995d8632f971fe45653e79abb9ff584ea72be5447483dc29149de")];
const PROSE_CATEGORY: &str = "Проза_по_авторам";
const RUSSIAN_WRITERS: &str = "Писатели_на_русском_языке";
const EXCLUDED_AUTHORS: [&str; 3] = ["Антон_Павлович_Чехов", "Николай_Семёнович_Лесков", "Аркадий_Тимофеевич_Аверченко"];
const PRE_REFORM_PAGE: &str = "/ДО";
const LAST_DEATH_YEAR: i64 = 1925;
const CATEGORY_DEPTH: i64 = 3;
const WORDS_SCALE: f64 = 1_000_000_000.0;
const ARTICLE_NAMESPACE: i64 = 0;
const CATEGORY_NAMESPACE: i64 = 14;
const AUTHOR_NAMESPACE: i64 = 102;

fn compiled(pattern: &str) -> regex::Regex {
    regex::Regex::new(pattern).expect("a valid pattern")
}

/// `wikitext_sentences`: the sentences of a Wikisource page, without templates, notes, headings and back matter.
pub fn wikitext_sentences(content: &str) -> Vec<String> {
    static PATTERNS: std::sync::LazyLock<[regex::Regex; 9]> = std::sync::LazyLock::new(|| [compiled(r"(?s)<!--.*?-->"), compiled(r"(?is)<ref[^>/]*/>|<ref[^>]*>.*?</ref>"), compiled(r"\{\{[^{}]*\}\}"), compiled(r"(?s)\{\|.*?\|\}"), compiled(r"(?m)^==+\s*(Примечания|Комментарии|См\. также|Ссылки)\s*==+|\[\[Категория:"), compiled(r"__[A-Z]+__"), compiled(r"\[\[(?:[^|\]]*\|)?([^\]]*)\]\]"), compiled(r"\[(?:https?:)?//\S+\s*([^\]]*)\]"), compiled(r"<[^>]+>")]);
    static HEADING: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| compiled(r"^=+.*=+$"));
    let [comment, reference, template, table, back, magic, link, external, tag] = &*PATTERNS;
    let mut text = reference.replace_all(&comment.replace_all(content, ""), "").into_owned();
    loop {
        let next = template.replace_all(&text, "").into_owned();
        if next == text {
            break;
        }
        text = next;
    }
    text = table.replace_all(&text, "").into_owned();
    if let Some(found) = back.find(&text) {
        text.truncate(found.start());
    }
    let without_magic = magic.replace_all(&text, "");
    let without_links = link.replace_all(&without_magic, "$1");
    text = external.replace_all(&without_links, "$1").into_owned();
    text = crate::messages::unescape(&tag.replace_all(&text.replace("'''", "").replace("''", ""), ""));
    let mut sentences = Vec::new();
    for line in pystr::splitlines(&text, false) {
        let line = pystr::strip(pystr::lstrip_chars(pystr::strip(line), "*#:;"));
        if line.is_empty() || HEADING.is_match(line) {
            continue;
        }
        let characters: Vec<char> = line.chars().collect();
        let (mut start, mut index) = (0, 0);
        let mut parts: Vec<String> = Vec::new();
        while index < characters.len() {
            let is_space = |character: char| pystr::isspace(character.encode_utf8(&mut [0; 4]));
            if index > 0 && is_space(characters[index]) && ".!?…".contains(characters[index - 1]) {
                let mut end = index;
                while end < characters.len() && is_space(characters[end]) {
                    end += 1;
                }
                if characters.get(end).is_some_and(|next| "—–«\"([".contains(*next) || next.is_ascii_uppercase() || ('А'..='Я').contains(next) || *next == 'Ё') {
                    parts.push(characters[start..index].iter().collect());
                    start = end;
                    index = end;
                    continue;
                }
            }
            index += 1;
        }
        parts.push(characters[start..].iter().collect());
        sentences.extend(parts.iter().map(|part| pystr::strip_chars(part, PROSE_LEADING).to_owned()).filter(|sentence| !sentence.is_empty()));
    }
    sentences
}

#[derive(Clone, Debug, PartialEq)]
enum SqlValue {
    Null,
    Number(i64),
    Text(Vec<u8>),
}

impl SqlValue {
    fn number(&self) -> i64 {
        if let SqlValue::Number(value) = self { *value } else { 0 }
    }

    fn text(&self) -> String {
        if let SqlValue::Text(value) = self { String::from_utf8_lossy(value).into_owned() } else { String::new() }
    }
}

fn sql_field(line: &[u8], mut index: usize) -> (SqlValue, usize) {
    if line[index] != b'\'' {
        let mut end = index;
        while !matches!(line[end], b',' | b')') {
            end += 1;
        }
        let token = std::str::from_utf8(&line[index..end]).unwrap_or("");
        let value = if token == "NULL" { SqlValue::Null } else { token.parse().map_or_else(|_| SqlValue::Text(token.as_bytes().to_vec()), SqlValue::Number) };
        return (value, end);
    }
    index += 1;
    let mut chunk = Vec::new();
    while line[index] != b'\'' {
        if line[index] == b'\\' {
            index += 1;
            chunk.push(match line[index] {
                b'n' => b'\n',
                b't' => b'\t',
                b'r' => b'\r',
                b'0' => 0,
                b'Z' => 0x1a,
                other => other,
            });
        } else {
            chunk.push(line[index]);
        }
        index += 1;
    }
    (SqlValue::Text(chunk), index + 1)
}

/// `sql_rows`: the rows of a `MediaWiki` SQL dump of a table, by the column names of its CREATE TABLE statement.
fn sql_rows(path: &Path, table: &str, wanted: &[&str], mut each: impl FnMut(&[SqlValue])) -> Result<(), String> {
    use std::io::BufRead;
    let (create, start) = (format!("CREATE TABLE `{table}` (").into_bytes(), format!("INSERT INTO `{table}` VALUES ").into_bytes());
    let mut reader = std::io::BufReader::new(MultiGzDecoder::new(std::fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?));
    let mut columns: Vec<String> = Vec::new();
    let mut declaring = false;
    let mut line = Vec::new();
    let mut places: Vec<usize> = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line).map_err(|error| error.to_string())? == 0 {
            break;
        }
        if line.starts_with(&create) {
            declaring = true;
        } else if declaring && line.first().is_some_and(u8::is_ascii_whitespace) && line.contains(&b'`') && String::from_utf8_lossy(&line).trim_start().starts_with('`') {
            let text = String::from_utf8_lossy(&line);
            if let Some(name) = text.trim_start().strip_prefix('`').and_then(|rest| rest.split('`').next()) {
                columns.push(name.to_owned());
            }
        } else if line.starts_with(&start) {
            declaring = false;
            if places.is_empty() {
                for name in wanted {
                    places.push(columns.iter().position(|column| column == name).ok_or_else(|| format!("{table} has no column {name}"))?);
                }
            }
            let mut index = start.len();
            let mut row: Vec<SqlValue> = Vec::with_capacity(columns.len());
            let mut picked: Vec<SqlValue> = vec![SqlValue::Null; places.len()];
            while index < line.len() {
                if line[index] != b'(' {
                    index += 1;
                    continue;
                }
                index += 1;
                row.clear();
                loop {
                    let (value, next) = sql_field(&line, index);
                    row.push(value);
                    let closing = line[next] == b')';
                    index = next + 1;
                    if closing {
                        break;
                    }
                }
                for (slot, place) in picked.iter_mut().zip(&places) {
                    slot.clone_from(&row[*place]);
                }
                each(&picked);
            }
        } else {
            declaring = declaring && !line.starts_with(b")");
        }
    }
    Ok(())
}

fn xml_text(raw: &str) -> String {
    raw.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#039;", "'").replace("&#13;", "\r").replace("&#10;", "\n").replace("&#9;", "\t").replace("&amp;", "&")
}

/// `articles`: (page id, revision id, wikitext) of the wanted pages of a pages-articles dump.
fn articles(path: &Path, wanted: &HashSet<i64>, mut each: impl FnMut(i64, i64, String)) -> Result<(), String> {
    use std::io::BufRead;
    let mut reader = std::io::BufReader::with_capacity(1 << 20, bzip2_rs::DecoderReader::new(std::io::BufReader::new(std::fs::File::open(path).map_err(|error| error.to_string())?)));
    let mut line = String::new();
    let (mut page, mut revision): (Option<i64>, Option<i64>) = (None, None);
    let mut in_revision = false;
    let mut text: Option<String> = None;
    let value = |line: &str, tag: &str| -> Option<i64> { pystr::strip(line).strip_prefix(&format!("<{tag}>"))?.strip_suffix(&format!("</{tag}>"))?.parse().ok() };
    loop {
        line.clear();
        if reader.read_line(&mut line).map_err(|error| error.to_string())? == 0 {
            return Ok(());
        }
        if let Some(body) = text.as_mut() {
            match line.find("</text>") {
                Some(end) => {
                    body.push_str(line.get(..end).unwrap_or_default());
                    let content = xml_text(&std::mem::take(body));
                    text = None;
                    if let (Some(id), Some(number)) = (page, revision)
                        && wanted.contains(&id) {
                            each(id, number, content);
                        }
                }
                None => body.push_str(&line),
            }
            continue;
        }
        let trimmed = pystr::strip(&line);
        if trimmed == "<page>" {
            (page, revision, in_revision) = (None, None, false);
        } else if trimmed == "<revision>" {
            in_revision = true;
        } else if page.is_none() && !in_revision {
            page = value(&line, "id");
        } else if in_revision && revision.is_none() && value(&line, "id").is_some() {
            revision = value(&line, "id");
        } else if trimmed.starts_with("<text") {
            if trimmed.ends_with("/>") {
                if let (Some(id), Some(number)) = (page, revision)
                    && wanted.contains(&id) {
                        each(id, number, String::new());
                    }
                continue;
            }
            let start = line.find('>').map_or(0, |place| place + 1);
            match line.get(start..).unwrap_or_default().find("</text>") {
                Some(end) => {
                    let content = xml_text(line.get(start..start + end).unwrap_or_default());
                    if let (Some(id), Some(number)) = (page, revision)
                        && wanted.contains(&id) {
                            each(id, number, content);
                        }
                }
                None => text = Some(line.get(start..).unwrap_or_default().to_owned()),
            }
        }
    }
}

fn prose_token(raw: &str) -> &str {
    let word = |character: char| character == '-' || character == '_' || pystr::isalnum(character.encode_utf8(&mut [0; 4]));
    pystr::strip_chars(raw.trim_start_matches(|character| !word(character)).trim_end_matches(|character| !word(character)), "-")
}

fn russian_form(token: &str) -> bool {
    !token.is_empty() && token.chars().all(|character| ('а'..='я').contains(&character) || character == 'ё')
}

/// `derive` of `ortho_prose_lexicon`: the forms of the selected public-domain pages and what they were selected from.
pub fn derive_prose(directory: &Path) -> Result<(BTreeMap<String, i64>, Value), String> {
    for (name, digest) in PROSE_FILES {
        if checksum(&directory.join(format!("{PROSE_DUMP}-{name}")))? != digest {
            return Err(format!("{PROSE_DUMP}-{name} is not the pinned dump file"));
        }
    }
    let file = |name: &str| directory.join(format!("{PROSE_DUMP}-{name}"));
    let mut titles: HashMap<i64, (i64, String, bool)> = HashMap::new();
    let mut order: Vec<i64> = Vec::new();
    sql_rows(&file("page.sql.gz"), "page", &["page_id", "page_namespace", "page_title", "page_is_redirect"], |row| {
        order.push(row[0].number());
        titles.insert(row[0].number(), (row[1].number(), row[2].text(), row[3].number() != 0));
    })?;
    let by_title: HashMap<(i64, String), i64> = titles.iter().map(|(page, (namespace, title, _))| ((*namespace, title.clone()), *page)).collect();
    let mut targets: HashMap<i64, String> = HashMap::new();
    sql_rows(&file("linktarget.sql.gz"), "linktarget", &["lt_id", "lt_namespace", "lt_title"], |row| {
        if row[1].number() == CATEGORY_NAMESPACE {
            targets.insert(row[0].number(), row[2].text());
        }
    })?;
    let mut members: HashMap<String, Vec<(i64, String)>> = HashMap::new();
    let mut parents: HashMap<i64, Vec<String>> = HashMap::new();
    sql_rows(&file("categorylinks.sql.gz"), "categorylinks", &["cl_target_id", "cl_from", "cl_type"], |row| {
        if let Some(category) = targets.get(&row[0].number()) {
            members.entry(category.clone()).or_default().push((row[1].number(), row[2].text()));
            parents.entry(row[1].number()).or_default().push(category.clone());
        }
    })?;
    let death = compiled(r"^Умершие_в_(\d+)_году$");
    let mut died: HashMap<String, i64> = HashMap::new();
    for page in &order {
        let (namespace, title, redirect) = &titles[page];
        let empty = Vec::new();
        let categories = parents.get(page).unwrap_or(&empty);
        let year = categories.iter().find_map(|category| death.captures(category).and_then(|found| found[1].parse::<i64>().ok()));
        if let (AUTHOR_NAMESPACE, false, Some(year), true) = (*namespace, *redirect, year, categories.iter().any(|category| category == RUSSIAN_WRITERS)) {
            died.insert(title.clone(), year);
        }
    }
    let missing = (ARTICLE_NAMESPACE, String::new(), true);
    let mut authors: BTreeMap<String, String> = BTreeMap::new();
    for (page, kind) in members.get(PROSE_CATEGORY).into_iter().flatten() {
        let (namespace, category, _) = titles.get(page).unwrap_or(&missing);
        if kind != "subcat" || *namespace != CATEGORY_NAMESPACE {
            continue;
        }
        let Some(own) = by_title.get(&(CATEGORY_NAMESPACE, category.clone())) else { continue };
        for parent in parents.get(own).into_iter().flatten() {
            if died.get(parent).copied().unwrap_or(LAST_DEATH_YEAR + 1) <= LAST_DEATH_YEAR && !EXCLUDED_AUTHORS.contains(&parent.as_str()) {
                authors.insert(category.clone(), parent.clone());
            }
        }
    }
    let mut chosen: HashMap<i64, (String, String)> = HashMap::new();
    for (category, author) in &authors {
        let mut frontier = vec![(category.clone(), 0)];
        let mut seen: HashSet<String> = HashSet::from([category.clone()]);
        while let Some((current, depth)) = frontier.pop() {
            for (page, kind) in members.get(&current).into_iter().flatten() {
                let (namespace, title, redirect) = titles.get(page).unwrap_or(&missing);
                if kind == "page" && *namespace == ARTICLE_NAMESPACE && !redirect && !title.ends_with(PRE_REFORM_PAGE) {
                    chosen.entry(*page).or_insert_with(|| (category.clone(), author.clone()));
                } else if kind == "subcat" && *namespace == CATEGORY_NAMESPACE && depth < CATEGORY_DEPTH && !seen.contains(title) {
                    seen.insert(title.clone());
                    frontier.push((title.clone(), depth + 1));
                }
            }
        }
    }
    let header = compiled(r"(?s)\{\{Отексте(.*?)\n\}\}");
    let field = compiled(r"\|\s*([А-ЯЁA-Z]+)\s*=\s*([^|\n]*)");
    let pre_reform = compiled("[ѣѢіІѳѲѵѴ]");
    let wanted: HashSet<i64> = chosen.keys().copied().collect();
    let mut pages: BTreeMap<String, (i64, String)> = BTreeMap::new();
    let mut dropped: BTreeMap<&str, i64> = BTreeMap::new();
    articles(&file("pages-articles.xml.bz2"), &wanted, |page, revision, content| {
        let (_, author) = &chosen[&page];
        let mut fields: HashMap<String, String> = HashMap::new();
        if let Some(found) = header.captures(&content) {
            for pair in field.captures_iter(&found[1]) {
                fields.insert(pair[1].to_owned(), pair[2].to_owned());
            }
        }
        let named = |name: &str| fields.get(name).map_or("", |value| pystr::strip(value));
        if pre_reform.is_match(&content) {
            *dropped.entry("pre-reform spelling").or_insert(0) += 1;
        } else if !named("ПЕРЕВОДЧИК").is_empty() {
            *dropped.entry("translation").or_insert(0) += 1;
        } else if !named("АВТОР").is_empty() && !fields["АВТОР"].contains(&author.replace('_', " ")) {
            *dropped.entry("another author").or_insert(0) += 1;
        } else {
            pages.insert(titles[&page].1.replace('_', " "), (revision, content));
        }
    })?;
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    let mut words = 0_i64;
    for (_, content) in pages.values() {
        for sentence in wikitext_sentences(content) {
            for raw in pystr::split(&sentence) {
                let token = prose_token(raw);
                if !token.chars().any(|character| pystr::isalpha(character.encode_utf8(&mut [0; 4]))) {
                    continue;
                }
                words += 1;
                let compound = token.contains('-') && token.split('-').all(russian_form);
                if (token.chars().count() >= WEB_MIN_CHARACTERS && russian_form(token)) || compound {
                    *counts.entry(token.to_owned()).or_insert(0) += 1;
                }
            }
        }
    }
    let table: BTreeMap<String, i64> = counts.into_iter().map(|(form, count)| { let rounded = (ks_text::py_round(count.to::<f64>() * WORDS_SCALE / words.to::<f64>())).to::<i64>(); let bits = rounded.cast_unsigned().bit_width(); (form, i64::from(bits.clamp(1, MAXIMUM_LEXICON_WEIGHT))) }).collect();
    let listed: String = pages.iter().map(|(title, (revision, _))| format!("{title}\t{revision}\n")).collect();
    let author_years: BTreeMap<String, i64> = authors.values().collect::<BTreeSet<_>>().into_iter().map(|author| (author.replace('_', " "), died[author])).collect();
    Ok((table, json!({"authors": author_years, "pages": pages.len(), "pages_sha256": hex_digest(&listed), "left_out": dropped, "words": words})))
}

/// `verify` of the two lexicons whose sources are too large for the repository.
pub fn verify_lexicon_sources(root: &Path, web: Option<&Path>, prose: Option<&Path>) -> Result<(), String> {
    let directory = root.join(DIRECTORY);
    let mut differences = 0;
    if let Some(source) = web {
        report_line(&mut differences, "web lexicon from its source", derive_web_forms(source)? == gz_read(&directory.join("web-lexicon-ru.json.gz"))?);
    }
    if let Some(source) = prose {
        let (table, summary) = derive_prose(source)?;
        let recorded: Value = serde_json::from_slice(&gz_read(&directory.join("prose-lexicon-ru.json.gz"))?).map_err(|error| error.to_string())?;
        report_line(&mut differences, "prose lexicon from its source", json!(table) == recorded["forms"]);
        let receipt = read_json(&directory.join("prose-lexicon-receipt.json"))?;
        for name in ["authors", "pages", "pages_sha256", "left_out", "words"] {
            report_line(&mut differences, name, summary[name] == receipt[name]);
        }
    }
    if differences != 0 {
        return Err(format!("{differences} lexicon checks differ"));
    }
    Ok(())
}

/// One downloaded file of an independent set.
struct SetSource {
    locale: String,
    name: String,
    sha256: String,
    wikisource: bool,
    sample_percent: u64,
    taken_percent: u64,
}

fn sampled(sentence: &str, percent: u64, namespace: &str) -> bool {
    u64::from_str_radix(hex_digest(&format!("{namespace}:{sentence}")).get(..CHOICE_HEX_DIGITS).unwrap_or_default(), 16).expect("a digest") % 100 < percent
}

fn set_sentences(source: &SetSource, directory: &Path, excluded: &[String], namespace: &str, previous: &str) -> Result<Vec<String>, String> {
    let path = directory.join(&source.name);
    if checksum(&path)? != source.sha256 {
        return Err(format!("{} is not the pinned file", source.name));
    }
    let text = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
    let lines: Vec<String> = if source.wikisource {
        let pages: Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
        let table = pages.as_object().ok_or("a set of pages is an object")?;
        let mut titles: Vec<&String> = table.keys().filter(|title| !excluded.contains(title)).collect();
        titles.sort();
        titles.into_iter().flat_map(|title| wikitext_sentences(table[title]["content"].as_str().unwrap_or(""))).collect()
    } else {
        pystr::splitlines(&text, false).into_iter().map(str::to_owned).collect()
    };
    Ok(lines.into_iter().filter(|line| sampled(line, source.sample_percent, namespace) && !sampled(line, source.taken_percent, previous)).collect())
}

/// `rows_from`: rows of the sentences, as a split of the corpus is built: one per (script, keys) with shapes.
fn set_rows(root: &Path, sources: &[SetSource], directory: &Path, excluded: &[String], namespace: &str, previous: &str) -> Result<Vec<TokenRow>, String> {
    let lists = word_lists(root)?;
    let mut counts: BTreeMap<(String, String), BTreeMap<String, i64>> = BTreeMap::new();
    for source in sources {
        for line in set_sentences(source, directory, excluded, namespace, previous)? {
            let text = normalize::nfc(&line);
            for (position, raw) in pystr::split(&text).into_iter().enumerate() {
                if raw.chars().count() > MAXIMUM_TOKEN_CHARACTERS {
                    continue;
                }
                let Some(script) = label(raw, &source.locale, &lists)? else { continue };
                let keys = to_keys(raw)?;
                if !keys.is_empty() {
                    *counts.entry((script.to_owned(), keys)).or_default().entry(shape_of(raw, position).to_owned()).or_insert(0) += 1;
                }
            }
        }
    }
    Ok(counts.into_iter().map(|((script, keys), shapes)| ("test".to_owned(), script, keys, shapes)).collect())
}

/// `--freeze DIR` of `ortho_independent`: a new independent set from downloaded files, before any model is scored on it.
/// The sources are described by a JSON file: `name`, `sample_namespace`, `previous_sample_namespace`, `excluded_pages` and `files`, each with
/// `locale`, `name`, `url`, `retrieved`, `sha256`, `license`, `reader` (`lines` or `wikisource`), `sample_percent` and
/// `taken_by_an_earlier_set_percent`. `model/ortho_v1/independent` must not exist yet.
pub fn freeze_independent(root: &Path, description: &Path, directory: &Path) -> Result<Value, String> {
    let described = read_json(description)?;
    let folder = root.join(DIRECTORY).join("independent");
    if folder.exists() {
        return Err("independent test already frozen; do not overwrite it".to_owned());
    }
    let (Some(set_name), Some(namespace), Some(previous)) = (described["name"].as_str(), described["sample_namespace"].as_str(), described["previous_sample_namespace"].as_str()) else { return Err("a description names the set and its sample namespaces".to_owned()) };
    let excluded: Vec<String> = described["excluded_pages"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect();
    let mut sources = Vec::new();
    for item in described["files"].as_array().ok_or("a description lists its files")? {
        let text = |name: &str| item[name].as_str().map(str::to_owned).ok_or_else(|| format!("a file without {name}"));
        sources.push(SetSource { locale: text("locale")?, name: text("name")?, sha256: text("sha256")?, wikisource: item["reader"].as_str() == Some("wikisource"), sample_percent: item["sample_percent"].as_u64().ok_or("sample_percent")?, taken_percent: item["taken_by_an_earlier_set_percent"].as_u64().unwrap_or(0) });
    }
    let rows = set_rows(root, &sources, directory, &excluded, namespace, previous)?;
    std::fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    let (tokens, evidence_path, receipt_path) = (folder.join("tokens.jsonl.gz"), folder.join("known-evidence.json.gz"), folder.join("receipt.json"));
    std::fs::write(&tokens, gz_write(&tokens_content(&rows))?).map_err(|error| error.to_string())?;
    let token_rows = read_set_rows(&tokens)?;
    let models = reference_models(root)?;
    let mut forms = empty_forms();
    for row in &token_rows {
        for direction in SCRIPTS {
            for token in served(&row.keys, direction) {
                if token.chars().all(|key| KEYS.contains(&key)) {
                    queried(&token, direction, &mut forms);
                }
            }
        }
    }
    let evidence = verdict(&models, forms);
    std::fs::write(&evidence_path, gz_write(&crate::artifact::canonical(&evidence))?).map_err(|error| error.to_string())?;
    let known = read_known(&evidence_path)?;
    let minimum = (read_json(&root.join(DIRECTORY).join("config.json"))?["minimum_length"].as_u64().ok_or("minimum_length")?).to::<usize>();
    let (kept, dropped) = independent_population(root, &token_rows, &known, minimum)?;
    let mut by_script: BTreeMap<&str, i64> = BTreeMap::new();
    for row in &token_rows {
        *by_script.entry(if row.script == "en" { "en" } else { "ru" }).or_insert(0) += 1;
    }
    let mut words: BTreeSet<&str> = LOOKED_AT_WORDS.iter().copied().collect();
    words.extend(ortho::AUTHORED_WORDS.iter().copied());
    let history: Map<String, Value> = HISTORY.iter().map(|entry| entry.0).chain(DEVELOPMENT_SETS).map(|set| { let path = root.join(DIRECTORY).join(set); Ok((path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default(), json!(checksum(&path.join("tokens.jsonl.gz"))?))) }).collect::<Result<_, String>>()?;
    let receipt = json!({
        "schema_version": INDEPENDENT_SCHEMA, "set": set_name, "source": {"excluded_pages": excluded, "files": described["files"], "sample_namespace": namespace},
        "population": "the corpus' own rules (label, to_keys, shape_of; population) with this verdict; a type whose served keys, every run read once, match a row of the snapshot, of an opened set or of a case written by hand is left out",
        "left_out_words": words, "rows_by_script": by_script, "types": type_counts(&kept), "excluded_as_seen": dropped,
        "snapshot_tokens_sha256": checksum(&root.join(DIRECTORY).join("tokens.jsonl.gz"))?, "history_tokens_sha256": history,
        "tokens_sha256": checksum(&tokens)?, "evidence_sha256": checksum(&evidence_path)?, "provenance": pins(root, &["crates/ks-train/src/ortho_inputs.rs", "crates/ks-train/src/ortho.rs", "crates/ks-lang/src/ortho_model.rs"])?,
    });
    std::fs::write(&receipt_path, pretty(&receipt)).map_err(|error| error.to_string())?;
    Ok(receipt)
}

/// A set's rows from its files against a frozen set, one source at a time: a row must be in the frozen file with at least as many
/// shapes. That is how far files can be compared when a set's other source is no longer available.
pub fn check_rows_within(root: &Path, description: &Path, directory: &Path, only: &str, frozen: &Path) -> Result<(usize, usize), String> {
    let described = read_json(description)?;
    let item = described["files"].as_array().into_iter().flatten().find(|item| item["name"].as_str() == Some(only)).ok_or("no such file in the description")?;
    let source = SetSource { locale: item["locale"].as_str().unwrap_or("").to_owned(), name: only.to_owned(), sha256: item["sha256"].as_str().unwrap_or("").to_owned(), wikisource: item["reader"].as_str() == Some("wikisource"), sample_percent: item["sample_percent"].as_u64().unwrap_or(100), taken_percent: item["taken_by_an_earlier_set_percent"].as_u64().unwrap_or(0) };
    let rows = set_rows(root, std::slice::from_ref(&source), directory, &[], described["sample_namespace"].as_str().unwrap_or(""), described["previous_sample_namespace"].as_str().unwrap_or(""))?;
    let table: HashMap<(String, String), HashMap<String, i64>> = read_set_rows(frozen)?.into_iter().map(|row| ((row.script, row.keys), row.shapes.into_iter().collect())).collect();
    let mut within = 0;
    for (_, script, keys, shapes) in &rows {
        if let Some(recorded) = table.get(&(script.clone(), keys.clone())) {
            within += usize::from(shapes.iter().all(|(shape, count)| recorded.get(shape).is_some_and(|known| known >= count)));
        }
    }
    Ok((within, rows.len()))
}
