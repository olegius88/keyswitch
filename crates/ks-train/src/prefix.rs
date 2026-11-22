//! The prefix candidate (`train_prefix_v2_model`, `prefix_v2_corpus`): a bounded counterfactual curriculum from already
//! exposed old splits. Application and role variants receive both intended-layout states; the candidate is chosen on
//! whole sequences, with no false conversion in either lexical profile.
use ks_cast::To as _;
use crate::corpus::{CALIBRATION, DEVELOPMENT, FITTING_SPLITS, TRAIN, checksum, read_json};
use crate::fit::{provenance, readable};
use crate::kernel::{self, Packed};
use crate::physical::{hex, pair};
use crate::prefix_cache::FramesCache;
use crate::tools::{Lexicons, Options, reference_models};
use crate::{ACTIONS, DETERMINISTIC_ROUNDING_DECIMALS, FEATURE_MASS_TOLERANCE, LOG_LOSS_PROBABILITY_FLOOR, PROBABILITY_SUM_TOLERANCE};
use flate2::Compression;
use flate2::read::MultiGzDecoder;
use flate2::write::GzEncoder;
use ks_backend::{FieldContext, FieldRole};
use ks_context::prefix_model::{PrefixInput, PrefixModel, PrefixSide};
use ks_context::constants::TERMS_PREFIX_FEATURE_VERSION;
use ks_context::prefix_schema::{PrefixReaders, features_with_readers};
use ks_context::prefix_terms::PrefixTerms;
use ks_lang::ortho_model::{OrthoModel, artifact_path};
use ks_context::flat_table::WeightTable;
use ks_lang::early_switch::{PrefixIndex, dictionary_stems};
use ks_lang::{Resources, LanguageModel};
use ks_text::{JsonOptions, fsum, json_dumps, py_round_ndigits, pystr};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{BufRead, Write as _};
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
/// The application of every row of a word corpus: no name of an application carries a label.
const WORD_CORPUS_APPLICATION: &str = "Editor";

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
    /// Typed in the other layout as well as as meant; otherwise only as meant.
    pub paired: bool,
    /// In its own situation only; otherwise in the nine situations of the prefix-v1 curriculum.
    pub own_context: bool,
    /// Typed as meant, its first keys are the ones a reading of keys most easily takes for the other layout (the word
    /// corpus's `hard`); a calibration cell counts these apart.
    pub hard: bool,
}

/// How a recipe labels the frames and which of them it measures. Without the recipe's keys, the rules every sealed recipe
/// was fitted under.
#[derive(Clone, Debug)]
pub struct FrameRules {
    /// A prefix typed in the other layout converts from this many letters and waits before it.
    pub convert_from: usize,
    /// Known terms, case-folded and sorted: a prefix typed in the other layout that begins one waits.
    pub terms: Option<Arc<Vec<String>>>,
    /// Measure the prefixes the engine asks the model about (its other reading all letters, its last key a letter);
    /// otherwise those whose both readings are letters.
    pub engine_support: bool,
    /// The fewest letters a measured prefix has.
    pub minimum_characters: usize,
    /// The thousandths of the training families whose frames read no term list (`closed_family`), so that a token
    /// missing from the lists is not taken for a word of the other layout.
    pub closed_term_families_per_mille: Option<u64>,
}

impl Default for FrameRules {
    fn default() -> Self {
        FrameRules { convert_from: PREFIX_V2_SHORT_PREFIX_CHARACTERS, terms: None, engine_support: false, minimum_characters: PREFIX_MIN_CHARACTERS, closed_term_families_per_mille: None }
    }
}

/// The label of a prefix: kept when typed as meant; typed in the other layout, it waits while it is short, while it
/// continues a word of the typed layout's lexicon or begins a known term, and converts after.
#[must_use]
pub fn label_of(desired: bool, length: usize, ambiguous: bool, term: bool, rules: &FrameRules) -> u8 {
    if !desired {
        0
    } else if length < rules.convert_from || ambiguous || term {
        PREFIX_V2_AMBIGUOUS_POSITIVE_LABEL
    } else {
        1
    }
}

/// Whether the model is asked about a prefix, and so whether a conversion there counts.
#[must_use]
pub fn supported_of(original: &str, alternate: &str, rules: &FrameRules) -> bool {
    let length = pystr::len(original);
    let interior_capital = |text: &str| text.chars().skip(1).any(|character| pystr::isupper(character.encode_utf8(&mut [0_u8; 4])));
    let letters = if rules.engine_support {
        original.chars().last().is_some_and(|character| pystr::isalpha(character.encode_utf8(&mut [0_u8; 4])))
    } else {
        pystr::isalpha(original)
    };
    (rules.minimum_characters..=PREFIX_MAX_CHARACTERS).contains(&length) && letters && pystr::isalpha(alternate) && !interior_capital(original) && !interior_capital(alternate)
}

/// Whether a prefix begins one of the sorted terms.
#[must_use]
pub fn begins_a_term(terms: &[String], prefix: &str) -> bool {
    let key = pystr::casefold(prefix);
    let index = terms.partition_point(|term| term.as_str() < key.as_str());
    terms.get(index).is_some_and(|term| term.starts_with(&key))
}

/// The terms the engine knows: the identifier lexicon and the protected tokens, case-folded and sorted.
pub fn known_terms(resource_dir: &Path) -> Result<Vec<String>, String> {
    #[derive(serde::Deserialize)]
    struct Identifiers {
        identifiers: Vec<String>,
    }
    let identifiers: Identifiers = ks_text::json::loads(std::fs::read(resource_dir.join("identifiers.json")).map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
    let protected = std::fs::read_to_string(resource_dir.join("protected_tokens.txt")).map_err(|error| error.to_string())?;
    let mut terms: Vec<String> = identifiers.identifiers.iter().map(String::as_str).chain(protected.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#'))).map(pystr::casefold).collect();
    terms.sort();
    terms.dedup();
    Ok(terms)
}

/// Which prefixes a fit measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Support {
    /// Both readings letters: every sealed recipe.
    Legacy,
    /// What the engine asks the model about.
    Engine,
}

/// How a fit chooses its epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EpochSelection {
    /// The best zero-false frontier on development: every sealed recipe.
    Frontier,
    /// The least development loss among the epochs with a zero-false frontier.
    DevelopmentLoss,
    /// On development alone: each direction at its first clean threshold of the grid, the epoch whose mean switch key is
    /// lowest among those not worse than the floor model in either direction (`KeyMeasure`, `chosen_epoch`); then each
    /// direction's threshold on calibration, read once, not below its development threshold (`calibrated_thresholds`).
    MeanKey,
}

/// How the importance of a split's frames is scaled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportanceNormalization {
    /// To a mean of one over the split.
    MeanOne,
}

/// The keys of a recipe that a sealed prefix-v1 recipe does not have; each one absent is the rule those were fitted under.
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct RecipeOptions {
    /// A word corpus, relative to the tree.
    pub corpus: Option<String>,
    /// Without it the importance of a frame is its family's share, as every sealed recipe has it.
    pub importance_normalization: Option<ImportanceNormalization>,
    pub convert_from_characters: Option<usize>,
    pub minimum_prefix_characters: Option<usize>,
    #[serde(default)]
    pub wait_on_known_terms: bool,
    pub support: Option<Support>,
    pub early_stop_patience: Option<usize>,
    pub epoch_selection: Option<EpochSelection>,
    /// The fewest words typed as meant a direction's calibration cell holds, and of them hard ones: a fit on fewer is
    /// refused.
    pub minimum_words_per_direction: Option<usize>,
    pub minimum_hard_words_per_direction: Option<usize>,
    /// Schema four: the thousandths of the training families whose frames read no term list.
    pub closed_term_families_per_mille: Option<u64>,
    /// The model an epoch chosen by the mean key must not be worse than on development, relative to the tree, its
    /// version and the threshold it is measured at.
    pub floor_model: Option<String>,
    pub floor_model_version: Option<String>,
    pub floor_threshold: Option<f64>,
}

impl RecipeOptions {
    /// The options of a recipe file.
    pub fn of(recipe_path: &Path) -> Result<RecipeOptions, String> {
        let bytes = std::fs::read(recipe_path).map_err(|error| format!("{}: {error}", recipe_path.display()))?;
        ks_text::json::loads(bytes).map_err(|error| format!("{}: {error}", recipe_path.display()))
    }

    /// Whether the fit writes what a word corpus's fit adds beside its seal.
    #[must_use]
    pub fn reports_classes(&self) -> bool {
        self.corpus.is_some() || self.early_stop_patience.is_some()
    }
}

/// The rules a recipe names.
pub fn frame_rules(options: &RecipeOptions, resource_dir: &Path) -> Result<FrameRules, String> {
    let mut rules = FrameRules::default();
    rules.convert_from = options.convert_from_characters.unwrap_or(rules.convert_from);
    rules.minimum_characters = options.minimum_prefix_characters.unwrap_or(rules.minimum_characters);
    if options.wait_on_known_terms {
        rules.terms = Some(Arc::new(known_terms(resource_dir)?));
    }
    rules.engine_support = options.support == Some(Support::Engine);
    rules.closed_term_families_per_mille = options.closed_term_families_per_mille;
    Ok(rules)
}

/// The namespace of the draw that closes the term lists to a family's frames.
const CLOSED_TERMS_NAMESPACE: &str = "keyswitch:prefix-v4:closed-terms";

/// Whether a training family's frames are made without the term lists: these thousandths of the families, by hash.
#[must_use]
pub fn closed_family(family: &str, per_mille: u64) -> bool {
    crate::prefix_words::draw(&format!("{CLOSED_TERMS_NAMESPACE}:{family}")) % 1000 < per_mille
}

/// The term lists a schema-four model reads, as the engine builds them: the identifier lexicon, the protected tokens and
/// the term table, without the tokens whose keys read as a word of the other layout's lexicon.
pub fn prefix_terms_of(resource_dir: &Path, models: &Lexicons) -> Result<PrefixTerms, String> {
    let identifiers = ks_lang::identifier_lexicon::IdentifierLexicon::load(&resource_dir.join("identifiers.json")).map_err(|error| format!("the identifier lexicon: {}", error.name))?;
    let protected = ks_lang::detector::ProtectedTokens::load(resource_dir);
    let tables = ks_context::context_model::TermTables::load(&resource_dir.join("models")).map_err(|error| format!("the term table: {}", error.name))?;
    let known = |word: &str, group: i64| models[usize::from(group == 1)].frequencies.contains(word);
    Ok(PrefixTerms::build(identifiers.identifiers.iter(), protected.iter(), tables.term("latin"), tables.term("cyrillic"), known))
}

/// The parents of a word corpus's split: each row a word in its own situation, typed in the other layout too when paired.
pub fn load_word_parents(directory: &Path, split: &str) -> Result<Vec<ParentWord>, String> {
    if !FITTING_SPLITS.contains(&split) {
        return Err("prefix-v2 may read only exposed train/development/calibration splits".to_owned());
    }
    crate::prefix_words::read_split(directory, split)?
        .into_iter()
        .enumerate()
        .map(|(index, row)| {
            Ok(ParentWord {
                original: row.text,
                group: row.group,
                family: row.family,
                before: row.before,
                identifier: format!("prefix-words:{split}:{index}"),
                category: row.class.as_str().to_owned(),
                application: WORD_CORPUS_APPLICATION.to_owned(),
                role: role_named(&row.role)?,
                paired: row.paired,
                own_context: true,
                hard: row.hard || row.class.hard(row.group),
            })
        })
        .collect()
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
            paired: category == "correct",
            own_context: SAFETY_CATEGORIES.contains(&category),
            hard: false,
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
    families.truncate(max_families.to::<usize>());
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
            if selected.len() == max_words_per_family.to::<usize>() {
                break;
            }
        }
    }
    Ok(result)
}

/// `contexts_for`, sorted and without repeats as `contexts` leaves them.
fn contexts_of(parent: &ParentWord) -> Vec<PrefixContext> {
    let values: Vec<PrefixContext> = if parent.own_context {
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
            let stems = dictionary_stems(Path::new(model.speller.source()), PREFIX_INDEX_DICTIONARY_MAX_BYTES);
            PrefixIndex::new(model.frequencies.words().iter().chain(stems.iter()), model.frequencies.clone())
        })
        .collect();
    Indexes { portable: plain, hunspell: stemmed }
}

