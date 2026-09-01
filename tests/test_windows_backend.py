"""Platform-independent verification of the Win32 keyboard backend."""

from __future__ import annotations

import contextlib
import io
import json
import logging
import runpy
import sys
import tempfile
import threading
import unittest
import warnings
from collections.abc import Callable
from pathlib import Path
from types import ModuleType
from unittest.mock import patch

from keyswitch import windows_app as windows_app_module
from keyswitch import launcher as launcher_module
from keyswitch.backend import (
    ALT_MASK,
    CONTROL_MASK,
    LOCK_MASK,
    SHIFT_MASK,
    SUPER_MASK,
    BackendProbe,
    KeyEvent,
    ScreenAnchor,
)
from keyswitch import config, history
from keyswitch.config import DEFAULTS
from keyswitch.engine import _default_backend
from keyswitch.windows_instance import WindowsSingleInstance
from keyswitch.windows_backend import (
    LANG_ENGLISH,
    LANG_RUSSIAN,
    NativeInput,
    NativeKeyEvent,
    VK_BACK,
    VK_CAPITAL,
    VK_CONTROL,
    VK_LWIN,
    VK_MENU,
    VK_SHIFT,
    WindowsBackend,
    WindowsBackendError,
    key_name,
    primary_language,
    select_layout_pair,
)
from keyswitch.windows_system import (
    WindowsApplicationCatalog,
    WindowsAutostartManager,
    WindowsSystemError,
    clean_windows_executable,
    windows_launcher_command,
)
from keyswitch.windows_tray import (
    WindowsTray,
    WindowsTrayActions,
    WindowsTrayState,
    _native_adapter,
    menu_activation_message,
)
from keyswitch.windows_ui_model import ALL_SETTING_SPECS


ENGLISH_LAYOUT = 0x00000409
RUSSIAN_LAYOUT = 0x00000419


class FakeWindowsAPI:
    def __init__(self) -> None:
        self.layout_values: tuple[int, ...] = (ENGLISH_LAYOUT, RUSSIAN_LAYOUT)
        self.current_layout = ENGLISH_LAYOUT
        self.application = "Notepad"
        self.caps_lock = False
        self.requests: list[int] = []
        self.sent: list[tuple[NativeInput, ...]] = []
        self.send_count: int | None = None
        self.accept_switch = True
        self.apply_switch = True
        self.hook_error: Exception | None = None
        self.signal_ready = True
        self.stop_event = threading.Event()
        self.hook_listener: Callable[[NativeKeyEvent], None] | None = None
        self.stop_calls = 0
        self.layout_calls = 0
        self.translation: dict[tuple[int, int], str] = {}
        self.anchor: ScreenAnchor | None = ScreenAnchor(100, 200, 300)
        self.activated_windows: list[int] = []

    def loaded_layouts(self) -> tuple[int, ...]:
        self.layout_calls += 1
        return self.layout_values

    def foreground_layout(self) -> int:
        return self.current_layout

    def request_layout(self, layout: int) -> bool:
        self.requests.append(layout)
        if self.accept_switch and self.apply_switch:
            self.current_layout = layout
        return self.accept_switch

    def translate_key(
        self,
        virtual_key: int,
        scan_code: int,
        state: int,
        layout: int,
    ) -> str:
        del scan_code, state
        return self.translation.get((virtual_key, layout), "")

    def send_inputs(self, inputs: tuple[NativeInput, ...]) -> int:
        self.sent.append(inputs)
        return self.send_count if self.send_count is not None else len(inputs)

    def active_application(self) -> str:
        return self.application

    def input_anchor(self) -> ScreenAnchor | None:
        return self.anchor

    def activate_window(self, window: int) -> bool:
        self.activated_windows.append(window)
        return True

    def caps_lock_enabled(self) -> bool:
        return self.caps_lock

    def run_keyboard_hook(
        self,
        listener: Callable[[NativeKeyEvent], None],
        ready: Callable[[], None],
    ) -> None:
        self.hook_listener = listener
        if self.hook_error is not None:
            raise self.hook_error
        if self.signal_ready:
            ready()
        self.stop_event.wait(1.0)

    def stop_keyboard_hook(self) -> None:
        self.stop_calls += 1
        self.stop_event.set()


