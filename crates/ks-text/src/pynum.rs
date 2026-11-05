//! Python numbers: `round`, `int(str)`, `float(str)`, `repr(float)`, `math.fsum`, `statistics.fmean` and
//! `pstdev`, and thin `math` wrappers.
//!
//! Where Python raises, the function returns `None` (or an `Err` that names the error); nothing panics.

use ks_cast::To as _;
use crate::bigint::BigUint;
use crate::ucd;

// ---------------------------------------------------------------- repr(float)

/// `repr(x)` and `str(x)` of a float: the shortest text that parses back to `x`, in Python's layout
/// (`1e+16`, `1.5e-07`, `0.0001`, `inf`, `nan`, `-0.0`).
pub fn float_repr(x: f64) -> String {
    if x.is_nan() {
        return "nan".to_string();
    }
    if x.is_infinite() {
        return if x < 0.0 { "-inf" } else { "inf" }.to_string();
    }
    let sign = if x.is_sign_negative() { "-" } else { "" };
    if x == 0.0 {
        return format!("{sign}0.0");
    }
    // Rust's `{:e}` prints the shortest digits that round-trip, as Python's repr does, but when two
    // candidates of that length are equally close to the exact value it takes the upper one (Python's
    // dtoa takes the even one: 742082672164581.25 is `...581.2`). The correctly rounded text of the same
    // length, when it still round-trips, is the one Python writes.
    let mut text = format!("{:e}", x.abs());
    let shortest_len = text.split_once('e').expect("exponent form").0.chars().filter(|c| c.is_ascii_digit()).count();
    let exact = format!("{:.*e}", shortest_len - 1, x.abs());
    if exact != text && exact.parse::<f64>().is_ok_and(|v| v == x.abs()) {
        text = exact;
    }
    let (mantissa, exponent) = text.split_once('e').expect("exponent form");
    let digits: String = mantissa.chars().filter(|&c| c != '.').collect();
    let exp10: i32 = exponent.parse().expect("exponent");
    let decpt = exp10 + 1;
    let n = digits.len().low::<i32>();
    let mut out = String::from(sign);
    if !(-4..16).contains(&exp10) {
        out.push_str(&digits[..1]);
        if n > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        out.push(if exp10 < 0 { '-' } else { '+' });
        out.push_str(&format!("{:02}", exp10.abs()));
    } else if decpt <= 0 {
        out.push_str("0.");
        out.extend(std::iter::repeat_n('0', (-decpt).to::<usize>()));
        out.push_str(&digits);
    } else if decpt >= n {
        out.push_str(&digits);
        out.extend(std::iter::repeat_n('0', (decpt - n).to::<usize>()));
        out.push_str(".0");
    } else {
        out.push_str(&digits[..decpt.to::<usize>()]);
        out.push('.');
        out.push_str(&digits[decpt.to::<usize>()..]);
    }
    out
}

/// `str(x)` of a float is its repr.
pub fn float_str(x: f64) -> String {
    float_repr(x)
}

// ---------------------------------------------------------------- parsing

