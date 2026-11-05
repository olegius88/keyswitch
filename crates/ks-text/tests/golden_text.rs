//! Parity of `ks-text` with CPython 3.14, against golden files dumped from the interpreter.
//!
//! The files live in `$KS_GOLDEN_DIR/text` (default: the `golden` folder of the private work tree); a test
//! skips with a message when its file is absent. Floats are compared by their bits (a NaN only has to be a NaN).

use ks_cast::To as _;
use flate2::read::GzDecoder;
use ks_text::{layouts, normalize, pyjson, pynum, pystr, russian_text, ucd};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;

fn golden(name: &str) -> Option<PathBuf> {
    let root = std::env::var("KS_GOLDEN_DIR")
        .unwrap_or_else(|_| "/mnt/data2/_projects/puntoswitcher/.t/rust-2026-11-04/golden".to_string());
    let path = PathBuf::from(root).join("text").join(name);
    if path.exists() {
        Some(path)
    } else {
        eprintln!("SKIPPED: golden file {} is absent", path.display());
        None
    }
}

fn lines(name: &str) -> Option<impl Iterator<Item = Value>> {
    let path = golden(name)?;
    let reader = BufReader::new(GzDecoder::new(std::fs::File::open(path).expect("open")));
    Some(reader.lines().map(|l| serde_json::from_str(&l.expect("line")).expect("json line")))
}

fn float(hex: &str) -> f64 {
    f64::from_bits(u64::from_str_radix(hex, 16).expect("hex bits"))
}

fn same_float(got: f64, want: f64) -> bool {
    if want.is_nan() { got.is_nan() } else { got.to_bits() == want.to_bits() }
}

fn text(cps: &Value) -> String {
    cps.as_array().expect("array").iter().map(|c| char::from_u32(c.as_u64().expect("cp").low::<u32>()).expect("scalar")).collect()
}

#[derive(Default)]
struct Failures {
    count: usize,
    total: usize,
    shown: Vec<String>,
}

impl Failures {
    fn check<T: PartialEq + std::fmt::Debug>(&mut self, what: &str, input: &str, got: T, want: T) {
        self.total += 1;
        if got != want {
            self.count += 1;
            if self.shown.len() < 25 {
                self.shown.push(format!("{what}({input:?}): got {got:?}, want {want:?}"));
            }
        }
    }
    fn finish(self, name: &str) {
        eprintln!("{name}: {} checks, {} failures", self.total, self.count);
        assert!(self.count == 0, "{} failures of {}:\n{}", self.count, self.total, self.shown.join("\n"));
    }
}

#[test]
fn predicates_for_every_code_point() {
    let Some(path) = golden("props.bin.gz") else { return };
    let mut data = Vec::new();
    GzDecoder::new(std::fs::File::open(path).unwrap()).read_to_end(&mut data).unwrap();
    assert_eq!(data.len(), 0x110000 * 4, "all 1 114 112 code points");
    let mut failures = Failures::default();
    for cp in 0..0x110000u32 {
        let v = u32::from_le_bytes(data[cp.to::<usize>() * 4..cp.to::<usize>() * 4 + 4].try_into().unwrap());
        let got: [bool; 16] = [
            ucd::isalpha_cp(cp), ucd::isalnum_cp(cp), ucd::isdecimal_cp(cp), ucd::isdigit_cp(cp), ucd::isnumeric_cp(cp),
            ucd::isspace_cp(cp), ucd::isupper_cp(cp), ucd::islower_cp(cp), ucd::istitle_cp(cp), ucd::isprintable_cp(cp),
            ucd::re_w_cp(cp), ucd::re_d_cp(cp), ucd::re_s_cp(cp), ucd::re_letter_cp(cp), ucd::is_cased_cp(cp),
            ucd::is_case_ignorable_cp(cp),
        ];
        for (bit, &g) in got.iter().enumerate() {
            failures.check(&format!("pred{bit}"), &format!("U+{cp:04X}"), g, v >> bit & 1 == 1);
        }
        if let Some(c) = char::from_u32(cp) {
            let want = if v >> 20 & 1 == 1 { Some(v >> 16 & 0xF) } else { None };
            failures.check("decimal_value", &format!("U+{cp:04X}"), ucd::decimal_value(c), want);
            // The `char` entry points agree with the raw ones.
            failures.check("isalpha(char)", &format!("U+{cp:04X}"), ucd::isalpha(c), v & 1 == 1);
        }
    }
    failures.finish("predicates");
}

