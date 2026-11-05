//! One key at a time: `handle_inner` and what it asks about a key.

use super::logging::technical_event;
use super::shared::SETTING_FROM_CORE;
use super::*;
use crate::unicode_category::category;
use serde_json::json;

impl EngineCore {
    // Let go of the text when keys may have passed the Windows hook; say whether they may have.
    //
    // Under CPU load the hook's thread waited 1.5 to 3.8 s for the processor (VM, 28.10.2026), and Windows
    // meanwhile gave every key that waited about 300 ms behind it to the window without the hook: the engine saw
    // `ghbd` of `ghbdtn`, and a correction made of that deletes the wrong letters. Each key that passed the hook
    // in those runs lay within the wait of a key the hook reached HOOK_GAP_MS late (take_hook_gap). What follows
    // is left alone as after an overflowing queue.
    pub(crate) fn take_hook_gap(&mut self) -> bool {
        let Some(gap) = self.backend.take_hook_gap() else {
            return false;
        };
        technical_event!(self.shared, "hook_gap", "late_ms": gap.late_ms, "callback_ms": gap.callback_ms, "threshold_ms": HOOK_GAP_MS);
        self.clear_word(Some("Клавиши прошли мимо KeySwitch"), "hook_gap");
        self.untracked_token = true;
        self.context_policy.stream.clear();
        true
    }

