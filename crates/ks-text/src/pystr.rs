//! `str` operations with Python semantics: case mapping with full special casing, strip, split, splitlines,
//! whole-string predicates, and code point indexing with Python's negative-index and clamping rules.
//!
//! Python counts, indexes and slices by code point; Rust's `str` by byte. Everything here counts code points.
//! Case mapping follows `CPython`'s `unicodeobject.c`: `lower` and the lowering inside `title`, `capitalize` and
//! `swapcase` apply the final sigma rule, `upper`/`casefold` map every character independently.

use ks_cast::To as _;
use crate::ucd::{flags, is_case_ignorable, is_cased};
use crate::unicode_tables as t;

// ---------------------------------------------------------------- case mapping

fn single(runs: &[(u32, u16, u8, i32)], cp: u32) -> Option<u32> {
    let i = runs.partition_point(|r| r.0 <= cp);
    if i == 0 {
        return None;
    }
    let (start, n, step, delta) = runs[i - 1];
    let off = cp - start;
    let step = u32::from(step);
    if off.is_multiple_of(step) && off / step < u32::from(n) {
        Some((cp.to::<i64>() + i64::from(delta)).low::<u32>())
    } else {
        None
    }
}

fn push_mapped(out: &mut String, c: char, runs: &[(u32, u16, u8, i32)], multi_cp: &[u32], multi: &[[u32; 3]]) {
    let cp = c.to::<u32>();
    if let Ok(i) = multi_cp.binary_search(&cp) {
        for &m in &multi[i] {
            if m != 0 {
                out.push(char::from_u32(m).expect("table holds scalar values"));
            }
        }
    } else if let Some(m) = single(runs, cp) {
        out.push(char::from_u32(m).expect("table holds scalar values"));
    } else {
        out.push(c);
    }
}

fn push_lower_plain(out: &mut String, c: char) {
    push_mapped(out, c, &t::LOWER_RUNS, &t::LOWER_MULTI_CP, &t::LOWER_MULTI);
}
fn push_upper(out: &mut String, c: char) {
    push_mapped(out, c, &t::UPPER_RUNS, &t::UPPER_MULTI_CP, &t::UPPER_MULTI);
}
fn push_title(out: &mut String, c: char) {
    push_mapped(out, c, &t::TITLE_RUNS, &t::TITLE_MULTI_CP, &t::TITLE_MULTI);
}
fn push_fold(out: &mut String, c: char) {
    push_mapped(out, c, &t::FOLD_RUNS, &t::FOLD_MULTI_CP, &t::FOLD_MULTI);
}

/// Lowercase of `chars[i]` with the context rule for the capital sigma (`CPython` `lower_ucs4`).
fn push_lower(out: &mut String, chars: &[char], i: usize) {
    let c = chars[i];
    if c != '\u{3A3}' {
        return push_lower_plain(out, c);
    }
    // Final sigma: preceded by a cased letter (skipping case-ignorable ones) and not followed by one.
    let mut j = i;
    let mut before = None;
    while j > 0 {
        j -= 1;
        if !is_case_ignorable(chars[j]) {
            before = Some(chars[j]);
            break;
        }
    }
    let mut final_sigma = before.is_some_and(is_cased);
    if final_sigma {
        let after = chars[i + 1..].iter().find(|&&c| !is_case_ignorable(c));
        final_sigma = after.is_none_or(|&c| !is_cased(c));
    }
    out.push(if final_sigma { '\u{3C2}' } else { '\u{3C3}' });
}

/// `str.lower()`.
#[must_use]
pub fn lower(s: &str) -> String {
    if s.is_ascii() {
        return s.to_ascii_lowercase();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for i in 0..chars.len() {
        push_lower(&mut out, &chars, i);
    }
    out
}

/// `str.upper()`.
#[must_use]
pub fn upper(s: &str) -> String {
    if s.is_ascii() {
        return s.to_ascii_uppercase();
    }
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        push_upper(&mut out, c);
    }
    out
}

