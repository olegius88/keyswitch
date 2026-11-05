//! Counts in Russian texts: the number comes from its constant, the noun agrees with it.
//!
//! A number written into a sentence of a window or a message is the constant it describes,
//! formatted here, so the sentence cannot keep an old value after the constant changes
//! (AGENTS.md). Russian makes the noun agree with the number: `quantity(3.0, SECONDS)` is
//! "3 секунды", `quantity(5.0, SECONDS)` is "5 секунд", `quantity(2.5, SECONDS)` is
//! "2,5 секунды".
//!
//! The three forms of a noun are the ones after 1, after 2-4 and after 5-20, in one
//! grammatical case. Sentences are shaped to need the accusative ("через 3 секунды",
//! "за 5 секунд", "на 3 секунды", "раз в 6 часов", "доступно 10 секунд") or the nominative;
//! in both a fraction takes the genitive singular, which is the second form.

use ks_cast::To as _;
use crate::bigint::BigUint;
use crate::constants::russian_text::{
    RUSSIAN_PLURAL_ELEVEN_EXCEPTION, RUSSIAN_PLURAL_FEW_LAST_DIGITS, RUSSIAN_PLURAL_MOD_HUNDRED,
    RUSSIAN_PLURAL_MOD_TEN, RUSSIAN_PLURAL_TEEN_EXCEPTIONS,
};
use crate::pynum::float_repr;

pub type NounForms = [&'static str; 3];

/// Accusative: "через 1 секунду", "через 2 секунды", "через 5 секунд".
pub const SECONDS: NounForms = ["секунду", "секунды", "секунд"];
pub const HOURS: NounForms = ["час", "часа", "часов"];
/// Nominative, which the accusative of an inanimate noun repeats: "предел — 1000 символов".
pub const CHARACTERS: NounForms = ["символ", "символа", "символов"];
/// Nominative: "1 буква", "2 буквы", "5 букв".
pub const LETTERS: NounForms = ["буква", "буквы", "букв"];

fn is_integer(value: f64) -> bool {
    value.is_finite() && value.fract() == 0.0
}

/// `str(int(value))` of an integral float, exact for any size.
fn integer_text(value: f64) -> String {
    if value == 0.0 { "0".to_string() } else { format!("{value:.0}") }
}

/// A whole value without a fraction part, a fraction with a decimal comma: 3, 2,5.
pub fn russian_number(value: f64) -> String {
    if is_integer(value) {
        return integer_text(value);
    }
    float_repr(value).replace('.', ",")
}

/// `abs(int(value))` of an integral float taken modulo `m`.
fn abs_int_mod(value: f64, m: u32) -> u32 {
    let bits = value.abs().to_bits();
    let exp = ((bits >> 52) & 0x7FF).bits::<i64>();
    if exp == 0 {
        return 0;
    }
    let mantissa = (bits & ((1u64 << 52) - 1)) | (1 << 52);
    let shift = exp - 1075;
    let big = BigUint::from_u64(mantissa);
    let big = if shift >= 0 { big.shl(shift.bits::<u64>()) } else { big.shr((-shift).bits::<u64>()) };
    big.rem_small(m)
}

/// The number and the noun form it takes: 1 секунду, 2 секунды, 5 секунд, 2,5 секунды.
pub fn quantity(value: f64, forms: NounForms) -> String {
    let [one, few, many] = forms;
    let written = russian_number(value);
    if !is_integer(value) {
        return format!("{written} {few}");
    }
    let last = abs_int_mod(value, RUSSIAN_PLURAL_MOD_TEN);
    let last_two = abs_int_mod(value, RUSSIAN_PLURAL_MOD_HUNDRED);
    if last == 1 && last_two != RUSSIAN_PLURAL_ELEVEN_EXCEPTION {
        return format!("{written} {one}");
    }
    if RUSSIAN_PLURAL_FEW_LAST_DIGITS.contains(&last) && !RUSSIAN_PLURAL_TEEN_EXCEPTIONS.contains(&last_two) {
        return format!("{written} {few}");
    }
    format!("{written} {many}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agrees_with_the_number() {
        assert_eq!(quantity(1.0, SECONDS), "1 секунду");
        assert_eq!(quantity(3.0, SECONDS), "3 секунды");
        assert_eq!(quantity(5.0, SECONDS), "5 секунд");
        assert_eq!(quantity(2.5, SECONDS), "2,5 секунды");
        assert_eq!(quantity(11.0, HOURS), "11 часов");
        assert_eq!(quantity(21.0, HOURS), "21 час");
        assert_eq!(quantity(112.0, LETTERS), "112 букв");
        assert_eq!(quantity(0.0, LETTERS), "0 букв");
        assert_eq!(quantity(-2.0, CHARACTERS), "-2 символа");
        assert_eq!(quantity(1e22, CHARACTERS), "10000000000000000000000 символов");
        assert_eq!(russian_number(f64::INFINITY), "inf");
        assert_eq!(russian_number(1.5e-7), "1,5e-07");
        assert_eq!(russian_number(-0.0), "0");
    }
}
