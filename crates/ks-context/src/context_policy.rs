//! Contextual policy orchestration, independent of keyboard injection.

use crate::constants::{
    FIELD_CARET_LAG_MAX_CHARACTERS, FIELD_PENDING_MAX_CHARACTERS, MINIMUM_SHAPED_TOKEN_CHARACTERS, QUOTED_WORD_CLOSING_SIGNS, RUSSIAN_SLANG_MIN_TERM_BUCKET,
    WORD_QUOTES,
};
use crate::context_action_features::{
    alnum_question, cyrletter_question, digit_dot_question, letter_question, resume_question, variable_question, FeatureError,
};
use crate::context_model::{one_typo_from_word, term_bucket, AfterOrigin, BeforeOrigin, ContextAction, ContextEvidence, ContextModel, ContextPrediction, TermTables};
use crate::input_context::{Clock, FieldContext, FieldReader, InputContext};
use ks_lang::detector::{DetectionDecision, DetectorError, LanguageDetector};
use ks_lang::identifier_lexicon::IdentifierLexicon;
use ks_lang::ortho_model::{artifact_path, shape_of, KnownWord, OrthoEvidence, OrthoModel};
use ks_lang::short_words::{begins_words, is_short_word_override, opens_sentences, ISOLATED_SHORT_WORD_REASON};
use ks_lang::word_decision::NOT_A_WORD_REASON;
use ks_lang::{LanguageModel, Resources};
use ks_text::{pystr, ucd};
use std::sync::Arc;

/// A curated one-letter Russian word typed in the English layout right after an English term inside a Russian phrase
/// (KeySwitchEngine._stranded_letter): an explicit rule like the message-start one.
pub const STRANDED_SHORT_WORD_REASON: &str = "однобуквенное слово из безопасного списка после английского термина в русской фразе";
/// A lone letter the action model alone would convert: it has never seen one (ContextPolicy.decide).
pub const LONE_LETTER_REASON: &str = "одиночную букву переводят только правила из безопасного списка";
/// A Russian abbreviation or slang word the action model would convert into Latin keys nobody uses.
pub const RUSSIAN_SLANG_REASON: &str = "русское сокращение или сленг: латинское прочтение не встречается";
/// A curated short word at a pause in typing whose letters begin many words of their own language (ContextPolicy.decide).
pub const PAUSE_WORD_START_REASON: &str = "на паузе это может быть начало слова: правило коротких слов ждёт его границы";
/// A word the action model only suggests converting while the detector converts it (ContextPolicy.decide).
pub const SUGGESTION_CONFIRMED_REASON: &str = "контекстная модель предлагает другую раскладку, детектор её подтверждает";

/// An exception the policy raises: a `KeyError` of the detector, or a `ValueError` of the features.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyError {
    pub name: &'static str,
    pub message: String,
}

impl std::fmt::Display for PolicyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for PolicyError {}

impl From<DetectorError> for PolicyError {
    fn from(error: DetectorError) -> Self {
        PolicyError { name: error.name, message: error.message }
    }
}

impl From<FeatureError> for PolicyError {
    fn from(error: FeatureError) -> Self {
        PolicyError { name: "ValueError", message: error.0.to_string() }
    }
}

/// A Russian abbreviation or slang word typed as intended: Russian text uses it, nothing uses its Latin keys.
///
/// `пдф` and `впн` are in no lexicon, so they read as gibberish both ways and the language models lean to the Latin
/// reading; the action model learned the term tables (context_action_features) but on the owner's typing still turned
/// them into `gla` and `dgy` (field logs, 03.10.2026). The tables (context-term-frequency.json) say what decides: the
/// Cyrillic form occurs inside Russian technical text again and again, and its Latin keys occur neither there nor in
/// English text. A form whose Latin keys are a term (`тз` and `np`) is the model's to decide.
pub fn russian_slang(original: &str, alternative: &str, source_group: i64, source_known: bool, tables: &TermTables) -> bool {
    let bucket = term_bucket(original, "cyrillic", tables);
    source_group == 1
        && !source_known
        && bucket.parse::<usize>().is_ok_and(|bucket| bucket >= RUSSIAN_SLANG_MIN_TERM_BUCKET)
        && term_bucket(alternative, "latin", tables) == "0"
        && term_bucket(alternative, "english", tables) == "0"
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContextResult {
    pub decision: DetectionDecision,
    pub prediction: Option<ContextPrediction>,
    pub field: Option<FieldContext>,
    pub policy_applied: bool,
    pub decision_source: String,
    pub fallback_reason: String,
    /// Characters the field reader showed right of the caret; the model is not given them.
    pub withheld_after_characters: usize,
}

impl ContextResult {
    /// `ContextResult(decision)` with its defaults.
    pub fn new(decision: DetectionDecision) -> Self {
        ContextResult {
            decision,
            prediction: None,
            field: None,
            policy_applied: false,
            decision_source: "baseline".to_string(),
            fallback_reason: String::new(),
            withheld_after_characters: 0,
        }
    }
}

pub const INNER_MARKS: &str = "-'’";

/// `re.split(r"::?|\.", core)` (or `r"\."`): the parts between separators, empty ones kept.
fn split_scoped(core: &str, colons: bool) -> Vec<String> {
    let chars: Vec<char> = core.chars().collect();
    let mut parts = vec![String::new()];
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        if c == '.' {
            parts.push(String::new());
        } else if colons && c == ':' {
            if chars.get(at + 1) == Some(&':') {
                at += 1;
            }
            parts.push(String::new());
        } else {
            parts.last_mut().expect("one part at least").push(c);
        }
        at += 1;
    }
    parts
}