class FakeRegistry:
    def __init__(self) -> None:
        self.autostart: dict[str, str] = {}
        self.apps: tuple[tuple[str, str], ...] = ()

    def read_autostart(self, name: str) -> str | None:
        return self.autostart.get(name)

    def write_autostart(self, name: str, command: str) -> None:
        self.autostart[name] = command

    def delete_autostart(self, name: str) -> None:
        self.autostart.pop(name, None)

    def application_paths(self) -> tuple[tuple[str, str], ...]:
        return self.apps


class FakeInstanceAPI:
    def __init__(self, *, acquired: bool = True, activated: bool = True) -> None:
        self.acquired = acquired
        self.activated = activated
        self.acquire_calls = 0
        self.activate_calls = 0
        self.close_calls = 0

    def acquire(self) -> bool:
        self.acquire_calls += 1
        return self.acquired

    def activate_existing(self) -> bool:
        self.activate_calls += 1
        return self.activated

    def close(self) -> None:
        self.close_calls += 1


class FakeTrayAdapter:
    def __init__(self) -> None:
        self.actions: WindowsTrayActions | None = None
        self.state_reader: Callable[[], WindowsTrayState] | None = None
        self.states: list[WindowsTrayState] = []
        self.notifications: list[tuple[str, str]] = []
        self.close_calls = 0

    def start(
        self,
        actions: WindowsTrayActions,
        state: Callable[[], WindowsTrayState],
    ) -> None:
        self.actions = actions
        self.state_reader = state

    def update(self, state: WindowsTrayState) -> None:
        self.states.append(state)

    def notify(self, title: str, message: str) -> None:
        self.notifications.append((title, message))

    def close(self) -> None:
        self.close_calls += 1


def key_event(
    *,
    keycode: int = 30,
    character: str = "a",
    characters: tuple[str, ...] = ("a", "ф"),
    group: int = 0,
    state: int = 0,
) -> KeyEvent:
    return KeyEvent(
        True,
        keycode,
        "a",
        character,
        characters,
        group,
        state,
        100,
    )


class WindowsBackendHelperTests(unittest.TestCase):
    def test_runtime_platform_helpers_report_the_current_host(self) -> None:
        expected = sys.platform == "win32"
        self.assertEqual(launcher_module._running_on_windows(), expected)
        from keyswitch import windows_backend, windows_instance, windows_system

        self.assertEqual(windows_backend._running_on_windows(), expected)
        self.assertEqual(windows_instance._running_on_windows(), expected)
        self.assertEqual(windows_system._running_on_windows(), expected)

    def test_engine_default_backend_remains_lazy_and_linux_specific(self) -> None:
        backend = WindowsBackend(FakeWindowsAPI())
        with patch("keyswitch.x11_backend.X11Backend", return_value=backend) as factory:
            self.assertIs(_default_backend(2), backend)
        factory.assert_called_once_with(group_count=2)

    def test_layout_languages_pair_selection_and_key_names(self) -> None:
        self.assertEqual(primary_language(0xF0010409), LANG_ENGLISH)
        self.assertEqual(primary_language(RUSSIAN_LAYOUT), LANG_RUSSIAN)
        self.assertEqual(
            select_layout_pair((RUSSIAN_LAYOUT, ENGLISH_LAYOUT, ENGLISH_LAYOUT)),
            (ENGLISH_LAYOUT, RUSSIAN_LAYOUT),
        )
        with self.assertRaisesRegex(WindowsBackendError, "английская и русская"):
            select_layout_pair((ENGLISH_LAYOUT,))
        with self.assertRaisesRegex(WindowsBackendError, "английская и русская"):
            select_layout_pair((RUSSIAN_LAYOUT,))
        self.assertEqual(key_name(VK_BACK), "BackSpace")
        self.assertEqual(key_name(ord("A")), "a")
        self.assertEqual(key_name(ord("7")), "7")
        self.assertEqual(key_name(0xFE), "VK_FE")

    def test_constructor_rejects_default_native_api_outside_windows(self) -> None:
        with patch("keyswitch.windows_backend._running_on_windows", return_value=False):
            with self.assertRaisesRegex(WindowsBackendError, "только в Windows"):
                WindowsBackend()

    def test_constructor_can_load_the_isolated_native_adapter(self) -> None:
        api = FakeWindowsAPI()
        native_module = ModuleType("keyswitch.windows_native")
        setattr(native_module, "CtypesWindowsAPI", lambda: api)
        with (
            patch("keyswitch.windows_backend._running_on_windows", return_value=True),
            patch.dict(sys.modules, {"keyswitch.windows_native": native_module}),
        ):
            backend = WindowsBackend()
        self.assertEqual(backend.layouts, (ENGLISH_LAYOUT, RUSSIAN_LAYOUT))

    def test_modifier_properties_share_one_platform_neutral_contract(self) -> None:
        event = key_event(
            state=SHIFT_MASK | LOCK_MASK | CONTROL_MASK | ALT_MASK | SUPER_MASK
        )
        self.assertTrue(event.shift)
        self.assertTrue(event.caps_lock)
        self.assertTrue(event.control)
        self.assertTrue(event.alt)
        self.assertTrue(event.super_key)
        self.assertEqual(event.character_for(1), "ф")
        self.assertEqual(event.character_for(9), "")


