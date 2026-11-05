//! Values used by the modules of this crate, each once, as in the Python's `constants` folder (the file each group
//! comes from is named on top of it).

use serde_json::{json, Map, Value};
use std::sync::LazyLock;

// ---- constants/units.py
pub const BYTES_PER_KIBIBYTE: u64 = 1024;
pub const BYTES_PER_MEBIBYTE: u64 = BYTES_PER_KIBIBYTE * BYTES_PER_KIBIBYTE;

// ---- constants/file_formats.py
/// Indentation of the settings and learning files the application keeps for the user.
pub const USER_DATA_JSON_INDENT: usize = 2;
/// Decimal places a correction's confidence keeps in the history file.
pub const HISTORY_CONFIDENCE_DECIMALS: i64 = 2;
/// `schema_version` of the learning file: 3 keeps a list of rules from the rule window; 2 and older
/// kept counters of manual conversions and forbidden directions, converted on first load.
pub const LEARNING_STORE_SCHEMA_VERSION: i64 = 3;
/// Name suffix of the copy an older learning file is kept as before it is converted.
pub const LEARNING_STORE_BACKUP_SUFFIX: &str = ".v2-backup";
/// A settings or learning file that could not be read is moved aside under its name, this suffix
/// and the local time in `UNREADABLE_FILE_TIME_FORMAT`, so the defaults saved next do not replace it.
pub const UNREADABLE_FILE_SUFFIX: &str = ".unreadable-";
pub const UNREADABLE_FILE_TIME_FORMAT: &str = "%Y%m%d-%H%M%S";
/// The confirmation threshold a learning file of schema 2 was used with unless the settings said
/// otherwise (the removed `detection.learning_confirmations` defaulted to it): a counter that reached
/// it acted as a rule and is carried over as one.
pub const LEGACY_RULE_CONFIRMATIONS_REQUIRED: i64 = 2;

// ---- constants/log_files.py
/// Decimal places kept when a probability, score or confidence is logged.
pub const LOGGED_SCORE_DECIMALS: i64 = 6;
/// Rotation budgets: (bytes per file, backup files), excluding the active file.
pub const DEFAULT_LOG_ROTATION: (u64, u64) = (BYTES_PER_MEBIBYTE, 2);
/// The diagnostics mode keeps more and larger files because a busy hour of typing fills megabytes.
pub const TECHNICAL_LOG_ROTATION: (u64, u64) = (5 * BYTES_PER_MEBIBYTE, 5);
/// Words that look like a password go into the log as stars (`log_privacy)`: a token of at least this many characters,
/// edge signs aside, that mixes letters and digits, and the word right after one of these words in either layout. The
/// last few such words are remembered for the records that name them again (a correction, an edit).
pub const LOG_SECRET_MIN_CHARACTERS: usize = 8;
pub const LOG_SECRET_REMEMBERED_WORDS: usize = 16;
pub const LOG_SECRET_MARKERS: [&str; 7] =
    ["пароль", "пароля", "паролю", "паролем", "пароле", "password", "passwd"];
/// What a technical record holds besides typed text, which the masking leaves as it is: the event's name, versions,
/// checksums, the application's name and the states of the models and the field reader.
pub const LOG_STRUCTURE_KEYS: [&str; 10] = [
    "event", "schema", "application", "backend", "build_platform", "intent_model", "context_model_status",
    "language_models", "field_reader_details", "field_reader_status",
];
pub const LOG_STRUCTURE_KEY_SUFFIXES: [&str; 5] = ["version", "sha256", "checksum", "_id", "path"];
/// The record's own word and its readings, masked when the word before it was one of `LOG_SECRET_MARKERS`.
pub const LOG_WORD_KEYS: [&str; 5] = ["original", "replacement", "literal_head", "alternatives", "word"];

// ---- constants/text.py
/// The engine backs up simple single-key text edits; anything above the Basic Multilingual Plane is
/// composed text a backspace cannot safely undo alone.
pub const BASIC_MULTILINGUAL_PLANE_MAX_CODEPOINT: u32 = 0xFFFF;
/// A string setting value longer than this is shortened (with "...") before it is logged.
pub const LOGGED_SETTING_VALUE_MAX_CHARACTERS: usize = 80;
/// The longest selection a selection hotkey changes (layout, case, transliteration). The replacement is typed key by
/// key, so the bound keeps one press of the hotkey within a moment of typing, and it bounds the read too: UI
/// Automation, AT-SPI and the clipboard hand over no more than one character past it.
pub const SELECTION_COMMAND_MAX_CHARACTERS: usize = 1000;
/// A word counts as typed with Caps Lock on (`пРИВЕТ`: small first letter, capitals after it) only with at least this
/// many capitals after the first letter: shorter words of that shape are names written that way (`iOS`, `eSIM`), and
/// the case command takes them through its plain cycle.
pub const CAPS_LOCK_WORD_MIN_CAPITALS: usize = 4;

