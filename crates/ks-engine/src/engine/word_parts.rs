//! Which keys of a token are the word: its literal head, the signs off its end, the parts a sign joins.

use super::decide::WordOptions;
use super::logging::technical_event;
use super::*;
use ks_context::context_action_features::digit_dot_question;
use ks_lang::boundary_model::MAX_SUFFIX;
use ks_lang::detector::DecideOptions as DetectorOptions;
use ks_lang::intent_model::CorrectionTrigger;
use ks_lang::short_words::is_short_word_override;

impl EngineCore {
    // Resolve a key that is punctuation here but a letter in another layout.
    //
    // If the word accumulated before this key is already recognisable, the
    // key is punctuation and can safely trigger a correction. Otherwise it is
    // retained as a physical stroke (for example `,fpf` -> `база`).
    pub(crate) fn _ambiguous_key_is_boundary(&mut self, event: &KeyEvent) -> EngineResult<bool> {
        if self.boundary_model.is_some() {
            // No completed-word classifier gets to cut an unfinished prefix.
            // Retain the key until a hard boundary or an idle evaluation.
            return Ok(false);
        }
        if self._strokes.is_empty() || self._source_group < 0 {
            return Ok(false);
        }
        if event.character == "'" || event.character == "-" {
            return Ok(false);
        }
        if self._strokes.iter().any(|stroke| !pystr::isalpha(&stroke.character) && Self::_is_layout_letter(stroke)) {
            return Ok(false);
        }
        let strokes = self._strokes.clone();
        let source_group = self._source_group;
        let original = _text_for_group(&strokes, source_group);
        let alternatives = self.alternatives_of(&strokes, source_group);
        let application = self.backend.active_application();
        let decision = self._decide_word(&original, &alternatives, source_group, &application, CorrectionTrigger::BoundaryProbe, WordOptions::default())?;
        let mut effective_length = pystr::len(&LanguageModel::normalize(&original));
        for (_, value) in &alternatives {
            effective_length = effective_length.max(pystr::len(&LanguageModel::normalize(value)));
        }
        let protected_boundary = self.flag("detection.protect_code", true) && self.detector.is_protected_token(&original);
        let key = LanguageDetector::token_key(&original);
        let ignored_boundary = self.words_setting("exclusions.words").iter().any(|word| LanguageDetector::token_key(word) == key);
        let natural_source_boundary = effective_length >= NATURAL_SOURCE_BOUNDARY_MIN_CHARACTERS
            && decision.source_score.ngram_score >= NATURAL_SOURCE_BOUNDARY_NGRAM_FLOOR;
        // A one- or two-letter word from the trusted list is recognisable, but
        // it is just as often the start of a longer word whose next letter
        // sits on this key: `j,ob[` is "общих", not "о" and a comma.
        let recognisable = decision.should_convert && !is_short_word_override(&decision);
        Ok(recognisable
            || protected_boundary
            || ignored_boundary
            || (decision.source_score.known && effective_length as i64 >= self._minimum_word_length())
            || natural_source_boundary)
    }

