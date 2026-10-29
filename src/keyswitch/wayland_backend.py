"""Wayland keyboard backend with exclusive evdev capture, uinput injection and compositor IPC."""

from __future__ import annotations

from collections.abc import Callable, Iterable, Sequence
from dataclasses import dataclass, replace
import logging
import threading
import time
from typing import Protocol

from .backend import (
    BackendProbe,
    FocusInfo,
    KeyDisposition,
    KeyEvent,
    LayoutSwitchUnconfirmed,
    ScreenAnchor,
)
from .constants.keyboard import (
    ALT_MASK,
    COMPLETED_ACTION_EVENT_COUNT,
    CONTROL_MASK,
    LAYOUT_GROUP_COUNT,
    LOCK_MASK,
    POINTER_EVENT_GROUP,
    SHIFT_MASK,
    SUPER_MASK,
)
from .constants.text import CYRILLIC_CODEPOINT_RANGE
from .constants.timing import (
    KEYBOARD_LISTENER_START_TIMEOUT_SECONDS,
    KEYBOARD_LISTENER_STOP_TIMEOUT_SECONDS,
    LAYOUT_SWITCH_POLL_SECONDS,
    LAYOUT_SWITCH_TIMEOUT_SECONDS,
    MODIFIER_RECHECK_GAP_MS,
    WAYLAND_TYPED_INPUT_SETTLE_SECONDS,
)
from .constants.units import MILLISECONDS_PER_SECOND
from .constants.wayland import (
    EVDEV_ALT_KEYS,
    EVDEV_CONTROL_KEYS,
    EVDEV_KEY_APOSTROPHE,
    EVDEV_KEY_BACKSLASH,
    EVDEV_KEY_BACKSPACE,
    EVDEV_KEY_CAPSLOCK,
    EVDEV_KEY_COMMA,
    EVDEV_KEY_DELETE,
    EVDEV_KEY_DOT,
    EVDEV_KEY_DOWN,
    EVDEV_KEY_END,
    EVDEV_KEY_ENTER,
    EVDEV_KEY_EQUAL,
    EVDEV_KEY_ESC,
    EVDEV_KEY_GRAVE,
    EVDEV_KEY_HOME,
    EVDEV_KEY_INSERT,
    EVDEV_KEY_KPENTER,
    EVDEV_KEY_LEFT,
    EVDEV_KEY_LEFTBRACE,
    EVDEV_KEY_LEFTCTRL,
    EVDEV_KEY_LEFTMETA,
    EVDEV_KEY_LEFTSHIFT,
    EVDEV_KEY_LEFTALT,
    EVDEV_KEY_MINUS,
    EVDEV_KEY_PAGEDOWN,
    EVDEV_KEY_PAGEUP,
    EVDEV_KEY_PAUSE,
    EVDEV_KEY_Q,
    EVDEV_KEY_RIGHT,
    EVDEV_KEY_RIGHTALT,
    EVDEV_KEY_RIGHTBRACE,
    EVDEV_KEY_RIGHTCTRL,
    EVDEV_KEY_RIGHTMETA,
    EVDEV_KEY_RIGHTSHIFT,
    EVDEV_KEY_SEMICOLON,
    EVDEV_KEY_SLASH,
    EVDEV_KEY_SPACE,
    EVDEV_KEY_TAB,
    EVDEV_KEY_UP,
    EVDEV_MODIFIER_KEYS,
    EVDEV_SHIFT_KEYS,
    EVDEV_SUPER_KEYS,
    UINT32_MODULO_MASK,
    WAYLAND_UINPUT_MAX_KEYCODE,
)
from .input_delay import InputDelay, InputDelayReport
from .russian_text import SECONDS, quantity

LOGGER = logging.getLogger(__name__)

SESSION_TYPE = "wayland"
CAPTURE_METHOD = "evdev (EVIOCGRAB)"
INJECT_METHOD = "uinput"
POINTER_KEY_NAME = "Pointer"
INPUT_AHEAD_ERROR = "Клавиша дошла до приложения раньше замены; текст не изменён"

