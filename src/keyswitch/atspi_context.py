"""Optional Linux AT-SPI text access with bounded traversal and IPC timeouts."""

from __future__ import annotations

import importlib
import time
from collections.abc import Callable
from functools import lru_cache
from pathlib import Path
from dataclasses import dataclass
from typing import Final, Protocol, cast

from .input_context import CONTEXT_LIMIT, FieldContext
from .constants.text import (
    ATSPI_MAX_TRAVERSED_CHILDREN,
    ATSPI_MAX_VISITED_NODES,
    FIELD_AFTER_CARET_MAX_CHARACTERS,
    SELECTION_COMMAND_MAX_CHARACTERS,
)
from .constants.timing import ATSPI_CALL_TIMEOUT_MS, ATSPI_FIELD_READ_DEADLINE_SECONDS
from .selection_commands import SOURCE_ATSPI, SelectedText, SelectionRefused

# What GTK4 shows for each character of a hidden entry exposed as ordinary text.
_PASSWORD_MASKS: Final = frozenset({"•", "●", "*"})


class _States(Protocol):
    def contains(self, state: object) -> bool: ...


class _Text(Protocol):
    def get_caret_offset(self) -> int: ...
    def get_character_count(self) -> int: ...
    def get_n_selections(self) -> int: ...


class _Range(Protocol):
    @property
    def start_offset(self) -> int: ...
    @property
    def end_offset(self) -> int: ...


class _TextApi(Protocol):
    def get_text(self, text: _Text, start: int, end: int) -> str: ...
    def get_selection(self, text: _Text, selection_num: int) -> _Range: ...
    def set_selection(self, text: _Text, selection_num: int, start: int, end: int) -> bool: ...


class _EditableText(Protocol):
    """An object that implements Atspi.EditableText; its calls go through _EditableTextApi."""


class _EditableTextApi(Protocol):
    def delete_text(self, editable: _EditableText, start: int, end: int) -> bool: ...
    def insert_text(self, editable: _EditableText, position: int, text: str, length: int) -> bool: ...


class _Accessible(Protocol):
    def clear_cache(self) -> None: ...
    def get_child_count(self) -> int: ...
    def get_child_at_index(self, index: int) -> _Accessible | None: ...
    def get_name(self) -> str: ...
    def get_process_id(self) -> int: ...
    def get_role(self) -> object: ...
    def get_state_set(self) -> _States: ...
    def get_text_iface(self) -> _Text | None: ...
    def get_editable_text_iface(self) -> _EditableText | None: ...


class _StateTypes(Protocol):
    @property
    def FOCUSED(self) -> object: ...
    @property
    def EDITABLE(self) -> object: ...


class _Roles(Protocol):
    @property
    def PASSWORD_TEXT(self) -> object: ...


class _Atspi(Protocol):
    @property
    def Text(self) -> _TextApi: ...
    @property
    def EditableText(self) -> _EditableTextApi: ...
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


@dataclass(frozen=True)
class _ReadSelection:
    """Where the last selection was read: the field, its text interface and the selection's bounds and text."""

    node: _Accessible
    text: _Text
    start: int
    end: int
    selected: str


