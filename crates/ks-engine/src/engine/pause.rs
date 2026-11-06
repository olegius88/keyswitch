//! The user stopped typing: a word waiting for its neighbour decides, and a word left open is judged.

use super::decide::WordOptions;
use super::logging::{technical_event, WordEvaluation};
use super::{Stroke, EngineCore, EngineResult, CONTEXT_WAIT_SETTLE_MIN_SECONDS, STALE_PRESS_SECONDS, KeyEvent, ms, sorted_keycodes, text_for_group, Strokes, InsertionPoint, pystr, PlanRef, DetectionDecision, Arc, KeptWord, Update};
use ks_context::context_policy::DecideOptions as PolicyOptions;
use ks_lang::intent_model::CorrectionTrigger;

fn characters_of(parts: &[&[Stroke]]) -> String {
    parts.iter().flat_map(|part| part.iter()).map(|stroke| stroke.character.as_str()).collect()
}

impl EngineCore {
    // The user stopped typing: a wait with no next word coming has to decide.
    //
    // A waiting word is a word whose direction the model wanted the next word to
    // settle. When the next word arrives, `resolve_context_wait` decides the pair.
    // When it does not, the wait used to lapse in silence and the word stood as
    // typed - which is a refusal the user never asked for, and it is how `ша ` came
    // to stay itself where the curated table names `if`. The pause is the moment
    // that possibility ends, so the word is decided once more with the pause
    // trigger, where a curated exception applies and the model still decides
    // everything else.
    //
    // A conversion decided here is made here, as a pause correction is. It used to
    // be left pending until the next key came up, and by then that key was in the
    // field after the word: the correction typed it again in the other layout, so
    // `tot ` + pause + `вход` gave `еще dход`. A key still down at the pause puts
    // the decision off to a later timer, as it puts off a pause correction. Being a
    // correction at a pause, it is made only while KeySwitch is on and corrects at
    // a pause; otherwise the word waits on for its neighbour.
    fn settle_context_wait_after_pause(&mut self, now: f64) -> EngineResult<()> {
        let (Some(waiting), Some(last)) = (self.context_waiting.as_ref(), self.last_word_input_at) else {
            return Ok(());
        };
        let settle_after = if self.pause_delay() > CONTEXT_WAIT_SETTLE_MIN_SECONDS { self.pause_delay() } else { CONTEXT_WAIT_SETTLE_MIN_SECONDS };
        if self.pending.is_some()
            || !self.strokes.is_empty()
            || now < last + settle_after
            || self.last_committed_stale
            || waiting.window != self.focus_or_zero()
            || self.text_setting("detection.context_policy", "assist") != "assist"
            || !self.flag("enabled", true)
            || !self.flag("detection.correct_on_pause", true)
        {
            return Ok(());
        }
        self.prune_stale_presses(now, STALE_PRESS_SECONDS, None);
        let waiting = self.context_waiting.clone().expect("checked above");
        if !self.pressed.is_empty() || !self.modifier_keycodes.is_empty() {
            if self.wait_deferral_logged != waiting.diagnostic_id {
                self.wait_deferral_logged = waiting.diagnostic_id;
                let reason = if self.modifier_keycodes.is_empty() { "keys_pressed" } else { "modifiers_pressed" };
                self.log_context_wait("context_wait_deferred", Some(&waiting), reason, &[]);
            }
            return Ok(());
        }
        if !waiting.settles_on_pause {
            self.log_context_wait("context_wait_cancelled", Some(&waiting), "suggestion_not_settled", &[]);
            self.context_waiting = None;
            return Ok(());
        }
        let application = self.backend.active_application();
        if waiting.plan.application != application || self.application_excluded(&application) {
            self.log_context_wait("context_wait_cancelled", Some(&waiting), "preconditions_changed", &[]);
            self.context_waiting = None;
            return Ok(());
        }
        let mode = self.text_setting("detection.context_policy", "assist");
        let result = self.context_policy.decide(&waiting.decision, &waiting.plan.replacement, waiting.plan.target_group, &self.detector, "pause", &mode, &PolicyOptions {
            field_override: Some(waiting.field.clone()),
            ..PolicyOptions::default()
        })?;
        self.context_waiting = None;
        if !result.decision.should_convert {
            self.log_context_wait("context_wait_cancelled", Some(&waiting), "pause_kept_the_word", &[]);
            return Ok(());
        }
        self.log_context_wait("context_wait_settled", Some(&waiting), "pause", &[]);
        self.learning_prompt_after = None;
        let confidence = result.decision.confidence;
        self.execute_correction(waiting.plan.replaced_with(|plan| plan.confidence = confidence), None);
        Ok(())
    }

