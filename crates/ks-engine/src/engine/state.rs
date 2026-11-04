//! What resets or watches the engine's state: the timers of a held Enter and a manual correction, clearing the word,
//! the settings, the focus and the layout.

use super::logging::technical_event;
use super::*;
use ks_config::settings_diagnostics::setting_change;
use ks_config::pycompat::truthy;
use std::sync::atomic::Ordering;

impl EngineCore {
    pub(crate) fn _complete_deferred_action(&mut self, deliver: bool, reason: &str) {
        let action = self._deferred_action.take();
        self._action_deadline = 0.0;
        match self.backend.complete_action(deliver) {
            Ok(_) => {
                technical_event!(
                    self.shared, if deliver { "action_delivered" } else { "action_cancelled" },
                    "key_name": action.as_ref().map_or("unknown", |action| action.key_name.as_str()), "reason": reason,
                    "text_verified": false,
                );
                if !deliver {
                    self.shared._update(Update { last_action: Some("Enter/Tab не передан · проверьте текст и нажмите ещё раз".to_owned()), ..Update::default() });
                }
            }
            Err(error) => {
                technical_event!(self.shared, "action_failed", "reason": reason, "error": error.to_string(), "text_verified": false);
                self.shared._update(Update {
                    last_error: Some(error.to_string()),
                    last_action: Some("Ошибка Enter/Tab · проверьте приложение".to_owned()),
                    ..Update::default()
                });
            }
        }
    }

    /// Enter waited for every key to come up. Decide what a key that never did means.
    ///
    /// A press whose release was lost holds the action hostage: the engine forgets such
    /// a press only after three seconds, while the action gives up after two, so an
    /// Enter waiting behind an unrelated stuck key was dropped a second before the
    /// engine would have freed it - and a dropped Enter is a keystroke that vanishes
    /// with nothing on screen to explain it. A key already held when the Enter arrived
    /// and still held at the deadline is not coming up; it is forgotten and the action
    /// goes through. The action's own key is never forgotten that way: a release nobody
    /// saw is a press nobody finished, and that case keeps the cautious answer.
    pub fn _expire_deferred_action(&mut self) -> EngineResult<()> {
        let Some(action) = self._deferred_action.clone() else {
            return Ok(());
        };
        if self.monotonic() < self._action_deadline {
            return Ok(());
        }
        let now = self.monotonic();
        self._prune_stale_presses(now, now - (self._action_deadline - ACTION_TIMEOUT_SECONDS), Some(action.keycode));
        if !self._pressed.is_empty() || !self._modifier_keycodes.is_empty() {
            // Either the Enter itself was never seen to come up, or a key pressed after
            // it is still down. Both keep the cautious answer.
            self._clear_word(None, "action_release_timeout");
            return Ok(());
        }
        self._maybe_execute_pending(&action)
    }

    pub fn _expire_manual_correction(&mut self) {
        if self._pending.as_ref().is_some_and(|pending| !pending.automatic)
            && self._manual_release_deadline > 0.0
            && self.monotonic() >= self._manual_release_deadline
        {
            technical_event!(
                self.shared, "manual_conversion_timeout", "reason": "key_release_not_observed",
                "pressed_keycodes": sorted_keycodes(&self._pressed),
                "modifier_keycodes": sorted_keycodes(&self._modifier_keycodes),
            );
            // A timeout is not proof that a physical key is up. Do not inject,
            // switch layout or turn this uncertain attempt into learning.
            self._clear_word(Some("Замена отменена: не получено отпускание клавиш · проверьте текст"), "manual_release_timeout");
        }
        self._expire_selection_command();
    }

    /// `_expire_learning_prompt`, the fifth timer of the harness and the sixth of the live loop.
    pub fn _expire_learning_prompt(&self, now: Option<f64>) -> bool {
        self.shared._expire_learning_prompt(now)
    }

