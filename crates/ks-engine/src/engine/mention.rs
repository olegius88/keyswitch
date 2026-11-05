//! A quote typed where the application reads `@` as the start of a mention.

use super::*;
use ks_config::app_quirks::mention_head;

impl EngineCore {
    // The same key pressed against the end of the word closes a quotation.
    //
    // `"john"` is a quoted name, not a mention: a quote typed with no space
    // after the word can only be the closing one, and then the quote in front
    // of it was the opening one. The word is still judged on its own, so the
    // name inside the quotation is corrected while both quotes stay.
    pub(crate) fn _quotation_closed(head: &KeyEvent, trailing: &[Stroke], boundary: Option<&Stroke>) -> bool {
        trailing.iter().chain(boundary).any(|stroke| stroke.keycode == head.keycode)
    }

    /// The pending symbol in front of the current word that may be a mention.
    ///
    /// The symbol stays out of the analysed word on purpose: the models read
    /// `ощрт` and `john`, not `"ощрт` and `@john`, and with the quote attached
    /// they fall below their own threshold. The word alone decides, and this
    /// head is then replaced together with it (`execute_correction`).
    pub(crate) fn mention_head(&self, application: &str) -> Strokes {
        if self.symbol_strokes.len() != 1 || self.strokes.is_empty() {
            return Vec::new();
        }
        let stroke = &self.symbol_strokes[0];
        if self.mention_convention(stroke, application) { vec![stroke.clone()] } else { Vec::new() }
    }

    /// Whether this application reads the symbol on this key as the start of a mention.
    fn mention_convention(&self, stroke: &Stroke, application: &str) -> bool {
        let source_group = i64::from(stroke.group);
        let target = self.other_groups(source_group).first().copied();
        let Some(target) = target.filter(|_| self.in_models(source_group)) else {
            return false;
        };
        let key = std::slice::from_ref(stroke);
        let typed = text_for_group(key, source_group);
        let meant = text_for_group(key, target);
        mention_head(application, &typed, &meant, &|path| self.flag(path, true)).is_some()
    }

    /// Write a lone quote as ``@`` at once where the application reads it as a mention.
    ///
    /// Telegram opens its member list only after a real ``@``, and that list is
    /// how a mention is made: in the collected logs every ``@`` in the chat was
    /// followed by the arrow keys, Enter or a click, never by letters. Waiting
    /// for a word that never comes left the quote a quote. So the ``@`` appears
    /// on the key's release, in place, without changing the layout: a message
    /// continued in Russian after the list stays Russian. It stays only while
    /// nothing is typed after it; the next printable key writes the quote back
    /// (`after_mention_head`) and the word that follows decides, as before,
    /// whether the pair was ``@name`` or a quotation. A quote typed after a
    /// caret move or after the layout was chosen by hand is left alone.
    pub(crate) fn show_mention_head(&mut self, stroke: &Stroke) {
        let application = self.backend.active_application();
        if !self.flag("enabled", true)
            || self.application_excluded(&application)
            || self.caret_unknown()
            || self.word_protected(i64::from(stroke.group))
            || !self.mention_convention(stroke, &application)
            // A quote typed right against text closes a quotation: `привет,"`.
            // Only one at the start of the text or after a space, Enter or Tab
            // can open a mention.
            || (!self.character_before_key.is_empty() && !pystr::isspace(&self.character_before_key))
        {
            return;
        }
        let group = i64::from(stroke.group);
        let target = self.other_groups(group)[0];
        let shown = key_of(KeyEvent {
            group: target.to::<i32>(),
            character: stroke.character_for(target.to::<i32>()).to_owned(),
            ..(**stroke).clone()
        });
        // Nothing is retyped in another layout: the "@" goes out as a literal of
        // the layout it belongs to, and the backends restore the current one after it.
        self.pending = Some(Arc::new(CorrectionPlan {
            mode: "mention_shown",
            trailing: vec![shown.clone()],
            ..CorrectionPlan::new(
                Vec::new(), None, group, group, stroke.character.clone(), shown.character.clone(), UNSCORED_CORRECTION_CONFIDENCE, &application,
            )
        }));
        self.learning_prompt_after = None;
        self.pending_trigger_keycode = i64::from(stroke.keycode);
    }

    /// A key typed after the quote: the member list was not what came next.
    pub(crate) fn after_mention_head(&mut self, event: &Stroke, boundary: Option<&Stroke>) {
        if self.pending.as_ref().is_some_and(|pending| pending.mode == "mention_shown") {
            // Typed before the quote key came up: nothing was rewritten yet.
            self.log_pending_dropped("typing_continued");
            self.pending = None;
            return;
        }
        let Some(head) = self.mention_shown.clone() else {
            return;
        };
        if event.group != head.group {
            // The user switched the layout after the "@" and typed on: a name
            // in the other layout after a real "@" is exactly a mention.
            self.mention_shown = None;
            return;
        }
        self.hide_mention_head(&head, boundary, i64::from(event.keycode));
    }

    pub(crate) fn hide_mention_head(&mut self, head: &Stroke, boundary: Option<&Stroke>, trigger_keycode: i64) {
        if boundary.is_none() && self.pending.as_ref().is_some_and(|pending| pending.mode == "mention_hidden") {
            // Already waiting for the first letter to come up; it takes the
            // letters typed meanwhile along (maybe_execute_pending).
            return;
        }
        self.log_pending_dropped("mention_head_written_back");
        let strokes = self.strokes.clone();
        let application = self.backend.active_application();
        self.pending = Some(self.mention_write_back(head, &strokes, &[], boundary.cloned(), &application));
        self.learning_prompt_after = None;
        self.pending_trigger_keycode = trigger_keycode;
    }

    /// The plan that turns the shown `@` back into the quote, keeping what follows it.
    pub(crate) fn mention_write_back(&self, head: &Stroke, strokes: &[Stroke], trailing: &[Stroke], boundary: Option<Stroke>, application: &str) -> PlanRef {
        let head_group = i64::from(head.group);
        let shown_group = self.other_groups(head_group)[0];
        let head_key = std::slice::from_ref(head);
        let mut keys = vec![head.clone()];
        keys.extend(strokes.iter().cloned());
        let original = format!("{}{}", text_for_group(head_key, shown_group), text_for_group(strokes, head_group));
        let replacement = text_for_group(&keys, head_group);
        Arc::new(CorrectionPlan {
            mode: "mention_hidden",
            trailing: trailing.to_vec(),
            ..CorrectionPlan::new(keys, boundary, shown_group, head_group, original, replacement, UNSCORED_CORRECTION_CONFIDENCE, application)
        })
    }
}
