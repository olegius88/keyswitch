//! A modifier pressed and released alone switches the layout (`detection.layout_switch_key`), as the switchers people come
//! from do it with a lone Shift or Ctrl, or does what the conversion hotkey does (`detection.convert_last_key`).
//!
//! The layout tap is a keyboard switch, like the system's own Alt+Shift, not the menu's: the engine books it as the person's
//! choice (`observe_group`), so the next word stays as typed, a word being typed is let go by its next letter in the other
//! layout (`layout_changed_mid_word`), and Pause, undo, a waiting word and the learning prompt keep what they had. The key
//! itself goes on to the window unchanged: modifiers always pass (`Shared::consumes_key`).
//!
//! The conversion tap is a press of the conversion hotkey made when the key comes up (`convert_by_tap`), its release
//! with it: the word is converted, with nothing to convert the layout switches, and a second tap right after offers a
//! rule. What ends a lone press of the one key ends a lone press of the other, and one key is never both: a file that
//! names one key in both settings leaves it to the layout switch key (`configured_convert_last_key`).

use super::logging::technical_event;
use super::*;
use crate::constants::{
    CONVERT_LAST_KEYS, HOTKEY_MESSAGE_KEY_PLACE, LAYOUT_SWITCH_KEYS, LAYOUT_TAP_MAX_SECONDS, SINGLE_KEY_MESSAGE_SETTING_PLACE,
    SINGLE_KEY_TAKEN_MESSAGE,
};
use ks_config::config::Key;
use ks_config::constants::{DETECTION_CONVERT_LAST_KEY, DETECTION_LAYOUT_SWITCH_KEY};
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

/// What every settings window says of `detection.convert_last_key`; it offers the keys the layout switch key does.
pub const CONVERT_LAST_KEY_TITLE: &str = "Преобразовать последнее слово одной клавишей";
pub const CONVERT_LAST_KEY_DESCRIPTION: &str = "Нажатие и отпускание выбранной клавиши без других клавиш делает то же, \
                                                что «Преобразовать последнее слово»: меняет раскладку последнего слова, \
                                                а если преобразовывать нечего, переключает раскладку; двойное нажатие \
                                                предлагает правило переключения для этого слова. Shift с буквой, щелчок \
                                                или прокрутка с зажатой клавишей, Ctrl+C и долгое удержание ничего не \
                                                меняют. Клавишу, которая уже переключает раскладку одной клавишей, \
                                                выбрать нельзя. Не включайте, если система уже переключает раскладку \
                                                этой клавишей. С залипанием клавиш не работает. Если включён ввод от \
                                                других программ, слово преобразует и нажатие с удалённого компьютера";
pub const CONVERT_LAST_KEY_LABELS: [(&str, &str); 5] = LAYOUT_SWITCH_KEY_LABELS;

/// The settings of a lone tap, with the titles the windows give them: a key is chosen in one of them at most.
const SINGLE_KEY_SETTINGS: [(Key<String>, &str); 2] =
    [(DETECTION_LAYOUT_SWITCH_KEY, LAYOUT_SWITCH_KEY_TITLE), (DETECTION_CONVERT_LAST_KEY, CONVERT_LAST_KEY_TITLE)];

/// The layout switch key of `settings`, as every backend names it (`Shift_L`, `Control_R`); None for none or for a value
/// not offered.
pub(crate) fn configured_layout_switch_key(settings: &SettingsStore) -> Option<&'static str> {
    settings.with_value("detection.layout_switch_key", |value| {
        let name = value.and_then(Value::as_str)?;
        LAYOUT_SWITCH_KEYS.iter().copied().find(|key| *key == name)
    })
}

/// The conversion key of `settings`, named as the layout switch key is; None for none, for a value not offered, and for the
/// layout switch key itself. The settings windows never store one key in both settings; a file edited by hand that does
/// keeps the key's older meaning, the layout switch, which leaves the text as it is.
pub(crate) fn configured_convert_last_key(settings: &SettingsStore) -> Option<&'static str> {
    let name = settings.read(DETECTION_CONVERT_LAST_KEY)?;
    CONVERT_LAST_KEYS.iter().copied().find(|key| *key == name).filter(|key| configured_layout_switch_key(settings) != Some(*key))
}

/// Why `key` cannot be stored in the setting of a lone tap at `path`: the other such setting has it already. The answer is
/// what the settings window says, naming the key as its list does and that setting by its title; None for a setting of
/// another kind, for no key and for a key the other setting does not have.
#[must_use]
pub fn single_key_conflict(settings: &SettingsStore, path: &str, key: &str) -> Option<String> {
    if key.is_empty() || !SINGLE_KEY_SETTINGS.iter().any(|(setting, _)| setting.path() == path) {
        return None;
    }
    let (other, title) = SINGLE_KEY_SETTINGS.iter().find(|(setting, _)| setting.path() != path).expect("two settings, one of them at `path`");
    if settings.read(*other).as_deref() != Some(key) {
        return None;
    }
    let label = LAYOUT_SWITCH_KEY_LABELS.iter().find(|(name, _)| *name == key).map_or(key, |(_, label)| *label);
    Some(SINGLE_KEY_TAKEN_MESSAGE.replace(HOTKEY_MESSAGE_KEY_PLACE, label).replace(SINGLE_KEY_MESSAGE_SETTING_PLACE, title))
}

