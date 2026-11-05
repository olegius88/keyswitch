//! The curricula that add frames to TRAIN and to the heads' own DEVELOPMENT: each answers a question the corpus's natural
//! frames do not hold, as `train_context_action_model.py` frames it. The reasons for each frame are in the module's
//! documentation of the model (`model/context_v3/README.md`) and in the Python trainer's history.
use ks_cast::To as _;
use crate::corpus::{CorpusRow, DEVELOPMENT, TRAIN, typo_variants};
use crate::physical::{hex_digest, reading, sha256, translated, variant_choice};
use crate::rows::{ActionRow, ContextRing, application_of, cyrillic, curated_letter, field_of, natural_mixed_contexts, public_field, varied_boundary, word_spans};
use crate::rows::capital_citation;
use crate::tools::{Lexicons, Options, Tools};
use ks_backend::FieldRole;
use ks_context::context_action_features as features;
use ks_context::context_model::{AfterOrigin, ContextAction};
use ks_lang::short_words::TRUSTED_SINGLE_LETTER_WORDS;
use ks_text::pystr;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use ContextAction::{Convert, Keep, Suggest, Wait};

const ABBREVIATION_SPLIT_MODULUS: usize = 5;
const JOINED_REPEAT_CAPITAL_CHOICES: usize = 2;
const JOINED_REPEAT_MAX_LETTERS: usize = 4;
const JOINED_REPEAT_MIN_LETTERS: usize = 2;
const JOINED_REPEAT_SPLIT_MODULUS: usize = 5;
const BRIEF_BOUNDARIES: [(&str, &str); 3] = [("space", " "), ("enter", "\n"), ("pause", "")];
const BRIEF_SPLIT_MODULUS: usize = 5;
const BRIEF_HEAD_LETTERS: [usize; 2] = [2, 3];
const FIRST_PAUSE_WORD_START_MIN_BUCKET: i64 = 6;
const TERM_CAPITALS_MAX_LETTERS: usize = 4;
const TERM_FRAGMENT_MIN_ENDING_BUCKET: i64 = 3;
const TERM_FRAGMENT_MIN_REST: usize = 2;
const TERM_ALONE_WAIT_MAX_LETTERS: usize = 2;
const TERM_REVISIT_FOLLOWERS: usize = 30;
const TERM_REVISIT_FOLLOWER_MAX_LETTERS: usize = 3;
const TERM_SPLIT_MODULUS: usize = 5;
const TERM_HEAD_ENDING_LETTERS: [usize; 2] = [2, 3];
const TERM_HEAD_TRAILING_SIGNS: &str = ".,";
const UNKNOWN_TOKEN_MAX_CHARACTERS: usize = 20;
const UNKNOWN_TOKEN_SPLIT_MODULUS: usize = 5;
const UNKNOWN_HEAD_MIN_CHARACTERS: usize = 4;
const UNKNOWN_HEAD_SIGN_KEY_LETTERS: [char; 7] = ['ё', 'б', 'э', 'х', 'ъ', 'ж', 'ю'];
const UNCOUNTED: u64 = 5;
const KEPT_NEIGHBOUR_CAPITALS_MODULUS: usize = 4;
const KEPT_NEIGHBOUR_FRAMES_PER_WORD: usize = 3;
const KEPT_NEIGHBOUR_MIN_LETTERS: usize = 2;
const KEPT_CONTEXT_WORD_MAX_CHARACTERS: usize = 5;
const LETTER_CURRICULUM_MESSAGE_START_MODULUS: usize = 3;
const LONE_WORD_BOUNDARIES: [(&str, &str); 9] = [("enter", "\n"), ("enter", ""), ("tab", "\t"), ("tab", ""), ("punctuation", "."), ("punctuation", ","), ("punctuation", "?"), ("punctuation", "!"), ("pause", "")];
const LONE_WORD_SPLIT_MODULUS: usize = 10;
const ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS: i64 = 512;
const PLANNED_CONTEXT_AFTER_MAX_CHARACTERS: usize = 64;
const PLANNED_CONTEXT_WORD_MAX_CHARACTERS: usize = 3;
const LAYOUT_GROUP_COUNT: usize = 2;

pub type Curriculum = (Vec<ActionRow>, Value);

/// `Counter`, written as `dict(sorted(counts.items()))`.
#[derive(Default)]
pub struct Counts(BTreeMap<String, i64>);

impl Counts {
    pub fn add(&mut self, name: impl Into<String>) {
        self.add_by(name, 1);
    }

    pub fn add_by(&mut self, name: impl Into<String>, count: i64) {
        *self.0.entry(name.into()).or_insert(0) += count;
    }

    pub fn get(&self, name: &str) -> i64 {
        self.0.get(name).copied().unwrap_or(0)
    }

    pub fn report(&self) -> Value {
        let mut map = Map::new();
        for (name, count) in &self.0 {
            map.insert(name.clone(), json!(count));
        }
        Value::Object(map)
    }
}

/// `before[-512:]`.
fn clipped(before: &str) -> &str {
    pystr::slice(before, Some(-ACTION_FEATURE_BEFORE_CONTEXT_CHARACTERS), None)
}

fn not_used() -> Curriculum {
    (Vec::new(), json!({"frames": 0, "scope": "not used"}))
}

/// Which split a family's hash gives it.
fn share_of(family: &str, modulus: usize) -> &'static str {
    if variant_choice(family, "split", modulus) == 0 { DEVELOPMENT } else { TRAIN }
}

/// `_monolingual_rows`: rows whose word and every word before it are in the script of the row's layout.
struct Monolingual<'a> {
    row: &'a CorpusRow,
    group: i64,
    text: String,
    tokens: Vec<(usize, usize)>,
}

fn monolingual_rows(source_rows: &[CorpusRow]) -> Vec<Monolingual<'_>> {
    let mut result = Vec::new();
    for row in source_rows {
        let Some(group) = row.group.filter(|group| *group == 0 || *group == 1) else { continue };
        if !row.layout_representable || !pystr::isalpha(&row.original) || cyrillic(&row.original) != (group == 1) {
            continue;
        }
        let text = format!("{}{}", row.before, row.original);
        let tokens = word_spans(&text);
        if tokens.iter().any(|(start, end)| cyrillic(&text[*start..*end]) != (group == 1)) {
            continue;
        }
        result.push(Monolingual { row, group, text, tokens });
    }
    result
}

/// `_russian_contexts`: the left context and the word of every Russian row of Russian words only, in a fixed order.
fn russian_contexts(source_rows: &[CorpusRow]) -> Vec<(String, String)> {
    let mut contexts: Vec<(String, String)> =
        monolingual_rows(source_rows).iter().filter(|row| row.group == 1).map(|row| (row.row.before.clone(), row.row.original.clone())).collect();
    contexts.sort();
    contexts
}

fn letters_only_lower(term: &str) -> bool {
    pystr::isascii(term) && pystr::isalpha(term) && pystr::islower(term)
}

/// `_counted_terms`: Latin words Russian technical text uses at least `minimum` times whose Cyrillic reading is nothing.
fn counted_terms(tools: &Tools, minimum: u64) -> Vec<String> {
    let mut terms = Vec::new();
    for term in tools.tables.term("latin").words() {
        if tools.count("latin", term) < minimum || !letters_only_lower(term) {
            continue;
        }
        if !(KEPT_NEIGHBOUR_MIN_LETTERS..=KEPT_CONTEXT_WORD_MAX_CHARACTERS).contains(&pystr::len(term)) {
            continue;
        }
        let other = reading(term, 0);
        if !pystr::isalpha(&other)
            || tools.plausible_reading(&other, 1)
            || tools.count("cyrillic", &other) >= UNCOUNTED
            || tools.count("russian", &other) >= UNCOUNTED
            || tools.is_refused(&[term, &other])
        {
            continue;
        }
        terms.push(term.to_owned());
    }
    terms
}

