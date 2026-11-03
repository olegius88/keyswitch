"""Default values of user settings and the ranges and steps the settings allow."""

from __future__ import annotations

from typing import Final

# Default of detection.minimum_length; every fallback to the setting uses it.
DEFAULT_MINIMUM_WORD_LENGTH: Final = 3
# Default of detection.confidence; every fallback to the setting uses it.
DEFAULT_CONFIDENCE_THRESHOLD: Final = 2.0
# Default of detection.pause_delay_seconds; every fallback to the setting uses it. A word typed in the
# wrong layout whose first letters the early switch cannot yet decide is converted after this pause, and
# the user watching it waited the whole pause in the middle of the word: 1.5 s until schema 8.
DEFAULT_PAUSE_DELAY_SECONDS: Final = 0.7
# The pause delay schema 7 and older seeded the settings file with; such a file adopts the new default once.
PREVIOUS_DEFAULT_PAUSE_DELAY_SECONDS: Final = 1.5
# detection.field_catch_up: how long a correction may wait for an editor that shows a word a moment after its keys
# (a browser or an Electron editor on Windows reports its text late), in milliseconds, or "off". A correction is made
# only once the field shows the very word typed; one that never does is left as typed, as before the setting. The
# owner's choice of 20.10.2026: a third of a second by default, a fixed choice or off.
FIELD_CATCH_UP_OFF: Final = "off"
FIELD_CATCH_UP_CHOICES: Final = (FIELD_CATCH_UP_OFF, "100", "300", "500", "1000")
DEFAULT_FIELD_CATCH_UP: Final = "300"
# What both settings windows show for each choice of detection.field_catch_up.
FIELD_CATCH_UP_LABELS: Final = {FIELD_CATCH_UP_OFF: "Не ждать", "100": "0,1 с", "300": "0,3 с", "500": "0,5 с",
                                "1000": "1 с"}
# What both settings windows say of detection.field_catch_up.
FIELD_CATCH_UP_TITLE: Final = "Ждать отстающее поле"
FIELD_CATCH_UP_DESCRIPTION: Final = (
    "Браузеры и редакторы на Electron показывают набранное с задержкой. Сколько ждать, пока поле покажет слово, "
    "прежде чем отказаться от его исправления")
