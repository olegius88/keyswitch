//! Small local, learned contextual action policy, separate from Layout Intent.
//!
//! The model is a four-class sparse softmax classifier. It receives the recent sentence, application/field evidence and
//! the existing detector's lexical evidence. The context action trainer produces feature3 weights, the one schema this
//! port reads (the Python still loads the context-v1 schemas 2, 5, 6 and 7 of old artifacts; no such artifact is
//! shipped, and `extract_context_features` is not ported). Probabilities are corpus scores, not a promise of
//! real-world correctness. Hard safety/explicit user intent live in the engine.
//!
//! `context_action_features` imports `ContextEvidence` and `term_bucket` from here and this module imports its
//! extractor; in Rust both are modules of one crate, so the cycle is only two `use` lines.

use ks_cast::To as _;
use crate::constants::{
    ACTION_FEATURE_CHARACTER_TEXT_FIELD_INDEX, ALNUM_FEATURE_PREFIX, BOUNDARY_MISSING_LETTERS, CONTEXT_ACTION_FEATURE_VERSION,
    CONTEXT_DOUBTFUL_KEEP_PROBABILITY, CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS, CONTEXT_TYPO_MIN_CHARACTERS, CYRLETTER_FEATURE_PREFIX,
    DIGIT_DOT_FEATURE_PREFIX, KEPT_FEATURE_PREFIX, LETTER_FEATURE_PREFIX, MAX_CONTEXT_ACTION_MODEL_BYTES,
    MAX_CONTEXT_ACTION_MODEL_FEATURES, MAX_CONTEXT_ALNUM_FREQUENCY_BYTES, MAX_CONTEXT_FEATURE_NAME_CHARACTERS, MAX_CONTEXT_MODEL_BYTES,
    MAX_CONTEXT_MODEL_VERSION_CHARACTERS, MAX_CONTEXT_SHORT_FREQUENCY_BYTES, MAX_CONTEXT_TERM_FREQUENCY_BYTES,
    MAX_CONTEXT_WEIGHT_MAGNITUDE, MIN_CONTEXT_CONVERSION_THRESHOLD, RESUME_FEATURE_PREFIX, SHORT_FEATURE_PREFIX,
    VARIABLE_FEATURE_PREFIX,
};
use crate::context_action_features::{extract_action_features, FeatureMap};
use crate::flat_table::{load_artifact, load_term_frequency, CountTable, CountTables, WeightSpec, WeightTable, ACTION_COUNT};
use ks_lang::{py_sum, LanguageModel, LoadError, WordScore};
use ks_text::{pystr, ucd};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub use crate::input_context::FieldContext;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ContextAction {
    Keep,
    Convert,
    Wait,
    Suggest,
}

/// `ACTIONS`: the order of the four weights of a feature.
pub const ACTIONS: [ContextAction; ACTION_COUNT] = [ContextAction::Keep, ContextAction::Convert, ContextAction::Wait, ContextAction::Suggest];

impl ContextAction {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ContextAction::Keep => "keep",
            ContextAction::Convert => "convert",
            ContextAction::Wait => "wait",
            ContextAction::Suggest => "suggest",
        }
    }
}

/// Where the text after the word came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfterOrigin {
    None,
    Field,
    PlannedNextConversion,
    KeptNextWord,
}

impl AfterOrigin {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            AfterOrigin::None => "none",
            AfterOrigin::Field => "field",
            AfterOrigin::PlannedNextConversion => "planned_next_conversion",
            AfterOrigin::KeptNextWord => "kept_next_word",
        }
    }
}

/// How the text before a word was given: as the field holds it, or ending in the other reading of a waiting word the
/// engine asks the next word after (`engine.decide_after_waiting_word)`: a hypothesis, not text that was typed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BeforeOrigin {
    Field,
    PlannedPreviousReading,
}

impl BeforeOrigin {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            BeforeOrigin::Field => "field",
            BeforeOrigin::PlannedPreviousReading => "planned_previous_reading",
        }
    }
}

