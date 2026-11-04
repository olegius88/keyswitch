//! Bounded experimental action features shared by training and inference.
//!
//! The features of a word are an ordered map from names to values: the order is part of the sum the model takes, and a
//! name set twice keeps its first place, as in a Python `dict`. `FeatureMap` is an `IndexMap` for that.

use crate::constants::*;
use crate::context_model::{bucket_of, term_bucket, AfterOrigin, BeforeOrigin, ContextEvidence, TermTables, TERM_EDGE_SIGNS};
use indexmap::IndexMap;
use ks_text::layouts::LayoutPair;
use ks_text::{pystr, ucd};
use std::collections::HashSet;

pub type FeatureMap = IndexMap<String, f64>;

/// Features in the order a head adds them to a map.
type Pairs = Vec<(String, f64)>;

/// A `ValueError` of the extractor: the model then answers "unsupported".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeatureError(pub &'static str);

impl std::fmt::Display for FeatureError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for FeatureError {}

type Checked<T> = Result<T, FeatureError>;

// ---------------------------------------------------------------- small Python helpers

/// `max(lower, min(upper, value))` on floats, with the choices Python's `min` and `max` make for equal values.
fn clamp(value: f64, lower: f64, upper: f64) -> f64 {
    let capped = if value < upper { value } else { upper };
    if capped > lower { capped } else { lower }
}

fn bounded(value: f64, lower: f64, upper: f64) -> Checked<f64> {
    if !value.is_finite() {
        return Err(FeatureError("nonfinite context action evidence"));
    }
    Ok(clamp(value, lower, upper))
}

/// `min(a, b)` of floats: `b` only if it is smaller.
fn py_min(first: f64, second: f64) -> f64 {
    if second < first { second } else { first }
}

/// `max(a, b)` of floats: `b` only if it is greater.
fn py_max(first: f64, second: f64) -> f64 {
    if second > first { second } else { first }
}

fn flag(value: bool) -> u8 {
    u8::from(value)
}

fn count(text: &str) -> usize {
    pystr::len(text)
}

/// `text[-n:]`.
fn last(text: &str, n: usize) -> &str {
    pystr::slice(text, Some(-(n as i64)), None)
}

/// `text[:n]`.
fn first(text: &str, n: usize) -> &str {
    pystr::slice(text, None, Some(n as i64))
}

fn first_char(text: &str) -> &str {
    first(text, 1)
}

/// `int.bit_length()`.
fn bit_length(value: u64) -> i64 {
    i64::from(64 - value.leading_zeros())
}

fn clamp_int(value: i64, bound: i64) -> i64 {
    value.clamp(-bound, bound)
}

/// `max(-bands, min(bands, math.floor(delta / width)))`.
fn band(delta: f64, width: f64, bands: i64) -> i64 {
    (delta / width).floor().clamp(-(bands as f64), bands as f64) as i64
}

fn translate(text: &str, from: &str, to: &str) -> String {
    LayoutPair::default().translate(text, from, to).expect("the default layouts hold both names")
}

fn has_alpha(text: &str) -> bool {
    text.chars().any(ucd::isalpha)
}

/// `low <= char.casefold() <= high`, as the strings compare.
fn fold_between(c: char, low: &str, high: &str) -> bool {
    let mut buffer = [0_u8; 4];
    let folded = pystr::casefold(c.encode_utf8(&mut buffer));
    low <= folded.as_str() && folded.as_str() <= high
}

fn cyrillic_letter(c: char) -> bool {
    fold_between(c, "а", "я") || pystr::casefold(c.encode_utf8(&mut [0_u8; 4])) == "ё"
}

fn all_cyrillic(text: &str) -> bool {
    text.chars().all(cyrillic_letter)
}

/// `"а" <= char <= "я" or char == "ё"` of a character already casefolded.
fn russian_letter(c: char) -> bool {
    ('а'..='я').contains(&c) || c == 'ё'
}

fn english_letter(c: char) -> bool {
    c.is_ascii_lowercase()
}

/// The words of `_WORDS = [^\W\d_]+(?:['’\-][^\W\d_]+)*`.
fn words_of(text: &str) -> Vec<&str> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut words = Vec::new();
    let mut at = 0;
    let run = |from: usize| -> usize {
        let mut end = from;
        while end < chars.len() && ucd::re_letter(chars[end].1) {
            end += 1;
        }
        end
    };
    while at < chars.len() {
        if !ucd::re_letter(chars[at].1) {
            at += 1;
            continue;
        }
        let mut end = run(at);
        while end + 1 < chars.len() && matches!(chars[end].1, '\'' | '’' | '-') && ucd::re_letter(chars[end + 1].1) {
            end = run(end + 1);
        }
        let stop = if end < chars.len() { chars[end].0 } else { text.len() };
        words.push(&text[chars[at].0..stop]);
        at = end;
    }
    words
}

/// `re.findall("[a-z0-9]+", text)`.
fn ascii_tokens(text: &str) -> Vec<&str> {
    text.split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit())).filter(|part| !part.is_empty()).collect()
}

/// `re.findall(r"[^\W_]+", text)`.
fn alnum_tokens(text: &str) -> Vec<&str> {
    text.split(|c: char| !(ucd::re_w(c) && c != '_')).filter(|part| !part.is_empty()).collect()
}

/// `(text.split() or [""])[0]`.
fn first_word(text: &str) -> &str {
    pystr::split(text).first().copied().unwrap_or("")
}

/// `sorted(set(text))`.
fn sorted_chars(text: &str) -> Vec<char> {
    let mut chars: Vec<char> = text.chars().collect();
    chars.sort_unstable();
    chars.dedup();
    chars
}

fn char_count(text: &str, wanted: char) -> usize {
    text.chars().filter(|&c| c == wanted).count()
}

fn any_in(signs: &str, text: &str) -> bool {
    text.chars().any(|c| signs.contains(c))
}

fn script(text: &str) -> &'static str {
    let ru = text.chars().any(russian_letter);
    let en = text.chars().any(english_letter);
    if ru && en { "mixed" } else if ru { "ru" } else if en { "en" } else { "none" }
}

/// The script with more letters in `text`, as the line and context evidence names it.
fn dominant(text: &str) -> &'static str {
    let ru = text.chars().filter(|&c| russian_letter(c)).count();
    let en = text.chars().filter(|&c| english_letter(c)).count();
    if ru > en { "ru" } else if en > ru { "en" } else { "none" }
}

fn before_tail(item: &ContextEvidence) -> String {
    pystr::casefold(last(&item.field.before, ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS))
}

/// A dot between two Latin letters: the Latin reading is a domain or a file name (`sefan.ru`).
fn latin_dot(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    chars.windows(3).any(|window| window[0].is_ascii_alphabetic() && window[1] == '.' && window[2].is_ascii_alphabetic())
}

fn insert(features: &mut FeatureMap, name: String, value: f64) {
    features.insert(name, value);
}

/// The names of a head, each as a feature of value 1 under the head's prefix and a suffix.
fn head_pairs<S: AsRef<str>>(prefix: &str, names: &[S], suffix: &str) -> Pairs {
    names.iter().map(|name| (format!("{prefix}{}{suffix}", name.as_ref()), 1.0)).collect()
}

/// `{prefix + name: value for name, value in features.items() if value}`.
fn prefixed(prefix: &str, features: &FeatureMap) -> Pairs {
    features.iter().filter(|(_, value)| **value != 0.0).map(|(name, &value)| (format!("{prefix}{name}"), value)).collect()
}

fn update(features: &mut FeatureMap, pairs: Pairs) {
    for (name, value) in pairs {
        features.insert(name, value);
    }
}

// ---------------------------------------------------------------- the shared features

fn characters(features: &mut FeatureMap, label: &str, text: &str, direction: &str) {
    if text.is_empty() {
        return;
    }
    let padded: Vec<char> = std::iter::once('^').chain(text.chars()).chain(std::iter::once('$')).collect();
    for &order in ACTION_FEATURE_NGRAM_ORDERS {
        let grams = (padded.len() + 1).saturating_sub(order);
        for index in 0..grams {
            let gram: String = padded[index..index + order].iter().collect();
            let name = format!("{label}:char:{direction}:{order}:{gram}");
            let found = features.get(&name).copied().unwrap_or(0.0);
            features.insert(name, py_min(ACTION_FEATURE_CHARACTER_WEIGHT_CAP, found + 1.0 / (grams as f64).sqrt()));
        }
    }
}

fn word_score(features: &mut FeatureMap, label: &str, score: Option<&ks_lang::WordScore>, known: bool) -> Checked<()> {
    insert(features, format!("{label}:known:{}", flag(known)), 1.0);
    let Some(score) = score else {
        insert(features, format!("{label}:score_missing"), 1.0);
        return Ok(());
    };
    if score.frequency < 0 {
        return Err(FeatureError("invalid context action frequency"));
    }
    let bound = ACTION_FEATURE_WORD_SCORE_BOUND;
    let entries = [
        (format!("{label}:value"), bounded(score.value, -bound, bound)? / bound),
        (format!("{label}:gram_ratio"), bounded(score.gram_ratio, 0.0, 1.0)?),
        (format!("{label}:ngram_score"), bounded(score.ngram_score, -bound, bound)? / bound),
        (format!("{label}:invalid_ratio"), bounded(score.invalid_ratio, 0.0, 1.0)?),
        (
            format!("{label}:raw_ngram_score"),
            bounded(score.raw_ngram_score, -ACTION_FEATURE_RAW_NGRAM_SCORE_BOUND, 0.0)? / ACTION_FEATURE_RAW_NGRAM_SCORE_BOUND,
        ),
        (
            format!("{label}:logfrequency"),
            (score.frequency.min(ACTION_FEATURE_FREQUENCY_CAP) as f64).ln_1p() / (ACTION_FEATURE_FREQUENCY_CAP as f64).ln_1p(),
        ),
        (format!("{label}:exact:{}", flag(score.exact)), 1.0),
        (format!("{label}:spell_known:{}", flag(score.spell_known)), 1.0),
    ];
    for (name, value) in entries {
        features.insert(name, value);
    }
    Ok(())
}

/// How often a reading of the decided token occurs in one term table (context_model.term_bucket).
///
/// A training frame drawn with its counts dropped (ContextEvidence.term_counts_dropped) reads as uncounted: a term no
/// table holds (`webjs`) is what such a frame stands for, and its shape and context must decide it. The counts of the
/// words around the token are never dropped.
pub fn own_bucket(item: &ContextEvidence, text: &str, alphabet: &str, tables: &TermTables) -> &'static str {
    let bucket = term_bucket(text, alphabet, tables);
    if item.term_counts_dropped && bucket != "na" { "0" } else { bucket }
}

/// The words of a token of letters joined by hyphens, its edge signs aside (`ку-ку`, `pre-order`); None otherwise.
pub fn joined_parts(text: &str) -> Option<Vec<&str>> {
    let parts: Vec<&str> = pystr::strip_chars(text, TERM_EDGE_SIGNS).split('-').collect();
    (parts.len() > 1 && parts.iter().all(|part| pystr::isalpha(part))).then_some(parts)
}

/// Where the word stands on its line, whether a reading opens sentences, how often each occurs.
///
/// context-v1 schema 7 decided by this evidence (context_model.extract_context_features), and the action scheme had
/// none of it. Replayed through the engine, the owner's own typing (field logs of 0.26-0.36.3) showed what that cost:
/// 0.37.0 left words in the other layout about twice as often as 0.36.3, and nearly all of them were what the term
/// tables tell apart - an English term typed in the Russian layout amid Russian prose (`зк` is never seen inside Russian
/// text, `pr` often), a common Russian word typed in the English layout after an English term (`dct` is no term, `все`
/// the commonest word), a Russian abbreviation that stays (`тз` is frequent in Russian text). The tables are the
/// packaged ones schema 7 reads (context-term-frequency.json), counted on public text; an artifact trained before these
/// features carries no weight for them and scores as it did.
fn term_features(features: &mut FeatureMap, item: &ContextEvidence, direction: &str, tables: &TermTables) {
    let before = pystr::casefold(last(&item.field.before, ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS));
    let after = pystr::casefold(first(&item.field.after, ACTION_FEATURE_AFTER_CONTEXT_CHARACTERS));
    let known = format!("known:{}:{}", flag(item.source_known), flag(item.target_known));
    let (earlier, line) = match item.field.before.rfind('\n') {
        Some(at) => (Some(&item.field.before[..at]), &item.field.before[at + 1..]),
        None => (None, item.field.before.as_str()),
    };
    let state = if has_alpha(line) { "same" } else if earlier.is_some() { "newline" } else { "empty" };
    insert(features, format!("line:{state}:direction:{direction}"), 1.0);
    insert(features, format!("line:{state}:{known}:direction:{direction}"), 1.0);
    if state == "newline" {
        let previous_line = earlier.map_or("", |text| text.rsplit('\n').next().unwrap_or(""));
        let previous = pystr::casefold(previous_line);
        insert(features, format!("line:newline:previous:{}:direction:{direction}", dominant(&previous)), 1.0);
    }
    if item.source_opening || item.target_opening {
        let opening = format!("opening:{}:{}", flag(item.source_opening), flag(item.target_opening));
        let alone = flag(pystr::strip(&before).is_empty() && pystr::strip(&after).is_empty());
        insert(features, format!("{opening}:direction:{direction}"), 1.0);
        insert(features, format!("{opening}:alone:{alone}:direction:{direction}"), 1.0);
        insert(features, format!("{opening}:line:{state}:direction:{direction}"), 1.0);
    }
    let (latin, cyrillic) = if item.source_group == 0 { (&item.original, &item.alternative) } else { (&item.alternative, &item.original) };
    insert(features, format!("latin:dot:direction:{direction}"), f64::from(flag(latin_dot(latin))));
    insert(features, format!("latin:underscore:direction:{direction}"), f64::from(flag(pystr::strip_chars(latin, "_").contains('_'))));
    let cyrillic_known = if item.source_group == 1 { item.source_known } else { item.target_known };
    let latin_bucket = own_bucket(item, latin, "latin", tables);
    let cyrillic_bucket = if cyrillic_known { "lexicon" } else { own_bucket(item, cyrillic, "cyrillic", tables) };
    capitals_features(features, item, direction, &before, cyrillic, cyrillic_known, tables);
    if latin_bucket != "na" || cyrillic_bucket != "na" {
        frequency_features(features, item, direction, (&before, &after, state), (latin, cyrillic), (latin_bucket, cyrillic_bucket), cyrillic_known, tables);
    }
    if !has_alpha(&item.original) {
        let digits = flag(item.original.chars().any(ucd::isdigit));
        let kind = if has_alpha(&item.alternative) { "letters" } else { "only" };
        insert(features, format!("signs:{kind}:direction:{direction}"), 1.0);
        insert(features, format!("signs:{kind}:digits:{digits}:direction:{direction}"), 1.0);
    }
}

/// A token of letters only, all capitals, three or more: whether its Cyrillic reading is a known word, how often
/// Russian text uses it, and the script of the text before it.
///
/// `WBC` after Russian prose reads `ЦИС`, which the lexicon knows (the OpenSubtitles supplement) and Russian text never
/// uses; `CIF` reads `США`, which Russian text uses all the time. Without a feature of their own, the frames teaching
/// that such a citation stays moved the weights every Latin token after Russian text shares, and short Latin keys of
/// Russian words (`f` for `а`) stayed with them.
fn capitals_features(features: &mut FeatureMap, item: &ContextEvidence, direction: &str, before: &str, cyrillic: &str, cyrillic_known: bool, tables: &TermTables) {
    let letters = &item.original;
    if count(letters) < ACTION_FEATURE_CAPITALS_MIN_LETTERS || !pystr::isalpha(letters) || !pystr::isupper(letters) {
        return;
    }
    let russian = own_bucket(item, cyrillic, "russian", tables);
    let known = flag(cyrillic_known);
    insert(features, format!("capitals:known:{known}:russian:{russian}:direction:{direction}"), 1.0);
    insert(features, format!("capitals:known:{known}:russian:{russian}:before:{}:direction:{direction}", dominant(before)), 1.0);
}

