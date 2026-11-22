//! The orthotactic model's gram tables packed into two arrays: the same answers in a small part of the memory.
//!
//! A gram is its characters translated to one byte each; the keys, each as wide as the longest gram with zeros in
//! front, sit sorted in one buffer and a lookup is a binary search. In the Python `pack_channels` packs a loaded
//! model's dicts; here the channels are built packed from the start, so `pack_channels` has nothing left to do. The
//! artifact stores integers and a scale, and a value is `raw / scale` computed when asked, the very float the dict
//! held, so a gram costs five bytes of key and two of value instead of a float and an object.
//!
//! The first codes of a key are not stored either: they number the bucket the key lies in, and the start of each
//! bucket among the sorted keys is kept instead. A key of five codes costs three bytes so, and the binary search runs
//! inside its bucket.

use ks_cast::To as _;
use crate::constants::{PACKED_GRAM_KEY_BYTES, PACKED_GRAM_LAST_CODE, PACKED_GRAM_MAX_CHARACTERS, PACKED_GRAM_MIN_ENTRIES};
use crate::tables::{TableBuilder, WordTable};

/// How many leading codes of a key at most number its bucket instead of being stored. Declared here and not in
/// `constants.rs`: that file is pinned by the seals of the intent, boundary and orthotactic models.
const BUCKET_CODES: usize = 2;

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
    /// How many leading codes of a key number its bucket.
    lead: usize,
    /// `starts[bucket]..starts[bucket + 1]` are the entries of a bucket; a bucket is the number its leading codes
    /// write in base `alphabet.len() + 1`.
    starts: Vec<u32>,
    /// The codes after the leading ones, `width - lead` of them per entry, in the order of the full keys.
    keys: Vec<u8>,
    values: GramValues,
    scale: f64,
}

