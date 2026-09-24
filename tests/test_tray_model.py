"""One description of the menu, drawn by two platforms.

The menu's wording and switches used to live inside the Windows adapter. They
now live in :mod:`keyswitch.tray_model`, and these checks hold both the
description and the Windows rendering of it to the same account, on Linux.
"""

from __future__ import annotations

import sys
import types
import unittest
from typing import cast
from unittest.mock import patch

from keyswitch.tray_model import (
    ABOUT_LABEL,
    AUTO_SWITCH_LABEL,
    CURRENT_LAYOUT_PREFIX,
    EXCLUSIONS_LABEL,
    HISTORY_LABEL,
    NOTIFICATIONS_LABEL,
    QUIT_LABEL,
    SETTINGS_LABEL,
    SOUND_LABEL,
    TrayActions,
    TrayState,
    menu_entries,
)

ENGLISH_GROUP = 0
RUSSIAN_GROUP = 1
# Standard Windows message codes, reused by the fake pystray win32 module and
# by the primary/secondary click test below.
WM_LBUTTONUP = 0x0202
WM_RBUTTONUP = 0x0205


def recording_actions() -> tuple[TrayActions, list[str]]:
    called: list[str] = []
    names = (
        "show_settings", "switch_layout", "toggle_engine", "toggle_sound",
        "toggle_notifications", "show_history", "show_exclusions", "show_about",
        "quit_application",
    )
    return TrayActions(*[
        (lambda name: lambda: called.append(name))(name) for name in names
    ]), called


class MenuDescriptionTests(unittest.TestCase):
    LAYOUT_HEADER_ENTRY_COUNT = 2
    EXPECTED_SEPARATOR_COUNT = 3

    def test_the_menu_reads_the_layout_and_offers_the_other_one(self) -> None:
        actions, _called = recording_actions()
        entries = menu_entries(TrayState(group=ENGLISH_GROUP), actions)
        self.assertTrue(entries[0].label.startswith(CURRENT_LAYOUT_PREFIX))
        self.assertIn("EN", entries[0].label)
        self.assertFalse(entries[0].enabled)
        self.assertIn("RU", entries[1].label)
        self.assertTrue(entries[1].enabled)

    def test_an_unknown_layout_leaves_nothing_to_switch_to(self) -> None:
        actions, _called = recording_actions()
        entries = menu_entries(TrayState(), actions)
        self.assertFalse(entries[1].enabled)

    def test_the_switches_follow_the_state(self) -> None:
        actions, _called = recording_actions()
        state = TrayState(group=RUSSIAN_GROUP, enabled=False, sound_enabled=True,
                          notifications_enabled=False)
        switches = {entry.label: entry.checked for entry in menu_entries(state, actions)}
        self.assertFalse(switches[AUTO_SWITCH_LABEL])
        self.assertTrue(switches[SOUND_LABEL])
        self.assertFalse(switches[NOTIFICATIONS_LABEL])

    def test_only_the_three_switches_are_drawn_as_switches(self) -> None:
        """A checkbox on a plain line is how a menu starts lying about state."""

        actions, _called = recording_actions()
        entries = menu_entries(TrayState(group=ENGLISH_GROUP), actions)
        switches = {entry.label for entry in entries if entry.checked is not None}
        self.assertEqual(switches, {AUTO_SWITCH_LABEL, SOUND_LABEL, NOTIFICATIONS_LABEL})

    def test_every_line_that_can_be_chosen_calls_its_own_action(self) -> None:
        actions, called = recording_actions()
        for entry in menu_entries(TrayState(group=ENGLISH_GROUP), actions):
            if entry.action is not None:
                entry.action()
        self.assertEqual(called, [
            "switch_layout", "show_settings", "toggle_engine", "toggle_sound",
            "toggle_notifications", "show_history", "show_exclusions", "show_about",
            "quit_application",
        ])

    def test_the_menu_keeps_its_shape(self) -> None:
        actions, _called = recording_actions()
        entries = menu_entries(TrayState(group=ENGLISH_GROUP), actions)
        labels = [entry.label for entry in entries if not entry.separator]
        self.assertEqual(labels[self.LAYOUT_HEADER_ENTRY_COUNT:], [
            SETTINGS_LABEL, AUTO_SWITCH_LABEL, SOUND_LABEL, NOTIFICATIONS_LABEL,
            HISTORY_LABEL, EXCLUSIONS_LABEL, ABOUT_LABEL, QUIT_LABEL,
        ])
        self.assertEqual(sum(1 for entry in entries if entry.separator), self.EXPECTED_SEPARATOR_COUNT)


