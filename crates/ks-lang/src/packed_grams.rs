//! The orthotactic model's gram tables packed into two arrays: the same answers in a small part of the memory.
//!
//! A gram is its characters translated to one byte each; the keys, each as wide as the longest gram with zeros in
//! front, sit sorted in one buffer and a lookup is a binary search. In the Python `pack_channels` packs a loaded
//! model's dicts; here the channels are built packed from the start, so `pack_channels` has nothing left to do. The
//! artifact stores integers and a scale, and a value is `raw / scale` computed when asked, the very float the dict
//! held, so a gram costs five bytes of key and two of value instead of a float and an object.

use ks_cast::To as _;
use crate::constants::{PACKED_GRAM_KEY_BYTES, PACKED_GRAM_LAST_CODE, PACKED_GRAM_MAX_CHARACTERS, PACKED_GRAM_MIN_ENTRIES};
use crate::ortho_model::OrthoModel;
use crate::tables::{TableBuilder, WordTable};

#[derive(Clone, Debug)]
enum GramValues {
    Small(Vec<i16>),
    Int(Vec<i32>),
    Float(Vec<f64>),
}

/// A read-only table of grams to floats, keyed by the grams' characters packed into integers.
#[derive(Clone, Debug)]
pub struct PackedGrams {
    /// Sorted; every character the table holds has a code from 1, its position plus one.
    alphabet: Vec<char>,
    width: usize,
    keys: Vec<u8>,
    values: GramValues,
    scale: f64,
}

/// The sorted keys of a table and, for each, the position of the entry that holds it (the last of equal grams).
struct SortedKeys {
    alphabet: Vec<char>,
    width: usize,
    keys: Vec<u8>,
    winners: Vec<u32>,
}

/// `None` where packing does not fit: too many characters, a zero character, an empty or too long gram.
fn sorted_keys<'a, I: Iterator<Item = &'a str> + Clone>(names: I, max_width: usize) -> Option<SortedKeys> {
    let mut seen = vec![0u64; 0x11_0000 / 64];
    let (mut count, mut width) = (0usize, 0usize);
    for name in names.clone() {
        let mut length = 0;
        for c in name.chars() {
            seen[c.to::<usize>() / 64] |= 1 << (c.to::<usize>() % 64);
            length += 1;
        }
        if length == 0 || length > max_width {
            return None;
        }
        width = width.max(length);
        count += 1;
    }
    let alphabet: Vec<char> = (0..0x11_0000u32).filter(|&cp| seen[cp.to::<usize>() / 64] >> (cp % 64) & 1 == 1).filter_map(char::from_u32).collect();
    drop(seen);
    if alphabet.len() > PACKED_GRAM_LAST_CODE || alphabet.first() == Some(&'\0') {
        return None;
    }
    let width = width.max(1);
    // A direct table for the build: a million characters are translated here.
    let mut codes = vec![0u8; alphabet.last().map_or(0, |&c| c.to::<usize>() + 1)];
    for (index, &c) in alphabet.iter().enumerate() {
        codes[c.to::<usize>()] = index.to::<u8>() + 1;
    }
    let mut buffer = vec![0u8; count * width];
    for (slot, name) in buffer.chunks_exact_mut(width).zip(names) {
        let length = name.chars().count();
        for (byte, c) in slot[width - length..].iter_mut().zip(name.chars()) {
            *byte = codes[c.to::<usize>()];
        }
    }
    drop(codes);
    let key = |index: u32| &buffer[index.to::<usize>() * width..(index.to::<usize>() + 1) * width];
    let mut order: Vec<u32> = (0..count.to::<u32>()).collect();
    order.sort_by(|&a, &b| key(a).cmp(key(b)));
    let mut keys = Vec::with_capacity(count * width);
    let mut winners = Vec::with_capacity(count);
    for (position, &index) in order.iter().enumerate() {
        if position + 1 == order.len() || key(order[position + 1]) != key(index) {
            keys.extend_from_slice(key(index));
            winners.push(index);
        }
    }
    keys.shrink_to_fit();
    winners.shrink_to_fit();
    Some(SortedKeys { alphabet, width, keys, winners })
}

