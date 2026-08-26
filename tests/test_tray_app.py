"""Tests for the D-Bus tray item and application lifecycle glue."""

from __future__ import annotations

import contextlib
import importlib
import io
import json
import logging
import runpy
import signal
import sys
import tempfile
import unittest
import warnings
from collections.abc import Callable, Mapping, Sequence
from pathlib import Path
from types import SimpleNamespace
from typing import TypeVar, overload
from unittest.mock import Mock, call, patch

import dbus

from keyswitch import app as app_module
from keyswitch import tray as tray_module
from keyswitch.app import KeySwitchApplication
from keyswitch.engine import CorrectionPlan, EngineSnapshot
from keyswitch.x11_backend import BackendProbe, KeyEvent


_T = TypeVar("_T")


def as_int(value: object) -> int:
    if not isinstance(value, int):
        raise AssertionError(f"Expected an integer D-Bus value, got {value!r}")
    return value


def as_sequence(value: object) -> Sequence[object]:
    if not isinstance(value, Sequence):
        raise AssertionError(f"Expected a D-Bus sequence, got {value!r}")
    return value


def as_mapping(value: object) -> Mapping[object, object]:
    if not isinstance(value, Mapping):
        raise AssertionError(f"Expected a D-Bus mapping, got {value!r}")
    return value


