//! Comparing what the two engines left: exact, floats to the bit.

use ks_backend::KeyDisposition;
use serde_json::Value;

/// Equal JSON, an integer and a float of one value being different things as they are to `json.dumps`.
pub fn same(left: &Value, right: &Value) -> bool {
    first_difference(left, right, "").is_none()
}

/// `KeyDisposition = bool | Literal["defer"]`.
pub fn disposition_value(disposition: KeyDisposition) -> Value {
    match disposition {
        KeyDisposition::Pass => Value::Bool(false),
        KeyDisposition::Consume => Value::Bool(true),
        KeyDisposition::Defer => Value::String("defer".to_owned()),
    }
}

/// The path of the first place two values differ, with both values.
pub fn first_difference(left: &Value, right: &Value, path: &str) -> Option<(String, Value, Value)> {
    match (left, right) {
        (Value::Object(a), Value::Object(b)) => {
            for (key, value) in a {
                let inner = format!("{path}.{key}");
                match b.get(key) {
                    Some(other) => {
                        if let Some(found) = first_difference(value, other, &inner) {
                            return Some(found);
                        }
                    }
                    None => return Some((inner, value.clone(), Value::Null)),
                }
            }
            b.iter().find(|(key, _)| !a.contains_key(*key)).map(|(key, value)| (format!("{path}.{key}"), Value::Null, value.clone()))
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                return Some((format!("{path}.len"), left.clone(), right.clone()));
            }
            a.iter().zip(b).enumerate().find_map(|(index, (x, y))| first_difference(x, y, &format!("{path}[{index}]")))
        }
        (Value::Number(a), Value::Number(b)) => {
            let equal = match (a.as_i64(), b.as_i64()) {
                (Some(x), Some(y)) => x == y,
                (None, None) => a.as_f64().map(f64::to_bits) == b.as_f64().map(f64::to_bits),
                _ => false,
            };
            (!equal).then(|| (path.to_owned(), left.clone(), right.clone()))
        }
        _ => (left != right).then(|| (path.to_owned(), left.clone(), right.clone())),
    }
}
