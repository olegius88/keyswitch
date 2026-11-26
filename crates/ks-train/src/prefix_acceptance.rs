//! The acceptance samples of a prefix candidate and their one replay (`prefix-acceptance build|run`).
//!
//! The samples come from two sources no prefix fit reads: the acceptance split of the word corpus (A1 its paired words
//! typed in the other layout, A2 the same words typed as meant, A3 its tokens typed only as meant and its hard paired
//! words typed as meant) and plain text (N1 its words of letters only typed in the other layout, N2 its words typed as
//! meant), without the words any prefix selection or held-out sample used. A row is one token typed first in an empty
//! text field and followed by a space, in the default mode of sequence protocol 5, in each lexical profile; A3 is
//! replayed twice, with the term lists open and with them closed to its tokens. Only counts are written: no text of a
//! sample is printed, before the run or after it.
use ks_cast::To as _;
use crate::corpus::{CALIBRATION, DEVELOPMENT, checksum, load_rows};
use crate::evaluate::{EARLY_KIND, PROFILES, first_word, script_group, trimmed_window, typed_first};
use crate::physical::{expanded_aliases, hex};
use crate::prefix_words::{ACCEPTANCE_SPLIT, draw, held_out_of, read_split};
use crate::replay::{Mode, Plan, Replayer, prefix_indexes, prefix_indexes_of};
use crate::sequence_report::Case;
use crate::tools::{Lexicons, Tools, reference_models};
use flate2::Compression;
use flate2::read::MultiGzDecoder;
use flate2::write::GzEncoder;
use ks_context::VersionedPrefixModel;
use ks_context::context_model::ContextModel;
use ks_context::prefix_terms::PrefixTerms;
use ks_lang::Resources;
use ks_lang::early_switch::{PrefixIndex, PrefixLexicon};
use ks_text::{Indent, JsonOptions, pystr};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::{BufRead, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// The rows of every sample, the words of N2 for the count without the engine, and what the samples were made of.
pub const ROWS_FILE: &str = "rows.jsonl.gz";
pub const PLAIN_AS_MEANT_FILE: &str = "n2-words.txt";
/// The words of N2 the development documents of the bases hold: the source the samples of plain text first had.
pub const PLAIN_DEVELOPMENT_FILE: &str = "n2-development-words.txt";
/// The names of the sources of plain text, as the receipt counts them.
const SOURCE_TEXT: &str = "text";
pub const RECEIPT_FILE: &str = "receipt.json";
pub const SAMPLES_KIND: &str = "prefix-acceptance-samples";
pub const REPORT_KIND: &str = "prefix-acceptance-report";
/// The fewest paired words of the acceptance split a direction of A1 holds, the fewest words of plain text a direction of
/// N2 holds for the count without the engine, and the words of plain text a direction of N1 and of N2 the engine types.
pub const MINIMUM_PAIRED_WORDS_PER_DIRECTION: usize = 2000;
pub const MINIMUM_PLAIN_WORDS_PER_DIRECTION: usize = 20_000;
pub const ENGINE_PLAIN_WORDS_PER_DIRECTION: usize = 5000;
/// The fewest letters of a word of plain text.
pub const MINIMUM_PLAIN_LETTERS: usize = 4;
/// The key a switch is counted by: the first key the engine asks the prefix model about.
pub const SWITCH_KEY: usize = 4;
/// Hex digits of an alias compared with the held-out ones: what a word corpus keeps of each.
const HELD_OUT_ALIAS_HEX_DIGITS: usize = 16;
/// The rows a replay task types: enough that loading the context model is a small part of it.
const ROWS_PER_TASK: usize = 400;
/// The name the samples give a word of plain text, in place of a class of the word corpus.
const PLAIN_CLASS: &str = "plain";

/// The samples of the pre-registration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Sample {
    A1,
    A2,
    A3,
    N1,
    N2,
}

impl Sample {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Sample::A1 => "A1",
            Sample::A2 => "A2",
            Sample::A3 => "A3",
            Sample::N1 => "N1",
            Sample::N2 => "N2",
        }
    }
}

/// A row of a sample: one word, typed as meant or in the other layout.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptanceRow {
    pub sample: Sample,
    /// The word as meant.
    pub text: String,
    /// The layout it is meant in: 0 Latin, 1 Cyrillic.
    pub group: i64,
    /// Its class in the word corpus, or `plain` for a word of plain text.
    pub class: String,
    /// Typed in the other layout; otherwise as meant.
    pub other: bool,
    /// Typed through the engine: every row of A1 to A3, the share of N1 and N2 drawn by hash.
    pub engine: bool,
}

impl AcceptanceRow {
    /// The layout group the keys are typed in: the direction of a switch.
    #[must_use]
    pub fn direction(&self) -> usize {
        usize::from((self.group == 1) != self.other)
    }
}

/// How large the samples must be; the pre-registration's sizes by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Minimums {
    pub paired: usize,
    pub plain: usize,
    pub engine_plain: usize,
}

impl Default for Minimums {
    fn default() -> Minimums {
        Minimums { paired: MINIMUM_PAIRED_WORDS_PER_DIRECTION, plain: MINIMUM_PLAIN_WORDS_PER_DIRECTION, engine_plain: ENGINE_PLAIN_WORDS_PER_DIRECTION }
    }
}

/// What `prefix-acceptance build` reads.
pub struct BuildRequest<'a> {
    /// The word corpus whose acceptance split A1 to A3 are made of.
    pub corpus: &'a Path,
    /// Frozen base corpora whose development split is plain text.
    pub bases: &'a [PathBuf],
    /// Their calibration split too; never their training or test split.
    pub base_calibration: bool,
    /// Files of plain text, a text a line (gzip when named `.gz`).
    pub texts: &'a [PathBuf],
    /// Exposure inventories (`base inventory`): the aliases of every word a prefix fit read.
    pub inventories: &'a [PathBuf],
    /// Development samples and lists of words, as a word corpus's freeze holds them out.
    pub hold_out: &'a [PathBuf],
    pub namespace: &'a str,
    pub minimums: Minimums,
    pub output: &'a Path,
}

/// A file the samples read, by its SHA-256.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputFile {
    pub path: String,
    pub sha256: String,
}

/// The words of a sample, and of them those the engine types, by the layout group the keys are typed in.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SampleCount {
    pub words: [usize; 2],
    pub engine: [usize; 2],
}

/// `receipt.json` beside the samples: what they were made of and what they hold, in numbers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SamplesReceipt {
    pub kind: String,
    pub namespace: String,
    pub corpus_receipt_sha256: String,
    pub bases: Vec<InputFile>,
    pub texts: Vec<InputFile>,
    pub inventories: Vec<InputFile>,
    pub hold_out: Vec<InputFile>,
    pub held_out_aliases: usize,
    pub minimums: Minimums,
    pub counts: BTreeMap<String, SampleCount>,
    /// The rows left out because the replay cannot type them (a key the layout has not), by sample and by direction.
    pub untyped: BTreeMap<String, [usize; 2]>,
    /// The words of N2 by where they were found (`development`, `calibration` of the bases, `text`), by direction; a word
    /// found in more than one source counts in each.
    pub plain_by_source: BTreeMap<String, [usize; 2]>,
    pub sha256: BTreeMap<String, String>,
    pub scope: String,
}

