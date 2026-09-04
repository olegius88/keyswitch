"""Windows keyboard backend built around a small, testable native API."""

from __future__ import annotations

import sys
import threading
import time
from collections.abc import Callable, Iterable, Sequence
from dataclasses import dataclass, replace
from typing import Protocol

from .backend import (
    ALT_MASK,
    CONTROL_MASK,
    LOCK_MASK,
    SHIFT_MASK,
    SUPER_MASK,
    BackendProbe,
    FocusInfo,
    KeyEvent,
    KeyDisposition,
    ScreenAnchor,
)


LANG_ENGLISH = 0x09
LANG_RUSSIAN = 0x19
LAYOUT_SWITCH_TIMEOUT = 0.5
HOOK_START_TIMEOUT = 5.0

VK_BACK = 0x08
VK_TAB = 0x09
VK_RETURN = 0x0D
VK_SHIFT = 0x10
VK_CONTROL = 0x11
VK_MENU = 0x12
VK_PAUSE = 0x13
VK_CAPITAL = 0x14
VK_ESCAPE = 0x1B
VK_SPACE = 0x20
VK_PRIOR = 0x21
VK_NEXT = 0x22
VK_END = 0x23
VK_HOME = 0x24
VK_LEFT = 0x25
VK_UP = 0x26
VK_RIGHT = 0x27
VK_DOWN = 0x28
VK_INSERT = 0x2D
VK_DELETE = 0x2E
VK_LWIN = 0x5B
VK_RWIN = 0x5C
VK_LSHIFT = 0xA0
VK_RSHIFT = 0xA1
VK_LCONTROL = 0xA2
VK_RCONTROL = 0xA3
VK_LMENU = 0xA4
VK_RMENU = 0xA5
# Punctuation keys of the US layout; named after the X11 keysyms so the log
# reads the same on both platforms instead of showing "VK_BC" for a comma.
VK_OEM_1 = 0xBA
VK_OEM_PLUS = 0xBB
VK_OEM_COMMA = 0xBC
VK_OEM_MINUS = 0xBD
VK_OEM_PERIOD = 0xBE
VK_OEM_2 = 0xBF
VK_OEM_3 = 0xC0
VK_OEM_4 = 0xDB
VK_OEM_5 = 0xDC
VK_OEM_6 = 0xDD
VK_OEM_7 = 0xDE

SHIFT_KEYS = frozenset((VK_SHIFT, VK_LSHIFT, VK_RSHIFT))
CONTROL_KEYS = frozenset((VK_CONTROL, VK_LCONTROL, VK_RCONTROL))
ALT_KEYS = frozenset((VK_MENU, VK_LMENU, VK_RMENU))
SUPER_KEYS = frozenset((VK_LWIN, VK_RWIN))


def _running_on_windows() -> bool:
    """Keep the runtime guard testable without platform-based type narrowing."""

    return sys.platform == "win32"


class WindowsBackendError(RuntimeError):
    pass


@dataclass(frozen=True)
class NativeKeyEvent:
    pressed: bool
    virtual_key: int
    scan_code: int
    extended: bool
    injected: bool
    timestamp: int
    replayed: bool = False


@dataclass(frozen=True)
class NativeInput:
    pressed: bool
    virtual_key: int = 0
    scan_code: int = 0
    extended: bool = False
    synthetic: bool = True
    replayed: bool = False


class WindowsAPI(Protocol):
    def loaded_layouts(self) -> tuple[int, ...]: ...

    def foreground_layout(self) -> int: ...

    def request_layout(self, layout: int) -> bool: ...

    def translate_key(
        self,
        virtual_key: int,
        scan_code: int,
        state: int,
        layout: int,
    ) -> str: ...

    def send_inputs(self, inputs: tuple[NativeInput, ...]) -> int: ...

    def active_application(self) -> str: ...

    def foreground_window(self) -> int: ...

    def focused_control(self) -> int: ...

    def window_process_id(self, window: int) -> int: ...

    def current_process_id(self) -> int: ...

    def input_anchor(self) -> ScreenAnchor | None: ...

    def activate_window(self, window: int) -> bool: ...

    def keep_window_inactive(self, window: int) -> bool: ...

    def caps_lock_enabled(self) -> bool: ...

    def run_keyboard_hook(
        self,
        listener: Callable[[NativeKeyEvent], bool],
        ready: Callable[[], None],
    ) -> None: ...

    def stop_keyboard_hook(self) -> None: ...