EVDEV_KEY_NAMES = {
    EVDEV_KEY_BACKSPACE: "BackSpace",
    EVDEV_KEY_TAB: "Tab",
    EVDEV_KEY_ENTER: "Return",
    EVDEV_KEY_KPENTER: "KP_Enter",
    EVDEV_KEY_ESC: "Escape",
    EVDEV_KEY_SPACE: "space",
    EVDEV_KEY_PAUSE: "Pause",
    EVDEV_KEY_CAPSLOCK: "Caps_Lock",
    EVDEV_KEY_LEFTSHIFT: "Shift_L",
    EVDEV_KEY_RIGHTSHIFT: "Shift_R",
    EVDEV_KEY_LEFTCTRL: "Control_L",
    EVDEV_KEY_RIGHTCTRL: "Control_R",
    EVDEV_KEY_LEFTALT: "Alt_L",
    EVDEV_KEY_RIGHTALT: "Alt_R",
    EVDEV_KEY_LEFTMETA: "Super_L",
    EVDEV_KEY_RIGHTMETA: "Super_R",
    EVDEV_KEY_HOME: "Home",
    EVDEV_KEY_END: "End",
    EVDEV_KEY_PAGEUP: "Page_Up",
    EVDEV_KEY_PAGEDOWN: "Page_Down",
    EVDEV_KEY_LEFT: "Left",
    EVDEV_KEY_RIGHT: "Right",
    EVDEV_KEY_UP: "Up",
    EVDEV_KEY_DOWN: "Down",
    EVDEV_KEY_INSERT: "Insert",
    EVDEV_KEY_DELETE: "Delete",
    EVDEV_KEY_SEMICOLON: "semicolon",
    EVDEV_KEY_EQUAL: "equal",
    EVDEV_KEY_COMMA: "comma",
    EVDEV_KEY_MINUS: "minus",
    EVDEV_KEY_DOT: "period",
    EVDEV_KEY_SLASH: "slash",
    EVDEV_KEY_GRAVE: "grave",
    EVDEV_KEY_LEFTBRACE: "bracketleft",
    EVDEV_KEY_BACKSLASH: "backslash",
    EVDEV_KEY_RIGHTBRACE: "bracketright",
    EVDEV_KEY_APOSTROPHE: "apostrophe",
}


class WaylandBackendError(RuntimeError):
    """The Wayland backend cannot perform the requested operation."""


class WaylandLayoutSwitchUnconfirmed(WaylandBackendError, LayoutSwitchUnconfirmed):
    """The Wayland compositor did not report the requested layout in time."""


@dataclass(frozen=True)
class NativeKeyEvent:
    """One keyboard or pointer event received from the Wayland input helper.

    ``seq`` numbers the helper's physical keys. ``delivered`` is a key the helper typed without
    asking (its verdict was late); ``forced`` repeats an asked key the helper typed when its time
    ran out; ``input_reset`` says the grab changed and the keys held down are unknown.
    """

    pressed: bool
    keycode: int
    timestamp: int
    injected: bool = False
    replayed: bool = False
    pointer: bool = False
    repeat_reset: bool = False
    seq: int = 0
    delivered: bool = False
    forced: bool = False
    input_reset: bool = False


@dataclass(frozen=True)
class NativeInput:
    """One synthetic or replayed key transition destined for ``/dev/uinput``."""

    pressed: bool
    keycode: int = 0
    synthetic: bool = True
    replayed: bool = False


class WaylandAPI(Protocol):
    """Everything ``WaylandBackend`` needs from Linux, XKB and the Wayland compositor."""

    def desktop_name(self) -> str: ...

    def permission_error(self) -> str: ...

    def layout_names(self) -> tuple[str, ...]: ...

    def current_layout_index(self) -> int: ...

    def select_layout_index(self, index: int) -> bool: ...

    def translate_key(self, keycode: int, state: int, layout_index: int) -> str: ...

    def keysym_name(self, keycode: int, layout_index: int = 0) -> str: ...

    def post_inputs(
        self, inputs: tuple[tuple[bool, int], ...], known_seq: int | None = None, action: bool = False
    ) -> int: ...

    def active_application(self) -> str: ...

    def focused_window(self) -> int: ...

    def window_process_id(self, window: int) -> int: ...

    def current_process_id(self) -> int: ...

    def window_origin(self, window: int) -> tuple[int, int] | None: ...

    def window_is_password(self, window: int) -> bool: ...

    def input_anchor(self) -> ScreenAnchor | None: ...

    def position_window(self, window: int, x: int, y: int) -> bool: ...

    def activate_window(self, window: int) -> bool: ...

    def isolated_layout(self) -> bool: ...

    def caps_lock_enabled(self) -> bool: ...

    def key_down(self, keycode: int) -> bool: ...

    def monotonic_ms(self) -> int: ...

    def run_input_listener(
        self,
        listener: Callable[[NativeKeyEvent], bool],
        ready: Callable[[], None],
    ) -> None: ...

    def stop_input_listener(self) -> None: ...

    def close(self) -> None: ...


