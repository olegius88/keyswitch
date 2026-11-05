//! The heads a model carries beside its word-at-a-boundary weights: their feature prefixes, their curricula, and which
//! frames ask a question of one (`HEAD_PREFIXES`, `OWN_EPOCH_HEADS`, `head_question`).
use crate::captured::captured_head_curriculum;
use crate::corpus::CorpusRow;
use crate::curricula::{self, Curriculum};
use crate::physical::reading;
use crate::rows::ActionRow;
use crate::tools::{Options, Tools};
use ks_context::constants::{CAPITALS_FEATURE_PREFIX, ALONE_FEATURE_PREFIX, START_FEATURE_PREFIX, LETTER_FEATURE_PREFIX, ABBREVIATION_FEATURE_PREFIX, LATIN_ABBREVIATION_FEATURE_PREFIX, UNKNOWN_FEATURE_PREFIX, JOINED_FEATURE_PREFIX, TERM_FEATURE_PREFIX, BRIEF_FEATURE_PREFIX, FIRST_FEATURE_PREFIX, VARIABLE_FEATURE_PREFIX, DOTTED_FEATURE_PREFIX, ALNUM_FEATURE_PREFIX, HYPOTHESIS_FEATURE_PREFIX, SHORT_FEATURE_PREFIX, SLIP_FEATURE_PREFIX, SIGN_FEATURE_PREFIX, RESUME_FEATURE_PREFIX, CYRLETTER_FEATURE_PREFIX, DASH_FEATURE_PREFIX, DIGIT_DOT_FEATURE_PREFIX, QUOTE_FEATURE_PREFIX, KNOWN_REPLY_FEATURE_PREFIX, KEPT_FEATURE_PREFIX};
use ks_context::context_action_features as f;
use ks_context::context_model::{AfterOrigin, BeforeOrigin, ContextEvidence};
use ks_text::pystr;
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// The heads a frozen base can carry besides the kept-neighbour question, by the recipe's name.
pub const HEAD_PREFIXES: [(&str, &str); 24] = [
    ("capitals", CAPITALS_FEATURE_PREFIX),
    ("alone", ALONE_FEATURE_PREFIX),
    ("start", START_FEATURE_PREFIX),
    ("letter", LETTER_FEATURE_PREFIX),
    ("abbreviation", ABBREVIATION_FEATURE_PREFIX),
    ("latin_abbreviation", LATIN_ABBREVIATION_FEATURE_PREFIX),
    ("unknown", UNKNOWN_FEATURE_PREFIX),
    ("joined", JOINED_FEATURE_PREFIX),
    ("term", TERM_FEATURE_PREFIX),
    ("brief", BRIEF_FEATURE_PREFIX),
    ("first", FIRST_FEATURE_PREFIX),
    ("variable", VARIABLE_FEATURE_PREFIX),
    ("dotted", DOTTED_FEATURE_PREFIX),
    ("alnum", ALNUM_FEATURE_PREFIX),
    ("hypothesis", HYPOTHESIS_FEATURE_PREFIX),
    ("short", SHORT_FEATURE_PREFIX),
    ("slip", SLIP_FEATURE_PREFIX),
    ("sign", SIGN_FEATURE_PREFIX),
    ("resume", RESUME_FEATURE_PREFIX),
    ("cyrletter", CYRLETTER_FEATURE_PREFIX),
    ("dash", DASH_FEATURE_PREFIX),
    ("digitdot", DIGIT_DOT_FEATURE_PREFIX),
    ("quote", QUOTE_FEATURE_PREFIX),
    ("knownreply", KNOWN_REPLY_FEATURE_PREFIX),
];

