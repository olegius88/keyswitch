//! Keyboard-layout transformations used by the detector and tests (port of `layouts.py`).

use crate::ucd::isalpha;
use std::fmt;
use std::sync::OnceLock;

pub const US_KEYS: &str = "`qwertyuiop[]\\asdfghjkl;'zxcvbnm,.";
pub const RU_KEYS: &str = "ёйцукенгшщзхъ\\фывапролджэячсмитьбю";

/// Python's `ValueError` of this module, with its message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutError(pub String);

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LayoutError {}

/// The one-character `str.upper()` of a key; the key rows hold no character whose capital is longer.
fn upper_char(c: char) -> char {
    let mut it = crate::pystr::upper(&c.to_string()).chars().collect::<Vec<_>>().into_iter();
    match (it.next(), it.next()) {
        (Some(u), None) => u,
        _ => panic!("str.translate tables need one-character keys"),
    }
}

/// A dict in insertion order: re-assigning a key keeps its place.
fn set(map: &mut Vec<(char, char)>, key: char, value: char) {
    match map.iter_mut().find(|(k, _)| *k == key) {
        Some(entry) => entry.1 = value,
        None => map.push((key, value)),
    }
}

fn case_aware_map(source: &str, target: &str) -> Vec<(char, char)> {
    let (source, target): (Vec<char>, Vec<char>) = (source.chars().collect(), target.chars().collect());
    assert_eq!(source.len(), target.len(), "zip(strict=True)");
    let mut mapping = Vec::new();
    for (&left, &right) in source.iter().zip(&target) {
        set(&mut mapping, left, right);
    }
    for (&left, &right) in source.iter().zip(&target) {
        if isalpha(left) && isalpha(right) {
            set(&mut mapping, upper_char(left), upper_char(right));
        }
    }
    mapping
}

struct Maps {
    us_to_ru: Vec<(char, char)>,
    ru_to_us: Vec<(char, char)>,
    us_to_ru_sorted: Vec<(char, char)>,
    ru_to_us_sorted: Vec<(char, char)>,
}

// The mappings are pure functions of the two key rows, so they are built once.
fn maps() -> &'static Maps {
    static MAPS: OnceLock<Maps> = OnceLock::new();
    MAPS.get_or_init(|| {
        let us_to_ru = case_aware_map(US_KEYS, RU_KEYS);
        let ru_to_us = case_aware_map(RU_KEYS, US_KEYS);
        let sorted = |m: &Vec<(char, char)>| {
            let mut s = m.clone();
            s.sort();
            s
        };
        Maps { us_to_ru_sorted: sorted(&us_to_ru), ru_to_us_sorted: sorted(&ru_to_us), us_to_ru, ru_to_us }
    })
}

fn translate_with(text: &str, table: &[(char, char)]) -> String {
    text.chars()
        .map(|c| match table.binary_search_by_key(&c, |e| e.0) {
            Ok(i) => table[i].1,
            Err(_) => c,
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutPair {
    pub first: String,
    pub second: String,
}

impl Default for LayoutPair {
    fn default() -> Self {
        LayoutPair { first: "us".into(), second: "ru".into() }
    }
}

impl LayoutPair {
    /// `LayoutPair(first, second)`: only the us/ru pair, in either order.
    pub fn new(first: &str, second: &str) -> Result<Self, LayoutError> {
        if !((first == "us" && second == "ru") || (first == "ru" && second == "us")) {
            return Err(LayoutError("The static mapper currently supports the us/ru pair".into()));
        }
        Ok(LayoutPair { first: first.into(), second: second.into() })
    }

    /// `us_to_ru`, in the dict's insertion order.
    pub fn us_to_ru(&self) -> Vec<(char, char)> {
        maps().us_to_ru.clone()
    }

    /// `ru_to_us`, in the dict's insertion order.
    pub fn ru_to_us(&self) -> Vec<(char, char)> {
        maps().ru_to_us.clone()
    }

    /// `translate(text, source, target)`: replaces exactly the characters the mapping contains.
    pub fn translate(&self, text: &str, source: &str, target: &str) -> Result<String, LayoutError> {
        if source == target {
            return Ok(text.to_string());
        }
        match (source, target) {
            ("us", "ru") => Ok(translate_with(text, &maps().us_to_ru_sorted)),
            ("ru", "us") => Ok(translate_with(text, &maps().ru_to_us_sorted)),
            _ => Err(LayoutError(format!("Unsupported layout conversion: {source} -> {target}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_both_ways_with_case() {
        let pair = LayoutPair::default();
        assert_eq!(pair.translate("ghbdtn", "us", "ru").unwrap(), "привет");
        assert_eq!(pair.translate("GHBDTN, `", "us", "ru").unwrap(), "ПРИВЕТб ё");
        assert_eq!(pair.translate("Привет ёЁ", "ru", "us").unwrap(), "Ghbdtn `Ё");
        assert_eq!(pair.translate("abc", "us", "us").unwrap(), "abc");
        assert_eq!(pair.translate("a", "us", "de").unwrap_err().0, "Unsupported layout conversion: us -> de");
        assert!(LayoutPair::new("us", "de").is_err());
        assert!(LayoutPair::new("ru", "us").is_ok());
    }

    #[test]
    fn mapping_order_and_size() {
        let pair = LayoutPair::default();
        let m = pair.us_to_ru();
        assert_eq!(m[0], ('`', 'ё'));
        assert_eq!(m.len(), 60); // 34 keys, plus a capital for each of the 26 letter pairs
        assert_eq!(m[34], ('Q', 'Й'));
    }
}
