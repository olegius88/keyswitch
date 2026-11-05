//! The two big tables of the context layer in flat form: names in one UTF-8 arena with offsets, an index sorted by a
//! 32-bit hash of the name (the name itself is compared on a hit), and the values in arrays beside it.
//!
//! `context_policy_v1.json` has 119 559 feature names with four weights each and `context-term-frequency.json` has
//! 195 641 words; as maps of `String`s they cost several times their text. Lookups answer what the Python dict answered.
//! The files are read through serde visitors, so no tree of `serde_json::Value` is built.

use ks_cast::To as _;
use ks_lang::LoadError;
use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer as _};
use sha2::{Digest, Sha256};
use std::fmt;
use std::io::BufReader;
use std::path::Path;

/// FNV-1a over the bytes of a name.
fn name_hash(name: &str) -> u32 {
    let mut hash = 0x811c_9dc5_u32;
    for &byte in name.as_bytes() {
        hash = (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193);
    }
    hash
}

/// Names in one arena in the order of the file, and an index of them sorted by hash. The values of a table lie in
/// arrays in the order of the file, beside the arena: nothing is copied into a second order, which would double the
/// memory the parse needs.
#[derive(Default)]
pub struct NameIndex {
    arena: String,
    /// `ends[i]` is the end of the `i`-th name of the file; it starts where the one before ends.
    ends: Vec<u32>,
    /// The hashes of the names in the index, sorted.
    hashes: Vec<u32>,
    /// For each hash its entry in the file. A name given twice is indexed once, at its last place.
    order: Vec<u32>,
}

impl NameIndex {
    /// How many names the table holds (`len(dict)`).
    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// The name of an entry of the file.
    pub fn name(&self, entry: usize) -> &str {
        let start = if entry == 0 { 0 } else { (self.ends[entry - 1]).to::<usize>() };
        &self.arena[start..self.ends[entry].to::<usize>()]
    }

    /// The entry of a name, if the table holds it.
    pub fn find(&self, name: &str) -> Option<usize> {
        let hash = name_hash(name);
        let mut at = self.hashes.partition_point(|&found| found < hash);
        while at < self.hashes.len() && self.hashes[at] == hash {
            let entry = self.order[at].to::<usize>();
            if self.name(entry) == name {
                return Some(entry);
            }
            at += 1;
        }
        None
    }

    /// The names the table holds, in no particular order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.order.iter().map(|&entry| self.name(entry.to::<usize>()))
    }

    /// Bytes of heap the index holds.
    pub fn heap_bytes(&self) -> usize {
        self.arena.capacity() + 4 * (self.ends.capacity() + self.hashes.capacity() + self.order.capacity())
    }
}

/// Names in the order of the file, as they are read.
#[derive(Default)]
struct Names {
    arena: String,
    ends: Vec<u32>,
}

impl Names {
    fn push(&mut self, name: &str) -> Result<(), String> {
        self.arena.push_str(name);
        let end = u32::try_from(self.arena.len()).map_err(|_| "the table is too large".to_string())?;
        self.ends.push(end);
        Ok(())
    }

    fn len(&self) -> usize {
        self.ends.len()
    }

    fn name(&self, entry: usize) -> &str {
        let start = if entry == 0 { 0 } else { (self.ends[entry - 1]).to::<usize>() };
        &self.arena[start..self.ends[entry].to::<usize>()]
    }

    /// The index sorted by hash. A name given twice keeps its last value, as `dict` does.
    fn index(self) -> NameIndex {
        let mut keyed: Vec<(u32, u32)> = (0..self.len()).map(|entry| (name_hash(self.name(entry)), entry.to::<u32>())).collect();
        keyed.sort_unstable_by(|a, b| a.0.cmp(&b.0).then_with(|| self.name(a.1.to::<usize>()).cmp(self.name(b.1.to::<usize>()))).then(a.1.cmp(&b.1)));
        let mut kept: Vec<(u32, u32)> = Vec::with_capacity(keyed.len());
        for &(hash, entry) in &keyed {
            match kept.last() {
                Some(&(_, last)) if self.name(last.to::<usize>()) == self.name(entry.to::<usize>()) => *kept.last_mut().expect("not empty") = (hash, entry),
                _ => kept.push((hash, entry)),
            }
        }
        drop(keyed);
        let Names { mut arena, mut ends } = self;
        arena.shrink_to_fit();
        ends.shrink_to_fit();
        NameIndex { arena, ends, hashes: kept.iter().map(|entry| entry.0).collect(), order: kept.iter().map(|entry| entry.1).collect() }
    }
}

