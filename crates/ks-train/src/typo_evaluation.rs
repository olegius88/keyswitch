//! The typo correction measured on development text and on the owner's typing (`evaluate_typo_correction`). Three
//! deterministic measurements: `synthetic`, one slip in one word out of many, tuned on one half of the messages and reported
//! on the other; `clean`, every word judged as typed; `field`, every word of the owner's typing, with the personal
//! dictionary growing as it would in use. Only development splits are read, never a test file. Lists of words go to the
//! private output folder.
use ks_cast::To as _;
use crate::physical::{hex_digest, pair};
use crate::tools::reference_models;
use flate2::read::MultiGzDecoder;
use ks_config::personal_words::PersonalWords;
use ks_context::context_model::TermTables;
use ks_context::typo_correction::{Candidate, OPERATIONS, Operation, Reason, TypoCorrector, TypoJudgement, TypoLexicon, TypoPolicy, decide_under, known_elsewhere};
use ks_engine::engine::{PUNCTUATION, WORD_JOINERS};
use ks_lang::Resources;
use ks_lang::detector::ProtectedTokens;
use ks_lang::identifier_lexicon::IdentifierLexicon;
use ks_lang::tables::WordTable;
use ks_text::pystr;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const DRAW_HEX_DIGITS: usize = 16;
const SAMPLE_MODULUS: u64 = 32;
const VOCABULARY_MIN_COUNTS: [i64; 6] = [2, 3, 5, 10, 20, 50];
const VOCABULARY_MAX_RECALL_LOSS: f64 = 0.02;
const MIN_LETTERS: usize = 5;
const TRANSPOSED_LETTERS: usize = 2;
const CANDIDATE_MIN_COUNT: i64 = 20;
const TRANSPOSITION_LOG_PROBABILITY: f64 = -1.4;
const MIN_COUNTS: [i64; 4] = [10, 20, 50, 100];
const RATIOS: [f64; 7] = [1.0, 1.5, 2.0, 3.0, 5.0, 10.0, 20.0];
const DELETION_LOGS: [f64; 4] = [-3.7, -5.0, -6.5, -8.0];
const INSERTION_LOGS: [f64; 3] = [-1.4, -2.5, -3.7];
const SUBSTITUTION_LOGS: [f64; 3] = [-3.0, -4.0, -5.0];
const MAX_WRONG_SHARE: f64 = 0.02;
const WILSON_Z: f64 = 1.96;
const PROSE_WORDS_PER_FALSE: i64 = 10_000;
const FORUM_WORDS_PER_FALSE: i64 = 5_000;
const RETYPE_WINDOW_SECONDS: f64 = 30.0;
const SENTENCE_END: &str = ".!?…";
const EDGE_SIGNS: &str = ".,!?:;\"'()«»…";
const PROSE_FILES: [&str; 2] = ["tatoeba-dev", "taiga-dev"];
const FORUM_FILES: [&str; 1] = ["so-dev"];
const CODE_FILE: &str = "code-dev.txt.gz";
const LOCALES: [&str; 2] = ["en_US", "ru_RU"];
const CYRILLIC: &str = "абвгдеёжзийклмнопрстуфхцчшщъыьэюя";

fn draw(parts: &[&str]) -> u64 {
    u64::from_str_radix(hex_digest(&parts.join(":")).get(..DRAW_HEX_DIGITS).unwrap_or_default(), 16).expect("a digest")
}

fn locale_of(token: &str) -> Option<&'static str> {
    // `character.casefold()` of every character, as the Python compares them.
    if token.chars().all(|character| { let folded = pystr::casefold(character.encode_utf8(&mut [0; 4])); folded.as_str() >= "a" && folded.as_str() <= "z" }) {
        return Some("en_US");
    }
    if token.chars().all(|character| { let folded = pystr::casefold(character.encode_utf8(&mut [0; 4])); CYRILLIC.contains(folded.as_str()) }) {
        return Some("ru_RU");
    }
    None
}

fn other_reading(text: &str, locale: &str) -> (&'static str, String) {
    if locale == "en_US" { ("ru_RU", pair().translate(text, "us", "ru").expect("layouts")) } else { ("en_US", pair().translate(text, "ru", "us").expect("layouts")) }
}

fn is_word_letter(character: char) -> bool {
    character.is_ascii_alphabetic() || ('А'..='я').contains(&character) || character == 'Ё' || character == 'ё'
}

fn ends_word(character: char) -> bool {
    let text = character.to_string();
    pystr::isspace(&text) || (PUNCTUATION.contains(character) && !WORD_JOINERS.contains(&text.as_str()))
}

