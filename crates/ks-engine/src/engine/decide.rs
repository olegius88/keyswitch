//! Asking about a word: the detector's baseline, the contextual model, the word begun inside text.

use super::logging::technical_event;
use super::*;
use ks_context::context_policy::{DecideOptions as PolicyOptions, STRANDED_SHORT_WORD_REASON};
use ks_lang::detector::DecideOptions as DetectorOptions;
use ks_lang::intent_model::CorrectionTrigger;
use ks_lang::short_words::{SINGLE_LETTER_CONFIDENCE, TRUSTED_SINGLE_LETTER_WORDS};
use ks_lang::word_decision::{automatic_word_decision, AutomaticOptions};

/// The keyword arguments of `_decide_word`.
#[derive(Default)]
pub(crate) struct WordOptions {
    pub literal_tail: String,
    pub boundary_text: String,
    pub field_override: Option<FieldContext>,
    pub inside: bool,
    pub following: Option<Alternatives>,
}

fn latin_letter(character: char) -> bool {
    character.is_ascii_alphabetic()
}

fn cyrillic_letter(character: char) -> bool {
    matches!(character, 'А'..='Я' | 'а'..='я' | 'Ё' | 'ё')
}

impl EngineCore {
    pub(crate) fn _begin_word(&mut self, event: &Stroke) {
        if !self._strokes.is_empty() {
            return;
        }
        self._personal_word_settled(true);
        if self._position_unknown {
            self._insertion = self._read_insertion(event);
        }
        self._position_unknown = false;
    }

    /// What stands around the caret where a word begins after a click or a caret move.
    ///
    /// Clicking into written text and typing is how a word is fixed in place:
    /// `сд|лать` gets its missing `е`. The engine saw none of that text, so
    /// the letters were judged as a word of their own at the start of an empty
    /// field. The field is read once, as the word begins, and the letters
    /// right before and after the caret say whether this is a new word or the
    /// middle of an existing one (_decide_inside_word).
    pub fn _read_insertion(&mut self, event: &Stroke) -> Option<InsertionPoint> {
        let reader = self.context_policy.reader.clone().filter(|_| self.flag("detection.context_read_field", false))?;
        let application = self.backend.active_application();
        if self._application_excluded(&application) {
            return None;
        }
        let snapshot = reader.read(&application, self.focus_or_zero())?;
        if snapshot.sensitive || snapshot.selection || snapshot.application != application {
            return None;
        }
        // The key may already have reached the editor, or not yet, and the editor may report its
        // caret short of the key that has (caret_lag): VS Code Insiders put the first letter of
        // every new word after the caret, and each read as typed into the word it began. The keys
        // queued behind it may stand there too: the Claude app showed `lt` with the caret after `l`
        // as `делай` began, and the `t` read as a word's letter after the caret (09.10.2026).
        let around = if event.character.is_empty() {
            None
        } else {
            Self::_typed_in_field(&snapshot, &event.character, &self._typed_ahead(std::slice::from_ref(event)))
        };
        let (before, after) = around.unwrap_or((snapshot.before, snapshot.after));
        let point = InsertionPoint { head: Self::_letters_before(&before), tail: Self::_letters_after(&after) };
        if point.inside_word() {
            technical_event!(
                self.shared, "word_started_inside_text", "application": application,
                "head_letters": pystr::len(&point.head), "tail_letters": pystr::len(&point.tail),
            );
        }
        Some(point)
    }

