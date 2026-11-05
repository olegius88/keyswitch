//! The counted word lists the typo correction reads (`resources/typo-frequency-<locale>.bin`), from the frequency lists of
//! OpenSubtitles 2018 and the technical vocabulary of the ru.stackoverflow training split (`build_typo_frequency`). The
//! receipt records the digest of every decompressed section, so another deflate that compresses the same bytes otherwise
//! still verifies.
use ks_cast::To as _;
use crate::corpus::{checksum, read_json};
use crate::physical::hex;
use flate2::Compression;
use flate2::read::{MultiGzDecoder, ZlibDecoder};
use flate2::write::ZlibEncoder;
use ks_context::typo_correction::occurrence_positions;
use ks_text::{Indent, JsonOptions, json_dumps, pystr};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MIN_LETTERS: usize = 5;
const VOCABULARY_MIN_COUNT: i64 = 20;
const SCHEMA_VERSION: i64 = 1;
const STORED_MIN_COUNT: i64 = 5;
const OCCURRENCE_MIN_COUNT: i64 = 1;
const COUNT_LOG_SCALE: f64 = 8.0;
const COUNT_LOG_SCALE_STORED: i64 = 8;
const COUNT_MAX_STORED: i64 = 255;
const FALSE_POSITIVE_RATE: f64 = 0.01;
const FORM_SEPARATOR: u8 = 0;
const ZLIB_LEVEL: u32 = 9;
const BITS_PER_BYTE: u64 = 8;
const REPOSITORY: &str = "https://github.com/hermitdave/FrequencyWords";
const COMMIT: &str = "525f9b560de45753a5ea01069454e72e9aa541c6";
const ATTRIBUTION: &str = "Hermit Dave, FrequencyWords, from OpenSubtitles 2018 (OPUS, http://opus.nlpl.eu/OpenSubtitles2018.php)";
const LICENSE: &str = "CC BY-SA 4.0 (content), MIT (code), per the repository README at the commit";
const VOCABULARY_ATTRIBUTION: &str = "ru.stackoverflow.com posts and their code, training split, CC BY-SA 4.0";
const MESSAGES_SHA256: &str = "b22ec01e8e62931f35b171814b68a2192014abb097ed0feffa4eb42d7109b9cd";
const CODE_SHA256: &str = "8ae3f9d850fb712b78d00aaed1a04d22037a6dd8405909fed0bb4472ea587066";
const RECEIPT: &str = "resources/typo-frequency.receipt.json";
const TOOL: &str = "crates/ks-train/src/typo_frequency.rs";

pub(crate) struct Source {
    pub(crate) locale: &'static str,
    path_in_repository: &'static str,
    default: &'static str,
    sha256: &'static str,
}

pub(crate) const SOURCES: [Source; 2] = [
    Source { locale: "ru_RU", path_in_repository: "content/2018/ru/ru_full.txt", default: "reliable-release-2026-09-12/lexicon-sources/FrequencyWords-full/ru_full.txt", sha256: "32dfd94138aea2667ec455dcc8512029e39bd4b8ea70b09410d569b69d511a30" },
    Source { locale: "en_US", path_in_repository: "content/2018/en/en_full.txt", default: "typo/FrequencyWords-en/en_full.txt", sha256: "7fea67ab954e2c01df6c608c9826e594cf36f8823b3243554f88245fb75dc506" },
];

fn alphabet(locale: &str) -> &'static str {
    ks_context::constants::TYPO_ALPHABETS.iter().find(|entry| entry.0 == locale).map_or("", |entry| entry.1)
}

