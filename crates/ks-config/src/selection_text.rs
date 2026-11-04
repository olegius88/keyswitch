//! What the selection hotkeys do to the selected text: its layout, its case and its transliteration.
//!
//! Pure functions of the text, verified without any platform; the platforms only read the selection and type the
//! result over it (selection_commands).

use crate::constants::CAPS_LOCK_WORD_MIN_CAPITALS;
use ks_text::layouts::{LayoutPair, RU_KEYS, US_KEYS};
use ks_text::pystr;
use ks_text::ucd;
use std::collections::HashSet;
use std::sync::OnceLock;

/// The two layouts the selection is converted between, by the names `detection.layouts` gives them.
pub const US_LAYOUT: &str = "us";
pub const RU_LAYOUT: &str = "ru";

/// Signs the two layouts type with Shift, on the keys of US_KEYS and RU_KEYS and on the keys those rows leave out (the
/// number row and the slash key). LayoutPair maps the unshifted rows and the capital letters; these are the rest of the
/// keys a selection typed in the wrong layout holds. Read from xkeyboard-config: /usr/share/X11/xkb/symbols/us,
/// variant "basic", and /usr/share/X11/xkb/symbols/ru, its default variant "winkeys" (the Russian layout of Windows):
/// Shift+2 is `@` and `"`, Shift+3 `#` and `№`, Shift+4 `$` and `;`, Shift+6 `^` and `:`, Shift+7 `&` and `?`, the
/// slash key `/` `?` and `.` `,`, the backslash key with Shift `|` and `/`.
pub const US_SHIFTED_SIGNS: &str = "~{}|:\"<>/?@#$^&";
pub const RU_SHIFTED_SIGNS: &str = "ЁХЪ/ЖЭБЮ.,\"№;:?";

/// A character table, sorted by the character it replaces.
struct Table(Vec<(char, char)>);

impl Table {
    /// `_full_map`: the base mapping with the shifted signs laid over it.
    fn full_map(base: Vec<(char, char)>, source: &str, target: &str) -> Table {
        let mut mapping = base;
        // zip(source, target, strict=True): the two rows have the same length.
        assert_eq!(source.chars().count(), target.chars().count());
        for (from, to) in source.chars().zip(target.chars()) {
            match mapping.iter_mut().find(|(key, _)| *key == from) {
                Some(entry) => entry.1 = to,
                None => mapping.push((from, to)),
            }
        }
        mapping.sort();
        Table(mapping)
    }

    fn get(&self, c: char) -> Option<char> {
        self.0.binary_search_by_key(&c, |entry| entry.0).ok().map(|i| self.0[i].1)
    }

    fn contains(&self, c: char) -> bool {
        self.get(c).is_some()
    }

    /// `text.translate(table)`.
    fn translate(&self, text: &str) -> String {
        text.chars().map(|c| self.get(c).unwrap_or(c)).collect()
    }
}

struct Tables {
    us_to_ru: Table,
    ru_to_us: Table,
    latin_letters: HashSet<char>,
    cyrillic_letters: HashSet<char>,
}

fn letters_of(keys: &str) -> HashSet<char> {
    let mut set = HashSet::new();
    for key in keys.chars() {
        if ucd::isalpha(key) {
            set.insert(key);
            // key.upper() of a letter key is one character.
            set.extend(pystr::upper(&key.to_string()).chars());
        }
    }
    set
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| {
        let pair = LayoutPair::default();
        // What the same key types in the other layout, for every key of the two layouts that types different
        // characters in them. A character the source layout does not type at all (a digit, a space, a sign on the same
        // key in both, a letter of the other script, an emoji) is left as it is.
        Tables {
            us_to_ru: Table::full_map(pair.us_to_ru(), US_SHIFTED_SIGNS, RU_SHIFTED_SIGNS),
            ru_to_us: Table::full_map(pair.ru_to_us(), RU_SHIFTED_SIGNS, US_SHIFTED_SIGNS),
            // The letters each layout types, small and capital: they decide which layout a selection was typed in.
            latin_letters: letters_of(US_KEYS),
            cyrillic_letters: letters_of(RU_KEYS),
        }
    })
}

/// LATIN_LETTERS.
pub fn latin_letters() -> &'static HashSet<char> {
    &tables().latin_letters
}

/// CYRILLIC_LETTERS.
pub fn cyrillic_letters() -> &'static HashSet<char> {
    &tables().cyrillic_letters
}

/// The selection as the same keys type it in the other layout, and the layouts it went between.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutConversion {
    pub text: String,
    pub source: &'static str,
    pub target: &'static str,
}

