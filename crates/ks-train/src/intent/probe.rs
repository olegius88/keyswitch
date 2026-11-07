//! What this machine computes, not what it calls itself (`environment_probe`): the primitives the trainer and the runtime
//! depend on, each answer as its exact bit image, reduced to a digest. It is evidence and not proof: it never votes on the
//! identity of a model, it explains a divergence. Only a full replay settles that.
use ks_cast::To as _;
use super::fit::Twister;
use crate::physical::hex;
use ks_text::pynum::{float_hex, float_repr, fsum, py_round_ndigits};
use ks_text::{normalize, pystr};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const SCHEMA_VERSION: i64 = 1;
const FTRL_ALPHA: f64 = 0.1;
const FTRL_BETA: f64 = 1.0;
const FTRL_L1: f64 = 0.5;
const FTRL_L2: f64 = 0.01;
const FTRL_WEIGHT_PER_GRADIENT: f64 = 0.5;
const SUM_CANCELLING: f64 = 1e16;
const SUM_SMALL_TERM: f64 = 1e-16;
const SUM_SMALL_COUNT: usize = 64;
const SUM_ALTERNATING_STEP: f64 = 1e-13;
const SUM_ALTERNATING_COUNT: usize = 128;
const SUM_SUBNORMAL_COUNT: usize = 32;
const SMALLEST_SUBNORMAL: f64 = 5e-324;
const ROUND_DIGITS: [i64; 4] = [0, 1, 6, 15];
const SORT_PAIRS: usize = 64;
const SORT_MODULUS: usize = 5;
const RANDOM_SEEDS: [u64; 4] = [0, 1, 20_260_909, (1 << 31) - 1];
const RANDOM_EPOCHS: [u64; 3] = [0, 1, 7];
const SHUFFLE_LENGTH: usize = 64;
const RANDOM_DRAWS: usize = 8;
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
const MAX_CODEPOINT: u32 = 0x10_ffff;
const PLANE_BITS: u32 = 16;
const INTEGERS: [i128; 10] = [-(1 << 70), -(1 << 63), -7, -1, 0, 1, 7, 1 << 31, (1 << 63) - 1, 1 << 70];
const SHIFT_BITS: u32 = 3;
const ROUNDING: [f64; 7] = [0.0, 0.5, 1.5, 2.5, -0.5, -1.5, 1e15 + 0.5];
const STRINGS: [&str; 16] = ["", "a", "A", "ß", "İ", "ı", "A\u{30a}", "Å", "\u{212b}", "й", "и\u{306}", "\u{1d11e}", "\u{1f600}", "ﬃ", "ẞ", "e\u{327}\u{301}"];

fn doubles() -> [f64; 21] {
    [0.0, -0.0, SMALLEST_SUBNORMAL, 2.225_073_858_507_201_4e-308, 1e-300, 1e-15, 0.1, 0.5, 1.0 - f64::EPSILON / 2.0, 1.0, 1.0 + f64::EPSILON, 1.5, 2.0, 3.0, 10.0, 1e15, 1e15 + 1.0, 9_007_199_254_740_991.0, 9_007_199_254_740_992.0, 1e300, 1.797_693_134_862_315_7e308]
}

fn double(value: f64) -> Vec<u8> {
    value.to_le_bytes().to_vec()
}

fn integer(value: i128) -> Vec<u8> {
    value.to_le_bytes().to_vec()
}

type Entry = (String, Vec<u8>);

fn float_arithmetic() -> Vec<Entry> {
    let mut out = Vec::new();
    // Every double of the table is a finite accumulator that is not below zero (-0.0 is not), as an `n` of FTRL is.
    for old_n in doubles() {
        for gradient in doubles() {
            let new_n = old_n + gradient * gradient;
            if !new_n.is_finite() {
                continue;
            }
            let name = format!("{}:{}", float_repr(old_n), float_repr(gradient));
            let sigma = (new_n.sqrt() - old_n.sqrt()) / FTRL_ALPHA;
            out.push((format!("sigma:{name}"), double(sigma)));
            let z_value = gradient - sigma * (gradient * FTRL_WEIGHT_PER_GRADIENT);
            out.push((format!("z:{name}"), double(z_value)));
            if z_value.abs() <= FTRL_L1 {
                continue;
            }
            let sign = if z_value < 0.0 { -1.0 } else { 1.0 };
            let denominator = (FTRL_BETA + new_n.sqrt()) / FTRL_ALPHA + FTRL_L2;
            out.push((format!("weight:{name}"), double(-(z_value - sign * FTRL_L1) / denominator)));
        }
    }
    out
}

fn libm() -> Vec<Entry> {
    let unary: [(&str, fn(f64) -> f64); 6] = [("sqrt", f64::sqrt), ("exp", f64::exp), ("log", f64::ln), ("log1p", f64::ln_1p), ("floor", f64::floor), ("ceil", f64::ceil)];
    let mut out = Vec::new();
    for (name, function) in unary {
        for value in doubles() {
            out.push((format!("{name}:{}", float_repr(value)), double(function(value))));
        }
    }
    for left in doubles() {
        for right in doubles() {
            out.push((format!("nextafter:{}:{}", float_repr(left), float_repr(right)), double(next_after(left, right))));
        }
    }
    out
}

