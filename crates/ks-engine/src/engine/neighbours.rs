//! A word and the words next to it: waiting for the next word, taking the kept words along, going back with it.

use super::logging::technical_event;
use super::*;
use ks_context::context_action_features::{cyrletter_question, resume_question, sign_token_asked, term_question, variable_question};
use ks_context::context_model::{AfterOrigin, BeforeOrigin};
use ks_context::context_policy::{DecideOptions as PolicyOptions, STRANDED_SHORT_WORD_REASON};
use ks_context::input_context::CONTEXT_TTL;
use ks_lang::intent_model::CorrectionTrigger;
use ks_lang::ortho_model::{shape_of, OrthoEvidence};
use ks_lang::short_words::{is_short_word_override, ISOLATED_SHORT_WORD_REASON};
use serde_json::json;

impl EngineCore {
    /// The model converted a short word that opens the message, with nothing before it.
    pub fn opening_conversion(&self, inside: Option<&DetectionDecision>, original: &str) -> bool {
        let Some(result) = self.context_result.as_ref().filter(|_| inside.is_none()) else {
            return false;
        };
        result.decision_source == "context_model"
            && result.decision.should_convert
            && result.field.as_ref().is_some_and(|field| pystr::strip(&field.before).is_empty())
            && pystr::len(&LanguageModel::normalize(original)) <= TRUSTED_SHORT_WORD_MAX_LENGTH
    }

    /// Let a word at a space wait for its next word; say whether it waits.
    ///
    /// The model asks for it with a ``wait`` verdict, whatever the word's length:
    /// `tot` at the start of a message may be `еще`, and only its neighbour can
    /// tell. A ``suggest`` verdict - the model leans to converting but is not sure
    /// enough - waits too, for the same neighbour: `vs` after a Russian sentence
    /// was left as it was with p=0.984 even when the next word turned out to be
    /// `хотим` typed in the same wrong layout. Such a word is only asked again
    /// with its neighbour, not at a pause. A lone letter converted by the
    /// message-start rule waits as well, whatever the model said: if the user
    /// keeps typing, the immediate correction aborts as unsafe and the next word
    /// has to decide the letter; if it went through, the observed text no longer
    /// matches and the wait cancels itself. A token of signs the other layout
    /// types the same (`-`) has nothing to wait for: it stands among the kept
    /// words instead (`keep_for_next_word`).
    pub fn start_context_wait(
        &mut self, plan: &PlanRef, decision: &DetectionDecision, boundary: &Stroke, trailing: &[Stroke], head: usize, fallback: bool,
    ) -> bool {
        let Some(result) = self.context_result.clone() else {
            return false;
        };
        let (Some(prediction), Some(field)) = (&result.prediction, &result.field) else {
            return false;
        };
        let action = prediction.action.as_str();
        let settles = action == "wait" || fallback;
        if (settles || action == "suggest")
            && boundary.character == " "
            && trailing.is_empty()
            && head == 0
            && !boundary.deferred
            && !Self::_reads_alike(plan)
            && self.context_assists()
        {
            self.context_wait_sequence += 1;
            // A word waits as long as the text around it stays context. The wait
            // used to lapse after ten seconds, so `tot`, a thought and then `ghbdtn`
            // twelve seconds later gave `tot привет` (0.31.0 log, 24.09.2026).
            // Anything that makes the pair stale ends the wait sooner: another
            // window, a caret move, Backspace, a changed field.
            self.context_waiting = Some(WaitingContextWord {
                plan: plan.clone(),
                decision: decision.clone(),
                field: field.clone(),
                window: self.focus_or_zero(),
                deadline: self.monotonic() + CONTEXT_TTL,
                diagnostic_id: self.context_wait_sequence,
                settles_on_pause: settles,
                baseline: self.last_baseline.clone(),
                converted: fallback,
            });
            // The pause that settles this wait (settle_context_wait_after_pause) is
            // counted from its boundary. The commit has just cleared the word timer,
            // and nothing else would set it until the next key, so without this the
            // pause never settled anything. The pause correction stays off.
            self.last_word_input_at = Some(self.monotonic());
            self.log_context_wait("context_wait_started", self.context_waiting.as_ref(), if settles { "model_wait" } else { "model_suggest" }, &[]);
            return true;
        }
        false
    }

    /// Whether the context model may settle words with their neighbours (`detection.context_policy` is `assist`).
    #[must_use]
    pub fn context_assists(&self) -> bool {
        self.text_setting("detection.context_policy", "assist") == "assist"
    }

    pub fn cancel_context_wait(&mut self, reason: &str) {
        self.log_context_wait("context_wait_cancelled", self.context_waiting.as_ref(), reason, &[]);
        self.context_waiting = None;
        // Whatever ends a wait - another field, a caret move, Backspace, a manual
        // conversion - also ends a converted or kept word's claim on the next word.
        self.provisional = None;
        self.paused_conversion = None;
        self.kept = Vec::new();
    }

