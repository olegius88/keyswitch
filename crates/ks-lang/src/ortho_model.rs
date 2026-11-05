//! Orthotactic plausibility over the physical key sequence.
//!
//! The engine observes which keys were pressed, not which letters appeared. Because the us/ru map is a bijection on
//! the mapped keys, one key sequence has exactly two readings, and a character model per language over that shared
//! space turns the question "was the layout wrong?" into a likelihood ratio that needs no dictionary at all. That is
//! what lets an unknown token such as `htop` be judged: the model never asks whether `htop` is a word, only whether
//! `рещз` is a worse Russian sequence than `htop` is an English one.
//!
//! The ratio alone is not enough, and the shape channel is why. The worst genuine Russian sequences under the ratio
//! are abbreviations - РСФСР, ЛДПР, РНК - whose key renderings look thoroughly English. A person writing an
//! abbreviation writes it in capitals, so an all-lowercase token cannot be excused as one. Case is therefore counted
//! evidence here, not a veto.
//!
//! An artifact of schema 2 adds token features to that score, each with a weight in nats per direction: whether the
//! Latin reading is a known command, whether the token repeats itself around a hyphen, whether both halves of a
//! hyphenated reading are dictionary words, and whether the Latin reading is a dot and a dictionary word. Schema 3
//! adds `extra_key`: the reading in the target language is no word but becomes one without one of its letters. Schema
//! 4 adds `stretch_runs`: a run of at least that many identical keys is read both ways, and the reading that argues
//! less for the other layout wins; it also weighs `source_word`. Schema 5 reads a hyphen between two copies of one
//! key as a drawn-out sound, lets compound evidence on both sides cancel, may require of a compound a half of some
//! length (`compound_min_letters`), and weighs `target_unknown`, `name_unknown` and `source_plausibility`. Schema 6
//! adds `target_plausibility` and may count a feature only for a reading less plausible than the centre of the layout
//! typed (`gated_features`). A schema 2 artifact may also carry a second channel for a script, used only when that
//! script is the source of the evidence (`source_models`).
//!
//! This module only scores. It never decides, never injects and never overrides an explicit user rule; the policy
//! layer applies it.

use ks_cast::To as _;
use crate::constants::{ORTHO_FEATURE_ARTIFACT_SCHEMA_VERSION, ORTHO_EXTRA_KEY_ARTIFACT_SCHEMA_VERSION, ORTHO_STRETCH_ARTIFACT_SCHEMA_VERSION, ORTHO_REPLACEMENT_ARTIFACT_SCHEMA_VERSION, ORTHO_PLAUSIBILITY_ARTIFACT_SCHEMA_VERSION, CONTEXT_TYPO_MIN_CHARACTERS, ACRONYM_MIN_LETTERS, ORTHO_CHANNEL_MAX_GRAMS, ORTHO_ARTIFACT_MAX_BYTES, ORTHO_MIN_ORDER, ORTHO_MAX_ORDER, ORTHO_MAX_SCALE, ORTHO_MAX_MINIMUM_LENGTH_CHARACTERS, ORTHO_MIN_COLLAPSED_RUN, ORTHO_MIN_STRETCH_RUN};
use crate::load_error::{json_text, read_limited, LoadError};
use crate::packed_grams::GramTable;
use crate::pysum::py_sum;
use crate::resources::Resources;
use crate::tables::WordList;
use indexmap::IndexMap;
use ks_text::layouts::LayoutPair;
use ks_text::{min_by_key_first, normalize, pystr, ucd};
use serde::de::{self, DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

/// `ARTIFACT_PATH` under the `resources` folder of the install.
#[must_use]
pub fn artifact_path(resource_dir: &Path) -> PathBuf {
    resource_dir.join("models").join("ortho_v1.json")
}

pub const SCRIPTS: [&str; 2] = ["en", "ru"];
pub const SHAPES: [&str; 4] = ["lower", "initial", "inner", "upper"];
pub const BOS: char = '\x02';
pub const EOS: char = '\x03';
/// The token features a schema-2 artifact weighs, in the order the trainer fits them.
pub const FEATURES_SCHEMA_2: [&str; 5] = ["ident", "repeat", "parts_source", "parts_target", "dot_word"];
pub const FEATURES_SCHEMA_3: [&str; 6] = ["ident", "repeat", "parts_source", "parts_target", "dot_word", "extra_key"];
pub const FEATURES_SCHEMA_4: [&str; 7] = ["ident", "repeat", "parts_source", "parts_target", "dot_word", "extra_key", "source_word"];
pub const FEATURES_SCHEMA_5: [&str; 10] =
    ["ident", "repeat", "parts_source", "parts_target", "dot_word", "extra_key", "source_word", "name_unknown", "target_unknown", "source_plausibility"];
pub const FEATURES: [&str; 11] = [
    "ident", "repeat", "parts_source", "parts_target", "dot_word", "extra_key", "source_word", "name_unknown", "target_unknown", "source_plausibility",
    "target_plausibility",
];
/// The features that are not indicators: how word-like the reading is in the layout typed - the source channel's
/// log-probability per scored position - and how word-like the replacement is in its own language - the target
/// channel's -, each measured from the centre the artifact carries.
pub const CONTINUOUS_FEATURE: &str = "source_plausibility";
pub const TARGET_FEATURE: &str = "target_plausibility";

/// `SCHEMA_FEATURES.get(schema)`: the features an artifact of each schema weighs.
#[must_use]
pub fn schema_features(schema: i64) -> Option<&'static [&'static str]> {
    match schema {
        ORTHO_FEATURE_ARTIFACT_SCHEMA_VERSION => Some(&FEATURES_SCHEMA_2),
        ORTHO_EXTRA_KEY_ARTIFACT_SCHEMA_VERSION => Some(&FEATURES_SCHEMA_3),
        ORTHO_STRETCH_ARTIFACT_SCHEMA_VERSION => Some(&FEATURES_SCHEMA_4),
        ORTHO_REPLACEMENT_ARTIFACT_SCHEMA_VERSION => Some(&FEATURES_SCHEMA_5),
        ORTHO_PLAUSIBILITY_ARTIFACT_SCHEMA_VERSION => Some(&FEATURES),
        _ => None,
    }
}