/// How often each reading occurs inside Russian text and in text of its own language.
///
/// Only for a token at least one of whose readings is a word the tables could count. A token with digits or inner signs
/// in both readings has no entry in either, and as one shared bucket such tokens all read like the Latin ones with
/// digits the corpus restores (`зь2` is `pm2`): `1С`, `2фа` and `а1` in Russian prose became `1C`, `2af` and `f1`. An
/// absent count is no evidence.
#[allow(clippy::too_many_arguments)]
fn frequency_features(
    features: &mut FeatureMap, item: &ContextEvidence, direction: &str, surroundings: (&str, &str, &str), readings: (&str, &str),
    buckets: (&str, &str), cyrillic_known: bool, tables: &TermTables,
) {
    let (before, after, state) = surroundings;
    let (latin, cyrillic) = readings;
    let planned = item.after_origin == AfterOrigin::PlannedNextConversion;
    let context = format!("before:{}:{}:{}", dominant(before), if planned { "next" } else { "after" }, dominant(after));
    let mut latin_bucket = buckets.0.to_string();
    let mut cyrillic_bucket = buckets.1.to_string();
    if !item.inside && (!pystr::strip(before).is_empty() || !pystr::strip(after).is_empty()) {
        let english_bucket = own_bucket(item, latin, "english", tables);
        let russian_bucket = own_bucket(item, cyrillic, "russian", tables);
        if latin_bucket == "0" {
            latin_bucket = format!("0:{english_bucket}");
        }
        if cyrillic_known {
            cyrillic_bucket = format!("lexicon:{russian_bucket}");
        }
        let languages = format!("freq:english:{english_bucket}:russian:{russian_bucket}");
        insert(features, format!("freq:english:{english_bucket}:direction:{direction}"), 1.0);
        insert(features, format!("freq:russian:{russian_bucket}:direction:{direction}"), 1.0);
        insert(features, format!("{languages}:direction:{direction}"), 1.0);
        insert(features, format!("{languages}:{context}:direction:{direction}"), 1.0);
        insert(features, format!("{languages}:line:{state}:direction:{direction}"), 1.0);
    }
    let frequency = if item.inside { "inside:freq" } else { "freq" };
    insert(features, format!("{frequency}:latin:{latin_bucket}:direction:{direction}"), 1.0);
    insert(features, format!("{frequency}:cyrillic:{cyrillic_bucket}:direction:{direction}"), 1.0);
    insert(features, format!("{frequency}:{latin_bucket}:{cyrillic_bucket}:direction:{direction}"), 1.0);
    insert(features, format!("{frequency}:{latin_bucket}:{cyrillic_bucket}:{context}:direction:{direction}"), 1.0);
}

/// Whether a word can be asked again beside a next word kept as typed: a short word ended by a space, and a next word
/// of at most PLANNED_CONTEXT_AFTER_MAX_CHARACTERS with no space in it.
pub fn kept_context_reachable(original: &str, trigger: &str, boundary_text: &str, after: &str) -> bool {
    let length = count(original);
    0 < length
        && length <= KEPT_CONTEXT_WORD_MAX_CHARACTERS
        && trigger == "space"
        && boundary_text == " "
        && !after.is_empty()
        && count(after) <= PLANNED_CONTEXT_AFTER_MAX_CHARACTERS
        && !after.chars().any(ucd::isspace)
}

/// Describe each reading separately; no token-specific conversion rules.
pub fn extract_action_features(item: &ContextEvidence, tables: &TermTables) -> Checked<FeatureMap> {
    if item.source_group != 0 && item.source_group != 1 {
        return Err(FeatureError("invalid context action direction"));
    }
    let origin = item.after_origin;
    if item.before_origin == BeforeOrigin::PlannedPreviousReading && !matches!(origin, AfterOrigin::None | AfterOrigin::Field) {
        return Err(FeatureError("invalid left-context origin"));
    }
    if origin == AfterOrigin::KeptNextWord && !kept_context_reachable(&item.original, &item.trigger, &item.boundary_text, &item.field.after) {
        return Err(FeatureError("unreachable kept right context"));
    }
    if origin == AfterOrigin::PlannedNextConversion {
        let length = count(&item.original);
        if !(0 < length && length <= PLANNED_CONTEXT_WORD_MAX_CHARACTERS)
            || item.trigger != "space"
            || item.boundary_text != " "
            || item.field.after.is_empty()
            || count(&item.field.after) > PLANNED_CONTEXT_AFTER_MAX_CHARACTERS
            || item.field.after.chars().any(ucd::isspace)
        {
            return Err(FeatureError("unreachable planned right context"));
        }
    }
    let direction = item.source_group.to_string();
    let direction = direction.as_str();
    let origin_name = origin.as_str();
    let length = ACTION_FEATURE_LENGTH_BUCKET_MAX_CHARACTERS.min(count(&item.original).max(count(&item.alternative)));
    let baseline = flag(item.baseline_convert);
    let bound = ACTION_FEATURE_WORD_SCORE_BOUND;
    let mut features = FeatureMap::new();
    for (name, value) in [
        ("bias".to_string(), 1.0),
        (format!("direction:{direction}"), 1.0),
        (format!("baseline:{baseline}"), 1.0),
        (format!("length:{length}"), 1.0),
        (format!("baseline:{baseline}:length:{length}"), 1.0),
        (format!("known:{}:{}:length:{length}", flag(item.source_known), flag(item.target_known)), 1.0),
        (format!("identifier:{}:{}", flag(item.source_identifier), flag(item.target_identifier)), 1.0),
        (format!("role:{}", first(item.field.role.as_str(), ACTION_FEATURE_FIELD_LABEL_MAX_CHARACTERS)), 1.0),
        (format!("trigger:{}", first(&item.trigger, ACTION_FEATURE_FIELD_LABEL_MAX_CHARACTERS)), 1.0),
        ("score_delta".to_string(), bounded(item.score_delta, -bound, bound)? / bound),
        (format!("after_origin:{origin_name}"), 1.0),
        (format!("after_origin:{origin_name}:direction:{direction}:length:{length}"), 1.0),
        (
            format!(
                "after_origin:{origin_name}:script:{}:direction:{direction}:length:{length}",
                script(&pystr::casefold(first(&item.field.after, ACTION_FEATURE_AFTER_CONTEXT_CHARACTERS)))
            ),
            1.0,
        ),
    ] {
        features.insert(name, value);
    }
    if item.source_identifier || item.target_identifier {
        // Only an identifier reading carries new information; without one these combinations would merely duplicate the
        // length, direction and left-context evidence and double their learned weight.
        let prefix = format!("identifier:{}:{}", flag(item.source_identifier), flag(item.target_identifier));
        insert(&mut features, format!("{prefix}:direction:{direction}"), 1.0);
        insert(&mut features, format!("{prefix}:length:{length}"), 1.0);
        insert(&mut features, format!("{prefix}:before:{}", script(&before_tail(item))), 1.0);
    }
    let mut source_case = "none";
    for (label, raw, score, known, identifier) in [
        ("source", &item.original, item.source_score.as_ref(), item.source_known, item.source_identifier),
        ("target", &item.alternative, item.target_score.as_ref(), item.target_known, item.target_identifier),
    ] {
        let raw = first(raw, ACTION_FEATURE_WORD_MAX_CHARACTERS);
        let folded = ks_text::normalize::nfc(&pystr::casefold(raw));
        let text = first(&folded, ACTION_FEATURE_WORD_MAX_CHARACTERS);
        characters(&mut features, label, text, direction);
        word_score(&mut features, label, score, known)?;
        insert(&mut features, format!("{label}:identifier:{}", flag(identifier)), 1.0);
        let letters: String = raw.chars().filter(|&c| ucd::isalpha(c)).collect();
        let case = if letters.is_empty() {
            "none"
        } else if pystr::isupper(&letters) {
            "upper"
        } else if pystr::islower(&letters) {
            "lower"
        } else if pystr::istitle(&letters) {
            "title"
        } else {
            "mixed"
        };
        if label == "source" {
            source_case = case;
        }
        let raw_length = count(raw).max(1) as f64;
        insert(&mut features, format!("{label}:case:{case}"), 1.0);
        insert(&mut features, format!("{label}:script:{}", script(text)), 1.0);
        insert(&mut features, format!("{label}:length"), count(text) as f64 / ACTION_FEATURE_WORD_MAX_CHARACTERS as f64);
        insert(&mut features, format!("{label}:digits"), raw.chars().filter(|&c| ucd::isdigit(c)).count() as f64 / raw_length);
        for mark in ",.;[]'’`-_/@\\=<>\"".chars() {
            insert(&mut features, format!("{label}:mark:{:04x}", mark as u32), char_count(raw, mark) as f64 / raw_length);
        }
    }
    for (label, raw) in [
        ("before", last(&item.field.before, ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS)),
        ("after", first(&item.field.after, ACTION_FEATURE_AFTER_CONTEXT_CHARACTERS)),
    ] {
        let text = ks_text::normalize::nfc(&pystr::casefold(raw));
        let kind = script(&text);
        insert(&mut features, format!("{label}:script:{kind}:direction:{direction}"), 1.0);
        insert(&mut features, format!("{label}:script:{kind}:direction:{direction}:length:{length}"), 1.0);
        let words = words_of(&text);
        let neighbours: &[&str] = if label == "before" {
            &words[words.len().saturating_sub(ACTION_FEATURE_NEIGHBOUR_WORD_COUNT)..]
        } else {
            &words[..words.len().min(ACTION_FEATURE_NEIGHBOUR_WORD_COUNT)]
        };
        for word in neighbours {
            characters(&mut features, label, first(word, ACTION_FEATURE_NEIGHBOUR_WORD_MAX_CHARACTERS), direction);
        }
        let whitespace = if label == "before" {
            &raw[pystr::rstrip(raw).len()..]
        } else {
            &raw[..raw.len() - pystr::lstrip(raw).len()]
        };
        let whitespace = first(whitespace, ACTION_FEATURE_WHITESPACE_MAX_CHARACTERS);
        let limit = ACTION_FEATURE_WHITESPACE_MAX_CHARACTERS as f64;
        insert(&mut features, format!("{label}:space_count"), count(whitespace) as f64 / limit);
        for c in sorted_chars(whitespace) {
            insert(&mut features, format!("{label}:space:{:04x}", c as u32), char_count(whitespace, c) as f64 / limit);
        }
    }
    let short = ACTION_FEATURE_SHORT_TEXT_CHARACTERS as f64;
    let tail = first(&item.literal_tail, ACTION_FEATURE_SHORT_TEXT_CHARACTERS);
    insert(&mut features, "tail:length".to_string(), count(tail) as f64 / short);
    insert(&mut features, "tail:overflow".to_string(), f64::from(flag(count(&item.literal_tail) > ACTION_FEATURE_SHORT_TEXT_CHARACTERS)));
    for c in sorted_chars(tail) {
        insert(&mut features, format!("tail:char:{:04x}", c as u32), char_count(tail, c) as f64 / short);
    }
    let boundary = first(&item.boundary_text, ACTION_FEATURE_SHORT_TEXT_CHARACTERS);
    insert(&mut features, "boundary:length".to_string(), count(boundary) as f64 / short);
    insert(&mut features, "boundary:overflow".to_string(), f64::from(flag(count(&item.boundary_text) > ACTION_FEATURE_SHORT_TEXT_CHARACTERS)));
    insert(&mut features, "boundary:isspace".to_string(), f64::from(flag(pystr::isspace(boundary))));
    for c in sorted_chars(boundary) {
        insert(&mut features, format!("boundary:char:{:04x}", c as u32), char_count(boundary, c) as f64 / short);
    }
    let application = pystr::casefold(&item.field.application);
    for part in ascii_tokens(first(&application, ACTION_FEATURE_APPLICATION_NAME_CHARACTERS)).into_iter().take(ACTION_FEATURE_APPLICATION_TOKEN_COUNT) {
        insert(&mut features, format!("app:{part}"), 1.0);
    }
    for (label, value) in [("baseline:probability", item.model_probability), ("baseline:threshold", item.model_threshold)] {
        if let Some(value) = value {
            insert(&mut features, label.to_string(), bounded(value, 0.0, 1.0)?);
            insert(&mut features, format!("{label}:present"), 1.0);
        }
    }
    if let (Some(probability), Some(threshold)) = (item.model_probability, item.model_threshold) {
        insert(&mut features, "baseline:margin".to_string(), bounded(probability - threshold, -1.0, 1.0)?);
    }
    let ortho_bound = ACTION_FEATURE_ORTHO_SCORE_BOUND;
    for (label, value) in [("ortho:score", item.ortho_score), ("ortho:threshold", item.ortho_threshold)] {
        if let Some(value) = value {
            insert(&mut features, label.to_string(), bounded(value, -ortho_bound, ortho_bound)? / ortho_bound);
            insert(&mut features, format!("{label}:present"), 1.0);
        }
    }
    if let (Some(score), Some(threshold)) = (item.ortho_score, item.ortho_threshold) {
        let margin = bounded(score - threshold, -ortho_bound, ortho_bound)? / ortho_bound;
        insert(&mut features, "ortho:margin".to_string(), margin);
        // The threshold belongs to the frozen orthographic model. Its side is evidence with learned coefficients,
        // alongside the continuous margin.
        let side = if margin > 0.0 { "positive" } else if margin < 0.0 { "negative" } else { "zero" };
        let support = format!("direction:{direction}:source_known:{}", flag(item.source_known));
        insert(&mut features, format!("ortho:side:{side}:{support}"), 1.0);
        insert(&mut features, format!("ortho:side:{side}:{support}:case:{source_case}"), 1.0);
        let context_script = script(&before_tail(item));
        let category = format!("direction:{direction}:source_known:{}:context:{context_script}", flag(item.source_known));
        // Orthographic evidence may differ by language and lexical support. Both slopes are learned; neither licenses a
        // conversion by itself.
        for (sign, value) in [("positive", py_max(0.0, margin)), ("negative", py_min(0.0, margin))] {
            insert(&mut features, format!("ortho:{sign}:direction:{direction}"), value);
            insert(&mut features, format!("ortho:{sign}:{category}"), value);
        }
    }
    term_features(&mut features, item, direction, tables);
    heads(item, direction, features, tables)
}