fn readable() -> JsonOptions {
    JsonOptions { ensure_ascii: false, indent: Some(Indent::Spaces(1)), sort_keys: true, ..JsonOptions::default() }
}

fn compact() -> JsonOptions {
    JsonOptions { ensure_ascii: false, separators: Some((",".to_owned(), ":".to_owned())), sort_keys: true, ..JsonOptions::default() }
}

fn input_files(paths: &[PathBuf]) -> Result<Vec<InputFile>, String> {
    paths.iter().map(|path| Ok(InputFile { path: path.display().to_string(), sha256: checksum(path)? })).collect()
}

fn read_text(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    if path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("gz")) {
        let mut text = String::new();
        MultiGzDecoder::new(bytes.as_slice()).read_to_string(&mut text).map_err(|error| format!("{}: {error}", path.display()))?;
        Ok(text)
    } else {
        String::from_utf8(bytes).map_err(|error| format!("{}: {error}", path.display()))
    }
}

/// A word of plain text: a token between spaces without the signs around it, all of it letters of one layout, at least
/// `MINIMUM_PLAIN_LETTERS` of them; case-folded.
#[must_use]
pub fn plain_word(token: &str) -> Option<(String, i64)> {
    let trimmed = token.trim_matches(|character: char| !character.is_alphanumeric());
    let group = trimmed.chars().next().and_then(script_group)?;
    let letters = trimmed.chars().all(|character| script_group(character) == Some(group));
    (letters && trimmed.chars().count() >= MINIMUM_PLAIN_LETTERS).then(|| (pystr::casefold(trimmed), group))
}

/// The words of plain text in a text.
fn words_of(text: &str, found: &mut BTreeSet<(String, i64)>) {
    found.extend(text.split(|character: char| pystr::isspace(character.encode_utf8(&mut [0_u8; 4]))).filter_map(plain_word));
}

/// Whether a word's keys read as letters in the other layout too: a word of N1.
#[must_use]
pub fn letters_both_ways(text: &str, group: i64) -> bool {
    let (from, to) = if group == 1 { ("ru", "us") } else { ("us", "ru") };
    crate::physical::pair().translate(text, from, to).is_ok_and(|other| pystr::isalpha(&other))
}

/// The first hex digits of every alias of a word, as the held-out aliases are kept.
fn aliases_of(text: &str) -> impl Iterator<Item = String> {
    expanded_aliases(text).into_iter().map(|alias| alias.get(..HELD_OUT_ALIAS_HEX_DIGITS).unwrap_or_default().to_owned())
}

/// The aliases an exposure inventory lists.
fn inventory_aliases(path: &Path) -> Result<Vec<String>, String> {
    /// What the samples read of an inventory.
    #[derive(Deserialize)]
    struct Inventory {
        aliases_sha256: Vec<String>,
    }
    let inventory: Inventory = ks_text::json::loads(std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(inventory.aliases_sha256.into_iter().map(|alias| alias.get(..HELD_OUT_ALIAS_HEX_DIGITS).unwrap_or_default().to_owned()).collect())
}

fn gzip(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(bytes).map_err(|error| error.to_string())?;
    encoder.finish().map_err(|error| error.to_string())
}

/// The words of a sample of plain text, typed as meant (N2) or in the other layout (N1), those the engine types drawn by
/// hash in each direction.
fn plain_rows(words: &BTreeSet<(String, i64)>, sample: Sample, namespace: &str, engine: usize, untyped: &mut BTreeMap<String, [usize; 2]>) -> Vec<AcceptanceRow> {
    let other = sample == Sample::N1;
    let rows: Vec<AcceptanceRow> = words
        .iter()
        .filter(|(text, group)| !other || letters_both_ways(text, *group))
        .map(|(text, group)| AcceptanceRow { sample, text: text.clone(), group: *group, class: PLAIN_CLASS.to_owned(), other, engine: false })
        .collect();
    // A row the replay cannot type is left out before the engine's share is drawn.
    let mut rows = typable(rows, untyped);
    for direction in 0..2 {
        let mut drawn: Vec<&mut AcceptanceRow> = rows.iter_mut().filter(|row| row.direction() == direction).collect();
        drawn.sort_by_cached_key(|row| (draw(&format!("{namespace}:engine:{}:{}:{}", sample.as_str(), row.group, row.text)), row.text.clone()));
        for row in drawn.into_iter().take(engine) {
            row.engine = true;
        }
    }
    rows
}

/// The rows the replay can type, every key of them a key of a layout; the others are counted by sample and direction.
fn typable(rows: Vec<AcceptanceRow>, untyped: &mut BTreeMap<String, [usize; 2]>) -> Vec<AcceptanceRow> {
    rows.into_iter()
        .filter(|row| {
            let typed = row_plan(0, row).unsupported.is_empty();
            if !typed {
                untyped.entry(row.sample.as_str().to_owned()).or_default()[row.direction()] += 1;
            }
            typed
        })
        .collect()
}

/// The splits of a base corpus whose texts are plain text: development, and calibration when asked.
fn base_splits(calibration: bool) -> &'static [&'static str] {
    if calibration { &[DEVELOPMENT, CALIBRATION] } else { &[DEVELOPMENT] }
}

/// The counts of the rows of each sample.
fn sample_counts(rows: &[AcceptanceRow]) -> BTreeMap<String, SampleCount> {
    let mut counts: BTreeMap<String, SampleCount> = [Sample::A1, Sample::A2, Sample::A3, Sample::N1, Sample::N2].iter().map(|sample| (sample.as_str().to_owned(), SampleCount::default())).collect();
    for row in rows {
        let count = counts.entry(row.sample.as_str().to_owned()).or_default();
        count.words[row.direction()] += 1;
        count.engine[row.direction()] += usize::from(row.engine);
    }
    counts
}