/// The heads whose own frames of DEVELOPMENT choose their epoch, by feature prefix: the name their epoch goes by in the
/// seal and the recipe's key of their curriculum.
pub const OWN_EPOCH_HEADS: [(&str, &str, &str); 17] = [
    (JOINED_FEATURE_PREFIX, "joined", "joined_token_curriculum"),
    (TERM_FEATURE_PREFIX, "term", "term_curriculum"),
    (BRIEF_FEATURE_PREFIX, "brief", "brief_curriculum"),
    (FIRST_FEATURE_PREFIX, "first", "first_curriculum"),
    (VARIABLE_FEATURE_PREFIX, "variable", "variable_curriculum"),
    (DOTTED_FEATURE_PREFIX, "dotted", "dotted_curriculum"),
    (ALNUM_FEATURE_PREFIX, "alnum", "alnum_curriculum"),
    (HYPOTHESIS_FEATURE_PREFIX, "hypothesis", "hypothesis_curriculum"),
    (SHORT_FEATURE_PREFIX, "short", "short_curriculum"),
    (SLIP_FEATURE_PREFIX, "slip", "slip_curriculum"),
    (SIGN_FEATURE_PREFIX, "sign", "sign_curriculum"),
    (RESUME_FEATURE_PREFIX, "resume", "resume_curriculum"),
    (CYRLETTER_FEATURE_PREFIX, "cyrletter", "cyrletter_curriculum"),
    (DASH_FEATURE_PREFIX, "dash", "dash_curriculum"),
    (DIGIT_DOT_FEATURE_PREFIX, "digitdot", "digitdot_curriculum"),
    (QUOTE_FEATURE_PREFIX, "quote", "quote_curriculum"),
    (KNOWN_REPLY_FEATURE_PREFIX, "knownreply", "brief_curriculum"),
];

/// A recipe without a curriculum's options frames nothing.
fn absent() -> Curriculum {
    (Vec::new(), json!({"frames": 0, "scope": "not used"}))
}

/// The curriculum of a head by its label; the recipe's options are under its key.
pub fn own_curriculum(tools: &Tools, label: &str, split: &str, source_rows: &[CorpusRow], recipe: &Value) -> Result<Curriculum, String> {
    let key = OWN_EPOCH_HEADS.iter().find(|head| head.1 == label).map_or(label, |head| head.2);
    let Some(options) = recipe.get(key) else { return Ok(absent()) };
    let t = &tools.tables;
    match label {
        "joined" => Ok(curricula::joined_token_curriculum(tools, split, source_rows, options)),
        "term" => Ok(curricula::term_curriculum(tools, split, source_rows, options)),
        "brief" => Ok(curricula::brief_curriculum(tools, split, options)),
        "first" => Ok(curricula::first_curriculum(tools, split, options)),
        "variable" => captured_head_curriculum(tools, split, options, &|item| f::variable_question(&item.original, &item.alternative, &item.field.before, &item.trigger), "variable"),
        "dotted" => captured_head_curriculum(tools, split, options, &|item| f::dotted_question(&item.original, &item.alternative), "dotted"),
        "alnum" => captured_head_curriculum(tools, split, options, &|item| f::alnum_question(&item.original, &item.alternative, &item.field.before, t).expect("a question"), "alnum"),
        "hypothesis" => {
            let held = options.held_out();
            captured_head_curriculum(tools, split, options, &|item| f::hypothesis_question(item) && !held.contains(&pystr::casefold(&item.original)) && !held.contains(&pystr::casefold(&item.alternative)), "hypothesis")
        }
        "short" => {
            let held = options.held_out();
            captured_head_curriculum(tools, split, options, &|item| item.before_origin == BeforeOrigin::Field && f::short_question(&item.original, &item.field.before, t).expect("a question") && !held.contains(&pystr::casefold(&item.original)), "short")
        }
        "slip" => {
            let held = options.held_out();
            captured_head_curriculum(tools, split, options, &|item| item.before_origin == BeforeOrigin::Field && f::slip_question(&item.original, &item.field.before, t) && !held.contains(&pystr::casefold(&item.original)), "slip")
        }
        "sign" => captured_head_curriculum(tools, split, options, &|item| f::sign_question(&item.original, &item.alternative, &item.field.before, &item.field.after, &item.trigger, item.after_origin), "sign"),
        "resume" => captured_head_curriculum(tools, split, options, &|item: &ContextEvidence| f::resume_question(&item.original, &item.alternative, &item.field.before, &item.field.after, item.after_origin), "resume"),
        "cyrletter" => captured_head_curriculum(tools, split, options, &|item| f::cyrletter_question(&item.original, &item.alternative, &item.field.before, &item.trigger), "cyrletter"),
        "dash" => captured_head_curriculum(tools, split, options, &|item| f::dash_question(&item.original, &item.alternative, &item.trigger), "dash"),
        "digitdot" => captured_head_curriculum(tools, split, options, &|item| f::digit_dot_question(&item.original, &item.alternative, &item.trigger), "digitdot"),
        "quote" => captured_head_curriculum(tools, split, options, &|item| f::quote_question(&item.original, &item.alternative, &item.trigger), "quote"),
        "knownreply" => {
            let (rows, mut report) = curricula::brief_curriculum(tools, split, options);
            let rows: Vec<ActionRow> = rows.into_iter().filter(|row| f::known_reply_question(&row.original, &reading(&row.original, 0), "", "", t)).collect();
            report["frames"] = json!(rows.len());
            Ok((rows, report))
        }
        _ => Err(format!("no curriculum of a head {label}")),
    }
}