/// `math.nextafter`.
#[expect(clippy::float_cmp, reason = "the exact double is the meaning here, as it is in the Python this was checked against")]
fn next_after(from: f64, to: f64) -> f64 {
    if from == to {
        return to;
    }
    if from == 0.0 {
        return f64::copysign(SMALLEST_SUBNORMAL, to);
    }
    let bits = from.to_bits();
    f64::from_bits(if (from < to) == (from > 0.0) { bits + 1 } else { bits - 1 })
}

fn builtin_sum() -> Vec<Entry> {
    let alternating: Vec<f64> = (0..SUM_ALTERNATING_COUNT).map(|index| (if index % 2 != 0 { 1.0 } else { -1.0 }) * (1.0 + index.to::<f64>() * SUM_ALTERNATING_STEP)).collect();
    let mut many_small = vec![1.0];
    many_small.extend(vec![SUM_SMALL_TERM; SUM_SMALL_COUNT]);
    let mut small_first = vec![SUM_SMALL_TERM; SUM_SMALL_COUNT];
    small_first.push(1.0);
    let sequences: Vec<(&str, Vec<f64>)> = vec![("cancel", vec![SUM_CANCELLING, 1.0, -SUM_CANCELLING]), ("cancel_reversed", vec![-SUM_CANCELLING, 1.0, SUM_CANCELLING]), ("many_small", many_small), ("many_small_reversed", small_first), ("alternating", alternating), ("subnormal", vec![SMALLEST_SUBNORMAL; SUM_SUBNORMAL_COUNT]), ("mixed", doubles().to_vec())];
    let mut out = Vec::new();
    for (name, values) in sequences {
        out.push((format!("sum:{name}"), double(ks_lang::py_sum(values.iter().copied()))));
        out.push((format!("fsum:{name}"), fsum(&values).map_or_else(|_| b"OverflowError".to_vec(), double)));
        out.push((format!("loop:{name}"), double(values.iter().fold(0.0, |total, value| total + value))));
    }
    out
}

fn float_text() -> Vec<Entry> {
    let mut out = Vec::new();
    for value in doubles() {
        let name = float_repr(value);
        out.push((format!("repr:{name}"), name.clone().into_bytes()));
        out.push((format!("hex:{name}"), float_hex(value).into_bytes()));
        out.push((format!("json:{name}"), ks_text::pyjson::json_float(value).into_bytes()));
        out.push((format!("roundtrip:{name}"), double(name.parse().expect("repr parses"))));
        for digits in ROUND_DIGITS {
            out.push((format!("round:{name}:{digits}"), double(py_round_ndigits(value, digits).expect("a finite number"))));
        }
    }
    out
}

fn unicode() -> Vec<Entry> {
    let mut out = Vec::new();
    for point in 0..=MAX_CODEPOINT {
        let Some(character) = char::from_u32(point) else { continue };
        let text = character.to_string();
        let (folded, composed, decomposed) = (pystr::casefold(&text), normalize::nfc(&text), normalize::nfd(&text));
        if folded == text && composed == text && decomposed == text {
            out.push((format!("U+{point:04X}"), vec![0]));
            continue;
        }
        out.push((format!("U+{point:04X}:fold"), folded.into_bytes()));
        out.push((format!("U+{point:04X}:nfc"), composed.into_bytes()));
        out.push((format!("U+{point:04X}:nfd"), decomposed.into_bytes()));
    }
    for value in STRINGS {
        out.push((format!("str:fold:{value:?}"), pystr::casefold(value).into_bytes()));
        out.push((format!("str:token:{value:?}"), ks_lang::intent_model::normalize_token(value).into_bytes()));
    }
    out
}

fn hashing() -> Vec<Entry> {
    let mut out = Vec::new();
    for value in STRINGS {
        out.push((format!("utf8:{value:?}"), value.as_bytes().to_vec()));
        let mut result = FNV_OFFSET;
        for byte in value.bytes() {
            result = (result ^ u64::from(byte)).wrapping_mul(FNV_PRIME);
        }
        out.push((format!("fnv1a64:{value:?}"), integer(i128::from(result))));
    }
    for number in doubles() {
        out.push((format!("pack_le:{}", float_repr(number)), number.to_le_bytes().to_vec()));
        out.push((format!("pack_be:{}", float_repr(number)), number.to_be_bytes().to_vec()));
    }
    out.push(("byteorder".to_owned(), if cfg!(target_endian = "little") { b"little".to_vec() } else { b"big".to_vec() }));
    out.push(("sha256:empty".to_owned(), Sha256::digest(b"").to_vec()));
    out.push(("sha256:probe".to_owned(), Sha256::digest(b"keyswitch-environment-probe").to_vec()));
    out
}