fn letter_counts(text: &str) -> (usize, usize) {
    let t = tables();
    let latin = text.chars().filter(|c| t.latin_letters.contains(c)).count();
    let cyrillic = text.chars().filter(|c| t.cyrillic_letters.contains(c)).count();
    (latin, cyrillic)
}

/// For each layout: the table to the other layout, the other layout's name, and the letters it types.
fn conversion(source: &str) -> (&'static Table, &'static str, &'static HashSet<char>) {
    let t = tables();
    if source == US_LAYOUT {
        (&t.us_to_ru, RU_LAYOUT, &t.latin_letters)
    } else {
        (&t.ru_to_us, US_LAYOUT, &t.cyrillic_letters)
    }
}

/// `text` with only the runs typed in `source` retyped in the other layout.
///
/// A run goes from a letter of the source layout to its last letter before a space, a digit or a letter of the other
/// script, signs on the source layout's keys inside it included: `j,]trn` is `объект` wherever it stands. A sign at
/// either end of a run stays as typed (`vbh.` in `привет vbh.` is `мир.`): in a sentence of the other script it most
/// likely belongs to that sentence, and a sign alone between words (the comma of `привет, vbh`) is no run at all.
fn converted_runs(text: &str, source: &str) -> String {
    let (table, _target, letters) = conversion(source);
    let mut out = String::new();
    let mut run: Vec<char> = Vec::new();
    let flush = |run: &mut Vec<char>, out: &mut String| {
        let inside: Vec<usize> = run.iter().enumerate().filter(|(_, item)| letters.contains(item)).map(|(i, _)| i).collect();
        if let (Some(&first), Some(&last)) = (inside.first(), inside.last()) {
            let last = last + 1;
            out.extend(&run[..first]);
            out.extend(run[first..last].iter().map(|c| table.get(*c).unwrap_or(*c)));
            out.extend(&run[last..]);
        } else {
            out.extend(run.iter());
        }
        run.clear();
    };
    for character in text.chars() {
        if table.contains(character) {
            run.push(character);
            continue;
        }
        flush(&mut run, &mut out);
        out.push(character);
    }
    flush(&mut run, &mut out);
    out
}

/// The selection retyped on the same keys in the other layout: `ghbdtn vbh` is `привет мир`.
///
/// A selection in one script was typed in the other layout as a whole, and all of it is retyped; pressing again
/// converts back. A selection mixing the scripts is mostly right: the script with more letters is what was meant, and
/// only the runs of the other one are retyped into its layout (`привет vbh` is `привет мир`, `hello руддщ` is
/// `hello hello`), with the signs on their keys inside them (converted_runs). A selection with as many letters of one
/// as of the other, or with no letters at all (`?/`), was typed in the layout that is on now and is retyped whole.
/// The conversion names the layout the keyboard is switched to afterwards: the other one for a whole selection, that
/// of the majority for a mixed one.
pub fn convert_layout(text: &str, current_layout: &str) -> LayoutConversion {
    let (latin, cyrillic) = letter_counts(text);
    if latin > 0 && cyrillic > 0 && latin != cyrillic {
        let minority = if latin < cyrillic { US_LAYOUT } else { RU_LAYOUT };
        return LayoutConversion { text: converted_runs(text, minority), source: minority, target: conversion(minority).1 };
    }
    let source = if latin > cyrillic {
        US_LAYOUT
    } else if cyrillic > latin || current_layout == RU_LAYOUT {
        RU_LAYOUT
    } else {
        US_LAYOUT
    };
    let (table, target, _letters) = conversion(source);
    LayoutConversion { text: table.translate(text), source, target }
}

/// Letters inside a word: an apostrophe does not start a new word (`don't` is one word, `Don't` in Title case).
fn word_inside(c: char) -> bool {
    c == '\'' || c == '’'
}

fn one(c: char) -> String {
    c.to_string()
}

fn is_upper(c: char) -> bool {
    pystr::isupper(&one(c))
}
fn is_lower(c: char) -> bool {
    pystr::islower(&one(c))
}

/// The characters whose lower and upper case differ.
fn cased(text: &str) -> Vec<char> {
    text.chars().filter(|c| pystr::lower(&one(*c)) != pystr::upper(&one(*c))).collect()
}

