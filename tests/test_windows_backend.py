"""Platform-independent verification of the Win32 keyboard backend."""

from __future__ import annotations

import contextlib
import ctypes
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
from pathlib import Path, PureWindowsPath
from types import ModuleType
from unittest.mock import patch

from keyswitch import logsetup
from keyswitch import windows_app as windows_app_module
from keyswitch import launcher as launcher_module
from keyswitch.backend import BackendProbe, FocusInfo, KeyEvent, ScreenAnchor
from keyswitch.constants.keyboard import (
    ALT_MASK,
    CONTROL_MASK,
    LAYOUT_GROUP_COUNT,
    LOCK_MASK,
    SHIFT_MASK,
    SUPER_MASK,
)
from keyswitch.constants.models import PREFIX_MAX_CHARACTERS, PREFIX_MIN_CHARACTERS
from keyswitch.constants.timing import (
    LAYOUT_SWITCH_POLL_SECONDS,
    SMOKE_UI_QUIT_AFTER_MS,
    UNDO_AVAILABLE_WINDOW_SECONDS,
)
from keyswitch.constants.units import SECONDS_PER_HOUR
from keyswitch.constants.updates import UPDATE_CHECK_INTERVAL_SECONDS
from keyswitch import config, history
from keyswitch.constants.settings_defaults import (
    DEFAULT_EARLY_SWITCH_MIN_LENGTH,
    DEFAULT_SETTINGS,
    EARLY_SWITCH_MIN_LENGTH_SETTING_MIN,
)
from keyswitch.russian_text import HOURS, SECONDS, quantity
from keyswitch.engine import _default_backend
from keyswitch.windows_instance import WindowsSingleInstance
from keyswitch.windows_backend import (
    NativeInput,
    NativeKeyEvent,
    WindowsBackend,
    WindowsBackendError,
    key_name,
    primary_language,
    select_layout_pair,
)
from keyswitch.constants.windows import (
    GA_ROOT,
    LANG_ENGLISH,
    LANG_RUSSIAN,
    STARTUP_APPROVAL_ENABLED_BYTES,
    VK_BACK,
    VK_CAPITAL,
    VK_CONTROL,
    VK_LWIN,
    VK_MENU,
    VK_OEM_7,
    VK_OEM_COMMA,
    VK_OEM_PERIOD,
    VK_RETURN,
    VK_SHIFT,
)
from keyswitch.windows_system import (
    AutostartStatus,
    _executable_exists,
    _installed_executable,
    WindowsApplicationCatalog,
    WindowsAutostartManager,
    WindowsSystemError,
    clean_windows_executable,
    open_directory,
    windows_launcher_command,
)
from keyswitch.windows_tray import (
    WindowsTray,
    WindowsTrayActions,
    WindowsTrayState,
    _native_adapter,
    menu_activation_message,
)
from keyswitch.windows_native import CtypesWindowsAPI
from keyswitch.windows_ui_model import ALL_SETTING_SPECS
from fixture_values.clock import (
    LAYOUT_SWITCH_POLL_MONOTONIC_READINGS,
    SHORT_LISTENER_START_TIMEOUT_SECONDS,
    WINDOWS_CAPS_LOCK_REPEAT_TIMESTAMP,
    WINDOWS_FAKE_KEY_TIMESTAMP,
    WINDOWS_FILTERED_PRESS_TIMESTAMP,
    WINDOWS_FILTERED_RELEASE_TIMESTAMP,
    WINDOWS_FILTER_CLEARED_PRESS_TIMESTAMP,
    WINDOWS_HELD_KEY_PRESS_TIMESTAMP,
    WINDOWS_HELD_KEY_RELEASE_TIMESTAMP,
    WINDOWS_INJECTED_KEY_TIMESTAMP,
    WINDOWS_INJECTED_PRESS_TIMESTAMP,
    WINDOWS_MODIFIER_RELEASE_TIMESTAMP,
    WINDOWS_NO_FILTER_PRESS_TIMESTAMP,
    WINDOWS_NO_LISTENER_RELEASE_TIMESTAMP,
    WINDOWS_POST_HOLD_PRESS_TIMESTAMP,
    WINDOWS_UNFILTERED_KEY_PRESS_TIMESTAMP,
    WINDOWS_UNKNOWN_LAYOUT_PRESS_TIMESTAMP,
    WINDOWS_UNMATCHED_RELEASE_TIMESTAMP,
)
from fixture_values.counts import (
    EXPECTED_EARLY_SWITCH_MIN_LENGTH_SETTING_MAX,
    EXPECTED_EARLY_SWITCH_MIN_LENGTH_SETTING_MIN,
    OVERLONG_PATH_CHARACTERS,
    WINDOWS_EXPECTED_BACKSPACE_EVENTS,
    WINDOWS_EXPECTED_DELIVERED_COUNT,
    WINDOWS_EXPECTED_HELD_COUNT,
    WINDOWS_EXPECTED_SEND_BATCHES,
    WINDOWS_SUBTEST_PATH_LABEL_CHARACTERS,
)
from fixture_values.keys import (
    FAKE_FOREGROUND_PROCESS_ID,
    HELD_KEY_SCAN_CODE,
    INJECTED_KEY_SCAN_CODE,
    LATE_KEY_SCAN_CODE,
    NONEXISTENT_LAYOUT_GROUP,
    SCAN_CODE_A,
    SCAN_CODE_ENTER,
    SCAN_CODE_SEMICOLON,
    UNSUPPORTED_LAYOUT_GROUP,
    WINDOWS_FAKE_HWND,
    WINDOWS_INACTIVE_HWND,
    WINDOWS_INVALID_SOURCE_GROUP,
    WINDOWS_OTHER_HWND,
    WINDOWS_OWN_PROCESS_ID,
    WINDOWS_ROOT_HWND,
    WINDOWS_UNNAMED_VIRTUAL_KEY,
)
from fixture_values.platform import (
    ABSTRACT_MENU_CLICK_MESSAGE,
    ABSTRACT_OTHER_MESSAGE,
    ABSTRACT_PRIMARY_CLICK_MESSAGE,
    ENGLISH_HKL,
    ENGLISH_HKL_WITH_DEVICE_FLAGS,
    ENGLISH_HKL_WITH_HANDLE,
    FAKE_DIAGNOSE_EXIT_CODE,
    FAKE_WINDOWS_UI_EXIT_CODE,
    GERMAN_HKL,
    RUSSIAN_HKL,
    RUSSIAN_HKL_WITH_HANDLE,
    STARTUP_APPROVAL_DISABLED_BYTE,
    STARTUP_APPROVAL_PADDING_BYTES,
)
from fixture_values.ui import WINDOWS_FAKE_ANCHOR_X, WINDOWS_FAKE_ANCHOR_Y