/// The heads' questions, in the order the Python asks them, over the shared features.
fn heads(item: &ContextEvidence, direction: &str, features: FeatureMap, tables: &TermTables) -> Checked<FeatureMap> {
    let origin = item.after_origin;
    let letter = letter_question(&item.original, &item.alternative, &item.field.before);
    if origin == AfterOrigin::KeptNextWord {
        // A waiting word whose next word stayed as typed is a question of its own: the same evidence, its own weights
        // (KEPT_FEATURE_PREFIX). Sharing them, the question took the weights of the converted-neighbour frames and
        // turned `еще` before `поищу` into `tot`.
        let mut kept = FeatureMap::new();
        update(&mut kept, prefixed(KEPT_FEATURE_PREFIX, &features));
        if letter {
            // The kept-neighbour frames hold no word of one letter, and the question answered `keep` at p=1.00 for `b`
            // before `redis` in `nats b redis` (`nats и redis`, the owner's typing).
            let copy = prefixed(LETTER_FEATURE_PREFIX, &kept);
            update(&mut kept, copy);
            update(&mut kept, letter_evidence(item, direction, tables));
        }
        if variable_question(&item.original, &item.alternative, &item.field.before, &item.trigger) {
            // Nor a letter after a word of Cyrillic letters: the variable head answers there too.
            update(&mut kept, variable_evidence(item, tables));
        }
        if sign_question(&item.original, &item.alternative, &item.field.before, &item.field.after, &item.trigger, origin) {
            // And a sign after such a letter is evidence the variable head reads as no script at all: asked with `=`
            // after it, `ч` of `для ч = 0` was kept at p=0.95 (the sign head).
            update(&mut kept, sign_evidence(item, tables));
        }
        if alnum_question(&item.original, &item.alternative, &item.field.before, tables)? {
            // Nor a token of letters and digits: asked again with `как` kept after it, the question kept `м67` at p=1.00
            // where `v67` was meant (the owner's typing).
            update(&mut kept, alnum_evidence(item, tables)?);
        }
        if cyrletter_question(&item.original, &item.alternative, &item.field.before, &item.trigger) {
            // Nor a Latin letter meant as a Russian word of one letter after Russian text, waiting before a word typed in
            // the Russian layout (`d когда`): the Cyrillic-letter head answers it beside every next word.
            let evidence = cyrletter_evidence(item, direction, &kept, tables);
            update(&mut kept, evidence);
        }
        if digit_dot_question(&item.original, &item.alternative, &item.trigger) {
            // Nor digits with `ю` or `б` waiting before a word kept as typed (`2ю потом`): the digit-dot head answers
            // there too.
            let evidence = digit_dot_evidence(item, direction, &kept, tables);
            update(&mut kept, evidence);
        }
        return Ok(kept);
    }
    let mut result = FeatureMap::new();
    update(&mut result, prefixed("", &features));
    if capitals_question(&item.original, &item.field.before) {
        // The class the capitals head answers: its features once more under their own names, beside the shared ones, so
        // a head fitted onto a frozen model moves no decision outside the class. `BP` in English prose read `ИЗ` at
        // p=0.997, and the frames that kept it moved the weights the Latin keys of `не` and `что` share (`yt`, `xnj`
        // stayed as typed).
        let copy = prefixed(CAPITALS_FEATURE_PREFIX, &result);
        update(&mut result, copy);
    }
    if cyrletter_question(&item.original, &item.alternative, &item.field.before, &item.trigger) {
        // A Latin letter whose Cyrillic reading is a Russian word of one letter, after Russian text: every one-letter
        // row of the corpus sits in quarantine, and the frozen model's verdict on `d` in `разрабатывал и d документ` was
        // no opinion (convert at p=0.998 in the owner's field, keep at 0.93 on public text). The Cyrillic-letter head
        // answers it.
        update(&mut result, cyrletter_evidence(item, direction, &features, tables));
    }
    if digit_dot_question(&item.original, &item.alternative, &item.trigger) {
        // Digits with `ю` or `б` typed in the Russian layout: a lone letter to the frozen model, which waited on `2ю` at
        // p=0.999 where `2.` was meant (the owner's typing). The digit-dot head answers it.
        update(&mut result, digit_dot_evidence(item, direction, &features, tables));
    }
    if quote_question(&item.original, &item.alternative, &item.trigger) {
        // `@` and Latin letters whose other reading is a quotation mark and Cyrillic letters: the frozen model left `@Z`
        // where `"Я` opened a Russian message begun in the other layout. The quote head answers it.
        update(&mut result, quote_evidence(item, direction, &features, tables));
    }
    if dash_question(&item.original, &item.alternative, &item.trigger) {
        // A command-line flag in either layout: the frozen model read its leading sign, and `-v` after `git` became `-м`
        // at p=0.9995 (the owner's typing). The dash head answers it.
        update(&mut result, dash_evidence(item, direction, &features, tables));
    }
    if resume_question(&item.original, &item.alternative, &item.field.before, &item.field.after, origin) {
        // A short word right after Latin text asked again once its next word is planned for conversion: no head answered
        // it, and with Hunspell, which knows `f` and `z` as English words, the frozen model kept `а` in `seek() , f там`
        // and `я` in `TSet - z использовал` (the resume head).
        update(&mut result, resume_evidence(item, direction, &features, tables));
    }
    if alone_question(&item.original, &item.alternative, &item.field.before, &item.field.after, &item.trigger) {
        // The class the lone-word head answers, in the same way: every corpus frame of it was deferred, so the frozen
        // model waits on `гш` alone and never converts it to `ui`. The head also reads how often each reading occurs,
        // which the shared features leave out for a word with nothing around it: `ой` (Russian prose, 1298 times) and
        // `гш` (never) were the same to it.
        let copy = prefixed(ALONE_FEATURE_PREFIX, &result);
        update(&mut result, copy);
        update(&mut result, alone_counts(item, direction, tables));
    }
    if start_question(&item.original, &item.alternative, &item.field.before, &item.field.after) {
        // The class the message-start head answers, in the same way. With nothing around the word the shared features
        // leave out how often each reading occurs in prose, and a rare Russian word opening a message was converted on
        // its letters alone; the head reads both prose counts and the score delta beyond the shared feature's bound.
        let copy = prefixed(START_FEATURE_PREFIX, &result);
        update(&mut result, copy);
        update(&mut result, start_evidence(item, direction, tables));
    }
    if letter {
        // The class the single-letter head answers, in the same way. After a word of Latin letters the frozen model kept
        // `f` in `lid f номер` at p=1.00 with the next word converted (`lid а номер`, the owner's typing): no frame of
        // the corpus puts a Russian word of one letter there.
        let copy = prefixed(LETTER_FEATURE_PREFIX, &result);
        update(&mut result, copy);
        update(&mut result, letter_evidence(item, direction, tables));
    }
    if abbreviation_question(&item.original, &item.alternative, &item.field.before, tables) {
        // The class the abbreviation head answers. After Russian prose the frozen model turned abbreviations Russian text
        // uses into the Latin readings they spell (`страны ЕС приняли` became `страны TC приняли`). Unlike the other heads
        // it reads no copy of the shared features: the corpus holds a few hundred kinds of abbreviation, and a head
        // reading their letter n-grams learned those kinds by heart and turned `ТЗ` into `NP`. It reads what tells the
        // kinds apart instead: how often each reading occurs.
        update(&mut result, head_pairs(ABBREVIATION_FEATURE_PREFIX, &abbreviation_names(item, tables), &format!(":direction:{direction}")));
    }
    if latin_abbreviation_question(&item.original, &item.alternative, &item.field.before, tables) {
        // The same abbreviation after Latin text, the Latin-context abbreviation head's class: the same counts, and how
        // often the word before it occurs in English prose and in Russian technical text, which tell a sentence of English
        // prose from a command or an identifier (`/designv2 ТЗ`, the owner's typing).
        update(&mut result, latin_abbreviation_evidence(item, direction, tables));
    }
    if unknown_question(&item.original, &item.field.before, tables) && !item.source_known {
        // The class the unknown-token head answers: its features once more under their own names, beside the shared
        // ones, and evidence of its own. After Russian prose the frozen model kept a token no table counts as Russian
        // slang or a Latin citation, and the words typed in the wrong layout with it: `цуиоы` (`webjs`) at p=0.94 with
        // `webjs` typed right earlier in the same field, `мгуоы` (`vuejs`), `реьч` (`htmx`) (the owner's typing,
        // 13.10.2026), and the last word of a short Russian message, a word no table counts, typed in the English layout
        // (test v56).
        update(&mut result, prefixed(UNKNOWN_FEATURE_PREFIX, &features));
        update(&mut result, unknown_evidence(item));
    }
    if joined_question(&item.original) {
        // The class the joined-token head answers: its features once more under their own names, beside the shared ones,
        // and evidence of its own. No term table counts such a token, so the shared features read no count of it at all,
        // and the frozen model kept `Re-re!` after a Russian sentence, meant as `Ку-ку!`, at p=0.99 (sealed test v58,
        // both pairs); the head reads the counts of its words.
        update(&mut result, prefixed(JOINED_FEATURE_PREFIX, &features));
        update(&mut result, joined_evidence(item, tables)?);
    }
    if brief_question(&item.original, &item.field.before, &item.field.after)
        && (if item.source_known { brief_dominant(&item.original, &item.alternative, tables) } else { brief_settled(&item.original, &item.alternative, tables) })
    {
        // The class the short-message head answers: a short reply typed in the English layout, which the frozen model kept
        // or left waiting for a next word that never came (`ytn` for `нет`). The head reads how often each reading
        // occurs, the boundary and the case, and no copy of the shared features: with one it learned that a known Latin
        // word stays, and kept `vs` sent alone for `мы` (experiment E6). It answers only a token whose counts settle it:
        // a pair both readings count (`ij` and `шо`, `ds` and `вы`) stays with the frozen model, which turned no Russian
        // reply into Latin (candidate v63e), and so does a token a lexicon of the profile knows as typed, unless its
        // Russian reading outnumbers it by far (`jr` and `ок`, brief_dominant).
        update(&mut result, brief_evidence(item, tables));
    }
    if known_reply_question(&item.original, &item.alternative, &item.field.before, &item.field.after, tables) {
        // A short message whose Russian reading outnumbers its keys by far: the frozen model suggested a look at `jr` sent
        // alone in Telegram for `ок`, the short-message head answering it or not. The known-reply head answers it.
        update(&mut result, known_reply_evidence(item));
    }
    if variable_question(&item.original, &item.alternative, &item.field.before, &item.trigger) {
        // The class the variable head answers: a letter of the Russian layout after a word of Cyrillic letters that is no
        // Russian word of one letter. Every single-letter row of the corpus sits in quarantine, and the frozen model's
        // verdict on such a letter is no opinion: it turned `ч` into `x` in the owner's chat and kept `т` for `n` in
        // `результат n - 1`; the lone-letter refusal (context_policy) kept the conversions it got right too.
        update(&mut result, variable_evidence(item, tables));
    }
    if dotted_question(&item.original, &item.alternative) {
        // The class the dotted-abbreviation head answers: no frame of the corpus types `т.к.` in the English layout, and
        // the frozen model kept `n/r/` at p=1.00 as a path would be kept (the developer-chat development set).
        update(&mut result, dotted_evidence(item, tables)?);
    }
    if alnum_question(&item.original, &item.alternative, &item.field.before, tables)? {
        // The class the letters-and-digits head answers: the shared features read every digit as one class, and the
        // frozen model left `м67` for `v67` a suggestion, kept `x64` typed in the Russian layout as `ч64` and turned
        // `100мс` into `100vc`; its conversions of a token of one letter the lone-letter refusal kept as well
        // (context_policy).
        update(&mut result, alnum_evidence(item, tables)?);
    }
    if hypothesis_question(item) {
        // The class the hypothesis head answers: the next word of a waiting word asked with that word's other reading
        // before it. The frozen model read the reading as typed text, and a word that is no word of either layout went the
        // reading's way: `тыв` (a typo of `ты`) after `tot ` became `nsd` at p=1.00, and `еще тыв` became `tot nsd` (the
        // owner's typing, 22.10.2026).
        update(&mut result, hypothesis_evidence(item, tables));
    } else if slip_question(&item.original, &item.field.before, tables) && !item.source_known && !item.source_identifier {
        // The class the slip head answers: a word of Russian prose typed with the English layout on after a line of
        // Russian text, whose Latin reading no lexicon, identifier index or table of Latin text supports. The frozen model
        // left such a word a suggestion just under the threshold, which no pause settles (`я cgtie` for `я спешу`), or
        // kept it (`волк pfdsk` for `волк завыл`): its keep weights for unknown Latin tokens after Russian prose pulled
        // them down.
        update(&mut result, slip_evidence(item, tables));
    } else if short_question(&item.original, &item.field.before, tables)? && !item.source_known {
        // The class the short-token head answers: a token of two or three Russian letters after Russian text that no table
        // of Russian text counted, whose Latin reading the English lexicon knows. The frozen model turned `кз` after
        // `говорит он ` into `rp` at p=0.997 (the owner's typing, 22.10.2026), and `дз` and `ср` into `lp` and `ch` in
        // Russian phrases; the head reads how often Russian web and news text writes the token.
        update(&mut result, short_evidence(item, tables)?);
    }
    let known = item.source_known && !tail_like(&item.original, tables);
    if first_question(&item.original, &item.field.before, &item.field.after, tables) && item.field.role != ks_backend::FieldRole::Unknown && !known {
        // The class the first-term head answers: a term of up to three letters alone in a field a reader read empty, where
        // it opens a message (`йк ` for `qr`: the owner's requirement of 16.10.2026 that it convert at once, where the
        // frozen model waited for a pause). The term head, which chooses an early epoch for its other frames, stopped
        // short of the threshold there: `зк` and `йк` became suggestions at p 0.98, which no pause settles (candidates
        // v64b to v64e). The tail of a Russian word is no token of this class: an edit that leaves one as a token leaves
        // letters before it, or reopens the word and asks the whole of it.
        update(&mut result, first_evidence(item, tables));
    } else if term_question(&item.original, &item.field.before, tables) && !known {
        // The class the term head answers: a term typed in the Russian layout, which the frozen model left at p 0.92-0.98
        // the first time a message held it (`сымю` for `csv.`, `сш` for `ci`). The head reads evidence of its own alone:
        // with a copy of the shared features it learned that two letters and a known Latin reading stay, and kept `зк` for
        // `pr` after `принимай все` (experiment E6); the frozen model reads the context. A word the profile's lexicon
        // knows is a Russian word however rare in the tables, and stays with the frozen model: a word of four letters
        // opening a correct Russian sentence, known to Hunspell, went to its Latin reading, a term of 58 counts (sealed
        // test v62, the reference profile). A tail of Russian words the lexicon knows (`ев`, `ась`) stays the head's: the
        // head learns to keep tails from such frames, and without them it stopped learning after its first epoch, as it
        // did reading whether the lexicon knows the token (corpora v62 and v63).
        update(&mut result, term_evidence(item, tables));
    }
    Ok(result)
}

/// An attested Cyrillic abbreviation (attested_abbreviation) whose Latin reading is letters too, after text with more
/// Cyrillic letters than Latin ones: the abbreviation as written (`страны ЕС`) or a Latin one typed in the Russian
/// layout whose keys spell it (`TC`). After Latin text the capitals head answers the same shape (capitals_question); an
/// abbreviation Russian text does not use is the frozen model's.
pub fn abbreviation_question(original: &str, alternative: &str, before: &str, tables: &TermTables) -> bool {
    attested_abbreviation(original, tables)
        && pystr::isalpha(alternative)
        && dominant(&pystr::casefold(last(before, ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS))) == "ru"
}

/// A token of letters only, all capitals, of CAPITALS_HEAD_MIN_LETTERS to CAPITALS_HEAD_MAX_LETTERS, in Cyrillic, that
/// Russian technical text uses and the lexicon does not hold: counted among the Cyrillic words of Russian technical
/// text, which the term table keeps for words outside the lexicon (`ЕС`, `ТЗ`, `ИД`). A lexicon word in capitals
/// (`ЛУНЫ`) is a word typed with Caps Lock, which the frozen model keeps.
pub fn attested_abbreviation(original: &str, tables: &TermTables) -> bool {
    let length = count(original);
    if !((CAPITALS_HEAD_MIN_LETTERS..=CAPITALS_HEAD_MAX_LETTERS).contains(&length)
        && pystr::isalpha(original)
        && pystr::isupper(original)
        )
    {
        return false;
    }
    // `not any(char.isascii() ...)`: no character is ASCII.
    if original.chars().any(|c| c.is_ascii()) {
        return false;
    }
    tables.term("cyrillic").count(&pystr::casefold(original)) > 0
}

/// The evidence both abbreviation heads read (abbreviation evidence), without their prefix: the binary order of
/// magnitude of how often the Latin reading occurs among the Latin words of Russian technical text and in English prose,
/// and the Cyrillic one in Russian prose and among the Cyrillic words of Russian technical text, and of their ratio;
/// whether the lexicons and the identifier index know each reading; the length; whether the Cyrillic reading has a
/// letter no Russian word starts with (ABBREVIATION_NON_INITIAL_LETTERS), which no abbreviation of Russian words has;
/// the boundary. No letter of its own: a head reading them learned the corpus's kinds of abbreviation and turned `ДТП`
/// into `LNG`.
fn abbreviation_names(item: &ContextEvidence, tables: &TermTables) -> Vec<String> {
    let latin = pystr::casefold(&item.alternative);
    let cyrillic = pystr::casefold(&item.original);
    let magnitudes = [
        ("latin", bit_length(tables.term("latin").count(&latin))),
        ("english", bit_length(tables.term("english").count(&latin))),
        ("russian", bit_length(tables.term("russian").count(&cyrillic))),
        ("cyrillic", bit_length(tables.term("cyrillic").count(&cyrillic))),
    ];
    let latin_side = magnitudes[0].1.max(magnitudes[1].1);
    let cyrillic_side = magnitudes[2].1.max(magnitudes[3].1);
    let ratio = clamp_int(latin_side - cyrillic_side, ALONE_COUNT_RATIO_BOUND);
    let mut names = vec!["bias".to_string()];
    names.extend(magnitudes.iter().map(|(name, value)| format!("count:{name}:{value}")));
    names.push(format!("count:ratio:{ratio}"));
    names.push(format!("count:{latin_side}:{cyrillic_side}"));
    names.push(format!("letters:{}", count(&item.original)));
    names.push(format!("known:{}:{}", flag(item.source_known), flag(item.target_known)));
    names.push(format!("identifier:{}:{}", flag(item.source_identifier), flag(item.target_identifier)));
    names.push(format!("non_initial:{}", flag(item.original.chars().any(|c| ABBREVIATION_NON_INITIAL_LETTERS.contains(&c)))));
    names.push(format!("trigger:{}", first(&item.trigger, ACTION_FEATURE_FIELD_LABEL_MAX_CHARACTERS)));
    names
}

/// An attested Cyrillic abbreviation (attested_abbreviation) whose Latin reading is letters too, after text with more
/// Latin letters than Cyrillic ones: `ТЗ` after `/designv2 `, or `US` after English prose typed in the Russian layout
/// (`ГЫ`). The capitals head answers the same shape with a copy of the shared features, and both turned abbreviations
/// the Latin reading of which nobody writes into it (`ТЗ` into `NP`, `НГ` into `YU`).
pub fn latin_abbreviation_question(original: &str, alternative: &str, before: &str, tables: &TermTables) -> bool {
    attested_abbreviation(original, tables)
        && pystr::isalpha(alternative)
        && dominant(&pystr::casefold(last(before, ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS))) == "en"
}

/// The Latin-context abbreviation head's evidence: the abbreviation head's (abbreviation_names), and the binary order of
/// magnitude of how often the word before it occurs in English prose and among the Latin words of Russian technical text
/// (neighbour_names).
fn latin_abbreviation_evidence(item: &ContextEvidence, direction: &str, tables: &TermTables) -> Pairs {
    let words = pystr::split(&item.field.before);
    let mut names = abbreviation_names(item, tables);
    names.extend(neighbour_names("previous_word", words.last().copied().unwrap_or(""), tables));
    head_pairs(LATIN_ABBREVIATION_FEATURE_PREFIX, &names, &format!(":direction:{direction}"))
}

