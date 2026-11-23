//! A word ends: `commit_word` and the case fix made at its boundary.

use super::decide::WordOptions;
use super::logging::{technical_event, WordEvaluation};
use super::*;
use ks_lang::short_words::ISOLATED_SHORT_WORD_REASON;

fn strokes_of(parts: &[&[Stroke]]) -> Strokes {
    parts.iter().flat_map(|part| part.iter().cloned()).collect()
}

fn characters_of(strokes: &[&[Stroke]]) -> String {
    strokes.iter().flat_map(|part| part.iter()).map(|stroke| stroke.character.as_str()).collect()
}

impl EngineCore {
    /// The word stays as typed because the user picked its layout by hand: when the model would have converted it, the
    /// windows are told (`Shared::subscribe_kept_by_choice`). The model is asked only while someone listens.
    fn tell_kept_by_choice(
        &mut self, original: &str, alternatives: &Alternatives, group: i64, application: &str, trigger: ks_lang::intent_model::CorrectionTrigger,
    ) -> EngineResult<()> {
        let callbacks = self.shared.kept_by_choice_callbacks();
        if callbacks.is_empty() {
            return Ok(());
        }
        if self.decide_word(original, alternatives, group, application, trigger, WordOptions::default())?.should_convert {
            for callback in callbacks {
                callback();
            }
        }
        Ok(())
    }