class WindowsBackendLifecycleTests(unittest.TestCase):
    def test_layouts_probe_group_and_application(self) -> None:
        api = FakeWindowsAPI()
        backend = WindowsBackend(api)
        self.assertEqual(backend.layouts, (ENGLISH_LAYOUT, RUSSIAN_LAYOUT))
        self.assertEqual(backend.layouts, (ENGLISH_LAYOUT, RUSSIAN_LAYOUT))
        self.assertEqual(api.layout_calls, 1)
        probe = backend.probe()
        self.assertTrue(probe.available)
        self.assertEqual(probe.session_type, "windows")
        self.assertIn("00000409", probe.xkb_version)
        self.assertEqual(backend.current_group(), 0)
        self.assertEqual(backend.active_application(), "Notepad")
        self.assertEqual(backend.input_anchor(), ScreenAnchor(100, 200, 300))
        self.assertFalse(backend.restore_window(None))
        self.assertTrue(backend.restore_window(300))
        self.assertEqual(api.activated_windows, [300])

        api.current_layout = 0x12340409
        self.assertEqual(backend.current_group(), 0)
        api.current_layout = 0x12340419
        self.assertEqual(backend.current_group(), 1)
        api.current_layout = 0x00000407
        self.assertEqual(backend.current_group(), -1)

        with self.assertRaisesRegex(WindowsBackendError, "Неизвестная группа"):
            backend.switch_group(2)
        backend.switch_group(1)
        self.assertEqual(api.requests[-1], RUSSIAN_LAYOUT)

    def test_probe_reports_layout_failure(self) -> None:
        api = FakeWindowsAPI()
        api.layout_values = (ENGLISH_LAYOUT,)
        probe = WindowsBackend(api).probe()
        self.assertFalse(probe.available)
        self.assertEqual(probe.current_group, -1)
        self.assertIn("русская", probe.error)

    def test_start_events_idempotence_stop_and_close(self) -> None:
        api = FakeWindowsAPI()
        api.translation[(ord("A"), ENGLISH_LAYOUT)] = "a"
        api.translation[(ord("A"), RUSSIAN_LAYOUT)] = "ф"
        api.caps_lock = True
        backend = WindowsBackend(api)
        events: list[KeyEvent] = []
        backend.start(events.append)
        self.assertTrue(backend.running)
        backend.start(events.append)
        listener = api.hook_listener
        self.assertIsNotNone(listener)
        assert listener is not None
        listener(NativeKeyEvent(True, ord("A"), 30, False, True, 123))
        self.assertEqual(len(events), 1)
        self.assertEqual(events[0].characters, ("a", "ф"))
        self.assertEqual(events[0].character, "a")
        self.assertTrue(events[0].synthetic)
        self.assertTrue(events[0].caps_lock)
        backend.stop()
        self.assertFalse(backend.running)
        self.assertEqual(api.stop_calls, 1)
        backend.stop()
        backend.close()

    def test_start_propagates_hook_failure(self) -> None:
        api = FakeWindowsAPI()
        api.hook_error = OSError("hook denied")
        backend = WindowsBackend(api)
        with self.assertRaisesRegex(WindowsBackendError, "hook denied"):
            backend.start(lambda _event: None)
        self.assertFalse(backend.running)
        backend.stop()

    def test_start_timeout_stops_native_hook(self) -> None:
        api = FakeWindowsAPI()
        api.signal_ready = False
        backend = WindowsBackend(api)
        with patch("keyswitch.windows_backend.HOOK_START_TIMEOUT", 0.01):
            with self.assertRaisesRegex(WindowsBackendError, "не подтвердил"):
                backend.start(lambda _event: None)
        self.assertEqual(api.stop_calls, 1)

    def test_stop_does_not_join_current_thread(self) -> None:
        api = FakeWindowsAPI()
        backend = WindowsBackend(api)
        backend._thread = threading.current_thread()
        backend._running.set()
        backend.stop()
        self.assertFalse(backend.running)
        self.assertEqual(api.stop_calls, 1)

    def test_event_state_tracks_modifiers_caps_and_unknown_group(self) -> None:
        api = FakeWindowsAPI()
        backend = WindowsBackend(api)
        collected: list[KeyEvent] = []
        backend._listener = collected.append

        for virtual_key in (VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_CAPITAL):
            backend._handle_native(
                NativeKeyEvent(True, virtual_key, virtual_key, False, False, 1)
            )
        # A repeated CapsLock key-down must not toggle the lock back off.
        backend._handle_native(
            NativeKeyEvent(True, VK_CAPITAL, VK_CAPITAL, False, False, 2)
        )
        self.assertEqual(
            collected[-1].state,
            SHIFT_MASK | CONTROL_MASK | ALT_MASK | SUPER_MASK | LOCK_MASK,
        )
        for virtual_key in (VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_CAPITAL):
            backend._handle_native(
                NativeKeyEvent(False, virtual_key, virtual_key, False, False, 3)
            )
        self.assertEqual(collected[-1].state, LOCK_MASK)

        api.current_layout = 0x00000407
        backend._handle_native(NativeKeyEvent(True, ord("A"), 30, False, False, 4))
        self.assertEqual(collected[-1].character, "")
        backend._listener = None
        backend._handle_native(NativeKeyEvent(False, ord("A"), 30, False, False, 5))