FAKE_ANCHOR = ScreenAnchor(WINDOWS_FAKE_ANCHOR_X, WINDOWS_FAKE_ANCHOR_Y, WINDOWS_FAKE_HWND)


class FakeWindowsAPI:

    def __init__(self) -> None:
        self.layout_values: tuple[int, ...] = (ENGLISH_HKL, RUSSIAN_HKL)
        self.current_layout = ENGLISH_HKL
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
        self.hook_listener: Callable[[NativeKeyEvent], bool] | None = None
        self.stop_calls = 0
        self.layout_calls = 0
        self.translation: dict[tuple[int, int], str] = {}
        self.anchor: ScreenAnchor | None = FAKE_ANCHOR
        self.activated_windows: list[int] = []
        self.foreground = WINDOWS_FAKE_HWND
        self.window_owners: dict[int, int] = {WINDOWS_FAKE_HWND: FAKE_FOREGROUND_PROCESS_ID}
        self.process_id = WINDOWS_OWN_PROCESS_ID
        self.inactive_windows: list[int] = []

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

    def foreground_window(self) -> int:
        return self.foreground

    def focused_control(self) -> int:
        return self.foreground

    def window_process_id(self, window: int) -> int:
        return self.window_owners.get(window, 0)

    def current_process_id(self) -> int:
        return self.process_id

    def keep_window_inactive(self, window: int) -> bool:
        self.inactive_windows.append(window)
        return True

    def caps_lock_enabled(self) -> bool:
        return self.caps_lock

    def run_keyboard_hook(
        self,
        listener: Callable[[NativeKeyEvent], bool],
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


class FakeActivationUser32:
    def __init__(self, *, root: int | None, activated: bool = True) -> None:
        self.root = root
        self.activated = activated
        self.ancestor_calls: list[tuple[int, int]] = []
        self.foreground_calls: list[int] = []
        self.show_calls: list[tuple[int, int]] = []

    def GetAncestor(self, handle: ctypes.c_void_p, flag: int) -> int | None:
        self.ancestor_calls.append((int(handle.value or 0), flag))
        return self.root

    def SetForegroundWindow(self, handle: ctypes.c_void_p | int) -> int:
        value = (
            int(handle.value or 0)
            if isinstance(handle, ctypes.c_void_p)
            else handle
        )
        self.foreground_calls.append(value)
        return int(self.activated)

    def ShowWindow(self, handle: ctypes.c_void_p | int, command: int) -> int:
        value = (
            int(handle.value or 0)
            if isinstance(handle, ctypes.c_void_p)
            else handle
        )
        self.show_calls.append((value, command))
        return 1


class FakeRegistry:
    def __init__(self) -> None:
        self.autostart: dict[str, str] = {}
        self.startup_approval: dict[str, bytes] = {}
        self.apps: tuple[tuple[str, str], ...] = ()

    def read_autostart(self, name: str) -> str | None:
        return self.autostart.get(name)

    def write_autostart(self, name: str, command: str) -> None:
        self.autostart[name] = command

    def delete_autostart(self, name: str) -> None:
        self.autostart.pop(name, None)

    def read_startup_approval(self, name: str) -> bytes | None:
        return self.startup_approval.get(name)

    def clear_startup_approval(self, name: str) -> None:
        self.startup_approval.pop(name, None)

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
    keycode: int = SCAN_CODE_A,
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
        WINDOWS_FAKE_KEY_TIMESTAMP,
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
            self.assertIs(_default_backend(LAYOUT_GROUP_COUNT), backend)
        factory.assert_called_once_with(group_count=LAYOUT_GROUP_COUNT)

    def test_layout_languages_pair_selection_and_key_names(self) -> None:
        self.assertEqual(
            primary_language(ENGLISH_HKL_WITH_DEVICE_FLAGS), LANG_ENGLISH
        )
        self.assertEqual(primary_language(RUSSIAN_HKL), LANG_RUSSIAN)
        self.assertEqual(
            select_layout_pair((RUSSIAN_HKL, ENGLISH_HKL, ENGLISH_HKL)),
            (ENGLISH_HKL, RUSSIAN_HKL),
        )
        with self.assertRaisesRegex(WindowsBackendError, "английская и русская"):
            select_layout_pair((ENGLISH_HKL,))
        with self.assertRaisesRegex(WindowsBackendError, "английская и русская"):
            select_layout_pair((RUSSIAN_HKL,))
        self.assertEqual(key_name(VK_BACK), "BackSpace")
        self.assertEqual(key_name(ord("A")), "a")
        self.assertEqual(key_name(ord("7")), "7")
        self.assertEqual(key_name(WINDOWS_UNNAMED_VIRTUAL_KEY), "VK_FE")
        # Punctuation keys carry the X11 keysym names, so a log line reads
        # "comma", never "VK_BC".
        self.assertEqual(key_name(VK_OEM_COMMA), "comma")
        self.assertEqual(key_name(VK_OEM_PERIOD), "period")
        self.assertEqual(key_name(VK_OEM_7), "apostrophe")

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
        self.assertEqual(backend.layouts, (ENGLISH_HKL, RUSSIAN_HKL))

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
        self.assertEqual(event.character_for(NONEXISTENT_LAYOUT_GROUP), "")


class WindowsNativeActivationTests(unittest.TestCase):

    @staticmethod
    def api_with(user32: FakeActivationUser32) -> CtypesWindowsAPI:
        api = CtypesWindowsAPI.__new__(CtypesWindowsAPI)
        object.__setattr__(api, "user32", user32)
        return api

    def test_activate_window_preserves_top_level_show_state(self) -> None:
        user32 = FakeActivationUser32(root=WINDOWS_ROOT_HWND)
        api = self.api_with(user32)

        self.assertTrue(api.activate_window(WINDOWS_FAKE_HWND))
        self.assertEqual(user32.ancestor_calls, [(WINDOWS_FAKE_HWND, GA_ROOT)])
        self.assertEqual(user32.foreground_calls, [WINDOWS_ROOT_HWND])
        self.assertEqual(user32.show_calls, [])

    def test_activate_window_falls_back_to_child_and_reports_failure(self) -> None:
        user32 = FakeActivationUser32(root=None, activated=False)
        api = self.api_with(user32)

        self.assertFalse(api.activate_window(WINDOWS_FAKE_HWND))
        self.assertEqual(user32.ancestor_calls, [(WINDOWS_FAKE_HWND, GA_ROOT)])
        self.assertEqual(user32.foreground_calls, [WINDOWS_FAKE_HWND])
        self.assertEqual(user32.show_calls, [])


class WindowsBackendLifecycleTests(unittest.TestCase):

    def test_layouts_probe_group_and_application(self) -> None:
        api = FakeWindowsAPI()
        backend = WindowsBackend(api)
        self.assertEqual(backend.layouts, (ENGLISH_HKL, RUSSIAN_HKL))
        self.assertEqual(backend.layouts, (ENGLISH_HKL, RUSSIAN_HKL))
        self.assertEqual(api.layout_calls, 1)
        probe = backend.probe()
        self.assertTrue(probe.available)
        self.assertEqual(probe.session_type, "windows")
        self.assertIn("00000409", probe.xkb_version)
        self.assertEqual(backend.current_group(), 0)
        self.assertEqual(backend.active_application(), "Notepad")
        self.assertEqual(backend.input_anchor(), FAKE_ANCHOR)
        self.assertFalse(backend.restore_window(None))
        self.assertTrue(backend.restore_window(WINDOWS_FAKE_HWND))
        self.assertEqual(api.activated_windows, [WINDOWS_FAKE_HWND])

        api.current_layout = ENGLISH_HKL_WITH_HANDLE
        self.assertEqual(backend.current_group(), 0)
        api.current_layout = RUSSIAN_HKL_WITH_HANDLE
        self.assertEqual(backend.current_group(), 1)
        api.current_layout = GERMAN_HKL
        self.assertEqual(backend.current_group(), -1)

        with self.assertRaisesRegex(WindowsBackendError, "Неизвестная группа"):
            backend.switch_group(UNSUPPORTED_LAYOUT_GROUP)
        backend.switch_group(1)
        self.assertEqual(api.requests[-1], RUSSIAN_HKL)

    def test_probe_reports_layout_failure(self) -> None:
        api = FakeWindowsAPI()
        api.layout_values = (ENGLISH_HKL,)
        probe = WindowsBackend(api).probe()
        self.assertFalse(probe.available)
        self.assertEqual(probe.current_group, -1)
        self.assertIn("русская", probe.error)

    def test_start_events_idempotence_stop_and_close(self) -> None:
        api = FakeWindowsAPI()
        api.translation[(ord("A"), ENGLISH_HKL)] = "a"
        api.translation[(ord("A"), RUSSIAN_HKL)] = "ф"
        api.caps_lock = True
        backend = WindowsBackend(api)
        events: list[KeyEvent] = []
        backend.start(events.append)
        self.assertTrue(backend.running)
        backend.start(events.append)
        listener = api.hook_listener
        self.assertIsNotNone(listener)
        assert listener is not None
        listener(
            NativeKeyEvent(
                True, ord("A"), SCAN_CODE_A, False, True, WINDOWS_INJECTED_PRESS_TIMESTAMP
            )
        )
        self.assertEqual(len(events), 1)
        self.assertEqual(events[0].characters, ("a", "ф"))
        self.assertEqual(events[0].character, "a")
        self.assertTrue(events[0].synthetic)
        self.assertTrue(events[0].caps_lock)

        # Without a filter every key reaches the window.
        self.assertFalse(
            listener(
                NativeKeyEvent(
                    True, VK_RETURN, SCAN_CODE_ENTER, False, False,
                    WINDOWS_NO_FILTER_PRESS_TIMESTAMP,
                )
            )
        )

        # The filter answers for presses; the matching release is hidden with
        # them so the window never sees a key-up it has no key-down for.
        consumed: list[KeyEvent] = []

        def only_enter(event: KeyEvent) -> bool:
            consumed.append(event)
            return event.key_name == "Return"

        backend.set_key_filter(only_enter)
        self.assertTrue(
            listener(
                NativeKeyEvent(
                    True, VK_RETURN, SCAN_CODE_ENTER, False, False,
                    WINDOWS_FILTERED_PRESS_TIMESTAMP,
                )
            )
        )
        self.assertTrue(
            listener(
                NativeKeyEvent(
                    False, VK_RETURN, SCAN_CODE_ENTER, False, False,
                    WINDOWS_FILTERED_RELEASE_TIMESTAMP,
                )
            )
        )
        # A release with no swallowed press behind it passes through.
        self.assertFalse(
            listener(
                NativeKeyEvent(
                    False, VK_RETURN, SCAN_CODE_ENTER, False, False,
                    WINDOWS_UNMATCHED_RELEASE_TIMESTAMP,
                )
            )
        )
        self.assertFalse(
            listener(
                NativeKeyEvent(
                    True, ord("A"), SCAN_CODE_A, False, False,
                    WINDOWS_UNFILTERED_KEY_PRESS_TIMESTAMP,
                )
            )
        )
        self.assertEqual(
            [event.key_name for event in consumed], ["Return", "a"]
        )

        backend.set_key_filter(None)
        self.assertFalse(
            listener(
                NativeKeyEvent(
                    True, VK_RETURN, SCAN_CODE_ENTER, False, False,
                    WINDOWS_FILTER_CLEARED_PRESS_TIMESTAMP,
                )
            )
        )
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
        with patch(
            "keyswitch.windows_backend.KEYBOARD_LISTENER_START_TIMEOUT_SECONDS", SHORT_LISTENER_START_TIMEOUT_SECONDS
        ):
            with self.assertRaisesRegex(
                WindowsBackendError, f"не подтвердил запуск за {quantity(SHORT_LISTENER_START_TIMEOUT_SECONDS, SECONDS)}"
            ):
                backend.start(lambda _event: None)
        self.assertEqual(api.stop_calls, 1)

    def test_focused_window_marks_windows_of_this_process(self) -> None:
        api = FakeWindowsAPI()
        backend = WindowsBackend(api)
        self.assertEqual(backend.focused_window(), FocusInfo(WINDOWS_FAKE_HWND, False))
        api.window_owners[WINDOWS_FAKE_HWND] = api.process_id
        self.assertEqual(backend.focused_window(), FocusInfo(WINDOWS_FAKE_HWND, True, True))
        api.foreground = WINDOWS_OTHER_HWND
        self.assertEqual(backend.focused_window(), FocusInfo(WINDOWS_OTHER_HWND, False))
        api.foreground = 0
        self.assertIsNone(backend.focused_window())
        self.assertTrue(backend.keep_window_inactive(WINDOWS_INACTIVE_HWND))
        self.assertEqual(api.inactive_windows, [WINDOWS_INACTIVE_HWND])

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
            NativeKeyEvent(
                True, VK_CAPITAL, VK_CAPITAL, False, False,
                WINDOWS_CAPS_LOCK_REPEAT_TIMESTAMP,
            )
        )
        self.assertEqual(
            collected[-1].state,
            SHIFT_MASK | CONTROL_MASK | ALT_MASK | SUPER_MASK | LOCK_MASK,
        )
        for virtual_key in (VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_CAPITAL):
            backend._handle_native(
                NativeKeyEvent(
                    False, virtual_key, virtual_key, False, False,
                    WINDOWS_MODIFIER_RELEASE_TIMESTAMP,
                )
            )
        self.assertEqual(collected[-1].state, LOCK_MASK)

        api.current_layout = GERMAN_HKL
        backend._handle_native(
            NativeKeyEvent(
                True, ord("A"), SCAN_CODE_A, False, False,
                WINDOWS_UNKNOWN_LAYOUT_PRESS_TIMESTAMP,
            )
        )
        self.assertEqual(collected[-1].character, "")
        backend._listener = None
        backend._handle_native(
            NativeKeyEvent(
                False, ord("A"), SCAN_CODE_A, False, False,
                WINDOWS_NO_LISTENER_RELEASE_TIMESTAMP,
            )
        )


