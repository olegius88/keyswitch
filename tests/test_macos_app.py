"""The macOS entry point, checked without a Mac.

Everything platform-bound is reached through names this test replaces, so the
module's own decisions - what it reports, what it starts and what it restores -
are verified on any host.
"""

from __future__ import annotations

import contextlib
import io
import json
import runpy
import signal
import sys
import threading
import types
import unittest
import unittest.mock
from typing import cast
import warnings
from unittest.mock import patch

from keyswitch import launcher as launcher_module
from keyswitch import macos_app
from keyswitch.backend import BackendProbe
from keyswitch.macos_backend import MacBackend
from fixture_values.platform import (
    FAKE_DIAGNOSE_EXIT_CODE,
    FAKE_GTK_MAIN_EXIT_CODE,
    FAKE_MACOS_MAIN_EXIT_CODE,
    FAKE_WINDOW_RUN_EXIT_CODE,
)
from keyswitch.constants.timing import MACOS_PERMISSION_POLL_SECONDS


class FakeBackend:
    def __init__(self, probe: BackendProbe, *, trusted: bool = True) -> None:
        self._probe = probe
        self.closed = False
        self.trusted = trusted
        self.requests = 0
        self.checks = 0

    def probe(self) -> BackendProbe:
        return self._probe

    def close(self) -> None:
        self.closed = True

    def permission_granted(self) -> bool:
        self.checks += 1
        return self.trusted

    def request_permission(self) -> bool:
        self.requests += 1
        return self.trusted


def available_probe() -> BackendProbe:
    return BackendProbe(True, "quartz", "macOS window server", "CGEventTap", "CGEventPost",
                        "com.apple.keylayout.ABC,com.apple.keylayout.Russian", 0)


class LoadedStatus:
    def as_dict(self) -> dict[str, object]:
        return {"available": True}


class PlatformSelectionTests(unittest.TestCase):
    def test_the_helper_reports_the_host_it_runs_on(self) -> None:
        expected = sys.platform == "darwin"
        self.assertEqual(macos_app._running_on_macos(), expected)
        self.assertEqual(launcher_module._running_on_macos(), expected)

    def test_a_mac_is_sent_to_the_macos_frontend(self) -> None:
        with patch.object(launcher_module, "_running_on_windows", return_value=False), \
                patch.object(launcher_module, "_running_on_macos", return_value=True), \
                patch("keyswitch.macos_app.main",
                      return_value=FAKE_MACOS_MAIN_EXIT_CODE) as platform_main:
            self.assertEqual(launcher_module.main(["--hidden"]), FAKE_MACOS_MAIN_EXIT_CODE)
        platform_main.assert_called_once_with(["--hidden"])

    def test_anything_else_still_goes_to_the_gtk_frontend(self) -> None:
        """The GTK frontend cannot be imported here, only chosen."""

        gtk_frontend = types.ModuleType("keyswitch.app")
        chosen: list[object] = []

        def main(argv: object) -> int:
            chosen.append(argv)
            return FAKE_GTK_MAIN_EXIT_CODE

        gtk_frontend.main = main  # type: ignore[attr-defined]
        with patch.dict(sys.modules, {"keyswitch.app": gtk_frontend}), \
                patch.object(launcher_module, "_running_on_windows", return_value=False), \
                patch.object(launcher_module, "_running_on_macos", return_value=False):
            self.assertEqual(launcher_module.main(None), FAKE_GTK_MAIN_EXIT_CODE)
        self.assertEqual(chosen, [None])