fn digest(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// `read_counts`: the forms written in the alphabet alone, and how many lines the list has.
fn read_counts(text: &str, letters: &str) -> (HashMap<String, i64>, i64) {
    let mut counts: HashMap<String, i64> = HashMap::new();
    let mut total = 0;
    for line in pystr::splitlines(text, false) {
        total += 1;
        let line = line.trim_end_matches('\n');
        if let Some((form, count)) = line.rsplit_once(' ')
            && !form.is_empty()
            && form.chars().all(|character| letters.contains(character))
        {
            *counts.entry(form.to_owned()).or_insert(0) += pystr::strip(count).parse::<i64>().unwrap_or(0);
        }
    }
    (counts, total)
}

fn is_token_letter(character: char) -> bool {
    character.is_ascii_alphabetic() || ('А'..='я').contains(&character) || character == 'Ё' || character == 'ё'
}

/// `vocabulary_counts`: forms of at least MIN_LETTERS letters, lowercase or with one leading capital, by locale.
pub(crate) fn vocabulary_counts(messages: &str, code: &str) -> Result<BTreeMap<&'static str, HashMap<String, i64>>, String> {
    let mut counts: BTreeMap<&'static str, HashMap<String, i64>> = SOURCES.iter().map(|source| (source.locale, HashMap::new())).collect();
    let mut add = |text: &str| {
        for word in text.split(|character| !is_token_letter(character)).filter(|word| !word.is_empty()) {
            let lower = pystr::casefold(word);
            if word.chars().count() < MIN_LETTERS || !word.chars().skip(1).eq(lower.chars().skip(1)) {
                continue;
            }
            for source in &SOURCES {
                let letters = alphabet(source.locale);
                if lower.chars().all(|character| letters.contains(character)) {
                    *counts.get_mut(source.locale).expect("a locale").entry(lower.clone()).or_insert(0) += 1;
                }
            }
        }
    };
    for line in messages.lines() {
        let row: Value = serde_json::from_str(line).map_err(|error| error.to_string())?;
        add(row["text"].as_str().ok_or("a message without text")?);
    }
    for line in code.lines() {
        add(line);
    }
    Ok(counts)
}

/// `bloom`: the filter's bits, its size in bits and its number of positions per form.
fn bloom(forms: &[String]) -> (Vec<u8>, u64, u64) {
    // The optimal Bloom filter: ln(1 / p) / ln(2)^2 bits per form, ln(2) bits per form and position.
    let ln_two = 1.0 / std::f64::consts::E.log2();
    let bits = ((-forms.len().to::<f64>() * FALSE_POSITIVE_RATE.ln() / ln_two / ln_two).ceil()).to::<u64>().max(1);
    let hashes = (ks_text::py_round(bits.to::<f64>() / forms.len().max(1).to::<f64>() * ln_two)).to::<u64>().max(1);
    let mut filter = vec![0_u8; bits.div_ceil(BITS_PER_BYTE).to::<usize>()];
    for form in forms {
        for position in occurrence_positions(form, bits, hashes) {
            filter[(position / BITS_PER_BYTE).to::<usize>()] |= 1 << (position % BITS_PER_BYTE);
        }
    }
    (filter, bits, hashes)
}

pub(crate) struct Built {
    pub(crate) file: Vec<u8>,
    sections: BTreeMap<&'static str, Vec<u8>>,
    selection: Value,
    seen: Value,
}

fn deflate(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(ZLIB_LEVEL));
    encoder.write_all(bytes).map_err(|error| error.to_string())?;
    encoder.finish().map_err(|error| error.to_string())
}

fn inflate(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    ZlibDecoder::new(bytes).read_to_end(&mut output).map_err(|error| error.to_string())?;
    Ok(output)
}

fn compact() -> JsonOptions {
    JsonOptions { ensure_ascii: false, sort_keys: true, separators: Some((",".to_owned(), ":".to_owned())), ..JsonOptions::default() }
}