#[test]
fn case_mapping_for_every_code_point() {
    let Some(rows) = lines("case.jsonl.gz") else { return };
    let mut table = std::collections::HashMap::new();
    for row in rows {
        let r = row.as_array().unwrap();
        table.insert(r[0].as_u64().unwrap().low::<u32>(), [text(&r[1]), text(&r[2]), text(&r[3]), text(&r[4])]);
    }
    let mut failures = Failures::default();
    for cp in 0..0x110000u32 {
        let Some(c) = char::from_u32(cp) else { continue };
        let s = c.to_string();
        let want = table.get(&cp).cloned().unwrap_or_else(|| [s.clone(), s.clone(), s.clone(), s.clone()]);
        let input = format!("U+{cp:04X}");
        failures.check("lower", &input, pystr::lower(&s), want[0].clone());
        failures.check("upper", &input, pystr::upper(&s), want[1].clone());
        failures.check("casefold", &input, pystr::casefold(&s), want[2].clone());
        failures.check("title", &input, pystr::title(&s), want[3].clone());
        failures.check("capitalize", &input, pystr::capitalize(&s), want[3].clone());
    }
    failures.finish("case mapping");
}

#[test]
fn normalisation_for_every_code_point() {
    let Some(rows) = lines("norm.jsonl.gz") else { return };
    let mut table = std::collections::HashMap::new();
    for row in rows {
        let r = row.as_array().unwrap();
        table.insert(r[0].as_u64().unwrap().low::<u32>(), (text(&r[1]), text(&r[2]), r[3].as_u64().unwrap().low::<u8>()));
    }
    let mut failures = Failures::default();
    for cp in 0..0x110000u32 {
        let Some(c) = char::from_u32(cp) else { continue };
        let s = c.to_string();
        let want = table.get(&cp).cloned().unwrap_or_else(|| (s.clone(), s.clone(), 0));
        let input = format!("U+{cp:04X}");
        failures.check("nfc", &input, normalize::nfc(&s), want.0.clone());
        failures.check("nfd", &input, normalize::nfd(&s), want.1.clone());
        failures.check("ccc", &input, normalize::combining_class(c), want.2);
    }
    failures.finish("normalisation");
}

fn strs(v: &Value) -> Vec<&str> {
    v.as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect()
}

fn opt(v: &Value) -> Option<i64> {
    v.as_i64()
}