def is_cyrillic(text: str) -> bool:
    return (
        bool(text)
        and CYRILLIC_CODEPOINT_RANGE[0] <= ord(text[0]) <= CYRILLIC_CODEPOINT_RANGE[1]
    )


def select_layout_indices(
    layout_count: int,
    translate: Callable[[int, int, int], str],
) -> tuple[int, int]:
    """Identify the English and Russian compositor layout indices by probing ``KEY_Q``."""

    english: int | None = None
    russian: int | None = None
    for index in range(layout_count):
        sample = translate(EVDEV_KEY_Q, 0, index)
        if english is None and sample.isascii() and sample.isalpha():
            english = index
        elif russian is None and is_cyrillic(sample):
            russian = index
    if english is None or russian is None:
        raise WaylandBackendError(
            "В среде Wayland должны быть включены английская и русская раскладки"
        )
    return english, russian


def key_name(keycode: int, symbol_name: str = "", plain_char: str = "") -> str:
    """Translate an evdev keycode and XKB keysym into the engine's X11 key vocabulary."""

    if keycode in EVDEV_KEY_NAMES:
        return EVDEV_KEY_NAMES[keycode]
    if symbol_name:
        if len(symbol_name) == 1 and symbol_name.isascii() and symbol_name.isalnum():
            return symbol_name.casefold()
        return symbol_name
    if len(plain_char) == 1 and plain_char.isascii() and plain_char.isalnum():
        return plain_char.casefold()
    return f"KEY_{keycode}"


