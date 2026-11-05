//! The early switch, the typo correction and the bookkeeping of a finished correction.

use ks_cast::To as _;
use super::logging::technical_event;
use super::*;
use ks_context::context_policy::{caret_lag, typed_short_of};
use ks_context::prefix_model::{PrefixInput, PrefixSide};
use ks_context::typo_correction::{corrected_keys, known_elsewhere};
use ks_lang::early_switch::{early_switch_decision, EarlySwitchDecision, EarlySwitchPolicy, PrefixScorer};
use serde_json::json;

/// `type(error).__name__` of an operating system error.
pub(crate) fn os_error_name(error: &std::io::Error) -> &'static str {
    match error.kind() {
        std::io::ErrorKind::PermissionDenied => "PermissionError",
        std::io::ErrorKind::NotFound => "FileNotFoundError",
        std::io::ErrorKind::AlreadyExists => "FileExistsError",
        std::io::ErrorKind::IsADirectory => "IsADirectoryError",
        _ => "OSError",
    }
}

impl EngineCore {
    fn early_switch_policy(&self) -> EarlySwitchPolicy {
        let minimum = self.bounded_setting(
            "detection.early_switch_min_length", DEFAULT_EARLY_SWITCH_MIN_LENGTH,
            EARLY_SWITCH_MIN_LENGTH_SETTING_MIN, EARLY_SWITCH_MIN_LENGTH_SETTING_MAX,
        ).to::<i64>();
        EarlySwitchPolicy { minimum_length: minimum.to::<usize>(), ..EarlySwitchPolicy::default() }
    }

    /// Switch the layout as soon as the typed prefix proves it wrong.
    pub(crate) fn maybe_early_switch(&mut self) -> EngineResult<()> {
        if self.boundary_model.is_some()
            && let Some(last) = self.strokes.last()
            && !pystr::isalpha(&last.character)
            && Self::_is_layout_letter(last)
        {
            // A sign key that ends the prefix may still be punctuation; the boundary model
            // decides it once the word ends. A sign key with a letter after it is a letter of
            // the other layout (`lj,f` is `доба`), and the prefix model is trained on such
            // prefixes. Any such key used to block the early switch for the whole word: a
            // third of the long Russian words typed in the English layout in field logs
            // carry б, ю, ж, х, э or ъ and could only convert at their boundary.
            return Ok(());
        }
        if !self.flag("detection.early_switch", true) || !self.flag("enabled", true) || self.caret_unknown() {
            return Ok(());
        }
        if self.early_switch_origin.is_some() || self.pending.is_some() {
            return Ok(());
        }
        let policy = self.early_switch_policy();
        if self.strokes.len() < policy.minimum_length {
            return Ok(());
        }
        let source_group = self.source_group;
        if !self.in_models(source_group) || self.word_protected(source_group) {
            return Ok(());
        }
        let strokes = self.strokes.clone();
        let original = text_for_group(&strokes, source_group);
        let alternatives = self.alternatives_of(&strokes, source_group);
        let mut decision = {
            let indexes: Vec<(i64, &PrefixIndex)> = self.prefix_indexes.iter().enumerate().map(|(group, index)| (group.to::<i64>(), &**index)).collect();
            let scorers: Vec<(i64, &dyn PrefixScorer)> =
                self.models.iter().enumerate().map(|(group, model)| -> (i64, &dyn PrefixScorer) { (group.to::<i64>(), &**model) }).collect();
            let texts: Vec<(i64, &str)> = alternatives.iter().map(|(group, text)| (*group, text.as_str())).collect();
            early_switch_decision(&indexes, &scorers, &original, &texts, source_group, &policy)
        };
        let application = self.backend.active_application();
        let mut excluded = self.application_excluded(&application);
        let mut field: Option<FieldContext> = None;
        let mut confidence = EARLY_SWITCH_CONFIDENCE;
        let mut protection = self.early_prefix_protection(&original, source_group, decision.target_group);
        if !excluded && protection.is_empty() {
            let (found, refused) = self.early_prefix_field(&original, &application);
            field = Some(found);
            protection = refused;
            excluded = self.application_excluded(&application);
        }
        if !protection.is_empty() {
            decision.should_switch = false;
            decision.reason = protection.to_owned();
        } else if !excluded {
            (decision, confidence) = self.decide_prefix(decision, field.as_ref())?;
        }
        if strokes.len() == policy.minimum_length || decision.should_switch {
            self.log_early_switch(&decision, &policy, &application, excluded);
        }
        if !decision.should_switch || excluded {
            return Ok(());
        }
        let head = self.mention_head(&application);
        let plan = CorrectionPlan {
            automatic: true,
            mode: "early",
            context_field: field.as_ref().filter(|field| field.source != "observed").map_or_else(String::new, |field| field.field_id.clone()),
            head,
            ..CorrectionPlan::new(strokes.clone(), None, source_group, decision.target_group, original, decision.replacement.clone(), confidence, &application)
        };
        // The last letter's key is physically still down: a synthetic press of a
        // held key is ignored by the X server and the retyped letter would be
        // lost. Like boundary corrections, execute on that key's release and
        // absorb letters pressed before it (rollover typing).
        self.pending = Some(Arc::new(plan));
        self.learning_prompt_after = None;
        let trigger = strokes.last().map_or(-1, |stroke| i64::from(stroke.keycode));
        self.pending_trigger_keycode = trigger;
        technical_event!(self.shared, "early_switch_scheduled", "trigger_keycode": trigger, "prefix_length": strokes.len());
        Ok(())
    }