class WindowsBackendInjectionTests(unittest.TestCase):
    def test_correction_deletes_switches_replays_shift_and_boundary(self) -> None:
        api = FakeWindowsAPI()
        backend = WindowsBackend(api)
        stroke = key_event(state=SHIFT_MASK)
        boundary = key_event(
            keycode=39,
            character=";",
            characters=(";", "ж"),
        )
        backend.inject_correction((stroke,), 1, boundary, source_group=0)

        self.assertEqual(
            api.sent[0],
            (
                NativeInput(True, virtual_key=VK_BACK),
                NativeInput(False, virtual_key=VK_BACK),
                NativeInput(True, virtual_key=VK_BACK),
                NativeInput(False, virtual_key=VK_BACK),
            ),
        )
        self.assertEqual(
            api.sent[1],
            (
                NativeInput(True, virtual_key=VK_SHIFT),
                NativeInput(True, scan_code=30),
                NativeInput(False, scan_code=30),
                NativeInput(False, virtual_key=VK_SHIFT),
            ),
        )
        self.assertEqual(api.sent[2][0].scan_code, 39)
        self.assertEqual(
            api.requests,
            [RUSSIAN_LAYOUT, ENGLISH_LAYOUT, RUSSIAN_LAYOUT],
        )

    def test_boundary_can_stay_in_target_and_empty_batches_are_skipped(self) -> None:
        api = FakeWindowsAPI()
        backend = WindowsBackend(api)
        boundary = key_event(character=" ", characters=(" ", " "))
        backend.inject_correction((), 0, None)
        self.assertEqual(api.sent, [])
        backend.inject_correction((key_event(),), 1, boundary)
        self.assertEqual(api.requests, [RUSSIAN_LAYOUT])
        self.assertEqual(len(api.sent), 3)

    def test_invalid_groups_partial_send_and_rejected_switch_fail_loudly(self) -> None:
        api = FakeWindowsAPI()
        backend = WindowsBackend(api)
        with self.assertRaisesRegex(WindowsBackendError, "Неизвестная группа"):
            backend.inject_correction((), 2, None)
        with self.assertRaisesRegex(WindowsBackendError, "исходная группа"):
            backend.inject_correction((key_event(group=7),), 1, None)

        api.send_count = 0
        with self.assertRaisesRegex(WindowsBackendError, "SendInput"):
            backend.inject_correction((key_event(),), 1, None)

        api.send_count = None
        api.accept_switch = False
        with self.assertRaisesRegex(WindowsBackendError, "отклонило"):
            backend.inject_correction((key_event(),), 1, None)

    def test_switch_timeout_and_already_selected_group(self) -> None:
        api = FakeWindowsAPI()
        backend = WindowsBackend(api)
        backend._switch_group(0)
        self.assertEqual(api.requests, [])

        api.apply_switch = False
        with (
            patch(
                "keyswitch.windows_backend.time.monotonic",
                side_effect=(0.0, 0.1, 0.6),
            ),
            patch("keyswitch.windows_backend.time.sleep") as sleep,
        ):
            with self.assertRaisesRegex(WindowsBackendError, "не подтвердило"):
                backend._switch_group(1)
        sleep.assert_called_once_with(0.01)