/// `head_question`: whether the features of a frame hold one of these prefixes.
pub fn head_question(tools: &Tools, original: &str, alternative: &str, before: &str, after: &str, trigger: &str, origin: AfterOrigin, prefixes: &BTreeSet<String>, before_origin: BeforeOrigin) -> bool {
    let has = |prefix: &str| prefixes.contains(prefix);
    let t = &tools.tables;
    if has(HYPOTHESIS_FEATURE_PREFIX) && before_origin == BeforeOrigin::PlannedPreviousReading && origin != AfterOrigin::KeptNextWord {
        return true;
    }
    let letter = has(LETTER_FEATURE_PREFIX) && f::letter_question(original, alternative, before);
    let variable = has(VARIABLE_FEATURE_PREFIX) && f::variable_question(original, alternative, before, trigger);
    let alnum = has(ALNUM_FEATURE_PREFIX) && f::alnum_question(original, alternative, before, t).expect("a question");
    let cyrletter = has(CYRLETTER_FEATURE_PREFIX) && f::cyrletter_question(original, alternative, before, trigger);
    let digit_dot = has(DIGIT_DOT_FEATURE_PREFIX) && f::digit_dot_question(original, alternative, trigger);
    if origin == AfterOrigin::KeptNextWord {
        let sign = has(SIGN_FEATURE_PREFIX) && f::sign_question(original, alternative, before, after, trigger, origin);
        return has(KEPT_FEATURE_PREFIX) || letter || variable || alnum || sign || cyrletter || digit_dot;
    }
    letter
        || (has(CAPITALS_FEATURE_PREFIX) && f::capitals_question(original, before))
        || (has(ABBREVIATION_FEATURE_PREFIX) && f::abbreviation_question(original, alternative, before, t))
        || (has(LATIN_ABBREVIATION_FEATURE_PREFIX) && f::latin_abbreviation_question(original, alternative, before, t))
        || (has(ALONE_FEATURE_PREFIX) && f::alone_question(original, alternative, before, after, trigger))
        || (has(START_FEATURE_PREFIX) && f::start_question(original, alternative, before, after))
        || (has(UNKNOWN_FEATURE_PREFIX) && f::unknown_question(original, before, t))
        || (has(JOINED_FEATURE_PREFIX) && f::joined_question(original))
        || (has(TERM_FEATURE_PREFIX) && f::term_question(original, before, t))
        || (has(BRIEF_FEATURE_PREFIX) && f::brief_question(original, before, after))
        || (has(FIRST_FEATURE_PREFIX) && f::first_question(original, before, after, t))
        || variable
        || (has(DOTTED_FEATURE_PREFIX) && f::dotted_question(original, alternative))
        || alnum
        || (has(SHORT_FEATURE_PREFIX) && before_origin == BeforeOrigin::Field && f::short_question(original, before, t).expect("a question"))
        || (has(SLIP_FEATURE_PREFIX) && before_origin == BeforeOrigin::Field && f::slip_question(original, before, t))
        || (has(RESUME_FEATURE_PREFIX) && f::resume_question(original, alternative, before, after, origin))
        || cyrletter
        || (has(DASH_FEATURE_PREFIX) && f::dash_question(original, alternative, trigger))
        || digit_dot
        || (has(QUOTE_FEATURE_PREFIX) && f::quote_question(original, alternative, trigger))
        || (has(KNOWN_REPLY_FEATURE_PREFIX) && f::known_reply_question(original, alternative, before, after, t))
}
