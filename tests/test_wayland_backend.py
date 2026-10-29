"""Platform-independent unit and engine-integration tests for WaylandBackend."""

from __future__ import annotations

from collections.abc import Callable
from pathlib import Path
import sys
import tempfile
import threading
import time
from types import ModuleType
import unittest
from unittest.mock import patch

from engine_models import share_engine_models
from fixture_values.clock import SHORT_LISTENER_START_TIMEOUT_SECONDS
from fixture_values.wayland import (
    WL_FIXTURE_ANCHOR_X_PIXELS,
    WL_FIXTURE_ANCHOR_Y_PIXELS,
    WL_FIXTURE_EXPECTED_FOUR_BACKSPACES,
    WL_FIXTURE_EXPECTED_LATENCY_MS,
    WL_FIXTURE_EXPECTED_THREE_BATCHES,
    WL_FIXTURE_EXPECTED_TWO_EVENTS,
    WL_FIXTURE_INVALID_GROUP,
    WL_FIXTURE_INVALID_SOURCE_GROUP,
    WL_FIXTURE_KEY_B,
    WL_FIXTURE_KEY_D,
    WL_FIXTURE_KEY_F,
    WL_FIXTURE_KEY_G,
    WL_FIXTURE_KEY_H,
    WL_FIXTURE_KEY_J,
    WL_FIXTURE_KEY_N,
    WL_FIXTURE_KEY_R,
    WL_FIXTURE_KEY_T,
    WL_FIXTURE_LAYOUT_DE_INDEX,
    WL_FIXTURE_LAYOUT_EN_INDEX,
    WL_FIXTURE_LAYOUT_RU_INDEX,
    WL_FIXTURE_MONOTONIC_NOW_MS,
    WL_FIXTURE_OWN_PROCESS_ID,
    WL_FIXTURE_POLL_DEADLINE_READINGS,
    WL_FIXTURE_POPUP_X_PIXELS,
    WL_FIXTURE_POPUP_Y_PIXELS,
    WL_FIXTURE_PROCESS_ID,
    WL_FIXTURE_SECOND_WINDOW_ID,
    WL_FIXTURE_SEQ_FIRST,
    WL_FIXTURE_SEQ_SECOND,
    WL_FIXTURE_SEQ_THIRD,
    WL_FIXTURE_TIMESTAMP_BASE_MS,
    WL_FIXTURE_TIMESTAMP_STEP_MS,
    WL_FIXTURE_UNNAMED_KEYCODE,
    WL_FIXTURE_WINDOW_ID,
    WL_FIXTURE_WINDOW_X_PIXELS,
    WL_FIXTURE_WINDOW_Y_PIXELS,
)
from keyswitch.backend import FocusInfo, KeyDisposition, KeyEvent, LayoutSwitchUnconfirmed, ScreenAnchor
from keyswitch.config import SettingsStore
from keyswitch.context_access import PlatformFieldReader
from keyswitch.constants.keyboard import (
    ALT_MASK,
    COMPLETED_ACTION_EVENT_COUNT,
    CONTROL_MASK,
    LAYOUT_GROUP_COUNT,
    LOCK_MASK,
    POINTER_EVENT_GROUP,
    SHIFT_MASK,
    SUPER_MASK,
)
from keyswitch.constants.timing import (
    KEYBOARD_LISTENER_START_TIMEOUT_SECONDS,
    LAYOUT_SWITCH_POLL_SECONDS,
    MODIFIER_RECHECK_GAP_MS,
    WAYLAND_TYPED_INPUT_SETTLE_SECONDS,
)
from keyswitch.constants.wayland import (
    EVDEV_KEY_A,
    EVDEV_KEY_BACKSPACE,
    EVDEV_KEY_CAPSLOCK,
    EVDEV_KEY_COMMA,
    EVDEV_KEY_ENTER,
    EVDEV_KEY_LEFTALT,
    EVDEV_KEY_LEFTCTRL,
    EVDEV_KEY_LEFTMETA,
    EVDEV_KEY_LEFTSHIFT,
    EVDEV_KEY_Q,
    EVDEV_KEY_PAUSE,
    EVDEV_KEY_SEMICOLON,
    EVDEV_KEY_SLASH,
    EVDEV_KEY_SPACE,
    EVDEV_KEY_Z,
)
from keyswitch.engine import KeySwitchEngine, _default_backend
from keyswitch.history import HistoryStore
from keyswitch.input_context import FieldContext
from keyswitch.learning import LearningStore
from keyswitch.wayland_backend import (
    INPUT_AHEAD_ERROR,
    NativeInput,
    NativeKeyEvent,
    WaylandBackend,
    WaylandBackendError,
    WaylandLayoutSwitchUnconfirmed,
    is_cyrillic,
    key_name,
    select_layout_indices,
)


def setUpModule() -> None:
    share_engine_models()


QWERTY_MAP: dict[int, tuple[tuple[str, str], tuple[str, str], str]] = {
    EVDEV_KEY_Q: (("q", "Q"), ("й", "Й"), "q"),
    EVDEV_KEY_A: (("a", "A"), ("ф", "Ф"), "a"),
    EVDEV_KEY_Z: (("z", "Z"), ("я", "Я"), "z"),
    WL_FIXTURE_KEY_G: (("g", "G"), ("п", "П"), "g"),
    WL_FIXTURE_KEY_H: (("h", "H"), ("р", "Р"), "h"),
    WL_FIXTURE_KEY_B: (("b", "B"), ("и", "И"), "b"),
    WL_FIXTURE_KEY_D: (("d", "D"), ("в", "В"), "d"),
    WL_FIXTURE_KEY_T: (("t", "T"), ("е", "Е"), "t"),
    WL_FIXTURE_KEY_N: (("n", "N"), ("т", "Т"), "n"),
    WL_FIXTURE_KEY_J: (("j", "J"), ("о", "О"), "j"),
    WL_FIXTURE_KEY_R: (("r", "R"), ("к", "К"), "r"),
    WL_FIXTURE_KEY_F: (("f", "F"), ("а", "А"), "f"),
    EVDEV_KEY_SEMICOLON: ((";", ":"), ("ж", "Ж"), "semicolon"),
    EVDEV_KEY_COMMA: ((",", "<"), ("б", "Б"), "comma"),
    # The Russian layout has `,` on Shift with the slash key: a comma after a word is typed there.
    EVDEV_KEY_SLASH: (("/", "?"), (".", ","), "slash"),
    EVDEV_KEY_SPACE: ((" ", " "), (" ", " "), "space"),
}


