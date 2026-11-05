//! The macOS answers the settings window asks for.
pub const SYSTEM_DARK_STYLE: &str = "Dark";

/// Whether the AppleInterfaceStyle default says the dark appearance is on.
pub fn dark_from_interface_style(style: &str) -> bool {
    style == SYSTEM_DARK_STYLE
}