/// A Latin reading that is a quoted word or parts joined by colons, the shapes of code and of a cited word.
///
/// A quote opens it (WORD_QUOTES) and may close it, with one sign after the closing quote (`"legacy",`); the parts
/// between dots, or between colons (`std::swap`, `x:Name`), are letters. A colon is a part's edge only when the token was
/// typed with a small letter: the `Ж` of a Russian word in capitals is a colon too (`ОЖОГ` is `J:JU`).
///
/// The Python reads `core[0]` of an empty reading in a branch it takes for any empty string (`"" in WORD_QUOTES`) and
/// raises; an empty reading here is no quoted word.
pub fn quoted_or_scoped(replacement: &str, original: &str) -> bool {
    let mut core = replacement.to_string();
    let lead = pystr::slice(&core, None, Some(1)).to_string();
    if WORD_QUOTES.contains(lead.as_str()) {
        if lead.is_empty() {
            return false;
        }
        let quote = lead;
        core = pystr::slice(&core, Some(1), None).to_string();
        let closing = pystr::slice(&core, Some(-1), None);
        if QUOTED_WORD_CLOSING_SIGNS.contains(closing) && pystr::slice(&core, None, Some(-1)).ends_with(&quote) {
            core = pystr::slice(&core, None, Some(-1)).to_string();
        }
        if let Some(stripped) = core.strip_suffix(&quote) {
            core = stripped.to_string();
        }
    }
    !core.is_empty() && split_scoped(&core, !pystr::isupper(original)).iter().all(|part| pystr::isalpha(part))
}

/// Is this a word in the layout it was typed in, or a fragment?
///
/// A character model trained on words can only speak about words. `и"ю` is a quotation mark caught between two letters,
/// not Russian written badly, and left in the population such fragments set every threshold. A hyphen or an apostrophe
/// inside a token is different: the engine joins those to the word before it tests for a boundary at all, so `Я-то`,
/// `из-за` and `don't` reach a model whole and are ordinary words of their language.
pub fn word_shaped(token: &str) -> bool {
    let chars: Vec<char> = token.chars().collect();
    if chars.len() < MINIMUM_SHAPED_TOKEN_CHARACTERS || !ucd::isalpha(chars[0]) || !ucd::isalpha(chars[chars.len() - 1]) {
        return false;
    }
    let mut marks = 0;
    for &character in &chars[1..chars.len() - 1] {
        if ucd::isalpha(character) {
            continue;
        }
        if !INNER_MARKS.contains(character) {
            return false;
        }
        marks += 1;
        if marks > 1 {
            return false;
        }
    }
    true
}

/// `_one_typo_from_word`: the typo check reads a frequency lexicon; a scorer without one never reports a typo.
fn typo_of(text: &str, detector: &LanguageDetector, group: i64) -> Result<bool, DetectorError> {
    Ok(detector.model_for(group)?.as_language_model().is_some_and(|model| one_typo_from_word(text, model)))
}

/// Whether `text` ends with what was `typed`, its first letter in either case.
///
/// An editor may capitalise the first word of a sentence as it is finished, as macOS automatic capitalisation does. The
/// letters are still the ones the user typed, so the word is the one the observer saw: TextEdit showed `Ghbdtn` where
/// `ghbdtn` was typed, and the field checks refused every first word of a sentence (02.10.2026).
pub fn ends_with_typed(text: &str, typed: &str) -> bool {
    if text.ends_with(typed) {
        return true;
    }
    let (text_length, typed_length) = (pystr::len(text), pystr::len(typed));
    let tail = if typed_length <= text_length { pystr::slice(text, Some((text_length - typed_length) as i64), None) } else { "" };
    !typed.is_empty()
        && pystr::len(tail) == typed_length
        && pystr::slice(tail, Some(1), None) == pystr::slice(typed, Some(1), None)
        && pystr::casefold(pystr::slice(tail, None, Some(1))) == pystr::casefold(pystr::slice(typed, None, Some(1)))
}

/// How many characters typed after `typed` a field already shows past it, from two on; 0 if none.
///
/// `ahead` is the boundary and the keys the engine has queued behind it: the window takes a key in as soon as the hook
/// lets it through, while the engine decides on the word a moment later. One character after the word is the boundary
/// (ends_with_typed on `before[:-1]`); more, and every such word was refused as typed into a changed field: `e 'njuj `
/// typed at 40 ms a key into a GTK window of KDE Plasma kept its `e` in five of ten runs (27.10.2026).
pub fn shown_ahead(before: &str, typed: &str, ahead: &str) -> usize {
    for shown in (2..=pystr::len(ahead)).rev() {
        if before.ends_with(pystr::slice(ahead, None, Some(shown as i64))) && ends_with_typed(pystr::slice(before, None, Some(-(shown as i64))), typed) {
            return shown;
        }
    }
    0
}

