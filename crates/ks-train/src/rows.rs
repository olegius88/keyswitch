//! What a frame of the action corpus is (`ActionRow`), the words of a text (`WORDS`), the pools curricula draw their
//! contexts from, and the frames of the natural corpus rows (`action_rows`).
use ks_cast::To as _;
use crate::corpus::CorpusRow;
use crate::physical::{py_repr, reading, ring_point, sha256, translated, variant_choice};
use crate::tools::Tools;
use ks_backend::{FieldContext, FieldRole};
use ks_context::context_model::{AfterOrigin, BeforeOrigin, ContextAction};
use ks_text::pystr;
use std::collections::{BTreeSet, HashMap};

pub const APPLICATIONS: [&str; 4] = ["Telegram", "Code", "chrome", "UnseenEditor"];
pub const TRIGGERS: [&str; 6] = ["space", "space", "enter", "punctuation", "tab", "pause"];
pub const PUBLIC_TRAINING: &str = "public-training";
const FIELD_AFTER_SAMPLE_MODULUS: usize = 8;
const BOUNDARY_EVENT_CHOICES: usize = 2;
const MAX_CONTEXTS_PER_FAMILY: i64 = 2;
const LAYOUT_GROUP_COUNT: usize = 2;
const CONTEXT_RING_POINTS: usize = 16;
const CAPITAL_CITATION_PUNCTUATION: &str = ",.";
const CAPITAL_CITATION_MIN_LETTERS: usize = 3;
const CAPITAL_CITATION_MAX_LETTERS: usize = 5;
const CAPITAL_CITATION_RUSSIAN_BUCKET: &str = "0";
const CITATION_SIGN_HEADS: [&str; 3] = ["``", "'", "\""];
const QUOTE_TAIL: &str = "\"";
const QUOTE_TAIL_MODULUS: usize = 4;
const STRANDED_PREVIOUS_MIN_LETTERS: usize = 2;
const STRANDED_PREVIOUS_WEIGHT: f64 = 2.0;

#[derive(Clone, Debug, PartialEq)]
pub struct ActionRow {
    pub identifier: String,
    pub original: String,
    pub group: i64,
    pub field: FieldContext,
    pub trigger: &'static str,
    pub literal_tail: String,
    pub action: ContextAction,
    pub category: String,
    pub boundary_text: String,
    pub sample_weight: f64,
    pub after_origin: AfterOrigin,
    pub parent_family: String,
    /// Asked again as the engine asks a converted word whose next word went back.
    pub revisit: bool,
    /// The next word of a waiting word asked with that word's other reading before it.
    pub before_origin: BeforeOrigin,
}

impl ActionRow {
    pub fn new(identifier: impl Into<String>, original: impl Into<String>, group: i64, field: FieldContext, trigger: &'static str, action: ContextAction, category: impl Into<String>, boundary_text: impl Into<String>) -> ActionRow {
        ActionRow {
            identifier: identifier.into(),
            original: original.into(),
            group,
            field,
            trigger,
            literal_tail: String::new(),
            action,
            category: category.into(),
            boundary_text: boundary_text.into(),
            sample_weight: 1.0,
            after_origin: AfterOrigin::None,
            parent_family: String::new(),
            revisit: false,
            before_origin: BeforeOrigin::Field,
        }
    }

    #[must_use]
    pub fn weighing(mut self, sample_weight: f64) -> ActionRow {
        self.sample_weight = sample_weight;
        self
    }

    #[must_use]
    pub fn originating(mut self, after_origin: AfterOrigin) -> ActionRow {
        self.after_origin = after_origin;
        self
    }
}

/// `FieldContext(application, field_id, before, after, role)`.
#[must_use]
pub fn field_of(application: &str, field_id: &str, before: &str, after: &str, role: FieldRole) -> FieldContext {
    let mut field = FieldContext::new(application, field_id);
    before.clone_into(&mut field.before);
    after.clone_into(&mut field.after);
    field.role = role;
    field
}