    pub fn commit_word(&mut self, boundary: &Stroke) -> EngineResult<()> {
        if self.strokes.is_empty() {
            return Ok(());
        }
        // A word of a third layout: no language model can judge it, so it stays as typed.
        if !self.in_models(self.source_group) {
            self.clear_word(None, "layout_without_model");
            return Ok(());
        }
        if self.pending.as_ref().is_some_and(|pending| pending.mode != "early") {
            self.log_pending_dropped("next_word_committed");
            self.pending = None;
            self.learning_prompt_after = None;
        }
        let typed = self.strokes.clone();
        // Signs typed in front of the word's first letter: a quotation mark there is closed by one at its end.
        let opened: Strokes =
            self.symbol_strokes.iter().chain(typed.iter().take_while(|stroke| !pystr::isalpha(&stroke.character))).cloned().collect();
        let shown = self.mention_shown.take();
        let application = self.backend.active_application();
        let mention = self.mention_head(&application);
        let source_group = self.source_group;
        let mut head = self.literal_head(&typed, source_group);
        let (mut strokes, mut trailing, mut segmentation_certain, mut abstained) = self.completed_word(&typed[head..], source_group);
        let ahead = format!("{}{}", if boundary.deferred { "" } else { boundary.character.as_str() }, self.typed_ahead(&typed));
        if self.insertion_holds(&text_for_group(&typed, source_group), &ahead, &application) {
            // Typed into a word, every key is part of it: `,` between `те` and `е`
            // is the `б` of `тебе`, not punctuation in front of a word.
            head = 0;
            strokes.clone_from(&typed);
            trailing = Vec::new();
            segmentation_certain = true;
            abstained = false;
        }
        // The word typed where a change of layout was erased, if it is the word as first typed, is the user's rule from
        // now on: made before the word is decided, it keeps this word too.
        self.settle_erased_layout(&typed[..typed.len() - trailing.len()], &application);
        let mut signs: Strokes = Vec::new();
        if trailing.is_empty() && segmentation_certain && !self.insertion.as_ref().is_some_and(InsertionPoint::inside_word) {
            let count = self.replayed_signs(&strokes, source_group);
            if count != 0 {
                signs = strokes.split_off(strokes.len() - count);
            }
        }
        self.reset_pause_correction();
        let mut original = text_for_group(&strokes, source_group);
        let mut alternatives = self.alternatives_of(&strokes, source_group);
        let context = self.context_for(&application);
        // Pause on the last committed token still converts it whole: the
        // literal head only narrows what an automatic decision may replace.
        let first_target = alternatives.first().map_or(source_group, |entry| entry.0);
        let first_reading = alternatives.first().map_or_else(|| original.clone(), |entry| entry.1.clone());
        let mut plan: PlanRef = Arc::new(CorrectionPlan {
            automatic: false,
            trailing: trailing.clone(),
            ..CorrectionPlan::new(
                strokes_of(&[&typed[..head], &strokes, &signs]),
                Some(boundary.clone()),
                source_group,
                first_target,
                format!("{original}{}", text_for_group(&signs, source_group)),
                format!("{first_reading}{}", text_for_group(&signs, first_target)),
                0.0,
                &application,
            )
        });
        let whole_keys = strokes_of(&[&typed, std::slice::from_ref(boundary)]);
        if typed.iter().all(|stroke| Self::joins_parts(stroke)) && self.pause_reaches_past(&whole_keys) {
            // `цуиоы - `: a dash between words is no word of its own, and Pause still converts `цуиоы`.
            self.extend_last_committed(&whole_keys);
        } else {
            self.last_committed = Some(plan.clone());
            self.last_committed_stale = false;
        }
        self.symbol_strokes = Vec::new();
        let early_switch_origin = self.early_switch_origin;
        // The layout the user chose, as against an early switch the user undid: both keep the word's layout.
        let chosen_layout = self.flag("detection.respect_manual_layout", true)
            && self.manual_layout_group == Some(source_group)
            && !self.early_switch_undone;
        let manual_layout_selected = self.word_protected(source_group);
        // An explicit layout selection is the strongest available user intent.
        // It protects exactly one word even when an older learned rule exists.
        let manual_layout_protected = manual_layout_selected;
        let protection = manual_layout_selected.then(|| self.protection_details());
        if manual_layout_selected {
            self.manual_layout_group = None;
            self.manual_layout_word = false;
            self.early_switch_undone = false;
        }
        let enabled = self.flag("enabled", true);
        let trigger_enabled = self.boundary_enabled(boundary);
        let caret_unknown = self.caret_unknown();
        self.caret_moved = false;
        let mut excluded = self.application_excluded(&application);
        let trigger = Self::trigger_for_boundary(boundary);
        // The layout picked by hand protects the word it was picked for, up to a joining sign (protected_joined_part).
        let analyzable = enabled && trigger_enabled && segmentation_certain && !caret_unknown;
        let protected_part = if manual_layout_protected && analyzable && !excluded && head == 0 && trailing.is_empty() {
            let targets: Vec<i64> = alternatives.iter().map(|entry| entry.0).collect();
            self.protected_joined_part(&strokes, &signs, source_group, &targets, &application, trigger, boundary)?
        } else {
            None
        };
        let should_analyze = (analyzable && !manual_layout_protected) || protected_part.is_some();
        let mut decision: Option<DetectionDecision> = None;
        let waiting = self.context_waiting.take();
        let provisional = self.provisional.take();
        let paused = self.paused_conversion.take();
        let mut kept = std::mem::take(&mut self.kept);
        if should_analyze && !excluded {
            let inside = if protected_part.is_some() || !trailing.is_empty() || head != 0 || !signs.is_empty() {
                None
            } else {
                self.decide_inside_word(&strokes, source_group, &alternatives, &application, trigger, Some(boundary))?
            };
            let mut decided = if let Some((_, part)) = protected_part.clone() {
                part
            } else if let Some(inside) = inside.clone() {
                inside
            } else {
                let closing = strokes_of(&[&trailing, &signs, std::slice::from_ref(boundary)]);
                let following = alternatives.iter().map(|(group, _)| (*group, text_for_group(&closing, *group))).collect();
                self.decide_word(&original, &alternatives, source_group, &application, trigger, WordOptions {
                    literal_tail: characters_of(&[&trailing, &signs]),
                    boundary_text: boundary.character.clone(),
                    following: Some(following),
                    ..WordOptions::default()
                })?
            };
            let stem = strokes.clone();
            let stem_signs = signs.clone();
            if protected_part.is_none() && !signs.is_empty() && !decided.should_convert {
                // The tail is split off only for a word the model converts without it (replayed_signs);
                // otherwise the token is judged whole, as before: `'nb/` is `эти.` at p=0.9998 and
                // `'nb` alone only at 0.978, while `ntrcn` is `текст` at 0.99999 and `ntrcn/` at 0.59.
                strokes.append(&mut signs);
                original = text_for_group(&strokes, source_group);
                alternatives = alternatives.iter().map(|(group, _)| (*group, text_for_group(&strokes, *group))).collect();
                let closing = strokes_of(&[&trailing, std::slice::from_ref(boundary)]);
                let following = alternatives.iter().map(|(group, _)| (*group, text_for_group(&closing, *group))).collect();
                decided = self.decide_word(&original, &alternatives, source_group, &application, trigger, WordOptions {
                    literal_tail: characters_of(&[&trailing]),
                    boundary_text: boundary.character.clone(),
                    following: Some(following),
                    ..WordOptions::default()
                })?;
            }
            if protected_part.is_none()
                && inside.is_none()
                && trailing.is_empty()
                && head == 0
                && signs.is_empty()
                && let Some(origin) = early_switch_origin
            {
                decided = self.settle_early_switch(decided, &strokes, source_group, origin, &application, trigger, &boundary.character)?;
            }
            // The layout an early switch left while the token was typed, when the part after its sign converts back to it.
            let mut begun_in: Option<i64> = None;
            if inside.is_none() && head == 0 && trailing.is_empty() {
                let targets: Vec<i64> = alternatives.iter().map(|entry| entry.0).collect();
                let joined = match protected_part.clone() {
                    Some(found) => Some(found),
                    None => self.decide_joined_part(&stem, &stem_signs, source_group, &targets, &application, trigger, Some(boundary), &decided)?,
                };
                if let Some((split, part)) = joined {
                    // `mongo-cthdbcf` converts after its hyphen: `mongo-сервиса`.
                    head = split;
                    decided = part;
                    strokes = stem[head..].to_vec();
                    signs = stem_signs;
                    original = text_for_group(&strokes, source_group);
                    alternatives = alternatives.iter().map(|(group, _)| (*group, text_for_group(&strokes, *group))).collect();
                    begun_in = early_switch_origin.filter(|origin| *origin != source_group && decided.target_group == *origin);
                }
            }
            excluded = self.application_excluded(&application);
            // The early switch converted this word while it was typed, and the switch stands:
            // for the words before it, it is a word converted from the switch's origin.
            let settled = trailing.is_empty() && head == 0 && signs.is_empty();
            let switched = inside.is_none()
                && early_switch_origin.is_some_and(|origin| origin != source_group)
                && settled
                && !decided.should_convert;
            let neighbour = match early_switch_origin {
                Some(origin) if switched => DetectionDecision {
                    should_convert: true,
                    source_group: origin,
                    target_group: source_group,
                    replacement: original.clone(),
                    ..decided.clone()
                },
                _ => decided.clone(),
            };
            let mut joint = if settled {
                self.resolve_context_wait(waiting.as_ref(), &strokes, Some(boundary), &neighbour, &application, &alternatives, switched)?
            } else {
                None
            };
            let mut alone: Option<PlanRef> = None;
            let mut head_alone = false;
            let unsplit = head == 0 && trailing.is_empty() && signs.is_empty() && !switched;
            if let (Some(found), Some(waited)) = (&joint, &waiting) {
                let taken = self.take_along(&kept, &found.0, pystr::len(&waited.plan.original), &application, switched)?;
                joint = Some((taken, found.1.clone()));
            } else {
                let mut matched = false;
                if let Some(waited) = waiting.as_ref().filter(|_| unsplit && !decided.should_convert) {
                    alone = self.decide_with_kept_neighbour(waited, &strokes, boundary, &decided, &application)?;
                    // The waiting word converts on its own; its kept neighbour stays as it was typed.
                    matched = alone.is_some();
                }
                if !matched && waiting.is_none() && unsplit && !decided.should_convert {
                    alone = self.decide_letter_before_sign(&kept, &strokes, boundary, &decided, &application)?;
                    if alone.is_some() {
                        // The kept letter converts with the sign after it; nothing before it is taken along any more.
                        kept = Vec::new();
                        matched = true;
                    }
                }
                // A token a layout picked by hand protects up to its sign has a head (`protected_joined_part`): not unsplit.
                // A token the model converted whole may be a term typed in the wrong layout before a word typed right
                // (`Цш-Аш-роутер`), unless an early switch converted a part of it while it was typed.
                if !matched && waiting.is_none() && unsplit && inside.is_none() && (!decided.should_convert || early_switch_origin.is_none()) {
                    alone = self.decide_joined_head(&strokes, boundary, source_group, alternatives[0].0, &application, decided.should_convert)?;
                    if alone.is_some() {
                        // Only the part before the sign converts (`pro-тарифу`); the words before it go along no more.
                        kept = Vec::new();
                        matched = true;
                        head_alone = true;
                    }
                }
                if !matched
                    && let Some(waited) = &waiting
                    && head == 0
                    && (!trailing.is_empty() || !signs.is_empty() || !decided.should_convert)
                {
                    // A waiting word whose neighbour did not convert either may still be settled by a
                    // later word (`тщ ш`, then `огые`); one the model declined with a converted
                    // neighbour has had its answer. A neighbour with a sign split off its end ends
                    // the wait, and the waiting word is asked with it as a kept word (`tot ghbdtn,`),
                    // as is one whose sign the model converted without (`Ша тщеб` is `If not,`: the
                    // waiting word was dropped, and the English sentence kept `Ша`).
                    kept.push(Arc::new(KeptWord::of_waiting(waited)));
                }
            }
            // A token whose head converts alone instead of the whole is no word gone the other way, nor one to take the
            // words before it along.
            if joint.is_none() && settled && !head_alone {
                joint = self.revert_with_next_word(provisional.as_ref(), &strokes, Some(boundary), &decided, &application)?;
            }
            if joint.is_none() && head == 0 && decided.should_convert && !head_alone {
                let base = self.plan_from_decision(&strokes, Some(boundary), &application, &decided, "boundary");
                let base = base.replaced_with(|plan| plan.trailing.clone_from(&trailing));
                let base = self.with_signs(&base, &signs);
                let taken = self.take_along(&kept, &base, pystr::len(&original), &application, false)?;
                if taken.mode == "context_phrase" {
                    joint = Some((taken, decided.clone()));
                }
            } else if let Some(origin) = early_switch_origin.filter(|_| joint.is_none() && switched) {
                let base = Arc::new(CorrectionPlan {
                    typed: text_for_group(&strokes, origin),
                    ..CorrectionPlan::new(strokes.clone(), Some(boundary.clone()), origin, source_group, original.clone(), original.clone(), decided.confidence, &application)
                });
                let taken = self.take_along(&kept, &base, pystr::len(&original), &application, true)?;
                if taken.mode == "context_phrase" {
                    joint = Some((taken, neighbour.clone()));
                }
            }
            if !trailing.is_empty() || head != 0 {
                self.log_context_wait("context_wait_cancelled", waiting.as_ref(), if trailing.is_empty() { "literal_head" } else { "literal_tail" }, &[]);
            }
            // A typo is corrected only in a plain word typed as one (typo_plan). A word the early switch converted is such a
            // word: its keys are all still here, and most words typed in the wrong layout go that way.
            let plain = !(head != 0
                || !trailing.is_empty()
                || !signs.is_empty()
                || !mention.is_empty()
                || shown.is_some()
                || inside.is_some()
                || self.insertion.as_ref().is_some_and(InsertionPoint::inside_word));
            if let Some((plan, found)) = joint {
                self.pending = Some(Self::replay_sign_with_word(&plan, &opened));
                self.learning_prompt_after = None;
                self.pending_trigger_keycode = i64::from(boundary.keycode);
                decided = DetectionDecision { should_convert: true, ..found };
            } else if decided.should_convert && !head_alone {
                let whole = begun_in.and_then(|origin| self.whole_token_from(&stem, &signs, head, boundary, &opened, origin, &decided, &application));
                let base = whole.clone().unwrap_or_else(|| self.plan_from_decision(&strokes, Some(boundary), &application, &decided, "boundary"));
                let quotation_closed = mention.first().is_some_and(|first| Self::quotation_closed(first, &trailing, Some(boundary)));
                let base = base.replaced_with(|plan| {
                    plan.trailing.clone_from(&trailing);
                    plan.head = if !mention.is_empty() && quotation_closed { Vec::new() } else { mention.clone() };
                });
                // The whole token's correction holds its signs, typed again in the layout it was begun in.
                let mut converted = if whole.is_some() { base } else { Self::replay_sign_with_word(&self.with_signs(&base, &signs), &opened) };
                if boundary.deferred {
                    // Enter/Tab has not reached the editor. Do not delete it
                    // as a character or include it in the text replacement.
                    converted = converted.replaced_with(|plan| {
                        plan.boundary = None;
                        plan.mode = "before_action";
                    });
                }
                // A boundary typed before the early switch's key was released
                // takes over the same word: say so instead of losing the plan.
                self.log_pending_dropped("superseded_by_boundary");
                self.pending = Some(converted.clone());
                self.learning_prompt_after = None;
                self.pending_trigger_keycode = i64::from(boundary.keycode);
                // A word converted at a space stays open for the next word: if that
                // one goes back the other way, the model is asked about this one again.
                if inside.is_none()
                    && trailing.is_empty()
                    && head == 0
                    && signs.is_empty()
                    && boundary.character == " "
                    && !boundary.deferred
                    && let Some(baseline) = self.last_baseline.clone()
                {
                    let field = self.context_result.as_ref().and_then(|result| result.field.clone());
                    let pending = self.pending.clone().expect("just set");
                    self.pending = Some(pending.replaced_with(|plan| plan.revisit = Some(Arc::new((baseline, field)))));
                }
                // A lone letter converts at once, but a user who keeps typing makes
                // that correction abort as unsafe; the next word then decides it,
                // exactly as it did while the letter waited for context. A short
                // word the model converts at the start of a message is the same
                // case: `z ctujlyz` typed in one go must still end as `я сегодня`.
                let mut waits = false;
                if decided.reason == ISOLATED_SHORT_WORD_REASON || self.opening_conversion(inside.as_ref(), &original) {
                    waits = self.start_context_wait(&converted, &decided, boundary, &trailing, head, true);
                }
                if plain && !waits && !self.pending.as_ref().expect("just set").sign_replayed {
                    let pending = self.pending.clone().expect("just set");
                    if let Some(typo) = self.typo_plan(&pending, &pending.strokes)? {
                        // Converted and misspelt: the corrected keys go in the same injection (`yhjvfkmyj` is `нормально`).
                        self.pending = Some(typo);
                    }
                }
                if !waits {
                    // Converted with Shift held one letter too long: the case is fixed in the same injection (`VFibyf`
                    // is `Машина`). A word that waits for its neighbour may yet be taken back with it, as typed.
                    let pending = self.pending.clone().expect("just set");
                    self.pending = Some(self.fix_converted_capitals(&pending));
                }
            } else {
                if let Some(alone) = alone.clone() {
                    self.log_pending_dropped("superseded_by_boundary");
                    self.pending = Some(alone);
                    self.learning_prompt_after = None;
                    self.pending_trigger_keycode = i64::from(boundary.keycode);
                }
                let remembered = strokes_of(&[&typed[..head], &strokes, &signs]);
                self.remember_context(&application, source_group, &remembered);
                // A token whose head converted alone is settled: no next word takes it back or along as it was typed, which
                // its keys no longer show.
                let waits = !head_alone && self.start_context_wait(&plan, &decided, boundary, &trailing, head, false);
                let typo = if waits || alone.is_some() || !plain {
                    None
                } else {
                    let base = Arc::new(CorrectionPlan {
                        context_field: self.context_field_id(),
                        ..CorrectionPlan::new(strokes.clone(), (!boundary.deferred).then(|| boundary.clone()), source_group, source_group, original.clone(), original.clone(), 0.0, &application)
                    });
                    self.typo_plan(&base, &strokes)?
                };
                let has_typo = typo.is_some();
                if let Some(typo) = typo {
                    // Kept as typed and misspelt: the word is settled in its own language, so no later word takes
                    // it along or back (it is no kept word and no provisional one).
                    self.pending = Some(typo);
                    self.learning_prompt_after = None;
                    self.pending_trigger_keycode = i64::from(boundary.keycode);
                }
                // The word stays in its layout for good: only now may its case be fixed. A word going with another
                // correction may yet be converted, and a conversion types the keys as they were typed.
                let stays = alone.is_none() && self.pending.is_none() && early_switch_origin.is_none() && mention.is_empty() && shown.is_none();
                if inside.is_none() && trailing.is_empty() && head == 0 {
                    let field = self.context_result.as_ref().and_then(|result| result.field.clone());
                    if !waits && stays {
                        // A word still waiting for its neighbour may yet be converted with it.
                        plan = self.fix_double_capitals(&plan, boundary);
                    }
                    // A word waiting for its neighbour joins the kept words only if the wait fails.
                    let word = if waits || has_typo || head_alone {
                        None
                    } else {
                        Some(Arc::new(KeptWord {
                            plan: plan.clone(),
                            baseline: self.last_baseline.clone(),
                            field,
                            window: self.focus_or_zero(),
                            deadline: self.monotonic() + ks_context::input_context::CONTEXT_TTL,
                            segmentation_open: false,
                            after_caret_move: false,
                        }))
                    };
                    self.keep_for_next_word(kept, word, boundary);
                    if !waits && alone.is_none() && !has_typo {
                        self.provisional = self.provisional_after_pause(paused.as_ref(), &strokes, source_group, boundary);
                    }
                } else if inside.is_none() && !trailing.is_empty() && head == 0 && self.sign_reads_as_letter(&plan) {
                    // The boundary model split a sign off the end of the word and the word
                    // stayed, but only the whole token reads as a word: `t\`` is `её`, and
                    // only the next word can tell (`мать`). The model is asked about the
                    // whole token then.
                    let whole = strokes_of(&[&strokes, &trailing]);
                    let word = plan.replaced_with(|plan| {
                        plan.strokes = strokes_of(&[&typed[..head], &whole]);
                        plan.original = text_for_group(&whole, source_group);
                        plan.replacement = text_for_group(&whole, plan.target_group);
                        plan.trailing = Vec::new();
                    });
                    let word = Arc::new(KeptWord {
                        plan: word,
                        baseline: None,
                        field: None,
                        window: self.focus_or_zero(),
                        deadline: self.monotonic() + ks_context::input_context::CONTEXT_TTL,
                        segmentation_open: true,
                        after_caret_move: false,
                    });
                    self.keep_for_next_word(kept, Some(word), boundary);
                } else if !trailing.is_empty() && stays {
                    // The word stays for good with the signs split off its end (`NOt.`): its case is fixed, and the signs
                    // are typed again as they were.
                    self.fix_double_capitals(&plan, boundary);
                }
            }
            decision = Some(decided);
        } else {
            // The word of a layout picked by hand is not analysed, but it is the context a waiting word waited for.
            // The model is asked with the word alone; the token is typed again whole, a sign after it included (`ожидал,`).
            let token = strokes_of(&[&strokes, &signs, &trailing]);
            let picked = match waiting.as_ref().filter(|_| manual_layout_protected) {
                Some(waited) => self.decide_before_picked_word(waited, &token, boundary, &original, source_group, &application, analyzable)?,
                None if manual_layout_protected => self.take_along_before_picked_word(&kept, &token, boundary, source_group, &application, analyzable)?,
                None => None,
            };
            if let Some(found) = picked {
                self.log_pending_dropped("superseded_by_boundary");
                self.pending = Some(found);
                self.learning_prompt_after = None;
                self.pending_trigger_keycode = i64::from(boundary.keycode);
            } else {
                self.log_context_wait("context_wait_cancelled", waiting.as_ref(), "analysis_skipped", &[]);
            }
            let remembered = strokes_of(&[&typed[..head], &strokes]);
            self.remember_context(&application, source_group, &remembered);
            if chosen_layout
                && enabled
                && trigger_enabled
                && segmentation_certain
                && !caret_unknown
                && !excluded
                && self.pending.is_none()
                && early_switch_origin.is_none()
                && !(head != 0 || !trailing.is_empty() || !signs.is_empty() || !mention.is_empty())
                && shown.is_none()
                && !self.insertion.as_ref().is_some_and(InsertionPoint::inside_word)
            {
                self.tell_kept_by_choice(&original, &alternatives, source_group, &application, trigger)?;
                // The layout the user chose keeps the word in it, but a slip in the word and Shift held one letter
                // too long belong to no layout: they are fixed as in a word the model kept (`нромально`, `ПРивет`
                // typed right after switching to the Russian layout).
                let base = Arc::new(CorrectionPlan {
                    context_field: self.context_field_id(),
                    ..CorrectionPlan::new(strokes.clone(), (!boundary.deferred).then(|| boundary.clone()), source_group, source_group, original.clone(), original.clone(), 0.0, &application)
                });
                if let Some(typo) = self.typo_plan(&base, &strokes)? {
                    self.pending = Some(typo);
                    self.learning_prompt_after = None;
                    self.pending_trigger_keycode = i64::from(boundary.keycode);
                } else {
                    self.fix_double_capitals(&plan, boundary);
                }
            }
            // A token the boundary model could not split was not decided at all;
            // the next word may still show what it was (`dc\`` before `тот`).
            // A word that is on, not protected and not excluded came here unanalysed for one of two reasons (`analyzable`):
            // the boundary model could not split it, or the caret had moved.
            let undecided = enabled && trigger_enabled && !manual_layout_protected && !excluded && head == 0;
            if undecided && !caret_unknown {
                let word = Arc::new(KeptWord {
                    plan: plan.clone(),
                    baseline: None,
                    field: None,
                    window: self.focus_or_zero(),
                    deadline: self.monotonic() + ks_context::input_context::CONTEXT_TTL,
                    segmentation_open: abstained,
                    after_caret_move: false,
                });
                self.keep_for_next_word(kept, Some(word), boundary);
            } else if undecided && segmentation_certain && trailing.is_empty() && mention.is_empty() {
                // The caret had moved. A word waits for the word after it, a sign the other layout types as a letter
                // with it (`nfr/` is `так.`); a mention head is not shown where the caret is unknown
                // (`show_mention_head`), so there is none to put back.
                self.wait_after_caret_move(&plan, &application, trigger, boundary)?;
            }
        }
        self.log_word_evaluation(WordEvaluation {
            trigger: trigger.as_str(),
            original: &original,
            alternatives: &alternatives,
            application: &application,
            enabled,
            trigger_enabled,
            manual_layout_protected,
            caret_unknown,
            application_excluded: excluded,
            decision: decision.as_ref(),
            protection,
            source_group: Some(source_group),
            early_switch_origin,
            idle_ms: None,
            context: Some(context),
            literal_head: &text_for_group(&typed[..head], source_group),
        })?;
        if decision.as_ref().is_none_or(|decision| !decision.should_convert) {
            let word = strokes_of(&[&typed[..head], &strokes, &signs]);
            self.finish_early_switch(word, Some(boundary.clone()), &application, source_group, trailing.clone());
        } else {
            self.early_switch_origin = None;
            self.early_switch_at = None;
        }
        let pending_has_head = self.pending.as_ref().is_some_and(|pending| !pending.head.is_empty());
        if !mention.is_empty() && !pending_has_head {
            technical_event!(
                self.shared, "mention_head_kept",
                "reason": match &decision {
                    None => "word_not_analysed",
                    Some(decision) if decision.should_convert => "converted_without_head",
                    Some(_) => "word_kept",
                },
                "shown": shown.is_some(),
                "application": application,
                "application_excluded": excluded,
            );
        }
        if let Some(shown) = shown.as_ref().filter(|_| !pending_has_head) {
            // The word ended before the key after the "@" could write the quote
            // back (keys held over each other). A word that converts took the
            // head with it above; any other ending means it was not a mention.
            self.log_pending_dropped("mention_head_written_back");
            let word = &typed[..typed.len() - trailing.len()];
            self.pending = Some(self.mention_write_back(shown, word, &trailing, (!boundary.deferred).then(|| boundary.clone()), &application));
            self.learning_prompt_after = None;
            self.pending_trigger_keycode = i64::from(boundary.keycode);
        }
        self.settle_erased_typo(&strokes);
        self.end_corrected_word(&typed, trailing.len());
        self.strokes = Vec::new();
        self.insertion = None;
        self.source_group = -1;
        self.early_switch_undone = false;
        self.shared.update(Update {
            current_word: Some(String::new()),
            current_group: Some(i64::from(boundary.group)),
            last_action: manual_layout_protected.then(|| format!("Ручная раскладка сохранена: {original}")),
            ..Update::default()
        });
        Ok(())
    }

