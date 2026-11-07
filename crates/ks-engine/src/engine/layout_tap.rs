//! A modifier pressed and released alone switches the layout (`detection.layout_switch_key`), as the switchers people come
//! from do it with a lone Shift or Ctrl.
//!
//! The tap is a keyboard switch, like the system's own Alt+Shift, not the menu's: the engine books it as the person's
//! choice (`observe_group`), so the next word stays as typed, a word being typed is let go by its next letter in the other
//! layout (`layout_changed_mid_word`), and Pause, undo, a waiting word and the learning prompt keep what they had. The key
//! itself goes on to the window unchanged: modifiers always pass (`Shared::consumes_key`).

use super::logging::technical_event;
use super::*;
use crate::constants::{LAYOUT_SWITCH_KEYS, LAYOUT_TAP_MAX_SECONDS};
use ks_config::indicator::alternate_layout_group;

/// `manual_layout_source` of a layout the person switched with the layout switch key.
pub const LAYOUT_TAP_SOURCE: &str = "modifier_tap";

/// What every settings window says of `detection.layout_switch_key`.
pub const LAYOUT_SWITCH_KEY_TITLE: &str = "Переключать раскладку одной клавишей";
pub const LAYOUT_SWITCH_KEY_DESCRIPTION: &str = "Нажатие и отпускание выбранной клавиши без других клавиш переключает \
                                                 раскладку EN ⇄ RU, как в Punto Switcher и Caramba; следующее слово \
                                                 остаётся как набрано. Shift с буквой, щелчок или прокрутка с зажатой \
                                                 клавишей, Ctrl+C и долгое удержание раскладку не меняют. Не включайте, \
                                                 если система уже переключает раскладку этой клавишей. С залипанием \
                                                 клавиш не работает. Если включён ввод от других программ, раскладку \
                                                 переключает и нажатие с удалённого компьютера";
pub const LAYOUT_SWITCH_KEY_LABELS: [(&str, &str); 5] =
    [("", "Выключено"), ("Shift_L", "Левый Shift"), ("Shift_R", "Правый Shift"), ("Control_L", "Левый Ctrl"), ("Control_R", "Правый Ctrl")];

/// The layout switch key of `settings`, as every backend names it (`Shift_L`, `Control_R`); None for none or for a value
/// not offered.
pub(crate) fn configured_layout_switch_key(settings: &SettingsStore) -> Option<&'static str> {
    settings.with_value("detection.layout_switch_key", |value| {
        let name = value.and_then(Value::as_str)?;
        LAYOUT_SWITCH_KEYS.iter().copied().find(|key| *key == name)
    })
}