/// What CPython does before parsing a number from text: decimal digits of any script become ASCII digits,
/// whitespace becomes a space, anything else non-ASCII becomes `?`; then the ends are stripped.
fn to_ascii_number_text(s: &str) -> String {
    // A pure ASCII text is left as it is (so `\x1c` is not a space there, but it is in a text with other
    // non-ASCII characters).
    if s.is_ascii() {
        return s.trim_matches(|c: char| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c')).to_string();
    }
    let mapped: String = s
        .chars()
        .map(|c| {
            if c.is_ascii() {
                if ucd::isspace(c) { ' ' } else { c }
            } else if let Some(v) = ucd::decimal_value(c) {
                char::from_digit(v, 10).expect("digit")
            } else if ucd::isspace(c) {
                ' '
            } else {
                '?'
            }
        })
        .collect();
    mapped.trim_matches(|c: char| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c')).to_string()
}

/// Remove underscores, which are valid only between two digits.
fn strip_underscores(text: &str) -> Option<String> {
    if !text.contains('_') {
        return Some(text.to_string());
    }
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'_' {
            let before = i > 0 && bytes[i - 1].is_ascii_digit();
            let after = i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit();
            if !(before && after) {
                return None;
            }
        } else {
            out.push(char::from(b));
        }
    }
    Some(out)
}

/// `int(text)` in base 10: spaces around, a sign, `_` between digits, digits of any script.
/// `None` where Python raises ValueError, and also beyond `i128` (the one place this differs: Python has
/// unbounded integers; the program's integers are small).
pub fn py_int(s: &str) -> Option<i128> {
    let text = strip_underscores(&to_ascii_number_text(s))?;
    let (negative, digits) = match text.as_bytes().first()? {
        b'-' => (true, &text[1..]),
        b'+' => (false, &text[1..]),
        _ => (false, &text[..]),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // CPython refuses more than 4300 digit characters, leading zeros included.
    let significant = digits.trim_start_matches('0');
    if digits.len() > 4300 {
        return None;
    }
    let magnitude: u128 = if significant.is_empty() { 0 } else { significant.parse().ok()? };
    if negative {
        if magnitude > 1u128 << 127 { None } else { Some(magnitude.to::<i128>().wrapping_neg()) }
    } else {
        i128::try_from(magnitude).ok()
    }
}

/// `float(text)`: spaces around, a sign, `inf`/`infinity`/`nan` in any case, `_` between digits, digits of
/// any script, exponent. `None` where Python raises ValueError.
pub fn py_float(s: &str) -> Option<f64> {
    let text = strip_underscores(&to_ascii_number_text(s))?;
    let (negative, body) = match text.as_bytes().first()? {
        b'-' => (true, &text[1..]),
        b'+' => (false, &text[1..]),
        _ => (false, &text[..]),
    };
    let lower = body.to_ascii_lowercase();
    let value = match lower.as_str() {
        "inf" | "infinity" => f64::INFINITY,
        "nan" => f64::NAN,
        _ => {
            // digits [. digits] [e [sign] digits], at least one digit in the mantissa.
            let b = lower.as_bytes();
            let mut i = 0;
            let mut mantissa_digits = 0;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
                mantissa_digits += 1;
            }
            if i < b.len() && b[i] == b'.' {
                i += 1;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                    mantissa_digits += 1;
                }
            }
            if mantissa_digits == 0 {
                return None;
            }
            if i < b.len() && b[i] == b'e' {
                i += 1;
                if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
                    i += 1;
                }
                let start = i;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
                if i == start {
                    return None;
                }
            }
            if i != b.len() {
                return None;
            }
            lower.parse::<f64>().ok()?
        }
    };
    Some(if negative { -value } else { value })
}

// ---------------------------------------------------------------- round

/// `round(x)` as a float: halves go to the even neighbour (`0.5 -> 0`, `2.5 -> 2`, `-0.5 -> 0`; never -0.0, the
/// Python result is an `int`).
/// CPython's recipe: round half away, then fix up the exact halves.
pub fn py_round(x: f64) -> f64 {
    let rounded = x.round();
    let rounded = if (x - rounded).abs() == 0.5 { 2.0 * (x / 2.0).round() } else { rounded };
    // Python's result is an int: there is no negative zero (`round(-0.3)` is `0`).
    if rounded == 0.0 { 0.0 } else { rounded }
}

/// `round(x)` as an `int`: `None` for inf and nan (Python raises), and beyond `i128`.
pub fn py_round_to_int(x: f64) -> Option<i128> {
    if !x.is_finite() {
        return None;
    }
    let r = py_round(x);
    if r.abs() >= 1.7e38 { None } else { Some(r.to::<i128>()) }
}