    /// Schedule the case fix of a word typed with two capitals at its start; return the word as it will stand.
    ///
    /// `ПРивет` and `HEllo` are Shift held down one letter too long. A word kept in its layout gets its second
    /// letter lowercase at its boundary when it has that shape (`double_capitals_shape`): at least
    /// `DOUBLE_CAPITALS_MIN_LETTERS` letters, two capitals, then lowercase letters only, maybe parts a hyphen joins in
    /// lowercase (`КТо-то`) and signs after it (`ТАк!`, `NOt.`, whose sign stays as typed). An abbreviation (`ПК`,
    /// `IT`, `URLs`), a word in capitals, a CamelCase name and a token with digits have other shapes and are never touched.
    ///
    /// The lexicon decides the rest (`double_capitals_fixed`). It holds words without their case, so a word it knows is
    /// known in lowercase, with a capital first and in capitals, and the shape typed is none of these; only a
    /// case-aware dictionary - Hunspell, where the system has one - can know a word so written (`HBase`,
    /// `XEmacs`), and such a word stays. The fixed word must be one the frequency lexicon of its layout
    /// holds, not one Hunspell alone accepts: that lexicon is the same on every system, and Hunspell knows
    /// `glib`, which would make `GLib` a `Glib`. Terms of this shape (`VMware`, `OAuth`, `JUnit`, `MBps`)
    /// are no words of the lexicon and stay as typed. A word of three letters is fixed only when it is one of the words
    /// the prose of its language is full of (`КАк`, `ANd`): the lexicon knows many abbreviations with an ending, units
    /// and plurals of abbreviations in lowercase (`ОСи`, `МВт`, `PCs`), and those stay. A name Russian text writes with
    /// two capitals on purpose stays too (`ВКонтакте`, whose lowercase the lexicon knows). So does a word the user
    /// excluded or one a rule of theirs names, and a word typed under Caps Lock, where its capitals were not Shift held too
    /// long.
    pub fn fix_double_capitals(&mut self, plan: &PlanRef, boundary: &Stroke) -> PlanRef {
        let group = plan.source_group;
        let Some((fixed, strokes)) = self.double_capitals_case(plan, group, &plan.original) else {
            return plan.clone();
        };
        let length = pystr::len(&plan.original);
        // Enter and Tab are held back until the fix is typed and are delivered after it, as after a conversion. Signs
        // the boundary model split off the word's end are deleted with it and typed again as they were.
        self.pending = Some(Arc::new(CorrectionPlan {
            mode: "case",
            replaced: plan.strokes.clone(),
            trailing: plan.trailing.clone(),
            ..CorrectionPlan::new(
                strokes.clone(), (!boundary.deferred).then(|| boundary.clone()), group, group, plan.original.clone(), fixed.clone(),
                UNSCORED_CORRECTION_CONFIDENCE, &plan.application,
            )
        }));
        self.learning_prompt_after = None;
        self.pending_trigger_keycode = i64::from(boundary.keycode);
        technical_event!(
            self.shared, "double_capitals_scheduled", "source_group": group, "letters": length, "boundary": boundary.key_name,
            "application": plan.application,
        );
        // The next word may take this one along into its own conversion (take_along): it takes the keys now on
        // screen, which the observed text shows once the fix is typed.
        let target = plan.target_group;
        plan.replaced_with(|plan| {
            plan.replacement = text_for_group(&strokes, target);
            plan.strokes = strokes;
            plan.original = fixed;
        })
    }

