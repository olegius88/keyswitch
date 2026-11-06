//! The edges of the Python functions, with the answers CPython gives: what the larger recorded sets happen to pass by.

use ks_text::normalize::{combining_class, nfc, nfd};
use ks_text::pystr::{center, char_at, count, find, ljust, lower, rfind, rjust, rsplit_sep, slice, split_max, zfill};
use ks_text::{FsumError, Indent, JsonOptions, exp, float_repr, fmean, fsum, json_dumps, log, log2, log_base, pstdev, py_float, py_int, py_round_ndigits, py_round_to_int, sqrt};
use serde_json::json;

#[test]
fn the_shortest_form_of_a_float_is_the_one_that_reads_back() {
    // Beside a power of two the nearest value of that length may read back as another float: Python keeps the longer way.
    assert_eq!(float_repr(f64::from_bits(4_494_592_428_115_755_008)), "2.9802322387695312e-08");
    assert_eq!(float_repr(f64::from_bits(27_021_597_764_222_976)), "7.120236347223045e-307");
    assert_eq!(float_repr(1.0e16), "1e+16");
    assert_eq!(float_repr(1.5), "1.5");
}

#[test]
fn whole_numbers_as_int_reads_them() {
    for (text, value) in [("+5", Some(5)), ("-5", Some(-5)), ("1_000", Some(1000)), (" 7 ", Some(7)), ("-", None), ("+", None), ("", None), ("_1", None), ("1__0", None), ("1_", None), ("1.5", None), ("x", None), ("٣", Some(3))] {
        assert_eq!(py_int(text), value, "{text:?}");
    }
    assert_eq!(py_int(&"9".repeat(4301)), None);
    assert_eq!(py_int("170141183460469231731687303715884105729"), None);
    assert_eq!(py_int("-170141183460469231731687303715884105728"), Some(i128::MIN));
    assert_eq!(py_int("-170141183460469231731687303715884105729"), None);
}

#[test]
fn floats_as_float_reads_them() {
    for (text, value) in [("1e5", Some(1e5)), ("1E+5", Some(1e5)), ("1e-2", Some(0.01)), ("+1.5", Some(1.5)), ("-.5", Some(-0.5)), ("5.", Some(5.0)), ("1_000.5", Some(1000.5)), (" 2\t", Some(2.0)), ("٣", Some(3.0)), (".", None), ("e5", None), ("1e", None), ("1e+", None), ("1a", None), ("", None), ("1.5.5", None), ("--1", None)] {
        assert_eq!(py_float(text), value, "{text:?}");
    }
    assert_eq!(py_float("-inf"), Some(f64::NEG_INFINITY));
    assert_eq!(py_float("Infinity"), Some(f64::INFINITY));
    assert!(py_float("nan").is_some_and(f64::is_nan));
}

#[test]
fn rounding_as_round_does_it() {
    for (x, digits, value) in [(0.125, 2, 0.12), (0.375, 2, 0.38), (2.5, 0, 2.0), (1.5, 0, 2.0), (1234.5678, -2, 1200.0), (9.96, 1, 10.0), (99.96, 1, 100.0), (-1.25, 1, -1.2), (1.35, 1, 1.4), (1.5, 400, 1.5), (1.5, -400, 0.0)] {
        assert_eq!(py_round_ndigits(x, digits), Some(value), "{x} {digits}");
    }
    assert_eq!(py_round_ndigits(f64::INFINITY, 2), Some(f64::INFINITY));
    assert!(py_round_ndigits(f64::NAN, 2).is_some_and(f64::is_nan));
    assert_eq!(py_round_ndigits(f64::MAX, -308), None);
    assert_eq!(py_round_to_int(2.5), Some(2));
    assert_eq!(py_round_to_int(f64::INFINITY), None);
    assert_eq!(py_round_to_int(f64::NAN), None);
}

#[test]
fn exact_sums_and_means() {
    assert_eq!(fsum(&[0.1; 10]), Ok(1.0));
    assert_eq!(fsum(&[1e100, 1.0, -1e100]), Ok(1.0));
    assert_eq!(fsum(&[]), Ok(0.0));
    assert_eq!(fsum(&[1.0]), Ok(1.0));
    assert_eq!(fsum(&[f64::INFINITY, 1.0]), Ok(f64::INFINITY));
    assert_eq!(fsum(&[f64::INFINITY, f64::NEG_INFINITY]), Err(FsumError::InfMinusInf));
    assert_eq!(fsum(&[f64::MAX, f64::MAX]), Err(FsumError::Overflow));
    assert!(fsum(&[f64::NAN, 1.0]).is_ok_and(f64::is_nan));
    assert_eq!(fsum(&[1.0, 1e-16, 1e-16]), Ok(1.0000000000000002));
    assert_eq!(fmean(&[]), None);
    assert_eq!(fmean(&[1.0, 2.0]), Some(1.5));
    assert_eq!(pstdev(&[]), None);
    assert_eq!(pstdev(&[1.0]), Some(0.0));
    assert_eq!(pstdev(&[0.0, 0.0]), Some(0.0));
    assert_eq!(pstdev(&[1.0, 3.0]), Some(1.0));
    assert_eq!(pstdev(&[1e-300, 2e-300]), Some(5e-301));
    assert_eq!(pstdev(&[1e300, 2e300]), Some(5e299));
    assert_eq!(pstdev(&[5e-324, 1e-323]).map(f64::is_finite), Some(true));
    assert!(pstdev(&[1.5, 1e-10]).is_some_and(|value| value > 0.7));
}

