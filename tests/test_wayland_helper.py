"""Unit tests for the isolated Wayland evdev/uinput helper and client protocol."""

from __future__ import annotations

from collections import deque
from collections.abc import Sequence
import select
import socket
import threading
import unittest
from unittest.mock import patch

from fixture_values.wayland import (
    WL_FIXTURE_LONG_BATCH_TAPS,
    WL_FIXTURE_SEQ_AFTER_WRAP,
    WL_FIXTURE_SEQ_BEFORE_WRAP,
    WL_FIXTURE_ADVANCE_PAST_GRAB_WAIT_SECONDS,
    WL_FIXTURE_ADVANCE_PAST_VERDICT_SECONDS,
    WL_FIXTURE_CHANNEL_FD,
    WL_FIXTURE_CHUNK_SPLIT_BYTES,
    WL_FIXTURE_CUSTOM_REPEAT_DELAY_MS,
    WL_FIXTURE_EMIT_COUNT_TWO,
    WL_FIXTURE_EVENT_SEC,
    WL_FIXTURE_EVENT_USEC,
    WL_FIXTURE_EXPECTED_EVENT_TIMESTAMP_MS,
    WL_FIXTURE_FIRST_KEYBOARD_FD,
    WL_FIXTURE_GENERIC_PRODUCT_ID,
    WL_FIXTURE_GENERIC_VENDOR_ID,
    WL_FIXTURE_INOTIFY_FD,
    WL_FIXTURE_LONG_HOLD_SECONDS,
    WL_FIXTURE_POINTER_FD,
    WL_FIXTURE_POLLOUT_MASK,
    WL_FIXTURE_REQ_ID_ONE,
    WL_FIXTURE_REQ_ID_UNKNOWN,
    WL_FIXTURE_SECOND_KEYBOARD_FD,
    WL_FIXTURE_SEQ_STALE,
    WL_FIXTURE_SHORT_HOLD_SECONDS,
    WL_FIXTURE_SHORT_TIMEOUT_SECONDS,
    WL_FIXTURE_THIRD_KEYBOARD_FD,
    WL_FIXTURE_UINPUT_FD,
    WL_FIXTURE_UNKNOWN_COMMAND_ID,
    WL_FIXTURE_UNKNOWN_KEY_VALUE,
    WL_FIXTURE_UNKNOWN_MESSAGE_ID,
)
from keyswitch.constants.wayland import (
    EV_ABS,
    EV_KEY,
    EV_REL,
    EV_SYN,
    EVDEV_ABS_MT_POSITION_X,
    EVDEV_BTN_GAMEPAD,
    EVDEV_BTN_MOUSE,
    EVDEV_BTN_STYLUS,
    EVDEV_KEY_A,
    EVDEV_KEY_CAPSLOCK,
    EVDEV_KEY_LEFTSHIFT,
    EVDEV_KEY_Q,
    EVDEV_REL_X,
    EVDEV_REL_Y,
    EVDEV_REQUIRED_KEYBOARD_KEYS,
    HELPER_CMD_EMIT_BATCH,
    HELPER_CMD_QUERY_KEY_DOWN,
    HELPER_CMD_SEQ_ACK,
    HELPER_CMD_SET_GRAB,
    HELPER_CMD_STOP,
    HELPER_CMD_VERDICT,
    HELPER_EMIT_ACTION,
    HELPER_EMIT_TEXT,
    HELPER_EMIT_UNCHECKED,
    HELPER_KEY_FORCED,
    HELPER_KEY_NEEDS_VERDICT,
    HELPER_MAX_FRAME_BYTES,
    HELPER_MSG_CAPS_LOCK,
    HELPER_MSG_EMIT_ACK,
    HELPER_MSG_ERROR,
    HELPER_MSG_INPUT_RESET,
    HELPER_MSG_KEY,
    HELPER_MSG_KEY_DOWN,
    HELPER_MSG_POINTER,
    HELPER_MSG_READY,
    HELPER_MSG_REPEAT_RESET,
    HELPER_SOCKET_CHUNK_BYTES,
    HELPER_VERDICT_PASS,
    HELPER_VERDICT_SWALLOW,
    KEY_VALUE_PRESS,
    KEY_VALUE_RELEASE,
    KEY_VALUE_REPEAT,
    LED_CAPSL,
    LED_NUML,
    SYN_REPORT,
    TRANSITION_EVENTS,
    WAYLAND_UINPUT_BURST_TRANSITIONS,
    WAYLAND_UINPUT_PRODUCT_ID,
    WAYLAND_UINPUT_VENDOR_ID,
)
from keyswitch.constants.timing import KEYBOARD_LISTENER_START_TIMEOUT_SECONDS, WAYLAND_UINPUT_BURST_PAUSE_SECONDS
from keyswitch.wayland_access import ENABLE_COMMAND
from keyswitch.wayland_helper import (
    CMD_BATCH_HEADER_STRUCT,
    CMD_BATCH_ITEM_STRUCT,
    CMD_QUERY_KEY_STRUCT,
    CMD_SET_GRAB_STRUCT,
    CMD_VERDICT_STRUCT,
    FRAME_HEADER,
    FrameReader,
    HelperClient,
    HelperKey,
    InputDeviceInfo,
    MSG_ACK_STRUCT,
    MSG_KEY_DOWN_STRUCT,
    MSG_KEY_STRUCT,
    RawInputEvent,
    U8_STRUCT,
    U32_STRUCT,
    WaylandInputHelper,
    classify_input_device,
    pack_frame,
    seq_newer,
)


def make_keyboard_info(path: str = "/dev/input/event0", name: str = "AT Keyboard") -> InputDeviceInfo:
    return InputDeviceInfo(
        path=path,
        name=name,
        vendor=WL_FIXTURE_GENERIC_VENDOR_ID,
        product=WL_FIXTURE_GENERIC_PRODUCT_ID,
        ev_bits=frozenset({EV_KEY}),
        key_bits=EVDEV_REQUIRED_KEYBOARD_KEYS,
    )


def make_mouse_info(path: str = "/dev/input/event1", name: str = "USB Mouse") -> InputDeviceInfo:
    return InputDeviceInfo(
        path=path,
        name=name,
        ev_bits=frozenset({EV_KEY, EV_REL}),
        key_bits=frozenset({EVDEV_BTN_MOUSE}),
        rel_bits=frozenset({EVDEV_REL_X, EVDEV_REL_Y}),
    )


class FakeByteChannel:
    def __init__(self, fd: int = WL_FIXTURE_CHANNEL_FD) -> None:
        self._fd = fd
        self.incoming: deque[bytes | Exception] = deque()
        self.sent: list[bytes] = []
        self.send_error: OSError | None = None
        self.close_error: OSError | None = None
        self.closed = False

    def recv(self, _bufsize: int) -> bytes:
        if not self.incoming:
            return b""
        item = self.incoming.popleft()
        if isinstance(item, Exception):
            raise item
        return item

    def sendall(self, data: bytes) -> None:
        if self.send_error is not None:
            raise self.send_error
        self.sent.append(data)

    def close(self) -> None:
        self.closed = True
        if self.close_error is not None:
            raise self.close_error

    def fileno(self) -> int:
        return self._fd

    def drain_frames(self) -> list[tuple[int, bytes]]:
        reader = FrameReader()
        frames: list[tuple[int, bytes]] = []
        while self.sent:
            frames.extend(reader.feed(self.sent.pop(0)))
        return frames