    /// The conversion `plan` with the case of its word fixed as well, when the word it types has two capitals at its
    /// start the way `fix_double_capitals` fixes them: `VFibyf` is `Машина`, not `МАшина`, and `ФТв` is `And`.
    ///
    /// The keys the conversion types are the keys as typed, Shift held one letter too long included; the fix lets go of
    /// that Shift in the same injection. The plan keeps the keys typed as the ones it replaces (`replaced`): undo types
    /// them back in the layout they were typed in, and the text is what it was (`VFibyf`). A conversion that types a
    /// mention head with its word, a typo correction that types keys of its own and the correction of a whole token that
    /// keeps its end in the layout it was begun in (`kept_tail`) stay as they are.
    pub fn fix_converted_capitals(&mut self, plan: &PlanRef) -> PlanRef {
        if !plan.head.is_empty() || !plan.replaced_strokes.is_empty() || plan.kept_tail != 0 {
            return plan.clone();
        }
        let Some((fixed, strokes)) = self.double_capitals_case(plan, plan.target_group, &plan.replacement) else {
            return plan.clone();
        };
        // Logged as the fix of a word kept in its layout is, with the layout of the word as it now stands.
        technical_event!(
            self.shared, "double_capitals_scheduled", "source_group": plan.target_group, "letters": pystr::len(&plan.replacement),
            "boundary": plan.boundary.as_ref().map(|boundary| boundary.key_name.as_str()), "application": plan.application,
        );
        plan.replaced_with(|plan| {
            plan.replaced = std::mem::replace(&mut plan.strokes, strokes);
            plan.replacement = fixed;
        })
    }

