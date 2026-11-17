//! A change of layout the engine made by itself, erased with Backspace and the word typed again as it was first typed: the
//! word is remembered as a "keep" rule of the user's, and is no longer converted.
//!
//! Erasing into the word starts it (`note_backspace`): the word committed next at its place is compared with the word as
//! first typed (`settle_erased_layout`). Typing the corrected form again, another word, or anything after a click, a caret
//! move, another window or Enter remembers nothing.

use super::logging::technical_event;
use super::*;
use ks_config::constants::DETECTION_REMEMBER_RETYPED;
use ks_config::learning::{LearnedRule, RuleAction};

impl EngineCore {
    /// Whether a word erased and typed again as first typed is remembered: `detection.remember_retyped`, and the rules
    /// acting at all (`detection.learning`), as a rule that would not act is not made.
    pub fn remembers_retyped(&self) -> bool {
        self.flag(DETECTION_REMEMBER_RETYPED.path(), true) && self.flag("detection.learning", true)
    }

    /// The word `plan` changed the layout of, as Backspace may erase it; None for a plan that changes no word's layout of
    /// itself: a manual conversion or an undo, a typo correction (`typo_erased` follows those), a case fix or a sign typed
    /// in the layout it belongs to (both keep their layout), and a word converted before a neighbour typed again as it was
    /// (`kept_tail`), which stands where the word's own keys would be read. A mention head is no correction remembered. A phrase converted with the word ends in it: the word is its keys after the last
    /// space. Keys the user picked the layout of by hand (`picked_tail`) were not typed in the layout of the rest, and such
    /// a word is never the word typed again as first typed.
    pub fn corrected_word_of(&self, plan: &CorrectionPlan) -> Option<CorrectedWord> {
        if !plan.automatic
            || plan.source_group == plan.target_group
            || plan.mode == TYPO_PLAN_MODE
            || plan.kept_tail != 0
        {
            return None;
        }
        let start = plan.strokes.iter().rposition(|key| pystr::isspace(&key.character)).map_or(0, |space| space + 1);
        Some(CorrectedWord {
            keys: plan.strokes[start..].to_vec(),
            source_group: plan.source_group,
            target_group: plan.target_group,
            mode: plan.mode,
            shown_at: self.monotonic(),
            in_progress: plan.boundary.is_none(),
            unseen: false,
            erased_after: 0.0,
        })
    }

    /// The word being typed, when the engine changed its layout while it is typed and Backspace has not reached it yet:
    /// the early switch, or a conversion at a pause the word went on after. Every key of it is then a letter of the
    /// corrected word, and so it is as it stands now that a Backspace erases into it.
    pub fn corrected_in_progress(&self) -> Option<CorrectedWord> {
        let first = self.strokes.first()?;
        let begins = |word: &CorrectedWord| word.keys.first().is_some_and(|key| Arc::ptr_eq(key, first));
        if self.layout_erased.as_ref().is_some_and(begins) {
            // Erased into already: the Backspaces go on in the same word.
            return None;
        }
        if let (Some(origin), Some(shown_at)) = (self.early_switch_origin, self.early_switch_at) {
            return Some(CorrectedWord {
                keys: self.strokes.clone(),
                source_group: origin,
                target_group: self.source_group,
                mode: EARLY_PLAN_MODE,
                shown_at,
                in_progress: true,
                unseen: false,
                erased_after: 0.0,
            });
        }
        // A word changed inside it ends at its boundary (`end_corrected_word`), and a click, a caret move or another window
        // forget it: while it is in progress, it is the word being typed.
        let word = self.corrected_word.as_ref().filter(|word| word.in_progress)?;
        Some(CorrectedWord { keys: self.strokes.clone(), ..word.clone() })
    }

    /// A Backspace erased a letter of `word`: the word committed next at this place is compared with it. Backspaces begun
    /// sooner after the change than a person reads the word (`ERASE_UNSEEN_SECONDS`) edit the text as typed: such a word
    /// is kept only to say so if it is typed again.
    pub fn layout_correction_erased(&mut self, mut word: CorrectedWord) {
        if !self.remembers_retyped() {
            return;
        }
        let elapsed = self.monotonic() - word.shown_at;
        if elapsed < ERASE_UNSEEN_SECONDS {
            self.erase_unseen = true;
        }
        word.unseen = self.erase_unseen;
        word.erased_after = elapsed;
        self.layout_erased = Some(word);
    }