class FakeHelperOS:
    def __init__(self) -> None:
        self.now = 0.0
        self.uinput_fd = WL_FIXTURE_UINPUT_FD
        self.inotify_fd = WL_FIXTURE_INOTIFY_FD
        self.create_uinput_error: Exception | None = None
        self.destroy_uinput_error: OSError | None = None
        self.write_uinput_error: OSError | None = None
        self.uinput_writes: list[tuple[tuple[int, int, int], ...]] = []
        self.uinput_leds: deque[Sequence[tuple[int, int]] | OSError] = deque()
        self.devices: list[InputDeviceInfo] = []
        self.open_results: dict[tuple[str, bool], int | Exception] = {}
        self.close_errors: set[int] = set()
        self.closed_fds: list[int] = []
        self.grabbed: set[int] = set()
        self.grab_errors: set[int] = set()
        self.ungrab_errors: set[int] = set()
        self.keys_down: dict[int, deque[frozenset[int] | OSError] | frozenset[int]] = {}
        self.caps_lock: dict[int, bool] = {}
        self.events: dict[int, deque[Sequence[RawInputEvent] | OSError]] = {}
        self.led_writes: list[tuple[int, int, int]] = []
        self.led_errors: set[int] = set()
        self.inotify_batches: deque[Sequence[str]] = deque()
        self.polls: deque[Sequence[tuple[int, int]]] = deque()
        self.sleeps: list[float] = []
        self.advance_on_sleep = 0.0

    def create_uinput(self) -> int:
        if self.create_uinput_error is not None:
            raise self.create_uinput_error
        return self.uinput_fd

    def destroy_uinput(self, fd: int) -> None:
        self.closed_fds.append(fd)
        if self.destroy_uinput_error is not None:
            raise self.destroy_uinput_error

    def write_uinput(self, _fd: int, events: Sequence[tuple[int, int, int]]) -> None:
        if self.write_uinput_error is not None:
            raise self.write_uinput_error
        self.uinput_writes.append(tuple(events))

    def read_uinput_leds(self, _fd: int) -> Sequence[tuple[int, int]]:
        if not self.uinput_leds:
            return ()
        item = self.uinput_leds.popleft()
        if isinstance(item, OSError):
            raise item
        return item

    def list_event_devices(self) -> Sequence[InputDeviceInfo]:
        return tuple(self.devices)

    def open_device(self, path: str, *, writable: bool) -> int:
        outcome = self.open_results[(path, writable)]
        if isinstance(outcome, Exception):
            raise outcome
        return outcome

    def close_device(self, fd: int) -> None:
        self.closed_fds.append(fd)
        if fd in self.close_errors:
            raise OSError("close failed")

    def grab_device(self, fd: int) -> None:
        if fd in self.grab_errors:
            raise OSError("grab failed")
        self.grabbed.add(fd)

    def ungrab_device(self, fd: int) -> None:
        self.grabbed.discard(fd)
        if fd in self.ungrab_errors:
            raise OSError("ungrab failed")

    def query_keys_down(self, fd: int) -> frozenset[int]:
        entry = self.keys_down.get(fd, frozenset())
        if isinstance(entry, deque):
            if not entry:
                return frozenset()
            item = entry.popleft()
            if isinstance(item, OSError):
                raise item
            return item
        return entry

    def query_caps_lock(self, fd: int) -> bool:
        return self.caps_lock.get(fd, False)

    def read_events(self, fd: int) -> Sequence[RawInputEvent]:
        queue_for_fd = self.events.get(fd)
        if not queue_for_fd:
            return ()
        item = queue_for_fd.popleft()
        if isinstance(item, OSError):
            raise item
        return item

    def write_led(self, fd: int, code: int, value: int) -> None:
        if fd in self.led_errors:
            raise OSError("led failed")
        self.led_writes.append((fd, code, value))

    def open_inotify(self) -> int:
        return self.inotify_fd

    def read_inotify(self, _fd: int) -> Sequence[str]:
        return self.inotify_batches.popleft() if self.inotify_batches else ()

    def poll(self, _fds: Sequence[int], _timeout_ms: int) -> Sequence[tuple[int, int]]:
        return self.polls.popleft() if self.polls else ((WL_FIXTURE_CHANNEL_FD, select.POLLHUP),)

    def monotonic(self) -> float:
        return self.now

    def sleep(self, seconds: float) -> None:
        self.sleeps.append(seconds)
        self.now += self.advance_on_sleep or seconds


class FramingAndClassifierTests(unittest.TestCase):
    def test_pack_frame_and_incremental_frame_reader(self) -> None:
        with self.assertRaisesRegex(ValueError, "exceeds limit"):
            pack_frame(HELPER_MSG_READY, b"x" * (HELPER_MAX_FRAME_BYTES + 1))

        combined = pack_frame(HELPER_MSG_READY, b"\x01") + pack_frame(HELPER_MSG_ERROR, b"oops")
        reader = FrameReader()
        self.assertEqual(reader.feed(combined[:WL_FIXTURE_CHUNK_SPLIT_BYTES]), [])
        self.assertEqual(
            reader.feed(combined[WL_FIXTURE_CHUNK_SPLIT_BYTES:-1]),
            [(HELPER_MSG_READY, b"\x01")],
        )
        self.assertEqual(
            reader.feed(combined[-1:]),
            [(HELPER_MSG_ERROR, b"oops")],
        )

        oversized = FRAME_HEADER.pack(HELPER_MSG_READY, HELPER_MAX_FRAME_BYTES + 1)
        with self.assertRaisesRegex(ValueError, "exceeds limit"):
            FrameReader().feed(oversized)

        event = RawInputEvent(
            WL_FIXTURE_EVENT_SEC,
            WL_FIXTURE_EVENT_USEC,
            EV_KEY,
            EVDEV_KEY_A,
            KEY_VALUE_PRESS,
        )
        self.assertEqual(event.timestamp_ms(), WL_FIXTURE_EXPECTED_EVENT_TIMESTAMP_MS)

    def test_classify_input_device_covers_keyboards_pointers_and_exclusions(self) -> None:
        self.assertEqual(classify_input_device(InputDeviceInfo("/dev/input/event0", "Power")), "ignore")
        self.assertEqual(
            classify_input_device(
                InputDeviceInfo(
                    "/dev/input/event0",
                    "Virtual",
                    vendor=WAYLAND_UINPUT_VENDOR_ID,
                    product=WAYLAND_UINPUT_PRODUCT_ID,
                    ev_bits=frozenset({EV_KEY}),
                    key_bits=EVDEV_REQUIRED_KEYBOARD_KEYS,
                )
            ),
            "ignore",
        )
        self.assertEqual(
            classify_input_device(
                InputDeviceInfo(
                    "/dev/input/event0",
                    "Generic",
                    phys="keyswitch/input0",
                    ev_bits=frozenset({EV_KEY}),
                    key_bits=EVDEV_REQUIRED_KEYBOARD_KEYS,
                )
            ),
            "ignore",
        )
        self.assertEqual(
            classify_input_device(make_keyboard_info(name="YubiKey OTP+FIDO")),
            "ignore",
        )
        self.assertEqual(
            classify_input_device(
                InputDeviceInfo(
                    "/dev/input/event0",
                    "Tablet",
                    ev_bits=frozenset({EV_KEY}),
                    key_bits=EVDEV_REQUIRED_KEYBOARD_KEYS | {EVDEV_BTN_STYLUS},
                )
            ),
            "ignore",
        )
        self.assertEqual(
            classify_input_device(
                InputDeviceInfo(
                    "/dev/input/event0",
                    "Pad",
                    ev_bits=frozenset({EV_KEY}),
                    key_bits=EVDEV_REQUIRED_KEYBOARD_KEYS | {EVDEV_BTN_GAMEPAD},
                )
            ),
            "ignore",
        )
        self.assertEqual(classify_input_device(make_mouse_info()), "pointer")
        self.assertEqual(classify_input_device(make_keyboard_info()), "keyboard")
        touchpad = InputDeviceInfo(
            "/dev/input/event2",
            "Synaptics TouchPad",
            ev_bits=frozenset({EV_KEY, EV_ABS}),
            key_bits=EVDEV_REQUIRED_KEYBOARD_KEYS | {EVDEV_BTN_MOUSE},
            abs_bits=frozenset({EVDEV_ABS_MT_POSITION_X}),
        )
        self.assertEqual(classify_input_device(touchpad), "pointer")
        media_keys = InputDeviceInfo(
            "/dev/input/event3",
            "Headset Buttons",
            ev_bits=frozenset({EV_KEY}),
            key_bits=frozenset({EVDEV_KEY_Q}),
        )
        self.assertEqual(classify_input_device(media_keys), "ignore")