class TrayItemTests(unittest.TestCase):
    def make_item(
        self,
    ) -> tuple[
        tray_module.StatusNotifierItem,
        dict[str, Mock],
        Mock,
        Mock,
        list[Mock],
    ]:
        bus = Mock()
        bus_name = Mock()
        watcher = Mock()
        interface = Mock()
        callbacks: dict[str, Mock] = {
            "settings": Mock(),
            "autoswitch": Mock(),
            "sound": Mock(),
            "notifications": Mock(),
            "history": Mock(),
            "exceptions": Mock(),
            "about": Mock(),
            "quit": Mock(),
        }
        patches = (
            patch("keyswitch.tray.DBusGMainLoop"),
            patch("keyswitch.tray.dbus.SessionBus", return_value=bus),
            patch("keyswitch.tray.dbus.service.BusName", return_value=bus_name),
            patch("keyswitch.tray.dbus.service.Object.__init__", return_value=None),
            patch("keyswitch.tray.dbus.Interface", return_value=interface),
        )
        with contextlib.ExitStack() as stack:
            mocks: list[Mock] = [stack.enter_context(item) for item in patches]
            bus.get_object.return_value = watcher
            item = tray_module.StatusNotifierItem(
                callbacks["settings"],
                callbacks["autoswitch"],
                Path("/icons"),
                on_sound_toggle=callbacks["sound"],
                on_notifications_toggle=callbacks["notifications"],
                on_history=callbacks["history"],
                on_exceptions=callbacks["exceptions"],
                on_about=callbacks["about"],
                on_quit=callbacks["quit"],
            )
            # dbus.service.Object normally creates this during its real
            # constructor; the unit test replaces the bus registration only.
            setattr(item, "_locations", [])
            setattr(item._menu, "_locations", [])
        return item, callbacks, bus, interface, mocks

    def test_constructor_registers_item_and_exposes_properties(self) -> None:
        item, _callbacks, bus, interface, mocks = self.make_item()
        mocks[0].assert_called_once_with(set_as_default=True)
        bus.get_object.assert_called_once_with("org.kde.StatusNotifierWatcher", "/StatusNotifierWatcher")
        interface.RegisterStatusNotifierItem.assert_called_once_with("/StatusNotifierItem")
        properties = item._properties()
        self.assertEqual(str(properties["IconName"]), "keyswitch")
        self.assertTrue(properties["ItemIsMenu"])
        self.assertEqual(str(properties["Menu"]), tray_module.MENU_PATH)
        self.assertEqual(str(item.Get(tray_module.ITEM_INTERFACE, "Title")), "KeySwitch")
        self.assertEqual(item.GetAll("wrong"), {})
        self.assertIn("ToolTip", item.GetAll(tray_module.ITEM_INTERFACE))
        with self.assertRaises(dbus.exceptions.DBusException):
            item.Get("wrong", "Title")
        with self.assertRaises(dbus.exceptions.DBusException):
            item.Get(tray_module.ITEM_INTERFACE, "missing")
        with self.assertRaises(dbus.exceptions.DBusException):
            item.Set("", "", None)

    def test_activation_layout_style_and_enabled_refresh(self) -> None:
        item, callbacks, _bus, _interface, _mocks = self.make_item()
        with (
            patch.object(item._menu, "request_open") as request_open,
            patch("keyswitch.tray.GLib.idle_add") as idle,
        ):
            item.Activate(0, 0)
            item.SecondaryActivate(0, 0)
            item.ContextMenu(0, 0)
        self.assertEqual(request_open.call_count, 2)
        idle.assert_called_once_with(callbacks["autoswitch"])
        self.assertIsNone(item.Scroll(1, "vertical"))
        self.assertIsNone(item.NewStatus("Active"))
        self.assertIsNone(item.NewIcon())
        self.assertIsNone(item.NewToolTip())

        with (
            patch.object(item, "NewStatus"),
            patch.object(item, "NewIcon") as new_icon,
            patch.object(item, "NewToolTip"),
        ):
            item._refresh()
            self.assertEqual(item._subtitle, "Автокоррекция включена")
            item.set_layout(1)
            item.set_indicator_style("flags")
            item.set_enabled(False)
            item.set_sound_enabled(True)
            item.set_notifications_enabled(False)
            self.assertEqual(item._icon_name, "keyswitch-flag-ru")
            self.assertEqual(item._subtitle, "RU · автокоррекция на паузе")
            self.assertTrue(item._sound_enabled)
            self.assertFalse(item._notifications_enabled)
            count = new_icon.call_count
            item.set_layout(1)
            item.set_indicator_style("flags")
            item.set_enabled(False)
            self.assertEqual(new_icon.call_count, count)

    def test_close_swallows_already_removed_item(self) -> None:
        item, *_rest = self.make_item()
        with (
            patch.object(item._menu, "close") as close_menu,
            patch.object(tray_module.StatusNotifierItem, "remove_from_connection") as remove,
        ):
            item.close()
            remove.side_effect = LookupError
            item.close()
        self.assertEqual(close_menu.call_count, 2)

    def test_menu_properties_layout_and_property_queries(self) -> None:
        item, _callbacks, _bus, _interface, _mocks = self.make_item()
        menu = item._menu
        self.assertEqual(as_int(menu.Get(tray_module.MENU_INTERFACE, "Version")), 3)
        self.assertEqual(str(menu.Get(tray_module.MENU_INTERFACE, "TextDirection")), "ltr")
        self.assertEqual(menu.GetAll("wrong"), {})
        self.assertEqual(len(menu.GetAll(tray_module.MENU_INTERFACE)), 4)
        with self.assertRaises(dbus.exceptions.DBusException):
            menu.Get("wrong", "Version")
        with self.assertRaises(dbus.exceptions.DBusException):
            menu.Get(tray_module.MENU_INTERFACE, "Missing")
        with self.assertRaises(dbus.exceptions.DBusException):
            menu.Set("", "", None)

        revision, root = menu.GetLayout(0, -1, [])
        self.assertEqual(int(revision), 1)
        root_values = as_sequence(root)
        self.assertEqual(as_int(root_values[0]), 0)
        children = as_sequence(root_values[2])
        self.assertEqual(len(children), len(tray_module.MENU_ITEM_IDS))
        labels: dict[int, str] = {}
        for raw_child in children:
            child = as_sequence(raw_child)
            properties = as_mapping(child[1])
            labels[as_int(child[0])] = str(properties.get("label", ""))
        self.assertEqual(labels[tray_module.MENU_SETTINGS], "Настройки KeySwitch…")
        self.assertEqual(labels[tray_module.MENU_AUTOSWITCH], "Автопереключение")
        self.assertEqual(labels[tray_module.MENU_QUIT], "Выход")
        _revision, shallow = menu.GetLayout(0, 0, ["label"])
        self.assertEqual(shallow[1], {})
        self.assertEqual(shallow[2], [])
        _revision, sound = menu.GetLayout(tray_module.MENU_SOUND, -1, ["label"])
        sound_values = as_sequence(sound)
        sound_properties = as_mapping(sound_values[1])
        self.assertEqual(str(sound_properties["label"]), "Звуковые эффекты")
        with self.assertRaises(dbus.exceptions.DBusException):
            menu.GetLayout(999, -1, [])

        all_properties = menu.GetGroupProperties([], ["label"])
        self.assertEqual(len(all_properties), len(tray_module.MENU_ITEM_IDS) + 1)
        selected = menu.GetGroupProperties(
            [tray_module.MENU_SETTINGS, 999], ["label"]
        )
        self.assertEqual(len(selected), 1)
        self.assertEqual(str(menu.GetProperty(tray_module.MENU_SETTINGS, "label")), "Настройки KeySwitch…")
        self.assertTrue(menu.GetProperty(tray_module.MENU_SETTINGS, "enabled"))
        with self.assertRaises(dbus.exceptions.DBusException):
            menu.GetProperty(tray_module.MENU_SETTINGS, "unknown")

    def test_menu_events_grouping_open_request_and_protocol_signals(self) -> None:
        item, callbacks, _bus, _interface, _mocks = self.make_item()
        menu = item._menu
        with patch("keyswitch.tray.GLib.idle_add") as idle:
            menu.Event(tray_module.MENU_SETTINGS, "clicked", dbus.String(""), 0)
            menu.Event(tray_module.MENU_LAYOUT, "hovered", dbus.String(""), 0)
            menu.Event(999, "clicked", dbus.String(""), 0)
            errors = menu.EventGroup(
                [
                    (tray_module.MENU_AUTOSWITCH, "clicked", dbus.String(""), 0),
                    (tray_module.MENU_SOUND, "hovered", dbus.String(""), 0),
                    (999, "clicked", dbus.String(""), 0),
                ]
            )
        self.assertEqual(
            idle.call_args_list,
            [call(callbacks["settings"]), call(callbacks["autoswitch"])],
        )
        self.assertEqual([int(item_id) for item_id in errors], [999])
        self.assertFalse(menu.AboutToShow(0))
        updates, errors = menu.AboutToShowGroup([0, tray_module.MENU_HISTORY, 999])
        self.assertEqual(list(updates), [])
        self.assertEqual([int(item_id) for item_id in errors], [999])
        self.assertIsNone(menu.ItemsPropertiesUpdated([], []))
        self.assertIsNone(menu.LayoutUpdated(1, 0))
        self.assertIsNone(menu.ItemActivationRequested(0, 0))
        with patch.object(menu, "ItemActivationRequested") as requested:
            menu.request_open()
        requested.assert_called_once()

    def test_menu_dynamic_state_missing_actions_and_close(self) -> None:
        item, _callbacks, _bus, _interface, _mocks = self.make_item()
        menu = item._menu
        with patch.object(menu, "ItemsPropertiesUpdated") as properties_updated:
            menu.set_indicator_state(True, -1, "keyswitch")
            menu.set_indicator_state(False, 1, "keyswitch-ru")
            self.assertEqual(properties_updated.call_count, 2)
            menu.set_indicator_state(False, 1, "keyswitch-ru")
            self.assertEqual(properties_updated.call_count, 2)
        self.assertEqual(
            str(menu.GetProperty(tray_module.MENU_LAYOUT, "label")),
            "Текущая раскладка: RU",
        )
        self.assertEqual(
            as_int(menu.GetProperty(tray_module.MENU_AUTOSWITCH, "toggle-state")),
            0,
        )

        menu.set_sound_enabled(False)
        menu.set_sound_enabled(True)
        menu.set_sound_enabled(True)
        menu.set_notifications_enabled(True)
        menu.set_notifications_enabled(False)
        menu.set_notifications_enabled(False)
        self.assertEqual(
            as_int(menu.GetProperty(tray_module.MENU_SOUND, "toggle-state")), 1
        )
        self.assertEqual(
            as_int(
                menu.GetProperty(tray_module.MENU_NOTIFICATIONS, "toggle-state")
            ),
            0,
        )

        menu._actions.pop(tray_module.MENU_ABOUT)
        self.assertFalse(menu._item_properties(tray_module.MENU_ABOUT)["enabled"])
        with self.assertRaises(dbus.exceptions.DBusException):
            menu._item_properties(999)
        with patch.object(tray_module.StatusNotifierMenu, "remove_from_connection") as remove:
            menu.close()
            remove.side_effect = LookupError
            menu.close()