KEY_NAMES = {
    VK_BACK: "BackSpace",
    VK_TAB: "Tab",
    VK_RETURN: "Return",
    VK_SHIFT: "Shift_L",
    VK_CONTROL: "Control_L",
    VK_MENU: "Alt_L",
    VK_PAUSE: "Pause",
    VK_CAPITAL: "Caps_Lock",
    VK_ESCAPE: "Escape",
    VK_SPACE: "space",
    VK_PRIOR: "Page_Up",
    VK_NEXT: "Page_Down",
    VK_END: "End",
    VK_HOME: "Home",
    VK_LEFT: "Left",
    VK_UP: "Up",
    VK_RIGHT: "Right",
    VK_DOWN: "Down",
    VK_INSERT: "Insert",
    VK_DELETE: "Delete",
    VK_LWIN: "Super_L",
    VK_RWIN: "Super_R",
    VK_LSHIFT: "Shift_L",
    VK_RSHIFT: "Shift_R",
    VK_LCONTROL: "Control_L",
    VK_RCONTROL: "Control_R",
    VK_LMENU: "Alt_L",
    VK_RMENU: "Alt_R",
    VK_OEM_1: "semicolon",
    VK_OEM_PLUS: "equal",
    VK_OEM_COMMA: "comma",
    VK_OEM_MINUS: "minus",
    VK_OEM_PERIOD: "period",
    VK_OEM_2: "slash",
    VK_OEM_3: "grave",
    VK_OEM_4: "bracketleft",
    VK_OEM_5: "backslash",
    VK_OEM_6: "bracketright",
    VK_OEM_7: "apostrophe",
}


def primary_language(layout: int) -> int:
    return (layout & 0xFFFF) & 0x03FF


def select_layout_pair(layouts: Iterable[int]) -> tuple[int, int]:
    unique = tuple(dict.fromkeys(layouts))
    english = next(
        (layout for layout in unique if primary_language(layout) == LANG_ENGLISH),
        0,
    )
    russian = next(
        (layout for layout in unique if primary_language(layout) == LANG_RUSSIAN),
        0,
    )
    if not english or not russian:
        raise WindowsBackendError(
            "В Windows должны быть установлены английская и русская раскладки"
        )
    return english, russian


def key_name(virtual_key: int) -> str:
    if virtual_key in KEY_NAMES:
        return KEY_NAMES[virtual_key]
    if 0x30 <= virtual_key <= 0x39 or 0x41 <= virtual_key <= 0x5A:
        return chr(virtual_key).casefold()
    return f"VK_{virtual_key:02X}"