    /// `text`, what `plan` leaves on screen in `group`, with its case fixed, and the keys that type it; None when it stays
    /// as it is. A text of another shape stays silently; one the lexicon or the keys keep is logged with the reason.
    fn double_capitals_case(&mut self, plan: &CorrectionPlan, group: i64, text: &str) -> Option<(String, Strokes)> {
        let (word, after) = double_capitals_shape(text)?;
        if !(self.in_models(group) && self.flag("detection.fix_double_capitals", true) && pystr::len(text) == plan.strokes.len()) {
            return None;
        }
        let fixed = self.double_capitals_fixed(word, group);
        let lowered = self.unshifted(&plan.strokes[1..DOUBLE_CAPITALS], group);
        // The early switch or a pause converted the word while it was typed: its keys are of two layouts, and a
        // conversion's keys are typed back as they were.
        let reason = if plan.strokes.iter().any(|stroke| i64::from(stroke.group) != plan.source_group) {
            "converted_while_typed"
        } else if self.insertion.as_ref().is_some_and(InsertionPoint::inside_word) {
            // Letters typed into a word: the word is longer than the keys the engine holds.
            "inside_word"
        } else if self.words_setting("exclusions.words").iter().any(|item| LanguageDetector::token_key(item) == LanguageDetector::token_key(text)) {
            "user_exclusion"
        } else if self.forced_target_group(group, text).is_some() || !self.rejected_targets(group, text).is_empty() {
            "user_rule"
        } else if plan.strokes.iter().any(|stroke| stroke.caps_lock()) {
            "caps_lock"
        } else if let Err(reason) = fixed {
            reason
        } else if lowered.is_none() {
            "key_unknown"
        } else {
            ""
        };
        let (Ok(fixed), Some(lowered)) = (fixed, lowered.filter(|_| reason.is_empty())) else {
            technical_event!(
                self.shared, "double_capitals_kept", "reason": reason, "source_group": group, "letters": pystr::len(text),
                "application": plan.application,
            );
            return None;
        };
        Some((format!("{fixed}{after}"), strokes_of(&[&plan.strokes[..1], &lowered, &plan.strokes[DOUBLE_CAPITALS..]])))
    }

