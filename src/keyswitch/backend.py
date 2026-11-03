"""Platform-neutral keyboard backend contracts."""

from __future__ import annotations

import inspect
from collections.abc import Callable, Iterable, Sequence
from dataclasses import dataclass
from typing import Literal, Protocol, cast
from .constants.keyboard import (
    ALT_MASK,
    CONTROL_MASK,
    LOCK_MASK as LOCK_MASK,
    SHIFT_MASK as SHIFT_MASK,
    SUPER_MASK,
)


KeyDisposition = bool | Literal["defer"]


class LayoutSwitchUnconfirmed(RuntimeError):
    """The layout was asked for, but the application did not confirm it in time.

    The request stays posted: the layout may still arrive, and when it does it is the
    engine's switch, not the user's.
    """

    def __init__(self, message: str, group: int) -> None:
        super().__init__(message)
        self.group = group


@dataclass(frozen=True)
class KeyEvent:
    pressed: bool
    keycode: int
    key_name: str
    character: str
    characters: tuple[str, ...]
    group: int
    state: int
    timestamp: int
    synthetic: bool = False
    deferred: bool = False
    # Injected by another program (remote control, a macro, an on-screen keyboard),
    # not typed on this keyboard; only the Windows backend can tell.
    foreign: bool = False

    @property
    def shift(self) -> bool:
        return bool(self.state & SHIFT_MASK)

    @property
    def caps_lock(self) -> bool:
        return bool(self.state & LOCK_MASK)

    @property
    def control(self) -> bool:
        return bool(self.state & CONTROL_MASK)

    @property
    def alt(self) -> bool:
        return bool(self.state & ALT_MASK)

    @property
    def super_key(self) -> bool:
        return bool(self.state & SUPER_MASK)

    def character_for(self, group: int) -> str:
        return self.characters[group] if 0 <= group < len(self.characters) else ""


@dataclass(frozen=True)
class BackendProbe:
    available: bool
    session_type: str
    display: str
    record_version: str
    xtest_version: str
    xkb_version: str
    current_group: int
    error: str = ""


@dataclass(frozen=True)
class ScreenAnchor:
    """A screen position captured before a learning prompt takes focus."""

    x: int
    y: int
    window: int | None = None


@dataclass(frozen=True)
class FocusInfo:
    """The focused top-level window as the backend sees it.

    ``window`` is the platform identity (an HWND or an X window id). ``own``
    marks windows of KeySwitch itself (settings, the learning prompt), which
    never count as the user moving to another window, except while the test
    field of the settings window has the keyboard: the window reports that to
    the engine (KeySwitchEngine.set_test_field_focused). ``isolated_layout``
    says that the window carries a keyboard layout of its own that the user
    did not choose (Windows keeps one per window), so a layout change seen
    while it is focused must be ignored; on X11 the layout is global.
    """

    window: int
    own: bool = False
    isolated_layout: bool = False


class InputBackend(Protocol):
    """Operations required by the correction engine and diagnostics UI."""

    def start(self, listener: Callable[[KeyEvent], None]) -> None: ...

    def set_key_filter(
        self, predicate: Callable[[KeyEvent], KeyDisposition] | None
    ) -> None: ...

    def hold_input(self) -> None: ...

    def release_input(self) -> int: ...

    def complete_action(self, deliver: bool) -> int: ...

    def stop(self) -> None: ...

    def close(self) -> None: ...

    def current_group(self) -> int: ...

    def switch_group(self, group: int) -> None: ...

    def active_application(self) -> str: ...

    def focused_window(self) -> FocusInfo | None: ...

    def probe(self) -> BackendProbe: ...

    def inject_correction(
        self,
        strokes: Iterable[KeyEvent],
        target_group: int,
        boundary: KeyEvent | None,
        source_group: int | None = None,
        late: Sequence[KeyEvent] = (),
        trailing: Sequence[KeyEvent] = (),
        kept_tail: int = 0,
    ) -> int: ...


class ErasingInjection(Protocol):
    """`inject_correction` of a backend that can also erase another number of characters than it types.

    A typo correction types more or fewer keys than the word on screen has (`клвиатура` is `клавиатура`); `erased`
    says how many characters the word occupies. The platform backends (X11, Windows, macOS, Wayland) take it; a
    backend that does not is offered only corrections of the word's own length (erasing_injection).
    """

    def __call__(
        self,
        strokes: Iterable[KeyEvent],
        target_group: int,
        boundary: KeyEvent | None,
        source_group: int | None = None,
        late: Sequence[KeyEvent] = (),
        trailing: Sequence[KeyEvent] = (),
        kept_tail: int = 0,
        erased: int | None = None,
    ) -> int: ...


def erasing_injection(backend: InputBackend) -> ErasingInjection | None:
    """The backend's injection when it can erase a given number of characters (ErasingInjection), else None."""

    inject = backend.inject_correction
    return cast(ErasingInjection, inject) if "erased" in inspect.signature(inject).parameters else None
