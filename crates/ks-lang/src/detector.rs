//! Precision-first ensemble for automatic keyboard-layout correction.
//!
//! Groups, alternatives and the previous words are what the Python keeps in `dict[int, str]`: here a slice of
//! `(group, text)` pairs in the dict's order (the caller keeps the keys unique), because the order decides ties.

use crate::constants::*;
use crate::intent_model::{normalize_token, CorrectionTrigger, IntentModelInput, LinearNgramModel, LinearPrediction};
use crate::language_model::{LanguageModel, WordScore};
use crate::load_error::LoadError;
use crate::script_names::SCRIPT_RUNS;
use ks_text::{max_by_key_first, pystr, ucd};
use std::collections::HashSet;
use std::fmt;
use std::path::Path;
use std::sync::Arc;

/// An exception the detector raises: `KeyError` for a group without a model, `ValueError` from the intent model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetectorError {
    pub name: &'static str,
    pub message: String,
}

impl fmt::Display for DetectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for DetectorError {}

impl From<LoadError> for DetectorError {
    fn from(error: LoadError) -> Self {
        DetectorError { name: error.name, message: error.message }
    }
}

fn key_error(group: i64) -> DetectorError {
    DetectorError { name: "KeyError", message: group.to_string() }
}

/// The tokens that are never converted: `resources/protected_tokens.txt`, case-folded. An unreadable file is no tokens.
#[derive(Clone, Debug, Default)]
pub struct ProtectedTokens {
    tokens: HashSet<String>,
}

impl ProtectedTokens {
    /// The tokens, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.tokens.iter().map(String::as_str)
    }

    /// `_load_protected_tokens()`.
    pub fn load(resource_dir: &Path) -> ProtectedTokens {
        let Ok(bytes) = std::fs::read(resource_dir.join("protected_tokens.txt")) else {
            return ProtectedTokens::default();
        };
        // `read_text(encoding="utf-8")`: a decoding error ends the Python's import, here it reads as no tokens.
        let Ok(text) = String::from_utf8(bytes) else {
            return ProtectedTokens::default();
        };
        ProtectedTokens::from_text(&text)
    }

    /// The lines of a file, as `read_text().splitlines()` sees them.
    pub fn from_text(text: &str) -> ProtectedTokens {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let tokens = pystr::splitlines(&text, false)
            .into_iter()
            .filter(|line| !pystr::strip(line).is_empty() && !pystr::lstrip(line).starts_with('#'))
            .map(|line| pystr::casefold(pystr::strip(line)))
            .collect();
        ProtectedTokens { tokens }
    }

    pub fn contains(&self, lowered: &str) -> bool {
        self.tokens.contains(lowered)
    }

    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    /// The tokens, sorted.
    pub fn sorted(&self) -> Vec<&str> {
        let mut tokens: Vec<&str> = self.tokens.iter().map(String::as_str).collect();
        tokens.sort_unstable();
        tokens
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DetectionDecision {
    pub should_convert: bool,
    pub original: String,
    pub replacement: String,
    pub source_group: i64,
    pub target_group: i64,
    pub confidence: f64,
    pub reason: String,
    pub source_score: WordScore,
    pub target_score: WordScore,
    pub model_probability: Option<f64>,
    pub model_threshold: Option<f64>,
    pub model_version: String,
}

impl DetectionDecision {
    /// `DetectionDecision(should, original, replacement, source, target, confidence, reason, source_score, target_score)`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        should_convert: bool,
        original: &str,
        replacement: &str,
        source_group: i64,
        target_group: i64,
        confidence: f64,
        reason: &str,
        source_score: WordScore,
        target_score: WordScore,
    ) -> Self {
        DetectionDecision {
            should_convert,
            original: original.to_string(),
            replacement: replacement.to_string(),
            source_group,
            target_group,
            confidence,
            reason: reason.to_string(),
            source_score,
            target_score,
            model_probability: None,
            model_threshold: None,
            model_version: String::new(),
        }
    }

    fn with_prediction(mut self, prediction: &Option<LinearPrediction>) -> Self {
        if let Some(prediction) = prediction {
            self.model_probability = Some(prediction.probability);
            self.model_threshold = Some(prediction.threshold);
            self.model_version = prediction.model_version.clone();
        }
        self
    }
}

/// Structural contract consumed by the detector.
pub trait LanguageScorer: Send + Sync {
    fn score(&self, word: &str) -> WordScore;
    fn context_score(&self, previous: &str, word: &str) -> f64;
    fn best_single_deletion(&self, word: &str) -> WordScore;
    /// `isinstance(model, LanguageModel)`, with the model.
    fn as_language_model(&self) -> Option<&LanguageModel> {
        None
    }
}