    /// `word`, two capitals and lowercase letters, with its second letter lowercase when the lexicon of `group` decides
    /// so (`fix_double_capitals`), or why it stays: Hunspell knows it as typed, the frequency lexicon does not hold the
    /// fixed word, the word is short and the prose of its language does not use it at least
    /// `DOUBLE_CAPITALS_SHORT_WORD_PROSE_MIN_COUNT` times, or Russian text writes it as typed on purpose.
    ///
    /// The lexicon holds words without their case, so it cannot tell a slip from a name written with two capitals: it
    /// knows `вконтакте`, and `ВКонтакте` would become `Вконтакте`. The table of such words
    /// (`DOUBLE_CAPITALS_FREQUENCY_FILE`, `ks-train capitals`) holds those the Leipzig word lists write so at least as
    /// often as with only their first letter a capital; it is asked last, so it is read on the first word that would be
    /// fixed and never before. A table that cannot be read keeps nothing.
    fn double_capitals_fixed(&self, word: &str, group: i64) -> Result<String, &'static str> {
        let model = self.model(group);
        let first_end = word.chars().next().map_or(0, char::len_utf8);
        let (first, rest) = word.split_at(first_end);
        let fixed = format!("{first}{}", pystr::lower(rest));
        if model.speller.check(word) {
            Err("known_as_typed")
        } else if !model.score(&fixed).exact {
            Err("fixed_word_unknown")
        } else if pystr::len(word) < DOUBLE_CAPITALS_LEXICON_MIN_LETTERS && self.prose_count(group, &fixed) < DOUBLE_CAPITALS_SHORT_WORD_PROSE_MIN_COUNT {
            Err("fixed_word_rare")
        } else if self.term_frequency().is_ok_and(|tables| tables.written_capitals().is_ok_and(|written| written.contains(word))) {
            Err("written_as_typed")
        } else {
            Ok(fixed)
        }
    }

    /// How often the prose of the language `group` types uses `word`; nought for a layout of no prose, or when the
    /// tables of the counts cannot be read.
    fn prose_count(&self, group: i64, word: &str) -> u64 {
        let tables = self.term_frequency();
        word_parts::prose_of(&self.layout_name(group))
            .zip(tables.ok())
            .map_or(0, |(prose, tables)| tables.term(prose).count(&pystr::casefold(word)))
    }

    /// Whether the letters of `text`, typed in `group`, are a word whose case `fix_double_capitals` fixes; their keys are
    /// not looked at.
    fn double_capitals_word(&self, text: &str, group: i64) -> bool {
        double_capitals_shape(text).is_some_and(|(word, _)| self.flag("detection.fix_double_capitals", true) && self.double_capitals_fixed(word, group).is_ok())
    }

    /// The token `strokes` typed in `source_group` as the code guard judges it before the boundary model is asked about
    /// its last `tail` keys, signs here and letters in the other layout (`completed_word`).
    ///
    /// A word with Shift held one letter too long is judged as it will stand once its case is fixed: its second capital
    /// alone looked like an identifier's to the guard, so `NOt.` and `HEllo.` were not decided at all, and the sign after
    /// them, a letter of the Russian layout, was never split off. The word may be fixed so in the layout it was typed in
    /// (`NOt` before its sign) or, converted, in the other one (`DCt[` is `ВСех`). A term of that shape stays guarded as
    /// typed (`PCs.`, `IList.Count`).
    pub fn guarded_text(&self, strokes: &[Stroke], tail: usize, source_group: i64) -> String {
        let original = text_for_group(strokes, source_group);
        let stem = text_for_group(&strokes[..strokes.len() - tail], source_group);
        if self.double_capitals_word(&stem, source_group)
            || self.other_groups(source_group).into_iter().any(|target| self.double_capitals_word(&text_for_group(strokes, target), target))
        {
            second_lowered(&original)
        } else {
            original
        }
    }

    /// `strokes`, capital letters of `group` typed with Shift, as their keys typed without it; None if unknown.
    ///
    /// Every backend types a stroke again from its key and its Shift, and the engine reads the stroke's text
    /// from `characters`, so both change. In the other layout a key is then the unshifted one of its pair:
    /// `Б` is `<` in the English layout, and `б` is `,`. The stroke's own text is the one of the layout it was typed in,
    /// which is not `group` when a conversion fixes the case of the word it types (`fix_converted_capitals`).
    fn unshifted(&self, strokes: &[Stroke], group: i64) -> Option<Strokes> {
        let pair = ks_text::layouts::LayoutPair::default();
        let mut result = Vec::with_capacity(strokes.len());
        for stroke in strokes {
            let names: Vec<String> = (0..stroke.characters.len()).map(|index| self.layout_name(index.to::<i64>())).collect();
            let letter = pystr::lower(stroke.character_for(group.to::<i32>()));
            if !stroke.shift() || pystr::len(&letter) != 1 || !names.iter().all(|name| name == "us" || name == "ru") {
                return None;
            }
            let source = names.get(group.to::<usize>())?;
            let mut characters = Vec::with_capacity(names.len());
            for name in &names {
                characters.push(pair.translate(&letter, source, name).ok()?);
            }
            result.push(key_of(KeyEvent {
                character: characters.get(usize::try_from(stroke.group).ok()?)?.clone(),
                characters,
                state: stroke.state & !ks_backend::constants::SHIFT_MASK,
                ..(**stroke).clone()
            }));
        }
        Some(result)
    }
}