    /// Whether the word begun inside text still has letters of that text around it.
    ///
    /// The first read can be wrong: an editor that shows a key before its caret has moved puts the
    /// word's own next letter after the caret. Of the 188 words read as begun inside text in the
    /// field logs (Claude app, VS Code, Telegram, Firefox, 05.09-09.10.2026), 121 had no letters
    /// around them by their decision; the early switch waited inside them 55 times (`делай` took
    /// three tries, 09.10.2026), and 22 were left as typed as a changed field (`офыы` for `jass`).
    /// Before the early switch, the boundary and the pause rely on it, the field is read again
    /// and the word typed so far found in it as the early switch finds it (_typed_in_field): with
    /// no letter right before or after it, the first read is dropped. A field that cannot be read
    /// or matched keeps it.
    pub(crate) fn _insertion_holds(&mut self, typed: &str, ahead: &str, application: &str) -> bool {
        let Some(point) = self._insertion.clone().filter(InsertionPoint::inside_word) else {
            return false;
        };
        let Some(reader) = self.context_policy.reader.clone().filter(|_| self.flag("detection.context_read_field", false)) else {
            return true;
        };
        let Some(snapshot) = reader.read(application, self.focus_or_zero()) else {
            return true;
        };
        if snapshot.sensitive || snapshot.selection || snapshot.application != application {
            return true;
        }
        // An input buffer read in place of the text was never the text around the word, and neither
        // was the first read (FieldContext.input_buffer_only).
        let buffer = snapshot.input_buffer_only(typed);
        let around = Self::_typed_in_field(&snapshot, typed, ahead);
        let letters = match &around {
            None => true,
            Some((before, after)) => !Self::_letters_before(before).is_empty() || !Self::_letters_after(after).is_empty(),
        };
        if !buffer && letters {
            return true;
        }
        technical_event!(
            self.shared, "insertion_dropped", "application": application,
            "head_letters": pystr::len(&point.head), "tail_letters": pystr::len(&point.tail),
            "reason": if buffer { "field_input_buffer" } else { "no_letters_around" },
        );
        self._insertion = None;
        false
    }

    /// Letters typed into the middle of a word are judged as that whole word.
    ///
    /// `t` typed between `сд` and `лать` is not a word: the question is what the
    /// whole word is. When the letters around it are in the other layout, the
    /// word is put to the usual decision as if all of it had been typed in the
    /// fragment's layout - `cltkfnm` against `сделать` - and only the typed
    /// fragment is replaced. When they are in the fragment's own layout, the
    /// fragment agrees with its word and is left alone. At a boundary the word
    /// ends there, so only the letters before the caret belong to it.
    /// Returns None when the word was not begun inside text.
    pub(crate) fn _decide_inside_word(
        &mut self, strokes: &[Stroke], source_group: i64, alternatives: &Alternatives, application: &str, trigger: CorrectionTrigger,
        boundary: Option<&Stroke>,
    ) -> EngineResult<Option<DetectionDecision>> {
        let point = self._insertion.clone();
        let target = alternatives.first().map(|entry| entry.0);
        let reader = self.context_policy.reader.clone();
        let (Some(point), Some(target), Some(reader)) = (point.filter(InsertionPoint::inside_word), target, reader) else {
            return Ok(None);
        };
        let fragment = _text_for_group(strokes, source_group);
        let closing = boundary.filter(|boundary| !boundary.deferred).map_or("", |boundary| boundary.character.as_str());
        let snapshot = reader.read(application, self.focus_or_zero());
        let mut around = None;
        if let Some(snapshot) = snapshot.as_ref().filter(|snapshot| !snapshot.sensitive && !snapshot.selection && snapshot.application == application) {
            // Found as the early switch finds a word: the boundary and the keys queued behind it may
            // stand after it, the caret short of them, or its last letters may not be shown yet.
            around = Self::_typed_in_field(snapshot, &fragment, &format!("{closing}{}", self._typed_ahead(strokes)));
        }
        let (Some(snapshot), Some((before, after))) = (snapshot, around) else {
            return Ok(Some(self._kept_inside_word(&fragment, source_group, "field_changed", &point)));
        };
        let head = Self::_letters_before(&before);
        let tail = if boundary.is_some() { String::new() } else { Self::_letters_after(&after) };
        if head.is_empty() && tail.is_empty() {
            return Ok(None);
        }
        let surroundings = format!("{head}{tail}");
        let source_letters = self._layout_letters(source_group);
        let target_letters = self._layout_letters(target);
        if target_letters.is_empty()
            || !surroundings.chars().all(|character| target_letters.contains(&character))
            || surroundings.chars().any(|character| source_letters.contains(&character))
        {
            let same = !source_letters.is_empty() && surroundings.chars().all(|character| source_letters.contains(&character));
            return Ok(Some(self._kept_inside_word(&fragment, source_group, if same { "same_layout" } else { "mixed_layout" }, &point)));
        }
        let pair = ks_text::layouts::LayoutPair::default();
        let source_name = self._layout_name(source_group);
        let target_name = self._layout_name(target);
        let whole = format!("{}{fragment}{}", pair.translate(&head, &target_name, &source_name)?, pair.translate(&tail, &target_name, &source_name)?);
        let alternative = &alternatives[0].1;
        if tail.is_empty()
            && !any_alpha(&fragment)
            && !self._signs_are_letters(&whole, &format!("{head}{alternative}"), source_group, target, pystr::len(&fragment))
        {
            // Signs typed right after a word's last letter end it: `,` after `привет` is a comma, `.` after `сдела`
            // the `ю` of `сделаю`. Judged as the whole word, a Russian word read in Latin keys loses to its Russian
            // reading whatever was typed after it, and `.` and `,` typed after a word just cut back with Backspace
            // became `ю` and `б` (field typing, 19-20.10.2026). The boundary model, which reads the signs at
            // the end of every word typed, decides it on the whole word; a sign it does not read as a letter stays.
            return Ok(Some(self._kept_inside_word(&fragment, source_group, "sign_after_word", &point)));
        }
        // The model is told that the word is being edited in place: every other letter of
        // it is already in the other layout.
        let before_head = pystr::slice(&before, None, Some((pystr::len(&before) - pystr::len(&head)) as i64)).to_owned();
        let after_tail = pystr::slice(&after, Some(pystr::len(&tail) as i64), None).to_owned();
        let decision = self._decide_word(&whole, &vec![(target, format!("{head}{alternative}{tail}"))], source_group, application, trigger, WordOptions {
            boundary_text: closing.to_owned(),
            field_override: Some(field_with(&snapshot, before_head, after_tail)),
            inside: true,
            ..WordOptions::default()
        })?;
        let converted = decision.should_convert && decision.target_group == target;
        technical_event!(
            self.shared, "inside_word_decision", "application": application, "surroundings": "other_layout",
            "head_letters": pystr::len(&head), "tail_letters": pystr::len(&tail), "converted": converted,
        );
        if !converted {
            // A wait or a suggestion is about a next word; this one is inside text.
            self._context_result = None;
            return Ok(Some(DetectionDecision { should_convert: false, original: fragment.clone(), replacement: fragment, ..decision }));
        }
        Ok(Some(DetectionDecision { original: fragment, replacement: alternative.clone(), ..decision }))
    }

