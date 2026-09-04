"""Deterministic ctypes tests for the X11 backend without a live display."""

from __future__ import annotations

import ctypes
import os
import struct
import threading
import time
import unittest
from collections import deque
from types import SimpleNamespace
from unittest.mock import Mock, patch

from keyswitch import x11_backend as x11
from keyswitch.backend import ScreenAnchor
from keyswitch.backend import FocusInfo
from keyswitch.x11_backend import (
    KEY_PRESS,
    KEY_RELEASE,
    LOCK_MASK,
    SHIFT_MASK,
    XRECORD_START_OF_DATA,
    BackendProbe,
    KeyEvent,
    X11Backend,
    X11Error,
    XClassHint,
    XkbStateRec,
    XRecordInterceptData,
    XRecordRange,
    _Libraries,
)


def functions(*names: str) -> SimpleNamespace:
    return SimpleNamespace(**{name: Mock(name=name) for name in names})


X11_FUNCTIONS = (
    "XInitThreads", "XOpenDisplay", "XCloseDisplay", "XFlush", "XSync", "XFree",
    "XkbKeycodeToKeysym", "XkbQueryExtension", "XkbLockGroup", "XkbGetState",
    "XKeysymToString", "XKeysymToKeycode", "XGetInputFocus", "XSetInputFocus", "XGetClassHint", "XQueryTree",
    "XDefaultScreen", "XRootWindow", "XQueryPointer", "XMoveWindow", "XRaiseWindow",
    "XInternAtom", "XGetWindowProperty",
)
XTST_FUNCTIONS = (
    "XRecordQueryVersion", "XRecordAllocRange", "XRecordCreateContext",
    "XRecordEnableContext", "XRecordDisableContext", "XRecordFreeContext",
    "XRecordFreeData", "XTestQueryExtension", "XTestFakeKeyEvent", "XTestGrabControl",
)


class FakeLibraries:
    def __init__(self) -> None:
        self.x11 = functions(*X11_FUNCTIONS)
        self.xtst = functions(*XTST_FUNCTIONS)
        self.xkb = functions("xkb_keysym_to_utf32")
        self.record_callback_type = lambda callback: callback
        self.x11.XOpenDisplay.side_effect = [101, 102]
        self.x11.XkbQueryExtension.return_value = 1
        self.x11.XkbLockGroup.return_value = 1
        self.x11.XKeysymToKeycode.side_effect = lambda _display, keysym: 22 if keysym == 0xFF08 else 50
        self.x11.XkbKeycodeToKeysym.return_value = ord("a")
        self.x11.XKeysymToString.return_value = b"a"
        self.x11.XkbGetState.return_value = 0
        self.x11.XGetInputFocus.return_value = 0
        self.x11.XGetClassHint.return_value = 0
        self.x11.XInternAtom.return_value = 0
        self.x11.XGetWindowProperty.return_value = 1
        self.x11.XQueryTree.return_value = 0
        self.xtst.XRecordQueryVersion.return_value = 1
        self.xtst.XRecordCreateContext.return_value = 42
        self.xtst.XRecordEnableContext.return_value = 1
        self.xtst.XTestQueryExtension.return_value = 1
        self.xtst.XTestFakeKeyEvent.return_value = 1
        self.xkb.xkb_keysym_to_utf32.return_value = ord("a")


def set_int(
    pointer: ctypes._CData | ctypes._CArgObject | int, value: int
) -> None:
    ctypes.cast(pointer, ctypes.POINTER(ctypes.c_int)).contents.value = value


def set_ulong(
    pointer: ctypes._CData | ctypes._CArgObject | int, value: int
) -> None:
    ctypes.cast(pointer, ctypes.POINTER(ctypes.c_ulong)).contents.value = value


def backend_with(libraries: FakeLibraries | None = None, group_count: int = 2) -> tuple[X11Backend, FakeLibraries]:
    libraries = libraries or FakeLibraries()
    with patch("keyswitch.x11_backend._Libraries", return_value=libraries):
        backend = X11Backend(group_count)
    return backend, libraries


def payload(event_type: int, *, keycode: int = 38, state: int = 0, timestamp: int = 77) -> bytes:
    return struct.pack(
        "=BBHIIIIhhhhHBB",
        event_type,
        keycode,
        0,
        timestamp,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        state,
        1,
        0,
    )


