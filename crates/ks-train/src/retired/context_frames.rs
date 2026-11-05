//! Model-blind action interventions on real phrases of the Tatoeba snapshot, not observed user intent. The frames of the
//! rejected context-v2 and what they were built from.
use ks_cast::To as _;
use crate::ortho_inputs::{Assigned, assign, load_phrases};
use crate::physical::{hex, hex_digest, pair};
use ks_engine::unicode_category::category;
use ks_text::pyjson::json_string;
use ks_text::{pystr, ucd};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::Path;

const AFTER_MAX: usize = 128;
const BEFORE_MAX: usize = 512;
const FOCUS_POSITIONS: usize = 2;
const LEXICAL_HOLDOUT_MODULUS: u64 = 10;
const SAFETY_BUCKETS: u64 = 10;
const SAFETY_TRAIN_BUCKETS: u64 = 6;
const SAFETY_DEVELOPMENT_BUCKET: u64 = 6;
const SAFETY_CALIBRATION_BUCKET: u64 = 7;
const SHORT_WORD_MAX: usize = 2;
const SPELLING_EDGE: usize = 2;
const SPELLING_MIN: usize = 4;
const SPELLING_MODULUS: u64 = 5;
const TEXT_FIELD_MODULUS: u64 = 7;
const WORD_MAX: usize = 32;
const WRONG_BEFORE_MODULUS: u64 = 5;
const CHOICE_DIGITS: usize = 8;
/// The trigger and the application of a frame are drawn from these, in this order.
const TRIGGERS: [&str; 6] = ["space", "space", "space", "pause", "punctuation", "enter"];
const APPLICATIONS: [&str; 4] = ["", "Telegram", "chrome", "Code"];
const TECHNICAL_BEFORE: [&str; 6] = ["запусти ", "в сообщении написано ", "const value = ", "return ", "$ ", "the command is "];
const TECHNICAL_APPLICATIONS: [&str; 4] = ["Code", "WindowsTerminal", "Telegram", ""];
const TECHNICAL_TRIGGERS: [&str; 4] = ["space", "pause", "enter", "punctuation"];
pub const REAL_PHRASE: &str = "real_phrase_layout_intervention";
pub const SPELLING_KEEP: &str = "synthetic_spelling_keep";
pub const TECHNICAL_KEEP: &str = "authored_technical_keep";

#[derive(Clone, Debug)]
pub struct Frame {
    pub identifier: String,
    pub cluster: String,
    pub split: &'static str,
    pub locale: String,
    pub original: String,
    pub alternative: String,
    pub group: i64,
    pub before: String,
    pub after: String,
    pub application: &'static str,
    pub role: &'static str,
    pub trigger: &'static str,
    pub action: &'static str,
    pub focus_family: String,
    pub category: &'static str,
}

impl Frame {
    /// The frame's `__dict__` under `canonical`: its keys in sorted order.
    fn canonical_into(&self, text: &mut String) {
        let string = |value: &str| json_string(value, false);
        text.push_str(&format!(
            "{{\"action\":{},\"after\":{},\"alternative\":{},\"application\":{},\"before\":{},\"category\":{},\"cluster\":{},\"focus_family\":{},\"group\":{},\"identifier\":{},\"locale\":{},\"original\":{},\"role\":{},\"split\":{},\"trigger\":{}}}",
            string(self.action), string(&self.after), string(&self.alternative), string(self.application), string(&self.before), string(self.category), string(&self.cluster),
            string(&self.focus_family), self.group, string(&self.identifier), string(&self.locale), string(&self.original), string(self.role), string(self.split), string(self.trigger),
        ));
    }
}

/// `hashlib.sha256(canonical([row.__dict__ for row in frames])).hexdigest()`.
#[must_use]
pub fn frames_sha256(frames: &[Frame]) -> String {
    let mut hasher = Sha256::new();
    let mut text = String::new();
    hasher.update(b"[");
    for (index, frame) in frames.iter().enumerate() {
        text.clear();
        if index > 0 {
            text.push(',');
        }
        frame.canonical_into(&mut text);
        hasher.update(text.as_bytes());
    }
    hasher.update(b"]\n");
    hex(&hasher.finalize())
}

/// The deterministic number a frame's choices are taken from.
#[must_use]
pub fn draw(key: &str) -> u64 {
    u64::from_str_radix(hex_digest(key).get(..CHOICE_DIGITS).unwrap_or_default(), 16).expect("a digest")
}

/// The focus family belongs to the separate lexical test track.
fn lexical_holdout(family: &str) -> bool {
    draw(&format!("focus-holdout:{family}")).is_multiple_of(LEXICAL_HOLDOUT_MODULUS)
}

fn attached(character: Option<char>) -> bool {
    character.is_some_and(|character| ucd::isalnum(character) || "_'’-‑".contains(character) || category(character).starts_with('M'))
}