/// A frame of the curriculum, with what the epochs and the metrics read.
pub struct PrefixFrame {
    pub label: u8,
    /// The parent's class and group, an index into `SplitFrames::classes`.
    pub class: u16,
    pub parent_family: u32,
    pub sequence: u32,
    pub length: u32,
    pub full_length: u32,
    pub desired: bool,
    pub profile: u8,
    pub sample_weight: f64,
    pub supported: bool,
    pub pair: u32,
    /// The layout group the keys were typed in: the direction of a switch.
    pub source: u8,
    pub hard: bool,
    pub features: Vec<(u32, f64)>,
}

/// The frames of one split and the names their features use.
pub struct SplitFrames {
    pub frames: Vec<PrefixFrame>,
    pub names: Vec<String>,
    pub families: Vec<String>,
    pub pairs: usize,
    /// `<category>:<group>` of the parents, by the frames' `class`.
    pub classes: Vec<String>,
    /// The whole text each sequence types, by its number.
    pub typed: Vec<String>,
}

struct ParentFrames {
    frames: Vec<PrefixFrame>,
    names: Vec<String>,
    pairs: Vec<String>,
    sequences: Vec<String>,
    typed: Vec<String>,
}

/// The frames of one parent. Sequences and pairs are numbered inside the parent and renumbered by the caller.
fn parent_frames(parent: &ParentWord, family_count: u64, family: u32, class: u16, models: [&Lexicons; 2], indexes: &Indexes, maximum_prefix_length: usize, feature_version: i64, readers: PrefixReaders<'_>, rules: &FrameRules) -> ParentFrames {
    let variants = contexts_of(parent);
    let states: &[bool] = if parent.paired { &[false, true] } else { &[false] };
    let parent_length = pystr::len(&parent.original);
    let length_count = parent_length.min(maximum_prefix_length);
    let weight = 1.0 / (family_count.to::<f64>() * variants.len().to::<f64>() * PROFILES.len().to::<f64>() * states.len().to::<f64>() * length_count.to::<f64>());
    let mut names: HashMap<String, u32> = HashMap::new();
    let mut out = ParentFrames { frames: Vec::new(), names: Vec::new(), pairs: Vec::new(), sequences: Vec::new(), typed: Vec::new() };
    let (from, to) = if parent.group != 0 { ("ru", "us") } else { ("us", "ru") };
    for context in &variants {
        let context_key = json!([context.before, context.application, context.role.as_str()]);
        for (profile_number, profile) in PROFILES.iter().enumerate() {
            let pair_id = format!("prefix-v2-pair:{}", digest(&json!([parent.identifier, context_key, profile])));
            let pair_number = out.pairs.len().to::<u32>();
            out.pairs.push(pair_id.clone());
            let table = if profile_number == 0 { &indexes.portable } else { &indexes.hunspell };
            let model_of = |group: usize| -> &LanguageModel { &models[profile_number][group] };
            let sides = [PrefixSide { index: &table[0], model: model_of(0) }, PrefixSide { index: &table[1], model: model_of(1) }];
            for desired in states {
                let source = if *desired { 1 - parent.group } else { parent.group };
                let full = if *desired { pair().translate(&parent.original, from, to).expect("the pair of layouts") } else { parent.original.clone() };
                assert_eq!(pystr::len(&full), parent_length, "prefix translation changed the parent length");
                let sequence_id = format!("prefix-v2-sequence:{}", digest(&json!([pair_id, desired])));
                let sequence_number = out.sequences.len().to::<u32>();
                out.sequences.push(sequence_id);
                out.typed.push(full.clone());
                for length in 1..=length_count {
                    let original = pystr::slice(&full, None, Some(length.to::<i64>())).to_owned();
                    let (source_layout, target_layout) = if source != 0 { ("ru", "us") } else { ("us", "ru") };
                    let alternate = pair().translate(&original, source_layout, target_layout).expect("the pair of layouts");
                    // A prefix is that many keys of one side of the pair, read in both layouts key for key: the runtime's
                    // checks of length and side hold for every frame here.
                    assert_eq!((pystr::len(&original), pystr::len(&alternate)), (length, length), "a prefix of that many letters");
                    assert!((0..=1).contains(&source), "a side of the pair");
                    let ambiguous = indexes.portable[source.to::<usize>()].completions(&original).completions > 0;
                    let term = *desired && rules.terms.as_ref().is_some_and(|terms| begins_a_term(terms, &original));
                    let label = label_of(*desired, length, ambiguous, term, rules);
                    let mut field = FieldContext::new(context.application.as_str(), pair_id.as_str());
                    field.before.clone_from(&context.before);
                    field.role = context.role;
                    let item = PrefixInput { original: original.clone(), alternative: alternate.clone(), source_group: source, field };
                    let values = features_with_readers(&item, [&sides[0], &sides[1]], feature_version, readers).expect("the schema of the recipe");
                    let features = values
                        .into_iter()
                        .map(|(name, value)| {
                            let next = out.names.len().to::<u32>();
                            let id = *names.entry(name.clone()).or_insert_with(|| {
                                out.names.push(name);
                                next
                            });
                            (id, value)
                        })
                        .collect();
                    // The prefix decision requires four to twelve letters and no interior capitals.
                    let supported = supported_of(&original, &alternate, rules);
                    out.frames.push(PrefixFrame {
                        label,
                        class,
                        parent_family: family,
                        sequence: sequence_number,
                        length: length.to::<u32>(),
                        full_length: parent_length.to::<u32>(),
                        desired: *desired,
                        profile: profile_number.to::<u8>(),
                        sample_weight: weight,
                        supported,
                        pair: pair_number,
                        source: source.to::<u8>(),
                        hard: parent.hard,
                        features,
                    });
                }
            }
        }
    }
    out
}

/// `generate_frames`: paired snapshots with unit total sampling mass per physical family, by `jobs` threads. The families
/// `closed` names read no term list.
fn generate_frames(parents: &[ParentWord], models: [&Lexicons; 2], indexes: &Indexes, maximum_prefix_length: usize, feature_version: i64, readers: PrefixReaders<'_>, closed: Option<u64>, rules: &FrameRules, jobs: usize) -> SplitFrames {
    let no_terms = PrefixTerms::default();
    let mut family_counts: BTreeMap<&str, u64> = BTreeMap::new();
    for parent in parents {
        *family_counts.entry(&parent.family).or_insert(0) += 1;
    }
    let families: Vec<String> = family_counts.keys().map(|name| (*name).to_owned()).collect();
    let family_number: HashMap<&str, u32> = families.iter().enumerate().map(|(index, name)| (name.as_str(), index.to::<u32>())).collect();
    let class_name = |parent: &ParentWord| format!("{}:{}", parent.category, parent.group);
    let classes: Vec<String> = parents.iter().map(class_name).collect::<BTreeSet<String>>().into_iter().collect();
    let class_number: HashMap<&str, u16> = classes.iter().enumerate().map(|(index, name)| (name.as_str(), index.to::<u16>())).collect();
    let next = AtomicUsize::new(0);
    let done: std::sync::Mutex<Vec<Option<ParentFrames>>> = std::sync::Mutex::new((0..parents.len()).map(|_| None).collect());
    std::thread::scope(|scope| {
        for _ in 0..jobs.max(1) {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(parent) = parents.get(index) else { break };
                    let class = class_number[class_name(parent).as_str()];
                    let shut = closed.is_some_and(|per_mille| closed_family(&parent.family, per_mille));
                    let readers = if shut { PrefixReaders { terms: readers.terms.map(|_| &no_terms), ..readers } } else { readers };
                    let built = parent_frames(parent, family_counts[parent.family.as_str()], family_number[parent.family.as_str()], class, models, indexes, maximum_prefix_length, feature_version, readers, rules);
                    done.lock().expect("a worker finished")[index] = Some(built);
                }
            });
        }
    });
    let mut names: Vec<String> = Vec::new();
    let mut numbers: HashMap<String, u32> = HashMap::new();
    let mut frames = Vec::new();
    let (mut sequences, mut pairs) = (0_u32, 0_u32);
    let mut typed = Vec::new();
    for part in done.into_inner().expect("the workers finished").into_iter().flatten() {
        typed.extend(part.typed);
        let mapping: Vec<u32> = part
            .names
            .iter()
            .map(|name| {
                *numbers.entry(name.clone()).or_insert_with(|| {
                    names.push(name.clone());
                    (names.len() - 1).to::<u32>()
                })
            })
            .collect();
        for mut frame in part.frames {
            frame.sequence += sequences;
            frame.pair += pairs;
            for feature in &mut frame.features {
                feature.0 = mapping[feature.0.to::<usize>()];
            }
            frames.push(frame);
        }
        sequences += part.sequences.len().to::<u32>();
        pairs += part.pairs.len().to::<u32>();
    }
    SplitFrames { frames, names, families, pairs: pairs.to::<usize>(), classes, typed }
}

/// `feature_vocabulary`: Kahan feature mass; family weight precedes class multipliers.
fn feature_vocabulary(split: &SplitFrames, minimum: f64, maximum: usize) -> Vec<String> {
    let mut totals = vec![0.0_f64; split.names.len()];
    let mut errors = vec![0.0_f64; split.names.len()];
    for frame in &split.frames {
        for (id, value) in &frame.features {
            assert!(value.is_finite(), "invalid feature value");
            let id = (*id).to::<usize>();
            let previous = totals[id];
            let corrected = frame.sample_weight - errors[id];
            let total = previous + corrected;
            errors[id] = (total - previous) - corrected;
            totals[id] = total;
        }
    }
    // Quantised ordering and a 1e-9 cutoff tolerate divided-parent roundoff. Every name is of a feature some frame holds:
    // `generate_frames` names only those.
    let mut ranked: Vec<(f64, &String)> = (0..split.names.len())
        .filter(|id| totals[*id] + FEATURE_MASS_TOLERANCE >= minimum)
        .map(|id| (py_round_ndigits(totals[id], DETERMINISTIC_ROUNDING_DECIMALS).expect("a finite mass"), &split.names[id]))
        .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    ranked.truncate(maximum);
    let mut selected: Vec<String> = ranked.into_iter().map(|entry| entry.1.clone()).collect();
    selected.sort();
    selected
}

