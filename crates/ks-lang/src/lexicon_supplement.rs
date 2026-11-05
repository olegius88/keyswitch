//! Packaged lexicon supplements: extra known forms outside the certified language-model toolchain.
//!
//! The frozen `language_model` and the onboard `.lm` files are pinned by the Layout Intent certification and must not
//! change. A supplement adds forms the onboard vocabulary lacks (colloquial words, interjections, transliterated
//! names) through the existing `extra_words` hook of `LanguageModel::load`. The same file feeds the engine, the
//! early-switch models, the trainers and the evaluators, so lexical evidence never differs between corpus
//! construction and serving.
//!
//! The file is 3.8 MB of 196 000 words; a visitor writes them straight into one arena.

use crate::constants::LEXICON_SUPPLEMENT_MAX_BYTES;
use crate::load_error::{json_text, py_equals_int, read_all, LoadError};
use crate::tables::WordList;
use serde::de::{self, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::Value;
use std::fmt;
use std::path::Path;

/// `SUPPLEMENT_PATTERNS` by hand: `ru_RU` `[а-яё]{3,}`, `en_US` `[a-z][a-z'-]*`.
fn matches_pattern(locale: &str, word: &str) -> Option<bool> {
    match locale {
        "ru_RU" => Some(word.chars().count() >= 3 && word.chars().all(|c| ('а'..='я').contains(&c) || c == 'ё')),
        "en_US" => {
            let mut chars = word.chars();
            Some(chars.next().is_some_and(|c| c.is_ascii_lowercase()) && chars.all(|c| c.is_ascii_lowercase() || c == '\'' || c == '-'))
        }
        _ => None,
    }
}

/// Two-letter Cyrillic forms are refused: the onboard vocabulary already holds every real two-letter Russian word, so
/// a frequency list adds only what its count floor misses at that length - doubled letters, transliteration fragments
/// and subtitle noise. Each one disguises a short Latin command as a Russian word, which is how `pm2` came to be
/// rewritten as `зь2`. Russian has exactly eight single-letter words, and "я" is the most frequent token of the
/// frequency list this supplement is built from; the {2,} shape dropped all of them, so the engine could only learn
/// them from a spell checker, which calls every lone Latin letter a word as well and therefore never told `z` from
/// `я` apart. Any other single letter is a letter, not a word, and is still refused.
fn single_letter_allowed(locale: &str, word: &str) -> bool {
    let letters = match locale {
        "ru_RU" => "аисвкуоя",
        "en_US" => "ai",
        _ => return false,
    };
    let mut chars = word.chars();
    matches!((chars.next(), chars.next()), (Some(c), None) if letters.contains(c))
}

/// The fields of the file that the checks read; `words` straight in an arena, with what the list held otherwise.
#[derive(Default)]
struct Supplement {
    schema_version: Option<Value>,
    locale: Option<Value>,
    words: Option<RawWords>,
}

struct RawWords {
    list: WordList,
    only_strings: bool,
}

impl<'de> de::Deserialize<'de> for RawWords {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct WordsVisitor;
        impl<'de> Visitor<'de> for WordsVisitor {
            type Value = RawWords;
            #[cfg_attr(coverage_nightly, coverage(off))] // The wording of a type error, which the program replaces with its own.
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a list of words")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<RawWords, A::Error> {
                let mut list = WordList::with_capacity(seq.size_hint().unwrap_or(0), 0);
                let mut only_strings = true;
                while let Some(element) = seq.next_element::<Element<'_>>()? {
                    match element {
                        Element::Text(text) => list.push(&text),
                        Element::Other => only_strings = false,
                    }
                }
                Ok(RawWords { list: list.shrunk(), only_strings })
            }
        }
        deserializer.deserialize_any(WordsVisitor)
    }
}

/// One element of the list: a string, or anything else, which the entry check refuses.
enum Element<'a> {
    Text(std::borrow::Cow<'a, str>),
    Other,
}