class FakeSettings:
    def __init__(self) -> None:
        self.values: dict[str, object] = {
            "history.limit": 25,
            "enabled": True,
            "appearance.theme": "system",
            "appearance.show_indicator": True,
            "appearance.indicator_style": "letters",
            "general.autostart": True,
            "general.start_hidden": True,
            "general.close_to_tray": True,
            "general.notifications": True,
            "general.sound": False,
        }
        self.callbacks: list[Callable[[str, object], None]] = []

    @overload
    def get(self, path: str) -> object | None: ...

    @overload
    def get(self, path: str, default: _T) -> _T: ...

    def get(self, path: str, default: object = None) -> object:
        return self.values.get(path, default)

    def set(self, path: str, value: object) -> None:
        self.values[path] = value
        for callback in tuple(self.callbacks):
            callback(path, value)

    def subscribe(self, callback: Callable[[str, object], None]) -> None:
        self.callbacks.append(callback)


class FakeAutostart:
    def __init__(self) -> None:
        self.calls: list[tuple[bool, bool]] = []
        self.error: Exception | None = None

    def set_enabled(
        self, enabled: bool, *, start_hidden: bool = True
    ) -> None:
        self.calls.append((enabled, start_hidden))
        if self.error:
            raise self.error


class FakeHistory:
    pass