/// `_counted_abbreviations`: Cyrillic tokens Russian technical text uses far more often than their Latin keys.
fn counted_abbreviations(tools: &Tools, minimum: u64, dominance: f64) -> Vec<String> {
    let mut abbreviations = Vec::new();
    for token in tools.tables.term("cyrillic").words() {
        let count = tools.count("cyrillic", token);
        if count < minimum
            || !pystr::isalpha(token)
            || !cyrillic(token)
            || !(KEPT_NEIGHBOUR_MIN_LETTERS..=KEPT_CONTEXT_WORD_MAX_CHARACTERS).contains(&pystr::len(token))
            || tools.plausible_reading(token, 1)
        {
            continue;
        }
        let keys = reading(token, 1);
        if count.to::<f64>() < dominance * tools.count("latin", &keys).to::<f64>() || tools.is_refused(&[token, &keys]) {
            continue;
        }
        abbreviations.push(token.to_owned());
    }
    abbreviations
}

/// `_term_typed`: a counted term's keys in the Russian layout, one term in a few in capitals.
fn term_typed(term: &str) -> String {
    let capitals = variant_choice(&format!("kept-term:{term}"), "capitals", KEPT_NEIGHBOUR_CAPITALS_MODULUS) == 0;
    reading(&if capitals { pystr::upper(term) } else { term.to_owned() }, 0)
}

fn technical_terms(tools: &Tools, minimum: u64) -> Vec<String> {
    let mut terms: Vec<String> = tools
        .tables
        .term("latin")
        .words()
        .filter(|term| {
            let count = tools.count("latin", term);
            count >= minimum && count >= tools.count("english", term) && pystr::len(term) > 1 && letters_only_lower(term) && !tools.is_refused(&[term])
        })
        .map(str::to_owned)
        .collect();
    terms.sort();
    terms
}

/// `capital_citation_curriculum`.
pub fn capital_citation_curriculum(tools: &Tools, source_rows: &[CorpusRow], options: &Value, models: &Lexicons) -> Curriculum {
    let budgets = options.budgets("words_by_length");
    let weight = options.float("sample_weight");
    let contrast = options.is_true("lowercase_contrast");
    let [_, russian] = natural_mixed_contexts(source_rows);
    let contexts = ContextRing::new(russian);
    if contexts.is_empty() || !budgets.iter().any(|entry| entry.1 != 0) {
        return (Vec::new(), json!({"words": 0, "words_by_length": {}, "scope": "not used"}));
    }
    let mut candidates: HashMap<usize, Vec<(String, String)>> = HashMap::new();
    for word in models[1].frequencies.words().iter() {
        let length = pystr::len(word);
        if !budgets.iter().any(|entry| entry.0 == length) || !word.chars().all(|character| ('а'..='я').contains(&character) || character == 'ё') {
            continue;
        }
        let upper = pystr::upper(word);
        let Ok(latin) = translated(&upper, 1) else { continue };
        if !capital_citation(tools, &latin, &upper) || tools.is_refused(&[word, &latin]) {
            continue;
        }
        candidates.entry(length).or_default().push((word.to_owned(), latin));
    }
    let mut rows = Vec::new();
    let mut chosen = Map::new();
    let mut available = Map::new();
    for &(length, budget) in &budgets {
        let mut ranked = candidates.remove(&length).unwrap_or_default();
        available.insert(length.to_string(), json!(ranked.len()));
        ranked.sort_by_cached_key(|pair| sha256(&format!("capital-citation:{}", pair.0)));
        ranked.truncate(budget);
        chosen.insert(length.to_string(), json!(ranked.iter().map(|(word, latin)| format!("{word}/{latin}")).collect::<Vec<_>>()));
        for (word, latin) in &ranked {
            let identifier = format!("capital-citation:{word}");
            let (trigger, boundary) = varied_boundary(&identifier);
            let field = public_field(application_of(&identifier), contexts.choice(&identifier, "context"), "");
            rows.push(ActionRow::new(identifier.as_str(), latin.as_str(), 0, field.clone(), trigger, Keep, "mixed_language_insertion", boundary.as_str()).weighing(weight));
            if contrast {
                rows.push(ActionRow::new(format!("{identifier}:lower"), pystr::lower(latin), 0, field, trigger, Convert, "mixed_language_layout_intervention", boundary.as_str()).weighing(weight));
            }
        }
    }
    let words: usize = chosen.values().map(|words| words.as_array().map_or(0, Vec::len)).sum();
    let frames = rows.len();
    (rows, json!({"words": words, "frames": frames, "lowercase_contrast": contrast, "words_by_length": chosen, "candidates_by_length": available, "sample_weight": weight,
        "scope": "TRAIN only: keep frames of Latin capitals after Russian prose whose Cyrillic reading is a rare lexicon word (capital_citation_curriculum)."}))
}

/// `english_capital_curriculum`.
pub fn english_capital_curriculum(tools: &Tools, source_rows: &[CorpusRow], options: &Value, models: &Lexicons) -> Curriculum {
    let budgets = options.budgets("words_by_length");
    let per_word = options.int("frames_per_word").to::<usize>();
    let weight = options.float("sample_weight");
    let contrast = options.is_true("lowercase_contrast");
    let [english, russian] = natural_mixed_contexts(source_rows);
    let (contexts, russian) = (ContextRing::new(english), ContextRing::new(russian));
    let russian_contrast = options.is_true("russian_contrast") && !russian.is_empty();
    if contexts.is_empty() || !budgets.iter().any(|entry| entry.1 != 0) || per_word == 0 {
        return (Vec::new(), json!({"frames": 0, "scope": "not used"}));
    }
    let frequencies = &models[1].frequencies;
    let mut words: Vec<(&str, i64)> = frequencies.iter().collect();
    words.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let mut candidates: HashMap<usize, Vec<(String, String)>> = HashMap::new();
    for (word, _) in words {
        let length = pystr::len(word);
        if !budgets.iter().any(|entry| entry.0 == length) || !cyrillic(word) || !pystr::islower(word) {
            continue;
        }
        let latin = reading(&pystr::upper(word), 1);
        if !pystr::isascii(&latin) || !pystr::isalpha(&latin) || tools.is_refused(&[word, &latin]) {
            continue;
        }
        candidates.entry(length).or_default().push((word.to_owned(), latin));
    }
    let mut rows = Vec::new();
    let (mut chosen, mut available) = (Map::new(), Map::new());
    for &(length, budget) in &budgets {
        let mut words = candidates.remove(&length).unwrap_or_default();
        available.insert(length.to_string(), json!(words.len()));
        words.truncate(budget);
        chosen.insert(length.to_string(), json!(words.iter().map(|(word, latin)| format!("{word}/{latin}")).collect::<Vec<_>>()));
        for (word, latin) in &words {
            for index in 0..per_word {
                let identifier = format!("english-capital:{word}:{index}");
                let (trigger, boundary) = varied_boundary(&identifier);
                let field = public_field(application_of(&identifier), contexts.choice(&identifier, "context"), "");
                rows.push(ActionRow::new(identifier.as_str(), latin.as_str(), 0, field.clone(), trigger, Keep, "english_capital", boundary.as_str()).weighing(weight));
                if contrast {
                    rows.push(ActionRow::new(format!("{identifier}:lower"), pystr::lower(latin), 0, field.clone(), trigger, Convert, "english_capital_lower", boundary.as_str()).weighing(weight));
                }
                if russian_contrast {
                    let other = format!("{identifier}:russian");
                    let mut replaced = field;
                    replaced.before.clone_from(russian.choice(&other, "context"));
                    rows.push(ActionRow::new(other, latin.as_str(), 0, replaced, trigger, Convert, "english_capital_russian", boundary.as_str()).weighing(weight));
                }
            }
        }
    }
    let frames = rows.len();
    (rows, json!({"frames": frames, "sample_weight": weight, "frames_per_word": per_word, "lowercase_contrast": contrast, "russian_contrast": russian_contrast,
        "words_by_length": chosen, "candidates_by_length": available,
        "scope": "TRAIN only: keep frames of the keys of common Russian words typed in Latin capitals after English prose (english_capital_curriculum)."}))
}