/// The case shapes of a token written with a capital letter as a word, not as an abbreviation.
pub const CAPITALISED: [&str; 2] = ["initial", "inner"];
/// The dictionaries the engine loads for the two scripts.
pub const LOCALES: [(&str, &str); 2] = [("en", "en_US"), ("ru", "ru_RU")];

/// Answers whether a Latin string is a known command.
pub type Identifier = Arc<dyn Fn(&str) -> bool + Send + Sync>;
/// Answers whether a word is in the dictionary of a script: `known(script, word)`.
pub type KnownWord = Arc<dyn Fn(&str, &str) -> bool + Send + Sync>;
/// The same question, borrowed for one call.
pub type KnownRef<'a> = &'a dyn Fn(&str, &str) -> bool;

/// One value per script of the model.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PerScript<T> {
    pub en: T,
    pub ru: T,
}

impl<T> PerScript<T> {
    /// The value of `"en"`, of `"ru"` otherwise; a script is checked where it comes in.
    pub fn get(&self, script: &str) -> &T {
        if script == "en" { &self.en } else { &self.ru }
    }
}

/// One token as the engine saw it: the keys, the case shape, the layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrthoEvidence<'a> {
    pub keys: &'a str,
    pub shape: &'a str,
    pub source_script: &'a str,
}

/// Nats in favour of reading the keys in the other layout.
#[derive(Clone, Debug, PartialEq)]
pub struct OrthoScore {
    pub ratio: f64,
    pub shape_ratio: f64,
    pub total: f64,
    pub source_logprob: f64,
    pub target_logprob: f64,
    pub supported: bool,
    pub features: f64,
    /// The keys the features read: the token as typed, or with a schema 4 artifact the reading kept.
    pub reading: String,
}

/// The token as typed and as it reads in the other layout; the keys are the Latin reading.
#[must_use]
pub fn readings(keys: &str, source_script: &str) -> (String, String) {
    let cyrillic = LayoutPair::default().translate(keys, "us", "ru").expect("the us/ru pair is supported");
    if source_script == "en" { (keys.to_string(), cyrillic) } else { (cyrillic, keys.to_string()) }
}

/// The two parts of a text split by hyphens when there are exactly two, `parts` of the Python.
fn two_parts(text: &str) -> Option<(&str, &str)> {
    let mut parts = text.split('-');
    let (head, tail) = (parts.next()?, parts.next()?);
    (parts.next().is_none() && !head.is_empty() && !tail.is_empty()).then_some((head, tail))
}

/// A token that repeats itself around one hyphen: `у-у`, `мда-а`, `ну-ну`.
#[must_use]
pub fn reduplicated(text: &str) -> bool {
    let Some((head, tail)) = two_parts(text) else {
        return false;
    };
    head.ends_with(tail) || head.contains(tail) || head.chars().last() == tail.chars().last()
}

/// The text with every run of one character longer than `limit` cut to `limit`: `каеффф` → `каефф`.
#[must_use]
pub fn collapse_runs(text: &str, limit: i64) -> String {
    let mut result = String::with_capacity(text.len());
    let mut last = None;
    let mut run = 0;
    for character in text.chars() {
        run = if last == Some(character) { run + 1 } else { 1 };
        if run <= limit {
            result.push(character);
            last = Some(character);
        }
    }
    result
}

/// The text with every run of at least `minimum` identical characters read once: `муууу` → `му`.
#[must_use]
pub fn read_once(text: &str, minimum: i64) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut result = String::with_capacity(text.len());
    let mut start = 0;
    for index in 1..=chars.len() {
        if index == chars.len() || chars[index] != chars[start] {
            let length = (index - start).to::<i64>();
            if length >= minimum {
                result.push(chars[start]);
            } else {
                result.extend(&chars[start..index]);
            }
            start = index;
        }
    }
    result
}

/// The text without every hyphen that stands between two copies of one character, and without the second copy: a
/// drawn-out sound (`ти-ише` → `тише`, `ура-а-а` → `ура`), not a boundary of parts.
#[must_use]
pub fn drop_hyphen_stretches(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut result = String::with_capacity(text.len());
    let mut last = None;
    let mut index = 0;
    while index < chars.len() {
        let stretched = chars[index] == '-' && last.is_some() && last != Some('-') && index + 1 < chars.len() && Some(chars[index + 1]) == last;
        if !stretched {
            result.push(chars[index]);
            last = Some(chars[index]);
        }
        index += 1 + usize::from(stretched);
    }
    result
}

/// The readings of a token the character models judge: as typed with a stretch cut to `collapse` and, with `stretch`,
/// also with every run of at least `stretch` keys read once - with `hyphens`, and every hyphen stretch dropped.
#[must_use]
pub fn stretch_readings(keys: &str, collapse: Option<i64>, stretch: Option<i64>, hyphens: bool) -> Vec<String> {
    let typed = collapse.map_or_else(|| keys.to_string(), |limit| collapse_runs(keys, limit));
    let Some(stretch) = stretch else {
        return vec![typed];
    };
    let once = if hyphens { read_once(&drop_hyphen_stretches(keys), stretch) } else { read_once(keys, stretch) };
    let once = collapse.map_or_else(|| once.clone(), |limit| collapse_runs(&once, limit));
    if once == typed { vec![typed] } else { vec![typed, once] }
}

fn compound_known(text: &str, script: &str, known: &dyn Fn(&str, &str) -> bool, min_letters: i64) -> bool {
    let Some((head, tail)) = two_parts(text) else {
        return false;
    };
    (pystr::len(head).max(pystr::len(tail))).to::<i64>() >= min_letters && known(script, head) && known(script, tail)
}

