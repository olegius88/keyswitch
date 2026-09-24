"""Thresholds and limits of the runtime decisions: engine, context policy, early switch, learning."""

from __future__ import annotations

from typing import Final

# The shortest token word_shaped() can judge: with one character its first- and last-letter checks
# look at the same position.
MINIMUM_SHAPED_TOKEN_CHARACTERS: Final[int] = 2
# Confidence an early (prefix) switch reports.
EARLY_SWITCH_CONFIDENCE: Final = 15.0
# Most strokes the engine keeps for one word.
MAX_WORD_STROKES: Final = 256
# Input events the engine queues before the producer blocks.
ENGINE_EVENT_QUEUE_MAX_SIZE: Final = 4096
# Per-application remembered context words; oldest is dropped past this cap.
MAX_REMEMBERED_APPLICATION_CONTEXTS: Final = 32
# A boundary is natural (not just a configured minimum length) once the word is this long and its
# own-language score is at least this uncertain.
NATURAL_SOURCE_BOUNDARY_MIN_CHARACTERS: Final = 4
NATURAL_SOURCE_BOUNDARY_NGRAM_FLOOR: Final = -0.25
# A correction no model scored - one the user triggered (manual toggle, undo) or one an
# application convention asks for (Telegram's quote shown as "@") - is certain; this stands in
# for the confidence a model would have reported.
UNSCORED_CORRECTION_CONFIDENCE: Final = 99.0
# A replacement needs at least this many letters to be offered as a rule.
MINIMUM_LEARNABLE_LETTERS: Final = 2
# Cap on a learned rule's confirmation counter and on the confirmations a rule may require.
MAX_LEARNING_CONFIRMATIONS: Final = 999
