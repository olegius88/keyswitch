//! Comparing what the two engines left: exact, floats to the bit.

use ks_backend::KeyDisposition;
use serde_json::Value;

/// Equal JSON, an integer and a float of one value being different things as they are to `json.dumps`.
pub(crate) fn same(left: &Value, right: &Value) -> bool {
    first_difference(left, right, "").is_none()
}

/// `KeyDisposition = bool | Literal["defer"]`.
pub(crate) fn disposition_value(disposition: KeyDisposition) -> Value {
    match disposition {
        KeyDisposition::Pass => Value::Bool(false),
        KeyDisposition::Consume => Value::Bool(true),
        KeyDisposition::Defer => Value::String("defer".to_owned()),
    }
}

/// The path of the first place two values differ, with both values.
pub(crate) fn first_difference(left: &Value, right: &Value, path: &str) -> Option<(String, Value, Value)> {
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

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn two_values_differ_where_a_field_is_missing_on_either_side() {
        let found = first_difference(&json!({"a": 1, "b": 2}), &json!({"a": 1}), "");
        assert_eq!(found, Some((".b".to_owned(), json!(2), Value::Null)));
        let found = first_difference(&json!({"a": 1}), &json!({"a": 1, "c": 3}), "");
        assert_eq!(found, Some((".c".to_owned(), Value::Null, json!(3))));
        assert!(same(&json!({"a": [1, 2]}), &json!({"a": [1, 2]})));
    }

    #[test]
    fn lists_of_other_lengths_differ_as_a_whole() {
        let found = first_difference(&json!([1, 2]), &json!([1]), ".x");
        assert_eq!(found, Some((".x.len".to_owned(), json!([1, 2]), json!([1]))));
        assert_eq!(first_difference(&json!([1, 2]), &json!([1, 3]), "").map(|found| found.0), Some("[1]".to_owned()));
    }

    #[test]
    fn an_integer_and_a_float_of_one_value_differ_and_floats_compare_to_the_bit() {
        assert!(!same(&json!(1), &json!(1.0)));
        assert!(!same(&json!(1.0), &json!(1)));
        assert!(same(&json!(0.1), &json!(0.1)));
        assert!(!same(&json!(0.1), &json!(0.2)));
        assert!(same(&json!(5), &json!(5)));
        assert!(!same(&json!("a"), &json!(1)));
    }

    #[test]
    fn a_disposition_reads_as_the_python_answer() {
        assert_eq!(disposition_value(KeyDisposition::Pass), json!(false));
        assert_eq!(disposition_value(KeyDisposition::Consume), json!(true));
        assert_eq!(disposition_value(KeyDisposition::Defer), json!("defer"));
    }
}