#[must_use]
pub fn public_field(application: &str, before: &str, after: &str) -> FieldContext {
    field_of(application, PUBLIC_TRAINING, before, after, FieldRole::Unknown)
}

#[must_use]
pub fn application_of(identifier: &str) -> &'static str {
    APPLICATIONS[variant_choice(identifier, "application", APPLICATIONS.len())]
}

/// `training_order`: corpus sources interleaved deterministically before optimizer updates. The sort is stable.
pub fn training_order(rows: &mut [ActionRow]) {
    rows.sort_by_cached_key(|row| sha256(&row.identifier));
}

fn word_letter(character: char) -> bool {
    character.is_ascii_alphabetic() || ('А'..='я').contains(&character) || character == 'Ё' || character == 'ё'
}

fn run_end(text: &str, start: usize) -> usize {
    let mut end = start;
    for (offset, character) in text.get(start..).unwrap_or_default().char_indices() {
        if !word_letter(character) {
            return start + offset;
        }
        end = start + offset + character.len_utf8();
    }
    end
}

/// `[A-Za-zА-Яа-яЁё]+(?:['’\-][A-Za-zА-Яа-яЁё]+)*` from byte offset `start`, which must begin it: where it ends.
fn word_end(text: &str, start: usize) -> usize {
    let mut end = run_end(text, start);
    loop {
        let Some(separator) = text.get(end..).unwrap_or_default().chars().next().filter(|character| matches!(character, '\'' | '’' | '-')) else { return end };
        let next = end + separator.len_utf8();
        match text.get(next..).unwrap_or_default().chars().next() {
            Some(character) if word_letter(character) => end = run_end(text, next),
            _ => return end,
        }
    }
}

/// `WORDS.finditer(text)`: the byte spans of the words.
#[must_use]
pub fn word_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut position = 0;
    while let Some((offset, _)) = text.get(position..).unwrap_or_default().char_indices().find(|(_, character)| word_letter(*character)) {
        let start = position + offset;
        let end = word_end(text, start);
        spans.push((start, end));
        position = end;
    }
    spans
}

/// `WORDS.findall(text)`.
#[must_use]
pub fn words_of(text: &str) -> Vec<&str> {
    word_spans(text).into_iter().map(|(start, end)| text.get(start..end).unwrap_or_default()).collect()
}

/// `WORDS.search(text) is not None`.
pub fn has_word(text: &str) -> bool {
    text.chars().any(word_letter)
}

/// `WORDS.match(text)`: the word at the very start of the text.
#[must_use]
pub fn word_at_start(text: &str) -> Option<&str> {
    text.chars().next().filter(|character| word_letter(*character))?;
    Some(text.get(..word_end(text, 0)).unwrap_or_default())
}

/// `JOINED_WORDS.finditer(text)`: words joined by hyphens, each with its byte span.
pub fn joined_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut position = 0;
    while let Some((offset, _)) = text.get(position..).unwrap_or_default().char_indices().find(|(_, character)| word_letter(*character)) {
        let start = position + offset;
        let mut end = run_end(text, start);
        let first = end;
        while text.get(end..).unwrap_or_default().starts_with('-') && text.get(end + 1..).unwrap_or_default().chars().next().is_some_and(word_letter) {
            end = run_end(text, end + 1);
        }
        if end > first {
            spans.push((start, end));
        }
        position = end;
    }
    spans
}

#[must_use]
pub fn is_alpha(character: char) -> bool {
    let mut buffer = [0_u8; 4];
    pystr::isalpha(character.encode_utf8(&mut buffer))
}

#[must_use]
pub fn alpha_only(text: &str) -> String {
    text.chars().filter(|character| is_alpha(*character)).collect()
}

/// `"а" <= char.casefold() <= "я" or char.casefold() == "ё"`.
#[must_use]
pub fn cyrillic_letter(character: char) -> bool {
    if character.is_ascii() {
        return false;
    }
    let mut buffer = [0_u8; 4];
    let folded = pystr::casefold(character.encode_utf8(&mut buffer));
    ("а" <= folded.as_str() && folded.as_str() <= "я") || folded == "ё"
}