impl LanguageScorer for LanguageModel {
    fn score(&self, word: &str) -> WordScore {
        LanguageModel::score(self, word)
    }

    fn context_score(&self, previous: &str, word: &str) -> f64 {
        LanguageModel::context_score(self, previous, word)
    }

    fn best_single_deletion(&self, word: &str) -> WordScore {
        LanguageModel::best_single_deletion(self, word)
    }

    fn as_language_model(&self) -> Option<&LanguageModel> {
        Some(self)
    }
}

/// Minimal inference contract used by the policy layer.
pub trait IntentClassifier: Send + Sync {
    fn veto_threshold(&self) -> f64;
    fn predict(&self, item: &IntentModelInput<'_>) -> Result<LinearPrediction, LoadError>;
}

impl IntentClassifier for LinearNgramModel {
    fn veto_threshold(&self) -> f64 {
        self.veto_threshold
    }

    fn predict(&self, item: &IntentModelInput<'_>) -> Result<LinearPrediction, LoadError> {
        LinearNgramModel::predict(self, item)
    }
}

/// The ordered `dict[int, str]` the detector is asked about.
pub type GroupTexts<'a> = &'a [(i64, String)];

fn text_of<'a>(texts: GroupTexts<'a>, group: i64) -> Option<&'a str> {
    texts.iter().find(|item| item.0 == group).map(|item| item.1.as_str())
}

/// The keyword arguments of `LanguageDetector.decide`, at the Python's defaults.
#[derive(Clone, Copy, Debug)]
pub struct DecideOptions<'a> {
    pub minimum_length: i64,
    pub confidence_threshold: f64,
    pub ignored_words: &'a [String],
    pub aggressive: bool,
    pub protect_code: bool,
    pub previous_words: GroupTexts<'a>,
    pub context_group: Option<i64>,
    pub forced_target_group: Option<i64>,
    pub rejected_targets: &'a [i64],
    pub trigger: CorrectionTrigger,
    pub use_intent_model: bool,
}

impl Default for DecideOptions<'_> {
    fn default() -> Self {
        DecideOptions {
            minimum_length: DEFAULT_MINIMUM_WORD_LENGTH,
            confidence_threshold: DEFAULT_CONFIDENCE_THRESHOLD,
            ignored_words: &[],
            aggressive: false,
            protect_code: true,
            previous_words: &[],
            context_group: None,
            forced_target_group: None,
            rejected_targets: &[],
            trigger: CorrectionTrigger::Space,
            use_intent_model: true,
        }
    }
}

/// Fuse lexicons, morphology, character statistics and recent context.
///
/// False corrections are deliberately more expensive than missed ones: an ambiguous token stays untouched and is
/// always available to the manual conversion hotkey. Explicit learned rules are evaluated before statistical
/// heuristics.
pub struct LanguageDetector {
    /// `models`, in the order given (the dict's order).
    pub models: Vec<(i64, Arc<dyn LanguageScorer>)>,
    pub intent_model: Option<Arc<dyn IntentClassifier>>,
    pub protected_tokens: Arc<ProtectedTokens>,
}

/// The scored candidate: `(delta, group, candidate, target_score, context_delta)`.
struct Scored {
    delta: f64,
    group: i64,
    candidate: String,
    target_score: WordScore,
    context_delta: f64,
}

impl LanguageDetector {
    pub fn new(models: Vec<(i64, Arc<dyn LanguageScorer>)>, intent_model: Option<Arc<dyn IntentClassifier>>, protected_tokens: Arc<ProtectedTokens>) -> Result<Self, LoadError> {
        if models.len() < LAYOUT_GROUP_COUNT {
            return Err(LoadError::value("At least two language models are required"));
        }
        Ok(LanguageDetector { models, intent_model, protected_tokens })
    }

    /// `self.models.get(group)`.
    pub fn model(&self, group: i64) -> Option<&Arc<dyn LanguageScorer>> {
        self.models.iter().find(|item| item.0 == group).map(|item| &item.1)
    }

    /// `self.models[group]`.
    pub fn model_for(&self, group: i64) -> Result<&Arc<dyn LanguageScorer>, DetectorError> {
        self.model(group).ok_or_else(|| key_error(group))
    }

