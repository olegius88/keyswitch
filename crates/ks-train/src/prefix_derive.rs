//! A word corpus derived from a frozen one without drawing its words again (`ks-train prefix-derive`). The training
//! families a draw under a new name picks leave training and are the new acceptance split; the acceptance split of the
//! corpus derived from is kept as development of its own (`FORMER_ACCEPTANCE_SPLIT`); development, calibration, the
//! held-out aliases and the config are copied byte for byte, each checked against the receipt and none read; and words of
//! one class may be added to training from a pinned file of rows, only in families its draw gives training and the cut
//! leaves there, never a word the corpus holds, one held out or one of a list given.
use crate::corpus::{CALIBRATION, DEVELOPMENT, TRAIN, checksum};
use crate::physical::{expanded_aliases, hex};
use crate::prefix_words::{
    ACCEPTANCE_SPLIT as ACCEPTANCE, CONFIG_FILE, Class, FORMER_ACCEPTANCE_SPLIT, HELD_OUT_ALIAS_HEX_DIGITS, HELD_OUT_FILE, HeldOut, HeldOutFile, RECEIPT_FILE, WordConfig, WordReceipt, WordRow, compact, draw, gunzip, gzip, read_gzip_lines, read_split, readable,
    split_counts, split_of,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Thousandths: the cut takes a family when its draw falls under its share of them.
const PER_MILLE: u64 = 1000;

/// Whether the cut of a derivation takes a family of training: its draw under the derivation's name falls under the share.
#[must_use]
pub fn cut_takes(namespace: &str, cut_per_mille: u64, family: &str) -> bool {
    draw(&format!("{namespace}:cut:{family}")) % PER_MILLE < cut_per_mille
}

/// Whether a family is one the cut of a derived corpus moved: a family of training by the config, which the cut takes.
#[must_use]
pub fn moved_by_cut(config: &WordConfig, derivation: &Derivation, family: &str) -> bool {
    split_of(config, family) == TRAIN && cut_takes(&derivation.namespace, derivation.cut_per_mille, family)
}

/// What `prefix-derive` is asked.
pub struct DeriveRequest<'a> {
    /// The frozen word corpus derived from.
    pub from: &'a Path,
    /// The name of the draws of the cut and of the words added.
    pub namespace: &'a str,
    pub cut_per_mille: u64,
    pub addition: Option<Addition<'a>>,
    /// Lists of words no word added may be (one a line, or rows by their `text`), by every physical alias.
    pub excluded: &'a [PathBuf],
    pub output: &'a Path,
}

/// Words of one class and source added to training from a file of word rows.
pub struct Addition<'a> {
    pub rows: &'a Path,
    /// The classes of the rows added: one at least.
    pub classes: &'a [Class],
    pub source: &'a str,
    /// How many, the first by their draw; every word that may be added when none.
    pub words: Option<usize>,
}

/// How a corpus was derived, in its receipt.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Derivation {
    /// The SHA-256 of the receipt of the corpus derived from.
    pub from_sha256: String,
    pub namespace: String,
    pub cut_per_mille: u64,
    /// The training families the cut took into acceptance.
    pub cut_families: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added: Option<Added>,
    /// The lists no word added is of: each file's SHA-256 and its words.
    pub excluded: Vec<HeldOutFile>,
}

/// The words added to training.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Added {
    /// The SHA-256 of the file of rows.
    pub sha256: String,
    /// The first class of the rows added, and the others (none in a receipt written before an addition took more).
    pub class: Class,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub more_classes: Vec<Class>,
    pub source: String,
    /// The words of the class and source the file holds, those of them that may be added, and those added.
    pub offered: usize,
    pub eligible: usize,
    pub words: usize,
    pub rows: usize,
}

/// The words of a list, one a line, or of a file of rows (`.jsonl.gz`) by their `text`.
fn words_of(path: &Path) -> Result<Vec<String>, String> {
    #[derive(Deserialize)]
    struct Text {
        text: String,
    }
    if path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("txt")) {
        let text = std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
        return Ok(text.lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_owned).collect());
    }
    read_gzip_lines(path)?.iter().map(|line| ks_text::json::loads::<Text>(line).map(|row| row.text).map_err(|error| format!("{}: {error}", path.display()))).collect()
}