/// A token of ALONE_HEAD_LETTERS letters, letters in both layouts, with no letter before or after it in the field, at a
/// boundary that ends it for good: a message of one such word (`гш` for `ui`) sent, or left at a pause. At a space the
/// next word may still come and decide it with its neighbour, so the frozen model's wait stands there. `хз` is no member:
/// its Latin keys `[p` are no word.
pub fn alone_question(original: &str, alternative: &str, before: &str, after: &str, trigger: &str) -> bool {
    count(original) == ALONE_HEAD_LETTERS
        && pystr::isalpha(original)
        && pystr::isalpha(alternative)
        && trigger != "space"
        && !has_alpha(before)
        && !has_alpha(after)
}

/// The lone-word head's own count evidence: the order of magnitude of each reading's count in the term table (`latin`;
/// `cyrillic` or `russian`, whichever is larger) and of their ratio.
fn alone_counts(item: &ContextEvidence, direction: &str, tables: &TermTables) -> Pairs {
    let (latin, cyrillic) = if item.source_group == 0 { (&item.original, &item.alternative) } else { (&item.alternative, &item.original) };
    let (latin, cyrillic) = (pystr::casefold(latin), pystr::casefold(cyrillic));
    let latin_count = tables.term("latin").count(&latin);
    let cyrillic_count = tables.term("cyrillic").count(&cyrillic).max(tables.term("russian").count(&cyrillic));
    let magnitudes = (bit_length(latin_count), bit_length(cyrillic_count));
    let ratio = clamp_int(magnitudes.0 - magnitudes.1, ALONE_COUNT_RATIO_BOUND);
    let names = [format!("count:latin:{}", magnitudes.0), format!("count:cyrillic:{}", magnitudes.1), format!("count:ratio:{ratio}"), format!("count:{}:{}", magnitudes.0, magnitudes.1)];
    head_pairs(ALONE_FEATURE_PREFIX, &names, &format!(":direction:{direction}"))
}

/// A word of at least START_HEAD_MIN_LETTERS letters, letters in both layouts, with no letter before or after it in the
/// field: the first word of a message, at any boundary. Two letters are the lone-word head's (alone_question) or wait for
/// the next word; a word whose other reading has signs (`J,thyed`) is no member.
pub fn start_question(original: &str, alternative: &str, before: &str, after: &str) -> bool {
    count(original) >= START_HEAD_MIN_LETTERS && pystr::isalpha(original) && pystr::isalpha(alternative) && !has_alpha(before) && !has_alpha(after)
}

/// The message-start head's own evidence: the binary order of magnitude of how often the Latin reading occurs in English
/// prose and the Cyrillic one in Russian prose, and of their ratio; and the band of the detector's score delta
/// (START_DELTA_BAND_WIDTH wide, START_DELTA_BANDS either way).
fn start_evidence(item: &ContextEvidence, direction: &str, tables: &TermTables) -> Pairs {
    let (latin, cyrillic) = if item.source_group == 0 { (&item.original, &item.alternative) } else { (&item.alternative, &item.original) };
    let magnitudes = (bit_length(tables.term("english").count(&pystr::casefold(latin))), bit_length(tables.term("russian").count(&pystr::casefold(cyrillic))));
    let ratio = clamp_int(magnitudes.0 - magnitudes.1, ALONE_COUNT_RATIO_BOUND);
    let band = band(item.score_delta, START_DELTA_BAND_WIDTH, START_DELTA_BANDS);
    let names = [
        format!("count:english:{}", magnitudes.0),
        format!("count:russian:{}", magnitudes.1),
        format!("count:ratio:{ratio}"),
        format!("count:{}:{}", magnitudes.0, magnitudes.1),
        format!("delta:{band}"),
    ];
    head_pairs(START_FEATURE_PREFIX, &names, &format!(":direction:{direction}"))
}

/// A short word right after Latin text asked again once the word after it is planned for conversion (after_origin
/// planned_next_conversion): the nearest letter before it is Latin, a word follows it, and in either reading it has at
/// most OPENING_SHORT_MAX_LETTERS letters once one leading sign of RESUME_LEADING_SIGNS is set aside (`f`/`а` in
/// `seek() , f там`, `ds`/`вы` in `Shift+Tab ds`, `(b`/`(и`). That is where a person who forgot to switch back to Russian
/// after a Latin token resumes Russian text, and where a Latin token is followed by Russian text typed in the English
/// layout. A word of one letter right after a Latin word and a space is the single-letter head's (letter_question). The
/// question reads only the token, the text around it and the origin, as the trainer and the GPU path route frames by
/// them.
pub fn resume_question(original: &str, alternative: &str, before: &str, after: &str, origin: AfterOrigin) -> bool {
    let nearest = last(before, ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS).chars().rev().find(|&c| ucd::isalpha(c));
    origin == AfterOrigin::PlannedNextConversion
        && nearest.is_some_and(|c| c.is_ascii())
        && has_alpha(after)
        && !resume_word(original).is_empty()
        && !resume_word(alternative).is_empty()
        && !letter_question(original, alternative, before)
}

/// The letters of a short word without its one leading sign, or "" when it is no such word.
fn resume_word(token: &str) -> &str {
    let lead = first_char(token);
    let word = if RESUME_LEADING_SIGNS.contains(lead) { pystr::slice(token, Some(1), None) } else { token };
    if pystr::isalpha(word) && count(word) <= OPENING_SHORT_MAX_LETTERS { word } else { "" }
}

/// The resume head's evidence: the shared features once more under its own names, so that it weighs the lexicons' and
/// Hunspell's verdicts in its class apart (Hunspell knows `f`, and the detector's score delta, which converted `f`
/// without it, is gone), and evidence of its own: how often the Latin word before it occurs in English prose and among
/// the Latin words of Russian technical text, how often the next word as planned occurs, the signs between the Latin
/// word and it, whether Hunspell knows each reading, the leading sign, the number of letters and their case.
fn resume_evidence(item: &ContextEvidence, direction: &str, features: &FeatureMap, tables: &TermTables) -> Pairs {
    let before = last(&item.field.before, ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS);
    let chars: Vec<char> = before.chars().collect();
    let at = chars.iter().rposition(|&c| ucd::isalpha(c)).expect("the question found a letter");
    let mut gap_chars = sorted_chars(&chars[at + 1..].iter().collect::<String>());
    gap_chars.retain(|&c| c != ' ');
    let gap: String = gap_chars.into_iter().collect();
    let gap = if gap.is_empty() { "space".to_string() } else { gap };
    let previous = before.split(|c: char| !c.is_ascii_alphabetic()).rfind(|part| !part.is_empty()).unwrap_or("");
    let word = resume_word(&item.original);
    let case = if pystr::isupper(word) && count(word) > 1 {
        "upper"
    } else if pystr::isupper(first_char(word)) {
        "title"
    } else {
        "lower"
    };
    let sign = if RESUME_LEADING_SIGNS.contains(first_char(&item.original)) { first_char(&item.original) } else { "none" };
    let spelled = (
        flag(item.source_score.as_ref().is_some_and(|score| score.spell_known)),
        flag(item.target_score.as_ref().is_some_and(|score| score.spell_known)),
    );
    let mut names = vec![
        "bias".to_string(),
        format!("gap:{gap}"),
        format!("spell:{}:{}", spelled.0, spelled.1),
        format!("sign:{sign}"),
        format!("letters:{}", count(word)),
        format!("case:{case}"),
        format!("letters:{}:spell:{}:{}", count(word), spelled.0, spelled.1),
    ];
    names.extend(neighbour_names("previous_word", previous, tables));
    names.extend(neighbour_names("next_word", first_word(&item.field.after), tables));
    let mut pairs = prefixed(RESUME_FEATURE_PREFIX, features);
    pairs.extend(head_pairs(RESUME_FEATURE_PREFIX, &names, &format!(":direction:{direction}")));
    pairs
}

/// A word of one letter, a letter in both layouts, right after a word whose letters are all Latin: `b` after `nats `
/// (`nats и redis`), `f` after `lid ` (`lid а номер`), `a` after `use `. The engine asks it at the letter's boundary and
/// again beside the next word, converted or kept as typed. After Cyrillic, after a sign or with nothing before it the
/// frozen model decides alone.
pub fn letter_question(original: &str, alternative: &str, before: &str) -> bool {
    if count(original) != 1 || !pystr::isalpha(original) || !pystr::isalpha(alternative) || !pystr::isspace(last(before, 1)) {
        return false;
    }
    let words = pystr::split(before);
    let Some(word) = words.last() else { return false };
    let mut letters = word.chars().filter(|&c| ucd::isalpha(c)).peekable();
    letters.peek().is_some() && letters.all(|c| c.is_ascii())
}

/// The single-letter head's own evidence: the binary order of magnitude of how often the word before the letter, and the
/// word after it, occur in English prose and among the Latin words of Russian technical text (a Cyrillic word after it: in
/// Russian prose). `nats` and `redis` are counted in Russian technical text alone, `plan` and `then` in English prose.
fn letter_evidence(item: &ContextEvidence, direction: &str, tables: &TermTables) -> Pairs {
    let previous = pystr::split(&item.field.before).last().copied().unwrap_or("");
    let mut names = neighbour_names("previous_word", previous, tables);
    names.extend(neighbour_names("next_word", first_word(&item.field.after), tables));
    head_pairs(LETTER_FEATURE_PREFIX, &names, &format!(":direction:{direction}"))
}

/// How often a neighbouring word occurs: a Latin one in English prose and among the Latin words of Russian technical
/// text, a Cyrillic one in Russian prose, each as a binary order of magnitude; `none` without letters.
fn neighbour_names(side: &str, text: &str, tables: &TermTables) -> Vec<String> {
    let word = pystr::casefold(&text.chars().filter(|&c| ucd::isalpha(c)).collect::<String>());
    if word.is_empty() {
        return vec![format!("{side}:none")];
    }
    if word.is_ascii() {
        let english = bit_length(tables.term("english").count(&word));
        let latin = bit_length(tables.term("latin").count(&word));
        return vec![format!("{side}:english:{english}"), format!("{side}:latin:{latin}"), format!("{side}:{english}:{latin}")];
    }
    vec![format!("{side}:russian:{}", bit_length(tables.term("russian").count(&word)))]
}

/// A token of letters only, all capitals, of CAPITALS_HEAD_MIN_LETTERS to CAPITALS_HEAD_MAX_LETTERS, after text with more
/// Latin letters than Cyrillic ones (`according to the BP`).
///
/// After Russian prose the same shape is as often a word typed with Caps Lock (`я YT` for `я НЕ`) as a cited
/// abbreviation (`по версии WBC`), and a head fitted there kept 54 of 72 such words the corpus v26 pair converts, or
/// converted the abbreviations; there the frozen model decides alone.
pub fn capitals_question(original: &str, before: &str) -> bool {
    (CAPITALS_HEAD_MIN_LETTERS..=CAPITALS_HEAD_MAX_LETTERS).contains(&count(original))
        && pystr::isalpha(original)
        && pystr::isupper(original)
        && dominant(&pystr::casefold(last(before, ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS))) == "en"
}

/// A token the unknown-token head's class may hold (unknown_question): UNKNOWN_HEAD_MIN_CHARACTERS long or more, not all
/// capitals, of digits and of letters of one script - Russian letters whose keys are letters too, or Latin ones. Capitals
/// are the abbreviation heads' shapes; a Russian letter whose key is a sign (UNKNOWN_HEAD_SIGN_KEY_LETTERS) makes the Latin
/// reading no term (`фолбек` is `ajk,tr`), and a Latin token's keys are such letters in no case.
pub fn unknown_shape(original: &str) -> bool {
    if count(original) < UNKNOWN_HEAD_MIN_CHARACTERS || !pystr::isalnum(original) || pystr::isupper(original) {
        return false;
    }
    let letters: Vec<String> = original.chars().filter(|&c| ucd::isalpha(c)).map(|c| pystr::casefold(c.encode_utf8(&mut [0_u8; 4]))).collect();
    let single = |letter: &str| -> Option<char> {
        let mut chars = letter.chars();
        chars.next().filter(|_| chars.next().is_none())
    };
    !letters.is_empty()
        && (letters.iter().all(|letter| "a" <= letter.as_str() && letter.as_str() <= "z")
            || letters.iter().all(|letter| {
                "а" <= letter.as_str() && letter.as_str() <= "я" && !single(letter).is_some_and(|c| UNKNOWN_HEAD_SIGN_KEY_LETTERS.contains(&c))
            }))
}

/// The Cyrillic reading of a token of the head's shape: the token itself, or the Russian letters of its keys.
pub fn unknown_cyrillic(original: &str) -> String {
    if original.chars().any(|c| fold_between(c, "а", "я")) { original.to_string() } else { translate(original, "us", "ru") }
}

/// The words of the text before a token, as the unknown-token head compares readings with them.
fn earlier_words(before: &str) -> HashSet<String> {
    alnum_tokens(last(before, ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS)).into_iter().map(pystr::casefold).collect()
}

/// A token of the unknown-token head's shape (unknown_shape) whose Cyrillic reading Russian prose does not use, after text
/// with more Cyrillic letters than Latin ones: Latin letters typed in either layout (a Russian word no table counts typed
/// in the English layout, which test v56 found kept, or a Latin term typed as meant), or Russian letters whose Latin
/// reading the text before already holds as a word (`цуиоы` after `webjs` typed right earlier in the same message). With
/// its typed reading no lexicon word of its layout it is the head's class (extract_action_features). Russian letters
/// without that echo stay with the frozen model: there the class is Russian slang, truncations and brands in Cyrillic as
/// often as terms, and the head fitted on them converted `пукты` and `иишку` in the owner's typing. The question reads only
/// the typed token, the text before it and the packaged term table, as the trainer and the GPU path route frames by them.
pub fn unknown_question(original: &str, before: &str, tables: &TermTables) -> bool {
    let cyrillic = unknown_cyrillic(original);
    if !(unknown_shape(original)
        && matches!(term_bucket(&cyrillic, "russian", tables), "0" | "na")
        && dominant(&pystr::casefold(last(before, ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS))) == "ru")
    {
        return false;
    }
    let latin = cyrillic != original;
    latin || earlier_words(before).contains(&pystr::casefold(&translate(original, "ru", "us")))
}

/// The unknown-token head's evidence beside its copy of the shared features.
///
/// Whether either reading occurs as a whole word earlier in the field: a term typed right earlier in the same message is
/// the term typed again (`webjs не дает информацию о номерах ... дело в цуиоы`), while the shared features read only the
/// two words next to the token. Whether a reading is no word but one typo from one (`вляется`), which the shared features
/// do not read either. And the token's length.
fn unknown_evidence(item: &ContextEvidence) -> Pairs {
    let words = earlier_words(&item.field.before);
    let echo = format!("echo:{}:{}", flag(words.contains(&pystr::casefold(&item.alternative))), flag(words.contains(&pystr::casefold(&item.original))));
    let letters = format!("letters:{}", count(&item.original).min(ACTION_FEATURE_LENGTH_BUCKET_MAX_CHARACTERS));
    let names = [echo.clone(), format!("typo:{}:{}", flag(item.source_typo), flag(item.target_typo)), letters.clone(), format!("{echo}:{letters}")];
    // Each layout learns its own weights: an echo converts `цуиоы` after `webjs`, while `vscode` typed right after its
    // keys left as `мысщву` by a miss stays.
    head_pairs(UNKNOWN_FEATURE_PREFIX, &names, &format!(":direction:{}", item.source_group))
}

/// A token one of whose readings is words joined by hyphens (joined_parts): the joined-token head's class.
///
/// The other reading is the same keys in the other layout: `j,sxyj-nfr` is `обычно-так`. The question reads only the typed
/// token, as the trainer and the GPU path route frames by it (head_question, context_action_cuda.heads_question).
pub fn joined_question(original: &str) -> bool {
    if joined_parts(original).is_some() {
        return true;
    }
    let latin = original.chars().any(|c| fold_between(c, "a", "z"));
    let other = if latin { translate(original, "us", "ru") } else { translate(original, "ru", "us") };
    joined_parts(&other).is_some()
}