/// `round(x, ndigits)`: correctly rounded through the exact decimal value (halves to even, on the exact
/// binary value), then parsed back, as CPython does. `None` where Python raises OverflowError.
pub fn py_round_ndigits(x: f64, ndigits: i64) -> Option<f64> {
    if !x.is_finite() {
        return Some(x);
    }
    // NDIGITS_MAX = 323, NDIGITS_MIN = -308 for doubles.
    if ndigits > 323 {
        return Some(x);
    }
    if ndigits < -308 {
        return Some(0.0 * x);
    }
    if x == 0.0 {
        return Some(x);
    }
    let rounded = if ndigits >= 0 {
        format!("{:.*}", ndigits.to::<usize>(), x).parse::<f64>().ok()?
    } else {
        round_to_tens(x, (-ndigits).to::<usize>()).parse::<f64>().ok()?
    };
    if rounded.is_infinite() { None } else { Some(rounded) }
}

/// Round `x` to a multiple of 10^k (halves to even on the exact value); returns decimal text.
fn round_to_tens(x: f64, k: usize) -> String {
    let negative = x.is_sign_negative();
    let a = x.abs();
    let whole = format!("{:.0}", a.trunc());
    let has_fraction = a.fract() != 0.0;
    let digits = whole.as_bytes();
    let (head, tail) = if digits.len() > k { digits.split_at(digits.len() - k) } else { (&digits[..0], digits) };
    // Compare the tail (padded to k digits) with half of 10^k.
    let mut padded = vec![b'0'; k - tail.len()];
    padded.extend_from_slice(tail);
    let mut half = vec![b'0'; k];
    half[0] = b'5';
    let round_up = match padded.cmp(&half) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Equal => has_fraction || head.last().is_some_and(|d| (d - b'0') % 2 == 1),
        std::cmp::Ordering::Less => false,
    };
    // head as a decimal number, plus one when rounding up.
    let mut head_digits: Vec<u8> = head.to_vec();
    if round_up {
        let mut i = head_digits.len();
        loop {
            if i == 0 {
                head_digits.insert(0, b'1');
                break;
            }
            i -= 1;
            if head_digits[i] == b'9' {
                head_digits[i] = b'0';
            } else {
                head_digits[i] += 1;
                break;
            }
        }
    }
    let mut text = String::new();
    if negative {
        text.push('-');
    }
    if head_digits.is_empty() {
        text.push('0');
    } else {
        text.push_str(std::str::from_utf8(&head_digits).expect("ascii"));
        text.push_str(&"0".repeat(k));
    }
    text
}

// ---------------------------------------------------------------- fsum, fmean, pstdev

/// The errors `math.fsum` raises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsumError {
    /// OverflowError: intermediate overflow in fsum.
    Overflow,
    /// ValueError: -inf + inf in fsum.
    InfMinusInf,
}

/// `math.fsum`: Shewchuk's exactly rounded sum, a port of CPython's `math_fsum`.
pub fn fsum(values: &[f64]) -> Result<f64, FsumError> {
    let mut partials: Vec<f64> = Vec::new();
    let mut special_sum = 0.0f64;
    let mut inf_sum = 0.0f64;
    for &xsave in values {
        let mut x = xsave;
        let mut i = 0;
        for j in 0..partials.len() {
            let mut y = partials[j];
            if x.abs() < y.abs() {
                std::mem::swap(&mut x, &mut y);
            }
            let hi = x + y;
            let yr = hi - x;
            let lo = y - yr;
            if lo != 0.0 {
                partials[i] = lo;
                i += 1;
            }
            x = hi;
        }
        partials.truncate(i);
        if x != 0.0 {
            if !x.is_finite() {
                // A non-finite x is an intermediate overflow or an inf/nan in the summands.
                if xsave.is_finite() {
                    return Err(FsumError::Overflow);
                }
                if xsave.is_infinite() {
                    inf_sum += xsave;
                }
                special_sum += xsave;
                partials.clear();
            } else {
                partials.push(x);
            }
        }
    }
    if special_sum != 0.0 {
        return if inf_sum.is_nan() { Err(FsumError::InfMinusInf) } else { Ok(special_sum) };
    }
    let mut hi = 0.0f64;
    let mut n = partials.len();
    if n > 0 {
        n -= 1;
        hi = partials[n];
        let mut lo = 0.0f64;
        // Sum exactly from the top, stop when the sum becomes inexact.
        while n > 0 {
            let x = hi;
            n -= 1;
            let y = partials[n];
            hi = x + y;
            let yr = hi - x;
            lo = y - yr;
            if lo != 0.0 {
                break;
            }
        }
        // Make half-even rounding work across multiple partials.
        if n > 0 && ((lo < 0.0 && partials[n - 1] < 0.0) || (lo > 0.0 && partials[n - 1] > 0.0)) {
            let y = lo * 2.0;
            let x = hi + y;
            let yr = x - hi;
            if y == yr {
                hi = x;
            }
        }
    }
    Ok(hi)
}