class WaylandBackend:
    """Global Wayland keyboard observer, key filter, input holder and injector."""

    swallows_hotkeys = True

    def __init__(
        self,
        api: WaylandAPI | None = None,
        *,
        group_count: int = LAYOUT_GROUP_COUNT,
    ) -> None:
        if api is None:
            from .wayland_native import NativeWaylandAPI

            api = NativeWaylandAPI()
        self._api = api
        self._group_count = group_count
        self._layouts: tuple[int, int] | None = None
        self._listener: Callable[[KeyEvent], None] | None = None
        self._thread: threading.Thread | None = None
        self._running = threading.Event()
        self._ready = threading.Event()
        self._start_error: Exception | None = None
        self._pressed: set[int] = set()
        self._key_filter: Callable[[KeyEvent], KeyDisposition] | None = None
        self._consumed_keys: set[int] = set()
        self._hold_lock = threading.Lock()
        self._holding = False
        self._held: list[NativeKeyEvent] = []
        self._pointer_epoch = 0
        self._hold_pointer_epoch = 0
        self._hold_window = 0
        self._deferred_action: NativeKeyEvent | None = None
        self._action_prior_keys: set[int] = set()
        # Keys down when a hold began: their releases pass, or the client repeats them meanwhile.
        self._hold_prior_keys: set[int] = set()
        # The last asked key and what was made of it, for a verdict the helper got too late.
        self._last_answer: tuple[int, str, NativeKeyEvent] | None = None
        # The last helper key handled, and the one handled when the current hold began.
        self._known_seq = 0
        self._hold_known_seq = 0
        # A key reached the application during this hold: a correction would edit unknown text.
        self._hold_broken = False
        # The withheld Enter/Tab reached the application: complete_action must not type it again.
        self._action_forced = False
        self._last_typed: tuple[int, int] = (0, 0)
        self._inject_lock = threading.Lock()
        self._input_delay = InputDelay()

    @property
    def running(self) -> bool:
        return self._running.is_set()

    @property
    def layouts(self) -> tuple[int, int]:
        if self._layouts is None:
            names = self._api.layout_names()
            self._layouts = select_layout_indices(len(names), self._api.translate_key)
        return self._layouts

    def probe(self) -> BackendProbe:
        display = self._api.desktop_name()
        try:
            denied = self._api.permission_error()
            if denied:
                return BackendProbe(
                    False,
                    SESSION_TYPE,
                    display,
                    "—",
                    "—",
                    "—",
                    -1,
                    denied,
                )
            names = self._api.layout_names()
            indices = self.layouts
            selected = tuple(
                names[idx] if 0 <= idx < len(names) else str(idx)
                for idx in indices
            )
            group = self.current_group()
            return BackendProbe(
                True,
                SESSION_TYPE,
                display,
                CAPTURE_METHOD,
                INJECT_METHOD,
                ",".join(selected),
                group,
            )
        except Exception as error:
            return BackendProbe(
                False,
                SESSION_TYPE,
                display,
                "—",
                "—",
                "—",
                -1,
                str(error),
            )

    def start(self, listener: Callable[[KeyEvent], None]) -> None:
        if self._running.is_set():
            return
        denied = self._api.permission_error()
        if denied:
            raise WaylandBackendError(denied)
        self.layouts
        self._listener = listener
        self._ready.clear()
        self._start_error = None
        self._thread = threading.Thread(
            target=self._listener_loop,
            name="keyswitch-wayland-input",
            daemon=True,
        )
        self._thread.start()
        if not self._ready.wait(KEYBOARD_LISTENER_START_TIMEOUT_SECONDS):
            self.stop()
            raise WaylandBackendError(
                f"Перехватчик клавиатуры Wayland не подтвердил запуск за "
                f"{quantity(KEYBOARD_LISTENER_START_TIMEOUT_SECONDS, SECONDS)}"
            )
        error = self._startup_error()
        if error is not None:
            self._thread = None
            raise WaylandBackendError(str(error)) from error

    def _startup_error(self) -> Exception | None:
        return self._start_error

    def _listener_loop(self) -> None:
        try:
            self._api.run_input_listener(self._handle_native, self._mark_ready)
        except Exception as error:
            self._start_error = error
            self._ready.set()
        finally:
            self._running.clear()

    def _mark_ready(self) -> None:
        self._running.set()
        self._ready.set()

    def stop(self) -> None:
        self._key_filter = None
        self.complete_action(False)
        thread = self._thread
        if thread is None:
            return
        self._api.stop_input_listener()
        if thread is not threading.current_thread():
            thread.join(timeout=KEYBOARD_LISTENER_STOP_TIMEOUT_SECONDS)
        self._thread = None
        self._running.clear()

    def close(self) -> None:
        self.stop()
        self._listener = None
        self._api.close()

    def __enter__(self) -> "WaylandBackend":
        return self

    def __exit__(self, *_args: object) -> None:
        self.close()

    def current_group(self) -> int:
        active = self._api.current_layout_index()
        indices = self.layouts
        return indices.index(active) if active in indices else -1

    def switch_group(self, group: int) -> None:
        if not 0 <= group < len(self.layouts):
            raise WaylandBackendError(f"Неизвестная группа раскладки {group}")
        with self._inject_lock:
            self._switch_group(group)

    def active_application(self) -> str:
        return self._api.active_application()

    def focused_window(self) -> FocusInfo | None:
        window = self._api.focused_window()
        if not window:
            return None
        pid = self._api.window_process_id(window)
        own = bool(pid) and pid == self._api.current_process_id()
        return FocusInfo(window, own, self._api.isolated_layout())

    def window_process_id(self, window: int) -> int:
        return self._api.window_process_id(window)

    def window_origin(self, window: int) -> tuple[int, int] | None:
        return self._api.window_origin(window)

    def window_is_password(self, window: int) -> bool:
        """Whether the compositor says the keyboard of ``window`` is in a password field."""

        return self._api.window_is_password(window)

    def input_anchor(self) -> ScreenAnchor | None:
        return self._api.input_anchor()

    def position_window(self, window: int, x: int, y: int) -> bool:
        return self._api.position_window(window, x, y)

    def restore_window(self, window: int | None) -> bool:
        return window is not None and window > 0 and self._api.activate_window(window)

    def take_input_delay(self) -> InputDelayReport:
        return self._input_delay.take()

    def set_key_filter(
        self, predicate: Callable[[KeyEvent], KeyDisposition] | None
    ) -> None:
        self._key_filter = predicate

    def _consumes(self, event: KeyEvent) -> KeyDisposition:
        if event.synthetic:
            return False
        if not event.pressed:
            if event.keycode not in self._consumed_keys:
                return False
            self._consumed_keys.discard(event.keycode)
            return True
        if event.keycode in self._consumed_keys:
            return True
        predicate = self._key_filter
        decision = predicate(event) if predicate is not None else False
        if not decision:
            return False
        self._consumed_keys.add(event.keycode)
        return decision

    def hold_input(self) -> None:
        with self._hold_lock:
            if not self._holding:
                self._hold_window = self._api.focused_window()
                self._hold_pointer_epoch = self._pointer_epoch
                self._hold_known_seq = self._known_seq
                self._hold_broken = False
                self._hold_prior_keys = set(self._pressed) - EVDEV_MODIFIER_KEYS
            self._holding = True

    def _pending_deferred_action(self) -> NativeKeyEvent | None:
        return self._deferred_action

    def release_input(self) -> int:
        if self._pending_deferred_action() is not None:
            return 0
        count = 0
        try:
            while True:
                with self._hold_lock:
                    if self._pending_deferred_action() is not None:
                        index = next(
                            (
                                idx
                                for idx, item in enumerate(self._held)
                                if not item.pressed and item.keycode in self._action_prior_keys
                            ),
                            None,
                        )
                        if index is None:
                            return count
                        item = self._held.pop(index)
                    elif self._held:
                        item = self._held.pop(0)
                    else:
                        self._holding = False
                        return count
                self._deliver_inputs(
                    (NativeInput(item.pressed, item.keycode, synthetic=False, replayed=True),)
                )
                count += 1
        finally:
            with self._hold_lock:
                if self._deferred_action is None:
                    self._holding = False
                    self._hold_prior_keys.clear()
                    self._hold_broken = False

    def complete_action(self, deliver: bool) -> int:
        action = self._deferred_action
        if action is None:
            if self._action_forced:
                # The helper typed the withheld key itself; what it held back still has to go.
                self._action_forced = False
                self.release_input()
            return 0
        try:
            if deliver:
                if (
                    self._pointer_epoch != self._hold_pointer_epoch
                    or self._api.focused_window() != self._hold_window
                ):
                    raise WaylandBackendError("Enter/Tab не передан: место ввода изменилось")
                self._deliver_inputs(
                    tuple(NativeInput(pressed, action.keycode) for pressed in (True, False)),
                    known_seq=action.seq or None,
                    action=True,
                )
        finally:
            self._deferred_action = None
            self._action_prior_keys.clear()
            self.release_input()
        return COMPLETED_ACTION_EVENT_COUNT if deliver else 0

    def _handle_native(self, native: NativeKeyEvent) -> bool:
        if native.input_reset:
            self._reset_input_state()
            return False
        if native.forced:
            self._settle_forced(native)
            self._known_seq = native.seq
            return False
        if native.pointer or native.repeat_reset or native.injected or native.replayed:
            return self._answer_native(native)
        started = time.perf_counter()
        late = (self._api.monotonic_ms() - native.timestamp) & UINT32_MODULO_MASK
        try:
            return self._answer_native(native)
        finally:
            # Handled, and handed to the engine's queue: a correction decided from here knows it.
            self._known_seq = native.seq
            callback = round((time.perf_counter() - started) * MILLISECONDS_PER_SECOND)
            if late <= UINT32_MODULO_MASK >> 1:
                self._input_delay.record(late, callback)

    def _settle_forced(self, native: NativeKeyEvent) -> None:
        """The helper typed an asked key itself, its verdict too late: undo what the verdict assumed.

        The key is in the application. Held, it leaves the hold and reaches the engine as typed; a
        consumed hotkey lets its release through; a withheld Enter/Tab is not typed a second time.
        A correction of this hold can no longer know the text and is refused.
        """

        last = self._last_answer
        if last is None or last[0] != native.seq:
            return
        _seq, kind, original = last
        self._last_answer = None
        if kind == "held":
            with self._hold_lock:
                index = next((idx for idx, item in enumerate(self._held) if item is original), None)
                if index is None:
                    return
                del self._held[index]
                self._hold_broken = True
            self._answer_native(replace(original, delivered=True))
        elif kind == "consumed":
            self._consumed_keys.discard(original.keycode)
            LOGGER.info("Ответ на клавишу опоздал: она дошла до приложения вместе с действием KeySwitch")
        elif kind == "deferred":
            self._consumed_keys.discard(original.keycode)
            with self._hold_lock:
                if self._deferred_action is original:
                    self._deferred_action = None
                    self._action_prior_keys.clear()
                    self._action_forced = True
                    self._hold_broken = True
            LOGGER.info("Ответ на Enter/Tab опоздал: клавиша дошла до приложения до замены")

    def _reset_input_state(self) -> None:
        """The grab changed: the keys believed held down and swallowed are unknown now."""

        self._pressed.clear()
        self._consumed_keys.clear()
        self._hold_prior_keys.clear()
        with self._hold_lock:
            if self._holding:
                self._hold_broken = True
        self._answer_native(NativeKeyEvent(True, 0, self._api.monotonic_ms(), repeat_reset=True))

    def _answer_native(self, native: NativeKeyEvent) -> bool:
        if native.pointer or native.repeat_reset:
            if native.pointer:
                self._pointer_epoch += 1
            listener = self._listener
            if listener is not None:
                listener(
                    KeyEvent(
                        True,
                        0,
                        POINTER_KEY_NAME,
                        "",
                        ("", ""),
                        POINTER_EVENT_GROUP,
                        0,
                        native.timestamp,
                    )
                )
            return False

        if native.delivered:
            with self._hold_lock:
                if self._holding:
                    self._hold_broken = True
            self._consumed_keys.discard(native.keycode)
        elif not native.injected and not native.replayed:
            with self._hold_lock:
                prior_release = not native.pressed and native.keycode in self._action_prior_keys
                hold_prior = (
                    not native.pressed
                    and native.keycode in self._hold_prior_keys
                    and not any(item.pressed and item.keycode == native.keycode for item in self._held)
                )
                action_key = (
                    self._deferred_action is not None and native.keycode in self._consumed_keys
                )
                if self._holding and not prior_release and not hold_prior and not action_key:
                    self._held.append(native)
                    self._last_answer = (native.seq, "held", native)
                    return True
                self._hold_prior_keys.discard(native.keycode)

        if not native.injected and not native.replayed:
            self._recheck_modifiers(native)
        self._track_key_state(native)

        state = self._normalized_state()
        indices = self.layouts
        characters = tuple(
            self._api.translate_key(native.keycode, state, layout_idx)
            for layout_idx in indices
        )
        group = self.current_group()
        character = characters[group] if 0 <= group < len(characters) else ""
        plain = self._api.translate_key(native.keycode, 0, indices[0])
        sym = self._api.keysym_name(native.keycode, indices[0])
        event = KeyEvent(
            native.pressed,
            native.keycode,
            key_name(native.keycode, sym, plain),
            character,
            characters,
            group,
            state,
            native.timestamp,
            native.injected,
        )
        repeated_answer = (
            not event.synthetic and event.pressed and event.keycode in self._consumed_keys
        )
        # A key typed without asking has reached the window already: nothing can be withheld.
        consumed = False if native.delivered else self._consumes(event)
        if consumed == "defer":
            self.hold_input()
            self._deferred_action = native
            self._action_prior_keys = set(self._pressed)
            event = replace(event, deferred=True)
        if native.seq and not native.delivered:
            kind = "deferred" if consumed == "defer" else "consumed" if consumed else "passed"
            self._last_answer = (native.seq, kind, native)
        listener = self._listener
        if listener is not None and not repeated_answer:
            listener(event)
        return bool(consumed)

    def _recheck_modifiers(self, native: NativeKeyEvent) -> None:
        window = self._api.focused_window()
        previous_time, previous_window = self._last_typed
        self._last_typed = (native.timestamp, window)
        held = (self._pressed & EVDEV_MODIFIER_KEYS) - {native.keycode}
        if not held or (
            window == previous_window
            and (native.timestamp - previous_time) & UINT32_MODULO_MASK < MODIFIER_RECHECK_GAP_MS
        ):
            return
        released = sorted(key for key in held if not self._api.key_down(key))
        if released:
            self._pressed.difference_update(released)
            LOGGER.info(
                "Отпускание модификаторов не дошло до перехватчика, они забыты: %s",
                ", ".join(key_name(key) for key in released),
            )

    def _track_key_state(self, native: NativeKeyEvent) -> None:
        if native.pressed:
            self._pressed.add(native.keycode)
            return
        self._pressed.discard(native.keycode)
        with self._hold_lock:
            if native.keycode in self._action_prior_keys and any(
                item.pressed and item.keycode == native.keycode for item in self._held
            ):
                self._held.append(replace(native, replayed=False))
        self._action_prior_keys.discard(native.keycode)

    def _normalized_state(self) -> int:
        state = LOCK_MASK if self._api.caps_lock_enabled() else 0
        if self._pressed & EVDEV_SHIFT_KEYS:
            state |= SHIFT_MASK
        if self._pressed & EVDEV_CONTROL_KEYS:
            state |= CONTROL_MASK
        if self._pressed & EVDEV_ALT_KEYS:
            state |= ALT_MASK
        if self._pressed & EVDEV_SUPER_KEYS:
            state |= SUPER_MASK
        return state

    def inject_correction(
        self,
        strokes: Iterable[KeyEvent],
        target_group: int,
        boundary: KeyEvent | None,
        source_group: int | None = None,
        late: Sequence[KeyEvent] = (),
        trailing: Sequence[KeyEvent] = (),
        kept_tail: int = 0,
    ) -> int:
        try:
            return self._inject_correction(
                strokes, target_group, boundary, source_group, late, trailing, kept_tail
            )
        finally:
            self.release_input()

    def _inject_correction(
        self,
        strokes: Iterable[KeyEvent],
        target_group: int,
        boundary: KeyEvent | None,
        source_group: int | None,
        late: Sequence[KeyEvent],
        trailing: Sequence[KeyEvent],
        kept_tail: int = 0,
    ) -> int:
        if not 0 <= target_group < len(self.layouts):
            raise WaylandBackendError(f"Неизвестная группа раскладки {target_group}")
        stroke_list = list(strokes)
        late_list = list(late)
        literal = tuple(trailing) + (() if boundary is None else (boundary,))
        rendered_source_group = (
            source_group
            if source_group is not None
            else stroke_list[0].group if stroke_list else target_group
        )
        if not 0 <= rendered_source_group < len(self.layouts):
            raise WaylandBackendError(
                f"Неизвестная исходная группа раскладки {rendered_source_group}"
            )
        if kept_tail < 0 or (
            kept_tail and (kept_tail >= len(stroke_list) or source_group is None)
        ):
            raise WaylandBackendError("Некорректная граница сохраняемого слова; замена отменена")

        final_group = rendered_source_group if kept_tail else target_group
        delete_count = len(stroke_list) + len(literal) + len(late_list)
        delete_inputs = tuple(
            NativeInput(pressed, EVDEV_KEY_BACKSPACE)
            for _ in range(delete_count)
            for pressed in (True, False)
        )
        converted = stroke_list[: len(stroke_list) - kept_tail]
        replay_inputs = tuple(
            item
            for stroke in converted
            for item in self._stroke_inputs(stroke, group=target_group)
        )
        kept_inputs = tuple(
            item
            for stroke in stroke_list[len(converted) :]
            for item in self._stroke_inputs(stroke, group=final_group)
        )
        boundary_inputs = tuple(
            item for stroke in literal for item in self._stroke_inputs(stroke)
        )
        late_inputs = tuple(
            item
            for stroke in late_list
            for item in self._stroke_inputs(stroke, synthetic=False, group=final_group)
        )
        preserve_boundary_layout = any(
            stroke.character_for(target_group) != stroke.character for stroke in literal
        )
        literal_group = literal[0].group if literal else rendered_source_group
        if not 0 <= literal_group < len(self.layouts) or any(
            stroke.group != literal_group for stroke in literal
        ):
            raise WaylandBackendError("Некорректная раскладка пунктуации; замена отменена")
        # A sign the target layout has on another key is typed there: switching back and forth
        # around it races the keys still on their way to the client.
        literal_in_target = (
            self._typed_in_group(literal, target_group)
            if preserve_boundary_layout and not kept_tail
            else None
        )
        if literal_in_target is not None:
            boundary_inputs = literal_in_target
            preserve_boundary_layout = False

        held_shifts = sorted(self._pressed & EVDEV_SHIFT_KEYS)
        unshift_inputs = tuple(NativeInput(False, code) for code in held_shifts)
        reshift_inputs = tuple(NativeInput(True, code) for code in held_shifts)

        late_deleted = False
        late_typed = False
        failure: Exception | None = None
        known_seq = self._hold_known_seq if self._holding else self._known_seq
        with self._inject_lock:
            try:
                if self._hold_broken:
                    raise WaylandBackendError(INPUT_AHEAD_ERROR)
                self._switch_group(target_group)
                if self._holding and (
                    self._pointer_epoch != self._hold_pointer_epoch
                    or self._api.focused_window() != self._hold_window
                ):
                    raise WaylandBackendError("Место ввода изменилось до замены; текст не изменён")
                self._deliver_inputs(
                    unshift_inputs
                    + delete_inputs
                    + replay_inputs
                    + (() if preserve_boundary_layout or kept_tail else boundary_inputs),
                    known_seq=known_seq,
                )
                late_deleted = True
                if kept_tail:
                    self._switch_group_after_typing(final_group)
                    self._deliver_inputs(kept_inputs + boundary_inputs)
                elif preserve_boundary_layout:
                    self._switch_group_after_typing(literal_group)
                    self._deliver_inputs(boundary_inputs)
                    self._switch_group_after_typing(target_group)
                late_typed = True
                self._deliver_inputs(late_inputs + reshift_inputs)
            except Exception as error:
                failure = error
            restore = (
                (late_inputs + reshift_inputs)
                if late_deleted and not late_typed
                else ()
            )
            held_count = 0
            try:
                self._deliver_inputs(restore)
                held_count = self.release_input()
            except Exception as error:
                failure = failure or error
        if failure is not None:
            raise failure
        return held_count

    def _stroke_inputs(
        self, stroke: KeyEvent, *, synthetic: bool = True, group: int | None = None
    ) -> tuple[NativeInput, ...]:
        result: list[NativeInput] = []
        rendered_group = stroke.group if group is None else group
        shifted = stroke.shift
        if stroke.character_for(rendered_group).isalpha():
            shifted ^= stroke.caps_lock != self._api.caps_lock_enabled()
        replayed = not synthetic
        if shifted:
            result.append(
                NativeInput(True, EVDEV_KEY_LEFTSHIFT, synthetic=synthetic, replayed=replayed)
            )
        result.extend((
            NativeInput(True, stroke.keycode, synthetic=synthetic, replayed=replayed),
            NativeInput(False, stroke.keycode, synthetic=synthetic, replayed=replayed),
        ))
        if shifted:
            result.append(
                NativeInput(False, EVDEV_KEY_LEFTSHIFT, synthetic=synthetic, replayed=replayed)
            )
        return tuple(result)

    def _typed_in_group(
        self, strokes: Sequence[KeyEvent], group: int
    ) -> tuple[NativeInput, ...] | None:
        """The keys that type the characters of ``strokes`` in ``group``, or None if one is missing there."""

        layout_index = self.layouts[group]
        lock = LOCK_MASK if self._api.caps_lock_enabled() else 0
        candidates = range(1, WAYLAND_UINPUT_MAX_KEYCODE + 1)
        result: list[NativeInput] = []
        for stroke in strokes:
            found = next(
                (
                    (code, shifted)
                    for code in (stroke.keycode, *candidates)
                    for shifted in (False, True)
                    if stroke.character
                    and self._api.translate_key(
                        code, lock | (SHIFT_MASK if shifted else 0), layout_index
                    ) == stroke.character
                ),
                None,
            )
            if found is None:
                return None
            code, shifted = found
            if shifted:
                result.append(NativeInput(True, EVDEV_KEY_LEFTSHIFT))
            result.extend((NativeInput(True, code), NativeInput(False, code)))
            if shifted:
                result.append(NativeInput(False, EVDEV_KEY_LEFTSHIFT))
        return tuple(result)

    def _switch_group_after_typing(self, group: int) -> None:
        """Switch the layout once the keys just typed had time to reach the client in the old one."""

        if self.current_group() != group:
            time.sleep(WAYLAND_TYPED_INPUT_SETTLE_SECONDS)
        self._switch_group(group)

    def _switch_group(self, group: int) -> None:
        if self.current_group() == group:
            return
        target_index = self.layouts[group]
        if not self._api.select_layout_index(target_index):
            raise WaylandBackendError("Среда Wayland отклонила запрос смены раскладки")
        deadline = time.monotonic() + LAYOUT_SWITCH_TIMEOUT_SECONDS
        while time.monotonic() < deadline:
            if self.current_group() == group:
                return
            time.sleep(LAYOUT_SWITCH_POLL_SECONDS)
        raise WaylandLayoutSwitchUnconfirmed(
            "Среда Wayland не подтвердила смену раскладки", group
        )

    def _deliver_inputs(
        self, inputs: tuple[NativeInput, ...], *, known_seq: int | None = None, action: bool = False
    ) -> None:
        """Type ``inputs``; with ``known_seq`` the first batch is typed only if no key reached the
        application unasked after that seq, or for an ``action`` the key of that seq is not there
        already (the helper checks; nothing is typed otherwise)."""

        if not inputs:
            return
        timestamp = self._api.monotonic_ms()
        idx = 0
        total = len(inputs)
        while idx < total:
            current = inputs[idx]
            if current.synthetic and not current.replayed:
                end = idx + 1
                while end < total and inputs[end].synthetic and not inputs[end].replayed:
                    end += 1
                batch = inputs[idx:end]
                raw = tuple((item.pressed, item.keycode) for item in batch)
                posted = self._api.post_inputs(raw, known_seq, action)
                if known_seq is not None and posted == 0:
                    raise WaylandBackendError(INPUT_AHEAD_ERROR)
                known_seq = None
                if posted != len(raw):
                    raise WaylandBackendError(
                        f"uinput принял {posted} из {len(raw)} событий ввода"
                    )
                for item in batch:
                    self._handle_native(
                        NativeKeyEvent(
                            item.pressed,
                            item.keycode,
                            timestamp,
                            injected=True,
                        )
                    )
                idx = end
            else:
                swallow = self._handle_native(
                    NativeKeyEvent(
                        current.pressed,
                        current.keycode,
                        timestamp,
                        injected=False,
                        replayed=True,
                    )
                )
                if not swallow:
                    posted = self._api.post_inputs(((current.pressed, current.keycode),))
                    if posted != 1:
                        raise WaylandBackendError(
                            f"uinput принял {posted} из 1 события ввода"
                        )
                idx += 1