/// A reading that is no word of the dictionary but becomes one without one of its letters.
pub fn one_key_extra(text: &str, script: &str, known: &dyn Fn(&str, &str) -> bool) -> bool {
    if pystr::len(text) < CONTEXT_TYPO_MIN_CHARACTERS || !pystr::isalpha(text) || known(script, text) {
        return false;
    }
    let chars: Vec<char> = text.chars().collect();
    (0..chars.len()).any(|index| {
        let shorter: String = chars[..index].iter().chain(&chars[index + 1..]).collect();
        known(script, &shorter)
    })
}

/// The values `token_features` gives, each 0 or 1; `as_pairs` lists them in the order of the Python's dict.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TokenFeatures {
    pub ident: i64,
    pub repeat: i64,
    pub parts_source: i64,
    pub parts_target: i64,
    pub dot_word: i64,
    pub extra_key: i64,
    pub source_word: i64,
    pub name_unknown: i64,
    pub target_unknown: i64,
}

impl TokenFeatures {
    #[must_use]
    pub fn as_pairs(&self) -> [(&'static str, i64); 9] {
        [
            ("ident", self.ident),
            ("repeat", self.repeat),
            ("parts_source", self.parts_source),
            ("parts_target", self.parts_target),
            ("dot_word", self.dot_word),
            ("extra_key", self.extra_key),
            ("source_word", self.source_word),
            ("name_unknown", self.name_unknown),
            ("target_unknown", self.target_unknown),
        ]
    }
}

/// The feature values of one token, from its keys, the layout it was typed in and its case shape.
///
/// `identifier` answers whether a Latin string is a known command, `known` whether a word is in the dictionary of a
/// script. The trainer passes the frozen answers, the runtime the packaged list and the engine's dictionaries. With
/// `symmetric` (schema 5) a token whose readings on both sides are compounds of dictionary words (`В-к` is `D-r`)
/// gives no compound evidence either way. `compound_min_letters` (schema 5) names, per script, how long the longer
/// half of a compound of that script's words must be: two halves that short are words of its dictionary by chance.
pub fn token_features(
    keys: &str,
    source_script: &str,
    identifier: &dyn Fn(&str) -> bool,
    known: &dyn Fn(&str, &str) -> bool,
    shape: &str,
    symmetric: bool,
    compound_min_letters: &PerScript<Option<i64>>,
) -> TokenFeatures {
    let target_script = if source_script == "en" { "ru" } else { "en" };
    let (source, target) = readings(keys, source_script);
    let unknown = pystr::isalpha(&target) && !known(target_script, &target);
    let mut parts_source = compound_known(&source, source_script, known, compound_min_letters.get(source_script).unwrap_or(0));
    let mut parts_target = compound_known(&target, target_script, known, compound_min_letters.get(target_script).unwrap_or(0));
    if symmetric && parts_source && parts_target {
        parts_source = false;
        parts_target = false;
    }
    TokenFeatures {
        ident: i64::from(identifier(keys)),
        repeat: i64::from(reduplicated(&source) || reduplicated(&target)),
        parts_source: i64::from(parts_source),
        parts_target: i64::from(parts_target),
        dot_word: i64::from(source_script == "ru" && pystr::len(keys) > 1 && keys.starts_with('.') && known("en", &keys[1..])),
        extra_key: i64::from(one_key_extra(&target, target_script, known)),
        source_word: i64::from(known(source_script, &source)),
        name_unknown: i64::from(unknown && CAPITALISED.contains(&shape)),
        target_unknown: i64::from(unknown),
    }
}

/// The case shape of a token as typed, in the vocabulary the model counts.
pub fn shape_of(token: &str, first_in_field: bool) -> &'static str {
    let mut letters = token.chars().filter(|&c| ucd::isalpha(c));
    let count = letters.clone().count();
    if count >= ACRONYM_MIN_LETTERS && letters.all(ucd::isupper) {
        return "upper";
    }
    if token.chars().next().is_some_and(ucd::isupper) {
        return if first_in_field { "initial" } else { "inner" };
    }
    "lower"
}

/// One language's character model in key space, ARPA-shaped for lookup.
#[derive(Clone, Debug)]
pub struct Channel {
    pub order: usize,
    pub logprob: GramTable,
    pub backoff: GramTable,
    pub uniform: f64,
}

impl Channel {
    /// log P(last | context), walking the backoff chain to the unigram.
    fn conditional(&self, gram: &[char]) -> f64 {
        let mut gram = gram;
        let mut accumulated = 0.0;
        loop {
            if let Some(found) = self.logprob.get_chars(gram) {
                return accumulated + found;
            }
            if gram.len() == 1 {
                return accumulated + self.uniform;
            }
            accumulated += self.backoff.get_chars(&gram[..gram.len() - 1]).unwrap_or(0.0);
            gram = &gram[1..];
        }
    }

    /// log P(keys) with an explicit boundary, so length is scored honestly.
    #[must_use]
    pub fn score(&self, keys: &str) -> f64 {
        let mut padded: Vec<char> = Vec::with_capacity(keys.len() + self.order);
        padded.resize(self.order - 1, BOS);
        padded.extend(keys.chars());
        padded.push(EOS);
        py_sum(padded.windows(self.order).map(|gram| self.conditional(gram)))
    }
}

// ---------------------------------------------------------------- reading the artifact

/// A channel of the file, compacted as soon as it is read: the file holds about a million grams.
struct RawChannel {
    logprob: GramTable,
    backoff: GramTable,
    uniform: i64,
}

/// `isinstance(value, int)`: an integer or a boolean, one element of a list of weights.
struct IntValue(i64);

impl<'de> de::Deserialize<'de> for IntValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct IntVisitor;
        impl Visitor<'_> for IntVisitor {
            type Value = IntValue;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an integer")
            }
            fn visit_bool<E>(self, value: bool) -> Result<IntValue, E> {
                Ok(IntValue(i64::from(value)))
            }
            fn visit_i64<E>(self, value: i64) -> Result<IntValue, E> {
                Ok(IntValue(value))
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<IntValue, E> {
                i64::try_from(value).map(IntValue).map_err(|_| E::custom("an integer out of range"))
            }
        }
        deserializer.deserialize_any(IntVisitor)
    }
}