class FakeWaylandAPI:
    def __init__(self) -> None:
        self.desktop = "GNOME"
        self.denied = ""
        self.names: tuple[str, ...] = ("us", "ru")
        self.current_index = WL_FIXTURE_LAYOUT_EN_INDEX
        self.accept_switch = True
        self.apply_switch = True
        self.switch_requests: list[int] = []
        self.posted: list[tuple[tuple[bool, int], ...]] = []
        self.post_result: int | None = None
        self.application = "org.gnome.TextEditor"
        self.window = WL_FIXTURE_WINDOW_ID
        self.owners: dict[int, int] = {WL_FIXTURE_WINDOW_ID: WL_FIXTURE_PROCESS_ID}
        self.own_pid = WL_FIXTURE_OWN_PROCESS_ID
        self.origins: dict[int, tuple[int, int]] = {
            WL_FIXTURE_WINDOW_ID: (WL_FIXTURE_WINDOW_X_PIXELS, WL_FIXTURE_WINDOW_Y_PIXELS)
        }
        self.anchor: ScreenAnchor | None = ScreenAnchor(
            WL_FIXTURE_ANCHOR_X_PIXELS,
            WL_FIXTURE_ANCHOR_Y_PIXELS,
            WL_FIXTURE_WINDOW_ID,
        )
        self.positioned: list[tuple[int, int, int]] = []
        self.activated: list[int] = []
        self.isolated = False
        self.caps_lock = False
        self.released_keys: set[int] = set()
        self.key_down_queries: list[int] = []
        self.now_ms = WL_FIXTURE_MONOTONIC_NOW_MS
        self.listener: Callable[[NativeKeyEvent], bool] | None = None
        self.signal_ready = True
        self.listener_error: Exception | None = None
        self.stop_event = threading.Event()
        self.stop_calls = 0
        self.close_calls = 0
        self.editor_text: list[str] = []
        self.record_editor = False
        self.password_windows: set[int] = set()
        self.known_seqs: list[int | None] = []
        self.actions: list[bool] = []
        self.refuse_checked = False

    def desktop_name(self) -> str:
        return self.desktop

    def permission_error(self) -> str:
        return self.denied

    def layout_names(self) -> tuple[str, ...]:
        return self.names

    def current_layout_index(self) -> int:
        return self.current_index

    def select_layout_index(self, index: int) -> bool:
        self.switch_requests.append(index)
        if self.accept_switch and self.apply_switch:
            self.current_index = index
        return self.accept_switch

    def translate_key(self, keycode: int, state: int, layout_index: int) -> str:
        if state & (CONTROL_MASK | ALT_MASK | SUPER_MASK):
            return ""
        spec = QWERTY_MAP.get(keycode)
        if spec is None:
            return ""
        pair = spec[1] if layout_index == WL_FIXTURE_LAYOUT_RU_INDEX else spec[0]
        shifted = bool(state & SHIFT_MASK)
        if pair[0].isalpha() and bool(state & LOCK_MASK):
            shifted = not shifted
        return pair[1] if shifted else pair[0]

    def keysym_name(self, keycode: int, layout_index: int = 0) -> str:
        del layout_index
        spec = QWERTY_MAP.get(keycode)
        if spec is None:
            return ""
        _en, _ru, sym = spec
        return sym

    def post_inputs(
        self, inputs: tuple[tuple[bool, int], ...], known_seq: int | None = None, action: bool = False
    ) -> int:
        self.posted.append(inputs)
        self.known_seqs.append(known_seq)
        self.actions.append(action)
        if known_seq is not None and self.refuse_checked:
            return 0
        if self.post_result is not None:
            return self.post_result
        if self.record_editor:
            shift_held = False
            for pressed, code in inputs:
                if code == EVDEV_KEY_LEFTSHIFT:
                    shift_held = pressed
                elif pressed:
                    if code == EVDEV_KEY_BACKSPACE:
                        if self.editor_text:
                            self.editor_text.pop()
                    elif code == EVDEV_KEY_ENTER:
                        self.editor_text.append("\n")
                    else:
                        state = (SHIFT_MASK if shift_held else 0) | (LOCK_MASK if self.caps_lock else 0)
                        ch = self.translate_key(code, state, self.current_index)
                        if ch:
                            self.editor_text.append(ch)
        return len(inputs)

    def active_application(self) -> str:
        return self.application

    def focused_window(self) -> int:
        return self.window

    def window_process_id(self, window: int) -> int:
        return self.owners.get(window, 0)

    def current_process_id(self) -> int:
        return self.own_pid

    def window_origin(self, window: int) -> tuple[int, int] | None:
        return self.origins.get(window)

    def window_is_password(self, window: int) -> bool:
        return window in self.password_windows

    def input_anchor(self) -> ScreenAnchor | None:
        return self.anchor

    def position_window(self, window: int, x: int, y: int) -> bool:
        self.positioned.append((window, x, y))
        return True

    def activate_window(self, window: int) -> bool:
        self.activated.append(window)
        return True

    def isolated_layout(self) -> bool:
        return self.isolated

    def caps_lock_enabled(self) -> bool:
        return self.caps_lock

    def key_down(self, keycode: int) -> bool:
        self.key_down_queries.append(keycode)
        return keycode not in self.released_keys

    def monotonic_ms(self) -> int:
        return self.now_ms

    def run_input_listener(
        self,
        listener: Callable[[NativeKeyEvent], bool],
        ready: Callable[[], None],
    ) -> None:
        self.listener = listener
        if self.listener_error is not None:
            raise self.listener_error
        if self.signal_ready:
            ready()
        self.stop_event.wait(KEYBOARD_LISTENER_START_TIMEOUT_SECONDS)

    def stop_input_listener(self) -> None:
        self.stop_calls += 1
        self.stop_event.set()

    def close(self) -> None:
        self.close_calls += 1


class _NullFieldReader:
    def read(self, application: str, window: int) -> FieldContext | None:
        _ = (application, window)
        return None


def stroke_event(
    keycode: int = EVDEV_KEY_A,
    *,
    character: str = "a",
    characters: tuple[str, ...] = ("a", "ф"),
    group: int = 0,
    state: int = 0,
) -> KeyEvent:
    return KeyEvent(
        True,
        keycode,
        key_name(keycode),
        character,
        characters,
        group,
        state,
        WL_FIXTURE_TIMESTAMP_BASE_MS,
    )