/// `statistics.fmean(data)`: `fsum(data) / len(data)`; `None` for empty data or where `fsum` raises.
pub fn fmean(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    Some(fsum(values).ok()? / values.len().to::<f64>())
}

/// Split a finite double into `(negative, mantissa, exponent)` with `|x| = mantissa * 2^exponent`.
fn decompose(x: f64) -> (bool, u64, i32) {
    let bits = x.to_bits();
    let negative = bits >> 63 == 1;
    let exp_bits = ((bits >> 52) & 0x7FF).to::<i32>();
    let frac = bits & ((1u64 << 52) - 1);
    if exp_bits == 0 { (negative, frac, -1074) } else { (negative, frac | (1 << 52), exp_bits - 1075) }
}

/// `statistics._float_sqrt_of_frac(n, m)`: the square root of n/m, correctly rounded.
fn float_sqrt_of_frac(n: &BigUint, m: &BigUint) -> f64 {
    const SQRT_BIT_WIDTH: i64 = 2 * 53 + 3;
    let q = (n.bit_length().bits::<i64>() - m.bit_length().bits::<i64>() - SQRT_BIT_WIDTH).div_euclid(2);
    // Round-to-odd integer square root of n/m (with the scale folded into n or m).
    let (nn, mm) = if q >= 0 { (n.clone(), m.shl(2 * q.bits::<u64>())) } else { (n.shl((-2 * q).bits::<u64>()), m.clone()) };
    let (quotient, _) = nn.divrem(&mm);
    let a = quotient.isqrt();
    let inexact = a.mul(&a).mul(&mm) != nn;
    let a = if inexact && a.rem_small(2) == 0 { a.add(&BigUint::from_u64(1)) } else { a };
    a.to_f64_scaled(q)
}

/// `statistics.pstdev(data)`: exact sums of squares as fractions and a correctly rounded square root.
/// `None` for empty data and for inf/nan in the data (Python raises).
pub fn pstdev(values: &[f64]) -> Option<f64> {
    let n = values.len();
    if n == 0 || values.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let parts: Vec<(bool, u64, i32)> = values.iter().map(|&v| decompose(v)).collect();
    let Some(emin) = parts.iter().filter(|p| p.1 != 0).map(|p| p.2).min() else {
        return Some(0.0);
    };
    // Every value is X * 2^emin for an integer X; sums are exact in that scale.
    let (mut positive, mut negative) = (BigUint::zero(), BigUint::zero());
    let mut squares = BigUint::zero();
    for &(neg, mant, exp) in &parts {
        if mant == 0 {
            continue;
        }
        let x = BigUint::from_u64(mant).shl((exp - emin).to::<u64>());
        squares = squares.add(&x.mul(&x));
        if neg {
            negative = negative.add(&x);
        } else {
            positive = positive.add(&x);
        }
    }
    let sum_abs = if positive >= negative { positive.sub(&negative) } else { negative.sub(&positive) };
    let count = BigUint::from_u64(n.to::<u64>());
    // mean square deviation = (n * sum(X^2) - sum(X)^2) / n^2 * 2^(2 emin)
    let numerator = count.mul(&squares).sub(&sum_abs.mul(&sum_abs));
    if numerator.is_zero() {
        return Some(0.0);
    }
    let denominator = count.mul(&count);
    let scale = 2 * i64::from(emin);
    let (numerator, denominator) = if scale >= 0 {
        (numerator.shl(scale.bits::<u64>()), denominator)
    } else {
        (numerator, denominator.shl((-scale).bits::<u64>()))
    };
    Some(float_sqrt_of_frac(&numerator, &denominator))
}

