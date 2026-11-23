//! The values the engine uses, each once, as in the Python's `constants` folder.

pub use ks_backend::constants::{HOOK_GAP_MS, INPUT_DELAY_LATE_MS, INPUT_DELAY_SLOW_CALLBACK_MS, INPUT_DELAY_UNSTAMPED_MS, POINTER_KEY_NAME, WHEEL_KEY_NAME};
pub use ks_config::constants::{CONVERT_LAST_KEYS, DEFAULT_CONVERT_LAST_KEY, DEFAULT_LAYOUT_SWITCH_KEY, EITHER_SHIFT_KEY, LAYOUT_SWITCH_KEYS, SHIFT_KEYS};
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
/// The shortest word whose capitals `fix_double_capitals` fixes: three letters, where Shift is held too long most often
/// (`КАк`, `ТАк`, `ANd`).
pub const DOUBLE_CAPITALS_MIN_LETTERS: usize = 3;
/// From this many letters the frequency lexicon alone decides that a word is fixed; a shorter one must also be among the
/// words the prose of its language is full of (`DOUBLE_CAPITALS_SHORT_WORD_PROSE_MIN_COUNT`).
pub const DOUBLE_CAPITALS_LEXICON_MIN_LETTERS: usize = 4;
/// How often the prose of its language uses a word of fewer than `DOUBLE_CAPITALS_LEXICON_MIN_LETTERS` letters, at
/// least, for its capitals to be fixed. Three letters are as often an abbreviation with an ending, a unit or a plural of
/// an abbreviation, and the lexicon knows many of them in lowercase. In the Russian word lists of the Leipzig corpora
/// (ten corpora of a million sentences each) every three-letter token of the shape whose fixed form the prose counts
/// reach this often was a slip: 21 of them, from `ЧТо` (`что`, 671 941) to `ВНе` (5 094); those written so on purpose
/// stay below a half of it: `ОСи` for operating systems (`оси`, 1 835), `СШа` (591), units (`МГц` 56, `МВт` 0) and plurals
/// of abbreviations (`IDs` 5, `PCs` 0).
pub const DOUBLE_CAPITALS_SHORT_WORD_PROSE_MIN_COUNT: u64 = 5000;
pub const EARLY_SWITCH_CONFIDENCE: f64 = 15.0;
pub const ENGINE_EVENT_QUEUE_MAX_SIZE: usize = 4096;
pub const JOINED_PART_MIN_LETTERS: usize = 2;
/// The layout a head has to be typed in for the parts of a token the model converts whole to be split
/// (`decide_joined_part`): the English one. An English term before a Russian word is common prose, typed with the term's
/// layout (`mongo-cthdbcf` is `mongo-сервиса`, 13.10.2026; `proxy-ghbkj;tybt` is `proxy-приложение`, 09.11.2026). A head
/// typed in the Russian layout that the model keeps when asked alone is mostly a short reading of an English term
/// (`црз-куппук` is `whp-regger` in the owner's typing, 09.11.2026; `мгу` is `vue`), and the whole token was right: there
/// the model's decision on the whole token stands.
pub const JOINED_HEAD_KEPT_GROUP: i64 = 0;
/// The head action logged when the head is not asked because the whole token's decision stands.
pub const JOINED_HEAD_UNASKED_ACTION: &str = "unasked";
/// Why a token typed right after a layout picked by hand stays as typed up to its last joining sign.
pub const MANUAL_LAYOUT_KEPT_REASON: &str = "раскладка выбрана вручную";
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
/// The longest run of letters a pause takes for the start of a word still being typed (`word_begun`); the shortest is
/// `PAUSE_WORD_START_LETTERS`, and as many words begin with either: `PAUSE_WORD_START_MIN_WORDS`, or fewer words that
/// the table of prose counts of the language holds `PAUSE_WORD_START_PROSE_MIN_COUNT` times together: `ес` begins 49 of
/// its words, met 675 158 times (`есть`, `если`), and `оши` 62 words met 159 972 times, while `сс` begins 95 met 79 167
/// times and `смы` 32 met 22 128 times.
pub const PAUSE_WORD_START_MAX_LETTERS: usize = 3;
pub const PAUSE_WORD_START_LETTERS: usize = 2;
pub const PAUSE_WORD_START_MIN_WORDS: u32 = 100;
pub const PAUSE_WORD_START_PROSE_MIN_COUNT: u64 = 100_000;
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
/// The mode of a plan made on the first letters of a word, before the word is over.
pub const EARLY_PLAN_MODE: &str = "early";
/// How long an Enter or Tab the hook keeps back waits behind the letters a correction types again for those letters to
/// come back through the hook, counted from the moment the correction has typed them. On Windows and Wayland they are
/// back by then; on macOS the event tap reports them a moment later. Letters that have not come back by then are not
/// waited for: the key goes on without them.
pub const LATE_KEYS_RETURN_SECONDS: f64 = 0.25;
/// How long a letter that did not come back in that time is still taken for one of those letters when it does come:
/// it stands before the Enter that went on without it, and is no letter of the line after that Enter. Short, because a
/// letter the person types in that time is taken for one of them too, and the word it begins is left as typed.
pub const LATE_KEYS_OWED_SECONDS: f64 = 0.5;
pub const LATE_STROKE_GRACE_SECONDS: f64 = 0.5;
/// The layout switch key (`detection.layout_switch_key`) switches the layout, and the conversion key
/// (`detection.convert_last_key`) converts, when it comes up at most this long after it went down. Longer, it was held for
/// something else: a click, a selection, a key the person thought better of. The window of a double press of the
/// conversion hotkey (`DOUBLE_CONVERT_PRESS_WINDOW_SECONDS`) is as long: a quick press takes a person well under it.
pub const LAYOUT_TAP_MAX_SECONDS: f64 = 0.5;
/// Two taps of the layout switch key are a double tap (`detection.double_tap_converts`) when the second goes down at most
/// this long after the first did, as two presses of the conversion hotkey are a double press.
pub const LAYOUT_DOUBLE_TAP_SECONDS: f64 = DOUBLE_CONVERT_PRESS_WINDOW_SECONDS;
/// Both Shifts are a chord (`detection.both_shifts_toggle`) when the last of them comes up at most this long after the
/// first went down: the two keys of the chord are a tap of both, as quick as a tap of one.
pub const BOTH_SHIFTS_MAX_SECONDS: f64 = LAYOUT_TAP_MAX_SECONDS;
pub const LEARNING_PROMPT_TIMEOUT_SECONDS: f64 = 8.0;
pub const MANUAL_RELEASE_TIMEOUT_SECONDS: f64 = 3.0;
pub const PAUSE_CORRECTION_AGREEMENT_SECONDS: f64 = 1.5;
pub const STALE_PRESS_SECONDS: f64 = 3.0;
pub const UNDO_AVAILABLE_WINDOW_SECONDS: f64 = 10.0;
/// The most keys the undo hotkey types again after the word it puts back (`undo_across_words`): the signs and the boundary
/// after the word and what was typed since, three or four words. Every one of them is deleted and typed again while the
/// keyboard is held, and on X11, where nothing holds it, typed over whatever comes meanwhile.
pub const UNDO_REACH_MAX_KEYS: usize = 24;
/// A Backspace this soon after an automatic correction, and the Backspaces right after it, were pressed before the person
/// could read the corrected word: the edit is of the text as they typed it, often the slip just corrected. Erasing a
/// correction found wrong starts later, once the changed word is read. Such a run reaching a typo correction leaves the
/// word to be corrected as any word, not the person's to finish; reaching a change of layout, it remembers nothing of the
/// word typed again there (`note_backspace`).
pub const ERASE_UNSEEN_SECONDS: f64 = 0.6;
/// The signs, besides letters, digits and spaces, the undo hotkey types again after the word it puts back: those an editor
/// leaves as typed when they stand alone. Brackets and quotes are paired by editors, and `--` or `...` turned into a dash
/// or an ellipsis, so text holding them is not known by its keys.
pub const UNDO_REACH_SIGNS: &str = ".,!?:;-";
/// The mode of the plan of a typo correction (`typo_plan`), which Pause, the undo and the count of what follows ask about.
pub const TYPO_PLAN_MODE: &str = "typo";
pub const FIELD_READER_RETRY_DELAYS_SECONDS: [f64; 3] = [5.0, 15.0, 60.0];
pub const TYPO_ENGLISH_LOCALE: &str = "en_US";
pub const MILLISECONDS_PER_SECOND: f64 = 1000.0;

