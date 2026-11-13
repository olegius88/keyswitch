//! Where the language layer finds its files: one `resources` folder given at load time, the `KEYSWITCH_MODEL_PATH`
//! override, and a speller chosen by the app. Holds what the Python keeps in `lru_cache`s: the engine's models and
//! the shared lexicons are built once.

use ks_cast::To as _;
use crate::boundary_model::{self, BoundaryModel};
use crate::boundary_policy::{self, BoundaryPolicy};
use crate::detector::{LanguageDetector, LanguageScorer, ProtectedTokens};
use crate::early_switch::PrefixIndex;
use crate::identifier_lexicon::{self, IdentifierLexicon};
use crate::intent_model::{IntentModelStatus, LinearNgramModel};
use crate::language_model::{model_roots, LanguageModel};
use crate::lexicon_supplement;
use crate::load_error::LoadError;
use crate::ortho_model::{artifact_path, Identifier, KnownWord, OrthoModel, LOCALES};
use crate::spellcheck::{DisabledSpellChecker, SpellChecker};
use crate::tables::{WordList, WordTable};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// Opens the speller of a locale: `HunspellDictionary(locale)`.
pub type SpellerFactory = Box<dyn Fn(&str) -> Box<dyn SpellChecker> + Send + Sync>;

/// The indexes built, by locale and lexicon source.
type PrefixIndexes = Vec<((String, String), Arc<PrefixIndex>)>;

pub struct Resources {
    resource_dir: PathBuf,
    spellers: SpellerFactory,
    /// One dictionary of the speller is opened at a time. Two were never opened at once before the dictionaries of a
    /// start were read side by side, and whether the library behind the speller takes two openings at once is not
    /// written down anywhere; opening one takes milliseconds, so they wait for each other here.
    opening_speller: Mutex<()>,
    /// One place per locale, filled by whoever reads that locale first; the others wait for it there and take what it
    /// read. The list is locked only to find the place, never while a model is read, so the dictionaries of a start are
    /// read side by side (`KeySwitchEngine::new`).
    engine_models: Mutex<Vec<(String, Arc<OnceLock<Arc<LanguageModel>>>)>>,
    plain_frequencies: Mutex<Vec<(String, Arc<WordTable>)>>,
    prefix_indexes: Mutex<PrefixIndexes>,
    identifiers: OnceLock<(Option<Arc<IdentifierLexicon>>, String)>,
    boundary_policy: OnceLock<Option<Arc<BoundaryPolicy>>>,
    boundary_model: OnceLock<Option<Arc<BoundaryModel>>>,
    protected_tokens: OnceLock<Arc<ProtectedTokens>>,
    intent_model: OnceLock<(Option<Arc<LinearNgramModel>>, IntentModelStatus)>,
}

impl Resources {
    /// The `resources` folder of the install, and no speller: what a build without Hunspell runs with.
    pub fn new(resource_dir: impl Into<PathBuf>) -> Arc<Self> {
        Self::with_speller(resource_dir, Box::new(|_locale| Box::new(DisabledSpellChecker)))
    }

    pub fn with_speller(resource_dir: impl Into<PathBuf>, spellers: SpellerFactory) -> Arc<Self> {
        Arc::new(Resources {
            resource_dir: resource_dir.into(),
            spellers,
            opening_speller: Mutex::new(()),
            engine_models: Mutex::new(Vec::new()),
            plain_frequencies: Mutex::new(Vec::new()),
            prefix_indexes: Mutex::new(Vec::new()),
            identifiers: OnceLock::new(),
            boundary_policy: OnceLock::new(),
            boundary_model: OnceLock::new(),
            protected_tokens: OnceLock::new(),
            intent_model: OnceLock::new(),
        })
    }

    pub fn resource_dir(&self) -> &Path {
        &self.resource_dir
    }

    /// `model_roots()`.
    pub fn model_roots(&self) -> Vec<PathBuf> {
        model_roots(&self.resource_dir)
    }

    /// `supplement_words(locale)`, read anew: only the models keep its words.
    pub fn supplement_words(&self, locale: &str) -> Result<WordList, LoadError> {
        lexicon_supplement::supplement_words(&self.resource_dir, locale)
    }

    /// `LanguageModel.load(locale, extra_words)`, not cached: the engine's models come from `engine_language_model`.
    pub fn language_model<S: AsRef<str>, I: IntoIterator<Item = S>>(&self, locale: &str, extra_words: I) -> LanguageModel {
        let speller = {
            let _one_at_a_time = self.opening_speller.lock().expect("the lock is not poisoned");
            (self.spellers)(locale)
        };
        LanguageModel::load(locale, extra_words, &self.model_roots(), speller)
    }

    /// `LanguageModel.load(locale, supplement_words(locale))`, built once per locale. A malformed supplement is an error.
    ///
    /// The list of models is locked only to find this locale's place in it, never while the model is read, so two
    /// locales are read side by side; locked for the read, the second waited for the first and a start took the two in
    /// turn. Two reads of one locale take the same place: the second waits there and takes what the first read.
    pub fn engine_language_model(&self, locale: &str) -> Result<Arc<LanguageModel>, LoadError> {
        let place = self.model_place(locale);
        if let Some(found) = place.get() {
            return Ok(found.clone());
        }
        let words = self.supplement_words(locale)?;
        Ok(place.get_or_init(|| Arc::new(self.language_model(locale, words.iter()))).clone())
    }

    fn model_place(&self, locale: &str) -> Arc<OnceLock<Arc<LanguageModel>>> {
        let mut models = self.engine_models.lock().expect("the lock is not poisoned");
        if let Some(found) = models.iter().find(|entry| entry.0 == locale) {
            return found.1.clone();
        }
        let place: Arc<OnceLock<Arc<LanguageModel>>> = Arc::new(OnceLock::new());
        models.push((locale.to_string(), place.clone()));
        place
    }