/// What a field shows of `typed` before its caret when it has not taken the last letters in yet, or None.
///
/// A browser or an Electron editor reports its text a moment after the key: up to FIELD_PENDING_MAX_CHARACTERS of the last
/// letters may be missing, and at least one letter is there. Missing, not shifted: letters standing right after the caret
/// are a caret reported short of them (caret_lag), and further off than it allows, a changed field. The early switch reads
/// a word so (engine._typed_in_field), and so does the decision at its boundary.
pub fn typed_short_of(before: &str, after: &str, typed: &str) -> Option<String> {
    let most = FIELD_PENDING_MAX_CHARACTERS.min(pystr::len(typed).saturating_sub(1));
    for pending in 1..=most {
        let shown = pystr::slice(typed, None, Some(-(pending as i64)));
        if before.ends_with(shown) && !after.starts_with(pystr::slice(typed, Some(-(pending as i64)), None)) {
            return Some(shown.to_string());
        }
    }
    None
}

/// How many characters a field reports its caret short of the end of `typed`, or None.
///
/// A native snapshot normally ends its text before the caret with what was typed, the boundary after it or not yet
/// (ends_with_typed). VS Code Insiders (06.10.2026) reported the caret of its chat box one character early: the last typed
/// letter stood after the caret, a word just begun read as typed into another one, and every word was refused as a changed
/// field although the text was exactly the one typed, as the manual conversion of the same words showed. Up to
/// FIELD_CARET_LAG_MAX_CHARACTERS characters after the reported caret are taken as typed before it when that is what makes
/// the text end with the typed word; text that does not hold it there is still a changed field. `exact` asks for the text
/// to end with `typed` itself, the boundary already part of it and nothing after it.
pub fn caret_lag(before: &str, after: &str, typed: &str, exact: bool) -> Option<usize> {
    for lag in 0..=FIELD_CARET_LAG_MAX_CHARACTERS.min(pystr::len(after)) {
        let head = format!("{before}{}", pystr::slice(after, None, Some(lag as i64)));
        if ends_with_typed(&head, typed) || (!exact && ends_with_typed(pystr::slice(&head, None, Some(-1)), typed)) {
            return Some(lag);
        }
    }
    None
}

/// What `evidence_for_decision` takes beside the decision: its keyword arguments.
#[derive(Clone, Copy)]
pub struct EvidenceOptions<'a> {
    pub literal_tail: &'a str,
    pub boundary_text: &'a str,
    pub ortho: Option<&'a OrthoModel>,
    pub after_origin: AfterOrigin,
    pub identifiers: Option<&'a IdentifierLexicon>,
    pub inside: bool,
    pub before_origin: BeforeOrigin,
}

impl Default for EvidenceOptions<'_> {
    fn default() -> Self {
        EvidenceOptions {
            literal_tail: "",
            boundary_text: "",
            ortho: None,
            after_origin: AfterOrigin::None,
            identifiers: None,
            inside: false,
            before_origin: BeforeOrigin::Field,
        }
    }
}

/// Build the same lexical and optional orthotactic evidence for any caller. `shared` is the packaged identifier lexicon
/// the engine, the trainer and the evaluator share (`shared_identifiers()`), used when `identifiers` is not given.
#[allow(clippy::too_many_arguments)]
pub fn evidence_for_decision(
    baseline: &DetectionDecision, alternative: &str, target_group: i64, detector: &LanguageDetector, field: &FieldContext, trigger: &str,
    options: &EvidenceOptions<'_>, shared: Option<&IdentifierLexicon>,
) -> Result<ContextEvidence, DetectorError> {
    let source = detector.model_for(baseline.source_group)?.score(&baseline.original);
    let target = detector.model_for(target_group)?.score(alternative);
    let lexicon = options.identifiers.or(shared);
    let source_identifier = lexicon.is_some_and(|lexicon| lexicon.contains(&baseline.original));
    let target_identifier = lexicon.is_some_and(|lexicon| lexicon.contains(alternative));
    let mut ortho_score = None;
    let mut ortho_threshold = None;
    if let Some(ortho) = options.ortho
        && (baseline.source_group == 0 || baseline.source_group == 1)
    {
        let script = if baseline.source_group == 0 { "en" } else { "ru" };
        let keys = if baseline.source_group == 0 { baseline.original.as_str() } else { alternative };
        let shape = shape_of(&baseline.original, pystr::strip(&field.before).is_empty());
        let scored = ortho.score(&OrthoEvidence { keys, shape, source_script: script }, None);
        if scored.supported {
            ortho_score = Some(scored.total);
            ortho_threshold = Some(*ortho.thresholds.get(script));
        }
    }
    let mut evidence = ContextEvidence::new(&baseline.original, alternative, baseline.source_group, field.clone());
    evidence.trigger = trigger.to_string();
    evidence.baseline_convert = baseline.should_convert;
    evidence.source_known = source.known;
    evidence.target_known = target.known;
    evidence.score_delta = target.value - source.value;
    evidence.source_score = Some(source);
    evidence.target_score = Some(target);
    evidence.model_probability = baseline.model_probability;
    evidence.model_threshold = baseline.model_threshold;
    evidence.literal_tail = options.literal_tail.to_string();
    evidence.ortho_score = ortho_score;
    evidence.ortho_threshold = ortho_threshold;
    evidence.boundary_text = options.boundary_text.to_string();
    evidence.after_origin = options.after_origin;
    evidence.source_identifier = source_identifier;
    evidence.target_identifier = target_identifier;
    evidence.source_typo = typo_of(&baseline.original, detector, baseline.source_group)?;
    evidence.target_typo = typo_of(alternative, detector, target_group)?;
    evidence.source_opening = opens_sentences(&baseline.original, baseline.source_group);
    evidence.target_opening = opens_sentences(alternative, target_group);
    evidence.inside = options.inside;
    evidence.before_origin = options.before_origin;
    Ok(evidence.finished())
}

