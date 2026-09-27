"""Operands and grids of tools/environment_probe.py.

The probe runs the primitives the intent trainer and runtime depend on over these fixed inputs and
digests the exact answers. A recorded measurement is only comparable with one taken over the same
inputs, so a changed value here moves every recorded digest of its cell.
"""

from __future__ import annotations

from typing import Final

# The smallest positive (subnormal) double.
PROBE_SMALLEST_SUBNORMAL: Final[float] = 5e-324
# Operands chosen to sit on the places doubles misbehave: subnormals, the exponent boundary, values
# whose difference cancels, and ordinary magnitudes from real training. Not a random sample - a list
# of known-awkward inputs.
PROBE_DOUBLES: Final[tuple[float, ...]] = (
    0.0,
    -0.0,
    PROBE_SMALLEST_SUBNORMAL,
    2.2250738585072014e-308,
    1e-300,
    1e-15,
    0.1,
    0.5,
    1.0 - 2**-53,
    1.0,
    1.0 + 2**-52,
    1.5,
    2.0,
    3.0,
    10.0,
    1e15,
    1e15 + 1.0,
    2**53 - 1.0,
    float(2**53),
    1e300,
    1.7976931348623157e308,
)
# An integer's exact image: this many little-endian two's-complement bytes.
PROBE_INTEGER_IMAGE_BYTES: Final[int] = 17
# The FTRL update of the float_arithmetic cell: learning rate alpha and beta, L1 and L2, and the
# stand-in weight as a share of the gradient.
PROBE_FTRL_ALPHA: Final[float] = 0.1
PROBE_FTRL_BETA: Final[float] = 1.0
PROBE_FTRL_L1: Final[float] = 0.5
PROBE_FTRL_L2: Final[float] = 0.01
PROBE_FTRL_WEIGHT_PER_GRADIENT: Final[float] = 0.5
# Sequences of the builtin_sum cell: a magnitude that cancels around a unit, a run of tiny terms
# after a unit, an alternating series drifting by a small step, and a run of subnormals.
PROBE_SUM_CANCELLING_MAGNITUDE: Final[float] = 1e16
PROBE_SUM_SMALL_TERM: Final[float] = 1e-16
PROBE_SUM_SMALL_TERM_COUNT: Final[int] = 64
PROBE_SUM_ALTERNATING_STEP: Final[float] = 1e-13
PROBE_SUM_ALTERNATING_COUNT: Final[int] = 128
# Every second term of the alternating series is positive.
PROBE_SUM_ALTERNATING_PERIOD: Final[int] = 2
PROBE_SUM_SUBNORMAL_COUNT: Final[int] = 32
# Decimal places each probe double is rounded to by the float_text cell.
PROBE_ROUND_DIGITS: Final[tuple[int, ...]] = (0, 1, 6, 15)
# The ordering cell sorts this many (index modulo the key modulus, index) pairs by their key.
PROBE_SORT_PAIR_COUNT: Final[int] = 64
PROBE_SORT_KEY_MODULUS: Final[int] = 5
# Operands of the integers cell: both signs around the int64 and 2**31 boundaries and beyond 64 bits.
PROBE_INTEGERS: Final[tuple[int, ...]] = (-(2**70), -(2**63), -7, -1, 0, 1, 7, 2**31, 2**63 - 1, 2**70)
PROBE_SHIFT_BITS: Final[int] = 3
# pow(base, exponent, modulus) of the integers cell; the modulus is the Mersenne prime 2**61 - 1.
PROBE_POW_BASE: Final[int] = 3
PROBE_POW_EXPONENT: Final[int] = 2**20
PROBE_POW_MODULUS: Final[int] = 2**61 - 1
# Doubles the integers cell rounds and truncates: ties to even on both signs, and a tie beyond the
# fractional precision of large doubles.
PROBE_ROUNDING_DOUBLES: Final[tuple[float, ...]] = (0.0, 0.5, 1.5, 2.5, -0.5, -1.5, 1e15 + 0.5)
# The random_stream cell seeds random.Random(seed + epoch) for each pair.
PROBE_RANDOM_SEEDS: Final[tuple[int, ...]] = (0, 1, 20260909, 2**31 - 1)
PROBE_RANDOM_EPOCHS: Final[tuple[int, ...]] = (0, 1, 7)
PROBE_SHUFFLE_LENGTH: Final[int] = 64
PROBE_RANDOM_DRAWS: Final[int] = 8
PROBE_RANDOM_BITS: Final[int] = 64
PROBE_SAMPLE_POPULATION: Final[int] = 1000
PROBE_SAMPLE_SIZE: Final[int] = 16
# The unicode cell's entries are bucketed by plane: a code point shifted right by this many bits.
UNICODE_PLANE_BITS: Final[int] = 16
# The 63 magnitude bits of a double's bit pattern (all but the sign).
DOUBLE_MAGNITUDE_MASK: Final[int] = 0x7FFFFFFFFFFFFFFF
# --ulp compares two packed doubles.
PROBE_ULP_OPERANDS: Final[int] = 2
