//! The numbers the shared window model uses (`constants/settings_defaults.py`, constants/updates.py).
pub const NUMBER_DISPLAY_DECIMAL_PLACES: i32 = 2;
pub const APPLICATION_CAPTURE_DELAY_MS: u32 = 3000;
/// The interval between automatic update checks.
pub const UPDATE_CHECK_INTERVAL_MS: u32 = 6 * 3600 * 1000;
pub const HISTORY_LIMIT_SETTING_MIN: f64 = 10.0;
pub const HISTORY_LIMIT_SETTING_MAX: f64 = 5000.0;
pub const HISTORY_LIMIT_SETTING_STEP: f64 = 10.0;
pub const CONFIDENCE_SETTING_STEP: f64 = 0.1;
pub const PAUSE_DELAY_SETTING_STEP_SECONDS: f64 = 0.1;
/// The default conversion hotkey as the texts of this system name it: a Mac keyboard has no Pause, and the macOS
/// default is Control+Option+K (`ks_config::constants::MACOS_HOTKEY_DEFAULTS`).
pub const CONVERT_HOTKEY_NAME: &str = if cfg!(target_os = "macos") { "Control+Option+K" } else { "Pause" };