fn packed_rows(split: &SplitFrames, names: &[String], cfg: &Value, mean_one: bool) -> Packed {
    let vocabulary: HashMap<&str, u32> = names.iter().enumerate().map(|(index, name)| (name.as_str(), index.to::<u32>())).collect();
    let column: Vec<Option<u32>> = split.names.iter().map(|name| vocabulary.get(name.as_str()).copied()).collect();
    let (keep, wait) = (cfg.float("keep_importance"), cfg.float("wait_importance"));
    let mut packed = Packed { offsets: vec![0], ..Packed::default() };
    for frame in &split.frames {
        for (id, value) in &frame.features {
            if let Some(target) = column[(*id).to::<usize>()] {
                packed.indices.push(target);
                packed.values.push(*value);
            }
        }
        packed.offsets.push(packed.indices.len().to::<u64>());
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
    // A family's weight is a share of one unit, about a thousandth of a row: AdaGrad's accumulators start at 1 and such
    // gradients never move them, so the optimiser is plain descent with a tiny step. A recipe may scale the split's
    // importance to a mean of one; without the key the importance is the family share, as every sealed recipe has it.
    // A split without frames has no mean and nothing to scale.
    if mean_one {
        let mean = fsum(&packed.importance).expect("finite importance") / packed.importance.len().to::<f64>();
        for value in &mut packed.importance {
            *value /= mean;
        }
    }
    packed
}

#[derive(Clone, Default)]
struct Sequence {
    profile: u8,
    desired: bool,
    full_length: u32,
    class: u16,
    /// The layout group of the keys: the threshold of this direction decides its conversions.
    source: u8,
    hard: bool,
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
    let count = split.frames.iter().map(|frame| frame.sequence.to::<usize>() + 1).max().unwrap_or(0);
    let mut sequences = vec![Sequence::default(); count];
    for (index, frame) in split.frames.iter().enumerate() {
        let scores = &probabilities[index * ACTIONS..(index + 1) * ACTIONS];
        assert!(scores.iter().all(|value| value.is_finite()), "invalid prefix probabilities");
        assert!(scores.iter().all(|value| (0.0..=1.0).contains(value)), "invalid prefix probabilities");
        assert!((ks_lang::py_sum(scores.iter().copied()) - 1.0).abs() <= PROBABILITY_SUM_TOLERANCE, "invalid prefix probabilities");
        let value = &mut sequences[frame.sequence.to::<usize>()];
        value.profile = frame.profile;
        value.desired = frame.desired;
        value.full_length = frame.full_length;
        value.class = frame.class;
        value.source = frame.source;
        value.hard = frame.hard;
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
        let counts = &mut results[value.profile.to::<usize>()];
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

/// A class of parents at its thresholds, both profiles together.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ClassMetrics {
    pub sequences: i64,
    pub desired: i64,
    pub converted: i64,
    pub converted_before_end: i64,
    #[serde(rename = "false")]
    pub false_conversions: i64,
    /// Sequences typed in the other layout by the length of their first conversion, `0` for none.
    pub first_conversion_length: BTreeMap<u32, i64>,
}

/// The key a measure of earliness counts a switch by: the first key the engine asks the prefix model about.
pub const SWITCH_KEY: u32 = 4;

/// One direction of typing (the layout group of the keys) at one threshold, in one profile or both together.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DirectionMetrics {
    /// Sequences typed in the other layout.
    pub desired: i64,
    pub converted: i64,
    pub converted_before_end: i64,
    /// Of those typed in the other layout, the ones whose first switch came by `SWITCH_KEY`.
    pub switched_by_key: i64,
    /// The keys of the first switches added up; a word never switched counts its boundary, one key after its last letter.
    pub key_sum: i64,
    /// Sequences typed as meant, and of those the ones converted.
    pub kept: i64,
    #[serde(rename = "false")]
    pub false_conversions: i64,
}

impl DirectionMetrics {
    fn add(&mut self, value: &Sequence, first: Option<u32>) {
        if value.desired {
            self.desired += 1;
            self.converted += i64::from(first.is_some());
            self.converted_before_end += i64::from(first.is_some_and(|length| length < value.full_length));
            self.switched_by_key += i64::from(first.is_some_and(|length| length <= SWITCH_KEY));
            self.key_sum += i64::from(first.unwrap_or(value.full_length + 1));
        } else {
            self.kept += 1;
            self.false_conversions += i64::from(first.is_some());
        }
    }

    fn merged(self, other: DirectionMetrics) -> DirectionMetrics {
        DirectionMetrics {
            desired: self.desired + other.desired,
            converted: self.converted + other.converted,
            converted_before_end: self.converted_before_end + other.converted_before_end,
            switched_by_key: self.switched_by_key + other.switched_by_key,
            key_sum: self.key_sum + other.key_sum,
            kept: self.kept + other.kept,
            false_conversions: self.false_conversions + other.false_conversions,
        }
    }

    /// The share of the words typed in the other layout switched by `SWITCH_KEY`; none of an empty cell.
    #[must_use]
    pub fn share_by_key(&self) -> f64 {
        if self.desired == 0 { 0.0 } else { self.switched_by_key.to::<f64>() / self.desired.to::<f64>() }
    }

    /// The mean key of the first switch of the words typed in the other layout.
    #[must_use]
    pub fn mean_key(&self) -> f64 {
        if self.desired == 0 { 0.0 } else { self.key_sum.to::<f64>() / self.desired.to::<f64>() }
    }

    fn early_recall(&self) -> f64 {
        if self.desired == 0 { 0.0 } else { self.converted_before_end.to::<f64>() / self.desired.to::<f64>() }
    }
}

/// The first conversion of a sequence at the threshold of its direction.
fn first_conversion(value: &Sequence, thresholds: [f64; 2]) -> Option<u32> {
    let threshold = thresholds[usize::from(value.source.min(1))];
    value.candidates.iter().filter(|candidate| candidate.1 >= threshold).map(|candidate| candidate.0).min()
}

/// Each profile's and direction's metrics at the thresholds of the two directions: `[profile][direction]`.
fn direction_metrics(scores: &[Sequence], thresholds: [f64; 2]) -> [[DirectionMetrics; 2]; 2] {
    let mut result = [[DirectionMetrics::default(); 2]; 2];
    for value in scores {
        result[usize::from(value.profile.min(1))][usize::from(value.source.min(1))].add(value, first_conversion(value, thresholds));
    }
    result
}

/// Each direction, both profiles together.
fn pooled(metrics: &[[DirectionMetrics; 2]; 2]) -> [DirectionMetrics; 2] {
    [metrics[0][0].merged(metrics[1][0]), metrics[0][1].merged(metrics[1][1])]
}

/// The distinct texts of the kept sequences the thresholds convert, each after its class: what holds a threshold up.
fn false_texts(scores: &[Sequence], thresholds: [f64; 2], classes: &[String], typed: &[Option<String>]) -> Vec<String> {
    let found: BTreeSet<String> = scores
        .iter()
        .zip(typed)
        .filter(|(value, _)| !value.desired && first_conversion(value, thresholds).is_some())
        .filter_map(|(value, text)| text.as_ref().map(|text| format!("{}: {text}", classes[usize::from(value.class)])))
        .collect();
    found.into_iter().collect()
}

/// The distinct texts typed as meant that the thresholds convert, in either profile, by the layout group of the keys.
fn false_words(scores: &[Sequence], thresholds: [f64; 2], typed: &[String]) -> [usize; 2] {
    let mut found: [BTreeSet<&str>; 2] = [BTreeSet::new(), BTreeSet::new()];
    for (value, text) in scores.iter().zip(typed) {
        if !value.desired && first_conversion(value, thresholds).is_some() {
            found[usize::from(value.source.min(1))].insert(text.as_str());
        }
    }
    found.map(|texts| texts.len())
}

/// The metrics of each class of parents, both profiles together, and the length of the first conversion of each
/// sequence typed in the other layout.
fn class_metrics(scores: &[Sequence], thresholds: [f64; 2], classes: &[String]) -> BTreeMap<String, ClassMetrics> {
    let mut by_class: BTreeMap<String, ClassMetrics> = BTreeMap::new();
    for value in scores {
        let counts = by_class.entry(classes[usize::from(value.class)].clone()).or_default();
        let first = first_conversion(value, thresholds);
        counts.sequences += 1;
        counts.desired += i64::from(value.desired);
        counts.converted += i64::from(value.desired && first.is_some());
        counts.converted_before_end += i64::from(value.desired && first.is_some_and(|length| length < value.full_length));
        counts.false_conversions += i64::from(!value.desired && first.is_some());
        if value.desired {
            *counts.first_conversion_length.entry(first.unwrap_or(0)).or_insert(0) += 1;
        }
    }
    by_class
}

/// The file beside the seal that keeps a fit's development and calibration scores.
pub const SCORES_FILE: &str = "scores.json.gz";

/// A sequence as a fit keeps it beside its seal: what every report of the fit is made from, without a refit.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SavedSequence {
    pub profile: u8,
    pub desired: bool,
    pub source: u8,
    pub hard: bool,
    pub full_length: u32,
    pub class: u16,
    /// The supported prefixes the model would convert, by length, with the probability of converting.
    pub candidates: Vec<(u32, f64)>,
    /// The text typed: development only; calibration keeps numbers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// One split's sequences and the names of their classes.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SavedSplit {
    pub classes: Vec<String>,
    pub sequences: Vec<SavedSequence>,
}

impl SavedSplit {
    fn of(scores: &[Sequence], frames: &SplitFrames, texts: bool) -> SavedSplit {
        let sequences = scores
            .iter()
            .zip(&frames.typed)
            .map(|(value, typed)| SavedSequence {
                profile: value.profile,
                desired: value.desired,
                source: value.source,
                hard: value.hard,
                full_length: value.full_length,
                class: value.class,
                candidates: value.candidates.clone(),
                text: texts.then(|| typed.clone()),
            })
            .collect();
        SavedSplit { classes: frames.classes.clone(), sequences }
    }

    fn scores(&self) -> Vec<Sequence> {
        self.sequences
            .iter()
            .map(|saved| Sequence { profile: saved.profile, desired: saved.desired, full_length: saved.full_length, class: saved.class, source: saved.source, hard: saved.hard, candidates: saved.candidates.clone() })
            .collect()
    }

    fn texts(&self) -> Vec<Option<String>> {
        self.sequences.iter().map(|saved| saved.text.clone()).collect()
    }
}

/// `scores.json.gz` beside the seal of a fit that reports its classes: each sequence of development and calibration
/// with the chosen epoch's probabilities. `prefix-report` makes the class report from it again.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SavedScores {
    pub model_version: String,
    pub epoch: i64,
    pub stopped_after_epoch: Option<i64>,
    /// The thresholds of the two directions, by the layout group of the keys.
    pub thresholds: [f64; 2],
    pub grid: Vec<f64>,
    pub development: SavedSplit,
    pub calibration: SavedSplit,
}

/// Both directions at one threshold of the grid, both profiles together.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ThresholdRow {
    pub development: [DirectionMetrics; 2],
    pub calibration: [DirectionMetrics; 2],
}

/// What a fit that reports its classes writes beside its seal (`classes.json`): where it stopped; each class on
/// development and calibration at the chosen thresholds and on calibration at every threshold of the grid; each
/// direction at every threshold; and the development texts typed as meant that each threshold converts. Calibration is
/// reported in numbers only: a threshold is chosen on it.
#[derive(serde::Serialize)]
pub struct ClassReport {
    model_version: String,
    thresholds: [f64; 2],
    epoch: i64,
    stopped_after_epoch: Option<i64>,
    development: BTreeMap<String, ClassMetrics>,
    calibration: BTreeMap<String, ClassMetrics>,
    calibration_by_threshold: BTreeMap<String, BTreeMap<String, ClassMetrics>>,
    by_threshold: BTreeMap<String, ThresholdRow>,
    development_false: BTreeMap<String, Vec<String>>,
}

impl ClassReport {
    /// The report of saved scores.
    #[must_use]
    pub fn of(saved: &SavedScores) -> ClassReport {
        let (development, calibration) = (saved.development.scores(), saved.calibration.scores());
        let texts = saved.development.texts();
        let both = |threshold: f64| [threshold, threshold];
        ClassReport {
            model_version: saved.model_version.clone(),
            thresholds: saved.thresholds,
            epoch: saved.epoch,
            stopped_after_epoch: saved.stopped_after_epoch,
            development: class_metrics(&development, saved.thresholds, &saved.development.classes),
            calibration: class_metrics(&calibration, saved.thresholds, &saved.calibration.classes),
            calibration_by_threshold: saved.grid.iter().map(|threshold| (threshold.to_string(), class_metrics(&calibration, both(*threshold), &saved.calibration.classes))).collect(),
            by_threshold: saved
                .grid
                .iter()
                .map(|threshold| (threshold.to_string(), ThresholdRow { development: pooled(&direction_metrics(&development, both(*threshold))), calibration: pooled(&direction_metrics(&calibration, both(*threshold))) }))
                .collect(),
            development_false: saved.grid.iter().map(|threshold| (threshold.to_string(), false_texts(&development, both(*threshold), &saved.development.classes, &texts))).collect(),
        }
    }

    /// The report as the fit writes it.
    #[must_use]
    pub fn bytes(&self) -> Vec<u8> {
        let options = JsonOptions { ensure_ascii: false, indent: Some(ks_text::Indent::Spaces(1)), sort_keys: true, ..JsonOptions::default() };
        (ks_text::json::dumps(self, &options) + "\n").into_bytes()
    }
}

/// The saved scores of a fit folder.
pub fn read_saved_scores(folder: &Path) -> Result<SavedScores, String> {
    let path = folder.join(SCORES_FILE);
    let file = std::fs::File::open(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let text = std::io::read_to_string(MultiGzDecoder::new(file)).map_err(|error| format!("{}: {error}", path.display()))?;
    ks_text::json::loads(text).map_err(|error| format!("{}: {error}", path.display()))
}

fn saved_bytes(saved: &SavedScores) -> Result<Vec<u8>, String> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(&crate::artifact::canonical_of(saved)).map_err(|error| error.to_string())?;
    encoder.finish().map_err(|error| error.to_string())
}

/// `prefix-report --fit <folder> --output <file>`: the class report of a fit, made again from its saved scores.
pub fn report_command(fit_folder: &Path, output: &Path) -> Result<(), String> {
    let saved = read_saved_scores(fit_folder)?;
    std::fs::write(output, ClassReport::of(&saved).bytes()).map_err(|error| format!("{}: {error}", output.display()))
}