    /// Whether the boundary model reads the last `signs` keys of a word as its letters (`сдела.` is `сделаю`)
    /// rather than signs after it (`привет,`); an abstention keeps them signs, as typed.
    fn _signs_are_letters(&self, original: &str, alternative: &str, source_group: i64, target: i64, signs: usize) -> bool {
        let Some(model) = self.boundary_model.as_ref().filter(|_| self.in_models(source_group) && self.in_models(target)) else {
            return false;
        };
        let candidates: Vec<_> = (0..=signs as i64)
            .map(|length| model.extract(original, alternative, length, self.model(source_group), self.model(target)))
            .collect();
        model.predict(&candidates).is_some_and(|prediction| prediction.suffix_length == Some(0))
    }

    fn _kept_inside_word(&self, fragment: &str, source_group: i64, reason: &str, point: &InsertionPoint) -> DetectionDecision {
        technical_event!(
            self.shared, "inside_word_decision", "surroundings": reason,
            "head_letters": pystr::len(&point.head), "tail_letters": pystr::len(&point.tail), "converted": false,
        );
        let score = self.model(source_group).score(fragment);
        DetectionDecision::new(false, fragment, fragment, source_group, source_group, 0.0, "слово набрано внутри другого слова", score.clone(), score)
    }

    pub(crate) fn _decide_word(
        &mut self, original: &str, alternatives: &Alternatives, source_group: i64, application: &str, trigger: CorrectionTrigger, options: WordOptions,
    ) -> EngineResult<DetectionDecision> {
        self._context_result = None;
        self._last_baseline = None;
        let mut decision = self._baseline_decision(original, alternatives, source_group, application, trigger, options.following.as_ref())?;
        if !decision.should_convert && !options.inside {
            decision = self._stranded_letter(decision, original, alternatives, source_group);
        }
        self._last_baseline = Some(decision.clone());
        let context_aware = self.flag("detection.context_aware", true);
        let key = LanguageDetector::token_key(original);
        let rejected_targets = self._rejected_targets(source_group, original);
        let forced_target = self._forced_target_group(source_group, original);
        if !context_aware
            || forced_target.is_some()
            || trigger == CorrectionTrigger::BoundaryProbe
            || self.words_setting("exclusions.words").iter().any(|word| LanguageDetector::token_key(word) == key)
            || self._application_excluded(application)
        {
            // A token the user excluded, a layout they chose by hand and an excluded
            // application are their own decisions; the model is not asked about those.
            // "Protect code" is not in that list any more: it used to skip the model for
            // every token carrying a digit, so `зь2` stayed while the model said convert
            // with 0.9993 - and the model keeps `pm2`, `npm`, `git` and `h264` on its own
            // (measured 17.09.2026). The detector still refuses such tokens in the
            // baseline; telling code from a mistyped word is exactly what the model is for.
            return Ok(decision);
        }
        self._consult_context_model(decision, original, alternatives, &rejected_targets, application, trigger, options)
    }