#[test]
fn logarithms_and_roots() {
    assert_eq!(log2(8.0), Some(3.0));
    assert_eq!(log2(f64::INFINITY), Some(f64::INFINITY));
    assert!(log2(f64::NAN).is_some_and(f64::is_nan));
    assert_eq!((log2(0.0), log2(-1.0)), (None, None));
    assert_eq!(log(1.0), Some(0.0));
    assert_eq!(log(f64::INFINITY), Some(f64::INFINITY));
    assert!(log(f64::NAN).is_some_and(f64::is_nan));
    assert_eq!((log(0.0), log(-1.0)), (None, None));
    assert_eq!(log_base(8.0, 1.0), None);
    assert_eq!(exp(0.0), Some(1.0));
    assert_eq!(exp(1000.0), None);
    assert_eq!(exp(f64::INFINITY), Some(f64::INFINITY));
    assert!(exp(f64::NAN).is_some_and(f64::is_nan));
    assert_eq!(sqrt(4.0), Some(2.0));
    assert_eq!(sqrt(-1.0), None);
    assert!(sqrt(f64::NAN).is_some_and(f64::is_nan));
}

#[test]
fn searching_and_filling_text() {
    assert_eq!(rsplit_sep("a,b,c", ",", -1), Some(vec!["a", "b", "c"]));
    assert_eq!(rsplit_sep("a,b,c", ",", 1), Some(vec!["a,b", "c"]));
    assert_eq!(split_max("a b c", 1), ["a", "b c"]);
    assert_eq!(find("abc", "z", None, None), -1);
    assert_eq!(find("abc", "", Some(5), None), -1);
    assert_eq!(find("abc", "c", Some(-1), None), 2);
    assert_eq!(find("abc", "abcd", None, None), -1);
    assert_eq!(rfind("abc", "b", Some(2), Some(1)), -1);
    assert_eq!(rfind("abc", "abcd", None, None), -1);
    assert_eq!(rfind("abcb", "b", None, None), 3);
    assert_eq!(count("abc", "abcd", None, None), 0);
    assert_eq!(count("abc", "a", Some(2), Some(1)), 0);
    assert_eq!(count("aaa", "a", None, None), 3);
    assert_eq!(ljust("a", 3, '.'), "a..");
    assert_eq!(rjust("a", 3, '.'), "..a");
    assert_eq!(rjust("abc", 1, '.'), "abc");
    assert_eq!(center("ab", 1, '.'), "ab");
    assert_eq!(center("a", 3, '.'), ".a.");
    assert_eq!(zfill("5", 1), "5");
    assert_eq!(zfill("-5", 3), "-05");
    assert_eq!(zfill("+5", 3), "+05");
    assert_eq!(zfill("5", 3), "005");
    assert_eq!(zfill("", 2), "00");
    assert_eq!((char_at("abc", 5), char_at("abc", -1), char_at("abc", -5)), (None, Some('c'), None));
    assert_eq!(slice("abc", Some(-2), None), "bc");
    assert_eq!(lower("Α'Σ"), "α'ς");
    assert_eq!(lower("Σ"), "σ");
}

#[test]
fn hangul_composes_only_what_it_should() {
    assert_eq!(nfc("\u{1100}\u{1161}"), "\u{ac00}");
    assert_eq!(nfc("\u{ac00}\u{11a8}"), "\u{ac01}");
    assert_eq!(nfc("\u{1100}a"), "\u{1100}a");
    assert_eq!(nfc("\u{ac01}\u{11a8}"), "\u{ac01}\u{11a8}");
    assert_eq!(nfc("\u{ac00}\u{11a7}"), "\u{ac00}\u{11a7}");
    assert_eq!(nfd("\u{ac01}"), "\u{1100}\u{1161}\u{11a8}");
    assert_eq!(nfc("a\u{5ae}\u{300}"), "à\u{5ae}");
    assert_eq!(combining_class('a'), 0);
    assert_eq!(combining_class('\u{300}'), 230);
}

#[test]
fn json_writes_every_escape_and_any_indent() {
    let options = JsonOptions { indent: Some(Indent::Text("\t".to_owned())), ..JsonOptions::default() };
    assert_eq!(json_dumps(&json!([1]), &options), "[\n\t1\n]");
    let plain = JsonOptions::default();
    assert_eq!(json_dumps(&json!("\r\t\n\"\\\u{8}\u{c}\u{1}"), &plain), r#""\r\t\n\"\\\b\f\u0001""#);
}