/// `_cyrillic`.
pub fn cyrillic(word: &str) -> bool {
    word.chars().all(cyrillic_letter)
}

/// `natural_mixed_contexts`: left contexts of a split's own rows, per language, for the insertion frames, sorted.
#[must_use]
pub fn natural_mixed_contexts(rows: &[CorpusRow]) -> [Vec<String>; 2] {
    let mut pools: [BTreeSet<&str>; 2] = [BTreeSet::new(), BTreeSet::new()];
    for row in rows {
        let Some(group) = row.group.filter(|group| *group == 0 || *group == 1) else { continue };
        if !row.layout_representable {
            continue;
        }
        let words = words_of(&row.before);
        if words.is_empty() {
            continue;
        }
        let script: Vec<bool> = words.iter().map(|word| word.chars().any(cyrillic_letter)).collect();
        let qualifies = if group == 1 { script.iter().all(|value| *value) } else { !script.iter().any(|value| *value) };
        if qualifies {
            pools[group.to::<usize>()].insert(&row.before);
        }
    }
    [pools[0].iter().map(|text| (*text).to_owned()).collect(), pools[1].iter().map(|text| (*text).to_owned()).collect()]
}

/// How `f"{index}:{context}"` writes a context of a pool.
pub trait RingContext: Clone {
    fn written(&self) -> String;
}

impl RingContext for String {
    fn written(&self) -> String {
        self.clone()
    }
}

impl RingContext for (String, String) {
    fn written(&self) -> String {
        format!("({}, {})", py_repr(&self.0), py_repr(&self.1))
    }
}

/// `ContextRing`: a pool of contexts a curriculum draws from by consistent hashing, a frame keeping its context while
/// it stays in the pool.
pub struct ContextRing<C> {
    pool: Vec<C>,
    points: Vec<u64>,
    owners: Vec<u32>,
}

impl<C: RingContext> ContextRing<C> {
    #[must_use]
    pub fn new(pool: Vec<C>) -> ContextRing<C> {
        let mut ring: Vec<(u64, u32)> = Vec::with_capacity(pool.len() * CONTEXT_RING_POINTS);
        for (position, context) in pool.iter().enumerate() {
            let written = context.written();
            for index in 0..CONTEXT_RING_POINTS {
                ring.push((ring_point(&format!("{index}:{written}")), position.to::<u32>()));
            }
        }
        ring.sort_unstable();
        ContextRing { pool, points: ring.iter().map(|entry| entry.0).collect(), owners: ring.iter().map(|entry| entry.1).collect() }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.pool.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pool.is_empty()
    }

    #[must_use]
    pub fn choice(&self, identifier: &str, purpose: &str) -> &C {
        let point = ring_point(&format!("{purpose}:{identifier}"));
        let slot = self.points.partition_point(|value| *value < point) % self.points.len();
        &self.pool[self.owners[slot].to::<usize>()]
    }
}

/// `_varied_boundary`: the boundary a curriculum word ends at, by hash.
#[must_use]
pub fn varied_boundary(identifier: &str) -> (&'static str, String) {
    let trigger = TRIGGERS[variant_choice(identifier, "trigger", TRIGGERS.len())];
    let mut boundary = String::new();
    if trigger == "space" {
        boundary.push(' ');
    } else if trigger == "punctuation" {
        let marks: Vec<char> = CAPITAL_CITATION_PUNCTUATION.chars().collect();
        boundary.push(marks[variant_choice(identifier, "punctuation", marks.len())]);
    } else if (trigger == "enter" || trigger == "tab") && variant_choice(identifier, "boundary-event", BOUNDARY_EVENT_CHOICES) != 0 {
        boundary.push(if trigger == "enter" { '\n' } else { '\t' });
    }
    (trigger, boundary)
}