class WaylandBackendUnitTests(unittest.TestCase):
    def test_helpers_default_api_and_engine_default_backend(self) -> None:
        self.assertFalse(is_cyrillic(""))
        self.assertFalse(is_cyrillic("q"))
        self.assertTrue(is_cyrillic("й"))

        table = {0: "", 1: "й", WL_FIXTURE_LAYOUT_DE_INDEX: "q", WL_FIXTURE_INVALID_SOURCE_GROUP: "ф"}
        self.assertEqual(
            select_layout_indices(
                WL_FIXTURE_INVALID_SOURCE_GROUP + 1,
                lambda _k, _s, idx: table.get(idx, "q"),
            ),
            (WL_FIXTURE_LAYOUT_DE_INDEX, 1),
        )
        with self.assertRaisesRegex(WaylandBackendError, "английская и русская"):
            select_layout_indices(1, lambda _k, _s, _i: "q")
        with self.assertRaisesRegex(WaylandBackendError, "английская и русская"):
            select_layout_indices(1, lambda _k, _s, _i: "й")

        self.assertEqual(key_name(EVDEV_KEY_BACKSPACE), "BackSpace")
        self.assertEqual(key_name(EVDEV_KEY_COMMA), "comma")
        self.assertEqual(key_name(WL_FIXTURE_UNNAMED_KEYCODE, "A"), "a")
        self.assertEqual(key_name(WL_FIXTURE_UNNAMED_KEYCODE, "Cyrillic_shorti"), "Cyrillic_shorti")
        self.assertEqual(key_name(WL_FIXTURE_UNNAMED_KEYCODE, "", "Q"), "q")
        self.assertEqual(key_name(WL_FIXTURE_UNNAMED_KEYCODE, "", ""), f"KEY_{WL_FIXTURE_UNNAMED_KEYCODE}")

        fake_api = FakeWaylandAPI()
        native_mod = ModuleType("keyswitch.wayland_native")
        setattr(native_mod, "NativeWaylandAPI", lambda: fake_api)
        with patch.dict(sys.modules, {"keyswitch.wayland_native": native_mod}):
            with WaylandBackend() as backend:
                self.assertEqual(backend.layouts, (WL_FIXTURE_LAYOUT_EN_INDEX, WL_FIXTURE_LAYOUT_RU_INDEX))
        self.assertEqual(fake_api.close_calls, 1)

        with (
            patch("keyswitch.wayland_desktop.wayland_session_active", return_value=True),
            patch("keyswitch.wayland_backend.WaylandBackend", return_value=backend) as factory,
        ):
            self.assertIs(_default_backend(LAYOUT_GROUP_COUNT), backend)
        factory.assert_called_once_with(group_count=LAYOUT_GROUP_COUNT)

    def test_probe_lifecycle_focus_window_helpers_and_switch_group(self) -> None:
        api = FakeWaylandAPI()
        backend = WaylandBackend(api)

        api.denied = "нет доступа к /dev/uinput"
        denied_probe = backend.probe()
        self.assertFalse(denied_probe.available)
        self.assertEqual(denied_probe.error, "нет доступа к /dev/uinput")
        with self.assertRaisesRegex(WaylandBackendError, "uinput"):
            backend.start(lambda _e: None)

        api.denied = ""
        ok_probe = backend.probe()
        self.assertTrue(ok_probe.available)
        self.assertEqual(ok_probe.xkb_version, "us,ru")
        api.names = ("us",)
        shrunk_probe = backend.probe()
        self.assertEqual(shrunk_probe.xkb_version, "us,1")
        api.names = ()
        backend._layouts = None
        err_probe = backend.probe()
        self.assertFalse(err_probe.available)
        api.names = ("us", "ru")

        self.assertEqual(backend.active_application(), "org.gnome.TextEditor")
        self.assertEqual(backend.window_process_id(WL_FIXTURE_WINDOW_ID), WL_FIXTURE_PROCESS_ID)
        self.assertEqual(
            backend.window_origin(WL_FIXTURE_WINDOW_ID),
            (WL_FIXTURE_WINDOW_X_PIXELS, WL_FIXTURE_WINDOW_Y_PIXELS),
        )
        self.assertEqual(backend.input_anchor(), api.anchor)
        self.assertTrue(
            backend.position_window(
                WL_FIXTURE_WINDOW_ID, WL_FIXTURE_POPUP_X_PIXELS, WL_FIXTURE_POPUP_Y_PIXELS
            )
        )
        self.assertFalse(backend.restore_window(None))
        self.assertFalse(backend.restore_window(0))
        self.assertTrue(backend.restore_window(WL_FIXTURE_WINDOW_ID))

        self.assertEqual(backend.focused_window(), FocusInfo(WL_FIXTURE_WINDOW_ID, False, False))
        api.owners[WL_FIXTURE_WINDOW_ID] = WL_FIXTURE_OWN_PROCESS_ID
        api.isolated = True
        self.assertEqual(backend.focused_window(), FocusInfo(WL_FIXTURE_WINDOW_ID, True, True))
        api.window = 0
        self.assertIsNone(backend.focused_window())
        api.window = WL_FIXTURE_WINDOW_ID

        # A layout that is neither of the pair (or one still being applied) is no group at all.
        api.current_index = WL_FIXTURE_LAYOUT_DE_INDEX
        self.assertEqual(backend.current_group(), -1)
        api.current_index = WL_FIXTURE_LAYOUT_EN_INDEX
        with self.assertRaisesRegex(WaylandBackendError, "Неизвестная группа"):
            backend.switch_group(WL_FIXTURE_INVALID_GROUP)
        backend.switch_group(0)
        self.assertEqual(api.switch_requests, [])

        # Switch confirmed after one poll iteration.
        api.apply_switch = False

        def confirm_on_sleep(_sec: float) -> None:
            api.current_index = WL_FIXTURE_LAYOUT_RU_INDEX

        with patch("keyswitch.wayland_backend.time.sleep", side_effect=confirm_on_sleep):
            backend.switch_group(1)
        self.assertEqual(backend.current_group(), 1)

        # Switch rejected and switch unconfirmed timeout.
        api.accept_switch = False
        with self.assertRaisesRegex(WaylandBackendError, "отклонила"):
            backend.switch_group(0)
        api.accept_switch = True
        with (
            patch(
                "keyswitch.wayland_backend.time.monotonic",
                side_effect=WL_FIXTURE_POLL_DEADLINE_READINGS,
            ),
            patch("keyswitch.wayland_backend.time.sleep") as sleep_mock,
        ):
            with self.assertRaises(WaylandLayoutSwitchUnconfirmed) as ctx:
                backend.switch_group(0)
        self.assertIsInstance(ctx.exception, LayoutSwitchUnconfirmed)
        sleep_mock.assert_called_once_with(LAYOUT_SWITCH_POLL_SECONDS)

    def test_start_errors_stop_threads_and_event_filtering(self) -> None:
        api_err = FakeWaylandAPI()
        api_err.listener_error = RuntimeError("helper crashed")
        backend_err = WaylandBackend(api_err)
        with self.assertRaisesRegex(WaylandBackendError, "helper crashed"):
            backend_err.start(lambda _e: None)

        api_timeout = FakeWaylandAPI()
        api_timeout.signal_ready = False
        backend_timeout = WaylandBackend(api_timeout)
        with patch(
            "keyswitch.wayland_backend.KEYBOARD_LISTENER_START_TIMEOUT_SECONDS",
            SHORT_LISTENER_START_TIMEOUT_SECONDS,
        ):
            with self.assertRaisesRegex(WaylandBackendError, "не подтвердил запуск"):
                backend_timeout.start(lambda _e: None)

        api = FakeWaylandAPI()
        backend = WaylandBackend(api)
        events: list[KeyEvent] = []
        backend.start(events.append)
        backend.start(events.append)
        self.assertTrue(backend.running)
        listener = api.listener
        assert listener is not None

        # Pointer & repeat_reset with and without listener.
        self.assertFalse(listener(NativeKeyEvent(True, 0, WL_FIXTURE_TIMESTAMP_BASE_MS, pointer=True)))
        self.assertEqual(events[-1].group, POINTER_EVENT_GROUP)
        self.assertFalse(
            listener(NativeKeyEvent(True, 0, WL_FIXTURE_TIMESTAMP_BASE_MS, repeat_reset=True))
        )
        backend._listener = None
        self.assertFalse(listener(NativeKeyEvent(True, 0, WL_FIXTURE_TIMESTAMP_BASE_MS, pointer=True)))
        backend._listener = events.append

        # Latency recording: valid latency vs wrapped future timestamp.
        self.assertFalse(listener(NativeKeyEvent(True, EVDEV_KEY_A, WL_FIXTURE_TIMESTAMP_BASE_MS)))
        self.assertFalse(listener(NativeKeyEvent(False, EVDEV_KEY_A, WL_FIXTURE_MONOTONIC_NOW_MS + 1)))
        report = backend.take_input_delay()
        self.assertEqual(report.keys, 1)
        self.assertEqual(report.worst_late_ms, WL_FIXTURE_EXPECTED_LATENCY_MS)

        # Synthetic, unmatched release, swallowed press + repeated press + matched release.
        self.assertFalse(
            listener(NativeKeyEvent(True, EVDEV_KEY_ENTER, WL_FIXTURE_TIMESTAMP_BASE_MS, injected=True))
        )
        backend.set_key_filter(lambda ev: ev.key_name == "Return")
        self.assertFalse(listener(NativeKeyEvent(False, EVDEV_KEY_ENTER, WL_FIXTURE_TIMESTAMP_BASE_MS)))
        before = len(events)
        self.assertTrue(listener(NativeKeyEvent(True, EVDEV_KEY_ENTER, WL_FIXTURE_TIMESTAMP_BASE_MS)))
        self.assertTrue(listener(NativeKeyEvent(True, EVDEV_KEY_ENTER, WL_FIXTURE_TIMESTAMP_BASE_MS)))
        self.assertEqual(len(events) - before, 1)
        self.assertTrue(listener(NativeKeyEvent(False, EVDEV_KEY_ENTER, WL_FIXTURE_TIMESTAMP_BASE_MS)))
        self.assertFalse(listener(NativeKeyEvent(True, EVDEV_KEY_A, WL_FIXTURE_TIMESTAMP_BASE_MS)))

        # Stop from current thread branch, then real stop.
        real_thread = backend._thread
        backend._thread = threading.current_thread()
        backend.stop()
        backend._thread = real_thread
        backend.stop()
        backend.stop()

    def test_modifiers_recheck_deferral_and_prior_release_tracking(self) -> None:
        api = FakeWaylandAPI()
        backend = WaylandBackend(api)
        events: list[KeyEvent] = []
        backend._listener = events.append

        # Modifiers + CapsLock state.
        api.caps_lock = True
        for code in (
            EVDEV_KEY_LEFTSHIFT,
            EVDEV_KEY_LEFTCTRL,
            EVDEV_KEY_LEFTALT,
            EVDEV_KEY_LEFTMETA,
            EVDEV_KEY_CAPSLOCK,
        ):
            backend._handle_native(NativeKeyEvent(True, code, WL_FIXTURE_TIMESTAMP_BASE_MS))
        self.assertEqual(
            events[-1].state,
            SHIFT_MASK | CONTROL_MASK | ALT_MASK | SUPER_MASK | LOCK_MASK,
        )
        for code in (
            EVDEV_KEY_LEFTALT,
            EVDEV_KEY_LEFTMETA,
            EVDEV_KEY_CAPSLOCK,
            EVDEV_KEY_LEFTSHIFT,
        ):
            backend._handle_native(NativeKeyEvent(False, code, WL_FIXTURE_TIMESTAMP_BASE_MS))
        api.caps_lock = False

        # Chord within gap: no key_down query. After gap: still down -> kept; released -> forgotten.
        backend._handle_native(
            NativeKeyEvent(True, EVDEV_KEY_A, WL_FIXTURE_TIMESTAMP_BASE_MS + 1)
        )
        self.assertEqual(api.key_down_queries, [])
        t_after = WL_FIXTURE_TIMESTAMP_BASE_MS + 1 + MODIFIER_RECHECK_GAP_MS
        backend._handle_native(NativeKeyEvent(True, EVDEV_KEY_A, t_after))
        self.assertEqual(api.key_down_queries, [EVDEV_KEY_LEFTCTRL])
        self.assertEqual(events[-1].state, CONTROL_MASK)

        api.released_keys.add(EVDEV_KEY_LEFTCTRL)
        api.window = WL_FIXTURE_SECOND_WINDOW_ID
        with self.assertLogs("keyswitch.wayland_backend", level="INFO") as logs:
            backend._handle_native(NativeKeyEvent(True, EVDEV_KEY_A, t_after + 1))
        self.assertEqual(events[-1].state, 0)
        self.assertIn("Control_L", logs.output[0])
        backend._handle_native(NativeKeyEvent(False, EVDEV_KEY_A, t_after + 1))

        # Deferred Enter + held second Enter + prior-key release tracking.
        def defer_enter(ev: KeyEvent) -> KeyDisposition:
            return "defer" if ev.key_name == "Return" else False

        backend.set_key_filter(defer_enter)
        self.assertEqual(backend.complete_action(True), 0)
        backend._handle_native(NativeKeyEvent(True, EVDEV_KEY_A, t_after))
        self.assertTrue(backend._handle_native(NativeKeyEvent(True, EVDEV_KEY_ENTER, t_after)))
        self.assertTrue(events[-1].deferred)
        self.assertTrue(backend._handle_native(NativeKeyEvent(False, EVDEV_KEY_ENTER, t_after)))
        self.assertEqual(backend.release_input(), 0)

        # While Enter is deferred: press A again (held), then release A (prior_release -> appended to _held),
        # then press Q, Enter, release Q, press Z, release second Enter (all held).
        backend.hold_input()
        self.assertTrue(backend._handle_native(NativeKeyEvent(True, EVDEV_KEY_A, t_after)))
        self.assertFalse(backend._handle_native(NativeKeyEvent(False, EVDEV_KEY_A, t_after)))
        self.assertTrue(backend._handle_native(NativeKeyEvent(True, EVDEV_KEY_Q, t_after)))
        self.assertTrue(backend._handle_native(NativeKeyEvent(True, EVDEV_KEY_ENTER, t_after)))
        self.assertTrue(backend._handle_native(NativeKeyEvent(False, EVDEV_KEY_Q, t_after)))
        self.assertTrue(backend._handle_native(NativeKeyEvent(True, EVDEV_KEY_Z, t_after)))
        self.assertTrue(backend._handle_native(NativeKeyEvent(False, EVDEV_KEY_ENTER, t_after)))

        # Complete first Enter: delivers Enter, replays A, Q, second Enter (defers again!),
        # flushes Q's release, and keeps Z held!
        self.assertEqual(backend.complete_action(True), COMPLETED_ACTION_EVENT_COUNT)
        self.assertIsNotNone(backend._deferred_action)
        self.assertEqual([item.keycode for item in backend._held], [EVDEV_KEY_Z])

        # Pointer move invalidates delivery of second Enter, while still releasing Z.
        backend._handle_native(NativeKeyEvent(True, 0, t_after, pointer=True))
        with self.assertRaisesRegex(WaylandBackendError, "место ввода изменилось"):
            backend.complete_action(True)
        self.assertFalse(backend._holding)

        # Cancel deferred action cleanly returns 0.
        self.assertTrue(backend._handle_native(NativeKeyEvent(True, EVDEV_KEY_ENTER, t_after)))
        self.assertEqual(backend.complete_action(False), 0)

    def test_inject_correction_shift_capslock_kept_tail_and_error_recovery(self) -> None:
        api = FakeWaylandAPI()
        backend = WaylandBackend(api)

        # Physically held Shift + CapsLock XOR + boundary preserving layout.
        backend._pressed.add(EVDEV_KEY_LEFTSHIFT)
        api.caps_lock = True
        stroke = stroke_event(EVDEV_KEY_A, state=SHIFT_MASK)
        plain_stroke = stroke_event(EVDEV_KEY_Q)
        semi = stroke_event(EVDEV_KEY_SEMICOLON, character=";", characters=(";", "ж"))
        self.assertEqual(
            backend.inject_correction((stroke, plain_stroke), 1, semi, source_group=0), 0
        )
        self.assertEqual(len(api.posted), WL_FIXTURE_EXPECTED_THREE_BATCHES)
        # First batch starts by releasing held LeftShift, and third batch restores LeftShift.
        self.assertEqual(api.posted[0][0], (False, EVDEV_KEY_LEFTSHIFT))
        self.assertEqual(api.posted[-1][-1], (True, EVDEV_KEY_LEFTSHIFT))
        backend._pressed.clear()
        api.caps_lock = False

        # Empty injection & space boundary staying in target layout.
        api.posted.clear()
        backend.inject_correction((), 1, None)
        self.assertEqual(api.posted, [])
        space = stroke_event(EVDEV_KEY_SPACE, character=" ", characters=(" ", " "), group=0)
        backend.inject_correction((stroke_event(EVDEV_KEY_A),), 0, space)

        # kept_tail valid and invalid.
        api.posted.clear()
        api.current_index = WL_FIXTURE_LAYOUT_RU_INDEX
        w1 = stroke_event(EVDEV_KEY_A, group=1)
        sp = stroke_event(EVDEV_KEY_SPACE, character=" ", characters=(" ", " "), group=1)
        w2 = stroke_event(EVDEV_KEY_Q, group=1)
        backend.inject_correction((w1, sp, w2), 0, sp, source_group=1, kept_tail=1)
        self.assertEqual(api.current_index, WL_FIXTURE_LAYOUT_RU_INDEX)
        for bad_source, bad_kept in (
            (1, -1),
            (1, WL_FIXTURE_EXPECTED_THREE_BATCHES),
            (None, 1),
        ):
            with self.assertRaisesRegex(WaylandBackendError, "сохраняемого слова"):
                backend.inject_correction((w1, sp, w2), 0, sp, source_group=bad_source, kept_tail=bad_kept)

        # Invalid target, source, and punctuation groups.
        with self.assertRaisesRegex(WaylandBackendError, "Неизвестная группа"):
            backend.inject_correction((), WL_FIXTURE_INVALID_GROUP, None)
        with self.assertRaisesRegex(WaylandBackendError, "исходная группа"):
            backend.inject_correction(
                (stroke_event(group=WL_FIXTURE_INVALID_SOURCE_GROUP),), 0, None
            )
        bad_semi = stroke_event(
            EVDEV_KEY_SEMICOLON, character=";", characters=(";", "ж"), group=WL_FIXTURE_INVALID_SOURCE_GROUP
        )
        with self.assertRaisesRegex(WaylandBackendError, "раскладка пунктуации"):
            backend.inject_correction((stroke_event(),), 0, bad_semi, source_group=0)

        # Focus changed during hold aborts before deleting anything.
        backend.hold_input()
        api.window = WL_FIXTURE_SECOND_WINDOW_ID
        with self.assertRaisesRegex(WaylandBackendError, "Место ввода изменилось до замены"):
            backend.inject_correction((stroke_event(),), 0, None)
        api.window = WL_FIXTURE_WINDOW_ID

        # Partial post_inputs failure on synthetic batch and on replayed held key.
        api.post_result = 1
        with self.assertRaisesRegex(WaylandBackendError, "uinput принял 1 из"):
            backend.inject_correction((stroke_event(),), 0, None)

        api.post_result = None
        backend.hold_input()
        backend.set_key_filter(lambda ev: ev.key_name == "q")
        backend._held.append(NativeKeyEvent(True, EVDEV_KEY_Q, WL_FIXTURE_TIMESTAMP_BASE_MS))
        backend._held.append(NativeKeyEvent(True, EVDEV_KEY_A, WL_FIXTURE_TIMESTAMP_BASE_MS))
        api.post_result = 0
        with self.assertRaisesRegex(WaylandBackendError, "uinput принял 0 из 1"):
            backend.release_input()
        backend.set_key_filter(None)

        # Mid-injection failure after deletion restores `late` keys, and secondary restore error keeps primary.
        api.post_result = None
        api.posted.clear()
        api.current_index = WL_FIXTURE_LAYOUT_EN_INDEX
        orig_select = api.select_layout_index
        calls = 0

        def fail_second_switch(index: int) -> bool:
            nonlocal calls
            calls += 1
            if calls >= WL_FIXTURE_EXPECTED_TWO_EVENTS:
                return False
            return orig_select(index)

        api.select_layout_index = fail_second_switch  # type: ignore[method-assign]
        late_stroke = stroke_event(EVDEV_KEY_Z)
        with self.assertRaisesRegex(WaylandBackendError, "отклонила"):
            backend.inject_correction(
                (stroke_event(EVDEV_KEY_A),),
                1,
                semi,
                source_group=0,
                late=(late_stroke,),
            )
        self.assertEqual(
            sum(1 for pressed, code in api.posted[0] if code == EVDEV_KEY_BACKSPACE),
            WL_FIXTURE_EXPECTED_FOUR_BACKSPACES + WL_FIXTURE_EXPECTED_TWO_EVENTS,
        )
        self.assertEqual(
            api.posted[-WL_FIXTURE_EXPECTED_TWO_EVENTS:],
            [((True, EVDEV_KEY_Z),), ((False, EVDEV_KEY_Z),)],
        )

        # Secondary failure inside restore preserves original failure.
        calls = 0
        api.current_index = WL_FIXTURE_LAYOUT_EN_INDEX

        def post_fail_on_restore(
            inputs: tuple[tuple[bool, int], ...], _known_seq: int | None = None, _action: bool = False
        ) -> int:
            if inputs == ((True, EVDEV_KEY_Z),):
                return 0
            return len(inputs)

        api.post_inputs = post_fail_on_restore  # type: ignore[method-assign, assignment]
        with self.assertRaisesRegex(WaylandBackendError, "отклонила"):
            backend.inject_correction(
                (stroke_event(EVDEV_KEY_A),),
                1,
                semi,
                source_group=0,
                late=(late_stroke,),
            )