fn is_token_letter(character: char) -> bool {
    character.is_ascii_alphabetic() || ('А'..='Я').contains(&character) || ('а'..='я').contains(&character) || character == 'Ё' || character == 'ё'
}

/// `[A-Za-zА-Яа-яЁё]+(?:['’\-][A-Za-zА-Яа-яЁё]+)*` as character positions.
fn tokens(text: &[char]) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut index = 0;
    while index < text.len() {
        if !is_token_letter(text[index]) {
            index += 1;
            continue;
        }
        let start = index;
        while index < text.len() && is_token_letter(text[index]) {
            index += 1;
        }
        while index + 1 < text.len() && matches!(text[index], '\'' | '’' | '-') && is_token_letter(text[index + 1]) {
            index += 1;
            while index < text.len() && is_token_letter(text[index]) {
                index += 1;
            }
        }
        found.push((start, index));
    }
    found
}

fn in_layout(character: char, group: i64) -> bool {
    if !ucd::isalpha(character) {
        return true;
    }
    let lowered = character.to_lowercase().next().unwrap_or(character);
    if group == 0 { lowered.is_ascii_lowercase() } else { ('а'..='я').contains(&lowered) || lowered == 'ё' }
}

/// Word spans of a phrase that are whole natural words of the layout, and no suffix of an identifier or an address.
fn word_spans(text: &[char], group: i64) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    for (start, end) in tokens(text) {
        let before = if start > 0 { Some(text[start - 1]) } else { None };
        let after = text.get(end).copied();
        if attached(before) || attached(after) || !(1..=WORD_MAX).contains(&(end - start)) {
            continue;
        }
        if !text[start..end].iter().all(|character| in_layout(*character, group)) {
            continue;
        }
        let mut left = start;
        while left > 0 && !ucd::isspace(text[left - 1]) {
            left -= 1;
        }
        let mut right = end;
        while right < text.len() && !ucd::isspace(text[right]) {
            right += 1;
        }
        let envelope = &text[left..right];
        if envelope.iter().any(|character| matches!(character, '@' | '_' | '/' | '\\' | '`') || ucd::isdigit(*character)) {
            continue;
        }
        spans.push((start, end));
    }
    spans
}

fn text_of(characters: &[char]) -> String {
    characters.iter().collect()
}

fn smaller(left: &str, right: &str) -> String {
    let (left, right) = (pystr::casefold(left), pystr::casefold(right));
    if right < left { right } else { left }
}