/// The joined-token head's evidence beside its copy of the shared features: how often the most and the least frequent word
/// of each reading occurs in its language's prose, and whether the token repeats one word.
///
/// No term table counts a token of words joined by hyphens, and the shared features read no count of it, while its words
/// tell the language: `ку` occurs in Russian prose and `re` nowhere in English (`Re-re!` is `Ку-ку!`), and `pre-order` is
/// English by `order` though `pre` is no word of either prose. The shared features stay as the frozen model read them: fed
/// these counts, its weights for single words turned `bye-bye` typed as meant into `ину-ину`.
fn joined_evidence(item: &ContextEvidence, tables: &TermTables) -> Checked<Pairs> {
    let (latin, cyrillic) = if item.source_group == 0 { (&item.original, &item.alternative) } else { (&item.alternative, &item.original) };
    let mut names: Vec<String> = Vec::new();
    for (reading, alphabet) in [(latin, "english"), (cyrillic, "russian")] {
        let mut buckets: Vec<&str> = match joined_parts(reading) {
            None => vec!["na"],
            Some(parts) => {
                let mut buckets = Vec::with_capacity(parts.len());
                for part in parts {
                    let bucket = term_bucket(part, alphabet, tables);
                    // `sorted(..., key=int)`: a part that is no word of the table raises.
                    if bucket == "na" {
                        return Err(FeatureError("invalid literal for int() with base 10: 'na'"));
                    }
                    buckets.push(bucket);
                }
                buckets.sort_by_key(|bucket| bucket.parse::<i64>().unwrap_or(0));
                buckets
            }
        };
        if item.term_counts_dropped {
            for bucket in &mut buckets {
                if *bucket != "na" {
                    *bucket = "0";
                }
            }
        }
        names.push(format!("{alphabet}:most:{}:least:{}", buckets[buckets.len() - 1], buckets[0]));
    }
    let words = joined_parts(&item.original).or_else(|| joined_parts(&item.alternative)).unwrap_or_default();
    let distinct: HashSet<String> = words.iter().map(|word| pystr::casefold(word)).collect();
    let repeat = format!("repeat:{}", flag(distinct.len() == 1));
    let counts = format!("{}:{}", names[0], names[1]);
    let all = [names[0].clone(), names[1].clone(), counts.clone(), repeat.clone(), format!("{repeat}:{counts}")];
    Ok(head_pairs(JOINED_FEATURE_PREFIX, &all, &format!(":direction:{}", item.source_group)))
}

/// A token of Russian letters whose Latin reading is Latin letters, a trailing sign (TERM_HEAD_TRAILING_SIGNS) and the
/// leading dot of an extension aside, that Russian technical text uses at least in bucket TERM_HEAD_MIN_LATIN_BUCKET of
/// the term table, while its Cyrillic reading occurs in no Russian text the tables count (`сымю` is `csv.`, `сш` is `ci`,
/// `рвьш` is `hdmi`): the term head's class. A token the unknown-token head answers (a term typed right earlier in the
/// text) is that head's, words joined by hyphens are the joined-token head's, and an attested abbreviation is counted
/// among Cyrillic words. The question reads only the typed token, the text before it and the packaged term table, as the
/// trainer and the GPU path route frames by them (head_question, context_action_cuda.heads_question).
pub fn term_question(original: &str, before: &str, tables: &TermTables) -> bool {
    if count(original) < TERM_HEAD_MIN_LETTERS || !all_cyrillic(original) {
        return false;
    }
    let latin_reading = translate(original, "ru", "us");
    let core = pystr::rstrip_chars(&latin_reading, TERM_HEAD_TRAILING_SIGNS);
    let core = core.strip_prefix(TERM_HEAD_LEADING_SIGN).unwrap_or(core);
    if count(core) < TERM_HEAD_MIN_LETTERS || !(core.is_ascii() && pystr::isalpha(core)) {
        return false;
    }
    let latin = term_bucket(core, "latin", tables);
    latin.parse::<usize>().is_ok_and(|bucket| bucket >= TERM_HEAD_MIN_LATIN_BUCKET)
        && term_bucket(original, "cyrillic", tables) == "0"
        && term_bucket(original, "russian", tables) == "0"
        && !unknown_question(original, before, tables)
}

/// The term head's evidence, all it reads (it has no copy of the shared features): how often Russian technical text and
/// English text use the Latin reading, whether it ends in a sign typed on a Russian letter's key (`сымю` is `csv.`) or
/// opens with the dot of an extension (`юучу` is `.exe`), how many Russian words end with it (russian_ending_bucket) with
/// the token's length, which script the text before it is in, the band of the detector's score delta (as the message-start
/// head reads it), how the text after it was given (asked again with its converted neighbour after it, or not) and, for a
/// token alone in the field, whether a space or an end (Enter, a pause, a sign) ended it, with its letters: a term of two
/// letters alone at a space waits, a longer one converts, and read without the letters the wait pulled the longer ones and
/// the convert the shorter (`гш` became a suggestion, experiment E9f). A term of up to three letters alone in a field a
/// reader read empty is the first-term head's (first_question), not this head's.
/// Without the endings the head could tell the tail of a Russian word an edit left (`ев`) from a term's keys (`зк`) by the
/// length alone, and it kept short terms; the length is read with the ending only, for the same reason. Alone at a space
/// the frozen model waits, and a wait stands until a pause of one and a half seconds settles it: the head converts there
/// by its own weights (`йк` for `qr`, the owner's typing of 15.10.2026). Asked again once the Russian word after it went
/// back to Russian, the term is the same term: `зк` became `pr` in `займись зк и тз`, the layout followed, `и` came out as
/// `b`, and asked again with `и` after it the frozen model leaned to keeping `зк`, and the engine took both back.
fn term_evidence(item: &ContextEvidence, tables: &TermTables) -> Pairs {
    let latin = item.alternative.as_str();
    let counts = format!("latin:{}:english:{}", term_bucket(latin, "latin", tables), term_bucket(latin, "english", tables));
    let signed = format!("signed:{}", flag(latin.chars().next_back().is_some_and(|c| TERM_HEAD_TRAILING_SIGNS.contains(c))));
    let extension = format!("extension:{}", flag(latin.starts_with(TERM_HEAD_LEADING_SIGN)));
    let letters = format!("letters:{}", latin.chars().filter(|&c| ucd::isalpha(c)).count().min(ACTION_FEATURE_LENGTH_BUCKET_MAX_CHARACTERS));
    let before = format!("before:{}", dominant(&before_tail(item)));
    let ending = format!("ending:{}", russian_ending_bucket(&item.original, tables));
    let delta = format!("delta:{}", band(item.score_delta, START_DELTA_BAND_WIDTH, START_DELTA_BANDS));
    let boundary = format!("{before}:boundary:{}", if item.trigger == "space" { "space" } else { "end" });
    let after = format!("after:{}", item.after_origin.as_str());
    let names = [
        "bias".to_string(),
        counts.clone(),
        signed.clone(),
        extension,
        before.clone(),
        ending.clone(),
        delta.clone(),
        boundary.clone(),
        after.clone(),
        format!("{counts}:{before}"),
        format!("{signed}:{before}"),
        format!("{ending}:{letters}"),
        format!("{ending}:{before}"),
        format!("{ending}:{counts}"),
        format!("{ending}:{letters}:{counts}"),
        format!("{delta}:{before}"),
        format!("{boundary}:{ending}:{letters}"),
        format!("{boundary}:{letters}"),
        format!("{after}:{ending}"),
        format!("{after}:{counts}"),
    ];
    head_pairs(TERM_FEATURE_PREFIX, &names, "")
}

/// How many words of Russian prose end with the token, as the binary order of magnitude of their number
/// (TERM_HEAD_ENDING_BUCKET_MAX at most); `na` for a token of another length than TERM_HEAD_ENDING_LETTERS.
pub fn russian_ending_bucket(token: &str, tables: &TermTables) -> String {
    let word = pystr::casefold(token);
    if !TERM_HEAD_ENDING_LETTERS.contains(&count(&word)) {
        return "na".to_string();
    }
    bit_length(tables.russian_endings().get(&word)).min(TERM_HEAD_ENDING_BUCKET_MAX as i64).to_string()
}

/// How many words of Russian prose begin with the token, as the binary order of magnitude of their number
/// (TERM_HEAD_ENDING_BUCKET_MAX at most); `na` for a token of another length than TERM_HEAD_ENDING_LETTERS.
pub fn russian_start_bucket(token: &str, tables: &TermTables) -> String {
    let word = pystr::casefold(token);
    if !TERM_HEAD_ENDING_LETTERS.contains(&count(&word)) {
        return "na".to_string();
    }
    bit_length(tables.russian_starts().get(&word)).min(TERM_HEAD_ENDING_BUCKET_MAX as i64).to_string()
}

/// Whether a token ends as many Russian words as a tail the term head keeps does (TERM_FRAGMENT_MIN_ENDING_BUCKET).
fn tail_like(token: &str, tables: &TermTables) -> bool {
    let ending = russian_ending_bucket(token, tables);
    ending != "na" && ending.parse::<usize>().is_ok_and(|bucket| bucket >= TERM_FRAGMENT_MIN_ENDING_BUCKET)
}

/// A token of the term head's class (term_question) of up to three letters, the most a term's ending is read for
/// (TERM_HEAD_ENDING_LETTERS), with no letter before it in the field: the first-term head's class when a field reader read
/// the field (the role, which the trainer's and the GPU path's routing do not see, is checked where the features are
/// made). The word after it is the one the engine asks it again with: a term converted there once a Russian word followed,
/// a token that waited once its next word was converted. The question reads only the typed token and the text before it.
pub fn first_question(original: &str, before: &str, _after: &str, tables: &TermTables) -> bool {
    count(original) <= *TERM_HEAD_ENDING_LETTERS.iter().max().expect("letters are listed") && term_question(original, before, tables) && !has_alpha(before)
}

/// The first-term head's evidence, every piece read with the boundary that ended the token (a space, or an end such as a
/// pause): its letters, how often Russian technical text and English prose use its Latin reading, how many Russian words
/// end and begin with it, and how the word after it was given (none, or the next word it is asked again with). Ended by a
/// pause, a token many Russian words begin with may be a word stopped in the middle (`уче` of `учеб`), and that answer
/// must not reach the space: evidence shared by both took the pause's answer to the space too and left `ци`, `пя` and `мц`
/// suggestions there at p 0.95-0.985 (candidate v67b). Asked again by the term head, a term converted there went back with
/// the Russian word after it on one corpus and stayed on another (`йк истек`, p 0.981 and 0.995, candidates v65 and v64f).
fn first_evidence(item: &ContextEvidence, tables: &TermTables) -> Pairs {
    let latin = item.alternative.as_str();
    let letters = format!("letters:{}", item.original.chars().filter(|&c| ucd::isalpha(c)).count());
    let boundary = format!("boundary:{}", if item.trigger == "space" { "space" } else { "end" });
    let counts = format!("latin:{}:english:{}", term_bucket(latin, "latin", tables), term_bucket(latin, "english", tables));
    let ending = format!("ending:{}", russian_ending_bucket(&item.original, tables));
    let start = format!("start:{}", russian_start_bucket(&item.original, tables));
    let after = format!("after:{}", item.after_origin.as_str());
    let names = ["".to_string(), letters.clone(), counts.clone(), format!("{ending}:{letters}"), start.clone(), format!("{start}:{letters}"), after.clone(), format!("{after}:{letters}"), format!("{after}:{counts}")];
    names.iter().map(|name| (format!("{FIRST_FEATURE_PREFIX}{boundary}{}{name}", if name.is_empty() { "" } else { ":" }), 1.0)).collect()
}

/// One letter typed in the Russian layout that is no Russian word of one letter (short_words.TRUSTED_SINGLE_LETTER_WORDS),
/// whose Latin reading is a letter too, right after a word of Cyrillic letters and a space, ended by a space or a sign
/// (VARIABLE_HEAD_TRIGGERS): a Latin variable typed in the Russian layout in technical prose (`т` for `n` in `результат n -
/// 1`, `ч` for `x`), or a letter meant as written (`т.` of `и т. д.`, a word broken off in chat). After a word of Latin
/// letters the single-letter head answers (letter_question); with no word before it, or at a pause, the frozen model.
pub fn variable_question(original: &str, alternative: &str, before: &str, trigger: &str) -> bool {
    let trusted = ks_lang::short_words::TRUSTED_SINGLE_LETTER_WORDS;
    if count(original) != 1
        || !original.chars().all(cyrillic_letter)
        || count(alternative) != 1
        || !pystr::isalpha(alternative)
        || trusted.contains(&pystr::casefold(original).as_str())
        || !pystr::isspace(last(before, 1))
        || !VARIABLE_HEAD_TRIGGERS.contains(&trigger)
    {
        return false;
    }
    pystr::split(before).last().is_some_and(|word| all_cyrillic(word))
}

/// The variable head's evidence: the letter and its case, the boundary (a space, or the sign that ended it), the word
/// before it and how often Russian prose uses that word, whether the text before holds signs of code
/// (VARIABLE_CODE_SIGNS), Latin letters or digits, and how the word after it was given; the letter and the boundary are
/// each read with the rest. In chat such a letter is meant as written (`да я про то ч`), in a question about code it is a
/// variable (`для ч = 0.3`).
fn variable_evidence(item: &ContextEvidence, tables: &TermTables) -> Pairs {
    let tail = last(&item.field.before, VARIABLE_CODE_CONTEXT_CHARACTERS);
    let previous = pystr::casefold(pystr::split(&item.field.before).last().copied().unwrap_or(""));
    let following = pystr::casefold(first_word(&item.field.after));
    let letter = format!("letter:{}", pystr::casefold(&item.original));
    let boundary = punctuation_boundary(item);
    let rest = [
        format!("case:{}", if pystr::isupper(&item.original) { "upper" } else { "lower" }),
        format!("previous:{}", term_bucket(&previous, "russian", tables)),
        format!("code:{}", flag(any_in(VARIABLE_CODE_SIGNS, tail))),
        format!("latin:{}", flag(tail.chars().any(|c| fold_between(c, "a", "z")))),
        format!("digit:{}", flag(tail.chars().any(ucd::isdigit))),
        format!("after:{}:{}", item.after_origin.as_str(), script(&following)),
    ];
    let mut names = vec!["bias".to_string(), letter.clone(), boundary.clone(), format!("word:{previous}")];
    names.extend(rest.iter().cloned());
    names.push(format!("{letter}:{boundary}"));
    for head in [&letter, &boundary] {
        names.extend(rest.iter().map(|name| format!("{head}:{name}")));
    }
    head_pairs(VARIABLE_FEATURE_PREFIX, &names, "")
}

/// `boundary:{trigger}:{boundary_text.strip() if trigger == 'punctuation' else ''}`.
fn punctuation_boundary(item: &ContextEvidence) -> String {
    format!("boundary:{}:{}", item.trigger, if item.trigger == "punctuation" { pystr::strip(&item.boundary_text) } else { "" })
}

/// One Latin letter whose Cyrillic reading is a Russian word of one letter (short_words.TRUSTED_SINGLE_LETTER_WORDS,
/// either case), the nearest letter before it Cyrillic, ended by a space or a sign (CYRLETTER_HEAD_TRIGGERS): `в` typed `d`
/// in `разрабатывал и d документ` and `и` typed `b` in `посмотри b добавь`, where the English layout came on in the middle
/// of Russian text, and a Latin letter meant as written after Russian text (`язык C`, `витамин D`, `функция f`). The mirror
/// of the variable head's class. Any origin of the next word: the engine asks the letter at its own boundary, again beside
/// its next word once that is planned for conversion, and beside a next word kept as typed. With no letter before it the
/// message-start rule speaks; after Latin text the single-letter and resume heads answer.
pub fn cyrletter_question(original: &str, alternative: &str, before: &str, trigger: &str) -> bool {
    let trusted = ks_lang::short_words::TRUSTED_SINGLE_LETTER_WORDS;
    if count(original) != 1
        || !original.is_ascii()
        || !pystr::isalpha(original)
        || count(alternative) != 1
        || !trusted.contains(&pystr::casefold(alternative).as_str())
        || !CYRLETTER_HEAD_TRIGGERS.contains(&trigger)
    {
        return false;
    }
    last(before, ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS).chars().rev().find(|&c| ucd::isalpha(c)).is_some_and(cyrillic_letter)
}

