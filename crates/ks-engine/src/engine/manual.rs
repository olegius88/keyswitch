//! What the user asks for by hand: Pause, the double press that offers a rule, undo, input of other programs.

use super::logging::technical_event;
use super::*;
use ks_config::indicator::{alternate_layout_group, layout_label};
use ks_config::learning::RuleAction;

impl EngineCore {
    // Pause: convert what was typed since the last boundary, or switch.
    pub fn schedule_manual_conversion(&mut self, trigger_keycode: i64) {
        if self.pending.as_ref().is_some_and(|pending| !pending.automatic) && self.strokes.is_empty() && self.symbol_strokes.is_empty() {
            technical_event!(
                self.shared, "manual_conversion_waiting", "reason": "previous_command_pending",
                "pressed_keycodes": sorted_keycodes(&self.pressed),
                "modifier_keycodes": sorted_keycodes(&self.modifier_keycodes),
            );
            self.shared.update(Update { last_action: Some("Замена ожидает отпускания клавиш".to_owned()), ..Update::default() });
            return;
        }
        if self.strokes.is_empty() && self.symbol_strokes.is_empty() {
            self.personal_word_settled(false);
        }
        if self.typo_correction_reversible() {
            // Pause right after a typo correction puts the word back as typed, as undo does; the next press converts.
            self.schedule_undo(trigger_keycode);
            return;
        }
        let mut mode = "manual";
        let mut trailing: Strokes = Vec::new();
        let mut strokes: Strokes;
        let mut source_group;
        let boundary;
        let application;
        let source;
        if !self.strokes.is_empty() {
            strokes = self.symbol_strokes.iter().chain(self.strokes.iter()).cloned().collect();
            source_group = self.source_group;
            boundary = None;
            application = self.backend.active_application();
            source = if self.symbol_strokes.is_empty() { "current_word" } else { "symbols_and_word" };
            self.mention_shown = None;
            self.early_switch_origin = None;
            self.early_switch_at = None;
        } else if !self.symbol_strokes.is_empty() {
            strokes = self.symbol_strokes.clone();
            source_group = i64::from(self.symbol_strokes.last().expect("not empty").group);
            if self.mention_shown.is_some() {
                // Pause on the "@" shown for a quote asks for the quote back, and
                // the quote is then kept whatever word follows it.
                source_group = self.other_groups(source_group)[0];
                self.mention_shown = None;
                self.symbol_strokes = Vec::new();
            }
            boundary = None;
            application = self.backend.active_application();
            source = "symbols";
            mode = "symbols";
        } else if let Some(committed) = self.last_committed.clone().filter(|_| !self.last_committed_stale) {
            strokes = committed.strokes.clone();
            source_group = committed.source_group;
            boundary = committed.boundary.clone();
            application = committed.application.clone();
            source = "last_committed";
            trailing.clone_from(&committed.trailing);
        } else {
            self.switch_layout_only(trigger_keycode);
            return;
        }
        let targets = self.other_groups(source_group);
        let Some(&target) = targets.first() else {
            return;
        };
        // A token of parts joined by a sign typed the same in both layouts, every part but the last settled as
        // typed: Pause converts the last part. `lid-'ne` was `дшв-эту` (a field log, 13.10.2026), and the
        // user pressed Pause again and typed the part anew; `lid-эту` was meant. Pause on what a correction
        // typed takes back all of it (`mongo-service`, converted whole, is `ьщтпщ-ыукмшсу` again).
        let restores = self.last_correction.as_ref().is_some_and(|previous| strokes == previous.strokes && source_group == previous.target_group);
        let kept_head = if mode == "manual" && !restores { self.joined_split(&strokes, source_group) } else { 0 };
        strokes.drain(..kept_head);
        let original = text_for_group(&strokes, source_group);
        let replacement = text_for_group(&strokes, target);
        let plan = Arc::new(CorrectionPlan {
            automatic: false,
            mode,
            trailing: trailing.clone(),
            ..CorrectionPlan::new(strokes, boundary, source_group, target, original.clone(), replacement.clone(), UNSCORED_CORRECTION_CONFIDENCE, &application)
        });
        // Pause right after a correction converts the same keys back. That
        // teaches nothing: a rule comes only from the rule window, which a
        // double press of the hotkey offers.
        let reversal = self.reversal_of_last_correction(&plan);
        if let Some(reversal) = reversal.as_ref().filter(|_| self.agrees_with_pause_correction(&plan)) {
            self.pause_agreement_open = false;
            technical_event!(
                self.shared, "manual_conversion_absorbed",
                "reason": "pause_correction_just_made",
                "source": source,
                "mode": reversal.mode,
                "elapsed_ms": ms(self.monotonic() - self.last_correction_time),
                "application": application,
                "application_excluded": self.application_excluded(&application),
            );
            self.shared.update(Update { last_action: Some(format!("Уже преобразовано: {original} · ещё одно нажатие вернёт")), ..Update::default() });
            return;
        }
        // A second Pause before the first one ran replaces the plan; without
        // this line the first conversion would vanish without a trace.
        self.log_pending_dropped("replaced_by_manual_conversion");
        self.pending = Some(plan.clone());
        self.learning_prompt_after = None;
        self.convert_outcome = Some(ConvertOutcome::Plan(plan));
        self.pending_trigger_keycode = trigger_keycode;
        self.manual_release_deadline = self.monotonic() + MANUAL_RELEASE_TIMEOUT_SECONDS;
        let excluded = self.application_excluded(&application);
        technical_event!(
            self.shared, "manual_conversion_scheduled",
            "wait_id": self.context_waiting.as_ref().map(|waiting| waiting.diagnostic_id),
            "source": source,
            "original": if excluded { "<redacted>" } else { original.as_str() },
            "replacement": if excluded { "<redacted>" } else { replacement.as_str() },
            "source_group": source_group,
            "target_group": target,
            "application": application,
            "application_excluded": excluded,
            "symbol_count": self.symbol_strokes.len(),
            "reversal": reversal.as_ref().map(|reversal| if reversal.automatic { "automatic" } else { "manual" }),
            "kept_head_characters": kept_head,
            "trailing_characters": trailing.len(),
        );
        // Explicit intent supersedes the model's old lookahead even while the
        // manual plan is waiting for release. Retain its ID in the event above,
        // but never let that stale wait block the following word's prefix.
        self.cancel_context_wait("manual_conversion");
        self.strokes = Vec::new();
        self.insertion = None;
        self.symbol_strokes = Vec::new();
        self.source_group = -1;
        self.early_switch_undone = false;
        self.last_committed_stale = true;
        self.reset_pause_correction();
    }