class WindowsBackendInjectionTests(unittest.TestCase):

    def test_correction_deletes_switches_replays_shift_and_boundary(self) -> None:
        api = FakeWindowsAPI()
        backend = WindowsBackend(api)
        stroke = key_event(state=SHIFT_MASK)
        boundary = key_event(
            keycode=SCAN_CODE_SEMICOLON,
            character=";",
            characters=(";", "ж"),
        )
        self.assertEqual(backend.inject_correction((stroke,), 1, boundary, source_group=0), 0)

        # The layout switches first, then deletion and replacement travel in
        # one SendInput call so the user's keys cannot slip in between.
        self.assertEqual(
            api.sent[0],
            (
                NativeInput(True, virtual_key=VK_BACK),
                NativeInput(False, virtual_key=VK_BACK),
                NativeInput(True, virtual_key=VK_BACK),
                NativeInput(False, virtual_key=VK_BACK),
                NativeInput(True, virtual_key=VK_SHIFT),
                NativeInput(True, scan_code=SCAN_CODE_A),
                NativeInput(False, scan_code=SCAN_CODE_A),
                NativeInput(False, virtual_key=VK_SHIFT),
            ),
        )
        # The boundary keeps its own layout: typed after a switch back.
        self.assertEqual(api.sent[1][0].scan_code, SCAN_CODE_SEMICOLON)
        self.assertEqual(len(api.sent), WINDOWS_EXPECTED_SEND_BATCHES)
        self.assertEqual(
            api.requests,
            [RUSSIAN_HKL, ENGLISH_HKL, RUSSIAN_HKL],
        )

    def test_boundary_can_stay_in_target_and_empty_batches_are_skipped(self) -> None:
        api = FakeWindowsAPI()
        backend = WindowsBackend(api)
        boundary = key_event(character=" ", characters=(" ", " "))
        backend.inject_correction((), 0, None)
        self.assertEqual(api.sent, [])
        backend.inject_correction((key_event(),), 1, boundary)
        self.assertEqual(api.requests, [RUSSIAN_HKL])
        self.assertEqual(len(api.sent), 1)
        self.assertEqual(
            [item.scan_code for item in api.sent[0] if item.scan_code],
            [SCAN_CODE_A, SCAN_CODE_A, SCAN_CODE_A, SCAN_CODE_A],
        )

    def test_invalid_groups_partial_send_and_rejected_switch_fail_loudly(self) -> None:
        api = FakeWindowsAPI()
        backend = WindowsBackend(api)
        with self.assertRaisesRegex(WindowsBackendError, "Неизвестная группа"):
            backend.inject_correction((), UNSUPPORTED_LAYOUT_GROUP, None)
        with self.assertRaisesRegex(WindowsBackendError, "исходная группа"):
            backend.inject_correction(
                (key_event(group=WINDOWS_INVALID_SOURCE_GROUP),), 1, None
            )

        api.send_count = 0
        with self.assertRaisesRegex(WindowsBackendError, "SendInput"):
            backend.inject_correction((key_event(),), 1, None)

        api.send_count = None
        # The layout now switches before anything is sent, so the refused
        # switch needs a layout that still has to change.
        api.current_layout = ENGLISH_HKL
        api.accept_switch = False
        with self.assertRaisesRegex(WindowsBackendError, "отклонило"):
            backend.inject_correction((key_event(),), 1, None)

    def test_late_and_held_keys_are_deleted_and_typed_again_as_the_users_own(self) -> None:
        api = FakeWindowsAPI()
        backend = WindowsBackend(api)
        delivered: list[KeyEvent] = []
        backend.start(delivered.append)
        listener = api.hook_listener
        assert listener is not None
        backend.hold_input()
        # Held keys are swallowed and never reach the engine directly.
        self.assertTrue(
            listener(
                NativeKeyEvent(
                    True, ord("D"), HELD_KEY_SCAN_CODE, False, False,
                    WINDOWS_HELD_KEY_PRESS_TIMESTAMP,
                )
            )
        )
        self.assertTrue(
            listener(
                NativeKeyEvent(
                    False, ord("D"), HELD_KEY_SCAN_CODE, False, False,
                    WINDOWS_HELD_KEY_RELEASE_TIMESTAMP,
                )
            )
        )
        # The backend's own injection is never held.
        self.assertFalse(
            listener(
                NativeKeyEvent(
                    True, ord("X"), INJECTED_KEY_SCAN_CODE, False, True,
                    WINDOWS_INJECTED_KEY_TIMESTAMP,
                )
            )
        )
        self.assertEqual([event.synthetic for event in delivered], [True])

        late = key_event(keycode=LATE_KEY_SCAN_CODE)
        held = backend.inject_correction((key_event(),), 1, None, source_group=0, late=(late,))
        self.assertEqual(held, WINDOWS_EXPECTED_HELD_COUNT)
        self.assertEqual(api.requests, [RUSSIAN_HKL])
        batch, late_sent, *held_batches = api.sent
        held_sent = tuple(item for batch in held_batches for item in batch)
        # Two characters to delete: the word and the late key after it.
        self.assertEqual(
            sum(1 for item in batch if item.virtual_key == VK_BACK),
            WINDOWS_EXPECTED_BACKSPACE_EVENTS,
        )
        self.assertEqual(
            [item.scan_code for item in batch if item.scan_code],
            [SCAN_CODE_A, SCAN_CODE_A],
        )
        self.assertTrue(all(item.synthetic for item in batch))
        self.assertEqual(
            late_sent,
            (
                NativeInput(True, scan_code=LATE_KEY_SCAN_CODE, synthetic=False, replayed=True),
                NativeInput(False, scan_code=LATE_KEY_SCAN_CODE, synthetic=False, replayed=True),
            ),
        )
        self.assertEqual(
            held_sent,
            (
                NativeInput(
                    True, virtual_key=ord("D"), scan_code=HELD_KEY_SCAN_CODE,
                    synthetic=False, replayed=True,
                ),
                NativeInput(
                    False, virtual_key=ord("D"), scan_code=HELD_KEY_SCAN_CODE,
                    synthetic=False, replayed=True,
                ),
            ),
        )
        # The hold is over: the next key is delivered as before.
        self.assertFalse(
            listener(
                NativeKeyEvent(
                    True, ord("A"), SCAN_CODE_A, False, False,
                    WINDOWS_POST_HOLD_PRESS_TIMESTAMP,
                )
            )
        )
        self.assertEqual(len(delivered), WINDOWS_EXPECTED_DELIVERED_COUNT)
        backend.stop()

    def test_a_failed_injection_still_types_the_held_keys_again(self) -> None:
        api = FakeWindowsAPI()
        backend = WindowsBackend(api)
        delivered: list[KeyEvent] = []
        backend.start(delivered.append)
        listener = api.hook_listener
        assert listener is not None

        # The switch is refused before anything was deleted: the late key is
        # still on screen and must not be typed twice; the held one is typed.
        backend.hold_input()
        self.assertTrue(
            listener(
                NativeKeyEvent(
                    True, ord("D"), HELD_KEY_SCAN_CODE, False, False,
                    WINDOWS_HELD_KEY_PRESS_TIMESTAMP,
                )
            )
        )
        api.accept_switch = False
        with self.assertRaisesRegex(WindowsBackendError, "отклонило"):
            backend.inject_correction(
                (key_event(),), 1, None, late=(key_event(keycode=LATE_KEY_SCAN_CODE),)
            )
        self.assertEqual(
            api.sent,
            [(
                NativeInput(
                    True, virtual_key=ord("D"), scan_code=HELD_KEY_SCAN_CODE,
                    synthetic=False, replayed=True,
                ),
            )],
        )
        self.assertFalse(backend._holding)

        # The batch fails after the switch: nothing was deleted either, and
        # a restore that fails too keeps the original error.
        api.accept_switch = True
        api.sent.clear()
        backend.hold_input()
        self.assertTrue(
            listener(
                NativeKeyEvent(
                    False, ord("D"), HELD_KEY_SCAN_CODE, False, False,
                    WINDOWS_HELD_KEY_RELEASE_TIMESTAMP,
                )
            )
        )
        api.send_count = 0
        with self.assertRaisesRegex(WindowsBackendError, "SendInput"):
            backend.inject_correction(
                (key_event(),), 1, None, late=(key_event(keycode=LATE_KEY_SCAN_CODE),)
            )
        self.assertEqual(len(api.sent), WINDOWS_EXPECTED_SEND_BATCHES)
        self.assertFalse(backend._holding)

        # The boundary switch fails after the batch: the late key was deleted,
        # so it is typed again together with the held keys.
        api.send_count = None
        api.sent.clear()
        api.current_layout = ENGLISH_HKL
        boundary = key_event(
            keycode=SCAN_CODE_SEMICOLON, character=";", characters=(";", "ж")
        )
        original_request = api.request_layout

        def refuse_second_switch(layout: int) -> bool:
            if layout == ENGLISH_HKL and len(api.requests) >= 1:
                api.requests.append(layout)
                return False
            return original_request(layout)

        api.request_layout = refuse_second_switch  # type: ignore[method-assign]
        backend.hold_input()
        with self.assertRaisesRegex(WindowsBackendError, "отклонило"):
            backend.inject_correction(
                (key_event(),), 1, boundary, source_group=0,
                late=(key_event(keycode=LATE_KEY_SCAN_CODE),),
            )
        self.assertEqual(
            api.sent[-1],
            (
                NativeInput(True, scan_code=LATE_KEY_SCAN_CODE, synthetic=False, replayed=True),
                NativeInput(False, scan_code=LATE_KEY_SCAN_CODE, synthetic=False, replayed=True),
            ),
        )
        backend.stop()

    def test_switch_timeout_and_already_selected_group(self) -> None:
        api = FakeWindowsAPI()
        backend = WindowsBackend(api)
        backend._switch_group(0)
        self.assertEqual(api.requests, [])

        api.apply_switch = False
        with (
            patch(
                "keyswitch.windows_backend.time.monotonic",
                side_effect=LAYOUT_SWITCH_POLL_MONOTONIC_READINGS,
            ),
            patch("keyswitch.windows_backend.time.sleep") as sleep,
        ):
            with self.assertRaisesRegex(WindowsBackendError, "не подтвердило"):
                backend._switch_group(1)
        sleep.assert_called_once_with(LAYOUT_SWITCH_POLL_SECONDS)


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
        command = '"C:\\Key Switch\\KeySwitch.exe" --hidden'
        manager = WindowsAutostartManager(registry, command=command, exists=lambda path: path == "C:\\Key Switch\\KeySwitch.exe")
        self.assertFalse(manager.enabled())
        manager.set_enabled(True)
        self.assertTrue(manager.enabled())
        self.assertIn("--hidden", registry.autostart["KeySwitch"])
        manager.set_enabled(False)
        self.assertFalse(manager.enabled())

    def test_the_default_target_check_reads_the_file_system_and_survives_a_bad_path(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            present = Path(temporary) / "KeySwitch.exe"
            present.write_bytes(b"binary")
            self.assertTrue(_executable_exists(str(present)))
            self.assertFalse(_executable_exists(str(present / "deeper")))
            for unusable in ("missing\x00path", "x" * OVERLONG_PATH_CHARACTERS, ""):
                with self.subTest(path=unusable[: WINDOWS_SUBTEST_PATH_LABEL_CHARACTERS]):
                    self.assertFalse(_executable_exists(unusable))

    def test_the_launcher_registers_the_installed_executable_not_the_interpreter(self) -> None:
        """A frozen build whose runner reports an interpreter still registers KeySwitch.exe."""
        with tempfile.TemporaryDirectory() as temporary:
            # The code resolves its own location, and Windows hands out 8.3 short
            # names for a temporary directory under a long user name (RUNNER~1).
            root = Path(temporary).resolve() / "KeySwitch"
            (root / "keyswitch").mkdir(parents=True)
            executable = root / "KeySwitch.exe"
            executable.write_bytes(b"binary")
            module = root / "keyswitch" / "windows_system.py"
            module.write_text("", encoding="utf-8")
            with patch("keyswitch.windows_system.__file__", str(module)):
                self.assertEqual(_installed_executable(), executable)
                command = windows_launcher_command(executable=PureWindowsPath(r"C:\Python\python.exe"))  # type: ignore[arg-type]
            self.assertIn("KeySwitch.exe", command)
            self.assertNotIn("-m", command)
            self.assertTrue(command.endswith("--hidden"))
            # A source checkout has no executable beside the package: the interpreter stays.
            (root / "KeySwitch.exe").unlink()
            with patch("keyswitch.windows_system.__file__", str(module)):
                self.assertIsNone(_installed_executable())
                fallback = windows_launcher_command(executable=PureWindowsPath(r"C:\Python\python.exe"))  # type: ignore[arg-type]
            self.assertIn("-m keyswitch", fallback)

    def test_an_unreadable_package_location_falls_back_instead_of_raising(self) -> None:
        """Autostart registration must not fail because the path could not be resolved."""
        with patch("keyswitch.windows_system.Path.resolve", side_effect=OSError("no such path")):
            self.assertIsNone(_installed_executable())

    def test_autostart_reports_what_the_next_logon_will_do(self) -> None:
        registry = FakeRegistry()
        present = "C:\\Programs\\KeySwitch\\KeySwitch.exe"
        command = f'"{present}" --hidden'
        manager = WindowsAutostartManager(registry, command=command, exists=lambda path: path == present)
        manager.set_enabled(True)
        self.assertEqual(manager.status().as_dict(),
                         {"command": command, "blocked_by_windows": False, "target_missing": False, "effective": True})
        # Task Manager's Startup tab disables the value; Windows then skips it at every logon.
        registry.startup_approval["KeySwitch"] = bytes(
            [STARTUP_APPROVAL_DISABLED_BYTE]
        ) + bytes(STARTUP_APPROVAL_PADDING_BYTES)
        self.assertFalse(manager.enabled())
        self.assertTrue(manager.status().blocked_by_windows)
        # The automatic sync at every launch must not overrule that choice.
        manager.set_enabled(True)
        self.assertFalse(manager.enabled())
        # An explicit toggle in the KeySwitch interface does lift it.
        manager.set_enabled(True, override_system_block=True)
        self.assertTrue(manager.enabled())
        self.assertNotIn("KeySwitch", registry.startup_approval)
        padding = STARTUP_APPROVAL_PADDING_BYTES
        for approval, expected in (
            (bytes([STARTUP_APPROVAL_ENABLED_BYTES[0]]) + bytes(padding), True),
            (bytes([STARTUP_APPROVAL_ENABLED_BYTES[1]]) + bytes(padding), True),
            (bytes([0x01]) + bytes(padding), False), (b"", False),
        ):
            with self.subTest(approval=approval.hex()):
                registry.startup_approval["KeySwitch"] = approval
                self.assertEqual(manager.enabled(), expected)
        registry.startup_approval.pop("KeySwitch")
        # A value left behind by another install location cannot start anything either.
        stale = WindowsAutostartManager(registry, command=command, exists=lambda path: False)
        self.assertFalse(stale.enabled())
        self.assertTrue(stale.status().target_missing)

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
        # The factory reached the isolated adapter; whether the logon would really start it
        # depends on the executable and on Windows itself, which the status tests cover.
        self.assertEqual(registry.autostart["KeySwitch"], "KeySwitch.exe --hidden")
        self.assertEqual(catalog.installed(), ())

    def test_open_directory_starts_explorer_and_rejects_a_missing_folder(self) -> None:
        started: list[list[str]] = []
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            open_directory(directory, spawn=started.append)
            self.assertEqual(started, [["explorer", str(directory)]])
            missing = directory / "gone"
            with self.assertRaisesRegex(WindowsSystemError, "Каталог не найден"):
                open_directory(missing, spawn=started.append)
        self.assertEqual(len(started), 1)

    def test_open_directory_spawns_a_detached_process_by_default(self) -> None:
        with (
            tempfile.TemporaryDirectory() as temporary,
            patch("keyswitch.windows_system.subprocess.Popen") as popen,
        ):
            open_directory(Path(temporary))
        popen.assert_called_once_with(["explorer", temporary], close_fds=True)


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
        self.assertEqual(
            menu_activation_message(
                ABSTRACT_PRIMARY_CLICK_MESSAGE, ABSTRACT_PRIMARY_CLICK_MESSAGE, ABSTRACT_MENU_CLICK_MESSAGE
            ),
            ABSTRACT_MENU_CLICK_MESSAGE,
        )
        self.assertEqual(
            menu_activation_message(
                ABSTRACT_OTHER_MESSAGE, ABSTRACT_PRIMARY_CLICK_MESSAGE, ABSTRACT_MENU_CLICK_MESSAGE
            ),
            ABSTRACT_OTHER_MESSAGE,
        )

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

    def test_model_settings_distinguish_prefix_support_without_changing_defaults(self) -> None:
        specs = {spec.path: spec for spec in ALL_SETTING_SPECS}
        self.assertEqual(specs["detection.context_aware"].title, "Учитывать контекст")
        self.assertIn("префиксов", specs["detection.context_policy"].description)
        self.assertIn("без контекста", specs["detection.early_switch"].description)
        self.assertIn("KSLM", specs["detection.intent_model_enabled"].title)
        minimum = specs["detection.early_switch_min_length"]
        self.assertEqual(minimum.title, "Символов до ранней смены")
        self.assertEqual(
            (minimum.minimum, minimum.maximum),
            (EXPECTED_EARLY_SWITCH_MIN_LENGTH_SETTING_MIN, EXPECTED_EARLY_SWITCH_MIN_LENGTH_SETTING_MAX),
        )
        self.assertIn(f"значение {EARLY_SWITCH_MIN_LENGTH_SETTING_MIN} ", minimum.description)
        self.assertIn(f"{PREFIX_MIN_CHARACTERS}–{PREFIX_MAX_CHARACTERS}", minimum.description)
        # Each window text names the value the code uses.
        self.assertIn(f"Доступно {quantity(UNDO_AVAILABLE_WINDOW_SECONDS, SECONDS)} ", specs["hotkeys.undo"].description)
        self.assertIn(f"раз в {quantity(UPDATE_CHECK_INTERVAL_SECONDS / SECONDS_PER_HOUR, HOURS)}.",
                      specs["updates.check_automatically"].description)
        detection = DEFAULT_SETTINGS["detection"]
        assert isinstance(detection, dict)
        # Off by default since 0.23.0: it changed correctly typed words before they ended.
        self.assertFalse(detection["early_switch"])
        self.assertTrue(detection["context_aware"])
        self.assertEqual(detection["context_policy"], "assist")
        self.assertEqual(
            detection["early_switch_min_length"], DEFAULT_EARLY_SWITCH_MIN_LENGTH
        )

    def test_catalogue_is_unique_complete_and_uses_valid_control_metadata(self) -> None:
        paths = [spec.path for spec in ALL_SETTING_SPECS]
        self.assertEqual(len(paths), len(set(paths)))
        for spec in ALL_SETTING_SPECS:
            value: object = DEFAULT_SETTINGS
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

    def test_autostart_status_reports_the_state_or_the_reason_it_cannot(self) -> None:
        """Both branches run on either platform: the registry is reachable, or it is not."""
        status = AutostartStatus('"C:\\Programs\\KeySwitch.exe" --hidden', False, False)
        with patch("keyswitch.windows_app.WindowsAutostartManager") as manager:
            manager.return_value.status.return_value = status
            self.assertEqual(windows_app_module.autostart_status(), status.as_dict())
        for error in (WindowsSystemError("Реестр Windows доступен только в Windows"), OSError("access denied")):
            with self.subTest(error=type(error).__name__):
                with patch("keyswitch.windows_app.WindowsAutostartManager", side_effect=error):
                    reported = windows_app_module.autostart_status()
                self.assertEqual(set(reported), {"error"})
                self.assertIn(type(error).__name__, str(reported["error"]))

    def test_logging_parser_diagnostics_and_ui_dispatch(self) -> None:
        # Logging is configured by the shared keyswitch.logsetup module, which
        # tests/test_logsetup.py covers including the rotation budgets.
        self.assertIs(
            windows_app_module.configure_logging, logsetup.configure_logging
        )

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
        failed_api.layout_values = (ENGLISH_HKL,)
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
            patch(
                "keyswitch.windows_app.diagnose",
                return_value=FAKE_DIAGNOSE_EXIT_CODE,
            ) as diagnose,
        ):
            self.assertEqual(
                windows_app_module.main(["--diagnose"]), FAKE_DIAGNOSE_EXIT_CODE
            )
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
            return FAKE_WINDOWS_UI_EXIT_CODE

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
                FAKE_WINDOWS_UI_EXIT_CODE,
            )
            self.assertEqual(
                windows_app_module.main(["--smoke-ui"]), FAKE_WINDOWS_UI_EXIT_CODE
            )
        self.assertEqual(
            calls,
            [
                (True, True, None),
                (False, True, SMOKE_UI_QUIT_AFTER_MS),
            ],
        )
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
                # logsetup imported the handler by name, so the patch has to
                # name it there or the run opens the real log file.
                patch(
                    "keyswitch.logsetup.RotatingFileHandler",
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