    pub fn _clear_word(&mut self, action: Option<&str>, reason: &str) {
        self._personal_word_settled(reason == "action_completed");
        self.context_policy.stream.clear();
        if let Some(waiting) = self._field_wait.take() {
            // A click, a caret move, Backspace or another window: the word may no longer stand where it was typed.
            technical_event!(
                self.shared, "field_wait_dropped", "mode": waiting.plan.mode,
                "reason": if reason.is_empty() { "word_cleared" } else { reason },
            );
        }
        // After a click, a caret move or another window nothing is known to
        // stand before the next key.
        self._last_key_character = String::new();
        self._position_unknown = true;
        self._insertion = None;
        self._cancel_context_wait(if reason.is_empty() { "word_cleared" } else { reason });
        if self._deferred_action.is_some() {
            self._complete_deferred_action(false, reason);
        }
        self._log_word_discarded(reason);
        self._log_pending_dropped(reason);
        self.shared.dismiss_learning_prompt(None, "word_cleared");
        if self._early_switch_origin.is_some() && !self._strokes.is_empty() {
            // The rewritten prefix stays on screen: record it so that the
            // history lists it and the undo hotkey can still revert it.
            let application = self.backend.active_application();
            self._finish_early_switch(self._strokes.clone(), None, &application, self._source_group, Vec::new());
            self._last_committed_stale = true;
        }
        self._early_switch_undone = false;
        if self._manual_layout_word {
            self._manual_layout_group = None;
            self._manual_layout_word = false;
        }
        self._strokes = Vec::new();
        self._symbol_strokes = Vec::new();
        // A shown "@" followed by the arrow keys, a click or Enter was the member
        // list at work: it stays on screen and is no longer tracked.
        self._mention_shown = None;
        self._source_group = -1;
        self._early_switch_origin = None;
        self._early_switch_at = None;
        self._reset_pause_correction();
        self._pending = None;
        self._learning_prompt_after = None;
        self._manual_release_deadline = 0.0;
        self._last_committed_stale = true;
        self.shared._update(Update { current_word: Some(String::new()), last_action: action.map(str::to_owned), ..Update::default() });
    }

    pub fn _settings_changed(&mut self, path: &str, value: &Value) {
        if matches!(path, "*" | "enabled" | "detection.context_aware" | "detection.context_policy" | "detection.context_read_field" | "exclusions.applications") {
            self.context_policy.stream.clear();
            self._cancel_context_wait("settings_changed");
            self.shared.set_sensitive_context_window(None);
        }
        self.shared.refresh_hook_settings();
        if path == "*" {
            self.shared.dismiss_learning_prompt(None, "settings_reloaded");
            self.shared._update(Update { enabled: Some(self.flag("enabled", true)), ..Update::default() });
            self._manual_layout_group = None;
        } else if path == "enabled" {
            self.shared._update(Update { enabled: Some(truthy(value)), ..Update::default() });
        } else if path == "detection.respect_manual_layout" && !truthy(value) {
            self._manual_layout_group = None;
        } else if path == "detection.learning" && !truthy(value) {
            self.shared.dismiss_learning_prompt(None, "learning_disabled");
        }
        if self.shared.technical_logging()
            && let Some(change) = setting_change(&self.settings, path, value)
        {
            let mut payload = serde_json::Map::new();
            payload.insert("schema".into(), serde_json::json!(1));
            payload.insert("event".into(), serde_json::json!("setting_changed"));
            payload.extend(change);
            self.shared.emit_technical(payload);
        }
        if path == "diagnostics.technical_logging" && truthy(value) {
            self._technical_session_event("technical_logging_enabled");
        }
    }

    /// Notice the user moving to another window.
    pub(crate) fn _track_focus(&mut self) -> _FocusChange {
        let Some(focus) = self.shared.focused_window().filter(|focus| focus.window != 0) else {
            return _FocusChange { changed: false, ignore_layout: false };
        };
        if focus.own && !self.shared._test_field_focused() {
            return _FocusChange { changed: false, ignore_layout: focus.isolated_layout };
        }
        let previous = self.shared._focus_window();
        self.shared.set_focus_window(focus.window);
        match previous {
            Some(previous) if previous != focus.window => {
                self._focus_changed(previous, focus.window);
                _FocusChange { changed: true, ignore_layout: false }
            }
            _ => _FocusChange { changed: false, ignore_layout: false },
        }
    }

