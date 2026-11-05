//! A correction is carried out: the keys typed after it collected, the backend's injection, and what the engine then knows.

use super::logging::technical_event;
use super::shared::QueueItem;
use super::*;
use ks_backend::{BackendError, Injection};
use ks_context::context_policy::{caret_lag, ends_with_typed, typed_short_of};
use ks_context::input_context::CONTEXT_TTL;
use std::sync::atomic::Ordering;

fn owned(strokes: &[Stroke]) -> Vec<KeyEvent> {
    strokes.iter().map(|stroke| (**stroke).clone()).collect()
}

fn is_planned(planned: &[*const KeyEvent], stroke: &Stroke) -> bool {
    planned.contains(&Arc::as_ptr(stroke))
}

impl EngineCore {
    // The kind of a key that keeps the keys typed after a word from being typed again; never its character.
    //
    // The log said only that one was there, and the one correction a user lost this way in October could
    // not be told apart from a space typed early (notebook, 10.10.2026: Pause found nothing to convert after it).
    fn _late_key_kind(event: &KeyEvent) -> &'static str {
        if is_modifier_key(&event.key_name) {
            return "modifier";
        }
        if event.control() || event.alt() || event.super_key() {
            return "shortcut";
        }
        if event.key_name == "BackSpace" {
            return "backspace";
        }
        if is_navigation_key(&event.key_name) {
            return "navigation";
        }
        if event.character == " " {
            return "space";
        }
        if !event.character.is_empty() && pystr::isprintable(&event.character) {
            return "composed";
        }
        "other_key"
    }

    fn _late_text_key(event: &KeyEvent) -> bool {
        event.pressed
            && !event.character.is_empty()
            && pystr::isprintable(&event.character)
            && event.character != " "
            && !is_modifier_key(&event.key_name)
            && !is_navigation_key(&event.key_name)
            && event.key_name != "BackSpace"
            && !(event.control() || event.alt() || event.super_key())
    }

    /// The text typed after ``original``: the keys queued, and the word begun since if ``original`` is
    /// not the word in progress (a decision put off to a later key: the next word's letters are taken).
    pub(crate) fn typed_after(&self, original: &str) -> String {
        let same = self.strokes.iter().map(|stroke| stroke.character.as_str()).collect::<String>() == original;
        if same { self.typed_ahead(&self.strokes) } else { self.typed_ahead(&[]) }
    }

    /// The text typed after the given strokes that the field may already show.
    ///
    /// The hook passes every key on to the window as it queues it, and a correction
    /// runs only once the key that triggered it is up. A user who presses the next
    /// letter before the space comes up (rollover typing) has the next word begun by
    /// then, and keys still queued behind the trigger stand in the field too.
    /// collect_late_input deletes such keys with the word and types them again; the
    /// field check has to expect them as well. It refused every such correction as a
    /// changed field: with the field reader on, `ghbdtn vbh` typed with the `v` pressed
    /// before the space came up stayed as typed, its next word with it (Windows field
    /// logs, 08.10.2026). Plain text only, up to the first key that is not.
    pub(crate) fn typed_ahead(&self, typed: &[Stroke]) -> String {
        let planned: Vec<*const KeyEvent> = typed.iter().map(Arc::as_ptr).collect();
        let mut ahead = String::new();
        // The strokes outside `typed` come first; the queue is looked at only if they do not end the text.
        for stroke in self.strokes.iter().filter(|stroke| !is_planned(&planned, stroke)) {
            if !stroke.pressed || is_modifier_key(&stroke.key_name) {
                continue;
            }
            if !(!stroke.character.is_empty() && pystr::isprintable(&stroke.character)) || stroke.control() || stroke.alt() || stroke.super_key() {
                return ahead;
            }
            ahead.push_str(&stroke.character);
        }
        for item in self.shared.queued() {
            let QueueItem::Key(item) = item else {
                break;
            };
            if !item.pressed || is_modifier_key(&item.key_name) {
                continue;
            }
            if !(!item.character.is_empty() && pystr::isprintable(&item.character)) || item.control() || item.alt() || item.super_key() {
                break;
            }
            ahead.push_str(&item.character);
        }
        ahead
    }

    /// Why the field read before a correction is not where its word was typed; empty if it is.
    ///
    /// The text before the caret ends with the word and its boundary, and then with as much
    /// of the text typed ahead as the window has taken in so far.
    fn _field_refusal(plan: &CorrectionPlan, field: Option<&FieldContext>, suffix: &str, ahead: &str) -> &'static str {
        let Some(field) = field else {
            return "unread";
        };
        if field.field_id != plan.context_field || field.application != plan.application {
            return "other_field";
        }
        if field.sensitive || field.selection {
            return "sensitive_or_selected";
        }
        let boundaries = ahead.char_indices().map(|(index, _)| index).chain(std::iter::once(ahead.len()));
        for end in boundaries {
            if caret_lag(&field.before, &field.after, &format!("{suffix}{}", &ahead[..end]), true).is_some() {
                return "";
            }
        }
        "other_text"
    }

    /// How long a correction may wait for its field (detection.field_catch_up); a value not offered is the default.
    fn field_catch_up_seconds(&self) -> f64 {
        let mut value = self.text_setting("detection.field_catch_up", DEFAULT_FIELD_CATCH_UP);
        if !FIELD_CATCH_UP_CHOICES.contains(&value.as_str()) {
            value = DEFAULT_FIELD_CATCH_UP.to_owned();
        }
        if value == FIELD_CATCH_UP_OFF { 0.0 } else { value.parse::<f64>().unwrap_or(0.0) / MILLISECONDS_PER_SECOND }
    }

    /// Whether a correction waits for a field that has not shown its word yet, rather than give it up.
    ///
    /// A browser or an Electron editor on Windows reports its text a moment after the keys: the word is there short
    /// of its last letters or its boundary (_typed_in_field), and refused as a changed field, the correction was
    /// lost (`ашкуащч` stayed for `firefox` in Antigravity, field typing of 19.10.2026). It is tried again on
    /// the engine's timer (retry_field_wait) for as long as detection.field_catch_up allows, and made only once the
    /// field shows the very word typed (_field_refusal); one that never does is given up as before.
    fn waits_for_field(&mut self, plan: &PlanRef, learning_prompt: Option<&LearningPrompt>, field: &FieldContext, waiting: Option<&FieldWait>) -> bool {
        let mut typed = plan.original.clone();
        typed.extend(plan.trailing.iter().map(|stroke| stroke.character.as_str()));
        let closing = plan.boundary.as_ref().filter(|boundary| !boundary.deferred).map_or("", |boundary| boundary.character.as_str());
        // Behind is missing, not shifted: the last letters, or the boundary after the whole word, are nowhere in the
        // field yet; standing after the caret, they are a caret seen off (caret_lag), which waiting does not mend.
        if typed_short_of(&field.before, &field.after, &typed).is_none()
            && !(!closing.is_empty() && ends_with_typed(&field.before, &typed) && !field.after.starts_with(closing))
        {
            return false;
        }
        let now = self.monotonic();
        let deadline = if let Some(waited) = waiting.filter(|waited| Arc::ptr_eq(&waited.plan, plan)) {
            waited.deadline
        } else {
            let catch_up = self.field_catch_up_seconds();
            if catch_up <= 0.0 {
                return false;
            }
            technical_event!(
                self.shared, "correction_awaiting_field", "mode": plan.mode, "application": plan.application,
                "catch_up_ms": ms(catch_up),
            );
            now + catch_up
        };
        if now >= deadline {
            technical_event!(self.shared, "field_wait_expired", "mode": plan.mode, "application": plan.application);
            return false;
        }
        self.field_wait = Some(FieldWait { plan: plan.clone(), learning_prompt: learning_prompt.cloned(), deadline });
        true
    }

    /// A correction waiting for its field (waits_for_field) is tried again once no key is held down; a correction
    /// planned since then supersedes it, and it is given up as an unseen field always was.
    pub fn retry_field_wait(&mut self) {
        let Some(waiting) = self.field_wait.clone() else {
            return;
        };
        if !self.pressed.is_empty() || !self.modifier_keycodes.is_empty() {
            return;
        }
        if self.pending.is_some() {
            self.field_wait = None;
            technical_event!(self.shared, "field_wait_dropped", "mode": waiting.plan.mode, "reason": "correction_pending");
            return;
        }
        self.execute_correction(waiting.plan, waiting.learning_prompt);
    }

    /// Keys typed after the word but before its correction lands.
    ///
    /// Their characters already follow the word on screen, so the backend
    /// deletes them with the word and types them again in the new layout;
    /// they come back through the hook as fresh input. Called while the hook
    /// holds input, so the queue cannot grow in the meantime. Anything that is
    /// not plain text (Enter, a caret move, a shortcut) makes the outcome
    /// unpredictable; then nothing is touched and the keys stay queued.
    fn collect_late_input(&mut self, plan: &CorrectionPlan) -> Option<Strokes> {
        let planned: Vec<*const KeyEvent> = plan.strokes.iter().chain(plan.trailing.iter()).map(Arc::as_ptr).collect();
        let rollover: Strokes = self.strokes.iter().filter(|stroke| !is_planned(&planned, stroke)).cloned().collect();
        let queued = self.shared.take_queued();
        let mut late = rollover.clone();
        let mut kept: Vec<QueueItem> = Vec::new();
        let mut refusal: &'static str = rollover.iter().find(|stroke| !Self::_late_text_key(stroke)).map_or("", |stroke| Self::_late_key_kind(stroke));
        for item in &queued {
            match item {
                QueueItem::Key(key) => {
                    if key.synthetic {
                        kept.push(item.clone());
                    } else if is_modifier_key(&key.key_name) {
                        if refusal.is_empty() {
                            refusal = "modifier";
                        }
                        kept.push(item.clone());
                    } else if !key.pressed || self.matches_hotkey("convert_last", key) {
                        // A key-up types nothing, and neither does a second press of the
                        // conversion hotkey: it is answered once this correction lands.
                        kept.push(item.clone());
                    } else if Self::_late_text_key(key) && Self::_safe_text_stroke(key) {
                        late.push(key.clone());
                    } else if refusal.is_empty() {
                        refusal = Self::_late_key_kind(key);
                    }
                }
                QueueItem::LayoutSelection(_) | QueueItem::Stop => {
                    if refusal.is_empty() {
                        refusal = if matches!(item, QueueItem::Stop) { "stop" } else { "menu_layout" };
                    }
                    kept.push(item.clone());
                }
            }
        }
        self.late_refusal = refusal.to_owned();
        let usable = refusal.is_empty();
        if !usable || late.is_empty() {
            self.shared.put_back(queued);
            return if usable { Some(Vec::new()) } else { None };
        }
        self.shared.put_back(kept);
        if !rollover.is_empty() {
            self.strokes.retain(|stroke| is_planned(&planned, stroke));
            // Letters begun after the boundary go with the correction and come back, typed again
            // in the new layout, as the start of a word: with none of the word left, nothing ties
            // it to the old layout. Kept, that layout read the first key coming back as a layout
            // change in the middle of a word and cleared the observed text with it: `тфеы и` with
            // the `и` down before the space came up became `nats b`, and the `b` lost the
            // `и redis` it gets when typed after the space.
            if self.strokes.is_empty() {
                self.source_group = -1;
            }
        }
        Some(late)
    }

    pub fn maybe_execute_pending(&mut self, event: &KeyEvent) -> EngineResult<()> {
        if self.selection_pending.is_some() {
            self.advance_selection_command();
            return Ok(());
        }
        if self.deferred_action.is_some() && self.pressed.is_empty() && self.modifier_keycodes.is_empty() {
            let plan = self.pending.take();
            self.learning_prompt_after = None;
            let corrected = plan.is_some();
            let succeeded = match plan {
                None => true,
                Some(plan) => self.execute_correction(plan, None),
            };
            if self.deferred_action.is_some() {
                self.complete_deferred_action(succeeded, if corrected { "corrected" } else { "no_correction" });
            }
            self.clear_word(None, "action_completed");
            self.contexts.clear();
            self.context_policy.stream.clear();
            return Ok(());
        }
        if self.pending.is_none() {
            return Ok(());
        }
        if i64::from(event.keycode) == self.pending_trigger_keycode && !event.pressed {
            self.pending_trigger_keycode = -1;
        }
        if self.pending_trigger_keycode != -1 || !self.modifier_keycodes.is_empty() || !self.pressed.is_empty() {
            return Ok(());
        }
        let mut plan = self.pending.take().expect("checked above");
        self.manual_release_deadline = 0.0;
        let learning_prompt = self.learning_prompt_after.take();
        if matches!(plan.mode, "early" | "early_undo" | "late_stroke") {
            let Some(refreshed) = self.refresh_early_plan(&plan)? else {
                technical_event!(
                    self.shared, "early_switch_dropped",
                    "reason": "word_changed_before_release",
                    "current_word_length": self.strokes.len(),
                );
                return Ok(());
            };
            plan = refreshed;
        }
        if plan.mode == "mention_hidden" && plan.boundary.is_none() && plan.trailing.is_empty() {
            // Letters typed before the first one came up stand after the "@" too.
            let typed = self.strokes.clone();
            if typed.len() >= plan.strokes.len() && typed[..plan.strokes.len() - 1] == plan.strokes[1..] {
                plan = self.mention_write_back(&plan.strokes[0], &typed, &[], None, &plan.application);
            }
        }
        self.execute_correction(plan, learning_prompt);
        Ok(())
    }

    /// `execute_correction`; what a replay asks to see is noted here, on success, as the plan was asked to run.
    pub fn execute_correction(&mut self, plan: PlanRef, learning_prompt: Option<LearningPrompt>) -> bool {
        let requested = plan.clone();
        let ok = self.execute_correction_inner(plan, learning_prompt);
        if ok && let Some(executed) = self.executed.as_mut() {
            executed.push(ExecutedPlan {
                mode: requested.mode,
                original: requested.original.clone(),
                replacement: requested.replacement.clone(),
                automatic: requested.automatic,
            });
        }
        ok
    }

    fn execute_correction_inner(&mut self, plan: PlanRef, learning_prompt: Option<LearningPrompt>) -> bool {
        let requested = plan.clone();
        let mut plan = plan;
        let waiting = self.field_wait.take();
        if !plan.head.is_empty() {
            // The word decided; the mention head is rewritten with it, and the
            // rule the user may confirm still names the word alone.
            let head = plan.head.clone();
            plan = plan.replaced_with(|plan| {
                plan.original = format!("{}{}", text_for_group(&head, plan.source_group), plan.original);
                plan.replacement = format!("{}{}", text_for_group(&head, plan.target_group), plan.replacement);
                let mut strokes = head.clone();
                strokes.append(&mut plan.strokes);
                plan.strokes = strokes;
                plan.head = Vec::new();
            });
            self.symbol_strokes = Vec::new();
            self.mention_shown = None;
            technical_event!(
                self.shared, "mention_head_applied",
                "mode": plan.mode,
                "application": plan.application,
                "application_excluded": self.application_excluded(&plan.application),
                "source_group": plan.source_group,
                "target_group": plan.target_group,
            );
        }
        if self.track_focus().changed {
            technical_event!(self.shared, "correction_aborted", "mode": plan.mode, "reason": "focus_changed");
            return false;
        }
        if self.shared.input_overflow.load(Ordering::Relaxed) {
            self.clear_word(None, "input_overflow");
            technical_event!(self.shared, "correction_aborted", "mode": plan.mode, "reason": "input_overflow");
            return false;
        }
        if self.take_hook_gap() {
            technical_event!(self.shared, "correction_aborted", "mode": plan.mode, "reason": "hook_gap");
            return false;
        }
        if plan.strokes.iter().chain(plan.trailing.iter()).any(|stroke| !Self::_safe_text_stroke(stroke)) {
            self.clear_word(None, "unrepresentable_text");
            technical_event!(
                self.shared, "correction_aborted", "mode": plan.mode, "reason": "unrepresentable_text",
                "character_lengths": plan.strokes.iter().map(|stroke| stroke.characters.iter().map(|char| pystr::len(char)).collect::<Vec<_>>()).collect::<Vec<_>>(),
            );
            return false;
        }
        if !plan.context_field.is_empty() {
            let field = self.context_policy.reader.as_ref().and_then(|reader| reader.read(&plan.application, self.focus_or_zero()));
            let mut suffix = plan.original.clone();
            suffix.extend(plan.trailing.iter().map(|stroke| stroke.character.as_str()));
            suffix.push_str(plan.boundary.as_ref().map_or("", |boundary| boundary.character.as_str()));
            // Read after the field: whatever the field can show was typed by then.
            let typed: Strokes = plan.strokes.iter().chain(plan.trailing.iter()).cloned().collect();
            let ahead = self.typed_ahead(&typed);
            let refusal = Self::_field_refusal(&plan, field.as_ref(), &suffix, &ahead);
            if refusal == "other_text" && plan.mode == "early" && field.as_ref().is_some_and(|field| Self::_typed_in_field(field, &plan.original, &ahead).is_some()) {
                // The field has not taken the last letters in yet (_typed_in_field). The word is
                // still the one typed: it stays tracked for a later letter or its boundary.
                technical_event!(self.shared, "early_switch_dropped", "reason": "field_behind", "current_word_length": self.strokes.len());
                return false;
            }
            if refusal == "other_text"
                && plan.mode != "early"
                && let Some(field) = &field
                && self.waits_for_field(&requested, learning_prompt.as_ref(), field, waiting.as_ref())
            {
                return false;
            }
            if !refusal.is_empty() {
                let typed_after_boundary = !self.strokes.is_empty();
                self.clear_word(None, "context_field_changed");
                // The letters already typed for the next word stand somewhere
                // the engine can no longer vouch for. The rest of that token is
                // left alone rather than judged on its own: "все" must not turn
                // into "в" + "се" with only the tail converted.
                self.untracked_token = self.untracked_token || typed_after_boundary;
                technical_event!(
                    self.shared, "correction_aborted", "mode": plan.mode, "reason": "context_field_changed",
                    "letters_untracked": typed_after_boundary, "field_check": refusal, "typed_ahead": pystr::len(&ahead),
                );
                return false;
            }
        }
        let application_excluded = self.application_excluded(&plan.application);
        let logged_original = if application_excluded { "<redacted>".to_owned() } else { plan.original.clone() };
        let logged_replacement = if application_excluded { "<redacted>".to_owned() } else { plan.replacement.clone() };
        let previous_group = self.shared.current_group();
        let started = self.monotonic();
        let typed_before = self.shared.typed_events.load(Ordering::Relaxed);
        let presses_before = self.shared.typed_presses.load(Ordering::Relaxed);
        self.correction_sequence += 1;
        let correction_id = self.correction_sequence;
        // From here on the hook keeps the user's keys back; whatever was typed
        // before this moment is collected and typed again after the word.
        let mut held: i64 = 0;
        let mut failure: Option<BackendError> = None;
        let mut unsafe_input = false;
        if let Err(error) = self.backend.try_hold_input() {
            failure = Some(error);
        }
        let planned: Vec<*const KeyEvent> = plan.strokes.iter().chain(plan.trailing.iter()).map(Arc::as_ptr).collect();
        // Letters of the next word the engine has taken already, and so the observed text too.
        let rolled: String = self.strokes.iter().filter(|stroke| !is_planned(&planned, stroke)).map(|stroke| stroke.character.as_str()).collect();
        let late = if failure.is_some() { Some(Vec::new()) } else { self.collect_late_input(&plan) };
        match late.as_ref() {
            _ if failure.is_some() => {}
            None => {
                if plan.mode == "early" {
                    // The keys stay queued with the word, and its boundary decides it: each of the 14 early
                    // switches field logs showed given up this way (01-26.10.2026) was followed by the
                    // correction at its space within 0.2 s. Logged as a correction lost, they read as 14 losses.
                    technical_event!(
                        self.shared, "early_switch_dropped", "reason": "unsafe_input_after_word", "unsafe_key": self.late_refusal,
                        "current_word_length": self.strokes.len(),
                    );
                } else {
                    technical_event!(
                        self.shared, "correction_aborted", "correction_id": correction_id,
                        "mode": plan.mode, "reason": "unsafe_input_after_word", "unsafe_key": self.late_refusal,
                    );
                }
                self.last_committed_stale = true;
                unsafe_input = true;
            }
            Some(late) => match self.inject_inner(&plan, late) {
                Ok(count) => held = i64::from(count),
                Err(error) => failure = Some(error),
            },
        }
        match self.backend.release_input() {
            Ok(count) => held += i64::from(count),
            Err(error) => failure = Some(error),
        }
        if let Some(error) = failure {
            technical_event!(
                self.shared, "correction_failed",
                "correction_id": correction_id,
                "mode": plan.mode,
                "original": logged_original,
                "replacement": logged_replacement,
                "source_group": plan.source_group,
                "target_group": plan.target_group,
                "application": plan.application,
                "application_excluded": application_excluded,
                "automatic": plan.automatic,
                "keys_during_injection": self.shared.typed_events.load(Ordering::Relaxed) - typed_before,
                "keypresses_during_injection": self.shared.typed_presses.load(Ordering::Relaxed) - presses_before,
                "late_keys": late.as_ref().map_or(0, Vec::len),
                "text_verified": false,
                "error": error.to_string(),
            );
            self.note_unconfirmed_switch(&error);
            self.early_switch_origin = None;
            self.early_switch_at = None;
            self.clear_word(None, "injection_failed");
            self.shared.update(Update {
                last_error: Some(error.to_string()),
                last_action: Some("Ошибка замены · проверьте текст в приложении".to_owned()),
                ..Update::default()
            });
            return false;
        }
        if unsafe_input {
            return false;
        }
        let late = late.unwrap_or_default();
        let typed_events = self.shared.typed_events.load(Ordering::Relaxed);
        let typed_presses = self.shared.typed_presses.load(Ordering::Relaxed);
        // A word converted before a kept neighbour leaves the layout where the neighbour was typed.
        let layout_after = if plan.kept_tail != 0 { plan.source_group } else { plan.target_group };
        self.note_engine_switch(layout_after);
        let replayed_only_releases = held > 0 && typed_events - typed_before == held && typed_presses == presses_before;
        // The keys held back while the word was replaced are typed again after it, in order, and reach
        // the engine after this line like any typed key: the text before them is still the context.
        // Cleared, it took `нажимаю на` from the `b` after an early-switched `Pause`, and the letter
        // stayed `b`. Only input that came through unheld (a click) leaves the text unknown.
        let unheld_input = held > 0 && typed_events - typed_before != held;
        let mut context_reset_reason = if !late.is_empty() {
            "late_input"
        } else if unheld_input {
            "held_text_or_unknown"
        } else {
            ""
        };
        if context_reset_reason == "late_input" && !(held != 0 && !replayed_only_releases) && self.context_policy.stream.withdraw(&rolled) {
            // The late keys come back typed again and are observed then: the text before them
            // stays the context of the next word. Cleared, it left the word after `nats` with
            // nothing before it, and the model declined `b redis` as `и redis` (rollover typing).
            context_reset_reason = "";
        }
        if !context_reset_reason.is_empty() {
            self.context_policy.stream.clear();
        } else {
            // The shown "@" is the replacement itself, not a literal after it.
            let boundary: String = if plan.mode == "mention_shown" {
                String::new()
            } else {
                let mut text: String = plan.trailing.iter().map(|stroke| stroke.character.as_str()).collect();
                text.push_str(plan.boundary.as_ref().map_or("", |boundary| boundary.character.as_str()));
                text
            };
            self.context_policy.stream.replace_suffix(&plan.original, &plan.replacement, &boundary);
        }
        let erased_strokes = if plan.replaced_strokes.is_empty() { plan.strokes.len() } else { plan.replaced_strokes.len() };
        technical_event!(
            self.shared, "correction_applied",
            "correction_id": correction_id,
            "text_verified": false,
            "mode": plan.mode,
            "original": logged_original,
            "replacement": logged_replacement,
            "source_group": plan.source_group,
            "target_group": plan.target_group,
            "previous_group": previous_group,
            "layout_switched": plan.source_group != plan.target_group,
            "deleted_characters": erased_strokes + plan.trailing.len() + usize::from(plan.boundary.is_some()) + late.len(),
            "literal_characters": plan.trailing.len(),
            "replayed_strokes": plan.strokes.len(),
            "boundary_replayed": plan.boundary.is_some(),
            "injection_ms": ms(self.monotonic() - started),
            // Legacy field counts presses AND releases, not typed characters.
            "keys_during_injection": typed_events - typed_before,
            "keypresses_during_injection": typed_presses - presses_before,
            "queued_events": self.shared.qsize(),
            "late_keys": late.len(),
            "held_keys": held,
            "context_reset_reason": context_reset_reason,
            "replayed_only_releases": replayed_only_releases,
            "application": plan.application,
            "application_excluded": application_excluded,
            "automatic": plan.automatic,
            "confidence": logged(plan.confidence),
            "boundary": plan.boundary.as_ref().map(|boundary| boundary.key_name.as_str()),
        );
        if plan.mode == "undo" && !plan.replaced_strokes.is_empty() {
            // A typo correction typed other keys than the user's: its undo typed those back (_undo_of), and the word is
            // the user's own from now on (personal_words). Not before the undo lands: one given up (a modifier still
            // coming up, another window) leaves the correction on screen, and the word would stay uncorrected after.
            let word = text_for_group(&plan.strokes, plan.source_group);
            self.personal.add(&word);
            technical_event!(self.shared, "personal_word_added", "reason": "correction_undone", "letters": pystr::len(&word));
        }
        if plan.mode == "mention_shown" || plan.mode == "mention_hidden" {
            // Only the head changed; the word being typed goes on as it was.
            self.mention_shown = if plan.mode == "mention_shown" { self.symbol_strokes.first().cloned() } else { None };
            self.shared.update(Update { current_group: Some(plan.target_group), last_error: Some(String::new()), ..Update::default() });
            return true;
        }
        if plan.mode == "early" || plan.mode == "late_stroke" {
            // The prefix is finished later; only then does it become a
            // correction that can be undone or listed in the history.
            if plan.mode == "early" {
                self.early_switch_origin = Some(plan.source_group);
                self.early_switch_at = Some(self.monotonic());
                self.early_switch_confidence = plan.confidence;
            }
            self.source_group = plan.target_group;
            self.shared.update(Update {
                current_group: Some(plan.target_group),
                current_word: Some(text_for_group(&self.strokes, plan.target_group)),
                last_error: Some(String::new()),
                ..Update::default()
            });
            return true;
        }
        if plan.mode == "early_undo" {
            self.early_switch_origin = None;
            self.early_switch_at = None;
            self.source_group = plan.target_group;
            self.early_switch_undone = true;
            self.manual_layout_group = self.flag("detection.respect_manual_layout", true).then_some(plan.target_group);
            self.manual_layout_observed_at = Some(self.monotonic());
            self.manual_layout_source = "early_undo".to_owned();
            self.manual_layout_word = true;
            self.shared.update(Update {
                current_group: Some(plan.target_group),
                current_word: Some(text_for_group(&self.strokes, plan.target_group)),
                last_action: Some(format!("{} → {} · раннее переключение отменено", plan.original, plan.replacement)),
                last_error: Some(String::new()),
                ..Update::default()
            });
            return true;
        }
        self.remember_correction(&plan);
        self.last_requested_plan = Some(requested.clone());
        if matches!(&self.convert_outcome, Some(ConvertOutcome::Plan(outcome)) if Arc::ptr_eq(outcome, &requested)) && self.convert_press_at != 0.0 {
            // A second press of the hotkey made while this conversion was being
            // typed waited behind it in the hook; slow typing into a slow program
            // must not turn a double press into two single ones, so the window
            // for the second press starts once the word is on screen.
            self.convert_press_at = self.monotonic();
        }
        if plan.mode == "symbols" {
            self.shared.update(Update {
                current_group: Some(plan.target_group),
                last_action: Some(format!("{} → {}", plan.original, plan.replacement)),
                last_error: Some(String::new()),
                ..Update::default()
            });
            for callback in self.shared.correction_callbacks() {
                callback(&plan);
            }
            return true;
        }
        // Pause right after a correction converts the same word back.
        self.last_committed = Some(Arc::new(CorrectionPlan {
            automatic: false,
            trailing: plan.trailing.clone(),
            ..CorrectionPlan::new(
                plan.strokes.clone(), plan.boundary.clone(), plan.target_group, plan.source_group, plan.replacement.clone(),
                if plan.typed.is_empty() { plan.original.clone() } else { plan.typed.clone() },
                plan.confidence, &plan.application,
            )
        }));
        // A sign plan typed no key in another layout: Pause has nothing of it to convert back.
        self.last_committed_stale = !context_reset_reason.is_empty() || plan.mode == "sign";
        let deadline = self.monotonic() + CONTEXT_TTL;
        let window = self.focus_or_zero();
        self.provisional = (plan.revisit.is_some() && plan.boundary.is_some() && context_reset_reason.is_empty())
            .then(|| ProvisionalWord { plan: plan.clone(), window, deadline });
        self.paused_conversion = (plan.revisit.is_some() && plan.boundary.is_none() && context_reset_reason.is_empty())
            .then(|| ProvisionalWord { plan: plan.clone(), window, deadline });
        if plan.boundary.is_none() && plan.trailing.is_empty() && !plan.sign_replayed && context_reset_reason.is_empty() && any_alpha(&plan.replacement) {
            // Idle/manual correction did not end the word. Keep its physical
            // prefix so continued typing and Backspace still refer to the
            // whole token instead of a detached suffix.
            self.strokes = plan.strokes.clone();
            self.source_group = plan.target_group;
            self.shared.update(Update { current_word: Some(plan.replacement.clone()), ..Update::default() });
            if !plan.automatic {
                self.manual_layout_group = Some(plan.target_group);
                self.manual_layout_observed_at = Some(self.monotonic());
                self.manual_layout_source = "current_word".to_owned();
                self.manual_layout_word = true;
            }
        }
        if plan.kept_tail == 0 {
            // The kept neighbour was remembered at its own boundary and is still the last word.
            self.remember_context(&plan.application, plan.target_group, &plan.strokes);
        }
        let count = self.shared.correction_count() + i64::from(plan.automatic);
        self.shared.update(Update {
            current_group: Some(layout_after),
            correction_count: Some(count),
            last_action: Some(format!("{} → {}", plan.original, plan.replacement)),
            last_error: Some(String::new()),
            ..Update::default()
        });
        if plan.automatic {
            self.record_history(&plan.original, &plan.replacement, &plan.application, plan.confidence);
        }
        for callback in self.shared.correction_callbacks() {
            callback(&plan);
        }
        if let Some(prompt) = learning_prompt {
            self.shared.show_learning_prompt(prompt);
        }
        true
    }

    /// The backend's injection for this plan.
    fn inject_inner(&mut self, plan: &CorrectionPlan, late: &[Stroke]) -> Result<i32, BackendError> {
        let late = owned(late);
        let source = Some(plan.source_group.to::<i32>());
        if plan.mode == "sign" {
            // Every key of the token, the signs after it and its boundary go out as literals of the layout the
            // sign belongs to; nothing is converted, and the backend restores the current layout after them.
            let meant = (self.other_groups(plan.source_group)[0]).to::<i32>();
            let literal: Vec<KeyEvent> = plan
                .strokes
                .iter()
                .chain(plan.trailing.iter())
                .chain(plan.boundary.iter())
                .map(|stroke| KeyEvent { group: meant, character: stroke.character_for(meant).to_owned(), ..(**stroke).clone() })
                .collect();
            return self.backend.inject_correction(Injection {
                strokes: &[],
                target_group: plan.source_group.to::<i32>(),
                boundary: None,
                source_group: source,
                late: &late,
                trailing: &literal,
                kept_tail: 0,
                erased: None,
            });
        }
        let strokes = owned(&plan.strokes);
        let trailing = owned(&plan.trailing);
        let boundary = plan.boundary.as_ref().map(|boundary| (**boundary).clone());
        let mut injection = Injection {
            strokes: &strokes,
            target_group: plan.target_group.to::<i32>(),
            boundary: boundary.as_ref(),
            source_group: source,
            late: &late,
            trailing: &trailing,
            kept_tail: 0,
            erased: None,
        };
        if !plan.replaced_strokes.is_empty() && plan.replaced_strokes.len() != plan.strokes.len() {
            // A typo correction (or its undo) types more or fewer keys than the word on screen has.
            if !self.backend.erases_any_length() {
                return Err(BackendError::Failed("Бэкенд не умеет стирать другое число символов; замена отменена".to_owned()));
            }
            injection.erased = Some(plan.replaced_strokes.len());
        } else if plan.kept_tail != 0 {
            injection.kept_tail = plan.kept_tail;
        }
        self.backend.inject_correction(injection)
    }
}