/// The attested abbreviations of the term table, in order, each with its Latin reading.
fn abbreviation_forms(tools: &Tools) -> Vec<(String, String)> {
    let mut forms: Vec<String> = tools.tables.term("cyrillic").words().map(pystr::upper).collect();
    forms.sort();
    let mut result = Vec::new();
    for form in forms {
        if !tools.attested_abbreviation(&form) {
            continue;
        }
        let latin = reading(&form, 1);
        if pystr::isalpha(&latin) {
            result.push((form, latin));
        }
    }
    result
}

/// `abbreviation_curriculum`.
pub fn abbreviation_curriculum(tools: &Tools, split: &str, source_rows: &[CorpusRow], options: &Value) -> Curriculum {
    let per_form = options.int("frames_per_form").to::<usize>();
    let [_, russian] = natural_mixed_contexts(source_rows);
    let contexts = ContextRing::new(russian);
    if per_form == 0 || contexts.is_empty() || (split != TRAIN && split != DEVELOPMENT) {
        return not_used();
    }
    let weight = options.float("sample_weight");
    let mut rows = Vec::new();
    let mut counts = Counts::default();
    for (form, latin) in abbreviation_forms(tools) {
        if share_of(&format!("abbreviation:{form}"), ABBREVIATION_SPLIT_MODULUS) != split {
            continue;
        }
        if tools.is_refused(&[&form, &latin]) {
            counts.add("refused");
            continue;
        }
        // Convert when its Latin reading occurs often enough and far more often than it does (`ФШ` is `AI`).
        let own = tools.count("cyrillic", &pystr::casefold(&form));
        let latin_count = tools.count("latin", &pystr::casefold(&latin));
        let convert = latin_count.to::<i64>() >= options.int("minimum_latin_count") && latin_count.to::<f64>() >= options.float("dominance") * own.to::<f64>();
        let action = if convert { Convert } else { Keep };
        counts.add(action.as_str());
        for index in 0..per_form {
            let identifier = format!("abbreviation:{form}:{index}");
            let (trigger, boundary) = varied_boundary(&identifier);
            let field = public_field(application_of(&identifier), contexts.choice(&identifier, "context"), "");
            rows.push(ActionRow::new(identifier, form.as_str(), 1, field, trigger, action, "abbreviation", boundary).weighing(weight));
        }
    }
    let frames = rows.len();
    (rows, json!({"frames": frames, "forms": counts.report(), "frames_per_form": per_form, "sample_weight": weight,
        "scope": "attested Cyrillic abbreviations of this split's share, typed as written after Russian prose, labelled by the term counts of both readings (abbreviation_curriculum)."}))
}

/// `latin_abbreviation_curriculum`.
pub fn latin_abbreviation_curriculum(tools: &Tools, split: &str, source_rows: &[CorpusRow], options: &Value) -> Curriculum {
    let per_form = options.int("frames_per_form").to::<usize>();
    let [english, _] = natural_mixed_contexts(source_rows);
    let english = ContextRing::new(english);
    let terms = ContextRing::new(if per_form != 0 { counted_terms(tools, options.int("minimum_term_count").to::<u64>()) } else { Vec::new() });
    if per_form == 0 || english.is_empty() || terms.is_empty() || (split != TRAIN && split != DEVELOPMENT) {
        return not_used();
    }
    let weight = options.float("sample_weight");
    let mut rows = Vec::new();
    let mut counts = Counts::default();
    for (form, latin) in abbreviation_forms(tools) {
        if share_of(&format!("abbreviation:{form}"), ABBREVIATION_SPLIT_MODULUS) != split {
            continue;
        }
        if tools.is_refused(&[&form, &latin]) {
            counts.add("refused");
            continue;
        }
        // Keep where Russian technical text uses it often enough and far more often than its Latin reading occurs.
        let own = tools.count("cyrillic", &pystr::casefold(&form));
        let folded = pystr::casefold(&latin);
        let latin_count = tools.count("latin", &folded).max(tools.count("english", &folded));
        let keep = own.to::<i64>() >= options.int("minimum_cyrillic_count") && own.to::<f64>() >= options.float("dominance") * latin_count.to::<f64>();
        let action = if keep { Keep } else { Convert };
        counts.add(action.as_str());
        for index in 0..per_form {
            let identifier = format!("latin-abbreviation:{form}:{index}");
            let (trigger, boundary) = varied_boundary(&identifier);
            let before = if index % LAYOUT_GROUP_COUNT == 0 {
                english.choice(&identifier, "context").clone()
            } else {
                let mut words = vec![terms.choice(&identifier, "context").as_str()];
                if variant_choice(&identifier, "context-words", LAYOUT_GROUP_COUNT) != 0 {
                    words.insert(0, terms.choice(&identifier, "first-context"));
                }
                format!("{} ", words.join(" "))
            };
            let field = public_field(application_of(&identifier), &before, "");
            rows.push(ActionRow::new(identifier, form.as_str(), 1, field, trigger, action, "latin_abbreviation", boundary).weighing(weight));
        }
    }
    let frames = rows.len();
    (rows, json!({"frames": frames, "forms": counts.report(), "frames_per_form": per_form, "sample_weight": weight,
        "scope": "attested Cyrillic abbreviations of this split's share, typed as written after English prose or Latin terms, labelled by the term counts of both readings (latin_abbreviation_curriculum)."}))
}

/// `unknown_token_curriculum`.
pub fn unknown_token_curriculum(tools: &Tools, split: &str, source_rows: &[CorpusRow], options: &Value) -> Curriculum {
    let item_modulus = options.int("item_modulus").to::<usize>();
    let [_, russian] = natural_mixed_contexts(source_rows);
    let contexts = ContextRing::new(russian);
    if item_modulus == 0 || contexts.is_empty() || (split != TRAIN && split != DEVELOPMENT) {
        return not_used();
    }
    let weight = options.float("sample_weight");
    let echo_modulus = options.int("echo_modulus").to::<usize>();
    let held_out = options.held_out();
    let minimum = options.int("minimum_term_count").to::<u64>();
    let mut rows = Vec::new();
    let mut counts = Counts::default();
    let frame = |identifier: String, typed: &str, group: i64, before: &str, action: ContextAction| {
        let (trigger, boundary) = varied_boundary(&identifier);
        let field = public_field(application_of(&identifier), clipped(before), "");
        ActionRow::new(identifier, typed, group, field, trigger, action, "unknown_token", boundary).weighing(weight)
    };
    let mut terms: Vec<&str> = tools.tables.term("latin").words().collect();
    terms.sort_unstable();
    for term in terms {
        let unique = term.chars().collect::<BTreeSet<_>>().len();
        let length = pystr::len(term);
        if tools.count("latin", term) < minimum
            || !pystr::isascii(term)
            || !pystr::isalnum(term)
            || !pystr::islower(term)
            || unique < UNKNOWN_HEAD_MIN_CHARACTERS
            || !(UNKNOWN_HEAD_MIN_CHARACTERS..=UNKNOWN_TOKEN_MAX_CHARACTERS).contains(&length)
        {
            continue;
        }
        let typed = reading(term, 0);
        let family = format!("unknown-term:{typed}");
        if !features::unknown_shape(&typed)
            || tools.plausible_reading(&typed, 1)
            || tools.count("cyrillic", &typed) >= UNCOUNTED
            || tools.count("russian", &typed) >= UNCOUNTED
            || variant_choice(&family, "draw", item_modulus) != 0
        {
            continue;
        }
        if share_of(&family, UNKNOWN_TOKEN_SPLIT_MODULUS) != split {
            continue;
        }
        if held_out.contains(term) || held_out.contains(&typed) || tools.is_refused(&[term, &typed]) {
            counts.add("refused");
            continue;
        }
        let echo = echo_modulus != 0 && variant_choice(&family, "echo", echo_modulus) == 0;
        let mut frames: Vec<(&str, i64, ContextAction, &str)> = vec![(typed.as_str(), 1, Convert, term), (term, 0, Keep, if echo { term } else { "" })];
        if echo {
            frames.push((term, 0, Keep, typed.as_str()));
        }
        for (index, (shown, group, action, earlier_word)) in frames.into_iter().enumerate() {
            let identifier = format!("{family}:{index}");
            let mut before = contexts.choice(&identifier, "context").clone();
            if !earlier_word.is_empty() {
                let earlier = contexts.choice(&identifier, "earlier-context");
                before = format!("{} {earlier_word}. {before}", pystr::rstrip(earlier));
            }
            if !features::unknown_question(shown, &before, &tools.tables) {
                // A long term earlier in a short text makes it Latin text: no frame of the class.
                counts.add("outside_class");
                continue;
            }
            counts.add(if earlier_word.is_empty() { action.as_str().to_owned() } else { format!("echo:{}", action.as_str()) });
            rows.push(frame(identifier, shown, group, &before, action));
        }
    }
    for item in monolingual_rows(source_rows) {
        let (row, group) = (item.row, item.group);
        let word = row.original.as_str();
        if group != 1
            || echo_modulus == 0
            || !features::unknown_question(word, &row.before, &tools.tables)
            || tools.plausible_reading(word, 1)
            || variant_choice(&row.identifier, "echo-word", echo_modulus) != 0
        {
            continue;
        }
        let keys = reading(word, 1);
        if held_out.contains(&pystr::casefold(word)) || held_out.contains(&pystr::casefold(&keys)) || tools.is_refused(&[word, &keys]) {
            counts.add("refused");
            continue;
        }
        let identifier = format!("unknown-echo:{}", row.identifier);
        let earlier = contexts.choice(&identifier, "earlier-context");
        let before = format!("{} {keys}. {}", pystr::rstrip(earlier), row.before);
        if !features::unknown_question(word, &before, &tools.tables) {
            counts.add("outside_class");
            continue;
        }
        counts.add("echo:keep");
        rows.push(frame(identifier, word, 1, &before, Keep));
    }
    let frames = rows.len();
    (rows, json!({"frames": frames, "items": counts.report(), "item_modulus": item_modulus, "echo_modulus": echo_modulus, "held_out": held_out.len(), "sample_weight": weight,
        "scope": "terms of the Latin table typed in the Russian layout after Russian prose (convert), some with the term typed right earlier in the text, and the class's Russian words of the split's rows with their keys earlier in the text (keep), of this split's share (unknown_token_curriculum)."}))
}