class WindowsSystemTests(unittest.TestCase):
    def test_profile_paths_use_roaming_and_local_appdata(self) -> None:
        with (
            patch("keyswitch.config._running_on_windows", return_value=True),
            patch.dict("os.environ", {"APPDATA": r"C:\Users\Me\Roaming"}, clear=True),
        ):
            self.assertEqual(config.config_dir(), Path(r"C:\Users\Me\Roaming") / "KeySwitch")
        with (
            patch("keyswitch.history._running_on_windows", return_value=True),
            patch.dict("os.environ", {"LOCALAPPDATA": r"C:\Users\Me\Local"}, clear=True),
        ):
            self.assertEqual(history.data_dir(), Path(r"C:\Users\Me\Local") / "KeySwitch")
        with (
            patch("keyswitch.config._running_on_windows", return_value=True),
            patch("keyswitch.config.Path.home", return_value=Path("/profile")),
            patch.dict("os.environ", {}, clear=True),
        ):
            self.assertEqual(
                config.config_dir(),
                Path("/profile/AppData/Roaming/KeySwitch"),
            )
        with (
            patch("keyswitch.history._running_on_windows", return_value=True),
            patch("keyswitch.history.Path.home", return_value=Path("/profile")),
            patch.dict("os.environ", {}, clear=True),
        ):
            self.assertEqual(
                history.data_dir(),
                Path("/profile/AppData/Local/KeySwitch"),
            )

    def test_autostart_manager_and_quoted_commands(self) -> None:
        registry = FakeRegistry()
        manager = WindowsAutostartManager(registry, command='"C:\\Key Switch\\KeySwitch.exe" --hidden')
        self.assertFalse(manager.enabled())
        manager.set_enabled(True)
        self.assertTrue(manager.enabled())
        self.assertIn("--hidden", registry.autostart["KeySwitch"])
        manager.set_enabled(False)
        self.assertFalse(manager.enabled())

        frozen = windows_launcher_command(
            start_hidden=False,
            executable=Path("/Program Files/KeySwitch.exe"),
        )
        self.assertIn("KeySwitch.exe", frozen)
        self.assertNotIn("--hidden", frozen)
        with tempfile.TemporaryDirectory() as temporary:
            python = Path(temporary) / "python.exe"
            pythonw = Path(temporary) / "pythonw.exe"
            pythonw.touch()
            source = windows_launcher_command(executable=python)
        self.assertIn("pythonw.exe", source)
        self.assertIn("-m keyswitch --hidden", source)
        with tempfile.TemporaryDirectory() as temporary:
            source_without_pythonw = windows_launcher_command(
                executable=Path(temporary) / "python.exe"
            )
        self.assertIn("python.exe", source_without_pythonw)

    def test_application_catalog_deduplicates_and_parses_picker_paths(self) -> None:
        registry = FakeRegistry()
        registry.apps = (
            ("notepad.exe", r"C:\Windows\notepad.exe"),
            ("NOTEPAD.EXE", r"D:\Other\notepad.exe"),
            ("", ""),
            ("Браузер", r'"C:\Program Files\Browser\browser.exe",0'),
        )
        applications = WindowsApplicationCatalog(registry).installed()
        self.assertEqual([item.identifier for item in applications], ["notepad", "browser"])
        self.assertEqual(applications[1].name, "Браузер")
        self.assertEqual(
            applications[1].executable,
            r"C:\Program Files\Browser\browser.exe",
        )
        selected = WindowsApplicationCatalog.from_executable(
            '"C:\\Program Files\\Editor\\Editor.exe"'
        )
        self.assertIsNotNone(selected)
        assert selected is not None
        self.assertEqual(selected.identifier, "editor")
        self.assertEqual(
            clean_windows_executable(
                r'"C:\Program Files\Tool\tool.exe" -silent'
            ),
            r"C:\Program Files\Tool\tool.exe",
        )
        self.assertEqual(
            clean_windows_executable(r"C:\Tool\tool.exe,-12"),
            r"C:\Tool\tool.exe",
        )
        self.assertEqual(clean_windows_executable('"unterminated.exe'), "unterminated.exe")
        self.assertIsNone(WindowsApplicationCatalog.from_executable(""))

    def test_native_registry_factory_is_guarded_outside_windows(self) -> None:
        with patch("keyswitch.windows_system._running_on_windows", return_value=False):
            with self.assertRaisesRegex(WindowsSystemError, "только в Windows"):
                WindowsAutostartManager()
            with self.assertRaisesRegex(WindowsSystemError, "только в Windows"):
                WindowsApplicationCatalog()

    def test_native_registry_factory_can_create_the_isolated_adapter(self) -> None:
        registry = FakeRegistry()
        native_module = ModuleType("keyswitch.windows_registry")
        setattr(native_module, "NativeWindowsRegistry", lambda: registry)
        with (
            patch("keyswitch.windows_system._running_on_windows", return_value=True),
            patch.dict(sys.modules, {"keyswitch.windows_registry": native_module}),
        ):
            manager = WindowsAutostartManager(command="KeySwitch.exe --hidden")
            catalog = WindowsApplicationCatalog()
        manager.set_enabled(True)
        self.assertTrue(manager.enabled())
        self.assertEqual(catalog.installed(), ())