    /// A plain space pressed while an automatic correction made at a sign, not at a space, waits for its keys to
    /// come up. A correction at a space may wait for the next word (`start_context_wait`), and one of another kind
    /// (Pause, the early switch, a mention) reaches text of its own: those still give way to the second boundary.
    pub fn space_follows_pending_sign(&self, event: &KeyEvent) -> bool {
        let Some(plan) = &self.pending else {
            return false;
        };
        plan.mode == "boundary"
            && plan.automatic
            && plan.kept_tail == 0
            && plan.head.is_empty()
            && match &plan.boundary {
                None => plan.sign_replayed,
                Some(boundary) => !pystr::isspace(&boundary.character) && !boundary.deferred,
            }
            && event.pressed
            && event.character == " "
            && !(event.control() || event.alt() || event.super_key())
    }

    /// Note a correction that landed and the chain of corrections it belongs to.
    ///
    /// Converting the same keys back continues the chain; anything else starts a
    /// new one. The chain's first plan is what a double press of the hotkey
    /// reasons about: the engine's own decision, or the user's first conversion.
    pub fn remember_correction(&mut self, plan: &PlanRef) {
        // An undo always puts back the last correction, also past the spaces typed after it, where its keys
        // after the word are more than the correction's (undo_past_spaces): the chain goes on.
        if self.correction_origin.is_none() || (plan.mode != "undo" && self.reversal_of_last_correction(plan).is_none()) {
            self.correction_origin = Some(plan.clone());
        }
        self.last_correction = Some(plan.clone());
        self.last_correction_time = self.monotonic();
        // maybe_correct_after_pause ends no word: its plans alone carry no boundary key.
        self.pause_agreement_open = plan.automatic && plan.boundary.is_none() && (plan.mode == "pause" || plan.mode == "context_phrase");
    }

    /// Whether ``plan``, the reversal of the last correction, asks for that correction instead.
    ///
    /// A word the engine converts at a pause is a word the user stopped typing to look at,
    /// and a user who sees it in the wrong layout stops to press the hotkey. Both answer the
    /// same moment: in the field logs the press came 7 ms to 1.2 s after the correction, often
    /// held back behind it, so it converted the word straight back and the user pressed the
    /// hotkey again. A press that soon, with nothing typed since, is taken as that agreement,
    /// once; the next press converts the word back as before.
    fn agrees_with_pause_correction(&self, plan: &CorrectionPlan) -> bool {
        self.pause_agreement_open && plan.boundary.is_none() && self.monotonic() - self.last_correction_time <= PAUSE_CORRECTION_AGREEMENT_SECONDS
    }

