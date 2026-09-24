"""Bounded worker-side recovery without retrying missing dependencies/access."""
from __future__ import annotations

import unittest
from unittest.mock import MagicMock, patch

from keyswitch.context_access import MILLISECONDS_PER_SECOND, RETRY_DELAYS, PlatformFieldReader
from keyswitch.input_context import FieldContext

RETRY_LIMIT = len(RETRY_DELAYS)
FIRST_RETRY_AFTER_MS = round(RETRY_DELAYS[0] * MILLISECONDS_PER_SECOND)
SECOND_RETRY_AFTER_MS = round(RETRY_DELAYS[1] * MILLISECONDS_PER_SECOND)
MAX_RETRY_AFTER_MS = round(RETRY_DELAYS[2] * MILLISECONDS_PER_SECOND)
ONE_RETRY_CALL_COUNT = 2
CALL_COUNT_AFTER_ALL_RETRIES = 1 + RETRY_LIMIT
CALL_COUNT_AFTER_ONE_MORE_CAPPED_RETRY = CALL_COUNT_AFTER_ALL_RETRIES + 1
CALL_COUNT_AFTER_REOPEN = CALL_COUNT_AFTER_ONE_MORE_CAPPED_RETRY + 1
ATTEMPTS_AFTER_ONE_MORE_CAPPED_RETRY = RETRY_LIMIT + 1

FIXTURE_START_SECONDS = 100.0
JUST_BEFORE_FIRST_RETRY_SECONDS = 104.999
FIRST_RETRY_DUE_SECONDS = 105.0
SECOND_RETRY_DUE_SECONDS = 120.0
THIRD_RETRY_DUE_SECONDS = 180.0
CAPPED_RETRY_DUE_SECONDS = 240.0
FAR_FUTURE_SECONDS = 5000.0
OVERDUE_SECONDS = 1000.0