class FakeEngine:
    def __init__(self) -> None:
        self.snapshot = EngineSnapshot(enabled=True, current_group=0)
        self.correction_callbacks: list[Callable[[CorrectionPlan], None]] = []
        self.snapshot_callbacks: list[Callable[[EngineSnapshot], None]] = []
        self.start_error: Exception | None = None
        self.start_calls = 0
        self.stop_calls = 0
        self.stop_error: Exception | None = None

    def subscribe_corrections(
        self, callback: Callable[[CorrectionPlan], None]
    ) -> None:
        self.correction_callbacks.append(callback)

    def subscribe(self, callback: Callable[[EngineSnapshot], None]) -> None:
        self.snapshot_callbacks.append(callback)
        callback(self.snapshot)

    def start(self) -> None:
        self.start_calls += 1
        if self.start_error:
            raise self.start_error

    def stop(self) -> None:
        self.stop_calls += 1
        if self.stop_error:
            raise self.stop_error


class FakeWindow:
    def __init__(self, *_args: object) -> None:
        self.present_calls = 0
        self.visible = False
        self.toasts: list[str] = []
        self.pages: list[str] = []

    def present(self) -> None:
        self.present_calls += 1
        self.visible = True

    def toast(self, message: str) -> None:
        self.toasts.append(message)

    def set_visible(self, visible: bool) -> None:
        self.visible = visible

    def get_visible(self) -> bool:
        return self.visible

    def show_page(self, page_name: str) -> bool:
        self.pages.append(page_name)
        return True


class FakeTray:
    def __init__(self, *_args: object, **kwargs: object) -> None:
        self.styles: list[object] = []
        self.groups: list[int] = []
        self.enabled: list[bool] = []
        self.sound: list[bool] = []
        self.notifications: list[bool] = []
        self.callbacks: dict[str, object] = kwargs
        self.closed = 0

    def set_indicator_style(self, value: object) -> None:
        self.styles.append(value)

    def set_layout(self, group: int) -> None:
        self.groups.append(group)

    def set_enabled(self, enabled: bool) -> None:
        self.enabled.append(enabled)

    def set_sound_enabled(self, enabled: bool) -> None:
        self.sound.append(enabled)

    def set_notifications_enabled(self, enabled: bool) -> None:
        self.notifications.append(enabled)

    def close(self) -> None:
        self.closed += 1


