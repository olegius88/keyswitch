//! The one door for reading and writing JSON: a typed value comes out of a file, and a typed value goes into one.
//!
//! [`loads`] reads a whole document into any `Deserialize` type and refuses anything after it but whitespace, as
//! `json.loads` does ("Extra data"): the check the loaders of the workspace make today. [`loads_seed`] and [`read_seed`]
//! do the same for a hand-written `DeserializeSeed`, from bytes in memory or from a stream, for the model files whose
//! loaders must not build their whole document at once. [`dumps`] writes any `Serialize` value with the bytes of
//! Python's `json.dumps`, through the writer of [`crate::pyjson`]: the value is first turned into the tree that
//! `serde_json::json!` would build from it (both go through `serde_json::to_value`), so a typed record writes exactly
//! the bytes that the same data built key by key wrote before.
//!
//! The untyped tree stays inside this module and [`crate::pyjson`]; nothing else names it.
#![expect(clippy::disallowed_methods, reason = "json.dumps over the tree serde_json parses: the one writer, the one reader")]
#![cfg_attr(test, expect(clippy::disallowed_macros, clippy::disallowed_types, reason = "the typed writer is checked against the tree it replaces"))]

use crate::pyjson::{json_dumps, JsonOptions};
use serde::de::{DeserializeOwned, DeserializeSeed};
use serde::Serialize;
use std::io;

/// Why a document could not be read: its text, where the parser stopped, and the kind of fault.
pub use serde_json::Error as JsonError;

/// `json.loads(text)` into a typed value: the whole text is one document, and nothing but whitespace follows it.
pub fn loads<T: DeserializeOwned>(text: impl AsRef<[u8]>) -> Result<T, JsonError> {
    serde_json::from_slice(text.as_ref())
}

/// [`loads`] with a hand-written seed: the seed reads the document, then nothing but whitespace may follow.
pub fn loads_seed<'de, S: DeserializeSeed<'de>>(text: &'de [u8], seed: S) -> Result<S::Value, JsonError> {
    let mut deserializer = serde_json::Deserializer::from_slice(text);
    let value = seed.deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(value)
}

/// [`loads_seed`] from a stream, for a file too large to hold as text and as values at once.
pub fn read_seed<'de, R: io::Read, S: DeserializeSeed<'de>>(reader: R, seed: S) -> Result<S::Value, JsonError> {
    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    let value = seed.deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(value)
}