/// `words_of`: the plain words of a text as the engine ends them: their index, the word, and whether a capital may open it.
fn words_of(text: &str) -> Vec<(usize, String, bool)> {
    let characters: Vec<char> = text.chars().collect();
    let mut result = Vec::new();
    let (mut index, mut position) = (0, 0);
    while position < characters.len() {
        if !is_word_letter(characters[position]) {
            position += 1;
            continue;
        }
        let start = position;
        while position < characters.len() && is_word_letter(characters[position]) {
            position += 1;
        }
        let before = if start > 0 { characters[start - 1] } else { ' ' };
        let after = characters.get(position).copied().unwrap_or(' ');
        if ends_word(before) && ends_word(after) {
            let preceding: String = characters[..start].iter().collect();
            let trimmed = pystr::rstrip(&preceding);
            let capital = trimmed.is_empty() || trimmed.chars().last().is_some_and(|last| SENTENCE_END.contains(last));
            result.push((index, characters[start..position].iter().collect(), capital));
        }
        index += 1;
    }
    result
}

fn read_lines(path: &Path) -> Result<Vec<String>, String> {
    let file = std::fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    BufReader::new(MultiGzDecoder::new(file)).lines().map(|line| line.map_err(|error| error.to_string())).collect()
}

fn messages(directory: &Path, name: &str) -> Result<Vec<(String, String)>, String> {
    read_lines(&directory.join(format!("{name}.jsonl.gz")))?
        .iter()
        .map(|line| {
            let item: Value = serde_json::from_str(line).map_err(|error| error.to_string())?;
            let qid = match &item["qid"] { Value::String(text) => text.clone(), other @ (Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) |
Value::Object(_)) => other.to_string() };
            Ok((qid, item["text"].as_str().unwrap_or("").to_owned()))
        })
        .collect()
}

/// `Knowledge`: what the engine knows from elsewhere, with the frozen reference lexicons.
struct Knowledge {
    models: crate::tools::Lexicons,
    terms: Arc<TermTables>,
    identifiers: Option<IdentifierLexicon>,
    protected: Arc<ProtectedTokens>,
}

impl Knowledge {
    fn load(root: &Path, resources: &Arc<Resources>) -> Result<Knowledge, String> {
        let error_text = |error: ks_lang::LoadError| error.name.to_string();
        Ok(Knowledge { models: reference_models(root, resources, false)?, terms: Arc::new(TermTables::load(&root.join("resources/models")).map_err(error_text)?), identifiers: IdentifierLexicon::load(&root.join("resources/identifiers.json")).ok(), protected: resources.protected_tokens() })
    }

    fn known(&self, text: &str) -> bool {
        let lexicons: Vec<&WordTable> = self.models.iter().map(|model| &*model.frequencies).collect();
        known_elsewhere(text, &lexicons, &self.terms, self.identifiers.as_ref(), &self.protected, &|_| false)
    }
}

/// `make_typo`: one slip in a lowercase word that the operation undoes, chosen by `seed`.
fn make_typo(word: &[char], operation: Operation, seed: u64, neighbours: &HashMap<char, String>) -> Option<String> {
    let letters = word.len().to::<u64>();
    let joined = |parts: &[&[char]]| -> String { parts.iter().flat_map(|part| part.iter()).collect() };
    match operation {
        Operation::Transposition => {
            let places: Vec<usize> = (0..word.len() - 1).filter(|index| word[*index] != word[index + 1]).collect();
            if places.is_empty() {
                return None;
            }
            let place = places[(seed % places.len().to::<u64>()).to::<usize>()];
            Some(joined(&[&word[..place], &[word[place + 1], word[place]], &word[place + TRANSPOSED_LETTERS..]]))
        }
        Operation::Insertion => {
            let place = (seed % letters).to::<usize>();
            Some(joined(&[&word[..place], &word[place + 1..]]))
        }
        Operation::Substitution => {
            let place = (seed % letters).to::<usize>();
            let options: Vec<char> = neighbours.get(&word[place]).map_or_else(Vec::new, |text| text.chars().collect());
            if options.is_empty() {
                return None;
            }
            Some(joined(&[&word[..place], &[options[((seed / letters) % options.len().to::<u64>()).to::<usize>()]], &word[place + 1..]]))
        }
        Operation::Deletion => {
            let place = (seed % (letters + 1)).to::<usize>();
            let around: Vec<char> = [place.checked_sub(1), Some(place)].into_iter().flatten().filter(|index| *index < word.len()).map(|index| word[index]).collect();
            let mut extra: BTreeSet<char> = around.iter().copied().collect();
            for letter in &around {
                extra.extend(neighbours.get(letter).map_or_else(Vec::new, |text| text.chars().collect::<Vec<char>>()));
            }
            let sorted: Vec<char> = extra.into_iter().collect();
            Some(joined(&[&word[..place], &[sorted[((seed / (letters + 1)) % sorted.len().to::<u64>()).to::<usize>()]], &word[place..]]))
        }
    }
}

