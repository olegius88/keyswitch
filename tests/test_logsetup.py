"""Log file placement and the rotation budget of the diagnostics mode."""

from __future__ import annotations

import logging
import tempfile
import unittest
from logging.handlers import RotatingFileHandler
from pathlib import Path
from unittest.mock import patch

from keyswitch import __version__, logsetup
from keyswitch.config import SettingsStore


class RotationLimitTests(unittest.TestCase):
    def test_the_diagnostics_mode_gets_the_larger_budget(self) -> None:
        ordinary = logsetup.rotation_limits(False)
        technical = logsetup.rotation_limits(True)
        self.assertEqual(ordinary, logsetup.DEFAULT_ROTATION)
        self.assertEqual(technical, logsetup.TECHNICAL_ROTATION)
        self.assertGreater(technical[0], ordinary[0])
        self.assertGreater(technical[1], ordinary[1])
        self.assertEqual(logsetup.rotation_summary(True), "5 МБ × 6 файлов")
        self.assertEqual(logsetup.rotation_summary(False), "1 МБ × 3 файлов")

    def test_the_log_lives_in_the_data_directory(self) -> None:
        with patch("keyswitch.logsetup.data_dir", return_value=Path("/data")):
            self.assertEqual(logsetup.log_directory(), Path("/data"))
            self.assertEqual(logsetup.log_path(), Path("/data/keyswitch.log"))


class RotationBudgetTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.directory = Path(self.temporary.name)
        self.handler = RotatingFileHandler(
            self.directory / "keyswitch.log",
            maxBytes=logsetup.DEFAULT_ROTATION[0],
            backupCount=logsetup.DEFAULT_ROTATION[1],
            encoding="utf-8",
        )

    def tearDown(self) -> None:
        self.handler.close()
        self.temporary.cleanup()

    def write(self, message: str) -> None:
        self.handler.emit(
            logging.LogRecord("keyswitch", logging.INFO, __file__, 1, message, None, None)
        )

    def test_enabling_the_mode_widens_the_budget_and_starts_a_new_file(self) -> None:
        self.write("before")
        self.assertFalse(logsetup.apply_rotation(self.handler, technical=False))

        self.assertTrue(logsetup.apply_rotation(self.handler, technical=True))
        self.assertEqual(
            (self.handler.maxBytes, self.handler.backupCount),
            logsetup.TECHNICAL_ROTATION,
        )
        log = self.directory / "keyswitch.log"
        self.assertEqual(log.read_text(encoding="utf-8"), "")
        self.assertIn("before", (self.directory / "keyswitch.log.1").read_text("utf-8"))

        # Staying in the mode neither rolls over nor changes the budget.
        self.write("during")
        self.assertFalse(logsetup.apply_rotation(self.handler, technical=True))
        self.assertIn("during", log.read_text(encoding="utf-8"))

        # Leaving it shrinks the budget but keeps what was recorded.
        self.assertFalse(logsetup.apply_rotation(self.handler, technical=False))
        self.assertEqual(
            (self.handler.maxBytes, self.handler.backupCount),
            logsetup.DEFAULT_ROTATION,
        )
        self.assertIn("during", log.read_text(encoding="utf-8"))

    def test_an_empty_or_missing_log_is_not_rolled_over(self) -> None:
        self.assertFalse(logsetup.apply_rotation(self.handler, technical=True))
        self.assertFalse((self.directory / "keyswitch.log.1").exists())

        logsetup.apply_rotation(self.handler, technical=False)
        self.handler.close()
        (self.directory / "keyswitch.log").unlink()
        self.assertFalse(logsetup.apply_rotation(self.handler, technical=True))


class SettingsFollowingTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.directory = Path(self.temporary.name)
        self.settings = SettingsStore(self.directory / "config.json")
        self.handler = RotatingFileHandler(
            self.directory / "keyswitch.log",
            maxBytes=logsetup.DEFAULT_ROTATION[0],
            backupCount=logsetup.DEFAULT_ROTATION[1],
            encoding="utf-8",
        )

    def tearDown(self) -> None:
        self.handler.close()
        self.temporary.cleanup()

    def test_the_handler_follows_the_setting_and_a_full_reload(self) -> None:
        unsubscribe = logsetup.follow_settings(self.settings, self.handler)
        assert unsubscribe is not None
        self.assertEqual(self.handler.maxBytes, logsetup.DEFAULT_ROTATION[0])

        self.settings.set("diagnostics.technical_logging", True)
        self.assertEqual(self.handler.maxBytes, logsetup.TECHNICAL_ROTATION[0])

        # An unrelated setting leaves the budget alone.
        self.settings.set("detection.aggressive", True)
        self.assertEqual(self.handler.maxBytes, logsetup.TECHNICAL_ROTATION[0])

        # Resetting every setting turns the mode off again.
        self.settings.reset()
        self.assertEqual(self.handler.maxBytes, logsetup.DEFAULT_ROTATION[0])

        unsubscribe()
        self.settings.set("diagnostics.technical_logging", True)
        self.assertEqual(self.handler.maxBytes, logsetup.DEFAULT_ROTATION[0])

    def test_following_starts_from_the_stored_mode(self) -> None:
        self.settings.set("diagnostics.technical_logging", True)
        logsetup.follow_settings(self.settings, self.handler)
        self.assertEqual(self.handler.maxBytes, logsetup.TECHNICAL_ROTATION[0])

    def test_without_a_file_handler_there_is_nothing_to_follow(self) -> None:
        with patch("keyswitch.logsetup.file_handler", return_value=None):
            self.assertIsNone(logsetup.follow_settings(self.settings))

    def test_the_installed_handler_is_discovered_on_the_root_logger(self) -> None:
        root = logging.getLogger()
        self.assertIsNone(logsetup.file_handler())
        # Handlers of other kinds (a console or a test capture) are skipped.
        other = logging.NullHandler()
        root.addHandler(other)
        root.addHandler(self.handler)
        try:
            self.assertIs(logsetup.file_handler(), self.handler)
            self.assertIsNotNone(logsetup.follow_settings(self.settings))
        finally:
            root.removeHandler(self.handler)
            root.removeHandler(other)


