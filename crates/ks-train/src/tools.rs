//! What every curriculum reads: the tree's files, the term tables, the portable lexicons and the answers built on them
//! (`context_deferral`, `reference_lexicon`).
use ks_cast::To as _;
use crate::corpus::{checksum, read_json};
use crate::physical::{expanded_aliases, translated};
use ks_context::context_action_features as features;
use ks_context::context_model::{TermTables, term_bucket};
use ks_lang::identifier_lexicon::IdentifierLexicon;
use ks_lang::{LanguageModel, Resources, WordList};
use ks_text::pystr;
use serde_json::Value;
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

const ACTION_SHORT_WORD_MAX_CHARACTERS: usize = 2;
const ACTION_DEFERRED_WORD_MAX_CHARACTERS: usize = 3;
pub const LOCALES: [&str; 2] = ["en_US", "ru_RU"];

pub type Lexicons = Vec<Arc<LanguageModel>>;

pub struct Tools {
    /// The tree that holds `model/` and `resources/`.
    pub root: PathBuf,
    /// The packaged resources, with Hunspell: what the engine's own dictionaries are.
    pub resources: Arc<Resources>,
    pub tables: Arc<TermTables>,
    pub identifiers: Option<Arc<IdentifierLexicon>>,
    /// `reference_models(False)`.
    pub portable: Lexicons,
    pub refused: BTreeSet<String>,
    onboard_capitals: OnceLock<HashSet<String>>,
}

impl Tools {
    pub fn new(root: &Path, resources: Arc<Resources>, portable: Lexicons, refused: BTreeSet<String>) -> Result<Tools, String> {
        let tables = TermTables::load(&resources.resource_dir().join("models")).map_err(|error| error.name.to_string())?;
        let identifiers = resources.identifier_lexicon().0;
        Ok(Tools { root: root.to_path_buf(), resources, tables: Arc::new(tables), identifiers, portable, refused, onboard_capitals: OnceLock::new() })
    }

    pub fn term_bucket(&self, text: &str, alphabet: &str) -> &'static str {
        term_bucket(text, alphabet, &self.tables)
    }

    pub fn count(&self, alphabet: &str, word: &str) -> u64 {
        self.tables.term(alphabet).count(word)
    }

    pub fn attested_abbreviation(&self, original: &str) -> bool {
        features::attested_abbreviation(original, &self.tables)
    }

    /// `plausible_reading`: a known word of its layout or an identifier.
    pub fn plausible_reading(&self, text: &str, group: i64) -> bool {
        if self.portable[group.to::<usize>()].score(text).known {
            return true;
        }
        self.identifiers.as_ref().is_some_and(|identifiers| identifiers.contains(text))
    }

    /// `deferred_isolated`: whether an isolated reading has no observable intent label and is deferred.
    pub fn deferred_isolated(&self, original: &str, alternate: &str, group: i64) -> bool {
        let length = pystr::len(original);
        if length > 0 && length <= ACTION_SHORT_WORD_MAX_CHARACTERS {
            return true;
        }
        if length > ACTION_SHORT_WORD_MAX_CHARACTERS && length <= ACTION_DEFERRED_WORD_MAX_CHARACTERS {
            return self.plausible_reading(original, group) && self.plausible_reading(alternate, 1 - group);
        }
        false
    }

    /// `lookahead_focus`: whether a word is one the engine may hold for its next word.
    pub fn lookahead_focus(&self, original: &str, group: i64) -> bool {
        match translated(original, group) {
            Ok(alternate) => self.deferred_isolated(original, &alternate, group),
            Err(_) => false,
        }
    }

    /// `counted_reading`: the layout group a lone token's counts settle, or None when they settle nothing.
    pub fn counted_reading(&self, original: &str, alternate: &str, group: i64, minimum: u64, other_maximum: u64, dominance: f64) -> Option<i64> {
        let (latin, cyrillic) = if group == 0 { (original, alternate) } else { (alternate, original) };
        let (latin, cyrillic) = (pystr::casefold(latin), pystr::casefold(cyrillic));
        let counts = [self.count("latin", &latin), self.count("cyrillic", &cyrillic).max(self.count("russian", &cyrillic))];
        for strong in 0..2_usize {
            let weak = 1 - strong;
            let weak_word = if weak == 0 { &latin } else { &cyrillic };
            if counts[strong] >= minimum
                && counts[strong].to::<f64>() >= dominance * counts[weak].to::<f64>()
                && counts[weak] < other_maximum
                && (!self.portable[weak].score(weak_word).known || (strong == 1 && features::brief_dominant(&latin, &cyrillic, &self.tables)))
            {
                return Some(strong.to::<i64>());
            }
        }
        None
    }

    /// `refused & (expanded_aliases(a) | expanded_aliases(b))`.
    pub fn is_refused(&self, forms: &[&str]) -> bool {
        forms.iter().any(|form| expanded_aliases(form).iter().any(|alias| self.refused.contains(alias)))
    }

    /// `onboard_capital_forms`: Russian forms the onboard lexicon writes in capitals, casefolded.
    pub fn onboard_capitals(&self) -> &HashSet<String> {
        self.onboard_capitals.get_or_init(|| {
            let text = std::fs::read_to_string(self.root.join("model/intent_v1/sources/ru_RU.lm")).expect("the pinned Russian lexicon");
            let mut forms = HashSet::new();
            let mut unigrams = false;
            for line in text.lines() {
                let line = pystr::strip(line);
                if line.starts_with('\\') {
                    unigrams = line == "\\1-grams:";
                    continue;
                }
                if !unigrams {
                    continue;
                }
                let parts = pystr::split(line);
                if parts.len() > 1 && pystr::len(parts[1]) > 1 && pystr::isalpha(parts[1]) && pystr::isupper(parts[1]) {
                    forms.insert(pystr::casefold(parts[1]));
                }
            }
            forms
        })
    }
}