class DiagnoseTests(unittest.TestCase):
    def report(self, probe: BackendProbe) -> tuple[int, dict[str, object], FakeBackend]:
        backend = FakeBackend(probe)
        stream = io.StringIO()
        with patch("keyswitch.macos_app.LinearNgramModel.try_load_default",
                   return_value=(None, LoadedStatus())), \
                patch("keyswitch.macos_app.ContextModel.try_load", return_value=(None, "absent")), \
                patch("keyswitch.macos_app.MacBackend", return_value=backend), \
                contextlib.redirect_stdout(stream):
            code = macos_app.diagnose()
        return code, json.loads(stream.getvalue()), backend

    def test_a_working_backend_is_reported_and_released(self) -> None:
        code, report, backend = self.report(available_probe())
        self.assertEqual(code, 0)
        self.assertTrue(report["available"])
        self.assertEqual(report["hook"], "CGEventTap")
        self.assertEqual(report["injection"], "CGEventPost")
        self.assertIn("keylayout", str(report["layouts"]))
        self.assertTrue(backend.closed)

    def test_a_refused_backend_reports_its_reason_and_fails(self) -> None:
        refused = BackendProbe(False, "quartz", "macOS window server", "—", "—", "—", -1,
                               "нет разрешения")
        code, report, _backend = self.report(refused)
        self.assertEqual(code, 1)
        self.assertEqual(report["error"], "нет разрешения")

    def test_the_report_ties_the_field_reader_to_the_permission(self) -> None:
        """Both read the accessibility tree, so neither works without it."""

        _code, report, _backend = self.report(available_probe())
        access = report["context_field_access"]
        assert isinstance(access, dict)
        self.assertTrue(access["available"])
        self.assertTrue(report["accessibility_permission"])

    def test_a_report_without_the_permission_says_the_field_reader_is_off(self) -> None:
        backend = FakeBackend(available_probe(), trusted=False)
        stream = io.StringIO()
        with patch("keyswitch.macos_app.LinearNgramModel.try_load_default",
                   return_value=(None, LoadedStatus())), \
                patch("keyswitch.macos_app.ContextModel.try_load", return_value=(None, "absent")), \
                patch("keyswitch.macos_app.MacBackend", return_value=backend), \
                contextlib.redirect_stdout(stream):
            macos_app.diagnose()
        report = json.loads(stream.getvalue())
        access = report["context_field_access"]
        assert isinstance(access, dict)
        self.assertFalse(access["available"])
        self.assertFalse(report["accessibility_permission"])


class PermissionTests(unittest.TestCase):
    """Nothing starts before macOS lets KeySwitch watch the keyboard."""

    def backend(self, *, trusted: bool) -> FakeBackend:
        return FakeBackend(available_probe(), trusted=trusted)

    def test_a_permission_already_given_starts_at_once(self) -> None:
        backend = self.backend(trusted=True)
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertTrue(macos_app.ensure_permission(
                cast(MacBackend, backend), threading.Event()))
        self.assertEqual(backend.requests, 0)

    def test_the_request_is_shown_and_the_wait_ends_when_it_is_granted(self) -> None:
        backend = self.backend(trusted=False)
        waits: list[float] = []

        def wait(seconds: float) -> None:
            waits.append(seconds)
            backend.trusted = True

        stream = io.StringIO()
        with contextlib.redirect_stdout(stream):
            self.assertTrue(macos_app.ensure_permission(
                cast(MacBackend, backend), threading.Event(), wait=wait))
        self.assertEqual(backend.requests, 1)
        self.assertEqual(waits, [MACOS_PERMISSION_POLL_SECONDS])
        self.assertIn("Универсальный доступ", stream.getvalue())
        self.assertIn(macos_app.PERMISSION_GRANTED_MESSAGE, stream.getvalue())

    def test_a_program_asked_to_quit_stops_waiting(self) -> None:
        backend = self.backend(trusted=False)
        finished = threading.Event()
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertFalse(macos_app.ensure_permission(
                cast(MacBackend, backend), finished,
                wait=lambda _seconds: finished.set()))