class LibraryBindingTests(unittest.TestCase):
    def setUp(self) -> None:
        self.original_initialized = _Libraries._initialized
        _Libraries._initialized = False

    def tearDown(self) -> None:
        _Libraries._initialized = self.original_initialized

    def test_missing_library_and_failed_thread_initialization(self) -> None:
        with patch("ctypes.util.find_library", return_value=None):
            with self.assertRaisesRegex(X11Error, "Не найдены"):
                _Libraries()
        fake_x11 = functions(*X11_FUNCTIONS)
        fake_xtst = functions(*XTST_FUNCTIONS)
        fake_xkb = functions("xkb_keysym_to_utf32")
        fake_x11.XInitThreads.return_value = 0
        with (
            patch("ctypes.util.find_library", side_effect=["X11", "Xtst", "xkb"]),
            patch("ctypes.CDLL", side_effect=[fake_x11, fake_xtst, fake_xkb]),
        ):
            with self.assertRaisesRegex(X11Error, "XInitThreads"):
                _Libraries()

    def test_signatures_are_declared_and_initialization_is_process_wide(self) -> None:
        fake_x11 = functions(*X11_FUNCTIONS)
        fake_xtst = functions(*XTST_FUNCTIONS)
        fake_xkb = functions("xkb_keysym_to_utf32")
        fake_x11.XInitThreads.return_value = 1
        libraries = [fake_x11, fake_xtst, fake_xkb, fake_x11, fake_xtst, fake_xkb]
        with (
            patch("ctypes.util.find_library", side_effect=["X11", "Xtst", "xkb"] * 2),
            patch("ctypes.CDLL", side_effect=libraries),
        ):
            first = _Libraries()
            second = _Libraries()
        self.assertIsNotNone(first.record_callback_type)
        self.assertIsNotNone(second.record_callback_type)
        fake_x11.XInitThreads.assert_called_once_with()
        self.assertEqual(first.x11.XOpenDisplay.restype, ctypes.c_void_p)
        self.assertEqual(first.xtst.XTestFakeKeyEvent.restype, ctypes.c_int)
        self.assertEqual(first.xkb.xkb_keysym_to_utf32.restype, ctypes.c_uint32)


class BackendOpenProbeTests(unittest.TestCase):
    def test_group_count_and_running_property_are_clamped(self) -> None:
        low, _ = backend_with(group_count=1)
        high, _ = backend_with(group_count=99)
        self.assertEqual((low.group_count, high.group_count), (2, 4))
        self.assertFalse(low.running)
        low._running.set()
        self.assertTrue(low.running)

    def test_a_key_filter_is_accepted_and_ignored(self) -> None:
        # XRecord only observes: a key cannot be withheld from the focused
        # client, so the shared contract is satisfied by doing nothing.
        backend, _ = backend_with()
        backend.set_key_filter(lambda _event: True)
        backend.set_key_filter(None)
        self.assertFalse(backend.running)

    def test_open_rejects_wayland_missing_display_and_open_failure(self) -> None:
        backend, libraries = backend_with()
        with patch.dict(os.environ, {"XDG_SESSION_TYPE": "wayland", "DISPLAY": ":1"}, clear=True):
            with self.assertRaisesRegex(X11Error, "Wayland"):
                backend._open()
        with patch.dict(os.environ, {"XDG_SESSION_TYPE": "x11"}, clear=True):
            with self.assertRaisesRegex(X11Error, "DISPLAY"):
                backend._open()
        libraries.x11.XOpenDisplay.side_effect = [0, 0]
        with patch.dict(os.environ, {"DISPLAY": ":9"}, clear=True):
            with self.assertRaisesRegex(X11Error, "открыть"):
                backend._open()

    def test_open_checks_xkb_and_records_negotiated_version(self) -> None:
        backend, libraries = backend_with()
        libraries.x11.XkbQueryExtension.return_value = 0
        with patch.dict(os.environ, {"DISPLAY": ":9"}, clear=True):
            with self.assertRaisesRegex(X11Error, "XKB"):
                backend._open()
        backend, libraries = backend_with()

        def query(
            _display: object,
            _opcode: object,
            _event: object,
            _error: object,
            major: ctypes._CData | ctypes._CArgObject | int,
            minor: ctypes._CData | ctypes._CArgObject | int,
        ) -> int:
            set_int(major, 1)
            set_int(minor, 2)
            return 1

        libraries.x11.XkbQueryExtension.side_effect = query
        with patch.dict(os.environ, {"DISPLAY": ":9"}, clear=True):
            backend._open()
        self.assertEqual((backend._control, backend._record, backend._xkb_version), (101, 102, "1.2"))

    def test_probe_success_failure_and_temporary_cleanup(self) -> None:
        backend, libraries = backend_with()
        backend._control, backend._record = 101, 102

        def record_version(
            _display: object,
            major: ctypes._CData | ctypes._CArgObject | int,
            minor: ctypes._CData | ctypes._CArgObject | int,
        ) -> int:
            set_int(major, 1)
            set_int(minor, 13)
            return 1

        def xtest_version(
            _display: object,
            _event: object,
            _error: object,
            major: ctypes._CData | ctypes._CArgObject | int,
            minor: ctypes._CData | ctypes._CArgObject | int,
        ) -> int:
            set_int(major, 2)
            set_int(minor, 2)
            return 1

        def group(
            _display: object,
            _device: object,
            state: ctypes._CData | ctypes._CArgObject | int,
        ) -> int:
            ctypes.cast(state, ctypes.POINTER(XkbStateRec)).contents.group = 1
            return 0

        libraries.xtst.XRecordQueryVersion.side_effect = record_version
        libraries.xtst.XTestQueryExtension.side_effect = xtest_version
        libraries.x11.XkbGetState.side_effect = group
        with patch.dict(os.environ, {"DISPLAY": ":1", "XDG_SESSION_TYPE": "x11"}, clear=True):
            result = backend.probe()
        self.assertEqual(result, BackendProbe(True, "x11", ":1", "1.13", "2.2", "—", 1))

        libraries.xtst.XRecordQueryVersion.return_value = 0
        libraries.xtst.XRecordQueryVersion.side_effect = None
        failed = backend.probe()
        self.assertFalse(failed.available)
        self.assertIn("XRecord", failed.error)

        temporary, _ = backend_with()
        with patch.object(temporary, "_open", side_effect=X11Error("no display")), patch.object(temporary, "close") as close:
            failed = temporary.probe()
        self.assertFalse(failed.available)
        close.assert_called_once_with()

    def test_probe_reports_missing_xtest(self) -> None:
        backend, libraries = backend_with()
        backend._control, backend._record = 1, 2
        libraries.xtst.XTestQueryExtension.return_value = 0
        result = backend.probe()
        self.assertFalse(result.available)
        self.assertIn("XTEST", result.error)


