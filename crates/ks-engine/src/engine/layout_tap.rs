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
//! names one key in both settings leaves it to the layout switch key (`configured_convert_last_key`). The layout switch key
//! may be either Shift (`EITHER_SHIFT_KEY`): then both Shifts are its keys.
//!
//! Two taps of the layout switch key in quick succession (`detection.double_tap_converts`) are a press of the conversion
//! hotkey: the first tap has switched the layout already, so the second takes that switch back and converts, and the
//! layout ends as the conversion leaves it (`convert_by_double_tap`). With no word to convert it converts the selection, as
//! the selection hotkey of the layout does.
//!
//! Both Shifts pressed together and released with nothing else (`detection.both_shifts_toggle`) turn the automatic
//! switching off or on, as the toggle hotkey does (`toggle_by_both_shifts`), unless the system does something with that
//! chord itself: an option of the system that the backend reports, or a layout the system switched meanwhile.

use super::logging::technical_event;
use super::*;
use crate::constants::{
    BOTH_SHIFTS_MAX_SECONDS, CONVERT_LAST_KEYS, EITHER_SHIFT_KEY, HOTKEY_MESSAGE_KEY_PLACE, LAYOUT_DOUBLE_TAP_SECONDS, LAYOUT_SWITCH_KEYS,
    LAYOUT_TAP_MAX_SECONDS, SHIFT_KEYS, SINGLE_KEY_MESSAGE_SETTING_PLACE, SINGLE_KEY_TAKEN_MESSAGE,
};
use ks_config::config::Key;
use ks_config::constants::{
    DEFAULT_BOTH_SHIFTS_TOGGLE, DEFAULT_DOUBLE_TAP_CONVERTS, DETECTION_BOTH_SHIFTS_TOGGLE, DETECTION_CONVERT_LAST_KEY, DETECTION_DOUBLE_TAP_CONVERTS,
    DETECTION_LAYOUT_SWITCH_KEY,
};
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
pub const LAYOUT_SWITCH_KEY_LABELS: [(&str, &str); 6] = [
    ("", "Выключено"),
    (EITHER_SHIFT_KEY, "Любой Shift"),
    ("Shift_L", "Левый Shift"),
    ("Shift_R", "Правый Shift"),
    ("Control_L", "Левый Ctrl"),
    ("Control_R", "Правый Ctrl"),
];

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
/// The keys the conversion key offers: those of the layout switch key but either Shift.
pub const CONVERT_LAST_KEY_LABELS: [(&str, &str); 5] =
    [LAYOUT_SWITCH_KEY_LABELS[0], LAYOUT_SWITCH_KEY_LABELS[2], LAYOUT_SWITCH_KEY_LABELS[3], LAYOUT_SWITCH_KEY_LABELS[4], LAYOUT_SWITCH_KEY_LABELS[5]];

/// The settings of a lone tap, with the titles the windows give them: a key is chosen in one of them at most.
const SINGLE_KEY_SETTINGS: [(Key<String>, &str); 2] =
    [(DETECTION_LAYOUT_SWITCH_KEY, LAYOUT_SWITCH_KEY_TITLE), (DETECTION_CONVERT_LAST_KEY, CONVERT_LAST_KEY_TITLE)];

/// The layout switch key of `settings`, as every backend names it (`Shift_L`, `Control_R`) or `EITHER_SHIFT_KEY`; None for
/// none or for a value not offered.
pub(crate) fn configured_layout_switch_key(settings: &SettingsStore) -> Option<&'static str> {
    settings.with_value("detection.layout_switch_key", |value| {
        let name = value.and_then(Value::as_str)?;
        LAYOUT_SWITCH_KEYS.iter().copied().find(|key| *key == name)
    })
}

/// Whether the key `name`, as a backend names it, is one of the keys the value `chosen` of a setting names: the key itself,
/// or either Shift for `EITHER_SHIFT_KEY`.
fn names_key(chosen: &str, name: &str) -> bool {
    chosen == name || (chosen == EITHER_SHIFT_KEY && SHIFT_KEYS.contains(&name))
}

/// The conversion key of `settings`, named as the layout switch key is; None for none, for a value not offered, and for a
/// key of the layout switch key. The settings windows never store one key in both settings; a file edited by hand that
/// does keeps the key's older meaning, the layout switch, which leaves the text as it is.
pub(crate) fn configured_convert_last_key(settings: &SettingsStore) -> Option<&'static str> {
    let name = settings.read(DETECTION_CONVERT_LAST_KEY)?;
    let switch = configured_layout_switch_key(settings);
    CONVERT_LAST_KEYS.iter().copied().find(|key| *key == name).filter(|key| !switch.is_some_and(|switch| names_key(switch, key)))
}