/// `joined_token_curriculum`.
pub fn joined_token_curriculum(tools: &Tools, split: &str, source_rows: &[CorpusRow], options: &Value) -> Curriculum {
    let item_modulus = options.int("item_modulus").to::<usize>();
    if item_modulus == 0 || (split != TRAIN && split != DEVELOPMENT) {
        return not_used();
    }
    let weight = options.float("sample_weight");
    let per_token = options.int("per_token");
    let held_out = options.held_out();
    let mut rows = Vec::new();
    let mut counts = Counts::default();
    let mut taken: HashMap<String, i64> = HashMap::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let pair_frames = |rows: &mut Vec<ActionRow>, counts: &mut Counts, stem: &str, token: &str, keys: &str, group: i64, before: &str, kind: &str| {
        for (index, (shown, layout, action)) in [(token, group, Keep), (keys, 1 - group, Convert)].into_iter().enumerate() {
            let frame = format!("{stem}:{index}");
            let (trigger, boundary) = varied_boundary(&frame);
            let field = public_field(application_of(&frame), before, "");
            counts.add(format!("{}:{kind}", action.as_str()));
            rows.push(ActionRow::new(frame, shown, layout, field, trigger, action, "joined_token", boundary).weighing(weight));
        }
    };
    for row in source_rows {
        if !row.layout_representable {
            continue;
        }
        let text = format!("{}{}{}", row.before, row.original, row.after);
        for (start, end) in crate::rows::joined_spans(&text) {
            let token = &text[start..end];
            let letters = token.replace('-', "");
            let group = if cyrillic(&letters) {
                Some(1)
            } else if pystr::isascii(&letters) {
                Some(0)
            } else {
                None
            };
            let before = clipped(&text[..start]);
            let occurrence = [row.document.as_str(), before, token].join("\0");
            let Some(group) = group else { continue };
            if !seen.insert(occurrence.clone()) {
                continue;
            }
            let identifier = format!("joined:{}", &hex_digest(&occurrence)[..12]);
            if variant_choice(&identifier, "draw", item_modulus) != 0 {
                continue;
            }
            let folded = pystr::casefold(token);
            if taken.get(&folded).copied().unwrap_or(0) >= per_token {
                counts.add("capped");
                continue;
            }
            let keys = reading(token, group);
            if held_out.contains(&folded) || held_out.contains(&pystr::casefold(&keys)) || tools.is_refused(&[token, &keys]) {
                counts.add("refused");
                continue;
            }
            let parts: BTreeSet<String> = token.split('-').map(pystr::casefold).collect();
            *taken.entry(folded).or_insert(0) += 1;
            let kind = if parts.len() == 1 { "repeat" } else { "words" };
            pair_frames(&mut rows, &mut counts, &identifier, token, &keys, group, before, kind);
        }
    }
    let repeat_words = options.int_or("repeat_words", 0).to::<usize>();
    let minimum = options.int_or("repeat_minimum_count", 0).to::<u64>();
    let [english, russian] = natural_mixed_contexts(source_rows);
    let contexts = [ContextRing::new(english), ContextRing::new(russian)];
    for (group, alphabet) in [(1_i64, "russian"), (0, "english")] {
        let mut words: Vec<&str> = tools
            .tables
            .term(alphabet)
            .words()
            .filter(|word| {
                let length = pystr::len(word);
                tools.count(alphabet, word) >= minimum
                    && (JOINED_REPEAT_MIN_LETTERS..=JOINED_REPEAT_MAX_LETTERS).contains(&length)
                    && pystr::isalpha(word)
                    && if group == 1 { cyrillic(word) && !word.chars().any(|letter| UNKNOWN_HEAD_SIGN_KEY_LETTERS.contains(&letter)) } else { pystr::isascii(word) }
            })
            .collect();
        words.sort_by_cached_key(|word| hex_digest(&format!("joined-repeat:{alphabet}:{word}")));
        let ring = &contexts[group.to::<usize>()];
        // The first `repeat_words` words in hash order, of both splits: each split frames its share of them.
        for word in words.into_iter().take(if ring.is_empty() { 0 } else { repeat_words }) {
            let identifier = format!("joined-repeat:{alphabet}:{word}");
            if share_of(&identifier, JOINED_REPEAT_SPLIT_MODULUS) != split {
                continue;
            }
            let capital = variant_choice(&identifier, "capital", JOINED_REPEAT_CAPITAL_CHOICES) == 0;
            let mut token = format!("{word}-{word}");
            if capital {
                let head: String = token.chars().take(1).collect();
                token = format!("{}{}", pystr::upper(&head), &token[head.len()..]);
            }
            let keys = reading(&token, group);
            if held_out.contains(&pystr::casefold(&token)) || held_out.contains(&pystr::casefold(&keys)) || tools.is_refused(&[&token, &keys]) {
                counts.add("refused");
                continue;
            }
            let context = ring.choice(&identifier, "context");
            let written = if capital { format!("{}. ", pystr::rstrip(context)) } else { context.clone() };
            pair_frames(&mut rows, &mut counts, &identifier, &token, &keys, group, clipped(&written), "synthetic");
        }
    }
    let frames = rows.len();
    (rows, json!({"frames": frames, "items": counts.report(), "tokens": taken.len(), "item_modulus": item_modulus, "per_token": per_token, "repeat_words": repeat_words,
        "repeat_minimum_count": minimum, "held_out": held_out.len(), "sample_weight": weight,
        "scope": "words joined by hyphens in the text of this split's rows, and counted words of either language joined to themselves, after the text before them: typed as written (keep) and their keys in the other layout (convert) (joined_token_curriculum)."}))
}