class WaylandInputHelperStateMachineTests(unittest.TestCase):
    def test_startup_failure_sends_error_and_tolerates_broken_pipe(self) -> None:
        channel = FakeByteChannel()
        os_api = FakeHelperOS()
        os_api.create_uinput_error = PermissionError("no /dev/uinput")
        helper = WaylandInputHelper(channel, os_api)
        self.assertEqual(helper.run(), 1)
        frames = channel.drain_frames()
        self.assertEqual(frames, [(HELPER_MSG_ERROR, b"no /dev/uinput")])

        channel.send_error = OSError("broken pipe")
        self.assertEqual(WaylandInputHelper(channel, os_api).run(), 1)

    def test_initial_permission_denial_and_device_open_fallbacks(self) -> None:
        channel = FakeByteChannel()
        os_api = FakeHelperOS()
        os_api.devices = [make_keyboard_info("/dev/input/event0")]
        os_api.open_results[("/dev/input/event0", True)] = PermissionError("ro")
        os_api.open_results[("/dev/input/event0", False)] = PermissionError("denied")
        self.assertEqual(WaylandInputHelper(channel, os_api).run(), 1)
        msg_type, payload = channel.drain_frames()[0]
        self.assertEqual(msg_type, HELPER_MSG_ERROR)
        self.assertIn(ENABLE_COMMAND, payload.decode("utf-8"))

        channel2 = FakeByteChannel()
        os_api2 = FakeHelperOS()
        os_api2.devices = [
            make_keyboard_info("/dev/input/event0"),
            make_keyboard_info("/dev/input/event1"),
            make_keyboard_info("/dev/input/event2"),
            make_keyboard_info("/dev/input/event3"),
            make_mouse_info("/dev/input/event4"),
            make_mouse_info("/dev/input/event5"),
            InputDeviceInfo("/dev/input/event6", "Power"),
        ]
        os_api2.open_results = {
            ("/dev/input/event0", True): PermissionError("no write"),
            ("/dev/input/event0", False): WL_FIXTURE_FIRST_KEYBOARD_FD,
            ("/dev/input/event1", True): PermissionError("no write"),
            ("/dev/input/event1", False): OSError("vanished"),
            ("/dev/input/event2", True): OSError("busy"),
            ("/dev/input/event3", True): WL_FIXTURE_SECOND_KEYBOARD_FD,
            ("/dev/input/event4", False): OSError("mouse vanished"),
            ("/dev/input/event5", False): WL_FIXTURE_POINTER_FD,
        }
        os_api2.grab_errors.add(WL_FIXTURE_SECOND_KEYBOARD_FD)
        os_api2.caps_lock[WL_FIXTURE_FIRST_KEYBOARD_FD] = True
        channel2.incoming.append(pack_frame(HELPER_CMD_STOP))
        os_api2.polls.append(((WL_FIXTURE_CHANNEL_FD, select.POLLIN),))
        helper2 = WaylandInputHelper(channel2, os_api2)
        self.assertEqual(helper2.run(), 0)
        self.assertEqual(channel2.drain_frames(), [(HELPER_MSG_READY, b"\x01")])

    def test_grab_waits_for_held_keys_with_settle_and_timeout(self) -> None:
        channel = FakeByteChannel()
        os_api = FakeHelperOS()
        helper = WaylandInputHelper(channel, os_api)
        os_api.keys_down[WL_FIXTURE_FIRST_KEYBOARD_FD] = deque([
            frozenset({EVDEV_KEY_A}),
            frozenset(),
        ])
        self.assertTrue(helper._grab_keyboard_safely(WL_FIXTURE_FIRST_KEYBOARD_FD))
        self.assertIn(WL_FIXTURE_FIRST_KEYBOARD_FD, os_api.grabbed)

        os_api.advance_on_sleep = WL_FIXTURE_ADVANCE_PAST_GRAB_WAIT_SECONDS
        os_api.keys_down[WL_FIXTURE_SECOND_KEYBOARD_FD] = frozenset({EVDEV_KEY_A})
        self.assertTrue(helper._grab_keyboard_safely(WL_FIXTURE_SECOND_KEYBOARD_FD))
        self.assertIn(WL_FIXTURE_SECOND_KEYBOARD_FD, os_api.grabbed)

    def test_verdict_flow_watchdog_passthrough_repeat_leds_and_commands(self) -> None:
        channel = FakeByteChannel()
        os_api = FakeHelperOS()
        os_api.devices = [
            make_keyboard_info("/dev/input/event0"),
            make_mouse_info("/dev/input/event1"),
        ]
        os_api.open_results = {
            ("/dev/input/event0", True): WL_FIXTURE_FIRST_KEYBOARD_FD,
            ("/dev/input/event1", False): WL_FIXTURE_POINTER_FD,
        }
        os_api.events[WL_FIXTURE_FIRST_KEYBOARD_FD] = deque()
        os_api.events[WL_FIXTURE_POINTER_FD] = deque()

        helper = WaylandInputHelper(channel, os_api, repeat_delay_ms=WL_FIXTURE_CUSTOM_REPEAT_DELAY_MS)
        os_api.uinput_fd = WL_FIXTURE_UINPUT_FD
        helper._uinput_fd = WL_FIXTURE_UINPUT_FD
        helper._inotify_fd = WL_FIXTURE_INOTIFY_FD
        helper._rescan_devices(initial=True)
        channel.drain_frames()

        # Step 1: keyboard press + non-key + unknown value (queued after initial grab flush).
        os_api.events[WL_FIXTURE_FIRST_KEYBOARD_FD].append((
            RawInputEvent(0, 0, EV_SYN, SYN_REPORT, 0),
            RawInputEvent(0, 0, EV_KEY, EVDEV_KEY_A, KEY_VALUE_PRESS),
            RawInputEvent(0, 0, EV_KEY, EVDEV_KEY_Q, KEY_VALUE_PRESS),
            RawInputEvent(0, 0, EV_KEY, EVDEV_KEY_A, WL_FIXTURE_UNKNOWN_KEY_VALUE),
        ))
        os_api.polls.append(((WL_FIXTURE_FIRST_KEYBOARD_FD, select.POLLIN),))

        # Step 2: channel POLLOUT (ignored), stale verdict, pass verdict for seq 2 (EVDEV_KEY_A),
        # swallow verdict for seq 3 (EVDEV_KEY_Q), unknown command, empty & non-empty emit_batch,
        # query_key_down True & False.
        os_api.polls.append(((WL_FIXTURE_CHANNEL_FD, WL_FIXTURE_POLLOUT_MASK),))
        os_api.keys_down[WL_FIXTURE_FIRST_KEYBOARD_FD] = frozenset({EVDEV_KEY_A})
        batch_two = (
            CMD_BATCH_HEADER_STRUCT.pack(WL_FIXTURE_REQ_ID_ONE, WL_FIXTURE_EMIT_COUNT_TWO, 0, 0)
            + CMD_BATCH_ITEM_STRUCT.pack(1, EVDEV_KEY_A)
            + CMD_BATCH_ITEM_STRUCT.pack(0, EVDEV_KEY_A)
        )
        batch_empty = CMD_BATCH_HEADER_STRUCT.pack(1, 0, 0, 0)
        channel.incoming.append(
            pack_frame(HELPER_CMD_VERDICT, CMD_VERDICT_STRUCT.pack(WL_FIXTURE_SEQ_STALE, HELPER_VERDICT_PASS))
            + pack_frame(HELPER_CMD_VERDICT, CMD_VERDICT_STRUCT.pack(WL_FIXTURE_REQ_ID_ONE, HELPER_VERDICT_PASS))
            + pack_frame(
                HELPER_CMD_VERDICT,
                CMD_VERDICT_STRUCT.pack(WL_FIXTURE_REQ_ID_ONE + 1, HELPER_VERDICT_SWALLOW),
            )
            + pack_frame(WL_FIXTURE_UNKNOWN_COMMAND_ID, b"")
            + pack_frame(HELPER_CMD_EMIT_BATCH, batch_empty)
            + pack_frame(HELPER_CMD_EMIT_BATCH, batch_two)
            + pack_frame(HELPER_CMD_QUERY_KEY_DOWN, CMD_QUERY_KEY_STRUCT.pack(1, EVDEV_KEY_A))
            + pack_frame(HELPER_CMD_QUERY_KEY_DOWN, CMD_QUERY_KEY_STRUCT.pack(WL_FIXTURE_REQ_ID_ONE, EVDEV_KEY_Q))
        )
        os_api.polls.append(((WL_FIXTURE_CHANNEL_FD, select.POLLIN),))

        # Step 3: next keyboard press + pass verdict.
        os_api.events[WL_FIXTURE_FIRST_KEYBOARD_FD].append((
            RawInputEvent(0, 0, EV_KEY, EVDEV_KEY_A, KEY_VALUE_PRESS),
        ))
        os_api.polls.append(((WL_FIXTURE_FIRST_KEYBOARD_FD, select.POLLIN),))
        channel.incoming.append(
            pack_frame(
                HELPER_CMD_VERDICT,
                CMD_VERDICT_STRUCT.pack(WL_FIXTURE_EXPECTED_FOUR_SEQ := WL_FIXTURE_REQ_ID_ONE + WL_FIXTURE_EMIT_COUNT_TWO, HELPER_VERDICT_PASS),
            )
        )
        os_api.polls.append(((WL_FIXTURE_CHANNEL_FD, select.POLLIN),))

        helper._step()
        helper._step()
        helper._step()
        helper._step()
        helper._step()
        self.assertEqual(WL_FIXTURE_EXPECTED_FOUR_SEQ, helper._next_seq)

        # Repeat before delay, modifier repeat, then repeat after delay (notifies once).
        os_api.now = WL_FIXTURE_SHORT_HOLD_SECONDS
        os_api.events[WL_FIXTURE_FIRST_KEYBOARD_FD].append((
            RawInputEvent(0, 0, EV_KEY, EVDEV_KEY_A, KEY_VALUE_REPEAT),
            RawInputEvent(0, 0, EV_KEY, EVDEV_KEY_LEFTSHIFT, KEY_VALUE_REPEAT),
        ))
        helper._read_keyboard(WL_FIXTURE_FIRST_KEYBOARD_FD)
        os_api.now = WL_FIXTURE_LONG_HOLD_SECONDS
        os_api.events[WL_FIXTURE_FIRST_KEYBOARD_FD].append((
            RawInputEvent(1, 0, EV_KEY, EVDEV_KEY_A, KEY_VALUE_REPEAT),
            RawInputEvent(1, 0, EV_KEY, EVDEV_KEY_A, KEY_VALUE_REPEAT),
            RawInputEvent(1, 0, EV_KEY, EVDEV_KEY_A, KEY_VALUE_RELEASE),
        ))
        helper._read_keyboard(WL_FIXTURE_FIRST_KEYBOARD_FD)
        frame_types = [msg_type for msg_type, _ in channel.drain_frames()]
        self.assertEqual(frame_types.count(HELPER_MSG_REPEAT_RESET), 1)
        # Answer the release key so queue is clean.
        helper._apply_verdict(helper._next_seq, False)

        # Watchdog timeout: enqueue two keys, advance past deadline -> both pass through,
        # and a third key arriving while stalled passes through immediately until late verdict arrives.
        os_api.events[WL_FIXTURE_FIRST_KEYBOARD_FD].append((
            RawInputEvent(1, 0, EV_KEY, EVDEV_KEY_A, KEY_VALUE_PRESS),
            RawInputEvent(1, 0, EV_KEY, EVDEV_KEY_A, KEY_VALUE_RELEASE),
        ))
        helper._read_keyboard(WL_FIXTURE_FIRST_KEYBOARD_FD)
        timed_seq = helper._next_seq
        os_api.now += WL_FIXTURE_ADVANCE_PAST_VERDICT_SECONDS
        helper._expire_pending_if_due()
        self.assertEqual(helper._forced_seq, timed_seq)
        os_api.events[WL_FIXTURE_FIRST_KEYBOARD_FD].append((
            RawInputEvent(1, 0, EV_KEY, EVDEV_KEY_Q, KEY_VALUE_PRESS),
        ))
        helper._read_keyboard(WL_FIXTURE_FIRST_KEYBOARD_FD)
        # The late verdict changes nothing; the parent's acknowledgement ends the pass-through.
        helper._apply_verdict(timed_seq, True)
        self.assertEqual(helper._forced_seq, timed_seq)
        helper._handle_command(HELPER_CMD_SEQ_ACK, U32_STRUCT.pack(timed_seq + 1))
        self.assertEqual(helper._forced_seq, timed_seq)
        helper._handle_command(HELPER_CMD_SEQ_ACK, U32_STRUCT.pack(timed_seq))
        self.assertIsNone(helper._forced_seq)

        # Pointer events, uinput LED forwarding, inotify rescan, and SET_GRAB disable/re-enable.
        os_api.events[WL_FIXTURE_POINTER_FD].append((
            RawInputEvent(1, 0, EV_REL, EVDEV_REL_X, 1),
            RawInputEvent(1, 0, EV_KEY, EVDEV_BTN_MOUSE, KEY_VALUE_PRESS),
        ))
        os_api.uinput_leds.append(((LED_NUML, 1), (LED_CAPSL, 1), (LED_CAPSL, 1)))
        os_api.inotify_batches.append(("mouse0",))
        os_api.polls.append((
            (WL_FIXTURE_POINTER_FD, select.POLLIN),
            (WL_FIXTURE_UINPUT_FD, select.POLLIN),
            (WL_FIXTURE_INOTIFY_FD, select.POLLIN),
        ))
        helper._step()
        emitted_types = [msg_type for msg_type, _ in channel.drain_frames()]
        self.assertIn(HELPER_MSG_POINTER, emitted_types)
        self.assertIn(HELPER_MSG_CAPS_LOCK, emitted_types)

        # LED read error & write error tolerated.
        os_api.uinput_leds.append(OSError("uinput led error"))
        helper._forward_uinput_leds(WL_FIXTURE_UINPUT_FD)
        os_api.led_errors.add(WL_FIXTURE_FIRST_KEYBOARD_FD)
        os_api.uinput_leds.append(((LED_NUML, 0),))
        helper._forward_uinput_leds(WL_FIXTURE_UINPUT_FD)

        # SET_GRAB no-op, disable (ungrab error tolerated, pointer & keyboard ignored while ungrabbed),
        # then re-enable with a newly hotplugged keyboard alongside the existing one.
        helper._handle_command(HELPER_CMD_SET_GRAB, CMD_SET_GRAB_STRUCT.pack(1, 1, 0))
        os_api.ungrab_errors.add(WL_FIXTURE_FIRST_KEYBOARD_FD)
        helper._handle_command(
            HELPER_CMD_SET_GRAB,
            CMD_SET_GRAB_STRUCT.pack(WL_FIXTURE_REQ_ID_ONE, 0, WL_FIXTURE_CUSTOM_REPEAT_DELAY_MS),
        )
        os_api.events[WL_FIXTURE_FIRST_KEYBOARD_FD].append((
            RawInputEvent(1, 0, EV_KEY, EVDEV_KEY_A, KEY_VALUE_PRESS),
        ))
        helper._read_keyboard(WL_FIXTURE_FIRST_KEYBOARD_FD)
        os_api.events[WL_FIXTURE_POINTER_FD].append((
            RawInputEvent(1, 0, EV_KEY, EVDEV_BTN_MOUSE, KEY_VALUE_PRESS),
        ))
        helper._read_pointer(WL_FIXTURE_POINTER_FD)

        os_api.devices.append(make_keyboard_info("/dev/input/event2", "External USB Keyboard"))
        os_api.open_results[("/dev/input/event2", True)] = WL_FIXTURE_SECOND_KEYBOARD_FD
        helper._handle_command(HELPER_CMD_SET_GRAB, CMD_SET_GRAB_STRUCT.pack(1, 1, 0))
        self.assertEqual(os_api.grabbed, {WL_FIXTURE_FIRST_KEYBOARD_FD, WL_FIXTURE_SECOND_KEYBOARD_FD})
        helper._cleanup()

    def test_hotplug_removal_orphan_release_and_error_cleanup_paths(self) -> None:
        channel = FakeByteChannel()
        os_api = FakeHelperOS()
        os_api.devices = [
            make_keyboard_info("/dev/input/event0"),
            make_keyboard_info("/dev/input/event1"),
            make_mouse_info("/dev/input/event2"),
        ]
        os_api.open_results = {
            ("/dev/input/event0", True): WL_FIXTURE_FIRST_KEYBOARD_FD,
            ("/dev/input/event1", True): WL_FIXTURE_SECOND_KEYBOARD_FD,
            ("/dev/input/event2", False): WL_FIXTURE_POINTER_FD,
        }
        helper = WaylandInputHelper(channel, os_api)
        helper._uinput_fd = None
        helper._emit_physical_key(True, EVDEV_KEY_A)
        helper._uinput_fd = WL_FIXTURE_UINPUT_FD
        helper._inotify_fd = WL_FIXTURE_INOTIFY_FD
        helper._rescan_devices(initial=True)

        # Both keyboards hold A; keyboard 0 also holds Q. Removing keyboard 0 releases Q, keeps A.
        helper._passed_keys = {EVDEV_KEY_A, EVDEV_KEY_Q}
        helper._device_keys_down[WL_FIXTURE_FIRST_KEYBOARD_FD] = {EVDEV_KEY_A, EVDEV_KEY_Q}
        helper._device_keys_down[WL_FIXTURE_SECOND_KEYBOARD_FD] = {EVDEV_KEY_A}
        os_api.devices = [make_keyboard_info("/dev/input/event1")]
        os_api.inotify_batches.append(("event0",))
        os_api.polls.append(((WL_FIXTURE_INOTIFY_FD, select.POLLIN),))
        helper._step()
        self.assertIn(EVDEV_KEY_A, helper._passed_keys)
        self.assertNotIn(EVDEV_KEY_Q, helper._passed_keys)

        # query_keys_down OSError closes keyboard 1 and releases orphaned A.
        os_api.keys_down[WL_FIXTURE_SECOND_KEYBOARD_FD] = deque([OSError("unplugged")])
        self.assertFalse(helper._is_key_physically_down(EVDEV_KEY_A))
        self.assertNotIn(EVDEV_KEY_A, helper._passed_keys)

        # Re-add keyboard + pointer, then exercise POLLHUP and read OSError on both, plus channel errors.
        os_api.devices = [
            make_keyboard_info("/dev/input/event3"),
            make_mouse_info("/dev/input/event4"),
        ]
        os_api.open_results[("/dev/input/event3", True)] = WL_FIXTURE_THIRD_KEYBOARD_FD
        os_api.open_results[("/dev/input/event4", False)] = WL_FIXTURE_POINTER_FD
        helper._grab_enabled = False
        helper._rescan_devices(initial=False)
        helper._grab_enabled = True
        os_api.events[WL_FIXTURE_THIRD_KEYBOARD_FD] = deque([OSError("kbd read error")])
        os_api.events[WL_FIXTURE_POINTER_FD] = deque([OSError("ptr read error")])
        os_api.close_errors.update({WL_FIXTURE_THIRD_KEYBOARD_FD, WL_FIXTURE_POINTER_FD, WL_FIXTURE_INOTIFY_FD})
        helper._read_keyboard(WL_FIXTURE_THIRD_KEYBOARD_FD)
        helper._read_pointer(WL_FIXTURE_POINTER_FD)

        helper._keyboards[WL_FIXTURE_FIRST_KEYBOARD_FD] = "/dev/input/event0"
        helper._pointers[WL_FIXTURE_POINTER_FD] = "/dev/input/event2"
        os_api.polls.append((
            (WL_FIXTURE_FIRST_KEYBOARD_FD, select.POLLHUP),
            (WL_FIXTURE_POINTER_FD, select.POLLHUP),
        ))
        helper._step()

        self.assertFalse(helper._read_channel(select.POLLIN))
        channel.incoming.append(OSError("recv failed"))
        self.assertFalse(helper._read_channel(select.POLLIN))
        self.assertFalse(helper._read_channel(select.POLLHUP))

        helper._passed_keys.add(EVDEV_KEY_A)
        os_api.write_uinput_error = OSError("uinput write failed")
        os_api.destroy_uinput_error = OSError("uinput destroy failed")
        helper.stop()
        helper._cleanup()