/// Every word with its first letter capital and the rest small.
fn title(text: &str) -> String {
    let mut result = String::new();
    let mut before: Option<char> = None;
    let mut previous: Option<char> = None;
    for character in text.chars() {
        // An apostrophe is inside a word only after a letter: `don't`, but `'hello'` starts a word after it.
        let inside = previous.is_some_and(ucd::isalnum)
            || (previous.is_some_and(word_inside) && before.is_some_and(ucd::isalnum));
        let text = one(character);
        result.push_str(&if inside { pystr::lower(&text) } else { pystr::upper(&text) });
        before = previous;
        previous = Some(character);
    }
    result
}

/// The runs of letters (`[^\W\d_]+`).
fn letter_runs(text: &str) -> Vec<String> {
    let mut runs = Vec::new();
    let mut current = String::new();
    for c in text.chars() {
        if ucd::re_letter(c) {
            current.push(c);
        } else if !current.is_empty() {
            runs.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        runs.push(current);
    }
    runs
}

/// Whether the text looks typed with Caps Lock on: `пРИВЕТ мИР`, every word small at its first letter only.
///
/// A word of one letter says nothing, and a word all in capitals fits too (`пРИВЕТ, Я ТУТ`); at least one word must
/// show the small first letter followed by CAPS_LOCK_WORD_MIN_CAPITALS capitals or more. `iOS` and `eSIM` have fewer
/// and are names written that way, not Caps Lock.
fn typed_under_caps_lock(text: &str) -> bool {
    let mut seen = false;
    for word in letter_runs(text) {
        let letters = cased(&word);
        if letters.len() <= 1 {
            continue;
        }
        if !letters[1..].iter().all(|letter| is_upper(*letter)) {
            return false;
        }
        seen = seen || (is_lower(letters[0]) && letters.len() > CAPS_LOCK_WORD_MIN_CAPITALS);
    }
    seen
}

/// The next case of the selection: small → CAPITALS → Title Case → small, and Caps Lock text turned over.
///
/// The cycle follows from the text alone, because the selection is read anew at every press: whatever was selected,
/// the press moves it one step on, and three presses bring back what was there. Text typed with Caps Lock on
/// (`пРИВЕТ`) is turned over first, to `Привет`, as Punto Switcher's case command does with this very example; any
/// other mixture of cases (`Привет мир`) goes to small letters. Title Case capitalizes every word rather than the
/// first of a sentence: words are found by letters alone, sentences are not (`т. е.`, `Dr.`). A step that would not
/// change the text (`Я` is in capitals and in Title Case at once) is passed over.
pub fn cycle_case(text: &str) -> String {
    let letters = cased(text);
    if letters.is_empty() {
        return text.to_string();
    }
    if typed_under_caps_lock(text) {
        return pystr::swapcase(text);
    }
    type Step = fn(&str) -> String;
    let steps: [Step; 3] = if letters.iter().all(|letter| is_lower(*letter)) {
        [pystr::upper, title, pystr::lower]
    } else if letters.iter().all(|letter| is_upper(*letter)) {
        [title, pystr::lower, pystr::upper]
    } else {
        [pystr::lower, pystr::upper, title]
    };
    steps.iter().map(|step| step(text)).find(|result| result != text).unwrap_or_else(|| text.to_string())
}

/// Russian letters in Latin by ICAO Doc 9303, Machine Readable Travel Documents, Eighth Edition 2021, Part 3, section
/// 6, table B "Transliteration of Cyrillic Characters" (https://www.icao.int/sites/default/files/publications/DocSeries/
/// 9303_p3_cons_en.pdf), the transliteration of machine readable travel documents. Ё and Э are E like Е, Й is I like И,
/// Ъ is IE, and Ь has no row in the table, so it is left out.
pub const CYRILLIC_TO_LATIN: &[(&str, &str)] = &[
    ("а", "a"), ("б", "b"), ("в", "v"), ("г", "g"), ("д", "d"), ("е", "e"), ("ё", "e"), ("ж", "zh"), ("з", "z"),
    ("и", "i"), ("й", "i"), ("к", "k"), ("л", "l"), ("м", "m"), ("н", "n"), ("о", "o"), ("п", "p"), ("р", "r"),
    ("с", "s"), ("т", "t"), ("у", "u"), ("ф", "f"), ("х", "kh"), ("ц", "ts"), ("ч", "ch"), ("ш", "sh"),
    ("щ", "shch"), ("ъ", "ie"), ("ы", "y"), ("ь", ""), ("э", "e"), ("ю", "iu"), ("я", "ia"),
];

/// Latin back to Russian, the best the lossy standard allows: each sequence it writes for one letter goes back to the
/// letter it is most often written for (`e` is е, not ё or э; `ia` is я, not и followed by а). `ie` is left to its two
/// letters, since ICAO writes и followed by е the same way and that is far more common than ъ. Common informal
/// spellings the standard never writes are read too: `ya`, `yu`, `yo` and `ja`, `ju`, `jo` are я, ю, ё (ICAO would
/// write ы before a vowel, which no Russian word has), and the letters `c` ц, `h` х, `j` й, `q` к, `w` в, `x` кс.
pub const LATIN_TO_CYRILLIC: &[(&str, &str)] = &[
    ("shch", "щ"), ("zh", "ж"), ("kh", "х"), ("ts", "ц"), ("ch", "ч"), ("sh", "ш"), ("iu", "ю"), ("ia", "я"),
    ("ya", "я"), ("yu", "ю"), ("yo", "ё"), ("ja", "я"), ("ju", "ю"), ("jo", "ё"),
    ("a", "а"), ("b", "б"), ("v", "в"), ("g", "г"), ("d", "д"), ("e", "е"), ("z", "з"), ("i", "и"), ("k", "к"),
    ("l", "л"), ("m", "м"), ("n", "н"), ("o", "о"), ("p", "п"), ("r", "р"), ("s", "с"), ("t", "т"), ("u", "у"),
    ("f", "ф"), ("y", "ы"), ("c", "ц"), ("h", "х"), ("j", "й"), ("q", "к"), ("w", "в"), ("x", "кс"),
];

fn cyrillic_to_latin(key: &str) -> Option<&'static str> {
    CYRILLIC_TO_LATIN.iter().find(|(cyrillic, _)| *cyrillic == key).map(|(_, latin)| *latin)
}