impl PackedGrams {
    /// `PackedGrams.pack(table)`: the table packed, or `None` where packing does not fit (too few entries, too many
    /// characters, too long or an empty gram, a zero character). A key given twice keeps its last value.
    #[must_use]
    pub fn pack(table: &[(&str, f64)]) -> Option<PackedGrams> {
        let sorted = sorted_keys(table.iter().map(|entry| entry.0), PACKED_GRAM_MAX_CHARACTERS)?;
        if sorted.winners.len() < PACKED_GRAM_MIN_ENTRIES {
            return None;
        }
        let values = sorted.winners.iter().map(|&index| table[index.to::<usize>()].1).collect();
        Some(PackedGrams { alphabet: sorted.alphabet, width: sorted.width, keys: sorted.keys, values: GramValues::Float(values), scale: 1.0 })
    }

    fn get_index(&self, gram: &[char]) -> Option<usize> {
        if gram.is_empty() || gram.len() > self.width {
            return None;
        }
        let mut key = [0u8; PACKED_GRAM_KEY_BYTES];
        let start = self.width - gram.len();
        for (byte, c) in key[start..self.width].iter_mut().zip(gram) {
            *byte = (self.alphabet.binary_search(c).ok()?).to::<u8>() + 1;
        }
        let key = &key[..self.width];
        let (mut low, mut high) = (0, self.len());
        while low < high {
            let middle = low + (high - low) / 2;
            match self.keys[middle * self.width..(middle + 1) * self.width].cmp(key) {
                std::cmp::Ordering::Less => low = middle + 1,
                std::cmp::Ordering::Greater => high = middle,
                std::cmp::Ordering::Equal => return Some(middle),
            }
        }
        None
    }

    fn value(&self, index: usize) -> f64 {
        match &self.values {
            GramValues::Small(values) => f64::from(values[index]) / self.scale,
            GramValues::Int(values) => f64::from(values[index]) / self.scale,
            GramValues::Float(values) => values[index],
        }
    }

    #[must_use]
    pub fn get(&self, gram: &str) -> Option<f64> {
        self.get_chars(&gram.chars().collect::<Vec<char>>())
    }

    #[must_use]
    pub fn get_chars(&self, gram: &[char]) -> Option<f64> {
        self.get_index(gram).map(|index| self.value(index))
    }

    /// `len(table)`.
    #[must_use]
    pub fn len(&self) -> usize {
        self.keys.len() / self.width
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// The grams in the order of their keys (shorter ones first), with their values.
    pub fn iter(&self) -> impl Iterator<Item = (String, f64)> + '_ {
        self.keys.chunks_exact(self.width).enumerate().map(|(index, key)| {
            let gram = key.iter().filter(|&&code| code != 0).map(|&code| self.alphabet[code.to::<usize>() - 1]).collect();
            (gram, self.value(index))
        })
    }

    /// The bytes of the two arrays, for the record.
    #[must_use]
    pub fn heap_bytes(&self) -> usize {
        self.keys.len()
            + match &self.values {
                GramValues::Small(values) => values.len() * 2,
                GramValues::Int(values) => values.len() * 4,
                GramValues::Float(values) => values.len() * 8,
            }
    }
}

/// A gram table of a channel: packed, or, where packing does not fit, a sorted table of the integers.
#[derive(Clone, Debug)]
pub enum GramTable {
    Packed(PackedGrams),
    Plain { table: WordTable, scale: f64, longest: usize },
}

impl GramTable {
    /// The table of the names and the integers the artifact stores, `dict(zip(names, raw))`.
    pub(crate) fn from_raw<'a, I: Iterator<Item = &'a str> + Clone>(names: I, raw: &[i32]) -> GramTable {
        if let Some(sorted) = sorted_keys(names.clone(), PACKED_GRAM_KEY_BYTES) {
            let picked = sorted.winners.iter().map(|&index| raw[index.to::<usize>()]);
            let values = if raw.iter().all(|&value| i16::try_from(value).is_ok()) {
                GramValues::Small(picked.map(ks_cast::To::to::<i16>).collect())
            } else {
                GramValues::Int(picked.collect())
            };
            GramTable::Packed(PackedGrams { alphabet: sorted.alphabet, width: sorted.width, keys: sorted.keys, values, scale: 1.0 })
        } else {
            let mut builder = TableBuilder::new();
            let mut longest = 0;
            for (name, &value) in names.zip(raw) {
                longest = longest.max(name.chars().count());
                builder.push(name, i64::from(value));
            }
            GramTable::Plain { table: builder.finish(|_earlier, later| later), scale: 1.0, longest }
        }
    }