impl EngineCore {
    /// The layout switch key of the settings; None when there is none.
    #[must_use]
    pub fn layout_switch_key(&self) -> Option<&'static str> {
        configured_layout_switch_key(&self.settings)
    }

    /// The conversion key of the settings; None when there is none.
    #[must_use]
    pub fn convert_last_key(&self) -> Option<&'static str> {
        configured_convert_last_key(&self.settings)
    }

    /// What a lone tap of the key of this name does; None for a key chosen in neither setting.
    #[must_use]
    pub fn tap_action(&self, name: &str) -> Option<TapAction> {
        if self.layout_switch_key() == Some(name) {
            Some(TapAction::SwitchLayout)
        } else if self.convert_last_key() == Some(name) {
            Some(TapAction::ConvertLast)
        } else {
            None
        }
    }

    /// Anything but the held key itself ends a lone press: another key going down or up, a click, a turned wheel, a key
    /// another program injected that the settings do not count as typing, the rule window taking the keyboard, the key
    /// chosen for something else meanwhile. Called for every key before anything else looks at it.
    pub fn follow_layout_tap(&mut self, event: &KeyEvent) {
        let Some(tap) = self.layout_tap else { return };
        let ended = event.keycode != tap.keycode
            || self.tap_action(&event.key_name) != Some(tap.action)
            || self.shared.input_suspended()
            || (event.foreign && !self.shared.accepts_injected(event));
        if ended {
            self.layout_tap = None;
        }
    }

    /// The layout switch key or the conversion key as the books of held keys just took it (the modifier branch of
    /// `handle_inner`). A press with no other key held starts a tap: Alt+Shift, Ctrl+Shift and both Shifts (the system's
    /// own switches) have the other key down first, and a key the engine still holds as down vetoes however long ago it
    /// went down, until the engine forgets it as it forgets any press whose release was lost (`prune_stale_presses`, a
    /// change of window). A repeat of the held key (`repeat`: Windows repeats a held Shift) leaves the tap as it was, so its
    /// time counts from the first press. The release within `LAYOUT_TAP_MAX_SECONDS` switches the layout or converts.
    pub fn track_layout_tap(&mut self, event: &KeyEvent, repeat: bool) -> EngineResult<()> {
        let Some(action) = self.tap_action(&event.key_name) else {
            return Ok(());
        };
        if event.pressed {
            let others_held = self.pressed.iter().chain(&self.modifier_keycodes).any(|keycode| *keycode != event.keycode);
            if !repeat && !others_held {
                self.layout_tap = Some(LayoutTap { keycode: event.keycode, pressed_at: self.monotonic(), group: i64::from(event.group), action });
            }
            return Ok(());
        }
        // Whatever came between the press and this release, the key chosen for another action included, ended the tap
        // (`follow_layout_tap`): a tap left here is one of `action`.
        let Some(tap) = self.layout_tap.take() else {
            return Ok(());
        };
        let held = self.monotonic() - tap.pressed_at;
        if held > LAYOUT_TAP_MAX_SECONDS {
            let ignored = match action {
                TapAction::SwitchLayout => "layout_tap_ignored",
                TapAction::ConvertLast => "convert_tap_ignored",
            };
            technical_event!(
                self.shared, ignored, "reason": "held_too_long", "key": event.key_name, "held_ms": ms(held),
                "limit_ms": ms(LAYOUT_TAP_MAX_SECONDS),
            );
            return Ok(());
        }
        // A window that came to the front while the key was down has its own layout (Windows keeps one per window), and
        // the word typed in the window before is gone with it.
        if self.track_focus().changed {
            return Ok(());
        }
        match action {
            TapAction::SwitchLayout => self.switch_layout_by_tap(tap, event, held),
            TapAction::ConvertLast => self.convert_by_tap(event, held),
        }
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

    /// A tap of the conversion key, as it comes up: the press of the conversion hotkey (`handle_inner`) and its release.
    /// Paused, in an excluded program and with nothing to convert, it does what the hotkey does there; two taps within
    /// `DOUBLE_CONVERT_PRESS_WINDOW_SECONDS` of each other, nothing typed between them, are its double press. The key is up
    /// already, so the conversion runs at once (`maybe_execute_pending`) and the next tap is a press of its own.
    fn convert_by_tap(&mut self, event: &KeyEvent, held: f64) -> EngineResult<()> {
        technical_event!(self.shared, "convert_key_tapped", "key": event.key_name, "held_ms": ms(held), "current_word_length": self.strokes.len());
        // What the press of a key other than a modifier does before the hotkey is answered: the input of other programs
        // has ended, the press stands in the text as no character, and a rule offered goes.
        self.foreign_input_active = false;
        self.character_before_key = std::mem::take(&mut self.last_key_character);
        if let Some(prompt) = self.shared.learning_prompt() {
            self.shared.dismiss_learning_prompt(Some(&prompt), "other_key");
        }
        let keycode = i64::from(event.keycode);
        if self.double_convert_press(event) {
            self.request_rule_after_double_press(keycode);
        } else {
            self.schedule_manual_conversion(keycode);
        }
        self.convert_released = true;
        self.maybe_execute_pending(event)
    }

    /// Whether `key`, a modifier queued behind a correction about to be typed (`collect_late_input`), is the conversion key
    /// and has come up again by the end of the queue: a tap of it made while the correction waited, answered once the
    /// correction lands, as a second press of the conversion hotkey is. A modifier still down would change the keys the
    /// correction types.
    pub(crate) fn queued_conversion_tap(&self, queued: &[QueueItem], key: &KeyEvent) -> bool {
        let comes_up = queued.iter().rev().find_map(|item| match item {
            QueueItem::Key(other) if other.keycode == key.keycode => Some(!other.pressed),
            QueueItem::Key(_) | QueueItem::LayoutSelection(_) | QueueItem::Stop => None,
        });
        self.convert_last_key() == Some(key.key_name.as_str()) && comes_up == Some(true)
    }
}