/// `str.casefold()`.
#[must_use]
pub fn casefold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii() {
            out.push(c.to_ascii_lowercase());
        } else {
            push_fold(&mut out, c);
        }
    }
    out
}

/// `str.capitalize()`: the first character titlecased, the rest lowercased.
#[must_use]
pub fn capitalize(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for i in 0..chars.len() {
        if i == 0 {
            push_title(&mut out, chars[0]);
        } else {
            push_lower(&mut out, &chars, i);
        }
    }
    out
}

/// `str.title()`: a cased character after an uncased one is titlecased, after a cased one lowercased.
#[must_use]
pub fn title(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut previous_is_cased = false;
    for i in 0..chars.len() {
        if previous_is_cased {
            push_lower(&mut out, &chars, i);
        } else {
            push_title(&mut out, chars[i]);
        }
        previous_is_cased = is_cased(chars[i]);
    }
    out
}

/// `str.swapcase()`.
#[must_use]
pub fn swapcase(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for i in 0..chars.len() {
        let f = flags(chars[i].to::<u32>());
        if f & t::UPPER != 0 {
            push_lower(&mut out, &chars, i);
        } else if f & t::LOWER != 0 {
            push_upper(&mut out, chars[i]);
        } else {
            out.push(chars[i]);
        }
    }
    out
}

// ---------------------------------------------------------------- whole-string predicates

fn all_nonempty(s: &str, mask: u16) -> bool {
    !s.is_empty() && s.chars().all(|c| flags(c.to::<u32>()) & mask != 0)
}

/// `str.isalpha()`: not empty and every character is a letter.
#[must_use]
pub fn isalpha(s: &str) -> bool {
    all_nonempty(s, t::ALPHA)
}
/// `str.isalnum()`.
#[must_use]
pub fn isalnum(s: &str) -> bool {
    all_nonempty(s, t::ALPHA | t::DECIMAL | t::DIGIT | t::NUMERIC)
}
/// `str.isdecimal()`.
#[must_use]
pub fn isdecimal(s: &str) -> bool {
    all_nonempty(s, t::DECIMAL)
}
/// `str.isdigit()`.
#[must_use]
pub fn isdigit(s: &str) -> bool {
    all_nonempty(s, t::DIGIT)
}
/// `str.isnumeric()`.
#[must_use]
pub fn isnumeric(s: &str) -> bool {
    all_nonempty(s, t::NUMERIC)
}
/// `str.isspace()`: not empty and every character is whitespace.
#[must_use]
pub fn isspace(s: &str) -> bool {
    all_nonempty(s, t::SPACE)
}
/// `str.isprintable()`: True for the empty string.
#[must_use]
pub fn isprintable(s: &str) -> bool {
    s.chars().all(|c| flags(c.to::<u32>()) & t::PRINTABLE != 0)
}
/// `str.isascii()`.
#[must_use]
pub fn isascii(s: &str) -> bool {
    s.is_ascii()
}

/// `str.isupper()`: at least one cased character, and no lowercase or titlecase one.
#[must_use]
pub fn isupper(s: &str) -> bool {
    let mut cased = false;
    for c in s.chars() {
        let f = flags(c.to::<u32>());
        if f & (t::LOWER | t::TITLE) != 0 {
            return false;
        }
        if f & t::UPPER != 0 {
            cased = true;
        }
    }
    cased
}

/// `str.islower()`: at least one cased character, and no uppercase or titlecase one.
#[must_use]
pub fn islower(s: &str) -> bool {
    let mut cased = false;
    for c in s.chars() {
        let f = flags(c.to::<u32>());
        if f & (t::UPPER | t::TITLE) != 0 {
            return false;
        }
        if f & t::LOWER != 0 {
            cased = true;
        }
    }
    cased
}