/// `build`: the file of one locale.
pub(crate) fn build(source: &Source, text: &str, vocabulary: &HashMap<String, i64>) -> Result<Built, String> {
    let letters = alphabet(source.locale);
    let (counts, total) = read_counts(text, letters);
    let codes: HashMap<char, u8> = letters.chars().enumerate().map(|(index, letter)| (letter, index.to::<u8>() + 1)).collect();
    let mut stored: Vec<(Vec<u8>, i64)> = counts.iter().filter(|(form, count)| **count >= STORED_MIN_COUNT && form.chars().count() >= MIN_LETTERS - 1).map(|(form, count)| (form.chars().map(|letter| codes[&letter]).collect(), *count)).collect();
    stored.sort();
    let technical: BTreeSet<String> = vocabulary.iter().filter(|(form, count)| **count >= VOCABULARY_MIN_COUNT && counts.get(*form).copied().unwrap_or(0) < OCCURRENCE_MIN_COUNT).map(|(form, _)| form.clone()).collect();
    let mut rare: BTreeSet<String> = counts.iter().filter(|(form, count)| (OCCURRENCE_MIN_COUNT..STORED_MIN_COUNT).contains(*count) && form.chars().count() >= MIN_LETTERS).map(|(form, _)| form.clone()).collect();
    rare.extend(technical.iter().cloned());
    let rare: Vec<String> = rare.into_iter().collect();
    let mut forms = Vec::new();
    for (form, _) in &stored {
        forms.extend_from_slice(form);
        forms.push(FORM_SEPARATOR);
    }
    let weights: Vec<u8> = stored.iter().map(|(_, count)| (COUNT_MAX_STORED.min((ks_text::py_round((*count).to::<f64>().ln() * COUNT_LOG_SCALE)).to::<i64>())).to::<u8>()).collect();
    let (filter, bits, hashes) = bloom(&rare);
    let (packed_forms, packed_weights) = (deflate(&forms)?, deflate(&weights)?);
    let mut payload = packed_forms.clone();
    payload.extend_from_slice(&packed_weights);
    payload.extend_from_slice(&filter);
    let selection = json!({"list_entries": total, "letter_forms": counts.len(), "stored": stored.len(), "seen_items": rare.len(), "vocabulary_forms": technical.len()});
    let rate = (1.0 - (-hashes.to::<f64>() * rare.len().to::<f64>() / bits.to::<f64>()).exp()).powf(hashes.to::<f64>());
    let seen = json!({"bits": bits, "hashes": hashes, "items": rare.len(), "false_positive_rate": rate});
    let header = json!({
        "schema_version": SCHEMA_VERSION, "name": format!("opensubtitles-2018-{}-typo-frequency", source.locale), "locale": source.locale, "alphabet": letters, "count_log_scale": COUNT_LOG_SCALE_STORED,
        "stored_min_count": STORED_MIN_COUNT, "occurrence_min_count": OCCURRENCE_MIN_COUNT, "selection": selection, "seen": seen,
        "sections": {"forms": packed_forms.len(), "weights": packed_weights.len(), "seen": filter.len()}, "payload_sha256": digest(&payload),
        "source": {"attribution": ATTRIBUTION, "repository": REPOSITORY, "commit": COMMIT, "path": source.path_in_repository, "sha256": source.sha256, "license": LICENSE},
        "vocabulary": {"attribution": VOCABULARY_ATTRIBUTION, "min_count": VOCABULARY_MIN_COUNT, "messages_sha256": MESSAGES_SHA256, "code_sha256": CODE_SHA256},
    });
    let mut file = json_dumps(&header, &compact()).into_bytes();
    file.push(b'\n');
    file.extend_from_slice(&payload);
    Ok(Built { file, sections: BTreeMap::from([("forms", forms), ("weights", weights), ("seen", filter)]), selection, seen })
}

fn file_of(locale: &str) -> String {
    format!("resources/typo-frequency-{locale}.bin")
}

fn pinned_text(path: &Path, pin: &str) -> Result<Vec<u8>, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    if digest(&bytes) != pin {
        return Err(format!("{}: SHA-256 differs from the pinned {pin}", path.display()));
    }
    Ok(bytes)
}

fn gz_text(bytes: &[u8]) -> Result<String, String> {
    let mut text = String::new();
    MultiGzDecoder::new(bytes).read_to_string(&mut text).map_err(|error| error.to_string())?;
    Ok(text)
}

pub struct Paths {
    pub ru: PathBuf,
    pub en: PathBuf,
    pub messages: PathBuf,
    pub code: PathBuf,
}

impl Paths {
    /// Where the sources lie in a private folder of the working tree.
    pub fn in_private(private: &Path) -> Paths {
        Paths {
            ru: private.join(SOURCES[0].default),
            en: private.join(SOURCES[1].default),
            messages: private.join("research-2026-09-29/messages-tool/so-train.jsonl.gz"),
            code: private.join("research-2026-09-29/messages-tool/code-train.txt.gz"),
        }
    }
}

fn map_of_sections(built: &Built) -> Value {
    Value::Object(built.sections.iter().map(|(name, bytes)| ((*name).to_owned(), Value::String(digest(bytes)))).collect())
}