/// Characters that mark a path, an address or an identifier inside a token.
pub const TECHNICAL_MARKS: &str = "_/@\\=<>";
/// Signs stripped off both ends of a token before its frequency is looked up, as when it was counted.
pub const TERM_EDGE_SIGNS: &str = ".,!?:;\"'()[]{}<>«»-";
/// The file names of the three count tables, in the `models` folder.
pub const TERM_FREQUENCY_FILE: &str = "context-term-frequency.json";
pub const ALNUM_FREQUENCY_FILE: &str = "context-alnum-frequency.json";
pub const SHORT_FREQUENCY_FILE: &str = "context-short-frequency.json";
pub const ARTIFACT_FILE: &str = "context_policy_v1.json";
/// The schema of the frozen baseline model the sealed tests compare a candidate with.
pub const BASELINE_FEATURE_VERSION: i64 = 2;
pub const TERM_ALPHABETS: [&str; 4] = ["cyrillic", "english", "latin", "russian"];
pub const ALNUM_ALPHABETS: [&str; 2] = ["cyrillic", "latin"];
pub const SHORT_ALPHABETS: [&str; 1] = ["cyrillic"];

#[derive(Clone, Debug, PartialEq)]
pub struct ContextEvidence {
    pub original: String,
    pub alternative: String,
    pub source_group: i64,
    pub field: FieldContext,
    pub trigger: String,
    pub baseline_convert: bool,
    pub source_known: bool,
    pub target_known: bool,
    pub score_delta: f64,
    pub source_score: Option<WordScore>,
    pub target_score: Option<WordScore>,
    pub model_probability: Option<f64>,
    pub model_threshold: Option<f64>,
    pub literal_tail: String,
    pub ortho_score: Option<f64>,
    pub ortho_threshold: Option<f64>,
    pub boundary_text: String,
    pub after_origin: AfterOrigin,
    pub before_origin: BeforeOrigin,
    pub source_identifier: bool,
    pub target_identifier: bool,
    /// Whether the reading is no word itself but one typo away from one (`one_typo_from_word`).
    pub source_typo: bool,
    pub target_typo: bool,
    /// Whether the reading is a word that opens sentences of its language (`short_words.opens_sentences`).
    pub source_opening: bool,
    pub target_opening: bool,
    /// Whether letters were typed into the middle of a word whose other letters are all in the other layout
    /// (`engine.decide_inside_word)`: the question is then what the whole word is.
    pub inside: bool,
    /// A training frame whose token reads as uncounted in every term table, as a term no table holds does at serving
    /// (`context_action_features.own_bucket`); never set outside training.
    pub term_counts_dropped: bool,
}

impl ContextEvidence {
    /// The evidence with its defaults (`ContextEvidence(original, alternative, source_group, field)`).
    #[must_use]
    pub fn new(original: &str, alternative: &str, source_group: i64, field: FieldContext) -> Self {
        ContextEvidence {
            original: original.to_string(),
            alternative: alternative.to_string(),
            source_group,
            field,
            trigger: "space".to_string(),
            baseline_convert: false,
            source_known: false,
            target_known: false,
            score_delta: 0.0,
            source_score: None,
            target_score: None,
            model_probability: None,
            model_threshold: None,
            literal_tail: String::new(),
            ortho_score: None,
            ortho_threshold: None,
            boundary_text: String::new(),
            after_origin: AfterOrigin::None,
            before_origin: BeforeOrigin::Field,
            source_identifier: false,
            target_identifier: false,
            source_typo: false,
            target_typo: false,
            source_opening: false,
            target_opening: false,
            inside: false,
            term_counts_dropped: false,
        }
        .finished()
    }