/// Whether a fit with this patience stops after this epoch: neither the chosen frontier nor the development loss has
/// improved for `patience` epochs.
#[must_use]
pub fn patience_spent(epoch: usize, last_selection: usize, last_loss: usize, patience: Option<usize>) -> bool {
    patience.is_some_and(|patience| epoch.saturating_sub(last_selection.max(last_loss)) >= patience)
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

/// Whether an epoch replaces the chosen one: by the frontier (`better`), or, when the recipe selects by development loss,
/// by a lower loss among the epochs that have a zero-false frontier.
fn chosen_over(candidate: &Selection, known: &Selection, by_loss: bool) -> bool {
    if by_loss { candidate.loss < known.loss } else { better(candidate, known) }
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
        assert!(metrics.iter().all(|row| row.desired != 0), "prefix development requires both classes in both lexical profiles");
        assert!(metrics.iter().all(|row| row.desired != row.sequences), "prefix development requires both classes in both lexical profiles");
        if metrics.iter().any(|row| row.false_conversions != 0) {
            continue;
        }
        let recall = |row: &Metrics| row.converted_before_end.to::<f64>() / row.desired.to::<f64>();
        let mut by_profile = Map::new();
        for (name, row) in PROFILES.iter().zip(&metrics) {
            let mut value = row.value();
            value["conversion_recall"] = json!(recall(row));
            by_profile.insert((*name).to_owned(), value);
        }
        let candidate = Selection {
            threshold,
            minimum_recall: recall(&metrics[0]).min(recall(&metrics[1])),
            pooled_recall: (metrics[0].converted_before_end + metrics[1].converted_before_end).to::<f64>() / (metrics[0].desired + metrics[1].desired).to::<f64>(),
            loss,
            by_profile: Value::Object(by_profile),
        };
        if best.as_ref().is_none_or(|known| better(&candidate, known)) {
            best = Some(candidate);
        }
    }
    best
}

/// The serving threshold: the first candidate that passes calibration, or 1.0 and a candidate that did not pass.
fn calibrated_threshold(scores: &[Sequence], thresholds: &[f64], minimum_recall: f64) -> (f64, bool) {
    for candidate in thresholds {
        if calibration_passed(&sequence_metrics(scores, *candidate), minimum_recall) {
            return (*candidate, true);
        }
    }
    (1.0, false)
}

fn calibration_passed(metrics: &[Metrics; 2], minimum_recall: f64) -> bool {
    if !(PREFIX_MIN_EARLY_RECALL_FLOOR..=1.0).contains(&minimum_recall) {
        return false;
    }
    metrics.iter().all(|counts| counts.desired != 0 && counts.sequences != counts.desired && counts.false_conversions == 0 && counts.converted_before_end.to::<f64>() / counts.desired.to::<f64>() >= minimum_recall)
}

/// The grid sorted, each threshold once.
fn sorted_grid(grid: &[f64]) -> Vec<f64> {
    let mut sorted = grid.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted.dedup();
    sorted
}

/// A direction's development threshold: the first of the grid at which, in both profiles, no sequence typed as meant in
/// that direction converts. None when none is clean or a profile lacks either kind of sequence.
fn clean_threshold(scores: &[Sequence], grid: &[f64], direction: usize) -> Option<f64> {
    sorted_grid(grid).into_iter().find(|threshold| {
        direction_metrics(scores, [*threshold, *threshold]).iter().all(|profile| {
            let cell = &profile[direction];
            cell.desired != 0 && cell.kept != 0 && cell.false_conversions == 0
        })
    })
}

/// A direction's threshold on calibration: the first of the grid, not below its development threshold, at which, in
/// both profiles, no sequence typed as meant in that direction converts and enough of those typed in the other layout
/// convert before their end. None when no threshold passes.
fn calibrated_direction(scores: &[Sequence], grid: &[f64], direction: usize, floor: f64, minimum_recall: f64) -> Option<f64> {
    if !(PREFIX_MIN_EARLY_RECALL_FLOOR..=1.0).contains(&minimum_recall) {
        return None;
    }
    sorted_grid(grid).into_iter().filter(|threshold| *threshold >= floor).find(|threshold| {
        direction_metrics(scores, [*threshold, *threshold]).iter().all(|profile| {
            let cell = &profile[direction];
            cell.desired != 0 && cell.kept != 0 && cell.false_conversions == 0 && cell.early_recall() >= minimum_recall
        })
    })
}

/// Both directions' thresholds on calibration, each not below its development threshold; a direction that allows none
/// keeps the top of the grid, and the calibration has not passed.
fn calibrated_thresholds(scores: &[Sequence], grid: &[f64], floors: [f64; 2], minimum_recall: f64) -> ([f64; 2], bool) {
    let found = [calibrated_direction(scores, grid, 0, floors[0], minimum_recall), calibrated_direction(scores, grid, 1, floors[1], minimum_recall)];
    match found {
        [Some(latin), Some(cyrillic)] => ([latin, cyrillic], true),
        _ => (found.map(|threshold| threshold.unwrap_or(1.0)), false),
    }
}

/// Epochs whose measures are this close are equal: the lower development loss, then the earlier epoch, is chosen.
pub const EQUAL_MEAN_KEY: f64 = 0.01;

/// What an epoch must not be worse than in each direction: the mean key and the count switched by `SWITCH_KEY` of the
/// installed model on the same development words at its threshold, both profiles together.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct KeyFloor {
    pub mean_key: [f64; 2],
    pub switched_by_key: [i64; 2],
}

impl KeyFloor {
    fn of(metrics: &[DirectionMetrics; 2]) -> KeyFloor {
        KeyFloor { mean_key: metrics.map(|cell| cell.mean_key()), switched_by_key: metrics.map(|cell| cell.switched_by_key) }
    }
}

/// An epoch as the mean key measures it, on development alone: each direction at its development threshold.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct KeyMeasure {
    pub epoch: i64,
    /// Each direction's development threshold.
    pub thresholds: [f64; 2],
    /// Each direction's mean switch key, both profiles together, and its count switched by `SWITCH_KEY`.
    pub mean_key: [f64; 2],
    pub switched_by_key: [i64; 2],
    /// The two directions' mean keys added: the lower, the earlier.
    pub measure: f64,
    pub loss: f64,
    /// Not worse than the floor in either direction: only such an epoch can be chosen.
    pub counts: bool,
}

impl KeyMeasure {
    /// The measure of an epoch's development scores; none when a direction has no development threshold.
    fn of(epoch: i64, scores: &[Sequence], grid: &[f64], loss: f64, floor: &KeyFloor) -> Option<KeyMeasure> {
        let thresholds = [clean_threshold(scores, grid, 0)?, clean_threshold(scores, grid, 1)?];
        let both = pooled(&direction_metrics(scores, thresholds));
        let (mean_key, switched_by_key) = (both.map(|cell| cell.mean_key()), both.map(|cell| cell.switched_by_key));
        let counts = (0..2).all(|direction| mean_key[direction] <= floor.mean_key[direction] && switched_by_key[direction] >= floor.switched_by_key[direction]);
        Some(KeyMeasure { epoch, thresholds, mean_key, switched_by_key, measure: mean_key[0] + mean_key[1], loss, counts })
    }
}

/// The trainer's kernel and the program's scoring of the written artifact count the same at these thresholds; otherwise
/// the thresholds would not be the ones the engine meets.
fn agree(trainer: &[Sequence], program: &[Sequence], thresholds: [f64; 2], split: &str) -> Result<(), String> {
    if direction_metrics(trainer, thresholds) == direction_metrics(program, thresholds) {
        Ok(())
    } else {
        Err(format!("the program's scoring of the written artifact differs from the trainer's on {split}"))
    }
}

/// An epoch of a fit by the mean key, as its seal's history lists it.
#[derive(serde::Serialize)]
struct KeyEpoch<'a> {
    epoch: i64,
    development_loss: f64,
    selection: Option<&'a KeyMeasure>,
}

/// The floor of a fit by the mean key: the installed model's mean key and count switched by `SWITCH_KEY` on the
/// development words, in each direction, both profiles together, at its own threshold. The recipe names the model, its
/// version and the threshold; a model of another version is refused.
fn key_floor(maker: &FrameMaker, parents: &[ParentWord], root: &Path, options: &RecipeOptions) -> Result<KeyFloor, String> {
    let (Some(path), Some(version), Some(threshold)) = (&options.floor_model, &options.floor_model_version, options.floor_threshold) else {
        return Err("a fit by the mean key names its floor: floor_model, floor_model_version and floor_threshold".to_owned());
    };
    let floor = ks_context::prefix_schema::VersionedPrefixModel::load(&root.join(path)).map_err(|error| format!("{path} does not load: {}", error.name))?;
    if floor.version() != version {
        return Err(format!("{path} is {}, the recipe names {version}", floor.version()));
    }
    let frames = maker.frames_of_schema(DEVELOPMENT, parents, floor.feature_version)?;
    let scores = sequence_scores(&frames, &runtime_probabilities(&floor.model, &frames));
    Ok(KeyFloor::of(&pooled(&direction_metrics(&scores, [threshold, threshold]))))
}

/// The chosen epoch among measured ones: of those that count, the lowest measure; those within `EQUAL_MEAN_KEY` of it are
/// equal, and of equal ones the lower development loss, then the earlier epoch.
fn chosen_epoch(measures: &[KeyMeasure]) -> Option<&KeyMeasure> {
    let counting: Vec<&KeyMeasure> = measures.iter().filter(|measure| measure.counts).collect();
    let lowest = counting.iter().map(|measure| measure.measure).fold(f64::INFINITY, f64::min);
    counting.into_iter().filter(|measure| measure.measure <= lowest + EQUAL_MEAN_KEY).min_by(|left, right| left.loss.total_cmp(&right.loss).then(left.epoch.cmp(&right.epoch)))
}

/// A direction's cell of a split's words typed as meant: its distinct words, the hard ones, and their distinct heads of
/// four keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Cell {
    pub words: usize,
    pub hard: usize,
    pub heads: usize,
}

/// The keys of a word's first four letters, as the US layout writes them.
fn head_of(word: &str, group: i64) -> String {
    let keys = if group == 1 { pair().translate(word, "ru", "us").expect("the pair of layouts") } else { word.to_owned() };
    pystr::casefold(&keys).chars().take(SWITCH_KEY.to::<usize>()).collect()
}

/// The words of a split typed as meant, by direction.
fn cells_of(parents: &[ParentWord]) -> [Cell; 2] {
    let mut words: [BTreeMap<&str, bool>; 2] = [BTreeMap::new(), BTreeMap::new()];
    let mut heads: [BTreeSet<String>; 2] = [BTreeSet::new(), BTreeSet::new()];
    for parent in parents {
        let direction = usize::from(parent.group == 1);
        let hard = words[direction].entry(&parent.original).or_insert(false);
        *hard = *hard || parent.hard;
        heads[direction].insert(head_of(&parent.original, parent.group));
    }
    [0, 1].map(|direction| Cell { words: words[direction].len(), hard: words[direction].values().filter(|hard| **hard).count(), heads: heads[direction].len() })
}

/// A recipe that names the size of a direction's cell is not fitted on a smaller one, in development or in calibration.
fn check_cells(split: &str, parents: &[ParentWord], options: &RecipeOptions) -> Result<[Cell; 2], String> {
    let cells = cells_of(parents);
    let (words, hard) = (options.minimum_words_per_direction.unwrap_or(0), options.minimum_hard_words_per_direction.unwrap_or(0));
    for (direction, cell) in cells.iter().enumerate() {
        if cell.words < words || cell.hard < hard {
            return Err(format!("{split} typed in layout group {direction} holds {} words, {} hard: the recipe asks for {words} and {hard}", cell.words, cell.hard));
        }
    }
    Ok(cells)
}

/// The inputs of a fit; what a seal pins.
fn prefix_provenance(root: &Path, recipe_path: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut result = provenance(root, recipe_path)?;
    if let Some(floor) = RecipeOptions::of(recipe_path).ok().and_then(|options| options.floor_model) {
        result.insert(floor.clone(), checksum(&root.join(&floor))?);
    }
    let names: Vec<String> = match corpus_of(recipe_path) {
        Some(corpus) => ["corpus.json", "config.json", "held-out.json.gz", "train.jsonl.gz", "development.jsonl.gz", "calibration.jsonl.gz"].iter().map(|name| format!("{corpus}/{name}")).collect(),
        None => ["model/prefix_v1/corpus.json", "model/prefix_v1/config.json", "model/prefix_v1/train.jsonl.gz", "model/prefix_v1/development.jsonl.gz", "model/prefix_v1/calibration.jsonl.gz"].map(str::to_owned).to_vec(),
    };
    for name in names {
        let digest = checksum(&root.join(&name))?;
        result.insert(name, digest);
    }
    Ok(result)
}

/// The corpus folder a recipe names, relative to the tree; without one (or without a readable recipe, which the fit
/// refuses on its own) the recipe reads the prefix-v1 corpus.
fn corpus_of(recipe_path: &Path) -> Option<String> {
    RecipeOptions::of(recipe_path).ok().and_then(|options| options.corpus)
}

/// The inputs a fit read at its start are the ones it seals.
fn prefix_inputs_unchanged(root: &Path, recipe_path: &Path, frozen: &BTreeMap<String, String>) -> Result<(), String> {
    if prefix_provenance(root, recipe_path)? != *frozen {
        return Err("prefix fit inputs changed".to_owned());
    }
    Ok(())
}