/// A weight as the tables store it. A weight outside 32 bits, a million nats at the largest scale, is refused.
fn weight<E: de::Error>(value: IntValue) -> Result<i32, E> {
    i32::try_from(value.0).map_err(|_| E::custom("invalid orthotactic weight"))
}

struct IntList(Vec<i32>);

impl<'de> de::Deserialize<'de> for IntList {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ListVisitor;
        impl<'de> Visitor<'de> for ListVisitor {
            type Value = IntList;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a list of integers")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<IntList, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = seq.next_element::<IntValue>()? {
                    values.push(weight::<A::Error>(value)?);
                }
                Ok(IntList(values))
            }
        }
        deserializer.deserialize_any(ListVisitor)
    }
}

/// `[name, value]` of a backoff list, appended to the two lists.
struct BackoffEntry<'a>(&'a mut WordList, &'a mut Vec<i32>);

impl<'de> DeserializeSeed<'de> for BackoffEntry<'_> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for BackoffEntry<'_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid orthotactic backoff")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        let invalid = || <A::Error as de::Error>::custom("invalid orthotactic backoff");
        seq.next_element_seed(PushText(self.0))?.ok_or_else(invalid)?;
        let value = seq.next_element::<IntValue>()?.ok_or_else(invalid)?;
        self.1.push(weight::<A::Error>(value)?);
        // ORTHO_BACKOFF_ENTRY_FIELDS: a stored backoff weight is a [context, weight] pair.
        if seq.next_element::<IgnoredAny>()?.is_some() {
            return Err(invalid());
        }
        Ok(())
    }
}

/// A string appended to a list without an allocation of its own.
struct PushText<'a>(&'a mut WordList);

impl<'de> DeserializeSeed<'de> for PushText<'_> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl Visitor<'_> for PushText<'_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a string")
    }
    fn visit_str<E>(self, value: &str) -> Result<(), E> {
        self.0.push(value);
        Ok(())
    }
}

struct BackoffList(WordList, Vec<i32>);

impl<'de> de::Deserialize<'de> for BackoffList {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ListVisitor;
        impl<'de> Visitor<'de> for ListVisitor {
            type Value = BackoffList;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a backoff list")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<BackoffList, A::Error> {
                let (mut names, mut values) = (WordList::new(), Vec::new());
                while seq.next_element_seed(BackoffEntry(&mut names, &mut values))?.is_some() {}
                Ok(BackoffList(names, values))
            }
        }
        deserializer.deserialize_any(ListVisitor)
    }
}

impl<'de> de::Deserialize<'de> for RawChannel {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ChannelVisitor;
        impl<'de> Visitor<'de> for ChannelVisitor {
            type Value = RawChannel;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("invalid orthotactic channel")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<RawChannel, A::Error> {
                let (mut grams, mut logprob, mut backoff, mut uniform) = (None::<String>, None::<IntList>, None::<BackoffList>, None::<IntValue>);
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "grams" => grams = Some(map.next_value()?),
                        "logprob" => logprob = Some(map.next_value()?),
                        "backoff" => backoff = Some(map.next_value()?),
                        "uniform" => uniform = Some(map.next_value()?),
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                let invalid = || <A::Error as de::Error>::custom("invalid orthotactic channel");
                let (grams, logprob, backoff, uniform) =
                    (grams.ok_or_else(invalid)?, logprob.ok_or_else(invalid)?, backoff.ok_or_else(invalid)?, uniform.ok_or_else(invalid)?);
                let names = if grams.is_empty() { 0 } else { grams.split('\n').count() };
                if names != logprob.0.len() || names > ORTHO_CHANNEL_MAX_GRAMS {
                    return Err(de::Error::custom("orthotactic channel is inconsistent"));
                }
                let table = if grams.is_empty() { GramTable::from_raw(std::iter::empty(), &[]) } else { GramTable::from_raw(grams.split('\n'), &logprob.0) };
                drop(grams);
                drop(logprob);
                let backoff = GramTable::from_raw(backoff.0.iter(), &backoff.1);
                Ok(RawChannel { logprob: table, backoff, uniform: uniform.0 })
            }
        }
        deserializer.deserialize_any(ChannelVisitor)
    }
}

/// A map of script to channel; the checks on its keys come later, as in the Python.
struct ChannelMap(Vec<(String, RawChannel)>);

impl ChannelMap {
    fn take(&mut self, script: &str) -> Option<RawChannel> {
        self.0.iter().position(|entry| entry.0 == script).map(|index| self.0.swap_remove(index).1)
    }

    fn has_exactly_scripts(&self) -> bool {
        SCRIPTS.iter().all(|script| self.0.iter().any(|entry| entry.0 == *script)) && self.has_only_scripts()
    }

    fn has_only_scripts(&self) -> bool {
        self.0.iter().all(|entry| SCRIPTS.contains(&entry.0.as_str()))
    }
}

impl<'de> de::Deserialize<'de> for ChannelMap {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct MapVisitor;
        impl<'de> Visitor<'de> for MapVisitor {
            type Value = ChannelMap;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a map of channels")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<ChannelMap, A::Error> {
                let mut channels: Vec<(String, RawChannel)> = Vec::new();
                while let Some(key) = map.next_key::<String>()? {
                    let channel = map.next_value()?;
                    channels.retain(|entry| entry.0 != key);
                    channels.push((key, channel));
                }
                Ok(ChannelMap(channels))
            }
        }
        deserializer.deserialize_any(MapVisitor)
    }
}

/// The file: the channels compacted on the way, every other field as a small tree.
struct Payload {
    fields: Map<String, Value>,
    models: Option<ChannelMap>,
    source_models: Option<ChannelMap>,
}