#[test]
fn string_operations_over_the_sample() {
    let Some(rows) = lines("strings.jsonl.gz") else { return };
    let mut failures = Failures::default();
    let mut strings = 0usize;
    for row in rows {
        strings += 1;
        let s = row["s"].as_str().unwrap();
        let o = &row["o"];
        let st = |k: &str| o[k].as_str().unwrap().to_string();
        failures.check("lower", s, pystr::lower(s), st("lower"));
        failures.check("upper", s, pystr::upper(s), st("upper"));
        failures.check("casefold", s, pystr::casefold(s), st("casefold"));
        failures.check("capitalize", s, pystr::capitalize(s), st("capitalize"));
        failures.check("title", s, pystr::title(s), st("title"));
        failures.check("swapcase", s, pystr::swapcase(s), st("swapcase"));
        failures.check("strip", s, pystr::strip(s).to_string(), st("strip"));
        failures.check("lstrip", s, pystr::lstrip(s).to_string(), st("lstrip"));
        failures.check("rstrip", s, pystr::rstrip(s).to_string(), st("rstrip"));
        failures.check("split", s, pystr::split(s), strs(&o["split"]));
        failures.check("splitlines", s, pystr::splitlines(s, false), strs(&o["splitlines"]));
        failures.check("splitlines_keep", s, pystr::splitlines(s, true), strs(&o["splitlines_keep"]));
        let is = o["is"].as_array().unwrap().iter().map(|b| b.as_bool().unwrap()).collect::<Vec<_>>();
        let got = vec![
            pystr::isalpha(s), pystr::isalnum(s), pystr::isdecimal(s), pystr::isdigit(s), pystr::isnumeric(s),
            pystr::isspace(s), pystr::isupper(s), pystr::islower(s), pystr::istitle(s), pystr::isprintable(s),
            pystr::isascii(s),
        ];
        failures.check("is*", s, got, is);
        failures.check("nfc", s, normalize::nfc(s), st("nfc"));
        failures.check("nfd", s, normalize::nfd(s), st("nfd"));
        failures.check("len", s, pystr::len(s).to::<u64>(), o["len"].as_u64().unwrap());
        if o.get("strip_chars").is_none() {
            continue;
        }
        let sc = o["strip_chars"].as_array().unwrap();
        let cs = sc[0].as_str().unwrap();
        failures.check("strip_chars", s, pystr::strip_chars(s, cs), sc[1].as_str().unwrap());
        failures.check("lstrip_chars", s, pystr::lstrip_chars(s, cs), sc[2].as_str().unwrap());
        failures.check("rstrip_chars", s, pystr::rstrip_chars(s, cs), sc[3].as_str().unwrap());
        let sm = o["split_max"].as_array().unwrap();
        failures.check("split_max", s, pystr::split_max(s, sm[0].as_i64().unwrap()), strs(&sm[1]));
        let ss = o["split_sep"].as_array().unwrap();
        let (sep, mx) = (ss[0].as_str().unwrap(), ss[1].as_i64().unwrap());
        failures.check("split_sep", s, pystr::split_sep(s, sep, mx).unwrap(), strs(&ss[2]));
        failures.check("rsplit_sep", s, pystr::rsplit_sep(s, sep, mx).unwrap(), strs(&ss[3]));
        for sl in o["slice"].as_array().unwrap() {
            let sl = sl.as_array().unwrap();
            failures.check("slice", s, pystr::slice(s, opt(&sl[0]), opt(&sl[1])).to_string(), sl[2].as_str().unwrap().to_string());
        }
        for sl in o["slice_step"].as_array().unwrap() {
            let sl = sl.as_array().unwrap();
            failures.check(
                "slice_step",
                s,
                pystr::slice_step(s, opt(&sl[0]), opt(&sl[1]), sl[2].as_i64().unwrap()).unwrap(),
                sl[3].as_str().unwrap().to_string(),
            );
        }
        for ca in o["char_at"].as_array().unwrap() {
            let ca = ca.as_array().unwrap();
            let want = ca[1].as_str().map(|x| x.chars().next().unwrap());
            failures.check("char_at", s, pystr::char_at(s, ca[0].as_i64().unwrap()), want);
        }
        let f = o["find"].as_array().unwrap();
        let (sub, a, b) = (f[0].as_str().unwrap(), opt(&f[1]), opt(&f[2]));
        failures.check("find", s, pystr::find(s, sub, a, b), f[3].as_i64().unwrap());
        failures.check("rfind", s, pystr::rfind(s, sub, a, b), f[4].as_i64().unwrap());
        failures.check("count", s, pystr::count(s, sub, a, b).to::<i64>(), f[5].as_i64().unwrap());
        let p = o["pad"].as_array().unwrap();
        let (w, fill) = (p[0].as_u64().unwrap().to::<usize>(), p[1].as_str().unwrap().chars().next().unwrap());
        failures.check("ljust", s, pystr::ljust(s, w, fill), p[2].as_str().unwrap().to_string());
        failures.check("rjust", s, pystr::rjust(s, w, fill), p[3].as_str().unwrap().to_string());
        failures.check("center", s, pystr::center(s, w, fill), p[4].as_str().unwrap().to_string());
        failures.check("zfill", s, pystr::zfill(s, w), p[5].as_str().unwrap().to_string());
    }
    assert!(strings >= 200_000, "only {strings} strings in the sample");
    failures.finish("string operations");
}