/// The keyword arguments of `ContextPolicy.decide`.
pub struct DecideOptions<'a> {
    pub after: &'a str,
    pub read_field: bool,
    pub field_override: Option<FieldContext>,
    pub literal_tail: &'a str,
    pub boundary_text: &'a str,
    pub after_origin: AfterOrigin,
    pub inside: bool,
    pub before_origin: BeforeOrigin,
    pub typed_ahead: Option<&'a dyn Fn() -> String>,
}

impl Default for DecideOptions<'_> {
    fn default() -> Self {
        DecideOptions {
            after: "",
            read_field: false,
            field_override: None,
            literal_tail: "",
            boundary_text: "",
            after_origin: AfterOrigin::None,
            inside: false,
            before_origin: BeforeOrigin::Field,
            typed_ahead: None,
        }
    }
}

pub struct ContextPolicy {
    pub stream: InputContext,
    pub model: Option<ContextModel>,
    pub status: String,
    pub ortho: Option<Arc<OrthoModel>>,
    pub ortho_status: String,
    pub reader: Option<Arc<dyn FieldReader>>,
    identifiers: Option<Arc<IdentifierLexicon>>,
}

/// The orthotactic model and its status, as the Python shares them between policies (`ContextPolicy._shared_ortho`).
pub type SharedOrtho = (Option<Arc<OrthoModel>>, String);

impl ContextPolicy {
    /// `ContextPolicy(reader, known=known)` with an orthotactic model of its own. `known` answers the orthotactic model's
    /// dictionary questions with the engine's own models, which it would otherwise find again by normalising the Russian
    /// supplement once more (ortho_model._engine_dictionaries).
    pub fn new(resources: &Arc<Resources>, reader: Option<Arc<dyn FieldReader>>, known: Option<KnownWord>, clock: Option<Clock>) -> Self {
        let shared = match OrthoModel::load(&artifact_path(resources.resource_dir()), None, known, Some(resources.clone())) {
            Ok(model) => {
                let version = model.version.clone();
                (Some(Arc::new(model)), version)
            }
            Err(error) => (None, format!("unavailable: {}", error.name)),
        };
        Self::with_ortho(resources, reader, shared, clock)
    }

    /// The same with the orthotactic artifact parsed once for many policies: a running application builds one engine,
    /// but replays and tests build many.
    pub fn with_ortho(resources: &Arc<Resources>, reader: Option<Arc<dyn FieldReader>>, shared: SharedOrtho, clock: Option<Clock>) -> Self {
        let stream = clock.map_or_else(InputContext::new, InputContext::with_clock);
        let (model, status) = ContextModel::try_load(&resources.resource_dir().join("models"));
        let (ortho, ortho_status) = shared;
        let identifiers = resources.identifier_lexicon().0;
        if model.is_some() {
            // Every question to the model asks whether a reading is a known identifier, and scores it with the
            // orthotactic model, which binds the engine's dictionaries at its first score. Both are done here, before the
            // keyboard hook starts, and not on the first word typed (ContextModel.load).
            if let Some(ortho) = &ortho {
                ortho.score(&OrthoEvidence { keys: "ghbdtn", shape: "lower", source_script: "en" }, None);
            }
        }
        ContextPolicy { stream, model, status, ortho, ortho_status, reader, identifiers }
    }

    /// `shared_identifiers()`.
    pub fn shared_identifiers(&self) -> Option<&IdentifierLexicon> {
        self.identifiers.as_deref()
    }

    fn tables(&self) -> &TermTables {
        &self.model.as_ref().expect("a policy with a model").tables
    }