/// `revisit_followers`: the most frequent short words of Russian prose, after which a term is asked again.
fn revisit_followers(tools: &Tools) -> Vec<String> {
    let mut words: Vec<(i64, &str)> = tools
        .tables
        .term("russian")
        .words()
        .filter(|word| pystr::len(word) <= TERM_REVISIT_FOLLOWER_MAX_LETTERS && pystr::isalpha(word))
        .map(|word| (-tools.count("russian", word).to::<i64>(), word))
        .collect();
    words.sort_unstable();
    words.into_iter().take(TERM_REVISIT_FOLLOWERS).map(|entry| entry.1.to_owned()).collect()
}

fn planned_or_field(typed: &str) -> AfterOrigin {
    if pystr::len(typed) <= PLANNED_CONTEXT_WORD_MAX_CHARACTERS { AfterOrigin::PlannedNextConversion } else { AfterOrigin::Field }
}

/// `term_curriculum`.
pub fn term_curriculum(tools: &Tools, split: &str, source_rows: &[CorpusRow], options: &Value) -> Curriculum {
    let item_modulus = options.int("item_modulus").to::<usize>();
    let [_, russian] = natural_mixed_contexts(source_rows);
    let contexts = ContextRing::new(russian);
    if item_modulus == 0 || contexts.is_empty() || (split != TRAIN && split != DEVELOPMENT) {
        return not_used();
    }
    let weight = options.float("sample_weight");
    let piece_weight = options.float_or("piece_weight", 1.0);
    let per_piece = options.int("per_piece");
    let held_out = options.held_out();
    let mut rows: Vec<ActionRow> = Vec::new();
    let mut counts = Counts::default();
    let question = |token: &str, before: &str| features::term_question(token, before, &tools.tables);
    let frame = |identifier: String, typed: &str, before: &str, action: ContextAction, share: f64| {
        let (trigger, boundary) = varied_boundary(&identifier);
        let field = public_field(application_of(&identifier), clipped(before), "");
        ActionRow::new(identifier, typed, 1, field, trigger, action, "term", boundary).weighing(weight * share)
    };
    let refuse = |token: &str, keys: &str| held_out.contains(&pystr::casefold(token)) || held_out.contains(&pystr::casefold(keys)) || tools.is_refused(&[token, keys]);
    let followers = revisit_followers(tools);
    let revisit = |identifier: String, typed: &str, before: &str, action: ContextAction, share: f64| {
        let follower = &followers[variant_choice(&identifier, "follower", followers.len())];
        let field = public_field(application_of(&identifier), clipped(before), follower);
        let mut row = ActionRow::new(identifier, typed, 1, field, "space", action, "term", " ").weighing(weight * share).originating(planned_or_field(typed));
        row.revisit = true;
        row
    };
    let mut terms: Vec<&str> = tools.tables.term("latin").words().collect();
    terms.sort_unstable();
    let signs: Vec<char> = reading(TERM_HEAD_TRAILING_SIGNS, 0).chars().collect();
    for term in terms {
        if !letters_only_lower(term) {
            continue;
        }
        let typed = reading(term, 0);
        let family = format!("term:{typed}");
        if variant_choice(&family, "draw", item_modulus) != 0 || !question(&typed, "") {
            continue;
        }
        if share_of(&family, TERM_SPLIT_MODULUS) != split {
            continue;
        }
        if refuse(term, &typed) {
            counts.add("refused");
            continue;
        }
        let context = contexts.choice(&family, "context");
        let other = contexts.choice(&family, "other-context");
        let mut variants = vec!["start", "signed", "capital"];
        if pystr::len(term) <= TERM_CAPITALS_MAX_LETTERS {
            variants.push("upper");
        }
        let variant = variants[variant_choice(&family, "variant", variants.len())];
        let shown = match variant {
            "start" => typed.clone(),
            "signed" => format!("{typed}{}", signs[variant_choice(&family, "sign", signs.len())]),
            "capital" => pystr::upper(pystr::slice(&typed, None, Some(1))) + pystr::slice(&typed, Some(1), None),
            _ => pystr::upper(&typed),
        };
        let before = match variant {
            "start" => String::new(),
            "capital" => format!("{}. ", pystr::rstrip(other)),
            _ => other.clone(),
        };
        for (index, (kind, token, left)) in [("plain", typed.as_str(), context.as_str()), (variant, shown.as_str(), before.as_str())].into_iter().enumerate() {
            if !question(token, left) {
                // A term written right earlier in the text is the unknown-token head's.
                counts.add("outside_class");
                continue;
            }
            let mut framed = frame(format!("{family}:{index}"), token, left, Convert, 1.0);
            if left.is_empty() && framed.trigger == "space" && pystr::len(token) <= TERM_ALONE_WAIT_MAX_LETTERS {
                framed.action = Wait;
            }
            counts.add(format!("{}:{kind}", framed.action.as_str()));
            rows.push(framed);
        }
        if question(&typed, context) {
            counts.add("convert:revisit");
            rows.push(revisit(format!("{family}:revisit"), &typed, context, Convert, 1.0));
        }
        let longest = TERM_HEAD_ENDING_LETTERS[TERM_HEAD_ENDING_LETTERS.len() - 1];
        if pystr::len(&typed) <= longest && question(&typed, "") && pystr::len(&typed) <= TERM_ALONE_WAIT_MAX_LETTERS {
            // Every short term once more alone at a space.
            let application = crate::rows::APPLICATIONS[variant_choice(&family, "alone-application", crate::rows::APPLICATIONS.len())];
            counts.add("wait:alone");
            rows.push(ActionRow::new(format!("{family}:alone"), typed.as_str(), 1, public_field(application, "", ""), "space", Wait, "term", " ").weighing(weight));
        }
    }
    let mut taken: HashMap<String, i64> = HashMap::new();
    let mut russian_rows: Vec<&CorpusRow> = monolingual_rows(source_rows).iter().filter(|item| item.group == 1).map(|item| item.row).collect();
    russian_rows.sort_by_cached_key(|row| sha256(&format!("term-piece:{}", row.identifier)));
    for row in russian_rows {
        if !row.before.chars().any(crate::rows::is_alpha) {
            // A tail with no word before it stands alone in its field, a term's place.
            counts.add("skipped:row-start");
            continue;
        }
        for size in TERM_HEAD_ENDING_LETTERS {
            let piece = pystr::slice(&row.original, Some(-size.to::<i64>()), None);
            let folded = pystr::casefold(piece);
            if pystr::len(&row.original) < size + TERM_FRAGMENT_MIN_REST
                || taken.get(&folded).copied().unwrap_or(0) >= per_piece
                || !question(piece, &row.before)
                || features::russian_ending_bucket(piece, &tools.tables).parse::<i64>().expect("an ending bucket is a number") < TERM_FRAGMENT_MIN_ENDING_BUCKET
            {
                continue;
            }
            if refuse(piece, &reading(piece, 1)) {
                counts.add("refused");
                continue;
            }
            *taken.entry(folded).or_insert(0) += 1;
            counts.add("keep:piece");
            counts.add("keep:piece-revisit");
            rows.push(frame(format!("term-piece:{}:{size}", row.identifier), piece, &row.before, Keep, piece_weight));
            rows.push(revisit(format!("term-piece:{}:{size}:revisit", row.identifier), piece, &row.before, Keep, piece_weight));
        }
    }
    let frames = rows.len();
    (rows, json!({"frames": frames, "items": counts.report(), "pieces": taken.len(), "item_modulus": item_modulus, "per_piece": per_piece, "piece_weight": piece_weight, "held_out": held_out.len(), "sample_weight": weight,
        "scope": "terms of the Latin table typed in the Russian layout after Russian prose, at the start of a message, with a sign after them, with a capital or in capitals, and asked again after a Russian word that went back (convert; alone at a space, a term of two letters waits), and the tails of the split's Russian words after a word that are such a token (keep), of this split's share (term_curriculum)."}))
}

