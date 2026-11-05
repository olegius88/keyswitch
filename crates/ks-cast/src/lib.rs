//! The one place where numbers change type. An `as` hides what happens to a value that does not fit, so the rest of the
//! project may not use it (`clippy::as_conversions`); it says which of these it means instead:
//!
//! - [`To::to`]: the nearest value the target type has. An integer too large or too small for the target becomes the largest
//!   or the smallest one; a float loses its fraction, and `NaN` becomes zero; a large integer becomes the nearest float.
//! - [`To::low`]: the low bits of an integer, for hashes and packed values, where dropping the rest is the point.
//! - [`To::bits`]: the same bits read with the other sign, as a system handle or status is.
#![expect(clippy::as_conversions, reason = "this module is the only place allowed to convert with `as`")]

/// The nearest value of type `T`.
pub trait Nearest<T> {
    /// The nearest value of type `T`.
    fn nearest(self) -> T;
}

impl<T> To for *const T {}
impl<T> To for *mut T {}

/// What a system call takes for a pointer it hands back later: the address as a number, the pointer stays valid for it.
macro_rules! pointer_to_address {
    ($($pointer:ty),+) => {$(
        impl<T> Nearest<usize> for $pointer {
            #[inline]
            fn nearest(self) -> usize {
                self.expose_provenance()
            }
        }
        impl<T> Nearest<isize> for $pointer {
            #[inline]
            fn nearest(self) -> isize {
                self.expose_provenance().cast_signed()
            }
        }
    )+};
}

pointer_to_address!(*const T, *mut T);

/// The low bits of an integer.
pub trait LowBits<T> {
    /// The low bits of this value as a `T`; the rest is dropped on purpose.
    fn low_bits(self) -> T;
}

/// The same bits with the other sign.
pub trait SameBits<T> {
    /// The same bits as a `T`.
    fn same_bits(self) -> T;
}

/// What a call site writes: `count.to::<f64>()`, `hash.low::<u32>()`, `status.bits::<u32>()`.
pub trait To: Sized {
    /// The nearest value of type `T`.
    #[inline]
    #[must_use]
    fn to<T>(self) -> T
    where
        Self: Nearest<T>,
    {
        self.nearest()
    }

    /// The low bits of this value as a `T`.
    #[inline]
    #[must_use]
    fn low<T>(self) -> T
    where
        Self: LowBits<T>,
    {
        self.low_bits()
    }

    /// The same bits as a `T`.
    #[inline]
    #[must_use]
    fn bits<T>(self) -> T
    where
        Self: SameBits<T>,
    {
        self.same_bits()
    }
}

macro_rules! numbers {
    ($($number:ty),+) => {$(impl To for $number {})+};
}

numbers!(u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize, f32, f64, bool, char);

macro_rules! integer_to_integer {
    ($($source:ty => $($target:ty),+;)+) => {$($(
        impl Nearest<$target> for $source {
            #[inline]
            fn nearest(self) -> $target {
                <$target>::try_from(self).unwrap_or(if self < <$source>::default() { <$target>::MIN } else { <$target>::MAX })
            }
        }
    )+)+};
}

integer_to_integer! {
    u8 => u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize;
    u16 => u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize;
    u32 => u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize;
    u64 => u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize;
    u128 => u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize;
    usize => u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize;
    i8 => u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize;
    i16 => u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize;
    i32 => u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize;
    i64 => u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize;
    i128 => u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize;
    isize => u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize;
}

/// `as` on floats already means the nearest value: rounding between floats and from integers, truncation toward zero with
/// saturation (and zero for `NaN`) from floats to integers. <https://doc.rust-lang.org/reference/expressions/operator-expr.html#numeric-cast>
macro_rules! through_as {
    ($($source:ty => $($target:ty),+;)+) => {$($(
        impl Nearest<$target> for $source {
            #[inline]
            fn nearest(self) -> $target {
                self as $target
            }
        }
    )+)+};
}

through_as! {
    u8 => f32, f64;
    u16 => f32, f64;
    u32 => f32, f64;
    u64 => f32, f64;
    u128 => f32, f64;
    usize => f32, f64;
    i8 => f32, f64;
    i16 => f32, f64;
    i32 => f32, f64;
    i64 => f32, f64;
    i128 => f32, f64;
    isize => f32, f64;
    f32 => f32, f64, u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize;
    f64 => f32, f64, u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize;
    bool => u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize;
    char => u32, u64, u128, usize, i64, i128;
}

macro_rules! low_bits {
    ($($source:ty => $($target:ty),+;)+) => {$($(
        impl LowBits<$target> for $source {
            #[inline]
            fn low_bits(self) -> $target {
                self as $target
            }
        }
    )+)+};
}