/// `select_rows`: languages balanced before scoring, at most two contexts per family.
#[must_use]
pub fn select_rows(rows: &[CorpusRow], maximum: usize) -> Vec<CorpusRow> {
    let mut ordered: Vec<&CorpusRow> = rows.iter().collect();
    ordered.sort_by_cached_key(|row| sha256(&row.identifier));
    let mut selected = Vec::new();
    let mut counts: HashMap<(i64, &str), i64> = HashMap::new();
    let mut languages = [0_usize; 2];
    for row in ordered {
        let Some(group) = row.group.filter(|group| *group == 0 || *group == 1) else { continue };
        if !row.layout_representable || translated(&row.original, group).is_err() {
            continue;
        }
        let count = counts.entry((group, &row.family)).or_insert(0);
        if languages[group.to::<usize>()] >= maximum / LAYOUT_GROUP_COUNT || *count >= MAX_CONTEXTS_PER_FAMILY {
            continue;
        }
        *count += 1;
        languages[group.to::<usize>()] += 1;
        selected.push(row.clone());
    }
    selected
}

/// `citation_shaped`: a word prose of the other language cites.
fn citation_shaped(tools: &Tools, original: &str, group: i64) -> bool {
    let letters = alpha_only(original);
    !letters.is_empty() && (pystr::isupper(&letters) || pystr::istitle(&letters) || !tools.plausible_reading(original, group))
}

/// `capital_citation`: a Latin abbreviation in capitals whose Cyrillic reading is a rare word, not a Russian abbreviation.
pub fn capital_citation(tools: &Tools, original: &str, alternate: &str) -> bool {
    let letters = pystr::len(&alpha_only(original));
    pystr::isascii(original)
        && pystr::isalpha(original)
        && pystr::isupper(original)
        && (CAPITAL_CITATION_MIN_LETTERS..=CAPITAL_CITATION_MAX_LETTERS).contains(&letters)
        && tools.plausible_reading(alternate, 1)
        && tools.term_bucket(alternate, "russian") == CAPITAL_CITATION_RUSSIAN_BUCKET
        && !tools.onboard_capitals().contains(&pystr::casefold(alternate))
}

/// `stranded_previous`: `before` with its last word as typed in the other layout, when that reading is a word there too.
fn stranded_previous(tools: &Tools, before: &str, group: i64) -> Option<String> {
    let words = words_of(before);
    let previous = *words.last()?;
    let head = pystr::rstrip(before);
    if !head.ends_with(previous) || pystr::len(previous) < STRANDED_PREVIOUS_MIN_LETTERS || !pystr::isalpha(previous) {
        return None;
    }
    let other = translated(previous, group).ok()?;
    if !pystr::isalpha(&other) || !tools.plausible_reading(&other, 1 - group) {
        return None;
    }
    Some(format!("{}{other}{}", head.get(..head.len() - previous.len()).unwrap_or_default(), before.get(head.len()..).unwrap_or_default()))
}

/// A curated letter, whose intent is observable: its own reading is a word or it is not.
#[must_use]
pub fn curated_letter(original: &str, alternate: &str) -> bool {
    let trusted = |text: &str| ks_lang::short_words::TRUSTED_SINGLE_LETTER_WORDS.contains(&pystr::casefold(text).as_str());
    pystr::len(original) == 1 && pystr::len(alternate) == 1 && (trusted(original) || trusted(alternate))
}