fn latin_to_cyrillic(key: &str) -> Option<&'static str> {
    if key.is_empty() {
        return None;
    }
    LATIN_TO_CYRILLIC.iter().find(|(latin, _)| *latin == key).map(|(_, cyrillic)| *cyrillic)
}

/// Sequences read by their place in the word, before the table. Each rule was measured on the Russian lexicon of the
/// language model (every word written by ICAO and read back, weighted by frequency: 92.5 % of the text came back before
/// them, 96.6 % with them): a word-final `tsia` is the -тся of verbs (`iavliaetsia`, not являеця; -ция is `tsiia`),
/// `ts` before `k` is тс (`sovetskii`, not совецкий), and a word that starts with `ye` starts with е (`Yelena`), since
/// no Russian word starts with ые.
const VERB_ENDING: (&str, &str) = ("tsia", "тся");
const TS_BEFORE_K: (&str, &str) = ("tsk", "тс");
const INITIAL_YE: (&str, &str) = ("ye", "е");
/// `i` after a vowel is й before a consonant (`voina`, `Chaikovskii`) and at the end of a word (`Sergei`, `krasnyi`),
/// except after и, where -ии of nouns (`Rossii`, `istorii`) is more common than -ий, unless к, г or х come before it
/// (`russkii`, `dolgii`): the measure above counts these too.
const SHORT_I: &str = "й";
const CYRILLIC_VOWELS: &str = "аеёиоуыэюя";
const LATIN_VOWELS: &str = "aeiouy";
const BEFORE_ADJECTIVE_ENDING: &str = "кгх";
/// An apostrophe between Latin letters is the soft sign of informal spelling (`Ol'ga`); ICAO writes none.
const SOFT_SIGN: &str = "ь";

/// The lengths of the table's sequences, longest first: the longest one that matches is read (`shch` before `sh`).
fn sequence_lengths() -> Vec<usize> {
    let mut lengths: Vec<usize> = LATIN_TO_CYRILLIC.iter().map(|(sequence, _)| sequence.chars().count()).collect();
    lengths.sort_unstable_by(|a, b| b.cmp(a));
    lengths.dedup();
    lengths
}

fn slice(text: &[char], start: usize, end: usize) -> String {
    text[start.min(text.len())..end.min(text.len())].iter().collect()
}

/// `output` in the case of `text[start:end]`, which it stands for: small, Capitalized, or all CAPITALS.
///
/// A capital stands for a word in capitals when the letter after it is a capital, or when the sequence it starts is in
/// capitals, or when it ends a word that a capital comes before: `ЖУК` is `ZHUK`, `Жук` is `Zhuk`, `ВЬЮ` is `VIU`.
fn in_case(output: &str, text: &[char], start: usize, end: usize) -> String {
    let first = text[start];
    if !is_upper(first) {
        return pystr::lower(output);
    }
    let after = text.get(end).copied();
    let before = if start > 0 { Some(text[start - 1]) } else { None };
    let capitals = after.is_some_and(is_upper)
        || (end - start > 1 && pystr::isupper(&slice(text, start, end)))
        || (!after.is_some_and(ucd::isalpha) && before.is_some_and(is_upper));
    if capitals { pystr::upper(output) } else { pystr::capitalize(output) }
}