/// The other names a key of a hotkey goes by, casefolded: a hotkey of Pause also fires on a key named Break, as in the
/// Python version (`Hotkey::matches`), and a keyboard with either has the key (`Hotkey::problem`).
pub const HOTKEY_KEY_ALIASES: [(&str, &[&str]); 1] = [("pause", &["break"])];
/// The modifier keys by the names every backend gives them, casefolded: none of them is the key of a hotkey
/// (`Hotkey::problem`).
pub const HOTKEY_MODIFIER_KEYS: [&str; 10] =
    ["shift_l", "shift_r", "control_l", "control_r", "alt_l", "alt_r", "super_l", "super_r", "meta_l", "meta_r"];
/// What the settings window says of a hotkey that can never fire (`HotkeyProblem::message`).
pub const HOTKEY_NO_KEY_MESSAGE: &str =
    "Сочетанию нужна обычная клавиша: одни Ctrl, Alt, Shift и Super не срабатывают. Например, Ctrl+Alt+K";
pub const HOTKEY_SEVERAL_KEYS_MESSAGE: &str =
    "В сочетании может быть только одна обычная клавиша, остальные — Ctrl, Alt, Shift и Super. Например, Ctrl+Alt+K";
/// `HOTKEY_MESSAGE_KEY_PLACE` stands for the key as it was typed.
pub const HOTKEY_UNKNOWN_KEY_MESSAGE: &str =
    "Клавиши «{key}» программа не знает. Буквы и цифры пишутся латиницей, например Ctrl+Alt+K";
pub const HOTKEY_MESSAGE_KEY_PLACE: &str = "{key}";
/// What the settings window says of a key chosen for a lone tap that the other such setting has already
/// (`single_key_conflict`): `HOTKEY_MESSAGE_KEY_PLACE` stands for the key as the list names it, and
/// `SINGLE_KEY_MESSAGE_SETTING_PLACE` for the title of the setting that has it.
pub const SINGLE_KEY_TAKEN_MESSAGE: &str = "{key} уже выбран в настройке «{setting}»: одна клавиша не может и переключать \
                                            раскладку, и преобразовывать слово. Выберите другую клавишу или сначала \
                                            выключите её там";
pub const SINGLE_KEY_MESSAGE_SETTING_PLACE: &str = "{setting}";