    /// Decide a waiting word together with the next word, at its boundary or at a pause.
    ///
    /// `switched` says the early switch converted the next word while it was typed: its keys
    /// are in two layouts, and `decision` is that conversion, from the waiting word's layout.
    pub fn resolve_context_wait(
        &mut self, waiting: Option<&WaitingContextWord>, strokes: &[Stroke], boundary: Option<&Stroke>, decision: &DetectionDecision,
        application: &str, alternatives: &Alternatives, switched: bool,
    ) -> EngineResult<Option<(PlanRef, DetectionDecision)>> {
        if waiting.is_none() || !self.context_assists() {
            self.log_context_wait("context_wait_cancelled", waiting, "policy_disabled", &[]);
            return Ok(None);
        }
        let waiting = waiting.expect("checked above");
        let previous = waiting.plan.clone();
        let now = self.monotonic();
        let focus = self.focus_or_zero();
        let same_keys = |group: i64, keys: &[Stroke]| keys.iter().all(|stroke| i64::from(stroke.group) == group);
        if now > waiting.deadline
            || waiting.window != focus
            || previous.application != application
            || previous.boundary.is_none()
            || previous.source_group != decision.source_group
            || !same_keys(previous.source_group, &previous.strokes)
            || (!switched && !same_keys(previous.source_group, strokes))
        {
            self.log_context_wait(
                "context_wait_cancelled", Some(waiting), "preconditions_changed",
                &[
                    ("expired", json!(now > waiting.deadline)),
                    ("same_window", json!(waiting.window == focus)),
                    ("same_application", json!(previous.application == application)),
                    ("boundary_available", json!(previous.boundary.is_some())),
                    ("same_layout", json!(previous.source_group == decision.source_group)),
                    ("strokes_same_layout", json!(same_keys(previous.source_group, &previous.strokes) && same_keys(previous.source_group, strokes))),
                ],
            );
            return Ok(None);
        }
        let previous_boundary = previous.boundary.clone().expect("checked above");
        let mut decision = decision.clone();
        if !decision.should_convert {
            // The next word was judged after the waiting word as it was typed, and
            // that word is exactly what is in doubt: `tot d` reads as English only
            // while `tot` is taken for English. The model is asked about the next
            // word once more after the waiting word's other reading, `еще в`; the
            // pair converts only if it then converts the next word and, below, the
            // waiting word with it.
            if let Some(converted) = self.decide_after_waiting_word(waiting, &decision, alternatives, boundary, &previous_boundary.character)? { decision = converted } else {
                self.log_context_wait("context_wait_cancelled", Some(waiting), "next_word_not_converted", &[]);
                return Ok(None);
            }
        }
        let original = format!("{}{}{}", previous.original, previous_boundary.character, decision.original);
        // At a pause the next word has no boundary yet; Enter/Tab has not reached the editor.
        let closing = boundary.filter(|boundary| !boundary.deferred).cloned();
        let suffix = closing.as_ref().map_or("", |closing| closing.character.as_str());
        if !self.context_policy.stream.text.ends_with(&format!("{original}{suffix}")) {
            self.log_context_wait("context_wait_cancelled", Some(waiting), "observed_suffix_changed", &[]);
            return Ok(None);
        }
        let group = decision.target_group;
        let alternative = text_for_group(&previous.strokes, group);
        // A feature-version-3 context model re-decides the waiting word with the
        // converted next word as its language context, exactly as a word after an
        // existing neighbour would be; the installed feature-version-2 model keeps
        // the decision it was certified with.
        let new_features = self.context_policy.model.as_ref().is_some_and(|model| model.feature_version == CONTEXT_ACTION_FEATURE_VERSION.to::<i64>());
        let planned_baseline = if new_features {
            self.planned_baseline(&previous.original, &vec![(group, alternative.clone())], previous.source_group, &decision.replacement, group)?
        } else {
            waiting.decision.clone()
        };
        let result = self.context_policy.decide(&planned_baseline, &alternative, group, &self.detector, "space", "assist", &PolicyOptions {
            after: &decision.replacement,
            field_override: Some(waiting.field.clone()),
            boundary_text: &previous_boundary.character,
            after_origin: Self::_neighbour_origin(&previous.original),
            ..PolicyOptions::default()
        })?;
        let mut pair_decision = result.decision;
        if !pair_decision.should_convert
            && planned_baseline.should_convert
            && (waiting.decision.reason == ISOLATED_SHORT_WORD_REASON
                || (pystr::len(&previous.original) == 1 && is_short_word_override(&planned_baseline)))
        {
            // The lone letter opening a message is the one curated rule the model does not
            // arbitrate (context_policy.decide): every single-letter row of its corpus sits in
            // quarantine, so its planned answer is no opinion either. The rule converted the
            // letter at its space and only its neighbour's layout was still in question; with
            // the neighbour converted the same way, the detector's verdict on the pair stands,
            // as it did under the feature-version-2 model (`z ctujlyz` ends as `я сегодня`).
            pair_decision = planned_baseline;
        }
        if !pair_decision.should_convert {
            self.log_context_wait("context_wait_cancelled", Some(waiting), "lookahead_not_converted", &[]);
            return Ok(None);
        }
        technical_event!(
            self.shared, "context_wait_resolved", "wait_id": waiting.diagnostic_id,
            "previous_characters": pystr::len(&previous.original), "next_characters": pystr::len(&decision.original),
        );
        let mut keys = previous.strokes.clone();
        keys.push(previous_boundary.clone());
        keys.extend(strokes.iter().cloned());
        let typed = if switched { text_for_group(&keys, previous.source_group) } else { String::new() };
        let replacement = format!("{alternative}{}{}", previous_boundary.character, decision.replacement);
        let plan = Arc::new(CorrectionPlan {
            mode: "context_phrase",
            context_field: self.context_field_id(),
            typed,
            ..CorrectionPlan::new(keys, closing, previous.source_group, group, original, replacement, pair_decision.confidence, application)
        });
        Ok(Some((plan, decision)))
    }