class BackendCaptureTests(unittest.TestCase):
    def test_start_idempotence_and_all_resource_failures(self) -> None:
        listener = Mock()
        backend, libraries = backend_with()
        backend._running.set()
        backend.start(listener)
        self.assertIsNone(backend._listener)

        for failure, message in (("query", "XRecord"), ("range", "диапазон"), ("context", "контекст")):
            with self.subTest(failure=failure):
                backend, libraries = backend_with()
                backend._control, backend._record = 1, 2
                range_value = XRecordRange()
                libraries.xtst.XRecordAllocRange.return_value = ctypes.pointer(range_value)
                if failure == "query":
                    libraries.xtst.XRecordQueryVersion.return_value = 0
                elif failure == "range":
                    libraries.xtst.XRecordAllocRange.return_value = None
                else:
                    libraries.xtst.XRecordCreateContext.return_value = 0
                with self.assertRaisesRegex(X11Error, message):
                    backend.start(listener)

    def test_successful_start_configures_range_callback_and_thread(self) -> None:
        backend, libraries = backend_with()
        backend._control, backend._record = 1, 2
        range_value = XRecordRange()
        libraries.xtst.XRecordAllocRange.return_value = ctypes.pointer(range_value)
        thread = Mock()

        def confirm_backend_start() -> None:
            backend._capture_ready.set()
            backend._capture_start_finished.set()

        thread.start.side_effect = confirm_backend_start
        with patch("keyswitch.x11_backend.threading.Thread", return_value=thread):
            backend.start(Mock())
        self.assertEqual(range_value.device_events.first, KEY_PRESS)
        self.assertEqual(range_value.device_events.last, 4)  # button presses invalidate the caret
        self.assertEqual(backend._context, 42)
        self.assertTrue(backend.running)
        libraries.x11.XSync.assert_called_once_with(1, 0)
        thread.start.assert_called_once_with()

        opened, opened_libraries = backend_with()
        opened_range = XRecordRange()
        opened_libraries.xtst.XRecordAllocRange.return_value = ctypes.pointer(opened_range)

        def open_backend() -> None:
            opened._control, opened._record = 1, 2

        opened_thread = Mock()

        def confirm_opened_start() -> None:
            opened._capture_ready.set()
            opened._capture_start_finished.set()

        opened_thread.start.side_effect = confirm_opened_start
        with patch.object(opened, "_open", side_effect=open_backend), patch(
            "keyswitch.x11_backend.threading.Thread", return_value=opened_thread
        ):
            opened.start(Mock())
        self.assertEqual((opened._control, opened._record), (1, 2))

    def test_start_requires_record_ready_confirmation(self) -> None:
        timed_out, timeout_libraries = backend_with()
        timed_out._control, timed_out._record = 1, 2
        timeout_range = XRecordRange()
        timeout_libraries.xtst.XRecordAllocRange.return_value = ctypes.pointer(
            timeout_range
        )
        timeout_thread = Mock()
        with (
            patch.object(x11, "XRECORD_START_TIMEOUT", 0.0),
            patch(
                "keyswitch.x11_backend.threading.Thread",
                return_value=timeout_thread,
            ),
        ):
            with self.assertRaisesRegex(X11Error, "5 секунд"):
                timed_out.start(Mock())
        self.assertIsNone(timed_out._control)
        self.assertIsNone(timed_out._record)

        stopped, stopped_libraries = backend_with()
        stopped._control, stopped._record = 1, 2
        stopped_range = XRecordRange()
        stopped_libraries.xtst.XRecordAllocRange.return_value = ctypes.pointer(
            stopped_range
        )
        stopped_thread = Mock()
        stopped_thread.start.side_effect = stopped._capture_start_finished.set
        with patch(
            "keyswitch.x11_backend.threading.Thread", return_value=stopped_thread
        ):
            with self.assertRaisesRegex(X11Error, "до подтверждения"):
                stopped.start(Mock())
        self.assertIsNone(stopped._control)
        self.assertIsNone(stopped._record)

    def test_record_loop_success_zero_status_and_exception(self) -> None:
        backend, libraries = backend_with()
        backend._record = 2
        backend._context = 42
        backend._record_callback = Mock()
        backend._running.set()
        libraries.xtst.XRecordEnableContext.return_value = 0
        with patch.object(x11.LOGGER, "error") as error:
            backend._record_loop()
        error.assert_called_once()
        self.assertFalse(backend.running)
        self.assertTrue(backend._capture_start_finished.is_set())

        backend._running.set()
        libraries.xtst.XRecordEnableContext.side_effect = RuntimeError("record crash")
        with patch.object(x11.LOGGER, "exception") as logged:
            backend._record_loop()
        logged.assert_called_once()
        self.assertFalse(backend.running)
        backend._running.clear()
        libraries.xtst.XRecordEnableContext.side_effect = None
        libraries.xtst.XRecordEnableContext.return_value = 0
        with patch.object(x11.LOGGER, "error") as error:
            backend._record_loop()
        error.assert_not_called()

    def test_record_callback_filters_and_dispatches_complete_events(self) -> None:
        backend, libraries = backend_with()
        listener = Mock()
        backend._listener = listener
        event_payload = payload(KEY_PRESS)
        buffer = (ctypes.c_ubyte * len(event_payload)).from_buffer_copy(event_payload)
        record = XRecordInterceptData(
            category=0,
            client_swapped=0,
            data=ctypes.cast(buffer, ctypes.POINTER(ctypes.c_ubyte)),
            data_len=8,
        )
        pointer = ctypes.pointer(record)
        decoded = KeyEvent(True, 38, "a", "a", ("a", "ф"), 0, 0, 77)
        with patch.object(backend, "_decode_event", return_value=decoded):
            backend._handle_record_data(0, pointer)
        listener.assert_called_once_with(decoded)
        libraries.xtst.XRecordFreeData.assert_called_with(pointer)

        listener.reset_mock()
        record.category = XRECORD_START_OF_DATA
        backend._handle_record_data(0, pointer)
        listener.assert_not_called()
        self.assertTrue(backend._capture_ready.is_set())
        self.assertTrue(backend._capture_start_finished.is_set())

        record.category = 1
        backend._handle_record_data(0, pointer)
        listener.assert_not_called()
        record.category = 0
        backend._listener = None
        with patch.object(backend, "_decode_event", return_value=decoded):
            backend._handle_record_data(0, pointer)
        backend._listener = listener
        with patch.object(backend, "_decode_event", return_value=None):
            backend._handle_record_data(0, pointer)
        null_pointer = ctypes.POINTER(XRecordInterceptData)()
        with patch.object(x11.LOGGER, "exception") as logged:
            backend._handle_record_data(0, null_pointer)
        logged.assert_called_once()

    def test_decode_ignores_non_keyboard_and_normalizes_special_keys(self) -> None:
        backend, _ = backend_with()
        backend._control = 1
        self.assertIsNone(backend._decode_event(payload(12)))
        with (
            patch.object(backend, "_character_for_keycode", side_effect=["a", "ф"]),
            patch.object(backend, "_key_name", return_value="Return"),
            patch.object(backend, "_consume_expected", return_value=True),
        ):
            event = backend._decode_event(payload(KEY_PRESS, state=1 << 13))
        assert event is not None
        self.assertEqual((event.character, event.group, event.synthetic), ("\n", 1, True))
        with patch.object(backend, "_character_for_keycode", side_effect=["a", "ф"]), patch.object(backend, "_key_name", return_value="ISO_Left_Tab"):
            event = backend._decode_event(payload(KEY_RELEASE))
        assert event is not None
        self.assertEqual((event.character, event.pressed), ("\t", False))
        with patch.object(backend, "_character_for_keycode", side_effect=["a", "ф"]), patch.object(backend, "_key_name", return_value="a"):
            event = backend._decode_event(payload(KEY_PRESS))
        assert event is not None
        self.assertEqual(event.character, "a")