class WindowsSingleInstanceTests(unittest.TestCase):
    def test_lifecycle_duplicate_activation_and_idempotent_close(self) -> None:
        api = FakeInstanceAPI(acquired=False, activated=True)
        instance = WindowsSingleInstance(api)
        self.assertFalse(instance.acquire())
        self.assertTrue(instance.activate_existing())
        instance.close()
        instance.close()
        self.assertFalse(instance.acquire())
        self.assertFalse(instance.activate_existing())
        self.assertEqual(
            (api.acquire_calls, api.activate_calls, api.close_calls),
            (1, 1, 1),
        )

    def test_native_factory_is_guarded_and_replaceable(self) -> None:
        from keyswitch import windows_instance

        with patch("keyswitch.windows_instance._running_on_windows", return_value=False):
            with self.assertRaisesRegex(WindowsSystemError, "только в Windows"):
                WindowsSingleInstance()

        api = FakeInstanceAPI()
        native_module = ModuleType("keyswitch.windows_instance_native")
        setattr(native_module, "CtypesWindowsInstanceAPI", lambda: api)
        with (
            patch("keyswitch.windows_instance._running_on_windows", return_value=True),
            patch.dict(
                sys.modules,
                {"keyswitch.windows_instance_native": native_module},
            ),
        ):
            instance = windows_instance.WindowsSingleInstance()
        self.assertTrue(instance.acquire())
        instance.close()