    /// A waiting word whose next word stayed, asked once more with that word after it.
    ///
    /// The pair converts together only when the next word converts too, and a term typed in the
    /// Russian layout amid Russian prose has a next word that is right as typed: `зк` stayed in
    /// `есть новые зк проверь`, `тзь сш` in `сам выполни тзь сш` (field typing, 0.38 and
    /// 0.39). The model is asked about the waiting word with the kept word as its right context,
    /// the `kept_next_word` question it has weights and frames of its own for, and a conversion
    /// it is sure of replaces that word alone: the next word is typed again in its own layout,
    /// and the layout stays there.
    pub fn decide_with_kept_neighbour(
        &mut self, waiting: &WaitingContextWord, strokes: &[Stroke], boundary: &Stroke, decision: &DetectionDecision, application: &str,
    ) -> EngineResult<Option<PlanRef>> {
        let previous = waiting.plan.clone();
        let Some(baseline) = &waiting.baseline else {
            return Ok(None);
        };
        let Some(previous_boundary) = previous.boundary.clone().filter(|boundary| boundary.character == " ") else {
            return Ok(None);
        };
        let new_features = self.context_policy.model.as_ref().is_some_and(|model| model.feature_version == CONTEXT_ACTION_FEATURE_VERSION.to::<i64>());
        let length = pystr::len(&previous.original);
        if !new_features
            || boundary.character != " "
            || boundary.deferred
            || !(0 < length && length <= KEPT_CONTEXT_WORD_MAX_CHARACTERS)
            || decision.original.is_empty()
            || decision.original.chars().any(char_isspace)
            || self.monotonic() > waiting.deadline
            || waiting.window != self.focus_or_zero()
            || previous.application != application
            || previous.source_group != decision.source_group
            || previous.strokes.iter().chain(strokes).any(|stroke| i64::from(stroke.group) != previous.source_group)
        {
            return Ok(None);
        }
        let original = format!("{}{}{}", previous.original, previous_boundary.character, decision.original);
        if !self.context_policy.stream.text.ends_with(&format!("{original}{}", boundary.character)) {
            return Ok(None);
        }
        let group = previous.target_group;
        let alternative = text_for_group(&previous.strokes, group);
        let result = self.context_policy.decide(baseline, &alternative, group, &self.detector, "space", "assist", &PolicyOptions {
            after: &decision.original,
            field_override: Some(waiting.field.clone()),
            boundary_text: &previous_boundary.character,
            after_origin: AfterOrigin::KeptNextWord,
            ..PolicyOptions::default()
        })?;
        if !result.decision.should_convert {
            self.log_context_wait("context_wait_cancelled", Some(waiting), "kept_neighbour_declined", &[]);
            return Ok(None);
        }
        technical_event!(
            self.shared, "context_wait_resolved", "wait_id": waiting.diagnostic_id,
            "previous_characters": length, "next_characters": pystr::len(&decision.original),
            "next_word_kept": true,
        );
        Ok(Some(self.word_before_kept(&previous, strokes, boundary, group, &alternative, &decision.original, result.decision.confidence, application)))
    }

    /// A waiting word whose next word was typed in the layout the user picked for it by hand, the layout the waiting
    /// word was suspected to belong to.
    ///
    /// The picked layout keeps its word as typed, and that word is the very context the waiting word waited for:
    /// `olegius888 ? z ` waits on `z`, the Russian layout is picked, and `ожидал` is typed in it (field typing,
    /// 10.11.2026), where `z ` stayed for good because the wait was dropped with the analysis of the protected word. The
    /// model is asked the question it is asked when the program converts the next word itself (`resolve_context_wait`):
    /// the next word is in the waiting word's other language either way. A conversion it is sure of replaces the
    /// waiting word alone; the picked word is typed again as it was, and the picked layout stays.
    ///
    /// `strokes` are the keys of the whole picked token, a sign after the word included; `word` is the word alone, what the
    /// model is asked with. `analyzable` says the word could be decided at all (the program on, its boundary a trigger,
    /// the caret where the engine saw it): only the layout picked for it held its analysis back.
    pub fn decide_before_picked_word(
        &mut self, waiting: &WaitingContextWord, strokes: &[Stroke], boundary: &Stroke, word: &str, picked: i64, application: &str,
        analyzable: bool,
    ) -> EngineResult<Option<PlanRef>> {
        let previous = waiting.plan.clone();
        let Some(previous_boundary) = previous.boundary.clone().filter(|boundary| boundary.character == " ") else {
            return Ok(None);
        };
        let new_features = self.context_policy.model.as_ref().is_some_and(|model| model.feature_version == CONTEXT_ACTION_FEATURE_VERSION.to::<i64>());
        if !analyzable
            || self.application_excluded(application)
            || !new_features
            || waiting.converted
            || !self.context_assists()
            || previous.target_group != picked
            || boundary.deferred
            || word.is_empty()
            || word.chars().any(char_isspace)
            || self.monotonic() > waiting.deadline
            || waiting.window != self.focus_or_zero()
            || previous.application != application
            || previous.strokes.iter().any(|stroke| i64::from(stroke.group) != previous.source_group)
            || strokes.iter().any(|stroke| i64::from(stroke.group) != picked)
        {
            return Ok(None);
        }
        let following = text_for_group(strokes, picked);
        let original = format!("{}{}{following}", previous.original, previous_boundary.character);
        if !self.context_policy.stream.text.ends_with(&format!("{original}{}", boundary.character)) {
            return Ok(None);
        }
        let group = previous.target_group;
        let alternative = text_for_group(&previous.strokes, group);
        let baseline = self.planned_baseline(&previous.original, &vec![(group, alternative.clone())], previous.source_group, word, group)?;
        let result = self.context_policy.decide(&baseline, &alternative, group, &self.detector, "space", "assist", &PolicyOptions {
            after: word,
            field_override: Some(waiting.field.clone()),
            boundary_text: &previous_boundary.character,
            after_origin: Self::_neighbour_origin(&previous.original),
            ..PolicyOptions::default()
        })?;
        if !result.decision.should_convert {
            // The caller logs the wait dropped with the analysis of the picked word, as it always did.
            return Ok(None);
        }
        technical_event!(
            self.shared, "context_wait_resolved", "wait_id": waiting.diagnostic_id,
            "previous_characters": pystr::len(&previous.original), "next_characters": pystr::len(word),
            "next_word_picked": true,
        );
        // The picked token's keys are in the target layout already: typed again in it they are what they were, and the
        // layout picked for them stays.
        let mut keys = previous.strokes.clone();
        keys.push(previous_boundary.clone());
        keys.extend(strokes.iter().cloned());
        Ok(Some(Arc::new(CorrectionPlan {
            mode: "context_word",
            context_field: self.context_field_id(),
            picked_tail: strokes.len(),
            ..CorrectionPlan::new(
                keys, Some(boundary.clone()), previous.source_group, group, original,
                format!("{alternative}{}{following}", previous_boundary.character), result.decision.confidence, application,
            )
        })))
    }