    #[allow(clippy::too_many_lines, clippy::too_many_arguments)]
    pub fn decide(
        &self, baseline: &DetectionDecision, alternative: &str, target_group: i64, detector: &LanguageDetector, trigger: &str, mode: &str,
        options: &DecideOptions<'_>,
    ) -> Result<ContextResult, PolicyError> {
        let enabled = mode == "assist" || mode == "shadow";
        let Some(model) = self.model.as_ref().filter(|_| enabled) else {
            let mut result = ContextResult::new(baseline.clone());
            result.fallback_reason = if enabled { "model_unavailable" } else { "mode_disabled" }.to_string();
            return Ok(result);
        };
        let mut baseline = baseline.clone();
        let original = baseline.original.clone();
        let anchor = format!("{original}{}", options.literal_tail);
        let mut field = options.field_override.clone().unwrap_or_else(|| self.stream.snapshot(&anchor));
        let mut ignored_read = "";
        if options.read_field
            && let Some(reader) = &self.reader
        {
            let snapshot = reader.read(&field.application, self.stream.window);
            if let Some(snapshot) = snapshot.filter(|snapshot| snapshot.application == field.application) {
                let mut snapshot = snapshot.bounded();
                // A native snapshot includes the current word and possibly a boundary. Only use it if anchored to
                // this exact suffix.
                if snapshot.sensitive || snapshot.selection {
                    let mut refused = baseline.clone();
                    refused.should_convert = false;
                    refused.reason = "защищённое поле или выделение".to_string();
                    let mut result = ContextResult::new(refused);
                    result.field = Some(snapshot);
                    result.decision_source = "safety".to_string();
                    result.fallback_reason = "sensitive_or_selected_field".to_string();
                    return Ok(result);
                }
                if snapshot.input_buffer_only(&anchor) {
                    // Not the text the word went into: the observed keys stand, as for a field that cannot be read
                    // at all.
                    ignored_read = "field_input_buffer";
                } else {
                    if let Some(lag) = caret_lag(&snapshot.before, &snapshot.after, &anchor, false).filter(|&lag| lag != 0) {
                        let moved = pystr::slice(&snapshot.after, None, Some(lag as i64)).to_string();
                        snapshot.after = pystr::slice(&snapshot.after, Some(lag as i64), None).to_string();
                        snapshot.before.push_str(&moved);
                    }
                    let before = snapshot.before.clone();
                    let anchor_length = pystr::len(&anchor) as i64;
                    let behind = |text: &str, cut: i64| pystr::slice(text, None, Some(-cut)).to_string();
                    let ahead = options.typed_ahead.filter(|_| !anchor.is_empty()).map(|ahead| shown_ahead(&before, &anchor, &format!("{}{}", options.boundary_text, ahead())));
                    if !anchor.is_empty() && ends_with_typed(&before, &anchor) {
                        snapshot.before = behind(&before, anchor_length);
                        field = snapshot;
                    } else if !anchor.is_empty() && ends_with_typed(pystr::slice(&before, None, Some(-1)), &anchor) {
                        snapshot.before = behind(&before, anchor_length + 1);
                        field = snapshot;
                    } else if let Some(ahead) = ahead.filter(|&ahead| ahead != 0) {
                        // Asked after the read: every key the field shows was queued before the window got it.
                        snapshot.before = behind(&before, anchor_length + ahead as i64);
                        field = snapshot;
                    } else if let Some(shown) = (!anchor.is_empty()).then(|| typed_short_of(&before, &snapshot.after, &anchor)).flatten() {
                        // The editor has not shown the last letters yet: the word is the one typed and is decided on
                        // its keys, and its correction waits for the field to show it (engine._waits_for_field).
                        // Refused as a changed field, `ашкуащч` stayed for `firefox` in Antigravity (the owner's
                        // typing, 19.10.2026), and a word in such an editor was never put to the model at its space.
                        snapshot.before = behind(&before, pystr::len(&shown) as i64);
                        field = snapshot;
                        ignored_read = "field_behind";
                    } else {
                        // The editor contradicts the observer: do not fall back to stale strokes and erase a
                        // different span of text.
                        let mut refused = baseline.clone();
                        refused.should_convert = false;
                        refused.reason = "текст активного поля изменился".to_string();
                        let mut result = ContextResult::new(refused);
                        result.field = Some(snapshot);
                        result.decision_source = "safety".to_string();
                        result.fallback_reason = "field_changed".to_string();
                        return Ok(result);
                    }
                }
            }
        }
        // The text right of the caret is the model's evidence only when the engine knows where it came from: the
        // neighbour it is about to convert or the word it kept. What a field reader shows there is not given to the
        // model. The certified protocol never reads the field, and the corpus has right text only as a sentence's own
        // continuation, so the model took its script for the word's language: `мне` typed in front of English text
        // already in a VS Code chat box became `vyt` at p=0.9998, and `vyt` stayed (the owner's laptop, 0.38.1 and
        // 0.52.0: 7 false conversions and 6 missed words in 102 such words, 12.10.2026). The owner's PC showed no gain
        // from the text in 470 words read with it.
        let withheld = pystr::len(&field.after);
        field.after = options.after.to_string();
        let evidence_options = EvidenceOptions {
            literal_tail: options.literal_tail,
            boundary_text: options.boundary_text,
            ortho: self.ortho.as_deref(),
            after_origin: options.after_origin,
            identifiers: None,
            inside: options.inside,
            before_origin: options.before_origin,
        };
        let evidence = evidence_for_decision(&baseline, alternative, target_group, detector, &field, trigger, &evidence_options, self.shared_identifiers())?;
        let source = evidence.source_score.clone().expect("evidence_for_decision scores both readings");
        let target = evidence.target_score.clone().expect("evidence_for_decision scores both readings");
        let mut prediction = model.predict(&evidence);
        if prediction.action == ContextAction::Keep
            && options.after_origin == AfterOrigin::None
            && model.answers_cyrletter
            && cyrletter_question(&baseline.original, alternative, &field.before, trigger)
        {
            // The Cyrillic-letter head keeping a letter at its own space waits for the word after it instead, and is asked
            // again beside it: there it answers well (577 of 592 converted development frames of candidate v81 e1 beside a
            // converted next word, against 898 of 4 324 at the letter's own space), and before it a keep is a guess. Kept
            // outright, `d` before `кабинете` and after `если` stayed where `в` was meant and 0.64.1, waiting, converted
            // it (the owner's typing). The text is the same either way until the next word comes.
            prediction.action = ContextAction::Wait;
        }
        if mode == "shadow" {
            let mut result = ContextResult::new(baseline);
            result.prediction = Some(prediction);
            result.field = Some(field);
            result.fallback_reason = "shadow_mode".to_string();
            result.withheld_after_characters = withheld;
            return Ok(result);
        }
        // The model's verdict stands whatever its support flag says. The flag only tells whether some weight names this
        // application or these exact neighbour words; the model knows a handful of each, so in Firefox, Edge or a terminal
        // a confident verdict used to be thrown away for the baseline - `tot привет` stayed (0.31.x logs, 24.09.2026) -
        // while the trainer evaluates and certifies the model on its own verdict, with no such fallback. The flag stays in
        // the log as a diagnostic.
        // A lone letter is outside what the action model learned: every single-letter row of its corpus sits in
        // quarantine, so its verdict on one is no opinion, and on its own it turned `ч` into `x` and `ы` into `s` at
        // p=0.99 (field logs, 03.10.2026). It converts a lone letter only together with a curated rule.
        // The same holds for a single letter with digits (`1С`, `а1`): a product name or a cell, and in Russian prose the
        // model turned `1С` into `1C`.
        // A model with the single-letter head has learned the letter right after a Latin word
        // (context_action_features.letter_question): there its verdict on a letter is an opinion, and `nats b redis` is
        // `nats и redis` (the owner's typing, 0.32-0.41). One with the variable head has learned a letter of the Russian
        // layout after a word of Cyrillic letters (variable_question): a Latin variable typed there (`ч` for `x`) or a
        // letter meant as written (`да я про то ч`). One with the letters-and-digits head has learned a token of Russian
        // letters and digits whose keys are Latin letters and digits, after Russian text (alnum_question): `м67` for
        // `v67` and `ч64` for `x64`, and `1с` or `3д` meant as typed. One with the resume head has learned a short word
        // right after Latin text asked again once its next word is planned for conversion (resume_question): `а` typed
        // `f` in `seek() , f там`, and a Latin `x` before Russian text. One with the Cyrillic-letter head has learned a
        // Latin letter whose Cyrillic reading is a Russian word of one letter, after Russian text (cyrletter_question):
        // `в` typed `d` in `разрабатывал и d документ`, and `C` meant in `язык C`. One with the digit-dot head has learned
        // digits with `ю` typed in the Russian layout (digit_dot_question): `2ю` for `2.`.
        let source_model = detector.model(baseline.source_group);
        if trigger == "pause"
            && is_short_word_override(&baseline)
            && source_model.and_then(|scorer| scorer.as_language_model()).is_some_and(|scorer: &LanguageModel| begins_words(scorer, &baseline.original))
        {
            // A curated rule speaks for a word, and at a pause in typing two letters that begin a hundred words and more
            // of their own language are as likely the start of one: `ша` of `шаблоне` became `if` while the owner thought
            // mid-word (field logs, 29.10.2026), and in every battery `ша` at a pause went on into a longer word. The
            // rule waits for the word's boundary, where it applies as before; the model's own verdict at the pause stands.
            baseline.should_convert = false;
            baseline.reason = PAUSE_WORD_START_REASON.to_string();
        }
        let tables = self.tables();
        let learned = (model.answers_letters && letter_question(&baseline.original, alternative, &field.before))
            || (model.answers_variables && variable_question(&baseline.original, alternative, &field.before, trigger))
            || (model.answers_alnum && alnum_question(&baseline.original, alternative, &field.before, tables)?)
            || (model.answers_resume && resume_question(&baseline.original, alternative, &field.before, &field.after, options.after_origin))
            || (model.answers_cyrletter && cyrletter_question(&baseline.original, alternative, &field.before, trigger))
            || (model.answers_digit_dot && digit_dot_question(&baseline.original, alternative, trigger));
        let lone = baseline.original.chars().filter(|&c| ucd::isalpha(c)).count() == 1
            && baseline.original.chars().all(|c| ucd::isalpha(c) || ucd::isdigit(c))
            && !learned;
        let slang = russian_slang(&baseline.original, alternative, baseline.source_group, source.known, tables);
        let converted = |reason: &str| {
            let mut decision = baseline.clone();
            decision.should_convert = true;
            decision.replacement = alternative.to_string();
            decision.target_group = target_group;
            decision.source_score = source.clone();
            decision.target_score = target.clone();
            decision.reason = reason.to_string();
            decision.confidence = prediction.probability;
            decision
        };
        let decision = if prediction.action == ContextAction::Convert && !slang && (!lone || is_short_word_override(&baseline)) {
            converted("решение контекстной модели")
        } else if prediction.action == ContextAction::Suggest && baseline.should_convert && !slang && !lone {
            // The model leans to the other layout without its threshold's certainty, and the detector, which the model was
            // asked to overrule, converts: two verdicts of the same direction. A word no lexicon holds (`byzpt` for
            // `инязе`, `ntby` for `теин`) was left as typed at a suggestion while the baseline pair converted it, and in
            // the owner's typing (0.44-0.53) such words were meant converted in 6 of the 7 cases of more than one letter.
            // Measured on the owner's field batteries: 15 more words converted right, 20 fewer missed, no more converted
            // wrongly; the development and calibration sequences lost no restoration and corrupted nothing. One live word
            // of the seven was meant as typed (`lokim`). A curated short word converts on a suggestion too: left to wait
            // for its neighbour, `b[` after `the.hoster` stayed where `их` was meant. The model's wait still delays it
            // below.
            converted(SUGGESTION_CONFIRMED_REASON)
        } else if (is_short_word_override(&baseline)
            && (baseline.reason == ISOLATED_SHORT_WORD_REASON || trigger == "pause" || (prediction.action != ContextAction::Wait && model.feature_version != 3)))
            || baseline.reason == STRANDED_SHORT_WORD_REASON
        {
            // A curated, reviewed exception is an explicit rule, not a guess, so neither a probabilistic `keep` nor an
            // under-confident `convert` cancels it. `wait` still delays it at a boundary, because that is about timing
            // rather than direction and the lookahead may resolve the word jointly - thirty-six of the thirty-seven
            // curated entries are Russian function words, exactly the words a phrase decides.
            // On the pause trigger the timing has already happened: the user stopped, no next word is coming, and a delay
            // with nothing left to wait for is a refusal wearing another name. Letting `wait` lose everywhere was measured
            // on 18.09.2026 and took the lookahead with it. The model's opinion is recorded either way.
            // A feature-version-3 model is the arbiter of the other curated rules. A lone letter opening a message is the
            // one named exception, and it beats a "wait" as well: the model cannot have an opinion here, because every
            // single-letter row of the corpus sits in quarantine - their families are held by other frozen evidence - so
            // it has never seen one. Waiting for a next word that may never come would leave `z ` standing in a sent
            // message. The rule itself is measured: a Russian utterance opens with one of а/и/с/в/к/у/о/я once in seven
            // sentences, an English one never opens with a lone f/b/c/d/r/e/j/z (UD Taiga and UD EWT, 17.09.2026).
            // Teaching this to the model needs a corpus that keeps those rows; until then it is an exception with a name,
            // visible in the model card and this comment. The second named exception is the same letter right after an
            // English term inside a Russian phrase (`поправь env b`): the term switched the layout, and the model has no
            // single-letter row to know the letter by.
            let mut result = ContextResult::new(baseline);
            result.prediction = Some(prediction);
            result.field = Some(field);
            result.decision_source = "short_word_override".to_string();
            result.fallback_reason = "trusted_short_word".to_string();
            result.withheld_after_characters = withheld;
            return Ok(result);
        } else {
            let mut decision = baseline.clone();
            decision.should_convert = false;
            decision.reason = if prediction.action == ContextAction::Convert {
                if slang { RUSSIAN_SLANG_REASON } else { LONE_LETTER_REASON }
            } else {
                match prediction.action {
                    ContextAction::Keep => "контекстная модель оставляет текст",
                    ContextAction::Wait => "контекстная модель ждёт продолжения",
                    _ => "контекстная модель предлагает проверить раскладку",
                }
            }
            .to_string();
            decision
        };
        let suggest = prediction.action == ContextAction::Suggest;
        let mut result = ContextResult::new(decision);
        result.prediction = Some(prediction);
        result.field = Some(field.clone());
        result.policy_applied = true;
        result.decision_source = "context_model".to_string();
        result.fallback_reason = ignored_read.to_string();
        if model.feature_version != 3 || suggest {
            // A feature-version-3 model decides alone, but when it only suggests the other layout, the orthotactic model
            // may settle it: the keys of a word no lexicon holds (`byzpt` for `инязе`, `ntby` for `теин`) were left as
            // typed at a suggestion, while the baseline pair converted them on that licence. Its keep and wait stand.
            result = self.licensed(result, &baseline, alternative, target_group, &field, Some(detector));
        }
        let mut result = Self::spelling_a_word(result);
        result.withheld_after_characters = withheld;
        Ok(result)
    }

