"""Bounded worker-side recovery without retrying missing dependencies/access."""
from __future__ import annotations

import unittest
from unittest.mock import MagicMock, patch

from keyswitch.context_access import PlatformFieldReader
from keyswitch.input_context import FieldContext


class ReaderRecoveryTests(unittest.TestCase):
    def test_transient_failure_reopens_provider_after_backoff_not_on_each_key(self) -> None:
        reader = PlatformFieldReader()
        failed, recovered = MagicMock(), MagicMock()
        failed.read.side_effect = RuntimeError("private provider details")
        field = FieldContext("editor", "field", "example")
        recovered.read.return_value = field
        with patch("keyswitch.context_access.sys.platform", "win32"), patch(
            "keyswitch.windows_context.WindowsFieldReader", side_effect=[failed, recovered],
        ) as factory, patch("time.monotonic", return_value=100.0) as clock:
            self.assertIsNone(reader.read("editor", 1))
            self.assertEqual(reader.retry_diagnostics(), {"attempts": 0, "limit": 3, "after_ms": 5000})
            clock.return_value = 104.999
            self.assertIsNone(reader.read("editor", 1))
            failed.close.assert_not_called()
            factory.assert_called_once()
            clock.return_value = 105.0
            self.assertEqual(reader.read("editor", 1), field)
            failed.close.assert_called_once()
            self.assertEqual(factory.call_count, 2)
            self.assertEqual(reader.status, "available")
            self.assertIsNone(reader.diagnostics()["failure_type"])
            self.assertEqual(reader.retry_diagnostics(), {"attempts": 0, "limit": 3, "after_ms": None})

    def test_attempts_are_bounded_and_close_explicitly_resets_them(self) -> None:
        reader = PlatformFieldReader()
        with patch("keyswitch.context_access.sys.platform", "win32"), patch(
            "keyswitch.windows_context.WindowsFieldReader", side_effect=RuntimeError("unavailable"),
        ) as factory, patch("time.monotonic", return_value=100.0) as clock:
            for instant in (100.0, 105.0, 120.0, 180.0, 5000.0):
                clock.return_value = instant
                self.assertIsNone(reader.read("editor", 1))
            self.assertEqual(factory.call_count, 4)  # Initial call + three retries.
            self.assertEqual(reader.retry_diagnostics(), {"attempts": 3, "limit": 3, "after_ms": None})
            reader.close()
            self.assertIsNone(reader.read("editor", 1))
            self.assertEqual(factory.call_count, 5)

    def test_missing_dependency_and_explicit_permission_failure_are_not_retried(self) -> None:
        for error in (ImportError, PermissionError):
            with self.subTest(error=error.__name__):
                reader = PlatformFieldReader()
                with patch("keyswitch.context_access.sys.platform", "win32"), patch(
                    "keyswitch.windows_context.WindowsFieldReader", side_effect=error("not available"),
                ) as factory, patch("time.monotonic", return_value=100.0) as clock:
                    self.assertIsNone(reader.read("editor", 1))
                    clock.return_value = 5000.0
                    self.assertIsNone(reader.read("editor", 1))
                    factory.assert_called_once()

    def test_cleanup_failure_stops_recovery_without_opening_another_provider(self) -> None:
        reader = PlatformFieldReader()
        with patch("keyswitch.context_access.sys.platform", "win32"), patch(
            "keyswitch.windows_context.WindowsFieldReader",
        ) as factory, patch("time.monotonic", return_value=100.0) as clock:
            factory.return_value.read.side_effect = RuntimeError("private field text")
            factory.return_value.close.side_effect = RuntimeError("private cleanup details")
            self.assertIsNone(reader.read("editor", 1))
            clock.return_value = 105.0
            self.assertIsNone(reader.read("editor", 1))
            self.assertEqual(reader.diagnostics(), {
                "status": "unavailable", "failure_stage": "recovery", "failure_type": "runtime_error",
            })
            self.assertEqual(reader.retry_diagnostics(), {"attempts": 1, "limit": 3, "after_ms": None})
            clock.return_value = 5000.0
            self.assertIsNone(reader.read("editor", 1))
            factory.assert_called_once()
            factory.return_value.close.assert_called_once()

    def test_overdue_retry_does_not_read_without_application_and_window(self) -> None:
        reader = PlatformFieldReader()
        with patch("keyswitch.context_access.sys.platform", "win32"), patch(
            "keyswitch.windows_context.WindowsFieldReader", side_effect=RuntimeError("temporary"),
        ) as factory, patch("time.monotonic", return_value=100.0) as clock:
            self.assertIsNone(reader.read("editor", 1))
            clock.return_value = 1000.0
            self.assertEqual(reader.retry_diagnostics()["after_ms"], 0)
            self.assertIsNone(reader.read("", 1))
            self.assertIsNone(reader.read("editor", 0))
            factory.assert_called_once()


if __name__ == "__main__":
    unittest.main()