/// `first_curriculum`.
pub fn first_curriculum(tools: &Tools, split: &str, options: &Value) -> Curriculum {
    if !options.get("enabled").and_then(Value::as_bool).unwrap_or(false) || (split != TRAIN && split != DEVELOPMENT) {
        return not_used();
    }
    let weight = options.float("sample_weight");
    let held_out = options.held_out();
    let followers = revisit_followers(tools);
    let mut rows = Vec::new();
    let mut counts = Counts::default();
    let mut terms: Vec<&str> = tools.tables.term("latin").words().collect();
    terms.sort_unstable();
    for term in terms {
        if !letters_only_lower(term) {
            continue;
        }
        let typed = reading(term, 0);
        let family = format!("term:{typed}");
        if !features::first_question(&typed, "", "", &tools.tables) {
            continue;
        }
        if share_of(&family, TERM_SPLIT_MODULUS) != split {
            continue;
        }
        if held_out.contains(&pystr::casefold(term)) || held_out.contains(&pystr::casefold(&typed)) || tools.is_refused(&[term, &typed]) {
            counts.add("refused");
            continue;
        }
        let stem = format!("first:{typed}");
        let mut field = field_of(application_of(&stem), crate::rows::PUBLIC_TRAINING, "", "", FieldRole::Text);
        field.source = "reader".to_owned();
        let start = features::russian_start_bucket(&typed, &tools.tables);
        let stopped = if start != "na" && start.parse::<i64>().expect("a start bucket is a number") >= FIRST_PAUSE_WORD_START_MIN_BUCKET { Keep } else { Convert };
        for (trigger, boundary, action) in [("space", " ", Convert), ("pause", "", stopped)] {
            counts.add(format!("{}:{trigger}", action.as_str()));
            rows.push(ActionRow::new(format!("first:{typed}:{trigger}"), typed.as_str(), 1, field.clone(), trigger, action, "first", boundary).weighing(weight));
        }
        let follower = &followers[variant_choice(&stem, "follower", followers.len())];
        counts.add("convert:next");
        let mut asked = field;
        asked.after.clone_from(follower);
        let mut row = ActionRow::new(format!("first:{typed}:next"), typed.as_str(), 1, asked, "space", Convert, "first", " ").weighing(weight).originating(planned_or_field(&typed));
        row.revisit = true;
        rows.push(row);
    }
    let frames = rows.len();
    (rows, json!({"frames": frames, "items": counts.report(), "held_out": held_out.len(), "sample_weight": weight,
        "scope": "terms of up to three letters of the Latin table typed in the Russian layout alone in a field a reader read empty, ended by a space and by a pause, and asked again with the word after them (convert; one that many Russian words begin with is kept at the pause) (first_curriculum)."}))
}

/// `brief_curriculum`.
pub fn brief_curriculum(tools: &Tools, split: &str, options: &Value) -> Curriculum {
    let minimum = options.int("minimum_count").to::<u64>();
    if minimum == 0 || (split != TRAIN && split != DEVELOPMENT) {
        return not_used();
    }
    let weight = options.float("sample_weight");
    let held_out = options.held_out();
    let brief_length = |word: &str| BRIEF_HEAD_LETTERS.contains(&pystr::len(word));
    let mut tokens: BTreeMap<String, String> = BTreeMap::new();
    for word in tools.tables.term("russian").words() {
        let keys = reading(word, 1);
        if tools.count("russian", word) >= minimum && brief_length(word) && pystr::isascii(&keys) && pystr::isalpha(&keys) {
            tokens.insert(pystr::casefold(&keys), word.to_owned());
        }
    }
    for alphabet in ["english", "latin"] {
        for word in tools.tables.term(alphabet).words() {
            if tools.count(alphabet, word) >= minimum && brief_length(word) && letters_only_lower(word) {
                let keys = reading(word, 0);
                if pystr::isalpha(&keys) && !pystr::isascii(&keys) {
                    tokens.entry(word.to_owned()).or_insert(keys);
                }
            }
        }
    }
    let mut rows = Vec::new();
    let mut counts = Counts::default();
    for (latin, cyrillic_keys) in &tokens {
        let family = format!("brief:{latin}");
        if share_of(&family, BRIEF_SPLIT_MODULUS) != split {
            continue;
        }
        if held_out.contains(latin) || held_out.contains(cyrillic_keys) || tools.is_refused(&[latin, cyrillic_keys]) {
            counts.add("refused");
            continue;
        }
        if !features::brief_settled(latin, cyrillic_keys, &tools.tables) {
            // The head answers no token the counts leave open.
            counts.add("unsettled");
            continue;
        }
        let russian = tools.count("russian", cyrillic_keys);
        let share = russian.to::<f64>() / (russian + tools.count("english", latin).max(tools.count("latin", latin))).to::<f64>();
        counts.add(if share > 0.0 && share < 1.0 {
            "both"
        } else if share != 0.0 {
            "russian"
        } else {
            "latin"
        });
        for (index, (trigger, boundary)) in BRIEF_BOUNDARIES.into_iter().enumerate() {
            for (case, shown) in [("lower", latin.clone()), ("title", pystr::capitalize(latin))] {
                let identifier = format!("{family}:{index}:{case}");
                let field = public_field(application_of(&identifier), "", "");
                for (action, mass) in [(Convert, share), (Keep, 1.0 - share)] {
                    if mass != 0.0 {
                        rows.push(ActionRow::new(format!("{identifier}:{}", action.as_str()), shown.as_str(), 0, field.clone(), trigger, action, "brief", boundary).weighing(weight * mass));
                    }
                }
            }
        }
    }
    let frames = rows.len();
    (rows, json!({"frames": frames, "tokens": counts.report(), "minimum_count": minimum, "held_out": held_out.len(), "sample_weight": weight,
        "scope": "tokens of two or three letters alone at the start of a message, typed in the English layout, that the counts settle as the keys of a Russian word (convert, a share of the Latin count kept) (brief_curriculum)."}))
}

