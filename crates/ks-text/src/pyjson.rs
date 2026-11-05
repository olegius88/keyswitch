//! `json.dumps` of Python, byte for byte, over `serde_json::Value` (with `preserve_order`).
//!
//! The program reads and writes settings, history and rule files that must stay byte compatible with the
//! Python's: same escapes, same float text (`repr`), same separators, same indentation, same key order.
//! Reading is serde_json's job (with `float_roundtrip`, so that doubles equal Python's).

use ks_cast::To as _;
use crate::pynum::float_repr;
use serde_json::Value;

/// The `indent` argument: `None` (compact), a number of spaces, or a string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Indent {
    Spaces(usize),
    Text(String),
}

/// The keyword arguments of `json.dumps`. `Default` is `json.dumps(value)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonOptions {
    pub ensure_ascii: bool,
    pub indent: Option<Indent>,
    /// `separators=(item, key)`; `None` is Python's default: `(", ", ": ")`, or `(",", ": ")` with an indent.
    pub separators: Option<(String, String)>,
    pub sort_keys: bool,
}

impl Default for JsonOptions {
    fn default() -> Self {
        JsonOptions { ensure_ascii: true, indent: None, separators: None, sort_keys: false }
    }
}

/// A JSON string literal as `json.dumps` writes it.
pub fn json_string(s: &str, ensure_ascii: bool) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if c.to::<u32>() < 0x20 => out.push_str(&format!("\\u{:04x}", c.to::<u32>())),
            // With ensure_ascii everything outside space..~ is escaped, DEL included.
            c if ensure_ascii && !(' '..='~').contains(&c) => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{:04x}", unit));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A float as `json.dumps` writes it: `repr`, with `NaN`, `Infinity` and `-Infinity` for the specials.
pub fn json_float(x: f64) -> String {
    if x.is_nan() {
        "NaN".to_string()
    } else if x.is_infinite() {
        if x < 0.0 { "-Infinity" } else { "Infinity" }.to_string()
    } else {
        float_repr(x)
    }
}

struct Writer<'a> {
    options: &'a JsonOptions,
    item_sep: String,
    key_sep: String,
    indent: Option<String>,
    out: String,
}

impl Writer<'_> {
    fn newline(&mut self, level: usize) {
        if let Some(unit) = &self.indent {
            self.out.push('\n');
            for _ in 0..level {
                self.out.push_str(unit);
            }
        }
    }

    fn value(&mut self, value: &Value, level: usize) {
        match value {
            Value::Null => self.out.push_str("null"),
            Value::Bool(true) => self.out.push_str("true"),
            Value::Bool(false) => self.out.push_str("false"),
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    self.out.push_str(&i.to_string());
                } else if let Some(u) = n.as_u64() {
                    self.out.push_str(&u.to_string());
                } else {
                    self.out.push_str(&json_float(n.as_f64().expect("a float")));
                }
            }
            Value::String(s) => self.out.push_str(&json_string(s, self.options.ensure_ascii)),
            Value::Array(items) => {
                if items.is_empty() {
                    self.out.push_str("[]");
                    return;
                }
                self.out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        let sep = self.item_sep.clone();
                        self.out.push_str(&sep);
                    }
                    self.newline(level + 1);
                    self.value(item, level + 1);
                }
                self.newline(level);
                self.out.push(']');
            }
            Value::Object(map) => {
                if map.is_empty() {
                    self.out.push_str("{}");
                    return;
                }
                let mut entries: Vec<(&String, &Value)> = map.iter().collect();
                if self.options.sort_keys {
                    entries.sort_by(|a, b| a.0.cmp(b.0));
                }
                self.out.push('{');
                for (i, (key, item)) in entries.into_iter().enumerate() {
                    if i > 0 {
                        let sep = self.item_sep.clone();
                        self.out.push_str(&sep);
                    }
                    self.newline(level + 1);
                    self.out.push_str(&json_string(key, self.options.ensure_ascii));
                    let sep = self.key_sep.clone();
                    self.out.push_str(&sep);
                    self.value(item, level + 1);
                }
                self.newline(level);
                self.out.push('}');
            }
        }
    }
}

/// `json.dumps(value, ensure_ascii=..., indent=..., separators=..., sort_keys=...)`.
pub fn json_dumps(value: &Value, options: &JsonOptions) -> String {
    let (item_sep, key_sep) = match &options.separators {
        Some((i, k)) => (i.clone(), k.clone()),
        None if options.indent.is_some() => (",".to_string(), ": ".to_string()),
        None => (", ".to_string(), ": ".to_string()),
    };
    let indent = options.indent.as_ref().map(|i| match i {
        Indent::Spaces(n) => " ".repeat(*n),
        Indent::Text(t) => t.clone(),
    });
    let mut writer = Writer { options, item_sep, key_sep, indent, out: String::new() };
    writer.value(value, 0);
    writer.out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dumps_like_python() {
        let v: Value = serde_json::from_str(r#"{"b": [1, 2.0, 1e16, "я\u0000\u007f😀"], "a": {}, "c": [], "d": null}"#).unwrap();
        assert_eq!(
            json_dumps(&v, &JsonOptions::default()),
            r#"{"b": [1, 2.0, 1e+16, "\u044f\u0000\u007f\ud83d\ude00"], "a": {}, "c": [], "d": null}"#
        );
        let o = JsonOptions { ensure_ascii: false, sort_keys: true, separators: Some((",".into(), ":".into())), ..Default::default() };
        assert_eq!(json_dumps(&v, &o), "{\"a\":{},\"b\":[1,2.0,1e+16,\"я\\u0000\u{7f}😀\"],\"c\":[],\"d\":null}");
        let o = JsonOptions { ensure_ascii: false, indent: Some(Indent::Spaces(2)), ..Default::default() };
        assert_eq!(json_dumps(&json!({"x": [1, {"y": "z"}], "e": []}), &o), "{\n  \"x\": [\n    1,\n    {\n      \"y\": \"z\"\n    }\n  ],\n  \"e\": []\n}");
        assert_eq!(json_dumps(&json!([]), &o), "[]");
        let o = JsonOptions { indent: Some(Indent::Spaces(0)), ..Default::default() };
        assert_eq!(json_dumps(&json!([1]), &o), "[\n1\n]");
    }

    #[test]
    fn escapes() {
        assert_eq!(json_string("a\"b\\c\n\u{8}\u{c}\u{1}", true), r#""a\"b\\c\n\b\f\u0001""#);
        assert_eq!(json_string("\u{7f}é", false), "\"\u{7f}é\"");
        assert_eq!(json_float(f64::NAN), "NaN");
    }
}
