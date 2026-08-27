"""Windows keyboard backend built around a small, testable native API."""

from __future__ import annotations

import sys
import threading
import time
from collections.abc import Callable, Iterable
from dataclasses import dataclass
from typing import Protocol

from .backend import (
    ALT_MASK,
    CONTROL_MASK,
    LOCK_MASK,
    SHIFT_MASK,
    SUPER_MASK,
    BackendProbe,
    KeyEvent,
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


@dataclass(frozen=True)
class NativeInput:
    pressed: bool
    virtual_key: int = 0
    scan_code: int = 0
    extended: bool = False
    synthetic: bool = True


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

    def caps_lock_enabled(self) -> bool: ...

    def run_keyboard_hook(
        self,
        listener: Callable[[NativeKeyEvent], None],
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
        self._caps_lock = False
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

    def _handle_native(self, native: NativeKeyEvent) -> None:
        if native.pressed:
            if native.virtual_key == VK_CAPITAL and native.virtual_key not in self._pressed:
                self._caps_lock = not self._caps_lock
            self._pressed.add(native.virtual_key)
        else:
            self._pressed.discard(native.virtual_key)
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
        listener = self._listener
        if listener is not None:
            listener(
                KeyEvent(
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
            )

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
    ) -> None:
        if not 0 <= target_group < len(self.layouts):
            raise WindowsBackendError(f"Неизвестная группа раскладки {target_group}")
        stroke_list = list(strokes)
        rendered_source_group = (
            source_group
            if source_group is not None
            else stroke_list[0].group if stroke_list else target_group
        )
        if not 0 <= rendered_source_group < len(self.layouts):
            raise WindowsBackendError(
                f"Неизвестная исходная группа раскладки {rendered_source_group}"
            )
        delete_count = len(stroke_list) + (1 if boundary is not None else 0)
        delete_inputs = tuple(
            NativeInput(pressed, virtual_key=VK_BACK)
            for _ in range(delete_count)
            for pressed in (True, False)
        )
        replay_inputs = tuple(
            item
            for stroke in stroke_list
            for item in self._stroke_inputs(stroke)
        )
        boundary_inputs = self._stroke_inputs(boundary) if boundary is not None else ()
        preserve_boundary_layout = bool(
            boundary is not None
            and rendered_source_group != target_group
            and boundary.character
            and boundary.character_for(target_group) != boundary.character
        )
        with self._inject_lock:
            self._send_exact(delete_inputs)
            self._switch_group(target_group)
            self._send_exact(replay_inputs)
            if preserve_boundary_layout:
                self._switch_group(rendered_source_group)
                self._send_exact(boundary_inputs)
                self._switch_group(target_group)
            else:
                self._send_exact(boundary_inputs)

    @staticmethod
    def _stroke_inputs(stroke: KeyEvent) -> tuple[NativeInput, ...]:
        result: list[NativeInput] = []
        if stroke.shift:
            result.append(NativeInput(True, virtual_key=VK_SHIFT))
        result.extend(
            (
                NativeInput(True, scan_code=stroke.keycode),
                NativeInput(False, scan_code=stroke.keycode),
            )
        )
        if stroke.shift:
            result.append(NativeInput(False, virtual_key=VK_SHIFT))
        return tuple(result)

    def _send_exact(self, inputs: tuple[NativeInput, ...]) -> None:
        if not inputs:
            return
        sent = self._api.send_inputs(inputs)
        if sent != len(inputs):
            raise WindowsBackendError(
                "SendInput не смог исправить текст; возможно, целевое "
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