    /// The word's keys, the signs split off its end, whether its segmentation is certain, and
    /// whether the boundary model, asked, could not split it (it abstained).
    pub(crate) fn _completed_word(&mut self, strokes: &[Stroke], source_group: i64) -> (Strokes, Strokes, bool, bool) {
        let whole = || strokes.to_vec();
        let Some(model) = self.boundary_model.clone().filter(|_| self.in_models(source_group)) else {
            return (whole(), Vec::new(), true, false);
        };
        let mut tail = 0;
        for stroke in strokes.iter().rev() {
            if pystr::isalpha(&stroke.character) || !Self::_is_layout_letter(stroke) {
                break;
            }
            tail += 1;
        }
        if tail == 0 {
            return (whole(), Vec::new(), true, false);
        }
        let targets = self.other_groups(source_group);
        if tail >= strokes.len() || tail as i64 > MAX_SUFFIX || targets.is_empty() {
            technical_event!(
                self.shared, "boundary_guard", "model_version": model.version,
                "reason": if tail >= strokes.len() { "no_word" } else if tail as i64 > MAX_SUFFIX { "suffix_too_long" } else { "no_target" },
                "observed_characters": strokes.len(), "ambiguous_tail": tail,
            );
            return (whole(), Vec::new(), false, false);
        }
        let original = _text_for_group(strokes, source_group);
        if self._forced_target_group(source_group, &original).is_some() {
            return (whole(), Vec::new(), true, false); // Explicit full-token rule outranks segmentation.
        }
        // Segmentation cannot turn an excluded token/path into an eligible word.
        let folded = pystr::casefold(&original);
        let rejected = self._rejected_targets(source_group, &original);
        if (self.flag("detection.protect_code", true) && self.detector.is_protected_token(&original))
            || self.words_setting("exclusions.words").iter().any(|word| pystr::casefold(word) == folded)
            || targets.iter().all(|target| rejected.contains(target))
        {
            return (whole(), Vec::new(), false, false);
        }
        let target = targets[0];
        let alternative = _text_for_group(strokes, target);
        let candidates: Vec<_> = (0..=tail as i64)
            .map(|length| model.extract(&original, &alternative, length, self.model(source_group), self.model(target)))
            .collect();
        let prediction = model.predict(&candidates).expect("at least one candidate");
        let length = prediction.suffix_length;
        technical_event!(
            self.shared, "boundary_decision", "model_version": prediction.version,
            "action": match length { None => "abstain", Some(0) => "word", Some(_) => "literal" },
            "score": logged(prediction.probability), "preserved_characters": length,
            "candidates": tail + 1, "observed_characters": strokes.len(),
        );
        match length {
            None => (whole(), Vec::new(), false, true),
            Some(0) => (whole(), Vec::new(), true, false),
            Some(length) => (strokes[..strokes.len() - length].to_vec(), strokes[strokes.len() - length..].to_vec(), true, false),
        }
    }

    /// Count leading strokes up to the last ``/`` that stay literal.
    ///
    /// A slash is punctuation in both layouts, so ``bild/c,jhrb`` or a chat
    /// ``/c,jhrb`` would hide the wrong-layout ``сборки`` behind the code
    /// guard. Only the word after the last slash is analysed, and only when
    /// the contextual policy can weigh the surrounding text: the legacy
    /// detector alone keeps treating the whole token as code. The head is
    /// never replaced. A head that itself looks like a wrong-layout word
    /// abstains: one span cannot fix two words, and manual Pause still
    /// converts the whole token.
    pub(crate) fn _literal_head(&self, strokes: &[Stroke], source_group: i64) -> usize {
        if !self.in_models(source_group)
            || !self.flag("detection.protect_code", true)
            || !self.flag("detection.context_aware", true)
            || self.text_setting("detection.context_policy", "assist") != "assist"
            || self.context_policy.model.is_none()
        {
            return 0;
        }
        let last_slash = strokes.iter().rposition(|stroke| stroke.character == "/");
        let targets = self.other_groups(source_group);
        let head = last_slash.map_or(0, |index| index + 1);
        let core = &strokes[head..];
        if last_slash.is_none()
            || targets.is_empty()
            || core.is_empty()
            || !self.detector.is_protected_token(&_text_for_group(strokes, source_group))
            || self.detector.is_protected_token(&_text_for_group(core, source_group))
            || !core.iter().all(|stroke| {
                pystr::isalpha(&stroke.character) || Self::_is_layout_letter(stroke) || WORD_JOINERS.contains(&stroke.character.as_str())
            })
        {
            return 0;
        }
        let mut segment: Strokes = Vec::new();
        for stroke in &strokes[..head] {
            if stroke.character != "/" {
                segment.push(stroke.clone());
            } else if !segment.is_empty() && !self._head_segment_settled(&segment, source_group, targets[0]) {
                return 0;
            } else {
                segment = Vec::new();
            }
        }
        head
    }