class WaylandTypedTextAndLateVerdictTests(unittest.TestCase):
    """What the backend does with keys the helper typed on its own, and how it types around layouts."""

    def make(self) -> tuple[FakeWaylandAPI, WaylandBackend, list[KeyEvent]]:
        api = FakeWaylandAPI()
        backend = WaylandBackend(api)
        events: list[KeyEvent] = []
        backend._listener = events.append
        return api, backend, events

    @staticmethod
    def key(code: int, seq: int, *, pressed: bool = True, **flags: bool) -> NativeKeyEvent:
        return NativeKeyEvent(pressed, code, WL_FIXTURE_TIMESTAMP_BASE_MS, seq=seq, **flags)

    def test_a_sign_the_target_layout_has_is_typed_there_without_switching_back(self) -> None:
        api, backend, _events = self.make()
        comma = stroke_event(EVDEV_KEY_COMMA, character=",", characters=(",", "б"))
        with patch("keyswitch.wayland_backend.time.sleep") as sleep:
            backend.inject_correction((stroke_event(EVDEV_KEY_A),), 1, comma, source_group=0)
        self.assertEqual(api.switch_requests, [WL_FIXTURE_LAYOUT_RU_INDEX])
        sleep.assert_not_called()
        self.assertEqual(len(api.posted), 1)
        shifted_comma = len(((True, EVDEV_KEY_LEFTSHIFT), (True, EVDEV_KEY_SLASH), (False, EVDEV_KEY_SLASH), (False, EVDEV_KEY_LEFTSHIFT)))
        self.assertEqual(api.posted[0][-shifted_comma:], (
            (True, EVDEV_KEY_LEFTSHIFT), (True, EVDEV_KEY_SLASH), (False, EVDEV_KEY_SLASH), (False, EVDEV_KEY_LEFTSHIFT),
        ))

    def test_signs_after_a_word_are_typed_with_the_keys_the_target_layout_has_them_on(self) -> None:
        api, backend, _events = self.make()
        comma = stroke_event(EVDEV_KEY_COMMA, character=",", characters=(",", "б"))
        period = stroke_event(EVDEV_KEY_SLASH, character=".", characters=("/", "."), group=1)
        literal = backend._typed_in_group((comma,), 1)
        self.assertIsNotNone(literal)
        self.assertEqual(backend._typed_in_group((comma, comma), 1), (*(literal or ()), *(literal or ())))
        # Found on its own key, unshifted.
        self.assertEqual(backend._typed_in_group((period,), 1), (
            NativeInput(True, EVDEV_KEY_SLASH), NativeInput(False, EVDEV_KEY_SLASH),
        ))
        del api

    def test_a_sign_missing_from_the_target_layout_waits_before_each_switch(self) -> None:
        api, backend, _events = self.make()
        semicolon = stroke_event(EVDEV_KEY_SEMICOLON, character=";", characters=(";", "ж"))
        with patch("keyswitch.wayland_backend.time.sleep") as sleep:
            backend.inject_correction((stroke_event(EVDEV_KEY_A),), 1, semicolon, source_group=0)
        self.assertEqual(api.switch_requests, [
            WL_FIXTURE_LAYOUT_RU_INDEX, WL_FIXTURE_LAYOUT_EN_INDEX, WL_FIXTURE_LAYOUT_RU_INDEX,
        ])
        self.assertEqual(
            [call.args[0] for call in sleep.call_args_list],
            [WAYLAND_TYPED_INPUT_SETTLE_SECONDS, WAYLAND_TYPED_INPUT_SETTLE_SECONDS],
        )
        # A character no key gives is no reason to look further.
        self.assertIsNone(backend._typed_in_group((stroke_event(EVDEV_KEY_A, character=""),), 1))
        # Already there: no wait and no switch.
        with patch("keyswitch.wayland_backend.time.sleep") as sleep:
            backend._switch_group_after_typing(backend.current_group())
        sleep.assert_not_called()

    def test_a_held_key_typed_by_the_helper_leaves_the_hold_and_the_correction_is_refused(self) -> None:
        api, backend, events = self.make()
        backend._handle_native(self.key(EVDEV_KEY_A, WL_FIXTURE_SEQ_FIRST))
        backend.hold_input()
        self.assertEqual(backend._hold_known_seq, WL_FIXTURE_SEQ_FIRST)
        self.assertTrue(backend._handle_native(self.key(EVDEV_KEY_Q, WL_FIXTURE_SEQ_SECOND)))
        seen = len(events)
        self.assertFalse(backend._handle_native(self.key(EVDEV_KEY_Q, WL_FIXTURE_SEQ_SECOND, forced=True)))
        self.assertEqual(backend._held, [])
        self.assertEqual([event.keycode for event in events[seen:]], [EVDEV_KEY_Q])
        self.assertEqual(backend._known_seq, WL_FIXTURE_SEQ_SECOND)
        with self.assertRaisesRegex(WaylandBackendError, INPUT_AHEAD_ERROR):
            backend.inject_correction((stroke_event(),), 1, None)
        self.assertEqual(api.switch_requests, [])
        self.assertFalse(backend._holding)

        # A forced key whose answer is gone, or was a pass, changes nothing.
        backend._handle_native(self.key(EVDEV_KEY_A, WL_FIXTURE_SEQ_THIRD, forced=True))
        backend._handle_native(self.key(EVDEV_KEY_A, WL_FIXTURE_SEQ_THIRD))
        backend._handle_native(self.key(EVDEV_KEY_A, WL_FIXTURE_SEQ_THIRD, forced=True))
        self.assertFalse(backend._hold_broken)
        # A held key already typed again before the helper's word came.
        backend.hold_input()
        held = self.key(EVDEV_KEY_Z, WL_FIXTURE_SEQ_FIRST)
        self.assertTrue(backend._handle_native(held))
        backend._held.clear()
        backend._handle_native(self.key(EVDEV_KEY_Z, WL_FIXTURE_SEQ_FIRST, forced=True))
        self.assertFalse(backend._hold_broken)

    def test_a_consumed_hotkey_typed_by_the_helper_lets_its_release_through(self) -> None:
        _api, backend, _events = self.make()
        backend.set_key_filter(lambda event: event.key_name == "Pause")
        self.assertTrue(backend._handle_native(self.key(EVDEV_KEY_PAUSE, WL_FIXTURE_SEQ_FIRST)))
        with self.assertLogs("keyswitch.wayland_backend", level="INFO"):
            backend._handle_native(self.key(EVDEV_KEY_PAUSE, WL_FIXTURE_SEQ_FIRST, forced=True))
        self.assertFalse(backend._handle_native(self.key(EVDEV_KEY_PAUSE, WL_FIXTURE_SEQ_SECOND, pressed=False)))

    def test_a_withheld_enter_typed_by_the_helper_is_not_typed_again(self) -> None:
        api, backend, _events = self.make()
        backend.set_key_filter(lambda event: "defer" if event.key_name == "Return" else False)
        self.assertTrue(backend._handle_native(self.key(EVDEV_KEY_ENTER, WL_FIXTURE_SEQ_FIRST)))
        # The helper reports the forced key before any later one (its queue is typed after it).
        with self.assertLogs("keyswitch.wayland_backend", level="INFO"):
            backend._handle_native(self.key(EVDEV_KEY_ENTER, WL_FIXTURE_SEQ_FIRST, forced=True))
        self.assertIsNone(backend._deferred_action)
        self.assertFalse(backend._handle_native(self.key(EVDEV_KEY_ENTER, WL_FIXTURE_SEQ_SECOND, pressed=False)))
        with self.assertRaisesRegex(WaylandBackendError, INPUT_AHEAD_ERROR):
            backend.inject_correction((stroke_event(),), 1, None)
        # The failed correction ended the hold; the action handed back types nothing.
        self.assertTrue(backend._action_forced)
        self.assertEqual(backend.complete_action(True), 0)
        self.assertFalse(backend._action_forced)
        self.assertEqual(api.posted, [])
        self.assertFalse(backend._holding)
        self.assertEqual(backend.complete_action(True), 0)

    def test_a_withheld_enter_is_typed_checked_against_its_own_key(self) -> None:
        api, backend, _events = self.make()
        backend.set_key_filter(lambda event: "defer" if event.key_name == "Return" else False)
        self.assertTrue(backend._handle_native(self.key(EVDEV_KEY_ENTER, WL_FIXTURE_SEQ_FIRST)))
        self.assertEqual(backend.complete_action(True), COMPLETED_ACTION_EVENT_COUNT)
        self.assertEqual((api.known_seqs[-1], api.actions[-1]), (WL_FIXTURE_SEQ_FIRST, True))
        self.assertTrue(backend._handle_native(self.key(EVDEV_KEY_ENTER, WL_FIXTURE_SEQ_FIRST, pressed=False)))
        # The helper typed it itself first: refused, and not typed twice.
        self.assertTrue(backend._handle_native(self.key(EVDEV_KEY_ENTER, WL_FIXTURE_SEQ_SECOND)))
        api.refuse_checked = True
        with self.assertRaisesRegex(WaylandBackendError, INPUT_AHEAD_ERROR):
            backend.complete_action(True)
        self.assertFalse(backend._holding)
        # Its late word, after the action was given up, only lets the release through.
        api.refuse_checked = False
        self.assertTrue(backend._handle_native(self.key(EVDEV_KEY_ENTER, WL_FIXTURE_SEQ_SECOND, pressed=False)))
        self.assertTrue(backend._handle_native(self.key(EVDEV_KEY_ENTER, WL_FIXTURE_SEQ_THIRD)))
        self.assertEqual(backend.complete_action(False), 0)
        with self.assertLogs("keyswitch.wayland_backend", level="INFO"):
            backend._handle_native(self.key(EVDEV_KEY_ENTER, WL_FIXTURE_SEQ_THIRD, forced=True))
        self.assertFalse(backend._action_forced)
        self.assertNotIn(EVDEV_KEY_ENTER, backend._consumed_keys)

    def test_a_password_prompt_the_compositor_draws_is_a_password_field(self) -> None:
        api, backend, _events = self.make()
        reader = PlatformFieldReader(backend)
        api.password_windows.add(WL_FIXTURE_SECOND_WINDOW_ID)
        self.assertTrue(backend.window_is_password(WL_FIXTURE_SECOND_WINDOW_ID))
        field = reader.read("gnome-shell", WL_FIXTURE_SECOND_WINDOW_ID)
        self.assertIsNotNone(field)
        assert field is not None
        self.assertEqual((field.role, field.sensitive, field.source), ("password", True, "compositor"))
        self.assertFalse(backend.window_is_password(WL_FIXTURE_WINDOW_ID))

    def test_keys_typed_unasked_reach_the_engine_and_spoil_a_hold(self) -> None:
        _api, backend, events = self.make()
        backend.set_key_filter(lambda _event: True)
        backend._consumed_keys.add(EVDEV_KEY_Q)
        self.assertFalse(backend._handle_native(self.key(EVDEV_KEY_A, WL_FIXTURE_SEQ_FIRST, delivered=True)))
        self.assertEqual(events[-1].keycode, EVDEV_KEY_A)
        backend.hold_input()
        self.assertFalse(backend._handle_native(
            self.key(EVDEV_KEY_Q, WL_FIXTURE_SEQ_SECOND, pressed=False, delivered=True)))
        self.assertTrue(backend._hold_broken)
        self.assertNotIn(EVDEV_KEY_Q, backend._consumed_keys)
        self.assertEqual(backend._known_seq, WL_FIXTURE_SEQ_SECOND)

    def test_a_correction_the_helper_refuses_changes_no_text(self) -> None:
        api, backend, _events = self.make()
        backend._handle_native(self.key(EVDEV_KEY_A, WL_FIXTURE_SEQ_FIRST))
        backend.hold_input()
        api.refuse_checked = True
        with self.assertRaisesRegex(WaylandBackendError, INPUT_AHEAD_ERROR):
            backend.inject_correction((stroke_event(),), 1, None)
        self.assertEqual(api.known_seqs, [WL_FIXTURE_SEQ_FIRST])

    def test_a_grab_change_forgets_held_keys_and_resets_the_word(self) -> None:
        _api, backend, events = self.make()
        backend.set_key_filter(lambda event: event.key_name == "Pause")
        backend._handle_native(self.key(EVDEV_KEY_LEFTSHIFT, WL_FIXTURE_SEQ_FIRST))
        backend._handle_native(self.key(EVDEV_KEY_PAUSE, WL_FIXTURE_SEQ_SECOND))
        backend.hold_input()
        self.assertFalse(backend._handle_native(NativeKeyEvent(True, 0, WL_FIXTURE_TIMESTAMP_BASE_MS, input_reset=True)))
        self.assertEqual(backend._pressed, set())
        self.assertEqual(backend._consumed_keys, set())
        self.assertTrue(backend._hold_broken)
        self.assertEqual(events[-1].group, POINTER_EVENT_GROUP)
        backend.release_input()
        backend._handle_native(NativeKeyEvent(True, 0, WL_FIXTURE_TIMESTAMP_BASE_MS, input_reset=True))
        self.assertFalse(backend._hold_broken)

    def test_releases_of_keys_down_when_a_hold_began_pass_but_not_a_modifier_or_a_held_press(self) -> None:
        _api, backend, _events = self.make()
        for code in (EVDEV_KEY_A, EVDEV_KEY_Q, EVDEV_KEY_LEFTSHIFT):
            backend._handle_native(self.key(code, WL_FIXTURE_SEQ_FIRST))
        backend.hold_input()
        # The client repeats a key whose release is kept back: the release goes on.
        self.assertFalse(backend._handle_native(self.key(EVDEV_KEY_A, WL_FIXTURE_SEQ_SECOND, pressed=False)))
        # Shift's release would change the case of what the correction types: it waits.
        self.assertTrue(backend._handle_native(self.key(EVDEV_KEY_LEFTSHIFT, WL_FIXTURE_SEQ_SECOND, pressed=False)))
        # Pressed again during the hold, its release belongs to the held press.
        self.assertTrue(backend._handle_native(self.key(EVDEV_KEY_Q, WL_FIXTURE_SEQ_THIRD)))
        self.assertTrue(backend._handle_native(self.key(EVDEV_KEY_Q, WL_FIXTURE_SEQ_THIRD, pressed=False)))
        backend.release_input()
        self.assertEqual(backend._hold_prior_keys, set())