/// The Cyrillic-letter head's evidence: the features it is asked with once more under its own names, so that it weighs the
/// lexicons' and Hunspell's verdicts in its class apart, and evidence of its own: the letter and its case, the boundary (a
/// space, or the sign that ended it), how often the word before it and the word after it occur, how the word after it was
/// given, whether the text before holds signs of code (VARIABLE_CODE_SIGNS), Latin letters or digits, and whether Hunspell
/// knows each reading; the letter, its case and the next word's origin are each read with the rest. In Russian prose the
/// letter is a word of it typed in the English layout (`и d документ`); in technical prose a Latin letter is meant as
/// written (`язык C`, `от a до b`).
fn cyrletter_evidence(item: &ContextEvidence, direction: &str, features: &FeatureMap, tables: &TermTables) -> Pairs {
    let previous = pystr::split(&item.field.before).last().copied().unwrap_or("");
    // Signs of code, Latin letters and digits in the word right before it: a Latin letter is named right after them (`x =
    // 2 C`), and a digit two words back said nothing of `если d` (`в 2 1 если d документ`, the owner's typing).
    let tail = previous;
    let following = first_word(&item.field.after);
    let letter = format!("letter:{}", pystr::casefold(&item.original));
    let case = format!("case:{}", if pystr::isupper(&item.original) { "upper" } else { "lower" });
    let after = format!("after:{}:{}", item.after_origin.as_str(), script(&pystr::casefold(following)));
    let spelled = (
        flag(item.source_score.as_ref().is_some_and(|score| score.spell_known)),
        flag(item.target_score.as_ref().is_some_and(|score| score.spell_known)),
    );
    let mut rest = vec![
        punctuation_boundary(item),
        format!("code:{}", flag(any_in(VARIABLE_CODE_SIGNS, tail))),
        format!("latin:{}", flag(tail.chars().any(|c| fold_between(c, "a", "z")))),
        format!("digit:{}", flag(tail.chars().any(ucd::isdigit))),
        format!("spell:{}:{}", spelled.0, spelled.1),
    ];
    rest.extend(neighbour_names("previous_word", previous, tables));
    rest.extend(neighbour_names("next_word", following, tables));
    // The word before it as such: after `и` or `что` a Russian word of one letter comes often, after `язык` or `функция` a
    // Latin letter named by it (candidate v81 e1, which read only how often that word occurs, left three in four of its
    // converted development frames a suggestion at the letter's own space).
    let spoken = pystr::casefold(&previous.chars().filter(|&c| ucd::isalpha(c)).collect::<String>());
    let word = format!("word:{}", if spoken.is_empty() { "none" } else { spoken.as_str() });
    let mut names = vec!["bias".to_string(), letter.clone(), case.clone(), after.clone(), word.clone()];
    names.extend(rest.iter().cloned());
    names.push(format!("{letter}:{case}"));
    names.push(format!("{letter}:{after}"));
    names.push(format!("{letter}:{word}"));
    for head in [&letter, &case, &after] {
        names.extend(rest.iter().map(|name| format!("{head}:{name}")));
    }
    let mut pairs = prefixed(CYRLETTER_FEATURE_PREFIX, features);
    pairs.extend(head_pairs(CYRLETTER_FEATURE_PREFIX, &names, &format!(":direction:{direction}")));
    pairs
}

/// Digits, then `ю` typed in the Russian layout (DIGIT_DOT_SIGN_LETTERS), digits after it allowed, whose other reading has
/// the dot there (DIGIT_DOT_SIGNS), at a space (DIGIT_DOT_HEAD_TRIGGERS): `2ю` for `2.` and `5ю2` for `5.2` where a person
/// used to the English layout pressed its dot key in Russian text.
pub fn digit_dot_question(original: &str, alternative: &str, trigger: &str) -> bool {
    if !DIGIT_DOT_HEAD_TRIGGERS.contains(&trigger) || count(original) != count(alternative) {
        return false;
    }
    let head = leading_digits(original);
    let original: Vec<char> = original.chars().collect();
    let alternative: Vec<char> = alternative.chars().collect();
    if !(0 < head && head < original.len()) {
        return false;
    }
    let rest = &original[head + 1..];
    DIGIT_DOT_SIGN_LETTERS.contains(original[head])
        && DIGIT_DOT_SIGNS.contains(alternative[head])
        && alternative[..head] == original[..head]
        && alternative[head + 1..] == *rest
        && (rest.is_empty() || (rest.iter().all(|&c| ucd::isdigit(c))))
}

fn leading_digits(token: &str) -> usize {
    token.chars().position(|c| !ucd::isdigit(c)).unwrap_or_else(|| count(token))
}

/// The digit-dot head's evidence: the features it is asked with once more under its own names, and evidence of its own:
/// the sign letter, how many digits stand before it and whether digits follow it, how often the word before it occurs, the
/// script of the word after it and how it was given, and whether the text before holds signs of code or Latin letters; the
/// sign letter is read with the rest. `пункт 2ю` and `версии 5ю2` are a dot.
fn digit_dot_evidence(item: &ContextEvidence, direction: &str, features: &FeatureMap, tables: &TermTables) -> Pairs {
    let head = leading_digits(&item.original);
    let sign = format!("sign:{}", pystr::slice(&item.original, Some(head as i64), Some(head as i64 + 1)));
    let previous = pystr::split(&item.field.before).last().copied().unwrap_or("");
    let following = first_word(&item.field.after);
    let tail = last(&item.field.before, VARIABLE_CODE_CONTEXT_CHARACTERS);
    let mut rest = vec![
        format!("digits:{}", head.min(OPENING_SHORT_MAX_LETTERS)),
        format!("after_digits:{}", flag(head + 1 < count(&item.original))),
        format!("after:{}:{}", item.after_origin.as_str(), script(&pystr::casefold(following))),
        format!("code:{}", flag(any_in(VARIABLE_CODE_SIGNS, tail))),
        format!("latin:{}", flag(tail.chars().any(|c| fold_between(c, "a", "z")))),
    ];
    rest.extend(neighbour_names("previous_word", previous, tables));
    let mut names = vec!["bias".to_string(), sign.clone()];
    names.extend(rest.iter().cloned());
    names.extend(rest.iter().map(|name| format!("{sign}:{name}")));
    let mut pairs = prefixed(DIGIT_DOT_FEATURE_PREFIX, features);
    pairs.extend(head_pairs(DIGIT_DOT_FEATURE_PREFIX, &names, &format!(":direction:{direction}")));
    pairs
}

/// A token of one sign of QUOTE_HEAD_SIGNS and letters of one script whose other reading is the other sign and letters, at
/// a space or a sign (QUOTE_HEAD_TRIGGERS): `@Z` typed for `"Я` where a Russian message was begun in the English layout with
/// a quotation mark, and `@media`, `@property`, a mention or `"Я` meant as typed.
pub fn quote_question(original: &str, alternative: &str, trigger: &str) -> bool {
    if !QUOTE_HEAD_TRIGGERS.contains(&trigger) || original.is_empty() || count(original) != count(alternative) {
        return false;
    }
    let (a, b) = (original.chars().next().expect("not empty"), alternative.chars().next().expect("not empty"));
    if !QUOTE_HEAD_SIGNS.contains(a) || !QUOTE_HEAD_SIGNS.contains(b) || a == b {
        return false;
    }
    [original, alternative].iter().all(|part| one_script_letters(pystr::slice(part, Some(1), None)))
}

fn one_script_letters(text: &str) -> bool {
    pystr::isalpha(text) && (text.is_ascii() || all_cyrillic(text))
}

/// The quote head's evidence: the features it is asked with once more under its own names, and evidence of its own: the
/// sign typed, the number of letters and their case, how often the letters occur as a Latin word in English prose and among
/// the Latin words of Russian technical text and as a Cyrillic one in Russian prose, whether text stands before it, how the
/// word after it was given and its script, and whether the text before holds signs of code; the sign is read with the rest.
/// `@Z сейчас` opens a Russian sentence; `@media (max-width` and `@property` open code.
fn quote_evidence(item: &ContextEvidence, direction: &str, features: &FeatureMap, tables: &TermTables) -> Pairs {
    let word = pystr::slice(&item.original, Some(1), None);
    let other = pystr::slice(&item.alternative, Some(1), None);
    let (latin, cyrillic) = if word.is_ascii() { (word, other) } else { (other, word) };
    let case = if pystr::isupper(word) && count(word) > 1 {
        "upper"
    } else if pystr::isupper(first_char(word)) {
        "title"
    } else {
        "lower"
    };
    let following = first_word(&item.field.after);
    let tail = last(&item.field.before, VARIABLE_CODE_CONTEXT_CHARACTERS);
    let sign = format!("sign:{}", first_char(&item.original));
    let mut rest = vec![
        format!("letters:{}", count(word).min(OPENING_SHORT_MAX_LETTERS + 1)),
        format!("case:{case}"),
        format!("start:{}", flag(!has_alpha(&item.field.before))),
        format!("after:{}:{}", item.after_origin.as_str(), script(&pystr::casefold(following))),
        format!("code:{}", flag(any_in(VARIABLE_CODE_SIGNS, tail))),
    ];
    rest.extend(neighbour_names("letters", latin, tables));
    rest.extend(neighbour_names("reading", cyrillic, tables));
    let mut names = vec!["bias".to_string(), sign.clone()];
    names.extend(rest.iter().cloned());
    names.extend(rest.iter().map(|name| format!("{sign}:{name}")));
    let mut pairs = prefixed(QUOTE_FEATURE_PREFIX, features);
    pairs.extend(head_pairs(QUOTE_FEATURE_PREFIX, &names, &format!(":direction:{direction}")));
    pairs
}

/// One or two leading dashes (DASH_HEAD_MAX_DASHES) and letters of one script, digits after them allowed, in either
/// reading, at a space, a sign or the end of a line (DASH_HEAD_TRIGGERS): a command-line flag typed in either layout, `-v`
/// meant as typed after `git` and `-м` typed for it with the Russian layout on, `--force`, `-v2`, `-цфеср` for `-watch`.
pub fn dash_question(original: &str, alternative: &str, trigger: &str) -> bool {
    DASH_HEAD_TRIGGERS.contains(&trigger) && dash_shape(original) && dash_shape(alternative)
}

fn dash_shape(token: &str) -> bool {
    let dashes = count(token) - count(pystr::lstrip_chars(token, "-"));
    let letters = letters_of_flag(pystr::slice(token, Some(dashes as i64), None));
    (1..=DASH_HEAD_MAX_DASHES).contains(&dashes) && pystr::isalpha(letters) && (letters.is_ascii() || all_cyrillic(letters))
}

/// The letters of a flag without the digits after them (`v` of `v2`).
fn letters_of_flag(rest: &str) -> &str {
    let mut end = rest.len();
    while let Some(c) = rest[..end].chars().next_back() {
        if !ucd::isdigit(c) {
            break;
        }
        end -= c.len_utf8();
    }
    &rest[..end]
}

/// The dash head's evidence: the features it is asked with once more under its own names, and evidence of its own: the
/// number of dashes and of letters, their case, whether digits follow them, how often the letters occur as a Latin word in
/// English prose and among the Latin words of Russian technical text and as a Cyrillic one in Russian prose, how often the
/// word before the flag occurs, and whether the text before holds signs of code (VARIABLE_CODE_SIGNS). A flag follows a
/// command (`git -v`, `npm i --save`); a Russian word after a dash is rarely typed so.
fn dash_evidence(item: &ContextEvidence, direction: &str, features: &FeatureMap, tables: &TermTables) -> Pairs {
    let dashes = count(&item.original) - count(pystr::lstrip_chars(&item.original, "-"));
    let letters = letters_of_flag(pystr::slice(&item.original, Some(dashes as i64), None));
    let other = letters_of_flag(pystr::slice(&item.alternative, Some(dashes as i64), None));
    let (latin, cyrillic) = if letters.is_ascii() { (letters, other) } else { (other, letters) };
    let case = if pystr::isupper(letters) {
        "upper"
    } else if pystr::isupper(first_char(letters)) {
        "title"
    } else {
        "lower"
    };
    let previous = pystr::split(&item.field.before).last().copied().unwrap_or("");
    let tail = last(&item.field.before, VARIABLE_CODE_CONTEXT_CHARACTERS);
    let mut rest = vec![
        format!("dashes:{dashes}"),
        format!("letters:{}", count(letters).min(OPENING_SHORT_MAX_LETTERS + 1)),
        format!("case:{case}"),
        format!("digits:{}", flag(count(letters) < count(&item.original) - dashes)),
        format!("code:{}", flag(any_in(VARIABLE_CODE_SIGNS, tail))),
    ];
    rest.extend(neighbour_names("letters", latin, tables));
    rest.extend(neighbour_names("reading", cyrillic, tables));
    rest.extend(neighbour_names("previous_word", previous, tables));
    let mut names = vec!["bias".to_string()];
    names.extend(rest.iter().cloned());
    names.extend(rest.iter().map(|name| format!("dashes:{dashes}:{name}")));
    let mut pairs = prefixed(DASH_FEATURE_PREFIX, features);
    pairs.extend(head_pairs(DASH_FEATURE_PREFIX, &names, &format!(":direction:{direction}")));
    pairs
}

/// A letter of the variable head's class (variable_question) asked once more with the token after it given as kept text
/// (after_origin kept_next_word), that token holding no letter and no dash alone (sign_token_asked): a sign, signs or
/// digits (`ч =`, `м +`, `ч 2`). The engine asks a kept letter so when a token that reads alike in both layouts follows it
/// (_decide_letter_before_sign), and a waiting one before any token kept as typed (_decide_with_kept_neighbour). In a
/// question about code the letter is a Latin variable; before a number in Russian prose it is meant as written (`ГОСТ Р
/// 54960`). The question reads only the letter, the text around it and the origin, as the trainer and the GPU path route
/// frames by them.
pub fn sign_question(original: &str, alternative: &str, before: &str, after: &str, trigger: &str, origin: AfterOrigin) -> bool {
    origin == AfterOrigin::KeptNextWord && sign_token_asked(first_word(after)) && variable_question(original, alternative, before, trigger)
}

/// Whether a letter is asked again with this token after it: one holding no letter that is no dash alone (SIGN_DASH).
pub fn sign_token_asked(token: &str) -> bool {
    !pystr::strip_chars(token, SIGN_DASH).is_empty() && !has_alpha(token)
}

fn sign_token(after: &str) -> String {
    let token = first_word(after);
    if pystr::isdigit(token) {
        return "digits".to_string();
    }
    if count(token) <= SIGN_TOKEN_MAX_CHARACTERS { token.to_string() } else { "other".to_string() }
}

fn sign_kind(after: &str) -> &'static str {
    let token = first_word(after);
    if pystr::isdigit(token) {
        return "digits";
    }
    if token.chars().all(|c| SIGN_OPERATOR_SIGNS.contains(c)) { "operator" } else { "other" }
}

/// The sign head's evidence, all it reads beside the kept-neighbour question's: the token after the letter itself (up to
/// SIGN_TOKEN_MAX_CHARACTERS signs, or digits, or longer) and its kind (an operator, digits, other), the letter, its case,
/// how often Russian prose uses the word before it, and whether the text before holds signs of code (VARIABLE_CODE_SIGNS),
/// Latin letters or digits, each also read with the token and with its kind; the letter alone, as the two Russian `м +` of
/// the Leipzig sentences (`50 кв м + балкон`) outweighed every `м` before a formula sign once read with the sign
/// (candidate v76 e3). The variable head reads every such token as the same script: `=` after `для ч` went the way of the
/// dash in `е - все то, что`.
fn sign_evidence(item: &ContextEvidence, tables: &TermTables) -> Pairs {
    let tail = last(&item.field.before, VARIABLE_CODE_CONTEXT_CHARACTERS);
    let previous = pystr::casefold(pystr::split(&item.field.before).last().copied().unwrap_or(""));
    let token = format!("token:{}", sign_token(&item.field.after));
    let kind = format!("kind:{}", sign_kind(&item.field.after));
    let letter = format!("letter:{}", pystr::casefold(&item.original));
    let rest = [
        format!("case:{}", if pystr::isupper(&item.original) { "upper" } else { "lower" }),
        format!("previous:{}", term_bucket(&previous, "russian", tables)),
        format!("code:{}", flag(any_in(VARIABLE_CODE_SIGNS, tail))),
        format!("latin:{}", flag(tail.chars().any(|c| fold_between(c, "a", "z")))),
        format!("digit:{}", flag(tail.chars().any(ucd::isdigit))),
    ];
    let mut names = vec!["bias".to_string(), token.clone(), kind.clone(), letter];
    names.extend(rest.iter().cloned());
    for head in [&token, &kind] {
        names.extend(rest.iter().map(|name| format!("{head}:{name}")));
    }
    head_pairs(SIGN_FEATURE_PREFIX, &names, "")
}

/// `[а-яё]` with `re.IGNORECASE`: the letter in either case, and the old Cyrillic letters that fold to `в`, `д`, `о`, `с`,
/// `т` and `ъ`.
fn cyrillic_ignore_case(c: char) -> bool {
    matches!(c, 'а'..='я' | 'А'..='Я' | 'ё' | 'Ё' | '\u{1C80}'..='\u{1C86}')
}

/// `_DOTTED_READING.fullmatch(text)`: `[а-яё]{1,3}(?:\.[а-яё]{1,3})+\.?`, ignoring case.
fn dotted_reading(text: &str) -> bool {
    let mut parts: Vec<&str> = text.split('.').collect();
    if parts.last() == Some(&"") {
        parts.pop();
    }
    parts.len() >= 2 && parts.iter().all(|part| (1..=DOTTED_PART_MAX_LETTERS).contains(&count(part)) && part.chars().all(cyrillic_ignore_case))
}

/// A token typed in the English layout with `/`, the key of the Russian dot, whose Cyrillic reading is two or more groups of
/// up to DOTTED_PART_MAX_LETTERS letters joined by dots: a Russian abbreviation typed in the wrong layout (`n/r/` for
/// `т.к.`, `l/,/` for `д.б.`), or a Latin token of short parts meant as typed (`n/a`, `I/O`, `usr/bin`). A reading of one
/// group (`cv/` for `см.`, `dev/` for `вум.`) is no member: a word with the key of a dot after it is as often a part of a
/// path as a short Russian word ending a sentence, and the frozen model decides it.
pub fn dotted_question(original: &str, alternative: &str) -> bool {
    original.is_ascii() && original.contains('/') && dotted_reading(alternative)
}

