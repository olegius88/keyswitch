"""Log file sizes and rotation."""

from __future__ import annotations

from typing import Final

from .units import BYTES_PER_MEBIBYTE

# Decimal places kept when a probability, score or confidence is logged.
LOGGED_SCORE_DECIMALS: Final = 6
# Rotation budgets: (bytes per file, backup files), excluding the active file.
DEFAULT_LOG_ROTATION: Final = (BYTES_PER_MEBIBYTE, 2)
# The diagnostics mode keeps more and larger files because a busy hour of typing fills megabytes.
TECHNICAL_LOG_ROTATION: Final = (5 * BYTES_PER_MEBIBYTE, 5)
# Words that look like a password go into the log as stars (log_privacy): a token of at least this many characters,
# edge signs aside, that mixes letters and digits, and the word right after one of these words in either layout. The
# last few such words are remembered for the records that name them again (a correction, an edit).
LOG_SECRET_MIN_CHARACTERS: Final = 8
LOG_SECRET_REMEMBERED_WORDS: Final = 16
LOG_SECRET_MARKERS: Final = ("пароль", "пароля", "паролю", "паролем", "пароле", "password", "passwd")
# What a technical record holds besides typed text, which the masking leaves as it is: the event's name, versions,
# checksums, the application's name and the states of the models and the field reader.
LOG_STRUCTURE_KEYS: Final = frozenset({
    "event", "schema", "application", "backend", "build_platform", "intent_model", "context_model_status",
    "language_models", "field_reader_details", "field_reader_status",
})
LOG_STRUCTURE_KEY_SUFFIXES: Final = ("version", "sha256", "checksum", "_id", "path")
# The record's own word and its readings, masked when the word before it was one of LOG_SECRET_MARKERS.
LOG_WORD_KEYS: Final = frozenset({"original", "replacement", "literal_head", "alternatives", "word"})