class BackendTranslationTests(unittest.TestCase):
    def test_character_conversion_control_bounds_caps_and_printability(self) -> None:
        backend, libraries = backend_with()
        self.assertEqual(backend._character_for_keycode(38, 0, 0), "")
        backend._control = 1
        libraries.xkb.xkb_keysym_to_utf32.return_value = 0
        self.assertEqual(backend._character_for_keycode(38, 0, 0), "")
        libraries.xkb.xkb_keysym_to_utf32.return_value = 0x110000
        self.assertEqual(backend._character_for_keycode(38, 0, 0), "")
        libraries.xkb.xkb_keysym_to_utf32.return_value = ord("a")
        self.assertEqual(backend._character_for_keycode(38, 0, LOCK_MASK), "A")
        libraries.xkb.xkb_keysym_to_utf32.return_value = ord("A")
        self.assertEqual(backend._character_for_keycode(38, 0, LOCK_MASK | SHIFT_MASK), "a")
        libraries.xkb.xkb_keysym_to_utf32.return_value = 1
        self.assertEqual(backend._character_for_keycode(38, 0, 0), "")
        libraries.xkb.xkb_keysym_to_utf32.return_value = ord("\n")
        self.assertEqual(backend._character_for_keycode(38, 0, 0), "\n")

    def test_key_name_and_expected_synthetic_queue(self) -> None:
        backend, libraries = backend_with()
        self.assertEqual(backend._key_name(38), "")
        backend._control = 1
        libraries.x11.XKeysymToString.return_value = None
        self.assertEqual(backend._key_name(38), "")
        libraries.x11.XKeysymToString.return_value = b"Return"
        self.assertEqual(backend._key_name(38), "Return")

        backend._expected = deque([(True, 38)])
        backend._expected_deadline = time.monotonic() + 5
        self.assertTrue(backend._consume_expected(True, 38))
        self.assertFalse(backend._consume_expected(True, 38))
        backend._expected = deque([(True, 39)])
        backend._expected_deadline = time.monotonic() - 1
        self.assertFalse(backend._consume_expected(True, 39))
        self.assertEqual(backend._expected, deque())

    def test_current_and_switch_group_paths(self) -> None:
        backend, libraries = backend_with()
        self.assertEqual(backend.current_group(), -1)
        with self.assertRaisesRegex(X11Error, "Неизвестная группа"):
            backend.switch_group(2)
        with self.assertRaisesRegex(X11Error, "не запущен"):
            backend.switch_group(1)
        backend._control = 1

        def state_ok(
            _display: object,
            _device: object,
            state: ctypes._CData | ctypes._CArgObject | int,
        ) -> int:
            ctypes.cast(state, ctypes.POINTER(XkbStateRec)).contents.group = 3
            return 0

        libraries.x11.XkbGetState.side_effect = state_ok
        self.assertEqual(backend.current_group(), 3)
        libraries.x11.XkbGetState.side_effect = None
        libraries.x11.XkbGetState.return_value = 1
        self.assertEqual(backend.current_group(), -1)
        libraries.x11.XkbLockGroup.return_value = 0
        with self.assertRaisesRegex(X11Error, "переключить"):
            backend.switch_group(1)
        libraries.x11.XkbLockGroup.return_value = 1
        backend.switch_group(1)
        libraries.x11.XFlush.assert_called_with(1)

    def test_pointer_anchor_and_popup_window_positioning(self) -> None:
        backend, libraries = backend_with()
        self.assertIsNone(backend.input_anchor())
        self.assertFalse(backend.position_window(55, 1, 2))
        self.assertFalse(backend.restore_window(55))
        backend._control = 1
        libraries.x11.XDefaultScreen.return_value = 2
        libraries.x11.XRootWindow.return_value = 99
        libraries.x11.XQueryPointer.return_value = 0
        self.assertIsNone(backend.input_anchor())

        def query_pointer(
            _display: object,
            _root: object,
            root_return: ctypes._CData | ctypes._CArgObject | int,
            child_return: ctypes._CData | ctypes._CArgObject | int,
            root_x: ctypes._CData | ctypes._CArgObject | int,
            root_y: ctypes._CData | ctypes._CArgObject | int,
            window_x: ctypes._CData | ctypes._CArgObject | int,
            window_y: ctypes._CData | ctypes._CArgObject | int,
            mask: ctypes._CData | ctypes._CArgObject | int,
        ) -> int:
            set_ulong(root_return, 99)
            set_ulong(child_return, 100)
            set_int(root_x, 640)
            set_int(root_y, 480)
            set_int(window_x, 12)
            set_int(window_y, 34)
            set_int(mask, 0)
            return 1

        def focused(
            _display: object,
            window: ctypes._CData | ctypes._CArgObject | int,
            revert: ctypes._CData | ctypes._CArgObject | int,
        ) -> int:
            set_ulong(window, 777)
            set_int(revert, 0)
            return 1

        libraries.x11.XQueryPointer.side_effect = query_pointer
        libraries.x11.XGetInputFocus.side_effect = focused
        self.assertEqual(backend.input_anchor(), ScreenAnchor(640, 480, 777))
        self.assertFalse(backend.position_window(0, 1, 2))
        self.assertFalse(backend.restore_window(None))
        self.assertTrue(backend.position_window(55, 10, 20))
        self.assertTrue(backend.restore_window(777))
        libraries.x11.XMoveWindow.assert_called_once_with(1, 55, 10, 20)
        libraries.x11.XRaiseWindow.assert_called_once_with(1, 55)
        libraries.x11.XSetInputFocus.assert_called_once_with(1, 777, 2, 0)
        libraries.x11.XFlush.assert_called_with(1)


    def test_focused_window_recognises_this_process_by_net_wm_pid(self) -> None:
        backend, libraries = backend_with()
        self.assertIsNone(backend.focused_window())
        backend._control = 1
        focus_value = 0
        owners: dict[int, int] = {}
        parents: dict[int, int] = {}
        property_format = 32
        property_count = 1
        children_pointer: int | None = None
        owner = (ctypes.c_ulong * 1)(0)

        def focused(
            _display: object,
            window: ctypes._CData | ctypes._CArgObject | int,
            revert: ctypes._CData | ctypes._CArgObject | int,
        ) -> int:
            set_ulong(window, focus_value)
            set_int(revert, 0)
            return 1

        def get_property(
            _display: object,
            window: int,
            _atom: int,
            _offset: int,
            _length: int,
            _delete: int,
            _type: int,
            _actual_type: ctypes._CData | ctypes._CArgObject | int,
            actual_format: ctypes._CData | ctypes._CArgObject | int,
            count: ctypes._CData | ctypes._CArgObject | int,
            _remaining: ctypes._CData | ctypes._CArgObject | int,
            data: ctypes._CData | ctypes._CArgObject | int,
        ) -> int:
            if window not in owners:
                return 1  # BadWindow, or no such property on this window
            owner[0] = owners[window]
            set_int(actual_format, property_format)
            set_ulong(count, property_count)
            ctypes.cast(data, ctypes.POINTER(ctypes.c_void_p)).contents.value = (
                ctypes.addressof(owner)
            )
            return 0

        def query_tree(
            _display: object,
            window: int,
            root: ctypes._CData | ctypes._CArgObject | int,
            parent: ctypes._CData | ctypes._CArgObject | int,
            children: ctypes._CData | ctypes._CArgObject | int,
            count: ctypes._CData | ctypes._CArgObject | int,
        ) -> int:
            if window not in parents:
                return 0  # the window is gone
            set_ulong(root, 99)
            set_ulong(parent, parents[window])
            ctypes.cast(children, ctypes.POINTER(ctypes.c_void_p)).contents.value = (
                children_pointer
            )
            ctypes.cast(count, ctypes.POINTER(ctypes.c_uint)).contents.value = 0
            return 1

        libraries.x11.XGetInputFocus.side_effect = focused
        libraries.x11.XGetWindowProperty.side_effect = get_property
        libraries.x11.XQueryTree.side_effect = query_tree

        # No window holds the focus: None and PointerRoot mean nobody.
        self.assertIsNone(backend.focused_window())
        focus_value = 1
        self.assertIsNone(backend.focused_window())
        libraries.x11.XGetInputFocus.side_effect = None
        libraries.x11.XGetInputFocus.return_value = 0
        self.assertIsNone(backend.focused_window())
        libraries.x11.XGetInputFocus.side_effect = focused

        # Without the _NET_WM_PID atom no window can be attributed.
        focus_value = 777
        libraries.x11.XInternAtom.return_value = 0
        self.assertEqual(backend.focused_window(), FocusInfo(777, False))
        libraries.x11.XGetWindowProperty.assert_not_called()

        backend._own_windows.clear()
        backend._net_wm_pid_atom = None
        libraries.x11.XInternAtom.return_value = 5
        owners[777] = os.getpid()
        owners[555] = os.getpid() + 1
        parents[778] = 777  # a child window of ours, as toolkits create them
        parents[779] = 779  # its own parent: a root window
        self.assertEqual(backend.focused_window(), FocusInfo(777, True))
        libraries.x11.XFree.assert_called()
        focus_value = 778
        self.assertEqual(backend.focused_window(), FocusInfo(778, True))
        focus_value = 555
        self.assertEqual(backend.focused_window(), FocusInfo(555, False))
        # A window whose ancestry ends without the property, one that is gone
        # while it is examined, and one that is its own parent.
        focus_value = 780
        self.assertEqual(backend.focused_window(), FocusInfo(780, False))
        focus_value = 779
        self.assertEqual(backend.focused_window(), FocusInfo(779, False))

        # Verdicts are cached per window id, and the cache is bounded.
        libraries.x11.XGetWindowProperty.reset_mock()
        focus_value = 777
        self.assertEqual(backend.focused_window(), FocusInfo(777, True))
        libraries.x11.XGetWindowProperty.assert_not_called()
        backend._own_windows.clear()
        backend._own_windows.update({index: False for index in range(1000, 1256)})
        self.assertEqual(backend.focused_window(), FocusInfo(777, True))
        self.assertEqual(backend._own_windows, {777: True})

        # A property of an unexpected shape is ignored, and so is a window
        # tower deeper than the bounded walk.
        backend._own_windows.clear()
        property_format = 8
        self.assertEqual(backend.focused_window(), FocusInfo(777, False))
        backend._own_windows.clear()
        property_format = 32
        property_count = 0
        self.assertEqual(backend.focused_window(), FocusInfo(777, False))
        backend._own_windows.clear()
        property_count = 1
        owners.pop(777)
        parents.update({window: window + 1 for window in range(777, 900)})
        children_pointer = ctypes.addressof(owner)
        libraries.x11.XQueryTree.reset_mock()
        self.assertEqual(backend.focused_window(), FocusInfo(777, False))
        self.assertEqual(libraries.x11.XQueryTree.call_count, 16)


