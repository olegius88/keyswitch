"""Isolated Wayland evdev/uinput helper protocol, device classifier and state machine.

The helper runs in its own child process so an unresponsive GTK main loop or a crash
in the parent can never freeze physical keyboards: if the parent does not answer a
key within ``WAYLAND_HELPER_VERDICT_TIMEOUT_MS``, the helper passes the key through
to ``/dev/uinput`` and switches into zero-latency pass-through until the parent catches
up; if the socket closes, ``EVIOCGRAB`` is released immediately.
"""

from __future__ import annotations

from collections import deque
from collections.abc import Callable, Sequence
from dataclasses import dataclass
import queue
import select
import struct
import threading
from typing import Literal, Protocol

from .constants.timing import (
    KEYBOARD_LISTENER_START_TIMEOUT_SECONDS,
    KEYBOARD_LISTENER_STOP_TIMEOUT_SECONDS,
    WAYLAND_GRAB_POLL_SECONDS,
    WAYLAND_GRAB_RELEASE_SETTLE_SECONDS,
    WAYLAND_GRAB_WAIT_TIMEOUT_SECONDS,
    WAYLAND_HELPER_COMMAND_TIMEOUT_SECONDS,
    WAYLAND_HELPER_POLL_TIMEOUT_MS,
    WAYLAND_HELPER_VERDICT_TIMEOUT_MS,
    WAYLAND_REPEAT_DEFAULT_DELAY_MS,
    WAYLAND_UINPUT_BURST_PAUSE_SECONDS,
    WAYLAND_UINPUT_SETTLE_SECONDS,
)
from .constants.units import (
    MICROSECONDS_PER_MILLISECOND,
    MILLISECONDS_PER_SECOND,
)
from .constants.wayland import (
    EV_ABS,
    EV_KEY,
    EV_REL,
    EV_SYN,
    EVDEV_ABS_MT_POSITION_X,
    EVDEV_BTN_GAMEPAD,
    EVDEV_BTN_MOUSE,
    EVDEV_BTN_STYLUS,
    EVDEV_BTN_TASK,
    EVDEV_KEY_CAPSLOCK,
    EVDEV_MODIFIER_KEYS,
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
    HELPER_EMIT_ITEM_BYTES,
    HELPER_EMIT_TEXT,
    HELPER_EMIT_UNCHECKED,
    HELPER_FRAME_HEADER_BYTES,
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
    SYN_REPORT,
    TRANSITION_EVENTS,
    UINT32_MODULO_MASK,
    WAYLAND_UINPUT_BURST_TRANSITIONS,
    WAYLAND_UINPUT_PRODUCT_ID,
    WAYLAND_UINPUT_VENDOR_ID,
)

FRAME_HEADER = struct.Struct("!BI")
U8_STRUCT = struct.Struct("!B")
U32_STRUCT = struct.Struct("!I")
# seq, pressed, keycode, time, flags (HELPER_KEY_*)
MSG_KEY_STRUCT = struct.Struct("!IBHIB")
MSG_ACK_STRUCT = struct.Struct("!II")
MSG_KEY_DOWN_STRUCT = struct.Struct("!IB")
CMD_VERDICT_STRUCT = struct.Struct("!IB")
# request, count, check (HELPER_EMIT_*), the seq the check is against
CMD_BATCH_HEADER_STRUCT = struct.Struct("!IHBI")
CMD_BATCH_ITEM_STRUCT = struct.Struct("!BH")
CMD_SET_GRAB_STRUCT = struct.Struct("!IBH")
CMD_QUERY_KEY_STRUCT = struct.Struct("!IH")

EXCLUDED_DEVICE_NAME_MARKERS = (
    "keyswitch",
    "ydotool",
    "uinput",
    "xremap",
    "kanata",
    "input-remapper",
    "power_switcher",
    "keyd virtual",
    "yubikey",
)

POLL_ERROR_MASK = select.POLLHUP | select.POLLERR | select.POLLNVAL

DeviceRole = Literal["keyboard", "pointer", "ignore"]


def seq_newer(seq: int, than: int) -> bool:
    """Whether ``seq`` came after ``than`` among the helper's wrapping 32-bit key numbers."""

    return seq != than and (seq - than) & UINT32_MODULO_MASK <= UINT32_MODULO_MASK >> 1


def pack_frame(msg_type: int, payload: bytes = b"") -> bytes:
    """Encode one binary control/event frame between parent and helper."""

    if len(payload) > HELPER_MAX_FRAME_BYTES:
        raise ValueError("Helper frame payload exceeds limit")
    return FRAME_HEADER.pack(msg_type, len(payload)) + payload