    /// A curated Russian letter typed in the English layout right after an English term in a Russian phrase.
    ///
    /// A term typed amid Russian prose is converted, or typed in the English layout on purpose,
    /// and the layout stays English; the Russian phrase goes on, and its one-letter words come
    /// out as `f`, `b`, `d`, `c`: `поправь env b`. The curated rule needs the previous word in
    /// Russian, and the model has no single-letter row to know the letter by, so in a user's
    /// logs this was the largest class of words left in the wrong layout (about fifty, a fifth
    /// of all). A lower-case letter whose other reading is one of the curated words converts when
    /// the words before it on its line are a Latin term of up to STRANDED_TERM_MAX_WORDS words with
    /// a Russian word before the term and no end of a sentence in between. A capital letter is
    /// left alone (`plan B` in English prose), and so is a letter after Latin text alone.
    fn _stranded_letter(&self, decision: DetectionDecision, original: &str, alternatives: &Alternatives, source_group: i64) -> DetectionDecision {
        let mut characters = original.chars();
        let (Some(only), None) = (characters.next(), characters.next()) else {
            return decision;
        };
        if !latin_letter(only) || !pystr::islower(original) {
            return decision;
        }
        let rejected = self._rejected_targets(source_group, original);
        let target = alternatives
            .iter()
            .find(|(group, text)| *group != source_group && !rejected.contains(group) && TRUSTED_SINGLE_LETTER_WORDS.contains(&text.as_str()));
        let Some((group, replacement)) = target else {
            return decision;
        };
        // The words the engine saw typed first; a word typed into text already there (after a click)
        // has none of them, and the field's own text before it tells: `почему я не вижу в vscode d`,
        // `в vscode d` typed in the middle of the line, kept `d` (a laptop, 13.10.2026).
        if !Self::_after_russian_term(&self.context_policy.stream.before_word(original)) {
            match self._field_before(original) {
                Some(field) if Self::_after_russian_term(&field) => {}
                _ => return decision,
            }
        }
        DetectionDecision {
            should_convert: true,
            replacement: replacement.clone(),
            target_group: *group,
            confidence: SINGLE_LETTER_CONFIDENCE,
            reason: STRANDED_SHORT_WORD_REASON.to_owned(),
            target_score: self.model(*group).score(replacement),
            ..decision
        }
    }

    /// Whether text ends, on its line, with a Latin term of up to STRANDED_TERM_MAX_WORDS words after a
    /// Russian word, with no end of a sentence in between, and a space after it.
    fn _after_russian_term(before: &str) -> bool {
        match before.chars().next_back() {
            // A letter joined to the term (`usb-c`, `gen-z`) is part of it, not a word of the phrase.
            Some(last) if char_isspace(last) => {}
            _ => return false,
        }
        let line = pystr::rsplit_sep(before, "\n", 1).and_then(|parts| parts.last().copied()).unwrap_or(before);
        let mut words = pystr::split(line);
        let mut term = 0;
        while let Some(word) = words.last().copied() {
            if term >= STRANDED_TERM_MAX_WORDS || !word.chars().any(latin_letter) || word.chars().any(cyrillic_letter) {
                break;
            }
            if word.chars().next_back().is_some_and(|last| SENTENCE_END_SIGNS.contains(last)) {
                return false;
            }
            words.pop();
            term += 1;
        }
        term != 0
            && words.last().is_some_and(|word| {
                word.chars().any(cyrillic_letter) && !word.chars().next_back().is_some_and(|last| SENTENCE_END_SIGNS.contains(last))
            })
    }