    /// Enter or Tab arrived while a word waits for its next word: no next word is coming.
    ///
    /// The word is asked once more with the action's trigger, the question it would have been asked had the action
    /// followed it without the space, and a conversion is made before the action goes through (`maybe_execute_pending`).
    /// The action used to end the wait in silence: `jr`, a space and Enter sent `jr` alone in Telegram where `ок` was
    /// meant, while `jr` and Enter at once was asked at Enter (the laptop, 02.11.2026). A suggestion is asked again too:
    /// a pause brings no new context, but the end of the message is the action's own question. A word already
    /// converted at its boundary has nothing left to convert.
    pub fn settle_context_wait_at_action(&mut self, event: &KeyEvent) -> EngineResult<()> {
        let Some(waiting) = self.context_waiting.clone() else {
            return Ok(());
        };
        if waiting.converted
            || self.pending.is_some()
            || self.last_committed_stale
            || waiting.window != self.focus_or_zero()
            || self.text_setting("detection.context_policy", "assist") != "assist"
            || !self.flag("enabled", true)
            || !self.boundary_enabled(event)
        {
            return Ok(());
        }
        let application = self.backend.active_application();
        if waiting.plan.application != application || self.application_excluded(&application) {
            return Ok(());
        }
        let trigger = Self::_trigger_for_boundary(event);
        let mode = self.text_setting("detection.context_policy", "assist");
        let result = self.context_policy.decide(&waiting.decision, &waiting.plan.replacement, waiting.plan.target_group, &self.detector, trigger.as_str(), &mode, &PolicyOptions {
            field_override: Some(waiting.field.clone()),
            ..PolicyOptions::default()
        })?;
        self.context_waiting = None;
        if !result.decision.should_convert {
            self.log_context_wait("context_wait_cancelled", Some(&waiting), "action_kept_the_word", &[]);
            return Ok(());
        }
        self.log_context_wait("context_wait_settled", Some(&waiting), trigger.as_str(), &[]);
        self.learning_prompt_after = None;
        let confidence = result.decision.confidence;
        self.pending = Some(waiting.plan.replaced_with(|plan| plan.confidence = confidence));
        Ok(())
    }