    /// `__post_init__`: replacing literal field contents must not retain a stale empty flag. A planned conversion is
    /// explicit and remains subject to validation. Call it again after changing `field` or `after_origin`.
    #[must_use]
    pub fn finished(mut self) -> Self {
        if matches!(self.after_origin, AfterOrigin::None | AfterOrigin::Field) {
            self.after_origin = if self.field.after.is_empty() { AfterOrigin::None } else { AfterOrigin::Field };
        }
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContextPrediction {
    pub action: ContextAction,
    pub probability: f64,
    pub probabilities: [f64; ACTION_COUNT],
    pub model_version: String,
    pub supported: bool,
}

#[must_use]
pub fn normalized(text: &str) -> String {
    ks_text::normalize::nfc(&pystr::casefold(text))
}

/// A reading that is no word of the lexicon but one typo away from one.
///
/// One letter extra, missing or wrong, or two neighbours swapped: `фпривет` is `привет` with a stray key in front. Only
/// the frequency lexicon is read, as for the boundary model's misspellings, so the check costs dictionary lookups and
/// no spell checker calls. Short readings are not checked: almost every short string has a neighbour in a lexicon this
/// large.
#[must_use]
pub fn one_typo_from_word(text: &str, model: &LanguageModel) -> bool {
    let word = LanguageModel::normalize(text);
    if pystr::len(&word) < CONTEXT_TYPO_MIN_CHARACTERS || word != pystr::casefold(text) || !pystr::isalpha(&word) || model.frequencies.get_or_zero(&word) != 0 {
        return false;
    }
    let known = |candidate: &str| model.frequencies.get_or_zero(candidate) != 0;
    let letters: Vec<char> = BOUNDARY_MISSING_LETTERS.iter().find(|entry| entry.0 == model.locale).map_or("", |entry| entry.1).chars().collect();
    let chars: Vec<char> = word.chars().collect();
    // head + letter + tail, the letter optional.
    let assembled = |head: &[char], middle: Option<char>, tail: &[char]| -> String {
        let mut text: String = head.iter().collect();
        text.extend(middle);
        text.extend(tail);
        text
    };
    for (index, &current) in chars.iter().enumerate() {
        let (head, tail) = (&chars[..index], &chars[index + 1..]);
        if known(&assembled(head, None, tail)) {
            return true;
        }
        if let Some(&first) = tail.first() {
            // head + tail[:1] + current + tail[1:]
            let mut swapped: String = head.iter().collect();
            swapped.push(first);
            swapped.push(current);
            swapped.extend(&tail[1..]);
            if known(&swapped) {
                return true;
            }
        }
        if letters.iter().any(|&letter| letter != current && known(&assembled(head, Some(letter), tail))) {
            return true;
        }
    }
    (0..=chars.len()).any(|index| letters.iter().any(|&letter| known(&assembled(&chars[..index], Some(letter), &chars[index..]))))
}

// ---------------------------------------------------------------- the count tables

/// The endings (or beginnings) of two and three letters of the words of Russian prose, with how many words have each.
#[derive(Default)]
pub struct AffixCounts {
    entries: Vec<(String, u32)>,
}

impl AffixCounts {
    #[must_use]
    pub fn get(&self, affix: &str) -> u64 {
        self.entries.binary_search_by(|entry| entry.0.as_str().cmp(affix)).map_or(0, |found| u64::from(self.entries[found].1))
    }

    fn build(table: &CountTable, ending: bool) -> Self {
        let mut counts: HashMap<String, u32> = HashMap::new();
        for word in table.words() {
            let chars: Vec<(usize, char)> = word.char_indices().collect();
            for size in crate::constants::TERM_HEAD_ENDING_LETTERS {
                if chars.len() > *size {
                    let affix = if ending { word.get(chars[chars.len() - size].0..).unwrap_or_default() } else { word.get(..chars[*size].0).unwrap_or_default() };
                    *counts.entry(affix.to_string()).or_insert(0) += 1;
                }
            }
        }
        let mut entries: Vec<(String, u32)> = counts.into_iter().collect();
        entries.sort_unstable();
        AffixCounts { entries }
    }
}

/// How often plain words occur (schemas 7 and 3), read as a model that counts them loads: `latin` and `cyrillic` count
/// words inside Russian text, `english` and `russian` words in text of their own language. The letters-and-digits and
/// short-token tables are read on first use, as the Python reads them.
pub struct TermTables {
    dir: PathBuf,
    term: CountTables,
    alnum: OnceLock<Result<CountTables, LoadError>>,
    short: OnceLock<Result<CountTables, LoadError>>,
    endings: OnceLock<AffixCounts>,
    starts: OnceLock<AffixCounts>,
}

impl TermTables {
    /// `term_frequency()`: read at once, before the keyboard hook starts.
    pub fn load(models_dir: &Path) -> Result<TermTables, LoadError> {
        let term = load_term_frequency(&models_dir.join(TERM_FREQUENCY_FILE), &TERM_ALPHABETS, MAX_CONTEXT_TERM_FREQUENCY_BYTES)?;
        Ok(TermTables { dir: models_dir.to_path_buf(), term, alnum: OnceLock::new(), short: OnceLock::new(), endings: OnceLock::new(), starts: OnceLock::new() })
    }

    /// `term_frequency()[alphabet]`.
    pub fn term(&self, alphabet: &str) -> &CountTable {
        self.term.alphabet(alphabet)
    }

    /// `_alnum_frequency()`; a table that does not load is a `ValueError` where the Python reads it.
    pub fn alnum(&self) -> Result<&CountTables, LoadError> {
        self.alnum
            .get_or_init(|| load_term_frequency(&self.dir.join(ALNUM_FREQUENCY_FILE), &ALNUM_ALPHABETS, MAX_CONTEXT_ALNUM_FREQUENCY_BYTES))
            .as_ref()
            .map_err(Clone::clone)
    }

    /// `_short_frequency()`.
    pub fn short(&self) -> Result<&CountTables, LoadError> {
        self.short
            .get_or_init(|| load_term_frequency(&self.dir.join(SHORT_FREQUENCY_FILE), &SHORT_ALPHABETS, MAX_CONTEXT_SHORT_FREQUENCY_BYTES))
            .as_ref()
            .map_err(Clone::clone)
    }

    /// `_russian_endings()`: built on the first term the head is asked about.
    pub fn russian_endings(&self) -> &AffixCounts {
        self.endings.get_or_init(|| AffixCounts::build(self.term("russian"), true))
    }

    /// `_russian_starts()`.
    pub fn russian_starts(&self) -> &AffixCounts {
        self.starts.get_or_init(|| AffixCounts::build(self.term("russian"), false))
    }

}

/// `bisect.bisect_right(CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS, count)`.
#[must_use]
pub fn bucket_of(count: u64) -> usize {
    CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS.partition_point(|&bound| bound.to::<u64>() <= count)
}

const BUCKET_NAMES: [&str; 6] = ["0", "1", "2", "3", "4", "5"];

/// The bucket of how often a plain word occurs in the text one table counts; `na` for anything else.
pub fn term_bucket(text: &str, alphabet: &str, tables: &TermTables) -> &'static str {
    let core = pystr::casefold(pystr::strip_chars(text, TERM_EDGE_SIGNS));
    if core.is_empty() || !pystr::isalpha(&core) {
        return "na";
    }
    BUCKET_NAMES[bucket_of(tables.term(alphabet).count(&core))]
}

/// `softmax`: the sum of the exponentials is Python's `sum`.
#[must_use]
pub fn softmax(scores: &[f64]) -> [f64; ACTION_COUNT] {
    let maximum = ks_text::max_first(scores.iter().copied()).expect("four scores");
    let values: Vec<f64> = scores.iter().map(|&value| (value - maximum).exp()).collect();
    let total = py_sum(values.iter().copied());
    let mut result = [0.0; ACTION_COUNT];
    for (slot, value) in result.iter_mut().zip(&values) {
        *slot = value / total;
    }
    result
}

/// The index of the first greatest probability: `max(range(4), key=probabilities.__getitem__)`.
#[must_use]
pub fn first_greatest(probabilities: &[f64; ACTION_COUNT]) -> usize {
    let mut best = 0;
    for index in 1..ACTION_COUNT {
        if probabilities[index] > probabilities[best] {
            best = index;
        }
    }
    best
}

pub struct ContextModel {
    pub weights: WeightTable,
    pub version: String,
    pub conversion_threshold: f64,
    pub feature_version: i64,
    /// Whether the single-letter head has weights here: its class is the one place a verdict on a letter is the
    /// model's own opinion (`context_policy`).
    pub answers_letters: bool,
    /// Whether the variable head has weights here: a letter after a word of Cyrillic letters is its class.
    pub answers_variables: bool,
    /// Whether the letters-and-digits head has weights here: a token of letters and digits in both readings is its class.
    pub answers_alnum: bool,
    /// Whether the resume head has weights here: a short word right after Latin text, asked again once its next word is
    /// planned for conversion, is its class, and the engine takes such a word along in the middle of a line on its
    /// verdict (`engine.take_along`).
    pub answers_resume: bool,
    /// Whether the Cyrillic-letter head has weights here: a Latin letter whose Cyrillic reading is a Russian word of one
    /// letter, after Russian text, is its class.
    pub answers_cyrletter: bool,
    /// Whether the digit-dot head has weights here: digits with `ю` or `б` typed in the Russian layout are its class.
    pub answers_digit_dot: bool,
    pub tables: std::sync::Arc<TermTables>,
    /// Answers in place of the weights: a replay asks the model's questions of a recorded run.
    pub answer: Option<PredictAnswer>,
}

/// What stands in for `predict` when the answers come from elsewhere.
pub type PredictAnswer = std::sync::Arc<dyn Fn(&ContextEvidence) -> ContextPrediction + Send + Sync>;

impl ContextModel {
    pub fn new(weights: WeightTable, version: &str, conversion_threshold: f64, tables: std::sync::Arc<TermTables>) -> Self {
        let answers = |prefix: &str| weights.names().any(|name| name.starts_with(prefix));
        let flags = (
            answers(LETTER_FEATURE_PREFIX),
            answers(VARIABLE_FEATURE_PREFIX),
            answers(ALNUM_FEATURE_PREFIX),
            answers(RESUME_FEATURE_PREFIX),
            answers(CYRLETTER_FEATURE_PREFIX),
            answers(DIGIT_DOT_FEATURE_PREFIX),
        );
        ContextModel {
            weights,
            version: version.to_string(),
            conversion_threshold,
            feature_version: CONTEXT_ACTION_FEATURE_VERSION.to::<i64>(),
            answers_letters: flags.0,
            answers_variables: flags.1,
            answers_alnum: flags.2,
            answers_resume: flags.3,
            answers_cyrletter: flags.4,
            answers_digit_dot: flags.5,
            tables,
            answer: None,
        }
    }