/// `prefix-acceptance build`: the samples, written once into a new folder.
pub fn build(request: &BuildRequest<'_>) -> Result<SamplesReceipt, String> {
    if request.output.join(RECEIPT_FILE).exists() {
        return Err("refusing to overwrite acceptance samples".to_owned());
    }
    // A1 to A3: the acceptance split, each word once.
    let mut words: BTreeMap<(String, i64), (crate::prefix_words::Class, bool)> = BTreeMap::new();
    for row in read_split(request.corpus, ACCEPTANCE_SPLIT)? {
        let hard = row.hard || row.class.hard(row.group);
        let entry = words.entry((row.text, row.group)).or_insert((row.class, hard));
        entry.1 |= hard;
    }
    let mut drawn: Vec<AcceptanceRow> = Vec::new();
    for ((text, group), (class, hard)) in &words {
        let row = |sample: Sample, other: bool| AcceptanceRow { sample, text: text.clone(), group: *group, class: class.as_str().to_owned(), other, engine: true };
        if class.paired() {
            drawn.push(row(Sample::A1, true));
            drawn.push(row(Sample::A2, false));
        }
        if !class.paired() || *hard {
            drawn.push(row(Sample::A3, false));
        }
    }
    let mut untyped: BTreeMap<String, [usize; 2]> = BTreeMap::new();
    let mut rows = typable(drawn, &mut untyped);
    // N1 and N2: plain text, without every word a prefix selection, a held-out sample or the acceptance split holds.
    let mut held: HashSet<String> = HashSet::new();
    for path in request.inventories {
        held.extend(inventory_aliases(path)?);
    }
    let held_out = held_out_of(request.hold_out)?;
    held.extend(held_out.aliases.iter().cloned());
    for (text, _) in words.keys() {
        held.extend(aliases_of(text));
    }
    // The words of each source, and their union.
    let mut by_source: BTreeMap<&'static str, BTreeSet<(String, i64)>> = BTreeMap::new();
    let splits = base_splits(request.base_calibration);
    for base in request.bases {
        for split in splits {
            for row in load_rows(base, split)? {
                let (before, after, _, _) = trimmed_window(&row);
                words_of(&format!("{before}{}{after}", row.original), by_source.entry(split).or_default());
            }
        }
    }
    for path in request.texts {
        words_of(&read_text(path)?, by_source.entry(SOURCE_TEXT).or_default());
    }
    let mut plain: BTreeSet<(String, i64)> = by_source.values().flatten().cloned().collect();
    plain.retain(|(text, _)| !aliases_of(text).any(|alias| held.contains(&alias)));
    rows.extend(plain_rows(&plain, Sample::N1, request.namespace, request.minimums.engine_plain, &mut untyped));
    rows.extend(plain_rows(&plain, Sample::N2, request.namespace, request.minimums.engine_plain, &mut untyped));
    rows.sort();
    let in_source = |source: &str, row: &AcceptanceRow| by_source.get(source).is_some_and(|found| found.contains(&(row.text.clone(), row.group)));
    let mut plain_by_source: BTreeMap<String, [usize; 2]> = BTreeMap::new();
    for row in rows.iter().filter(|row| row.sample == Sample::N2) {
        for source in by_source.keys().filter(|source| in_source(source, row)) {
            plain_by_source.entry((*source).to_owned()).or_default()[row.direction()] += 1;
        }
    }
    let counts = sample_counts(&rows);
    let minimums = request.minimums;
    for direction in 0..2 {
        let (paired, as_meant, other) = (&counts[Sample::A1.as_str()], &counts[Sample::N2.as_str()], &counts[Sample::N1.as_str()]);
        let sizes = [("A1", paired.words, minimums.paired), ("N2", as_meant.words, minimums.plain), ("N2 for the engine", as_meant.engine, minimums.engine_plain), ("N1 for the engine", other.engine, minimums.engine_plain)];
        if let Some((name, found, asked)) = sizes.iter().find(|(_, found, asked)| found[direction] < *asked) {
            return Err(format!("typed in layout group {direction}: {name} holds {} words, the samples ask for {asked}", found[direction]));
        }
    }
    let lines: String = rows.iter().map(|row| ks_text::json::dumps(row, &compact()) + "\n").collect();
    let packed = gzip(lines.as_bytes())?;
    let as_meant: String = rows.iter().filter(|row| row.sample == Sample::N2).map(|row| format!("{}\n", row.text)).collect();
    let of_development: String = rows.iter().filter(|row| row.sample == Sample::N2 && in_source(DEVELOPMENT, row)).map(|row| format!("{}\n", row.text)).collect();
    std::fs::create_dir_all(request.output).map_err(|error| error.to_string())?;
    std::fs::write(request.output.join(ROWS_FILE), &packed).map_err(|error| error.to_string())?;
    std::fs::write(request.output.join(PLAIN_AS_MEANT_FILE), as_meant.as_bytes()).map_err(|error| error.to_string())?;
    std::fs::write(request.output.join(PLAIN_DEVELOPMENT_FILE), of_development.as_bytes()).map_err(|error| error.to_string())?;
    let receipt = SamplesReceipt {
        kind: SAMPLES_KIND.to_owned(),
        namespace: request.namespace.to_owned(),
        corpus_receipt_sha256: checksum(&request.corpus.join("corpus.json"))?,
        bases: input_files(&request.bases.iter().flat_map(|base| splits.iter().map(move |split| base.join(format!("{split}.jsonl.gz")))).collect::<Vec<_>>())?,
        texts: input_files(request.texts)?,
        inventories: input_files(request.inventories)?,
        hold_out: input_files(request.hold_out)?,
        held_out_aliases: held.len(),
        minimums,
        counts,
        untyped,
        plain_by_source,
        sha256: [(ROWS_FILE, packed.as_slice()), (PLAIN_AS_MEANT_FILE, as_meant.as_bytes()), (PLAIN_DEVELOPMENT_FILE, of_development.as_bytes())].into_iter().map(|(name, bytes)| (name.to_owned(), hex(&Sha256::digest(bytes)))).collect(),
        scope: "A1 to A3 from the acceptance split of the word corpus, which no fit reads; N1 and N2 from the development split (and, when asked, the calibration split) of frozen base corpora and files of plain text, without the words any prefix selection, held-out sample or the acceptance split holds, by their physical aliases; N2 counted by source, the words of the development documents also listed apart. A row is one token typed first in an empty text field and followed by a space; a row the replay cannot type is left out and counted; only counts are read.".to_owned(),
    };
    std::fs::write(request.output.join(RECEIPT_FILE), ks_text::json::dumps(&receipt, &readable()) + "\n").map_err(|error| error.to_string())?;
    Ok(receipt)
}

/// The receipt of a folder of samples, and its rows, their checksum checked.
pub fn read_samples(folder: &Path) -> Result<(SamplesReceipt, Vec<AcceptanceRow>), String> {
    let receipt: SamplesReceipt = ks_text::json::loads(std::fs::read(folder.join(RECEIPT_FILE)).map_err(|error| format!("{}: {error}", folder.display()))?).map_err(|error| format!("{RECEIPT_FILE}: {error}"))?;
    if receipt.kind != SAMPLES_KIND {
        return Err(format!("{} holds no acceptance samples", folder.display()));
    }
    let path = folder.join(ROWS_FILE);
    if receipt.sha256.get(ROWS_FILE) != Some(&checksum(&path)?) {
        return Err("the acceptance rows differ from their receipt".to_owned());
    }
    let file = std::fs::File::open(&path).map_err(|error| error.to_string())?;
    let rows = std::io::BufReader::new(MultiGzDecoder::new(file)).lines().map(|line| ks_text::json::loads(line.map_err(|error| error.to_string())?).map_err(|error| format!("an acceptance row: {error}"))).collect::<Result<Vec<AcceptanceRow>, String>>()?;
    Ok((receipt, rows))
}

/// A pair replayed: a context model and a prefix model, under a name.
#[derive(Clone, Debug)]
pub struct NamedPair {
    pub name: String,
    pub context: PathBuf,
    pub prefix: PathBuf,
    /// The words of the prefix index when not those the prefix model runs with (`VersionedPrefixModel::prefix_lexicon`):
    /// the installed pair as it was measured before the program and the tools gave it one index (`M`), beside the pair
    /// as users run it.
    pub lexicon: Option<PrefixLexicon>,
}

/// Which corrections give a row typed in the other layout its key in the mean: one of kind early, or one of any kind.
/// The gate holds both.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyKind {
    Early,
    Any,
}