pub struct PrefixRequest<'a> {
    pub root: &'a Path,
    pub resource_dir: &'a Path,
    pub recipe: &'a Path,
    pub output: &'a Path,
    pub jobs: usize,
    pub reuse_installed: bool,
    /// A folder of frames kept by what they read (`FramesCache`).
    pub frames_cache: Option<&'a Path>,
    /// A fit by the mean key that does not read calibration: its thresholds are its development thresholds, and it has
    /// not passed calibration. Recipes are compared so; calibration is read once, by the fit of the chosen recipe.
    pub development_only: bool,
}

/// `reuse_installed`: the installed candidate and its seal as the output of a fit, when the seal pins this tree's inputs.
fn reuse_installed(request: &PrefixRequest<'_>) -> Result<Option<Value>, String> {
    let seal = read_json(&request.root.join("model/prefix_v2/candidate-seal.json"))?;
    let artifact = request.resource_dir.join("models/prefix_policy_v1.json");
    /// What decides whether the installed candidate is this tree's: the inputs its seal pins, its checksum, its calibration.
    #[derive(serde::Deserialize)]
    struct Installed {
        provenance: Option<BTreeMap<String, String>>,
        candidate_sha256: Option<String>,
        calibration_passed: Option<bool>,
    }
    let Ok(installed) = crate::artifact::typed::<Installed, _>(&seal) else {
        return Ok(None);
    };
    if installed.provenance != Some(prefix_provenance(request.root, request.recipe)?) || installed.candidate_sha256 != Some(checksum(&artifact)?) || installed.calibration_passed != Some(true) {
        return Ok(None);
    }
    std::fs::create_dir_all(request.output).map_err(|error| error.to_string())?;
    let copy = |from: &Path, name: &str| std::fs::copy(from, request.output.join(name)).map_err(|error| error.to_string());
    copy(&artifact, "prefix-candidate.json")?;
    copy(&request.root.join("model/prefix_v2/candidate-seal.json"), "seal.json")?;
    copy(request.recipe, "recipe.json")?;
    Ok(Some(seal))
}

/// What makes the frames of a split: the lexicons of both profiles, the orthotactic models when the schema reads them,
/// the recipe's rules, and the cache the frames may be kept in.
struct FrameMaker {
    portable: Lexicons,
    spelling: Lexicons,
    indexes: Indexes,
    ortho: Option<OrthoModel>,
    terms: Option<PrefixTerms>,
    rules: FrameRules,
    feature_version: i64,
    maximum_prefix_length: usize,
    jobs: usize,
    cache: Option<FramesCache>,
}

impl FrameMaker {
    fn new(root: &Path, resource_dir: &Path, feature_version: i64, maximum_prefix_length: usize, rules: FrameRules, jobs: usize, cache: Option<&Path>) -> Result<FrameMaker, String> {
        let resources: Arc<Resources> = Resources::with_speller(resource_dir.to_path_buf(), ks_hunspell::hunspell_speller(ks_config::history::data_dir()));
        let portable = reference_models(root, &resources, false)?;
        let spelling = reference_models(root, &resources, true)?;
        let indexes = indexes_of(&portable, &spelling);
        // Schemas three and four read the keys with the packaged orthotactic models, as the engine does; four the term
        // lists as well.
        let terms = (feature_version == TERMS_PREFIX_FEATURE_VERSION.to::<i64>()).then(|| prefix_terms_of(resource_dir, &portable)).transpose()?;
        let ortho = if feature_version == ks_context::constants::ORTHO_PREFIX_FEATURE_VERSION.to::<i64>() || terms.is_some() {
            Some(OrthoModel::load(&artifact_path(resource_dir), None, None, Some(resources.clone())).map_err(|error| format!("the orthotactic model: {}", error.name))?)
        } else {
            None
        };
        let cache = cache.map(|folder| FramesCache::open(folder, root, resource_dir)).transpose()?;
        Ok(FrameMaker { portable, spelling, indexes, ortho, terms, rules, feature_version, maximum_prefix_length, jobs, cache })
    }

    /// What the frames read besides their parents, worded for the frames cache.
    fn settings(&self, feature_version: i64, closed: Option<u64>) -> String {
        let terms = self.rules.terms.as_ref().map_or_else(String::new, |terms| hex(&Sha256::digest(terms.join("\n").as_bytes())));
        format!(
            "features {feature_version}\nmaximum prefix {}\nconvert from {}\nminimum {}\nengine support {}\nterms {terms}\nclosed {closed:?}",
            self.maximum_prefix_length, self.rules.convert_from, self.rules.minimum_characters, self.rules.engine_support
        )
    }

    /// The frames of a split's parents in the recipe's schema.
    fn frames(&self, split: &str, parents: &[ParentWord]) -> Result<SplitFrames, String> {
        self.frames_of_schema(split, parents, self.feature_version)
    }

    /// The frames of a split's parents in a schema, read back from the cache when it holds them. Only training closes the
    /// term lists to some families: development and calibration read them as the engine does.
    fn frames_of_schema(&self, split: &str, parents: &[ParentWord], feature_version: i64) -> Result<SplitFrames, String> {
        let closed = self.rules.closed_term_families_per_mille.filter(|_| split == TRAIN);
        let readers = PrefixReaders { ortho: self.ortho.as_ref(), terms: self.terms.as_ref() };
        let build = || generate_frames(parents, [&self.portable, &self.spelling], &self.indexes, self.maximum_prefix_length, feature_version, readers, closed, &self.rules, self.jobs);
        let Some(cache) = &self.cache else {
            return Ok(build());
        };
        let key = cache.key(&self.settings(feature_version, closed), parents);
        if let Some(found) = cache.read(&key) {
            println!("{split}: frames read from the cache");
            return Ok(found);
        }
        let built = build();
        cache.write(&key, &built)?;
        Ok(built)
    }
}

/// The probabilities the installed runtime gives each frame of a split: its own features and softmax.
fn runtime_probabilities(model: &PrefixModel, frames: &SplitFrames) -> Vec<f64> {
    let mut probabilities = Vec::with_capacity(frames.frames.len() * ACTIONS);
    for frame in &frames.frames {
        let values: indexmap_like::Map = frame.features.iter().map(|(id, value)| (frames.names[(*id).to::<usize>()].clone(), *value)).collect();
        probabilities.extend_from_slice(&model.predict_features(&values).probabilities);
    }
    probabilities
}

/// Each profile's directions, as a seal names them.
#[derive(serde::Serialize)]
struct ProfileDirections {
    portable: [DirectionMetrics; 2],
    reference_hunspell: [DirectionMetrics; 2],
}

impl ProfileDirections {
    fn of(metrics: [[DirectionMetrics; 2]; 2]) -> ProfileDirections {
        ProfileDirections { portable: metrics[0], reference_hunspell: metrics[1] }
    }
}

/// The floor an epoch chosen by the mean key was held to: the model, its threshold, its development measures.
#[derive(serde::Serialize)]
struct FloorSeal {
    model_version: String,
    threshold: f64,
    #[serde(flatten)]
    measures: KeyFloor,
}

/// What a fit by the mean key adds to its seal.
#[derive(serde::Serialize)]
struct DirectionSeal {
    thresholds: [f64; 2],
    development_thresholds: [f64; 2],
    calibration_read: bool,
    floor: FloorSeal,
    development_cells: [Cell; 2],
    calibration_cells: [Cell; 2],
    development: ProfileDirections,
    #[serde(skip_serializing_if = "Option::is_none")]
    calibration: Option<ProfileDirections>,
}

/// What it adds to its artifact: the threshold of each direction.
#[derive(serde::Serialize)]
struct DirectionArtifact {
    conversion_thresholds: [f64; 2],
}

/// The fields of a record added to a tree that is an object.
fn merged<T: serde::Serialize>(tree: &mut Value, fields: &T) -> Result<(), String> {
    match (tree.as_object_mut(), serde_json::to_value(fields).map_err(|error| error.to_string())?) {
        (Some(found), Value::Object(added)) => {
            found.extend(added);
            Ok(())
        }
        _ => Err("fields merged into a tree that is no object, or that are none".to_owned()),
    }
}

/// The numbers of a recipe a fit and a probe read.
#[derive(serde::Deserialize)]
struct RecipeNumbers {
    thresholds: Vec<f64>,
    maximum_prefix_length: usize,
    #[serde(default)]
    maximum_families: BTreeMap<String, i64>,
    #[serde(default)]
    maximum_words_per_family: i64,
}