impl EngineCore {
    /// The layout switch key of the settings; None when there is none.
    #[must_use]
    pub fn layout_switch_key(&self) -> Option<&'static str> {
        configured_layout_switch_key(&self.settings)
    }

    /// Anything but the held key itself ends a lone press: another key going down or up, a click, a turned wheel, a key
    /// another program injected that the settings do not count as typing, the rule window taking the keyboard. Called
    /// for every key before anything else looks at it.
    pub fn follow_layout_tap(&mut self, event: &KeyEvent) {
        let Some(tap) = self.layout_tap else { return };
        let ended = event.keycode != tap.keycode
            || self.layout_switch_key() != Some(event.key_name.as_str())
            || self.shared.input_suspended()
            || (event.foreign && !self.shared.accepts_injected(event));
        if ended {
            self.layout_tap = None;
        }
    }

    /// The layout switch key as the books of held keys just took it (the modifier branch of `handle_inner`). A press with
    /// no other key held starts a tap: Alt+Shift, Ctrl+Shift and both Shifts (the system's own switches) have the other
    /// key down first, and a key the engine still holds as down vetoes however long ago it went down, until the engine
    /// forgets it as it forgets any press whose release was lost (`prune_stale_presses`, a change of window). A repeat of
    /// the held key (`repeat`: Windows repeats a held Shift) leaves the tap as it was, so its time counts from the first
    /// press. The release within `LAYOUT_TAP_MAX_SECONDS` switches the layout.
    pub fn track_layout_tap(&mut self, event: &KeyEvent, repeat: bool) -> EngineResult<()> {
        if self.layout_switch_key() != Some(event.key_name.as_str()) {
            return Ok(());
        }
        if event.pressed {
            let others_held = self.pressed.iter().chain(&self.modifier_keycodes).any(|keycode| *keycode != event.keycode);
            if !repeat && !others_held {
                self.layout_tap = Some(LayoutTap { keycode: event.keycode, pressed_at: self.monotonic(), group: i64::from(event.group) });
            }
            return Ok(());
        }
        let Some(tap) = self.layout_tap.take() else {
            return Ok(());
        };
        let held = self.monotonic() - tap.pressed_at;
        if held > LAYOUT_TAP_MAX_SECONDS {
            technical_event!(
                self.shared, "layout_tap_ignored", "reason": "held_too_long", "key": event.key_name, "held_ms": ms(held),
                "limit_ms": ms(LAYOUT_TAP_MAX_SECONDS),
            );
            return Ok(());
        }
        // A window that came to the front while the key was down has its own layout (Windows keeps one per window).
        if self.track_focus().changed {
            return Ok(());
        }
        self.switch_layout_by_tap(tap, event, held)
    }

    fn switch_layout_by_tap(&mut self, tap: LayoutTap, event: &KeyEvent, held: f64) -> EngineResult<()> {
        let current = i64::from(self.backend.current_group()?);
        if self.in_models(tap.group) && current != tap.group {
            // The system switched the layout on this key itself (an XKB option such as grp:lshift_toggle): the switch is
            // booked as the person's and not made a second time, which would take the layout back.
            self.book_layout_tap(tap, event, held, current, "system");
            return Ok(());
        }
        let Some(target) = alternate_layout_group(current).filter(|target| self.in_models(*target)) else {
            technical_event!(self.shared, "layout_tap_ignored", "reason": "no_alternate_layout", "key": event.key_name, "current_group": current);
            return Ok(());
        };
        if let Err(error) = self.backend.switch_group(target.to::<i32>()) {
            technical_event!(self.shared, "layout_switch_failed", "source": LAYOUT_TAP_SOURCE, "requested_group": target, "error": error.to_string());
            // A switch asked for and not confirmed in time is the engine's when it comes, as the Pause's is.
            self.note_unconfirmed_switch(&error);
            self.shared.update(Update { last_error: Some(error.to_string()), last_action: Some("Раскладка не переключена".to_owned()), ..Update::default() });
            return Ok(());
        }
        self.note_engine_switch(target);
        self.book_layout_tap(tap, event, held, target, "keyswitch");
        Ok(())
    }

    /// What `observe_group` books for a layout the person picked by hand.
    fn book_layout_tap(&mut self, tap: LayoutTap, event: &KeyEvent, held: f64, group: i64, switched_by: &str) {
        let respect = self.flag("detection.respect_manual_layout", true);
        technical_event!(
            self.shared, "layout_key_tapped",
            "key": event.key_name,
            "previous_group": tap.group,
            "selected_group": group,
            "switched_by": switched_by,
            "held_ms": ms(held),
            "protects_next_word": respect,
            "current_word_length": self.strokes.len(),
        );
        // A letter in the layout an early switch came from is the person's now, not a key that reached the window before
        // that switch (late_stroke_after_early_switch); the switch itself is still recorded when its word ends.
        self.early_switch_at = None;
        // Two presses of the conversion hotkey with a switch between them are no double press.
        self.convert_press_at = 0.0;
        if respect {
            self.manual_layout_group = Some(group);
            self.manual_layout_observed_at = Some(self.monotonic());
            LAYOUT_TAP_SOURCE.clone_into(&mut self.manual_layout_source);
            self.manual_layout_word = false;
            self.shared.update(Update {
                current_group: Some(group),
                last_action: Some("Ручная смена раскладки · следующее слово без автокоррекции".to_owned()),
                ..Update::default()
            });
            return;
        }
        self.shared.update(Update { current_group: Some(group), ..Update::default() });
    }
}