// ---------------------------------------------------------------- counts

/// `dict[str, int]` of word counts.
#[derive(Default)]
pub struct CountTable {
    index: NameIndex,
    counts: Vec<u32>,
}

impl CountTable {
    /// `table.get(word, 0)`.
    pub fn count(&self, word: &str) -> u64 {
        self.index.find(word).map_or(0, |entry| u64::from(self.counts[entry]))
    }

    /// `word in table`.
    pub fn contains(&self, word: &str) -> bool {
        self.index.find(word).is_some()
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// The words, in no particular order.
    pub fn words(&self) -> impl Iterator<Item = &str> {
        self.index.names()
    }

    pub fn heap_bytes(&self) -> usize {
        self.index.heap_bytes() + 4 * self.counts.capacity()
    }
}

/// What a count table reader found wrong with a count.
struct CountsBuilder {
    names: Names,
    counts: Vec<u32>,
    valid: bool,
}

struct NameSeed<'a>(&'a mut Names);

impl<'de> DeserializeSeed<'de> for NameSeed<'_> {
    type Value = Result<(), String>;

    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        struct NameVisitor<'a>(&'a mut Names);
        impl Visitor<'_> for NameVisitor<'_> {
            type Value = Result<(), String>;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a string")
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(self.0.push(value))
            }
        }
        deserializer.deserialize_str(NameVisitor(self.0))
    }
}

/// A count: `type(count) is int and count > 0`; anything else is no count.
enum Count {
    Good(u32),
    Bad,
}