class HelperClientProtocolTests(unittest.TestCase):
    def test_client_two_thread_reentrant_requests_and_callbacks(self) -> None:
        parent_sock, child_sock = socket.socketpair()
        os_api = FakeHelperOS()
        os_api.devices = [
            make_keyboard_info("/dev/input/event0"),
            make_mouse_info("/dev/input/event1"),
        ]
        os_api.open_results = {
            ("/dev/input/event0", True): WL_FIXTURE_FIRST_KEYBOARD_FD,
            ("/dev/input/event1", False): WL_FIXTURE_POINTER_FD,
        }
        os_api.keys_down[WL_FIXTURE_FIRST_KEYBOARD_FD] = frozenset({EVDEV_KEY_LEFTSHIFT})
        os_api.caps_lock[WL_FIXTURE_FIRST_KEYBOARD_FD] = True

        pointers: list[int] = []
        repeats: list[int] = []
        caps_updates: list[bool] = []
        reentrant_results: list[tuple[bool, int]] = []
        key_handled = threading.Event()
        acknowledged: list[int] = []
        resets: list[bool] = []
        unasked: list[HelperKey] = []

        client_ref: list[HelperClient] = []
        # The rest follows once the client has started: sent with the ready message, its caps-lock update could be read
        # before start() returned the state the ready message gave (a CI run of 29.10.2026).
        started = threading.Event()

        def on_key(key: HelperKey) -> bool:
            if key.pressed and key.keycode == EVDEV_KEY_A:
                down = client_ref[0].query_key_down(EVDEV_KEY_LEFTSHIFT)
                emitted = client_ref[0].emit_batch(((True, EVDEV_KEY_Q), (False, EVDEV_KEY_Q)))
                reentrant_results.append((down, emitted))
                key_handled.set()
                return True
            if not key.needs_verdict:
                unasked.append(key)
                return False
            raise RuntimeError("callback failure defaults to pass")

        def run_child() -> None:
            try:
                child_reader = FrameReader()
                child_sock.sendall(pack_frame(HELPER_MSG_READY, U8_STRUCT.pack(1)))
                started.wait(KEYBOARD_LISTENER_START_TIMEOUT_SECONDS)
                child_sock.sendall(
                    pack_frame(HELPER_MSG_KEY, MSG_KEY_STRUCT.pack(1, 1, EVDEV_KEY_A, 1, HELPER_KEY_NEEDS_VERDICT))
                    + pack_frame(
                        HELPER_MSG_KEY,
                        MSG_KEY_STRUCT.pack(WL_FIXTURE_REQ_ID_ONE, 0, EVDEV_KEY_A, 1, HELPER_KEY_NEEDS_VERDICT),
                    )
                    + pack_frame(
                        HELPER_MSG_KEY,
                        MSG_KEY_STRUCT.pack(WL_FIXTURE_REQ_ID_ONE, 0, EVDEV_KEY_A, 1, HELPER_KEY_FORCED),
                    )
                    + pack_frame(HELPER_MSG_KEY, MSG_KEY_STRUCT.pack(WL_FIXTURE_REQ_ID_UNKNOWN, 0, EVDEV_KEY_Q, 1, 0))
                    + pack_frame(HELPER_MSG_INPUT_RESET)
                    + pack_frame(HELPER_MSG_POINTER, U32_STRUCT.pack(1))
                    + pack_frame(HELPER_MSG_REPEAT_RESET, U32_STRUCT.pack(1))
                    + pack_frame(HELPER_MSG_CAPS_LOCK, U8_STRUCT.pack(0))
                    + pack_frame(WL_FIXTURE_UNKNOWN_MESSAGE_ID, b"")
                    + pack_frame(
                        HELPER_MSG_EMIT_ACK,
                        MSG_ACK_STRUCT.pack(WL_FIXTURE_REQ_ID_UNKNOWN, 0),
                    )
                )
                while True:
                    chunk = child_sock.recv(HELPER_MAX_FRAME_BYTES)
                    if not chunk:
                        break
                    for msg_type, payload in child_reader.feed(chunk):
                        if msg_type == HELPER_CMD_STOP:
                            return
                        if msg_type == HELPER_CMD_QUERY_KEY_DOWN:
                            req_id, _code = CMD_QUERY_KEY_STRUCT.unpack(payload)
                            child_sock.sendall(
                                pack_frame(HELPER_MSG_KEY_DOWN, MSG_KEY_DOWN_STRUCT.pack(req_id, 1))
                            )
                        elif msg_type == HELPER_CMD_SEQ_ACK:
                            acknowledged.append(U32_STRUCT.unpack(payload)[0])
                        elif msg_type == HELPER_CMD_EMIT_BATCH:
                            req_id, count, _checked, _known = CMD_BATCH_HEADER_STRUCT.unpack_from(payload, 0)
                            child_sock.sendall(
                                pack_frame(HELPER_MSG_EMIT_ACK, MSG_ACK_STRUCT.pack(req_id, count))
                            )
                        elif msg_type == HELPER_CMD_SET_GRAB:
                            req_id, enabled_u8, _delay = CMD_SET_GRAB_STRUCT.unpack(payload)
                            child_sock.sendall(
                                pack_frame(
                                    HELPER_MSG_EMIT_ACK,
                                    MSG_ACK_STRUCT.pack(req_id, enabled_u8),
                                )
                            )
            finally:
                child_sock.close()

        thread = threading.Thread(target=run_child, daemon=True)
        thread.start()
        client = HelperClient(
            parent_sock,
            on_key=on_key,
            on_pointer=pointers.append,
            on_repeat_reset=repeats.append,
            on_caps_lock=caps_updates.append,
            on_input_reset=lambda: resets.append(True),
        )
        client_ref.append(client)
        self.assertTrue(client.start())
        self.assertTrue(client.caps_lock)
        started.set()
        self.assertEqual(client.emit_batch(()), 0)
        self.assertTrue(key_handled.wait(KEYBOARD_LISTENER_START_TIMEOUT_SECONDS))
        self.assertEqual(reentrant_results, [(True, WL_FIXTURE_EMIT_COUNT_TWO)])
        self.assertTrue(client.set_grab(True))
        client.close()
        client.close()
        thread.join(timeout=1.0)
        self.assertEqual(pointers, [1])
        self.assertEqual(repeats, [1])
        self.assertEqual(caps_updates, [False])
        self.assertFalse(client.caps_lock)
        # A forced key is acknowledged once handled; a key typed unasked gets no answer at all.
        self.assertEqual(acknowledged, [WL_FIXTURE_REQ_ID_ONE])
        self.assertEqual([(key.seq, key.forced) for key in unasked], [
            (WL_FIXTURE_REQ_ID_ONE, True), (WL_FIXTURE_REQ_ID_UNKNOWN, False),
        ])
        self.assertEqual(resets, [True])
        self.assertTrue(client.closed)

    def test_client_start_failures_request_timeout_and_broken_verdict_send(self) -> None:
        # 1. Start timeout.
        s1_parent, s1_child = socket.socketpair()
        client1 = HelperClient(
            s1_parent,
            on_key=lambda *_a: False,
            on_pointer=lambda _t: None,
            on_repeat_reset=lambda _t: None,
            on_caps_lock=lambda _c: None,
            on_input_reset=lambda: None,
        )
        try:
            with self.assertRaisesRegex(RuntimeError, "не ответил вовремя"):
                client1.start(timeout=WL_FIXTURE_SHORT_TIMEOUT_SECONDS)
        finally:
            s1_child.close()
            client1.close()

        # 2. Startup error frame.
        s2_parent, s2_child = socket.socketpair()
        s2_child.sendall(pack_frame(HELPER_MSG_ERROR, "нет прав udev".encode("utf-8")))
        s2_child.close()
        client2 = HelperClient(
            s2_parent,
            on_key=lambda *_a: False,
            on_pointer=lambda _t: None,
            on_repeat_reset=lambda _t: None,
            on_caps_lock=lambda _c: None,
            on_input_reset=lambda: None,
        )
        try:
            with self.assertRaisesRegex(RuntimeError, "нет прав udev"):
                client2.start()
        finally:
            client2.close()

        # 3. Immediate EOF before READY.
        s3_parent, s3_child = socket.socketpair()
        s3_child.close()
        client3 = HelperClient(
            s3_parent,
            on_key=lambda *_a: False,
            on_pointer=lambda _t: None,
            on_repeat_reset=lambda _t: None,
            on_caps_lock=lambda _c: None,
            on_input_reset=lambda: None,
        )
        try:
            with self.assertRaisesRegex(RuntimeError, "завершился при запуске"):
                client3.start()
        finally:
            client3.close()

        # 4a. Request failure when helper already closed (sendall OSError) + broken verdict send.
        s4_parent, s4_child = socket.socketpair()
        s4_child.sendall(
            pack_frame(HELPER_MSG_READY, b"\x00")
            + pack_frame(HELPER_MSG_KEY, MSG_KEY_STRUCT.pack(1, 1, EVDEV_KEY_A, 1, HELPER_KEY_NEEDS_VERDICT))
        )
        gate = threading.Event()

        def slow_key(*_args: object) -> bool:
            gate.wait(1.0)
            return False

        client4 = HelperClient(
            s4_parent,
            on_key=slow_key,
            on_pointer=lambda _t: None,
            on_repeat_reset=lambda _t: None,
            on_caps_lock=lambda _c: None,
            on_input_reset=lambda: None,
        )
        try:
            client4.start()
            s4_child.close()
            s4_parent.close()
            with self.assertRaisesRegex(RuntimeError, "Нет ответа"):
                client4.query_key_down(EVDEV_KEY_A)
        finally:
            gate.set()
            client4.close()

        # 4b. Request failure when helper disconnects after sendall while waiting on reply slot.
        s5_parent, s5_child = socket.socketpair()
        s5_child.sendall(pack_frame(HELPER_MSG_READY, b"\x00"))
        client5 = HelperClient(
            s5_parent,
            on_key=lambda *_a: False,
            on_pointer=lambda _t: None,
            on_repeat_reset=lambda _t: None,
            on_caps_lock=lambda _c: None,
            on_input_reset=lambda: None,
        )

        def close_peer_soon() -> None:
            _ = s5_child.recv(HELPER_SOCKET_CHUNK_BYTES)
            s5_child.close()

        closer = threading.Thread(target=close_peer_soon, daemon=True)
        try:
            client5.start()
            closer.start()
            with self.assertRaisesRegex(RuntimeError, "Нет ответа"):
                client5.query_key_down(EVDEV_KEY_A)
        finally:
            closer.join(timeout=1.0)
            s5_child.close()
            client5.close()

        # Also cover FakeByteChannel close/send OSError and pre-closed _read_loop.
        err_chan = FakeByteChannel()
        err_chan.send_error = OSError("send err")
        err_chan.close_error = OSError("close err")
        client6 = HelperClient(
            err_chan,
            on_key=lambda *_a: False,
            on_pointer=lambda _t: None,
            on_repeat_reset=lambda _t: None,
            on_caps_lock=lambda _c: None,
            on_input_reset=lambda: None,
        )
        client6.close()
        client6._read_loop()