class ApplicationGlueTests(unittest.TestCase):
    def setUp(self) -> None:
        self.settings = FakeSettings()
        self.autostart = FakeAutostart()
        self.history = FakeHistory()
        self.engine = FakeEngine()
        self.patches = (
            patch("keyswitch.app.SettingsStore", return_value=self.settings),
            patch("keyswitch.app.AutostartManager", return_value=self.autostart),
            patch("keyswitch.app.HistoryStore", return_value=self.history),
            patch("keyswitch.app.KeySwitchEngine", return_value=self.engine),
            patch.object(app_module, "APP_ID", "io.github.olegius88.KeySwitchTests"),
        )
        self.stack = contextlib.ExitStack()
        for item in self.patches:
            self.stack.enter_context(item)
        self.application = KeySwitchApplication(hidden=False, no_engine=False)

    def tearDown(self) -> None:
        self.stack.close()

    def test_constructor_and_simple_actions(self) -> None:
        self.assertEqual((self.application.hidden, self.application.no_engine), (False, False))
        self.assertIs(self.application.settings, self.settings)
        self.application.window = FakeWindow()
        self.assertFalse(self.application.show_window())
        self.assertEqual(self.application.window.present_calls, 1)
        self.assertFalse(self.application.show_history())
        self.assertFalse(self.application.show_exceptions())
        self.assertFalse(self.application.show_about())
        self.assertEqual(
            self.application.window.pages,
            ["history", "exceptions", "diagnostics"],
        )
        self.assertEqual(self.application.window.present_calls, 4)
        self.application.window = None
        self.assertFalse(self.application.show_window())
        self.assertFalse(self.application._show_page("history"))
        self.assertFalse(self.application.toggle_engine())
        self.assertFalse(self.settings.get("enabled"))
        self.assertFalse(self.application.toggle_sound())
        self.assertTrue(self.settings.get("general.sound"))
        self.assertFalse(self.application.toggle_notifications())
        self.assertFalse(self.settings.get("general.notifications"))
        with patch.object(self.application, "quit") as quit_mock:
            self.assertFalse(self.application.quit_application())
            self.assertFalse(self.application._signal_quit())
        self.assertEqual(quit_mock.call_count, 2)

    def test_startup_registers_actions_theme_autostart_and_signals(self) -> None:
        with (
            patch("keyswitch.app.Adw.Application.do_startup") as base_startup,
            patch.object(self.application, "_apply_theme") as theme,
            patch.object(self.application, "_sync_autostart") as autostart,
            patch("keyswitch.app.GLib.unix_signal_add") as signal_add,
        ):
            self.application.do_startup()
        base_startup.assert_called_once_with(self.application)
        theme.assert_called_once_with("system")
        autostart.assert_called_once_with()
        self.assertIsNotNone(self.application.lookup_action("show"))
        self.assertIsNotNone(self.application.lookup_action("toggle"))
        self.assertIsNotNone(self.application.lookup_action("quit"))
        show_action = self.application.lookup_action("show")
        toggle_action = self.application.lookup_action("toggle")
        quit_action = self.application.lookup_action("quit")
        assert show_action is not None
        assert toggle_action is not None
        assert quit_action is not None
        with (
            patch.object(self.application, "show_window") as show,
            patch.object(self.application, "toggle_engine") as toggle,
            patch.object(self.application, "quit_application") as quit_application,
        ):
            show_action.activate(None)
            toggle_action.activate(None)
            quit_action.activate(None)
        show.assert_called_once_with()
        toggle.assert_called_once_with()
        quit_application.assert_called_once_with()
        self.assertEqual(
            [item.args[1] for item in signal_add.call_args_list],
            [signal.SIGINT, signal.SIGTERM],
        )

    def test_activate_holds_application_on_first_window(self) -> None:
        self.application._held = False
        self.application.no_engine = True
        with (
            patch("keyswitch.app.MainWindow", FakeWindow),
            patch.object(self.application, "_sync_tray"),
            patch.object(self.application, "hold") as hold,
        ):
            self.application.do_activate()
        hold.assert_called_once_with()
        self.assertTrue(self.application._held)

    def test_activate_success_reactivate_hidden_and_engine_error(self) -> None:
        self.application._held = True
        with patch("keyswitch.app.MainWindow", FakeWindow), patch.object(self.application, "_sync_tray"):
            self.application.do_activate()
            window = self.application.window
            assert isinstance(window, FakeWindow)
            self.assertEqual(self.engine.start_calls, 1)
            self.assertEqual(window.present_calls, 1)
            self.application.do_activate()
            self.assertEqual(window.present_calls, 2)

        second = KeySwitchApplication(hidden=True, no_engine=True)
        second._held = True
        with patch("keyswitch.app.MainWindow", FakeWindow), patch.object(second, "_sync_tray"):
            second.do_activate()
        second_window = second.window
        assert isinstance(second_window, FakeWindow)
        self.assertEqual(second_window.present_calls, 0)
        self.assertEqual(self.engine.start_calls, 1)

        third_engine = FakeEngine()
        third_engine.start_error = RuntimeError("record failed")
        with patch("keyswitch.app.KeySwitchEngine", return_value=third_engine):
            third = KeySwitchApplication(hidden=True)
        third._held = True
        with (
            patch("keyswitch.app.MainWindow", FakeWindow),
            patch.object(third, "_sync_tray"),
            patch.object(app_module.LOGGER, "exception") as logged,
        ):
            third.do_activate()
        third_window = third.window
        assert isinstance(third_window, FakeWindow)
        self.assertEqual(third_window.present_calls, 1)
        self.assertIn("record failed", third_window.toasts[0])
        logged.assert_called_once()

    def test_window_close_to_tray_and_quit_path(self) -> None:
        window = FakeWindow()
        window.visible = True
        self.application.window = window
        self.application.tray = FakeTray()
        self.assertTrue(self.application._window_close_requested())
        self.assertFalse(window.visible)
        self.settings.values["general.close_to_tray"] = False
        with patch.object(self.application, "quit_application") as quit_mock:
            self.assertTrue(self.application._window_close_requested())
        quit_mock.assert_called_once_with()

    def test_sync_tray_create_update_remove_and_failure(self) -> None:
        window = FakeWindow()
        self.application.window = window
        with patch("keyswitch.app.StatusNotifierItem", FakeTray):
            self.application._sync_tray()
        self.assertIsInstance(self.application.tray, FakeTray)
        tray = self.application.tray
        assert isinstance(tray, FakeTray)
        self.assertEqual(tray.styles, ["letters"])
        self.assertEqual(tray.groups, [0])
        self.assertEqual(tray.enabled, [True])
        self.assertEqual(tray.sound, [False])
        self.assertEqual(tray.notifications, [True])
        self.assertEqual(
            set(tray.callbacks),
            {
                "on_sound_toggle",
                "on_notifications_toggle",
                "on_history",
                "on_exceptions",
                "on_about",
                "on_quit",
            },
        )
        same = tray
        self.application._sync_tray()
        self.assertIs(self.application.tray, same)
        self.settings.values["appearance.show_indicator"] = False
        self.application._sync_tray()
        self.assertEqual(same.closed, 1)
        self.assertIsNone(self.application.tray)

        self.settings.values["appearance.show_indicator"] = True
        with patch("keyswitch.app.StatusNotifierItem", side_effect=RuntimeError("no watcher")), patch.object(app_module.LOGGER, "warning") as warning:
            self.application._sync_tray()
        self.assertIsNone(self.application.tray)
        self.assertIn("no watcher", window.toasts[-1])
        warning.assert_called_once()

        self.application.window = None
        with (
            patch("keyswitch.app.StatusNotifierItem", side_effect=RuntimeError("still no watcher")),
            patch.object(app_module.LOGGER, "warning"),
        ):
            self.application._sync_tray()

    def test_setting_dispatch_snapshot_and_idle_marshalling(self) -> None:
        tray = FakeTray()
        self.application.tray = tray
        with patch("keyswitch.app.GLib.idle_add") as idle:
            self.application._settings_changed("enabled", False)
            snapshot = EngineSnapshot(enabled=False, current_group=1)
            self.application._engine_snapshot_from_thread(snapshot)
        self.assertEqual(idle.call_args_list, [
            call(self.application._apply_setting, "enabled", False),
            call(self.application._apply_engine_snapshot, snapshot),
        ])
        self.assertFalse(self.application._apply_setting("enabled", False))
        self.assertFalse(self.application._apply_setting("general.sound", True))
        self.assertFalse(
            self.application._apply_setting("general.notifications", False)
        )
        self.assertEqual(tray.sound[-1], True)
        self.assertEqual(tray.notifications[-1], False)
        self.assertFalse(self.application._apply_setting("appearance.indicator_style", "flags"))
        with patch.object(self.application, "_sync_tray") as sync_tray:
            self.application._apply_setting("appearance.show_indicator", False)
        sync_tray.assert_called_once_with()
        with patch.object(self.application, "_apply_theme") as theme:
            self.application._apply_setting("appearance.theme", "dark")
        theme.assert_called_once_with("dark")
        with patch.object(self.application, "_sync_autostart") as sync_autostart:
            self.application._apply_setting("general.autostart", False)
            self.application._apply_setting("general.start_hidden", False)
        self.assertEqual(sync_autostart.call_count, 2)
        snapshot = EngineSnapshot(enabled=True, current_group=1)
        self.assertFalse(self.application._apply_engine_snapshot(snapshot))
        self.assertEqual(tray.groups[-1], 1)
        self.application.tray = None
        self.assertFalse(self.application._apply_setting("unrelated", object()))
        self.assertFalse(self.application._apply_engine_snapshot(snapshot))

    def test_autostart_theme_and_error(self) -> None:
        self.application._sync_autostart()
        self.assertEqual(self.autostart.calls[-1], (True, True))
        self.autostart.error = OSError("readonly")
        with patch.object(app_module.LOGGER, "warning") as warning:
            self.application._sync_autostart()
        warning.assert_called_once()
        manager = Mock()
        with patch("keyswitch.app.Adw.StyleManager.get_default", return_value=manager):
            for theme in ("system", "light", "dark", "invalid"):
                self.application._apply_theme(theme)
        self.assertEqual(manager.set_color_scheme.call_count, 4)

    def test_announce_notification_sound_window_and_thread_marshalling(self) -> None:
        event = KeyEvent(True, 38, "a", "a", ("a", "ф"), 0, 0, 1)
        plan = CorrectionPlan((event,), None, 0, 1, "a", "ф", 5.0, "Editor")
        with patch("keyswitch.app.GLib.idle_add") as idle:
            self.application._correction_from_thread(plan)
        idle.assert_called_once_with(self.application._announce_correction, plan)
        window = FakeWindow()
        window.visible = True
        self.application.window = window
        display = Mock()
        with (
            patch.object(self.application, "send_notification") as send,
            patch("keyswitch.app.Gdk.Display.get_default", return_value=display),
        ):
            self.settings.values["general.sound"] = True
            self.assertFalse(self.application._announce_correction(plan))
        send.assert_called_once()
        display.beep.assert_called_once_with()
        self.assertIn("Исправлено", window.toasts[-1])

        self.settings.values["general.notifications"] = False
        self.settings.values["general.sound"] = True
        window.visible = False
        with patch("keyswitch.app.Gdk.Display.get_default", return_value=None), patch.object(self.application, "send_notification") as send:
            self.application._announce_correction(plan)
        send.assert_not_called()
        self.settings.values["general.sound"] = False
        self.application.window = None
        self.application._announce_correction(plan)

    def test_shutdown_closes_resources_releases_hold_and_logs_stop_error(self) -> None:
        tray = FakeTray()
        self.application.tray = tray
        self.application._held = True
        self.engine.stop_error = RuntimeError("stop failed")
        with (
            patch.object(self.application, "release") as release,
            patch.object(app_module.LOGGER, "exception") as logged,
            patch("keyswitch.app.Adw.Application.do_shutdown") as base_shutdown,
        ):
            self.application.do_shutdown()
        logged.assert_called_once()
        self.assertEqual(tray.closed, 1)
        release.assert_called_once_with()
        base_shutdown.assert_called_once_with(self.application)

        self.engine.stop_error = None
        with patch("keyswitch.app.Adw.Application.do_shutdown") as base_shutdown:
            self.application.do_shutdown()
        base_shutdown.assert_called_once_with(self.application)