low_bits! {
    u16 => u8, i8;
    u32 => u8, u16, i8, i16;
    u64 => u8, u16, u32, i8, i16, i32;
    u128 => u8, u16, u32, u64, i8, i16, i32, i64;
    usize => u8, u16, u32, i8, i16, i32;
    i16 => u8, i8;
    i32 => u8, u16, i8, i16;
    i64 => u8, u16, u32, i8, i16, i32;
    i128 => u8, u16, u32, u64, i8, i16, i32, i64;
    isize => u8, u16, u32, i8, i16, i32;
}

macro_rules! same_bits {
    ($($left:ty, $right:ty;)+) => {$(
        impl SameBits<$right> for $left {
            #[inline]
            fn same_bits(self) -> $right {
                self as $right
            }
        }
        impl SameBits<$left> for $right {
            #[inline]
            fn same_bits(self) -> $left {
                self as $left
            }
        }
    )+};
}

same_bits! {
    u8, i8;
    u16, i16;
    u32, i32;
    u64, i64;
    u128, i128;
    usize, isize;
}

/// A `usize` constant as an `i64`, where a trait method cannot run. A value that does not fit stops the build.
#[must_use]
pub const fn const_i64(value: usize) -> i64 {
    assert!(value <= i64::MAX as usize);
    value as i64
}

/// A `usize` constant as an `i32`, where a trait method cannot run. A value that does not fit stops the build.
#[must_use]
pub const fn const_i32(value: usize) -> i32 {
    assert!(value <= i32::MAX as usize);
    value as i32
}

/// An `i64` constant as a `usize`, where a trait method cannot run. A value that does not fit stops the build.
#[must_use]
pub const fn const_usize(value: i64) -> usize {
    assert!(value >= 0);
    value as usize
}

#[cfg(test)]
mod tests {
    use super::{To, const_i32, const_i64, const_usize};

    #[test]
    fn a_constant_converts_while_the_program_is_built() {
        const WIDE: i64 = const_i64(5);
        const NARROW: i32 = const_i32(6);
        const SIZE: usize = const_usize(7);
        assert_eq!((WIDE, NARROW, SIZE), (5, 6, 7));
    }

    #[test]
    fn an_integer_that_fits_stays_the_same() {
        assert_eq!(300_usize.to::<u32>(), 300);
        assert_eq!((-5_i64).to::<i32>(), -5);
        assert_eq!(7_u8.to::<i64>(), 7);
    }

    #[test]
    fn an_integer_that_does_not_fit_becomes_the_nearest_end() {
        assert_eq!(300_u32.to::<u8>(), u8::MAX);
        assert_eq!((-1_i64).to::<usize>(), 0);
        assert_eq!(i64::MIN.to::<i32>(), i32::MIN);
        assert_eq!(u64::MAX.to::<i64>(), i64::MAX);
    }

    #[test]
    fn a_float_loses_its_fraction_and_stays_in_range() {
        assert_eq!(2.9_f64.to::<i64>(), 2);
        assert_eq!((-2.9_f64).to::<i64>(), -2);
        assert_eq!((-1.0_f64).to::<usize>(), 0);
        assert_eq!(1e300_f64.to::<i32>(), i32::MAX);
        assert_eq!(f64::NAN.to::<u32>(), 0);
    }

    #[test]
    fn an_integer_becomes_the_nearest_float() {
        assert_eq!(3_usize.to::<f64>().to_bits(), 3.0_f64.to_bits());
        assert_eq!(9_007_199_254_740_993_i64.to::<f64>().to_bits(), 9_007_199_254_740_992.0_f64.to_bits());
        assert_eq!(0.1_f64.to::<f32>().to_bits(), 0.1_f32.to_bits());
    }

    #[test]
    fn truth_and_characters_are_numbers() {
        assert_eq!(true.to::<u32>(), 1);
        assert_eq!(false.to::<i64>(), 0);
        assert_eq!('я'.to::<u32>(), 1103);
    }

    #[test]
    fn a_pointer_becomes_its_address() {
        let value = 7_u8;
        let pointer = std::ptr::from_ref(&value);
        assert_eq!(pointer.to::<usize>(), pointer.expose_provenance());
        assert_eq!(pointer.cast_mut().to::<isize>(), pointer.expose_provenance().cast_signed());
    }

    #[test]
    fn low_keeps_only_the_low_bits() {
        assert_eq!(0x1_2345_6789_u64.low::<u32>(), 0x2345_6789);
        assert_eq!(0x1ff_u32.low::<u8>(), 0xff);
    }

    #[test]
    fn bits_keeps_every_bit() {
        assert_eq!(0xffff_ffff_u32.bits::<i32>(), -1);
        assert_eq!((-1_i32).bits::<u32>(), 0xffff_ffff);
    }
}