    pub(crate) fn set_scale(&mut self, value: f64) {
        match self {
            GramTable::Packed(packed) => packed.scale = value,
            GramTable::Plain { scale, .. } => *scale = value,
        }
    }

    /// The length of the longest gram.
    pub(crate) fn longest(&self) -> usize {
        match self {
            GramTable::Packed(packed) if packed.is_empty() => 0,
            GramTable::Packed(packed) => packed.width,
            GramTable::Plain { longest, .. } => *longest,
        }
    }

    #[must_use]
    pub fn get(&self, gram: &str) -> Option<f64> {
        match self {
            GramTable::Packed(packed) => packed.get(gram),
            GramTable::Plain { table, scale, .. } => table.get(gram).map(|value| value.to::<f64>() / *scale),
        }
    }

    #[must_use]
    pub fn get_chars(&self, gram: &[char]) -> Option<f64> {
        match self {
            GramTable::Packed(packed) => packed.get_chars(gram),
            GramTable::Plain { table, scale, .. } => table.get(&gram.iter().collect::<String>()).map(|value| value.to::<f64>() / *scale),
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            GramTable::Packed(packed) => packed.len(),
            GramTable::Plain { table, .. } => table.len(),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The packed table, where it is one.
    #[must_use]
    pub fn packed(&self) -> Option<&PackedGrams> {
        match self {
            GramTable::Packed(packed) => Some(packed),
            GramTable::Plain { .. } => None,
        }
    }
}

/// `pack_channels(model)`: the channels of a loaded model are packed already.
pub fn pack_channels(_model: Option<&mut OrthoModel>) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> Vec<(String, f64)> {
        let letters: Vec<char> = ('a'..='z').chain('а'..='я').chain("\x02\x03,.;[]'`-".chars()).collect();
        let mut table = Vec::new();
        for &a in &letters {
            for &b in &letters {
                table.push((format!("{a}{b}"), -table.len().to::<f64>() / 512.0));
            }
        }
        table
    }

    #[test]
    fn packs_what_fits() {
        let table = grid();
        let entries: Vec<(&str, f64)> = table.iter().map(|(gram, value)| (gram.as_str(), *value)).collect();
        let packed = PackedGrams::pack(&entries).unwrap();
        assert_eq!(packed.len(), entries.len());
        for (gram, value) in &entries {
            assert_eq!(packed.get(gram), Some(*value));
        }
        for gram in ["", "\0", "é", "aé", "abc", "\u{1d41a}"] {
            assert_eq!(packed.get(gram), None);
        }
        assert!(PackedGrams::pack(&entries[..PACKED_GRAM_MIN_ENTRIES - 1]).is_none());
        assert!(PackedGrams::pack(&entries[..PACKED_GRAM_MIN_ENTRIES]).is_some());
        for extra in ["", "a\0", "abcdefgh"] {
            let mut more = entries.clone();
            more.push((extra, -1.0));
            assert!(PackedGrams::pack(&more).is_none(), "{extra:?}");
        }
        let mut seven = entries.clone();
        seven.push(("abcdefg", -2.5));
        assert_eq!(PackedGrams::pack(&seven).unwrap().get("abcdefg"), Some(-2.5));
        assert!(PackedGrams::pack(&[]).is_none());
    }

    #[test]
    fn raw_tables_answer_the_same_either_way() {
        let names = ["a", "ab", "b", "a"];
        let mut packed = GramTable::from_raw(names.iter().copied(), &[1, 2, 3, 40_000]);
        packed.set_scale(512.0);
        assert!(packed.packed().is_some());
        assert_eq!(packed.get("a"), Some(40_000.0 / 512.0));
        assert_eq!(packed.get("ab"), Some(2.0 / 512.0));
        assert_eq!(packed.get("ba"), None);
        assert_eq!(packed.len(), 3);
        assert_eq!(packed.longest(), 2);
        let names = ["a", "", "b"];
        let mut plain = GramTable::from_raw(names.iter().copied(), &[1, 2, 3]);
        plain.set_scale(2.0);
        assert!(plain.packed().is_none());
        assert_eq!(plain.get(""), Some(1.0));
        assert_eq!(plain.get_chars(&['b']), Some(1.5));
        assert_eq!(GramTable::from_raw(std::iter::empty(), &[]).len(), 0);
    }
}