    /// The chain `plan` would continue once it runs.
    fn chain_origin(&self, plan: &PlanRef) -> PlanRef {
        if let Some(origin) = &self.correction_origin
            && self.reversal_of_last_correction(plan).is_some()
        {
            return origin.clone();
        }
        plan.clone()
    }

    /// The layout the user wants the word in when they object to ``origin``.
    ///
    /// An automatic correction was the engine's idea, so the user wants the keys
    /// as typed; a manual conversion was theirs, so they want its result.
    fn _wanted_group(origin: &CorrectionPlan) -> i64 {
        if origin.automatic { origin.source_group } else { origin.target_group }
    }

    /// Whether this press of the conversion hotkey completes a double press.
    pub fn double_convert_press(&mut self, event: &KeyEvent) -> bool {
        let keycode = i64::from(event.keycode);
        if keycode == self.convert_keycode && !self.convert_released && self.convert_press_at != 0.0 {
            // Auto-repeat of the held key: still the first press.
            return false;
        }
        let now = self.monotonic();
        let double = self.flag("detection.learning", true)
            && self.convert_released
            && keycode == self.convert_keycode
            && self.convert_outcome.is_some()
            && now - self.convert_press_at <= DOUBLE_CONVERT_PRESS_WINDOW_SECONDS;
        self.convert_press_at = if double { 0.0 } else { now };
        self.convert_keycode = keycode;
        self.convert_released = false;
        if !double {
            self.convert_outcome = None;
        }
        double
    }

    /// Second press of the hotkey: leave the word as the user wants it and offer a rule.
    ///
    /// The first press already did what a single press does. If that put the
    /// word where the user wants it - a wrong automatic correction undone, a
    /// missed one converted - the second press only opens the offer. If the
    /// first press undid a fix the user had made earlier, the second press puts
    /// the fix back, exactly as a single press would, and the offer follows.
    pub fn request_rule_after_double_press(&mut self, trigger_keycode: i64) {
        let outcome = self.convert_outcome.take();
        let application = self.backend.active_application();
        if let Some(ConvertOutcome::Plan(outcome)) = &outcome
            && outcome.mode != "symbols"
        {
            if self.pending.as_ref().is_some_and(|pending| Arc::ptr_eq(pending, outcome)) {
                let origin = self.chain_origin(outcome);
                let offer = self.rule_offer(&origin, &application);
                if Self::_wanted_group(&origin) == outcome.source_group {
                    // The first press, still waiting for its keys, would undo
                    // the user's own fix: it never runs.
                    self.log_pending_dropped("rule_requested");
                    self.pending = None;
                    self.manual_release_deadline = 0.0;
                    self.shared.show_learning_prompt(offer);
                } else {
                    self.learning_prompt_after = Some(offer);
                }
                return;
            }
            if self.last_requested_plan.as_ref().is_some_and(|requested| Arc::ptr_eq(requested, outcome)) {
                let origin = self.correction_origin.clone().unwrap_or_else(|| outcome.clone());
                let offer = self.rule_offer(&origin, &application);
                if Self::_wanted_group(&origin) == outcome.target_group {
                    self.shared.show_learning_prompt(offer);
                    return;
                }
                self.schedule_manual_conversion(trigger_keycode);
                let scheduled = self.convert_outcome.take();
                match scheduled {
                    Some(ConvertOutcome::Plan(_)) => {
                        self.learning_prompt_after = Some(offer);
                    }
                    _ => self.shared.show_learning_prompt(offer),
                }
                return;
            }
        }
        if matches!(outcome, Some(ConvertOutcome::Switch)) {
            // The first press toggled the layout; the second puts it back.
            self.switch_layout_only(trigger_keycode);
            self.convert_outcome = None;
        }
        let offer = self.empty_rule_offer(&application);
        self.shared.show_learning_prompt(offer);
    }

    fn rule_offer(&self, origin: &CorrectionPlan, application: &str) -> LearningPrompt {
        if !any_alpha(&origin.original) {
            return self.empty_rule_offer(application);
        }
        LearningPrompt {
            source_group: origin.source_group,
            target_group: origin.target_group,
            original: origin.original.clone(),
            replacement: origin.replacement.clone(),
            application: application.to_owned(),
            action: if origin.automatic { RuleAction::Keep } else { RuleAction::Convert },
        }
    }