impl<'de> Deserialize<'de> for Count {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct CountVisitor;
        impl<'de> Visitor<'de> for CountVisitor {
            type Value = Count;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a value")
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Count, E> {
                Ok(u32::try_from(value).ok().filter(|&count| count > 0).map_or(Count::Bad, Count::Good))
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Count, E> {
                Ok(u32::try_from(value).ok().filter(|&count| count > 0).map_or(Count::Bad, Count::Good))
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Count, E> {
                Ok(Count::Bad)
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Count, E> {
                Ok(Count::Bad)
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<Count, E> {
                Ok(Count::Bad)
            }
            fn visit_unit<E: de::Error>(self) -> Result<Count, E> {
                Ok(Count::Bad)
            }
            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Count, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(Count::Bad)
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Count, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(Count::Bad)
            }
        }
        deserializer.deserialize_any(CountVisitor)
    }
}

/// One alphabet of a counts file: an object of word to count, or something else (`Bad`).
struct Counts(Option<(CountTable, bool)>);

impl<'de> Deserialize<'de> for Counts {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct CountsVisitor;
        impl<'de> Visitor<'de> for CountsVisitor {
            type Value = Counts;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an object of counts")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Counts, A::Error> {
                let mut builder = CountsBuilder { names: Names::default(), counts: Vec::new(), valid: true };
                while let Some(pushed) = map.next_key_seed(NameSeed(&mut builder.names))? {
                    if pushed.is_err() {
                        builder.valid = false;
                    }
                    match map.next_value::<Count>()? {
                        Count::Good(count) => builder.counts.push(count),
                        Count::Bad => {
                            builder.valid = false;
                            builder.counts.push(0);
                        }
                    }
                }
                let index = builder.names.index();
                Ok(Counts(Some((CountTable { index, counts: builder.counts }, builder.valid))))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Counts, E> {
                Ok(Counts(None))
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Counts, E> {
                Ok(Counts(None))
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<Counts, E> {
                Ok(Counts(None))
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<Counts, E> {
                Ok(Counts(None))
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Counts, E> {
                Ok(Counts(None))
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<Counts, E> {
                Ok(Counts(None))
            }
            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Counts, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(Counts(None))
            }
        }
        deserializer.deserialize_any(CountsVisitor)
    }
}

/// The count tables of a file, by alphabet.
pub struct CountTables {
    tables: Vec<(String, CountTable)>,
}

impl CountTables {
    /// The table of an alphabet (`table[alphabet]`); every alphabet the file was checked for is there.
    pub fn alphabet(&self, name: &str) -> &CountTable {
        static EMPTY: std::sync::OnceLock<CountTable> = std::sync::OnceLock::new();
        self.tables.iter().find(|entry| entry.0 == name).map_or_else(|| EMPTY.get_or_init(CountTable::default), |entry| &entry.1)
    }

    pub fn heap_bytes(&self) -> usize {
        self.tables.iter().map(|entry| entry.1.heap_bytes()).sum()
    }
}

struct TablesVisitor;

impl<'de> Visitor<'de> for TablesVisitor {
    type Value = Option<Vec<(String, Counts)>>;
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an object")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut tables: Vec<(String, Counts)> = Vec::new();
        while let Some(key) = map.next_key::<String>()? {
            let counts = map.next_value::<Counts>()?;
            match tables.iter_mut().find(|entry| entry.0 == key) {
                Some(entry) => entry.1 = counts,
                None => tables.push((key, counts)),
            }
        }
        Ok(Some(tables))
    }
    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_str<E: de::Error>(self, _: &str) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(None)
    }
}

fn json_error(error: &serde_json::Error) -> LoadError {
    use serde_json::error::Category;
    match error.classify() {
        Category::Data => LoadError::value(error.to_string()),
        _ => LoadError { name: "JSONDecodeError", message: error.to_string() },
    }
}

fn open_limited(path: &Path, limit: usize, too_large: &str) -> Result<BufReader<std::fs::File>, LoadError> {
    let file = std::fs::File::open(path).map_err(|error| LoadError::os(&error, path))?;
    let length = file.metadata().map_err(|error| LoadError::os(&error, path))?.len();
    if length > limit.to::<u64>() {
        return Err(LoadError::value(too_large));
    }
    Ok(BufReader::with_capacity(1 << 16, file))
}

/// `context_model.load_term_frequency(path, alphabets, maximum)`: the counts of each alphabet, checked like the
/// artifact. A count above 2^32 - 1 is refused with the others that are no counts.
pub fn load_term_frequency(path: &Path, alphabets: &[&str], limit: usize) -> Result<CountTables, LoadError> {
    let reader = open_limited(path, limit, "term frequency table is too large")?;
    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    let parsed = deserializer.deserialize_any(TablesVisitor).map_err(|error| json_error(&error))?;
    deserializer.end().map_err(|error| json_error(&error))?;
    let invalid = || LoadError::value("invalid term frequency table");
    let mut tables = parsed.ok_or_else(invalid)?;
    // `sorted(payload) != list(alphabets)`: the alphabets are given in sorted order.
    let mut found: Vec<&str> = tables.iter().map(|entry| entry.0.as_str()).collect();
    found.sort_unstable();
    if found != alphabets {
        return Err(invalid());
    }
    let mut result = Vec::with_capacity(tables.len());
    for (alphabet, counts) in tables.drain(..) {
        match counts.0 {
            Some((table, true)) => result.push((alphabet, table)),
            _ => return Err(invalid()),
        }
    }
    Ok(CountTables { tables: result })
}

// ---------------------------------------------------------------- weights

/// What the loader of a weight table checks of each entry; the context model and the prefix model differ in it.
pub struct WeightSpec {
    pub max_entries: usize,
    pub max_name_characters: usize,
    /// The prefix model refuses an empty name.
    pub name_required: bool,
    /// The context model refuses a weight above this magnitude.
    pub max_magnitude: Option<f64>,
    pub entry_error: &'static str,
    pub weight_error: &'static str,
    pub table_error: &'static str,
}

/// `dict[str, tuple[float, float, float, float]]`: the weights of four actions per feature name.
#[derive(Default)]
pub struct WeightTable {
    index: NameIndex,
    values: Vec<f64>,
}

impl WeightTable {
    /// A table with no features: a model whose answers come from elsewhere.
    pub fn empty() -> WeightTable {
        WeightTable { index: NameIndex { arena: String::new(), ends: Vec::new(), hashes: Vec::new(), order: Vec::new() }, values: Vec::new() }
    }
}

pub const ACTION_COUNT: usize = 4;

impl WeightTable {
    /// `weights.get(name)`.
    pub fn get(&self, name: &str) -> Option<&[f64; ACTION_COUNT]> {
        self.index.find(name).map(|entry| self.values[ACTION_COUNT * entry..ACTION_COUNT * (entry + 1)].try_into().expect("four weights"))
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// The names, in no particular order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.index.names()
    }