fn bitvec(v: &Value) -> Vec<f64> {
    v.as_array().unwrap().iter().map(|x| float(x.as_str().unwrap())).collect()
}

fn result_float(r: &Value) -> Option<f64> {
    match r.as_str().unwrap() {
        "err" => None,
        h => Some(float(h)),
    }
}

fn same_opt(got: Option<f64>, want: Option<f64>) -> bool {
    match (got, want) {
        (Some(g), Some(w)) => same_float(g, w),
        (None, None) => true,
        _ => false,
    }
}

#[test]
fn numbers() {
    let Some(rows) = lines("numbers.jsonl.gz") else { return };
    let mut failures = Failures::default();
    let mut seen = std::collections::BTreeMap::<String, usize>::new();
    for row in rows {
        let k = row["k"].as_str().unwrap().to_string();
        *seen.entry(k.clone()).or_default() += 1;
        match k.as_str() {
            "round" => {
                let x = float(row["x"].as_str().unwrap());
                let input = format!("{x:e}");
                match row["r"].as_str().unwrap() {
                    "err" => failures.check("round_to_int", &input, pynum::py_round_to_int(x), None),
                    r => {
                        let want = float(row["f"].as_str().unwrap());
                        failures.check("py_round", &input, same_float(pynum::py_round(x), want), true);
                        match r.parse::<i128>() {
                            Ok(v) if v.unsigned_abs() < 170_000_000_000_000_000_000_000_000_000_000_000_000 => {
                                failures.check("round_to_int", &input, pynum::py_round_to_int(x), Some(v))
                            }
                            _ => {}
                        }
                    }
                }
            }
            "roundn" => {
                let x = float(row["x"].as_str().unwrap());
                let n = row["n"].as_i64().unwrap();
                let got = pynum::py_round_ndigits(x, n);
                let want = result_float(&row["r"]);
                failures.check("round_ndigits", &format!("{x:e},{n}"), same_opt(got, want), true);
            }
            "repr" => {
                let x = float(row["x"].as_str().unwrap());
                failures.check("repr", &format!("{:016x}", x.to_bits()), pynum::float_repr(x), row["r"].as_str().unwrap().to_string());
            }
            "float" => {
                let s = row["s"].as_str().unwrap();
                let want = row["r"].as_str().map(float);
                failures.check("py_float", s, same_opt(pynum::py_float(s), want), true);
            }
            "int" => {
                let s = row["s"].as_str().unwrap();
                let want = match row["r"].as_str() {
                    None | Some("big") => None,
                    Some(v) => Some(v.parse::<i128>().unwrap()),
                };
                failures.check("py_int", s, pynum::py_int(s), want);
            }
            "fsum" => {
                let xs = bitvec(&row["xs"]);
                let want = match row["r"].as_str().unwrap() {
                    "err" => None,
                    h => Some(float(h)),
                };
                let got = pynum::fsum(&xs).ok();
                failures.check("fsum", &format!("{} values", xs.len()), same_opt(got, want), true);
            }
            "fmean" => {
                let xs = bitvec(&row["xs"]);
                let want = result_float(&row["r"]);
                failures.check("fmean", &format!("{} values", xs.len()), same_opt(pynum::fmean(&xs), want), true);
            }
            "pstdev" => {
                let xs = bitvec(&row["xs"]);
                let want = result_float(&row["r"]);
                failures.check("pstdev", &format!("{:?}", &xs[..xs.len().min(4)]), same_opt(pynum::pstdev(&xs), want), true);
            }
            "log" | "log2" | "exp" | "sqrt" => {
                let x = float(row["x"].as_str().unwrap());
                let got = match k.as_str() {
                    "log" => pynum::log(x),
                    "log2" => pynum::log2(x),
                    "exp" => pynum::exp(x),
                    _ => pynum::sqrt(x),
                };
                failures.check(&k, &format!("{x:e}"), same_opt(got, result_float(&row["r"])), true);
            }
            "logb" => {
                let x = float(row["x"].as_str().unwrap());
                let b = float(row["b"].as_str().unwrap());
                failures.check("log_base", &format!("{x:e},{b:e}"), same_opt(pynum::log_base(x, b), result_float(&row["r"])), true);
            }
            other => panic!("unknown kind {other}"),
        }
    }
    eprintln!("numbers by kind: {seen:?}");
    failures.finish("numbers");
}