    /// The frequencies of `LanguageModel.load(locale)`: the file and the fallback words, no supplement.
    fn plain_frequencies(&self, locale: &str) -> Arc<WordTable> {
        let mut tables = self.plain_frequencies.lock().expect("the lock is not poisoned");
        if let Some(found) = tables.iter().find(|entry| entry.0 == locale) {
            return found.1.clone();
        }
        let table = Arc::new(LanguageModel::load_tables(locale, std::iter::empty::<&str>(), &self.model_roots()).frequencies);
        tables.push((locale.to_string(), table.clone()));
        table
    }

    /// `PrefixIndex.for_language_model(model)`: built once per lexicon source, from the lexicon plus Hunspell stems.
    /// The lexicon is that of `LanguageModel.load(locale)`, without the supplement, whatever the model holds: the
    /// Python does the same (a replay passes its reference model's frequencies to `PrefixIndex::from_lexicon`).
    pub fn prefix_index(&self, model: &LanguageModel) -> Arc<PrefixIndex> {
        let key = (model.locale.clone(), model.source.clone());
        let mut indexes = self.prefix_indexes.lock().expect("the lock is not poisoned");
        if let Some(found) = indexes.iter().find(|entry| entry.0 == key) {
            return found.1.clone();
        }
        let index = Arc::new(PrefixIndex::from_lexicon(self.plain_frequencies(&model.locale), &model.source));
        indexes.push((key, index.clone()));
        index
    }

    /// `IdentifierLexicon.try_load()`: the packaged lexicon and its version, or nothing and why.
    pub fn identifier_lexicon(&self) -> (Option<Arc<IdentifierLexicon>>, String) {
        self.identifiers
            .get_or_init(|| {
                let (lexicon, status) = IdentifierLexicon::try_load(&identifier_lexicon::resource_path(&self.resource_dir));
                (lexicon.map(Arc::new), status)
            })
            .clone()
    }

    /// `ortho_model._packaged_identifier()`.
    pub fn packaged_identifier(&self) -> Identifier {
        match self.identifier_lexicon().0 {
            Some(lexicon) => Arc::new(move |token| lexicon.contains(token)),
            None => Arc::new(|_token| false),
        }
    }

    /// `ortho_model._engine_dictionaries()`: `known` as the engine answers it, for a caller that does not pass the
    /// engine's models. A malformed supplement stops the program, as the exception does in the Python.
    pub fn engine_dictionaries(&self) -> KnownWord {
        let model = |locale: &str| self.engine_language_model(locale).expect("the packaged lexicon supplement is well formed");
        let (en, ru) = (model(LOCALES[0].1), model(LOCALES[1].1));
        Arc::new(move |script, word| if script == "en" { en.score(word).known } else { ru.score(word).known })
    }

    /// `OrthoModel.try_load()`: the model and its version, or nothing and why. Not cached: `ContextPolicy` keeps it.
    pub fn ortho_model(self: &Arc<Self>) -> (Option<OrthoModel>, String) {
        OrthoModel::try_load(&artifact_path(&self.resource_dir), Some(self.clone()))
    }

    /// `BoundaryPolicy.default()`.
    pub fn boundary_policy(&self) -> Option<Arc<BoundaryPolicy>> {
        self.boundary_policy.get_or_init(|| BoundaryPolicy::load(&boundary_policy::artifact(&self.resource_dir)).ok().map(Arc::new)).clone()
    }

    /// `BoundaryModel.default()`.
    pub fn boundary_model(&self) -> Option<Arc<BoundaryModel>> {
        self.boundary_model.get_or_init(|| BoundaryModel::load(&boundary_model::artifact(&self.resource_dir)).ok().map(Arc::new)).clone()
    }

    /// `detector.PROTECTED_TOKENS`: read once.
    pub fn protected_tokens(&self) -> Arc<ProtectedTokens> {
        self.protected_tokens.get_or_init(|| Arc::new(ProtectedTokens::load(&self.resource_dir))).clone()
    }

    /// `LinearNgramModel.try_load_default()`, read once: the model of `KEYSWITCH_INTENT_MODEL_PATH` or the packaged one,
    /// and the status either way.
    pub fn intent_model(&self) -> (Option<Arc<LinearNgramModel>>, IntentModelStatus) {
        self.intent_model
            .get_or_init(|| {
                let value = std::env::var_os("KEYSWITCH_INTENT_MODEL_PATH").map(|value| value.to_string_lossy().into_owned());
                let (model, status) = LinearNgramModel::try_load_default(&self.resource_dir, value.as_deref());
                (model.map(Arc::new), status)
            })
            .clone()
    }

    /// `LanguageDetector({index: LanguageModel.load(locale, supplement_words(locale))}, intent_model)` of the engine:
    /// group `index` is the model of `locales[index]`.
    pub fn language_detector(&self, locales: &[&str]) -> Result<LanguageDetector, LoadError> {
        let mut models: Vec<(i64, Arc<dyn LanguageScorer>)> = Vec::new();
        for (index, locale) in locales.iter().take(crate::constants::LAYOUT_GROUP_COUNT).enumerate() {
            models.push((index.to::<i64>(), self.engine_language_model(locale)?));
        }
        let intent: Option<Arc<dyn crate::detector::IntentClassifier>> = self.intent_model().0.map(|model| -> Arc<dyn crate::detector::IntentClassifier> { model });
        LanguageDetector::new(models, intent, self.protected_tokens())
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn what_was_read_once_is_kept() {
        let shipped = Resources::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources"));
        let first = shipped.plain_frequencies("en_US");
        assert!(Arc::ptr_eq(&first, &shipped.plain_frequencies("en_US")));
    }
}
