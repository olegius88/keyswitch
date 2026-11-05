//! Compact tables of strings: one UTF-8 arena plus offsets, in sorted order, answered by binary search.
//!
//! A lexicon has hundreds of thousands of words; a map of `String`s would spend more on the allocations than on the
//! text. Sorted UTF-8 is sorted by code point, the order of Python's `sorted` and `bisect` over `str`.

use ks_cast::To as _;
/// A list of strings in one allocation. `offsets[i]..offsets[i + 1]` is the `i`-th string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WordList {
    arena: String,
    offsets: Vec<u32>,
}

impl Default for WordList {
    fn default() -> Self {
        WordList::new()
    }
}

impl WordList {
    pub fn new() -> Self {
        WordList { arena: String::new(), offsets: vec![0] }
    }

    /// Room for `words` strings of `bytes` bytes together, so that the list is built without reallocating.
    pub fn with_capacity(words: usize, bytes: usize) -> Self {
        let mut offsets = Vec::with_capacity(words + 1);
        offsets.push(0);
        WordList { arena: String::with_capacity(bytes), offsets }
    }

    pub fn push(&mut self, word: &str) {
        self.arena.push_str(word);
        self.offsets.push(u32::try_from(self.arena.len()).expect("a word list stays under 4 GiB"));
    }