/// `str.istitle()`: uppercase and titlecase characters only follow uncased ones, lowercase only cased ones.
#[must_use]
pub fn istitle(s: &str) -> bool {
    let mut cased = false;
    let mut previous_is_cased = false;
    for c in s.chars() {
        let f = flags(c.to::<u32>());
        if f & (t::UPPER | t::TITLE) != 0 {
            if previous_is_cased {
                return false;
            }
            previous_is_cased = true;
            cased = true;
        } else if f & t::LOWER != 0 {
            if !previous_is_cased {
                return false;
            }
            previous_is_cased = true;
            cased = true;
        } else {
            previous_is_cased = false;
        }
    }
    cased
}

// ---------------------------------------------------------------- strip

fn is_ws(c: char) -> bool {
    flags(c.to::<u32>()) & t::SPACE != 0
}

/// `str.strip()` without arguments: Python whitespace on both ends.
pub fn strip(s: &str) -> &str {
    s.trim_matches(is_ws)
}
/// `str.lstrip()` without arguments.
pub fn lstrip(s: &str) -> &str {
    s.trim_start_matches(is_ws)
}
/// `str.rstrip()` without arguments.
pub fn rstrip(s: &str) -> &str {
    s.trim_end_matches(is_ws)
}
/// `str.strip(chars)`: `chars` is a set of code points.
#[must_use]
pub fn strip_chars<'a>(s: &'a str, chars: &str) -> &'a str {
    s.trim_matches(|c| chars.contains(c))
}
/// `str.lstrip(chars)`.
#[must_use]
pub fn lstrip_chars<'a>(s: &'a str, chars: &str) -> &'a str {
    s.trim_start_matches(|c| chars.contains(c))
}
/// `str.rstrip(chars)`.
#[must_use]
pub fn rstrip_chars<'a>(s: &'a str, chars: &str) -> &'a str {
    s.trim_end_matches(|c| chars.contains(c))
}

// ---------------------------------------------------------------- split

/// `str.split()`: runs of Python whitespace separate, empty parts are dropped.
#[must_use]
pub fn split(s: &str) -> Vec<&str> {
    split_max(s, -1)
}

/// `str.split(None, maxsplit)`; a negative `maxsplit` means no limit. The rest after the last split keeps
/// its trailing whitespace but loses the leading one, as in Python.
pub fn split_max(s: &str, maxsplit: i64) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut rest = s;
    let mut left = maxsplit;
    loop {
        rest = rest.trim_start_matches(is_ws);
        if rest.is_empty() {
            break;
        }
        if left == 0 {
            parts.push(rest);
            break;
        }
        if let Some(i) = rest.find(is_ws) {
            parts.push(&rest[..i]);
            rest = &rest[i..];
            left -= 1;
        } else {
            parts.push(rest);
            break;
        }
    }
    parts
}

/// `str.split(sep, maxsplit)`; `None` for an empty separator (Python raises `ValueError`).
#[must_use]
pub fn split_sep<'a>(s: &'a str, sep: &str, maxsplit: i64) -> Option<Vec<&'a str>> {
    if sep.is_empty() {
        return None;
    }
    Some(if maxsplit < 0 {
        s.split(sep).collect()
    } else {
        s.splitn(usize::try_from(maxsplit).unwrap_or(usize::MAX).saturating_add(1), sep).collect()
    })
}

/// `str.rsplit(sep, maxsplit)`; `None` for an empty separator.
#[must_use]
pub fn rsplit_sep<'a>(s: &'a str, sep: &str, maxsplit: i64) -> Option<Vec<&'a str>> {
    if sep.is_empty() {
        return None;
    }
    let mut parts: Vec<&str> = if maxsplit < 0 {
        s.rsplit(sep).collect()
    } else {
        s.rsplitn(usize::try_from(maxsplit).unwrap_or(usize::MAX).saturating_add(1), sep).collect()
    };
    parts.reverse();
    Some(parts)
}