class ReaderRecoveryTests(unittest.TestCase):
    def test_transient_failure_reopens_provider_after_backoff_not_on_each_key(self) -> None:
        reader = PlatformFieldReader()
        failed, recovered = MagicMock(), MagicMock()
        failed.read.side_effect = RuntimeError("private provider details")
        field = FieldContext("editor", "field", "example")
        recovered.read.return_value = field
        with patch("keyswitch.context_access.sys.platform", "win32"), patch(
            "keyswitch.windows_context.WindowsFieldReader", side_effect=[failed, recovered],
        ) as factory, patch("time.monotonic", return_value=FIXTURE_START_SECONDS) as clock:
            self.assertIsNone(reader.read("editor", 1))
            self.assertEqual(reader.retry_diagnostics(), {"attempts": 0, "limit": RETRY_LIMIT, "after_ms": FIRST_RETRY_AFTER_MS})
            clock.return_value = JUST_BEFORE_FIRST_RETRY_SECONDS
            self.assertIsNone(reader.read("editor", 1))
            failed.close.assert_not_called()
            factory.assert_called_once()
            clock.return_value = FIRST_RETRY_DUE_SECONDS
            self.assertEqual(reader.read("editor", 1), field)
            failed.close.assert_called_once()
            self.assertEqual(factory.call_count, ONE_RETRY_CALL_COUNT)
            self.assertEqual(reader.status, "available")
            self.assertIsNone(reader.diagnostics()["failure_type"])
            self.assertEqual(reader.retry_diagnostics(), {"attempts": 0, "limit": RETRY_LIMIT, "after_ms": None})

    def test_attempts_slow_down_but_never_stop_for_a_transient_failure(self) -> None:
        """A provider that is silent now may answer in the next window.

        The delays grow to a minute and stay there: the reader keeps a way back
        instead of switching itself off for the rest of the session, which is
        what a single unreadable field used to do.
        """

        reader = PlatformFieldReader()
        with patch("keyswitch.context_access.sys.platform", "win32"), patch(
            "keyswitch.windows_context.WindowsFieldReader", side_effect=RuntimeError("unavailable"),
        ) as factory, patch("time.monotonic", return_value=FIXTURE_START_SECONDS) as clock:
            for instant in (FIXTURE_START_SECONDS, FIRST_RETRY_DUE_SECONDS, SECOND_RETRY_DUE_SECONDS, THIRD_RETRY_DUE_SECONDS):
                clock.return_value = instant
                self.assertIsNone(reader.read("editor", 1))
            self.assertEqual(factory.call_count, CALL_COUNT_AFTER_ALL_RETRIES)  # Initial call + three retries.
            self.assertEqual(reader.retry_diagnostics(), {"attempts": RETRY_LIMIT, "limit": RETRY_LIMIT, "after_ms": MAX_RETRY_AFTER_MS})
            clock.return_value = CAPPED_RETRY_DUE_SECONDS
            self.assertIsNone(reader.read("editor", 1))
            self.assertEqual(factory.call_count, CALL_COUNT_AFTER_ONE_MORE_CAPPED_RETRY)
            self.assertEqual(reader.retry_diagnostics(), {"attempts": ATTEMPTS_AFTER_ONE_MORE_CAPPED_RETRY, "limit": RETRY_LIMIT, "after_ms": MAX_RETRY_AFTER_MS})
            reader.close()
            self.assertEqual(reader.retry_diagnostics(), {"attempts": 0, "limit": RETRY_LIMIT, "after_ms": None})
            self.assertIsNone(reader.read("editor", 1))
            self.assertEqual(factory.call_count, CALL_COUNT_AFTER_REOPEN)

    def test_missing_dependency_and_explicit_permission_failure_are_not_retried(self) -> None:
        for error in (ImportError, PermissionError):
            with self.subTest(error=error.__name__):
                reader = PlatformFieldReader()
                with patch("keyswitch.context_access.sys.platform", "win32"), patch(
                    "keyswitch.windows_context.WindowsFieldReader", side_effect=error("not available"),
                ) as factory, patch("time.monotonic", return_value=FIXTURE_START_SECONDS) as clock:
                    self.assertIsNone(reader.read("editor", 1))
                    clock.return_value = FAR_FUTURE_SECONDS
                    self.assertIsNone(reader.read("editor", 1))
                    factory.assert_called_once()

    def test_a_bridge_that_cannot_be_closed_is_dropped_not_reused(self) -> None:
        reader = PlatformFieldReader()
        with patch("keyswitch.context_access.sys.platform", "win32"), patch(
            "keyswitch.windows_context.WindowsFieldReader",
        ) as factory, patch("time.monotonic", return_value=FIXTURE_START_SECONDS) as clock:
            factory.return_value.read.side_effect = RuntimeError("private field text")
            factory.return_value.close.side_effect = RuntimeError("private cleanup details")
            self.assertIsNone(reader.read("editor", 1))
            clock.return_value = FIRST_RETRY_DUE_SECONDS
            self.assertIsNone(reader.read("editor", 1))
            failed = reader.diagnostics()
            self.assertEqual(
                {key: failed[key] for key in ("status", "failure_stage", "failure_type", "failure_name")},
                {"status": "unavailable", "failure_stage": "recovery",
                 "failure_type": "runtime_error", "failure_name": "RuntimeError"},
            )
            self.assertEqual(reader.retry_diagnostics(), {"attempts": 1, "limit": RETRY_LIMIT, "after_ms": SECOND_RETRY_AFTER_MS})
            # The broken bridge is released, so the next attempt builds a new one
            # instead of calling close() on the same object again.
            clock.return_value = FAR_FUTURE_SECONDS
            self.assertIsNone(reader.read("editor", 1))
            self.assertEqual(factory.call_count, ONE_RETRY_CALL_COUNT)
            factory.return_value.close.assert_called_once()

    def test_overdue_retry_does_not_read_without_application_and_window(self) -> None:
        reader = PlatformFieldReader()
        with patch("keyswitch.context_access.sys.platform", "win32"), patch(
            "keyswitch.windows_context.WindowsFieldReader", side_effect=RuntimeError("temporary"),
        ) as factory, patch("time.monotonic", return_value=FIXTURE_START_SECONDS) as clock:
            self.assertIsNone(reader.read("editor", 1))
            clock.return_value = OVERDUE_SECONDS
            self.assertEqual(reader.retry_diagnostics()["after_ms"], 0)
            self.assertIsNone(reader.read("", 1))
            self.assertIsNone(reader.read("editor", 0))
            factory.assert_called_once()


if __name__ == "__main__":
    unittest.main()