class WaylandEngineIntegrationTests(unittest.TestCase):
    def test_engine_converts_word_defers_enter_and_swallows_undo_hotkey(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            settings = SettingsStore(root / "settings.json")
            settings.set("detection.early_switch", False)
            settings.set("detection.respect_manual_layout", False)
            history = HistoryStore(root / "history.json")
            learning = LearningStore(root / "learning.json")
            api = FakeWaylandAPI()
            api.record_editor = True
            backend = WaylandBackend(api)
            engine = KeySwitchEngine(
                settings,
                history,
                backend=backend,
                learning=learning,
                context_reader=_NullFieldReader(),
                backend_label="Wayland evdev + uinput",
            )
            corrected = threading.Event()
            engine.subscribe_corrections(lambda _plan: corrected.set())
            engine.start()
            try:
                listener = api.listener
                assert listener is not None
                ts = WL_FIXTURE_TIMESTAMP_BASE_MS

                def tap(code: int) -> tuple[bool, bool]:
                    nonlocal ts
                    ts += WL_FIXTURE_TIMESTAMP_STEP_MS
                    swallow_down = listener(NativeKeyEvent(True, code, ts))
                    if not swallow_down:
                        api.post_inputs(((True, code),))
                    ts += WL_FIXTURE_TIMESTAMP_STEP_MS
                    swallow_up = listener(NativeKeyEvent(False, code, ts))
                    if not swallow_up:
                        api.post_inputs(((False, code),))
                    return swallow_down, swallow_up

                def wait_for_editor(expected: str) -> str:
                    deadline = time.monotonic() + KEYBOARD_LISTENER_START_TIMEOUT_SECONDS
                    while time.monotonic() < deadline:
                        text = "".join(api.editor_text)
                        if text == expected:
                            return text
                        time.sleep(LAYOUT_SWITCH_POLL_SECONDS)
                    return "".join(api.editor_text)

                for code in (
                    WL_FIXTURE_KEY_G,
                    WL_FIXTURE_KEY_H,
                    WL_FIXTURE_KEY_B,
                    WL_FIXTURE_KEY_D,
                    WL_FIXTURE_KEY_T,
                    WL_FIXTURE_KEY_N,
                ):
                    tap(code)
                enter_down, enter_up = tap(EVDEV_KEY_ENTER)
                self.assertEqual((enter_down, enter_up), (True, True))
                self.assertTrue(corrected.wait(KEYBOARD_LISTENER_START_TIMEOUT_SECONDS))
                self.assertEqual(wait_for_editor("привет\n"), "привет\n")
                self.assertEqual(backend.current_group(), 1)

                # Undo hotkey Ctrl+Alt+Z after a space-terminated correction is swallowed
                # and reverts the converted word without typing 'z'/'я' into the editor.
                backend.switch_group(0)
                api.editor_text.clear()
                corrected.clear()
                for code in (
                    WL_FIXTURE_KEY_G,
                    WL_FIXTURE_KEY_H,
                    WL_FIXTURE_KEY_B,
                    WL_FIXTURE_KEY_D,
                    WL_FIXTURE_KEY_T,
                    WL_FIXTURE_KEY_N,
                ):
                    tap(code)
                tap(EVDEV_KEY_SPACE)
                self.assertTrue(corrected.wait(KEYBOARD_LISTENER_START_TIMEOUT_SECONDS))
                self.assertEqual(wait_for_editor("привет "), "привет ")
                ts += MODIFIER_RECHECK_GAP_MS
                listener(NativeKeyEvent(True, EVDEV_KEY_LEFTCTRL, ts))
                listener(NativeKeyEvent(True, EVDEV_KEY_LEFTALT, ts + 1))
                swallow_z = listener(NativeKeyEvent(True, EVDEV_KEY_Z, ts + WL_FIXTURE_EXPECTED_TWO_EVENTS))
                listener(NativeKeyEvent(False, EVDEV_KEY_Z, ts + WL_FIXTURE_EXPECTED_THREE_BATCHES))
                listener(NativeKeyEvent(False, EVDEV_KEY_LEFTALT, ts + WL_FIXTURE_EXPECTED_FOUR_BACKSPACES))
                listener(NativeKeyEvent(False, EVDEV_KEY_LEFTCTRL, ts + WL_FIXTURE_EXPECTED_FOUR_BACKSPACES + 1))
                self.assertTrue(swallow_z)
                self.assertEqual(wait_for_editor("ghbdtn "), "ghbdtn ")
            finally:
                engine.stop()


if __name__ == "__main__":
    unittest.main()