// ---- constants/typo.py
/// The personal dictionary (`personal_words.py)`: a word left as typed this many times is the user's own and never
/// corrected again; one whose correction the user took back is theirs at once.
pub const PERSONAL_WORD_MIN_COUNT: i64 = 2;
/// Most of the user's own words kept, and most words counted on their way there; past either, the entry used longest
/// ago goes. The file stays well under a mebibyte.
pub const PERSONAL_WORDS_MAX: usize = 20_000;
pub const PERSONAL_WORD_COUNTS_MAX: usize = 5_000;
/// A word is stored as the first hexadecimal digits of the SHA-256 of a per-installation salt and the word: 64 bits,
/// so two words of a full dictionary share a hash with odds below one in ten billion.
pub const PERSONAL_WORD_HASH_HEX_CHARACTERS: usize = 16;
/// Random bytes of the salt, drawn once per installation and kept in the same file.
pub const PERSONAL_WORD_SALT_BYTES: usize = 16;
pub const PERSONAL_WORDS_SCHEMA_VERSION: i64 = 1;

// ---- constants/detection.py
/// Most strokes the engine keeps for one word.
pub const MAX_WORD_STROKES: usize = 256;

// ---- constants/keyboard.py
/// Short label of each language layout group.
pub const LAYOUT_LABELS: [(i64, &str); 2] = [(0, "EN"), (1, "RU")];
/// Readable name of each language layout group.
pub const LAYOUT_NAMES: [(i64, &str); 2] = [(0, "английский (EN)"), (1, "русский (RU)")];

// ---- constants/settings_defaults.py
/// Default of `detection.minimum_length`; every fallback to the setting uses it.
pub const DEFAULT_MINIMUM_WORD_LENGTH: i64 = 3;
/// Default of detection.confidence; every fallback to the setting uses it.
pub const DEFAULT_CONFIDENCE_THRESHOLD: f64 = 2.0;
/// Default of `detection.pause_delay_seconds`; every fallback to the setting uses it. A word typed in the
/// wrong layout whose first letters the early switch cannot yet decide is converted after this pause, and
/// the user watching it waited the whole pause in the middle of the word: 1.5 s until schema 8.
pub const DEFAULT_PAUSE_DELAY_SECONDS: f64 = 0.7;
/// The pause delay schema 7 and older seeded the settings file with; such a file adopts the new default once.
pub const PREVIOUS_DEFAULT_PAUSE_DELAY_SECONDS: f64 = 1.5;
/// `detection.field_catch_up`: how long a correction may wait for an editor that shows a word a moment after its keys
/// (a browser or an Electron editor on Windows reports its text late), in milliseconds, or "off". A correction is made
/// only once the field shows the very word typed; one that never does is left as typed, as before the setting.
pub const FIELD_CATCH_UP_OFF: &str = "off";
pub const DEFAULT_FIELD_CATCH_UP: &str = "300";
/// Default of `detection.early_switch_min_length`; every fallback to the setting uses it.
pub const DEFAULT_EARLY_SWITCH_MIN_LENGTH: i64 = 4;
/// Default of history.limit; every fallback to the setting uses it.
pub const DEFAULT_HISTORY_LIMIT: i64 = 200;
/// Settings files written by schema 6 carry `early_switch: false` as the seeded default of
/// the releases that kept the early switch off; loading them adopts the new default once.
pub const EARLY_SWITCH_DEFAULT_SCHEMA_VERSION: i64 = 7;
/// Settings files written by schema 7 or older carry the old pause delay as its seeded default; loading
/// them adopts the shorter one once.
pub const PAUSE_DELAY_DEFAULT_SCHEMA_VERSION: i64 = 8;
/// The hotkeys that act on the selected text, by their names under `hotkeys`. A settings file written before they
/// existed gains them from the defaults when it is merged over them (`config::deep_merge`), so they need no schema
/// step of their own.
pub const HOTKEY_SELECTION_LAYOUT: &str = "selection_layout";
pub const HOTKEY_SELECTION_CASE: &str = "selection_case";
pub const HOTKEY_SELECTION_TRANSLITERATE: &str = "selection_transliterate";
/// Their defaults are the ones Punto Switcher has for the same commands, so a user coming from it finds them where
/// the hands expect; none of them is taken by another hotkey here.
pub const DEFAULT_SELECTION_LAYOUT_HOTKEY: &str = "Shift+Pause";
pub const DEFAULT_SELECTION_CASE_HOTKEY: &str = "Alt+Pause";
pub const DEFAULT_SELECTION_TRANSLITERATE_HOTKEY: &str = "Alt+Scroll_Lock";
/// A Mac keyboard has neither Pause nor Scroll Lock, and the macOS backend names no such key. There the selection
/// hotkeys default to Control+Option with a letter, the modifiers of the shipped toggle and undo hotkeys.
/// `config::default_settings` puts them in place of the others on macOS.
pub const MACOS_SELECTION_HOTKEY_DEFAULTS: [(&str, &str); 3] = [
    (HOTKEY_SELECTION_LAYOUT, "Ctrl+Alt+L"),
    (HOTKEY_SELECTION_CASE, "Ctrl+Alt+U"),
    (HOTKEY_SELECTION_TRANSLITERATE, "Ctrl+Alt+T"),
];

