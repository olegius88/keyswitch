//! One frozen split of a corpus, read as `freeze_context_action_corpus.load_split` reads it, and the edits its words
//! are given (`typo_variants`). Reads exactly the split it is asked for, never a neighbouring file.
use ks_cast::To as _;
use crate::physical::{hex, hex_digest};
use flate2::read::MultiGzDecoder;
use ks_text::pystr;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader};
use std::path::Path;

pub const TRAIN: &str = "train";
pub const DEVELOPMENT: &str = "development";
pub const CALIBRATION: &str = "calibration";
pub const FITTING_SPLITS: [&str; 3] = [TRAIN, DEVELOPMENT, CALIBRATION];

const CORPUS_BEFORE_WINDOW_CHARACTERS: usize = 96;
const CORPUS_AFTER_WINDOW_CHARACTERS: usize = 64;
const DETERMINISTIC_DRAW_HEX_DIGITS: usize = 16;
const TYPO_SOURCE_MIN_CHARACTERS: usize = 4;
const TYPO_DELETE_DUPLICATE_MARGIN_CHARACTERS: usize = 2;
const TYPO_TRANSPOSE_MARGIN_CHARACTERS: usize = 3;
const TRANSPOSE_PAIR_CHARACTERS: usize = 2;

#[derive(Clone, Debug, PartialEq)]
pub struct CorpusRow {
    pub identifier: String,
    pub original: String,
    pub group: Option<i64>,
    pub before: String,
    pub after: String,
    pub family: String,
    pub document: String,
    pub spacing: String,
    pub literal_tail: String,
    pub layout_representable: bool,
    pub split: String,
    pub quarantine_reasons: Vec<String>,
    pub lemma: String,
    pub language: String,
    pub source: String,
    pub source_file: String,
    pub source_sentence: String,
    pub source_token: String,
    pub space_before: String,
    pub upos: String,
    pub features: String,
    pub misc: String,
    pub alignment: String,
}

impl CorpusRow {
    /// `canonical(asdict(row))`.
    #[must_use]
    pub fn line(&self) -> Vec<u8> {
        crate::artifact::canonical(&serde_json::json!({
            "identifier": self.identifier, "original": self.original, "group": self.group, "before": self.before, "after": self.after, "lemma": self.lemma, "family": self.family,
            "document": self.document, "language": self.language, "source": self.source, "source_file": self.source_file, "source_sentence": self.source_sentence, "source_token": self.source_token,
            "spacing": self.spacing, "space_before": self.space_before, "literal_tail": self.literal_tail, "upos": self.upos, "features": self.features, "misc": self.misc,
            "alignment": self.alignment, "layout_representable": self.layout_representable, "split": self.split, "quarantine_reasons": self.quarantine_reasons,
        }))
    }
}

pub fn checksum(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(hex(&Sha256::digest(&bytes)))
}

pub fn read_json(path: &Path) -> Result<Value, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display()))
}

fn text(value: &Value, name: &str) -> Result<String, String> {
    value.get(name).and_then(Value::as_str).map(str::to_owned).ok_or_else(|| format!("corpus row without {name}"))
}

/// `load_split` of a split that is fitted on: the trainers read nothing else.
pub fn load_split(directory: &Path, split: &str) -> Result<Vec<CorpusRow>, String> {
    if !FITTING_SPLITS.contains(&split) {
        return Err("only a fitting split is read here".to_owned());
    }
    load_rows(directory, split)
}

