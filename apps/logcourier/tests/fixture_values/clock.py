"""Fake timestamps and durations used by tests."""

from __future__ import annotations

from typing import Final

from logcourier.constants.telegram import GROUP_INTERVAL

FIXTURE_CLOCK_START: Final = 100.0
# Clock readings of three consecutive Telegram mutations paced GROUP_INTERVAL apart from the
# fixture clock: marker, data and catalog of a delivery, or two uploads and a pin.
EXPECTED_PACED_MUTATION_TIMES: Final = [
    FIXTURE_CLOCK_START,
    FIXTURE_CLOCK_START + GROUP_INTERVAL,
    FIXTURE_CLOCK_START + GROUP_INTERVAL + GROUP_INTERVAL,
]
# Telegram's reported retry_after, fixture-only.
BATCHING_RETRY_AFTER_SECONDS: Final = 37
# clock() + delay, where delay = max(1, RETRY_AFTER_SECONDS) + 1 (see RateLimitedClient._perform).
EXPECTED_COOLDOWN: Final = 138
# A clock reading for a second pacing group, still short of the first group's cooldown.
OTHER_GROUP_CLOCK: Final = 110.0
# A "next allowed send" timestamp comfortably past the fixture clock, so the pacing wait is still in
# effect when cancellation is checked.
PACING_UNTIL_TIMESTAMP: Final = 200
# An arbitrary retry_at offset, unrelated to RETRY_AFTER_SECONDS above.
RETRY_AT_OFFSET_SECONDS: Final = 37
SERVICE_BACKOFF_RETRY_AFTER_SECONDS: Final = 60
TELEGRAM_FAKE_RETRY_AFTER_SECONDS: Final = 42