# Default of detection.early_switch_min_length; every fallback to the setting uses it.
DEFAULT_EARLY_SWITCH_MIN_LENGTH: Final = 4
# Default of history.limit; every fallback to the setting uses it.
DEFAULT_HISTORY_LIMIT: Final = 200
# Settings files written by schema 6 carry `early_switch: false` as the seeded default of
# the releases that kept the early switch off; loading them adopts the new default once.
EARLY_SWITCH_DEFAULT_SCHEMA_VERSION: Final = 7
# Settings files written by schema 7 or older carry the old pause delay as its seeded default; loading
# them adopts the shorter one once.
PAUSE_DELAY_DEFAULT_SCHEMA_VERSION: Final = 8
# The hotkeys that act on the selected text, by their names under `hotkeys`: the engine, both settings windows and
# the tests read the names from here. A settings file written before they existed gains them from the defaults when it
# is merged over them (config._deep_merge), so they need no schema step of their own.
HOTKEY_SELECTION_LAYOUT: Final = "selection_layout"
HOTKEY_SELECTION_CASE: Final = "selection_case"
HOTKEY_SELECTION_TRANSLITERATE: Final = "selection_transliterate"
SELECTION_HOTKEYS: Final = (HOTKEY_SELECTION_LAYOUT, HOTKEY_SELECTION_CASE, HOTKEY_SELECTION_TRANSLITERATE)
# Their defaults are the ones Punto Switcher has for the same commands (its hotkey table: "Сменить раскладку
# выделенного текста | Shift + Break", "Сменить регистр выделенного текста | Alt + Break", "Транслитерировать
# выделенный текст | Alt + Scroll Lock", https://yandex.ru/support2/punto-win/ru/settings/hotkeys), so a user coming
# from it finds them where the hands expect; none of them is taken by another hotkey here.
DEFAULT_SELECTION_LAYOUT_HOTKEY: Final = "Shift+Pause"
DEFAULT_SELECTION_CASE_HOTKEY: Final = "Alt+Pause"
DEFAULT_SELECTION_TRANSLITERATE_HOTKEY: Final = "Alt+Scroll_Lock"
# A Mac keyboard has neither Pause nor Scroll Lock, and the macOS backend names no such key. There the selection hotkeys
# default to Control+Option with a letter, the modifiers of the shipped toggle and undo hotkeys; none of these is on
# Apple's list of Mac keyboard shortcuts (https://support.apple.com/en-us/102650). config.default_settings puts them in
# place of the others on macOS.
MACOS_SELECTION_HOTKEY_DEFAULTS: Final = {
    HOTKEY_SELECTION_LAYOUT: "Ctrl+Alt+L",
    HOTKEY_SELECTION_CASE: "Ctrl+Alt+U",
    HOTKEY_SELECTION_TRANSLITERATE: "Ctrl+Alt+T",
}
# What both settings windows call the selection hotkeys and say of them.
SELECTION_HOTKEY_TITLES: Final = {
    HOTKEY_SELECTION_LAYOUT: "Сменить раскладку выделенного текста",
    HOTKEY_SELECTION_CASE: "Сменить регистр выделенного текста",
    HOTKEY_SELECTION_TRANSLITERATE: "Транслитерировать выделенный текст",
}
SELECTION_HOTKEY_DESCRIPTIONS: Final = {
    HOTKEY_SELECTION_LAYOUT: (
        "Выделенный текст, набранный не в той раскладке, перепечатывается в другой (ghbdtn → привет) и раскладка "
        "переключается на неё. В выделении из двух алфавитов перепечатываются слова того, которого меньше "
        "(привет vbh → привет мир)"),
    HOTKEY_SELECTION_CASE: (
        "По кругу: строчные → ПРОПИСНЫЕ → Каждое Слово С Прописной → строчные. Текст, набранный с включённым "
        "Caps Lock (пРИВЕТ), исправляется сразу"),
    HOTKEY_SELECTION_TRANSLITERATE: (
        "Кириллица латиницей по таблице ICAO Doc 9303 (Щука → Shchuka) и латиница кириллицей "
        "(Privet → Привет). Мягкий знак эта таблица не передаёт"),
}
# The shipped settings; the persisted file is seeded from it and merged over it.
DEFAULT_SETTINGS: Final[dict[str, object]] = {
    "schema_version": PAUSE_DELAY_DEFAULT_SCHEMA_VERSION,
    "enabled": True,
    "general": {
        "start_hidden": True,
        "close_to_tray": True,
        "autostart": True,
        "notifications": True,
        "sound": False,
        "keep_history": True,
    },
    "detection": {
        "layouts": ["us", "ru"],
        "language_models": ["en_US", "ru_RU"],
        "minimum_length": DEFAULT_MINIMUM_WORD_LENGTH,
        "confidence": DEFAULT_CONFIDENCE_THRESHOLD,
        "correct_on_pause": True,
        "pause_delay_seconds": DEFAULT_PAUSE_DELAY_SECONDS,
        # On by default since the context-v3 + prefix-v2 pair passed its sealed test in this mode
        # with no corruption of correctly typed text (02.10.2026); one switch away in the settings.
        "early_switch": True,
        "early_switch_min_length": DEFAULT_EARLY_SWITCH_MIN_LENGTH,
        "correct_on_space": True,
        "correct_on_enter": True,
        "correct_on_tab": True,
        "correct_on_punctuation": True,
        "respect_manual_layout": True,
        # A word kept in its layout and typed with two capitals at its start (`ПРивет`, `HEllo`) gets
        # its second letter lowercase when the lexicon knows the word so written (engine._fix_double_capitals).
        "fix_double_capitals": True,
        "aggressive": True,
        "context_aware": True,
        "context_policy": "assist",
        "context_read_field": True,
        "hold_after_caret_move": True,
        "field_catch_up": DEFAULT_FIELD_CATCH_UP,
        "protect_code": True,
        "intent_model_enabled": True,
        # Typo correction at a word's boundary (typo_correction): one slip in a word nothing knows, undone when one
        # frequent word is far more likely than any other.
        "correct_typos": True,
        # English words are corrected too only when the user asks for it: in technical text and code an unknown English
        # word is more often an identifier than a slip (constants.typo.TYPO_ENGLISH_LOCALE).
        "correct_typos_english": False,
        # Switching rules from the rule window act, and a double press of the
        # conversion hotkey offers one.
        "learning": True,
        # Keys other programs inject (remote control, macros, on-screen keyboards)
        # count as typing on this keyboard. Off: the machine where they are typed
        # has its own switcher, and waiting for their key-ups held Enter back.
        "injected_input": False,
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
        # Per-application input conventions; see keyswitch.app_quirks.
        "telegram_quote_mention": True,
    },
    "exclusions": {
        "applications": ["keepassxc", "1password", "bitwarden"],
        "words": [],
    },
    "appearance": {
        "theme": "system",
        "show_indicator": True,
        "indicator_style": "letters",
    },
    "updates": {
        "check_automatically": True,
        "install_automatically": True,
    },
    "diagnostics": {
        "technical_logging": False,
    },
    "history": {"limit": DEFAULT_HISTORY_LIMIT},
}
# Allowed range of detection.early_switch_min_length: the settings controls offer it and the engine
# clamps a stored value into it.
EARLY_SWITCH_MIN_LENGTH_SETTING_MIN: Final = 3
EARLY_SWITCH_MIN_LENGTH_SETTING_MAX: Final = 8
# Allowed range of detection.minimum_length.
MINIMUM_WORD_LENGTH_SETTING_MIN: Final = 2
MINIMUM_WORD_LENGTH_SETTING_MAX: Final = 12
# Allowed range and step of detection.confidence: both settings windows offer it and the engine
# clamps a stored value into it.
CONFIDENCE_SETTING_MIN: Final = 0.5
CONFIDENCE_SETTING_MAX: Final = 10.0
CONFIDENCE_SETTING_STEP: Final = 0.1
# Allowed range and step of detection.pause_delay_seconds: both settings windows offer it and the
# engine clamps a stored value into it, so a malformed setting can neither stop pause correction
# nor fire it constantly.
PAUSE_DELAY_SETTING_MIN_SECONDS: Final = 0.3
PAUSE_DELAY_SETTING_MAX_SECONDS: Final = 5.0
PAUSE_DELAY_SETTING_STEP_SECONDS: Final = 0.1
# Allowed range and step of history.limit in the settings controls (only the Tk window offers it).
HISTORY_LIMIT_SETTING_MIN: Final = 10
HISTORY_LIMIT_SETTING_MAX: Final = 5000
HISTORY_LIMIT_SETTING_STEP: Final = 10
