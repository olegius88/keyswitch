//! Pure layout-indicator mapping shared by the tray and tests.

use crate::constants::{LAYOUT_LABELS, LAYOUT_NAMES};
pub use crate::settings_file::{layout_icon_name, normalize_indicator_style};

pub const INDICATOR_STYLES: [&str; 2] = ["letters", "flags"];

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

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn mapping() {
        assert_eq!(layout_label(0), "EN");
        assert_eq!(layout_label(5), "—");
        assert_eq!(alternate_layout_group(1), Some(0));
        assert_eq!(alternate_layout_group(2), None);
        assert_eq!(alternate_layout_action_label(0), "Переключить на русский (RU)");
        assert_eq!(alternate_layout_action_label(-1), "Переключить язык");
    }
}
