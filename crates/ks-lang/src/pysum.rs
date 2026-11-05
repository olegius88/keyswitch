//! The builtin `sum()` of `CPython` over floats, which is not a plain running addition.
//!
//! Since Python 3.12 `sum` keeps a Neumaier compensation next to the running total (`cs_add` in
//! `Python/bltinmodule.c`); the result differs in the last bits from `a + b + c ...` for about one list of floats in
//! three. `statistics.fmean` and `math.fsum` are a third thing (`ks_text::fsum`). Every Python `sum(...)` over floats
//! goes through here, in the Python's order.

/// `sum(values)` of floats. The empty sum is the integer 0 in Python, which reads as `0.0` in the arithmetic around it.
pub fn py_sum<I: IntoIterator<Item = f64>>(values: I) -> f64 {
    let mut values = values.into_iter();
    let Some(first) = values.next() else {
        return 0.0;
    };
    // The start value is the integer 0, added as a plain number: `0 + -0.0` is `0.0`.
    let mut high = 0.0 + first;
    let mut low = 0.0f64;
    for x in values {
        let total = high + x;
        if high.abs() >= x.abs() {
            low += (high - total) + x;
        } else {
            low += (x - total) + high;
        }
        high = total;
    }
    // A zero compensation keeps the sign of a negative zero; an infinite one must not turn the sum into a NaN.
    if low != 0.0 && low.is_finite() { high + low } else { high }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compensated_like_cpython() {
        // Checked against `sum()` of Python 3.14: the plain running sum of these gives 0.0.
        assert_eq!(py_sum([1.0, 1e100, 1.0, -1e100]), 2.0);
        assert_eq!(py_sum([1e16, 1.0, 1.0]), 1e16 + 2.0);
        assert_eq!(py_sum([]), 0.0);
        assert_eq!(py_sum([-0.0]).to_bits(), 0.0f64.to_bits());
        assert_eq!(py_sum([0.1, 0.2, 0.3]), 0.6);
        assert!(py_sum([f64::INFINITY, -f64::INFINITY]).is_nan());
        assert_eq!(py_sum([f64::INFINITY, 1.0]), f64::INFINITY);
    }
}