    /// `ContextModel.load(path)`: `models_dir` holds the artifact and the count tables.
    pub fn load(models_dir: &Path) -> Result<ContextModel, LoadError> {
        Self::load_artifact(&models_dir.join(ARTIFACT_FILE), models_dir)
    }

    pub fn load_artifact(path: &Path, models_dir: &Path) -> Result<ContextModel, LoadError> {
        let spec = WeightSpec {
            max_entries: MAX_CONTEXT_ACTION_MODEL_FEATURES,
            max_name_characters: MAX_CONTEXT_FEATURE_NAME_CHARACTERS,
            name_required: false,
            max_magnitude: Some(MAX_CONTEXT_WEIGHT_MAGNITUDE.to::<f64>()),
            entry_error: "invalid context feature",
            weight_error: "invalid context weight",
            table_error: "invalid context weights",
        };
        let (artifact, length) = load_artifact(path, MAX_CONTEXT_ACTION_MODEL_BYTES, "context model is too large", &spec)?;
        let artifact = artifact.ok_or_else(|| LoadError::value("context model must be an object"))?;
        let payload = &artifact.payload;
        // An action model (schema 3) is written one feature to a line and may be larger; every other schema keeps its limit.
        let feature_version = integer(payload.get("feature_version"));
        if length > MAX_CONTEXT_MODEL_BYTES.to::<u64>() && feature_version != Some(CONTEXT_ACTION_FEATURE_VERSION.to::<i64>()) {
            return Err(LoadError::value("context model is too large"));
        }
        let actions_match = payload.get("actions").and_then(Value::as_array).is_some_and(|actions| {
            actions.len() == ACTIONS.len() && actions.iter().zip(ACTIONS).all(|(found, wanted)| found.as_str() == Some(wanted.as_str()))
        });
        // Schema 2 is the frozen baseline the sealed tests compare a candidate with; no such model is shipped.
        if !matches!(feature_version, Some(version) if version == CONTEXT_ACTION_FEATURE_VERSION.to::<i64>() || version == BASELINE_FEATURE_VERSION) || !actions_match {
            return Err(LoadError::value("incompatible context model"));
        }
        let feature_version = feature_version.expect("checked above");
        // The model is not whole without the table of how often each reading occurs. It is read here, before the keyboard
        // hook starts, and not on the first word typed.
        let tables = TermTables::load(models_dir)?;
        let weights = artifact.weights.ok_or_else(|| LoadError::value(spec.table_error))?;
        // Up to six feature spaces, each with its own budget in the recipe (MAX_CONTEXT_ACTION_MODEL_FEATURES).
        if weights.entries == 0 || weights.entries > spec.max_entries {
            return Err(LoadError::value(spec.table_error));
        }
        if feature_version == BASELINE_FEATURE_VERSION && weights.entries > crate::constants::MAX_CONTEXT_MODEL_FEATURES {
            return Err(LoadError::value(spec.table_error));
        }
        if let Some(message) = weights.refusal {
            return Err(LoadError::value(message));
        }
        if payload.get("weights_sha256").and_then(Value::as_str) != Some(weights.digest.as_str()) {
            return Err(LoadError::value("context model checksum mismatch"));
        }
        let version = payload.get("version").and_then(Value::as_str);
        let version = match version {
            Some(version) if version.starts_with(if feature_version == BASELINE_FEATURE_VERSION { "context-v1-" } else { "context-v3-" }) && pystr::len(version) <= MAX_CONTEXT_MODEL_VERSION_CHARACTERS => version,
            _ => return Err(LoadError::value("invalid context version")),
        };
        let threshold = match payload.get("conversion_threshold") {
            Some(Value::Number(number)) => number.as_f64(),
            _ => None,
        };
        let threshold = match threshold {
            Some(threshold) if (MIN_CONTEXT_CONVERSION_THRESHOLD..=1.0).contains(&threshold) => threshold,
            _ => return Err(LoadError::value("unsafe context threshold")),
        };
        let table = weights.table;
        if table.names().any(|name| name.starts_with(SHORT_FEATURE_PREFIX)) {
            // The short-token head asks its table about many a short word typed after Russian text: it is read here,
            // before the keyboard hook starts, as the term table is.
            tables.short()?;
        }
        let mut model = ContextModel::new(table, version, threshold, std::sync::Arc::new(tables));
        model.feature_version = feature_version;
        Ok(model)
    }