/// `load_split`: exactly one requested split. The test split goes through here only from the sealed evaluation.
pub fn load_rows(directory: &Path, split: &str) -> Result<Vec<CorpusRow>, String> {
    let manifest = read_json(&directory.join("manifest.json"))?;
    let record = &manifest["splits"][split];
    if record["path"].as_str() != Some(&format!("{split}.jsonl.gz")) {
        return Err("unexpected split filename".to_owned());
    }
    let path = directory.join(format!("{split}.jsonl.gz"));
    if record["sha256"].as_str() != Some(&checksum(&path)?) {
        return Err("split compressed checksum mismatch".to_owned());
    }
    let file = std::fs::File::open(&path).map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(MultiGzDecoder::new(file));
    let mut content = Sha256::new();
    let mut rows = Vec::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line).map_err(|error| error.to_string())? == 0 {
            break;
        }
        content.update(&line);
        let value: Value = serde_json::from_slice(&line).map_err(|error| error.to_string())?;
        let row = CorpusRow {
            identifier: text(&value, "identifier")?,
            original: text(&value, "original")?,
            group: value["group"].as_i64(),
            before: text(&value, "before")?,
            after: text(&value, "after")?,
            family: text(&value, "family")?,
            document: text(&value, "document")?,
            spacing: text(&value, "spacing")?,
            literal_tail: text(&value, "literal_tail")?,
            layout_representable: value["layout_representable"].as_bool().ok_or("corpus row without layout_representable")?,
            split: text(&value, "split")?,
            quarantine_reasons: value["quarantine_reasons"]
                .as_array()
                .map(|items| items.iter().filter_map(|item| item.as_str().map(str::to_owned)).collect())
                .unwrap_or_default(),
            lemma: text(&value, "lemma")?,
            language: text(&value, "language")?,
            source: text(&value, "source")?,
            source_file: text(&value, "source_file")?,
            source_sentence: text(&value, "source_sentence")?,
            source_token: text(&value, "source_token")?,
            space_before: text(&value, "space_before")?,
            upos: text(&value, "upos")?,
            features: text(&value, "features")?,
            misc: text(&value, "misc")?,
            alignment: text(&value, "alignment")?,
        };
        if row.split != split || pystr::len(&row.before) > CORPUS_BEFORE_WINDOW_CHARACTERS || pystr::len(&row.after) > CORPUS_AFTER_WINDOW_CHARACTERS {
            return Err("invalid corpus row bounds or split".to_owned());
        }
        rows.push(row);
    }
    if Some(rows.len().to::<u64>()) != record["rows"].as_u64() || record["content_sha256"].as_str() != Some(&hex(&content.finalize())) {
        return Err("split content checksum or row count mismatch".to_owned());
    }
    Ok(rows)
}

fn draw(text: &str) -> u64 {
    u64::from_str_radix(hex_digest(text).get(..DETERMINISTIC_DRAW_HEX_DIGITS).unwrap_or_default(), 16).expect("hex digits")
}

/// `typo_variants`: three predetermined internal edits.
#[must_use]
pub fn typo_variants(original: &str, identifier: &str) -> Vec<String> {
    let letters: Vec<char> = original.chars().collect();
    let length = letters.len();
    if length < TYPO_SOURCE_MIN_CHARACTERS {
        return Vec::new();
    }
    let inside = 1 + (draw(&format!("delete-duplicate:{identifier}")) % (length - TYPO_DELETE_DUPLICATE_MARGIN_CHARACTERS).to::<u64>()).to::<usize>();
    let adjacent = 1 + (draw(&format!("transpose:{identifier}")) % (length - TYPO_TRANSPOSE_MARGIN_CHARACTERS).to::<u64>()).to::<usize>();
    let collect = |parts: &[&[char]]| -> String { parts.iter().flat_map(|part| part.iter()).collect() };
    let variants = [
        collect(&[&letters[..inside], &letters[inside + 1..]]),
        collect(&[&letters[..adjacent], &letters[adjacent + 1..=adjacent + 1], &letters[adjacent..=adjacent], &letters[adjacent + TRANSPOSE_PAIR_CHARACTERS..]]),
        collect(&[&letters[..inside], &letters[inside..=inside], &letters[inside..]]),
    ];
    let mut result: Vec<String> = Vec::new();
    for variant in variants {
        if variant != original && !result.contains(&variant) {
            result.push(variant);
        }
    }
    result
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn a_short_word_has_no_edits() {
        assert!(typo_variants("кот", "id").is_empty());
        assert!(!typo_variants("привет", "id").is_empty());
    }
}
