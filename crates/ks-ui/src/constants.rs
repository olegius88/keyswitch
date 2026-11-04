//! The numbers the shared window model uses (constants/settings_defaults.py, constants/updates.py).
pub const NUMBER_DISPLAY_DECIMAL_PLACES: i32 = 2;
pub const APPLICATION_CAPTURE_DELAY_MS: u32 = 3000;
/// The interval between automatic update checks.
pub const UPDATE_CHECK_INTERVAL_MS: u32 = 6 * 3600 * 1000;
pub const HISTORY_LIMIT_SETTING_MIN: f64 = 10.0;
pub const HISTORY_LIMIT_SETTING_MAX: f64 = 5000.0;
pub const HISTORY_LIMIT_SETTING_STEP: f64 = 10.0;
pub const CONFIDENCE_SETTING_STEP: f64 = 0.1;
pub const PAUSE_DELAY_SETTING_STEP_SECONDS: f64 = 0.1;
