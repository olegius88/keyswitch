//! The values the engine uses, each once, as in the Python's `constants` folder.

pub use ks_backend::constants::{HOOK_GAP_MS, INPUT_DELAY_LATE_MS, INPUT_DELAY_SLOW_CALLBACK_MS, INPUT_DELAY_UNSTAMPED_MS, WHEEL_KEY_NAME};
pub use ks_config::constants::{DEFAULT_LAYOUT_SWITCH_KEY, LAYOUT_SWITCH_KEYS};
pub use ks_context::constants::{
    CONTEXT_ACTION_FEATURE_VERSION, CONTEXT_TERM_FREQUENCY_BUCKET_BOUNDS, KEPT_CONTEXT_WORD_MAX_CHARACTERS,
    PLANNED_CONTEXT_WORD_MAX_CHARACTERS, PREFIX_MAX_CHARACTERS,
};

pub const CONFIDENCE_SETTING_MAX: f64 = 10.0;
pub const CONFIDENCE_SETTING_MIN: f64 = 0.5;
pub const DEFAULT_CONFIDENCE_THRESHOLD: f64 = 2.0;
pub const DEFAULT_EARLY_SWITCH_MIN_LENGTH: f64 = 4.0;
pub const DEFAULT_FIELD_CATCH_UP: &str = "300";
pub const FIELD_CATCH_UP_CHOICES: [&str; 5] = ["off", "100", "300", "500", "1000"];
pub const FIELD_CATCH_UP_OFF: &str = "off";
pub const DEFAULT_MINIMUM_WORD_LENGTH: f64 = 3.0;
pub const DEFAULT_PAUSE_DELAY_SECONDS: f64 = 0.7;
pub const EARLY_SWITCH_MIN_LENGTH_SETTING_MAX: f64 = 8.0;
pub const EARLY_SWITCH_MIN_LENGTH_SETTING_MIN: f64 = 3.0;
pub const MINIMUM_WORD_LENGTH_SETTING_MAX: f64 = 12.0;
pub const MINIMUM_WORD_LENGTH_SETTING_MIN: f64 = 2.0;
pub const PAUSE_DELAY_SETTING_MAX_SECONDS: f64 = 5.0;
pub const PAUSE_DELAY_SETTING_MIN_SECONDS: f64 = 0.3;
pub const SELECTION_HOTKEYS: [&str; 3] = ["selection_layout", "selection_case", "selection_transliterate"];
pub const DOUBLE_CAPITALS: usize = 2;
pub const DOUBLE_CAPITALS_MIN_LETTERS: usize = 4;
pub const EARLY_SWITCH_CONFIDENCE: f64 = 15.0;
pub const ENGINE_EVENT_QUEUE_MAX_SIZE: usize = 4096;
pub const JOINED_PART_MIN_LETTERS: usize = 2;
pub const KEPT_WORD_PROSE_MIN_COUNT: u64 = 500;
pub const KEPT_WORDS_TAKEN_ALONG: usize = 2;
pub const MAX_REMEMBERED_APPLICATION_CONTEXTS: usize = 32;
pub const MAX_WORD_STROKES: usize = 256;
pub const NATURAL_SOURCE_BOUNDARY_MIN_CHARACTERS: usize = 4;
/// A token whose end the boundary model could not split is taken as a word and its signs when the other layout spells a
/// known word of at least this many letters. Shorter ones are as often a word and a sign (`vs.` is `мыю`).
pub const WHOLE_TOKEN_WORD_MIN_LETTERS: usize = 4;
pub const NATURAL_SOURCE_BOUNDARY_NGRAM_FLOOR: f64 = -0.25;
pub const PAUSE_TRAILING_MAX_KEYS: usize = 8;
pub const PAUSE_WORD_START_LETTERS: usize = 2;
pub const PAUSE_WORD_START_MIN_WORDS: u32 = 100;
pub const REPLAYED_SIGNS_MIN_STEM_LETTERS: usize = 2;
pub const STRANDED_TERM_MAX_WORDS: usize = 3;
pub const TRUSTED_SHORT_WORD_MAX_LENGTH: usize = 2;
pub const UNSCORED_CORRECTION_CONFIDENCE: f64 = 99.0;
pub const LEGACY_RULE_CONFIRMATIONS_REQUIRED: i64 = 2;
pub const LOGGED_SCORE_DECIMALS: i64 = 6;
pub const PREFIX_MIN_CHARACTERS: usize = 4;
pub const BASIC_MULTILINGUAL_PLANE_MAX_CODEPOINT: u32 = 65535;
pub const ACTION_TIMEOUT_SECONDS: f64 = 2.0;
pub const CONTEXT_WAIT_SETTLE_MIN_SECONDS: f64 = 1.5;
pub const DOUBLE_CONVERT_PRESS_WINDOW_SECONDS: f64 = 0.5;
pub const ENGINE_LOOP_MAX_WAKE_SECONDS: f64 = 0.5;
pub const ENGINE_LOOP_MIN_WAKE_SECONDS: f64 = 0.01;
pub const ENGINE_SWITCH_GRACE_SECONDS: f64 = 1.5;
pub const ENGINE_WORKER_JOIN_TIMEOUT_SECONDS: f64 = 2.0;
pub const FIELD_CATCH_UP_RETRY_SECONDS: f64 = 0.025;
pub const HOOK_SILENCE_MS: i64 = 10000;
pub const INPUT_DELAY_REPORT_INTERVAL_SECONDS: f64 = 60.0;
pub const LATE_STROKE_GRACE_SECONDS: f64 = 0.5;
/// The layout switch key (`detection.layout_switch_key`) switches the layout when it comes up at most this long after it
/// went down. Longer, it was held for something else: a click, a selection, a key the person thought better of. The
/// window of a double press of the conversion hotkey (`DOUBLE_CONVERT_PRESS_WINDOW_SECONDS`) is as long: a quick press
/// takes a person well under it.
pub const LAYOUT_TAP_MAX_SECONDS: f64 = 0.5;
pub const LEARNING_PROMPT_TIMEOUT_SECONDS: f64 = 8.0;
pub const MANUAL_RELEASE_TIMEOUT_SECONDS: f64 = 3.0;
pub const PAUSE_CORRECTION_AGREEMENT_SECONDS: f64 = 1.5;
pub const STALE_PRESS_SECONDS: f64 = 3.0;
pub const UNDO_AVAILABLE_WINDOW_SECONDS: f64 = 10.0;
pub const FIELD_READER_RETRY_DELAYS_SECONDS: [f64; 3] = [5.0, 15.0, 60.0];
pub const TYPO_ENGLISH_LOCALE: &str = "en_US";
pub const MILLISECONDS_PER_SECOND: f64 = 1000.0;