class LogFormatTests(unittest.TestCase):
    def test_every_line_carries_the_running_version(self) -> None:
        record = logging.LogRecord(
            "keyswitch.engine", logging.INFO, __file__, 1, "исправление", None, None
        )
        line = logsetup.log_formatter().format(record)
        self.assertIn(f" {__version__} INFO keyswitch.engine: исправление", line)

    def test_the_state_of_a_missing_log_file_is_reported(self) -> None:
        missing = Path("/nonexistent-keyswitch") / "keyswitch.log"
        with patch("keyswitch.logsetup.log_path", return_value=missing):
            status = logsetup.log_status()
        self.assertEqual(status["size"], -1)
        self.assertFalse(status["installed"])
        self.assertEqual(status["path"], str(missing))

    def test_the_session_banner_names_the_version_and_the_budget(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary) / "data"
            settings = SettingsStore(Path(temporary) / "config.json")
            settings.set("diagnostics.technical_logging", True)
            root = logging.getLogger()
            previous, level = root.handlers[:], root.level
            root.handlers = []
            try:
                with patch("keyswitch.logsetup.data_dir", return_value=directory):
                    handler = logsetup.configure_logging(settings)
                    handler.close()
                banner = (directory / "keyswitch.log").read_text(encoding="utf-8")
            finally:
                root.handlers, root.level = previous, level
            self.assertIn(f"KeySwitch {__version__} запущен", banner)
            self.assertIn(str(directory / "keyswitch.log"), banner)
            self.assertIn(logsetup.rotation_summary(True), banner)
            self.assertIn("режим диагностики", banner)
            self.assertIn(f" {__version__} INFO keyswitch.logsetup:", banner)


class ConfigureLoggingTests(unittest.TestCase):
    def setUp(self) -> None:
        root = logging.getLogger()
        self.handlers, self.level = root.handlers[:], root.level

    def tearDown(self) -> None:
        root = logging.getLogger()
        for handler in root.handlers:
            if handler not in self.handlers:
                handler.close()
        root.handlers, root.level = self.handlers, self.level

    def test_the_handler_is_installed_with_the_budget_of_the_mode(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary) / "data"
            settings = SettingsStore(Path(temporary) / "config.json")
            settings.set("diagnostics.technical_logging", True)
            with patch("keyswitch.logsetup.data_dir", return_value=directory):
                handler = logsetup.configure_logging(settings)
                self.assertTrue(directory.is_dir())
                self.assertEqual(
                    Path(handler.baseFilename), directory / "keyswitch.log"
                )
                self.assertEqual(
                    (handler.maxBytes, handler.backupCount),
                    logsetup.TECHNICAL_ROTATION,
                )

                # The freshly installed handler keeps following the setting.
                settings.set("diagnostics.technical_logging", False)
                self.assertEqual(
                    (handler.maxBytes, handler.backupCount),
                    logsetup.DEFAULT_ROTATION,
                )

    def test_the_root_logger_is_wired_even_when_it_already_has_handlers(self) -> None:
        # logging.basicConfig would do nothing here, leaving the log silent.
        root = logging.getLogger()
        root.handlers = [logging.NullHandler()]
        root.setLevel(logging.CRITICAL)
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary) / "data"
            with patch("keyswitch.logsetup.data_dir", return_value=directory):
                handler = logsetup.configure_logging()
                self.assertIn(handler, root.handlers)
                self.assertIs(logsetup.file_handler(), handler)
                self.assertEqual(root.level, logging.INFO)
                logging.getLogger("keyswitch.engine").info("проверка")
                handler.flush()
                self.assertIn(
                    "проверка", (directory / "keyswitch.log").read_text("utf-8")
                )
                status = logsetup.log_status()
                self.assertTrue(status["installed"])
                self.assertEqual(status["level"], "INFO")
                self.assertEqual(status["path"], str(directory / "keyswitch.log"))
                self.assertGreater(int(str(status["size"])), 0)
                self.assertFalse(status["technical"])

                # A second start replaces the handler instead of doubling it.
                replacement = logsetup.configure_logging()
                self.assertNotIn(handler, root.handlers)
                self.assertIn(replacement, root.handlers)
                self.assertEqual(
                    len([h for h in root.handlers if isinstance(h, logging.FileHandler)]),
                    1,
                )

    def test_without_settings_the_ordinary_budget_is_used(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary) / "data"
            with patch("keyswitch.logsetup.data_dir", return_value=directory):
                handler = logsetup.configure_logging()
                self.assertEqual(
                    (handler.maxBytes, handler.backupCount),
                    logsetup.DEFAULT_ROTATION,
                )


if __name__ == "__main__":
    unittest.main()
