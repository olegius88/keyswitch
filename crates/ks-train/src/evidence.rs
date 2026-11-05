//! The evidence of a frame, as the engine would give the model (`evidence`, `frame_evidence`, `frame_features`).
use crate::rows::{ActionRow, words_of, word_at_start};
use crate::physical::{translated, variant_choice};
use crate::tools::Tools;
use ks_context::context_action_features::extract_action_features;
use ks_context::context_model::{AfterOrigin, BeforeOrigin, ContextEvidence};
use ks_context::context_policy::{EvidenceOptions, evidence_for_decision};
use ks_lang::detector::{DecideOptions, LanguageDetector};
use ks_lang::identifier_lexicon::IdentifierLexicon;
use ks_lang::intent_model::CorrectionTrigger;
use ks_lang::ortho_model::OrthoModel;
use ks_lang::word_decision::{AutomaticOptions, automatic_word_decision};
use ks_lang::WordList;
use ks_text::pystr;
use std::sync::LazyLock;

const COMMAND_FAMILY_IDENTIFIER_PARTS: usize = 3;
const IDENTIFIER_DROPOUT_FAMILIES: usize = 3;
const TERM_COUNT_DROPOUT_MIN_LETTERS: usize = 5;
const TERM_COUNT_DROPOUT_FAMILIES: usize = 3;

static BLIND_IDENTIFIERS: LazyLock<IdentifierLexicon> =
    LazyLock::new(|| IdentifierLexicon { identifiers: WordList::new(), name: "none".to_owned(), version: "identifiers-none".to_owned() });

pub fn trigger_of(name: &str) -> CorrectionTrigger {
    ks_lang::intent_model::TRIGGERS.iter().copied().find(|trigger| trigger.as_str() == name).unwrap_or_else(|| panic!("no trigger {name}"))
}

/// `previous_context`: the last completed natural word of a frozen phrase frame, in both readings.
fn previous_context(row: &ActionRow) -> (Vec<(i64, String)>, Option<i64>) {
    let matches = words_of(&row.field.before);
    let Some(word) = matches.last() else { return (Vec::new(), None) };
    if pystr::strip(&row.field.application).is_empty() {
        return (Vec::new(), None);
    }
    let group = i64::from(word.chars().any(crate::rows::cyrillic_letter));
    match translated(word, group) {
        Ok(alternative) => (vec![(group, (*word).to_owned()), (1 - group, alternative)], Some(group)),
        // A clipped mixed-script or curly-apostrophe word is not a known pair.
        Err(_) => (Vec::new(), None),
    }
}

/// `following_reading`: what was typed right after the word, as the other layout prints those keys.
fn following_reading(row: &ActionRow) -> String {
    let joined = format!("{}{}", row.literal_tail, row.boundary_text);
    let typed: String = joined.chars().take(1).collect();
    translated(&typed, row.group).unwrap_or(typed)
}

/// `evidence`.
pub fn evidence(tools: &Tools, row: &ActionRow, detector: &LanguageDetector, ortho: Option<&OrthoModel>, identifiers: Option<&IdentifierLexicon>) -> ContextEvidence {
    let alternative = crate::physical::reading(&row.original, row.group);
    let (mut previous, mut context_group) = previous_context(row);
    if row.after_origin == AfterOrigin::PlannedNextConversion && !row.revisit {
        // The engine re-decides a waiting word with its converted next word as the only language context.
        let word = word_at_start(pystr::lstrip(&row.field.after)).expect("planned frame without a next word");
        previous = vec![(1 - row.group, word.to_owned())];
        context_group = Some(1 - row.group);
    }
    if row.before_origin == BeforeOrigin::PlannedPreviousReading {
        if let Some(group) = context_group {
            previous.retain(|entry| entry.0 == group);
        }
    }
    let alternatives = [(1 - row.group, alternative.clone())];
    let following = [(1 - row.group, following_reading(row))];
    let options = AutomaticOptions {
        decide: DecideOptions { previous_words: &previous, context_group, trigger: trigger_of(row.trigger), ..DecideOptions::default() },
        following: &following,
        ..AutomaticOptions::default()
    };
    let mut decision = automatic_word_decision(detector, &row.original, &alternatives, row.group, &options).expect("both lexicons are given");
    if row.revisit {
        // A converted word asked again keeps the verdict of its own boundary with no conversion.
        decision.should_convert = false;
    }
    evidence_for_decision(
        &decision,
        &alternative,
        1 - row.group,
        detector,
        &row.field,
        row.trigger,
        &EvidenceOptions {
            literal_tail: &row.literal_tail,
            boundary_text: &row.boundary_text,
            ortho,
            after_origin: row.after_origin,
            identifiers,
            inside: false,
            before_origin: row.before_origin,
        },
        tools.identifiers.as_deref(),
    )
    .expect("both lexicons are given")
}

fn identifier_family(identifier: &str) -> String {
    let parts: Vec<&str> = identifier.split(':').collect();
    if parts.len() >= COMMAND_FAMILY_IDENTIFIER_PARTS && parts[1] == "command" {
        return parts[..COMMAND_FAMILY_IDENTIFIER_PARTS].join(":");
    }
    identifier.to_owned()
}

fn identifier_evidence_dropped(identifier: &str) -> bool {
    variant_choice(&identifier_family(identifier), "identifier-dropout", IDENTIFIER_DROPOUT_FAMILIES) == 0
}

/// `term_counts_dropped`: one unknown-token family of enough letters in a few is trained with its own term counts dropped.
fn term_counts_dropped(identifier: &str, original: &str) -> bool {
    if !identifier.starts_with("unknown-") || crate::rows::alpha_only(original).chars().count() < TERM_COUNT_DROPOUT_MIN_LETTERS {
        return false;
    }
    let family = identifier.rsplit_once(':').map_or(identifier, |pair| pair.0);
    variant_choice(family, "count-dropout", TERM_COUNT_DROPOUT_FAMILIES) == 0
}

/// `frame_evidence`: TRAIN drops the identifier evidence of some families, and the term counts of some unknown tokens.
pub fn frame_evidence(tools: &Tools, row: &ActionRow, split: &str, detector: &LanguageDetector, ortho: Option<&OrthoModel>) -> ContextEvidence {
    let train = split == crate::corpus::TRAIN;
    let dropped = train && identifier_evidence_dropped(&row.identifier);
    let mut item = evidence(tools, row, detector, ortho, if dropped { Some(&*BLIND_IDENTIFIERS) } else { None });
    if train && term_counts_dropped(&row.identifier, &row.original) {
        item.term_counts_dropped = true;
    }
    item
}

/// The features of a frame, sorted by name, as the epochs read them.
pub fn sorted_features(tools: &Tools, item: &ContextEvidence) -> Vec<(String, f64)> {
    let features = extract_action_features(item, &tools.tables).expect("the features of a frame");
    let mut sorted: Vec<(String, f64)> = features.into_iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    sorted
}
