//! Unicode normalisation forms C and D with the tables of the interpreter's Unicode version (16.0).
//!
//! Hand-written instead of a crate because `unicode-normalization` carries a newer Unicode version than
//! CPython 3.14 and the tables here are generated from CPython itself. Hangul is algorithmic.

use ks_cast::To as _;
use crate::unicode_tables as t;

const S_BASE: u32 = 0xAC00;
const L_BASE: u32 = 0x1100;
const V_BASE: u32 = 0x1161;
const T_BASE: u32 = 0x11A7;
const L_COUNT: u32 = 19;
const V_COUNT: u32 = 21;
const T_COUNT: u32 = 28;
const N_COUNT: u32 = V_COUNT * T_COUNT;
const S_COUNT: u32 = L_COUNT * N_COUNT;

/// Canonical combining class (`unicodedata.combining`).
pub fn combining_class(c: char) -> u8 {
    let cp = c.to::<u32>();
    if cp < 0x300 {
        return 0;
    }
    let i = t::CCC_RUNS.partition_point(|r| r.0 <= cp);
    if i > 0 && cp <= t::CCC_RUNS[i - 1].1 { t::CCC_RUNS[i - 1].2 } else { 0 }
}

fn decompose_char(c: char, out: &mut Vec<char>) {
    let cp = c.to::<u32>();
    if (S_BASE..S_BASE + S_COUNT).contains(&cp) {
        let s = cp - S_BASE;
        out.push(char::from_u32(L_BASE + s / N_COUNT).expect("hangul"));
        out.push(char::from_u32(V_BASE + (s % N_COUNT) / T_COUNT).expect("hangul"));
        if !s.is_multiple_of(T_COUNT) {
            out.push(char::from_u32(T_BASE + s % T_COUNT).expect("hangul"));
        }
        return;
    }
    if cp >= 0xC0 {
        if let Ok(i) = t::DECOMP_CP.binary_search(&cp) {
            decompose_char(char::from_u32(t::DECOMP_A[i]).expect("table"), out);
            if t::DECOMP_B[i] != 0 {
                decompose_char(char::from_u32(t::DECOMP_B[i]).expect("table"), out);
            }
            return;
        }
    }
    out.push(c);
}

fn compose_pair(a: char, b: char) -> Option<char> {
    let (a, b) = (a.to::<u32>(), b.to::<u32>());
    if (L_BASE..L_BASE + L_COUNT).contains(&a) && (V_BASE..V_BASE + V_COUNT).contains(&b) {
        return char::from_u32(S_BASE + ((a - L_BASE) * V_COUNT + (b - V_BASE)) * T_COUNT);
    }
    if (S_BASE..S_BASE + S_COUNT).contains(&a) && (a - S_BASE).is_multiple_of(T_COUNT) && (T_BASE + 1..T_BASE + T_COUNT).contains(&b) {
        return char::from_u32(a + (b - T_BASE));
    }
    // Sorted by (first, second): find the range of `a`, then `b` inside it.
    let lo = t::COMPOSE_A.partition_point(|&x| x < a);
    let hi = t::COMPOSE_A.partition_point(|&x| x <= a);
    let j = t::COMPOSE_B[lo..hi].binary_search(&b).ok()?;
    char::from_u32(t::COMPOSE_C[lo + j])
}

fn decomposed(s: &str) -> Vec<char> {
    let mut chars = Vec::with_capacity(s.len());
    for c in s.chars() {
        decompose_char(c, &mut chars);
    }
    // Canonical ordering: a stable sort of every run of characters with a non-zero class.
    let mut i = 0;
    while i < chars.len() {
        if combining_class(chars[i]) == 0 {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < chars.len() && combining_class(chars[j]) != 0 {
            j += 1;
        }
        chars[i..j].sort_by_key(|&c| combining_class(c));
        i = j;
    }
    chars
}

/// `unicodedata.normalize("NFD", s)`.
pub fn nfd(s: &str) -> String {
    if s.chars().all(|c| c.to::<u32>() < 0xC0) {
        return s.to_string();
    }
    decomposed(s).into_iter().collect()
}

/// `unicodedata.normalize("NFC", s)`. Everything below U+0300 is already in NFC.
pub fn nfc(s: &str) -> String {
    if s.chars().all(|c| c.to::<u32>() < 0x300) {
        return s.to_string();
    }
    let mut out: Vec<char> = Vec::with_capacity(s.len());
    let mut starter: Option<usize> = None;
    // Class of the last character kept after the starter; None while the starter is the last one.
    let mut prev_class: Option<u8> = None;
    for ch in decomposed(s) {
        let class = combining_class(ch);
        if let Some(si) = starter {
            let unblocked = prev_class.is_none_or(|p| p < class);
            if unblocked {
                if let Some(composed) = compose_pair(out[si], ch) {
                    out[si] = composed;
                    continue;
                }
            }
        }
        out.push(ch);
        if class == 0 {
            starter = Some(out.len() - 1);
            prev_class = None;
        } else {
            prev_class = Some(class);
        }
    }
    out.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composes_and_decomposes() {
        assert_eq!(nfc("и\u{306}"), "й");
        assert_eq!(nfd("й"), "и\u{306}");
        assert_eq!(nfc("e\u{301}\u{323}"), "ẹ\u{301}");
        assert_eq!(nfc("\u{212B}"), "Å");
        assert_eq!(nfc("\u{1100}\u{1161}\u{11A8}"), "각");
        assert_eq!(nfd("각"), "\u{1100}\u{1161}\u{11A8}");
        assert_eq!(nfc("abc привет"), "abc привет");
        assert_eq!(combining_class('\u{306}'), 230);
        assert_eq!(combining_class('a'), 0);
    }
}
