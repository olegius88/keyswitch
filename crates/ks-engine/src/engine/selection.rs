//! The selection hotkeys: a command waits for its keys to come up, then reads the selection and types the result.

use super::logging::technical_event;
use super::*;
use ks_backend::{RefusalReason, SelectedText, SelectionError, SelectionRefused};
use ks_config::selection_commands::{checked_selection, command_of_hotkey, plan_selection, refusal_message, PendingSelection, SelectionCommand, SelectionPlan};
use serde_json::{json, Map};

fn lengths(selected: Option<&SelectedText>, plan: &SelectionPlan) -> Map<String, Value> {
    // The selection as the log may describe it: where it was read, how long it was and what it became.
    let mut fields = Map::new();
    if let Some(selected) = selected {
        fields.insert("source".into(), json!(selected.source));
        fields.insert("selected_characters".into(), json!(pystr::len(&selected.text)));
        fields.insert("replacement_characters".into(), json!(pystr::len(&plan.replacement)));
        fields.insert("source_group".into(), json!(plan.source_group));
        fields.insert("target_group".into(), json!(plan.target_group));
    }
    fields
}

impl EngineCore {
    // A selection hotkey: its command runs once every key is up, as a Pause conversion waits for them.
    //
    // Text typed while a modifier of the hotkey is still held would come out as shortcuts (Ctrl+C under a held Alt is
    // Ctrl+Alt+C). The selection is not the word being typed, so that word and whatever waits for it are let go
    // first: a correction of it typed after the selection changed would land in the wrong place.
    pub(crate) fn _schedule_selection_command(&mut self, name: &str) {
        let Some(command) = command_of_hotkey(name) else {
            return;
        };
        self._clear_word(None, "selection_command");
        self._selection_pending = Some(PendingSelection { command, deadline: self.monotonic() + MANUAL_RELEASE_TIMEOUT_SECONDS });
        technical_event!(self.shared, "selection_command_scheduled", "command": command.as_str());
    }

    pub fn _advance_selection_command(&mut self) {
        let Some(pending) = self._selection_pending else {
            return;
        };
        if !self._pressed.is_empty() || !self._modifier_keycodes.is_empty() {
            return;
        }
        self._selection_pending = None;
        self._run_selection_command(pending.command);
    }

    /// A selection hotkey whose keys were not seen to come up in time changes nothing.
    ///
    /// As with _expire_manual_correction, a timeout is no proof that the keys are up, so nothing is typed.
    pub(crate) fn _expire_selection_command(&mut self) {
        let Some(pending) = self._selection_pending else {
            return;
        };
        if self.monotonic() < pending.deadline {
            return;
        }
        self._selection_pending = None;
        let mut fields = Map::new();
        fields.insert("reason".into(), json!("release_timeout"));
        fields.insert("pressed_keycodes".into(), json!(sorted_keycodes(&self._pressed)));
        fields.insert("modifier_keycodes".into(), json!(sorted_keycodes(&self._modifier_keycodes)));
        self._log_selection(pending.command, "refused", self.monotonic(), fields);
        self.shared._update(Update { last_action: Some(refusal_message(RefusalReason::ReleaseTimeout)), ..Update::default() });
    }

    /// Why a selection hotkey may not touch the focused field: the checks every correction passes.
    fn _selection_refusal(&self, application: &str, own_window: bool) -> Option<SelectionRefused> {
        let reason = if !self.flag("enabled", true) {
            RefusalReason::Disabled
        } else if own_window {
            // KeySwitch's own window answers accessibility requests on the thread that would be waiting for this one.
            RefusalReason::OwnWindow
        } else if self.shared._sensitive_context_window().is_some() && self.shared._sensitive_context_window() == self.shared._focus_window() {
            RefusalReason::SensitiveField
        } else if self._application_excluded(application) {
            RefusalReason::ExcludedApplication
        } else if !self.backend.supports_selection() {
            RefusalReason::Unsupported
        } else {
            return None;
        };
        Some(SelectionRefused::new(reason))
    }

