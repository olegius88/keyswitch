//! A small unsigned big integer: just what the exact `statistics.pstdev` and the correctly rounded
//! conversions need (add, subtract, multiply, shifts, division with remainder, integer square root).
//! Little-endian 32-bit limbs, no leading zero limbs. Speed is not a goal: the numbers are a few
//! hundred bits and the operations run once per model load.

use ks_cast::To as _;
use std::cmp::Ordering;

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(crate) struct BigUint {
    limbs: Vec<u32>,
}

impl BigUint {
    pub(crate) fn zero() -> Self {
        BigUint { limbs: Vec::new() }
    }

    pub(crate) fn from_u64(v: u64) -> Self {
        let mut b = BigUint { limbs: vec![v.low::<u32>(), (v >> 32).to::<u32>()] };
        b.trim();
        b
    }

    fn trim(&mut self) {
        while self.limbs.last() == Some(&0) {
            self.limbs.pop();
        }
    }

    pub(crate) fn is_zero(&self) -> bool {
        self.limbs.is_empty()
    }

    /// `int.bit_length()`.
    pub(crate) fn bit_length(&self) -> u64 {
        match self.limbs.last() {
            None => 0,
            Some(&top) => (self.limbs.len().to::<u64>() - 1) * 32 + u64::from(32 - top.leading_zeros()),
        }
    }

    fn bit(&self, i: u64) -> bool {
        let limb = (i / 32).to::<usize>();
        limb < self.limbs.len() && (self.limbs[limb] >> (i % 32)) & 1 == 1
    }

    pub(crate) fn shl(&self, n: u64) -> Self {
        if self.is_zero() {
            return Self::zero();
        }
        let (words, bits) = ((n / 32).to::<usize>(), (n % 32).to::<u32>());
        let mut limbs = vec![0u32; words];
        if bits == 0 {
            limbs.extend_from_slice(&self.limbs);
        } else {
            let mut carry = 0u32;
            for &l in &self.limbs {
                limbs.push((l << bits) | carry);
                carry = l >> (32 - bits);
            }
            limbs.push(carry);
        }
        let mut b = BigUint { limbs };
        b.trim();
        b
    }

    pub(crate) fn shr(&self, n: u64) -> Self {
        let (words, bits) = ((n / 32).to::<usize>(), (n % 32).to::<u32>());
        if words >= self.limbs.len() {
            return Self::zero();
        }
        let src = &self.limbs[words..];
        let mut limbs = Vec::with_capacity(src.len());
        for (i, &l) in src.iter().enumerate() {
            let hi = src.get(i + 1).copied().unwrap_or(0);
            limbs.push(if bits == 0 { l } else { (l >> bits) | (hi << (32 - bits)) });
        }
        let mut b = BigUint { limbs };
        b.trim();
        b
    }

    pub(crate) fn add(&self, other: &Self) -> Self {
        let n = self.limbs.len().max(other.limbs.len());
        let mut limbs = Vec::with_capacity(n + 1);
        let mut carry = 0u64;
        for i in 0..n {
            let s = u64::from(*self.limbs.get(i).unwrap_or(&0)) + u64::from(*other.limbs.get(i).unwrap_or(&0)) + carry;
            limbs.push(s.low::<u32>());
            carry = s >> 32;
        }
        if carry != 0 {
            limbs.push(carry.low::<u32>());
        }
        BigUint { limbs }
    }

    /// `self - other`; requires `self >= other`.
    pub(crate) fn sub(&self, other: &Self) -> Self {
        assert!(self.cmp(other) != Ordering::Less, "BigUint subtraction underflow");
        let mut limbs = Vec::with_capacity(self.limbs.len());
        let mut borrow = 0i64;
        for i in 0..self.limbs.len() {
            let mut d = i64::from(self.limbs[i]) - i64::from(*other.limbs.get(i).unwrap_or(&0)) - borrow;
            if d < 0 {
                d += 1 << 32;
                borrow = 1;
            } else {
                borrow = 0;
            }
            limbs.push(d.low::<u32>());
        }
        let mut b = BigUint { limbs };
        b.trim();
        b
    }

    pub(crate) fn mul(&self, other: &Self) -> Self {
        if self.is_zero() || other.is_zero() {
            return Self::zero();
        }
        let mut limbs = vec![0u32; self.limbs.len() + other.limbs.len()];
        for (i, &a) in self.limbs.iter().enumerate() {
            let mut carry = 0u64;
            for (j, &b) in other.limbs.iter().enumerate() {
                let cur = u64::from(limbs[i + j]) + u64::from(a) * u64::from(b) + carry;
                limbs[i + j] = cur.low::<u32>();
                carry = cur >> 32;
            }
            limbs[i + other.limbs.len()] = carry.low::<u32>();
        }
        let mut b = BigUint { limbs };
        b.trim();
        b
    }

    /// Quotient and remainder by shift-and-subtract; `divisor` must not be zero.
    pub(crate) fn divrem(&self, divisor: &Self) -> (Self, Self) {
        assert!(!divisor.is_zero(), "division by zero");
        if self.cmp(divisor) == Ordering::Less {
            return (Self::zero(), self.clone());
        }
        let bits = self.bit_length();
        let mut quotient = BigUint { limbs: vec![0; self.limbs.len()] };
        let mut rem = Self::zero();
        for i in (0..bits).rev() {
            rem = rem.shl(1);
            if self.bit(i) {
                if rem.limbs.is_empty() {
                    rem.limbs.push(1);
                } else {
                    rem.limbs[0] |= 1;
                }
            }
            if rem.cmp(divisor) != Ordering::Less {
                rem = rem.sub(divisor);
                quotient.limbs[(i / 32).to::<usize>()] |= 1 << (i % 32);
            }
        }
        quotient.trim();
        (quotient, rem)
    }