class WindowsTrayTests(unittest.TestCase):
    def test_primary_click_maps_to_the_popup_menu_notification(self) -> None:
        self.assertEqual(menu_activation_message(10, 10, 20), 20)
        self.assertEqual(menu_activation_message(30, 10, 20), 30)

    def test_state_updates_notifications_actions_and_idempotent_close(self) -> None:
        calls: list[str] = []
        actions = WindowsTrayActions(
            *(lambda name=name: calls.append(name) for name in (
                "settings",
                "layout",
                "engine",
                "sound",
                "notifications",
                "history",
                "exclusions",
                "about",
                "quit",
            ))
        )
        adapter = FakeTrayAdapter()
        tray = WindowsTray(actions, adapter)
        self.assertEqual(tray.state.label, "—")
        self.assertEqual(tray.state.alternate_layout_label, "Переключить язык")
        self.assertFalse(tray.state.can_switch_layout)
        tray.set_layout(1)
        tray.set_enabled(False)
        tray.set_sound_enabled(True)
        tray.set_notifications_enabled(False)
        tray.set_indicator_style("flags")
        tray.set_indicator_style("invalid")
        self.assertEqual(tray.state.label, "RU")
        self.assertEqual(
            tray.state.alternate_layout_label,
            "Переключить на английский (EN)",
        )
        self.assertTrue(tray.state.can_switch_layout)
        self.assertEqual(tray.state.indicator_style, "letters")
        tray.notify("Исправлено", "ghbdtn → привет")
        self.assertEqual(adapter.notifications, [("Исправлено", "ghbdtn → привет")])

        self.assertIsNotNone(adapter.actions)
        assert adapter.actions is not None
        for action in (
            adapter.actions.show_settings,
            adapter.actions.switch_layout,
            adapter.actions.toggle_engine,
            adapter.actions.toggle_sound,
            adapter.actions.toggle_notifications,
            adapter.actions.show_history,
            adapter.actions.show_exclusions,
            adapter.actions.show_about,
            adapter.actions.quit_application,
        ):
            action()
        self.assertEqual(
            calls,
            [
                "settings",
                "layout",
                "engine",
                "sound",
                "notifications",
                "history",
                "exclusions",
                "about",
                "quit",
            ],
        )
        self.assertIsNotNone(adapter.state_reader)
        assert adapter.state_reader is not None
        self.assertEqual(adapter.state_reader(), tray.state)

        before = len(adapter.states)
        tray.close()
        tray.close()
        tray.set_layout(0)
        tray.notify("ignored", "closed")
        self.assertEqual(adapter.close_calls, 1)
        self.assertEqual(len(adapter.states), before)
        self.assertEqual(len(adapter.notifications), 1)

    def test_native_tray_factory_is_an_isolated_boundary(self) -> None:
        adapter = FakeTrayAdapter()
        native_module = ModuleType("keyswitch.windows_tray_native")
        setattr(native_module, "PystrayWindowsAdapter", lambda: adapter)
        with patch.dict(
            sys.modules,
            {"keyswitch.windows_tray_native": native_module},
        ):
            self.assertIs(_native_adapter(), adapter)


class WindowsUIModelTests(unittest.TestCase):
    def test_catalogue_is_unique_complete_and_uses_valid_control_metadata(self) -> None:
        paths = [spec.path for spec in ALL_SETTING_SPECS]
        self.assertEqual(len(paths), len(set(paths)))
        for spec in ALL_SETTING_SPECS:
            value: object = DEFAULTS
            for part in spec.path.split("."):
                self.assertIsInstance(value, dict)
                assert isinstance(value, dict)
                self.assertIn(part, value)
                value = value[part]
            self.assertTrue(spec.title)
            self.assertTrue(spec.description)
            if spec.kind == "choice":
                self.assertTrue(spec.choices)
                self.assertIn(str(value), dict(spec.choices))
            if spec.kind in {"int", "float"}:
                self.assertLess(spec.minimum, spec.maximum)
                self.assertGreater(spec.step, 0)