    fn empty_rule_offer(&self, application: &str) -> LearningPrompt {
        let current = self.shared.current_group();
        let group = if self.in_models(current) { current } else { 0 };
        let target = self.other_groups(group).first().copied().unwrap_or(group);
        LearningPrompt {
            source_group: group,
            target_group: target,
            original: String::new(),
            replacement: String::new(),
            application: application.to_owned(),
            action: RuleAction::Convert,
        }
    }

    /// Input another program injected: the text changed in a way not typed here.
    ///
    /// Remote control (`TeamViewer`, `AnyDesk`), macros and on-screen keyboards send
    /// keys of their own. They are not this keyboard's typing - the machine where
    /// they are typed has its own switcher - so they are neither words nor
    /// hotkeys nor keys to wait for; only the text they change is no longer known.
    pub fn observe_foreign_input(&mut self, event: &KeyEvent) {
        if !event.pressed || is_modifier_key(&event.key_name) {
            return;
        }
        self.convert_press_at = 0.0;
        if !self.foreign_input_active {
            self.foreign_input_active = true;
            technical_event!(
                self.shared, "foreign_input_observed",
                "key_name": event.key_name,
                "application": self.backend.active_application(),
                "word_length": self.strokes.len(),
                "action_waiting": self.deferred_action.is_some(),
            );
        }
        if self.deferred_action.is_some() {
            // The user's own Enter is waiting for its keys to come up; the
            // injected key is held behind it and follows it. Nothing to undo.
            return;
        }
        self.clear_word(None, "foreign_input");
        self.untracked_token = false;
        self.last_committed_stale = true;
        self.contexts.clear();
    }

    /// The last correction, if `plan` converts the same keys back.
    pub fn reversal_of_last_correction(&self, plan: &CorrectionPlan) -> Option<PlanRef> {
        let previous = self.last_correction.as_ref()?;
        if plan.strokes != previous.strokes
            || plan.trailing != previous.trailing
            || plan.source_group != previous.target_group
            || plan.target_group != previous.source_group
        {
            return None;
        }
        Some(previous.clone())
    }

    /// Pause with nothing to convert just toggles the layout.
    pub fn switch_layout_only(&mut self, trigger_keycode: i64) {
        let current = self.shared.current_group();
        let Some(target) = alternate_layout_group(current).filter(|target| self.in_models(*target)) else {
            technical_event!(
                self.shared, "manual_conversion_impossible",
                "reason": "no_alternate_layout",
                "current_group": current,
                "trigger_keycode": trigger_keycode,
                "last_committed_stale": self.last_committed_stale,
            );
            self.shared.update(Update { last_action: Some("Нет слова для ручного преобразования".to_owned()), ..Update::default() });
            return;
        };
        if let Err(error) = self.backend.switch_group(target.to::<i32>()) {
            technical_event!(
                self.shared, "layout_switch_failed",
                "source": "convert_last",
                "requested_group": target,
                "error": error.to_string(),
            );
            self.note_unconfirmed_switch(&error);
            self.shared.update(Update { last_error: Some(error.to_string()), last_action: Some("Раскладка не переключена".to_owned()), ..Update::default() });
            return;
        }
        // The layout the engine switched to holds no word from correction (note_engine_switch): Pause with nothing to
        // convert was pressed to fix a word the engine had lost track of, after a click, a Backspace or another window,
        // not to keep the next word as typed. Held, the next word stayed as typed where the person went on in the
        // layout this left on: `djghjc` for `вопрос`, `cltkfq` for `сделай` (8 of the 9 such words of a user's field
        // batteries were meant converted, 02.11.2026). A layout the person picks themselves still holds the next word,
        // and so does this one when the Pause takes the layout back to where an automatic correction of the last
        // UNDO_AVAILABLE_WINDOW_SECONDS came from: the person erased that correction (Backspace, the editor's undo) and
        // types the word again as it was. `гупшуп` became `uegieg`, was undone in the editor, and Pause 2.2 s later
        // brought the Russian layout back for it (field typing, 28.09.2026); every such Pause of the batteries
        // came before a word meant as typed.
        let undone = self.last_correction.as_ref().is_some_and(|previous| {
            previous.automatic && previous.source_group == target && self.monotonic() - self.last_correction_time <= UNDO_AVAILABLE_WINDOW_SECONDS
        });
        self.note_engine_switch(target);
        self.convert_outcome = Some(ConvertOutcome::Switch);
        let protects = undone && self.flag("detection.respect_manual_layout", true);
        if protects {
            self.manual_layout_group = Some(target);
            self.manual_layout_observed_at = Some(self.monotonic());
            "correction_undone".clone_into(&mut self.manual_layout_source);
            self.manual_layout_word = false;
        }
        technical_event!(
            self.shared, "layout_switched_without_word",
            "trigger_keycode": trigger_keycode,
            "previous_group": current,
            "selected_group": target,
            "protects_next_word": protects,
            "last_committed_stale": self.last_committed_stale,
        );
        self.shared.update(Update {
            current_group: Some(target),
            last_action: Some(format!("Раскладка переключена: {}", layout_label(target))),
            last_error: Some(String::new()),
            ..Update::default()
        });
    }

