//! The automatic baseline shared by the event engine and action training.

use crate::constants::OPENING_LETTER_FOLLOWING_SIGNS;
use crate::detector::{DecideOptions, DetectionDecision, DetectorError, GroupTexts, LanguageDetector};
use crate::language_model::LanguageModel;
use crate::short_words::{natural_short_source_veto, trusted_short_word_decision, ISOLATED_SHORT_WORD_REASON, SINGLE_LETTER_CONFIDENCE, TRUSTED_SINGLE_LETTER_WORDS};
use ks_text::pystr;

/// The keyword arguments of `automatic_word_decision` beyond those of `LanguageDetector.decide`.
#[derive(Clone, Copy, Debug)]
pub struct AutomaticOptions<'a> {
    pub decide: DecideOptions<'a>,
    pub context_tracked: bool,
    pub following: GroupTexts<'a>,
}

impl Default for AutomaticOptions<'_> {
    fn default() -> Self {
        AutomaticOptions { decide: DecideOptions::default(), context_tracked: true, following: &[] }
    }
}

// Mirrors config.DEFAULTS["detection"]["minimum_length"/"confidence"]; engine.py imports this function, so this module
// cannot import its constants back from engine.py, which declares the same defaults locally.

/// The detector's verdict with the curated short-word rules applied.
///
/// `context_group` None reads as "no previous word" only while the caller keeps context at all. `context_tracked`
/// False - context awareness switched off, or an application without a name the engine could keep context for - means
/// the previous word is unknown, and the message-start exception for a lone letter does not apply: inside `plan b` the
/// `b` is not `и`. `following` is what was typed right after the word, as each target layout prints those keys (the
/// boundary and any signs before it); the message-start exception reads it.
pub fn automatic_word_decision(
    detector: &LanguageDetector,
    original: &str,
    alternatives: GroupTexts<'_>,
    source_group: i64,
    options: &AutomaticOptions<'_>,
) -> Result<DetectionDecision, DetectorError> {
    let decide = &options.decide;
    let mut decision = detector.decide(original, alternatives, source_group, decide)?;
    decision = natural_short_source_veto(decision, decide.context_group);
    decision = word_shape_veto(decision);
    if !decision.should_convert {
        if let Some(over) = trusted_short_word_decision(
            detector,
            original,
            alternatives,
            source_group,
            decide.ignored_words,
            decide.rejected_targets,
            decide.protect_code,
            decide.context_group,
        )? {
            return Ok(over);
        }
        let opening = if options.context_tracked {
            opening_letter_decision(detector, original, alternatives, source_group, decide.context_group, decide.ignored_words, decide.rejected_targets, decide.protect_code, options.following)?
        } else {
            None
        };
        if let Some(opening) = opening {
            return Ok(opening);
        }
    }
    Ok(decision)
}