fn ordering() -> Vec<Entry> {
    let pairs: Vec<(usize, usize)> = (0..SORT_PAIRS).map(|index| (index % SORT_MODULUS, index)).collect();
    let mut stable = pairs.clone();
    stable.sort_by_key(|item| item.0);
    let mut reversed = pairs;
    reversed.sort_by_key(|a| std::cmp::Reverse(a.0));
    let mut strings = STRINGS.to_vec();
    strings.sort_unstable();
    let mut folded = STRINGS.to_vec();
    folded.sort_by_cached_key(|value| pystr::casefold(value));
    let mut finite: Vec<f64> = doubles().to_vec();
    finite.sort_by(|a, b| a.partial_cmp(b).expect("finite operands"));
    let reprs: Vec<String> = finite.iter().map(|value| float_repr(*value)).collect();
    vec![("stable".to_owned(), format!("{stable:?}").into_bytes()), ("reverse".to_owned(), format!("{reversed:?}").into_bytes()), ("strings".to_owned(), format!("{strings:?}").into_bytes()), ("strings_fold".to_owned(), format!("{folded:?}").into_bytes()), ("doubles".to_owned(), reprs.join(",").into_bytes())]
}

fn integers() -> Vec<Entry> {
    let mut out = Vec::new();
    for left in INTEGERS {
        out.push((format!("str:{left}"), left.to_string().into_bytes()));
        for right in INTEGERS {
            if right == 0 {
                continue;
            }
            out.push((format!("floordiv:{left}:{right}"), integer(left.div_euclid(right) - i128::from(right < 0 && left.rem_euclid(right) != 0))));
            out.push((format!("mod:{left}:{right}"), integer(((left % right) + right) % right)));
        }
        out.push((format!("shift:{left}"), [integer(left >> SHIFT_BITS), integer(left << SHIFT_BITS)].concat()));
    }
    for value in ROUNDING {
        out.push((format!("round_int:{}", float_repr(value)), integer(ks_text::py_round(value).to::<i128>())));
        out.push((format!("trunc:{}", float_repr(value)), integer(value.trunc().to::<i128>())));
    }
    out
}

fn random_stream() -> Vec<Entry> {
    let mut out = Vec::new();
    for seed in RANDOM_SEEDS {
        for epoch in RANDOM_EPOCHS {
            let mut indices: Vec<u32> = (0..SHUFFLE_LENGTH.to::<u32>()).collect();
            Twister::new(seed + epoch).shuffle(&mut indices);
            out.push((format!("shuffle:{seed}:{epoch}"), format!("{indices:?}").into_bytes()));
            let mut generator = Twister::new(seed + epoch);
            let draws: Vec<u8> = (0..RANDOM_DRAWS).flat_map(|_| generator.next_u32().to_le_bytes()).collect();
            out.push((format!("random:{seed}:{epoch}"), draws));
        }
    }
    out
}

const CELLS: [(&str, fn() -> Vec<Entry>); 9] = [("float_arithmetic", float_arithmetic), ("libm", libm), ("builtin_sum", builtin_sum), ("float_text", float_text), ("unicode", unicode), ("hashing", hashing), ("ordering", ordering), ("integers", integers), ("random_stream", random_stream)];

fn bucket_of(cell: &str, label: &str) -> String {
    if cell == "unicode"
        && let Some(rest) = label.strip_prefix("U+") {
            let point = u32::from_str_radix(rest.split(':').next().unwrap_or(rest), 16).unwrap_or(0);
            return format!("plane-{:02X}", point >> PLANE_BITS);
        }
    label.split(':').next().unwrap_or(label).to_owned()
}

fn measure_cell(name: &str, entries: Vec<Entry>) -> Value {
    let mut overall = Sha256::new();
    let mut buckets: BTreeMap<String, Sha256> = BTreeMap::new();
    let count = entries.len();
    for (label, image) in entries {
        let mut record = label.clone().into_bytes();
        record.push(0);
        record.extend_from_slice(&image);
        record.push(b'\n');
        overall.update(&record);
        buckets.entry(bucket_of(name, &label)).or_default().update(&record);
    }
    let digests: Map<String, Value> = buckets.into_iter().map(|(bucket, hasher)| (bucket, json!(hex(&hasher.finalize())))).collect();
    json!({"sha256": hex(&overall.finalize()), "entries": count, "buckets": digests})
}

/// `measure`.
pub fn measure() -> Value {
    let mut cells = Map::new();
    for (name, cell) in CELLS {
        cells.insert(name.to_owned(), measure_cell(name, cell()));
    }
    let mut names: Vec<&String> = cells.keys().collect();
    names.sort();
    let mut combined = Sha256::new();
    for name in names {
        combined.update(name.as_bytes());
        combined.update(b"\0");
        combined.update(cells[name]["sha256"].as_str().expect("a digest").as_bytes());
        combined.update(b"\n");
    }
    json!({"schema_version": SCHEMA_VERSION, "probe_sha256": hex(&combined.finalize()), "implementation": "rust", "cells": cells})
}