    pub fn early_prefix_protection(&mut self, original: &str, source: i64, _target: i64) -> &'static str {
        let key = LanguageDetector::token_key(original);
        if self.words_setting("exclusions.words").iter().any(|word| LanguageDetector::token_key(word).starts_with(&key)) {
            return "excluded_word_prefix";
        }
        if self.flag("detection.learning", true) && self.learning.keeps_continuation(source, original) {
            return "learned_keep_rule";
        }
        if self.flag("detection.protect_code", true) && self.detector.is_protected_token(original) {
            return "protected_token";
        }
        if self.context_waiting.is_some() {
            return "context_word_waiting";
        }
        let ahead = self.typed_ahead(&self.strokes);
        if self.insertion_holds(original, &ahead, &self.backend.active_application()) {
            // Letters typed into another word are not the start of one.
            return "inside_word";
        }
        ""
    }

    pub fn early_prefix_field(&mut self, original: &str, application: &str) -> (FieldContext, &'static str) {
        let mut field = self.context_policy.stream.snapshot(original);
        if !self.flag("detection.context_aware", true) {
            field = FieldContext::new(application, self.focus_or_zero().to_string());
        }
        if self.flag("detection.context_read_field", false)
            && let Some(reader) = self.context_policy.reader.clone()
            && let Some(snapshot) = reader.read(application, self.focus_or_zero())
        {
            let snapshot = snapshot.bounded();
            if snapshot.sensitive {
                self.shared.set_sensitive_context_window(self.shared.focus_window());
                self.context_policy.stream.clear();
                self.contexts.clear();
                self.shared.update(Update {
                    current_word: Some(String::new()),
                    last_action: Some("Защищённое поле: обработка отключена".to_owned()),
                    ..Update::default()
                });
                return (snapshot, "sensitive_field");
            }
            let around = Self::_typed_in_field(&snapshot, original, &self.typed_ahead(&self.strokes));
            if around.is_none() && snapshot.input_buffer_only(original) && !snapshot.selection && snapshot.application == application {
                // Not the text the prefix went into (FieldContext.input_buffer_only): the observed keys stand.
                return (field, "");
            }
            let changed = snapshot.selection || snapshot.field_id.is_empty() || snapshot.application != application;
            match around {
                Some((before, after)) if !changed => field = field_with(&snapshot, before, after),
                _ => return (snapshot, "context_field_changed"),
            }
        }
        (field, "")
    }

    /// The text before and after a word being typed, from a field read as its last key goes down.
    ///
    /// The field may not show that key yet: a browser or an Electron editor reports its text a
    /// moment after the key, and the early switch was refused at the fourth letter as a changed
    /// field in 175 of 210 words in Firefox, 174 of 416 in the Claude app and 532 of 2 275 in VS
    /// Code (Windows logs, 05.09-08.10.2026). Up to `FIELD_PENDING_MAX_CHARACTERS` of the last letters
    /// may be missing, the caret right after the rest. Text typed ahead may follow the word, and
    /// the caret may be seen short of it (`caret_lag`). None for any other text.
    pub(crate) fn _typed_in_field(field: &FieldContext, typed: &str, ahead: &str) -> Option<(String, String)> {
        let boundaries: Vec<usize> = ahead.char_indices().map(|(index, _)| index).chain(std::iter::once(ahead.len())).collect();
        for &end in boundaries.iter().rev() {
            let shown = format!("{typed}{}", &ahead[..end]);
            if let Some(lag) = caret_lag(&field.before, &field.after, &shown, true) {
                let head = format!("{}{}", field.before, pystr::slice(&field.after, None, Some(lag.to::<i64>())));
                if head.ends_with(&shown) {
                    return Some((head[..head.len() - shown.len()].to_owned(), pystr::slice(&field.after, Some(lag.to::<i64>()), None).to_owned()));
                }
            }
        }
        typed_short_of(&field.before, &field.after, typed)
            .map(|short| (pystr::slice(&field.before, None, Some(-pystr::len(&short).to::<i64>())).to_owned(), field.after.clone()))
    }

    pub(crate) fn decide_prefix(&self, baseline: EarlySwitchDecision, field: Option<&FieldContext>) -> EngineResult<(EarlySwitchDecision, f64)> {
        let mode = self.text_setting("detection.context_policy", "assist");
        if !self.flag("detection.context_aware", true) || (mode != "assist" && mode != "shadow") {
            return Ok((baseline, EARLY_SWITCH_CONFIDENCE));
        }
        let length = pystr::len(&baseline.original);
        let upper_after_first = |text: &str| text.chars().skip(1).any(ks_text::ucd::isupper);
        let supported = (PREFIX_MIN_CHARACTERS..=PREFIX_MAX_CHARACTERS).contains(&length)
            && pystr::isalpha(&baseline.replacement)
            && !upper_after_first(&baseline.original)
            && !upper_after_first(&baseline.replacement)
            && (baseline.source_group == 0 || baseline.source_group == 1)
            && baseline.target_group == 1 - baseline.source_group
            && self.in_models(0)
            && self.in_models(1)
            && self.prefix_indexes.len() > 1;
        let (Some(model), Some(field), true) = (self.prefix_model.as_ref(), field, supported) else {
            let reason = if self.prefix_model.is_none() { "prefix_model_unavailable" } else { "prefix_input_unsupported" };
            let result = if mode == "assist" { EarlySwitchDecision { should_switch: false, reason: reason.to_owned(), ..baseline } } else { baseline };
            return Ok((result, EARLY_SWITCH_CONFIDENCE));
        };
        let sides = [
            PrefixSide { index: &self.prefix_indexes[0], model: &self.models[0] },
            PrefixSide { index: &self.prefix_indexes[1], model: &self.models[1] },
        ];
        let item = PrefixInput {
            original: baseline.original.clone(),
            alternative: baseline.replacement.clone(),
            source_group: baseline.source_group,
            field: field.clone(),
        };
        let prediction = model.predict(&item, [&sides[0], &sides[1]])?;
        let action = prediction.action.as_str();
        technical_event!(
            self.shared, "prefix_decision", "action": action, "score": logged(prediction.probability),
            "model_version": prediction.model_version, "mode": mode, "baseline_convert": baseline.should_switch,
            "policy_applied": mode == "assist", "decision_source": if mode == "assist" { "prefix_model" } else { "prefix_index" },
            "final_action": if mode == "assist" { action } else if baseline.should_switch { "convert" } else { "wait" },
            "prefix_length": length, "source_group": baseline.source_group,
            "context_source": field.source, "before_characters": pystr::len(&field.before), "after_characters": pystr::len(&field.after),
            "field_role": field.role.as_str(),
        );
        if mode == "shadow" {
            return Ok((baseline, EARLY_SWITCH_CONFIDENCE));
        }
        let reason = format!("префиксная модель: {action}");
        Ok((EarlySwitchDecision { should_switch: action == "convert", reason, ..baseline }, prediction.probability))
    }

    /// Extend a scheduled early switch with letters typed before release.
    pub(crate) fn refresh_early_plan(&mut self, plan: &PlanRef) -> EngineResult<Option<PlanRef>> {
        let strokes = self.strokes.clone();
        let prefix = plan.strokes.len();
        if strokes.len() < prefix || strokes[..prefix] != plan.strokes[..] || self.source_group != plan.source_group {
            return Ok(None);
        }
        let original = text_for_group(&strokes, plan.source_group);
        let replacement = text_for_group(&strokes, plan.target_group);
        if plan.mode == "early"
            && (!self.flag("detection.early_switch", true)
                || strokes.len() < self.early_switch_policy().minimum_length
                || !self.early_prefix_protection(&original, plan.source_group, plan.target_group).is_empty()
                || !pystr::isalpha(&replacement))
        {
            return Ok(None);
        }
        let mut confidence = plan.confidence;
        if plan.mode == "early" {
            let (field, protection) = self.early_prefix_field(&original, &plan.application);
            if !protection.is_empty() {
                return Ok(None);
            }
            let scheduled = EarlySwitchDecision {
                should_switch: true,
                source_group: plan.source_group,
                target_group: plan.target_group,
                original: original.clone(),
                replacement: replacement.clone(),
                reason: "scheduled_prefix".to_owned(),
                source_evidence: None,
                target_evidence: None,
            };
            let (decision, decided) = self.decide_prefix(scheduled, Some(&field))?;
            if !decision.should_switch {
                return Ok(None);
            }
            confidence = decided;
        }
        Ok(Some(plan.replaced_with(|refreshed| {
            refreshed.strokes = strokes;
            refreshed.original = original;
            refreshed.replacement = replacement;
            refreshed.confidence = confidence;
        })))
    }

    fn log_early_switch(&self, decision: &EarlySwitchDecision, policy: &EarlySwitchPolicy, application: &str, excluded: bool) {
        if !self.shared.technical_logging() {
            return;
        }
        let mut payload = decision.as_dict();
        if excluded && let Some(map) = payload.as_object_mut() {
            map.insert("replacement".into(), json!("<redacted>"));
        }
        technical_event!(
            self.shared, "early_switch_evaluation",
            "original": if excluded { "<redacted>" } else { decision.original.as_str() },
            "prefix_length": pystr::len(&decision.original),
            "application": application,
            "application_excluded": excluded,
            "policy": policy.as_dict(),
            "decision": payload,
        );
    }

    pub(crate) fn late_stroke_after_early_switch(&self, event: &KeyEvent) -> bool {
        let group = i64::from(event.group);
        self.early_switch_at.is_some_and(|switched_at| {
            Some(group) == self.early_switch_origin && group != self.source_group && self.monotonic() - switched_at <= LATE_STROKE_GRACE_SECONDS
        })
    }

    /// Schedule the mixed prefix for replacement after all keys are up.
    pub(crate) fn convert_late_stroke(&mut self, event: &Stroke) -> Stroke {
        let target_group = self.source_group;
        let delay_ms = self.early_switch_at.map(|switched_at| ms(self.monotonic() - switched_at));
        let converted = key_of(KeyEvent {
            character: event.character_for(target_group.to::<i32>()).to_owned(),
            group: target_group.to::<i32>(),
            ..(**event).clone()
        });
        let mut strokes = self.strokes.clone();
        strokes.push(converted.clone());
        let original = format!("{}{}", text_for_group(&self.strokes, target_group), event.character);
        let replacement = text_for_group(&strokes, target_group);
        self.pending = Some(Arc::new(CorrectionPlan {
            mode: "late_stroke",
            ..CorrectionPlan::new(strokes, None, target_group, target_group, original, replacement, EARLY_SWITCH_CONFIDENCE, &self.backend.active_application())
        }));
        self.pending_trigger_keycode = i64::from(event.keycode);
        self.learning_prompt_after = None;
        technical_event!(
            self.shared, "late_stroke_scheduled",
            "source_group": event.group, "target_group": target_group, "delay_ms": delay_ms,
        );
        converted
    }

    /// Remember that the engine itself just switched the layout.
    pub(crate) fn note_engine_switch(&mut self, group: i64) {
        self.engine_switch_at = Some(self.monotonic());
        self.engine_switch_group = Some(group);
        // The layout the engine chose supersedes the user's earlier manual
        // pick; otherwise that pick would revive when the engine returned to
        // its group minutes later.
        self.manual_layout_group = None;
    }

    /// A layout asked for and not confirmed in time is still the engine's when it arrives.
    ///
    /// The request stays posted. VS Code confirmed one a second after it was asked for, the
    /// correction had been given up by then, and the layout that came was booked as the
    /// user's own pick: it protected the next word from correction.
    pub(crate) fn note_unconfirmed_switch(&mut self, error: &ks_backend::BackendError) {
        if let ks_backend::BackendError::LayoutSwitchUnconfirmed { group, .. } = error {
            self.note_engine_switch(i64::from(*group));
        }
    }

    /// The current word must be neither corrected nor switched early.
    pub(crate) fn word_protected(&self, source_group: i64) -> bool {
        self.early_switch_undone || (self.flag("detection.respect_manual_layout", true) && self.manual_layout_group == Some(source_group))
    }

    pub(crate) fn protection_details(&self) -> Value {
        json!({
            "reason": if self.early_switch_undone { "early_switch_undone" } else { "manual_layout" },
            "group": self.manual_layout_group,
            "source": self.manual_layout_source,
            "observed_ms_ago": self.manual_layout_observed_at.map(|observed_at| ms(self.monotonic() - observed_at)),
        })
    }

    /// Record the completed word of an early switch as one correction.
    ///
    /// `strokes` and `trailing` are the committed word exactly as `commit_word`
    /// recorded it: Pause, undo and Backspace delete `strokes + trailing + boundary`,
    /// so a literal comma left out here leaves one letter of the word on screen.
    pub(crate) fn finish_early_switch(&mut self, strokes: Strokes, boundary: Option<Stroke>, application: &str, final_group: i64, trailing: Strokes) {
        let origin = self.early_switch_origin.take();
        self.early_switch_at = None;
        let Some(origin) = origin else {
            return;
        };
        let original = text_for_group(&strokes, origin);
        let replacement = text_for_group(&strokes, final_group);
        let plan = CorrectionPlan {
            mode: "early",
            trailing: trailing.clone(),
            ..CorrectionPlan::new(strokes.clone(), boundary.clone(), origin, final_group, original.clone(), replacement.clone(), self.early_switch_confidence, application)
        };
        let plan = Arc::new(plan);
        self.remember_correction(&plan);
        self.last_committed = Some(Arc::new(CorrectionPlan {
            automatic: false,
            trailing,
            ..CorrectionPlan::new(strokes.clone(), boundary, final_group, origin, replacement.clone(), original.clone(), self.early_switch_confidence, application)
        }));
        self.last_committed_stale = false;
        let excluded = self.application_excluded(application);
        technical_event!(
            self.shared, "early_switch_completed",
            "original": if excluded { "<redacted>" } else { original.as_str() },
            "replacement": if excluded { "<redacted>" } else { replacement.as_str() },
            "source_group": origin,
            "target_group": final_group,
            "application": application,
            "application_excluded": excluded,
            "word_length": strokes.len(),
        );
        self.shared.update(Update {
            correction_count: Some(self.shared.correction_count() + 1),
            last_action: Some(format!("{original} → {replacement}")),
            ..Update::default()
        });
        self.record_history(&original, &replacement, application, EARLY_SWITCH_CONFIDENCE);
        for callback in self.shared.correction_callbacks() {
            callback(&plan);
        }
    }

    /// Store a correction the screen already shows; a failed write stops nothing.
    ///
    /// The text was replaced before this runs, so a full or locked disk is reported
    /// and the bookkeeping after it (undo, callbacks, the learning prompt) goes on.
    pub(crate) fn record_history(&self, original: &str, replacement: &str, application: &str, confidence: f64) {
        if !self.flag("general.keep_history", true) {
            return;
        }
        if let Err(error) = self.history.append(&ks_config::history::HistoryEntry::create(original, replacement, application, confidence)) {
            technical_event!(self.shared, "history_write_failed", "error": os_error_name(&error));
            self.shared.update(Update { last_error: Some(format!("История не записана: {error}")), ..Update::default() });
        }
    }

    // -- Typo correction (typo_correction.py): one slip in a finished word, undone in the word's own injection --
    /// The corrector while `detection.correct_typos` is on; its lists load once per process, on first use.
    pub(crate) fn typo_corrector(&mut self) -> EngineResult<Option<Arc<TypoCorrector>>> {
        if !self.flag("detection.correct_typos", true) {
            return Ok(None);
        }
        if self.typo.is_none() {
            let locales: Vec<&str> = self.models.iter().map(|model| model.locale.as_str()).collect();
            self.typo = Some(Arc::new(TypoCorrector::packaged(&self.resource_dir, &locales)?));
        }
        Ok(self.typo.clone())
    }

    /// `base` with the slip in its word undone, or None.
    ///
    /// `base` is the word's final plan: kept as typed (both layouts the same) or converted, and the reading of `keys`,
    /// the keys typed, in its target layout is the word judged. The plan keeps the layouts of `base`, so a word typed
    /// in the wrong layout and misspelt (`yhjvfkmyj`) is `нормально` after one injection, not two. A capital opens
    /// the word only at the start of a message or a sentence: one in the middle of a sentence is a name. A word whose
    /// correction the user took back (`schedule_undo`) is left as typed from then on.
    pub(crate) fn typo_plan(&mut self, base: &PlanRef, keys: &[Stroke]) -> EngineResult<Option<PlanRef>> {
        let corrector = self.typo_corrector()?;
        let group = base.target_group;
        let Some(corrector) = corrector else {
            return Ok(None);
        };
        if self.model(group).locale == TYPO_ENGLISH_LOCALE && !self.flag("detection.correct_typos_english", false) {
            // English slips are corrected only on request (constants.typo.TYPO_ENGLISH_LOCALE).
            return Ok(None);
        }
        let word = text_for_group(keys, group);
        self.personal_pending = String::new();
        if self.personal.contains(&word) || (self.flag("detection.learning", true) && self.learning.keeps(group, &word)) {
            // The word is the user's own (personal_words), or a rule of theirs keeps it as it is.
            return Ok(None);
        }
        let readings: Vec<(String, String)> = self
            .other_groups(group)
            .into_iter()
            .map(|other| (self.model(other).locale.clone(), text_for_group(keys, other)))
            .collect();
        let reading_refs: Vec<(&str, &str)> = readings.iter().map(|(locale, text)| (locale.as_str(), text.as_str())).collect();
        // The observed text says where a sentence begins; with the context off nothing does, and a capital is left.
        let observed = self.flag("detection.context_aware", true) && self.text_setting("detection.context_policy", "assist") != "off";
        let before_word = self.context_policy.stream.before_word(&base.original);
        let before = pystr::rstrip(&before_word);
        let excluded: HashSet<String> = self.words_setting("exclusions.words").iter().map(|item| LanguageDetector::token_key(item)).collect();
        let lexicons: Vec<&ks_lang::WordTable> = self.models.iter().map(|model| &*model.frequencies).collect();
        let terms = self.term_frequency()?;
        let identifiers = self.context_policy.shared_identifiers();
        let protected = self.detector.protected_tokens.clone();
        let capital_allowed = observed && (before.is_empty() || before.chars().next_back().is_some_and(|last| SENTENCE_END_SIGNS.contains(last)));
        let judgement = corrector.judge(&word, &self.model(group).locale, &reading_refs, capital_allowed, &|text| {
            known_elsewhere(text, &lexicons, &terms, identifiers, &protected, &|lower| excluded.contains(lower))
        });
        drop(lexicons);
        let reason_name = judgement.reason.as_str();
        if matches!(reason_name, "no_candidate" | "rare" | "ambiguous" | "corrected") {
            // No list knows the word: left as typed, it counts towards the user's own (personal_word_settled).
            self.personal_pending = word.clone();
        }
        let Some(choice) = judgement.choice.as_ref() else {
            return Ok(None);
        };
        let mut reason = reason_name;
        let typed: Vec<KeyEvent> = keys.iter().map(|key| (**key).clone()).collect();
        let mut strokes = if reason == "corrected" { corrected_keys(&typed, group.to::<i32>(), choice, &self.typo_keys) } else { None };
        if reason == "corrected" && strokes.is_none() {
            reason = "no_key";
        } else if strokes.as_ref().is_some_and(|strokes| strokes.len() != keys.len()) && !self.backend.erases_any_length() {
            // This backend erases as many characters as it types: a letter left out or typed in excess stays.
            strokes = None;
            reason = "backend_erases_typed_length";
        }
        // Lengths, the slip and orders of magnitude only: the words are the user's text.
        let letters = pystr::len(&word);
        let operation = choice.edit.operation.as_str();
        let count_order = judgement.count_order();
        let Some(strokes) = strokes else {
            technical_event!(
                self.shared, "typo_correction_declined", "reason": reason, "letters": letters, "operation": operation,
                "candidates": judgement.candidates, "count_order": count_order, "source_group": base.source_group, "target_group": group,
            );
            return Ok(None);
        };
        self.personal_pending = String::new();
        technical_event!(
            self.shared, "typo_correction_planned", "position": choice.edit.position,
            "margin": if choice.margin.is_finite() { logged(choice.margin) } else { None },
            "letters": letters, "operation": operation, "candidates": judgement.candidates, "count_order": count_order,
            "source_group": base.source_group, "target_group": group,
        );
        let converted = base.source_group != group;
        let margin = choice.margin;
        let replacement = choice.word.clone();
        let keys = keys.to_vec();
        Ok(Some(base.replaced_with(|plan| {
            plan.strokes = strokes.into_iter().map(Arc::new).collect();
            plan.replaced_strokes = keys;
            plan.replacement = replacement;
            plan.mode = "typo";
            plan.revisit = None;
            plan.automatic = true;
            if !converted {
                plan.confidence = 1.0 / (1.0 + (-margin).exp());
            }
        })))
    }

    /// The word judged at its boundary (`typo_plan`) was left as typed, or the user went back to it.
    ///
    /// Typing on into the next word, or Enter or Tab delivered after it, leaves it; Backspace into it, Pause, undo, a
    /// click or the caret moved away go back to it, or may.
    pub(crate) fn personal_word_settled(&mut self, kept: bool) {
        let word = std::mem::take(&mut self.personal_pending);
        if !word.is_empty() && kept && self.personal.keep(&word) {
            technical_event!(self.shared, "personal_word_added", "reason": "left_as_typed", "letters": pystr::len(&word));
        }
    }

    /// Whether the last correction is a typo correction that undo can still take back (`schedule_undo`).
    pub(crate) fn typo_correction_reversible(&self) -> bool {
        let (Some(previous), Some(committed)) = (&self.last_correction, &self.last_committed) else {
            return false;
        };
        previous.mode == "typo"
            && self.strokes.is_empty()
            && self.symbol_strokes.is_empty()
            && !self.last_committed_stale
            && committed.strokes == previous.strokes
            && committed.trailing == previous.trailing
            && committed.boundary == previous.boundary
            && self.monotonic() - self.last_correction_time <= UNDO_AVAILABLE_WINDOW_SECONDS
    }
}
