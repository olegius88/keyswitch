//! The macOS answers the settings window asks for.
pub const SYSTEM_DARK_STYLE: &str = "Dark";

/// Whether the `AppleInterfaceStyle` default says the dark appearance is on.
#[must_use]
pub fn dark_from_interface_style(style: &str) -> bool {
    style == SYSTEM_DARK_STYLE
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn the_style_of_the_system_says_dark() {
        assert!(dark_from_interface_style(SYSTEM_DARK_STYLE));
        assert!(!dark_from_interface_style("Light"));
        assert!(!dark_from_interface_style(""));
    }
}