impl<'de> de::Deserialize<'de> for Payload {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct PayloadVisitor;
        impl<'de> Visitor<'de> for PayloadVisitor {
            type Value = Payload;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("unsupported orthotactic artifact")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Payload, A::Error> {
                let mut payload = Payload { fields: Map::new(), models: None, source_models: None };
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "models" => payload.models = Some(map.next_value()?),
                        "source_models" => payload.source_models = Some(map.next_value()?),
                        _ => {
                            let value = map.next_value()?;
                            payload.fields.insert(key, value);
                        }
                    }
                }
                Ok(payload)
            }
        }
        deserializer.deserialize_any(PayloadVisitor)
    }
}

/// `isinstance(value, int)`: `True` is the integer 1.
fn int_like(value: &Value) -> Option<i64> {
    match value {
        Value::Bool(flag) => Some(i64::from(*flag)),
        Value::Number(number) => number.as_i64(),
        _ => None,
    }
}

/// `type(value) is int`.
fn strict_int(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64(),
        _ => None,
    }
}

/// The value of `schema_version` when it equals a whole number in Python: `5`, `5.0` and, for 1, `true`.
fn schema_number(value: &Value) -> Option<i64> {
    match value {
        Value::Bool(flag) => Some(i64::from(*flag)),
        Value::Number(number) => number.as_i64().or_else(|| number.as_f64().filter(|x| x.fract() == 0.0 && x.abs() < 1e15).map(ks_cast::To::to::<i64>)),
        _ => None,
    }
}

fn has_exact_keys(table: &Map<String, Value>, names: &[&str]) -> bool {
    table.len() == names.len() && names.iter().all(|name| table.contains_key(*name))
}

/// A list a Python `set()` is built of raises `TypeError` on a list or a dict inside.
fn unhashable(list: &[Value]) -> bool {
    list.iter().any(|item| matches!(item, Value::Array(_) | Value::Object(_)))
}

fn value_error<T>(message: &str) -> Result<T, LoadError> {
    Err(LoadError::value(message))
}

/// Loaded orthotactic model. Construction validates; scoring never raises.
pub struct OrthoModel {
    pub order: usize,
    pub channels: PerScript<Channel>,
    /// The channel a script is scored with when the token was typed in it, if it differs.
    pub source_channels: PerScript<Option<Channel>>,
    /// Per script, in the order of `SHAPES`.
    pub prose_shape: PerScript<[f64; 4]>,
    pub acronym_shape: PerScript<[f64; 4]>,
    pub version: String,
    pub thresholds: PerScript<f64>,
    pub minimum_length: i64,
    /// `None` is the empty dict of a schema 1 artifact; the weights are in the order of the schema's features.
    pub features: Option<PerScript<Vec<(&'static str, f64)>>>,
    /// A schema 2 artifact may judge a stretched token by its word (`collapse`) and may declare case shapes it does
    /// not score at all (`unscored_shapes`), as it does not score tokens shorter than `minimum_length`.
    pub collapse: Option<i64>,
    pub unscored_shapes: Vec<String>,
    /// A schema 4 artifact also reads every run of at least `stretch` keys once (`read_once`); a schema 5 artifact
    /// also drops hyphen stretches in that reading (`drop_hyphen_stretches`).
    pub stretch: Option<i64>,
    pub hyphens: bool,
    /// A schema 5 artifact measures `source_plausibility` from these centres (per source script), and may require of
    /// a compound in a script a half of some length (`compound_min_letters`).
    pub centres: Option<PerScript<f64>>,
    pub compound_min_letters: PerScript<Option<i64>>,
    /// A schema 6 artifact also measures `target_plausibility` from these centres (per source script), and may count
    /// a feature only for a reading less plausible than the centre of the layout typed (`gated`).
    pub target_centres: Option<PerScript<f64>>,
    pub gated: Vec<&'static str>,
    identifier: OnceLock<Identifier>,
    known: OnceLock<KnownWord>,
    resources: Option<Arc<Resources>>,
}

impl OrthoModel {
    /// `OrthoModel.load(path, identifier=, known=)`. Without `identifier` or `known` the model asks `resources` when
    /// it first needs them, as the Python loads the packaged list and the engine's dictionaries; without `resources`
    /// too nothing is known.
    pub fn load(path: &Path, identifier: Option<Identifier>, known: Option<KnownWord>, resources: Option<Arc<Resources>>) -> Result<OrthoModel, LoadError> {
        let content = read_limited(path, ORTHO_ARTIFACT_MAX_BYTES)?;
        if content.len() > ORTHO_ARTIFACT_MAX_BYTES {
            return value_error("oversized orthotactic artifact");
        }
        let mut deserializer = serde_json::Deserializer::from_slice(json_text(&content)?);
        let payload: Result<Payload, _> = de::Deserialize::deserialize(&mut deserializer);
        let payload = payload.and_then(|payload| deserializer.end().map(|()| payload)).map_err(|error| LoadError::json(&error))?;
        drop(content);
        let mut model = Self::from_payload(payload)?;
        if let Some(identifier) = identifier {
            let _ = model.identifier.set(identifier);
        }
        if let Some(known) = known {
            let _ = model.known.set(known);
        }
        model.resources = resources;
        Ok(model)
    }

    /// `OrthoModel.try_load(path)`: the model and its version, or nothing and why.
    #[must_use]
    pub fn try_load(path: &Path, resources: Option<Arc<Resources>>) -> (Option<OrthoModel>, String) {
        match Self::load(path, None, None, resources) {
            Ok(model) => {
                let version = model.version.clone();
                (Some(model), version)
            }
            Err(error) => (None, format!("unavailable: {}", error.name)),
        }
    }