    /// `handle_inner`: every key the worker takes, in the order the hook saw them.
    pub fn handle_inner(&mut self, event: Stroke) -> EngineResult<()> {
        let mut event = event;
        self.expire_manual_correction();
        if self.shared.input_overflow.swap(false, std::sync::atomic::Ordering::Relaxed) {
            self.complete_deferred_action(false, "input_overflow");
            self.clear_word(Some("Очередь ввода переполнена"), "input_overflow");
            self.untracked_token = true;
            self.context_policy.stream.clear();
        }
        self.take_hook_gap();
        if event.pressed {
            self.track_focus();
        }
        if event.key_name == "Pointer" {
            if event.pressed {
                self.log_input_edit(&event, &self.backend.active_application());
            }
            self.shared.set_sensitive_context_window(None);
            self.convert_press_at = 0.0;
            self.clear_word(None, "pointer_activity");
            self.untracked_token = false;
            // A click often lands in an empty field, which is exactly where the first
            // word must still be corrected, so it is not treated as a caret move.
            self.caret_moved = false;
            self.contexts.clear();
            self.context_policy.stream.clear();
            return Ok(());
        }
        if self.shared.input_suspended() {
            // The rule window is open: what is typed there is a rule, not text.
            if event.pressed && (!self.strokes.is_empty() || !self.symbol_strokes.is_empty()) {
                self.clear_word(None, "rule_editor_open");
            }
            self.convert_press_at = 0.0;
            return Ok(());
        }
        if event.foreign && !self.shared.accepts_injected(&event) {
            self.observe_foreign_input(&event);
            return Ok(());
        }
        if event.pressed && !is_modifier_key(&event.key_name) {
            self.foreign_input_active = false;
        }
        self.shared.expire_learning_prompt(None);
        let prompt = self.shared.learning_prompt();
        if let Some(prompt) = prompt
            && event.pressed
            && !is_modifier_key(&event.key_name)
        {
            let unmodified = !(event.control() || event.alt() || event.super_key() || event.shift());
            if unmodified && (event.key_name == "Return" || event.key_name == "KP_Enter") {
                self.shared.confirm_learning_prompt(Some(&prompt));
                if event.deferred {
                    // The prompt may have appeared after the hook deferred
                    // Enter but before the worker reached it. It still owns
                    // this key; release the barrier without submitting.
                    self.backend.complete_action(false)?;
                }
                return Ok(());
            }
            if unmodified && event.key_name == "Escape" {
                self.shared.dismiss_learning_prompt(Some(&prompt), "escape");
                return Ok(());
            }
            self.shared.dismiss_learning_prompt(Some(&prompt), "other_key");
        }
        if event.pressed {
            let application = self.backend.active_application();
            if !event.character.is_empty()
                && self.strokes.is_empty()
                && self.shared.sensitive_context_window().is_none()
                && !self.application_excluded(&application)
                && self.flag("enabled", true)
                && self.flag("detection.context_read_field", false)
                && self.context_policy.reader.is_some()
            {
                let window = self.focus_or_zero();
                let field = self.context_policy.reader.as_ref().and_then(|reader| reader.read(&application, window));
                if field.is_some_and(|field| field.sensitive) {
                    self.shared.set_sensitive_context_window(self.shared.focus_window());
                    self.cancel_context_wait("sensitive_field");
                    self.context_policy.stream.clear();
                }
            }
            let context_enabled = self.flag("enabled", true)
                && self.flag("detection.context_aware", true)
                && self.text_setting("detection.context_policy", "assist") != "off"
                && !self.application_excluded(&application);
            if self.shared.sensitive_context_window().is_none() && !self.application_excluded(&application) {
                // Each press shows the key of its letters: a typo correction types a letter not in the word with it.
                self.typo_keys.observe(&event);
            }
            let window = self.focus_or_zero();
            self.context_policy.stream.focus(&application, window);
            self.log_input_edit(&event, &application);
            if context_enabled {
                let hotkey = ["toggle", "convert_last", "undo"].iter().chain(SELECTION_HOTKEYS.iter()).any(|name| self.matches_hotkey(name, &event));
                if !hotkey {
                    self.context_policy.stream.observe(&event);
                }
            } else {
                self.context_policy.stream.clear();
            }
            if event.key_name == "BackSpace" || event.control() || event.alt() || event.super_key() {
                self.cancel_context_wait(if event.key_name == "BackSpace" { "backspace" } else { "modifier_shortcut" });
            }
        }
        // Only presses carry a meaningful group: a release reports whatever
        // layout was active when the finger came up, which is stale right
        // after the engine switched the layout itself.
        if event.pressed {
            // A key pressed before an early switch landed still reports the
            // old layout; that is a race, not the user switching back.
            if !self.late_stroke_after_early_switch(&event) {
                self.observe_group(i64::from(event.group), "keystroke");
            }
            self.pressed.insert(event.keycode);
            let now = self.monotonic();
            match self.pressed_since.iter_mut().find(|entry| entry.0 == event.keycode) {
                Some(entry) => entry.1 = now,
                None => self.pressed_since.push((event.keycode, now)),
            }
        } else {
            self.pressed.remove(&event.keycode);
            self.pressed_since.retain(|entry| entry.0 != event.keycode);
        }
        if is_modifier_key(&event.key_name) {
            if event.pressed {
                self.modifier_keycodes.insert(event.keycode);
            } else {
                self.modifier_keycodes.remove(&event.keycode);
            }
            self.maybe_execute_pending(&event)?;
            return Ok(());
        }
        if event.pressed && !event.synthetic {
            let typed = if event.key_name == "BackSpace" || event.key_name == "Delete" { "" } else { event.character.as_str() };
            self.character_before_key = std::mem::take(&mut self.last_key_character);
            self.last_key_character = if pystr::isprintable(typed) || pystr::isspace(typed) { typed.to_owned() } else { String::new() };
        }
        if !event.pressed {
            if i64::from(event.keycode) == self.convert_keycode {
                self.convert_released = true;
            }
            self.maybe_execute_pending(&event)?;
            return Ok(());
        }
        let is_convert = self.matches_hotkey("convert_last", &event);
        if !is_convert {
            // Only two presses of the hotkey with nothing in between are a double press.
            self.convert_press_at = 0.0;
        }
        if self.matches_hotkey("toggle", &event) {
            let enabled = !self.flag("enabled", true);
            SETTING_FROM_CORE.set(true);
            let saved = self.settings.set("enabled", json!(enabled), true);
            SETTING_FROM_CORE.set(false);
            if saved.is_ok() {
                self.settings_changed("enabled", &json!(enabled));
            }
            saved.map_err(|error| EngineError(error.to_string()))?;
            self.clear_word(Some(if enabled { "Автокоррекция включена" } else { "Автокоррекция на паузе" }), "engine_toggled");
            return Ok(());
        }
        if is_convert {
            if self.double_convert_press(&event) {
                self.request_rule_after_double_press(i64::from(event.keycode));
            } else {
                self.schedule_manual_conversion(i64::from(event.keycode));
            }
            return Ok(());
        }
        if self.matches_hotkey("undo", &event) {
            self.schedule_undo_hotkey(i64::from(event.keycode));
            return Ok(());
        }
        if let Some(selection_hotkey) = self.selection_hotkey(&event) {
            self.schedule_selection_command(selection_hotkey);
            return Ok(());
        }
        if event.control() || event.alt() || event.super_key() {
            self.clear_word(None, "modifier_shortcut");
            return Ok(());
        }
        if event.key_name == "BackSpace" {
            if self.strokes.is_empty() {
                self.personal_word_settled(false);
            }
            let reopenable = self.reopenable_committed_word();
            self.last_committed_stale = true;
            self.log_pending_dropped("backspace");
            self.pending = None;
            self.learning_prompt_after = None;
            // Erasing is not appending. A caret moved into finished text makes the next
            // word untouchable because nobody knows what stands in front of it; a user
            // who then rubs that text out has answered the question - nothing does.
            self.caret_moved = false;
            if !self.strokes.is_empty() {
                self.strokes.pop();
                if self.strokes.is_empty() {
                    self.source_group = -1;
                    self.reset_pause_correction();
                    self.early_switch_origin = None;
                    self.early_switch_at = None;
                } else {
                    // Erasing is editing, not typing: the letters left are a word the
                    // user is still changing, and a pause over them is a pause to think,
                    // not a boundary. Re-arming the pause timer here corrected the
                    // half-erased word a pause delay after the Backspace, as if the
                    // erased letter had never gone (seen 02.10.2026). The next
                    // letter arms the timer again, as it does for a reopened word.
                    self.reset_pause_correction();
                }
                self.shared.update(Update { current_word: Some(text_for_group(&self.strokes, self.source_group)), ..Update::default() });
            } else if !self.symbol_strokes.is_empty() {
                self.symbol_strokes.pop();
                if self.symbol_strokes.is_empty() {
                    // The "@" shown for the quote is what this key erased.
                    self.mention_shown = None;
                }
            } else if let Some(reopenable) = reopenable {
                // This Backspace took away the boundary that ended the last
                // word, so the caret stands right after that word again and
                // letters typed now continue it: "создаш" + "ь" is judged as
                // "создашь", not as a lone "ь" that no dictionary knows.
                // The word was already judged at that boundary, so reopening
                // alone does not arm the pause timer: a user who stops to
                // think after the Backspace must not get a second, unasked
                // judgement of the same letters. The next letter arms it.
                self.strokes.clone_from(&reopenable.strokes);
                self.source_group = reopenable.source_group;
                technical_event!(
                    self.shared, "committed_word_reopened",
                    "characters": self.strokes.len(),
                    "source_group": self.source_group,
                    "boundary": reopenable.boundary.as_ref().map_or("", |boundary| boundary.key_name.as_str()),
                );
                self.shared.update(Update { current_word: Some(text_for_group(&self.strokes, self.source_group)), ..Update::default() });
            } else {
                // This Backspace erased text no word of the engine holds: letters typed now may
                // continue a word in front of the caret, as after a click. `первую игру` cut back
                // to `перв` and finished as `первый` had its `ый` judged a word of its own, and
                // `sq` replaced it (0.44.0 log, 08.10.2026); the next word reads the field as it
                // begins (read_insertion).
                self.position_unknown = true;
            }
            if let Some(shown) = self.mention_shown.clone()
                && !self.strokes.is_empty()
            {
                // The pending write-back went with the erased letter; the
                // letters left still stand after the "@".
                self.hide_mention_head(&shown, None, i64::from(event.keycode));
            }
            return Ok(());
        }
        if is_action_boundary_key(&event.key_name) {
            if event.deferred {
                self.deferred_action = Some(event.clone());
                self.action_deadline = self.monotonic() + ACTION_TIMEOUT_SECONDS;
                technical_event!(self.shared, "action_deferred", "key_name": event.key_name);
                if !self.strokes.is_empty() && !self.untracked_token {
                    self.commit_word(&event)?;
                } else {
                    self.settle_context_wait_at_action(&event)?;
                }
                return Ok(());
            }
            // These events have already reached the application. Enter may
            // submit a chat and Tab may focus another field; neither can be
            // undone with one BackSpace and safely replayed.
            self.clear_word(None, "action_boundary_already_delivered");
            self.untracked_token = false;
            self.caret_moved = false;
            self.contexts.clear();
            self.context_policy.stream.clear();
            self.shared.set_sensitive_context_window(None);
            return Ok(());
        }
        if self.shared.sensitive_context_window().is_some() {
            return Ok(());
        }
        if self.untracked_token {
            if Self::_is_boundary(&event) {
                self.untracked_token = false;
            }
            return Ok(());
        }
        if !(event.character.is_empty() || Self::_safe_text_stroke(&event) || pystr::len(&event.character) == 1 && pystr::isspace(&event.character))
        {
            technical_event!(
                self.shared, "input_not_representable", "key_name": event.key_name,
                "group": event.group,
                "character_lengths": event.characters.iter().map(|char| pystr::len(char)).collect::<Vec<_>>(),
                "character_categories": event.characters.iter().map(|char| char.chars().next().map_or("empty", category)).collect::<Vec<_>>(),
            );
            self.clear_word(None, "unrepresentable_text");
            self.untracked_token = !Self::_is_boundary(&event);
            return Ok(());
        }
        if self.strokes.len() + self.symbol_strokes.len() >= MAX_WORD_STROKES {
            self.clear_word(None, "token_too_long");
            self.untracked_token = !Self::_is_boundary(&event);
            return Ok(());
        }
        if !self.strokes.is_empty() && (WORD_JOINERS.contains(&event.character.as_str()) || matches!(event.character.as_str(), "_" | "@" | "/" | "\\" | "=")) {
            self.strokes.push(event.clone());
            self.mark_word_activity();
            self.shared.update(Update { current_word: Some(text_for_group(&self.strokes, self.source_group)), ..Update::default() });
            return Ok(());
        }
        if Self::_is_layout_letter(&event) {
            self.last_committed_stale = true;
            if !pystr::isalpha(&event.character) && self.ambiguous_key_is_boundary(&event)? {
                self.commit_word(&event)?;
                return Ok(());
            }
            if self.late_stroke_after_early_switch(&event) {
                event = self.convert_late_stroke(&event);
            }
            if self.source_group != -1 && self.source_group != i64::from(event.group) {
                self.clear_word(None, "layout_changed_mid_word");
            }
            if self.strokes.is_empty() && self.symbol_strokes.iter().any(|stroke| matches!(stroke.character.as_str(), "@" | "_" | "/" | "\\")) {
                self.strokes = std::mem::take(&mut self.symbol_strokes);
            }
            self.begin_word(&event);
            self.source_group = i64::from(event.group);
            self.strokes.push(event.clone());
            self.mark_word_activity();
            self.shared.update(Update {
                current_group: Some(i64::from(event.group)),
                current_word: Some(text_for_group(&self.strokes, i64::from(event.group))),
                last_error: Some(String::new()),
                ..Update::default()
            });
            self.after_mention_head(&event, None);
            self.maybe_early_switch()?;
            return Ok(());
        }
        if Self::_is_boundary(&event) {
            if self.strokes.is_empty() && event.character == "-" {
                self.source_group = i64::from(event.group);
                self.strokes.push(event.clone());
                self.mark_word_activity();
                self.after_mention_head(&event, None);
                return Ok(());
            }
            if !self.strokes.is_empty() {
                self.commit_word(&event)?;
                return Ok(());
            }
            if self.space_follows_pending_sign(&event) {
                // The correction made at a sign waits until its keys come up, and a space pressed meanwhile stands
                // after the sign on screen: the correction deletes and types it as its boundary. Taken for a second
                // boundary, it dropped the correction: `bcntr?` and a space 37 ms later, the `?` key still down,
                // stayed as typed though the model converted the word (0.56.0 log, 15.10.2026).
                let pending = self.pending.clone().expect("a pending plan");
                let mut literal = pending.trailing.clone();
                literal.extend(pending.boundary.clone());
                self.pending = Some(pending.replaced_with(|plan| {
                    plan.trailing = literal;
                    plan.boundary = Some(event.clone());
                    plan.revisit = None;
                }));
            } else {
                self.log_pending_dropped("additional_boundary");
                self.pending = None;
            }
            self.learning_prompt_after = None;
            // Nothing typed since the last boundary: remember layout-dependent
            // symbols so Pause converts just them (RU quote -> "@"). A space or a
            // sign typed the same in every layout leaves the previous word to
            // Pause (extend_last_committed); anything else ends its reach.
            if WORD_BOUNDARY_KEYS.contains(&event.key_name.as_str()) || pystr::isspace(&event.character) {
                self.symbol_strokes = Vec::new();
            } else if Self::_layout_dependent(&event) {
                self.symbol_strokes.push(event.clone());
            }
            if self.pause_reaches_past(std::slice::from_ref(&event)) {
                self.extend_last_committed(std::slice::from_ref(&event));
            } else {
                self.last_committed_stale = true;
            }
            if self.mention_shown.is_some() {
                self.after_mention_head(&event, Some(&event));
            } else if self.symbol_strokes.len() == 1 && Arc::ptr_eq(&self.symbol_strokes[0], &event) {
                self.show_mention_head(&event);
            }
            return Ok(());
        }
        if is_navigation_key(&event.key_name) {
            // The caret moved: what was typed belongs to another position, and
            // the engine no longer knows what stands in front of the new one.
            self.last_committed_stale = true;
            self.clear_word(None, "navigation");
            self.caret_moved = true;
            return Ok(());
        }
        if !event.character.is_empty() {
            if !(Self::_joins_parts(&event) && self.strokes.iter().all(|stroke| Self::_joins_parts(stroke))) {
                // A token of signs typed the same in every layout (`+`, `=>`) is no word: the word
                // before it stays Pause's until a letter or a digit comes (pause_reaches_past).
                self.last_committed_stale = true;
            }
            if !self.strokes.is_empty() {
                // A digit or another printable key inside a word ("зь2") stays
                // part of it, so Pause still converts the whole token. Nothing
                // changes for automatic correction: the detector treats a token
                // carrying a digit as code and leaves it alone.
                self.strokes.push(event.clone());
                self.mark_word_activity();
                self.shared.update(Update { current_word: Some(text_for_group(&self.strokes, self.source_group)), ..Update::default() });
                return Ok(());
            }
            if Self::_layout_dependent(&event) {
                // "@" typed in the US layout but meant as the RU quote, or the
                // other way round: what it is depends on the word that follows,
                // so it waits here until that word is decided (mention_head).
                self.symbol_strokes.push(event.clone());
                return Ok(());
            }
            // A leading digit or path marker belongs to the token too:
            // `2ghbdtn` must not be seen as the unrelated word `ghbdtn`.
            self.begin_word(&event);
            self.source_group = i64::from(event.group);
            self.strokes.push(event.clone());
            self.mark_word_activity();
            self.shared.update(Update { current_word: Some(text_for_group(&self.strokes, i64::from(event.group))), ..Update::default() });
            self.after_mention_head(&event, None);
        } else if event.key_name != "Pause" && event.key_name != "Break" {
            self.clear_word(None, "untracked_key");
            self.untracked_token = true;
        }
        Ok(())
    }