/// What `prefix-acceptance run` reads.
pub struct RunRequest<'a> {
    pub root: &'a Path,
    pub resource_dir: &'a Path,
    pub samples: &'a Path,
    pub pairs: &'a [NamedPair],
    /// The pair the others are held against: the installed one.
    pub reference: &'a str,
    pub jobs: usize,
    pub output: &'a Path,
}

/// What one row's replay says, as the counts read it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Observed {
    /// The replay failed.
    pub error: bool,
    /// The final text is the text typed as meant.
    pub exact: bool,
    /// Its first word is the first word meant.
    pub right: bool,
    /// The letters of the word.
    pub letters: usize,
    /// Typed in the other layout: the key of the first correction into the layout meant, and whether it was early.
    pub first_into: Option<(usize, bool)>,
    /// Typed in the other layout: the kind of the first correction away from the layout meant after an early switch into
    /// it by the word's last letter, when the first word ends wrong; `none` when no correction undid it.
    pub undone_by: Option<String>,
    /// Typed as meant: a correction of kind early away from the layout typed in, at any key.
    pub false_early: bool,
}

impl Observed {
    /// What the replay of a row says.
    #[must_use]
    pub fn of(row: &AcceptanceRow, case: &Case) -> Observed {
        let early = |kind: Option<&String>| kind.is_some_and(|kind| kind == EARLY_KIND);
        let letters = case.focus_letters.map_or(0, ks_cast::To::to::<usize>);
        let right = first_word(&case.actual) == first_word(&case.expected);
        let mut observed = Observed { error: case.error.is_some(), exact: case.exact, right, letters, ..Observed::default() };
        if row.other {
            let first = case.corrections.iter().position(|item| item.target_group == row.group);
            observed.first_into = first.map(|index| (case.corrections[index].event, early(case.corrections[index].kind.as_ref())));
            if let (Some(index), Some((key, true)), false) = (first, observed.first_into, right)
                && key <= letters
            {
                let undoing = case.corrections.iter().skip(index + 1).find(|item| item.target_group != row.group);
                observed.undone_by = Some(undoing.and_then(|item| item.kind.clone()).unwrap_or_else(|| "none".to_owned()));
            }
        } else {
            observed.false_early = case.corrections.iter().any(|item| early(item.kind.as_ref()) && item.target_group != row.group);
        }
        observed
    }

    /// The key a row typed in the other layout counts in the mean: that of its first correction into the layout meant,
    /// of the kind asked, when not above the word's letters and the first word ends right; the key after the last letter
    /// otherwise.
    #[must_use]
    pub fn key(&self, kind: KeyKind) -> usize {
        match self.first_into {
            Some((key, early)) if self.right && key <= self.letters && (early || kind == KeyKind::Any) => key,
            _ => self.letters + 1,
        }
    }

    fn switched_by_key(&self) -> bool {
        self.right && self.first_into.is_some_and(|(key, early)| early && key <= SWITCH_KEY)
    }

    /// Switched inside the word: a first correction of kind early at a key not above the word's letters.
    fn inside_word(&self) -> bool {
        self.right && self.first_into.is_some_and(|(key, early)| early && key <= self.letters)
    }
}

/// A count of rows and of distinct words: a word counts when any of its rows shows the event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Count {
    pub rows: usize,
    pub words: usize,
}

/// How early a pair switched the rows of a sample typed in the other layout, in one direction, both profiles together.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Earliness {
    pub rows: usize,
    pub words: usize,
    /// The keys of `Observed::key`, of an early first correction and of any, added up, and their means over the rows.
    pub early_key_sum: usize,
    pub any_key_sum: usize,
    pub early_mean_key: f64,
    pub any_mean_key: f64,
    /// A first correction into the layout meant of kind early by `SWITCH_KEY`, and not after the word's last letter, with
    /// the first word right in the end.
    pub switched_by_key: Count,
    pub inside_word: Count,
    pub first_word_wrong: Count,
    pub started_right_ended_wrong: Count,
    /// Of the rows started right and ended wrong, by the kind of the correction that undid the switch.
    pub undone_by: BTreeMap<String, usize>,
    pub exact: Count,
    pub execution_errors: usize,
}

impl Earliness {
    /// The mean key of the kind asked.
    #[must_use]
    pub fn mean_key(&self, kind: KeyKind) -> f64 {
        if kind == KeyKind::Early { self.early_mean_key } else { self.any_mean_key }
    }
}

/// The rows typed as meant of a cell, both profiles together.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Safety {
    pub rows: usize,
    pub words: usize,
    pub false_early: Count,
    /// Of those, the rows whose final text is not the text typed.
    pub stays: Count,
    /// The final text is not the text typed, whatever corrected it.
    pub corrupted: Count,
    pub execution_errors: usize,
}

/// A word and how a row of it was typed, which is what makes rows of one word.
type WordKey = (String, i64);

/// A count of flags over rows, by row and by distinct word.
fn counted<'a>(rows: impl Iterator<Item = (&'a WordKey, bool)>) -> Count {
    let mut words: BTreeSet<&WordKey> = BTreeSet::new();
    let mut count = Count::default();
    for (key, _) in rows.filter(|(_, flag)| *flag) {
        count.rows += 1;
        words.insert(key);
    }
    count.words = words.len();
    count
}

fn distinct<'a>(rows: impl Iterator<Item = &'a WordKey>) -> usize {
    rows.collect::<BTreeSet<_>>().len()
}

/// The earliness of rows typed in the other layout.
#[must_use]
pub fn earliness(rows: &[(WordKey, Observed)]) -> Earliness {
    let mean = |sum: usize| if rows.is_empty() { 0.0 } else { sum.to::<f64>() / rows.len().to::<f64>() };
    let (early_key_sum, any_key_sum) = (rows.iter().map(|(_, row)| row.key(KeyKind::Early)).sum(), rows.iter().map(|(_, row)| row.key(KeyKind::Any)).sum());
    let mut undone_by: BTreeMap<String, usize> = BTreeMap::new();
    for kind in rows.iter().filter_map(|(_, row)| row.undone_by.as_ref()) {
        *undone_by.entry(kind.clone()).or_insert(0) += 1;
    }
    Earliness {
        rows: rows.len(),
        words: distinct(rows.iter().map(|(key, _)| key)),
        early_key_sum,
        any_key_sum,
        early_mean_key: mean(early_key_sum),
        any_mean_key: mean(any_key_sum),
        switched_by_key: counted(rows.iter().map(|(key, row)| (key, row.switched_by_key()))),
        inside_word: counted(rows.iter().map(|(key, row)| (key, row.inside_word()))),
        first_word_wrong: counted(rows.iter().map(|(key, row)| (key, !row.right))),
        started_right_ended_wrong: counted(rows.iter().map(|(key, row)| (key, row.undone_by.is_some()))),
        undone_by,
        exact: counted(rows.iter().map(|(key, row)| (key, row.exact))),
        execution_errors: rows.iter().filter(|(_, row)| row.error).count(),
    }
}