    /// A table from given entries (the constructor tests use).
    pub fn from_entries<'a>(entries: impl IntoIterator<Item = (&'a str, [f64; ACTION_COUNT])>) -> Self {
        let mut names = Names::default();
        let mut values = Vec::new();
        for (name, weights) in entries {
            names.push(name).expect("a small table");
            values.extend(weights);
        }
        WeightTable { index: names.index(), values }
    }

    pub fn heap_bytes(&self) -> usize {
        self.index.heap_bytes() + 8 * self.values.capacity()
    }
}

/// One weight as the JSON wrote it: an int prints without a point in the checksum.
#[derive(Clone, Copy)]
struct Weight {
    value: f64,
    integer: bool,
    valid: bool,
}

impl<'de> Deserialize<'de> for Weight {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct WeightVisitor;
        impl<'de> Visitor<'de> for WeightVisitor {
            type Value = Weight;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a value")
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Weight, E> {
                Ok(Weight { value: value.to::<f64>(), integer: true, valid: true })
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Weight, E> {
                Ok(Weight { value: value.to::<f64>(), integer: true, valid: true })
            }
            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Weight, E> {
                Ok(Weight { value, integer: false, valid: true })
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Weight, E> {
                Ok(Weight { value: 0.0, integer: false, valid: false })
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<Weight, E> {
                Ok(Weight { value: 0.0, integer: false, valid: false })
            }
            fn visit_unit<E: de::Error>(self) -> Result<Weight, E> {
                Ok(Weight { value: 0.0, integer: false, valid: false })
            }
            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Weight, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(Weight { value: 0.0, integer: false, valid: false })
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Weight, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(Weight { value: 0.0, integer: false, valid: false })
            }
        }
        deserializer.deserialize_any(WeightVisitor)
    }
}

/// The value of one feature name: a list of weights or something else.
struct Entry {
    weights: [Weight; ACTION_COUNT],
    /// How many items the list had; `None` when it was no list.
    items: Option<usize>,
}

impl<'de> Deserialize<'de> for Entry {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct EntryVisitor;
        const NONE: Weight = Weight { value: 0.0, integer: false, valid: false };
        impl<'de> Visitor<'de> for EntryVisitor {
            type Value = Entry;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a value")
            }
            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Entry, A::Error> {
                let mut weights = [NONE; ACTION_COUNT];
                let mut items = 0;
                while let Some(weight) = seq.next_element::<Weight>()? {
                    if items < ACTION_COUNT {
                        weights[items] = weight;
                    }
                    items += 1;
                }
                Ok(Entry { weights, items: Some(items) })
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Entry, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(Entry { weights: [NONE; ACTION_COUNT], items: None })
            }
            fn visit_unit<E: de::Error>(self) -> Result<Entry, E> {
                Ok(Entry { weights: [NONE; ACTION_COUNT], items: None })
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Entry, E> {
                Ok(Entry { weights: [NONE; ACTION_COUNT], items: None })
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<Entry, E> {
                Ok(Entry { weights: [NONE; ACTION_COUNT], items: None })
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<Entry, E> {
                Ok(Entry { weights: [NONE; ACTION_COUNT], items: None })
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Entry, E> {
                Ok(Entry { weights: [NONE; ACTION_COUNT], items: None })
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<Entry, E> {
                Ok(Entry { weights: [NONE; ACTION_COUNT], items: None })
            }
        }
        deserializer.deserialize_any(EntryVisitor)
    }
}

/// The `weights` member of an artifact, parsed and checked entry by entry.
pub struct Weights {
    pub table: WeightTable,
    /// `sha256(json.dumps(weights, sort_keys=True, separators=(",", ":"), ensure_ascii=False))`, hex.
    pub digest: String,
    /// The first entry the spec refused, with its message: raised once the whole file has parsed and the count is right.
    pub refusal: Option<&'static str>,
    /// `len(weights)` of the object, duplicates folded.
    pub entries: usize,
}

struct WeightsSeed<'a>(&'a WeightSpec);

impl<'de> DeserializeSeed<'de> for WeightsSeed<'_> {
    type Value = Option<Weights>;

    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        struct WeightsVisitor<'a>(&'a WeightSpec);
        impl<'de> Visitor<'de> for WeightsVisitor<'_> {
            type Value = Option<Weights>;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an object of weights")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let spec = self.0;
                let mut names = Names::default();
                let mut values: Vec<f64> = Vec::new();
                let mut integers: Vec<u8> = Vec::new();
                let mut refusal: Option<&'static str> = None;
                while let Some(pushed) = map.next_key_seed(NameSeed(&mut names))? {
                    let entry = names.len().wrapping_sub(1);
                    if pushed.is_err() && refusal.is_none() {
                        refusal = Some("the table is too large");
                    }
                    let item = map.next_value::<Entry>()?;
                    let name = if entry < names.len() { names.name(entry) } else { "" };
                    if refusal.is_none() {
                        let bad_name = (spec.name_required && name.is_empty()) || ks_text::pystr::len(name) > spec.max_name_characters;
                        if bad_name || item.items != Some(ACTION_COUNT) {
                            refusal = Some(spec.entry_error);
                        } else if item.weights.iter().any(|weight| {
                            !weight.valid || !weight.value.is_finite() || spec.max_magnitude.is_some_and(|bound| weight.value.abs() > bound)
                        }) {
                            refusal = Some(spec.weight_error);
                        }
                    }
                    let mut mask = 0_u8;
                    for (position, weight) in item.weights.iter().enumerate() {
                        values.push(weight.value);
                        if weight.integer {
                            mask |= 1 << position;
                        }
                    }
                    integers.push(mask);
                }
                values.shrink_to_fit();
                let table = WeightTable { index: names.index(), values };
                let digest = checksum(&table, &integers);
                let entries = table.len();
                Ok(Some(Weights { table, digest, refusal, entries }))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(None)
            }
        }
        deserializer.deserialize_any(WeightsVisitor(self.0))
    }
}