    /// The correction of a word converted alone before the token after it, which stays as typed.
    ///
    /// A token the other layout types the same (`+`, `=`, `2`) is no text of either language: typed again in the target
    /// layout it is what it was, so the correction covers it as well, and the layout follows the converted word. With
    /// the token kept in its own layout (`kept_tail`), `м + ` became `v + ` and the next letter of the formula came out
    /// Russian again: `v + ы`, and a lone letter there is refused. A word is typed again in its own layout, which stays.
    fn word_before_kept(
        &self, previous: &PlanRef, strokes: &[Stroke], boundary: &Stroke, group: i64, alternative: &str, following: &str, confidence: f64, application: &str,
    ) -> PlanRef {
        let previous_boundary = previous.boundary.clone().expect("a boundary");
        let alike = text_for_group(strokes, group) == following && !any_alpha(following);
        let mut keys = previous.strokes.clone();
        keys.push(previous_boundary.clone());
        keys.extend(strokes.iter().cloned());
        Arc::new(CorrectionPlan {
            mode: "context_word",
            context_field: self.context_field_id(),
            kept_tail: if alike { 0 } else { strokes.len() },
            ..CorrectionPlan::new(
                keys, Some(boundary.clone()), previous.source_group, group,
                format!("{}{}{following}", previous.original, previous_boundary.character),
                format!("{alternative}{}{following}", previous_boundary.character),
                confidence, application,
            )
        })
    }

    /// A letter kept at its space, asked once more when a token the other layout types the same follows it.
    ///
    /// `м` after `прохождения ` is kept: a Russian letter there is as likely meant as written (`да я про то ч`), and
    /// the model kept it at p=0.94. A sign after it is evidence the word itself never had: `м + ` is the formula
    /// `v + ` typed in the Russian layout (field typing, 19.10.2026, which he corrected with Pause). The letter
    /// is asked with the sign after it, the question a waiting letter is asked before its kept neighbour
    /// (`decide_with_kept_neighbour`), and only a letter of the class the variable and sign heads answer
    /// (`context_action_features.sign_question)`: a one-letter word of the Russian layout after a Cyrillic word, no
    /// Russian word of one letter, before a token that is no dash alone (a dash tells a minus from the dash of a
    /// sentence only by what follows it). The model decides; a conversion takes the sign along (`word_before_kept`).
    pub fn decide_letter_before_sign(
        &mut self, kept: &[Arc<KeptWord>], strokes: &[Stroke], boundary: &Stroke, decision: &DetectionDecision, application: &str,
    ) -> EngineResult<Option<PlanRef>> {
        let Some(word) = kept.last() else {
            return Ok(None);
        };
        let previous = word.plan.clone();
        let (Some(baseline), Some(field)) = (&word.baseline, &word.field) else {
            return Ok(None);
        };
        let Some(previous_boundary) = previous.boundary.clone().filter(|boundary| boundary.character == " ") else {
            return Ok(None);
        };
        let new_features = self.context_policy.model.as_ref().is_some_and(|model| model.feature_version == CONTEXT_ACTION_FEATURE_VERSION.to::<i64>());
        if !new_features
            || word.segmentation_open
            || boundary.character != " "
            || boundary.deferred
            || decision.original.is_empty()
            || decision.original.chars().any(char_isspace)
            || !sign_token_asked(&decision.original)
            || text_for_group(strokes, previous.target_group) != decision.original
            || !variable_question(&previous.original, &text_for_group(&previous.strokes, previous.target_group), &field.before, "space")
            || self.monotonic() > word.deadline
            || word.window != self.focus_or_zero()
            || previous.application != application
            || previous.source_group != decision.source_group
            || previous.strokes.iter().chain(strokes).any(|stroke| i64::from(stroke.group) != previous.source_group)
            || !self.context_policy.stream.text.ends_with(&format!(
                "{}{}{}{}", previous.original, previous_boundary.character, decision.original, boundary.character
            ))
        {
            return Ok(None);
        }
        let group = previous.target_group;
        let alternative = text_for_group(&previous.strokes, group);
        let result = self.context_policy.decide(baseline, &alternative, group, &self.detector, "space", "assist", &PolicyOptions {
            after: &decision.original,
            field_override: Some(field.clone()),
            boundary_text: &previous_boundary.character,
            after_origin: AfterOrigin::KeptNextWord,
            ..PolicyOptions::default()
        })?;
        technical_event!(
            self.shared, "kept_letter_asked", "previous_characters": pystr::len(&previous.original),
            "next_characters": pystr::len(&decision.original), "converted": result.decision.should_convert,
        );
        if !result.decision.should_convert {
            return Ok(None);
        }
        Ok(Some(self.word_before_kept(&previous, strokes, boundary, group, &alternative, &decision.original, result.decision.confidence, application)))
    }