/// The safety of rows typed as meant.
#[must_use]
pub fn safety<'a>(rows: impl Iterator<Item = &'a (WordKey, Observed)> + Clone) -> Safety {
    Safety {
        rows: rows.clone().count(),
        words: distinct(rows.clone().map(|(key, _)| key)),
        false_early: counted(rows.clone().map(|(key, row)| (key, row.false_early))),
        stays: counted(rows.clone().map(|(key, row)| (key, row.false_early && !row.exact))),
        corrupted: counted(rows.clone().map(|(key, row)| (key, !row.exact))),
        execution_errors: rows.filter(|(_, row)| row.error).count(),
    }
}

/// The part of the samples a replay types: every row with the term lists the engine loads, or the rows of A3 with the
/// lists closed to its tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Part {
    Open,
    Closed,
}

/// One replayed row of a pair: its part, its profile and what it says.
#[derive(Clone, Debug)]
struct Replayed {
    part: Part,
    profile: usize,
    row: usize,
    observed: Observed,
}

/// What a pair's replay gives, in numbers.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairReport {
    pub context_sha256: String,
    pub prefix_sha256: String,
    pub prefix_version: String,
    /// A1 and N1, by direction.
    pub earliness: BTreeMap<String, [Earliness; 2]>,
    /// A2, A3 with the lists open and closed, and N2 together, by direction (`latin`, `cyrillic`) and in all.
    pub safety: BTreeMap<String, Safety>,
    /// The same by sample (`A3_closed` the second replay of A3), by profile and by class, each by direction.
    pub safety_by_sample: BTreeMap<String, [Safety; 2]>,
    pub safety_by_profile: BTreeMap<String, [Safety; 2]>,
    pub safety_by_class: BTreeMap<String, [Safety; 2]>,
}

/// The cells of the safety gates.
pub const SAFETY_CELLS: [&str; 3] = ["latin", "cyrillic", "all"];

/// A pair's report from its replayed rows.
fn pair_report(rows: &[AcceptanceRow], replayed: &[Replayed]) -> PairReport {
    let key_of = |item: &Replayed| (rows[item.row].text.clone(), rows[item.row].group);
    let mut report = PairReport::default();
    for sample in [Sample::A1, Sample::N1] {
        let by_direction = [0, 1].map(|direction| {
            // Every row of A1 and N1 is typed in the other layout.
            let found: Vec<(WordKey, Observed)> = replayed.iter().filter(|item| rows[item.row].sample == sample && rows[item.row].direction() == direction).map(|item| (key_of(item), item.observed.clone())).collect();
            earliness(&found)
        });
        report.earliness.insert(sample.as_str().to_owned(), by_direction);
    }
    // A word typed as meant is the same word in every sample: A3 holds the hard words of A2.
    let as_meant: Vec<(&Replayed, (WordKey, Observed))> = replayed.iter().filter(|item| !rows[item.row].other).map(|item| (item, (key_of(item), item.observed.clone()))).collect();
    let cell = |filter: &dyn Fn(&Replayed) -> bool| safety(as_meant.iter().filter(|(item, _)| filter(item)).map(|(_, row)| row));
    report.safety.insert(SAFETY_CELLS[0].to_owned(), cell(&|item| rows[item.row].direction() == 0));
    report.safety.insert(SAFETY_CELLS[1].to_owned(), cell(&|item| rows[item.row].direction() == 1));
    report.safety.insert(SAFETY_CELLS[2].to_owned(), cell(&|_| true));
    let sample_name = |item: &Replayed| if item.part == Part::Closed { "A3_closed".to_owned() } else { rows[item.row].sample.as_str().to_owned() };
    let mut names: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    for (item, _) in &as_meant {
        names.entry("sample").or_default().insert(sample_name(item));
        names.entry("profile").or_default().insert(PROFILES[item.profile].to_owned());
        names.entry("class").or_default().insert(rows[item.row].class.clone());
    }
    let split = |kind: &str, name_of: &dyn Fn(&Replayed) -> String| -> BTreeMap<String, [Safety; 2]> {
        names.get(kind).into_iter().flatten().map(|name| (name.clone(), [0, 1].map(|direction| cell(&|item| name_of(item) == *name && rows[item.row].direction() == direction)))).collect()
    };
    report.safety_by_sample = split("sample", &sample_name);
    report.safety_by_profile = split("profile", &|item| PROFILES[item.profile].to_owned());
    report.safety_by_class = split("class", &|item| rows[item.row].class.clone());
    report
}

/// E1 for one sample and direction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EarlierGate {
    /// The mean key lower, from corrections of kind early and from corrections of any kind.
    pub early_mean_key_lower: bool,
    pub any_mean_key_lower: bool,
    /// Switched by key 4, and switched inside the word: not fewer, by rows and by distinct words.
    pub switched_by_key_not_fewer: CountGate,
    pub inside_word_not_fewer: CountGate,
}

/// A count of rows and of distinct words, each held to the reference's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CountGate {
    pub rows: bool,
    pub words: bool,
}

impl CountGate {
    fn not_fewer(ours: Count, theirs: Count) -> CountGate {
        CountGate { rows: ours.rows >= theirs.rows, words: ours.words >= theirs.words }
    }
}

/// S1 for one cell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SafetyGate {
    pub false_early_not_more: bool,
    pub stays_not_more: bool,
}

impl SafetyGate {
    fn of(ours: &Safety, theirs: &Safety) -> SafetyGate {
        SafetyGate { false_early_not_more: ours.false_early.words <= theirs.false_early.words, stays_not_more: ours.stays.words <= theirs.stays.words }
    }
}

/// The samples S1 and S2 also hold alone, in each direction: a gain on the tokens of A3 pays for no loss on ordinary words.
pub const SAMPLES_ALONE: [Sample; 2] = [Sample::A2, Sample::N2];

/// The gates of a pair held against the reference.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gates {
    pub e1: BTreeMap<String, [EarlierGate; 2]>,
    /// First words wrong in the end, distinct words: not more.
    pub e2: BTreeMap<String, [bool; 2]>,
    /// Started right, ended wrong, distinct words: not more; gated only for a pair with the M1 head.
    pub e2_started_right_ended_wrong: BTreeMap<String, [bool; 2]>,
    /// By cell (`latin`, `cyrillic`, `all`), and on A2 alone and N2 alone in each direction.
    pub s1: BTreeMap<String, SafetyGate>,
    pub s1_alone: BTreeMap<String, [SafetyGate; 2]>,
    /// Corruption by cell and on A2 alone and N2 alone in each direction: not more.
    pub s2_corruption: BTreeMap<String, bool>,
    pub s2_corruption_alone: BTreeMap<String, [bool; 2]>,
    /// Exact restorations of A1 and N1, in each direction, by rows and by distinct words: not fewer.
    pub s2_exact: BTreeMap<String, [CountGate; 2]>,
    /// No row of either pair failed to replay.
    pub execution: bool,
    /// E1, E2's first sentence, S1, S2 and the execution.
    pub all_held: bool,
}