/// The shipped settings; the persisted file is seeded from it and merged over it. Key order is the file's order.
pub static DEFAULT_SETTINGS: LazyLock<Map<String, Value>> = LazyLock::new(|| {
    let value = json!({
        "schema_version": PAUSE_DELAY_DEFAULT_SCHEMA_VERSION,
        "enabled": true,
        "general": {
            "start_hidden": true,
            "close_to_tray": true,
            "autostart": true,
            "notifications": true,
            "sound": false,
            "keep_history": true,
        },
        "detection": {
            "layouts": ["us", "ru"],
            "language_models": ["en_US", "ru_RU"],
            "minimum_length": DEFAULT_MINIMUM_WORD_LENGTH,
            "confidence": DEFAULT_CONFIDENCE_THRESHOLD,
            "correct_on_pause": true,
            "pause_delay_seconds": DEFAULT_PAUSE_DELAY_SECONDS,
            // On by default since the context-v3 + prefix-v2 pair passed its sealed test in this mode
            // with no corruption of correctly typed text; one switch away in the settings.
            "early_switch": true,
            "early_switch_min_length": DEFAULT_EARLY_SWITCH_MIN_LENGTH,
            "correct_on_space": true,
            "correct_on_enter": true,
            "correct_on_tab": true,
            "correct_on_punctuation": true,
            "respect_manual_layout": true,
            // A word kept in its layout and typed with two capitals at its start (`ПРивет`, `HEllo`) gets
            // its second letter lowercase when the lexicon knows the word so written (engine fix_double_capitals).
            "fix_double_capitals": true,
            "aggressive": true,
            "context_aware": true,
            "context_policy": "assist",
            "context_read_field": true,
            "hold_after_caret_move": true,
            "field_catch_up": DEFAULT_FIELD_CATCH_UP,
            "protect_code": true,
            "intent_model_enabled": true,
            // Typo correction at a word's boundary (typo_correction): one slip in a word nothing knows, undone when
            // one frequent word is far more likely than any other.
            "correct_typos": true,
            // English words are corrected too only when the user asks for it: in technical text and code an unknown
            // English word is more often an identifier than a slip (constants.typo.TYPO_ENGLISH_LOCALE).
            "correct_typos_english": false,
            // Switching rules from the rule window act, and a double press of the conversion hotkey offers one.
            "learning": true,
            // Keys other programs inject (remote control, macros, on-screen keyboards) count as typing on this
            // keyboard. Off: the machine where they are typed has its own switcher, and waiting for their key-ups
            // held Enter back.
            "injected_input": false,
        },
        "hotkeys": {
            "toggle": "Ctrl+Alt+P",
            "convert_last": "Pause",
            "undo": "Ctrl+Alt+Z",
            HOTKEY_SELECTION_LAYOUT: DEFAULT_SELECTION_LAYOUT_HOTKEY,
            HOTKEY_SELECTION_CASE: DEFAULT_SELECTION_CASE_HOTKEY,
            HOTKEY_SELECTION_TRANSLITERATE: DEFAULT_SELECTION_TRANSLITERATE_HOTKEY,
        },
        "applications": {
            // Per-application input conventions; see app_quirks.
            "telegram_quote_mention": true,
        },
        "exclusions": {
            "applications": ["keepassxc", "1password", "bitwarden"],
            "words": [],
        },
        "appearance": {
            "theme": "system",
            "show_indicator": true,
            "indicator_style": "letters",
        },
        "updates": {
            "check_automatically": true,
            "install_automatically": true,
        },
        "diagnostics": {
            "technical_logging": false,
        },
        "history": {"limit": DEFAULT_HISTORY_LIMIT},
    });
    match value {
        Value::Object(map) => map,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => unreachable!("the defaults are an object"),
    }
});
