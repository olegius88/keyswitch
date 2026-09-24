"""Telegram Bot API limits and statuses."""

from __future__ import annotations

from typing import Final

# at most 15 operations/minute, below Telegram's 20 messages/minute
GROUP_INTERVAL: Final = 4.0
WAIT_POLL_SECONDS: Final = 0.2
TELEGRAM_RATE_LIMIT_STATUS: Final = 429
# used when Telegram's 429 omits retry_after
DEFAULT_RETRY_AFTER_SECONDS: Final = 60
# below the cloud Bot API's 20 MB getFile limit
MAX_DOWNLOAD: Final = 19_000_000
ERROR_BODY_MAX_BYTES: Final = 8192
ERROR_DESCRIPTION_MAX_CHARACTERS: Final = 250
TELEGRAM_RESPONSE_MAX_BYTES: Final = 1024 * 1024
TELEGRAM_MAX_CAPTION_CHARACTERS: Final = 900
TELEGRAM_GET_UPDATES_LIMIT: Final = 100