/// Whether both Shifts together pause the automatic switching, as the settings say.
pub(crate) fn both_shifts_toggle(settings: &SettingsStore) -> bool {
    settings.read(DETECTION_BOTH_SHIFTS_TOGGLE).unwrap_or(DEFAULT_BOTH_SHIFTS_TOGGLE)
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
    let taken = settings.read(*other)?;
    // Either Shift shares its keys with the left and the right one: the window names the key the other setting holds.
    if !(names_key(key, &taken) || names_key(&taken, key)) {
        return None;
    }
    let label = LAYOUT_SWITCH_KEY_LABELS.iter().find(|(name, _)| *name == taken).map_or(taken.as_str(), |(_, label)| *label);
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
        if self.layout_switch_key().is_some_and(|switch| names_key(switch, name)) {
            Some(TapAction::SwitchLayout)
        } else if self.convert_last_key() == Some(name) {
            Some(TapAction::ConvertLast)
        } else {
            None
        }
    }

    /// Anything but the held key itself ends a lone press: another key going down or up, a click, a turned wheel, a key
    /// another program injected that the settings do not count as typing, the rule window taking the keyboard, the key
    /// chosen for something else meanwhile. Anything but a key of the layout switch key ends a double tap begun, and
    /// anything but a Shift ends the chord of both. Called for every key before anything else looks at it.
    pub fn follow_layout_tap(&mut self, event: &KeyEvent) {
        if self.layout_tap.is_none() && self.switch_tapped.is_none() && self.shift_chord.is_none() {
            return;
        }
        if self.shared.input_suspended() || (event.foreign && !self.shared.accepts_injected(event)) {
            self.forget_taps();
            return;
        }
        let action = self.tap_action(&event.key_name);
        if self.layout_tap.is_some_and(|tap| event.keycode != tap.keycode || action != Some(tap.action)) {
            self.layout_tap = None;
        }
        if action != Some(TapAction::SwitchLayout) {
            self.switch_tapped = None;
        }
        if !SHIFT_KEYS.contains(&event.key_name.as_str()) {
            self.shift_chord = None;
        }
    }

    /// A tap, a double tap or a chord on its way is over: the window changed, keys went past the engine, a wheel turned.
    pub fn forget_taps(&mut self) {
        self.layout_tap = None;
        self.switch_tapped = None;
        self.shift_chord = None;
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
            if others_held {
                // The other Shift of a chord, or a Shift held with another key: no second tap.
                self.switch_tapped = None;
            }
            return Ok(());
        }
        // Whatever came between the press and this release, the key chosen for another action included, ended the tap
        // (`follow_layout_tap`): a tap left here is one of `action`.
        let Some(tap) = self.layout_tap.take() else {
            return Ok(());
        };
        let held = self.monotonic() - tap.pressed_at;
        // A first tap waits for its second only until this release: each tap of a double tap is quick.
        let first = self.switch_tapped.take();
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
            TapAction::SwitchLayout => match first.filter(|first| self.double_tap_converts() && tap.pressed_at - first.pressed_at <= LAYOUT_DOUBLE_TAP_SECONDS) {
                Some(first) => self.convert_by_double_tap(first, event, held),
                None => self.switch_layout_by_tap(tap, event, held),
            },
            TapAction::ConvertLast => self.convert_by_tap(event, held),
        }
    }

    fn double_tap_converts(&self) -> bool {
        self.settings.read(DETECTION_DOUBLE_TAP_CONVERTS).unwrap_or(DEFAULT_DOUBLE_TAP_CONVERTS)
    }

    /// The choice of layout the engine has booked, as `book_layout_tap` changes it.
    fn manual_layout(&self) -> ManualLayout {
        ManualLayout {
            group: self.manual_layout_group,
            observed_at: self.manual_layout_observed_at,
            source: self.manual_layout_source.clone(),
            word: self.manual_layout_word,
        }
    }

    fn switch_layout_by_tap(&mut self, tap: LayoutTap, event: &KeyEvent, held: f64) -> EngineResult<()> {
        let current = i64::from(self.backend.current_group()?);
        let before = self.manual_layout();
        if self.in_models(tap.group) && current != tap.group {
            // The system switched the layout on this key itself (an XKB option such as grp:lshift_toggle): the switch is
            // booked as the person's and not made a second time, which would take the layout back.
            self.book_layout_tap(tap, event, held, current, "system");
            self.switch_tapped = Some(SwitchTapped { pressed_at: tap.pressed_at, group: tap.group, manual: before });
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
        self.switch_tapped = Some(SwitchTapped { pressed_at: tap.pressed_at, group: current, manual: before });
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

    /// The second tap of a double tap of the layout switch key, as it comes up: the press of the conversion hotkey, made
    /// where the layout was before the first tap. That tap switched the layout and booked it as the person's choice; both
    /// are taken back, so that the conversion finds the word, the layout and the choice as they were, and leaves the layout
    /// where a conversion leaves it. A double tap is one press of the hotkey: it never offers a rule. With no word to
    /// convert it converts the selection (`schedule_conversion`).
    fn convert_by_double_tap(&mut self, first: SwitchTapped, event: &KeyEvent, held: f64) -> EngineResult<()> {
        technical_event!(
            self.shared, "layout_key_double_tapped", "key": event.key_name, "held_ms": ms(held), "previous_group": first.group,
            "current_word_length": self.strokes.len(),
        );
        let current = i64::from(self.backend.current_group()?);
        // A system that switches on this key itself has switched back with the second tap.
        if current != first.group {
            if let Err(error) = self.backend.switch_group(first.group.to::<i32>()) {
                technical_event!(self.shared, "layout_switch_failed", "source": LAYOUT_TAP_SOURCE, "requested_group": first.group, "error": error.to_string());
                self.note_unconfirmed_switch(&error);
                self.shared.update(Update { last_error: Some(error.to_string()), last_action: Some("Раскладка не переключена".to_owned()), ..Update::default() });
                return Ok(());
            }
            self.note_engine_switch(first.group);
        }
        self.manual_layout_group = first.manual.group;
        self.manual_layout_observed_at = first.manual.observed_at;
        self.manual_layout_source = first.manual.source;
        self.manual_layout_word = first.manual.word;
        self.shared.update(Update { current_group: Some(first.group), ..Update::default() });
        // What the press of the conversion key does first (`convert_by_tap`).
        self.foreign_input_active = false;
        self.character_before_key = std::mem::take(&mut self.last_key_character);
        if let Some(prompt) = self.shared.learning_prompt() {
            self.shared.dismiss_learning_prompt(Some(&prompt), "other_key");
        }
        let keycode = i64::from(event.keycode);
        self.convert_press_at = self.monotonic();
        self.convert_keycode = keycode;
        self.convert_outcome = None;
        self.schedule_conversion(keycode, true);
        self.convert_released = true;
        self.maybe_execute_pending(event)
    }

    /// A Shift going down or coming up, for the chord of both (`detection.both_shifts_toggle`). The first goes down with
    /// nothing else held, the other while it is held; once both are up again the chord is made, if it was quick. Anything
    /// else in between has ended it (`follow_layout_tap`), and so does a third press of a Shift.
    pub fn track_shift_chord(&mut self, event: &KeyEvent, repeat: bool) -> EngineResult<()> {
        if !SHIFT_KEYS.contains(&event.key_name.as_str()) || repeat {
            return Ok(());
        }
        if event.pressed {
            self.shift_chord = match self.shift_chord {
                None => {
                    let others_held = self.pressed.iter().chain(&self.modifier_keycodes).any(|keycode| *keycode != event.keycode);
                    (!others_held && both_shifts_toggle(&self.settings))
                        .then(|| ShiftChord { first: event.keycode, second: None, pressed_at: self.monotonic(), group: i64::from(event.group) })
                }
                Some(chord) if chord.second.is_none() && chord.first != event.keycode => Some(ShiftChord { second: Some(event.keycode), ..chord }),
                Some(_) => None,
            };
            return Ok(());
        }
        let Some(chord) = self.shift_chord else { return Ok(()) };
        let Some(second) = chord.second else {
            // The first Shift came up alone: a tap, or a Shift held for a capital.
            self.shift_chord = None;
            return Ok(());
        };
        if self.modifier_keycodes.contains(&chord.first) || self.modifier_keycodes.contains(&second) {
            return Ok(());
        }
        self.shift_chord = None;
        self.toggle_by_both_shifts(chord, event)
    }

    /// Both Shifts came up: the automatic switching is turned off or on, as by the toggle hotkey, and the platforms show it
    /// at the caret (`Shared::subscribe_auto_switch`). A chord held too long was meant for something else, and one the
    /// system answers itself is left to it: an option of the system that the backend knows of
    /// (`InputBackend::system_takes_both_shifts`), or a layout that changed while the keys were down.
    fn toggle_by_both_shifts(&mut self, chord: ShiftChord, event: &KeyEvent) -> EngineResult<()> {
        let held = self.monotonic() - chord.pressed_at;
        let reason = if held > BOTH_SHIFTS_MAX_SECONDS {
            Some("held_too_long")
        } else if self.backend.system_takes_both_shifts() {
            Some("system_option")
        } else if i64::from(self.backend.current_group()?) != chord.group {
            Some("system_switched")
        } else {
            None
        };
        if let Some(reason) = reason {
            technical_event!(self.shared, "both_shifts_ignored", "reason": reason, "key": event.key_name, "held_ms": ms(held));
            return Ok(());
        }
        let enabled = self.toggle_enabled()?;
        technical_event!(self.shared, "both_shifts_toggled", "enabled": enabled, "held_ms": ms(held));
        for callback in self.shared.auto_switch_callbacks() {
            callback(enabled);
        }
        Ok(())
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
