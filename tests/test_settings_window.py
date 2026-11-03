"""What the Tk settings window shows, read without Tk: its theme and where each setting stands.

The window itself (desktop_ui) needs Tk and is driven by the Windows end-to-end test; the decisions
it takes are made here, where every platform's tests can reach them.
"""

from __future__ import annotations

import unittest
from unittest.mock import Mock

from keyswitch.constants.settings_defaults import DEFAULT_SETTINGS
from keyswitch.desktop_theme import (
    MACOS_DARK_INTERFACE_STYLE,
    dark_from_apps_use_light_theme,
    dark_from_interface_style,
    dark_theme,
)
from keyswitch.windows_ui_model import ALL_SETTING_SPECS, AUTOCORRECTION_SETTINGS, MANUAL_SWITCH_SETTINGS


class ThemeTests(unittest.TestCase):
    def test_a_fixed_theme_never_asks_the_system(self) -> None:
        system = Mock(return_value=True)
        self.assertTrue(dark_theme("dark", system))
        self.assertFalse(dark_theme("light", system))
        system.assert_not_called()

    def test_the_system_theme_follows_the_system_each_time_it_is_asked(self) -> None:
        system = Mock(side_effect=[True, False])
        self.assertTrue(dark_theme("system", system))
        self.assertFalse(dark_theme("system", system))
        # An unknown value reads as the system's choice, as the settings windows show it.
        self.assertFalse(dark_theme("unknown", Mock(return_value=False)))
        self.assertTrue(dark_theme("unknown", Mock(return_value=True)))

    def test_a_system_that_cannot_answer_is_light(self) -> None:
        with self.assertLogs("keyswitch.desktop_theme", level="WARNING"):
            self.assertFalse(dark_theme("system", Mock(side_effect=OSError("registry unreadable"))))

    def test_windows_app_mode_zero_is_dark_and_anything_else_light(self) -> None:
        self.assertTrue(dark_from_apps_use_light_theme(0))
        self.assertFalse(dark_from_apps_use_light_theme(1))
        self.assertFalse(dark_from_apps_use_light_theme(None))

    def test_macos_dark_appearance_is_named_and_the_light_one_is_not_set(self) -> None:
        self.assertTrue(dark_from_interface_style(MACOS_DARK_INTERFACE_STYLE))
        self.assertFalse(dark_from_interface_style(""))
        self.assertFalse(dark_from_interface_style("Light"))


class SettingPlacementTests(unittest.TestCase):
    def test_the_manual_layout_switch_is_shown_once_on_the_layouts_page(self) -> None:
        paths = [spec.path for spec in ALL_SETTING_SPECS]
        self.assertEqual(paths.count("detection.respect_manual_layout"), 1)
        self.assertEqual([spec.path for spec in MANUAL_SWITCH_SETTINGS], ["detection.respect_manual_layout"])
        self.assertNotIn("detection.respect_manual_layout", [spec.path for spec in AUTOCORRECTION_SETTINGS])

    def test_the_double_capitals_fix_is_on_the_autocorrection_page_and_on_by_default(self) -> None:
        spec = next(spec for spec in AUTOCORRECTION_SETTINGS if spec.path == "detection.fix_double_capitals")
        self.assertEqual((spec.title, spec.kind), ("Исправлять ДВе заглавные", "bool"))
        self.assertIn("ПРивет", spec.description)
        detection = DEFAULT_SETTINGS["detection"]
        assert isinstance(detection, dict)
        self.assertIs(detection["fix_double_capitals"], True)
        # The overview shows the first setting of the page, the switch of automatic correction.
        self.assertEqual(AUTOCORRECTION_SETTINGS[0].path, "enabled")


if __name__ == "__main__":
    unittest.main()