/// `fit`: an explicit candidate fit, preserving all existing model directories.
pub fn fit(request: &PrefixRequest<'_>) -> Result<Value, String> {
    let output = request.output;
    if output.exists() {
        return Err("refusing to overwrite a prefix experiment".to_owned());
    }
    if request.reuse_installed
        && let Some(seal) = reuse_installed(request)? {
            return Ok(seal);
        }
    let cfg = read_json(request.recipe)?;
    let numbers: RecipeNumbers = crate::artifact::typed(&cfg).map_err(|error| format!("{}: {error}", request.recipe.display()))?;
    let feature_version = cfg.int("runtime_feature_version");
    // A recipe that names a corpus reads that word corpus whole; one that does not reads its share of prefix-v1.
    let options = RecipeOptions::of(request.recipe)?;
    let directory: PathBuf = request.root.join(options.corpus.as_deref().unwrap_or("model/prefix_v1"));
    let words = options.corpus.is_some();
    if words && !crate::prefix_words::is_word_corpus(&directory) {
        return Err(format!("{} holds no word corpus", directory.display()));
    }
    let frozen = prefix_provenance(request.root, request.recipe)?;
    std::fs::create_dir_all(output).map_err(|error| error.to_string())?;
    let mut parents: HashMap<&str, Vec<ParentWord>> = HashMap::new();
    for split in FITTING_SPLITS {
        let loaded = if words {
            load_word_parents(&directory, split)?
        } else {
            load_parents(&directory, split, numbers.maximum_families.get(split).copied().unwrap_or_default(), numbers.maximum_words_per_family)?
        };
        parents.insert(split, loaded);
    }
    let rules = frame_rules(&options, request.resource_dir)?;
    let patience = options.early_stop_patience;
    let selection_rule = options.epoch_selection.unwrap_or(EpochSelection::Frontier);
    let (by_loss, by_key) = (selection_rule == EpochSelection::DevelopmentLoss, selection_rule == EpochSelection::MeanKey);
    if request.development_only && !by_key {
        return Err("only a fit by the mean key leaves calibration unread".to_owned());
    }
    let family_sets: Vec<BTreeSet<&str>> = FITTING_SPLITS.iter().map(|split| parents[split].iter().map(|row| row.family.as_str()).collect()).collect();
    for left in 0..family_sets.len() {
        for right in left + 1..family_sets.len() {
            if family_sets[left].intersection(&family_sets[right]).next().is_some() {
                return Err("source physical families overlap".to_owned());
            }
        }
    }
    let development_cells = check_cells(DEVELOPMENT, &parents[DEVELOPMENT], &options)?;
    let calibration_cells = check_cells(CALIBRATION, &parents[CALIBRATION], &options)?;
    let maker = FrameMaker::new(request.root, request.resource_dir, feature_version, numbers.maximum_prefix_length, rules, request.jobs, request.frames_cache)?;
    let train_frames = maker.frames(TRAIN, &parents[TRAIN])?;
    println!("train: {} frames", train_frames.frames.len());
    let names = feature_vocabulary(&train_frames, cfg.float("minimum_feature_mass"), cfg.int("maximum_features").to::<usize>());
    if names.is_empty() {
        return Err("no prefix features survive the family-mass cutoff".to_owned());
    }
    let mean_one = options.importance_normalization == Some(ImportanceNormalization::MeanOne);
    let train = packed_rows(&train_frames, &names, &cfg, mean_one);
    let development_frames = maker.frames(DEVELOPMENT, &parents[DEVELOPMENT])?;
    let development = packed_rows(&development_frames, &names, &cfg, mean_one);
    // A feature survived, so a TRAIN frame holds it.
    assert!(train.rows() != 0, "TRAIN frames under the surviving features");
    if development.rows() == 0 {
        return Err("empty training or development split".to_owned());
    }
    // The floor of the mean key: the installed model on the same development words, at its own threshold.
    let floor = if by_key { Some(key_floor(&maker, &parents[DEVELOPMENT], request.root, &options)?) } else { None };
    // Calibration is read once: before the epochs only by the rule of v88, which chooses on development and needs no
    // calibration frames until the end; by the mean key after the epochs, for the chosen one.
    let mut weights = vec![0.0; names.len() * ACTIONS];
    let mut accumulator = vec![1.0; names.len() * ACTIONS];
    let mut best = weights.clone();
    let mut selected_epoch = 0;
    let mut selected: Option<Selection> = None;
    // By the mean key: every epoch's measure, and the weights of those that may still be chosen (`chosen_epoch`: the lowest
    // measure only falls, so an epoch above it by more than `EQUAL_MEAN_KEY` never comes back).
    let mut measures: Vec<KeyMeasure> = Vec::new();
    let mut kept_weights: BTreeMap<i64, Vec<f64>> = BTreeMap::new();
    let (mut lowest_measure, mut measure_epoch) = (f64::INFINITY, 0);
    let mut history = Vec::new();
    let denominator = fsum(&development.importance).expect("finite importance");
    let thresholds = numbers.thresholds;
    let minimum_recall = cfg.float("minimum_early_recall");
    let learning_rate = cfg.float("learning_rate");
    let (mut best_loss, mut best_loss_epoch, mut stopped) = (f64::INFINITY, 0, None);
    for epoch in 0..cfg.int("epochs") {
        kernel::epoch(&train, &mut weights, &mut accumulator, learning_rate);
        let rounded: Vec<f64> = weights.iter().map(|value| py_round_ndigits(*value, DETERMINISTIC_ROUNDING_DECIMALS).expect("a finite weight")).collect();
        let probabilities = kernel::predict(&development, &rounded);
        let terms: Vec<f64> = (0..development.rows()).map(|index| -development.importance[index] * probabilities[index * ACTIONS + usize::from(development.labels[index])].max(LOG_LOSS_PROBABILITY_FLOOR).ln()).collect();
        let loss = fsum(&terms).expect("finite losses") / denominator;
        assert!(loss.is_finite(), "prefix optimisation diverged");
        let development_scores = sequence_scores(&development_frames, &probabilities);
        if let Some(floor) = &floor {
            let measure = KeyMeasure::of(epoch + 1, &development_scores, &thresholds, loss, floor);
            println!(
                "prefix-v2 epoch {}: development loss {loss:.9}, mean key {:?}, counts {:?}",
                epoch + 1,
                measure.as_ref().map(|found| found.mean_key),
                measure.as_ref().map(|found| found.counts)
            );
            history.push(crate::artifact::typed(&KeyEpoch { epoch: epoch + 1, development_loss: loss, selection: measure.as_ref() })?);
            if let Some(found) = measure.filter(|found| found.counts) {
                if found.measure < lowest_measure {
                    (lowest_measure, measure_epoch) = (found.measure, epoch + 1);
                }
                kept_weights.insert(found.epoch, rounded.clone());
                measures.push(found);
                kept_weights.retain(|kept, _| measures.iter().any(|known| known.epoch == *kept && known.measure <= lowest_measure + EQUAL_MEAN_KEY));
            }
        } else {
            let candidate = assess(&development_scores, &thresholds, loss);
            let improved = candidate.as_ref().is_some_and(|found| selected.as_ref().is_none_or(|known| chosen_over(found, known, by_loss)));
            println!("prefix-v2 epoch {}: development loss {loss:.9}, early recall {:?}", epoch + 1, candidate.as_ref().map(|found| found.minimum_recall));
            history.push(json!({"epoch": epoch + 1, "development_loss": loss, "selection": candidate.as_ref().map_or_default(Selection::value)}));
            if improved {
                selected = candidate;
                best.clone_from(&rounded);
                selected_epoch = epoch + 1;
            }
        }
        if loss < best_loss {
            (best_loss, best_loss_epoch) = (loss, epoch + 1);
        }
        // By the mean key, patience watches the measure and the loss: the measure is a step function that may stand for
        // epochs and then move.
        let watched = if by_key { measure_epoch } else { selected_epoch };
        if patience_spent((epoch + 1).to::<usize>(), watched.to::<usize>(), best_loss_epoch.to::<usize>(), patience) {
            stopped = Some(epoch + 1);
            break;
        }
    }
    let (selection_loss, selection_value, development_thresholds) = if by_key {
        let chosen = chosen_epoch(&measures).ok_or("no epoch is as early as the installed model in both directions")?.clone();
        best.clone_from(kept_weights.get(&chosen.epoch).expect("the weights of an epoch that may be chosen are kept"));
        selected_epoch = chosen.epoch;
        (chosen.loss, crate::artifact::typed(&chosen)?, chosen.thresholds)
    } else {
        let chosen = selected.ok_or("no zero-false development epoch frontier")?;
        (chosen.loss, chosen.value(), [1.0, 1.0])
    };
    let row_of = |index: usize| best[index * ACTIONS..(index + 1) * ACTIONS].to_vec();
    let table: Map<String, Value> = names.iter().enumerate().map(|(index, name)| (name.clone(), json!(row_of(index)))).collect();
    let learned = Value::Object(table);
    let weight_digest = hex(&Sha256::digest(compact(&learned).as_bytes()));
    let version = format!("prefix-v{feature_version}-{}", weight_digest.get(..VERSION_HASH_CHARACTERS).unwrap_or_default());
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
    let final_development = sequence_scores(&development_frames, &kernel::predict(&development, &best));
    let actions: Vec<&str> = ks_context::context_model::ACTIONS.iter().map(|action| action.as_str()).collect();
    // The artifact's bytes at a threshold, with each direction's own when the fit chose them: a reader that knows only the
    // scalar takes the larger, stricter one.
    let artifact_of = |threshold: f64, directions: Option<[f64; 2]>| -> Result<Vec<u8>, String> {
        let mut payload = json!({"kind": "keyswitch.prefix-policy", "feature_version": feature_version, "prefix_feature_version": feature_version, "actions": actions,
            "version": version, "conversion_threshold": threshold, "weights_sha256": weight_digest, "weights": learned});
        if let Some(both) = directions {
            merged(&mut payload, &DirectionArtifact { conversion_thresholds: both })?;
        }
        Ok(crate::artifact::canonical(&payload))
    };
    // By the mean key the program scores the artifact as written, its weights read back from the file, and the trainer's
    // kernel must count as it does where the epoch was chosen and where the thresholds were: they are then the ones the
    // engine meets.
    let program = if by_key {
        let path = output.join("prefix-candidate.json");
        std::fs::write(&path, artifact_of(1.0, None)?).map_err(|error| error.to_string())?;
        ks_context::prefix_schema::VersionedPrefixModel::load(&path).map_err(|error| format!("prefix export does not load: {}", error.name))?.model
    } else {
        model
    };
    if by_key {
        agree(&final_development, &sequence_scores(&development_frames, &runtime_probabilities(&program, &development_frames)), development_thresholds, DEVELOPMENT)?;
    }
    let calibration_frames = if request.development_only { None } else { Some(maker.frames(CALIBRATION, &parents[CALIBRATION])?) };
    // The runtime's own softmax: what the installed candidate answers.
    let scores = calibration_frames.as_ref().map(|frames| sequence_scores(frames, &runtime_probabilities(&program, frames)));
    let (per_direction, passed) = match &scores {
        None => (development_thresholds, false),
        Some(scores) if by_key => calibrated_thresholds(scores, &thresholds, development_thresholds, minimum_recall),
        Some(scores) => {
            let (threshold, passed) = calibrated_threshold(scores, &thresholds, minimum_recall);
            ([threshold, threshold], passed)
        }
    };
    if by_key && let (Some(frames), Some(scores)) = (&calibration_frames, &scores) {
        let trainer = sequence_scores(frames, &kernel::predict(&packed_rows(frames, &names, &cfg, mean_one), &best));
        agree(&trainer, scores, per_direction, CALIBRATION)?;
    }
    let threshold = per_direction[0].max(per_direction[1]);
    let raw = artifact_of(threshold, by_key.then_some(per_direction))?;
    let mut budgets = Map::new();
    let mut pair_counts = Map::new();
    for (split, frames) in [(TRAIN, Some(&train_frames)), (DEVELOPMENT, Some(&development_frames)), (CALIBRATION, calibration_frames.as_ref())] {
        let Some(frames) = frames else { continue };
        let mut amounts = vec![0.0_f64; frames.families.len()];
        let mut errors = vec![0.0_f64; frames.families.len()];
        for frame in &frames.frames {
            let family = frame.parent_family.to::<usize>();
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
    let calibration_value = scores.as_ref().map_or(Value::Null, |scores| metrics_value(&sequence_metrics(scores, threshold)));
    let mut seal = json!({
        "schema_version": 1, "candidate_sha256": hex(&Sha256::digest(&raw)), "provenance": frozen, "recipe": cfg, "model_version": version, "features": names.len(),
        "epoch": selected_epoch, "development_loss": selection_loss, "threshold": threshold, "epoch_selection": selection_value, "epoch_history": history,
        "training_rows": train.rows(), "development_rows": development.rows(), "family_budgets": budgets, "pair_counts": pair_counts, "selected_parents": selected_parents,
        "development": metrics_value(&sequence_metrics(&final_development, threshold)), "calibration": calibration_value, "calibration_passed": passed,
        "promotion_accepted": false, "independent_test_evaluated": false, "scope": cfg["scope"],
    });
    if by_key {
        let floor = floor.expect("a fit by the mean key has its floor");
        let fields = DirectionSeal {
            thresholds: per_direction,
            development_thresholds,
            calibration_read: scores.is_some(),
            floor: FloorSeal {
                model_version: options.floor_model_version.clone().unwrap_or_default(),
                threshold: options.floor_threshold.unwrap_or_default(),
                measures: floor,
            },
            development_cells,
            calibration_cells,
            development: ProfileDirections::of(direction_metrics(&final_development, per_direction)),
            calibration: scores.as_ref().map(|scores| ProfileDirections::of(direction_metrics(scores, per_direction))),
        };
        merged(&mut seal, &fields)?;
    }
    prefix_inputs_unchanged(request.root, request.recipe, &frozen)?;
    let write = |name: &str, bytes: &[u8]| std::fs::write(output.join(name), bytes).map_err(|error| error.to_string());
    write("prefix-candidate.json", &raw)?;
    ks_context::prefix_schema::VersionedPrefixModel::load(&output.join("prefix-candidate.json")).map_err(|error| format!("prefix export does not load: {}", error.name))?;
    write("seal.json", &readable(&seal))?;
    // What a word corpus's fit adds, beside the seal: a sealed prefix-v1 recipe's outputs stay as they were.
    if options.reports_classes() {
        let saved = SavedScores {
            model_version: version,
            epoch: selected_epoch,
            stopped_after_epoch: stopped,
            thresholds: per_direction,
            grid: thresholds,
            development: SavedSplit::of(&final_development, &development_frames, true),
            calibration: match (&scores, &calibration_frames) {
                (Some(scores), Some(frames)) => SavedSplit::of(scores, frames, false),
                _ => SavedSplit { classes: Vec::new(), sequences: Vec::new() },
            },
        };
        write(SCORES_FILE, &saved_bytes(&saved)?)?;
        write("classes.json", &ClassReport::of(&saved).bytes())?;
    }
    write("recipe.json", &std::fs::read(request.recipe).map_err(|error| error.to_string())?)?;
    Ok(seal)
}

/// What `prefix-probe` measures a prefix model on: a split of the recipe's corpus, or words of a list, each typed
/// alone in an empty field as meant and in the other layout.
pub enum ProbeWords<'a> {
    Split(&'a str),
    List(&'a Path),
}

pub struct ProbeRequest<'a> {
    pub root: &'a Path,
    pub resource_dir: &'a Path,
    pub model: &'a Path,
    pub recipe: &'a Path,
    pub words: ProbeWords<'a>,
    /// The fewest letters of a prefix the model is asked about, when not the recipe's.
    pub minimum_characters: Option<usize>,
    /// Write no text of the words: a sample whose texts are read nowhere.
    pub numbers_only: bool,
    pub output: &'a Path,
    pub jobs: usize,
    pub frames_cache: Option<&'a Path>,
}

/// What `prefix-probe` writes: each direction at the model's thresholds and at every threshold of the grid, each class,
/// and the texts typed as meant that the model's thresholds convert (none for calibration).
#[derive(serde::Serialize)]
pub struct ProbeReport {
    model_version: String,
    thresholds: [f64; 2],
    minimum_characters: usize,
    words: usize,
    /// `[profile][direction]` at the model's thresholds.
    at_model: [[DirectionMetrics; 2]; 2],
    /// Both profiles together, each direction, at every threshold of the grid.
    by_threshold: BTreeMap<String, [DirectionMetrics; 2]>,
    by_class: BTreeMap<String, ClassMetrics>,
    /// The distinct words typed as meant that the model's thresholds convert at any prefix it is asked about, in either
    /// profile, by the layout group of the keys.
    false_words: [usize; 2],
    false_texts: Vec<String>,
}

/// The parents of a list of words, one a line: each typed alone in an empty field, as meant and in the other layout.
fn listed_parents(path: &Path) -> Result<Vec<ParentWord>, String> {
    let text = std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut parents = Vec::new();
    for word in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let group = match word.chars().find(|character| character.is_alphabetic()).and_then(crate::evaluate::script_group) {
            Some(group) => group,
            None => return Err(format!("{}: {word} opens with a letter of neither layout", path.display())),
        };
        parents.push(ParentWord {
            original: word.to_owned(),
            group,
            family: crate::prefix_corpus::family(word, group),
            before: String::new(),
            identifier: format!("words:{}", parents.len()),
            category: "word".to_owned(),
            application: WORD_CORPUS_APPLICATION.to_owned(),
            role: FieldRole::Unknown,
            paired: true,
            own_context: true,
            hard: false,
        });
    }
    Ok(parents)
}

/// `prefix-probe`: a prefix model measured without the engine, as a fit measures its development and calibration.
pub fn probe(request: &ProbeRequest<'_>) -> Result<ProbeReport, String> {
    let cfg: RecipeNumbers = ks_text::json::loads(std::fs::read(request.recipe).map_err(|error| format!("{}: {error}", request.recipe.display()))?).map_err(|error| format!("{}: {error}", request.recipe.display()))?;
    let options = RecipeOptions::of(request.recipe)?;
    let loaded = ks_context::prefix_schema::VersionedPrefixModel::load(request.model).map_err(|error| format!("{}: {}", request.model.display(), error.name))?;
    let (parents, texts) = match request.words {
        ProbeWords::List(path) => (listed_parents(path)?, !request.numbers_only),
        // Calibration is read once, by the fit of the chosen recipe; the acceptance split by its samples alone.
        ProbeWords::Split(split) if split != DEVELOPMENT => return Err(format!("a probe reads development or a list of words, not {split}")),
        ProbeWords::Split(split) => {
            let directory = request.root.join(options.corpus.as_deref().unwrap_or("model/prefix_v1"));
            let parents = if options.corpus.is_some() {
                load_word_parents(&directory, split)?
            } else {
                load_parents(&directory, split, cfg.maximum_families.get(split).copied().unwrap_or_default(), cfg.maximum_words_per_family)?
            };
            (parents, !request.numbers_only)
        }
    };
    let mut rules = frame_rules(&options, request.resource_dir)?;
    if let Some(minimum) = request.minimum_characters {
        rules.minimum_characters = minimum;
    }
    let minimum_characters = rules.minimum_characters;
    let maker = FrameMaker::new(request.root, request.resource_dir, loaded.feature_version, cfg.maximum_prefix_length, rules, request.jobs, request.frames_cache)?;
    let frames = maker.frames("probe", &parents)?;
    let scores = sequence_scores(&frames, &runtime_probabilities(&loaded.model, &frames));
    let thresholds = loaded.thresholds();
    let grid = cfg.thresholds;
    let typed: Vec<Option<String>> = frames.typed.iter().map(|text| texts.then(|| text.clone())).collect();
    let report = ProbeReport {
        model_version: loaded.version().to_owned(),
        thresholds,
        minimum_characters,
        words: parents.len(),
        at_model: direction_metrics(&scores, thresholds),
        by_threshold: grid.iter().map(|threshold| (threshold.to_string(), pooled(&direction_metrics(&scores, [*threshold, *threshold])))).collect(),
        by_class: class_metrics(&scores, thresholds, &frames.classes),
        false_words: false_words(&scores, thresholds, &frames.typed),
        false_texts: false_texts(&scores, thresholds, &frames.classes, &typed),
    };
    let options = JsonOptions { ensure_ascii: false, indent: Some(ks_text::Indent::Spaces(1)), sort_keys: true, ..JsonOptions::default() };
    std::fs::write(request.output, ks_text::json::dumps(&report, &options) + "\n").map_err(|error| format!("{}: {error}", request.output.display()))?;
    Ok(report)
}

mod indexmap_like {
    pub(super) type Map = ks_context::context_action_features::FeatureMap;
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    fn sequence(profile: u8, desired: bool, candidates: &[(u32, f64)]) -> Sequence {
        Sequence { profile, desired, full_length: 6, class: 0, source: 0, hard: false, candidates: candidates.to_vec() }
    }

    #[test]
    fn a_prefix_typed_in_the_other_layout_converts_from_the_recipes_letters_unless_a_word_or_a_term_goes_on() {
        let (four, three) = (FrameRules::default(), FrameRules { convert_from: 3, ..FrameRules::default() });
        assert_eq!([2, 3, 4].map(|length| label_of(true, length, false, false, &four)), [2, 2, 1]);
        assert_eq!([2, 3, 4].map(|length| label_of(true, length, false, false, &three)), [2, 1, 1]);
        assert_eq!((label_of(true, 5, true, false, &three), label_of(true, 5, false, true, &three), label_of(false, 5, false, false, &three)), (2, 2, 0));
        let terms = vec!["git".to_owned(), "github".to_owned(), "kubectl".to_owned()];
        assert!(begins_a_term(&terms, "Gith") && begins_a_term(&terms, "kub") && !begins_a_term(&terms, "gk") && !begins_a_term(&terms, "zzz"));
    }

    #[test]
    fn the_engine_rule_asks_about_a_prefix_whose_other_reading_is_letters_and_whose_last_key_is_a_letter() {
        let engine = FrameRules { engine_support: true, ..FrameRules::default() };
        let legacy = FrameRules::default();
        // `<kap` is `Бкап`: the engine asks; the legacy rule wants letters on both sides.
        assert!(supported_of("<kap", "Бкап", &engine) && !supported_of("<kap", "Бкап", &legacy));
        assert!(supported_of("hous", "рщгы", &engine) && supported_of("hous", "рщгы", &legacy));
        // A sign inside the prefix is a letter of the other layout; one that ends it may be punctuation; a capital inside a
        // word is not asked about.
        assert!(supported_of("lj,f", "доба", &engine) && !supported_of("lj,f", "доба", &legacy));
        assert!(!supported_of("ljf,", "доаб", &engine));
        assert!(!supported_of("hoUs", "рщГы", &engine) && !supported_of("hou", "рщг", &engine));
        assert!(supported_of("hou", "рщг", &FrameRules { minimum_characters: 3, ..engine }));
    }

    #[test]
    fn an_epoch_is_chosen_by_its_frontier_or_by_its_development_loss() {
        let point = |recall: f64, loss: f64| Selection { threshold: 0.99, minimum_recall: recall, pooled_recall: recall, loss, by_profile: json!({}) };
        let (early, late) = (point(0.86, 0.17), point(0.84, 0.15));
        assert!(!chosen_over(&late, &early, false) && chosen_over(&late, &early, true));
        assert!(chosen_over(&early, &late, false) && !chosen_over(&early, &late, true));
    }

    #[test]
    fn a_fit_stops_when_neither_its_frontier_nor_its_loss_moved_for_its_patience() {
        assert!(!patience_spent(30, 25, 20, None));
        assert!(!patience_spent(30, 25, 20, Some(10)));
        assert!(patience_spent(35, 25, 20, Some(10)));
        assert!(!patience_spent(35, 25, 30, Some(10)));
    }

    #[test]
    fn the_class_metrics_count_each_class_and_where_its_first_conversion_came() {
        let classes = vec!["name:0".to_owned(), "rare:1".to_owned()];
        let scores = [Sequence { class: 1, ..sequence(0, true, &[(4, 0.99), (5, 0.999)]) }, sequence(1, false, &[(5, 0.999)]), Sequence { class: 1, ..sequence(0, true, &[]) }];
        let value = class_metrics(&scores, [0.99, 0.99], &classes);
        let rare = &value["rare:1"];
        assert_eq!((rare.desired, rare.converted_before_end, rare.first_conversion_length.clone()), (2, 1, [(0, 1), (4, 1)].into_iter().collect()));
        assert_eq!(value["name:0"].false_conversions, 1);
        let typed = [Some("ghbdtn".to_owned()), Some("kaplan".to_owned()), Some("ghbd".to_owned())];
        assert_eq!(false_texts(&scores, [0.99, 0.99], &classes, &typed), ["name:0: kaplan"]);
        assert!(false_texts(&scores, [0.9999, 0.9999], &classes, &typed).is_empty());
        // A text not kept (calibration's) is not named.
        assert!(false_texts(&scores, [0.99, 0.99], &classes, &[None, None, None]).is_empty());
        assert_eq!(ks_text::json::dumps(&value["name:0"], &JsonOptions::default()), r#"{"sequences": 1, "desired": 0, "converted": 0, "converted_before_end": 0, "false": 1, "first_conversion_length": {}}"#);
    }

    /// Each profile: a sequence to convert, converted early at 0.99, and one to keep, converted at 0.9.
    fn development() -> Vec<Sequence> {
        (0..2).flat_map(|profile| [sequence(profile, true, &[(4, 0.99), (6, 0.999)]), sequence(profile, false, &[(5, 0.9)])]).collect()
    }

    #[test]
    fn an_operating_point_with_a_false_conversion_is_never_chosen() {
        let chosen = assess(&development(), &[0.95, 0.5, 0.95], 0.25).expect("a point without false conversions");
        assert_eq!((chosen.threshold, chosen.minimum_recall, chosen.pooled_recall), (0.95, 1.0, 1.0));
        assert!(assess(&development(), &[0.5], 0.25).is_none());
        assert!(assess(&development(), &[], 0.25).is_none());
    }

    #[test]
    fn calibration_passes_on_both_classes_in_both_profiles_with_no_false_conversion_and_enough_recall() {
        let counts = |sequences: i64, desired: i64, early: i64, false_conversions: i64| {
            let row = Metrics { sequences, desired, converted: early, converted_before_end: early, false_conversions };
            [row, row]
        };
        assert!(calibration_passed(&counts(4, 2, 2, 0), 0.7));
        assert!(!calibration_passed(&counts(4, 2, 2, 0), 0.5));
        assert!(!calibration_passed(&counts(4, 2, 1, 0), 0.7));
        assert!(!calibration_passed(&counts(4, 0, 0, 0), 0.7));
        assert!(!calibration_passed(&counts(2, 2, 2, 0), 0.7));
        assert!(!calibration_passed(&counts(4, 2, 2, 1), 0.7));
        // The first candidate that passes serves; with none, the candidate serves at 1.0 and has not passed.
        assert_eq!(calibrated_threshold(&development(), &[0.5, 0.95, 0.995], 0.7), (0.95, true));
        assert_eq!(calibrated_threshold(&development(), &[0.5], 0.7), (1.0, false));
    }

    #[test]
    fn a_prefix_fit_whose_inputs_changed_while_it_ran_is_not_sealed() {
        let root = std::env::temp_dir().join(format!("ks-train-prefix-unchanged-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let write = |name: &str, text: &str| {
            let path = root.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        for name in ["ks-text", "ks-backend", "ks-config", "ks-lang", "ks-context", "ks-engine", "ks-replay", "ks-train", "ks-hunspell"] {
            write(&format!("crates/{name}/src/lib.rs"), name);
        }
        for name in [
            "Cargo.lock", "resources/models/model.json", "model/context_v1/scenarios.json", "model/context_v1/captured/manifest.json", "model/context_v3/captured/manifest.json",
            "model/context_v3/baseline-context-v1.json", "model/intent_v1/config.json", "model/intent_v1/sources/en_US.lm", "model/intent_v1/sources/ru_RU.lm", "resources/protected_tokens.txt",
            "resources/identifiers.json", "resources/lexicon-supplement-ru_RU.json", "recipe.json", "model/prefix_v1/corpus.json", "model/prefix_v1/config.json",
            "model/prefix_v1/train.jsonl.gz", "model/prefix_v1/development.jsonl.gz", "model/prefix_v1/calibration.jsonl.gz",
        ] {
            write(name, name);
        }
        let recipe = root.join("recipe.json");
        let frozen = prefix_provenance(&root, &recipe).unwrap();
        assert!(frozen.contains_key("model/prefix_v1/train.jsonl.gz"));
        prefix_inputs_unchanged(&root, &recipe, &frozen).unwrap();
        write("model/prefix_v1/development.jsonl.gz", "other");
        assert!(prefix_inputs_unchanged(&root, &recipe, &frozen).unwrap_err().contains("inputs changed"));
    }

    /// A sequence typed in a direction: desired ones convert at the lengths given.
    fn typed(profile: u8, source: u8, desired: bool, candidates: &[(u32, f64)]) -> Sequence {
        Sequence { source, ..sequence(profile, desired, candidates) }
    }

    #[test]
    fn a_direction_counts_the_key_of_each_first_switch_and_the_boundary_of_a_word_never_switched() {
        let scores = [typed(0, 1, true, &[(4, 0.99)]), typed(0, 1, true, &[(5, 0.999)]), typed(0, 1, true, &[]), typed(0, 1, false, &[(6, 0.999)]), typed(1, 0, true, &[(4, 0.9999)])];
        let metrics = direction_metrics(&scores, [0.995, 0.99]);
        let cyrillic = metrics[0][1];
        // Keys 4, 5 and the boundary 7 of a six-letter word.
        assert_eq!((cyrillic.desired, cyrillic.converted, cyrillic.converted_before_end, cyrillic.switched_by_key, cyrillic.key_sum), (3, 2, 2, 1, 16));
        assert_eq!((cyrillic.kept, cyrillic.false_conversions), (1, 1));
        assert!((cyrillic.share_by_key() - 1.0 / 3.0).abs() < 1e-12 && (cyrillic.mean_key() - 16.0 / 3.0).abs() < 1e-12);
        assert_eq!(metrics[1][0].switched_by_key, 1);
        // The other direction's threshold does not apply: at 0.99999 for the Latin keys the last word is not switched.
        assert_eq!(direction_metrics(&scores, [0.99999, 0.99])[1][0].switched_by_key, 0);
        let both = pooled(&metrics);
        assert_eq!((both[0].desired, both[1].desired), (1, 3));
        let empty = DirectionMetrics::default();
        assert_eq!((empty.share_by_key(), empty.mean_key(), empty.early_recall()), (0.0, 0.0, 0.0));
        assert_eq!(ks_text::json::dumps(&empty, &JsonOptions::default()), r#"{"desired": 0, "converted": 0, "converted_before_end": 0, "switched_by_key": 0, "key_sum": 0, "kept": 0, "false": 0}"#);
    }

    #[test]
    fn the_trainer_and_the_program_agree_when_they_count_the_same_at_the_thresholds() {
        let trainer = [typed(0, 0, true, &[(4, 0.9991)]), typed(0, 0, false, &[])];
        // A probability that differs and still converts at the same key counts the same.
        let same = [typed(0, 0, true, &[(4, 0.9992)]), typed(0, 0, false, &[])];
        assert!(agree(&trainer, &same, [0.999, 0.999], "development").is_ok());
        let later = [typed(0, 0, true, &[(5, 0.9992)]), typed(0, 0, false, &[])];
        assert_eq!(agree(&trainer, &later, [0.999, 0.999], "calibration").unwrap_err(), "the program's scoring of the written artifact differs from the trainer's on calibration");
    }

    /// Calibration of both profiles: in each direction a word to convert (at 4 keys from 0.99) and a word to keep, which
    /// the Latin direction converts at 0.995 and below and the Cyrillic direction never.
    fn calibration() -> Vec<Sequence> {
        (0..2).flat_map(|profile| [typed(profile, 0, true, &[(4, 0.999)]), typed(profile, 0, false, &[(5, 0.995)]), typed(profile, 1, true, &[(4, 0.99)]), typed(profile, 1, false, &[])]).collect()
    }

    #[test]
    fn a_direction_is_clean_on_development_at_the_first_threshold_no_word_typed_as_meant_converts() {
        let grid = [0.999, 0.985, 0.99, 0.995, 0.999];
        // The Latin word to keep converts at 0.995 and below; the Cyrillic one never.
        assert_eq!((clean_threshold(&calibration(), &grid, 0), clean_threshold(&calibration(), &grid, 1)), (Some(0.999), Some(0.985)));
        assert_eq!(clean_threshold(&calibration(), &[0.99], 0), None);
        // A direction without words to keep in a profile has no threshold.
        let lacking: Vec<Sequence> = calibration().into_iter().filter(|value| value.desired || value.source == 0 || value.profile == 0).collect();
        assert_eq!(clean_threshold(&lacking, &grid, 1), None);
        // Nor one without words to convert: calibration then has no threshold for it either.
        let wanting: Vec<Sequence> = calibration().into_iter().filter(|value| !value.desired || value.source == 0 || value.profile == 0).collect();
        assert_eq!(clean_threshold(&wanting, &grid, 1), None);
        assert_eq!((calibrated_direction(&wanting, &grid, 1, 0.985, 0.7), calibrated_direction(&lacking, &grid, 1, 0.985, 0.7)), (None, None));
    }

    #[test]
    fn calibration_takes_each_direction_s_first_threshold_not_below_its_development_threshold() {
        let grid = [0.999, 0.985, 0.99, 0.995, 0.999];
        assert_eq!(calibrated_thresholds(&calibration(), &grid, [0.985, 0.985], 0.7), ([0.999, 0.985], true));
        // Development made the Cyrillic direction clean at 0.99 only: calibration does not go below it.
        assert_eq!(calibrated_thresholds(&calibration(), &grid, [0.985, 0.99], 0.7), ([0.999, 0.99], true));
        // Too much recall asked of the Cyrillic direction at no threshold: it is converted at 0.99 only. It keeps the top of
        // the grid, and calibration has not passed.
        assert_eq!(calibrated_direction(&calibration(), &[0.995, 0.999], 1, 0.985, 0.7), None);
        assert_eq!(calibrated_thresholds(&calibration(), &[0.995, 0.999], [0.985, 0.985], 0.7), ([0.999, 1.0], false));
        // A recall floor below the program's is no rule.
        assert_eq!(calibrated_direction(&calibration(), &grid, 1, 0.985, 0.5), None);
    }

    /// Development of both profiles: the Latin word to convert switches at `key`, the Cyrillic one at 5; a word to keep in
    /// each direction as in `calibration`.
    fn switching(key: u32) -> Vec<Sequence> {
        (0..2)
            .flat_map(|profile| [typed(profile, 0, true, &[(key, 0.999)]), typed(profile, 0, false, &[(5, 0.995)]), typed(profile, 1, true, &[(5, 0.99)]), typed(profile, 1, false, &[])])
            .collect()
    }

    #[test]
    fn an_epoch_is_measured_by_its_mean_key_on_development_and_counts_only_if_no_later_than_the_floor() {
        let grid = [0.985, 0.99, 0.995, 0.999];
        let floor = KeyFloor { mean_key: [4.5, 5.0], switched_by_key: [2, 0] };
        let early = KeyMeasure::of(1, &switching(4), &grid, 0.2, &floor).expect("a clean threshold in both directions");
        assert_eq!((early.thresholds, early.mean_key, early.switched_by_key), ([0.999, 0.985], [4.0, 5.0], [2, 0]));
        assert!((early.measure - 9.0).abs() < 1e-12 && early.counts);
        // Later than the floor in the Latin direction, or fewer words switched by the switch key: it does not count.
        assert!(!KeyMeasure::of(1, &switching(5), &grid, 0.2, &floor).expect("measured").counts);
        assert!(!KeyMeasure::of(1, &switching(4), &grid, 0.2, &KeyFloor { switched_by_key: [3, 0], ..floor }).expect("measured").counts);
        // No clean threshold in a direction: no measure.
        assert!(KeyMeasure::of(1, &switching(4), &[0.99], 0.2, &floor).is_none());
        let measure = |epoch: i64, measure: f64, loss: f64, counts: bool| KeyMeasure { epoch, measure, loss, counts, ..early.clone() };
        // Of measures within `EQUAL_MEAN_KEY` of the lowest, the lower loss, then the earlier epoch; an epoch that does not
        // count is never chosen.
        let measures = [measure(1, 9.0, 0.2, true), measure(2, 9.005, 0.1, true), measure(3, 8.0, 0.0, false)];
        assert_eq!(chosen_epoch(&measures).map(|found| found.epoch), Some(2));
        let tied = [measure(1, 9.0, 0.1, true), measure(2, 9.0, 0.1, true)];
        assert_eq!(chosen_epoch(&tied).map(|found| found.epoch), Some(1));
        assert_eq!(chosen_epoch(&[measure(4, 8.5, 0.5, true), measures[0].clone()]).map(|found| found.epoch), Some(4));
        assert!(chosen_epoch(&[measure(3, 8.0, 0.0, false)]).is_none());
    }

    #[test]
    fn a_cell_counts_distinct_words_typed_as_meant_by_direction_its_hard_ones_and_their_heads() {
        let parent = |original: &str, group: i64, hard: bool| ParentWord {
            original: original.to_owned(),
            group,
            family: String::new(),
            before: String::new(),
            identifier: original.to_owned(),
            category: "rare".to_owned(),
            application: String::new(),
            role: FieldRole::Unknown,
            paired: true,
            own_context: true,
            hard,
        };
        let parents = [parent("cntk", 0, true), parent("cntk", 0, false), parent("house", 0, false), parent("housing", 0, false), parent("офсет", 1, true)];
        let cells = [Cell { words: 3, hard: 1, heads: 2 }, Cell { words: 1, hard: 1, heads: 1 }];
        assert_eq!(cells_of(&parents), cells);
        let options = |words: usize, hard: usize| RecipeOptions { minimum_words_per_direction: Some(words), minimum_hard_words_per_direction: Some(hard), ..RecipeOptions::default() };
        assert_eq!(check_cells(CALIBRATION, &parents, &options(1, 1)), Ok(cells));
        assert!(check_cells(CALIBRATION, &parents, &options(2, 1)).unwrap_err().contains("calibration typed in layout group 1 holds 1 words, 1 hard"));
        assert!(check_cells(DEVELOPMENT, &parents, &options(1, 2)).unwrap_err().contains("development typed in layout group 0 holds 3 words, 1 hard"));
        assert!(check_cells(CALIBRATION, &[], &RecipeOptions::default()).is_ok());
        assert_eq!(head_of("офсет", 1), "jact");
    }

    #[test]
    fn the_class_report_is_made_again_from_the_saved_scores_of_a_fit() {
        let frames = SplitFrames {
            frames: Vec::new(),
            names: Vec::new(),
            families: Vec::new(),
            pairs: 0,
            classes: vec!["rare:0".to_owned()],
            typed: ["ghbdtn", "cntk", "ghbd", "house", "ghbdtn", "cntk", "ghbd", "house"].map(str::to_owned).to_vec(),
        };
        let saved = SavedScores {
            model_version: "prefix-v3-0".to_owned(),
            epoch: 3,
            stopped_after_epoch: Some(5),
            thresholds: [0.999, 0.985],
            grid: vec![0.985, 0.999],
            development: SavedSplit::of(&calibration(), &frames, true),
            calibration: SavedSplit::of(&calibration(), &frames, false),
        };
        assert!(saved.calibration.sequences.iter().all(|item| item.text.is_none()));
        let bytes = saved_bytes(&saved).expect("bytes");
        let folder = std::env::temp_dir().join(format!("ks-train-saved-scores-{}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("a folder");
        std::fs::write(folder.join(SCORES_FILE), bytes).expect("a file");
        assert_eq!(read_saved_scores(&folder), Ok(saved.clone()));
        let output = folder.join("classes.json");
        report_command(&folder, &output).expect("a report");
        assert_eq!(std::fs::read(&output).expect("the report"), ClassReport::of(&saved).bytes());
        let report: Value = serde_json::from_slice(&ClassReport::of(&saved).bytes()).expect("json");
        // At 0.985 the Latin direction converts `cntk` (the word kept in that direction) in both profiles.
        assert_eq!(report["development_false"]["0.985"], json!(["rare:0: cntk"]));
        assert_eq!(report["by_threshold"]["0.985"]["calibration"][0]["false"], json!(2));
        assert_eq!(report["calibration"]["rare:0"]["false"], json!(0));
        assert!(read_saved_scores(&folder.join("missing")).is_err());
        std::fs::write(folder.join(SCORES_FILE), b"not gzip").expect("a file");
        assert!(read_saved_scores(&folder).is_err());
    }

    #[test]
    fn fields_are_merged_only_into_an_object_and_only_from_a_record() {
        let mut tree = json!({"a": 1});
        merged(&mut tree, &Cell { words: 2, hard: 1, heads: 3 }).expect("merged");
        assert_eq!(tree, json!({"a": 1, "words": 2, "hard": 1, "heads": 3}));
        assert!(merged(&mut tree, &5).is_err());
        assert!(merged(&mut json!([1]), &Cell { words: 2, hard: 1, heads: 3 }).is_err());
    }
}