/// The term-table bucket (term_bucket) of the rarest part of a token in one table; a part that is no word at all (`,` of
/// `l/,/`, typed for `д.б.`) is rarer than any word: `na`.
fn rarest_part(parts: &[&str], alphabet: &str, tables: &TermTables) -> Checked<&'static str> {
    let buckets: Vec<&str> = parts.iter().filter(|part| !part.is_empty()).map(|part| term_bucket(part, alphabet, tables)).collect();
    if buckets.contains(&"na") {
        return Ok("na");
    }
    buckets.iter().copied().min_by_key(|bucket| bucket.parse::<i64>().unwrap_or(0)).ok_or(FeatureError("min() iterable argument is empty"))
}

/// The dotted-abbreviation head's evidence: the Cyrillic reading itself (the abbreviations are few: `т.к.`, `т.е.`, `т.д.`,
/// `т.п.`, `д.б.`), the lengths of its parts and its final dot, its case, the script of the word before and after it, and
/// how often the parts occur as words: the rarest part of the reading in Russian prose and the rarest part typed among the
/// Latin words of Russian technical text (term_bucket). Each part of `т.к.` is a Russian word of the table, no part of
/// `гык.ишт` (`usr/bin`) is. The reading, its shape and the two counts are read with each of the rest.
fn dotted_evidence(item: &ContextEvidence, tables: &TermTables) -> Checked<Pairs> {
    let reading = pystr::casefold(&item.alternative);
    let words = pystr::split(&item.field.before);
    let following = pystr::split(&item.field.after);
    let lengths: Vec<String> = reading.split('.').filter(|part| !part.is_empty()).map(|part| count(part).to_string()).collect();
    let shape = format!("shape:{}:{}", lengths.join("-"), flag(reading.ends_with('.')));
    let case = format!("case:{}", if pystr::isupper(first_char(&item.original)) { "upper" } else { "lower" });
    let before = format!("before:{}", words.last().map_or("start", |word| script(&pystr::casefold(word))));
    let after = format!("after:{}:{}", item.after_origin.as_str(), following.first().map_or("none", |word| script(&pystr::casefold(word))));
    let russian = rarest_part(&reading.split('.').collect::<Vec<_>>(), "russian", tables)?;
    let latin = rarest_part(&pystr::casefold(&item.original).split('/').collect::<Vec<_>>(), "latin", tables)?;
    let counts = format!("russian:{russian}:latin:{latin}");
    let reading_name = format!("reading:{reading}");
    let mut names = vec!["bias".to_string(), reading_name.clone(), shape.clone(), case.clone(), before.clone(), after.clone(), format!("russian:{russian}"), format!("latin:{latin}"), counts.clone()];
    for head in [&reading_name, &shape, &counts] {
        for name in [&case, &before, &after] {
            names.push(format!("{head}:{name}"));
        }
    }
    Ok(head_pairs(DOTTED_FEATURE_PREFIX, &names, ""))
}

/// Letters and ASCII digits alone, at least one of each, the letters all Latin or all Cyrillic.
pub fn letters_and_digits(token: &str) -> bool {
    let mut letters = token.chars().filter(|&c| ucd::isalpha(c)).peekable();
    let has_letters = letters.peek().is_some();
    has_letters
        && token.chars().any(|c| c.is_ascii_digit())
        && token.chars().all(|c| ucd::isalpha(c) || c.is_ascii_digit())
        && (token.chars().filter(|&c| ucd::isalpha(c)).all(|c| c.is_ascii()) || token.chars().filter(|&c| ucd::isalpha(c)).all(cyrillic_letter))
}

/// A token typed in Russian letters and digits, of at most ALNUM_HEAD_MAX_CHARACTERS, whose keys in the English layout are
/// Latin letters and digits, that Russian text is known to write as a whole token one way or the other
/// (context-alnum-frequency.json counts it as typed or its Latin reading), after text that is Russian or none: a Latin
/// token typed in the Russian layout in Russian text (`шзм6` for `ipv6`, `зь2` for `pm2`, `ч64` for `x64`, `2аф` for
/// `2fa`), or a Russian one meant as typed (`1с`, `3д`, `100мс`): the letters-and-digits head's class. A reading with a sign
/// is none (`2х` reads `2[`), nor a token with a sign inside (`-v2`, `5ю2`): the frozen model decides those, and the
/// lone-letter refusal (context_policy) a token of one letter. After English text the frozen model decides too, and for a
/// token typed in Latin letters, whose typed questions label `1c` of Russian prose a conversion to `1с`. A token the table
/// counts in neither script is the frozen model's as well: fitted on the 2023 dump, where such shapes are Russian (`2х`,
/// `ГОСТ2012`), the head kept model names typed in the Russian layout in Russian sentences that the frozen model had
/// converted (`В7000` for `D7000`, `К2В2` for `R2D2`, `220ф` for `220a`; corpus v72, candidates v72 and v72b). The question
/// reads only the token, its other reading, the text before it and the packaged table, as the trainer and the GPU path
/// route frames by them.
pub fn alnum_question(original: &str, alternative: &str, before: &str, tables: &TermTables) -> Checked<bool> {
    if !(count(original) <= ALNUM_HEAD_MAX_CHARACTERS
        && !original.is_ascii()
        && letters_and_digits(original)
        && alternative.is_ascii()
        && letters_and_digits(alternative)
        && dominant(&pystr::casefold(last(before, ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS))) != "en")
    {
        return Ok(false);
    }
    let tokens = tables.alnum().map_err(|_| FeatureError("invalid term frequency table"))?;
    Ok(tokens.alphabet("cyrillic").count(&pystr::casefold(original)) > 0 || tokens.alphabet("latin").count(&pystr::casefold(alternative)) > 0)
}

/// The runs of letters (`L`) and digits (`D`) of a token, each with its length up to ALNUM_RUN_LENGTH_MAX: `м67` is `L1D2`,
/// `ц4и` `L1D1L1`, `100мс` `D3L2`; a token of more than ALNUM_SHAPE_MAX_RUNS runs is `many`.
fn alnum_shape(token: &str) -> String {
    let mut runs: Vec<(char, usize)> = Vec::new();
    for c in token.chars() {
        let kind = if ucd::isdigit(c) { 'D' } else { 'L' };
        match runs.last_mut() {
            Some(run) if run.0 == kind => run.1 += 1,
            _ => runs.push((kind, 1)),
        }
    }
    if runs.len() > ALNUM_SHAPE_MAX_RUNS {
        return "many".to_string();
    }
    runs.iter().map(|(kind, size)| format!("{kind}{}", (*size).min(ALNUM_RUN_LENGTH_MAX))).collect()
}

/// The binary order of magnitude by which a Cyrillic count outnumbers a Latin one, ALNUM_RATIO_BOUND at most either way.
fn ratio_of(cyrillic: u64, latin: u64) -> i64 {
    clamp_int(bit_length(cyrillic) - bit_length(latin), ALNUM_RATIO_BOUND)
}

/// The longest run of the characters `wanted` accepts, the first of equals: `max(re.findall(..), key=len)`.
fn longest_run(text: &str, wanted: fn(char) -> bool) -> Checked<&str> {
    let mut best: Option<&str> = None;
    for run in text.split(|c: char| !wanted(c)).filter(|part| !part.is_empty()) {
        if best.is_none_or(|found| count(run) > count(found)) {
            best = Some(run);
        }
    }
    best.ok_or(FeatureError("max() iterable argument is empty"))
}

/// The letters-and-digits head's evidence: how often Russian text writes the token as typed and its Latin reading as whole
/// tokens (context-alnum-frequency.json: `1с` 1 363 times and `1c` 134, `ipv6` 351 and `шзм6` never), each as a bucket of
/// the term tables' bounds, with the binary order of magnitude by which one outnumbers the other (ALNUM_RATIO_BOUND at most
/// either way); the same order for the longest run of letters of each reading in the term tables (Russian text for the
/// Cyrillic run, English prose or Russian technical text for the Latin one) and whether either occurs; its shape (runs of
/// letters and digits), the case of its letters, the script of the text and of the word before it, whether that text holds
/// signs of code (VARIABLE_CODE_SIGNS) or digits, the boundary and how the word after it was given; the shape and both
/// orders are read with the rest. The shared features read every digit as one class: `1С` became `1C` in Russian prose and
/// `2фа` `2af`, while `м67` for `v67` stayed a suggestion and `x64` typed in the Russian layout stayed `ч64` (the owner's
/// typing, ru.stackoverflow messages of 2023).
fn alnum_evidence(item: &ContextEvidence, tables: &TermTables) -> Checked<Pairs> {
    let tokens = tables.alnum().map_err(|_| FeatureError("invalid term frequency table"))?;
    let cyrillic = pystr::casefold(&item.original);
    let latin = pystr::casefold(&item.alternative);
    let typed = tokens.alphabet("cyrillic").count(&cyrillic);
    let other = tokens.alphabet("latin").count(&latin);
    let token = format!("token:{}:{}", bucket_of(typed), bucket_of(other));
    let whole = format!("whole:{}", ratio_of(typed, other));
    let cyrillic_letters = longest_run(&cyrillic, russian_letter)?;
    let latin_letters = longest_run(&latin, english_letter)?;
    let russian = tables.term("russian").count(cyrillic_letters).max(tables.term("cyrillic").count(cyrillic_letters));
    let english = tables.term("english").count(latin_letters).max(tables.term("latin").count(latin_letters));
    let ratio = format!("ratio:{}", ratio_of(russian, english));
    let seen = format!("seen:{}:{}", flag(russian > 0), flag(english > 0));
    let shape = format!("shape:{}", alnum_shape(&item.original));
    let letters: Vec<char> = item.original.chars().filter(|&c| ucd::isalpha(c)).collect();
    let case = format!(
        "case:{}",
        if letters.iter().all(|&c| ucd::isupper(c)) {
            "upper"
        } else if !letters.iter().any(|&c| ucd::isupper(c)) {
            "lower"
        } else {
            "mixed"
        }
    );
    let tail = last(&item.field.before, VARIABLE_CODE_CONTEXT_CHARACTERS);
    let words = pystr::split(&item.field.before);
    let following = pystr::casefold(first_word(&item.field.after));
    let before = format!("before:{}:{}", dominant(&pystr::casefold(tail)), words.last().map_or("start", |word| script(&pystr::casefold(word))));
    let code = format!("code:{}:digit:{}", flag(any_in(VARIABLE_CODE_SIGNS, tail)), flag(tail.chars().any(ucd::isdigit)));
    let boundary = punctuation_boundary(item);
    let after = format!("after:{}:{}", item.after_origin.as_str(), script(&following));
    let rest = [case, before, code, boundary, after];
    let mut names = vec!["bias".to_string(), token, whole.clone(), shape.clone(), ratio.clone(), seen.clone()];
    names.extend(rest.iter().cloned());
    names.push(format!("{shape}:{whole}"));
    names.push(format!("{shape}:{ratio}"));
    names.push(format!("{whole}:{ratio}"));
    names.push(format!("{ratio}:{seen}"));
    for head in [&shape, &whole, &ratio] {
        names.extend(rest.iter().map(|name| format!("{head}:{name}")));
    }
    Ok(head_pairs(ALNUM_FEATURE_PREFIX, &names, ""))
}

/// A token of BRIEF_HEAD_LETTERS Latin letters with no letter before or after it in the field, whose keys in the Russian
/// layout are Russian letters: a short message typed in the English layout, `ytn` for `нет`, `jr` for `ок`, `tot` for
/// `еще`. The message-start head starts at four letters, because at three it turned Russian words typed as meant into Latin
/// (`чук`, `дес`): this head answers Latin letters alone, and a Russian token stays with the frozen model. The question
/// reads only the typed token and the text around it, as the trainer and the GPU path route frames by them (head_question,
/// context_action_cuda.heads_question).
pub fn brief_question(original: &str, before: &str, after: &str) -> bool {
    if !(BRIEF_HEAD_LETTERS.contains(&count(original)) && original.is_ascii() && pystr::isalpha(original)) {
        return false;
    }
    let cyrillic = translate(original, "us", "ru");
    pystr::isalpha(&cyrillic) && !cyrillic.chars().any(|c| c.is_ascii()) && !has_alpha(before) && !has_alpha(after)
}

/// The larger of two counts of the Russian reading and of the Latin one, as the brief heads read them.
fn brief_counts(latin: &str, cyrillic: &str, tables: &TermTables) -> (i64, i64) {
    let (latin, cyrillic) = (pystr::casefold(latin), pystr::casefold(cyrillic));
    let russian = tables.term("russian").count(&cyrillic).max(tables.term("cyrillic").count(&cyrillic));
    let other = tables.term("english").count(&latin).max(tables.term("latin").count(&latin));
    (russian as i64, other as i64)
}

/// Whether the counts settle a short Latin token as the keys of a Russian word: its Cyrillic reading counted in Russian
/// text at least BRIEF_SETTLED_MIN_COUNT times and BRIEF_SETTLED_DOMINANCE times as often as the token in English prose or
/// Russian technical text, where it occurs fewer than BRIEF_SETTLED_OTHER_MAXIMUM times (`ytn` and `нет`, `gy` and `пн`), as
/// context_deferral.counted_reading settles the calibration's lone words with the Latin count of English prose added. `ds`
/// (`вы`, 108 Latin counts), `ij` (`шо`, 84 against 29) are not settled.
pub fn brief_settled(latin: &str, cyrillic: &str, tables: &TermTables) -> bool {
    let (russian, other) = brief_counts(latin, cyrillic, tables);
    russian >= BRIEF_SETTLED_MIN_COUNT && other < BRIEF_SETTLED_OTHER_MAXIMUM && russian >= BRIEF_SETTLED_DOMINANCE * other
}

/// Whether the counts settle a short Latin token a lexicon knows as the keys of a Russian word all the same: its Cyrillic
/// reading counted in Russian text at least BRIEF_KNOWN_MIN_COUNT times and BRIEF_KNOWN_DOMINANCE times as often as the
/// token in English prose or Russian technical text. The English lexicon knows `jr` (`Jr`), which English prose uses 13
/// times and Russian text `ок` 7 180 times; `ha` (28 against 273) stays a word.
pub fn brief_dominant(latin: &str, cyrillic: &str, tables: &TermTables) -> bool {
    let (russian, other) = brief_counts(latin, cyrillic, tables);
    russian >= BRIEF_KNOWN_MIN_COUNT && russian >= BRIEF_KNOWN_DOMINANCE * other && other < BRIEF_SETTLED_OTHER_MAXIMUM
}

/// A short message typed in the English layout (brief_question) whose Russian reading outnumbers its keys by far
/// (brief_dominant): `ytn` for `нет`, and `jr` for `ок`, which the English lexicon knows as `Jr`. The question reads only
/// the typed token and the text around it, as the trainer and the GPU path route frames by them.
pub fn known_reply_question(original: &str, alternative: &str, before: &str, after: &str, tables: &TermTables) -> bool {
    brief_question(original, before, after) && brief_dominant(original, alternative, tables)
}

/// The known-reply head's evidence: whether a lexicon of the profile knows the token as typed, the boundary that ended it,
/// its case and the application, the first read with each of the others. No count: every token of the class is counted far
/// more often in its Russian reading, and the frozen model's lone-word head, which reads the application, suggested such a
/// token sent alone where the lexicon knew it.
fn known_reply_evidence(item: &ContextEvidence) -> Pairs {
    let application = pystr::casefold(&item.field.application);
    let application = ascii_tokens(first(&application, ACTION_FEATURE_APPLICATION_NAME_CHARACTERS)).first().copied().unwrap_or("none").to_string();
    let known = format!("known:{}", flag(item.source_known));
    let rest = [format!("trigger:{}", item.trigger), format!("case:{}", short_case(&item.original)), format!("app:{application}")];
    let mut names = vec!["bias".to_string(), known.clone()];
    names.extend(rest.iter().cloned());
    names.extend(rest.iter().map(|name| format!("{known}:{name}")));
    head_pairs(KNOWN_REPLY_FEATURE_PREFIX, &names, "")
}

/// `"upper" if text.isupper() else "title" if text[:1].isupper() else "lower"`.
fn short_case(text: &str) -> &'static str {
    if pystr::isupper(text) { "upper" } else if pystr::isupper(first_char(text)) { "title" } else { "lower" }
}