/// `kept_neighbour_curriculum`.
pub fn kept_neighbour_curriculum(tools: &Tools, source_rows: &[CorpusRow], options: &Value, models: &Lexicons) -> Curriculum {
    let budget = |name: &str| options.int(name).to::<usize>();
    let (natural_frames, term_frames, abbreviation_frames, typo_frames) = (budget("natural_frames"), budget("term_frames"), budget("abbreviation_frames"), budget("typo_frames"));
    if natural_frames == 0 && term_frames == 0 && abbreviation_frames == 0 && typo_frames == 0 {
        return (Vec::new(), json!({"frames": 0, "scope": "not used"}));
    }
    let weight = options.float("sample_weight");
    let ranked = |values: Vec<String>, purpose: &str| -> Vec<String> {
        let mut values = values;
        values.sort_by_cached_key(|value| sha256(&format!("kept-{purpose}:{value}")));
        values
    };
    let frame = |identifier: String, word: &str, group: i64, before: &str, after: &str, action: ContextAction, category: String| {
        let field = public_field(application_of(&identifier), before, after);
        ActionRow::new(identifier, word, group, field, "space", action, category, " ").weighing(weight).originating(AfterOrigin::KeptNextWord)
    };
    let mut natural: [BTreeMap<String, (String, String, String)>; 2] = [BTreeMap::new(), BTreeMap::new()];
    let monolingual = monolingual_rows(source_rows);
    for item in &monolingual {
        // Every short word of the sentence followed, after one space, by another word is the question asked as written.
        for pair in item.tokens.windows(2) {
            let (current, following) = (pair[0], pair[1]);
            let word = &item.text[current.0..current.1];
            let next = &item.text[following.0..following.1];
            if (KEPT_NEIGHBOUR_MIN_LETTERS..=KEPT_CONTEXT_WORD_MAX_CHARACTERS).contains(&pystr::len(word))
                && pystr::isalpha(word)
                && pystr::isalpha(next)
                && &item.text[current.1..following.0] == " "
            {
                let before = &item.text[..current.0];
                natural[item.group.to::<usize>()].insert(format!("{before}|{word}"), (before.to_owned(), word.to_owned(), next.to_owned()));
            }
        }
    }
    let russian = ContextRing::new(russian_contexts(source_rows));
    let mut rows: Vec<ActionRow> = Vec::new();
    let mut counts = Counts::default();
    for group in 0..2_usize {
        let keys: Vec<String> = natural[group].keys().cloned().collect();
        for key in ranked(keys, &format!("natural-{group}")).into_iter().take(natural_frames / LAYOUT_GROUP_COUNT) {
            let (before, word, following) = &natural[group][&key];
            let identifier = format!("kept-natural:{}", hex_digest(&key));
            rows.push(frame(identifier, word, group.to::<i64>(), before, following, Keep, "kept_neighbour".to_owned()));
            counts.add(format!("natural_{group}"));
        }
    }
    let mut inserted = |words: &[String], budget: usize, purpose: &str, typed: &dyn Fn(&str) -> String, action: ContextAction| -> Vec<String> {
        let mut chosen = Vec::new();
        if russian.is_empty() {
            return chosen;
        }
        for word in words {
            if counts.get(purpose).to::<usize>() >= budget {
                break;
            }
            chosen.push(word.clone());
            for index in 0..KEPT_NEIGHBOUR_FRAMES_PER_WORD {
                if counts.get(purpose).to::<usize>() >= budget {
                    break;
                }
                let identifier = format!("kept-{purpose}:{word}:{index}");
                let (before, following) = russian.choice(&identifier, "context");
                rows.push(frame(identifier, &typed(word), 1, before, following, action, format!("kept_neighbour_{purpose}")));
                counts.add(purpose);
            }
        }
        chosen
    };
    let terms = counted_terms(tools, options.int("minimum_term_count").to::<u64>());
    let chosen_terms = inserted(&ranked(terms.clone(), "term"), term_frames, "term", &term_typed, Convert);
    let abbreviations = counted_abbreviations(tools, options.int("minimum_abbreviation_count").to::<u64>(), options.float("abbreviation_dominance"));
    let chosen_abbreviations = inserted(&ranked(abbreviations.clone(), "abbreviation"), abbreviation_frames, "abbreviation", &|token| token.to_owned(), Keep);
    let mut misspelt: BTreeSet<String> = BTreeSet::new();
    for word in models[1].frequencies.words().iter() {
        if !cyrillic(word) || !pystr::isalpha(word) || tools.is_refused(&[word]) {
            continue;
        }
        for variant in typo_variants(word, &format!("kept-typo:{word}")) {
            if !(KEPT_NEIGHBOUR_MIN_LETTERS..=KEPT_CONTEXT_WORD_MAX_CHARACTERS).contains(&pystr::len(&variant))
                || !pystr::isalpha(&variant)
                || tools.plausible_reading(&variant, 1)
            {
                continue;
            }
            let keys = reading(&variant, 1);
            if tools.term_bucket(&keys, "latin") != "0" || tools.plausible_reading(&keys, 0) {
                continue;
            }
            misspelt.insert(variant);
        }
    }
    let misspelt_count = misspelt.len();
    let chosen_misspelt = inserted(&ranked(misspelt.into_iter().collect(), "typo"), typo_frames, "typo", &|word| word.to_owned(), Keep);
    let frames = rows.len();
    (rows, json!({"frames": frames, "counts": counts.report(), "sample_weight": weight,
        "candidates": {"natural_0": natural[0].len(), "natural_1": natural[1].len(), "terms": terms.len(), "abbreviations": abbreviations.len(), "misspelt": misspelt_count, "contexts": russian.len()},
        "terms": chosen_terms, "abbreviations": chosen_abbreviations, "misspelt": chosen_misspelt,
        "scope": "TRAIN only: kept_next_word frames, the question a waiting word is asked again with its kept next word (kept_neighbour_curriculum)."}))
}

/// `counted_token_curriculum`.
pub fn counted_token_curriculum(tools: &Tools, source_rows: &[CorpusRow], options: &Value) -> Curriculum {
    let (term_frames, abbreviation_frames) = (options.int("term_frames").to::<usize>(), options.int("abbreviation_frames").to::<usize>());
    let per_word = options.int("frames_per_word").to::<usize>();
    let russian = ContextRing::new(russian_contexts(source_rows));
    if (term_frames == 0 && abbreviation_frames == 0) || russian.is_empty() {
        return (Vec::new(), json!({"frames": 0, "scope": "not used"}));
    }
    let weight = options.float("sample_weight");
    let terms = counted_terms(tools, options.int("minimum_term_count").to::<u64>());
    let abbreviations = counted_abbreviations(tools, options.int("minimum_abbreviation_count").to::<u64>(), options.float("abbreviation_dominance"));
    let capitals = options.is_true("capitals");
    let mut rows: Vec<ActionRow> = Vec::new();
    let mut chosen: [Vec<String>; 2] = [Vec::new(), Vec::new()];
    let plans: [(&str, &Vec<String>, usize, ContextAction); 2] = [("term", &terms, term_frames, Convert), ("abbreviation", &abbreviations, abbreviation_frames, Keep)];
    for (slot, (purpose, candidates, budget, action)) in plans.into_iter().enumerate() {
        let mut words = candidates.clone();
        words.sort_by_cached_key(|word| sha256(&format!("counted-{purpose}:{word}")));
        let mut frames = 0;
        for word in &words {
            if frames >= budget {
                break;
            }
            chosen[slot].push(word.clone());
            for index in 0..per_word.min(budget - frames) {
                let identifier = format!("counted-{purpose}:{word}:{index}");
                let (trigger, boundary) = varied_boundary(&identifier);
                let (before, _) = russian.choice(&identifier, "context");
                let typed = match (slot, capitals) {
                    (0, true) => reading(&pystr::upper(word), 0),
                    (0, false) => term_typed(word),
                    (_, true) => pystr::upper(word),
                    (_, false) => word.clone(),
                };
                let field = public_field(application_of(&identifier), before, "");
                rows.push(ActionRow::new(identifier, typed, 1, field, trigger, action, format!("counted_{purpose}"), boundary).weighing(weight));
                frames += 1;
            }
        }
    }
    let mut by_category = Counts::default();
    for row in &rows {
        by_category.add(row.category.as_str());
    }
    let frames = rows.len();
    (rows, json!({"frames": frames, "counts": by_category.report(), "candidates": {"term": terms.len(), "abbreviation": abbreviations.len()},
        "sample_weight": weight, "frames_per_word": per_word, "capitals": capitals, "terms": chosen[0], "abbreviations": chosen[1],
        "scope": "TRAIN only: counted Russian abbreviations (keep) and counted Latin terms typed in the Russian layout (convert) after Russian prose, decided at their own boundary (counted_token_curriculum)."}))
}