/// The word a text opens with, and what follows it, when the text has the shape of Shift held one letter too long:
/// `DOUBLE_CAPITALS_MIN_LETTERS` letters or more, two capitals, then lowercase letters only (`ПРивет`, `КАк`); after the
/// word only parts a hyphen joins, in lowercase letters (`КТо-то`, `ГДе-нибудь`), then signs with no letter or digit
/// among them (`ТАк!`, `NOt.`). An abbreviation (`ПК`, `URLs`), a word in capitals, a CamelCase name (`IList.Count`) and
/// a token with digits have other shapes.
fn double_capitals_shape(text: &str) -> Option<(&str, &str)> {
    let letters = |text: &str| text.char_indices().find(|(_, character)| !ks_text::ucd::isalpha(*character)).map_or(text.len(), |(index, _)| index);
    let (word, after) = text.split_at(letters(text));
    let (capitals, rest) = word.split_at(word.char_indices().nth(DOUBLE_CAPITALS).map_or(word.len(), |(index, _)| index));
    if pystr::len(word) < DOUBLE_CAPITALS_MIN_LETTERS || !pystr::isupper(capitals) || !pystr::islower(rest) {
        return None;
    }
    let mut signs = after;
    while let Some(joined) = signs.strip_prefix('-') {
        let (part, next) = joined.split_at(letters(joined));
        if !pystr::islower(part) {
            return None;
        }
        signs = next;
    }
    (!signs.chars().any(|character| ks_text::ucd::isalpha(character) || ks_text::ucd::isdigit(character))).then_some((word, after))
}

/// `text` with its second character lowercase: the text a word of two capitals at its start shows once its case is fixed.
fn second_lowered(text: &str) -> String {
    let mut characters = text.chars();
    let first: String = characters.by_ref().take(1).collect();
    let second: String = characters.by_ref().take(1).collect();
    format!("{first}{}{}", pystr::lower(&second), characters.as_str())
}
