//! Per-character predicates identical to `CPython` 3.14 (Unicode 16.0), for every code point 0..=0x10FFFF.
//!
//! Rust's `char::is_alphabetic`, `is_whitespace`, `is_numeric`, `is_uppercase` ... differ from Python's
//! `str` methods and from what `re` treats as `\w`, `\d`, `\s` (for example `'\x1c'.isspace()` is True in
//! Python and false in Rust, `'²'.isdigit()` is True and `isdecimal()` False). Everything the program's
//! decisions see must go through this module.
//!
//! The data is one table of runs of equal flags (`unicode_tables`, generated from the interpreter itself)
//! behind a binary search, with direct tables for ASCII and Cyrillic, where nearly all text lives.
//! Each predicate exists twice: on `char` and, with the suffix `_cp`, on a raw code point `u32`, because
//! Python strings may hold lone surrogates (U+D800..=U+DFFF) that a Rust `char` cannot.
//!
//! The `istitle`/`isupper`/`islower` of a single character are what `c.istitle()` ... return in Python
//! for a one-character string; the raw flags the string methods need are `is_upper_flag` and friends.

use ks_cast::To as _;
use crate::unicode_tables as t;

pub use t::UNICODE_VERSION;

/// All property flags of a code point (`t::ALPHA | ...`); 0 above U+10FFFF.
#[inline]
#[must_use]
pub fn flags(cp: u32) -> u16 {
    if cp < 128 {
        t::ASCII_FLAGS[cp.to::<usize>()]
    } else if (t::CYRILLIC_LO..t::CYRILLIC_HI).contains(&cp) {
        t::CYRILLIC_FLAGS[(cp - t::CYRILLIC_LO).to::<usize>()]
    } else if cp > 0x0010_FFFF {
        0
    } else {
        let i = t::FLAG_STARTS.partition_point(|&s| s <= cp) - 1;
        t::FLAG_VALUES[i]
    }
}

const ALNUM: u16 = t::ALPHA | t::DECIMAL | t::DIGIT | t::NUMERIC;