    /// The unfinished word and the last committed one stay in the old window.
    fn _focus_changed(&mut self, previous: i64, window: i64) {
        self._own_layout_ignored = false;
        self.shared.set_sensitive_context_window(None);
        self._convert_press_at = 0.0;
        let dropped = self._strokes.len();
        self._clear_word(None, "focus_changed");
        self._untracked_token = false;
        self._contexts.clear();
        self.context_policy.stream.clear();
        self._manual_layout_group = None;
        self._last_committed_stale = true;
        // A key held while the window changed reports its release to the window that
        // took over, and often to nobody the hook can see. Such a press then sits in
        // the engine's books until the stale timer forgets it three seconds later -
        // long enough for a withheld Enter, which gives up after two, to be dropped
        // while waiting for a key that will never come up. The moment the focus moves
        // is when that press stops meaning anything, so it is forgotten here rather
        // than by a timer. A key that really is still down loses nothing: its release,
        // if it ever arrives, simply finds nothing to clear.
        let mut held: Vec<u32> = self._pressed.iter().chain(self._modifier_keycodes.iter()).copied().collect();
        held.sort_unstable();
        held.dedup();
        self._pressed.clear();
        self._modifier_keycodes.clear();
        self._pressed_since.clear();
        technical_event!(
            self.shared, "focus_changed",
            "previous_window": previous,
            "window": window,
            "dropped_word_length": dropped,
            "released_keycodes": held,
        );
    }

    pub(crate) fn _observe_group(&mut self, group: i64, source: &str) {
        let focus = self._track_focus();
        if !focus.ignore_layout {
            // Any look outside an own window ends the ignored episode, so the
            // next visit to the settings window is logged once again.
            self._own_layout_ignored = false;
        }
        let current_group = self.shared.current_group();
        if !self.in_models(group) || group == current_group {
            return;
        }
        if focus.ignore_layout {
            // Windows keeps a layout per window: the settings window or the
            // learning prompt of KeySwitch itself says nothing about the
            // layout the user types in, so it neither protects nor updates.
            // Its test field is typed in as any program is (set_test_field_focused).
            // Every poll repeats the observation; the log needs it once.
            if !self._own_layout_ignored {
                self._own_layout_ignored = true;
                technical_event!(
                    self.shared, "layout_change_ignored",
                    "source": source,
                    "reason": "own_window",
                    "previous_group": current_group,
                    "selected_group": group,
                );
            }
            return;
        }
        let engine_switch_ms = self._engine_switch_at.map(|switched_at| ms(self.monotonic() - switched_at));
        // Only a change *to* the layout the engine itself just selected is the
        // engine's own switch; the user switching away right after a wrong
        // correction is manual and must protect the retyped word.
        let initiated_by_engine = engine_switch_ms.is_some_and(|elapsed| (elapsed as f64) <= ENGINE_SWITCH_GRACE_SECONDS * MILLISECONDS_PER_SECOND)
            && Some(group) == self._engine_switch_group;
        let respect = self.flag("detection.respect_manual_layout", true);
        // A layout that arrived together with another window is that window's
        // own layout (Windows keeps one per window), not a choice of the user.
        let protects = current_group >= 0 && respect && !initiated_by_engine && !focus.changed;
        technical_event!(
            self.shared, if protects { "manual_layout_observed" } else { "layout_change_observed" },
            "source": source,
            "previous_group": current_group,
            "selected_group": group,
            "application": self.backend.active_application(),
            "initiated_by_engine": initiated_by_engine,
            "focus_changed": focus.changed,
            "engine_switch_ms_ago": engine_switch_ms,
            "respect_manual_layout": respect,
            "protects_next_word": protects,
            "current_word_length": self._strokes.len(),
            "wait_id": self._context_waiting.as_ref().map(|waiting| waiting.diagnostic_id),
        );
        if focus.changed {
            // The manual pick belonged to the previous window.
            self._manual_layout_group = None;
        }
        if protects {
            self._manual_layout_group = Some(group);
            self._manual_layout_observed_at = Some(self.monotonic());
            self._manual_layout_source = source.to_owned();
            self._manual_layout_word = false;
            self.shared._update(Update {
                current_group: Some(group),
                last_action: Some("Ручная смена раскладки · следующее слово без автокоррекции".to_owned()),
                ..Update::default()
            });
            return;
        }
        self.shared._update(Update { current_group: Some(group), ..Update::default() });
    }

    pub fn _poll_current_group(&mut self) -> EngineResult<()> {
        let group = self.backend.current_group()?;
        self._observe_group(i64::from(group), "poll");
        Ok(())
    }

    /// `_update`, for a replay that sets the group as a test does.
    pub fn _update(&self, update: Update) {
        self.shared._update(update);
    }

    /// Whether the input overflowed since the worker last looked.
    pub fn input_overflow(&self) -> bool {
        self.shared._input_overflow.load(Ordering::Relaxed)
    }
}