    /// Undo hotkey while an early-switched word is still being typed.
    ///
    /// The prefix is not a finished correction yet, so the generic undo would
    /// revert the *previous* correction and delete the wrong characters. This
    /// returns the prefix to the layout the user typed it in and protects
    /// the rest of the word from being switched again.
    fn schedule_early_switch_undo(&mut self, origin: i64, trigger_keycode: i64) {
        let strokes = self.strokes.clone();
        let current_group = self.source_group;
        self.log_pending_dropped("replaced_by_undo");
        let prefix_length = strokes.len();
        let original = text_for_group(&strokes, current_group);
        let replacement = text_for_group(&strokes, origin);
        self.pending = Some(Arc::new(CorrectionPlan {
            automatic: false,
            mode: "early_undo",
            ..CorrectionPlan::new(strokes, None, current_group, origin, original, replacement, UNSCORED_CORRECTION_CONFIDENCE, &self.backend.active_application())
        }));
        self.learning_prompt_after = None;
        self.pending_trigger_keycode = trigger_keycode;
        self.manual_release_deadline = self.monotonic() + MANUAL_RELEASE_TIMEOUT_SECONDS;
        technical_event!(
            self.shared, "early_switch_undo_scheduled",
            "source_group": current_group,
            "target_group": origin,
            "prefix_length": prefix_length,
        );
    }

    pub fn schedule_undo(&mut self, trigger_keycode: i64) {
        self.personal_word_settled(false);
        if self.pending.as_ref().is_some_and(|pending| pending.mode == "early") {
            self.log_pending_dropped("replaced_by_undo");
            self.pending = None;
            self.learning_prompt_after = None;
            self.early_switch_undone = true;
            self.shared.update(Update { last_action: Some("Раннее переключение отменено до замены".to_owned()), ..Update::default() });
            return;
        }
        if let Some(early_origin) = self.early_switch_origin
            && !self.strokes.is_empty()
        {
            self.schedule_early_switch_undo(early_origin, trigger_keycode);
            return;
        }
        let available = match (&self.last_correction, &self.last_committed) {
            (Some(previous), Some(committed)) => {
                (self.strokes == previous.strokes || self.strokes.is_empty()) && self.symbol_strokes.is_empty() && committed.boundary == previous.boundary && committed.trailing == previous.trailing && committed.strokes == previous.strokes && !self.last_committed_stale
                    && self.monotonic() - self.last_correction_time <= UNDO_AVAILABLE_WINDOW_SECONDS
            }
            _ => false,
        };
        if !available {
            technical_event!(self.shared, "undo_unavailable", "reason": "text_changed_or_expired");
            self.shared.update(Update { last_action: Some("Последнее исправление уже нельзя отменить".to_owned()), ..Update::default() });
            return;
        }
        let previous = self.last_correction.clone().expect("checked above");
        let undo = Self::_undo_of(&previous, previous.boundary.clone(), previous.trailing.clone());
        self.queue_undo(undo, trigger_keycode);
    }