struct Sample {
    half: u64,
    locale: &'static str,
    operation: Operation,
    word: String,
    weight: i64,
    capital: bool,
    gate: String,
    candidates: Vec<Candidate>,
}

fn judge(corrector: &TypoCorrector, knowledge: &Knowledge, token: &str, locale: &str, capital_allowed: bool) -> TypoJudgement {
    let (other, reading) = other_reading(token, locale);
    corrector.judge(token, locale, &[(other, reading.as_str())], capital_allowed, &|text| knowledge.known(text))
}

fn synthetic_samples(directory: &Path, corrector: &TypoCorrector, knowledge: &Knowledge) -> Result<Vec<Sample>, String> {
    let mut samples = Vec::new();
    for name in PROSE_FILES.iter().chain(&FORUM_FILES) {
        for (qid, text) in messages(directory, name)? {
            let half = draw(&["half", name, &qid]) % 2;
            for (index, token, capital_allowed) in words_of(&text) {
                let Some(locale) = locale_of(&token) else { continue };
                let lower = pystr::casefold(&token);
                let characters: Vec<char> = token.chars().collect();
                if characters.len() < MIN_LETTERS || !(token == lower || capital_allowed) {
                    continue;
                }
                let lowered: Vec<char> = lower.chars().collect();
                if characters[1..] != lowered[1..] {
                    continue;
                }
                let seed = draw(&["typo", name, &qid, &index.to_string()]);
                if !seed.is_multiple_of(SAMPLE_MODULUS) {
                    continue;
                }
                let lexicon = corrector.lexicon(locale).ok_or("a locale without a list")?;
                let Some(weight) = lexicon.weight(&lower) else { continue };
                let operation = OPERATIONS[((seed / SAMPLE_MODULUS) % OPERATIONS.len().to::<u64>()).to::<usize>()];
                let Some(typo) = make_typo(&lowered, operation, seed / SAMPLE_MODULUS / OPERATIONS.len().to::<u64>(), corrector.neighbours_of(locale)) else { continue };
                if typo == lower {
                    continue;
                }
                let capital = token != lower;
                let typed = if capital { let mut letters = typo.chars(); let first = letters.next().map(|letter| pystr::upper(letter.encode_utf8(&mut [0; 4]))).unwrap_or_default(); format!("{first}{}", letters.as_str()) } else { typo.clone() };
                let judgement = judge(corrector, knowledge, &typed, locale, true);
                let mut sample = Sample { half, locale, operation, word: lower, weight, capital, gate: judgement.reason.as_str().to_owned(), candidates: Vec::new() };
                if matches!(judgement.reason, Reason::NoCandidate | Reason::Rare | Reason::Ambiguous | Reason::Corrected) {
                    sample.gate = String::new();
                    sample.candidates = corrector.candidates(&typo, locale);
                }
                samples.push(sample);
            }
        }
    }
    Ok(samples)
}

fn score(samples: &[Sample], policy: &TypoPolicy, half: u64) -> Value {
    let minimum = policy.minimum_weight();
    let mut table: BTreeMap<String, BTreeMap<String, i64>> = BTreeMap::new();
    for sample in samples.iter().filter(|sample| sample.half == half && sample.weight >= minimum) {
        let outcome = if sample.gate.is_empty() {
            let judgement = decide_under(policy, &sample.candidates, sample.capital);
            match (&judgement.choice, judgement.reason) {
                (Some(choice), Reason::Corrected) => if pystr::casefold(&choice.word) == sample.word { "right".to_owned() } else { "wrong".to_owned() },
                _ => judgement.reason.as_str().to_owned(),
            }
        } else {
            sample.gate.clone()
        };
        for key in [format!("{}:{}", sample.locale, sample.operation.as_str()), sample.locale.to_owned(), "all".to_owned()] {
            *table.entry(key).or_default().entry(outcome.clone()).or_insert(0) += 1;
        }
    }
    Value::Object(table.into_iter().map(|(key, counts)| {
        let total: i64 = counts.values().sum();
        let (right, wrong) = (*counts.get("right").unwrap_or(&0), *counts.get("wrong").unwrap_or(&0));
        let corrected = right + wrong;
        let left: Map<String, Value> = counts.iter().filter(|(reason, _)| *reason != "right" && *reason != "wrong").map(|(reason, count)| (reason.clone(), json!(count))).collect();
        (key, json!({"samples": total, "right": right, "wrong": wrong, "right_share": if total != 0 { right.to::<f64>() / total.to::<f64>() } else { 0.0 }, "wrong_share_of_corrections": if corrected != 0 { wrong.to::<f64>() / corrected.to::<f64>() } else { 0.0 }, "left": left, "real_word_typos": counts.get("occurs").copied().unwrap_or(0)}))
    }).collect())
}