#[test]
fn json_dumps_byte_for_byte() {
    let Some(rows) = lines("json.jsonl.gz") else { return };
    let mut failures = Failures::default();
    for row in rows {
        let text = row["v"].as_str().unwrap();
        let value: Value = serde_json::from_str(text).unwrap();
        for case in row["c"].as_array().unwrap() {
            let case = case.as_array().unwrap();
            let o = case[0].as_array().unwrap();
            let indent = match &o[1] {
                Value::Null => None,
                Value::Number(n) => Some(pyjson::Indent::Spaces(n.as_u64().unwrap().to::<usize>())),
                Value::String(s) => Some(pyjson::Indent::Text(s.clone())),
                _ => unreachable!(),
            };
            let separators = o[2].as_array().map(|a| (a[0].as_str().unwrap().to_string(), a[1].as_str().unwrap().to_string()));
            let options = pyjson::JsonOptions { ensure_ascii: o[0].as_bool().unwrap(), indent, separators, sort_keys: o[3].as_bool().unwrap() };
            failures.check("json_dumps", &format!("{text} {options:?}"), pyjson::json_dumps(&value, &options), case[1].as_str().unwrap().to_string());
        }
    }
    failures.finish("json_dumps");
}

#[test]
fn layouts_and_russian_text() {
    let Some(rows) = lines("misc.jsonl.gz") else { return };
    let mut failures = Failures::default();
    let pair = layouts::LayoutPair::default();
    let pairs = |v: &Value| -> Vec<(char, char)> {
        v.as_array().unwrap().iter().map(|p| {
            let p = p.as_array().unwrap();
            (p[0].as_str().unwrap().chars().next().unwrap(), p[1].as_str().unwrap().chars().next().unwrap())
        }).collect()
    };
    for row in rows {
        match row["k"].as_str().unwrap() {
            "maps" => {
                failures.check("us_to_ru", "", pair.us_to_ru(), pairs(&row["us_to_ru"]));
                failures.check("ru_to_us", "", pair.ru_to_us(), pairs(&row["ru_to_us"]));
            }
            "tr" => {
                let s = row["s"].as_str().unwrap();
                failures.check("translate us->ru", s, pair.translate(s, "us", "ru").unwrap(), row["us_ru"].as_str().unwrap().to_string());
                failures.check("translate ru->us", s, pair.translate(s, "ru", "us").unwrap(), row["ru_us"].as_str().unwrap().to_string());
            }
            "q" => {
                let v = float(row["v"].as_str().unwrap());
                let forms = match row["f"].as_str().unwrap() {
                    "SECONDS" => russian_text::SECONDS,
                    "HOURS" => russian_text::HOURS,
                    "CHARACTERS" => russian_text::CHARACTERS,
                    _ => russian_text::LETTERS,
                };
                failures.check("quantity", &format!("{v:e}"), Some(russian_text::quantity(v, forms)), row["r"].as_str().map(String::from));
            }
            "n" => {
                let v = float(row["v"].as_str().unwrap());
                failures.check("russian_number", &format!("{v:e}"), russian_text::russian_number(v), row["r"].as_str().unwrap().to_string());
            }
            other => panic!("unknown kind {other}"),
        }
    }
    failures.finish("layouts and russian_text");
}
