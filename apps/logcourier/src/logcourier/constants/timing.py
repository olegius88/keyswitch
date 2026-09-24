"""Delays, timeouts, polling intervals and grace periods of the running application."""

from __future__ import annotations

from typing import Final

SECONDS_PER_MINUTE: Final = 60
# background loop tick; matches README's "every 5 seconds"
WAKE_POLL_SECONDS: Final = 5
# retry soon when items remain queued after a send
PENDING_RETRY_SECONDS: Final = 5
MAX_RETRY_DELAY_SECONDS: Final = 900
RETRY_BASE_SECONDS: Final = 15
RETRY_BACKOFF_BASE: Final = 2
RETRY_BACKOFF_MAX_EXPONENT: Final = 6
DB_CONNECT_TIMEOUT_SECONDS: Final = 10
RETENTION_DAYS: Final = 30
SECONDS_PER_DAY: Final = 86400
REQUEST_TIMEOUT_SECONDS: Final = 30