fn wilson_upper(wrong: i64, total: i64) -> f64 {
    if total == 0 {
        return 1.0;
    }
    let z_squared = WILSON_Z * WILSON_Z;
    let share = wrong.to::<f64>() / total.to::<f64>();
    let twice = (total + total).to::<f64>();
    let centre = share + z_squared / twice;
    let spread = WILSON_Z * (share * (1.0 - share) / total.to::<f64>() + z_squared / (twice * twice)).sqrt();
    (centre + spread) / (1.0 + z_squared / total.to::<f64>())
}

fn policy_record(policy: &TypoPolicy) -> Value {
    let probabilities: Map<String, Value> = policy.log_probabilities.iter().map(|(operation, value)| (operation.as_str().to_owned(), json!(value))).collect();
    json!({"candidate_min_count": policy.candidate_min_count, "posterior_ratio": policy.posterior_ratio, "log_probabilities": probabilities})
}

/// `tune`: the most right corrections on the tuning half whose wrong share stays within the acceptance.
fn tune(samples: &[Sample]) -> Result<(TypoPolicy, Vec<Value>), String> {
    let mut tried = Vec::new();
    let mut best: Option<(i64, TypoPolicy)> = None;
    let tuning: Vec<&Sample> = samples.iter().filter(|sample| sample.half == 0 && sample.gate.is_empty()).collect();
    let mut population: HashMap<i64, i64> = HashMap::new();
    for sample in samples.iter().filter(|sample| sample.half == 0) {
        *population.entry(sample.weight).or_insert(0) += 1;
    }
    for deletion in DELETION_LOGS {
        for insertion in INSERTION_LOGS {
            for substitution in SUBSTITUTION_LOGS {
                let channel = [(Operation::Transposition, TRANSPOSITION_LOG_PROBABILITY), (Operation::Deletion, deletion), (Operation::Insertion, insertion), (Operation::Substitution, substitution)];
                let trial = TypoPolicy { log_probabilities: channel, ..TypoPolicy::default() };
                let mut choices: Vec<(i64, i64, f64, bool)> = Vec::new();
                for sample in &tuning {
                    if let Some(choice) = decide_under(&trial, &sample.candidates, sample.capital).choice {
                        choices.push((sample.weight, choice.weight, choice.margin, pystr::casefold(&choice.word) == sample.word));
                    }
                }
                for minimum in MIN_COUNTS {
                    for ratio in RATIOS {
                        let policy = TypoPolicy { candidate_min_count: minimum, posterior_ratio: ratio, log_probabilities: channel };
                        let (floor, threshold) = (policy.minimum_weight(), ratio.ln());
                        let taken: Vec<bool> = choices.iter().filter(|(weight, chosen, margin, _)| *weight >= floor && *chosen >= floor && *margin >= threshold).map(|choice| choice.3).collect();
                        let right = taken.iter().filter(|value| **value).count().to::<i64>();
                        let wrong = taken.len().to::<i64>() - right;
                        let upper = wilson_upper(wrong, taken.len().to::<i64>());
                        let feasible = upper <= MAX_WRONG_SHARE;
                        let in_scope: i64 = population.iter().filter(|(weight, _)| **weight >= floor).map(|(_, count)| *count).sum();
                        tried.push(json!({"policy": policy_record(&policy), "samples": in_scope, "right": right, "wrong": wrong, "wrong_share_upper_bound": upper, "feasible": feasible}));
                        if feasible && minimum == CANDIDATE_MIN_COUNT && best.as_ref().is_none_or(|(most, _)| right > *most) {
                            best = Some((right, policy));
                        }
                    }
                }
            }
        }
    }
    best.map(|(_, policy)| (policy, tried)).ok_or_else(|| "no policy keeps the wrong corrections within the allowed share".to_owned())
}

fn tsv_safe(text: &str) -> String {
    text.replace(['\t', '\n'], " ")
}