    pub fn decide(&self, original: &str, alternatives: GroupTexts<'_>, source_group: i64, options: &DecideOptions<'_>) -> Result<DetectionDecision, DetectorError> {
        let DecideOptions { minimum_length, confidence_threshold, aggressive, protect_code, context_group, .. } = *options;
        let ignored: HashSet<String> = options.ignored_words.iter().map(|word| Self::token_key(word)).collect();
        let source_model = self.model_for(source_group)?;
        let source_score = source_model.score(original);
        let rejected = |reason: &str| DetectionDecision::new(false, original, original, source_group, source_group, 0.0, reason, source_score.clone(), source_score.clone());
        let mut scored: Vec<Scored> = Vec::new();
        for (group, candidate) in alternatives {
            let Some(target_model) = self.model(*group) else {
                continue;
            };
            if *group == source_group || candidate == original {
                continue;
            }
            let target_score = target_model.score(candidate);
            let mut delta = target_score.value - source_score.value;
            let context_delta = self.context_delta(source_group, *group, original, candidate, options.previous_words, context_group)?;
            delta += context_delta;
            scored.push(Scored { delta, group: *group, candidate: candidate.clone(), target_score, context_delta });
        }
        if scored.is_empty() {
            return Ok(rejected("нет другой раскладки"));
        }

        let best = max_by_key_first(scored.iter().enumerate(), |item| item.1.delta).expect("scored is not empty").0;
        let Scored { delta, group, candidate: replacement, target_score, context_delta } = &scored[best];
        let (delta, group, context_delta) = (*delta, *group, *context_delta);
        let normalized = LanguageModel::normalize(original);
        let original_key = Self::token_key(original);
        let replacement_normalized = LanguageModel::normalize(replacement);
        let effective_length = std::iter::once(pystr::len(&normalized))
            .chain(std::iter::once(pystr::len(&replacement_normalized)))
            .chain(alternatives.iter().map(|item| pystr::len(&LanguageModel::normalize(&item.1))))
            .max()
            .expect("two lengths at least");
        if (effective_length as i64) < minimum_length {
            return Ok(rejected("короткое слово"));
        }
        if ignored.contains(&original_key) {
            return Ok(rejected("исключение пользователя"));
        }
        if options.rejected_targets.contains(&group) {
            return Ok(rejected("отклонённое пользователем исправление"));
        }

        if let Some(forced) = options.forced_target_group.and_then(|forced_group| scored.iter().find(|item| item.group == forced_group)) {
            return Ok(DetectionDecision::new(
                true,
                original,
                &forced.candidate,
                source_group,
                forced.group,
                CONFIRMED_RULE_MIN_CONFIDENCE.max(forced.delta),
                "подтверждённое правило пользователя",
                source_score.clone(),
                forced.target_score.clone(),
            ));
        }

        if protect_code && looks_like_protected_token(original, &self.protected_tokens) {
            return Ok(rejected("код, адрес или аббревиатура"));
        }

        // A valid source token is the strongest false-positive guard. When both decodings are valid, leave the
        // inherently ambiguous word alone.
        if source_score.known {
            return Ok(rejected(if target_score.known { "обе раскладки дают допустимое слово" } else { "исходное слово допустимо" }));
        }

        let prediction = self.intent_prediction(original, replacement, source_group, group, options.trigger, &source_score, target_score, context_delta, context_group, options.use_intent_model)?;

        // Once a supported model has observed a token, its calibrated, trigger-specific decision is the sole
        // statistical switching signal. Applying feature-coverage or language-score heuristics afterwards is a second
        // statistical veto: it invalidates the recall certified for the selected model threshold. Coverage and lexical
        // scores remain diagnostics only. The heuristic ensemble below is available for short tokens and installations
        // where the artifact is absent or explicitly disabled.
        let decision = |should_convert: bool, confidence: f64, reason: &str| {
            DetectionDecision::new(should_convert, original, replacement, source_group, group, confidence, reason, source_score.clone(), target_score.clone())
                .with_prediction(&prediction)
        };
        if let Some(prediction) = &prediction {
            let should_convert = prediction.should_switch;
            let reason = if should_convert { "уверенное решение линейной n-граммной модели" } else { "линейная модель не достигла безопасного порога" };
            return Ok(decision(should_convert, delta, reason));
        }

        if target_score.known {
            let length_relief = (effective_length.saturating_sub(HEURISTIC_KNOWN_TARGET_RELIEF_START_CHARACTERS) as f64 * HEURISTIC_KNOWN_TARGET_RELIEF_PER_CHARACTER).min(1.0);
            let mut required = HEURISTIC_KNOWN_TARGET_MIN_MARGIN.max(confidence_threshold - length_relief);
            if target_score.spell_known && !target_score.exact {
                required += HEURISTIC_SPELL_ONLY_TARGET_EXTRA_MARGIN;
            }
            let context_supports_target = context_group == Some(group);
            // A Hunspell hit is already strong morphological evidence. The n-gram score is clamped at
            // LANGUAGE_MODEL_KNOWN_WORD_NATURALNESS_FLOOR for valid but very rare forms, so accept that floor while
            // still requiring an invalid source and a clear total-score margin.
            let morphology_is_plausible = target_score.ngram_score >= LANGUAGE_MODEL_KNOWN_WORD_NATURALNESS_FLOOR || context_supports_target;
            let should_convert = delta >= required && morphology_is_plausible;
            let reason = if target_score.exact {
                "слово найдено только в целевом частотном словаре"
            } else if should_convert {
                "словоформа подтверждена морфологическим словарём"
            } else if !morphology_is_plausible {
                "редкая словоформа без достаточного контекста"
            } else {
                "недостаточный перевес целевой словоформы"
            };
            return Ok(decision(should_convert, delta, reason));
        }

        // One accidental extra character should not erase otherwise decisive dictionary evidence. This does not
        // correct the typo itself; it only chooses the layout of the original physical sequence.
        if effective_length >= HEURISTIC_TYPO_DELETION_MIN_CHARACTERS {
            let target_without_one = self.model_for(group)?.best_single_deletion(replacement);
            let source_without_one = source_model.best_single_deletion(original);
            let typo_delta = target_without_one.value - source_score.value.max(source_without_one.value);
            let typo_supported = target_without_one.known && !source_without_one.known && typo_delta >= confidence_threshold + HEURISTIC_TYPO_DELETION_EXTRA_MARGIN;
            if typo_supported {
                return Ok(decision(true, delta.max(typo_delta), "целевая раскладка подтверждается после удаления одной опечатки"));
            }
        }

        // Unknown words can still be recognised by smoothed character n-grams, but only when the source looks
        // distinctly unnatural. The required margin decreases with length because longer sequences carry more
        // independent evidence.
        let length_relief = HEURISTIC_NGRAM_RELIEF_CAP.min(effective_length.saturating_sub(HEURISTIC_NGRAM_RELIEF_START_CHARACTERS) as f64 * HEURISTIC_NGRAM_RELIEF_PER_CHARACTER);
        let mut required = confidence_threshold + HEURISTIC_NGRAM_EXTRA_MARGIN - length_relief;
        if aggressive {
            required -= HEURISTIC_NGRAM_AGGRESSIVE_MARGIN_REDUCTION;
        }
        required = (confidence_threshold + HEURISTIC_NGRAM_MIN_EXTRA_MARGIN).max(required);
        let source_is_unlikely =
            source_score.ngram_score <= if aggressive { HEURISTIC_UNLIKELY_SOURCE_NGRAM_MAX_AGGRESSIVE } else { HEURISTIC_UNLIKELY_SOURCE_NGRAM_MAX };
        let target_is_plausible =
            target_score.ngram_score >= if aggressive { HEURISTIC_PLAUSIBLE_TARGET_NGRAM_MIN_AGGRESSIVE } else { HEURISTIC_PLAUSIBLE_TARGET_NGRAM_MIN };
        let should_convert = effective_length >= if aggressive { HEURISTIC_NGRAM_MIN_CHARACTERS_AGGRESSIVE } else { HEURISTIC_NGRAM_MIN_CHARACTERS }
            && source_is_unlikely
            && target_is_plausible
            && delta >= required;
        let reason = if should_convert {
            "устойчивый перевес символьной языковой модели"
        } else if !source_is_unlikely {
            "исходная последовательность похожа на допустимое слово"
        } else if !target_is_plausible {
            "целевая последовательность тоже нетипична"
        } else {
            "недостаточная уверенность статистической модели"
        };
        Ok(decision(should_convert, delta, reason))
    }