    /// Only one simple character per physical key can be backspaced.
    ///
    /// Composed text, surrogate pairs, combining marks and IME results do
    /// not have the one-key/one-deletion contract used by these backends.
    pub(crate) fn _safe_text_stroke(event: &KeyEvent) -> bool {
        event.characters.iter().all(|char| {
            let mut characters = char.chars();
            match (characters.next(), characters.next()) {
                (Some(only), None) => {
                    let group = category(only).as_bytes()[0];
                    only.to::<u32>() <= BASIC_MULTILINGUAL_PLANE_MAX_CODEPOINT && group != b'M' && group != b'C'
                }
                _ => false,
            }
        })
    }

    /// A printable key whose character differs between the layouts.
    pub(crate) fn _layout_dependent(event: &KeyEvent) -> bool {
        if event.character.is_empty() {
            return false;
        }
        let mut seen: Option<&str> = None;
        for character in &event.characters {
            if character.is_empty() {
                continue;
            }
            match seen {
                None => seen = Some(character),
                Some(first) if first != character => return true,
                Some(_) => {}
            }
        }
        false
    }

    pub(crate) fn _is_layout_letter(event: &KeyEvent) -> bool {
        event.characters.iter().any(|character| pystr::isalpha(character))
    }

    /// A key that types one and the same sign in every layout (`-`, `_`, `+`, `=`) can join two
    /// words typed in different layouts. A sign that is a letter in another layout is a letter of the
    /// word: `,` is the `б` of `cj,thb` (`собери`), `;` the `ж` of `ye;yj` (`нужно`).
    pub(crate) fn _joins_parts(stroke: &KeyEvent) -> bool {
        let Some(first) = stroke.characters.first() else {
            return false;
        };
        stroke.characters.iter().all(|character| character == first)
            && stroke.character == *first
            && !pystr::isalnum(first)
            && !pystr::isspace(first)
    }