    /// A word changed while it was typed ends at its boundary: from now on the corrected word is all of it but its
    /// `trailing` signs, which stand after it with the boundary. The word being typed is that word: nothing else is typed
    /// while it is in progress.
    pub fn end_corrected_word(&mut self, token: &[Stroke], trailing: usize) {
        let Some(word) = self.corrected_word.as_mut().filter(|word| word.in_progress) else {
            return;
        };
        word.keys = token[..token.len() - trailing].to_vec();
        word.in_progress = false;
        self.after_correction = Some(trailing + 1);
    }

    /// Whether `keys`, typed where a change of layout was erased, begin the word as first typed, in the layout it was
    /// typed in: nothing converts them while they may still be that word, neither the early switch nor a pause.
    pub fn retyping_erased_word(&self, keys: &[Stroke]) -> bool {
        let Some(word) = self.layout_erased.as_ref().filter(|word| !word.unseen) else {
            return false;
        };
        keys.len() <= word.keys.len()
            && keys.iter().all(|key| i64::from(key.group) == word.source_group)
            && pystr::casefold(&text_for_group(&word.keys, word.source_group)).starts_with(&pystr::casefold(&text_for_group(keys, word.source_group)))
    }

    /// The word typed where a change of layout was erased (`layout_erased`) is committed: `token`, its keys but for the
    /// signs after it. If it ends in the word as first typed, letter for letter but for case, typed in the layout the word
    /// was typed in, and after no letter (a hyphen or a quote may stand before it, as before a part of a joined word),
    /// the word becomes a "keep" rule for that layout, and the windows are told. A rule that already decides about the
    /// word is left as it is. Called before the word is decided, so that the word itself stays as typed.
    pub fn settle_erased_layout(&mut self, token: &[Stroke], application: &str) {
        let Some(word) = self.layout_erased.take() else {
            return;
        };
        let group = word.source_group;
        let Some(start) = token.len().checked_sub(word.keys.len()) else {
            return;
        };
        let first = text_for_group(&word.keys, group);
        let typed = &token[start..];
        let after_letter = start != 0 && pystr::isalpha(token[start - 1].character_for(group.to::<i32>()));
        if after_letter
            || !typed.iter().all(|key| i64::from(key.group) == group)
            || pystr::casefold(&text_for_group(typed, group)) != pystr::casefold(&first)
            || !self.remembers_retyped()
        {
            return;
        }
        let excluded = self.application_excluded(application);
        let not_remembered = |shared: &Shared, reason: &str| {
            technical_event!(
                shared, "retyped_word_not_remembered", "reason": reason, "mode": word.mode, "letters": pystr::len(&first),
                "source_group": group, "erased_after_ms": ms(word.erased_after), "application": application,
                "application_excluded": excluded,
            );
        };
        if word.unseen {
            not_remembered(&self.shared, "erased_before_seen");
            return;
        }
        if self.learning.r#match(group, &first).is_some() {
            not_remembered(&self.shared, "rule_exists");
            return;
        }
        let rule = LearnedRule { action: RuleAction::Keep, ..LearnedRule::new(&first, group, word.target_group) };
        let stored = match self.learning.add_rule(rule) {
            Ok(stored) => stored,
            Err(error) => {
                technical_event!(
                    self.shared, "retyped_word_not_remembered", "reason": "not_stored", "error": error.to_string(), "mode": word.mode,
                    "letters": pystr::len(&first), "source_group": group, "application": application, "application_excluded": excluded,
                );
                return;
            }
        };
        technical_event!(
            self.shared, "retyped_word_remembered",
            "letters": pystr::len(&first),
            "source_group": stored.source_group,
            "target_group": stored.target_group,
            "mode": word.mode,
            "erased_after_ms": ms(word.erased_after),
            "application": application,
            "application_excluded": excluded,
        );
        self.shared.update(Update { last_action: Some(format!("Слово запомнено: «{}»", stored.pattern)), ..Update::default() });
        let remembered = RememberedWord::new(stored, self.learning.clone(), self.shared.clone());
        for callback in self.shared.remembered_callbacks() {
            callback(&remembered);
        }
    }
}