    fn from_payload(payload: Payload) -> Result<OrthoModel, LoadError> {
        let Payload { fields, models, source_models } = payload;
        let Some(schema) = fields.get("schema_version").and_then(schema_number).filter(|&schema| schema == 1 || schema_features(schema).is_some()) else {
            return value_error("unsupported orthotactic artifact");
        };
        let order = fields.get("order").and_then(int_like).filter(|order| (ORTHO_MIN_ORDER..=ORTHO_MAX_ORDER).contains(order));
        let version = fields.get("version").and_then(Value::as_str).filter(|version| version.starts_with("ortho-v1-"));
        let scale = fields.get("scale").and_then(int_like).filter(|scale| (1..=ORTHO_MAX_SCALE).contains(scale));
        let (Some(order), Some(version), Some(scale)) = (order, version, scale) else {
            return value_error("invalid orthotactic identity");
        };
        let order = order.to::<usize>();
        let scale = scale.to::<f64>();
        let mut models = match models {
            Some(models) if models.has_exactly_scripts() => models,
            _ => return value_error("orthotactic artifact must carry both scripts"),
        };
        let channel = |raw: RawChannel| -> Result<Channel, LoadError> {
            if raw.logprob.longest() > order {
                return value_error("invalid orthotactic weight");
            }
            let (mut logprob, mut backoff) = (raw.logprob, raw.backoff);
            logprob.set_scale(scale);
            backoff.set_scale(scale);
            Ok(Channel { order, logprob, backoff, uniform: raw.uniform.to::<f64>() / scale })
        };
        let channels = PerScript {
            en: channel(models.take("en").expect("both scripts are there"))?,
            ru: channel(models.take("ru").expect("both scripts are there"))?,
        };
        let minimum_length = fields.get("minimum_length").and_then(int_like).filter(|length| (1..=ORTHO_MAX_MINIMUM_LENGTH_CHARACTERS).contains(length));
        let raw_thresholds = fields.get("thresholds").and_then(Value::as_object).filter(|table| has_exact_keys(table, &SCRIPTS));
        let (Some(raw_thresholds), Some(minimum_length)) = (raw_thresholds, minimum_length) else {
            return value_error("orthotactic artifact carries no usable decision threshold");
        };
        let threshold = |script: &str| int_like(&raw_thresholds[script]).map(|value| value.to::<f64>() / scale);
        let (Some(en), Some(ru)) = (threshold("en"), threshold("ru")) else {
            return value_error("invalid orthotactic threshold");
        };
        let thresholds = PerScript { en, ru };
        let names = schema_features(schema);
        let features = names.map(|names| Self::_features(fields.get("features"), scale, names)).transpose()?;
        if names.is_none() && (["features", "collapse_runs", "unscored_shapes"].iter().any(|name| fields.contains_key(*name)) || source_models.is_some()) {
            return value_error("a schema 1 orthotactic artifact carries no features");
        }
        let mut sources = match source_models {
            Some(sources) if !sources.has_only_scripts() => return value_error("invalid orthotactic source channels"),
            Some(sources) => sources,
            None => ChannelMap(Vec::new()),
        };
        let source_channels = PerScript { en: sources.take("en").map(channel).transpose()?, ru: sources.take("ru").map(channel).transpose()? };
        let collapse = match fields.get("collapse_runs") {
            None | Some(Value::Null) => None,
            Some(value) => match strict_int(value).filter(|&value| value >= ORTHO_MIN_COLLAPSED_RUN) {
                Some(value) => Some(value),
                None => return value_error("invalid orthotactic run collapse"),
            },
        };
        let unscored_shapes = match fields.get("unscored_shapes") {
            None => Vec::new(),
            Some(Value::Array(list)) if unhashable(list) => return Err(LoadError::type_error("unhashable type")),
            Some(Value::Array(list)) => {
                let mut shapes = Vec::new();
                for item in list {
                    match item.as_str() {
                        Some(shape) if SHAPES.contains(&shape) => shapes.push(shape.to_string()),
                        _ => return value_error("invalid orthotactic shape scope"),
                    }
                }
                shapes
            }
            Some(_) => return value_error("invalid orthotactic shape scope"),
        };
        let stretch = fields.get("stretch_runs").filter(|value| !value.is_null());
        if (schema >= ORTHO_STRETCH_ARTIFACT_SCHEMA_VERSION) != stretch.is_some() {
            return value_error("only a schema 4 or 5 orthotactic artifact reads stretches, and it must");
        }
        let stretch = match stretch {
            None => None,
            Some(value) => match strict_int(value).filter(|&value| value >= ORTHO_MIN_STRETCH_RUN) {
                Some(value) => Some(value),
                None => return value_error("invalid orthotactic stretch run"),
            },
        };
        let prose_shape = Self::_shape(fields.get("prose_shape"), scale)?;
        let acronym_shape = Self::_shape(fields.get("acronym_shape"), scale)?;
        let centres = Self::_centres(&fields, schema, scale, "feature_centres", ORTHO_REPLACEMENT_ARTIFACT_SCHEMA_VERSION)?;
        let compound_min_letters = Self::_compound_minimum(&fields, schema)?;
        let target_centres = Self::_centres(&fields, schema, scale, "target_centres", ORTHO_PLAUSIBILITY_ARTIFACT_SCHEMA_VERSION)?;
        let gated = Self::_gated(&fields, schema)?;
        Ok(OrthoModel {
            order,
            channels,
            source_channels,
            prose_shape,
            acronym_shape,
            version: version.to_string(),
            thresholds,
            minimum_length,
            features,
            collapse,
            unscored_shapes,
            stretch,
            hyphens: schema >= ORTHO_REPLACEMENT_ARTIFACT_SCHEMA_VERSION,
            centres,
            compound_min_letters,
            target_centres,
            gated,
            identifier: OnceLock::new(),
            known: OnceLock::new(),
            resources: None,
        })
    }