    /// A word is never replaced by something that is not one, whichever layer asked.
    ///
    /// Six Russian letters sit on punctuation keys, so the Latin reading of a Russian abbreviation such as `збс` is
    /// `p,c`. The detector's own conversions pass `word_shape_veto` in `automatic_word_decision`; a conversion the model or
    /// the orthotactic licence asked for did not, and the shipped model asked for exactly that, with p=0.997, in every
    /// application it knows (found 20.09.2026 while scoring a candidate). The model is, however, trained to restore what
    /// the detector is not asked about: a dotfile (`ювшые` is `.dist`), a contraction (`вщтэе` is `don't`) and a dotted
    /// name - a domain, an address or a file (`учфьздуюсщь` is `example.com`) - so a leading dot, one inner apostrophe and
    /// letters joined by single dots are the shapes a Latin reading may keep. Until 04.10.2026 a dot inside was refused as
    /// well (`дюп` is `l.g`), and on the owner's own typing that left every domain and address typed in the Russian layout
    /// as it was, with the model sure of each at p=1.0; the model of 0.38.0 keeps `дюп` itself. A quoted token and parts
    /// joined by colons are shapes too (quoted_or_scoped): `"Boot` typed in the Russian layout is `ЭИщще`, `std::swap` is
    /// `ыевЖЖыцфз`, and the model converts them, most at p=1.0, which this refusal overruled in 146 messages of the
    /// developer-chat development set (19.10.2026); a sign after a word without a quote stays refused, since `б` and `ю`
    /// end Russian words (`учеб` is `ext,`). The refusal is recorded as a safety decision, not as the model's opinion.
    pub fn spelling_a_word(result: ContextResult) -> ContextResult {
        let decision = &result.decision;
        if !decision.should_convert || !pystr::isalpha(&decision.original) {
            return result;
        }
        let body = decision.replacement.strip_prefix('.').unwrap_or(&decision.replacement);
        if pystr::isalpha(body) || word_shaped(body) || body.split('.').all(pystr::isalpha) || quoted_or_scoped(body, &decision.original) {
            return result;
        }
        let mut vetoed = decision.clone();
        vetoed.should_convert = false;
        vetoed.reason = NOT_A_WORD_REASON.to_string();
        let mut refused = ContextResult::new(vetoed);
        refused.prediction = result.prediction;
        refused.field = result.field;
        refused.policy_applied = result.policy_applied;
        refused.decision_source = "safety".to_string();
        refused.fallback_reason = "replacement_not_a_word".to_string();
        refused
    }