    pub fn len(&self) -> usize {
        self.offsets.len() - 1
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The bytes of the arena, for the record of what a table costs.
    pub fn arena_bytes(&self) -> usize {
        self.arena.len()
    }

    pub fn get(&self, index: usize) -> &str {
        &self.arena[self.offsets[index].to::<usize>()..(self.offsets[index + 1]).to::<usize>()]
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &str> + Clone {
        (0..self.len()).map(|index| self.get(index))
    }

    /// `bisect_left` over a sorted list.
    pub fn lower_bound(&self, word: &str) -> usize {
        let (mut low, mut high) = (0, self.len());
        while low < high {
            let middle = low + (high - low) / 2;
            if self.get(middle) < word {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        low
    }

    /// Whether a sorted list holds the word.
    pub fn contains_sorted(&self, word: &str) -> bool {
        let index = self.lower_bound(word);
        index < self.len() && self.get(index) == word
    }

    /// Whether every string is greater than the one before: `words == sorted(set(words))`.
    pub fn is_sorted_set(&self) -> bool {
        (1..self.len()).all(|index| self.get(index - 1) < self.get(index))
    }

    /// The same strings sorted, without repeats: `sorted(set(words))`.
    pub fn sorted_unique(self) -> WordList {
        if self.is_sorted_set() {
            return self.shrunk();
        }
        let mut order: Vec<u32> = (0..self.len().to::<u32>()).collect();
        order.sort_unstable_by(|&a, &b| self.get(a.to::<usize>()).cmp(self.get(b.to::<usize>())));
        let mut result = WordList::with_capacity(self.len(), self.arena.len());
        for (position, &index) in order.iter().enumerate() {
            let word = self.get(index.to::<usize>());
            if position == 0 || self.get((order[position - 1]).to::<usize>()) != word {
                result.push(word);
            }
        }
        result.shrunk()
    }

    pub fn shrunk(mut self) -> WordList {
        self.arena.shrink_to_fit();
        self.offsets.shrink_to_fit();
        self
    }

    /// `sorted(set(a) | set(b))` of two sorted sets.
    pub fn merged(&self, other: &WordList) -> WordList {
        let mut result = WordList::with_capacity(self.len() + other.len(), self.arena.len() + other.arena.len());
        let (mut i, mut j) = (0, 0);
        while i < self.len() || j < other.len() {
            if j == other.len() || (i < self.len() && self.get(i) <= other.get(j)) {
                if j < other.len() && self.get(i) == other.get(j) {
                    j += 1;
                }
                result.push(self.get(i));
                i += 1;
            } else {
                result.push(other.get(j));
                j += 1;
            }
        }
        result.shrunk()
    }
}

/// The values of a table: nearly every count of a lexicon fits 32 bits, so the rare table that does not pays alone.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Values {
    Narrow(Vec<i32>),
    Wide(Vec<i64>),
}

impl Values {
    fn new(values: Vec<i64>) -> Self {
        if values.iter().all(|&value| i32::try_from(value).is_ok()) {
            Values::Narrow(values.into_iter().map(|value| value.to::<i32>()).collect())
        } else {
            let mut values = values;
            values.shrink_to_fit();
            Values::Wide(values)
        }
    }

    fn get(&self, index: usize) -> i64 {
        match self {
            Values::Narrow(values) => i64::from(values[index]),
            Values::Wide(values) => values[index],
        }
    }
}

/// A `dict[str, int]`: lookups answer what the dict answers; iteration is in sorted order, not insertion order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WordTable {
    words: WordList,
    values: Values,
}

impl Default for WordTable {
    fn default() -> Self {
        WordTable { words: WordList::new(), values: Values::Narrow(Vec::new()) }
    }
}

impl WordTable {
    /// `dict(pairs)`: a key given twice keeps its last value.
    pub fn from_pairs<S: AsRef<str>, I: IntoIterator<Item = (S, i64)>>(pairs: I) -> Self {
        let mut builder = TableBuilder::new();
        for (word, value) in pairs {
            builder.push(word.as_ref(), value);
        }
        builder.finish(|_earlier, later| later)
    }

    pub fn len(&self) -> usize {
        self.words.len()
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// `word in table and table[word]`.
    pub fn get(&self, word: &str) -> Option<i64> {
        let index = self.words.lower_bound(word);
        (index < self.len() && self.words.get(index) == word).then(|| self.values.get(index))
    }

    /// `table.get(word, 0)`.
    pub fn get_or_zero(&self, word: &str) -> i64 {
        self.get(word).unwrap_or(0)
    }

    /// `word in table`.
    pub fn contains(&self, word: &str) -> bool {
        self.get(word).is_some()
    }

    /// `max(table.values(), default=None)`.
    pub fn max_value(&self) -> Option<i64> {
        (0..self.len()).map(|index| self.values.get(index)).max()
    }

    /// Sorted by key.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&str, i64)> + Clone {
        (0..self.len()).map(|index| (self.words.get(index), self.values.get(index)))
    }

    /// The keys, sorted.
    pub fn words(&self) -> &WordList {
        &self.words
    }

    /// The value of the `index`-th key in sorted order.
    pub fn value_at(&self, index: usize) -> i64 {
        self.values.get(index)
    }

    /// Every key of `extras` (a sorted set) set to `max(table.get(key, 0), value)`.
    pub fn with_floor(&self, extras: &WordList, value: i64) -> WordTable {
        let mut words = WordList::with_capacity(self.len() + extras.len(), self.words.arena_bytes() + extras.arena_bytes());
        let mut values = Vec::with_capacity(self.len() + extras.len());
        let (mut i, mut j) = (0, 0);
        while i < self.len() || j < extras.len() {
            if j == extras.len() || (i < self.len() && self.words.get(i) < extras.get(j)) {
                words.push(self.words.get(i));
                values.push(self.values.get(i));
                i += 1;
            } else if i == self.len() || extras.get(j) < self.words.get(i) {
                words.push(extras.get(j));
                values.push(value.max(0));
                j += 1;
            } else {
                words.push(self.words.get(i));
                values.push(self.values.get(i).max(value));
                i += 1;
                j += 1;
            }
        }
        WordTable { words: words.shrunk(), values: Values::new(values) }
    }
}

/// Collects entries in any order and any number of repeats, then sorts them into one table.
#[derive(Default)]
pub struct TableBuilder {
    words: WordList,
    values: Vec<i64>,
}

impl TableBuilder {
    pub fn new() -> Self {
        TableBuilder::default()
    }

    pub fn push(&mut self, word: &str, value: i64) {
        self.words.push(word);
        self.values.push(value);
    }

    /// The table; entries of one key are folded in the order they came, `merge(earlier, later)`.
    pub fn finish(self, merge: impl Fn(i64, i64) -> i64) -> WordTable {
        let mut order: Vec<u32> = (0..self.words.len().to::<u32>()).collect();
        // Stable: repeats of a key stay in the order they came.
        order.sort_by(|&a, &b| self.words.get(a.to::<usize>()).cmp(self.words.get(b.to::<usize>())));
        let mut words = WordList::with_capacity(self.words.len(), self.words.arena_bytes());
        let mut values: Vec<i64> = Vec::with_capacity(self.words.len());
        for (position, &index) in order.iter().enumerate() {
            let (word, value) = (self.words.get(index.to::<usize>()), self.values[index.to::<usize>()]);
            if position > 0 && self.words.get((order[position - 1]).to::<usize>()) == word {
                let last = values.last_mut().expect("the entry before this one is there");
                *last = merge(*last, value);
            } else {
                words.push(word);
                values.push(value);
            }
        }
        WordTable { words: words.shrunk(), values: Values::new(values) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_list_is_sorted_by_code_point() {
        let mut list = WordList::new();
        for word in ["я", "a", "ё", "z", "a", "\u{1d41a}", "\u{ffff}", ""] {
            list.push(word);
        }
        let sorted = list.sorted_unique();
        let words: Vec<&str> = sorted.iter().collect();
        assert_eq!(words, ["", "a", "z", "я", "ё", "\u{ffff}", "\u{1d41a}"]);
        assert!(sorted.is_sorted_set());
        assert_eq!(sorted.lower_bound("a"), 1);
        assert_eq!(sorted.lower_bound("b"), 2);
        assert_eq!(sorted.lower_bound("\u{10000}"), 6);
        assert!(sorted.contains_sorted("ё") && !sorted.contains_sorted("е"));
    }

    #[test]
    fn merged_sets() {
        let list = |words: &[&str]| {
            let mut l = WordList::new();
            words.iter().for_each(|w| l.push(w));
            l
        };
        let merged = list(&["a", "c", "e"]).merged(&list(&["b", "c", "d", "f"]));
        assert_eq!(merged.iter().collect::<Vec<_>>(), ["a", "b", "c", "d", "e", "f"]);
        assert_eq!(list(&[]).merged(&list(&["x"])).len(), 1);
        assert_eq!(list(&["x"]).merged(&list(&[])).len(), 1);
    }

    #[test]
    fn tables_answer_like_a_dict() {
        let table = WordTable::from_pairs([("b", 2), ("a", 1), ("b", 3)]);
        assert_eq!(table.get("b"), Some(3));
        assert_eq!(table.get("c"), None);
        assert_eq!(table.get_or_zero("c"), 0);
        assert_eq!(table.len(), 2);
        assert_eq!(table.max_value(), Some(3));
        assert_eq!(WordTable::default().max_value(), None);
        let wide = WordTable::from_pairs([("big", i64::MAX), ("small", -1)]);
        assert_eq!(wide.get("big"), Some(i64::MAX));
        assert_eq!(wide.get("small"), Some(-1));
        let mut extras = WordList::new();
        extras.push("a");
        extras.push("c");
        let floored = WordTable::from_pairs([("a", 100), ("b", 1)]).with_floor(&extras, 50);
        assert_eq!(floored.iter().collect::<Vec<_>>(), [("a", 100), ("b", 1), ("c", 50)]);
    }

    #[test]
    fn builder_folds_in_order() {
        let mut builder = TableBuilder::new();
        builder.push("x", 1);
        builder.push("a", 5);
        builder.push("x", 2);
        builder.push("x", 4);
        let table = builder.finish(|a, b| a * 10 + b);
        assert_eq!(table.get("x"), Some(124));
        assert_eq!(table.get("a"), Some(5));
    }
}
