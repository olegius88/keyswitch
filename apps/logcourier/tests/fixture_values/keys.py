"""Fake keycodes, keysyms, scan codes and window or process ids used by tests."""

from __future__ import annotations

from typing import Final

# Numeric form of the "-100123" supergroup chat_id used across the fixtures below.
PRIVATE_LOGS_CHAT_ID: Final = -100123
# A message id/user id fixture standing in for something outside this delivery's own catalog chain
# (another process's pin, an unrelated sender).
FOREIGN_MESSAGE_ID: Final = 99
FOREIGN_USER_ID: Final = 777
SECRETS_TEST_TOKEN: Final = "123456:" + "B" * 30
TELEGRAM_TEST_TOKEN: Final = "123456:" + "A" * 30
TELEGRAM_TEST_CHAT_ID: Final = -100
WRONG_CHAT_ID: Final = -999