    /// `ContextModel.try_load()`: the model and its version, or nothing and why.
    #[must_use]
    pub fn try_load(models_dir: &Path) -> (Option<ContextModel>, String) {
        match Self::load(models_dir) {
            Ok(model) => {
                let version = model.version.clone();
                (Some(model), version)
            }
            Err(error) => (None, error.to_string()),
        }
    }

    /// Use the same language-support gate during calibration and inference.
    #[must_use]
    pub fn supports_features(&self, features: &FeatureMap) -> bool {
        if self.feature_version == BASELINE_FEATURE_VERSION {
            return features.keys().any(|name| (name.starts_with("pair:") || name.starts_with("before:word:") || name.starts_with("after:word:") || name.starts_with("app:")) && self.weights.get(name).is_some());
        }
        let text = ACTION_FEATURE_CHARACTER_TEXT_FIELD_INDEX;
        // The kept-neighbour question names its letters under its own prefix, and the single-letter head once more under
        // its own: the kept-neighbour frames hold no word of one letter, so only the head knows the letters of `b` before
        // a kept `redis`. In the digit-dot head's class a reading with no letter has none to know: `2.`, the other
        // reading of `2ю`, left the head's every conversion a suggestion (candidate v81 e5). Elsewhere a reading with no
        // letter stays unsupported: let through, `,` and `.` typed as meant became `б` and `ю` (candidate v81 e6).
        let signs_only = features.keys().any(|name| name.starts_with(DIGIT_DOT_FEATURE_PREFIX));
        ["source", "target"].iter().all(|label| {
            let mut lettered = features.keys().filter(|name| {
                let bare = name.strip_prefix(LETTER_FEATURE_PREFIX).unwrap_or(name);
                let bare = bare.strip_prefix(KEPT_FEATURE_PREFIX).unwrap_or(bare);
                bare.strip_prefix(label).is_some_and(|rest| rest.starts_with(":char:"))
                    && bare.splitn(text + 1, ':').nth(text).is_some_and(|gram| gram.chars().any(ucd::isalpha))
            });
            if signs_only {
                // `(signs_only and not names) or any(name in weights)`: no name at all, or one the weights hold.
                let first = lettered.next();
                first.is_none() || std::iter::once(first).flatten().chain(lettered).any(|name| self.weights.get(name).is_some())
            } else {
                lettered.any(|name| self.weights.get(name).is_some())
            }
        })
    }