class WindowRunTests(unittest.TestCase):
    """The window owns the main thread; the engine and the item live inside it."""

    def setUp(self) -> None:
        # The window itself needs Tk, which this host may not have; only the
        # wiring around it is checked here.
        self.window = types.ModuleType("keyswitch.desktop_ui")
        self.opened: list[dict[str, object]] = []

        def run_application(services: object, **arguments: object) -> int:
            self.opened.append({"services": services, **arguments})
            return FAKE_WINDOW_RUN_EXIT_CODE

        self.window.run_application = run_application  # type: ignore[attr-defined]
        patcher = patch.dict(sys.modules, {"keyswitch.desktop_ui": self.window})
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_the_window_is_given_the_backend_the_permission_was_checked_on(self) -> None:
        """Building a second backend would put two taps on one keyboard."""

        backend = object()
        services = types.ModuleType("keyswitch.macos_services")
        built: list[object] = []
        def make_services(supplied: object) -> str:
            built.append(supplied)
            return "services"

        services.MacServices = make_services  # type: ignore[attr-defined]
        with patch.dict(sys.modules, {"keyswitch.macos_services": services}), \
                patch("keyswitch.macos_app.MacBackend", return_value=backend), \
                patch("keyswitch.macos_app.ensure_permission", return_value=True) as granted:
            self.assertEqual(
                macos_app.run_window(hidden=True, no_engine=False), FAKE_WINDOW_RUN_EXIT_CODE)
        self.assertEqual(built, [backend])
        self.assertIs(granted.call_args.args[0], backend)
        self.assertEqual(self.opened, [{"services": "services", "hidden": True, "no_engine": False}])

    def test_no_window_is_opened_without_the_permission(self) -> None:
        with patch("keyswitch.macos_app.MacBackend"), \
                patch("keyswitch.macos_app.ensure_permission", return_value=False):
            self.assertEqual(macos_app.run_window(hidden=False, no_engine=False), 0)
        self.assertEqual(self.opened, [])


class CommandLineTests(unittest.TestCase):
    def test_the_version_is_printed_and_the_program_stops(self) -> None:
        stream = io.StringIO()
        with contextlib.redirect_stdout(stream), self.assertRaises(SystemExit):
            macos_app.build_parser().parse_args(["--version"])
        self.assertIn("KeySwitch", stream.getvalue())

    def test_diagnose_is_answered_without_starting_anything(self) -> None:
        with patch("keyswitch.macos_app.configure_logging"), \
                patch("keyswitch.macos_app.diagnose",
                      return_value=FAKE_DIAGNOSE_EXIT_CODE) as report, \
                patch("keyswitch.macos_app.run_window") as run:
            self.assertEqual(macos_app.main(["--diagnose"]), FAKE_DIAGNOSE_EXIT_CODE)
        report.assert_called_once_with()
        run.assert_not_called()

    def test_running_the_module_itself_goes_through_the_same_entry(self) -> None:
        """A fresh run of the module has its own names, so the system is
        replaced under it rather than in the copy already imported."""

        window = types.ModuleType("keyswitch.desktop_ui")
        window.run_application = lambda services, **arguments: 0  # type: ignore[attr-defined]
        services = types.ModuleType("keyswitch.macos_services")
        services.MacServices = lambda backend: backend  # type: ignore[attr-defined]
        backend = unittest.mock.MagicMock()
        backend.permission_granted.return_value = True
        with patch("keyswitch.macos_backend.MacBackend", return_value=backend), \
                patch.dict(sys.modules, {"keyswitch.desktop_ui": window,
                                         "keyswitch.macos_services": services}), \
                patch("keyswitch.logsetup.configure_logging"), \
                patch("keyswitch.history.data_dir"), \
                patch.object(sys, "argv", ["keyswitch", "--hidden"]), \
                warnings.catch_warnings(), \
                self.assertRaises(SystemExit) as stopped:
            warnings.simplefilter("ignore", RuntimeWarning)
            runpy.run_module("keyswitch.macos_app", run_name="__main__")
        self.assertEqual(stopped.exception.code, 0)

    def test_a_plain_launch_prepares_the_data_directory_and_opens_the_window(self) -> None:
        with patch("keyswitch.macos_app.configure_logging"), \
                patch("keyswitch.macos_app.data_dir") as directory, \
                patch("keyswitch.macos_app.run_window", return_value=0) as run:
            self.assertEqual(macos_app.main(["--hidden"]), 0)
        directory.return_value.mkdir.assert_called_once_with(parents=True, exist_ok=True)
        run.assert_called_once_with(hidden=True, no_engine=False)


if __name__ == "__main__":
    unittest.main()


if __name__ == "__main__":
    unittest.main()