/// `str.splitlines(keepends)`: line boundaries are `\n \r \r\n \v \f \x1c \x1d \x1e \x85    `.
#[must_use]
pub fn splitlines(s: &str, keepends: bool) -> Vec<&str> {
    let mut lines = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0;
    let mut iter = s.char_indices().peekable();
    while let Some((i, c)) = iter.next() {
        let is_break = matches!(c, '\n' | '\r' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}');
        if !is_break {
            continue;
        }
        let mut end = i + c.len_utf8();
        if c == '\r' && bytes.get(end) == Some(&b'\n') {
            iter.next();
            end += 1;
        }
        lines.push(if keepends { &s[start..end] } else { &s[start..i] });
        start = end;
    }
    if start < s.len() {
        lines.push(&s[start..]);
    }
    lines
}

// ---------------------------------------------------------------- prefixes

/// `s.startswith(prefix)`.
#[must_use]
pub fn startswith(s: &str, prefix: &str) -> bool {
    s.starts_with(prefix)
}
/// `s.endswith(suffix)`.
#[must_use]
pub fn endswith(s: &str, suffix: &str) -> bool {
    s.ends_with(suffix)
}
/// `s.startswith((a, b, ...))`.
#[must_use]
pub fn startswith_any(s: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|p| s.starts_with(p))
}
/// `s.endswith((a, b, ...))`.
#[must_use]
pub fn endswith_any(s: &str, suffixes: &[&str]) -> bool {
    suffixes.iter().any(|p| s.ends_with(p))
}

// ---------------------------------------------------------------- code point indexing

/// `len(s)`: the number of code points.
#[must_use]
pub fn len(s: &str) -> usize {
    if s.is_ascii() { s.len() } else { s.chars().count() }
}

/// Python's clamping of a slice or search bound against a length: negative values count from the end,
/// the result lies in `0..=len`.
#[must_use]
pub fn clamp_index(index: i64, len: usize) -> usize {
    let len = len.to::<i64>();
    let adjusted = if index < 0 { index.saturating_add(len) } else { index };
    adjusted.clamp(0, len).to::<usize>()
}

/// Byte offset of the code point `n` (`n` may be the length).
fn byte_offset(s: &str, n: usize) -> usize {
    if s.is_ascii() {
        n.min(s.len())
    } else {
        s.char_indices().nth(n).map_or(s.len(), |(i, _)| i)
    }
}

/// `s[i]`: negative indexes count from the end; `None` where Python raises `IndexError`.
#[must_use]
pub fn char_at(s: &str, index: i64) -> Option<char> {
    let n = len(s).to::<i64>();
    let i = if index < 0 { index + n } else { index };
    if i < 0 || i >= n {
        return None;
    }
    s.chars().nth(i.to::<usize>())
}

/// `s[start:stop]`: bounds are optional, may be negative, and are clamped.
#[must_use]
pub fn slice(s: &str, start: Option<i64>, stop: Option<i64>) -> &str {
    let n = len(s);
    let a = start.map_or(0, |v| clamp_index(v, n));
    let b = stop.map_or(n, |v| clamp_index(v, n));
    if b <= a {
        return "";
    }
    let from = byte_offset(s, a);
    let to = from + byte_offset(&s[from..], b - a);
    &s[from..to]
}

/// `s[start:stop:step]` with any non-zero step; `None` for a zero step (Python raises `ValueError`).
#[must_use]
pub fn slice_step(s: &str, start: Option<i64>, stop: Option<i64>, step: i64) -> Option<String> {
    if step == 0 {
        return None;
    }
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len().to::<i64>();
    let (lo, hi) = if step > 0 { (0, n) } else { (-1, n - 1) };
    let adjust = |v: i64| {
        let v = if v < 0 { v.saturating_add(n) } else { v };
        v.clamp(lo, hi)
    };
    let (a, b) = if step > 0 {
        (start.map_or(0, adjust), stop.map_or(n, adjust))
    } else {
        (start.map_or(n - 1, adjust), stop.map_or(-1, adjust))
    };
    let mut out = String::new();
    let mut i = a;
    if step > 0 {
        while i < b {
            out.push(chars[i.to::<usize>()]);
            i += step;
        }
    } else {
        while i > b {
            out.push(chars[i.to::<usize>()]);
            i += step;
        }
    }
    Some(out)
}