    /// Read the selection, change it and type the result over it; the log gets lengths, never the text.
    pub fn _run_selection_command(&mut self, command: SelectionCommand) {
        let started = self.monotonic();
        let application = self.backend.active_application();
        let focus = self.backend.focused_window();
        if let Some(refusal) = self._selection_refusal(&application, focus.is_some_and(|focus| focus.own)) {
            let mut fields = Map::new();
            fields.insert("reason".into(), json!(refusal.reason.as_str()));
            fields.insert("application".into(), json!(application));
            self._log_selection(command, "refused", started, fields);
            self.shared._update(Update { last_action: Some(refusal_message(refusal.reason)), ..Update::default() });
            return;
        }
        let backend = self.backend.clone();
        let mut selected: Option<SelectedText> = None;
        let mut plan = SelectionPlan { replacement: String::new(), source_group: None, target_group: None };
        let mut reselected = false;
        let hold = backend.try_hold_input();
        let hold_failed = hold.is_err();
        let mut outcome: Result<(), SelectionError> = (|| {
            hold.map_err(SelectionError::Failed)?;
            selected = Some(backend.read_selection(&application, focus.map_or(0, |focus| focus.window))?);
            let text = checked_selection(&selected.as_ref().expect("just read").text)?.to_owned();
            let group = backend.current_group().map_err(SelectionError::Failed)?;
            plan = plan_selection(command, &text, i64::from(group), &self._layout_names())?;
            if plan.changes(&text) {
                let read = selected.as_ref().expect("just read");
                reselected = backend.replace_selection(read, &plan.replacement, plan.target_group.map(|group| group.to::<i32>()))?;
            }
            Ok(())
        })();
        if !hold_failed
            && let Err(error) = backend.release_input()
        {
            outcome = Err(SelectionError::Failed(error));
        }
        match outcome {
            Err(SelectionError::Refused(error)) => {
                let mut fields = lengths(selected.as_ref(), &plan);
                fields.insert("reason".into(), json!(error.reason.as_str()));
                fields.insert("application".into(), json!(application));
                self._log_selection(command, "refused", started, fields);
                self.shared._update(Update { last_action: Some(refusal_message(error.reason)), ..Update::default() });
                return;
            }
            Err(SelectionError::Failed(error)) => {
                // The type names the failure for the log; the message, which names the step, goes to the status line.
                self._note_unconfirmed_switch(&error);
                self._clear_word(None, "selection_failed");
                let mut fields = lengths(selected.as_ref(), &plan);
                // The type names the failure for the log.
                let name = if matches!(error, ks_backend::BackendError::LayoutSwitchUnconfirmed { .. }) { "LayoutSwitchUnconfirmed" } else { "BackendError" };
                fields.insert("error".into(), json!(name));
                fields.insert("application".into(), json!(application));
                self._log_selection(command, "failed", started, fields);
                self.shared._update(Update {
                    last_error: Some(error.to_string()),
                    last_action: Some("Ошибка замены выделения · проверьте текст в приложении".to_owned()),
                    ..Update::default()
                });
                return;
            }
            Ok(()) => {}
        }
        let changed = selected.as_ref().is_some_and(|selected| plan.changes(&selected.text));
        if changed {
            // The text around the caret is no longer what the engine saw typed.
            self._clear_word(None, "selection_replaced");
        }
        if changed && let Some(target) = plan.target_group {
            // As Pause with no word to convert: the layout is the user's explicit choice and protects the next word.
            self._note_engine_switch(target);
            let protects = self.flag("detection.respect_manual_layout", true);
            self._manual_layout_group = protects.then_some(target);
            self._manual_layout_observed_at = Some(self.monotonic());
            self._manual_layout_source = "selection_layout".to_owned();
            self._manual_layout_word = false;
            self.shared._update(Update { current_group: Some(target), ..Update::default() });
        }
        let mut fields = lengths(selected.as_ref(), &plan);
        fields.insert("application".into(), json!(application));
        fields.insert("reselected".into(), json!(reselected));
        self._log_selection(command, if changed { "applied" } else { "unchanged" }, started, fields);
        self.shared._update(Update {
            last_action: Some(if changed { command.title().to_owned() } else { "Выделение не изменилось".to_owned() }),
            last_error: Some(String::new()),
            ..Update::default()
        });
    }

    fn _layout_names(&self) -> Vec<String> {
        let names: Vec<String> = self.settings.with_value("detection.layouts", |names| match names {
            Some(Value::Array(items)) => items.iter().map(|item| item.as_str().map_or_else(|| item.to_string(), str::to_owned)).collect(),
            _ => vec!["us".to_owned(), "ru".to_owned()],
        });
        names.into_iter().take(self.models.len()).collect()
    }

    fn _log_selection(&self, command: SelectionCommand, outcome: &str, started: f64, fields: Map<String, Value>) {
        if !self.shared.technical_logging() {
            return;
        }
        let mut payload = Map::new();
        payload.insert("schema".into(), json!(1));
        payload.insert("event".into(), json!("selection_command"));
        payload.insert("command".into(), json!(command.as_str()));
        payload.insert("outcome".into(), json!(outcome));
        payload.insert("elapsed_ms".into(), json!(ms(self.monotonic() - started)));
        payload.extend(fields);
        self.shared.emit_technical(payload);
    }
}