/// `build`: the first candidate's frames; the phrase reserve is never made into a row.
pub fn build(phrases: &[Assigned<'_>]) -> Result<Vec<Frame>, String> {
    let layouts = pair();
    let translate = |text: &str, from: &str, to: &str| layouts.translate(text, from, to).map_err(|_| "a phrase off the layout".to_owned());
    let mut frames = Vec::new();
    for item in phrases {
        if item.split == "reserve" {
            continue;
        }
        let phrase = item.phrase;
        let target: i64 = i64::from(phrase.locale != "eng");
        let (own, other) = if target == 0 { ("us", "ru") } else { ("ru", "us") };
        let text: Vec<char> = phrase.text.chars().collect();
        let matches = word_spans(&text, target);
        let Some(first) = matches.first().copied() else { continue };
        let mut ordered = matches.clone();
        ordered.sort_by_cached_key(|(start, _)| hex_digest(&format!("focus:{}:{start}", phrase.identifier)));
        let mut selected = Vec::with_capacity(ordered.len() + 1);
        if first.1 - first.0 <= SHORT_WORD_MAX {
            selected.push(first);
        }
        selected.extend(ordered);
        let mut positions = BTreeSet::new();
        for (start, end) in selected {
            if !positions.insert(start) {
                continue;
            }
            if positions.len() > FOCUS_POSITIONS {
                break;
            }
            let token = text_of(&text[start..end]);
            let wrong = translate(&token, own, other)?;
            if wrong == token || translate(&wrong, other, own)? != token {
                continue;
            }
            let key = format!("tatoeba:{}:{start}", phrase.identifier);
            let sample = draw(&key);
            let mut before = text_of(&text[start.saturating_sub(BEFORE_MAX)..start]);
            if sample.is_multiple_of(WRONG_BEFORE_MODULUS) {
                before = translate(&before, own, other)?;
            }
            let text_field = sample.is_multiple_of(TEXT_FIELD_MODULUS);
            let after = if text_field { text_of(&text[end..(end + AFTER_MAX).min(text.len())]) } else { String::new() };
            let trigger = TRIGGERS[(sample % TRIGGERS.len().to::<u64>()).to::<usize>()];
            let application = APPLICATIONS[(sample % APPLICATIONS.len().to::<u64>()).to::<usize>()];
            let role = if text_field { "text" } else { "unknown" };
            let family = smaller(&token, &wrong);
            let lexical = lexical_holdout(&family);
            if lexical && item.split != "test" {
                continue;
            }
            let split = if lexical { "lexical_test" } else { item.split };
            let length = pystr::len(&token);
            let blank = pystr::strip(&before).is_empty() && pystr::strip(&after).is_empty();
            let base = |identifier: String, original: &str, alternative: &str, group: i64, action: &'static str, family: &str, category: &'static str| Frame {
                identifier,
                cluster: item.group.clone(),
                split,
                locale: phrase.locale.clone(),
                original: original.to_owned(),
                alternative: alternative.to_owned(),
                group,
                before: before.clone(),
                after: after.clone(),
                application,
                role,
                trigger,
                action,
                focus_family: family.to_owned(),
                category,
            };
            frames.push(base(format!("{key}:keep"), &token, &wrong, target, "keep", &family, REAL_PHRASE));
            let desired = if length <= SHORT_WORD_MAX && blank {
                if trigger == "enter" || trigger == "punctuation" { "suggest" } else { "wait" }
            } else {
                "convert"
            };
            frames.push(base(format!("{key}:convert"), &wrong, &token, 1 - target, desired, &family, REAL_PHRASE));
            // A spelling error is not automatically a layout error. The intended language here is known from the
            // intervention, not inferred from dictionary membership or a teacher's prediction.
            if split != "lexical_test" && sample.is_multiple_of(SPELLING_MODULUS) && length >= SPELLING_MIN && pystr::isalpha(&token) {
                let position = 1 + (sample % (length - SPELLING_EDGE).to::<u64>()).to::<usize>();
                let typo: String = token.chars().enumerate().filter(|(index, _)| *index != position).map(|(_, character)| character).collect();
                let alternate = translate(&typo, own, other)?;
                let typo_family = smaller(&typo, &alternate);
                if !lexical_holdout(&typo_family) {
                    frames.push(base(format!("{key}:spelling"), &typo, &alternate, target, "keep", &typo_family, SPELLING_KEEP));
                }
            }
        }
    }
    Ok(frames)
}

/// `technical_frames`: a small authored safety curriculum; expansions are never counted as sources.
pub fn technical_frames(path: &Path) -> Result<Vec<Frame>, String> {
    let payload = crate::corpus::read_json(path)?;
    if payload.get("schema_version").and_then(Value::as_i64) != Some(1) {
        return Err("invalid safety curriculum".to_owned());
    }
    let tokens = payload.get("technical").and_then(Value::as_array).ok_or("invalid technical tokens")?;
    let layouts = pair();
    let mut rows = Vec::new();
    for value in tokens {
        let value = value.as_str().ok_or("invalid technical tokens")?;
        let alternate = layouts.translate(value, "us", "ru").map_err(|_| "a token off the layout")?;
        let family = smaller(value, &alternate);
        let bucket = draw(&format!("keyswitch:context-v2:safety:{family}")) % SAFETY_BUCKETS;
        let split = if bucket < SAFETY_TRAIN_BUCKETS {
            "train"
        } else if bucket == SAFETY_DEVELOPMENT_BUCKET {
            "development"
        } else if bucket == SAFETY_CALIBRATION_BUCKET {
            "calibration"
        } else {
            "test"
        };
        let lexical = lexical_holdout(&family);
        if lexical && split != "test" {
            continue;
        }
        let split = if lexical { "lexical_test" } else { split };
        for (offset, before) in TECHNICAL_BEFORE.iter().enumerate() {
            for application in TECHNICAL_APPLICATIONS {
                for trigger in TECHNICAL_TRIGGERS {
                    rows.push(Frame {
                        identifier: format!("authored:{value}:{offset}:{application}:{trigger}"),
                        cluster: format!("authored:{family}"),
                        split,
                        locale: "eng".to_owned(),
                        original: value.to_owned(),
                        alternative: alternate.clone(),
                        group: 0,
                        before: (*before).to_owned(),
                        after: String::new(),
                        application,
                        role: "unknown",
                        trigger,
                        action: "keep",
                        focus_family: family.clone(),
                        category: TECHNICAL_KEEP,
                    });
                }
            }
        }
    }
    Ok(rows)
}

/// `all_frames`: the frames of the phrase snapshot, then the authored ones.
pub fn all_frames(root: &Path) -> Result<Vec<Frame>, String> {
    let phrases = load_phrases(root)?;
    let mut frames = build(&assign(&phrases))?;
    frames.extend(technical_frames(&root.join("model/context_v2/safety.json"))?);
    let mut seen = std::collections::HashSet::with_capacity(frames.len());
    if !frames.iter().all(|row| seen.insert(row.identifier.as_str())) {
        return Err("duplicate frame IDs".to_owned());
    }
    Ok(frames)
}