class WindowsApplicationEntrypointTests(unittest.TestCase):
    def test_logging_parser_diagnostics_and_ui_dispatch(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            handler = logging.NullHandler()
            with (
                patch("keyswitch.windows_app.data_dir", return_value=directory),
                patch("keyswitch.windows_app.logging.basicConfig") as basic,
                patch(
                    "keyswitch.windows_app.RotatingFileHandler",
                    return_value=handler,
                ) as rotating,
            ):
                windows_app_module.configure_logging()
            self.assertTrue(directory.is_dir())
            self.assertEqual(basic.call_args.kwargs["level"], logging.INFO)
            rotating.assert_called_once_with(
                directory / "keyswitch.log",
                maxBytes=5 * 1024 * 1024,
                backupCount=3,
                encoding="utf-8",
            )
            self.assertEqual(basic.call_args.kwargs["handlers"], [handler])

        api = FakeWindowsAPI()
        backend = WindowsBackend(api)
        output = io.StringIO()
        with (
            patch("keyswitch.windows_app.WindowsBackend", return_value=backend),
            contextlib.redirect_stdout(output),
        ):
            self.assertEqual(windows_app_module.diagnose(), 0)
        payload = json.loads(output.getvalue())
        self.assertTrue(payload["available"])
        self.assertEqual(payload["hook"], "WH_KEYBOARD_LL")

        failed_api = FakeWindowsAPI()
        failed_api.layout_values = (ENGLISH_LAYOUT,)
        failed = WindowsBackend(failed_api)
        with (
            patch("keyswitch.windows_app.WindowsBackend", return_value=failed),
            contextlib.redirect_stdout(io.StringIO()),
        ):
            self.assertEqual(windows_app_module.diagnose(), 1)

        arguments = windows_app_module.build_parser().parse_args(
            ["--hidden", "--no-engine"]
        )
        self.assertTrue(arguments.hidden)
        self.assertTrue(arguments.no_engine)

        with (
            patch("keyswitch.windows_app.configure_logging"),
            patch("keyswitch.windows_app.diagnose", return_value=7) as diagnose,
        ):
            self.assertEqual(windows_app_module.main(["--diagnose"]), 7)
        diagnose.assert_called_once_with()

        calls: list[tuple[bool, bool, int | None]] = []
        fake_ui = ModuleType("keyswitch.windows_ui")

        def run_windows_application(
            *,
            hidden: bool,
            no_engine: bool,
            quit_after_ms: int | None = None,
        ) -> int:
            calls.append((hidden, no_engine, quit_after_ms))
            return 9

        setattr(fake_ui, "run_windows_application", run_windows_application)
        guard_apis = (FakeInstanceAPI(), FakeInstanceAPI())
        guards = tuple(WindowsSingleInstance(api) for api in guard_apis)
        with (
            patch("keyswitch.windows_app.configure_logging"),
            patch(
                "keyswitch.windows_instance.WindowsSingleInstance",
                side_effect=guards,
            ),
            patch.dict(sys.modules, {"keyswitch.windows_ui": fake_ui}),
        ):
            self.assertEqual(
                windows_app_module.main(["--hidden", "--no-engine"]),
                9,
            )
            self.assertEqual(windows_app_module.main(["--smoke-ui"]), 9)
        self.assertEqual(calls, [(True, True, None), (False, True, 300)])
        self.assertEqual([api.close_calls for api in guard_apis], [1, 1])

        duplicate_api = FakeInstanceAPI(acquired=False)
        duplicate = WindowsSingleInstance(duplicate_api)
        with (
            patch("keyswitch.windows_app.configure_logging"),
            patch(
                "keyswitch.windows_instance.WindowsSingleInstance",
                return_value=duplicate,
            ),
        ):
            self.assertEqual(windows_app_module.main([]), 0)
        self.assertEqual(
            (duplicate_api.acquire_calls, duplicate_api.activate_calls, duplicate_api.close_calls),
            (1, 1, 1),
        )

    def test_windows_app_module_direct_execution_uses_diagnostic_exit(self) -> None:
        api = FakeWindowsAPI()
        backend = WindowsBackend(api)
        with tempfile.TemporaryDirectory() as temporary:
            with (
                patch.object(sys, "argv", ["windows_app.py", "--diagnose"]),
                patch("keyswitch.windows_backend.WindowsBackend", return_value=backend),
                patch("keyswitch.history.data_dir", return_value=Path(temporary)),
                patch("logging.basicConfig"),
                patch(
                    "logging.handlers.RotatingFileHandler",
                    return_value=logging.NullHandler(),
                ),
                contextlib.redirect_stdout(io.StringIO()),
                warnings.catch_warnings(),
                self.assertRaises(SystemExit) as stopped,
            ):
                warnings.simplefilter("ignore", RuntimeWarning)
                runpy.run_module("keyswitch.windows_app", run_name="__main__")
        self.assertEqual(stopped.exception.code, 0)


if __name__ == "__main__":
    unittest.main()