impl<'de> de::Deserialize<'de> for Element<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ElementVisitor;
        impl<'de> Visitor<'de> for ElementVisitor {
            type Value = Element<'de>;
            #[cfg_attr(coverage_nightly, coverage(off))] // The wording of a type error, which the program replaces with its own.
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a list element")
            }
            fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Element<'de>, E> {
                Ok(Element::Text(value.into()))
            }
            fn visit_str<E>(self, value: &str) -> Result<Element<'de>, E> {
                Ok(Element::Text(value.to_string().into()))
            }
            fn visit_bool<E>(self, _: bool) -> Result<Element<'de>, E> {
                Ok(Element::Other)
            }
            fn visit_i64<E>(self, _: i64) -> Result<Element<'de>, E> {
                Ok(Element::Other)
            }
            fn visit_u64<E>(self, _: u64) -> Result<Element<'de>, E> {
                Ok(Element::Other)
            }
            fn visit_f64<E>(self, _: f64) -> Result<Element<'de>, E> {
                Ok(Element::Other)
            }
            fn visit_unit<E>(self) -> Result<Element<'de>, E> {
                Ok(Element::Other)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Element<'de>, A::Error> {
                IgnoredAny.visit_seq(seq)?;
                Ok(Element::Other)
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Element<'de>, A::Error> {
                IgnoredAny.visit_map(map)?;
                Ok(Element::Other)
            }
        }
        deserializer.deserialize_any(ElementVisitor)
    }
}

impl<'de> de::Deserialize<'de> for Supplement {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct SupplementVisitor;
        impl<'de> Visitor<'de> for SupplementVisitor {
            type Value = Supplement;
            #[cfg_attr(coverage_nightly, coverage(off))] // The wording of a type error, which the program replaces with its own.
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("invalid lexicon supplement")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Supplement, A::Error> {
                let mut result = Supplement::default();
                while let Some(key) = map.next_key::<std::borrow::Cow<'de, str>>()? {
                    match key.as_ref() {
                        "schema_version" => result.schema_version = Some(map.next_value()?),
                        "locale" => result.locale = Some(map.next_value()?),
                        "words" => result.words = Some(map.next_value()?),
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(result)
            }
        }
        deserializer.deserialize_any(SupplementVisitor)
    }
}

/// `supplement_words(locale)`: sorted extra forms for a locale; empty when no supplement is packaged.
/// `resource_dir` is `SUPPLEMENT_ROOT`, the `resources` folder of the install.
///
/// A malformed supplement is an error, never a silent gap.
pub fn supplement_words(resource_dir: &Path, locale: &str) -> Result<WordList, LoadError> {
    let path = resource_dir.join(format!("lexicon-supplement-{locale}.json"));
    if !path.is_file() {
        return Ok(WordList::new());
    }
    let raw = read_all(&path)?;
    if raw.len() > LEXICON_SUPPLEMENT_MAX_BYTES {
        return Err(LoadError::value("lexicon supplement is too large"));
    }
    let name = path.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let invalid = || LoadError::value(format!("invalid lexicon supplement: {name}"));
    let mut deserializer = serde_json::Deserializer::from_slice(json_text(&raw)?);
    let payload = match de::Deserialize::deserialize(&mut deserializer).and_then(|payload: Supplement| deserializer.end().map(|()| payload)) {
        Ok(payload) => payload,
        Err(error) => return Err(LoadError::json(&error)),
    };
    drop(raw);
    let (Some(schema_version), Some(Value::String(found_locale)), Some(words)) = (&payload.schema_version, &payload.locale, payload.words) else {
        return Err(invalid());
    };
    if !py_equals_int(schema_version, 1) || found_locale != locale {
        return Err(invalid());
    }
    let sorted_set = "lexicon supplement must be a sorted set for a supported locale";
    if matches_pattern(locale, "").is_none() || words.list.is_empty() || !words.only_strings || !words.list.is_sorted_set() {
        // Elements other than strings fail one of the two checks in the Python: both raise ValueError.
        return Err(LoadError::value(sorted_set));
    }
    for word in words.list.iter() {
        if matches_pattern(locale, word) != Some(true) && !single_letter_allowed(locale, word) {
            return Err(LoadError::value("invalid lexicon supplement entry"));
        }
    }
    Ok(words.list)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn patterns_and_letters() {
        assert_eq!(matches_pattern("ru_RU", "ёжа"), Some(true));
        assert_eq!(matches_pattern("ru_RU", "ап"), Some(false));
        assert_eq!(matches_pattern("ru_RU", "абв\n"), Some(false));
        assert_eq!(matches_pattern("en_US", "it's"), Some(true));
        assert_eq!(matches_pattern("en_US", "-a"), Some(false));
        assert_eq!(matches_pattern("en_US", ""), Some(false));
        assert_eq!(matches_pattern("de_DE", "a"), None);
        assert!(single_letter_allowed("ru_RU", "я") && !single_letter_allowed("ru_RU", "б") && !single_letter_allowed("ru_RU", "яя"));
        assert!(single_letter_allowed("en_US", "a") && !single_letter_allowed("en_US", "z") && !single_letter_allowed("de_DE", "a"));
    }

    #[test]
    fn no_file_is_no_words() {
        let words = supplement_words(Path::new("/nonexistent-ks-lang"), "ru_RU").unwrap();
        assert!(words.is_empty());
    }
}