/// `clean`: every word of the development texts judged as typed.
fn clean(directory: &Path, corrector: &TypoCorrector, knowledge: &Knowledge, out: &Path) -> Result<Value, String> {
    let (mut words, mut eligible, mut corrections): (BTreeMap<String, i64>, BTreeMap<String, i64>, BTreeMap<String, i64>) = Default::default();
    let mut cache: HashMap<(String, bool), TypoJudgement> = HashMap::new();
    let mut rows: Vec<String> = Vec::new();
    let mut sources: Vec<(&str, Vec<(String, String)>)> = Vec::new();
    for name in PROSE_FILES.iter().chain(&FORUM_FILES) {
        sources.push((name, messages(directory, name)?));
    }
    sources.push(("code-dev", read_lines(&directory.join(CODE_FILE))?.into_iter().enumerate().map(|(number, line)| ((number + 1).to_string(), line)).collect()));
    for (name, texts) in sources {
        for (qid, text) in texts {
            for (_, token, capital_allowed) in words_of(&text) {
                let Some(locale) = locale_of(&token) else { continue };
                let kind = if name == "code-dev" { "code" } else if FORUM_FILES.contains(&name) { "forum" } else { "prose" };
                let key = format!("{name}:{locale}");
                *words.entry(key.clone()).or_insert(0) += 1;
                let judgement = cache.entry((token.clone(), capital_allowed)).or_insert_with(|| judge(corrector, knowledge, &token, locale, capital_allowed));
                if judgement.reason == Reason::Shape {
                    continue;
                }
                *eligible.entry(key.clone()).or_insert(0) += 1;
                if judgement.reason == Reason::Corrected {
                    let choice = judgement.choice.as_ref().ok_or("a correction without a choice")?;
                    *corrections.entry(key).or_insert(0) += 1;
                    rows.push([name, kind, locale, &qid, &token, &choice.word, choice.edit.operation.as_str(), &judgement.count_order().to_string(), &format!("{:.2}", choice.margin), &tsv_safe(&text)].join("\t"));
                }
            }
        }
    }
    std::fs::write(out.join("clean-corrections.tsv"), format!("corpus\tkind\tlocale\tqid\tword\tcorrection\toperation\tcount_order\tmargin\ttext\n{}\n", rows.join("\n"))).map_err(|error| error.to_string())?;
    let mut report: Map<String, Value> = words.iter().map(|(key, count)| { let fixed = corrections.get(key).copied().unwrap_or(0); (key.clone(), json!({"words": count, "eligible": eligible.get(key).copied().unwrap_or(0), "corrections": fixed, "per_10000_words": (fixed * PROSE_WORDS_PER_FALSE).to::<f64>() / (*count).to::<f64>()})) }).collect();
    report.insert("accepted".to_owned(), json!({"prose_words_per_false_correction": PROSE_WORDS_PER_FALSE, "forum_words_per_false_correction": FORUM_WORDS_PER_FALSE, "code": 0}));
    Ok(Value::Object(report))
}

/// `datetime.strptime(text, "%Y-%m-%d %H:%M:%S,%f").timestamp()` differences only need the same local day arithmetic.
fn stamp_seconds(text: &str, local_offset: i64) -> Option<f64> {
    let (date, time) = text.split_once(' ')?;
    let mut date_parts = date.split('-').map(|part| part.parse::<i64>().ok());
    let (year, month, day) = (date_parts.next()??, date_parts.next()??, date_parts.next()??);
    let (clock, fraction) = time.split_once(',')?;
    let mut clock_parts = clock.split(':').map(|part| part.parse::<i64>().ok());
    let (hour, minute, second) = (clock_parts.next()??, clock_parts.next()??, clock_parts.next()??);
    let shifted = year - i64::from(month <= 2);
    let era = shifted.div_euclid(400);
    let of_era = shifted - era * 400;
    let of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let days = era * 146_097 + of_era * 365 + of_era / 4 - of_era / 100 + of_year - 719_468;
    let micros: f64 = format!("0.{fraction}").parse().ok()?;
    Some((days * 86_400 + hour * 3_600 + minute * 60 + second - local_offset).to::<f64>() + micros)
}

fn kept_of(item: &Value) -> &str {
    if item["intended_group"] == item["group"] { item["original"].as_str().unwrap_or("") } else { item["other"].as_str().unwrap_or("") }
}

/// `retyped_words`: the words of the stream right after an erasing one, in the same application and soon after it.
fn retyped_words(stream: &Value, following: Option<&Value>, local_offset: i64) -> BTreeSet<String> {
    let mut words = BTreeSet::new();
    let Some(following) = following else { return words };
    if following["app"] != stream["app"] || stream["deleted_chars"].as_i64().unwrap_or(0) == 0 {
        return words;
    }
    let (Some(items), Some(closed)) = (following["items"].as_array().filter(|items| !items.is_empty()), stream["closed_ts"].as_f64()) else { return words };
    let Some(started) = items[0]["t"].as_str().and_then(|text| stamp_seconds(text, local_offset)) else { return words };
    if started - closed >= RETYPE_WINDOW_SECONDS {
        return words;
    }
    for item in items {
        words.insert(pystr::casefold(pystr::strip_chars(kept_of(item), EDGE_SIGNS)));
    }
    words
}