    #[must_use]
    pub fn _is_boundary(event: &KeyEvent) -> bool {
        let name = event.key_name.as_str();
        if WORD_BOUNDARY_KEYS.contains(&name) || ACTION_BOUNDARY_KEYS.contains(&name) {
            return true;
        }
        let character = event.character.as_str();
        if character.is_empty() {
            return false;
        }
        pystr::isspace(character)
            || (pystr::len(character) == 1 && PUNCTUATION.contains(character))
            || (!matches!(character, "_" | "/" | "\\" | "@")
                && character.chars().next().is_some_and(|first| category(first).starts_with('P')))
    }

    pub(crate) fn boundary_enabled(&self, event: &KeyEvent) -> bool {
        let name = event.key_name.as_str();
        if name == "space" || (pystr::isspace(&event.character) && !is_action_boundary_key(name)) {
            return self.flag("detection.correct_on_space", true);
        }
        if name == "Return" || name == "KP_Enter" {
            return self.flag("detection.correct_on_enter", true);
        }
        if name == "Tab" || name == "ISO_Left_Tab" {
            return self.flag("detection.correct_on_tab", true);
        }
        self.flag("detection.correct_on_punctuation", true)
    }

    pub(crate) fn _trigger_for_boundary(event: &KeyEvent) -> ks_lang::intent_model::CorrectionTrigger {
        use ks_lang::intent_model::CorrectionTrigger;
        let name = event.key_name.as_str();
        if name == "space" || (pystr::isspace(&event.character) && !is_action_boundary_key(name)) {
            return CorrectionTrigger::Space;
        }
        if name == "Return" || name == "KP_Enter" {
            return CorrectionTrigger::Enter;
        }
        if name == "Tab" || name == "ISO_Left_Tab" {
            return CorrectionTrigger::Tab;
        }
        CorrectionTrigger::Punctuation
    }