/// The command: rebuild the files and the receipt, or with `verify` check the shipped ones against the sources.
pub fn command(root: &Path, paths: &Paths, verify: bool) -> Result<bool, String> {
    let messages = gz_text(&pinned_text(&paths.messages, MESSAGES_SHA256)?)?;
    let code = gz_text(&pinned_text(&paths.code, CODE_SHA256)?)?;
    let vocabulary = vocabulary_counts(&messages, &code)?;
    let mut entries = Map::new();
    let mut failures = Vec::new();
    for source in &SOURCES {
        let path = if source.locale == "ru_RU" { &paths.ru } else { &paths.en };
        let raw = pinned_text(path, source.sha256)?;
        let text = String::from_utf8(raw).map_err(|error| error.to_string())?;
        let built = build(source, &text, &vocabulary[source.locale])?;
        let target = root.join(file_of(source.locale));
        if verify {
            let shipped = std::fs::read(&target).map_err(|error| error.to_string())?;
            let cut = shipped.iter().position(|byte| *byte == b'\n').ok_or("a typo table without a header")?;
            let header: Value = serde_json::from_slice(&shipped[..cut]).map_err(|error| error.to_string())?;
            let payload = &shipped[cut + 1..];
            let size = |name: &str| (header["sections"][name].as_u64().unwrap_or(0)).to::<usize>();
            let (forms, weights) = (inflate(&payload[..size("forms")])?, inflate(&payload[size("forms")..size("forms") + size("weights")])?);
            let filter = &payload[size("forms") + size("weights")..];
            if forms != built.sections["forms"] || weights != built.sections["weights"] || filter != built.sections["seen"].as_slice() {
                failures.push(source.locale.to_owned());
            }
        } else {
            std::fs::write(&target, &built.file).map_err(|error| error.to_string())?;
        }
        entries.insert(source.locale.to_owned(), json!({
            "source": {"repository": REPOSITORY, "commit": COMMIT, "path": source.path_in_repository, "sha256": source.sha256, "license": LICENSE, "attribution": ATTRIBUTION},
            "selection": built.selection, "seen": built.seen, "sections_sha256": map_of_sections(&built),
            "file": {"path": file_of(source.locale), "bytes": built.file.len(), "sha256": digest(&built.file)},
        }));
    }
    let receipt_path = root.join(RECEIPT);
    if verify {
        let shipped = read_json(&receipt_path)?;
        for (locale, entry) in &entries {
            let recorded = &shipped["locales"][locale];
            if recorded["sections_sha256"] != entry["sections_sha256"] || recorded["selection"] != entry["selection"] {
                failures.push(format!("{locale} receipt"));
            }
        }
        println!("{}", if failures.is_empty() { "verified".to_owned() } else { format!("differs: {}", failures.join(", ")) });
        return Ok(failures.is_empty());
    }
    let forms_counted: BTreeMap<&str, usize> = vocabulary.iter().map(|(locale, table)| (*locale, table.len())).collect();
    let receipt = json!({
        "schema_version": SCHEMA_VERSION, "tool": TOOL,
        "parameters": {"stored_min_count": STORED_MIN_COUNT, "occurrence_min_count": OCCURRENCE_MIN_COUNT, "min_letters": MIN_LETTERS, "count_log_scale": COUNT_LOG_SCALE_STORED, "occurrence_false_positive_rate": FALSE_POSITIVE_RATE, "zlib_level": ZLIB_LEVEL, "vocabulary_min_count": VOCABULARY_MIN_COUNT},
        "vocabulary": {"attribution": VOCABULARY_ATTRIBUTION, "messages": {"file": "so-train.jsonl.gz", "sha256": MESSAGES_SHA256}, "code": {"file": "code-train.txt.gz", "sha256": CODE_SHA256}, "forms_counted": forms_counted},
        "locales": entries,
    });
    std::fs::write(&receipt_path, json_dumps(&receipt, &JsonOptions { ensure_ascii: false, indent: Some(Indent::Spaces(2)), ..JsonOptions::default() }) + "\n").map_err(|error| error.to_string())?;
    let _ = checksum(&receipt_path)?;
    Ok(true)
}