class WindowsBackend:
    """Observe global Win32 keyboard events and inject physical corrections."""

    def __init__(self, api: WindowsAPI | None = None) -> None:
        if api is None:
            if not _running_on_windows():
                raise WindowsBackendError("Win32 backend доступен только в Windows")
            from .windows_native import CtypesWindowsAPI

            api = CtypesWindowsAPI()
        self._api = api
        self._layouts: tuple[int, int] | None = None
        self._listener: Callable[[KeyEvent], None] | None = None
        self._thread: threading.Thread | None = None
        self._running = threading.Event()
        self._ready = threading.Event()
        self._start_error: Exception | None = None
        self._pressed: set[int] = set()
        self._key_filter: Callable[[KeyEvent], KeyDisposition] | None = None
        self._consumed_keys: set[int] = set()
        # While a correction is being injected the user's own keys are kept
        # back here and typed again afterwards, so they can neither land
        # between the backspaces and the replacement nor be lost.
        self._hold_lock = threading.Lock()
        self._holding = False
        self._held: list[NativeKeyEvent] = []
        self._caps_lock = False
        self._pointer_epoch = 0
        self._hold_pointer_epoch = 0
        self._hold_window = 0
        self._deferred_action: NativeKeyEvent | None = None
        self._action_prior_keys: set[int] = set()
        self._inject_lock = threading.Lock()

    @property
    def running(self) -> bool:
        return self._running.is_set()

    @property
    def layouts(self) -> tuple[int, int]:
        if self._layouts is None:
            self._layouts = select_layout_pair(self._api.loaded_layouts())
        return self._layouts

    def probe(self) -> BackendProbe:
        try:
            layouts = self.layouts
            group = self._group_for_layout(self._api.foreground_layout())
            layout_names = ",".join(f"{layout & 0xFFFFFFFF:08X}" for layout in layouts)
            return BackendProbe(
                True,
                "windows",
                "Win32 desktop",
                "WH_KEYBOARD_LL",
                "SendInput",
                f"HKL {layout_names}",
                group,
            )
        except Exception as error:
            return BackendProbe(
                False,
                "windows",
                "Win32 desktop",
                "—",
                "—",
                "—",
                -1,
                str(error),
            )

    def start(self, listener: Callable[[KeyEvent], None]) -> None:
        if self._running.is_set():
            return
        self.layouts
        self._listener = listener
        self._caps_lock = self._api.caps_lock_enabled()
        self._ready.clear()
        self._start_error = None
        self._thread = threading.Thread(
            target=self._hook_loop,
            name="keyswitch-win32-hook",
            daemon=True,
        )
        self._thread.start()
        if not self._ready.wait(HOOK_START_TIMEOUT):
            self.stop()
            raise WindowsBackendError("Win32 hook не подтвердил запуск за 5 секунд")
        error = self._startup_error()
        if error is not None:
            self._thread = None
            raise WindowsBackendError(str(error)) from error

    def _startup_error(self) -> Exception | None:
        """Read the error set by the hook thread after its ready signal."""

        return self._start_error

    def _hook_loop(self) -> None:
        try:
            self._api.run_keyboard_hook(self._handle_native, self._mark_ready)
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
        self._api.stop_keyboard_hook()
        if thread is not threading.current_thread():
            thread.join(timeout=2.0)
        self._thread = None
        self._running.clear()

    def close(self) -> None:
        self.stop()
        self._listener = None

    def current_group(self) -> int:
        return self._group_for_layout(self._api.foreground_layout())

    def switch_group(self, group: int) -> None:
        if not 0 <= group < len(self.layouts):
            raise WindowsBackendError(f"Неизвестная группа раскладки {group}")
        with self._inject_lock:
            self._switch_group(group)

    def _group_for_layout(self, layout: int) -> int:
        try:
            return self.layouts.index(layout)
        except ValueError:
            language = primary_language(layout)
            for index, candidate in enumerate(self.layouts):
                if primary_language(candidate) == language:
                    return index
            return -1

    def active_application(self) -> str:
        return self._api.active_application()

    def focused_window(self) -> FocusInfo | None:
        window = self._api.foreground_window()
        if not window:
            return None
        process_id = self._api.window_process_id(window)
        own = bool(process_id) and process_id == self._api.current_process_id()
        return FocusInfo(self._api.focused_control() or window, own, isolated_layout=own)

    def input_anchor(self) -> ScreenAnchor | None:
        return self._api.input_anchor()

    def restore_window(self, window: int | None) -> bool:
        return window is not None and self._api.activate_window(window)

    def keep_window_inactive(self, window: int) -> bool:
        return self._api.keep_window_inactive(window)

    def set_key_filter(
        self, predicate: Callable[[KeyEvent], KeyDisposition] | None
    ) -> None:
        """Decide, inside the hook, which keys must not reach the window."""

        self._key_filter = predicate

    def _consumes(self, event: KeyEvent) -> KeyDisposition:
        """Ask the filter about a press, and hide the matching release.

        A window that saw no key-down must not receive the key-up either, so
        the release of a swallowed key is swallowed with it.
        """

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
        """Keep the user's keys back until the next correction has landed."""

        with self._hold_lock:
            if not self._holding:
                self._hold_window = self._api.focused_control() or self._api.foreground_window()
                self._hold_pointer_epoch = self._pointer_epoch
            self._holding = True

    def release_input(self) -> int:
        """Replay held events before allowing fresh physical input through.

        Replays have their own native marker: they reach the engine but must
        never enter this buffer again. Never hold the mutex across SendInput,
        which calls the hook synchronously on another thread.
        """

        active = self._deferred_action
        if active is not None:
            return 0
        count = 0
        try:
            while True:
                with self._hold_lock:
                    if self._deferred_action is not None:
                        # A replayed second Enter starts the next transaction.
                        # Let its preceding key-ups reach the worker, retaining
                        # all following text until that action is completed.
                        index = next((
                            index for index, item in enumerate(self._held)
                            if not item.pressed and item.virtual_key in self._action_prior_keys
                        ), None)
                        if index is None:
                            return count
                        item = self._held.pop(index)
                    elif self._held:
                        item = self._held.pop(0)
                    else:
                        self._holding = False
                        return count
                self._send_exact((NativeInput(
                    item.pressed, virtual_key=item.virtual_key,
                    scan_code=item.scan_code, extended=item.extended,
                    synthetic=False, replayed=True,
                ),))
                count += 1
        finally:
            # An OS injection failure must never leave the keyboard captured.
            with self._hold_lock:
                if self._deferred_action is None:
                    self._holding = False

    def complete_action(self, deliver: bool) -> int:
        """Deliver a withheld Enter/Tab once, before releasing subsequent input."""

        action = self._deferred_action
        if action is None:
            return 0
        try:
            if deliver:
                if (
                    self._pointer_epoch != self._hold_pointer_epoch
                    or (self._api.focused_control() or self._api.foreground_window()) != self._hold_window
                ):
                    raise WindowsBackendError("Enter/Tab не передан: место ввода изменилось")
                self._send_exact(tuple(
                    NativeInput(pressed, virtual_key=action.virtual_key,
                                scan_code=action.scan_code, extended=action.extended)
                    for pressed in (True, False)
                ))
        finally:
            self._deferred_action = None
            self._action_prior_keys.clear()
            self.release_input()
        return 2 if deliver else 0

    def _handle_native(self, native: NativeKeyEvent) -> bool:
        if native.virtual_key == 0:
            self._pointer_epoch += 1
            if self._listener is not None:
                self._listener(KeyEvent(True, 0, "Pointer", "", ("", ""), -1, 0, native.timestamp))
            return False
        if not native.injected and not native.replayed:
            with self._hold_lock:
                prior_release = not native.pressed and native.virtual_key in self._action_prior_keys
                action_key = self._deferred_action is not None and native.scan_code in self._consumed_keys
                if self._holding and not prior_release and not action_key:
                    self._held.append(native)
                    return True
        if native.pressed:
            if native.virtual_key == VK_CAPITAL and native.virtual_key not in self._pressed:
                self._caps_lock = not self._caps_lock
            self._pressed.add(native.virtual_key)
        else:
            self._pressed.discard(native.virtual_key)
            with self._hold_lock:
                if native.virtual_key in self._action_prior_keys and any(
                    item.pressed and item.virtual_key == native.virtual_key
                    for item in self._held
                ):
                    # An auto-repeat after Enter belongs to the held next
                    # input. Its key-up also has to follow that replay.
                    self._held.append(replace(native, replayed=False))
            self._action_prior_keys.discard(native.virtual_key)
        state = self._normalized_state()
        characters = tuple(
            self._api.translate_key(
                native.virtual_key,
                native.scan_code,
                state,
                layout,
            )
            for layout in self.layouts
        )
        group = self.current_group()
        character = characters[group] if 0 <= group < len(characters) else ""
        event = KeyEvent(
            native.pressed,
            native.scan_code,
            key_name(native.virtual_key),
            character,
            characters,
            group,
            state,
            native.timestamp,
            native.injected,
        )
        repeated_answer = not event.synthetic and event.pressed and event.keycode in self._consumed_keys
        consumed = self._consumes(event)
        if consumed == "defer":
            self.hold_input()
            self._deferred_action = native
            self._action_prior_keys = set(self._pressed)
            event = replace(event, deferred=True)
        listener = self._listener
        if listener is not None and not repeated_answer:
            listener(event)
        return bool(consumed)

    def _normalized_state(self) -> int:
        state = LOCK_MASK if self._caps_lock else 0
        if self._pressed & SHIFT_KEYS:
            state |= SHIFT_MASK
        if self._pressed & CONTROL_KEYS:
            state |= CONTROL_MASK
        if self._pressed & ALT_KEYS:
            state |= ALT_MASK
        if self._pressed & SUPER_KEYS:
            state |= SUPER_MASK
        return state

    def inject_correction(
        self,
        strokes: Iterable[KeyEvent],
        target_group: int,
        boundary: KeyEvent | None,
        source_group: int | None = None,
        late: Sequence[KeyEvent] = (),
    ) -> int:
        """Replace the word and return how many held keys were typed again.

        ``late`` are keys the user typed after the word but before this call:
        their characters already follow the word on screen, so they are
        deleted with it and typed again after the replacement, in the new
        layout. Keys arriving during the injection are held by the hook (see
        :meth:`hold_input`) and typed again last.
        """

        try:
            return self._inject_correction(strokes, target_group, boundary, source_group, late)
        finally:
            self.release_input()

    def _inject_correction(
        self, strokes: Iterable[KeyEvent], target_group: int,
        boundary: KeyEvent | None, source_group: int | None,
        late: Sequence[KeyEvent],
    ) -> int:
        if not 0 <= target_group < len(self.layouts):
            raise WindowsBackendError(f"Неизвестная группа раскладки {target_group}")
        stroke_list = list(strokes)
        late_list = list(late)
        rendered_source_group = (
            source_group
            if source_group is not None
            else stroke_list[0].group if stroke_list else target_group
        )
        if not 0 <= rendered_source_group < len(self.layouts):
            raise WindowsBackendError(
                f"Неизвестная исходная группа раскладки {rendered_source_group}"
            )
        delete_count = (
            len(stroke_list) + (1 if boundary is not None else 0) + len(late_list)
        )
        delete_inputs = tuple(
            NativeInput(pressed, virtual_key=VK_BACK)
            for _ in range(delete_count)
            for pressed in (True, False)
        )
        replay_inputs = tuple(
            item
            for stroke in stroke_list
            for item in self._stroke_inputs(stroke, group=target_group)
        )
        boundary_inputs = self._stroke_inputs(boundary) if boundary is not None else ()
        # Typed again as the user's own input: the engine must see these keys
        # as the start of the next word, not as its own injection.
        late_inputs = tuple(
            item
            for stroke in late_list
            for item in self._stroke_inputs(stroke, synthetic=False, group=target_group)
        )
        preserve_boundary_layout = bool(
            boundary is not None
            and rendered_source_group != target_group
            and boundary.character
            and boundary.character_for(target_group) != boundary.character
        )
        late_deleted = False
        late_typed = False
        failure: Exception | None = None
        with self._inject_lock:
            try:
                # The layout is switched first so that the deletion and the
                # replacement travel in one SendInput call: Windows keeps the
                # events of a single call together, but lets the user's keys
                # slip in between separate calls.
                self._switch_group(target_group)
                if self._holding and (
                    self._pointer_epoch != self._hold_pointer_epoch
                    or (self._api.focused_control() or self._api.foreground_window()) != self._hold_window
                ):
                    raise WindowsBackendError("Место ввода изменилось до замены; текст не изменён")
                self._send_exact(
                    delete_inputs
                    + replay_inputs
                    + (() if preserve_boundary_layout else boundary_inputs)
                )
                late_deleted = True
                if preserve_boundary_layout:
                    self._switch_group(rendered_source_group)
                    self._send_exact(boundary_inputs)
                    self._switch_group(target_group)
                # A partial send is not an all-or-nothing failure; retrying
                # this whole batch could duplicate an already delivered prefix.
                late_typed = True
                self._send_exact(late_inputs)
            except Exception as error:
                failure = error
            restore = late_inputs if late_deleted and not late_typed else ()
            held_count = 0
            try:
                self._send_exact(restore)
                held_count = self.release_input()
            except Exception as error:
                # The primary failure explains more than a failed restore.
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
            shifted ^= stroke.caps_lock != self._caps_lock
        if shifted:
            result.append(NativeInput(True, virtual_key=VK_SHIFT, synthetic=synthetic, replayed=not synthetic))
        result.extend(
            (
                NativeInput(True, scan_code=stroke.keycode, synthetic=synthetic, replayed=not synthetic),
                NativeInput(False, scan_code=stroke.keycode, synthetic=synthetic, replayed=not synthetic),
            )
        )
        if shifted:
            result.append(NativeInput(False, virtual_key=VK_SHIFT, synthetic=synthetic, replayed=not synthetic))
        return tuple(result)

    def _send_exact(self, inputs: tuple[NativeInput, ...]) -> None:
        if not inputs:
            return
        sent = self._api.send_inputs(inputs)
        if sent != len(inputs):
            raise WindowsBackendError(
                f"SendInput отправил {sent} из {len(inputs)} событий; результат текста неизвестен; возможно, целевое "
                "приложение запущено с правами администратора"
            )

    def _switch_group(self, group: int) -> None:
        layout = self.layouts[group]
        if self._group_for_layout(self._api.foreground_layout()) == group:
            return
        if not self._api.request_layout(layout):
            raise WindowsBackendError("Окно отклонило запрос смены раскладки")
        deadline = time.monotonic() + LAYOUT_SWITCH_TIMEOUT
        while time.monotonic() < deadline:
            if self._group_for_layout(self._api.foreground_layout()) == group:
                return
            time.sleep(0.01)
        raise WindowsBackendError("Приложение не подтвердило смену раскладки")
