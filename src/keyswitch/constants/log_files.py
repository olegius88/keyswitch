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