class FrameReader:
    """Incremental parser for length-prefixed helper frames."""

    def __init__(self) -> None:
        self._buffer = bytearray()

    def feed(self, chunk: bytes) -> list[tuple[int, bytes]]:
        self._buffer.extend(chunk)
        frames: list[tuple[int, bytes]] = []
        while len(self._buffer) >= HELPER_FRAME_HEADER_BYTES:
            msg_type, length = FRAME_HEADER.unpack_from(self._buffer, 0)
            if length > HELPER_MAX_FRAME_BYTES:
                raise ValueError("Helper frame length exceeds limit")
            end = HELPER_FRAME_HEADER_BYTES + length
            if len(self._buffer) < end:
                break
            payload = bytes(self._buffer[HELPER_FRAME_HEADER_BYTES:end])
            del self._buffer[:end]
            frames.append((int(msg_type), payload))
        return frames


@dataclass(frozen=True)
class InputDeviceInfo:
    """Capabilities of a ``/dev/input/event*`` node inspected before opening."""

    path: str
    name: str
    phys: str = ""
    bustype: int = 0
    vendor: int = 0
    product: int = 0
    ev_bits: frozenset[int] = frozenset()
    key_bits: frozenset[int] = frozenset()
    rel_bits: frozenset[int] = frozenset()
    abs_bits: frozenset[int] = frozenset()


@dataclass(frozen=True)
class RawInputEvent:
    """One kernel ``struct input_event`` read from an ``evdev`` descriptor."""

    sec: int
    usec: int
    ev_type: int
    code: int
    value: int

    def timestamp_ms(self) -> int:
        total = self.sec * MILLISECONDS_PER_SECOND + self.usec // MICROSECONDS_PER_MILLISECOND
        return total & UINT32_MODULO_MASK


def classify_input_device(info: InputDeviceInfo) -> DeviceRole:
    """Classify an evdev node as a physical keyboard to grab, passive pointer, or ignored."""

    if EV_KEY not in info.ev_bits:
        return "ignore"
    if (info.vendor, info.product) == (WAYLAND_UINPUT_VENDOR_ID, WAYLAND_UINPUT_PRODUCT_ID):
        return "ignore"
    lowered_name = info.name.casefold()
    lowered_phys = info.phys.casefold()
    if lowered_phys.startswith("keyswitch") or any(
        marker in lowered_name for marker in EXCLUDED_DEVICE_NAME_MARKERS
    ):
        return "ignore"
    if EVDEV_BTN_STYLUS in info.key_bits or EVDEV_BTN_GAMEPAD in info.key_bits:
        return "ignore"
    has_multitouch = EV_ABS in info.ev_bits and EVDEV_ABS_MT_POSITION_X in info.abs_bits
    has_rel_mouse = (
        EV_REL in info.ev_bits
        and EVDEV_REL_X in info.rel_bits
        and EVDEV_REL_Y in info.rel_bits
        and EVDEV_BTN_MOUSE in info.key_bits
    )
    if has_rel_mouse:
        return "pointer"
    if not has_multitouch and EVDEV_REQUIRED_KEYBOARD_KEYS <= info.key_bits:
        return "keyboard"
    if EVDEV_BTN_MOUSE in info.key_bits:
        return "pointer"
    return "ignore"


class HelperOS(Protocol):
    """Kernel boundary of the input helper, substituted in unit tests."""

    def create_uinput(self) -> int: ...

    def destroy_uinput(self, fd: int) -> None: ...

    def write_uinput(self, fd: int, events: Sequence[tuple[int, int, int]]) -> None: ...

    def read_uinput_leds(self, fd: int) -> Sequence[tuple[int, int]]: ...

    def list_event_devices(self) -> Sequence[InputDeviceInfo]: ...

    def open_device(self, path: str, *, writable: bool) -> int: ...

    def close_device(self, fd: int) -> None: ...

    def grab_device(self, fd: int) -> None: ...

    def ungrab_device(self, fd: int) -> None: ...

    def query_keys_down(self, fd: int) -> frozenset[int]: ...

    def query_caps_lock(self, fd: int) -> bool: ...

    def read_events(self, fd: int) -> Sequence[RawInputEvent]: ...

    def write_led(self, fd: int, code: int, value: int) -> None: ...

    def open_inotify(self) -> int: ...

    def read_inotify(self, fd: int) -> Sequence[str]: ...

    def poll(self, fds: Sequence[int], timeout_ms: int) -> Sequence[tuple[int, int]]: ...

    def monotonic(self) -> float: ...

    def sleep(self, seconds: float) -> None: ...


class ByteChannel(Protocol):
    """Bidirectional byte socket shared between ``HelperClient`` and ``WaylandInputHelper``."""

    def recv(self, bufsize: int) -> bytes: ...

    def sendall(self, data: bytes) -> None: ...

    def close(self) -> None: ...

    def fileno(self) -> int: ...


@dataclass(frozen=True)
class _QueuedKey:
    pressed: bool
    code: int
    timestamp_ms: int
    pressed_at: float


@dataclass(frozen=True)
class _PendingKey:
    seq: int
    pressed: bool
    code: int
    timestamp_ms: int
    deadline: float