    pub fn maybe_correct_after_pause(&mut self, now: Option<f64>) -> EngineResult<()> {
        let at = now.unwrap_or_else(|| self.monotonic());
        self.settle_context_wait_after_pause(at)?;
        if !self.pause_correction_pending {
            return Ok(());
        }
        // A word of a third layout has no language model to judge it, at a pause as at a boundary.
        if !self.flag("detection.correct_on_pause", true) || !self.flag("enabled", true) || !self.in_models(self.source_group) {
            self.reset_pause_correction();
            return Ok(());
        }
        let Some(last_input) = self.last_word_input_at else {
            self.reset_pause_correction();
            return Ok(());
        };
        let current_time = now.unwrap_or_else(|| self.monotonic());
        let idle_ms = ms(current_time - last_input);
        // The deadline as a caller computes it: `now - last < delay` reads 0.6999999999999993 for 0.7 s after 10.0 s.
        if current_time < last_input + self.pause_delay() {
            return Ok(());
        }
        self.prune_stale_presses(current_time, STALE_PRESS_SECONDS, None);
        let deferral = if !self.pressed.is_empty() {
            Some("keys_pressed")
        } else if !self.modifier_keycodes.is_empty() {
            Some("modifiers_pressed")
        } else if self.pending.is_some() {
            Some("correction_pending")
        } else {
            None
        };
        if let Some(deferral) = deferral {
            if !self.pause_deferral_logged {
                self.pause_deferral_logged = true;
                technical_event!(
                    self.shared, "pause_correction_deferred",
                    "reason": deferral,
                    "idle_ms": idle_ms,
                    "pressed_keycodes": sorted_keycodes(&self.pressed),
                    "modifier_keycodes": sorted_keycodes(&self.modifier_keycodes),
                );
            }
            return Ok(());
        }

        self.pause_correction_pending = false;
        if self.strokes.is_empty() {
            return Ok(());
        }
        let typed = self.strokes.clone();
        let source_group = self.source_group;
        let mut head = self.literal_head(&typed, source_group);
        let (mut strokes, mut trailing, mut segmentation_certain, _abstained) = self.completed_word(&typed[head..], source_group);
        let application = self.backend.active_application();
        if self.insertion_holds(&text_for_group(&typed, source_group), &self.typed_ahead(&typed), &application) {
            // Typed into a word, every key is part of it: `,` between `те` and `е`
            // is the `б` of `тебе`, not punctuation in front of a word.
            head = 0;
            strokes.clone_from(&typed);
            trailing = Vec::new();
            segmentation_certain = true;
        }
        let mut signs: Strokes = Vec::new();
        if trailing.is_empty() && segmentation_certain && !self.insertion.as_ref().is_some_and(InsertionPoint::inside_word) {
            // Letters at the end that are signs in the other layout are split off and replayed, as at a
            // boundary (commit_word): `What.` typed in the Russian layout and left without a space was
            // judged whole, with its period, and the shape check refused the conversion.
            let count = self.replayed_signs(&strokes, source_group);
            if count != 0 {
                signs = strokes.split_off(strokes.len() - count);
            }
        }
        let manual_layout_selected = self.word_protected(source_group);
        let mut original = text_for_group(&strokes, source_group);
        let mut literal_head = text_for_group(&typed[..head], source_group);
        let mut alternatives = self.alternatives_of(&strokes, source_group);
        let mut excluded = self.application_excluded(&application);
        if manual_layout_selected {
            let protection = self.protection_details();
            self.log_word_evaluation(WordEvaluation {
                trigger: "pause",
                original: &original,
                alternatives: &alternatives,
                application: &application,
                enabled: true,
                trigger_enabled: true,
                manual_layout_protected: true,
                application_excluded: excluded,
                decision: None,
                caret_unknown: false,
                protection: Some(protection),
                source_group: Some(source_group),
                early_switch_origin: self.early_switch_origin,
                idle_ms: Some(idle_ms),
                context: None,
                literal_head: &literal_head,
            })?;
            return Ok(());
        }
        if excluded {
            self.log_word_evaluation(WordEvaluation {
                trigger: "pause",
                original: &original,
                alternatives: &alternatives,
                application: &application,
                enabled: true,
                trigger_enabled: true,
                manual_layout_protected: false,
                application_excluded: true,
                decision: None,
                caret_unknown: false,
                protection: None,
                source_group: Some(source_group),
                early_switch_origin: self.early_switch_origin,
                idle_ms: Some(idle_ms),
                context: None,
                literal_head: &literal_head,
            })?;
            return Ok(());
        }
        if !segmentation_certain {
            return Ok(());
        }
        let inside_word = self.insertion.as_ref().is_some_and(InsertionPoint::inside_word);
        let target = alternatives.first().map_or(source_group, |entry| entry.0);
        let alternative = alternatives.iter().find(|entry| entry.0 == target).map_or("", |entry| entry.1.as_str()).to_owned();
        if trailing.is_empty() && head == 0 && !inside_word && self.word_begun(&original, source_group, &alternative, target)? {
            // A pause in the middle of a word is a pause to think: the word is decided at its boundary.
            technical_event!(self.shared, "pause_correction_skipped", "reason": "word_begun", "idle_ms": idle_ms, "word_characters": pystr::len(&original));
            return Ok(());
        }
        let inside = if !trailing.is_empty() || head != 0 || !signs.is_empty() {
            None
        } else {
            self.decide_inside_word(&strokes, source_group, &alternatives, &application, CorrectionTrigger::Pause, None)?
        };
        let mut decision = if let Some(inside) = inside.clone() { inside } else {
            let closing: Strokes = trailing.iter().chain(signs.iter()).cloned().collect();
            let following = alternatives.iter().map(|(group, _)| (*group, text_for_group(&closing, *group))).collect();
            self.decide_word(&original, &alternatives, source_group, &application, CorrectionTrigger::Pause, WordOptions {
                literal_tail: characters_of(&[&trailing, &signs]),
                following: Some(following),
                ..WordOptions::default()
            })?
        };
        let stem = strokes.clone();
        let stem_signs = signs.clone();
        if !signs.is_empty() && !decision.should_convert {
            // As at a boundary (commit_word): the tail is split off only for a word the model converts without it.
            strokes.append(&mut signs);
            original = text_for_group(&strokes, source_group);
            alternatives = alternatives.iter().map(|(group, _)| (*group, text_for_group(&strokes, *group))).collect();
            let following = alternatives.iter().map(|(group, _)| (*group, text_for_group(&trailing, *group))).collect();
            decision = self.decide_word(&original, &alternatives, source_group, &application, CorrectionTrigger::Pause, WordOptions {
                literal_tail: characters_of(&[&trailing]),
                following: Some(following),
                ..WordOptions::default()
            })?;
        }
        if inside.is_none() && head == 0 && trailing.is_empty() {
            let targets: Vec<i64> = alternatives.iter().map(|entry| entry.0).collect();
            if let Some((split, part)) =
                self.decide_joined_part(&stem, &stem_signs, source_group, &targets, &application, CorrectionTrigger::Pause, None, &decision)?
            {
                // As at a boundary (commit_word): the token converts after its hyphen.
                head = split;
                decision = part;
                strokes = stem[head..].to_vec();
                signs = stem_signs;
                original = text_for_group(&strokes, source_group);
                alternatives = alternatives.iter().map(|(group, _)| (*group, text_for_group(&strokes, *group))).collect();
                literal_head = text_for_group(&typed[..head], source_group);
            }
        }
        excluded = self.application_excluded(&application);
        self.log_word_evaluation(WordEvaluation {
            trigger: "pause",
            original: &original,
            alternatives: &alternatives,
            application: &application,
            enabled: true,
            trigger_enabled: true,
            manual_layout_protected: false,
            application_excluded: false,
            decision: Some(&decision),
            caret_unknown: false,
            protection: None,
            source_group: Some(source_group),
            early_switch_origin: self.early_switch_origin,
            idle_ms: Some(idle_ms),
            context: None,
            literal_head: &literal_head,
        })?;
        if !decision.should_convert {
            if !self.mention_head(&application).is_empty() {
                technical_event!(
                    self.shared, "mention_head_kept", "reason": "word_kept", "shown": false,
                    "application": application, "application_excluded": excluded,
                );
            }
            return Ok(());
        }

        self.early_switch_origin = None;
        self.early_switch_at = None;
        // A word converted at a pause is the neighbour a waiting word was waiting
        // for, exactly as at a space: `tot`, then `ghbdtn` and a pause before the
        // space converted `привет` alone, switched the layout, and the space in the
        // other layout ended the wait with `tot` left standing.
        let waiting = self.context_waiting.take();
        let provisional = self.provisional.take();
        let mut kept = std::mem::take(&mut self.kept);
        let mut joint: Option<(PlanRef, DetectionDecision)> = None;
        if !trailing.is_empty() || head != 0 || !signs.is_empty() {
            self.log_context_wait(
                "context_wait_cancelled", waiting.as_ref(), if !trailing.is_empty() || !signs.is_empty() { "literal_tail" } else { "literal_head" }, &[],
            );
            if head == 0 {
                // As at a space (commit_word): a word with a sign split off its end takes the words
                // before it along, a waiting one among them (`tot ghbdtn,`), and the sign stays as
                // typed after them. `lbre. rjire/` at the end of a message was left `lbre. кошку.`, and
                // with the sign alone split off, `я cgtie ljvjq/` was left `я cgtie домой.` (Tatoeba
                // development sentences typed as the sealed protocol types them, 23.10.2026).
                if let Some(waited) = waiting.as_ref().filter(|_| !trailing.is_empty() || !signs.is_empty()) {
                    kept.push(Arc::new(KeptWord {
                        plan: waited.plan.clone(),
                        baseline: Some(waited.decision.clone()),
                        field: Some(waited.field.clone()),
                        window: waited.window,
                        deadline: waited.deadline,
                        segmentation_open: false,
                    }));
                }
                let base = self.plan_from_decision(&strokes, None, &application, &decision, "pause");
                let base = base.replaced_with(|plan| plan.trailing.clone_from(&trailing));
                let base = self.with_signs(&base, &signs);
                let taken = self.take_along(&kept, &base, pystr::len(&original), &application, false)?;
                if taken.mode == "context_phrase" {
                    joint = Some((taken, decision.clone()));
                }
            }
        } else {
            joint = self.resolve_context_wait(waiting.as_ref(), &strokes, None, &decision, &application, &alternatives, false)?;
            if let (Some(found), Some(waited)) = (joint.clone(), waiting.as_ref()) {
                let taken = self.take_along(&kept, &found.0, pystr::len(&waited.plan.original), &application, false)?;
                joint = Some((taken, found.1));
            }
            if joint.is_none() {
                joint = self.revert_with_next_word(provisional.as_ref(), &strokes, None, &decision, &application)?;
            }
            if joint.is_none() {
                let base = self.plan_from_decision(&strokes, None, &application, &decision, "pause");
                let taken = self.take_along(&kept, &base, pystr::len(&original), &application, false)?;
                if taken.mode == "context_phrase" {
                    joint = Some((taken, decision.clone()));
                }
            }
        }
        let plan = if let Some((plan, _)) = joint { plan } else {
            let mention = self.mention_head(&application);
            let base = self.plan_from_decision(&strokes, None, &application, &decision, "pause");
            let base = base.replaced_with(|plan| {
                plan.trailing.clone_from(&trailing);
                // A sign left after the word at a pause is a key with a letter on it: never the quote that opened it.
                plan.head.clone_from(&mention);
            });
            let mut plan = self.with_signs(&base, &signs);
            if inside.is_none()
                && trailing.is_empty()
                && head == 0
                && signs.is_empty()
                && mention.is_empty()
            {
                // Asked again if the next word goes back the other way (provisional_after_pause).
                let field = self.context_result.as_ref().and_then(|result| result.field.clone());
                let revisit = self.last_baseline.clone().map(|baseline| Arc::new((baseline, field)));
                plan = plan.replaced_with(|plan| plan.revisit = revisit);
            }
            plan
        };
        self.strokes = Vec::new();
        self.insertion = None;
        self.source_group = -1;
        self.early_switch_undone = false;
        self.reset_pause_correction();
        self.shared.update(Update { current_word: Some(String::new()), ..Update::default() });
        self.execute_correction(plan, None);
        Ok(())
    }

    /// Forget presses whose release was never delivered (focus changes).
    pub fn prune_stale_presses(&mut self, now: f64, older_than: f64, keep: Option<u32>) {
        let mut stale: Vec<u32> = self
            .pressed_since
            .iter()
            .filter(|(keycode, since)| now - since > older_than && Some(*keycode) != keep)
            .map(|(keycode, _)| *keycode)
            .collect();
        if stale.is_empty() {
            return;
        }
        for keycode in &stale {
            self.pressed_since.retain(|entry| entry.0 != *keycode);
            self.pressed.remove(keycode);
            self.modifier_keycodes.remove(keycode);
        }
        stale.sort_unstable();
        technical_event!(self.shared, "stale_presses_pruned", "keycodes": stale);
    }
}
