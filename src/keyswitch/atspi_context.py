"""Optional Linux AT-SPI text access with bounded traversal and IPC timeouts."""

from __future__ import annotations

import importlib
import time
from collections.abc import Callable
from functools import lru_cache
from pathlib import Path
from typing import Protocol, cast

from .input_context import CONTEXT_LIMIT, FieldContext


class _States(Protocol):
    def contains(self, state: object) -> bool: ...


class _Text(Protocol):
    def get_caret_offset(self) -> int: ...
    def get_character_count(self) -> int: ...
    def get_n_selections(self) -> int: ...


class _TextApi(Protocol):
    def get_text(self, text: _Text, start: int, end: int) -> str: ...


class _Accessible(Protocol):
    def clear_cache(self) -> None: ...
    def get_child_count(self) -> int: ...
    def get_child_at_index(self, index: int) -> _Accessible | None: ...
    def get_name(self) -> str: ...
    def get_process_id(self) -> int: ...
    def get_role(self) -> object: ...
    def get_state_set(self) -> _States: ...
    def get_text_iface(self) -> _Text | None: ...


class _StateTypes(Protocol):
    @property
    def FOCUSED(self) -> object: ...


class _Roles(Protocol):
    @property
    def PASSWORD_TEXT(self) -> object: ...


class _Atspi(Protocol):
    @property
    def Text(self) -> _TextApi: ...
    @property
    def StateType(self) -> _StateTypes: ...
    @property
    def Role(self) -> _Roles: ...
    def init(self) -> int: ...
    def set_timeout(self, val: int, startup_time: int) -> None: ...
    def get_desktop(self, index: int) -> _Accessible: ...


@lru_cache(maxsize=1)
def _native_api() -> _Atspi | None:
    class _Gi(Protocol):
        def require_version(self, name: str, version: str) -> None: ...

    cast(_Gi, importlib.import_module("gi")).require_version("Atspi", "2.0")
    api = cast(_Atspi, importlib.import_module("gi.repository.Atspi"))
    # get_desktop() implicitly initializes libatspi and calls fatal g_error()
    # if the bus is unavailable. Check init() before entering that path.
    # Cache failure too: libatspi marks itself initialized even when init()
    # returns 2, so a second init() can return 1 despite a missing connection.
    return api if api.init() in (0, 1) else None


class AtspiFieldReader:
    def __init__(self, api: _Atspi | None = None, *, process_for_window: Callable[[int], int] | None = None) -> None:
        if api is None:
            api = _native_api()
            if api is None:
                raise RuntimeError("AT-SPI initialization failed")
        self.api = api
        self.process_for_window = process_for_window
        self.api.set_timeout(50, 50)

    def close(self) -> None:
        # No retained accessible objects or per-reader native resources.
        return

    @staticmethod
    def _matches(application: str, node: _Accessible) -> bool:
        name = node.get_name().casefold()
        try:
            process = Path(f"/proc/{node.get_process_id()}/comm").read_text().strip().casefold()
        except OSError:
            process = ""
        return application.casefold() in {name, process}

    def read(self, application: str, window: int) -> FieldContext | None:
        deadline = time.monotonic() + 0.15
        pid = self.process_for_window(window) if self.process_for_window is not None else 0
        desktop = self.api.get_desktop(0)
        stack: list[tuple[_Accessible, str]] = []
        for index in range(min(desktop.get_child_count(), 64)):
            if time.monotonic() >= deadline:
                return None
            app = desktop.get_child_at_index(index)
            if app is not None and (app.get_process_id() == pid if pid else self._matches(application, app)):
                stack.append((app, str(index)))
        visited = 0
        while stack and time.monotonic() < deadline and visited < 128:
            node, path = stack.pop()
            visited += 1
            # Role/focus can change in-place (e.g. reveal/hide password).
            # Do not reuse libatspi's previously cached public-field role.
            node.clear_cache()
            if node.get_state_set().contains(self.api.StateType.FOCUSED):
                field_id = f"{window}:{path}"
                if node.get_role() == self.api.Role.PASSWORD_TEXT:
                    return FieldContext(application, field_id, role="password", sensitive=True, source="atspi")
                text = node.get_text_iface()
                if text is None:
                    return None
                if text.get_n_selections():
                    return FieldContext(application, field_id, selection=True, source="atspi")
                caret = text.get_caret_offset()
                if caret < 0:
                    return None
                # GI returns an Accessible implementing Text. Its legacy
                # get_text() shadows Text.get_text(start, end): call the
                # interface explicitly, as exposed by Atspi.Text's typelib.
                before = self.api.Text.get_text(text, max(0, caret - CONTEXT_LIMIT), caret)
                after = self.api.Text.get_text(text, caret, min(text.get_character_count(), caret + 128))
                visible = (before + after).strip()
                if visible and set(visible) <= {"•", "●", "*"}:
                    # GTK4 can expose a hidden Entry as ordinary TEXT while
                    # GetText returns only password masks. Treat that evidence
                    # conservatively; never feed masked fields to the model.
                    return FieldContext(application, field_id, role="password", sensitive=True, source="atspi")
                if text.get_n_selections():
                    return FieldContext(application, field_id, selection=True, source="atspi")
                if not node.get_state_set().contains(self.api.StateType.FOCUSED) or text.get_caret_offset() != caret:
                    return FieldContext(application, field_id, source="atspi")
                return FieldContext(application, field_id, before, after, "text", source="atspi").bounded()
            for index in reversed(range(min(node.get_child_count(), 64))):
                if time.monotonic() >= deadline:
                    return None
                child = node.get_child_at_index(index)
                if child is not None:
                    stack.append((child, f"{path}:{index}"))
        return None