class AtspiFieldReader:
    def __init__(self, api: _Atspi | None = None, *, process_for_window: Callable[[int], int] | None = None) -> None:
        if api is None:
            api = _native_api()
            if api is None:
                raise RuntimeError("AT-SPI initialization failed")
        self.api = api
        self.process_for_window = process_for_window
        self.api.set_timeout(ATSPI_CALL_TIMEOUT_MS, ATSPI_CALL_TIMEOUT_MS)
        # The selection read_selection found, for write_selection to write over.
        self._read: _ReadSelection | None = None

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

    def _focused(self, application: str, window: int, deadline: float) -> tuple[_Accessible, str] | None:
        """The focused node of the application's window and its field id, or None within the bounds of the walk."""

        pid = self.process_for_window(window) if self.process_for_window is not None else 0
        desktop = self.api.get_desktop(0)
        stack: list[tuple[_Accessible, str]] = []
        for index in range(min(desktop.get_child_count(), ATSPI_MAX_TRAVERSED_CHILDREN)):
            if time.monotonic() >= deadline:
                return None
            app = desktop.get_child_at_index(index)
            if app is not None and (app.get_process_id() == pid if pid else self._matches(application, app)):
                stack.append((app, str(index)))
        visited = 0
        while stack and time.monotonic() < deadline and visited < ATSPI_MAX_VISITED_NODES:
            node, path = stack.pop()
            visited += 1
            # Role/focus can change in-place (e.g. reveal/hide password).
            # Do not reuse libatspi's previously cached public-field role.
            node.clear_cache()
            if node.get_state_set().contains(self.api.StateType.FOCUSED):
                return node, f"{window}:{path}"
            for index in reversed(range(min(node.get_child_count(), ATSPI_MAX_TRAVERSED_CHILDREN))):
                if time.monotonic() >= deadline:
                    return None
                child = node.get_child_at_index(index)
                if child is not None:
                    stack.append((child, f"{path}:{index}"))
        return None

    def read(self, application: str, window: int) -> FieldContext | None:
        found = self._focused(application, window, time.monotonic() + ATSPI_FIELD_READ_DEADLINE_SECONDS)
        if found is None:
            return None
        node, field_id = found
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
        after = self.api.Text.get_text(
            text, caret, min(text.get_character_count(), caret + FIELD_AFTER_CARET_MAX_CHARACTERS)
        )
        visible = (before + after).strip()
        if visible and set(visible) <= _PASSWORD_MASKS:
            # GTK4 can expose a hidden Entry as ordinary TEXT while
            # GetText returns only password masks. Treat that evidence
            # conservatively; never feed masked fields to the model.
            return FieldContext(application, field_id, role="password", sensitive=True, source="atspi")
        if text.get_n_selections():
            return FieldContext(application, field_id, selection=True, source="atspi")
        if not node.get_state_set().contains(self.api.StateType.FOCUSED) or text.get_caret_offset() != caret:
            return FieldContext(application, field_id, source="atspi")
        return FieldContext(application, field_id, before, after, "text", source="atspi").bounded()

    def read_selection(self, application: str, window: int) -> SelectedText:
        """The text selected in the focused field, for the selection hotkeys; SelectionRefused when there is none to
        change: no field found or no text in it, a password field, a field the user cannot edit, no selection or
        several. The selection's bounds are passed to Text.get_text as they come: its end is "the first character
        past the desired range" (docs.gtk.org/atspi2/method.Text.get_text.html)."""

        found = self._focused(application, window, time.monotonic() + ATSPI_FIELD_READ_DEADLINE_SECONDS)
        if found is None:
            raise SelectionRefused("unsupported")
        node, _field_id = found
        if node.get_role() == self.api.Role.PASSWORD_TEXT:
            raise SelectionRefused("sensitive_field")
        text = node.get_text_iface()
        if text is None:
            raise SelectionRefused("unsupported")
        if not node.get_state_set().contains(self.api.StateType.EDITABLE):
            raise SelectionRefused("read_only")
        selections = text.get_n_selections()
        if selections > 1:
            raise SelectionRefused("multiple_selection")
        span = self.api.Text.get_selection(text, 0) if selections else None
        if span is None or span.end_offset <= span.start_offset:
            raise SelectionRefused("no_selection")
        # One character past the bound is read, so that a longer selection is told apart.
        end = min(span.end_offset, span.start_offset + SELECTION_COMMAND_MAX_CHARACTERS + 1)
        selected = self.api.Text.get_text(text, span.start_offset, end)
        visible = selected.strip()
        if visible and set(visible) <= _PASSWORD_MASKS:
            raise SelectionRefused("sensitive_field")
        self._read = _ReadSelection(node, text, span.start_offset, span.end_offset, selected)
        return SelectedText(selected, SOURCE_ATSPI)

    def write_selection(self, selected: SelectedText, replacement: str) -> bool | None:
        """Write ``replacement`` over the selection last read, through the field's EditableText; None without one.

        The program changes its own text, so any character goes in as it is (`—`, `«»`, an emoji) and no layout or key
        is involved. The field must still be focused and show the very selection that was read. The selected range is
        deleted and the replacement inserted at its start (EditableText.delete_text takes the first character past the
        range as its end; insert_text inserts all of a text whose byte count is within `length`, so the UTF-8 byte
        count is given). Says whether the result was selected again.
        """

        read = self._read
        if read is None or read.selected != selected.text:
            raise SelectionRefused("focus_changed")
        editable = read.node.get_editable_text_iface()
        if editable is None:
            return None
        read.node.clear_cache()
        span = self.api.Text.get_selection(read.text, 0) if read.text.get_n_selections() == 1 else None
        if (
            span is None
            or not read.node.get_state_set().contains(self.api.StateType.FOCUSED)
            or (span.start_offset, span.end_offset) != (read.start, read.end)
            or self.api.Text.get_text(read.text, read.start, read.end) != read.selected
        ):
            raise SelectionRefused("focus_changed")
        self._read = None
        if not self.api.EditableText.delete_text(editable, read.start, read.end):
            raise SelectionRefused("not_settable")
        if not self.api.EditableText.insert_text(
                editable, read.start, replacement, len(replacement.encode("utf-8"))):
            # The selection is gone and nothing came in its place: the selected text is put back.
            restored = self.api.EditableText.insert_text(
                editable, read.start, read.selected, len(read.selected.encode("utf-8")))
            raise RuntimeError(
                "AT-SPI не вставил новый текст; выделенный текст " + ("возвращён" if restored else "не вернулся"))
        return bool(self.api.Text.set_selection(read.text, 0, read.start, read.start + len(replacement)))


class AtspiSelectionReader:
    """The selection hotkeys' reader on Linux, X11 and Wayland alike, built on first use.

    The PRIMARY selection of X11 holds whatever was selected last anywhere, and nothing tells whether that lies in the
    focused field: text typed over a selection made in another window would land where the caret is. AT-SPI says
    which field has the focus, whether it can be edited and what is selected in it, so it is the one source here.
    """

    def __init__(self, process_for_window: Callable[[int], int] | None = None) -> None:
        self._process_for_window = process_for_window
        self._reader: AtspiFieldReader | None = None

    def read_selection(self, application: str, window: int) -> SelectedText:
        try:
            if self._reader is None:
                self._reader = AtspiFieldReader(process_for_window=self._process_for_window)
            return self._reader.read_selection(application, window)
        except SelectionRefused:
            raise
        except Exception as error:
            # No accessibility bus or a call that ran out of time: the program's selection cannot be read.
            raise SelectionRefused("unsupported") from error

    def write_selection(self, selected: SelectedText, replacement: str) -> bool | None:
        """AtspiFieldReader.write_selection, or None before any read."""

        return None if self._reader is None else self._reader.write_selection(selected, replacement)