    #[allow(clippy::too_many_arguments)]
    fn intent_prediction(
        &self,
        original: &str,
        replacement: &str,
        source_group: i64,
        target_group: i64,
        trigger: CorrectionTrigger,
        source_score: &WordScore,
        target_score: &WordScore,
        context_delta: f64,
        context_group: Option<i64>,
        use_intent_model: bool,
    ) -> Result<Option<LinearPrediction>, DetectorError> {
        let Some(model) = self.intent_model.as_ref().filter(|_| use_intent_model) else {
            return Ok(None);
        };
        if pystr::len(&normalize_token(original)).max(pystr::len(&normalize_token(replacement))) < INTENT_MIN_RUNTIME_TOKEN_CHARACTERS {
            return Ok(None);
        }
        let item = IntentModelInput { original, alternative: replacement, source_group, target_group, trigger, source_score, target_score, context_delta, context_group };
        Ok(Some(model.predict(&item)?))
    }

    /// `_context_delta`.
    pub fn context_delta(
        &self,
        source_group: i64,
        target_group: i64,
        source_word: &str,
        target_word: &str,
        previous_words: GroupTexts<'_>,
        context_group: Option<i64>,
    ) -> Result<f64, DetectorError> {
        let source_context = self.model_for(source_group)?.context_score(text_of(previous_words, source_group).unwrap_or(""), source_word);
        let target_context = self.model_for(target_group)?.context_score(text_of(previous_words, target_group).unwrap_or(""), target_word);
        let mut delta = CONTEXT_DELTA_MULTIPLIER * (target_context - source_context);
        if context_group == Some(target_group) {
            delta += CONTEXT_TARGET_GROUP_BONUS;
        } else if context_group == Some(source_group) {
            delta -= CONTEXT_SOURCE_GROUP_PENALTY;
        }
        Ok(delta)
    }