class WaylandInputHelper:
    """Child-side evdev grabber and single ``/dev/uinput`` writer."""

    def __init__(
        self,
        channel: ByteChannel,
        os_api: HelperOS,
        *,
        repeat_delay_ms: int = WAYLAND_REPEAT_DEFAULT_DELAY_MS,
    ) -> None:
        self._channel = channel
        self._os = os_api
        self._repeat_delay_ms = repeat_delay_ms
        self._reader = FrameReader()
        self._uinput_fd: int | None = None
        self._inotify_fd: int | None = None
        self._keyboards: dict[int, str] = {}
        self._grabbed_fds: set[int] = set()
        self._pointers: dict[int, str] = {}
        self._device_keys_down: dict[int, set[int]] = {}
        self._passed_keys: set[int] = set()
        self._press_started: dict[int, float] = {}
        self._repeat_notified: set[int] = set()
        self._key_queue: deque[_QueuedKey] = deque()
        self._pending: _PendingKey | None = None
        # After a late verdict: the seq the helper typed itself; until the parent acknowledges it,
        # every key is typed at once and reported as typed.
        self._forced_seq: int | None = None
        # The last key typed without the parent's verdict: a correction decided before the parent
        # saw it would edit text it does not know.
        self._unverdicted_seq = 0
        self._next_seq = 1
        self._grab_enabled = True
        self._caps_lock = False
        self._running = False

    def stop(self) -> None:
        """Signal the helper loop to release grabs and exit cleanly."""

        self._running = False

    def run(self) -> int:
        try:
            self._uinput_fd = self._os.create_uinput()
            self._os.sleep(WAYLAND_UINPUT_SETTLE_SECONDS)
            self._inotify_fd = self._os.open_inotify()
            self._rescan_devices(initial=True)
            self._caps_lock = any(
                self._os.query_caps_lock(fd) for fd in self._keyboards
            )
            self._send(HELPER_MSG_READY, U8_STRUCT.pack(1 if self._caps_lock else 0))
            self._running = True
            while self._running:
                self._step()
            return 0
        except Exception as error:
            try:
                self._send(HELPER_MSG_ERROR, str(error).encode("utf-8"))
            except OSError:
                pass
            return 1
        finally:
            self._cleanup()

    def _step(self) -> None:
        chan_fd = self._channel.fileno()
        uinput_fd = self._uinput_fd
        inotify_fd = self._inotify_fd
        assert uinput_fd is not None and inotify_fd is not None

        fds = [chan_fd, uinput_fd, inotify_fd, *self._keyboards, *self._pointers]
        timeout_ms = WAYLAND_HELPER_POLL_TIMEOUT_MS
        if self._pending is not None:
            remaining_ms = max(
                0,
                round((self._pending.deadline - self._os.monotonic()) * MILLISECONDS_PER_SECOND),
            )
            timeout_ms = min(timeout_ms, remaining_ms)

        events = dict(self._os.poll(fds, timeout_ms))

        if chan_fd in events:
            if not self._read_channel(events[chan_fd]):
                self._running = False
                return

        self._expire_pending_if_due()

        if uinput_fd in events and (events[uinput_fd] & select.POLLIN):
            self._forward_uinput_leds(uinput_fd)

        if inotify_fd in events and (events[inotify_fd] & select.POLLIN):
            changed = self._os.read_inotify(inotify_fd)
            if any(name.startswith("event") for name in changed):
                self._rescan_devices(initial=False)

        for fd in tuple(self._keyboards):
            mask = events.get(fd, 0)
            if mask & POLL_ERROR_MASK:
                self._close_keyboard(fd)
            elif mask & select.POLLIN:
                self._read_keyboard(fd)

        for fd in tuple(self._pointers):
            mask = events.get(fd, 0)
            if mask & POLL_ERROR_MASK:
                self._close_pointer(fd)
            elif mask & select.POLLIN:
                self._read_pointer(fd)

    def _read_channel(self, mask: int) -> bool:
        if mask & select.POLLIN:
            try:
                chunk = self._channel.recv(HELPER_SOCKET_CHUNK_BYTES)
            except OSError:
                return False
            if not chunk:
                return False
            for msg_type, payload in self._reader.feed(chunk):
                if not self._handle_command(msg_type, payload):
                    return False
            return True
        return not bool(mask & POLL_ERROR_MASK)

    def _handle_command(self, msg_type: int, payload: bytes) -> bool:
        if msg_type == HELPER_CMD_STOP:
            return False
        if msg_type == HELPER_CMD_VERDICT:
            seq, swallow = CMD_VERDICT_STRUCT.unpack(payload)
            self._apply_verdict(int(seq), bool(swallow))
            return True
        if msg_type == HELPER_CMD_EMIT_BATCH:
            req_id, count, check, known_seq = CMD_BATCH_HEADER_STRUCT.unpack_from(payload, 0)
            if self._refuses(check, known_seq):
                # A key reached the application after the parent decided on this text, or the
                # withheld key is there already: refused whole.
                self._send(HELPER_MSG_EMIT_ACK, MSG_ACK_STRUCT.pack(req_id, 0))
                return True
            settled = check == HELPER_EMIT_ACTION and self._pending is not None and self._pending.seq == known_seq
            if settled:
                # The parent types the key it is still to answer: it has handled it, swallowed.
                self._pending = None
            offset = CMD_BATCH_HEADER_STRUCT.size
            emitted = 0
            raw_events: list[tuple[int, int, int]] = []
            for _ in range(count):
                pressed_u8, keycode = CMD_BATCH_ITEM_STRUCT.unpack_from(payload, offset)
                offset += HELPER_EMIT_ITEM_BYTES
                pressed = bool(pressed_u8)
                if pressed and keycode in self._passed_keys:
                    # The key is still down in the virtual keyboard (the user holds it): the kernel
                    # would drop a second press, and the character typed with it would be missing.
                    raw_events.append((EV_KEY, keycode, KEY_VALUE_RELEASE))
                    raw_events.append((EV_SYN, SYN_REPORT, 0))
                if pressed:
                    self._passed_keys.add(keycode)
                else:
                    self._passed_keys.discard(keycode)
                raw_events.append((
                    EV_KEY,
                    keycode,
                    KEY_VALUE_PRESS if pressed else KEY_VALUE_RELEASE,
                ))
                raw_events.append((EV_SYN, SYN_REPORT, 0))
                emitted += 1
            if raw_events and self._uinput_fd is not None:
                self._write_paced(self._uinput_fd, raw_events)
            self._send(HELPER_MSG_EMIT_ACK, MSG_ACK_STRUCT.pack(req_id, emitted))
            if settled:
                self._dispatch_next_queued_key()
            return True
        if msg_type == HELPER_CMD_SEQ_ACK:
            (seq,) = U32_STRUCT.unpack(payload)
            if self._forced_seq == seq:
                self._forced_seq = None
            return True
        if msg_type == HELPER_CMD_QUERY_KEY_DOWN:
            req_id, keycode = CMD_QUERY_KEY_STRUCT.unpack(payload)
            down = self._is_key_physically_down(int(keycode))
            self._send(HELPER_MSG_KEY_DOWN, MSG_KEY_DOWN_STRUCT.pack(req_id, 1 if down else 0))
            return True
        if msg_type == HELPER_CMD_SET_GRAB:
            req_id, enabled_u8, repeat_delay_ms = CMD_SET_GRAB_STRUCT.unpack(payload)
            if repeat_delay_ms > 0:
                self._repeat_delay_ms = int(repeat_delay_ms)
            self._set_grab_enabled(bool(enabled_u8))
            self._send(
                HELPER_MSG_EMIT_ACK,
                MSG_ACK_STRUCT.pack(req_id, 1 if self._grab_enabled else 0),
            )
            return True
        return True

    def _refuses(self, check: int, known_seq: int) -> bool:
        """Whether an injection would land after keys typed without the parent's verdict."""

        if check == HELPER_EMIT_TEXT:
            return self._forced_seq is not None or seq_newer(self._unverdicted_seq, known_seq)
        if check == HELPER_EMIT_ACTION:
            return self._forced_seq is not None or (
                self._unverdicted_seq != 0 and not seq_newer(known_seq, self._unverdicted_seq)
            )
        return False

    def _write_paced(self, fd: int, raw_events: Sequence[tuple[int, int, int]]) -> None:
        """Write key transitions in bursts the compositor's event queue holds (see the burst constant)."""

        burst = WAYLAND_UINPUT_BURST_TRANSITIONS * TRANSITION_EVENTS
        for start in range(0, len(raw_events), burst):
            if start:
                self._os.sleep(WAYLAND_UINPUT_BURST_PAUSE_SECONDS)
            self._os.write_uinput(fd, raw_events[start : start + burst])

    def _apply_verdict(self, seq: int, swallow: bool) -> None:
        pending = self._pending
        if pending is None or pending.seq != seq:
            # Late: the key was typed when its time ran out (_expire_pending_if_due).
            return
        self._pending = None
        if not swallow:
            self._emit_physical_key(pending.pressed, pending.code)
        self._dispatch_next_queued_key()

    def _expire_pending_if_due(self) -> None:
        pending = self._pending
        if pending is None or self._os.monotonic() < pending.deadline:
            return
        self._pending = None
        self._forced_seq = pending.seq
        self._unverdicted_seq = pending.seq
        self._emit_physical_key(pending.pressed, pending.code)
        # Reported before the queued keys, in the order the application received them.
        self._send_key(pending.seq, pending.pressed, pending.code, pending.timestamp_ms, HELPER_KEY_FORCED)
        while self._key_queue:
            self._type_unasked(self._key_queue.popleft())

    def _type_unasked(self, item: _QueuedKey) -> None:
        """Type a key without the parent's verdict, and tell the parent so that it misses none."""

        seq = self._allocate_seq()
        self._unverdicted_seq = seq
        self._emit_physical_key(item.pressed, item.code)
        self._send_key(seq, item.pressed, item.code, item.timestamp_ms, 0)

    def _enqueue_physical_key(self, item: _QueuedKey) -> None:
        if self._forced_seq is not None:
            self._type_unasked(item)
            return
        self._key_queue.append(item)
        if self._pending is None:
            self._dispatch_next_queued_key()

    def _allocate_seq(self) -> int:
        seq = (self._next_seq + 1) & UINT32_MODULO_MASK or 1
        self._next_seq = seq
        return seq

    def _dispatch_next_queued_key(self) -> None:
        if not self._key_queue:
            return
        item = self._key_queue.popleft()
        seq = self._allocate_seq()
        deadline = (
            self._os.monotonic()
            + WAYLAND_HELPER_VERDICT_TIMEOUT_MS / MILLISECONDS_PER_SECOND
        )
        self._pending = _PendingKey(seq, item.pressed, item.code, item.timestamp_ms, deadline)
        self._send_key(seq, item.pressed, item.code, item.timestamp_ms, HELPER_KEY_NEEDS_VERDICT)

    def _send_key(self, seq: int, pressed: bool, code: int, timestamp_ms: int, flags: int) -> None:
        self._send(
            HELPER_MSG_KEY,
            MSG_KEY_STRUCT.pack(seq, 1 if pressed else 0, code, timestamp_ms, flags),
        )

    def _emit_physical_key(self, pressed: bool, code: int) -> None:
        if self._uinput_fd is None:
            return
        if pressed:
            self._passed_keys.add(code)
        else:
            self._passed_keys.discard(code)
        self._os.write_uinput(
            self._uinput_fd,
            (
                (EV_KEY, code, KEY_VALUE_PRESS if pressed else KEY_VALUE_RELEASE),
                (EV_SYN, SYN_REPORT, 0),
            ),
        )

    def _read_keyboard(self, fd: int) -> None:
        try:
            events = self._os.read_events(fd)
        except OSError:
            self._close_keyboard(fd)
            return
        if not self._grab_enabled or fd not in self._grabbed_fds:
            return
        now = self._os.monotonic()
        held = self._device_keys_down.setdefault(fd, set())
        for ev in events:
            if ev.ev_type != EV_KEY:
                continue
            code = ev.code
            if ev.value == KEY_VALUE_REPEAT:
                if (
                    code in self._passed_keys
                    and code not in EVDEV_MODIFIER_KEYS
                    and code != EVDEV_KEY_CAPSLOCK
                    and code not in self._repeat_notified
                ):
                    started = self._press_started.get(code, now)
                    elapsed_ms = (now - started) * MILLISECONDS_PER_SECOND
                    if elapsed_ms >= self._repeat_delay_ms:
                        self._repeat_notified.add(code)
                        self._send(
                            HELPER_MSG_REPEAT_RESET,
                            U32_STRUCT.pack(ev.timestamp_ms()),
                        )
                continue
            if ev.value == KEY_VALUE_PRESS:
                held.add(code)
                self._press_started[code] = now
                self._repeat_notified.discard(code)
                self._enqueue_physical_key(
                    _QueuedKey(True, code, ev.timestamp_ms(), now)
                )
            elif ev.value == KEY_VALUE_RELEASE:
                held.discard(code)
                self._press_started.pop(code, None)
                self._repeat_notified.discard(code)
                self._enqueue_physical_key(
                    _QueuedKey(False, code, ev.timestamp_ms(), now)
                )

    def _read_pointer(self, fd: int) -> None:
        try:
            events = self._os.read_events(fd)
        except OSError:
            self._close_pointer(fd)
            return
        if not self._grab_enabled:
            return
        for ev in events:
            if (
                ev.ev_type == EV_KEY
                and EVDEV_BTN_MOUSE <= ev.code <= EVDEV_BTN_TASK
                and ev.value == KEY_VALUE_PRESS
            ):
                self._send(HELPER_MSG_POINTER, U32_STRUCT.pack(ev.timestamp_ms()))

    def _forward_uinput_leds(self, uinput_fd: int) -> None:
        try:
            leds = self._os.read_uinput_leds(uinput_fd)
        except OSError:
            return
        for code, value in leds:
            for kbd_fd in tuple(self._keyboards):
                try:
                    self._os.write_led(kbd_fd, code, value)
                except OSError:
                    pass
            if code == LED_CAPSL:
                enabled = bool(value)
                if enabled != self._caps_lock:
                    self._caps_lock = enabled
                    self._send(HELPER_MSG_CAPS_LOCK, U8_STRUCT.pack(1 if enabled else 0))

    def _is_key_physically_down(self, keycode: int) -> bool:
        for fd in tuple(self._keyboards):
            try:
                if keycode in self._os.query_keys_down(fd):
                    return True
            except OSError:
                self._close_keyboard(fd)
        return False

    def _set_grab_enabled(self, enabled: bool) -> None:
        if enabled == self._grab_enabled:
            return
        self._grab_enabled = enabled
        # The keys down so far went through the grab: the parent forgets them either way.
        self._send(HELPER_MSG_INPUT_RESET, b"")
        if not enabled:
            self._pending = None
            self._forced_seq = None
            self._key_queue.clear()
            self._press_started.clear()
            self._repeat_notified.clear()
            self._release_all_passed_keys()
            for fd in tuple(self._grabbed_fds):
                try:
                    self._os.ungrab_device(fd)
                except OSError:
                    pass
                self._device_keys_down.get(fd, set()).clear()
            self._grabbed_fds.clear()
            return
        self._rescan_devices(initial=False)
        for fd in tuple(self._keyboards):
            if fd not in self._grabbed_fds:
                self._grab_keyboard_safely(fd)

    def _rescan_devices(self, *, initial: bool) -> None:
        known_keyboards = {path: fd for fd, path in self._keyboards.items()}
        known_pointers = {path: fd for fd, path in self._pointers.items()}
        seen_paths: set[str] = set()
        denied_keyboard = False

        for info in self._os.list_event_devices():
            seen_paths.add(info.path)
            role = classify_input_device(info)
            if role == "keyboard" and info.path not in known_keyboards:
                try:
                    fd = self._os.open_device(info.path, writable=True)
                except PermissionError:
                    try:
                        fd = self._os.open_device(info.path, writable=False)
                    except PermissionError:
                        denied_keyboard = True
                        continue
                    except OSError:
                        continue
                except OSError:
                    continue
                self._keyboards[fd] = info.path
                self._device_keys_down[fd] = set()
                if self._grab_enabled and not self._grab_keyboard_safely(fd):
                    self._close_keyboard(fd)
            elif role == "pointer" and info.path not in known_pointers:
                try:
                    fd = self._os.open_device(info.path, writable=False)
                except OSError:
                    continue
                self._pointers[fd] = info.path

        for path, fd in known_keyboards.items():
            if path not in seen_paths:
                self._close_keyboard(fd)
        for path, fd in known_pointers.items():
            if path not in seen_paths:
                self._close_pointer(fd)

        if initial and denied_keyboard and not self._keyboards:
            raise PermissionError(
                "Нет доступа к клавиатурам /dev/input/event*: проверьте правила udev "
                "пакета keyswitch или перезайдите в сеанс"
            )

    def _grab_keyboard_safely(self, fd: int) -> bool:
        try:
            down = self._os.query_keys_down(fd)
            if down:
                deadline = self._os.monotonic() + WAYLAND_GRAB_WAIT_TIMEOUT_SECONDS
                while down and self._os.monotonic() < deadline:
                    self._os.sleep(WAYLAND_GRAB_POLL_SECONDS)
                    down = self._os.query_keys_down(fd)
                if not down:
                    self._os.sleep(WAYLAND_GRAB_RELEASE_SETTLE_SECONDS)
            self._os.grab_device(fd)
            self._os.read_events(fd)
            self._grabbed_fds.add(fd)
            return True
        except OSError:
            return False

    def _close_keyboard(self, fd: int) -> None:
        self._keyboards.pop(fd, None)
        held = self._device_keys_down.pop(fd, set())
        if fd in self._grabbed_fds:
            self._grabbed_fds.discard(fd)
            try:
                self._os.ungrab_device(fd)
            except OSError:
                pass
        try:
            self._os.close_device(fd)
        except OSError:
            pass
        still_held = set().union(*self._device_keys_down.values()) if self._device_keys_down else set()
        orphaned = (held & self._passed_keys) - still_held
        for code in sorted(orphaned):
            self._emit_physical_key(False, code)

    def _close_pointer(self, fd: int) -> None:
        self._pointers.pop(fd, None)
        try:
            self._os.close_device(fd)
        except OSError:
            pass

    def _release_all_passed_keys(self) -> None:
        if not self._passed_keys or self._uinput_fd is None:
            self._passed_keys.clear()
            return
        events: list[tuple[int, int, int]] = []
        for code in sorted(self._passed_keys):
            events.append((EV_KEY, code, KEY_VALUE_RELEASE))
            events.append((EV_SYN, SYN_REPORT, 0))
        self._passed_keys.clear()
        try:
            self._os.write_uinput(self._uinput_fd, events)
        except OSError:
            pass

    def _cleanup(self) -> None:
        self._release_all_passed_keys()
        for fd in tuple(self._keyboards):
            self._close_keyboard(fd)
        for fd in tuple(self._pointers):
            self._close_pointer(fd)
        if self._inotify_fd is not None:
            try:
                self._os.close_device(self._inotify_fd)
            except OSError:
                pass
            self._inotify_fd = None
        if self._uinput_fd is not None:
            try:
                self._os.destroy_uinput(self._uinput_fd)
            except OSError:
                pass
            self._uinput_fd = None

    def _send(self, msg_type: int, payload: bytes) -> None:
        self._channel.sendall(pack_frame(msg_type, payload))