/// The short-message head's evidence: the binary order of magnitude by which how often Russian prose uses the Cyrillic
/// reading outnumbers how often English prose or Russian technical text uses the Latin one (BRIEF_RATIO_BOUND at most
/// either way), whether either reading occurs at all, the boundary that ended the token and its case. The orders of each
/// count on their own told few tokens apart, and the head learnt them by heart: its development loss rose from the second
/// epoch on, and it turned `ye` sent alone, which the frozen model converted to `ну`, into a suggestion (experiment E9).
fn brief_evidence(item: &ContextEvidence, tables: &TermTables) -> Pairs {
    let latin = pystr::casefold(&item.original);
    let cyrillic = pystr::casefold(&item.alternative);
    let russian = bit_length(tables.term("russian").count(&cyrillic));
    let other = bit_length(tables.term("english").count(&latin).max(tables.term("latin").count(&latin)));
    let ratio = clamp_int(russian - other, BRIEF_RATIO_BOUND);
    let seen = format!("seen:{}:{}", flag(russian > 0), flag(other > 0));
    let case = short_case(&item.original);
    let names = [
        "bias".to_string(),
        format!("ratio:{ratio}"),
        seen.clone(),
        format!("trigger:{}", item.trigger),
        format!("case:{case}"),
        format!("ratio:{ratio}:trigger:{}", item.trigger),
        format!("ratio:{ratio}:case:{case}"),
        format!("{seen}:trigger:{}", item.trigger),
    ];
    head_pairs(BRIEF_FEATURE_PREFIX, &names, "")
}

const RUSSIAN_LETTERS: &str = "абвгдеёжзийклмнопрстуфхцчшщъыьэюя";
const ENGLISH_LETTERS: &str = "abcdefghijklmnopqrstuvwxyz";
/// Signs stripped off both ends of a reading before it is looked up, as the term tables were counted.
const EDGE_SIGNS: &str = ".,!?:;\"'()[]{}<>«»-";

fn magnitude(count: u64, bound: i64) -> i64 {
    bit_length(count).min(bound)
}

/// How often the commonest word one edit from `word` occurs in the prose of its language (the term table's `russian` or
/// `english`): a letter missed, one extra, one wrong, or two neighbours swapped; 0 for a word of anything but letters.
/// `тыв` is `ты` with a letter extra, which Russian prose uses 99 523 times. The words a word of three letters or more is
/// one typo from are read for the readings of the hypothesis question only: almost every short string has some neighbour,
/// and how common the commonest is tells a typo of a common word from a string of keys.
pub fn nearest_count(word: &str, language: &str, tables: &TermTables) -> u64 {
    let table = tables.term(language);
    let letters: Vec<char> = if language == "russian" { RUSSIAN_LETTERS } else { ENGLISH_LETTERS }.chars().collect();
    let word = pystr::casefold(pystr::strip_chars(word, EDGE_SIGNS));
    if word.is_empty() || !word.chars().all(|c| letters.contains(&c)) {
        return 0;
    }
    let chars: Vec<char> = word.chars().collect();
    let mut best = 0;
    let mut candidate = String::new();
    let mut look = |head: &[char], middle: &[char], tail: &[char]| {
        candidate.clear();
        candidate.extend(head);
        candidate.extend(middle);
        candidate.extend(tail);
        best = best.max(table.count(&candidate));
    };
    for index in 0..=chars.len() {
        let (head, tail) = (&chars[..index], &chars[index..]);
        for &letter in &letters {
            look(head, &[letter], tail);
        }
        if let Some(&first_tail) = tail.first() {
            look(head, &[], &tail[1..]);
            for &letter in &letters {
                if letter != first_tail {
                    look(head, &[letter], &tail[1..]);
                }
            }
            // swapped = tail[1:][:1] + tail[:1]
            if tail.len() > 1 {
                let swapped = [tail[1], tail[0]];
                if swapped != tail[..2] {
                    look(head, &swapped, &tail[2..]);
                }
            }
        }
    }
    best
}

fn prose_count(word: &str, language: &str, tables: &TermTables) -> u64 {
    tables.term(language).count(&pystr::casefold(pystr::strip_chars(word, EDGE_SIGNS)))
}

/// The question the engine asks about the next word of a waiting word with that word's other reading before it
/// (engine._decide_after_waiting_word): the hypothesis head's class. Only the question says so (before_origin); the same
/// text before a word is typed when the layout was switched by hand after a word of the other script, and that word is the
/// frozen model's.
pub fn hypothesis_question(item: &ContextEvidence) -> bool {
    item.before_origin == BeforeOrigin::PlannedPreviousReading
}

fn magnitude_ratio(first: u64, second: u64) -> i64 {
    clamp_int(magnitude(first, HYPOTHESIS_MAGNITUDE_MAX) - magnitude(second, HYPOTHESIS_MAGNITUDE_MAX), HYPOTHESIS_RATIO_BOUND)
}

/// The hypothesis head's evidence, all it reads (no copy of the shared features, which read the reading before the word as
/// typed text: `before:script:en` and the identifier `nsd` turned `тыв` after `tot ` into `nsd`): by how many binary orders
/// of magnitude the prose of each reading's language uses the word as typed more than the other reading, and the commonest
/// word one edit from each (nearest_count) more than the other's; whether either reading is a known identifier; by how much
/// the waiting word as typed is commoner in its language than the reading put before the word (`еще` against `tot`); the
/// token's letters, the band of the detector's score delta and the boundary.
fn hypothesis_evidence(item: &ContextEvidence, tables: &TermTables) -> Pairs {
    let (typed_language, other_language) = if item.source_group == 1 { ("russian", "english") } else { ("english", "russian") };
    let typed_count = prose_count(&item.original, typed_language, tables);
    let other_count = prose_count(&item.alternative, other_language, tables);
    let typed_near = typed_count.max(nearest_count(&item.original, typed_language, tables));
    let other_near = other_count.max(nearest_count(&item.alternative, other_language, tables));
    let words = words_of(&item.field.before);
    let reading = words.last().copied().unwrap_or("");
    let waiting = if item.source_group == 1 { translate(reading, "us", "ru") } else { translate(reading, "ru", "us") };
    let word = format!("word:{}", magnitude_ratio(typed_count, other_count));
    let near = format!("near:{}", magnitude_ratio(typed_near, other_near));
    let seen = format!("seen:{}:{}", flag(typed_count > 0), flag(other_count > 0));
    let identifier = format!("identifier:{}:{}", flag(item.source_identifier), flag(item.target_identifier));
    let held = format!("waiting:{}", magnitude_ratio(prose_count(&waiting, typed_language, tables), prose_count(reading, other_language, tables)));
    let letters = format!("letters:{}", item.original.chars().filter(|&c| ucd::isalpha(c)).count().min(ACTION_FEATURE_LENGTH_BUCKET_MAX_CHARACTERS));
    let delta = format!("delta:{}", band(item.score_delta, START_DELTA_BAND_WIDTH, START_DELTA_BANDS));
    let boundary = format!("boundary:{}", if item.trigger == "space" { "space" } else { "end" });
    let names = [
        "bias".to_string(),
        word.clone(),
        near.clone(),
        seen.clone(),
        identifier.clone(),
        held.clone(),
        letters.clone(),
        delta,
        boundary,
        format!("{word}:{near}"),
        format!("{near}:{identifier}"),
        format!("{near}:{held}"),
        format!("{near}:{letters}"),
        format!("{seen}:{identifier}"),
    ];
    head_pairs(HYPOTHESIS_FEATURE_PREFIX, &names, &format!(":direction:{}", item.source_group))
}

/// A token of SHORT_HEAD_LETTERS Russian letters, not all capitals, whose keys are Latin letters, after text with more
/// Cyrillic letters than Latin ones, that Russian text uses at least in bucket SHORT_HEAD_MIN_RUSSIAN_BUCKET (web and news
/// text, prose or technical text) while English prose and Russian technical text use its Latin reading at most in bucket
/// SHORT_HEAD_MAX_LATIN_BUCKET: the short-token head's class (the lexicon's own words are left out where the features are
/// made). `кз` (68 in web and news text, `rp` 23 in technical text) and `дз` are; `ьк` (`mr`), `ин` (`by`), `сс` (`cc`) and
/// `ср` (`ch`), whose Latin readings are common, are the frozen model's, which converted them as meant, and so is a token
/// Russian text hardly uses (`ге`, `сщ`). Capitals are the abbreviation head's, a word of one letter the letter heads', a
/// term of Russian technical text the term head's. The question reads only the typed token, the text before it and the
/// packaged tables, as the trainer and the GPU path route frames by them (head_question, context_action_cuda.heads_question).
pub fn short_question(original: &str, before: &str, tables: &TermTables) -> Checked<bool> {
    if !(SHORT_HEAD_LETTERS.contains(&count(original)) && all_cyrillic(original) && !pystr::isupper(original)) {
        return Ok(false);
    }
    let latin = translate(original, "ru", "us");
    if !(latin.is_ascii() && pystr::isalpha(&latin) && dominant(&pystr::casefold(last(before, ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS))) == "ru") {
        return Ok(false);
    }
    let cyrillic = pystr::casefold(original);
    let short = tables.short().map_err(|_| FeatureError("invalid term frequency table"))?;
    let russian = short.alphabet("cyrillic").count(&cyrillic).max(tables.term("russian").count(&cyrillic)).max(tables.term("cyrillic").count(&cyrillic));
    let folded = pystr::casefold(&latin);
    let other = tables.term("latin").count(&folded).max(tables.term("english").count(&folded));
    Ok(bucket_of(russian) >= SHORT_HEAD_MIN_RUSSIAN_BUCKET && bucket_of(other) <= SHORT_HEAD_MAX_LATIN_BUCKET && !term_question(original, before, tables))
}

/// The short-token head's evidence, all it reads: the binary order of magnitude of how often Russian web and news text
/// writes the token (a table of its own, context-short-frequency.json), Russian prose and technical text write it, Russian
/// technical text writes its Latin reading and English prose uses it, and by how much the larger Russian count outnumbers
/// the larger Latin one; whether the English lexicon knows the Latin reading and the identifier index holds it (`rp` is a
/// word to it); the token's letters, the boundary, how the text after it was given and the band of the detector's score
/// delta. No letter of its own: an abbreviation head reading the letters learned the corpus's tokens by heart and turned
/// `ТЗ` into `NP`.
fn short_evidence(item: &ContextEvidence, tables: &TermTables) -> Checked<Pairs> {
    let cyrillic = pystr::casefold(&item.original);
    let latin = pystr::casefold(&item.alternative);
    let web = tables.short().map_err(|_| FeatureError("invalid term frequency table"))?.alphabet("cyrillic").count(&cyrillic);
    let russian = tables.term("russian").count(&cyrillic).max(tables.term("cyrillic").count(&cyrillic));
    let (technical, english) = (tables.term("latin").count(&latin), tables.term("english").count(&latin));
    let magnitudes = [
        ("web", magnitude(web, SHORT_MAGNITUDE_MAX)),
        ("russian", magnitude(russian, SHORT_MAGNITUDE_MAX)),
        ("latin", magnitude(technical, SHORT_MAGNITUDE_MAX)),
        ("english", magnitude(english, SHORT_MAGNITUDE_MAX)),
    ];
    let difference = magnitudes[0].1.max(magnitudes[1].1) - magnitudes[2].1.max(magnitudes[3].1);
    let ratio = format!("ratio:{}", clamp_int(difference, SHORT_RATIO_BOUND));
    let known = format!("known:{}:identifier:{}", flag(item.target_known), flag(item.target_identifier));
    let letters = format!("letters:{}", count(&item.original));
    let seen = format!("seen:{}", [web, russian, technical, english].iter().map(|&value| flag(value > 0).to_string()).collect::<Vec<_>>().join(":"));
    let boundary = format!("boundary:{}", if item.trigger == "space" { "space" } else { "end" });
    let mut names = vec!["bias".to_string()];
    names.extend(magnitudes.iter().map(|(name, value)| format!("{name}:{value}")));
    names.push(ratio.clone());
    names.push(seen.clone());
    names.push(known.clone());
    names.push(letters.clone());
    names.push(boundary.clone());
    names.push(format!("after:{}", item.after_origin.as_str()));
    names.push(format!("delta:{}", band(item.score_delta, START_DELTA_BAND_WIDTH, START_DELTA_BANDS)));
    names.push(format!("{ratio}:{known}"));
    names.push(format!("{ratio}:{letters}"));
    names.push(format!("{ratio}:{boundary}"));
    names.push(format!("web:{}:latin:{}", magnitudes[0].1, magnitudes[2].1));
    names.push(format!("{seen}:{known}"));
    Ok(head_pairs(SHORT_FEATURE_PREFIX, &names, ""))
}

/// A token of at least SLIP_HEAD_MIN_LETTERS Latin letters after a line with more Cyrillic letters than Latin ones, whose
/// Latin reading English prose and Russian technical text use at most in bucket SLIP_HEAD_MAX_LATIN_BUCKET and whose
/// Cyrillic reading Russian prose uses: the slip head's class (a word the lexicon knows or the identifier index holds is
/// left out where the features are made). It is a word of Russian prose typed with the English layout on, the layout having
/// slipped with no language boundary (`cgtie` for `спешу` after `я `); a term Russian technical text writes in Latin letters
/// (`gtk`, `env`) stays with the frozen model. The question reads only the typed token, the text before it and the packaged
/// term table, as the trainer and the GPU path route frames by them (head_question, context_action_cuda.heads_question).
pub fn slip_question(original: &str, before: &str, tables: &TermTables) -> bool {
    if !(count(original) >= SLIP_HEAD_MIN_LETTERS && original.is_ascii() && pystr::isalpha(original)) {
        return false;
    }
    let line = before.rsplit('\n').next().unwrap_or("");
    if dominant(&pystr::casefold(last(line, ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS))) != "ru" {
        return false;
    }
    let cyrillic = pystr::casefold(&translate(original, "us", "ru"));
    let small = |alphabet: &str| term_bucket(original, alphabet, tables).parse::<usize>().is_ok_and(|bucket| bucket <= SLIP_HEAD_MAX_LATIN_BUCKET);
    all_cyrillic(&cyrillic) && tables.term("russian").count(&cyrillic) > 0 && small("latin") && small("english")
}

/// The slip head's evidence, all it reads (no copy of the shared features, which hold the frozen model's keep weights for
/// unknown Latin tokens after Russian prose): by how many binary orders of magnitude Russian prose uses the Cyrillic
/// reading more than English prose or Russian technical text use the Latin one, and the commonest word one edit from each
/// more than the other's (nearest_count); whether the Russian lexicon knows the Cyrillic reading; the detector's verdict and
/// the band of its probability; the band of the orthotactic margin; how often Russian prose uses the word before the token
/// (`я` against a technical word); the token's letters and case, the band of the score delta and how the text after it was
/// given. Nothing of the boundary: the same word asked at a pause and at its space gets the same answer.
fn slip_evidence(item: &ContextEvidence, tables: &TermTables) -> Pairs {
    let latin = pystr::casefold(&item.original);
    let cyrillic = pystr::casefold(&item.alternative);
    let russian = tables.term("russian").count(&cyrillic);
    let english = tables.term("english").count(&latin).max(tables.term("latin").count(&latin));
    let russian_near = russian.max(nearest_count(&cyrillic, "russian", tables));
    let english_near = english.max(nearest_count(&latin, "english", tables));
    let probability = match item.model_probability {
        None => "na".to_string(),
        Some(probability) => (((probability * SLIP_PROBABILITY_BANDS as f64).floor()).clamp(0.0, (SLIP_PROBABILITY_BANDS - 1) as f64) as i64).to_string(),
    };
    let margin = match (item.ortho_score, item.ortho_threshold) {
        (Some(score), Some(threshold)) => band(score - threshold, SLIP_ORTHO_BAND_WIDTH, SLIP_ORTHO_BANDS).to_string(),
        _ => "na".to_string(),
    };
    let line = item.field.before.rsplit('\n').next().unwrap_or("");
    let words = words_of(line);
    let previous = words.last().map_or(0, |word| magnitude(tables.term("russian").count(&pystr::casefold(word)), HYPOTHESIS_MAGNITUDE_MAX));
    let word = format!("word:{}", magnitude_ratio(russian, english));
    let near = format!("near:{}", magnitude_ratio(russian_near, english_near));
    let known = format!("known:{}", flag(item.target_known));
    let detector = format!("detector:{}:{}", flag(item.baseline_convert), probability);
    let ortho = format!("ortho:{margin}");
    let before = format!("before:{}", previous.div_euclid(SLIP_BEFORE_MAGNITUDE_STEP));
    let letters = format!("letters:{}", count(&item.original).min(ACTION_FEATURE_LENGTH_BUCKET_MAX_CHARACTERS));
    let case = format!("case:{}", short_case(&item.original));
    let names = [
        "bias".to_string(),
        word.clone(),
        near.clone(),
        known.clone(),
        detector.clone(),
        ortho.clone(),
        before.clone(),
        letters.clone(),
        format!("delta:{}", band(item.score_delta, START_DELTA_BAND_WIDTH, START_DELTA_BANDS)),
        case,
        format!("after:{}", item.after_origin.as_str()),
        format!("{word}:{near}"),
        format!("{word}:{known}"),
        format!("{near}:{detector}"),
        format!("{word}:{before}"),
        format!("{near}:{letters}"),
        format!("{ortho}:{known}"),
    ];
    head_pairs(SLIP_FEATURE_PREFIX, &names, "")
}