/// `reference_models(spelling)`: the pinned onboard lexicons plus the packaged supplements, as the engine loads them;
/// with `spelling`, the reference morphology, whose bytes the config pins.
pub fn reference_models(root: &Path, resources: &Resources, spelling: bool) -> Result<Lexicons, String> {
    let config = read_json(&root.join("model/intent_v1/config.json"))?;
    let languages = &config["sources"]["languages"];
    let dictionaries = &config["external_evaluation"]["hunspell"];
    let data_dir = ks_config::history::data_dir();
    let mut models = Vec::new();
    for locale in LOCALES {
        let spec = &languages[locale];
        let path = root.join(spec["path"].as_str().ok_or("a lexicon without a path")?);
        if Value::String(checksum(&path)?) != spec["sha256"] {
            return Err("reference lexicon checksum mismatch".to_owned());
        }
        let words = resources.supplement_words(locale).map_err(|error| error.name.to_string())?;
        let directory = path.parent().ok_or("a lexicon outside a folder")?.to_path_buf();
        let speller: Box<dyn ks_lang::SpellChecker> =
            if spelling { Box::new(ks_hunspell::HunspellDictionary::new(locale, &data_dir)) } else { LanguageModel::disabled_speller() };
        let model = LanguageModel::load(locale, words_iter(&words), &[directory], speller);
        if spelling {
            let dictionary = PathBuf::from(model.speller.source());
            if !model.speller.available()
                || Value::String(checksum(&dictionary)?) != dictionaries[locale]["dictionary_sha256"]
                || Value::String(checksum(&dictionary.with_extension("aff"))?) != dictionaries[locale]["affix_sha256"]
            {
                return Err("reference morphology unavailable or changed".to_owned());
            }
        }
        models.push(Arc::new(model));
    }
    Ok(models)
}

fn words_iter(words: &WordList) -> impl Iterator<Item = &str> {
    words.iter()
}

/// Reading a recipe's options as the trainer does: `int(options[name])`, `float(options[name])`; a key that is missing
/// stops the fit.
pub trait Options {
    fn int(&self, name: &str) -> i64;
    fn float(&self, name: &str) -> f64;
    fn int_or(&self, name: &str, default: i64) -> i64;
    fn float_or(&self, name: &str, default: f64) -> f64;
    fn is_true(&self, name: &str) -> bool;
    /// `frozenset(str(token).casefold() for token in options.get("held_out", []))`.
    fn held_out(&self) -> HashSet<String>;
    fn budgets(&self, name: &str) -> Vec<(usize, usize)>;
}

fn number(value: &Value, name: &str) -> f64 {
    value.as_f64().unwrap_or_else(|| panic!("the recipe's {name} is no number"))
}

impl Options for Value {
    fn int(&self, name: &str) -> i64 {
        let value = self.get(name).unwrap_or_else(|| panic!("the recipe has no {name}"));
        value.as_i64().unwrap_or_else(|| number(value, name).to::<i64>())
    }

    fn float(&self, name: &str) -> f64 {
        number(self.get(name).unwrap_or_else(|| panic!("the recipe has no {name}")), name)
    }

    fn int_or(&self, name: &str, default: i64) -> i64 {
        if self.get(name).is_some() { self.int(name) } else { default }
    }

    fn float_or(&self, name: &str, default: f64) -> f64 {
        if self.get(name).is_some() { self.float(name) } else { default }
    }

    fn is_true(&self, name: &str) -> bool {
        self.get(name).and_then(Value::as_bool) == Some(true)
    }

    fn held_out(&self) -> HashSet<String> {
        let mut result = HashSet::new();
        for token in self.get("held_out").and_then(Value::as_array).into_iter().flatten() {
            result.insert(pystr::casefold(token.as_str().expect("a held-out token")));
        }
        result
    }

    fn budgets(&self, name: &str) -> Vec<(usize, usize)> {
        let table = self.get(name).and_then(Value::as_object).unwrap_or_else(|| panic!("the recipe has no {name}"));
        let mut budgets: Vec<(usize, usize)> =
            table.iter().map(|(length, count)| (length.parse().expect("a length"), count.as_u64().expect("a count").to::<usize>())).collect();
        budgets.sort_unstable();
        budgets
    }
}