// ---------------------------------------------------------------- math

/// `math.log2(x)`: the platform libm. `None` where Python raises ValueError (x <= 0).
pub fn log2(x: f64) -> Option<f64> {
    if x.is_nan() {
        return Some(x);
    }
    if x <= 0.0 { None } else { Some(x.log2()) }
}

/// `math.log(x)`: the platform libm. `None` where Python raises ValueError (x <= 0).
pub fn log(x: f64) -> Option<f64> {
    if x.is_nan() {
        return Some(x);
    }
    if x <= 0.0 { None } else { Some(x.ln()) }
}

/// `math.log(x, base)`: `log(x) / log(base)`; `None` for a domain error or a base of 1.
pub fn log_base(x: f64, base: f64) -> Option<f64> {
    let (num, den) = (log(x)?, log(base)?);
    if den == 0.0 { None } else { Some(num / den) }
}

/// `math.exp(x)`: the platform libm. `None` where Python raises OverflowError (finite x, infinite result).
pub fn exp(x: f64) -> Option<f64> {
    let r = x.exp();
    if r.is_infinite() && x.is_finite() { None } else { Some(r) }
}

/// `math.sqrt(x)`: `None` for negative x (ValueError).
pub fn sqrt(x: f64) -> Option<f64> {
    if x < 0.0 { None } else { Some(x.sqrt()) }
}

// ---------------------------------------------------------------- max / min with first-of-equals

/// `max(items, key=key)`: the FIRST of the maximal elements (`Iterator::max_by_key` returns the last).
/// A later element replaces the best only when its key is strictly greater (Python's `>`).
#[allow(clippy::neg_cmp_op_on_partial_ord)] // Python keeps the first unless `>` holds; NaN keys never replace
pub fn max_by_key_first<I, K, F>(items: I, mut key: F) -> Option<I::Item>
where
    I: IntoIterator,
    K: PartialOrd,
    F: FnMut(&I::Item) -> K,
{
    let mut best: Option<(I::Item, K)> = None;
    for item in items {
        let k = key(&item);
        match &best {
            Some((_, bk)) if !(k > *bk) => {}
            _ => best = Some((item, k)),
        }
    }
    best.map(|b| b.0)
}

/// `min(items, key=key)`: the first of the minimal elements.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
pub fn min_by_key_first<I, K, F>(items: I, mut key: F) -> Option<I::Item>
where
    I: IntoIterator,
    K: PartialOrd,
    F: FnMut(&I::Item) -> K,
{
    let mut best: Option<(I::Item, K)> = None;
    for item in items {
        let k = key(&item);
        match &best {
            Some((_, bk)) if !(k < *bk) => {}
            _ => best = Some((item, k)),
        }
    }
    best.map(|b| b.0)
}

/// `max(items)` without a key: the first maximal element.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
pub fn max_first<I>(items: I) -> Option<I::Item>
where
    I: IntoIterator,
    I::Item: PartialOrd,
{
    let mut best: Option<I::Item> = None;
    for item in items {
        match &best {
            Some(b) if !(item > *b) => {}
            _ => best = Some(item),
        }
    }
    best
}