    /// Count the strokes up to the last joining sign that stay as typed while the part after it is asked alone.
    ///
    /// A hyphen joins an English term and a Russian word in Russian prose (`mongo-сервиса`,
    /// `docker-образ`), and the layout the term was typed in often types the word after it:
    /// `mongo-cthdbcf`, `lid-ht;bvt`, `multidevice-htitybz[` stayed in field logs, since
    /// the token was judged whole and its whole other reading (`ьщтпщ-сервиса`) is no word
    /// either. Asked alone, behind the head as typed, the part after the hyphen is decided as
    /// any word (19 of 20 authored terms and Russian words, 13.10.2026). Any sign typed the same
    /// in both layouts joins parts the same way (_joins_parts); a slash has its own rule
    /// (_literal_head). A token the code guard protects stays whole, as before: an identifier, an
    /// assignment or an address (`user_name`, `foo=bar`, `user@host`) is not prose whose words were
    /// typed in two layouts. As after a slash, the head is never replaced, a head segment that itself
    /// looks like a word in the wrong layout abstains (one span cannot fix two words), and only
    /// the contextual model is asked.
    fn _joined_head(&self, strokes: &[Stroke], source_group: i64) -> usize {
        if !self.flag("detection.context_aware", true)
            || self.text_setting("detection.context_policy", "assist") != "assist"
            || self.context_policy.model.is_none()
        {
            return 0;
        }
        self._joined_split(strokes, source_group)
    }

    /// The strokes up to the last joining sign, when the token is parts joined by it and every part before
    /// it is settled as typed (_joined_head); 0 otherwise. Pause converts the part after them alone, whatever
    /// decides automatic corrections (_schedule_manual_conversion).
    pub(crate) fn _joined_split(&self, strokes: &[Stroke], source_group: i64) -> usize {
        let last_join = strokes.iter().rposition(|stroke| Self::_joins_parts(stroke));
        let targets = self.other_groups(source_group);
        let Some(last_join) = last_join else {
            return 0;
        };
        if !self.in_models(source_group) || targets.is_empty() || self.detector.is_protected_token(&_text_for_group(strokes, source_group)) {
            return 0;
        }
        let head = last_join + 1;
        let tail = &strokes[head..];
        if tail.len() < JOINED_PART_MIN_LETTERS || !tail.iter().all(|stroke| pystr::isalpha(&stroke.character) || Self::_is_layout_letter(stroke)) {
            return 0;
        }
        let mut segment: Strokes = Vec::new();
        let mut lettered = false;
        for stroke in &strokes[..head] {
            if !Self::_joins_parts(stroke) {
                segment.push(stroke.clone());
            } else if !segment.is_empty() {
                // Signs in a row (`C++-`) join as one; each run of letters must be settled as typed.
                if !self._head_segment_settled(&segment, source_group, targets[0]) {
                    return 0;
                }
                lettered = true;
                segment = Vec::new();
            }
        }
        if lettered { head } else { 0 }
    }