    fn _features(raw: Option<&Value>, scale: f64, names: &'static [&'static str]) -> Result<PerScript<Vec<(&'static str, f64)>>, LoadError> {
        let Some(raw) = raw.and_then(Value::as_object).filter(|raw| has_exact_keys(raw, &SCRIPTS)) else {
            return value_error("orthotactic artifact must weigh its features for both scripts");
        };
        let table = |script: &str| -> Result<Vec<(&'static str, f64)>, LoadError> {
            let Some(table) = raw[script].as_object().filter(|table| has_exact_keys(table, names)) else {
                return value_error("invalid orthotactic features");
            };
            let mut weights = Vec::with_capacity(names.len());
            for &name in names {
                match int_like(&table[name]) {
                    Some(value) => weights.push((name, value.to::<f64>() / scale)),
                    None => return value_error("invalid orthotactic feature weight"),
                }
            }
            Ok(weights)
        };
        Ok(PerScript { en: table("en")?, ru: table("ru")? })
    }

    /// The length a schema 5 artifact requires of the longer half of a compound, per script.
    fn _compound_minimum(fields: &Map<String, Value>, schema: i64) -> Result<PerScript<Option<i64>>, LoadError> {
        let invalid = "invalid orthotactic compound minimum";
        let empty = Map::new();
        let raw = match fields.get("compound_min_letters") {
            None => &empty,
            Some(Value::Object(raw)) => raw,
            Some(_) => return value_error(invalid),
        };
        let valid = raw.keys().all(|key| SCRIPTS.contains(&key.as_str())) && raw.values().all(|value| strict_int(value).is_some_and(|value| value >= 1));
        if !valid || (!raw.is_empty() && schema < ORTHO_REPLACEMENT_ARTIFACT_SCHEMA_VERSION) {
            return value_error(invalid);
        }
        Ok(PerScript { en: raw.get("en").and_then(strict_int), ru: raw.get("ru").and_then(strict_int) })
    }