    /// Case-insensitive identity that keeps layout-significant punctuation.
    pub fn token_key(token: &str) -> String {
        pystr::casefold(pystr::strip(token))
    }

    /// Expose the structural guard to the physical-token boundary logic.
    pub fn is_protected_token(&self, token: &str) -> bool {
        looks_like_protected_token(token, &self.protected_tokens)
    }
}

/// `LanguageDetector.is_protected_token(token)` of the Python's class: the guard with the packaged list.
pub fn is_protected_token(token: &str, protected_tokens: &ProtectedTokens) -> bool {
    looks_like_protected_token(token, protected_tokens)
}

/// `"CYRILLIC"` (1) or `"LATIN"` (2) when the name of a letter holds one of them, else 0.
fn script_of(c: char) -> u8 {
    if !ucd::isalpha(c) {
        return 0;
    }
    let code = c as u32;
    let at = SCRIPT_RUNS.partition_point(|run| run.1 < code);
    match SCRIPT_RUNS.get(at) {
        Some(run) if run.0 <= code => run.2,
        _ => 0,
    }
}

fn looks_like_protected_token(token: &str, protected_tokens: &ProtectedTokens) -> bool {
    if pystr::len(token) > MAX_CONVERTIBLE_TOKEN_CHARACTERS {
        return true;
    }
    let lowered = pystr::casefold(token);
    if protected_tokens.contains(&lowered) {
        return true;
    }
    if lowered.starts_with('-') {
        return true;
    }
    if ["http://", "https://", "www.", "@"].iter().any(|marker| lowered.contains(marker)) {
        return true;
    }
    if token.chars().any(ucd::isdigit) {
        return true;
    }
    if token.chars().any(|c| "_/\\=:".contains(c)) {
        return true;
    }
    let letters: Vec<char> = token.chars().filter(|&c| ucd::isalpha(c)).collect();
    if letters.len() >= ACRONYM_MIN_LETTERS && letters.iter().all(|&c| ucd::isupper(c)) {
        return true;
    }
    if letters.iter().skip(1).any(|&c| ucd::isupper(c)) {
        return true;
    }
    let lowered_chars: Vec<char> = lowered.chars().collect();
    if lowered_chars.windows(REPEATED_CHARACTER_RUN).any(|run| run.iter().all(|&c| c == run[0])) {
        return true;
    }
    let (mut cyrillic, mut latin) = (false, false);
    for &c in &letters {
        match script_of(c) {
            1 => cyrillic = true,
            2 => latin = true,
            _ => {}
        }
    }
    cyrillic && latin
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guard(token: &str) -> bool {
        looks_like_protected_token(token, &ProtectedTokens::from_text("# c\nNPM\n\n  Docker \n"))
    }

    #[test]
    fn the_structural_guard() {
        assert!(guard("npm") && guard("docker") && guard("-x") && guard("a@b") && guard("a1") && guard("a_b") && guard("ABC") && guard("aBc"));
        assert!(guard("aaaa") && !guard("привет") && guard("приvет") && guard(&"x".repeat(65)));
        assert!(!guard("Привет") && !guard("aaa") && !guard("") && !guard("A"));
    }

    #[test]
    fn token_keys() {
        assert_eq!(LanguageDetector::token_key("  ÀB\u{1c} "), "àb");
        assert_eq!(LanguageDetector::token_key("Straße"), "strasse");
    }
}