class HelperLateVerdictAndInjectionTests(unittest.TestCase):
    """The helper never hides a key from the parent, and never types a correction over one it hid."""

    def make_helper(self) -> tuple[WaylandInputHelper, FakeByteChannel, FakeHelperOS]:
        channel = FakeByteChannel()
        os_api = FakeHelperOS()
        os_api.devices = [make_keyboard_info("/dev/input/event0")]
        os_api.open_results = {("/dev/input/event0", True): WL_FIXTURE_FIRST_KEYBOARD_FD}
        os_api.events[WL_FIXTURE_FIRST_KEYBOARD_FD] = deque()
        helper = WaylandInputHelper(channel, os_api)
        helper._uinput_fd = WL_FIXTURE_UINPUT_FD
        helper._inotify_fd = WL_FIXTURE_INOTIFY_FD
        helper._rescan_devices(initial=True)
        channel.drain_frames()
        os_api.uinput_writes.clear()
        return helper, channel, os_api

    @staticmethod
    def keys(channel: FakeByteChannel) -> list[tuple[int, bool, int, int]]:
        result = []
        for msg_type, payload in channel.drain_frames():
            if msg_type == HELPER_MSG_KEY:
                seq, pressed, code, _ts, flags = MSG_KEY_STRUCT.unpack(payload)
                result.append((seq, bool(pressed), code, flags))
        return result

    @staticmethod
    def batch(
        req_id: int, items: Sequence[tuple[bool, int]], known_seq: int | None = None, check: int = HELPER_EMIT_TEXT,
    ) -> bytes:
        header = CMD_BATCH_HEADER_STRUCT.pack(
            req_id, len(items), HELPER_EMIT_UNCHECKED if known_seq is None else check, known_seq or 0,
        )
        return header + b"".join(CMD_BATCH_ITEM_STRUCT.pack(1 if pressed else 0, code) for pressed, code in items)

    @staticmethod
    def acks(channel: FakeByteChannel) -> list[tuple[int, int]]:
        return [MSG_ACK_STRUCT.unpack(payload) for msg_type, payload in channel.drain_frames()
                if msg_type == HELPER_MSG_EMIT_ACK]

    def test_late_verdict_reports_the_typed_keys_and_refuses_a_correction_decided_before_them(self) -> None:
        helper, channel, os_api = self.make_helper()
        os_api.events[WL_FIXTURE_FIRST_KEYBOARD_FD].append((
            RawInputEvent(0, 0, EV_KEY, EVDEV_KEY_A, KEY_VALUE_PRESS),
            RawInputEvent(0, 0, EV_KEY, EVDEV_KEY_Q, KEY_VALUE_PRESS),
        ))
        helper._read_keyboard(WL_FIXTURE_FIRST_KEYBOARD_FD)
        [(asked, _pressed, _code, flags)] = self.keys(channel)
        self.assertEqual(flags, HELPER_KEY_NEEDS_VERDICT)

        os_api.now += WL_FIXTURE_ADVANCE_PAST_VERDICT_SECONDS
        helper._expire_pending_if_due()
        # The asked key comes back marked forced, then the queued one, typed without asking.
        forced, unasked = self.keys(channel)
        self.assertEqual(forced, (asked, True, EVDEV_KEY_A, HELPER_KEY_FORCED))
        self.assertEqual(unasked[1:], (True, EVDEV_KEY_Q, 0))
        self.assertTrue(seq_newer(unasked[0], asked))
        typed = [event[1] for write in os_api.uinput_writes for event in write if event[0] == EV_KEY]
        self.assertEqual(typed, [EVDEV_KEY_A, EVDEV_KEY_Q])

        # Until the parent acknowledges, every key is typed at once and reported.
        os_api.events[WL_FIXTURE_FIRST_KEYBOARD_FD].append((
            RawInputEvent(0, 0, EV_KEY, EVDEV_KEY_A, KEY_VALUE_RELEASE),
        ))
        helper._read_keyboard(WL_FIXTURE_FIRST_KEYBOARD_FD)
        [(latest, _pressed, _code, flags)] = self.keys(channel)
        self.assertEqual(flags, 0)
        self.assertIsNone(helper._pending)

        os_api.uinput_writes.clear()
        correction = ((True, EVDEV_KEY_LEFTSHIFT), (False, EVDEV_KEY_LEFTSHIFT))
        helper._handle_command(HELPER_CMD_EMIT_BATCH, self.batch(WL_FIXTURE_REQ_ID_ONE, correction, asked))
        self.assertEqual(self.acks(channel), [(WL_FIXTURE_REQ_ID_ONE, 0)])
        helper._handle_command(HELPER_CMD_SEQ_ACK, U32_STRUCT.pack(asked))
        # Acknowledged, but the parent decided before the keys typed unasked: still refused.
        helper._handle_command(HELPER_CMD_EMIT_BATCH, self.batch(WL_FIXTURE_REQ_ID_ONE, correction, asked))
        self.assertEqual(self.acks(channel), [(WL_FIXTURE_REQ_ID_ONE, 0)])
        self.assertEqual(os_api.uinput_writes, [])
        # Decided knowing them, or a replay that is not checked: typed.
        helper._handle_command(HELPER_CMD_EMIT_BATCH, self.batch(WL_FIXTURE_REQ_ID_ONE, correction, latest))
        helper._handle_command(HELPER_CMD_EMIT_BATCH, self.batch(WL_FIXTURE_REQ_ID_ONE + 1, correction))
        self.assertEqual(self.acks(channel), [
            (WL_FIXTURE_REQ_ID_ONE, WL_FIXTURE_EMIT_COUNT_TWO),
            (WL_FIXTURE_REQ_ID_ONE + 1, WL_FIXTURE_EMIT_COUNT_TWO),
        ])

        # Back to asking.
        os_api.events[WL_FIXTURE_FIRST_KEYBOARD_FD].append((
            RawInputEvent(0, 0, EV_KEY, EVDEV_KEY_Q, KEY_VALUE_RELEASE),
        ))
        helper._read_keyboard(WL_FIXTURE_FIRST_KEYBOARD_FD)
        [(_seq, _pressed, _code, flags)] = self.keys(channel)
        self.assertEqual(flags, HELPER_KEY_NEEDS_VERDICT)

    def test_a_withheld_key_is_typed_once_whoever_is_first(self) -> None:
        helper, channel, os_api = self.make_helper()
        os_api.events[WL_FIXTURE_FIRST_KEYBOARD_FD].append((
            RawInputEvent(0, 0, EV_KEY, EVDEV_KEY_A, KEY_VALUE_PRESS),
            RawInputEvent(0, 0, EV_KEY, EVDEV_KEY_Q, KEY_VALUE_PRESS),
        ))
        helper._read_keyboard(WL_FIXTURE_FIRST_KEYBOARD_FD)
        [(asked, _pressed, _code, _flags)] = self.keys(channel)
        action = ((True, EVDEV_KEY_A), (False, EVDEV_KEY_A))
        # The parent types the key it still owes an answer for: the wait for it ends, the next key is asked.
        helper._handle_command(HELPER_CMD_EMIT_BATCH, self.batch(WL_FIXTURE_REQ_ID_ONE, action, asked, HELPER_EMIT_ACTION))
        frames = channel.drain_frames()
        self.assertIn((HELPER_MSG_EMIT_ACK, MSG_ACK_STRUCT.pack(WL_FIXTURE_REQ_ID_ONE, WL_FIXTURE_EMIT_COUNT_TWO)), frames)
        [next_key] = [MSG_KEY_STRUCT.unpack(payload) for msg_type, payload in frames if msg_type == HELPER_MSG_KEY]
        _seq, _pressed, next_code, _time, _flags = next_key
        self.assertEqual(next_code, EVDEV_KEY_Q)
        self.assertIsNotNone(helper._pending)
        # Typed by the helper when its time ran out: the parent's own typing of it is refused.
        os_api.now += WL_FIXTURE_ADVANCE_PAST_VERDICT_SECONDS
        helper._expire_pending_if_due()
        forced = helper._forced_seq
        assert forced is not None
        self.keys(channel)
        helper._handle_command(HELPER_CMD_EMIT_BATCH, self.batch(WL_FIXTURE_REQ_ID_ONE, action, forced, HELPER_EMIT_ACTION))
        self.assertEqual(self.acks(channel), [(WL_FIXTURE_REQ_ID_ONE, 0)])
        helper._handle_command(HELPER_CMD_SEQ_ACK, U32_STRUCT.pack(forced))
        helper._handle_command(HELPER_CMD_EMIT_BATCH, self.batch(WL_FIXTURE_REQ_ID_ONE, action, forced, HELPER_EMIT_ACTION))
        self.assertEqual(self.acks(channel), [(WL_FIXTURE_REQ_ID_ONE, 0)])
        # A later key is no reason to refuse an earlier one... once the helper is asking again.
        helper._handle_command(HELPER_CMD_EMIT_BATCH, self.batch(WL_FIXTURE_REQ_ID_ONE, action, forced + 1, HELPER_EMIT_ACTION))
        self.assertEqual(self.acks(channel), [(WL_FIXTURE_REQ_ID_ONE, WL_FIXTURE_EMIT_COUNT_TWO)])
        self.assertFalse(helper._refuses(HELPER_EMIT_UNCHECKED, 0))

    def test_a_key_held_down_is_released_before_the_correction_types_it(self) -> None:
        helper, channel, os_api = self.make_helper()
        helper._passed_keys.add(EVDEV_KEY_A)
        helper._handle_command(
            HELPER_CMD_EMIT_BATCH, self.batch(WL_FIXTURE_REQ_ID_ONE, ((True, EVDEV_KEY_A), (False, EVDEV_KEY_A))),
        )
        keys = [(code, value) for write in os_api.uinput_writes for ev_type, code, value in write if ev_type == EV_KEY]
        # The kernel drops a second press of a key that is down: it is released first.
        self.assertEqual(keys, [
            (EVDEV_KEY_A, KEY_VALUE_RELEASE), (EVDEV_KEY_A, KEY_VALUE_PRESS), (EVDEV_KEY_A, KEY_VALUE_RELEASE),
        ])
        self.assertEqual(self.acks(channel), [(WL_FIXTURE_REQ_ID_ONE, WL_FIXTURE_EMIT_COUNT_TWO)])

    def test_a_long_injection_is_written_in_bursts_the_event_queue_holds(self) -> None:
        helper, channel, os_api = self.make_helper()
        taps = tuple(item for _ in range(WL_FIXTURE_LONG_BATCH_TAPS) for item in ((True, EVDEV_KEY_A), (False, EVDEV_KEY_A)))
        helper._handle_command(HELPER_CMD_EMIT_BATCH, self.batch(WL_FIXTURE_REQ_ID_ONE, taps))
        burst = WAYLAND_UINPUT_BURST_TRANSITIONS * TRANSITION_EVENTS
        self.assertGreater(len(os_api.uinput_writes), 1)
        self.assertTrue(all(len(write) <= burst for write in os_api.uinput_writes))
        self.assertEqual(sum(len(write) for write in os_api.uinput_writes), len(taps) * TRANSITION_EVENTS)
        self.assertEqual(os_api.sleeps, [WAYLAND_UINPUT_BURST_PAUSE_SECONDS] * (len(os_api.uinput_writes) - 1))
        self.assertEqual(self.acks(channel), [(WL_FIXTURE_REQ_ID_ONE, len(taps))])

    def test_a_grab_change_tells_the_parent_to_forget_held_keys_and_ends_a_pass_through(self) -> None:
        helper, channel, os_api = self.make_helper()
        helper._forced_seq = WL_FIXTURE_REQ_ID_ONE
        helper._handle_command(HELPER_CMD_SET_GRAB, CMD_SET_GRAB_STRUCT.pack(WL_FIXTURE_REQ_ID_ONE, 0, 0))
        self.assertIsNone(helper._forced_seq)
        self.assertIn(HELPER_MSG_INPUT_RESET, [msg_type for msg_type, _payload in channel.drain_frames()])
        helper._handle_command(HELPER_CMD_SET_GRAB, CMD_SET_GRAB_STRUCT.pack(WL_FIXTURE_REQ_ID_ONE, 1, 0))
        self.assertIn(HELPER_MSG_INPUT_RESET, [msg_type for msg_type, _payload in channel.drain_frames()])

    def test_key_numbers_compare_across_the_wrap(self) -> None:
        self.assertTrue(seq_newer(WL_FIXTURE_SEQ_AFTER_WRAP, WL_FIXTURE_SEQ_BEFORE_WRAP))
        self.assertFalse(seq_newer(WL_FIXTURE_SEQ_BEFORE_WRAP, WL_FIXTURE_SEQ_AFTER_WRAP))
        self.assertFalse(seq_newer(WL_FIXTURE_SEQ_AFTER_WRAP, WL_FIXTURE_SEQ_AFTER_WRAP))
        self.assertFalse(seq_newer(0, WL_FIXTURE_SEQ_AFTER_WRAP))


if __name__ == "__main__":
    unittest.main()
