"""Platform-neutral keyboard backend contracts."""

from __future__ import annotations

from collections.abc import Callable, Iterable
from dataclasses import dataclass
from typing import Protocol


# Internal modifier bits intentionally match the X11 core masks. Platform
# backends normalize their native state into these values before emitting an
# event, keeping the engine independent from an operating system API.
SHIFT_MASK = 1 << 0
LOCK_MASK = 1 << 1
CONTROL_MASK = 1 << 2
ALT_MASK = 1 << 3
SUPER_MASK = 1 << 6


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
    never count as the user moving to another window. ``isolated_layout``
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
    ) -> None: ...