    fn unsupported(&self) -> ContextPrediction {
        ContextPrediction { action: ContextAction::Suggest, probability: 0.0, probabilities: [0.0, 0.0, 0.0, 1.0], model_version: self.version.clone(), supported: false }
    }

    /// The frozen baseline of the sealed tests answers on features of its own, with no check beyond its threshold.
    fn predict_baseline(&self, item: &ContextEvidence) -> ContextPrediction {
        let features = crate::context_action_features::extract_baseline_features(item);
        let mut scores = [0.0_f64; ACTION_COUNT];
        for (name, &value) in &features {
            if let Some(weights) = self.weights.get(name) {
                for (index, &weight) in weights.iter().enumerate() {
                    scores[index] += weight * value;
                }
            }
        }
        let probabilities = softmax(&scores);
        let selected = first_greatest(&probabilities);
        let mut action = ACTIONS[selected];
        if action == ContextAction::Convert && probabilities[selected] < self.conversion_threshold {
            action = ContextAction::Suggest;
        }
        ContextPrediction { action, probability: probabilities[selected], probabilities, model_version: self.version.clone(), supported: self.supports_features(&features) }
    }

    #[must_use]
    pub fn predict(&self, item: &ContextEvidence) -> ContextPrediction {
        if let Some(answer) = &self.answer {
            return answer(item);
        }
        if self.feature_version == BASELINE_FEATURE_VERSION {
            return self.predict_baseline(item);
        }
        let Ok(features) = extract_action_features(item, &self.tables) else {
            return self.unsupported();
        };
        let mut scores = [0.0_f64; ACTION_COUNT];
        for (name, &value) in &features {
            if let Some(weights) = self.weights.get(name) {
                for (index, &weight) in weights.iter().enumerate() {
                    scores[index] += weight * value;
                }
            }
        }
        self.prediction_of(&scores, &features)
    }