/// A pair against the reference.
#[must_use]
pub fn gates(candidate: &PairReport, reference: &PairReport) -> Gates {
    let mut gates = Gates::default();
    for (sample, ours) in &candidate.earliness {
        let theirs = &reference.earliness[sample];
        let by = |test: &dyn Fn(&Earliness, &Earliness) -> bool| [0, 1].map(|direction| test(&ours[direction], &theirs[direction]));
        let earlier = [0, 1].map(|direction| {
            let (ours, theirs) = (&ours[direction], &theirs[direction]);
            EarlierGate {
                early_mean_key_lower: ours.mean_key(KeyKind::Early) < theirs.mean_key(KeyKind::Early),
                any_mean_key_lower: ours.mean_key(KeyKind::Any) < theirs.mean_key(KeyKind::Any),
                switched_by_key_not_fewer: CountGate::not_fewer(ours.switched_by_key, theirs.switched_by_key),
                inside_word_not_fewer: CountGate::not_fewer(ours.inside_word, theirs.inside_word),
            }
        });
        gates.e1.insert(sample.clone(), earlier);
        gates.e2.insert(sample.clone(), by(&|ours, theirs| ours.first_word_wrong.words <= theirs.first_word_wrong.words));
        gates.e2_started_right_ended_wrong.insert(sample.clone(), by(&|ours, theirs| ours.started_right_ended_wrong.words <= theirs.started_right_ended_wrong.words));
        gates.s2_exact.insert(sample.clone(), [0, 1].map(|direction| CountGate::not_fewer(ours[direction].exact, theirs[direction].exact)));
    }
    for (cell, ours) in &candidate.safety {
        let theirs = &reference.safety[cell];
        gates.s1.insert(cell.clone(), SafetyGate::of(ours, theirs));
        gates.s2_corruption.insert(cell.clone(), ours.corrupted.words <= theirs.corrupted.words);
    }
    let none = [Safety::default(), Safety::default()];
    for sample in SAMPLES_ALONE.map(Sample::as_str) {
        let (ours, theirs) = (candidate.safety_by_sample.get(sample).unwrap_or(&none), reference.safety_by_sample.get(sample).unwrap_or(&none));
        gates.s1_alone.insert(sample.to_owned(), [0, 1].map(|direction| SafetyGate::of(&ours[direction], &theirs[direction])));
        gates.s2_corruption_alone.insert(sample.to_owned(), [0, 1].map(|direction| ours[direction].corrupted.words <= theirs[direction].corrupted.words));
    }
    let errors = |report: &PairReport| report.safety.get(SAFETY_CELLS[2]).map_or(0, |cell| cell.execution_errors) + report.earliness.values().flatten().map(|cell| cell.execution_errors).sum::<usize>();
    gates.execution = errors(candidate) + errors(reference) == 0;
    let mut held: Vec<bool> = vec![gates.execution];
    let counted = |gate: CountGate| [gate.rows, gate.words];
    held.extend(gates.e1.values().flatten().flat_map(|gate| [[gate.early_mean_key_lower, gate.any_mean_key_lower], counted(gate.switched_by_key_not_fewer), counted(gate.inside_word_not_fewer)]).flatten());
    held.extend(gates.e2.values().flatten().copied());
    let safe = |gate: &SafetyGate| [gate.false_early_not_more, gate.stays_not_more];
    held.extend(gates.s1.values().flat_map(safe));
    held.extend(gates.s1_alone.values().flatten().flat_map(safe));
    held.extend(gates.s2_corruption.values().copied());
    held.extend(gates.s2_corruption_alone.values().flatten().copied());
    held.extend(gates.s2_exact.values().flatten().copied().flat_map(counted));
    gates.all_held = !held.contains(&false);
    gates
}

/// `prefix-acceptance run`'s report: numbers only.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptanceReport {
    pub kind: String,
    pub samples_receipt_sha256: String,
    pub counts: BTreeMap<String, SampleCount>,
    pub reference: String,
    pub pairs: BTreeMap<String, PairReport>,
    pub gates: BTreeMap<String, Gates>,
}

/// The plan of a row: the word typed first in an empty field, a space after it.
#[must_use]
pub fn row_plan(index: usize, row: &AcceptanceRow) -> Plan {
    typed_first(format!("acceptance:{index}"), "acceptance".to_owned(), row.group, &row.text, " ", 0, row.other)
}

/// One task of a run: a pair, a profile, a part, and the rows it types.
struct Task {
    pair: usize,
    profile: usize,
    part: Part,
    rows: Vec<usize>,
}