/// The aliases a word is held out by: the first hex digits of each physical alias, as the held-out file keeps them.
fn aliases_of(word: &str) -> impl Iterator<Item = String> {
    expanded_aliases(word).into_iter().map(|alias| alias.get(..HELD_OUT_ALIAS_HEX_DIGITS).unwrap_or_default().to_owned())
}

/// The packed rows of a split and their SHA-256, written into `folder`.
fn write_split(folder: &Path, split: &str, rows: &[WordRow]) -> Result<String, String> {
    let bytes: Vec<u8> = rows.iter().flat_map(|row| (ks_text::json::dumps(row, &compact()) + "\n").into_bytes()).collect();
    let packed = gzip(&bytes)?;
    std::fs::write(folder.join(format!("{split}.jsonl.gz")), &packed).map_err(|error| error.to_string())?;
    Ok(hex(&Sha256::digest(&packed)))
}

/// `prefix-derive`: the derived corpus written into `output`, and its receipt.
pub fn derive(request: &DeriveRequest<'_>) -> Result<WordReceipt, String> {
    if request.output.join(RECEIPT_FILE).exists() {
        return Err(format!("{}: refusing to overwrite a word corpus", request.output.display()));
    }
    let receipt_path = request.from.join(RECEIPT_FILE);
    let mut receipt: WordReceipt = ks_text::json::loads(std::fs::read(&receipt_path).map_err(|error| format!("{}: {error}", receipt_path.display()))?).map_err(|error| format!("{RECEIPT_FILE}: {error}"))?;
    let config_path = request.from.join(CONFIG_FILE);
    if checksum(&config_path)? != receipt.config_sha256 {
        return Err(format!("{}: not the config the corpus was drawn by", config_path.display()));
    }
    let config: WordConfig = ks_text::json::loads(std::fs::read(&config_path).map_err(|error| error.to_string())?).map_err(|error| format!("{CONFIG_FILE}: {error}"))?;
    let former = receipt.sha256.remove(ACCEPTANCE).ok_or("the corpus has no acceptance split to keep as development")?;
    // Copied by their bytes, each the file its receipt pins: development and calibration are never read.
    std::fs::create_dir_all(request.output).map_err(|error| format!("{}: {error}", request.output.display()))?;
    let held_pin = receipt.held_out.sha256.clone();
    let copies = [
        (format!("{DEVELOPMENT}.jsonl.gz"), format!("{DEVELOPMENT}.jsonl.gz"), receipt.sha256.get(DEVELOPMENT).cloned()),
        (format!("{CALIBRATION}.jsonl.gz"), format!("{CALIBRATION}.jsonl.gz"), receipt.sha256.get(CALIBRATION).cloned()),
        (format!("{ACCEPTANCE}.jsonl.gz"), format!("{FORMER_ACCEPTANCE_SPLIT}.jsonl.gz"), Some(former.clone())),
        (HELD_OUT_FILE.to_owned(), HELD_OUT_FILE.to_owned(), Some(held_pin)),
        (CONFIG_FILE.to_owned(), CONFIG_FILE.to_owned(), Some(receipt.config_sha256.clone())),
    ];
    for (name, to, pin) in copies {
        let path = request.from.join(&name);
        if pin != Some(checksum(&path)?) {
            return Err(format!("{}: not the file the receipt pins", path.display()));
        }
        std::fs::copy(&path, request.output.join(to)).map_err(|error| error.to_string())?;
    }
    let held: HeldOut = ks_text::json::loads(gunzip(&request.from.join(HELD_OUT_FILE))?).map_err(|error| format!("{HELD_OUT_FILE}: {error}"))?;
    let mut barred: BTreeSet<String> = held.aliases.into_iter().collect();
    let mut excluded = Vec::new();
    for path in request.excluded {
        let words = words_of(path)?;
        barred.extend(words.iter().flat_map(|word| aliases_of(word)));
        excluded.push(HeldOutFile { sha256: checksum(path)?, rows: words.len() });
    }
    // The cut: training families whose draw under the new name falls under its share leave training for acceptance.
    let is_cut = |family: &str| cut_takes(request.namespace, request.cut_per_mille, family);
    let (moved, mut train): (Vec<WordRow>, Vec<WordRow>) = read_split(request.from, TRAIN)?.into_iter().partition(|row| is_cut(&row.family));
    let cut_families = moved.iter().map(|row| row.family.as_str()).collect::<BTreeSet<_>>().len();
    let mut added = None;
    if let Some(addition) = &request.addition {
        let offered: Vec<WordRow> = read_gzip_lines(addition.rows)?
            .iter()
            .map(|line| ks_text::json::loads::<WordRow>(line).map_err(|error| format!("{}: {error}", addition.rows.display())))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|row| addition.classes.contains(&row.class) && row.source == addition.source)
            .collect();
        let present: BTreeSet<(&str, i64)> = train.iter().chain(&moved).map(|row| (row.text.as_str(), row.group)).collect();
        let words: BTreeSet<(&str, i64, &str)> = offered.iter().map(|row| (row.text.as_str(), row.group, row.family.as_str())).collect();
        let mut eligible: Vec<(u64, &str, i64)> = words
            .iter()
            .filter(|(text, group, family)| split_of(&config, family) == TRAIN && !is_cut(family) && !present.contains(&(*text, *group)) && aliases_of(text).all(|alias| !barred.contains(&alias)))
            .map(|(text, group, _)| (draw(&format!("{}:add:{group}:{text}", request.namespace)), *text, *group))
            .collect();
        eligible.sort_unstable();
        let asked = addition.words.unwrap_or(eligible.len());
        if asked > eligible.len() {
            return Err(format!("{asked} words of the class asked, {} may be added", eligible.len()));
        }
        let chosen: BTreeSet<(String, i64)> = eligible.iter().take(asked).map(|(_, text, group)| ((*text).to_owned(), *group)).collect();
        let (offered_words, eligible_words) = (words.len(), eligible.len());
        let rows: Vec<WordRow> = offered.into_iter().filter(|row| chosen.contains(&(row.text.clone(), row.group))).collect();
        let (class, more_classes) = addition.classes.split_first().ok_or("an addition names the class of its rows")?;
        added = Some(Added { sha256: checksum(addition.rows)?, class: *class, more_classes: more_classes.to_vec(), source: addition.source.to_owned(), offered: offered_words, eligible: eligible_words, words: chosen.len(), rows: rows.len() });
        train.extend(rows);
    }
    // The old acceptance split is development of its own; training and the new acceptance are counted again.
    let counts = receipt.counts.remove(ACCEPTANCE).unwrap_or_default();
    receipt.families.insert(FORMER_ACCEPTANCE_SPLIT.to_owned(), counts.families);
    receipt.rows.insert(FORMER_ACCEPTANCE_SPLIT.to_owned(), counts.rows);
    receipt.counts.insert(FORMER_ACCEPTANCE_SPLIT.to_owned(), counts);
    receipt.sha256.insert(FORMER_ACCEPTANCE_SPLIT.to_owned(), former);
    for (split, rows) in [(TRAIN, &train), (ACCEPTANCE, &moved)] {
        receipt.sha256.insert(split.to_owned(), write_split(request.output, split, rows)?);
        let counts = split_counts(rows);
        receipt.families.insert(split.to_owned(), counts.families);
        receipt.rows.insert(split.to_owned(), counts.rows);
        receipt.counts.insert(split.to_owned(), counts);
    }
    receipt.derived = Some(Derivation { from_sha256: checksum(&receipt_path)?, namespace: request.namespace.to_owned(), cut_per_mille: request.cut_per_mille, cut_families, added, excluded });
    std::fs::write(request.output.join(RECEIPT_FILE), ks_text::json::dumps(&receipt, &readable()) + "\n").map_err(|error| error.to_string())?;
    Ok(receipt)
}