// ---------------------------------------------------------------- search

fn search_window(s: &str, start: Option<i64>, end: Option<i64>) -> (usize, usize) {
    let n = len(s);
    let end = end.map_or(n, |v| clamp_index(v, n));
    // A start past the end is kept as is: `"abc".find("", 5)` is -1.
    let start = match start {
        None => 0,
        Some(v) if v < 0 => clamp_index(v, n),
        Some(v) => usize::try_from(v).unwrap_or(usize::MAX),
    };
    (start, end)
}

/// `s.find(sub, start, end)`: the code point index, or -1.
#[must_use]
pub fn find(s: &str, sub: &str, start: Option<i64>, end: Option<i64>) -> i64 {
    let (a, b) = search_window(s, start, end);
    if a > b || b - a < len(sub) {
        return -1;
    }
    let from = byte_offset(s, a);
    let to = from + byte_offset(&s[from..], b - a);
    match s[from..to].find(sub) {
        Some(i) => (a + len(&s[from..from + i])).to::<i64>(),
        None => -1,
    }
}

/// `s.rfind(sub, start, end)`.
#[must_use]
pub fn rfind(s: &str, sub: &str, start: Option<i64>, end: Option<i64>) -> i64 {
    let (a, b) = search_window(s, start, end);
    if a > b || b - a < len(sub) {
        return -1;
    }
    let from = byte_offset(s, a);
    let to = from + byte_offset(&s[from..], b - a);
    match s[from..to].rfind(sub) {
        Some(i) => (a + len(&s[from..from + i])).to::<i64>(),
        None => -1,
    }
}

/// `s.count(sub, start, end)`: non-overlapping occurrences; an empty `sub` counts the gaps.
#[must_use]
pub fn count(s: &str, sub: &str, start: Option<i64>, end: Option<i64>) -> usize {
    let (a, b) = search_window(s, start, end);
    if a > b || b - a < len(sub) {
        return 0;
    }
    let from = byte_offset(s, a);
    let to = from + byte_offset(&s[from..], b - a);
    let window = &s[from..to];
    if sub.is_empty() { len(window) + 1 } else { window.matches(sub).count() }
}

// ---------------------------------------------------------------- padding

/// `s.ljust(width, fill)`.
#[must_use]
pub fn ljust(s: &str, width: usize, fill: char) -> String {
    let n = len(s);
    let mut out = s.to_string();
    out.extend(std::iter::repeat_n(fill, width.saturating_sub(n)));
    out
}

/// `s.rjust(width, fill)`.
#[must_use]
pub fn rjust(s: &str, width: usize, fill: char) -> String {
    let n = len(s);
    let mut out: String = std::iter::repeat_n(fill, width.saturating_sub(n)).collect();
    out.push_str(s);
    out
}

/// `s.center(width, fill)`: the odd extra fill goes left when `width` is odd, as `CPython` does.
#[must_use]
pub fn center(s: &str, width: usize, fill: char) -> String {
    let n = len(s);
    if width <= n {
        return s.to_string();
    }
    let pad = width - n;
    let left = pad / 2 + (pad & width & 1);
    let mut out: String = std::iter::repeat_n(fill, left).collect();
    out.push_str(s);
    out.extend(std::iter::repeat_n(fill, pad - left));
    out
}

/// `s.zfill(width)`: zeros after a leading sign.
#[must_use]
pub fn zfill(s: &str, width: usize) -> String {
    let n = len(s);
    if width <= n {
        return s.to_string();
    }
    let zeros = width - n;
    let (sign, rest) = match s.chars().next() {
        Some(c @ ('+' | '-')) => (Some(c), &s[1..]),
        _ => (None, s),
    };
    let mut out = String::new();
    out.extend(sign);
    out.extend(std::iter::repeat_n('0', zeros));
    out.push_str(rest);
    out
}