    /// A word converted at a pause and kept at its space, as the conversion a next word may take back.
    ///
    /// A correction at a pause leaves the word open, so its space commits the converted reading as a
    /// word of its own, which the model keeps; the conversion at a space is asked again when the next
    /// word goes back the other way (`revert_with_next_word`), and this one was not. In the sealed
    /// sequences every word pauses before its space: an English message opening with a token whose
    /// sign is a Russian letter's key was converted there, the next English word typed in the Russian
    /// layout went back, and the first word stayed Cyrillic (test v47). The plan is the pause's, ended
    /// by this space; only the very word converted, with nothing typed onto it, qualifies.
    pub fn provisional_after_pause(
        &self, paused: Option<&ProvisionalWord>, strokes: &[Stroke], source_group: i64, boundary: &Stroke,
    ) -> Option<ProvisionalWord> {
        let paused = paused?;
        if boundary.character != " "
            || boundary.deferred
            || self.monotonic() > paused.deadline
            || paused.window != self.focus_or_zero()
            || source_group != paused.plan.target_group
            || strokes != &paused.plan.strokes[..]
        {
            return None;
        }
        let boundary = boundary.clone();
        Some(ProvisionalWord { plan: paused.plan.replaced_with(|plan| plan.boundary = Some(boundary)), window: paused.window, deadline: paused.deadline })
    }

    /// How a word asked again with its converted neighbour names that right context.
    ///
    /// The model is trained on `planned_next_conversion` frames for the words it defers,
    /// up to `PLANNED_CONTEXT_WORD_MAX_CHARACTERS` letters, and its features refuse a planned
    /// frame of a longer word. A longer word that waited (`руку`, then `they`) or that was
    /// converted and is revisited (`ghbdtn`, then `hello`) is asked with the converted
    /// neighbour as the right context the field will hold, the `field` origin it was
    /// trained on for words of every length; asking the refused frame returned `suggest`,
    /// which cancelled the wait and took the converted word back (02.10.2026).
    fn _neighbour_origin(word: &str) -> AfterOrigin {
        let length = pystr::len(word);
        if 0 < length && length <= PLANNED_CONTEXT_WORD_MAX_CHARACTERS { AfterOrigin::PlannedNextConversion } else { AfterOrigin::Field }
    }

    /// The next word's decision after the waiting word's other reading.
    ///
    /// The detector gets that reading as its previous word and the model gets it
    /// in front of the caret, as they would after a word typed in that layout, and
    /// the model is told that the reading is the engine's hypothesis, not text that
    /// was typed (`before_origin)`: read as typed text, a reading of English letters
    /// before a word that is no word of either layout turned it into its Latin
    /// reading, `тыв` after `tot ` into `nsd` (`еще тыв`, field typing). Nothing
    /// is decided here that the model does not decide.
    fn decide_after_waiting_word(
        &self, waiting: &WaitingContextWord, decision: &DetectionDecision, alternatives: &Alternatives, boundary: Option<&Stroke>, boundary_character: &str,
    ) -> EngineResult<Option<DetectionDecision>> {
        let previous = &waiting.plan;
        let group = previous.target_group;
        // A word waiting as it was typed has its own layout for a target, and no reading in it: the worker forgets the word.
        let alternative = alternatives.iter().find(|entry| entry.0 == group).map(|entry| entry.1.clone()).ok_or_else(|| EngineError(group.to_string()))?;
        let reading = text_for_group(&previous.strokes, group);
        let baseline = self.planned_baseline(&decision.original, &vec![(group, alternative.clone())], decision.source_group, &reading, group)?;
        let trigger = boundary.map_or(CorrectionTrigger::Pause, |boundary| Self::_trigger_for_boundary(boundary));
        let field = field_with(&waiting.field, format!("{}{reading}{boundary_character}", waiting.field.before), waiting.field.after.clone());
        let result = self.context_policy.decide(&baseline, &alternative, group, &self.detector, trigger.as_str(), "assist", &PolicyOptions {
            field_override: Some(field),
            before_origin: BeforeOrigin::PlannedPreviousReading,
            ..PolicyOptions::default()
        })?;
        Ok(result.decision.should_convert.then_some(result.decision))
    }

