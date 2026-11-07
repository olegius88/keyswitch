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

// ---- the one-time hints (`hints`)
/// A hint at the caret waits until no key has been pressed for this long, so that it never appears in the middle of a
/// word.
pub const HINT_TYPING_SETTLE_SECONDS: f64 = 1.0;
/// It waits no longer than this for the typing to stop; then it stays unshown until the next event of its kind.
pub const HINT_SETTLE_WAIT_MAX_SECONDS: f64 = 6.0;
/// Hints are shown no more often than this, and no more of them than this in a calendar day; the cards in the settings
/// window, which only show while the user looks at the window, are not counted.
pub const HINT_QUIET_SECONDS: f64 = 600.0;
pub const HINTS_PER_DAY_MAX: i64 = 3;
/// A hint nobody closes goes after this long: at the caret, and near the tray icon.
pub const HINT_CARET_TIMEOUT_SECONDS: f64 = 12.0;
pub const HINT_TRAY_TIMEOUT_SECONDS: f64 = 20.0;
/// The longest text of a hint, with the shipped hotkeys put in: two or three lines of a card `HINT_CARD_WIDTH_PIXELS`
/// wide.
pub const HINT_TEXT_MAX_CHARACTERS: usize = 160;
pub const HINT_CARD_WIDTH_PIXELS: i32 = 460;
/// The first hint of a new installation comes this long after the engine started, the news of an update this long after
/// the program did.
pub const HINT_FIRST_START_DELAY_SECONDS: f64 = 5.0;
pub const HINT_WHATS_NEW_DELAY_SECONDS: f64 = 30.0;
/// The hint on the double press of the conversion hotkey is dropped as found out when a rule prompt follows a manual
/// conversion within this long.
pub const HINT_RULE_PROMPT_FOUND_SECONDS: f64 = 2.0;
/// A hint at the caret that could not be placed this many times running is shown near the tray icon instead.
pub const CARET_FALLBACK_AFTER_ATTEMPTS: i64 = 3;
/// The corrections a hint waits for.
pub const AUTOMATIC_CORRECTIONS_FOR_PAUSE_HINT: i64 = 5;
pub const AUTOMATIC_CORRECTIONS_FOR_NOTIFICATIONS_HINT: i64 = 3;
pub const AUTOMATIC_CORRECTIONS_FOR_HISTORY_HINT: i64 = 10;
pub const MANUAL_CONVERSIONS_FOR_SELECTION_HINT: i64 = 3;
/// How often a platform asks the hints what to show while one waits or is on screen.
pub const HINT_TICK_INTERVAL_MS: u32 = 250;
/// `schema_version` of `hints.json` (`ks_config::constants::HINTS_FILE_NAME`).
pub const HINTS_SCHEMA_VERSION: i64 = 1;
/// The first characters of a UTC timestamp (`2026-11-07T10:00:00+00:00`): its date, the day the daily limit counts.
pub const TIMESTAMP_DATE_CHARACTERS: usize = 10;
/// The release that brought the hints themselves, and the one that brought the layout switch key a hint of news is about.
pub const HINTS_INTRODUCED_VERSION: &str = "0.71.0";
pub const LAYOUT_SWITCH_KEY_VERSION: &str = "0.70.0";