/// Python orders strings by code point; Rust's `str` order by UTF-8 bytes is the same order.
#[must_use]
pub fn cmp(a: &str, b: &str) -> std::cmp::Ordering {
    a.cmp(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_mapping_special_cases() {
        assert_eq!(lower("ΑΣ"), "ας");
        assert_eq!(lower("ΑΣ Σ"), "ας σ");
        assert_eq!(lower("Σ"), "σ");
        assert_eq!(lower("İ"), "i\u{307}");
        assert_eq!(upper("ß"), "SS");
        assert_eq!(casefold("ß"), "ss");
        assert_eq!(casefold("Σς"), "σσ");
        assert_eq!(capitalize("ǆx"), "ǅx");
        assert_eq!(title("hello wORLD-x"), "Hello World-X");
        assert_eq!(swapcase("Привет, Ёж ß"), "пРИВЕТ, ёЖ SS");
        assert_eq!(upper("привет ёж"), "ПРИВЕТ ЁЖ");
    }

    #[test]
    fn whole_string_predicates() {
        assert!(!isalpha("") && !isspace("") && !isupper("") && isprintable(""));
        assert!(isupper("ABC1") && !isupper("1") && !isupper("Ab"));
        assert!(islower("abc 1") && istitle("Hello World") && !istitle("Hello world"));
        assert!(istitle("ǅ") && !isupper("ǅ") && !islower("ǅ"));
        assert!(isdigit("²") && !isdecimal("²") && isnumeric("½") && isalnum("a²"));
        assert!(isspace("\x1c \u{a0}"));
    }

    #[test]
    fn strip_and_split() {
        assert_eq!(strip(" \x1c a b \u{2003}"), "a b");
        assert_eq!(strip_chars("xxaxbxx", "x"), "axb");
        assert_eq!(split(" a  b\u{a0}c "), vec!["a", "b", "c"]);
        assert_eq!(split_max("a b  c ", 1), vec!["a", "b  c "]);
        assert_eq!(split_sep("a,b,,c", ",", -1).unwrap(), vec!["a", "b", "", "c"]);
        assert_eq!(split_sep("a,b,c", ",", 1).unwrap(), vec!["a", "b,c"]);
        assert_eq!(rsplit_sep("a,b,c", ",", 1).unwrap(), vec!["a,b", "c"]);
        assert_eq!(split_sep("a", "", -1), None);
        assert_eq!(splitlines("a\r\nb\rc\u{2028}d\n", false), vec!["a", "b", "c", "d"]);
        assert_eq!(splitlines("a\r\nb\n\nc", true), vec!["a\r\n", "b\n", "\n", "c"]);
        assert!(split("   ").is_empty());
    }

    #[test]
    fn indexing_like_python() {
        let s = "приветик";
        assert_eq!(len(s), 8);
        assert_eq!(slice(s, Some(-3), None), "тик");
        assert_eq!(slice(s, Some(2), Some(-2)), "ивет");
        assert_eq!(slice(s, Some(-100), Some(100)), s);
        assert_eq!(slice(s, Some(5), Some(2)), "");
        assert_eq!(char_at(s, -1), Some('к'));
        assert_eq!(char_at(s, 8), None);
        assert_eq!(slice_step("abcde", None, None, -1).unwrap(), "edcba");
        assert_eq!(slice_step("abcde", Some(-1), Some(0), -2).unwrap(), "ec");
        assert_eq!(slice_step("abcde", Some(1), None, 2).unwrap(), "bd");
        assert_eq!(find("абвабв", "в", Some(3), None), 5);
        assert_eq!(find("abc", "", Some(5), None), -1);
        assert_eq!(rfind("абвабв", "б", None, None), 4);
        assert_eq!(count("aaa", "aa", None, None), 1);
        assert_eq!(count("abc", "", None, None), 4);
        assert_eq!(center("ab", 5, '*'), "**ab*");
        assert_eq!(center("ab", 4, '*'), "*ab*");
        assert_eq!(zfill("-5", 4), "-005");
        assert_eq!(rjust("я", 3, ' '), "  я");
    }
}