    /// Ask the model about a converted word again when the next word goes the other way.
    ///
    /// `ns` alone becomes `ты` and the layout follows, so the next word is typed in
    /// Russian. When the engine then converts that word into the language `ns` was
    /// typed in - `vs` and `code` come out as `мы сщву` - the first word is decided
    /// once more with the converted next word after it, exactly as a waiting word
    /// is (`resolve_context_wait`). If the model no longer converts it, one
    /// correction returns both: `vs code`. The engine only asks; the model decides.
    /// Only a word that opened its text is asked again.
    pub fn revert_with_next_word(
        &mut self, provisional: Option<&ProvisionalWord>, strokes: &[Stroke], boundary: Option<&Stroke>, decision: &DetectionDecision, application: &str,
    ) -> EngineResult<Option<(PlanRef, DetectionDecision)>> {
        let Some(provisional) = provisional else {
            return Ok(None);
        };
        if !decision.should_convert || !self.context_assists() {
            return Ok(None);
        }
        let applied = provisional.plan.clone();
        let (Some(revisit), Some(applied_boundary)) = (applied.revisit.clone(), applied.boundary.clone()) else {
            return Ok(None);
        };
        if decision.target_group != applied.source_group
            || self.monotonic() > provisional.deadline
            || provisional.window != self.focus_or_zero()
            || applied.application != application
            || strokes.iter().any(|stroke| i64::from(stroke.group) != applied.target_group)
        {
            return Ok(None);
        }
        let original = format!("{}{}{}", applied.replacement, applied_boundary.character, decision.original);
        let closing = boundary.filter(|boundary| !boundary.deferred).cloned();
        if !self.context_policy.stream.text.ends_with(&format!("{original}{}", closing.as_ref().map_or("", |closing| closing.character.as_str()))) {
            return Ok(None);
        }
        let (baseline, field) = &*revisit;
        let tables = self.term_frequency()?;
        if baseline.reason == STRANDED_SHORT_WORD_REASON
            || term_question(&applied.original, field.as_ref().map_or("", |field| field.before.as_str()), &tables)
        {
            // A Russian word of one letter the rule set after an English term in a Russian phrase (`sendapi и`), or a
            // term of the term head's class converted into Latin letters (`qemu`, `gpu`): the next word going the
            // other way is what a Russian phrase holds around a term, no sign that the word went back. Asked again,
            // the model took them back (`и botcorp` became `b botcorp`, `в dev` `d dev`, `qemu чтобы` `йуьг чтобы`,
            // `gpu но` `пзг но`, `принимай зк и` with `pr`; field typing, 0.43.0-0.59.0). A word of no such
            // class is still asked about: `кз в` went back to `кз`, and `rp` had been wrong.
            return Ok(None);
        }
        // The question is how to read the word now that its neighbour went the other
        // way, not what the detector thinks of it alone: it is asked with no baseline
        // conversion, as the model was taught it (train_context_model: short_revisit),
        // and no curated rule answers in the model's place.
        let baseline = DetectionDecision { should_convert: false, reason: REVISIT_REASON.to_owned(), ..baseline.clone() };
        let again = self.context_policy.decide(&baseline, &applied.replacement, applied.target_group, &self.detector, "space", "assist", &PolicyOptions {
            after: &decision.replacement,
            field_override: field.clone(),
            boundary_text: &applied_boundary.character,
            after_origin: Self::_neighbour_origin(&applied.replacement),
            ..PolicyOptions::default()
        })?;
        if again.decision.should_convert {
            return Ok(None);
        }
        technical_event!(
            self.shared, "converted_word_reverted", "previous_characters": pystr::len(&applied.original),
            "next_characters": pystr::len(&decision.original),
        );
        let mut keys = applied.strokes.clone();
        keys.push(applied_boundary.clone());
        keys.extend(strokes.iter().cloned());
        let replacement = format!("{}{}{}", applied.original, applied_boundary.character, decision.replacement);
        let plan = Arc::new(CorrectionPlan {
            mode: "context_phrase",
            context_field: self.context_field_id(),
            ..CorrectionPlan::new(keys, closing, applied.target_group, applied.source_group, original, replacement, decision.confidence, application)
        });
        Ok(Some((plan, decision.clone())))
    }

    /// Remember the words left as typed at a space until a later word shows their layout.
    pub fn keep_for_next_word(&mut self, kept: Vec<Arc<KeptWord>>, word: Option<Arc<KeptWord>>, boundary: &Stroke) {
        if boundary.character != " " || boundary.deferred {
            return;
        }
        if let Some(word) = word.as_ref().filter(|word| !any_alpha(&word.plan.original)) {
            // A token of signs alone (`.`, `1.2`, `...`) is never converted on its own, and a
            // converted next word does not make it a word either: `.` before `ghbdtn` is no `ю`.
            // One that reads the same in either layout, alone after a kept word, stands between
            // that word and the next as it is (take_along): `Ytn - ult` kept `Ytn` for good
            // once `-` had cut the words apart, and only `где` converted.
            if Self::_reads_alike(&word.plan) && kept.last().is_some_and(|last| !Self::_reads_alike(&last.plan)) {
                let mut chain = kept;
                chain.push(word.clone());
                self.kept = chain;
            }
            return;
        }
        let mut chain = kept;
        if let Some(word) = word.filter(|word| word.plan.target_group != word.plan.source_group) {
            chain.push(word);
        }
        let words: Vec<usize> = chain.iter().enumerate().filter(|(_, item)| !Self::_reads_alike(&item.plan)).map(|(index, _)| index).collect();
        self.kept = if words.len() > KEPT_WORDS_TAKEN_ALONG { chain.split_off(words[words.len() - KEPT_WORDS_TAKEN_ALONG]) } else { chain };
    }

    fn ortho_score(&self, plan: &CorrectionPlan, before: &str) -> Option<ks_lang::ortho_model::OrthoScore> {
        let ortho = self.context_policy.ortho.as_ref()?;
        let script = if plan.source_group == 0 { "en" } else { "ru" };
        // The model reads keys as the US layout types them, whichever layout typed the token.
        let keys = if script == "en" { plan.original.as_str() } else { plan.replacement.as_str() };
        let shape = shape_of(&plan.original, pystr::strip(before).is_empty());
        let known = |language: &str, word: &str| self.model(i64::from(language != "en")).score(word).known;
        Some(ortho.score(&OrthoEvidence { keys, shape, source_script: script }, Some(&known)))
    }