/// `prefix-acceptance run`: every pair's replay of the samples, and the gates of every pair against the reference.
pub fn run(request: &RunRequest<'_>) -> Result<AcceptanceReport, String> {
    if request.output.exists() {
        return Err("refusing to overwrite an acceptance report".to_owned());
    }
    if !request.pairs.iter().any(|pair| pair.name == request.reference) {
        return Err(format!("no pair is named {}", request.reference));
    }
    let (receipt, all_rows) = read_samples(request.samples)?;
    let rows: Vec<AcceptanceRow> = all_rows.into_iter().filter(|row| row.engine).collect();
    let plans: Vec<Plan> = rows.iter().enumerate().map(|(index, row)| row_plan(index, row)).collect();
    // The samples leave out every row the replay cannot type: one left in fails the run.
    let untyped = plans.iter().filter(|plan| !plan.unsupported.is_empty()).count();
    if untyped != 0 {
        return Err(format!("{untyped} rows of the samples cannot be typed"));
    }
    let typed: Vec<usize> = (0..rows.len()).collect();
    let resources: Arc<Resources> = Resources::with_speller(request.resource_dir.to_path_buf(), ks_hunspell::hunspell_speller(ks_config::history::data_dir()));
    let portable = reference_models(request.root, &resources, false)?;
    let spelling = reference_models(request.root, &resources, true)?;
    let tools = Tools::new(request.root, resources.clone(), portable.clone(), BTreeSet::new())?;
    let lexicons: [&Lexicons; 2] = [&portable, &spelling];
    let shared = crate::evaluate::shared_ortho(&resources, &portable);
    let models_dir = resources.resource_dir().join("models");
    let prefixes: Vec<Arc<VersionedPrefixModel>> = request.pairs.iter().map(|pair| VersionedPrefixModel::load(&pair.prefix).map(Arc::new).map_err(|error| format!("{}: {}", pair.prefix.display(), error.name))).collect::<Result<_, _>>()?;
    // Each pair's indexes for each profile: those its prefix model runs with, unless the pair names others.
    let indexes: Vec<[Vec<Arc<PrefixIndex>>; 2]> = request
        .pairs
        .iter()
        .zip(&prefixes)
        .map(|(pair, prefix)| lexicons.map(|models| pair.lexicon.map_or_else(|| prefix_indexes(&resources, models, Some(prefix)), |lexicon| prefix_indexes_of(&resources, models, lexicon))))
        .collect();
    // The term lists of each profile, as the engine builds them, and closed to the tokens of A3.
    let tokens: BTreeSet<String> = rows.iter().filter(|row| row.sample == Sample::A3).map(|row| pystr::casefold(&row.text)).collect();
    let open: Vec<Arc<PrefixTerms>> = lexicons.iter().map(|models| crate::prefix::prefix_terms_of(request.resource_dir, models).map(Arc::new)).collect::<Result<_, _>>()?;
    let closed: Vec<Arc<PrefixTerms>> = open.iter().map(|terms| Arc::new(terms.without(&tokens))).collect();
    let mut tasks: Vec<Task> = Vec::new();
    for pair in 0..request.pairs.len() {
        for profile in 0..PROFILES.len() {
            for chunk in typed.chunks(ROWS_PER_TASK) {
                tasks.push(Task { pair, profile, part: Part::Open, rows: chunk.to_vec() });
            }
            // Lists closed to the tokens change nothing for a model that reads none: its open replay stands for both.
            if prefixes[pair].needs_terms() {
                let third: Vec<usize> = typed.iter().copied().filter(|index| rows[*index].sample == Sample::A3).collect();
                for chunk in third.chunks(ROWS_PER_TASK) {
                    tasks.push(Task { pair, profile, part: Part::Closed, rows: chunk.to_vec() });
                }
            }
        }
    }
    let next = AtomicUsize::new(0);
    let done: Mutex<Vec<Vec<Replayed>>> = Mutex::new(vec![Vec::new(); request.pairs.len()]);
    let failure: Mutex<Option<String>> = Mutex::new(None);
    std::thread::scope(|scope| {
        for worker in 0..request.jobs.max(1).min(tasks.len()) {
            let (tasks, next, done, failure, plans, rows, tools, shared, indexes, prefixes, open, closed, models_dir) = (&tasks, &next, &done, &failure, &plans, &rows, &tools, &shared, &indexes, &prefixes, &open, &closed, &models_dir);
            scope.spawn(move || {
                while let Some(task) = tasks.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let pair = &request.pairs[task.pair];
                    let model = match ContextModel::load_artifact(&pair.context, models_dir) {
                        Ok(model) => model,
                        Err(error) => {
                            *failure.lock().expect("a worker finished") = Some(format!("{}: {}", pair.context.display(), error.name));
                            break;
                        }
                    };
                    let folder = std::env::temp_dir().join(format!("ks-acceptance-{}-{worker}", std::process::id()));
                    let terms = if task.part == Part::Closed { &closed[task.profile] } else { &open[task.profile] };
                    let mut replayer = Replayer::new(tools, lexicons[task.profile], shared.clone(), folder.clone(), model, Some(prefixes[task.pair].clone()), indexes[task.pair][task.profile].clone()).with_prefix_terms(terms.clone());
                    let found: Vec<Replayed> = task
                        .rows
                        .iter()
                        .map(|index| {
                            let case = Case::new(&plans[*index], &replayer.replay(&plans[*index], Mode::Default));
                            Replayed { part: task.part, profile: task.profile, row: *index, observed: Observed::of(&rows[*index], &case) }
                        })
                        .collect();
                    let _ = std::fs::remove_dir_all(folder);
                    done.lock().expect("a worker finished")[task.pair].extend(found);
                }
            });
        }
    });
    if let Some(error) = failure.into_inner().expect("the workers finished") {
        return Err(error);
    }
    let mut replayed = done.into_inner().expect("the workers finished");
    let mut pairs: BTreeMap<String, PairReport> = BTreeMap::new();
    for (index, pair) in request.pairs.iter().enumerate() {
        let found = &mut replayed[index];
        if !prefixes[index].needs_terms() {
            let again: Vec<Replayed> = found.iter().filter(|item| rows[item.row].sample == Sample::A3).map(|item| Replayed { part: Part::Closed, ..item.clone() }).collect();
            found.extend(again);
        }
        found.sort_by_key(|item| (item.part, item.profile, item.row));
        let mut report = pair_report(&rows, found);
        report.context_sha256 = checksum(&pair.context)?;
        report.prefix_sha256 = checksum(&pair.prefix)?;
        prefixes[index].version().clone_into(&mut report.prefix_version);
        pairs.insert(pair.name.clone(), report);
    }
    let reference = &pairs[request.reference];
    let gates = pairs.iter().filter(|(name, _)| *name != request.reference).map(|(name, report)| (name.clone(), gates(report, reference))).collect();
    let report = AcceptanceReport {
        kind: REPORT_KIND.to_owned(),
        samples_receipt_sha256: checksum(&request.samples.join(RECEIPT_FILE))?,
        counts: receipt.counts,
        reference: request.reference.to_owned(),
        pairs,
        gates,
    };
    std::fs::write(request.output, ks_text::json::dumps(&report, &readable()) + "\n").map_err(|error| format!("{}: {error}", request.output.display()))?;
    Ok(report)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::sequence_report::CorrectionRecord;

    /// A row of a word meant in Cyrillic, typed in the other layout or as meant.
    fn row(other: bool) -> AcceptanceRow {
        AcceptanceRow { sample: if other { Sample::A1 } else { Sample::A2 }, text: "погода".to_owned(), group: 1, class: "rare".to_owned(), other, engine: true }
    }

    /// A case of six letters typed, with these corrections: (key, layout typed, kind).
    fn case(actual: &str, corrections: &[(usize, i64, &str)]) -> Case {
        let expected = "погода ";
        Case {
            identifier: String::new(),
            document: String::new(),
            group: Some(1),
            initially_wrong: true,
            expected: expected.to_owned(),
            trimmed_left: 0,
            trimmed_right: 0,
            focus_letters: Some(6),
            mode: Mode::Default.as_str().to_owned(),
            actual: actual.to_owned(),
            exact: actual == expected,
            length_mismatch: false,
            final_group: 1,
            expected_final_group: 1,
            final_layout_matches: true,
            corrections: corrections
                .iter()
                .map(|(event, target, kind)| CorrectionRecord { event: *event, before: String::new(), after: String::new(), source_group: 1 - target, target_group: *target, actual_group: *target, caret_before: 0, caret_after: 0, kind: Some((*kind).to_owned()) })
                .collect(),
            intervention: None,
            error: None,
        }
    }

    #[test]
    fn a_row_typed_in_the_other_layout_counts_the_key_of_its_first_correction_into_the_layout_meant() {
        let switched = Observed::of(&row(true), &case("погода ", &[(4, 1, "early")]));
        assert_eq!((switched.first_into, switched.right, switched.key(KeyKind::Early), switched.key(KeyKind::Any)), (Some((4, true)), true, 4, 4));
        assert!(switched.switched_by_key() && switched.inside_word() && switched.undone_by.is_none());
        // Early at the last letter: inside the word, not by key 4.
        let last = Observed::of(&row(true), &case("погода ", &[(6, 1, "early")]));
        assert!(last.inside_word() && !last.switched_by_key() && last.key(KeyKind::Early) == 6);
        // At the last letter and by a pause: the key counts only when any kind does.
        let paused = Observed::of(&row(true), &case("погода ", &[(6, 1, "pause")]));
        assert_eq!((paused.key(KeyKind::Early), paused.key(KeyKind::Any)), (7, 6));
        assert!(!paused.switched_by_key() && !paused.inside_word());
        // Started right, ended wrong: by what undid it, or by nothing.
        let undone = Observed::of(&row(true), &case("gjujlf ", &[(4, 1, "early"), (7, 0, "boundary")]));
        assert_eq!((undone.undone_by.as_deref(), undone.key(KeyKind::Early)), (Some("boundary"), 7));
        let lost = Observed::of(&row(true), &case("погодf ", &[(4, 1, "early")]));
        assert_eq!(lost.undone_by.as_deref(), Some("none"));
        // A first correction after the last letter is no early switch that went wrong.
        let late = Observed::of(&row(true), &case("gjujlf ", &[(7, 1, "early")]));
        assert!(late.undone_by.is_none() && late.key(KeyKind::Any) == 7);
        // Right in the end by a correction after the last letter: the key after it.
        let boundary = Observed::of(&row(true), &case("погода ", &[(7, 1, "boundary")]));
        assert!(boundary.right && boundary.key(KeyKind::Any) == 7 && !boundary.inside_word());
        let never = Observed::of(&row(true), &case("gjujlf ", &[]));
        assert!(never.first_into.is_none() && never.key(KeyKind::Any) == 7 && !never.right);
        let unclear = Case { error: Some("ValueError".to_owned()), ..case("погода ", &[]) };
        assert!(Observed::of(&row(true), &unclear).error);
    }

    #[test]
    fn a_row_typed_as_meant_counts_an_early_correction_away_from_its_layout() {
        assert!(Observed::of(&row(false), &case("gjujlf ", &[(4, 0, "early")])).false_early);
        // A correction back into its layout, or one of another kind, is no false early switch.
        assert!(!Observed::of(&row(false), &case("погода ", &[(4, 1, "early")])).false_early);
        assert!(!Observed::of(&row(false), &case("gjujlf ", &[(7, 0, "boundary")])).false_early);
    }

    fn key(text: &str) -> WordKey {
        (text.to_owned(), 1)
    }

    #[test]
    fn rows_are_counted_and_each_word_once() {
        let rows = vec![
            (key("а"), Observed { right: true, exact: true, letters: 6, first_into: Some((4, true)), ..Observed::default() }),
            (key("а"), Observed { right: false, letters: 6, first_into: Some((4, true)), undone_by: Some("boundary".to_owned()), ..Observed::default() }),
            (key("б"), Observed { right: false, letters: 6, error: true, ..Observed::default() }),
        ];
        let early = earliness(&rows);
        assert_eq!((early.rows, early.words, early.early_key_sum, early.any_key_sum), (3, 2, 18, 18));
        assert_eq!((early.switched_by_key, early.first_word_wrong, early.started_right_ended_wrong), (Count { rows: 1, words: 1 }, Count { rows: 2, words: 2 }, Count { rows: 1, words: 1 }));
        assert_eq!((early.undone_by["boundary"], early.exact, early.execution_errors), (1, Count { rows: 1, words: 1 }, 1));
        assert!((early.mean_key(KeyKind::Early) - 6.0).abs() < 1e-12 && (early.mean_key(KeyKind::Any) - 6.0).abs() < 1e-12);
        assert_eq!(earliness(&[]).early_mean_key, 0.0);
        let kept = [(key("а"), Observed { false_early: true, exact: false, ..Observed::default() }), (key("а"), Observed { exact: true, ..Observed::default() }), (key("в"), Observed { error: true, ..Observed::default() })];
        let cell = safety(kept.iter());
        assert_eq!((cell.rows, cell.words, cell.false_early, cell.stays, cell.corrupted, cell.execution_errors), (3, 2, Count { rows: 1, words: 1 }, Count { rows: 1, words: 1 }, Count { rows: 2, words: 2 }, 1));
    }

    #[test]
    fn a_pair_holds_its_gates_only_when_earlier_and_not_less_safe_than_the_reference() {
        let earliness = |key: f64, switched: usize, wrong: usize| Earliness { early_mean_key: key, any_mean_key: key, switched_by_key: Count { rows: switched, words: switched }, inside_word: Count { rows: switched, words: switched }, first_word_wrong: Count { rows: wrong, words: wrong }, exact: Count { rows: 10 - wrong, words: 10 - wrong }, ..Earliness::default() };
        let safe = |false_early: usize| Safety { false_early: Count { rows: false_early, words: false_early }, stays: Count::default(), corrupted: Count { rows: false_early, words: false_early }, ..Safety::default() };
        let report = |key: f64, switched: usize, false_early: usize| PairReport {
            earliness: [("A1".to_owned(), [earliness(key, switched, 1), earliness(key, switched, 1)])].into_iter().collect(),
            safety: SAFETY_CELLS.iter().map(|cell| ((*cell).to_owned(), safe(false_early))).collect(),
            safety_by_sample: [("A2".to_owned(), [safe(false_early), safe(false_early)])].into_iter().collect(),
            ..PairReport::default()
        };
        let reference = report(6.0, 3, 1);
        let earlier = gates(&report(5.0, 4, 1), &reference);
        assert!(earlier.all_held && earlier.execution && earlier.e2["A1"] == [true, true] && earlier.s1_alone["N2"][1].false_early_not_more);
        // As early is not earlier; one more false switch is too many, on a sample alone as in the cells.
        assert!(!gates(&report(6.0, 4, 1), &reference).all_held);
        let riskier = gates(&report(5.0, 4, 2), &reference);
        assert!(!riskier.all_held && !riskier.s1["latin"].false_early_not_more && !riskier.s2_corruption["all"]);
        assert!(!riskier.s1_alone["A2"][0].false_early_not_more && !riskier.s2_corruption_alone["A2"][1]);
        // Earlier by the mean key of any kind of correction only, or by fewer distinct words switched, is no gain.
        let any_only = PairReport { earliness: [("A1".to_owned(), [Earliness { early_mean_key: 6.0, ..earliness(5.0, 4, 1) }, earliness(5.0, 4, 1)])].into_iter().collect(), ..report(5.0, 4, 1) };
        let held = gates(&any_only, &reference);
        assert!(!held.all_held && held.e1["A1"][0].any_mean_key_lower && !held.e1["A1"][0].early_mean_key_lower);
        let fewer_words = PairReport { earliness: [("A1".to_owned(), [Earliness { inside_word: Count { rows: 4, words: 2 }, ..earliness(5.0, 4, 1) }, earliness(5.0, 4, 1)])].into_iter().collect(), ..report(5.0, 4, 1) };
        let held = gates(&fewer_words, &reference);
        assert!(!held.all_held && held.e1["A1"][0].inside_word_not_fewer == CountGate { rows: true, words: false });
        // A row that failed to replay, of either pair, stops the gates.
        let failed = |report: PairReport| PairReport { earliness: [("A1".to_owned(), [Earliness { execution_errors: 1, ..report.earliness["A1"][0].clone() }, report.earliness["A1"][1].clone()])].into_iter().collect(), ..report };
        assert!(!gates(&failed(report(5.0, 4, 1)), &reference).execution);
        assert!(!gates(&report(5.0, 4, 1), &failed(reference.clone())).all_held);
    }

    #[test]
    fn a_word_of_plain_text_is_letters_of_one_layout() {
        assert_eq!(plain_word("«Мама,"), Some(("мама".to_owned(), 1)));
        assert_eq!(plain_word("Hello!"), Some(("hello".to_owned(), 0)));
        for token in ["fox", "abc123", "кот-кит", "ёлка", "§§§", "Москваriver"] {
            assert_eq!(plain_word(token).is_some(), token == "ёлка", "{token}");
        }
        assert!(letters_both_ways("мама", 1) && !letters_both_ways("жук", 1) && letters_both_ways("quick", 0));
        assert_eq!(row(true).direction(), 0);
        assert_eq!(row(false).direction(), 1);
        assert_eq!((base_splits(false), base_splits(true)), (&[DEVELOPMENT][..], &[DEVELOPMENT, CALIBRATION][..]));
    }
}