/// `min(items)` without a key: the first minimal element.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
pub fn min_first<I>(items: I) -> Option<I::Item>
where
    I: IntoIterator,
    I::Item: PartialOrd,
{
    let mut best: Option<I::Item> = None;
    for item in items {
        match &best {
            Some(b) if !(item < *b) => {}
            _ => best = Some(item),
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repr_layout() {
        assert_eq!(float_repr(0.1), "0.1");
        assert_eq!(float_repr(1e16), "1e+16");
        assert_eq!(float_repr(1e15), "1000000000000000.0");
        assert_eq!(float_repr(1.5e-7), "1.5e-07");
        assert_eq!(float_repr(0.0001), "0.0001");
        assert_eq!(float_repr(0.00001), "1e-05");
        assert_eq!(float_repr(-0.0), "-0.0");
        assert_eq!(float_repr(123456.789), "123456.789");
        assert_eq!(float_repr(1.7976931348623157e308), "1.7976931348623157e+308");
        assert_eq!(float_repr(5e-324), "5e-324");
        assert_eq!(float_repr(f64::NAN), "nan");
        assert_eq!(float_repr(f64::NEG_INFINITY), "-inf");
        assert_eq!(float_repr(2.5), "2.5");
        assert_eq!(float_repr(100.0), "100.0");
    }

    #[test]
    fn rounding() {
        assert_eq!(py_round(0.5), 0.0);
        assert_eq!(py_round(1.5), 2.0);
        assert_eq!(py_round(2.5), 2.0);
        assert_eq!(py_round(-0.5).to_bits(), 0.0f64.to_bits());
        assert_eq!(py_round(-2.5), -2.0);
        assert_eq!(py_round(0.49999999999999994), 0.0);
        assert_eq!(py_round_to_int(2.5), Some(2));
        assert_eq!(py_round_to_int(f64::NAN), None);
        assert_eq!(py_round_ndigits(2.675, 2), Some(2.67));
        assert_eq!(py_round_ndigits(0.125, 2), Some(0.12));
        assert_eq!(py_round_ndigits(0.375, 2), Some(0.38));
        assert_eq!(py_round_ndigits(1234.5, -2), Some(1200.0));
        assert_eq!(py_round_ndigits(1250.0, -2), Some(1200.0));
        assert_eq!(py_round_ndigits(1350.0, -2), Some(1400.0));
        assert_eq!(py_round_ndigits(1250.5, -2), Some(1300.0));
        assert_eq!(py_round_ndigits(4.0, -2), Some(0.0));
        assert_eq!(py_round_ndigits(-0.4, 0).unwrap().to_bits(), (-0.0f64).to_bits());
        assert_eq!(py_round_ndigits(1e308, -308), Some(1e308));
    }

    #[test]
    fn parsing() {
        assert_eq!(py_int(" 42 "), Some(42));
        assert_eq!(py_int("-1_000"), Some(-1000));
        assert_eq!(py_int("١٢٣"), Some(123));
        assert_eq!(py_int("1__0"), None);
        assert_eq!(py_int("_1"), None);
        assert_eq!(py_int("1_"), None);
        assert_eq!(py_int("- 1"), None);
        assert_eq!(py_int(""), None);
        assert_eq!(py_int("1.0"), None);
        assert_eq!(py_int("007"), Some(7));
        assert_eq!(py_int("\u{a0}5\u{2003}"), Some(5));
        assert_eq!(py_float("  1e3 "), Some(1000.0));
        assert_eq!(py_float(".5"), Some(0.5));
        assert_eq!(py_float("5."), Some(5.0));
        assert_eq!(py_float("1_0.5"), Some(10.5));
        assert_eq!(py_float("-Infinity"), Some(f64::NEG_INFINITY));
        assert!(py_float("nan").unwrap().is_nan());
        assert_eq!(py_float("e5"), None);
        assert_eq!(py_float("."), None);
        assert_eq!(py_float("1e"), None);
        assert_eq!(py_float("0x10"), None);
        assert_eq!(py_float("１２.５"), Some(12.5));
        assert_eq!(py_float("1e999"), Some(f64::INFINITY));
    }

    #[test]
    fn sums_and_statistics() {
        assert_eq!(fsum(&[0.1; 10]), Ok(1.0));
        assert_eq!(fsum(&[1e100, 1.0, -1e100]), Ok(1.0));
        assert_eq!(fsum(&[f64::INFINITY, f64::NEG_INFINITY]), Err(FsumError::InfMinusInf));
        assert_eq!(fsum(&[f64::MAX, f64::MAX]), Err(FsumError::Overflow));
        assert_eq!(fsum(&[]), Ok(0.0));
        assert_eq!(fmean(&[3.5, 4.0, 5.25]), Some(4.25));
        assert_eq!(fmean(&[]), None);
        assert_eq!(pstdev(&[1.5, 2.5, 2.5, 2.75, 3.25, 4.75]), Some(0.986893273527251));
        assert_eq!(pstdev(&[2.0]), Some(0.0));
        assert_eq!(pstdev(&[]), None);
        assert_eq!(pstdev(&[1.0, f64::NAN]), None);
        assert_eq!(pstdev(&[1.0, 3.0]), Some(1.0));
    }

    #[test]
    fn first_of_equals() {
        let v = [(1, 'a'), (3, 'b'), (3, 'c'), (0, 'd'), (0, 'e')];
        assert_eq!(max_by_key_first(v, |p| p.0), Some((3, 'b')));
        assert_eq!(min_by_key_first(v, |p| p.0), Some((0, 'd')));
        assert_eq!(max_first([1.0, f64::NAN, 2.0]), Some(2.0));
        assert_eq!(max_by_key_first(Vec::<i32>::new(), |x| *x), None);
    }

    #[test]
    fn math_wrappers() {
        assert_eq!(log(0.0), None);
        assert_eq!(log2(8.0), Some(3.0));
        assert_eq!(exp(1000.0), None);
        assert_eq!(exp(0.0), Some(1.0));
        assert_eq!(sqrt(-1.0), None);
        assert_eq!(log_base(8.0, 2.0).map(|v| (v * 1e12).round()), Some(3e12));
    }
}

/// The inverse of `float.hex()`: exactly the forms it writes, nothing else.
pub fn float_fromhex(text: &str) -> Option<f64> {
    let (negative, body) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let magnitude = match body {
        "inf" => f64::INFINITY,
        "nan" => f64::NAN,
        "0x0.0p+0" => 0.0,
        _ => {
            let (digits, exponent) = body.strip_prefix("0x")?.split_once('p')?;
            let (lead, fraction) = digits.split_once('.')?;
            let exponent: i64 = exponent.parse().ok()?;
            if fraction.len() != 13 {
                return None;
            }
            let fraction = u64::from_str_radix(fraction, 16).ok()?;
            let bits = match lead {
                "1" => (u64::try_from(exponent + 1023).ok()? << 52) | fraction,
                "0" => fraction,
                _ => return None,
            };
            f64::from_bits(bits)
        }
    };
    Some(if negative { -magnitude } else { magnitude })
}

/// `float.hex()`.
pub fn float_hex(x: f64) -> String {
    if x.is_nan() {
        return "nan".to_owned();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf".to_owned() } else { "-inf".to_owned() };
    }
    let bits = x.to_bits();
    let sign = if bits >> 63 == 1 { "-" } else { "" };
    let exponent = ((bits >> 52) & 0x7ff).bits::<i64>();
    let fraction = bits & ((1_u64 << 52) - 1);
    if exponent == 0 && fraction == 0 {
        return format!("{sign}0x0.0p+0");
    }
    let (lead, power) = if exponent == 0 { (0, -1022) } else { (1, exponent - 1023) };
    format!("{sign}0x{lead}.{fraction:013x}p{power:+}")
}

#[cfg(test)]
mod float_hex_tests {
    use super::float_hex;

    #[test]
    fn as_python_writes_it() {
        assert_eq!(float_hex(1.0), "0x1.0000000000000p+0");
        assert_eq!(float_hex(-0.1), "-0x1.999999999999ap-4");
        assert_eq!(float_hex(0.0), "0x0.0p+0");
        assert_eq!(float_hex(5e-324), "0x0.0000000000001p-1022");
    }
}