class BackendInjectionTests(unittest.TestCase):
    @staticmethod
    def stroke(*, shifted: bool = False) -> KeyEvent:
        return KeyEvent(True, 38, "a", "A" if shifted else "a", ("a", "ф"), 0, SHIFT_MASK if shifted else 0, 1)

    def test_requires_open_backend_and_required_keycodes(self) -> None:
        backend, libraries = backend_with()
        with self.assertRaisesRegex(X11Error, "не запущен"):
            backend.inject_correction([], 1, None)
        backend._control = 1
        libraries.x11.XKeysymToKeycode.side_effect = [0, 50]
        with self.assertRaisesRegex(X11Error, "BackSpace"):
            backend.inject_correction([], 1, None)

    def test_success_includes_shift_delete_boundary_and_expected_events(self) -> None:
        backend, libraries = backend_with()
        backend._control = 1
        boundary = KeyEvent(True, 59, "comma", ",", (",", "б"), 0, SHIFT_MASK, 1)
        backend.inject_correction([self.stroke(shifted=True)], 1, boundary, source_group=0)
        self.assertTrue(backend._expected)
        self.assertEqual(libraries.xtst.XTestGrabControl.call_args_list[0].args, (1, 1))
        self.assertEqual(libraries.xtst.XTestGrabControl.call_args_list[-1].args, (1, 0))
        lock_groups = [call.args[2] for call in libraries.x11.XkbLockGroup.call_args_list]
        self.assertEqual(lock_groups, [1, 0, 1])
        libraries.x11.XSync.assert_called_once_with(1, 0)

    def test_empty_strokes_and_same_layout_boundary_use_main_sequence(self) -> None:
        backend, libraries = backend_with()
        backend._control = 1
        boundary = KeyEvent(True, 65, "space", " ", (" ", " "), 0, 0, 1)
        backend.inject_correction([], 1, boundary)
        self.assertEqual(libraries.x11.XkbLockGroup.call_count, 1)
        self.assertEqual(libraries.xtst.XTestFakeKeyEvent.call_count, 4)

    def test_late_keys_are_deleted_and_typed_again_outside_the_expected_echo(self) -> None:
        backend, libraries = backend_with()
        backend._control = 1
        backend.hold_input()
        self.assertEqual(
            backend.inject_correction([self.stroke()], 1, None, late=[self.stroke(shifted=True)]),
            0,
        )
        # Two backspaces, the word, then the late key with its shift: the late
        # taps are not expected echoes, so the engine sees them as typed.
        self.assertEqual(libraries.xtst.XTestFakeKeyEvent.call_count, 4 + 2 + 4)
        self.assertEqual(len(backend._expected), 6)

        libraries.xtst.XTestFakeKeyEvent.side_effect = [1] * 6 + [0]
        with self.assertRaises(X11Error):
            backend.inject_correction([self.stroke()], 1, None, late=[self.stroke()])
        self.assertEqual(backend._expected, deque())

    def test_target_lock_and_fake_event_errors_clear_expected_and_release_grab(self) -> None:
        for mode in ("lock", "fake"):
            with self.subTest(mode=mode):
                backend, libraries = backend_with()
                backend._control = 1
                if mode == "lock":
                    libraries.x11.XkbLockGroup.return_value = 0
                else:
                    libraries.xtst.XTestFakeKeyEvent.side_effect = [1, 0]
                with self.assertRaises(X11Error):
                    backend.inject_correction([self.stroke()], 1, None)
                self.assertEqual(backend._expected, deque())
                libraries.xtst.XTestGrabControl.assert_called_with(1, 0)
                libraries.x11.XFlush.assert_called_with(1)

    def test_target_boundary_fake_event_error_clears_expected(self) -> None:
        backend, libraries = backend_with()
        backend._control = 1
        boundary = KeyEvent(True, 65, "space", " ", (" ", " "), 0, 0, 1)
        libraries.xtst.XTestFakeKeyEvent.side_effect = [1, 1, 0]
        with self.assertRaisesRegex(X11Error, "keycode 65"):
            backend.inject_correction([], 1, boundary)
        self.assertEqual(backend._expected, deque())
        libraries.xtst.XTestGrabControl.assert_called_with(1, 0)
        libraries.x11.XFlush.assert_called_with(1)

    def test_preserved_boundary_group_lock_fake_and_restore_errors(self) -> None:
        boundary = KeyEvent(True, 59, "comma", ",", (",", "б"), 0, 0, 1)
        for mode in ("source_lock", "boundary_fake", "restore"):
            with self.subTest(mode=mode):
                backend, libraries = backend_with()
                backend._control = 1
                if mode == "source_lock":
                    libraries.x11.XkbLockGroup.side_effect = [1, 0]
                elif mode == "boundary_fake":
                    libraries.xtst.XTestFakeKeyEvent.side_effect = [1] * 6 + [0]
                else:
                    libraries.x11.XkbLockGroup.side_effect = [1, 1, 0]
                with self.assertRaises(X11Error):
                    backend.inject_correction([self.stroke()], 1, boundary, source_group=0)
                self.assertEqual(backend._expected, deque())