    /// Let the orthotactic model turn a refusal into a conversion, never the reverse.
    ///
    /// The word models answer "is this a word"; this one answers "could this sequence of keys have been produced by this
    /// language at all". That is the only evidence available for a token no dictionary contains, which is why an unknown
    /// command typed in the wrong layout used to survive every earlier layer untouched. It never vetoes, never overrides an
    /// explicit rule, and `wait` still outranks it, because waiting is about timing.
    ///
    /// It also stays out of a contextual veto: when the detector itself wanted to convert and the model refused, that
    /// refusal is the context model's job and is left alone. This layer only adds recall where the detector abstained,
    /// which is exactly the population no dictionary covers.
    pub fn licensed(
        &self, result: ContextResult, baseline: &DetectionDecision, alternative: &str, target_group: i64, field: &FieldContext,
        detector: Option<&LanguageDetector>,
    ) -> ContextResult {
        let waits = result.prediction.as_ref().is_none_or(|prediction| prediction.action == ContextAction::Wait);
        if result.decision.should_convert || baseline.should_convert || self.ortho.is_none() || waits {
            return result;
        }
        let Some(licensed) = self.orthotactic(baseline, alternative, target_group, field, detector) else {
            return result;
        };
        let mut licensed_result = ContextResult::new(licensed);
        licensed_result.prediction = result.prediction;
        licensed_result.field = Some(field.clone());
        licensed_result.decision_source = "ortho_model".to_string();
        licensed_result.fallback_reason = "orthotactic_licence".to_string();
        licensed_result
    }