/// `letter_curriculum`.
pub fn letter_curriculum(tools: &Tools, source_rows: &[CorpusRow], options: &Value) -> Curriculum {
    let (term_sentences, english_sentences) = (options.int("term_sentences").to::<usize>(), options.int("english_sentences").to::<usize>());
    let terms = ContextRing::new(technical_terms(tools, options.int("minimum_term_count").to::<u64>()));
    if (term_sentences == 0 && english_sentences == 0) || terms.is_empty() {
        return (Vec::new(), json!({"frames": 0, "scope": "not used"}));
    }
    let weight = options.float("sample_weight");
    let mut letters: Vec<String> = TRUSTED_SINGLE_LETTER_WORDS.iter().map(|letter| reading(letter, 1)).collect();
    letters.sort();
    let frame = |identifier: String, word: &str, group: i64, before: &str, after: &str, action: ContextAction, category: &str, origin: AfterOrigin| {
        let field = public_field(application_of(&identifier), before, after);
        ActionRow::new(identifier, word, group, field, "space", action, category, " ").weighing(weight).originating(origin)
    };
    let mut sentences: [BTreeMap<String, [String; 4]>; 2] = [BTreeMap::new(), BTreeMap::new()];
    for item in monolingual_rows(source_rows) {
        for triple in item.tokens.windows(3) {
            let (previous, current, following) = (triple[0], triple[1], triple[2]);
            let text = &item.text;
            let (word, after) = (&text[current.0..current.1], &text[following.0..following.1]);
            if &text[previous.1..current.0] != " "
                || &text[current.1..following.0] != " "
                || !pystr::isalpha(word)
                || !pystr::isalpha(after)
                || pystr::len(after) > PLANNED_CONTEXT_AFTER_MAX_CHARACTERS
                || tools.is_refused(&[word, after])
            {
                continue;
            }
            let previous_word = &text[previous.0..previous.1];
            let key = format!("{}|{word}", &text[..current.0]);
            if item.group == 1 && TRUSTED_SINGLE_LETTER_WORDS.contains(&word) {
                sentences[1].insert(key, [text[..previous.0].to_owned(), word.to_owned(), after.to_owned(), String::new()]);
            } else if item.group == 0 && pystr::isalpha(previous_word) {
                sentences[0].insert(key, [text[..previous.0].to_owned(), previous_word.to_owned(), word.to_owned(), after.to_owned()]);
            }
        }
    }
    let mut rows: Vec<ActionRow> = Vec::new();
    let mut counts = Counts::default();
    let ranked = |table: &BTreeMap<String, [String; 4]>, purpose: &str| -> Vec<String> {
        let mut keys: Vec<String> = table.keys().cloned().collect();
        keys.sort_by_cached_key(|key| sha256(&format!("letter-{purpose}:{key}")));
        keys
    };
    use AfterOrigin::{KeptNextWord, None as NoOrigin, PlannedNextConversion};
    for key in ranked(&sentences[1], "term").into_iter().take(term_sentences) {
        let [prose, word, after, _] = &sentences[1][&key];
        let identifier = format!("letter-term:{}", hex_digest(&key));
        let term = terms.choice(&identifier, "term");
        let other = terms.choice(&identifier, "next-term");
        let start = variant_choice(&identifier, "message-start", LETTER_CURRICULUM_MESSAGE_START_MODULUS) == 0;
        let before = if start { format!("{term} ") } else { format!("{prose}{term} ") };
        let keys = reading(word, 1);
        rows.push(frame(format!("{identifier}:own"), &keys, 0, &before, "", Wait, "letter_term", NoOrigin));
        rows.push(frame(format!("{identifier}:planned"), &keys, 0, &before, after, Convert, "letter_term", PlannedNextConversion));
        rows.push(frame(format!("{identifier}:kept"), &keys, 0, &before, other, Convert, "letter_term", KeptNextWord));
        rows.push(frame(format!("{identifier}:own-keep"), word, 1, &before, "", Keep, "letter_term_kept", NoOrigin));
        rows.push(frame(format!("{identifier}:planned-keep"), word, 1, &before, other, Keep, "letter_term_kept", PlannedNextConversion));
        rows.push(frame(format!("{identifier}:kept-keep"), word, 1, &before, after, Keep, "letter_term_kept", KeptNextWord));
        counts.add("term_sentences");
        counts.add_by("message_start", i64::from(start));
    }
    for key in ranked(&sentences[0], "english").into_iter().take(english_sentences) {
        let [prose, previous_word, word, after] = &sentences[0][&key];
        let identifier = format!("letter-english:{}", hex_digest(&key));
        let before = format!("{prose}{previous_word} ");
        if pystr::len(word) == 1 {
            rows.push(frame(format!("{identifier}:own"), word, 0, &before, "", Keep, "letter_english_word", NoOrigin));
            rows.push(frame(format!("{identifier}:kept"), word, 0, &before, after, Keep, "letter_english_word", KeptNextWord));
            counts.add("english_letters");
            continue;
        }
        let letter = &letters[variant_choice(&identifier, "letter", letters.len())];
        rows.push(frame(format!("{identifier}:own"), letter, 0, &before, "", Wait, "letter_english", NoOrigin));
        rows.push(frame(format!("{identifier}:kept"), letter, 0, &before, after, Keep, "letter_english", KeptNextWord));
        counts.add("english_sentences");
    }
    let frames = rows.len();
    (rows, json!({"frames": frames, "counts": counts.report(), "sample_weight": weight,
        "candidates": {"terms": terms.len(), "russian_sentences": sentences[1].len(), "english_sentences": sentences[0].len()},
        "scope": "TRAIN only: a word of one letter after a counted technical term in Russian sentences and in place of a word of English ones, at its boundary and beside its next word (letter_curriculum)."}))
}

/// What the labelling base answers on a frame, or None when the profiles' answers differ.
pub type Decide<'a> = &'a (dyn Fn(&ActionRow) -> Option<ContextAction> + Sync);

/// `alone_labels`: the frames of a split with every deferred frame of the lone-word head's class labelled.
pub fn alone_labels(tools: &Tools, rows: Vec<ActionRow>, evidence: Option<&Value>, decide: Option<Decide<'_>>) -> Vec<ActionRow> {
    let Some(evidence) = evidence else { return rows };
    let mut result = Vec::with_capacity(rows.len());
    for mut row in rows {
        if matches!(row.action, Wait | Suggest) && row.after_origin != AfterOrigin::PlannedNextConversion {
            let alternate = reading(&row.original, row.group);
            if features::alone_question(&row.original, &alternate, &row.field.before, &row.field.after, row.trigger) {
                let strong = tools.counted_reading(&row.original, &alternate, row.group, evidence.int("minimum_count").to::<u64>(), evidence.int("other_maximum_count").to::<u64>(), evidence.float("dominance"));
                if let Some(strong) = strong {
                    row.action = if strong == row.group { Keep } else { Convert };
                } else if let Some(own) = decide.and_then(|decide| decide(&row)) {
                    row.action = own;
                }
            }
        }
        result.push(row);
    }
    result
}

/// `lone_word_curriculum`: every pair of two Latin letters and its Russian keys, each sent alone.
pub fn lone_word_curriculum(tools: &Tools, split: &str, evidence: &Value, decide: Option<Decide<'_>>) -> Curriculum {
    let mut rows = Vec::new();
    let mut counts = Counts::default();
    let weight = evidence.float("sample_weight");
    for first in 'a'..='z' {
        for second in 'a'..='z' {
            let latin: String = [first, second].iter().collect();
            let cyrillic_keys = reading(&latin, 0);
            if !pystr::isalpha(&cyrillic_keys) {
                continue;
            }
            let share = match variant_choice(&format!("lone-word:{latin}"), "split", LONE_WORD_SPLIT_MODULUS) {
                0 => DEVELOPMENT,
                1 => crate::corpus::CALIBRATION,
                _ => TRAIN,
            };
            if share != split {
                continue;
            }
            if tools.is_refused(&[&latin, &cyrillic_keys]) {
                counts.add("refused");
                continue;
            }
            counts.add("pairs");
            for (group, written) in [(0_i64, &latin), (1, &cyrillic_keys)] {
                for typed in [written.clone(), pystr::capitalize(written), pystr::upper(written)] {
                    for (trigger, boundary) in LONE_WORD_BOUNDARIES {
                        let identifier = format!("lone-word:{typed}:{trigger}:{}", crate::physical::py_repr(boundary));
                        let field = public_field(application_of(&identifier), "", "");
                        let action = if trigger == "pause" { Wait } else { Suggest };
                        rows.push(ActionRow::new(identifier, typed.as_str(), group, field, trigger, action, "lone_word", boundary).weighing(weight));
                    }
                }
            }
        }
    }
    let rows = alone_labels(tools, rows, Some(evidence), decide);
    for row in &rows {
        counts.add(row.action.as_str());
    }
    let frames = rows.len();
    (rows, json!({"frames": frames, "counts": counts.report(), "sample_weight": weight,
        "scope": "lone_word frames of this split: every pair of two Latin letters whose Russian keys are letters too, each reading alone in the field at a boundary other than a space (lone_word_curriculum), labelled by alone_labels with the labelling base's answer where the counts settle nothing."}))
}

/// `curated_letter` for the captured curricula.
pub fn curated(original: &str, alternative: &str) -> bool {
    curated_letter(original, alternative)
}