/// `field_words`: every word the owner ended at a boundary, in typing order on each device.
fn field_words(manifest: &Path, corrector: &TypoCorrector, knowledge: &Knowledge, out: &Path, local_offset: i64) -> Result<Value, String> {
    let root = manifest.canonicalize().map_err(|error| error.to_string())?.parent().and_then(Path::parent).and_then(Path::parent).ok_or("a manifest in .t/field")?.to_path_buf();
    let batteries = crate::corpus::read_json(manifest)?;
    let mut devices: BTreeMap<String, Vec<(String, Value)>> = BTreeMap::new();
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    for (name, relative) in batteries["batteries"].as_object().ok_or("batteries")? {
        let text = std::fs::read_to_string(root.join(relative.as_str().ok_or("a battery path")?)).map_err(|error| error.to_string())?;
        for line in pystr::splitlines(&text, false).into_iter().filter(|line| !line.is_empty()) {
            let stream: Value = serde_json::from_str(line).map_err(|error| error.to_string())?;
            if seen.insert((stream["t"].to_string(), stream["app"].to_string())) {
                devices.entry(name.trim_end_matches(|character: char| character.is_ascii_digit()).to_owned()).or_default().push((name.clone(), stream));
            }
        }
    }
    for streams in devices.values_mut() {
        streams.sort_by(|a, b| a.1["t"].as_str().unwrap_or("").cmp(b.1["t"].as_str().unwrap_or("")));
    }
    let mut cache: HashMap<(String, bool), TypoJudgement> = HashMap::new();
    let mut rows: Vec<(String, String, Vec<String>)> = Vec::new();
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    let mut vocabulary: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let scratch = std::env::temp_dir().join(format!("ks-typo-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).map_err(|error| error.to_string())?;
    let add = |counts: &mut BTreeMap<String, i64>, name: &str| *counts.entry(name.to_owned()).or_insert(0) += 1;
    for (device, streams) in &devices {
        let personal = PersonalWords::new(Some(scratch.join(format!("{device}.json"))), Box::new(|| false));
        let mine = vocabulary.entry(device.clone()).or_default();
        for (index, (name, stream)) in streams.iter().enumerate() {
            let retyped = retyped_words(stream, streams.get(index + 1).map(|entry| &entry.1), local_offset);
            let mut previous = String::new();
            for item in stream["items"].as_array().ok_or("a stream without items")? {
                let kept = kept_of(item).to_owned();
                let token = pystr::strip_chars(&kept, EDGE_SIGNS).to_owned();
                let trimmed = pystr::rstrip(&previous);
                let capital_allowed = previous.is_empty() || trimmed.chars().last().is_some_and(|last| SENTENCE_END.contains(last));
                previous.clone_from(&kept);
                add(&mut counts, "words");
                if item["trigger"].as_str() == Some("pause") {
                    add(&mut counts, "pause_items");
                    continue;
                }
                let Some(locale) = (if token.is_empty() { None } else { locale_of(&token) }) else { continue };
                if personal.contains(&token) {
                    add(&mut counts, "personal");
                    continue;
                }
                let judgement = cache.entry((token.clone(), capital_allowed)).or_insert_with(|| judge(corrector, knowledge, &token, locale, capital_allowed));
                if judgement.reason == Reason::Shape {
                    continue;
                }
                add(&mut counts, "eligible");
                let erased = item["excluded"].as_str() == Some("erased");
                if !erased && matches!(judgement.reason, Reason::NoCandidate | Reason::Rare | Reason::Ambiguous | Reason::Corrected) && personal.keep(&token) {
                    mine.insert(pystr::casefold(&token));
                }
                if judgement.reason != Reason::Corrected {
                    continue;
                }
                let choice = judgement.choice.as_ref().ok_or("a correction without a choice")?;
                add(&mut counts, "corrections");
                let mark = if erased && retyped.contains(&pystr::casefold(&choice.word)) { "erased_retyped_as_correction" } else if erased && !retyped.is_empty() { "erased_retyped_otherwise" } else if erased { "erased" } else { "kept" };
                add(&mut counts, mark);
                let again: Vec<&str> = retyped.iter().map(String::as_str).collect();
                rows.push((device.clone(), pystr::casefold(&token), vec![device.clone(), name.clone(), stream["app"].as_str().unwrap_or("").to_owned(), item["t"].as_str().unwrap_or("").to_owned(), token.clone(), choice.word.clone(), choice.edit.operation.as_str().to_owned(), judgement.count_order().to_string(), format!("{:.2}", choice.margin), mark.to_owned(), again.join(" ")]));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    let harmful: Vec<&Vec<String>> = rows.iter().filter(|(device, word, _)| vocabulary[device].contains(word)).map(|row| &row.2).collect();
    counts.insert("corrections_of_words_that_became_the_owner_s".to_owned(), harmful.len().to::<i64>());
    let header = "device\tbattery\tapp\ttime\tword\tcorrection\toperation\tcount_order\tmargin\tmark\tretyped_next\n";
    let table = |lines: Vec<&Vec<String>>| format!("{header}{}", lines.iter().map(|row| row.join("\t") + "\n").collect::<String>());
    std::fs::write(out.join("field-corrections.tsv"), table(rows.iter().map(|row| &row.2).collect())).map_err(|error| error.to_string())?;
    std::fs::write(out.join("field-harmful.tsv"), table(harmful)).map_err(|error| error.to_string())?;
    Ok(json!(counts))
}

/// What the command is asked to measure.
pub struct Request<'a> {
    pub root: &'a Path,
    pub measurement: &'a str,
    pub messages: &'a Path,
    pub manifest: &'a Path,
    pub out: &'a Path,
    pub tune: bool,
    pub local_offset_seconds: i64,
    pub vocabulary: Option<VocabularyInputs>,
}

/// The files the choice of the technical vocabulary's count reads.
pub struct VocabularyInputs {
    pub clean: PathBuf,
    pub ru: PathBuf,
    pub en: PathBuf,
    pub train_messages: PathBuf,
    pub train_code: PathBuf,
}

/// The count of the technical vocabulary, chosen on the development splits. For each candidate count: how many corrections
/// of the clean development texts the vocabulary would leave as typed, and how many synthetic typos of each operation no
/// list holds that it would make known (the recall it costs, at most). The chosen count is the least one at which no
/// operation loses more than `VOCABULARY_MAX_RECALL_LOSS` of those typos.
fn vocabulary_floor(directory: &Path, inputs: &VocabularyInputs, out: &Path) -> Result<Value, String> {
    let read_gz = |path: &Path| -> Result<String, String> {
        let mut text = String::new();
        MultiGzDecoder::new(std::fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?).read_to_string(&mut text).map_err(|error| error.to_string())?;
        Ok(text)
    };
    let counts = crate::typo_frequency::vocabulary_counts(&read_gz(&inputs.train_messages)?, &read_gz(&inputs.train_code)?)?;
    let mut lexicons = Vec::new();
    for source in &crate::typo_frequency::SOURCES {
        let path = if source.locale == "ru_RU" { &inputs.ru } else { &inputs.en };
        let text = std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let built = crate::typo_frequency::build(source, &text, &HashMap::new())?;
        let target = out.join(format!(".vocabulary-{}.bin", source.locale));
        std::fs::write(&target, &built.file).map_err(|error| error.to_string())?;
        let lexicon = TypoLexicon::load(&target, source.locale).map_err(|error| error.name.to_string())?;
        let _ = std::fs::remove_file(&target);
        lexicons.push((source.locale.to_owned(), lexicon));
    }
    let corrector = TypoCorrector::new(lexicons, TypoPolicy::default()).map_err(|error| error.name.to_string())?;
    let known = |text: &str, floor: i64| counts.values().any(|table| table.get(&pystr::casefold(text)).copied().unwrap_or(0) >= floor);
    let mut removed: BTreeMap<i64, BTreeMap<String, i64>> = VOCABULARY_MIN_COUNTS.iter().map(|floor| (*floor, BTreeMap::new())).collect();
    let clean_rows = std::fs::read_to_string(&inputs.clean).map_err(|error| format!("{}: {error}", inputs.clean.display()))?;
    for line in clean_rows.lines().skip(1) {
        let fields: Vec<&str> = line.split('\t').collect();
        let (corpus, locale, word) = (fields[0], fields[2], fields[4]);
        let (_, reading) = other_reading(word, locale);
        for (floor, table) in &mut removed {
            if known(word, *floor) || known(&reading, *floor) {
                *table.entry(format!("{corpus}:{locale}")).or_insert(0) += 1;
            }
        }
    }
    let mut lost: BTreeMap<i64, HashMap<String, i64>> = VOCABULARY_MIN_COUNTS.iter().map(|floor| (*floor, HashMap::new())).collect();
    let mut typos: BTreeMap<String, i64> = BTreeMap::new();
    for name in PROSE_FILES.iter().chain(&FORUM_FILES) {
        for (qid, text) in messages(directory, name)? {
            for (index, token, capital_allowed) in words_of(&text) {
                let Some(locale) = locale_of(&token) else { continue };
                let lower = pystr::casefold(&token);
                let characters: Vec<char> = token.chars().collect();
                let lowered: Vec<char> = lower.chars().collect();
                if characters.len() < MIN_LETTERS || !(token == lower || capital_allowed) || characters[1..] != lowered[1..] {
                    continue;
                }
                let seed = draw(&["typo", name, &qid, &index.to_string()]);
                let lexicon = corrector.lexicon(locale).ok_or("a locale without a list")?;
                let Some(weight) = lexicon.weight(&lower) else { continue };
                if !seed.is_multiple_of(SAMPLE_MODULUS) || weight < corrector.policy.minimum_weight() {
                    continue;
                }
                let operation = OPERATIONS[((seed / SAMPLE_MODULUS) % OPERATIONS.len().to::<u64>()).to::<usize>()];
                let Some(typo) = make_typo(&lowered, operation, seed / SAMPLE_MODULUS / OPERATIONS.len().to::<u64>(), corrector.neighbours_of(locale)) else { continue };
                if typo == lower || typo.chars().count() < MIN_LETTERS || lexicon.occurs(&typo) {
                    continue;
                }
                let key = format!("{locale}:{}", operation.as_str());
                *typos.entry(key.clone()).or_insert(0) += 1;
                let (_, reading) = other_reading(&typo, locale);
                for (floor, table) in &mut lost {
                    if known(&typo, *floor) || known(&reading, *floor) {
                        *table.entry(key.clone()).or_insert(0) += 1;
                    }
                }
            }
        }
    }
    let mut floors = Map::new();
    for floor in VOCABULARY_MIN_COUNTS {
        let shares: Map<String, Value> = typos.iter().map(|(key, total)| (key.clone(), json!(lost[&floor].get(key).copied().unwrap_or(0).to::<f64>() / (*total).to::<f64>()))).collect();
        floors.insert(floor.to_string(), json!({"clean_corrections_left_as_typed": removed[&floor], "synthetic_typos_lost": shares}));
    }
    let chosen = VOCABULARY_MIN_COUNTS.iter().copied().find(|floor| typos.iter().all(|(key, total)| lost[floor].get(key).copied().unwrap_or(0).to::<f64>() <= VOCABULARY_MAX_RECALL_LOSS * (*total).to::<f64>())).ok_or("no count keeps the recall")?;
    Ok(json!({"synthetic_typos_no_list_holds": typos, "floors": floors, "chosen": chosen}))
}

pub fn command(request: &Request<'_>, resources: &Arc<Resources>) -> Result<(), String> {
    std::fs::create_dir_all(request.out).map_err(|error| error.to_string())?;
    let corrector = TypoCorrector::packaged(&request.root.join("resources"), &LOCALES).map_err(|error| error.name.to_string())?;
    let knowledge = Knowledge::load(request.root, resources)?;
    let mut report = Map::new();
    report.insert("policy".to_owned(), policy_record(&corrector.policy));
    if matches!(request.measurement, "synthetic" | "all") {
        let samples = synthetic_samples(request.messages, &corrector, &knowledge)?;
        eprintln!("synthetic samples: {}", samples.len());
        if request.tune {
            let (tuned, tried) = tune(&samples)?;
            std::fs::write(request.out.join("synthetic-tuning.json"), crate::train_json_unsorted(&Value::Array(tried))).map_err(|error| error.to_string())?;
            report.insert("tuned_policy".to_owned(), policy_record(&tuned));
            report.insert("tuned_on_tuning_half".to_owned(), score(&samples, &tuned, 0));
            report.insert("tuned_on_report_half".to_owned(), score(&samples, &tuned, 1));
        }
        report.insert("synthetic_tuning_half".to_owned(), score(&samples, &corrector.policy, 0));
        report.insert("synthetic_report_half".to_owned(), score(&samples, &corrector.policy, 1));
    }
    if matches!(request.measurement, "clean" | "all") {
        report.insert("clean".to_owned(), clean(request.messages, &corrector, &knowledge, request.out)?);
    }
    if request.measurement == "vocabulary" {
        let inputs = request.vocabulary.as_ref().ok_or("the vocabulary measurement needs --clean, --ru, --en, --train-messages and --train-code")?;
        report.insert("vocabulary".to_owned(), vocabulary_floor(request.messages, inputs, request.out)?);
    }
    if matches!(request.measurement, "field" | "all") {
        report.insert("field".to_owned(), field_words(request.manifest, &corrector, &knowledge, request.out, request.local_offset_seconds)?);
    }
    let text = crate::train_json_unsorted(&Value::Object(report));
    std::fs::write(request.out.join(format!("report-{}.json", request.measurement)), &text).map_err(|error| error.to_string())?;
    print!("{}", String::from_utf8_lossy(&text));
    Ok(())
}