@dataclass(frozen=True)
class HelperKey:
    """A physical key the helper reports: asked about, typed by the helper after a late verdict
    (``forced``, the same seq as when asked), or typed without asking (neither flag)."""

    seq: int
    pressed: bool
    keycode: int
    timestamp_ms: int
    flags: int

    @property
    def needs_verdict(self) -> bool:
        return bool(self.flags & HELPER_KEY_NEEDS_VERDICT)

    @property
    def forced(self) -> bool:
        return bool(self.flags & HELPER_KEY_FORCED)


@dataclass
class _PendingReply:
    event: threading.Event
    value: int = 0
    failed: bool = False


class HelperClient:
    """Parent-side client speaking the binary helper protocol without callback deadlocks."""

    def __init__(
        self,
        channel: ByteChannel,
        *,
        on_key: Callable[[HelperKey], bool],
        on_pointer: Callable[[int], None],
        on_repeat_reset: Callable[[int], None],
        on_caps_lock: Callable[[bool], None],
        on_input_reset: Callable[[], None],
    ) -> None:
        self._channel = channel
        self._on_key = on_key
        self._on_input_reset = on_input_reset
        self._on_pointer = on_pointer
        self._on_repeat_reset = on_repeat_reset
        self._on_caps_lock = on_caps_lock
        self._send_lock = threading.Lock()
        self._reply_lock = threading.Lock()
        self._replies: dict[int, _PendingReply] = {}
        self._next_req_id = 1
        self._ready = threading.Event()
        self._closed = threading.Event()
        self._error = ""
        self._caps_lock = False
        self._events: queue.SimpleQueue[tuple[int, bytes] | None] = queue.SimpleQueue()
        self._reader_thread: threading.Thread | None = None
        self._dispatch_thread: threading.Thread | None = None

    @property
    def caps_lock(self) -> bool:
        return self._caps_lock

    @property
    def closed(self) -> bool:
        """The helper's socket is gone: it exited, or the client was closed."""

        return self._closed.is_set()

    def start(self, timeout: float = KEYBOARD_LISTENER_START_TIMEOUT_SECONDS) -> bool:
        self._reader_thread = threading.Thread(
            target=self._read_loop,
            name="keyswitch-wayland-helper-read",
            daemon=True,
        )
        self._dispatch_thread = threading.Thread(
            target=self._dispatch_loop,
            name="keyswitch-wayland-helper-dispatch",
            daemon=True,
        )
        self._reader_thread.start()
        self._dispatch_thread.start()
        if not self._ready.wait(timeout):
            self.close()
            raise RuntimeError("Процесс перехвата клавиатуры Wayland не ответил вовремя")
        if self._error:
            message = self._error
            self.close()
            raise RuntimeError(message)
        if self._closed.is_set():
            self.close()
            raise RuntimeError("Процесс перехвата клавиатуры Wayland завершился при запуске")
        return self._caps_lock

    def emit_batch(
        self, items: Sequence[tuple[bool, int]], known_seq: int | None = None, *, action: bool = False
    ) -> int:
        """Type ``items``; with ``known_seq`` the helper types none of them (the answer is 0) if a key
        it typed without a verdict came after that seq - or, for an ``action``, if it typed that very
        key itself already."""

        if not items:
            return 0
        check = HELPER_EMIT_UNCHECKED if known_seq is None else HELPER_EMIT_ACTION if action else HELPER_EMIT_TEXT
        req_id, slot = self._allocate_reply()
        body = bytearray(CMD_BATCH_HEADER_STRUCT.pack(req_id, len(items), check, known_seq or 0))
        for pressed, keycode in items:
            body.extend(CMD_BATCH_ITEM_STRUCT.pack(1 if pressed else 0, keycode))
        return self._request(req_id, slot, HELPER_CMD_EMIT_BATCH, bytes(body))

    def query_key_down(self, keycode: int) -> bool:
        req_id, slot = self._allocate_reply()
        payload = CMD_QUERY_KEY_STRUCT.pack(req_id, keycode)
        return bool(self._request(req_id, slot, HELPER_CMD_QUERY_KEY_DOWN, payload))

    def set_grab(
        self,
        enabled: bool,
        *,
        repeat_delay_ms: int = WAYLAND_REPEAT_DEFAULT_DELAY_MS,
    ) -> bool:
        req_id, slot = self._allocate_reply()
        payload = CMD_SET_GRAB_STRUCT.pack(req_id, 1 if enabled else 0, repeat_delay_ms)
        return bool(self._request(req_id, slot, HELPER_CMD_SET_GRAB, payload))

    def close(self) -> None:
        if not self._closed.is_set():
            self._closed.set()
            try:
                self._send_frame(HELPER_CMD_STOP, b"")
            except OSError:
                pass
        try:
            self._channel.close()
        except OSError:
            pass
        self._fail_pending_replies()
        self._events.put(None)
        self._ready.set()
        current = threading.current_thread()
        for thread in (self._reader_thread, self._dispatch_thread):
            if thread is not None and thread is not current:
                thread.join(timeout=KEYBOARD_LISTENER_STOP_TIMEOUT_SECONDS)

    def _allocate_reply(self) -> tuple[int, _PendingReply]:
        with self._reply_lock:
            req_id = (self._next_req_id + 1) & UINT32_MODULO_MASK or 1
            self._next_req_id = req_id
            slot = _PendingReply(threading.Event())
            self._replies[req_id] = slot
            return req_id, slot

    def _request(
        self,
        req_id: int,
        slot: _PendingReply,
        cmd_type: int,
        payload: bytes,
    ) -> int:
        try:
            try:
                self._send_frame(cmd_type, payload)
            except OSError as error:
                raise RuntimeError("Нет ответа от процесса ввода Wayland") from error
            if not slot.event.wait(WAYLAND_HELPER_COMMAND_TIMEOUT_SECONDS) or slot.failed:
                raise RuntimeError("Нет ответа от процесса ввода Wayland")
            return slot.value
        finally:
            with self._reply_lock:
                self._replies.pop(req_id, None)

    def _send_frame(self, msg_type: int, payload: bytes) -> None:
        frame = pack_frame(msg_type, payload)
        with self._send_lock:
            self._channel.sendall(frame)

    def _read_loop(self) -> None:
        reader = FrameReader()
        try:
            while not self._closed.is_set():
                try:
                    chunk = self._channel.recv(HELPER_SOCKET_CHUNK_BYTES)
                except OSError:
                    break
                if not chunk:
                    break
                for msg_type, payload in reader.feed(chunk):
                    self._route_incoming(msg_type, payload)
        finally:
            self._closed.set()
            self._fail_pending_replies()
            self._events.put(None)
            self._ready.set()

    def _route_incoming(self, msg_type: int, payload: bytes) -> None:
        if msg_type == HELPER_MSG_READY:
            (caps,) = U8_STRUCT.unpack(payload)
            self._caps_lock = bool(caps)
            self._ready.set()
            return
        if msg_type == HELPER_MSG_ERROR:
            self._error = payload.decode("utf-8", errors="replace")
            self._ready.set()
            return
        if msg_type == HELPER_MSG_EMIT_ACK:
            req_id, value = MSG_ACK_STRUCT.unpack(payload)
            self._resolve_reply(int(req_id), int(value))
            return
        if msg_type == HELPER_MSG_KEY_DOWN:
            req_id, down = MSG_KEY_DOWN_STRUCT.unpack(payload)
            self._resolve_reply(int(req_id), int(down))
            return
        self._events.put((msg_type, payload))

    def _resolve_reply(self, req_id: int, value: int) -> None:
        with self._reply_lock:
            slot = self._replies.get(req_id)
            if slot is not None:
                slot.value = value
                slot.event.set()

    def _fail_pending_replies(self) -> None:
        with self._reply_lock:
            for slot in self._replies.values():
                slot.failed = True
                slot.event.set()

    def _dispatch_loop(self) -> None:
        while True:
            item = self._events.get()
            if item is None:
                return
            msg_type, payload = item
            if msg_type == HELPER_MSG_KEY:
                seq, pressed_u8, keycode, timestamp_ms, flags = MSG_KEY_STRUCT.unpack(payload)
                key = HelperKey(seq, bool(pressed_u8), int(keycode), int(timestamp_ms), int(flags))
                swallow = False
                try:
                    swallow = bool(self._on_key(key))
                except Exception:
                    swallow = False
                try:
                    if key.needs_verdict:
                        verdict = HELPER_VERDICT_SWALLOW if swallow else HELPER_VERDICT_PASS
                        self._send_frame(HELPER_CMD_VERDICT, CMD_VERDICT_STRUCT.pack(seq, verdict))
                    elif key.forced:
                        self._send_frame(HELPER_CMD_SEQ_ACK, U32_STRUCT.pack(seq))
                except OSError:
                    return
            elif msg_type == HELPER_MSG_INPUT_RESET:
                self._on_input_reset()
            elif msg_type == HELPER_MSG_POINTER:
                (timestamp_ms,) = U32_STRUCT.unpack(payload)
                self._on_pointer(int(timestamp_ms))
            elif msg_type == HELPER_MSG_REPEAT_RESET:
                (timestamp_ms,) = U32_STRUCT.unpack(payload)
                self._on_repeat_reset(int(timestamp_ms))
            elif msg_type == HELPER_MSG_CAPS_LOCK:
                (enabled_u8,) = U8_STRUCT.unpack(payload)
                self._caps_lock = bool(enabled_u8)
                self._on_caps_lock(self._caps_lock)
