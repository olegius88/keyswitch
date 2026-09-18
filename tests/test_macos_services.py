"""What the settings window is handed on macOS.

Each answer is a small piece of wiring, and wiring is exactly what goes wrong
silently: a second keyboard backend, a folder that never opens, an autostart
manager pointing at the wrong file.
"""

from __future__ import annotations

import sys
import types
import unittest
from pathlib import Path
from unittest.mock import patch

from keyswitch.macos_services import BACKEND_LABEL, MacServices
from keyswitch.macos_system import MacApplicationCatalog, MacAutostartManager, MacSystemError
from keyswitch.tray_model import TrayActions


def actions() -> TrayActions:
    def nothing() -> None:
        return None

    return TrayActions(*[nothing] * 9)


class ServiceTests(unittest.TestCase):
    def test_the_backend_the_engine_already_holds_is_reused(self) -> None:
        """A second backend would put a second tap on the same keyboard."""

        backend = object()
        services = MacServices(backend)  # type: ignore[arg-type]
        self.assertIs(services.backend(), backend)
        self.assertIs(services.backend(), backend)

    def test_a_backend_is_built_only_when_none_was_supplied(self) -> None:
        built = object()
        with patch("keyswitch.macos_services.MacBackend", return_value=built) as factory:
            services = MacServices()
            self.assertIs(services.backend(), built)
            self.assertIs(services.backend(), built)
        factory.assert_called_once_with()

    def test_the_window_is_told_which_backend_it_got(self) -> None:
        self.assertIn("CGEvent", MacServices().backend_label)
        self.assertEqual(MacServices().backend_label, BACKEND_LABEL)

    def test_autostart_and_the_program_list_are_the_macos_ones(self) -> None:
        services = MacServices()
        self.assertIsInstance(services.autostart(), MacAutostartManager)
        self.assertIsInstance(services.catalog(), MacApplicationCatalog)

    def test_the_menu_bar_item_is_built_from_the_shared_model(self) -> None:
        adapter = types.ModuleType("keyswitch.macos_tray_native")
        drawn: list[object] = []

        class Adapter:
            def start(self, supplied: object, state: object) -> None:
                drawn.append(supplied)

            def update(self, state: object) -> None:
                return None

            def notify(self, title: str, message: str) -> None:
                return None

            def close(self) -> None:
                return None

        adapter.StatusItemAdapter = Adapter  # type: ignore[attr-defined]
        given = actions()
        with patch.dict(sys.modules, {"keyswitch.macos_tray_native": adapter}):
            tray = MacServices().tray(given)
        self.assertEqual(drawn, [given])
        self.assertEqual(tray.state.group, -1)

    def test_opening_a_folder_that_macos_refuses_is_reported(self) -> None:
        module = types.ModuleType("keyswitch.macos_objc")
        module.open_path = lambda path: False  # type: ignore[attr-defined]
        with patch.dict(sys.modules, {"keyswitch.macos_objc": module}), \
                self.assertRaises(MacSystemError):
            MacServices().open_directory(Path("/var/log"))

    def test_a_folder_that_opens_says_nothing(self) -> None:
        module = types.ModuleType("keyswitch.macos_objc")
        opened: list[str] = []
        def open_path(path: str) -> bool:
            opened.append(path)
            return True

        module.open_path = open_path  # type: ignore[attr-defined]
        with patch.dict(sys.modules, {"keyswitch.macos_objc": module}):
            MacServices().open_directory(Path("/var/log"))
        self.assertEqual(opened, ["/var/log"])

    def test_the_alert_sound_is_the_systems_own(self) -> None:
        module = types.ModuleType("keyswitch.macos_objc")
        sounded: list[bool] = []
        module.beep = lambda: sounded.append(True)  # type: ignore[attr-defined]
        with patch.dict(sys.modules, {"keyswitch.macos_objc": module}):
            MacServices().beep()
        self.assertEqual(sounded, [True])


if __name__ == "__main__":
    unittest.main()