class ApplicationEntrypointTests(unittest.TestCase):
    def test_package_main_module_delegates_and_exits(self) -> None:
        imported = importlib.import_module("keyswitch.__main__")
        self.assertIs(imported.main, app_module.main)
        with patch("keyswitch.app.main", return_value=17) as main:
            with warnings.catch_warnings(), self.assertRaises(SystemExit) as stopped:
                warnings.simplefilter("ignore", RuntimeWarning)
                runpy.run_module("keyswitch.__main__", run_name="__main__")
        self.assertEqual(stopped.exception.code, 17)
        main.assert_called_once_with()

    def test_app_module_direct_execution_uses_diagnostic_exit_code(self) -> None:
        probe = BackendProbe(True, "x11", ":1", "1.13", "2.2", "1.0", 0)
        backend = Mock()
        backend.probe.return_value = probe
        with tempfile.TemporaryDirectory() as temporary:
            with (
                patch.object(sys, "argv", ["app.py", "--diagnose"]),
                patch("keyswitch.x11_backend.X11Backend", return_value=backend),
                patch("keyswitch.history.data_dir", return_value=Path(temporary)),
                contextlib.redirect_stdout(io.StringIO()),
                warnings.catch_warnings(),
                self.assertRaises(SystemExit) as stopped,
            ):
                warnings.simplefilter("ignore", RuntimeWarning)
                runpy.run_module("keyswitch.app", run_name="__main__")
        self.assertEqual(stopped.exception.code, 0)

    def test_logging_diagnostics_parser_and_main_modes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            with patch("keyswitch.app.data_dir", return_value=directory), patch("keyswitch.app.logging.basicConfig") as basic:
                app_module.configure_logging()
            self.assertTrue(directory.is_dir())
            self.assertEqual(basic.call_args.kwargs["level"], logging.INFO)

        probe = BackendProbe(True, "x11", ":1", "1.13", "2.2", "1.0", 1)
        backend = Mock()
        backend.probe.return_value = probe
        output = io.StringIO()
        with patch("keyswitch.app.X11Backend", return_value=backend), contextlib.redirect_stdout(output):
            self.assertEqual(app_module.diagnose(), 0)
        self.assertTrue(json.loads(output.getvalue())["available"])
        backend.probe.return_value = BackendProbe(False, "x11", "", "—", "—", "—", -1, "no display")
        with patch("keyswitch.app.X11Backend", return_value=backend), contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(app_module.diagnose(), 1)

        arguments = app_module.build_parser().parse_args(["--hidden", "--no-engine"])
        self.assertTrue(arguments.hidden and arguments.no_engine)
        with (
            patch("keyswitch.app.configure_logging"),
            patch("keyswitch.app.diagnose", return_value=7) as diagnose,
            patch("keyswitch.app.GLib.set_prgname"),
            patch("keyswitch.app.GLib.set_application_name"),
        ):
            self.assertEqual(app_module.main(["--diagnose"]), 7)
        diagnose.assert_called_once_with()

        application = Mock()
        application.run.return_value = 3
        with (
            patch("keyswitch.app.configure_logging"),
            patch("keyswitch.app.KeySwitchApplication", return_value=application) as application_class,
            patch("keyswitch.app.GLib.set_prgname"),
            patch("keyswitch.app.GLib.set_application_name"),
        ):
            self.assertEqual(app_module.main(["--hidden", "--no-engine"]), 3)
        application_class.assert_called_once_with(hidden=True, no_engine=True)


if __name__ == "__main__":
    unittest.main()