/// A lone curated letter with no previous word converts; after an English word it stays.
///
/// With no previous word at all - the start of a message, a fresh field, or a pause the engine no longer bridges - the
/// evidence points the other way from the middle of a sentence. In the Russian training treebank (UD Taiga, 121 967
/// sentences) 14% of sentences open with one of the curated single-letter words (в 5405, и 3852, а 3239, я 2187, с
/// 821, у 821, к 457, о 301); in the English one (UD EWT train, 12 544 sentences) no sentence opens with a lone
/// f/b/c/d/r/e/j/z, while inside sentences those letters do occur (b 40, r 18, c 15, f 15, d 11, e 7, j 3). Measured
/// 16.09.2026 on the base corpus sources. The rule sits here, in the layer the engine and the context-action corpus
/// share, and not in the curated table that generates the frozen context-v1 training scenarios.
///
/// Those counts are of the letter as a word. The treebanks keep an initial with its period (`J.`), so they could not
/// show that English does open with a letter followed by a sign: an initial, a list or a name (`J. Smith said`,
/// `C, D and E`, `C# rocks`), which the rule turned into `О. Smith said`, `С, В and E` and `С№ rocks` (0.38-0.42). The
/// rule therefore reads the letter as a word only when the target layout prints a space, nothing or a clause sign
/// right after it (`OPENING_LETTER_FOLLOWING_SIGNS)`: `f?` is `а,` and `f& xnj` is `а? что`, while `J.` is `Ою` and `C#`
/// is `С№` in the Russian layout, no word a message opens with.
pub fn opening_letter_decision(
    detector: &LanguageDetector,
    original: &str,
    alternatives: GroupTexts<'_>,
    source_group: i64,
    context_group: Option<i64>,
    ignored_words: &[String],
    rejected_targets: &[i64],
    protect_code: bool,
    following: GroupTexts<'_>,
) -> Result<Option<DetectionDecision>, DetectorError> {
    if context_group.is_some() || pystr::len(&LanguageModel::normalize(original)) != 1 {
        return Ok(None);
    }
    let original_key = LanguageDetector::token_key(original);
    if ignored_words.iter().any(|word| LanguageDetector::token_key(word) == original_key) || (protect_code && detector.is_protected_token(original)) {
        return Ok(None);
    }
    let source_model = detector.model_for(source_group)?;
    for (target_group, replacement) in alternatives {
        let target_group = *target_group;
        let target_model = detector.model(target_group);
        let after = following.iter().find(|item| item.0 == target_group).map_or("", |item| pystr::slice(&item.1, None, Some(1)));
        let Some(target_model) = target_model else {
            continue;
        };
        if target_group == source_group
            || replacement == original
            || rejected_targets.contains(&target_group)
            || !TRUSTED_SINGLE_LETTER_WORDS.contains(&LanguageModel::normalize(replacement).as_str())
            || !(after.is_empty() || pystr::isspace(after) || OPENING_LETTER_FOLLOWING_SIGNS.contains(after))
        {
            continue;
        }
        return Ok(Some(DetectionDecision::new(
            true,
            original,
            replacement,
            source_group,
            target_group,
            SINGLE_LETTER_CONFIDENCE,
            ISOLATED_SHORT_WORD_REASON,
            source_model.score(original),
            target_model.score(replacement),
        )));
    }
    Ok(None)
}

/// The refusal the detector path and the context policy share for a replacement that is not a word; the log reader
/// must see one reason from both.
pub const NOT_A_WORD_REASON: &str = "другая раскладка пишет это не буквами";

/// A word may not be replaced by something that is not a word.
///
/// Six Russian letters sit on punctuation keys, so the other reading of a token like `рукх` is `her[`. The character
/// models score such a reading on its letters alone and can prefer it, which is how a mistyped Russian word turned
/// into a bracket. The engine is asked about a word, and the keys it would type instead must spell one too; nothing is
/// vetoed in the opposite direction, where `[jhjij` becomes the ordinary `хорошо`, nor for a source that already
/// carries a hyphen or an apostrophe.
#[must_use]
pub fn word_shape_veto(decision: DetectionDecision) -> DetectionDecision {
    if !decision.should_convert || !pystr::isalpha(&decision.original) {
        return decision;
    }
    if pystr::isalpha(&decision.replacement) {
        return decision;
    }
    DetectionDecision { should_convert: false, reason: NOT_A_WORD_REASON.to_string(), ..decision }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::language_model::WordScore;

    fn score() -> WordScore {
        WordScore { value: 0.0, known: false, frequency: 0, gram_ratio: 0.0, exact: false, spell_known: false, ngram_score: 0.0, invalid_ratio: 0.0, raw_ngram_score: 0.0 }
    }

    #[test]
    fn a_bracket_is_not_a_word() {
        let converting = DetectionDecision::new(true, "рукх", "her[", 1, 0, 3.0, "x", score(), score());
        let vetoed = word_shape_veto(converting.clone());
        assert!(!vetoed.should_convert && vetoed.reason == NOT_A_WORD_REASON);
        let kept = DetectionDecision::new(true, "[jhjij", "хорошо", 0, 1, 3.0, "x", score(), score());
        assert!(word_shape_veto(kept).should_convert);
    }
}
