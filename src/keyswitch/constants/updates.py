"""Update checks: network timeouts, release-notes limits, progress."""

from __future__ import annotations

from typing import Final

from .units import BYTES_PER_KIBIBYTE, BYTES_PER_MEBIBYTE, MILLISECONDS_PER_SECOND, SECONDS_PER_HOUR

# The first automatic update check runs this long after start.
UPDATE_CHECK_INITIAL_DELAY_SECONDS: Final = 30
# Automatic update checks repeat at this interval.
UPDATE_CHECK_INTERVAL_SECONDS: Final = 6 * SECONDS_PER_HOUR
# UPDATE_CHECK_INITIAL_DELAY_SECONDS for Tk's millisecond timers.
UPDATE_CHECK_INITIAL_DELAY_MS: Final = UPDATE_CHECK_INITIAL_DELAY_SECONDS * MILLISECONDS_PER_SECOND
# UPDATE_CHECK_INTERVAL_SECONDS for Tk's millisecond timers.
UPDATE_CHECK_INTERVAL_MS: Final = UPDATE_CHECK_INTERVAL_SECONDS * MILLISECONDS_PER_SECOND
MAX_RELEASE_JSON_BYTES: Final = 1_000_000
MAX_ASSET_BYTES: Final = 300 * BYTES_PER_MEBIBYTE
DOWNLOAD_CHUNK_BYTES: Final = 128 * BYTES_PER_KIBIBYTE
# Default network timeout of one update request.
UPDATE_REQUEST_TIMEOUT_SECONDS: Final = 20.0
MAX_RELEASE_NOTES_CHARACTERS: Final = 4000
PROGRESS_COMPLETE_PERCENT: Final = 100