/// `json.dumps(value, ...)` of a typed value, byte for byte as Python writes the same data. Struct fields keep their
/// order unless `sort_keys`; `None` is `null`; a float that is not finite is `null`, as `json!` writes it.
#[must_use]
pub fn dumps<T: Serialize + ?Sized>(value: &T, options: &JsonOptions) -> String {
    let tree = serde_json::to_value(value).expect("a record of this program is JSON: its map keys are text, its paths are UTF-8");
    json_dumps(&tree, options)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::pyjson::Indent;
    use serde::Deserialize;
    use serde::de::{Deserializer, IgnoredAny, Visitor};
    use serde_json::{json, Value};
    use std::collections::BTreeMap;
    use std::fmt;
    use std::marker::PhantomData;

    #[derive(Serialize)]
    struct Inner {
        word: String,
        score: f64,
    }

    #[derive(Serialize)]
    #[serde(rename_all = "snake_case")]
    enum Mode {
        Early,
        Late,
    }

    #[derive(Serialize)]
    enum Shape {
        Sized(u8),
        Pair { left: i8, right: i8 },
    }

    /// The kinds of field the tree's writers put into their files, in an order that is not sorted.
    #[derive(Serialize)]
    struct Record {
        zeta: u64,
        alpha: Vec<f64>,
        text: String,
        mode: Mode,
        later: Mode,
        inner: Inner,
        missing: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        absent: Option<i64>,
        present: Option<i64>,
        pair: (i64, bool),
        letter: char,
        unit: (),
        empty: Vec<u8>,
        nothing: BTreeMap<String, u8>,
        counts: BTreeMap<String, i64>,
        numbered: BTreeMap<u32, bool>,
        shapes: Vec<Shape>,
        #[serde(rename = "the name")]
        renamed: i8,
        negative: i64,
    }

    fn record() -> Record {
        Record {
            zeta: u64::MAX,
            alpha: vec![2.0, 1e16, 0.1, -0.0, 1.5e-7, 1.0 / 3.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY],
            text: "я\u{0}\u{7f}😀\"\\\n\t".to_owned(),
            mode: Mode::Early,
            later: Mode::Late,
            inner: Inner { word: "довод".to_owned(), score: 0.25 },
            missing: None,
            absent: None,
            present: Some(-3),
            pair: (7, true),
            letter: 'ж',
            unit: (),
            empty: Vec::new(),
            nothing: BTreeMap::new(),
            counts: BTreeMap::from([("b".to_owned(), 2), ("a".to_owned(), 1), ("Б".to_owned(), 3)]),
            numbered: BTreeMap::from([(10, false), (2, true)]),
            shapes: vec![Shape::Sized(4), Shape::Pair { left: -1, right: 1 }],
            renamed: 5,
            negative: i64::MIN,
        }
    }

    /// The same data as the tree's writers build it today, key by key.
    fn tree() -> Value {
        json!({
            "zeta": u64::MAX,
            "alpha": [2.0, 1e16, 0.1, -0.0, 1.5e-7, 1.0 / 3.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY],
            "text": "я\u{0}\u{7f}😀\"\\\n\t",
            "mode": "early",
            "later": "late",
            "inner": {"word": "довод", "score": 0.25},
            "missing": null,
            "present": -3,
            "pair": [7, true],
            "letter": "ж",
            "unit": null,
            "empty": [],
            "nothing": {},
            "counts": {"a": 1, "b": 2, "Б": 3},
            "numbered": {"2": true, "10": false},
            "shapes": [{"Sized": 4}, {"Pair": {"left": -1, "right": 1}}],
            "the name": 5,
            "negative": i64::MIN,
        })
    }

    /// Every form of options the tree writes with: the default, the sorted compact form of hashes and log lines, the
    /// indented forms of the stores and reports, sorted and not, with and without `ensure_ascii`.
    fn every_options() -> Vec<JsonOptions> {
        let compact = Some((",".to_owned(), ":".to_owned()));
        vec![
            JsonOptions::default(),
            JsonOptions { ensure_ascii: false, ..JsonOptions::default() },
            JsonOptions { sort_keys: true, ..JsonOptions::default() },
            JsonOptions { ensure_ascii: false, sort_keys: true, separators: compact.clone(), ..JsonOptions::default() },
            JsonOptions { ensure_ascii: false, separators: compact, ..JsonOptions::default() },
            JsonOptions { ensure_ascii: false, indent: Some(Indent::Spaces(2)), ..JsonOptions::default() },
            JsonOptions { ensure_ascii: false, indent: Some(Indent::Spaces(1)), sort_keys: true, ..JsonOptions::default() },
            JsonOptions { indent: Some(Indent::Spaces(2)), ..JsonOptions::default() },
            JsonOptions { indent: Some(Indent::Spaces(0)), ..JsonOptions::default() },
            JsonOptions { indent: Some(Indent::Text("\t".to_owned())), ..JsonOptions::default() },
        ]
    }

    #[test]
    fn a_typed_record_writes_the_bytes_of_the_tree_it_replaces() {
        let (record, tree) = (record(), tree());
        for options in every_options() {
            assert_eq!(dumps(&record, &options), json_dumps(&tree, &options), "{options:?}");
        }
    }

    #[test]
    fn a_typed_record_writes_what_python_writes() {
        let options = JsonOptions { ensure_ascii: false, sort_keys: true, separators: Some((",".to_owned(), ":".to_owned())), ..JsonOptions::default() };
        let record = Inner { word: "я\"😀".to_owned(), score: 1e16 };
        assert_eq!(dumps(&record, &options), "{\"score\":1e+16,\"word\":\"я\\\"😀\"}");
        assert_eq!(dumps(&record, &JsonOptions::default()), r#"{"word": "\u044f\"\ud83d\ude00", "score": 1e+16}"#);
        let options = JsonOptions { ensure_ascii: false, indent: Some(Indent::Spaces(2)), ..JsonOptions::default() };
        assert_eq!(dumps(&vec![Some(2.0), None], &options), "[\n  2.0,\n  null\n]");
        assert_eq!(dumps("only text", &options), "\"only text\"");
        assert_eq!(dumps(&[f64::NAN], &options), "[\n  null\n]");
    }

    #[derive(Debug, PartialEq, Deserialize)]
    struct Settings {
        name: String,
        layouts: Vec<String>,
        delay: f64,
        limit: Option<u64>,
        #[serde(default)]
        later: bool,
    }

    #[test]
    fn loads_reads_a_typed_value_and_ignores_unknown_keys() {
        let text = r#" {"name": "довод", "layouts": ["us", "ru"], "delay": 0.1, "limit": null, "unknown": [1, {}]} "#;
        let settings: Settings = loads(text).unwrap();
        assert_eq!(settings, Settings { name: "довод".to_owned(), layouts: vec!["us".to_owned(), "ru".to_owned()], delay: 0.1, limit: None, later: false });
        let settings: Settings = loads(br#"{"name": "", "layouts": [], "delay": 2, "limit": 18446744073709551615, "later": true}"#).unwrap();
        assert_eq!(settings, Settings { name: String::new(), layouts: Vec::new(), delay: 2.0, limit: Some(u64::MAX), later: true });
        assert!(loads::<Settings>(r#"{"name": "", "layouts": [], "delay": 1e400}"#).unwrap_err().is_syntax());
    }

    #[test]
    fn loads_refuses_what_the_loaders_refuse_today_with_the_same_text() {
        for text in ["{} x", "{}{}", "[1] ]", "", "{", "{\"a\": NaN}", "\u{feff}{}", "[1,]", "{\"a\": 1,}"] {
            let typed = loads::<Value>(text).unwrap_err().to_string();
            let today = serde_json::from_str::<Value>(text).unwrap_err().to_string();
            assert_eq!(typed, today, "{text:?}");
            assert!(loads::<IgnoredAny>(text).is_err(), "{text:?}");
        }
        assert_eq!(loads::<IgnoredAny>("{} x").unwrap_err().to_string(), "trailing characters at line 1 column 4");
        assert!(loads::<Settings>(r#"{"name": 1}"#).unwrap_err().is_data());
        assert!(loads::<Settings>(b"\xff".as_slice()).is_err());
        assert!(loads::<IgnoredAny>(" {} \n\t\r ").is_ok());
    }

    #[test]
    fn a_number_reads_back_to_the_double_python_reads() {
        // The halfway case between 1 and the next double: Python rounds it to even, and so must the parse.
        let halfway: f64 = loads("1.00000000000000011102230246251565404236316680908203125").unwrap();
        assert_eq!(halfway.to_bits(), 1.0_f64.to_bits());
        let x: f64 = loads("0.30000000000000004").unwrap();
        assert_eq!(x.to_bits(), (0.1_f64 + 0.2).to_bits());
        let options = JsonOptions::default();
        for text in ["2.0", "1e+16", "-0.0", "1.5e-07", "0.1", "5e-324", "1.7976931348623157e+308"] {
            assert_eq!(dumps(&loads::<f64>(text).unwrap(), &options), text);
        }
    }

    /// A seed that counts the items of an array without keeping them, as the model loaders read their tables.
    struct CountItems;

    impl<'de> DeserializeSeed<'de> for CountItems {
        type Value = usize;

        fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<usize, D::Error> {
            deserializer.deserialize_seq(self)
        }
    }

    impl<'de> Visitor<'de> for CountItems {
        type Value = usize;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("an array")
        }

        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut items: A) -> Result<usize, A::Error> {
            let mut count = 0;
            while items.next_element::<IgnoredAny>()?.is_some() {
                count += 1;
            }
            Ok(count)
        }
    }

    #[test]
    fn a_seed_reads_the_whole_document_from_bytes_and_from_a_stream() {
        assert_eq!(loads_seed(b"[1, [2, 3], {}]", CountItems).unwrap(), 3);
        assert_eq!(read_seed(b" [1, 2] \n".as_slice(), CountItems).unwrap(), 2);
        assert_eq!(loads_seed(br#"["a", "b"]"#, PhantomData::<Vec<&str>>).unwrap(), vec!["a", "b"]);
        let texts: [&[u8]; 4] = [b"[1] x", b"[1][2]", b"{}", b"[1,"];
        for text in texts {
            let today = serde_json::from_slice::<Value>(text).err().map(|error| error.classify());
            let from_bytes = loads_seed(text, CountItems).unwrap_err();
            let from_stream = read_seed(text, CountItems).unwrap_err();
            // A stream may place a fault one column apart from bytes (`{}` read as an array: column 0, and 1); the kind is the same.
            assert_eq!(from_bytes.classify(), from_stream.classify(), "{text:?}");
            if let Some(category) = today {
                assert_eq!(from_bytes.classify(), category, "{text:?}");
            }
        }
        assert_eq!(loads_seed(b"[1] x", CountItems).unwrap_err().to_string(), "trailing characters at line 1 column 5");
        assert_eq!(read_seed(b"[1] x".as_slice(), CountItems).unwrap_err().to_string(), "trailing characters at line 1 column 5");
    }

    #[test]
    #[should_panic(expected = "a record of this program is JSON")]
    fn a_value_that_is_not_json_stops_the_writer() {
        let _ = dumps(&BTreeMap::from([((1, 2), 3)]), &JsonOptions::default());
    }
}