    /// The field's text before a word just typed, as the reader shows it; None without a read that holds it.
    fn _field_before(&self, typed: &str) -> Option<String> {
        let reader = self.context_policy.reader.as_ref().filter(|_| self.flag("detection.context_read_field", false))?;
        let application = self.backend.active_application();
        let snapshot = reader.read(&application, self.focus_or_zero())?;
        if snapshot.sensitive || snapshot.selection || snapshot.application != application {
            return None;
        }
        Self::_typed_in_field(&snapshot.bounded(), typed, " ").map(|around| around.0)
    }

    /// Judge an early-switched word, now complete, as the user typed it.
    ///
    /// The early switch answered on a prefix. At the boundary the word used to be judged in
    /// the layout the switch had put it in, where an unknown Russian word made Latin (a brand
    /// or slang word no lexicon holds) reads as gibberish both ways and the model keeps what
    /// is shown: the switch made at the sixth letter stayed for good, while the same model
    /// asked about the word as typed keeps it (field logs, 24.09.2026; the same with the
    /// pair of 0.37.0). The completed word is now asked the question the switch answered too
    /// soon - does what was typed need the other layout - in the field the shown word was
    /// judged in. A model that keeps the typed word takes the switch back; a convert confirms
    /// it; a model in doubt leaves the prefix model's choice standing. A switch that produced a
    /// word of the lexicon or the identifier index is not taken back: the prefix model was right
    /// about a term typed in the other layout that the context model, asked about the unknown
    /// typed reading alone, kept (an English word typed in the Russian layout after Russian prose).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn _settle_early_switch(
        &mut self, shown: DetectionDecision, strokes: &[Stroke], shown_group: i64, origin: i64, application: &str, trigger: CorrectionTrigger,
        boundary_text: &str,
    ) -> EngineResult<DetectionDecision> {
        if origin == shown_group || !self.in_models(origin) || shown.should_convert {
            return Ok(shown);
        }
        if shown.source_score.known || self.context_policy.shared_identifiers().is_some_and(|identifiers| identifiers.contains(&shown.original)) {
            return Ok(shown);
        }
        let result = self._context_result.clone();
        let baseline = self._last_baseline.clone();
        let Some(asked) = result.clone().filter(|result| result.prediction.is_some()) else {
            return Ok(shown);
        };
        let typed = _text_for_group(strokes, origin);
        let verdict = self._decide_word(&typed, &vec![(shown_group, shown.original.clone())], origin, application, trigger, WordOptions {
            boundary_text: boundary_text.to_owned(),
            field_override: asked.field.clone(),
            ..WordOptions::default()
        })?;
        let judged = self._context_result.clone();
        let prediction = judged.as_ref().and_then(|judged| judged.prediction.clone());
        let action = prediction.as_ref().map(|prediction| prediction.action.as_str());
        technical_event!(
            self.shared, "early_switch_settled", "verdict": if verdict.should_convert { "convert" } else { action.unwrap_or("baseline") },
            "word_characters": pystr::len(&typed), "origin_group": origin,
        );
        self._context_result = result;
        self._last_baseline = baseline;
        let Some(prediction) = prediction.filter(|_| !verdict.should_convert && action == Some("keep")) else {
            return Ok(shown);
        };
        // Taken back: the word is not offered for a revisit by the next word.
        self._last_baseline = None;
        Ok(DetectionDecision {
            should_convert: true,
            replacement: typed,
            target_group: origin,
            confidence: prediction.probability,
            reason: "ранняя смена отменена: целое слово верно в набранной раскладке".to_owned(),
            ..shown
        })
    }

    /// Every other layout when a "keep" rule of the user fits the word.
    pub(crate) fn _rejected_targets(&self, source_group: i64, original: &str) -> Vec<i64> {
        if !self.flag("detection.learning", true) || !self.learning.keeps(source_group, original) {
            return Vec::new();
        }
        self.other_groups(source_group)
    }

    pub(crate) fn _forced_target_group(&self, source_group: i64, word: &str) -> Option<i64> {
        if !self.flag("detection.learning", true) {
            return None;
        }
        self.learning.forced_target(source_group, word)
    }

    /// What the detector decides about a word before the context model is asked.
    ///
    /// `following` is what was typed right after the word, as each target layout prints it.
    pub(crate) fn _baseline_decision(
        &self, original: &str, alternatives: &Alternatives, source_group: i64, application: &str, trigger: CorrectionTrigger,
        following: Option<&Alternatives>,
    ) -> EngineResult<DetectionDecision> {
        let (context_words, context_group) = self._context_for(application);
        let context_aware = self.flag("detection.context_aware", true);
        let ignored_words = self.words_setting("exclusions.words");
        let forced_target = self._forced_target_group(source_group, original);
        let rejected_targets = self._rejected_targets(source_group, original);
        let no_words = Vec::new();
        Ok(automatic_word_decision(&self.detector, original, alternatives, source_group, &AutomaticOptions {
            decide: DetectorOptions {
                minimum_length: if forced_target.is_some() { 1 } else { self._minimum_word_length() },
                confidence_threshold: self._confidence_threshold(),
                ignored_words: &ignored_words,
                aggressive: self.flag("detection.aggressive", false),
                protect_code: self.flag("detection.protect_code", true),
                previous_words: if context_aware { &context_words } else { &no_words },
                context_group: if context_aware { context_group } else { None },
                forced_target_group: forced_target,
                rejected_targets: &rejected_targets,
                trigger,
                use_intent_model: self.flag("detection.intent_model_enabled", true),
            },
            // Context is kept per named application (_remember_context).
            context_tracked: context_aware && !pystr::strip(application).is_empty(),
            following: following.map_or(&no_words, |following| following),
        })?)
    }

    /// The detector's verdict on a word once a planned neighbour is known.
    ///
    /// Same settings as the ordinary word decision; the only context is the
    /// neighbour in its planned layout and that layout's language: the converted
    /// next word of a waiting word, which is what the corpus curriculum supplies
    /// for planned frames, or the other reading of the waiting word before it.
    pub(crate) fn _planned_baseline(
        &self, original: &str, alternatives: &Alternatives, source_group: i64, next_word: &str, next_group: i64,
    ) -> EngineResult<DetectionDecision> {
        let ignored_words = self.words_setting("exclusions.words");
        let rejected_targets = self._rejected_targets(source_group, original);
        let forced_target = self._forced_target_group(source_group, original);
        let previous = vec![(next_group, next_word.to_owned())];
        Ok(automatic_word_decision(&self.detector, original, alternatives, source_group, &AutomaticOptions {
            decide: DetectorOptions {
                minimum_length: if forced_target.is_some() { 1 } else { self._minimum_word_length() },
                confidence_threshold: self._confidence_threshold(),
                ignored_words: &ignored_words,
                aggressive: self.flag("detection.aggressive", false),
                protect_code: self.flag("detection.protect_code", true),
                previous_words: &previous,
                context_group: Some(next_group),
                forced_target_group: forced_target,
                rejected_targets: &rejected_targets,
                trigger: CorrectionTrigger::Space,
                use_intent_model: self.flag("detection.intent_model_enabled", true),
            },
            ..AutomaticOptions::default()
        })?)
    }

    #[allow(clippy::too_many_arguments)]
    fn _consult_context_model(
        &mut self, decision: DetectionDecision, original: &str, alternatives: &Alternatives, rejected_targets: &[i64], application: &str,
        trigger: CorrectionTrigger, options: WordOptions,
    ) -> EngineResult<DetectionDecision> {
        let Some((group, alternative)) = alternatives.iter().find(|(group, _)| !rejected_targets.contains(group)).cloned() else {
            return Ok(decision);
        };
        let WordOptions { literal_tail, boundary_text, field_override, inside, .. } = options;
        let mode = self.text_setting("detection.context_policy", "assist");
        let read_field = field_override.is_none() && self.flag("detection.context_read_field", false);
        // The keys typed after the word: the window may show them already (context_policy.shown_ahead).
        let result = {
            let ahead = || self._typed_after(original);
            self.context_policy.decide(&decision, &alternative, group, &self.detector, trigger.as_str(), &mode, &PolicyOptions {
                read_field,
                literal_tail: &literal_tail,
                boundary_text: &boundary_text,
                field_override,
                inside,
                typed_ahead: Some(&ahead),
                ..PolicyOptions::default()
            })?
        };
        let result = Arc::new(result);
        self._context_result = Some(result.clone());
        if result.fallback_reason == "field_changed"
            && let Some(field) = &result.field
        {
            // Where the typed word stands against the caret the field reported, as numbers only:
            // the shape tells a lagging caret from a field that really changed.
            let anchor = format!("{original}{literal_tail}");
            let text = format!("{}{}", field.before, field.after);
            let end = pystr::rfind(&text, &anchor, None, None);
            technical_event!(
                self.shared, "field_contradiction", "application": application, "trigger": trigger.as_str(),
                "before_characters": pystr::len(&field.before), "after_characters": pystr::len(&field.after),
                "typed_characters": pystr::len(&anchor),
                "typed_end_from_caret": (end >= 0).then(|| end + pystr::len(&anchor) as i64 - pystr::len(&field.before) as i64),
            );
        }
        if result.field.as_ref().is_some_and(|field| field.sensitive) {
            self.shared.set_sensitive_context_window(self.shared._focus_window());
            self.context_policy.stream.clear();
            self._contexts.clear();
            self.shared._update(Update {
                current_word: Some(String::new()),
                last_action: Some("Защищённое поле: обработка отключена".to_owned()),
                ..Update::default()
            });
        }
        if let Some(prediction) = &result.prediction {
            let action = prediction.action.as_str();
            self.shared._update(Update { context_action: Some(action.to_owned()), context_model: Some(prediction.model_version.clone()), ..Update::default() });
            let field = result.field.as_ref();
            technical_event!(
                self.shared, "context_decision", "action": action,
                "score": logged(prediction.probability), "model_version": prediction.model_version,
                "mode": mode,
                "applied": result.decision.should_convert, "baseline_convert": decision.should_convert,
                "baseline_reason": decision.reason,
                "model_supported": prediction.supported,
                "policy_applied": result.policy_applied,
                "decision_source": result.decision_source,
                "fallback_reason": result.fallback_reason,
                "final_action": if result.decision.should_convert { "convert" } else { "keep" },
                "field_read_requested": self.flag("detection.context_read_field", false),
                "field_reader_status": self._field_reader_status(),
                "field_reader_details": self._field_reader_details(),
                "context_source": field.map_or("unavailable", |field| field.source.as_str()),
                "before_characters": field.map_or(0, |field| pystr::len(&field.before)),
                "after_characters": field.map_or(0, |field| pystr::len(&field.after)),
                "withheld_after_characters": result.withheld_after_characters,
                // Context content and surrounding sentences are never logged.
                "field_role": field.map_or("unknown", |field| field.role.as_str()),
            );
            if action == "suggest" && !result.decision.should_convert && mode == "assist" {
                self.shared._update(Update { last_action: Some(format!("Возможно: {original} → {alternative} · Pause для замены")), ..Update::default() });
            }
        }
        Ok(result.decision.clone())
    }

    pub(crate) fn _context_for(&self, application: &str) -> (Alternatives, Option<i64>) {
        if pystr::strip(application).is_empty() {
            return (Vec::new(), None);
        }
        let key = pystr::casefold(application);
        match self._contexts.iter().find(|entry| entry.0 == key) {
            Some((_, context)) if self.monotonic() - context.updated_at <= ks_context::input_context::CONTEXT_TTL => (context.words.clone(), Some(context.group)),
            _ => (Vec::new(), None),
        }
    }

    pub fn _remember_context(&mut self, application: &str, group: i64, strokes: &[Stroke]) {
        if !self.in_models(group) || strokes.is_empty() || pystr::strip(application).is_empty() || self._application_excluded(application) {
            return;
        }
        let words = (0..self.models.len() as i64).map(|candidate| (candidate, _text_for_group(strokes, candidate))).collect();
        let key = pystr::casefold(application);
        self._contexts.retain(|entry| entry.0 != key);
        self._contexts.push((key, LanguageContext { group, words, updated_at: self.monotonic() }));
        while self._contexts.len() > MAX_REMEMBERED_APPLICATION_CONTEXTS {
            self._contexts.remove(0);
        }
    }
}
