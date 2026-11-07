//! The artifact's bytes: keys sorted, one feature with its weights to a line, so that a diff of two artifacts names the
//! features that changed. The seal is of these very bytes.
use ks_text::pyjson::json_string;
use ks_text::{JsonOptions, json_dumps};
use serde_json::Value;

fn compact(sort_keys: bool) -> JsonOptions {
    JsonOptions { ensure_ascii: false, separators: Some((",".to_owned(), ":".to_owned())), sort_keys, ..JsonOptions::default() }
}

/// `json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":")) + "\n"`.
#[must_use]
pub fn canonical(value: &Value) -> Vec<u8> {
    (json_dumps(value, &compact(true)) + "\n").into_bytes()
}

#[must_use]
pub fn artifact_bytes(payload: &Value) -> Vec<u8> {
    let object = payload.as_object().cloned().unwrap_or_default();
    let mut items: Vec<(&String, &Value)> = object.iter().collect();
    items.sort_by(|a, b| ks_text::pystr::cmp(a.0, b.0));
    let mut lines = vec!["{".to_owned()];
    for (index, (key, value)) in items.iter().enumerate() {
        let comma = if index + 1 < items.len() { "," } else { "" };
        if *key != "weights" {
            lines.push(format!(" {}: {}{comma}", json_string(key, true), json_dumps(value, &compact(true))));
            continue;
        }
        let table = value.as_object().cloned().unwrap_or_default();
        let mut weights: Vec<(&String, &Value)> = table.iter().collect();
        weights.sort_by(|a, b| ks_text::pystr::cmp(a.0, b.0));
        lines.push(" \"weights\": {".to_owned());
        for (position, (name, row)) in weights.iter().enumerate() {
            let separator = if position + 1 < weights.len() { "," } else { "" };
            lines.push(format!("  {}: {}{separator}", json_string(name, false), json_dumps(row, &compact(false))));
        }
        lines.push(format!(" }}{comma}"));
    }
    lines.push("}".to_owned());
    (lines.join("\n") + "\n").into_bytes()
}


#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_artifact_writes_one_feature_to_a_line() {
        let bytes = artifact_bytes(&json!({"version": "v", "weights": {"b": [1, 2], "a": [0.5]}, "actions": ["keep"]}));
        let text = String::from_utf8(bytes).expect("text");
        assert_eq!(text, "{\n \"actions\": [\"keep\"],\n \"version\": \"v\",\n \"weights\": {\n  \"a\": [0.5],\n  \"b\": [1,2]\n }\n}\n");
        assert_eq!(canonical(&json!({"b": 1, "a": "я"})), b"{\"a\":\"\xd1\x8f\",\"b\":1}\n".to_vec());
    }
}