class BackendApplicationLifecycleTests(unittest.TestCase):
    def test_active_application_empty_focus_and_direct_class_hint(self) -> None:
        backend, libraries = backend_with()
        self.assertEqual(backend.active_application(), "")
        backend._control = 1
        self.assertEqual(backend.active_application(), "")
        class_buffer = ctypes.create_string_buffer(b"EditorClass")
        name_buffer = ctypes.create_string_buffer(b"editor")

        def focus(
            _display: object,
            window: ctypes._CData | ctypes._CArgObject | int,
            _revert: object,
        ) -> int:
            ctypes.cast(window, ctypes.POINTER(ctypes.c_ulong)).contents.value = 55
            return 1

        def hint(
            _display: object,
            _window: object,
            output: ctypes._CData | ctypes._CArgObject | int,
        ) -> int:
            result = ctypes.cast(output, ctypes.POINTER(XClassHint)).contents
            result.res_class = ctypes.cast(class_buffer, ctypes.c_void_p).value
            return 1

        libraries.x11.XGetInputFocus.side_effect = focus
        libraries.x11.XGetClassHint.side_effect = hint
        self.assertEqual(backend.active_application(), "EditorClass")
        self.assertEqual(libraries.x11.XFree.call_count, 1)

    def test_resource_name_fallback_parent_traversal_and_tree_breaks(self) -> None:
        backend, libraries = backend_with()
        backend._control = 1
        name_buffer = ctypes.create_string_buffer(b"resource-only")

        def focus(
            _display: object,
            window: ctypes._CData | ctypes._CArgObject | int,
            _revert: object,
        ) -> int:
            ctypes.cast(window, ctypes.POINTER(ctypes.c_ulong)).contents.value = 10
            return 1

        hint_calls = 0

        def hint(
            _display: object,
            _window: object,
            output: ctypes._CData | ctypes._CArgObject | int,
        ) -> int:
            nonlocal hint_calls
            hint_calls += 1
            if hint_calls == 1:
                return 0
            result = ctypes.cast(output, ctypes.POINTER(XClassHint)).contents
            result.res_name = ctypes.cast(name_buffer, ctypes.c_void_p).value
            return 1

        child_storage = (ctypes.c_ulong * 1)(99)

        def query(
            _display: object,
            _window: object,
            _root: object,
            parent: ctypes._CData | ctypes._CArgObject | int,
            children: ctypes._CData | ctypes._CArgObject | int,
            _count: object,
        ) -> int:
            ctypes.cast(parent, ctypes.POINTER(ctypes.c_ulong)).contents.value = 20
            ctypes.cast(children, ctypes.POINTER(ctypes.POINTER(ctypes.c_ulong)))[0] = ctypes.cast(child_storage, ctypes.POINTER(ctypes.c_ulong))
            return 1

        libraries.x11.XGetInputFocus.side_effect = focus
        libraries.x11.XGetClassHint.side_effect = hint
        libraries.x11.XQueryTree.side_effect = query
        self.assertEqual(backend.active_application(), "resource-only")
        self.assertGreaterEqual(libraries.x11.XFree.call_count, 2)

        libraries.x11.XGetClassHint.side_effect = None
        libraries.x11.XGetClassHint.return_value = 0
        libraries.x11.XQueryTree.side_effect = None
        libraries.x11.XQueryTree.return_value = 0
        self.assertEqual(backend.active_application(), "")

        def query_same(
            _display: object,
            current: int,
            _root: object,
            parent: ctypes._CData | ctypes._CArgObject | int,
            _children: object,
            _count: object,
        ) -> int:
            ctypes.cast(parent, ctypes.POINTER(ctypes.c_ulong)).contents.value = current
            return 1

        libraries.x11.XQueryTree.side_effect = query_same
        self.assertEqual(backend.active_application(), "")

        def focus_none(
            _display: object,
            window: ctypes._CData | ctypes._CArgObject | int,
            _revert: object,
        ) -> int:
            ctypes.cast(window, ctypes.POINTER(ctypes.c_ulong)).contents.value = 0
            return 1

        libraries.x11.XGetInputFocus.side_effect = focus_none
        self.assertEqual(backend.active_application(), "")

        counter = 0

        def focus_one(
            _display: object,
            window: ctypes._CData | ctypes._CArgObject | int,
            _revert: object,
        ) -> int:
            ctypes.cast(window, ctypes.POINTER(ctypes.c_ulong)).contents.value = 1
            return 1

        def query_forever(
            _display: object,
            _current: object,
            _root: object,
            parent: ctypes._CData | ctypes._CArgObject | int,
            _children: object,
            _count: object,
        ) -> int:
            nonlocal counter
            counter += 1
            ctypes.cast(parent, ctypes.POINTER(ctypes.c_ulong)).contents.value = counter + 1
            return 1

        libraries.x11.XGetInputFocus.side_effect = focus_one
        libraries.x11.XQueryTree.side_effect = query_forever
        self.assertEqual(backend.active_application(), "")
        self.assertEqual(counter, 16)

    def test_stop_close_context_manager_and_all_resources(self) -> None:
        backend, libraries = backend_with()
        backend._control = 1
        backend._record = 2
        backend._context = 42
        range_value = XRecordRange()
        backend._range = ctypes.pointer(range_value)
        backend._running.set()
        thread = Mock()
        backend._thread = thread
        backend.stop()
        libraries.xtst.XRecordDisableContext.assert_called_once_with(1, 42)
        thread.join.assert_called_once_with(timeout=2.0)
        backend.close()
        libraries.xtst.XRecordFreeContext.assert_called_once_with(1, 42)
        self.assertIsNone(backend._control)
        self.assertIsNone(backend._record)
        self.assertIsNone(backend._range)
        with backend as returned:
            self.assertIs(returned, backend)

    def test_stop_does_not_join_current_thread(self) -> None:
        backend, _libraries = backend_with()
        backend._thread = threading.current_thread()
        backend.stop()
        self.assertIsNone(backend._thread)


if __name__ == "__main__":
    unittest.main()