    /// Remainder by a small number.
    pub(crate) fn rem_small(&self, m: u32) -> u32 {
        let mut r = 0u64;
        for &l in self.limbs.iter().rev() {
            r = ((r << 32) | u64::from(l)) % u64::from(m);
        }
        r.low::<u32>()
    }

    /// `math.isqrt`: the floor of the square root (Newton iteration from above).
    pub(crate) fn isqrt(&self) -> Self {
        if self.is_zero() {
            return Self::zero();
        }
        let mut x = Self::from_u64(1).shl(self.bit_length().div_ceil(2));
        loop {
            let (q, _) = self.divrem(&x);
            let y = x.add(&q).shr(1);
            if y.cmp(&x) != Ordering::Less {
                return x;
            }
            x = y;
        }
    }

    /// The correctly rounded (half to even) double nearest to `self * 2^exp2`, subnormals and overflow
    /// to infinity included: what `int / int` does in Python when the divisor is a power of two.
    pub(crate) fn to_f64_scaled(&self, exp2: i64) -> f64 {
        if self.is_zero() {
            return 0.0;
        }
        let bl = self.bit_length().bits::<i64>();
        let top = bl - 1 + exp2; // exponent of the leading bit
        if top > 1023 {
            return f64::INFINITY;
        }
        // Precision available at this magnitude: 53 bits, fewer in the subnormal range.
        let precision = if top >= -1022 { 53 } else { 53 - (-1022 - top) };
        if precision < 0 {
            return 0.0;
        }
        let shift = bl - precision;
        let (mant, exp) = if shift > 0 {
            let q = self.shr(shift.bits::<u64>());
            let rest = self.sub(&q.shl(shift.bits::<u64>()));
            let half = Self::from_u64(1).shl(shift.bits::<u64>() - 1);
            let mut q = q.to_u64();
            match rest.cmp(&half) {
                Ordering::Greater => q += 1,
                Ordering::Equal if q & 1 == 1 => q += 1,
                Ordering::Less | Ordering::Equal => {}
            }
            (q, exp2 + shift)
        } else {
            (self.to_u64() << (-shift).low::<u32>(), exp2 + shift)
        };
        if mant == 0 {
            return 0.0;
        }
        // mant has at most 54 bits; scaling by a power of two is exact unless the result overflows.
        let mut value = mant.to::<f64>();
        let mut e = exp;
        while e > 0 {
            let step = e.min(1000);
            value *= f64::from_bits((1023 + step).bits::<u64>() << 52);
            e -= step;
        }
        while e < 0 {
            let step = (-e).min(1000);
            value *= f64::from_bits((1023 - step).bits::<u64>() << 52);
            e += step;
        }
        value
    }

    fn to_u64(&self) -> u64 {
        assert!(self.limbs.len() <= 2);
        u64::from(*self.limbs.first().unwrap_or(&0)) | (u64::from(*self.limbs.get(1).unwrap_or(&0)) << 32)
    }
}

impl PartialOrd for BigUint {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BigUint {
    fn cmp(&self, other: &Self) -> Ordering {
        self.limbs.len().cmp(&other.limbs.len()).then_with(|| self.limbs.iter().rev().cmp(other.limbs.iter().rev()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn big(v: u128) -> BigUint {
        BigUint::from_u64((v >> 64).to::<u64>()).shl(64).add(&BigUint::from_u64(v.low::<u64>()))
    }

    #[test]
    fn arithmetic_matches_u128() {
        let (a, b) = (0x1234_5678_9abc_def0_1122_3344u128, 0x9_8765_4321u128);
        assert_eq!(big(a).add(&big(b)), big(a + b));
        assert_eq!(big(a).sub(&big(b)), big(a - b));
        assert_eq!(big(a >> 40).mul(&big(b >> 8)), big((a >> 40) * (b >> 8)));
        let (q, r) = big(a).divrem(&big(b));
        assert_eq!((q, r), (big(a / b), big(a % b)));
        assert_eq!(big(a).shl(7).shr(7), big(a));
        assert_eq!(big(a).bit_length(), 128 - a.leading_zeros().to::<u64>());
        assert_eq!(big(a).rem_small(97), (a % 97).to::<u32>());
        for v in [0u128, 1, 2, 3, 15, 16, 17, 1 << 100, (1 << 100) - 1, a] {
            let r = big(v).isqrt();
            let n = {
                let mut x = (v.to::<f64>().sqrt()).to::<u128>();
                while x * x > v {
                    x -= 1;
                }
                while (x + 1) * (x + 1) <= v {
                    x += 1;
                }
                x
            };
            assert_eq!(r, big(n), "isqrt({v})");
        }
    }

    #[test]
    fn conversion_to_double_rounds_to_even() {
        assert_eq!(big(1).to_f64_scaled(-1), 0.5);
        assert_eq!(big((1 << 53) + 1).to_f64_scaled(0), 9_007_199_254_740_992.0);
        assert_eq!(big((1 << 53) + 3).to_f64_scaled(0), 9_007_199_254_740_996.0);
        assert_eq!(big(1).to_f64_scaled(-1074), f64::from_bits(1));
        assert_eq!(big(3).to_f64_scaled(-1075), f64::from_bits(2)); // 1.5 * 2^-1074 -> even
        assert_eq!(big(1).to_f64_scaled(-1075), 0.0); // exactly half of the least subnormal -> even (0)
        assert_eq!(big(1).to_f64_scaled(1024), f64::INFINITY);
        assert_eq!(big(1).to_f64_scaled(1023), 2f64.powi(1023));
        assert_eq!(big(1).to_f64_scaled(0), 1.0);
    }
}