    /// The part after the last joining sign asked as a word of its own (_joined_head).
    ///
    /// Returns the head's length and the part's decision when the model converts the part and the
    /// head stays: a token kept whole (`mongo-cthdbcf`), or one converted whole although the model,
    /// asked about the head alone, keeps it (`docker-j,hfp` became `вщслук-образ`; `docker` alone is
    /// kept at p=1.0, while `BB` of `BB-fccbcntyn` is only a suggestion and the whole `ИИ-ассистент`
    /// stands). Signs split off the end (_replayed_signs) stay split off the part: `mongo-cthdbcf/` is
    /// `mongo-сервиса.`. Otherwise the token stays as the whole of it was judged, with that
    /// judgement's context result.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn _decide_joined_part(
        &mut self, strokes: &[Stroke], signs: &[Stroke], source_group: i64, targets: &[i64], application: &str, trigger: CorrectionTrigger,
        boundary: Option<&Stroke>, whole: &DetectionDecision,
    ) -> EngineResult<Option<(usize, DetectionDecision)>> {
        let split = self._joined_head(strokes, source_group);
        if split == 0 {
            return Ok(None);
        }
        let head = &strokes[..split - 1];
        let tail = &strokes[split..];
        let original = _text_for_group(tail, source_group);
        let alternatives: Alternatives = targets.iter().map(|group| (*group, _text_for_group(tail, *group))).collect();
        if trigger == CorrectionTrigger::Pause
            && signs.is_empty()
            && self._word_begun(&original, source_group, &alternatives[0].1, targets[0])?
        {
            // A pause in the middle of the part is a pause to think, as in the middle of a word.
            return Ok(None);
        }
        let result = self._context_result.clone();
        let baseline = self._last_baseline.clone();
        let closing: Strokes = signs.iter().chain(boundary).cloned().collect();
        let following: Alternatives = targets.iter().map(|group| (*group, _text_for_group(&closing, *group))).collect();
        let decision = self._decide_word(&original, &alternatives, source_group, application, trigger, WordOptions {
            literal_tail: signs.iter().map(|stroke| stroke.character.as_str()).collect(),
            boundary_text: boundary.map_or_else(String::new, |boundary| boundary.character.clone()),
            following: Some(following),
            ..WordOptions::default()
        })?;
        let parted = self._context_result.clone();
        let parted_baseline = self._last_baseline.clone();
        let mut head_action: Option<&'static str> = None;
        if decision.should_convert && whole.should_convert {
            let head_alternatives: Alternatives = targets.iter().map(|group| (*group, _text_for_group(head, *group))).collect();
            self._decide_word(&_text_for_group(head, source_group), &head_alternatives, source_group, application, CorrectionTrigger::Punctuation, WordOptions {
                boundary_text: strokes[split - 1].character.clone(),
                ..WordOptions::default()
            })?;
            head_action = Some(
                self._context_result.as_ref().and_then(|asked| asked.prediction.as_ref()).map_or("none", |prediction| prediction.action.as_str()),
            );
        }
        let taken = decision.should_convert && matches!(head_action, None | Some("keep"));
        technical_event!(
            self.shared, "joined_part_decision", "trigger": trigger.as_str(), "head_characters": split, "part_characters": tail.len(),
            "signs": signs.len(), "whole_converted": whole.should_convert, "part_converted": decision.should_convert,
            "head_action": head_action, "taken": taken,
        );
        if !taken {
            self._context_result = result;
            self._last_baseline = baseline;
            return Ok(None);
        }
        self._context_result = parted;
        self._last_baseline = parted_baseline;
        Ok(Some((split, decision)))
    }

    /// A head segment is settled when nothing suggests it needs conversion.
    fn _head_segment_settled(&self, segment: &[Stroke], source_group: i64, target: i64) -> bool {
        let original = _text_for_group(segment, source_group);
        if self.detector.is_protected_token(&original) {
            return true;
        }
        let alternative = _text_for_group(segment, target);
        if self.model(target).score(&alternative).known {
            return false;
        }
        let alternatives = [(target, alternative)];
        let decision = self.detector.decide(&original, &alternatives, source_group, &DetectorOptions {
            minimum_length: 1,
            confidence_threshold: self._confidence_threshold(),
            aggressive: self.flag("detection.aggressive", false),
            protect_code: true,
            use_intent_model: self.flag("detection.intent_model_enabled", true),
            ..DetectorOptions::default()
        });
        decision.is_ok_and(|decision| !decision.should_convert)
    }

    /// Keys at the end of a token that are signs in the other layout, when the token is no word.
    ///
    /// `hello,` typed in the Russian layout is `руддщб`: the comma is the key of `б`. And
    /// `текст.` typed in the English layout is `ntrcn/`: the period is the key of `/`, a
    /// sign in both layouts but another one. The word is judged without them and, if it
    /// converts, they are replayed with it; a token that is a word as typed (`хлеб`) keeps
    /// its letters, and a key that types the same sign in both layouts is no part of this.
    pub(crate) fn _replayed_signs(&self, strokes: &[Stroke], source_group: i64) -> usize {
        let targets = self.other_groups(source_group);
        let Some(&target) = targets.first() else {
            return 0;
        };
        let target = target as i32;
        let mut count = 0;
        for stroke in strokes.iter().rev() {
            let meant = stroke.character_for(target);
            if meant.is_empty() || pystr::isalpha(meant) || pystr::isspace(meant) || meant == stroke.character {
                break;
            }
            count += 1;
        }
        if count == 0
            || strokes.len() - count < REPLAYED_SIGNS_MIN_STEM_LETTERS
            || strokes[..strokes.len() - count]
                .iter()
                .any(|stroke| !(pystr::isalpha(&stroke.character) || pystr::isalpha(stroke.character_for(target)) || Self::_joins_parts(stroke)))
            || self.model(source_group).score(&_text_for_group(strokes, source_group)).known
        {
            // A token with digits or other signs inside (`з+1ю` for `p+1.`) is judged whole, as before,
            // and so is one whose rest is a single letter: `чё` is not `ч` with a backtick. A key that is
            // a letter in the other layout is a letter of the word: `Ct,z/` is `Себя.`; a sign typed the
            // same in both layouts joins its parts (_joins_parts): `mongo-cthdbcf/` is `mongo-сервиса.`.
            return 0;
        }
        count
    }

    pub(crate) fn _with_signs(&self, plan: &PlanRef, signs: &[Stroke]) -> PlanRef {
        if signs.is_empty() {
            return plan.clone();
        }
        plan.replaced_with(|plan| {
            plan.strokes.extend(signs.iter().cloned());
            plan.original.push_str(&_text_for_group(signs, plan.source_group));
            plan.replacement.push_str(&_text_for_group(signs, plan.target_group));
        })
    }

    /// Replay a sign typed right after a converted word in the new layout too.
    ///
    /// A person typing Russian in the English layout presses the keys of the Russian
    /// layout for signs as well: the comma is Shift+/, so `ghbdtn?` is `привет,`, and
    /// `ltkf&` is `дела?`; `support@mail.ru` typed in the Russian layout has `"` where
    /// `@` was meant. A key that is a sign in both layouts but a different one was
    /// pressed for the layout the word was meant in - unless it is a quotation mark and
    /// one opened the word (`opened`): then it closes the quotation, and `"john"` keeps
    /// both quotes.
    pub(crate) fn _replay_sign_with_word(plan: &PlanRef, opened: &[Stroke]) -> PlanRef {
        let Some(boundary) = plan.boundary.clone() else {
            return plan.clone();
        };
        if boundary.deferred
            || !plan.trailing.is_empty()
            || !plan.head.is_empty()
            || pystr::isspace(&boundary.character)
            || (boundary.character == "\"" && opened.iter().any(|stroke| stroke.character == "\""))
        {
            return plan.clone();
        }
        let meant = boundary.character_for(plan.target_group as i32);
        if meant.is_empty() || meant == boundary.character || pystr::isalpha(meant) || pystr::isspace(meant) {
            return plan.clone();
        }
        let meant = meant.to_owned();
        plan.replaced_with(|plan| {
            plan.strokes.push(boundary.clone());
            plan.boundary = None;
            plan.sign_replayed = true;
            plan.original.push_str(&boundary.character);
            plan.replacement.push_str(&meant);
        })
    }

    pub(crate) fn _plan_from_decision(
        &self, strokes: &[Stroke], boundary: Option<&Stroke>, application: &str, decision: &DetectionDecision, mode: &'static str,
    ) -> PlanRef {
        if mode == "boundary"
            && boundary.is_some_and(|boundary| pystr::isspace(&boundary.character) && !boundary.deferred)
            && digit_dot_question(&decision.original, &decision.replacement, "space")
        {
            // Digits with the Russian letter on the key of the English dot (`2ю` for `2.`): the person writes
            // Russian and pressed the English key for the sign, so the layout stays where it is (target_group is the
            // source one) and the keys are typed again in the other layout as literals, which the backends type in their
            // own layout before restoring the current one, as the "@" of a mention (_execute_correction). Retyped in the
            // English layout as a conversion, the next Russian word would come out in Latin letters.
            return Arc::new(CorrectionPlan {
                mode: "sign",
                context_field: self._context_field_id(),
                ..CorrectionPlan::new(
                    strokes.to_vec(), boundary.cloned(), decision.source_group, decision.source_group, decision.original.clone(),
                    decision.replacement.clone(), decision.confidence, application,
                )
            });
        }
        Arc::new(CorrectionPlan {
            mode,
            context_field: self._context_field_id(),
            ..CorrectionPlan::new(
                strokes.to_vec(), boundary.cloned(), decision.source_group, decision.target_group, decision.original.clone(),
                decision.replacement.clone(), decision.confidence, application,
            )
        })
    }

    pub(crate) fn _context_field_id(&self) -> String {
        self._context_result
            .as_ref()
            .and_then(|result| result.field.as_ref())
            .filter(|field| field.source != "observed")
            .map_or_else(String::new, |field| field.field_id.clone())
    }

    /// The start of a word still being typed: PAUSE_WORD_START_LETTERS letters that many words of
    /// their own language begin with, whose other reading is no word of the other language's prose.
    /// `иг` is the start of `игру` and `bu` no English word, while `ша` typed for `if` is.
    pub(crate) fn _word_begun(&self, original: &str, group: i64, alternative: &str, target: i64) -> EngineResult<bool> {
        if pystr::len(original) != PAUSE_WORD_START_LETTERS || !pystr::isalpha(original) || !self.in_models(group) {
            return Ok(false);
        }
        if ks_lang::short_words::word_starts(self.model(group)).count(&pystr::casefold(original)) < PAUSE_WORD_START_MIN_WORDS {
            return Ok(false);
        }
        let prose = match self._layout_name(target).as_str() {
            "us" => "english",
            "ru" => "russian",
            _ => return Ok(false),
        };
        Ok(self._term_frequency()?.term(prose).count(&pystr::casefold(alternative)) < CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS[0] as u64)
    }

    /// Whether Pause still converts the last word once `keys` follow it.
    ///
    /// Spaces and signs typed the same in every layout change nothing a conversion of the word would have to
    /// undo: they are deleted with it and typed again as they were. A user pressed Pause on `цуиоы` with a
    /// few such keys typed after it, got only the other layout, and erased and typed the word again (13.10.2026;
    /// 9 of the 16 such presses in the PC logs since 01.10 were followed by the word typed again by hand).
    /// Letters, Backspace, the arrow keys, a click, Enter and Tab end the reach as before.
    pub(crate) fn _pause_reaches_past(&self, keys: &[Stroke]) -> bool {
        let Some(plan) = &self._last_committed else {
            return false;
        };
        let Some(boundary) = &plan.boundary else {
            return false;
        };
        !self._last_committed_stale
            && !is_action_boundary_key(&boundary.key_name)
            && any_alpha(&plan.original)
            && plan.trailing.len() + 1 + keys.len() <= PAUSE_TRAILING_MAX_KEYS
            && keys.iter().all(|key| {
                !is_action_boundary_key(&key.key_name)
                    && ((!key.characters.is_empty() && key.characters.iter().all(|character| character == " ") && key.character == " ")
                        || Self::_joins_parts(key))
            })
    }

    /// Make `keys`, typed after the last word, part of what Pause deletes and types again (_pause_reaches_past).
    ///
    /// They are the same in every layout, so they take the group of the word's boundary: a correction types its
    /// literal keys in one layout.
    pub(crate) fn _extend_last_committed(&mut self, keys: &[Stroke]) {
        let plan = self._last_committed.clone().expect("a last committed word");
        let boundary = plan.boundary.clone().expect("with a boundary");
        let group = boundary.group;
        let extra: Strokes = keys.iter().map(|key| key_of(KeyEvent { group, ..(**key).clone() })).collect();
        self._last_committed = Some(plan.replaced_with(|plan| {
            plan.trailing.push(boundary);
            plan.trailing.extend(extra[..extra.len() - 1].iter().cloned());
            plan.boundary = extra.last().cloned();
        }));
    }

    /// The last word, if one Backspace puts the caret right after it.
    ///
    /// Only a word ended by one plain boundary character qualifies: nothing
    /// was typed or clicked since (it is not stale), no literal punctuation
    /// sits between the word and that boundary, and the boundary is not
    /// Enter or Tab, whose effect on the text one Backspace cannot undo.
    pub(crate) fn _reopenable_committed_word(&self) -> Option<PlanRef> {
        let plan = self._last_committed.as_ref()?;
        let boundary = plan.boundary.as_ref()?;
        if self._last_committed_stale
            || !self._strokes.is_empty()
            || !self._symbol_strokes.is_empty()
            || !plan.trailing.is_empty()
            || plan.strokes.is_empty()
            || is_action_boundary_key(&boundary.key_name)
            || pystr::len(&boundary.character) != 1
        {
            return None;
        }
        Some(plan.clone())
    }

    pub(crate) fn _layout_name(&self, group: i64) -> String {
        self.settings.with_value("detection.layouts", |names| {
            let default = ["us", "ru"];
            match names {
                Some(Value::Array(items)) => usize::try_from(group).ok().and_then(|index| items.get(index)).and_then(Value::as_str).unwrap_or("").to_owned(),
                _ => usize::try_from(group).ok().and_then(|index| default.get(index)).copied().unwrap_or("").to_owned(),
            }
        })
    }

    pub(crate) fn _layout_letters(&self, group: i64) -> HashSet<char> {
        let keys = match self._layout_name(group).as_str() {
            "us" => ks_text::layouts::US_KEYS,
            "ru" => ks_text::layouts::RU_KEYS,
            _ => "",
        };
        let mut letters = HashSet::new();
        for key in keys.chars().filter(|key| ks_text::ucd::isalpha(*key)) {
            letters.insert(key);
            letters.extend(pystr::upper(&key.to_string()).chars());
        }
        letters
    }

    pub(crate) fn _letters_before(text: &str) -> String {
        let mut start = text.len();
        for (index, character) in text.char_indices().rev() {
            if !ks_text::ucd::isalpha(character) {
                break;
            }
            start = index;
        }
        text[start..].to_owned()
    }

    pub(crate) fn _letters_after(text: &str) -> String {
        let end = text.char_indices().find(|(_, character)| !ks_text::ucd::isalpha(*character)).map_or(text.len(), |(index, _)| index);
        text[..end].to_owned()
    }

    pub(crate) fn _reads_alike(plan: &CorrectionPlan) -> bool {
        // A token of signs that the other layout types the same (`-`, `2`): nothing in it to convert.
        plan.original == plan.replacement && !any_alpha(&plan.original)
    }

    /// Only the whole token, the sign split off its end included, reads as a word in the other layout.
    ///
    /// `t\`` is `её`, while `t` alone, `е`, is no word. When the word without the
    /// sign is a word itself, the boundary model's split stands.
    pub(crate) fn _sign_reads_as_letter(&self, plan: &PlanRef) -> bool {
        let keys: Strokes = plan.strokes.iter().chain(plan.trailing.iter()).cloned().collect();
        let whole = _text_for_group(&keys, plan.target_group);
        self._known_reading(plan.target_group, &whole) && !self.model(plan.target_group).score(&plan.replacement).known
    }

    /// Whether a lexicon of the group holds the text, spelt with `ё` or with `е`: Russian writes `ё` as `е` as often
    /// as not, and a lexicon may hold one spelling alone. The shipped one holds `ее` and not `её`, and `T\`` opening a
    /// sentence stayed for `Её` (the development sentences of the Tatoeba dump, 22.10.2026).
    pub(crate) fn _known_reading(&self, group: i64, text: &str) -> bool {
        let model = self.model(group);
        model.score(text).known || model.score(&text.replace('ё', "е").replace('Ё', "Е")).known
    }

    /// Whether the text is a word of the group's language: the lexicon holds it (_known_reading), or prose of that
    /// language uses it at least KEPT_WORD_PROSE_MIN_COUNT times (the term tables): the shipped Russian lexicon holds no
    /// `ой`, `ох` or `эх`.
    pub(crate) fn _word_of_its_language(&self, group: i64, text: &str) -> EngineResult<bool> {
        let prose = if group == 1 { "russian" } else { "english" };
        Ok(self._known_reading(group, text) || self._term_frequency()?.term(prose).count(&pystr::casefold(text)) >= KEPT_WORD_PROSE_MIN_COUNT)
    }
}