class FakeMenuItem:
    def __init__(self, text: object, action: object, enabled: object = True,
                 checked: object = None, default: bool = False) -> None:
        self.text, self.action = text, action
        self.enabled, self.checked, self.default = enabled, checked, default

    def label(self) -> str:
        return self.text(self) if callable(self.text) else str(self.text)


class FakeMenu:
    SEPARATOR = "---"

    def __init__(self, *items: object) -> None:
        self.items = items


class FakeIcon:
    def __init__(self, *arguments: object) -> None:
        self.arguments = arguments

    def _on_notify(self, wparam: int, lparam: int) -> None:
        self.notified = (wparam, lparam)


def windows_tray_native() -> types.ModuleType:
    """Import the Windows adapter on this host, with its dependencies replaced."""

    pystray = types.ModuleType("pystray")
    pystray.Icon = FakeIcon  # type: ignore[attr-defined]
    pystray.Menu = FakeMenu  # type: ignore[attr-defined]
    pystray.MenuItem = FakeMenuItem  # type: ignore[attr-defined]
    util = types.ModuleType("pystray._util")
    win32 = types.ModuleType("pystray._util.win32")
    win32.WM_LBUTTONUP = WM_LBUTTONUP  # type: ignore[attr-defined]
    win32.WM_RBUTTONUP = WM_RBUTTONUP  # type: ignore[attr-defined]
    util.win32 = win32  # type: ignore[attr-defined]
    pillow = types.ModuleType("PIL")
    for name in ("Image", "ImageDraw", "ImageFont"):
        setattr(pillow, name, types.ModuleType(f"PIL.{name}"))
    modules = {
        "pystray": pystray, "pystray._util": util, "pystray._util.win32": win32,
        "PIL": pillow, "PIL.Image": pillow.Image, "PIL.ImageDraw": pillow.ImageDraw,
        "PIL.ImageFont": pillow.ImageFont,
    }
    with patch.dict(sys.modules, modules):
        sys.modules.pop("keyswitch.windows_tray_native", None)
        import keyswitch.windows_tray_native as module
    sys.modules.pop("keyswitch.windows_tray_native", None)
    return module


class WindowsRenderingTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = windows_tray_native()
        self.actions, self.called = recording_actions()
        self.state = TrayState(group=ENGLISH_GROUP)

    def items(self) -> tuple[object, ...]:
        return cast(
            tuple[object, ...],
            self.module._menu_items(self.actions, lambda: self.state),
        )

    def test_the_rendered_menu_matches_the_description_line_for_line(self) -> None:
        rendered = self.items()
        described = menu_entries(self.state, self.actions)
        self.assertEqual(len(rendered), len(described))
        for item, entry in zip(rendered, described):
            if entry.separator:
                self.assertEqual(item, FakeMenu.SEPARATOR)
            else:
                self.assertEqual(cast(FakeMenuItem, item).label(), entry.label)

    def test_a_plain_line_is_drawn_without_a_checkbox(self) -> None:
        for item, entry in zip(self.items(), menu_entries(self.state, self.actions)):
            if entry.separator or entry.checked is not None:
                continue
            self.assertIsNone(cast(FakeMenuItem, item).checked)

    def test_a_switch_reports_what_the_state_says_when_the_menu_opens(self) -> None:
        rendered = self.items()
        described = menu_entries(self.state, self.actions)
        index = next(i for i, entry in enumerate(described) if entry.label == SOUND_LABEL)
        checked = cast(FakeMenuItem, rendered[index]).checked
        assert callable(checked)
        self.assertFalse(checked(None))
        self.state = TrayState(group=ENGLISH_GROUP, sound_enabled=True)
        self.assertTrue(checked(None))

    def test_the_labels_follow_a_layout_change_without_rebuilding(self) -> None:
        item = cast(FakeMenuItem, self.items()[0])
        self.assertIn("EN", item.label())
        self.state = TrayState(group=RUSSIAN_GROUP)
        self.assertIn("RU", item.label())

    def test_choosing_a_line_runs_the_action_behind_it(self) -> None:
        rendered = self.items()
        described = menu_entries(self.state, self.actions)
        index = next(i for i, entry in enumerate(described) if entry.label == QUIT_LABEL)
        action = cast(FakeMenuItem, rendered[index]).action
        assert callable(action)
        action(None, None)
        self.assertEqual(self.called, ["quit_application"])

    def test_the_line_that_only_reports_the_layout_cannot_be_chosen(self) -> None:
        self.assertIsNone(cast(FakeMenuItem, self.items()[0]).action)

    def test_a_primary_click_opens_the_same_menu_as_the_secondary_one(self) -> None:
        self.assertEqual(self.module.menu_activation_message(WM_LBUTTONUP, WM_LBUTTONUP, WM_RBUTTONUP), WM_RBUTTONUP)
        self.assertEqual(self.module.menu_activation_message(WM_RBUTTONUP, WM_LBUTTONUP, WM_RBUTTONUP), WM_RBUTTONUP)


if __name__ == "__main__":
    unittest.main()


class ControllerTests(unittest.TestCase):
    """The controller between the state and whatever draws it."""

    EXPECTED_DRAW_COUNT = 4  # opening state + set_layout + set_sound_enabled + set_indicator_style

    def setUp(self) -> None:
        from keyswitch.tray_model import TrayController

        self.drawn: list[TrayState] = []
        self.notified: list[tuple[str, str]] = []
        self.closed = False
        adapter = self

        class Adapter:
            def start(self, actions: object, state: object) -> None:
                self.actions = actions

            def update(self, state: TrayState) -> None:
                adapter.drawn.append(state)

            def notify(self, title: str, message: str) -> None:
                adapter.notified.append((title, message))

            def close(self) -> None:
                adapter.closed = True

        self.actions, _called = recording_actions()
        self.controller = TrayController(self.actions, cast(object, Adapter()))  # type: ignore[arg-type]

    def test_every_change_is_drawn_once(self) -> None:
        self.controller.set_layout(RUSSIAN_GROUP)
        self.controller.set_sound_enabled(True)
        self.controller.set_indicator_style("flags")
        self.assertEqual(self.controller.state.group, RUSSIAN_GROUP)
        self.assertTrue(self.controller.state.sound_enabled)
        self.assertEqual(self.controller.state.indicator_style, "flags")
        self.assertEqual(len(self.drawn), self.EXPECTED_DRAW_COUNT)  # The first is the opening state.

    def test_the_remaining_switches_are_published_too(self) -> None:
        self.controller.set_enabled(False)
        self.controller.set_notifications_enabled(False)
        self.assertFalse(self.controller.state.enabled)
        self.assertFalse(self.controller.state.notifications_enabled)

    def test_a_message_reaches_the_adapter(self) -> None:
        self.controller.notify("KeySwitch", "исправлено")
        self.assertEqual(self.notified, [("KeySwitch", "исправлено")])

    def test_nothing_is_drawn_or_said_after_closing(self) -> None:
        """A closed item has no window left to draw into."""

        self.controller.close()
        self.controller.set_layout(RUSSIAN_GROUP)
        self.controller.notify("KeySwitch", "исправлено")
        self.assertEqual(len(self.drawn), 1)
        self.assertEqual(self.notified, [])

    def test_closing_twice_closes_once(self) -> None:
        self.controller.close()
        self.closed = False
        self.controller.close()
        self.assertFalse(self.closed)