/// `action_rows`: natural surface forms are keep labels; layout corruption and spelling errors are declared interventions.
pub fn action_rows(tools: &Tools, rows: &[CorpusRow]) -> Vec<ActionRow> {
    use ContextAction::{Convert, Keep, Suggest, Wait};

    let mut result = Vec::new();
    let natural = natural_mixed_contexts(rows);
    let insertion: [[&str; 2]; 2] = [["в сообщении написано ", "в оригинале «Ночь» ("], ["the message says ", "in the original \"Night\" ("]];
    for row in rows {
        let group = row.group.filter(|group| *group == 0 || *group == 1).filter(|_| row.layout_representable).expect("action rows require a representable single-layout token");
        let alternate = reading(&row.original, group);
        let digest = sha256(&row.identifier);
        let number = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]).to::<usize>();
        let trigger = TRIGGERS[number % TRIGGERS.len()];
        let mut boundary = String::new();
        if trigger == "space" {
            let first: String = row.spacing.chars().take(1).collect();
            boundary = if first.is_empty() { " ".to_owned() } else { first };
        } else if trigger == "punctuation" {
            let mark = row.literal_tail.chars().find(|character| !pystr::isspace(character.encode_utf8(&mut [0_u8; 4])));
            boundary = mark.unwrap_or('.').to_string();
        } else if (trigger == "enter" || trigger == "tab") && variant_choice(&row.identifier, "boundary-event", BOUNDARY_EVENT_CHOICES) != 0 {
            if trigger == "enter" { "\n" } else { "\t" }.clone_into(&mut boundary);
        }
        let application = application_of(&row.identifier);
        let frame = |identifier: String, original: &str, frame_group: i64, field: FieldContext, action: ContextAction, category: &str| {
            ActionRow::new(identifier, original, frame_group, field, trigger, action, category, boundary.as_str())
        };
        for (context_name, before) in [("observed", row.before.as_str()), ("empty", "")] {
            // Lookahead is optional field evidence, never assumed available.
            let after = if context_name == "observed" && variant_choice(&row.identifier, "field-after", FIELD_AFTER_SAMPLE_MODULUS) == 0 { row.after.as_str() } else { "" };
            let field = public_field(application, before, after);
            let identity = format!("{}:{context_name}", row.identifier);
            let (mut keep_action, mut action) = (Keep, Convert);
            let alone = !has_word(before) && !has_word(after);
            if alone && !curated_letter(&row.original, &alternate) && tools.deferred_isolated(&row.original, &alternate, group) {
                // An isolated short reading has no observable intent label: both members take the same deferred action.
                action = if matches!(trigger, "enter" | "tab" | "punctuation") { Suggest } else { Wait };
                keep_action = action;
            }
            result.push(frame(format!("{identity}:keep"), &row.original, group, field.clone(), keep_action, "natural_surface"));
            result.push(frame(format!("{identity}:wrong"), &alternate, 1 - group, field, action, "layout_intervention"));
        }
        // A legitimate insertion may have neighbours in the other language.
        let contexts = insertion[group.to::<usize>()];
        let mixed = contexts[variant_choice(&row.identifier, "mixed-context", contexts.len())];
        let mixed_field = public_field(application, mixed, "");
        result.push(frame(format!("{}:mixed", row.identifier), &row.original, group, mixed_field.clone(), Keep, "mixed_language_insertion"));
        // The keys of a Latin abbreviation that spell one Russian text uses are that abbreviation after Russian prose.
        let attested = group == 0 && tools.attested_abbreviation(&alternate);
        if !attested {
            result.push(frame(format!("{}:mixed:wrong", row.identifier), &alternate, 1 - group, mixed_field, Convert, "mixed_language_layout_intervention"));
        }
        let other = &natural[(1 - group).to::<usize>()];
        // A capital citation's reading is one Russian prose knows (`plausible_reading(alternate, 1)`): with group 0 that is
        // the reading of the other layout.
        if !other.is_empty() && group == 0 && capital_citation(tools, &row.original, &alternate) {
            let chosen = &other[variant_choice(&row.identifier, "mixed-natural", other.len())];
            result.push(frame(format!("{}:mixed-capital", row.identifier), &row.original, group, public_field(application, chosen, ""), Keep, "mixed_language_insertion"));
        }
        if !other.is_empty() && group == 0 && citation_shaped(tools, &row.original, group) && !tools.plausible_reading(&alternate, 1 - group) {
            let chosen = &other[variant_choice(&row.identifier, "mixed-natural", other.len())];
            let natural_field = public_field(application, chosen, "");
            result.push(frame(format!("{}:mixed-natural", row.identifier), &row.original, group, natural_field.clone(), Keep, "mixed_language_insertion"));
            if !attested {
                result.push(frame(format!("{}:mixed-natural:wrong", row.identifier), &alternate, 1 - group, natural_field.clone(), Convert, "mixed_language_layout_intervention"));
            }
            // Edited prose opens the citation with a sign typed in the Latin layout.
            let head = CITATION_SIGN_HEADS[variant_choice(&row.identifier, "citation-head", CITATION_SIGN_HEADS.len())];
            let headed = format!("{head}{}", row.original);
            result.push(frame(format!("{}:mixed-natural:head", row.identifier), &headed, group, natural_field.clone(), Keep, "mixed_language_insertion"));
            result.push(frame(format!("{}:mixed-natural:head:wrong", row.identifier), &reading(&headed, group), 1 - group, natural_field, Convert, "mixed_language_layout_intervention"));
        }
        if let Some(stranded) = stranded_previous(tools, &row.before, group) {
            // A phrase typed whole in the other layout: the word after a word left as typed is still the phrase's.
            let last = *words_of(&stranded).last().expect("the stranded reading is a word");
            let head = pystr::rstrip(&stranded).len() - last.len();
            let opening = stranded.get(head..).unwrap_or_default().to_owned();
            for (suffix, left) in [("", stranded.as_str()), (":opening", opening.as_str())] {
                result.push(frame(format!("{}:stranded-previous{suffix}:wrong", row.identifier), &alternate, 1 - group, public_field(application, left, ""), Convert, "layout_intervention").weighing(STRANDED_PREVIOUS_WEIGHT));
            }
        }
        let isolated = !has_word(&row.before) && tools.deferred_isolated(&row.original, &alternate, group);
        if group == 1 && pystr::isalpha(&row.original) && !isolated && variant_choice(&row.identifier, "quote-tail", QUOTE_TAIL_MODULUS) == 0 {
            // A quotation closes with its quote typed in the layout of the word.
            let quoted = format!("{}{QUOTE_TAIL}", row.original);
            let quote_field = public_field(application, &row.before, "");
            result.push(frame(format!("{}:quote-tail", row.identifier), &quoted, group, quote_field.clone(), Keep, "natural_surface"));
            result.push(frame(format!("{}:quote-tail:wrong", row.identifier), &reading(&quoted, group), 1 - group, quote_field, Convert, "layout_intervention"));
        }
        for (index, typo) in crate::corpus::typo_variants(&row.original, &row.identifier).iter().enumerate() {
            result.push(frame(format!("{}:spelling:{index}", row.identifier), typo, group, public_field(application, &row.before, ""), Keep, "spelling_intervention"));
        }
    }
    result
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn words_as_the_pattern_finds_them() {
        assert_eq!(words_of("it's a well-known-  ёж, ’x"), ["it's", "a", "well-known", "ёж", "x"]);
        assert_eq!(word_at_start("  x"), None);
        assert_eq!(word_at_start("да-нет ну"), Some("да-нет"));
        assert_eq!(joined_spans("a b-c-d e- f-g").len(), 2);
        assert_eq!(words_of("Ёж и ёлка"), ["Ёж", "и", "ёлка"]);
    }

    #[test]
    fn frames_are_ordered_by_the_hash_of_their_names() {
        let frame = |name: &str| ActionRow::new(name, "дом", 1, public_field("Code", "", ""), "space", ContextAction::Keep, "test", " ");
        let mut rows = vec![frame("a"), frame("b"), frame("c"), frame("d")];
        training_order(&mut rows);
        let hashes: Vec<[u8; 32]> = rows.iter().map(|row| sha256(&row.identifier)).collect();
        assert!(hashes.windows(2).all(|pair| pair[0] <= pair[1]));
    }
}