    /// Score the physical keys under both languages and license a conversion.
    ///
    /// The features of a schema 2 artifact ask dictionaries about parts of the token; the detector's own models answer, so
    /// the model never loads a second copy of them.
    pub fn orthotactic(
        &self, baseline: &DetectionDecision, alternative: &str, target_group: i64, field: &FieldContext, detector: Option<&LanguageDetector>,
    ) -> Option<DetectionDecision> {
        let model = self.ortho.as_deref();
        if baseline.source_score.known {
            // The dictionary of the layout in use knows this token. The word models hold stronger evidence than any
            // character model can, and their refusal stands: `руку` is a Russian word whose keys spell the English word
            // `here`.
            return None;
        }
        let model = model?;
        if baseline.source_group != 0 && baseline.source_group != 1 {
            return None;
        }
        let source_script = if baseline.source_group == 0 { "en" } else { "ru" };
        // Key space is the US rendering, so it is whichever side is Latin.
        let keys = if source_script == "en" { baseline.original.as_str() } else { alternative };
        if (pystr::len(keys) as i64) < model.minimum_length {
            return None;
        }
        if !word_shaped(&baseline.original) {
            // Punctuation caught between letters is not a word in the layout being typed, and a character model trained on
            // words cannot judge it: `и"ю` is a quotation mark between two letters, not Russian.
            return None;
        }
        let shape = shape_of(&baseline.original, pystr::strip(&field.before).is_empty());
        let evidence = OrthoEvidence { keys, shape, source_script };
        let score = match detector {
            Some(detector) => {
                let known = |script: &str, word: &str| detector.model(if script == "en" { 0 } else { 1 }).is_some_and(|scorer| scorer.score(word).known);
                model.score(&evidence, Some(&known))
            }
            None => model.score(&evidence, None),
        };
        if !score.supported || score.total <= *model.thresholds.get(source_script) {
            return None;
        }
        let mut decision = baseline.clone();
        decision.should_convert = true;
        decision.replacement = alternative.to_string();
        decision.target_group = target_group;
        decision.reason = "последовательность клавиш не соответствует языку".to_string();
        decision.confidence = score.total;
        Some(decision)
    }
}