macro_rules! predicate {
    ($(#[$doc:meta])* $name:ident, $cp_name:ident, |$f:ident, $cp:ident| $body:expr) => {
        $(#[$doc])*
        #[inline]
        pub fn $cp_name($cp: u32) -> bool {
            let $f = flags($cp);
            let _ = $cp;
            $body
        }
        $(#[$doc])*
        #[inline]
        pub fn $name(c: char) -> bool {
            $cp_name(u32::from(c))
        }
    };
}

predicate!(
    /// `str.isalpha()` of the character: Unicode categories Lm, Lt, Lu, Ll, Lo.
    isalpha, isalpha_cp, |f, cp| f & t::ALPHA != 0);
predicate!(
    /// `str.isalnum()`: alpha, decimal, digit or numeric.
    isalnum, isalnum_cp, |f, cp| f & ALNUM != 0);
predicate!(
    /// `str.isdecimal()`: Unicode category Nd.
    isdecimal, isdecimal_cp, |f, cp| f & t::DECIMAL != 0);
predicate!(
    /// `str.isdigit()`: decimal digits and others with the Digit property (`²` is a digit, not decimal).
    isdigit, isdigit_cp, |f, cp| f & t::DIGIT != 0);
predicate!(
    /// `str.isnumeric()`: anything with a numeric value (`½`, `Ⅳ`, `一`).
    isnumeric, isnumeric_cp, |f, cp| f & t::NUMERIC != 0);
predicate!(
    /// `str.isspace()`: bidirectional class WS, B or S, or category Zs. True for `\x1c`..`\x1f` and `\x85`.
    isspace, isspace_cp, |f, cp| f & t::SPACE != 0);
predicate!(
    /// `c.isupper()` of a one-character string: the Uppercase property.
    isupper, isupper_cp, |f, cp| f & t::UPPER != 0);
predicate!(
    /// `c.islower()` of a one-character string: the Lowercase property.
    islower, islower_cp, |f, cp| f & t::LOWER != 0);
predicate!(
    /// `c.istitle()` of a one-character string: uppercase or titlecase (category Lt).
    istitle, istitle_cp, |f, cp| f & (t::UPPER | t::TITLE) != 0);
predicate!(
    /// `str.isprintable()`: everything except categories Cc, Cf, Cs, Co, Cn, Zl, Zp, Zs, but the space.
    isprintable, isprintable_cp, |f, cp| f & t::PRINTABLE != 0);
predicate!(
    /// The raw Titlecase flag (category Lt), which `str.isupper/islower/istitle` use on whole strings.
    is_title_flag, is_title_flag_cp, |f, cp| f & t::TITLE != 0);
predicate!(
    /// The Cased property (Lowercase, Uppercase or Lt): what `title()` and the final sigma rule look at.
    is_cased, is_cased_cp, |f, cp| f & t::CASED != 0);
predicate!(
    /// The Case_Ignorable property, skipped by the final sigma rule.
    is_case_ignorable, is_case_ignorable_cp, |f, cp| f & t::CASE_IGNORABLE != 0);
predicate!(
    /// `\w` of `re` on a `str` pattern: `isalnum()` or `_`.
    re_w, re_w_cp, |f, cp| f & ALNUM != 0 || cp == 0x5F);
predicate!(
    /// `\d` of `re` on a `str` pattern: `isdecimal()`.
    re_d, re_d_cp, |f, cp| f & t::DECIMAL != 0);
predicate!(
    /// `\s` of `re` on a `str` pattern: `isspace()`.
    re_s, re_s_cp, |f, cp| f & t::SPACE != 0);
predicate!(
    /// The class `[^\W\d_]`: `\w` but not `\d` and not `_`. It keeps `²`, `½`, `Ⅳ` and drops combining marks.
    re_letter, re_letter_cp, |f, cp| f & ALNUM != 0 && f & t::DECIMAL == 0 && cp != 0x5F);

/// The value of a decimal digit (`unicodedata.decimal(c, None)`): `0..=9` or `None`.
#[must_use]
pub fn decimal_value(c: char) -> Option<u32> {
    let cp = c.to::<u32>();
    if cp < 128 {
        return if (0x30..=0x39).contains(&cp) { Some(cp - 0x30) } else { None };
    }
    if !isdecimal_cp(cp) {
        return None;
    }
    let i = t::DECIMAL_ZEROS.partition_point(|&z| z <= cp);
    Some(cp - t::DECIMAL_ZEROS[i - 1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_differs_from_rust_char_methods() {
        assert!(isspace('\x1c') && !'\x1c'.is_whitespace());
        assert!(isdigit('²') && !isdecimal('²') && isnumeric('²'));
        assert!(re_letter('²') && re_letter('½') && re_letter('Ⅳ'));
        assert!(!re_letter('\u{306}') && !re_letter('_') && !re_letter('1'));
        assert!(isalpha('й') && isalpha('ё') && isupper('Ё') && islower('ё'));
        assert!(istitle('ǅ') && !isupper('ǅ') && !islower('ǅ'));
        assert!(isprintable(' ') && !isprintable('\n') && !isprintable('\u{a0}'));
        assert!(isalpha_cp(0x61) && !isalpha_cp(0xD800) && !isalpha_cp(0x0011_0000));
        assert!(re_w('_') && re_w('٣') && !re_w('-'));
        assert!(re_s('\u{85}') && re_s('\u{2028}') && !re_s('\u{200b}'));
    }

    #[test]
    fn decimal_values() {
        assert_eq!(decimal_value('7'), Some(7));
        assert_eq!(decimal_value('٣'), Some(3));
        assert_eq!(decimal_value('९'), Some(9));
        assert_eq!(decimal_value('²'), None);
        assert_eq!(decimal_value('a'), None);
    }
}
