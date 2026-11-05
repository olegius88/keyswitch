//! Pure layout-indicator mapping shared by the tray and tests.

use crate::constants::{LAYOUT_LABELS, LAYOUT_NAMES};
use serde_json::Value;

pub const INDICATOR_STYLES: [&str; 2] = ["letters", "flags"];

/// `normalize_indicator_style(value)`: `str(value)` when it is a known style, "letters" otherwise. Only a string can
/// read as a style, whatever else `str` would make of the value.
#[must_use]
pub fn normalize_indicator_style(value: &Value) -> &'static str {
    match value {
        Value::String(style) => INDICATOR_STYLES.iter().copied().find(|known| known == style).unwrap_or("letters"),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_) => "letters",
    }
}

fn label_of(group: i64) -> Option<&'static str> {
    LAYOUT_LABELS.iter().find(|(key, _)| *key == group).map(|(_, label)| *label)
}

#[must_use]
pub fn layout_label(group: i64) -> &'static str {
    label_of(group).unwrap_or("—")
}

/// Return the other member of the supported EN/RU layout pair.
#[must_use]
pub fn alternate_layout_group(group: i64) -> Option<i64> {
    label_of(group).map(|_| 1 - group)
}

#[must_use]
pub fn alternate_layout_action_label(group: i64) -> String {
    match alternate_layout_group(group) {
        Some(target) => {
            let name = LAYOUT_NAMES.iter().find(|(key, _)| *key == target).map_or("", |(_, name)| *name);
            format!("Переключить на {name}")
        }
        None => "Переключить язык".to_string(),
    }
}

#[must_use]
pub fn layout_icon_name(style: &Value, group: i64) -> &'static str {
    match (normalize_indicator_style(style), group) {
        ("letters", 0) => "keyswitch-en",
        ("letters", 1) => "keyswitch-ru",
        ("flags", 0) => "keyswitch-flag-us",
        ("flags", 1) => "keyswitch-flag-ru",
        _ => "keyswitch",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn mapping() {
        assert_eq!(normalize_indicator_style(&json!("flags")), "flags");
        assert_eq!(normalize_indicator_style(&json!("x")), "letters");
        assert_eq!(normalize_indicator_style(&Value::Null), "letters");
        assert_eq!(layout_label(0), "EN");
        assert_eq!(layout_label(5), "—");
        assert_eq!(alternate_layout_group(1), Some(0));
        assert_eq!(alternate_layout_group(2), None);
        assert_eq!(alternate_layout_action_label(0), "Переключить на русский (RU)");
        assert_eq!(alternate_layout_action_label(-1), "Переключить язык");
        assert_eq!(layout_icon_name(&json!("flags"), 1), "keyswitch-flag-ru");
        assert_eq!(layout_icon_name(&json!("letters"), 7), "keyswitch");
    }
}