    /// The answer of the scores; scores that are no numbers are no answer.
    #[must_use]
    pub fn prediction_of(&self, scores: &[f64; ACTION_COUNT], features: &FeatureMap) -> ContextPrediction {
        if !scores.iter().all(|score| score.is_finite()) {
            return self.unsupported();
        }
        let probabilities = softmax(scores);
        let selected = first_greatest(&probabilities);
        let mut action = ACTIONS[selected];
        if action == ContextAction::Convert && probabilities[selected] < self.conversion_threshold {
            action = ContextAction::Suggest;
        }
        if action == ContextAction::Keep && probabilities[1] >= CONTEXT_DOUBTFUL_KEEP_PROBABILITY {
            // A keep the model is unsure of, as sure as not of the other layout, is a suggestion too: nothing is
            // converted, and at a space the engine waits for the next word and asks again with it, the question the
            // model is trained on (the kept-neighbour question). `зк` after `принимай ` was kept at p=0.51 against 0.49
            // for `pr` and stayed; asked beside `и` the same model converts it at p=1.00.
            action = ContextAction::Suggest;
        }
        let supported = self.supports_features(features);
        if !supported && matches!(action, ContextAction::Keep | ContextAction::Convert) {
            action = ContextAction::Suggest;
        }
        ContextPrediction { action, probability: probabilities[selected], probabilities, model_version: self.version.clone(), supported }
    }
}

fn integer(value: Option<&Value>) -> Option<i64> {
    match value {
        Some(Value::Number(number)) => number.as_i64(),
        _ => None,
    }
}