/// The checksum of the weights as the artifacts state it: the JSON text of the object with sorted keys, no spaces and
/// no escaping of non-ASCII letters.
fn checksum(table: &WeightTable, integers: &[u8]) -> String {
    let mut order: Vec<u32> = table.index.order.clone();
    order.sort_unstable_by(|&a, &b| table.index.name(a.to::<usize>()).cmp(table.index.name(b.to::<usize>())));
    let mut hasher = Sha256::new();
    let mut text = String::with_capacity(1 << 16);
    text.push('{');
    for (position, &entry) in order.iter().enumerate() {
        let entry = entry.to::<usize>();
        if position > 0 {
            text.push(',');
        }
        text.push_str(&ks_text::pyjson::json_string(table.index.name(entry), false));
        text.push_str(":[");
        for action in 0..ACTION_COUNT {
            if action > 0 {
                text.push(',');
            }
            let value = table.values[ACTION_COUNT * entry + action];
            if integers[entry] & (1 << action) != 0 {
                text.push_str(&format!("{}", value.to::<i64>()));
            } else {
                text.push_str(&ks_text::float_repr(value));
            }
        }
        text.push(']');
        if text.len() > 1 << 15 {
            hasher.update(text.as_bytes());
            text.clear();
        }
    }
    text.push('}');
    hasher.update(text.as_bytes());
    hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect()
}