fn to_latin(text: &[char]) -> String {
    let mut out = String::new();
    for (index, &character) in text.iter().enumerate() {
        match cyrillic_to_latin(&pystr::lower(&one(character))) {
            None => out.push(character),
            Some(latin) => out.push_str(&in_case(latin, text, index, index + 1)),
        }
    }
    out
}

fn latin_letter(c: char) -> bool {
    c.is_ascii() && ucd::isalpha(c)
}

/// Whether no Latin letter stands at `index` (the end of the text counts).
fn word_edge(text: &[char], index: usize) -> bool {
    index >= text.len() || !latin_letter(text[index])
}

/// A sequence read by its place in the word (VERB_ENDING, TS_BEFORE_K, INITIAL_YE), with its Cyrillic.
fn placed_sequence(text: &[char], index: usize) -> Option<(String, &'static str)> {
    let candidates = [
        (VERB_ENDING, word_edge(text, index + VERB_ENDING.0.len())),
        (TS_BEFORE_K, true),
        (INITIAL_YE, index == 0 || word_edge(text, index - 1)),
    ];
    for ((latin, cyrillic), placed) in candidates {
        if placed && pystr::lower(&slice(text, index, index + latin.len())) == latin {
            // The `k` after `ts` is read on its own: only тс is written for this match.
            let sequence = if (latin, cyrillic) == TS_BEFORE_K { &latin[..latin.len() - 1] } else { latin };
            return Some((sequence.to_string(), cyrillic));
        }
    }
    None
}

/// Whether the `i` that ends at `end` is й (see SHORT_I).
fn short_i(text: &[char], end: usize, parts: &str) -> bool {
    let written: Vec<char> = pystr::lower(parts).chars().collect();
    let vowel = written.last().copied();
    if !vowel.is_some_and(|v| CYRILLIC_VOWELS.contains(v)) {
        return false;
    }
    if !word_edge(text, end) {
        // The letter after it is a Latin one, so its lower case is one character.
        return !LATIN_VOWELS.contains(text[end].to_ascii_lowercase());
    }
    vowel != Some('и') || written.len() >= 2 && BEFORE_ADJECTIVE_ENDING.contains(written[written.len() - 2])
}

fn to_cyrillic(text: &[char]) -> String {
    let lengths = sequence_lengths();
    let mut parts = String::new();
    let mut index = 0;
    while index < text.len() {
        let placed = placed_sequence(text, index);
        // Only ASCII letters are read: `İ` folds to two characters and would shift every match after it.
        let sequence: Option<String> = match &placed {
            Some((sequence, _)) => Some(sequence.clone()),
            None => lengths.iter().find_map(|&length| {
                let chunk = slice(text, index, index + length);
                let lowered = pystr::lower(&chunk);
                (chunk.is_ascii() && latin_to_cyrillic(&lowered).is_some()).then_some(lowered)
            }),
        };
        let Some(sequence) = sequence else {
            let between_letters = index > 0 && latin_letter(text[index - 1]) && !word_edge(text, index + 1);
            if (text[index] == '\'' || text[index] == '’') && between_letters {
                parts.push_str(SOFT_SIGN);
            } else {
                parts.push(text[index]);
            }
            index += 1;
            continue;
        };
        let end = index + sequence.chars().count();
        let mut cyrillic = match &placed {
            Some((_, cyrillic)) => cyrillic,
            None => latin_to_cyrillic(&sequence).expect("the sequence is in the table"),
        };
        if sequence == "i" && short_i(text, end, &parts) {
            cyrillic = SHORT_I;
        }
        parts.push_str(&in_case(cyrillic, text, index, end));
        index = end;
    }
    parts
}

/// The selection in the other script: `Привет` is `Privet` and `Privet` is `Привет`.
///
/// The script with more letters in the selection is the one written over, and only its letters change: `Привет, John`
/// is `Privet, John`. A tie goes to the Russian letters, which the standard is written for. Letters of neither
/// alphabet stay as they are.
pub fn transliterate(text: &str) -> String {
    let (latin, cyrillic) = letter_counts(text);
    let chars: Vec<char> = text.chars().collect();
    if cyrillic > 0 && cyrillic >= latin {
        return to_latin(&chars);
    }
    if latin > 0 {
        return to_cyrillic(&chars);
    }
    text.to_string()
}