    /// Whether the orthotactic model reads a token's keys as the language of its other layout.
    ///
    /// The model of the policy's licence (`ContextPolicy`._licensed), asked about the whole token,
    /// signs at its end included: those are the keys of letters (`[` is `х`, `.` is `ю`).
    pub fn keys_of_target_language(&self, plan: &CorrectionPlan, before: &str) -> bool {
        let Some(ortho) = &self.context_policy.ortho else {
            return false;
        };
        let script = if plan.source_group == 0 { "en" } else { "ru" };
        let keys = if script == "en" { plan.original.as_str() } else { plan.replacement.as_str() };
        if pystr::len(keys).to::<i64>() < ortho.minimum_length {
            return false;
        }
        self.ortho_score(plan, before).is_some_and(|score| score.supported && score.total > *ortho.thresholds.get(script))
    }

    /// Whether the orthotactic model reads a token's keys, signs at its end included, as the language of its other
    /// layout more than as its own (a score above zero; keys_of_target_language asks for the policy's licence). A
    /// token shorter than the licence's minimum is read too, as the model supports it: `T\`` (`Её`) scores +37, the
    /// English `I.`, `A.`, `S.` from -10 to -31.
    pub fn keys_lean_to_target_language(&self, plan: &CorrectionPlan, before: &str) -> bool {
        self.ortho_score(plan, before).is_some_and(|score| score.supported && score.total > 0.0)
    }