    /// Features a schema 6 artifact counts only for a token less plausible than the centre of its layout.
    fn _gated(fields: &Map<String, Value>, schema: i64) -> Result<Vec<&'static str>, LoadError> {
        let invalid = "invalid orthotactic gated features";
        let empty = Vec::new();
        let raw = match fields.get("gated_features") {
            None => &empty,
            Some(Value::Array(raw)) => raw,
            Some(_) => return value_error(invalid),
        };
        if unhashable(raw) {
            return Err(LoadError::type_error("unhashable type"));
        }
        let mut gated = Vec::with_capacity(raw.len());
        for item in raw {
            match item.as_str().and_then(|name| FEATURES_SCHEMA_4.iter().find(|feature| **feature == name)) {
                Some(&name) => gated.push(name),
                None => return value_error(invalid),
            }
        }
        if !gated.is_empty() && schema != ORTHO_PLAUSIBILITY_ARTIFACT_SCHEMA_VERSION {
            return value_error(invalid);
        }
        gated.sort_unstable();
        Ok(gated)
    }

    /// Centres of a plausibility (`feature_centres` of the source channel from schema 5, `target_centres` of the
    /// target channel from schema 6): required from that schema on, absent before.
    fn _centres(fields: &Map<String, Value>, schema: i64, scale: f64, field: &str, since: i64) -> Result<Option<PerScript<f64>>, LoadError> {
        let raw = fields.get(field).filter(|value| !value.is_null());
        if (schema >= since) != raw.is_some() {
            return Err(LoadError::value(format!("an orthotactic artifact of schema {since} or later carries {field}, and no other")));
        }
        let Some(raw) = raw else {
            return Ok(None);
        };
        let table = raw.as_object().filter(|table| has_exact_keys(table, &SCRIPTS));
        let values = table.and_then(|table| Some((strict_int(&table["en"])?, strict_int(&table["ru"])?)));
        match values {
            Some((en, ru)) => Ok(Some(PerScript { en: en.to::<f64>() / scale, ru: ru.to::<f64>() / scale })),
            None => value_error("invalid orthotactic feature centres"),
        }
    }

    fn _shape(raw: Option<&Value>, scale: f64) -> Result<PerScript<[f64; 4]>, LoadError> {
        let Some(raw) = raw.and_then(Value::as_object).filter(|raw| has_exact_keys(raw, &SCRIPTS)) else {
            return value_error("orthotactic artifact must carry a shape channel");
        };
        let table = |script: &str| -> Result<[f64; 4], LoadError> {
            let Some(table) = raw[script].as_object().filter(|table| has_exact_keys(table, &SHAPES)) else {
                return value_error("invalid shape channel");
            };
            let mut values = [0.0; 4];
            for (slot, shape) in values.iter_mut().zip(SHAPES) {
                match int_like(&table[shape]) {
                    Some(value) => *slot = value.to::<f64>() / scale,
                    None => return value_error("invalid shape channel"),
                }
            }
            Ok(values)
        };
        Ok(PerScript { en: table("en")?, ru: table("ru")? })
    }

    fn identifier(&self) -> &Identifier {
        self.identifier.get_or_init(|| match &self.resources {
            Some(resources) => resources.packaged_identifier(),
            None => Arc::new(|_token| false),
        })
    }

    fn known(&self) -> &KnownWord {
        self.known.get_or_init(|| match &self.resources {
            Some(resources) => resources.engine_dictionaries(),
            None => Arc::new(|_script, _word| false),
        })
    }

    /// The token features, from the packaged identifier list and the dictionaries passed in. Without `known` the model
    /// uses the dictionaries it was loaded with, or loads the engine's.
    pub fn feature_values(&self, keys: &str, source_script: &str, known: Option<KnownRef<'_>>, shape: &str) -> IndexMap<&'static str, f64> {
        let identifier = self.identifier();
        let features = match known {
            Some(known) => token_features(keys, source_script, &**identifier, known, shape, self.hyphens, &self.compound_min_letters),
            None => token_features(keys, source_script, &**identifier, &**self.known(), shape, self.hyphens, &self.compound_min_letters),
        };
        features.as_pairs().into_iter().map(|(name, value)| (name, value.to::<f64>())).collect()
    }

    /// Evidence, in nats, that the keys were meant for the other layout.
    ///
    /// `known` answers the dictionary questions of the features (the context policy passes the detector's models); a
    /// schema 1 artifact asks none.
    pub fn score(&self, evidence: &OrthoEvidence<'_>, known: Option<KnownRef<'_>>) -> OrthoScore {
        let source = evidence.source_script;
        let target = if source == "en" { "ru" } else { "en" };
        let shape = if SHAPES.contains(&evidence.shape) { evidence.shape } else { "lower" };
        if !SCRIPTS.contains(&source) || evidence.keys.is_empty() || self.unscored_shapes.iter().any(|unscored| unscored == shape) {
            return OrthoScore {
                ratio: 0.0,
                shape_ratio: 0.0,
                total: 0.0,
                source_logprob: 0.0,
                target_logprob: 0.0,
                supported: false,
                features: 0.0,
                reading: String::new(),
            };
        }
        let keys = pystr::casefold(&normalize::nfc(evidence.keys));
        let source_channel = self.source_channels.get(source).as_ref().unwrap_or_else(|| self.channels.get(source));
        let target_channel = self.channels.get(target);
        // An abbreviation is written in capitals. A lowercase token therefore cannot be excused as one, which is what
        // removes the dominant class of false conversions: РСФСР, ЛДПР, РНК all read as fluent English keys.
        let slot = SHAPES.iter().position(|name| *name == shape).expect("a shape of SHAPES");
        let source_acronym = self.acronym_shape.get(source)[slot] - self.prose_shape.get(source)[slot];
        let shape_ratio = -source_acronym;
        // Up to schema 3 the one reading is the token as typed, a stretch cut as `collapse` cuts it, and the features
        // read the keys as typed. A schema 4 artifact reads a stretch both ways and judges each reading whole -
        // characters and features - keeping the one that argues less for the other layout: when the spelling is
        // ambiguous, the reading that protects the text wins.
        let judged = stretch_readings(&keys, self.collapse, self.stretch, self.hyphens).into_iter().map(|letters| {
            let featured = if self.stretch.is_none() { keys.as_str() } else { letters.as_str() };
            self.judged(featured, source_channel.score(&letters), target_channel.score(&letters), shape_ratio, source, known, shape)
        });
        min_by_key_first(judged, |scored| scored.total).expect("a token has at least one reading")
    }

    /// One reading scored: its character ratio, the case shape and the features of `featured`.
    #[expect(clippy::too_many_arguments)]
    fn judged(&self, featured: &str, source_logprob: f64, target_logprob: f64, shape_ratio: f64, source: &str, known: Option<KnownRef<'_>>, shape: &str) -> OrthoScore {
        let ratio = target_logprob - source_logprob;
        let mut features = 0.0;
        if let Some(weights) = &self.features {
            let mut values = self.feature_values(featured, source, known, shape);
            let positions = (pystr::len(featured) + 1).to::<f64>();
            if let Some(centres) = &self.centres {
                let plausibility = source_logprob / positions - centres.get(source);
                values.insert(CONTINUOUS_FEATURE, plausibility);
                // A gated feature is evidence only for a reading that is noise in the layout typed.
                for &name in &self.gated {
                    if plausibility >= 0.0 {
                        values.insert(name, 0.0);
                    }
                }
            }
            if let Some(centres) = &self.target_centres {
                values.insert(TARGET_FEATURE, target_logprob / positions - centres.get(source));
            }
            features = py_sum(weights.get(source).iter().map(|&(name, weight)| weight * values[name]));
        }
        OrthoScore {
            ratio,
            shape_ratio,
            total: ratio + shape_ratio + features,
            source_logprob,
            target_logprob,
            supported: true,
            features,
            reading: featured.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_helpers() {
        assert!(reduplicated("у-у") && reduplicated("мда-а") && reduplicated("ну-ну"));
        assert!(!reduplicated("а-б") && !reduplicated("абв") && !reduplicated("а-") && !reduplicated("а-а-а"));
        assert_eq!(collapse_runs("каеффф", 2), "каефф");
        assert_eq!(collapse_runs("aab", 0), "");
        assert_eq!(read_once("муууу", 2), "му");
        assert_eq!(read_once("ууaab", 3), "ууaab");
        assert_eq!(drop_hyphen_stretches("ти-ише"), "тише");
        assert_eq!(drop_hyphen_stretches("ура-а-а"), "ура");
        assert_eq!(drop_hyphen_stretches("a--a"), "a--a");
        assert_eq!(stretch_readings("штттук", Some(2), Some(3), false), ["шттук", "штук"]);
        assert_eq!(stretch_readings("штук", Some(2), Some(3), false), ["штук"]);
        assert_eq!(stretch_readings("aaa", None, None, false), ["aaa"]);
        assert_eq!(shape_of("ABC", true), "upper");
        assert_eq!(shape_of("Abc", true), "initial");
        assert_eq!(shape_of("Abc", false), "inner");
        assert_eq!(shape_of("a", true), "lower");
        assert_eq!(shape_of("A1", false), "inner");
        assert_eq!(shape_of("", false), "lower");
    }

    #[test]
    fn readings_translate_the_keys() {
        assert_eq!(readings("ghbdtn", "en"), ("ghbdtn".to_string(), "привет".to_string()));
        assert_eq!(readings("ghbdtn", "ru"), ("привет".to_string(), "ghbdtn".to_string()));
    }

    #[test]
    fn features_of_a_token() {
        let known = |script: &str, word: &str| (script == "ru" && word == "привет") || (script == "en" && word == "dist");
        let identifier = |token: &str| token == "ghbdtn";
        let none = PerScript::default();
        let features = token_features("ghbdtn", "en", &identifier, &known, "lower", false, &none);
        assert_eq!((features.ident, features.target_unknown, features.source_word), (1, 0, 0));
        let features = token_features(".dist", "ru", &|_| false, &known, "initial", false, &none);
        assert_eq!(features.dot_word, 1);
        assert!(one_key_extra("приввет", "ru", &known));
        assert!(!one_key_extra("привет", "ru", &known));
        assert!(!one_key_extra("прив", "ru", &known));
    }
}