    pub(crate) fn matches_hotkey(&self, name: &str, event: &KeyEvent) -> bool {
        if !event.pressed {
            return false;
        }
        let path = format!("hotkeys.{name}");
        self.settings.with_value(&path, |value| {
            let text;
            let value = match value {
                None => "",
                Some(Value::String(found)) => found.as_str(),
                Some(other) => {
                    text = other.to_string();
                    text.as_str()
                }
            };
            let mut cache = self.hotkey_cache.borrow_mut();
            if let Some(hotkey) = cache.get(value) {
                return hotkey.matches(event);
            }
            let hotkey = Arc::new(Hotkey::new(value));
            let answer = hotkey.matches(event);
            cache.insert(value.to_owned(), hotkey);
            answer
        })
    }

    pub(crate) fn selection_hotkey(&self, event: &KeyEvent) -> Option<&'static str> {
        SELECTION_HOTKEYS.iter().copied().find(|name| self.matches_hotkey(name, event))
    }

    pub(crate) fn caret_unknown(&self) -> bool {
        // Whether this word is being typed where the engine cannot see the surroundings.
        //
        // An arrow key, Home or Page Up puts the caret somewhere the engine has not
        // watched being typed. Until now the word that followed was analysed as if it
        // opened an empty field, which is the one thing it is least likely to be: the
        // caret is usually moved back *into* text, to finish or fix a word. Feeding
        // that invented emptiness to the models is worse than saying nothing, so the
        // word is left alone and Pause still converts it by hand.
        //
        // When the active field can be read, the surroundings are not a guess and the
        // rule does not apply - the models get the real text before the caret.
        if !self.caret_moved {
            return false;
        }
        if !self.flag("detection.hold_after_caret_move", true) {
            return false;
        }
        if !self.flag("detection.context_read_field", true) {
            return true;
        }
        self.context_policy.reader.as_ref().is_none_or(|reader| reader.status() != "available")
    }

    pub(crate) fn mark_word_activity(&mut self) {
        self.last_word_input_at = Some(self.monotonic());
        self.pause_correction_pending = true;
        self.pause_deferral_logged = false;
    }

    pub(crate) fn reset_pause_correction(&mut self) {
        self.last_word_input_at = None;
        self.pause_correction_pending = false;
    }

}