/// Sorted full keys of `width` codes split into the buckets of their leading codes and the codes after them. As many
/// leading codes number the bucket as there are entries for buckets of that size, up to `BUCKET_CODES`.
fn bucketed(alphabet_len: usize, width: usize, full: &[u8]) -> (usize, Vec<u32>, Vec<u8>) {
    let count = full.len() / width;
    let base = alphabet_len + 1;
    let mut lead = 0;
    while lead < BUCKET_CODES.min(width) && base.pow((lead + 1).to::<u32>()) <= count {
        lead += 1;
    }
    let buckets = base.pow(lead.to::<u32>());
    let mut starts = vec![0_u32; buckets + 1];
    let mut keys = Vec::with_capacity(count * (width - lead));
    for key in full.chunks_exact(width) {
        let bucket = key[..lead].iter().fold(0, |number, &code| number * base + usize::from(code));
        starts[bucket + 1] += 1;
        keys.extend_from_slice(&key[lead..]);
    }
    for bucket in 0..buckets {
        starts[bucket + 1] += starts[bucket];
    }
    (lead, starts, keys)
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
    // A key is at most eight bytes: as a big-endian integer it sorts as its bytes do. Sorted together with its place,
    // equal keys keep the order they came in, as the stable sort of the places by their bytes kept them, at a fraction
    // of the cost: a million grams are sorted as the program starts. Where the key leaves room, its place shares its
    // integer, so the sort holds no more than eight bytes a gram.
    let number = |index: u32| {
        let mut bytes = [0u8; PACKED_GRAM_KEY_BYTES];
        bytes[PACKED_GRAM_KEY_BYTES - width..].copy_from_slice(key(index));
        u64::from_be_bytes(bytes)
    };
    let mut keys = Vec::with_capacity(count * width);
    let mut winners = Vec::with_capacity(count);
    let place_bits = u64::BITS - u8::BITS * width.to::<u32>();
    if place_bits > 0 && count.to::<u64>() >> place_bits == 0 {
        let place_mask = (1_u64 << place_bits) - 1;
        let mut order: Vec<u64> = (0..count.to::<u32>()).map(|index| (number(index) << place_bits) | u64::from(index)).collect();
        order.sort_unstable();
        for (position, &entry) in order.iter().enumerate() {
            if position + 1 == order.len() || order[position + 1] >> place_bits != entry >> place_bits {
                let index = (entry & place_mask).to::<u32>();
                keys.extend_from_slice(key(index));
                winners.push(index);
            }
        }
    } else {
        let mut order: Vec<(u64, u32)> = (0..count.to::<u32>()).map(|index| (number(index), index)).collect();
        order.sort_unstable();
        for (position, &(number, index)) in order.iter().enumerate() {
            if position + 1 == order.len() || order[position + 1].0 != number {
                keys.extend_from_slice(key(index));
                winners.push(index);
            }
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
        Some(PackedGrams::new(sorted, GramValues::Float(values)))
    }

    /// The table of sorted keys and the values of their entries, its keys split into buckets.
    fn new(sorted: SortedKeys, values: GramValues) -> PackedGrams {
        let (lead, starts, mut keys) = bucketed(sorted.alphabet.len(), sorted.width, &sorted.keys);
        drop(sorted.keys);
        keys.shrink_to_fit();
        PackedGrams { alphabet: sorted.alphabet, width: sorted.width, lead, starts, keys, values, scale: 1.0 }
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
        let base = self.alphabet.len() + 1;
        let bucket = key[..self.lead].iter().fold(0, |number, &code| number * base + usize::from(code));
        let rest = &key[self.lead..self.width];
        let size = self.width - self.lead;
        let (mut low, mut high) = (self.starts[bucket].to::<usize>(), self.starts[bucket + 1].to::<usize>());
        while low < high {
            let middle = low + (high - low) / 2;
            match self.keys[middle * size..(middle + 1) * size].cmp(rest) {
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
        self.starts.last().map_or(0, |&count| count.to::<usize>())
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The grams in the order of their keys (shorter ones first), with their values.
    pub fn iter(&self) -> impl Iterator<Item = (String, f64)> + '_ {
        let (base, size) = (self.alphabet.len() + 1, self.width - self.lead);
        (0..self.starts.len() - 1).flat_map(move |bucket| {
            let mut leading = vec![0_u8; self.lead];
            let mut number = bucket;
            for code in leading.iter_mut().rev() {
                *code = (number % base).to::<u8>();
                number /= base;
            }
            (self.starts[bucket].to::<usize>()..self.starts[bucket + 1].to::<usize>()).map(move |index| {
                let rest = &self.keys[index * size..(index + 1) * size];
                let gram = leading.iter().chain(rest).filter(|&&code| code != 0).map(|&code| self.alphabet[code.to::<usize>() - 1]).collect();
                (gram, self.value(index))
            })
        })
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
            GramTable::Packed(PackedGrams::new(sorted, values))
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

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn a_table_counts_its_grams() {
        let plain = GramTable::Plain { table: WordTable::default(), scale: 1.0, longest: 0 };
        assert_eq!(plain.len(), 0);
    }

    #[test]
    fn a_plain_table_and_an_empty_packed_one() {
        let packed = GramTable::from_raw(std::iter::empty::<&str>(), &[]);
        assert_eq!((packed.longest(), packed.len()), (0, 0));
        assert!(packed.is_empty());
    }

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
    fn keys_split_into_buckets_answer_as_whole_keys() {
        // Eight letters: grams of one to three letters, 584 of them, more than the 81 buckets of two leading codes.
        let letters: Vec<char> = ('a'..='h').collect();
        let mut grams: Vec<String> = letters.iter().map(char::to_string).collect();
        for &a in &letters {
            for &b in &letters {
                grams.push(format!("{a}{b}"));
                for &c in &letters {
                    grams.push(format!("{a}{b}{c}"));
                }
            }
        }
        let raw: Vec<i32> = (0..grams.len().to::<i32>()).map(|value| value * 3 - 700).collect();
        let table = GramTable::from_raw(grams.iter().map(String::as_str), &raw);
        let packed = table.packed().unwrap();
        assert_eq!((packed.lead, packed.len()), (2, grams.len()));
        for (gram, &value) in grams.iter().zip(&raw) {
            assert_eq!(table.get(gram), Some(f64::from(value)), "{gram}");
        }
        for gram in ["i", "ai", "abi", "abcd", "ia", "\u{1d41a}"] {
            assert_eq!(table.get(gram), None, "{gram}");
        }
        // The grams come back in the order of their keys: shorter ones first, then by letters.
        let mut wanted: Vec<(String, f64)> = grams.iter().cloned().zip(raw.iter().map(|&value| f64::from(value))).collect();
        wanted.sort_by(|a, b| a.0.chars().count().cmp(&b.0.chars().count()).then_with(|| a.0.cmp(&b.0)));
        assert_eq!(packed.iter().collect::<Vec<_>>(), wanted);
        // Fewer grams than buckets of one leading code: one bucket holds the whole keys.
        let single = GramTable::from_raw(["b", "a", "c"].into_iter(), &[1, 2, 3]);
        let packed = single.packed().unwrap();
        assert_eq!((packed.lead, packed.width, packed.len()), (0, 1, 3));
        assert_eq!((single.get("a"), single.get("c"), single.get("d")), (Some(2.0), Some(3.0), None));
        assert_eq!(packed.iter().map(|entry| entry.0).collect::<Vec<_>>(), ["a", "b", "c"]);
    }

    #[test]
    fn keys_of_eight_letters_keep_the_last_value_of_a_gram() {
        // Eight letters leave no room for the place beside the key: the other way of sorting, with a gram given twice.
        let names = ["abcdefgh", "b", "abcdefgh", "a"];
        let packed = GramTable::from_raw(names.iter().copied(), &[1, 2, 3, 4]);
        assert!(packed.packed().is_some());
        assert_eq!((packed.get("abcdefgh"), packed.get("a"), packed.len()), (Some(3.0), Some(4.0), 3));
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