    /// The undo hotkey: the last correction back, also past the spaces typed after it.
    ///
    /// A wrong correction is noticed when it is seen, and by then a space or more often stands after
    /// it. The hotkey reaches the correction across those spaces, puts the word back as it was typed and
    /// keeps the spaces, as Pause reaches the last word past them (`pause_reaches_past`, which also bounds
    /// how many). Any other key typed after the correction still leaves nothing to undo: a letter, Backspace,
    /// an arrow key or a click leave text the correction did not, and Tab and Enter may have moved to
    /// another field or sent the text (they end the reach of Pause too).
    ///
    /// There are two paths for one reason. `schedule_undo` undoes a correction only while the text ends
    /// exactly as the correction left it, and a frozen historical test calls it directly and pins that
    /// behaviour: `tests/test_input_integrity.py`, `test_undo_is_unavailable_after_edit_or_pointer_move`, where
    /// a space typed after the correction leaves nothing to undo. The bytes of that file are part of sealed
    /// evidence and cannot change. What the hotkey does is the product's, so the hotkey comes here, and
    /// everything other than the spaces is left to `schedule_undo`.
    pub fn schedule_undo_hotkey(&mut self, trigger_keycode: i64) {
        let Some((plan, spaces)) = self.undo_past_spaces() else {
            self.schedule_undo(trigger_keycode);
            return;
        };
        technical_event!(
            self.shared, "undo_past_spaces", "spaces": spaces, "application": plan.application,
            "application_excluded": self.application_excluded(&plan.application),
        );
        self.queue_undo(plan, trigger_keycode);
    }

    /// The undo of the last correction together with the spaces typed after it, and how many there are.
    ///
    /// None unless the text before the caret is the correction's text followed by plain spaces alone: the
    /// word, the signs and the boundary the correction typed, then spaces (`extend_last_committed` adds each
    /// to the last committed word as it is typed, and anything else makes that word stale).
    pub fn undo_past_spaces(&self) -> Option<(PlanRef, usize)> {
        let (previous, committed) = (self.last_correction.as_ref()?, self.last_committed.as_ref()?);
        if self.pending.is_some()
            || self.last_committed_stale
            || !self.strokes.is_empty()
            || !self.symbol_strokes.is_empty()
            || committed.strokes != previous.strokes
            || self.monotonic() - self.last_correction_time > UNDO_AVAILABLE_WINDOW_SECONDS
        {
            return None;
        }
        let corrected: Strokes = previous.trailing.iter().chain(previous.boundary.iter()).cloned().collect();
        let typed: Strokes = committed.trailing.iter().chain(committed.boundary.iter()).cloned().collect();
        let after = typed.get(corrected.len()..).unwrap_or(&[]);
        if typed[..corrected.len().min(typed.len())] != corrected[..]
            || after.is_empty()
            || !after.iter().all(|key| key.character == " " && !key.characters.is_empty() && key.characters.iter().all(|character| character == " "))
        {
            return None;
        }
        Some((Self::_undo_of(previous, committed.boundary.clone(), committed.trailing.clone()), after.len()))
    }

    /// The plan that puts `previous` back, deleting and typing again `trailing` and `boundary` after the word.
    ///
    /// A case fix and a typo correction typed keys other than the ones typed (CorrectionPlan.replaced,
    /// `CorrectionPlan.replaced_strokes)`: undo types those back, and undoing the undo types the fix again.
    fn _undo_of(previous: &CorrectionPlan, boundary: Option<Stroke>, trailing: Strokes) -> PlanRef {
        let mut replaced: Strokes = Vec::new();
        let mut replaced_strokes: Strokes = Vec::new();
        let strokes;
        if !previous.replaced_strokes.is_empty() {
            // A typo correction typed other keys than the user's, and maybe not as many (CorrectionPlan.replaced_strokes):
            // undo types the user's keys back over the corrected word, and undoing the undo types the correction again.
            strokes = previous.replaced_strokes.clone();
            replaced_strokes.clone_from(&previous.strokes);
        } else if !previous.replaced.is_empty() {
            strokes = previous.replaced.clone();
            replaced.clone_from(&previous.strokes);
        } else {
            strokes = previous.strokes.clone();
        }
        Arc::new(CorrectionPlan {
            automatic: false,
            mode: "undo",
            trailing,
            replaced,
            replaced_strokes,
            ..CorrectionPlan::new(
                strokes, boundary, previous.target_group, previous.source_group, previous.replacement.clone(), previous.original.clone(),
                UNSCORED_CORRECTION_CONFIDENCE, &previous.application,
            )
        })
    }

    /// Run the undo `plan` once the hotkey's keys are up.
    fn queue_undo(&mut self, plan: PlanRef, trigger_keycode: i64) {
        self.log_pending_dropped("replaced_by_undo");
        self.pending = Some(plan);
        // Undo puts the text back and teaches nothing; a rule that keeps the
        // word comes only from the rule window.
        self.learning_prompt_after = None;
        self.pending_trigger_keycode = trigger_keycode;
        self.manual_release_deadline = self.monotonic() + MANUAL_RELEASE_TIMEOUT_SECONDS;
    }
}