    /// Ask the model about the words before a converted word again, nearest first.
    ///
    /// The first words of a line have nothing before them to tell their layout
    /// by: `руку` is a Russian word and `here` an English one, and `dc\`` is `всё` or
    /// `вс` with a backtick. When a word typed in the same layout converts - `ерун` is
    /// `they` - the model is asked about the word before it once more with the
    /// converted word after it, as a waiting word is (resolve_context_wait); while
    /// it converts them, one correction takes the words along: `hey here they`. The
    /// mirror of revert_with_next_word; the engine only asks, the model decides.
    ///
    /// Only the first words of a line are taken along: a word with words before it
    /// on its line was decided with them, and asked again it reads to the model like
    /// a first word. A Russian word typed as intended before a term typed in the other
    /// layout was converted then: `склонируй репо пшерги` would become
    /// `склонируй htgj github`. A short word right after Latin text, which a model with the
    /// resume head answers (`context_action_features.resume_question`), is taken along on its
    /// conversion wherever it stands on its line: a person who forgot to switch back to
    /// Russian after a term resumes Russian text there, and with Hunspell, which knows `f`
    /// and `z` as English words, `seek() , f там` and `TSet - z использовал` kept their
    /// first Russian word while the rest converted (the developer-chat development set,
    /// 29.10.2026). So is a Latin letter whose Cyrillic reading is a Russian word of one
    /// letter after Russian text, which a model with the Cyrillic-letter head answers
    /// (`context_action_features.cyrletter_question)`: `d` stayed in `разрабатывал и d документ`
    /// while `документ` converted (field typing, 30.10.2026). A token whose segmentation the boundary model left open
    /// (`KeptWord.segmentation_open`) was decided neither with the words before it nor at
    /// all, and is taken along wherever it stands on its line, on the model's own
    /// conversion and with the text before it read now: Russian words ending in a letter
    /// the English layout types as a sign stayed in the middle of every sentence
    /// (`в 2 vbkkbjyf[ cdtnjds[` converted only `световых`, though the model, asked,
    /// converts `миллионах` before it at p=1.0). A word the user excluded, or one a rule
    /// of theirs keeps, is never taken along: the model is not asked about it (`decide_word`).
    pub fn take_along(
        &mut self, kept: &[Arc<KeptWord>], plan: &PlanRef, first_characters: usize, application: &str, switched: bool,
    ) -> EngineResult<PlanRef> {
        if !self.context_assists() {
            return Ok(plan.clone());
        }
        let start = plan.clone();
        let mut plan = plan.clone();
        let mut next_replacement = pystr::slice(&plan.replacement, None, Some(first_characters.to::<i64>())).to_owned();
        // Enter/Tab has not reached the editor: it is neither in the text nor replaced. A sign
        // split off the converted word's end stays literal after the whole correction.
        let closing = plan.boundary.clone().filter(|boundary| !boundary.deferred);
        let mut suffix: String = plan.trailing.iter().map(|stroke| stroke.character.as_str()).collect();
        suffix.push_str(closing.as_ref().map_or("", |closing| closing.character.as_str()));
        let ignored: HashSet<String> = self.words_setting("exclusions.words").iter().map(|item| LanguageDetector::token_key(item)).collect();
        let mut settled = plan.clone();
        let mut taken: Vec<Arc<KeptWord>> = Vec::new();
        let mut resumed: Vec<Arc<KeptWord>> = Vec::new();
        for word in kept.iter().rev() {
            let previous = word.plan.clone();
            // A sign that reads the same in either layout (`-` in `Ytn - ult`) is passed as it
            // stands: the word before it is asked with the converted word after the sign, and
            // the sign goes with it only if that word converts.
            let passed = Self::_reads_alike(&previous);
            let Some(previous_boundary) = previous.boundary.clone() else {
                break;
            };
            let asked = format!("{}{}{}{suffix}", previous.original, previous_boundary.character, plan.original);
            if plan.source_group != previous.source_group
                || plan.target_group != previous.target_group
                || self.monotonic() > word.deadline
                || word.window != self.focus_or_zero()
                || previous.application != application
                || previous.strokes.iter().any(|stroke| i64::from(stroke.group) != previous.source_group)
                || (!switched && plan.strokes.iter().any(|stroke| i64::from(stroke.group) != previous.source_group))
                || !self.context_policy.stream.text.ends_with(&asked)
                // The user's exclusions and rules stand, as at the word's own space.
                || ignored.contains(&LanguageDetector::token_key(&previous.original))
                || self.rejected_targets(previous.source_group, &previous.original).contains(&previous.target_group)
            {
                break;
            }
            // A token that was not decided has no field of its own: the text before it is read now.
            let field = match &word.field {
                Some(field) => field.clone(),
                None => self.context_policy.stream.snapshot(&asked),
            };
            if !passed && !self.word_of_its_language(previous.target_group, &previous.replacement)? {
                // Only a word whose other reading is a word is asked: a term typed as intended before
                // a Russian word typed in the English layout (`htop gjrfpsdftn`) is no `рещз`, yet
                // asked with `показывает` after it the model converts it. A token whose segmentation
                // is open is asked as well when the orthotactic model reads its keys as the other
                // layout's language, as it does for words no lexicon holds (`vehpbkrf[` is
                // `мурзилках`) and does not for terms with a sign after them (`docs.`, `UI,`).
                if !(word.segmentation_open && self.keys_of_target_language(&previous, &field.before)) {
                    if word.segmentation_open {
                        technical_event!(
                            self.shared, "kept_word_not_asked", "previous_characters": pystr::len(&previous.original),
                            "reason": "keys_of_typed_language",
                        );
                    }
                    break;
                }
            }
            if !passed {
                let baseline = match &word.baseline {
                    Some(baseline) => baseline.clone(),
                    None => self.baseline_decision(
                        &previous.original, &vec![(previous.target_group, previous.replacement.clone())], previous.source_group, application,
                        CorrectionTrigger::Space, None,
                    )?,
                };
                let origin = Self::_neighbour_origin(&previous.replacement);
                let again = self.context_policy.decide(&baseline, &previous.replacement, previous.target_group, &self.detector, "space", "assist", &PolicyOptions {
                    after: &next_replacement,
                    field_override: Some(field.clone()),
                    boundary_text: &previous_boundary.character,
                    after_origin: origin,
                    ..PolicyOptions::default()
                })?;
                let action = again.prediction.as_ref().map(|prediction| prediction.action.as_str());
                // A token whose segmentation is open has no verdict of its own to confirm: the detector,
                // asked now, reads its sign as typed and the letter in the other reading - `it.` is no
                // English word to it, `шею` a Russian one - and its confirmation of a suggestion
                // (ContextPolicy.decide) does not take such a token along. The model's own conversion
                // or a curated short word does.
                // A suggestion of the model is confirmed for such a token by the orthotactic model instead, asked about
                // the whole token: its keys read as the other layout's language (`T\`` is `Её`, `dc.` is `всю`) and
                // not as the typed one's (`it.`, `ok.`). Unconfirmed, `Её платье` and `всю неделю` typed in the English
                // layout kept their first word (the development sentences of the Tatoeba dump, 22.10.2026).
                let suggested = word.segmentation_open && action == Some("suggest") && self.keys_lean_to_target_language(&previous, &field.before);
                if !suggested
                    && (!again.decision.should_convert
                        || (word.segmentation_open && !(again.decision_source == "short_word_override" || action == Some("convert"))))
                {
                    break;
                }
                taken.push(word.clone());
                if let Some(model) = &self.context_policy.model
                    && action == Some("convert")
                    && ((model.answers_resume && resume_question(&previous.original, &previous.replacement, &field.before, &next_replacement, origin))
                        || (model.answers_cyrletter && cyrletter_question(&previous.original, &previous.replacement, &field.before, "space")))
                {
                    resumed.push(word.clone());
                }
            }
            let context_field = self.context_field_id();
            plan = plan.replaced_with(|plan| {
                let mut strokes = previous.strokes.clone();
                strokes.push(previous_boundary.clone());
                strokes.append(&mut plan.strokes);
                plan.strokes = strokes;
                plan.boundary = closing.clone();
                plan.original = format!("{}{}{}", previous.original, previous_boundary.character, plan.original);
                plan.replacement = format!("{}{}{}", previous.replacement, previous_boundary.character, plan.replacement);
                plan.automatic = true;
                plan.mode = "context_phrase";
                plan.context_field = context_field;
                plan.typed = if plan.typed.is_empty() { String::new() } else { format!("{}{}{}", previous.original, previous_boundary.character, plan.typed) };
            });
            if !passed {
                settled = plan.clone();
                next_replacement.clone_from(&previous.replacement);
            }
        }
        let Some(farthest) = taken.last() else {
            return Ok(start);
        };
        let plan = settled;
        let before = if let Some(field) = &farthest.field { field.before.clone() } else {
            let cut = pystr::len(&plan.original) + pystr::len(&suffix);
            pystr::slice(&self.context_policy.stream.text, None, Some(-cut.to::<i64>())).to_owned()
        };
        let last_line = pystr::rsplit_sep(&before, "\n", 1).and_then(|parts| parts.last().copied()).unwrap_or(&before);
        if !taken.iter().all(|word| word.segmentation_open || resumed.iter().any(|other| Arc::ptr_eq(other, word))) && any_alpha(last_line) {
            technical_event!(self.shared, "kept_words_left", "words": taken.len(), "reason": "words_before_on_line");
            return Ok(start);
        }
        for word in &taken {
            technical_event!(
                self.shared, "kept_word_converted", "previous_characters": pystr::len(&word.plan.original),
                "boundary_undecided": word.baseline.is_none(),
            );
        }
        Ok(plan)
    }
}