/// An artifact: the members the loaders read, and the weights.
pub struct Artifact {
    pub payload: serde_json::Map<String, serde_json::Value>,
    pub weights: Option<Weights>,
}

struct ArtifactSeed<'a>(&'a WeightSpec);

impl<'de> DeserializeSeed<'de> for ArtifactSeed<'_> {
    type Value = Option<Artifact>;

    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        struct ArtifactVisitor<'a>(&'a WeightSpec);
        impl<'de> Visitor<'de> for ArtifactVisitor<'_> {
            type Value = Option<Artifact>;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut payload = serde_json::Map::new();
                let mut weights = None;
                while let Some(key) = map.next_key::<String>()? {
                    if key == "weights" {
                        // A repeated key keeps its last value, as `dict` does.
                        payload.remove("weights");
                        payload.insert(key, serde_json::Value::Null);
                        weights = map.next_value_seed(WeightsSeed(self.0))?;
                    } else {
                        let value = map.next_value::<serde_json::Value>()?;
                        payload.insert(key, value);
                    }
                }
                Ok(Some(Artifact { payload, weights }))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(None)
            }
        }
        deserializer.deserialize_any(ArtifactVisitor(self.0))
    }
}

/// Reads an artifact file of at most `limit` bytes: `None` where the JSON is no object. The size check is the file's
/// length, made before it is read.
const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";

pub fn load_artifact(path: &Path, limit: usize, too_large: &str, spec: &WeightSpec) -> Result<(Option<Artifact>, u64), LoadError> {
    let length = std::fs::metadata(path).map_err(|error| LoadError::os(&error, path))?.len();
    use std::io::BufRead;
    let mut reader = open_limited(path, limit, too_large)?;
    // A byte-order mark does not change what the file says: `json.loads` of bytes skips it too.
    let mark = reader.fill_buf().map_err(|error| LoadError::os(&error, path))?.starts_with(UTF8_BOM);
    if mark {
        reader.consume(UTF8_BOM.len());
    }
    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    let artifact = ArtifactSeed(spec).deserialize(&mut deserializer).map_err(|error| json_error(&error))?;
    deserializer.end().map_err(|error| json_error(&error))?;
    Ok((artifact, length))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_given_twice_keeps_its_last_value() {
        let table = WeightTable::from_entries([("a", [1.0, 2.0, 3.0, 4.0]), ("b", [0.0; 4]), ("a", [5.0, 6.0, 7.0, 8.0])]);
        assert_eq!(table.len(), 2);
        assert_eq!(table.get("a"), Some(&[5.0, 6.0, 7.0, 8.0]));
        assert_eq!(table.get("b"), Some(&[0.0; 4]));
        assert_eq!(table.get("c"), None);
    }

    #[test]
    fn the_checksum_is_that_of_the_sorted_compact_json() {
        // json.dumps({"b": [0, 0.5, 1e-05, -2.0], "a": [1, 1, 1, 1]}, sort_keys=True, separators=(",", ":"))
        let mut names = Names::default();
        for name in ["b", "a"] {
            names.push(name).expect("small");
        }
        let table = WeightTable { index: names.index(), values: vec![0.0, 0.5, 1e-5, -2.0, 1.0, 1.0, 1.0, 1.0] };
        let digest = checksum(&table, &[0b0001, 0b1111]);
        let text = r#"{"a":[1,1,1,1],"b":[0,0.5,1e-05,-2.0]}"#;
        let wanted: String = Sha256::digest(text.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(digest, wanted);
    }
}
