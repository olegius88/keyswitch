"""Default values of user settings and the ranges and steps the settings allow."""

from __future__ import annotations

from typing import Final

# Default of detection.minimum_length; every fallback to the setting uses it.
DEFAULT_MINIMUM_WORD_LENGTH: Final = 3
# Default of detection.confidence; every fallback to the setting uses it.
DEFAULT_CONFIDENCE_THRESHOLD: Final = 2.0
# Default of detection.pause_delay_seconds; every fallback to the setting uses it.
DEFAULT_PAUSE_DELAY_SECONDS: Final = 1.5
# Default of detection.early_switch_min_length; every fallback to the setting uses it.
DEFAULT_EARLY_SWITCH_MIN_LENGTH: Final = 4
# Default of detection.learning_confirmations; every fallback to the setting uses it.
DEFAULT_LEARNING_CONFIRMATIONS: Final = 2
# Default of history.limit; every fallback to the setting uses it.
DEFAULT_HISTORY_LIMIT: Final = 200
# The shipped settings; the persisted file is seeded from it and merged over it.
DEFAULT_SETTINGS: Final[dict[str, object]] = {
    "schema_version": 6,
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
        "early_switch": False,
        "early_switch_min_length": DEFAULT_EARLY_SWITCH_MIN_LENGTH,
        "correct_on_space": True,
        "correct_on_enter": True,
        "correct_on_tab": True,
        "correct_on_punctuation": True,
        "respect_manual_layout": True,
        "aggressive": True,
        "context_aware": True,
        "context_policy": "assist",
        "context_read_field": True,
        "hold_after_caret_move": True,
        "protect_code": True,
        "intent_model_enabled": True,
        "learning": True,
        # The threshold a rule must reach to act. Enter on the prompt reaches it
        # at once; rules left half-confirmed by older versions stay inactive.
        "learning_confirmations": DEFAULT_LEARNING_CONFIRMATIONS,
    },
    "hotkeys": {
        "toggle": "Ctrl+Alt+P",
        "convert_last": "Pause",
        "undo": "Ctrl+Alt+Z",
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
# Most confirmations detection.learning_confirmations may ask for: both settings windows offer
# 1 to this and the engine clamps a stored value into it.
LEARNING_CONFIRMATIONS_SETTING_MAX: Final = 10
# Allowed range and step of history.limit in the settings controls (only the Tk window offers it).
HISTORY_LIMIT_SETTING_MIN: Final = 10
HISTORY_LIMIT_SETTING_MAX: Final = 5000
HISTORY_LIMIT_SETTING_STEP: Final = 10
