"""Bounded, field-scoped context. User text is kept in RAM, never serialized.

An observed suffix is not a copy of the editor. Navigation, a shortcut, a
focus change or lost input invalidates it. Accessibility snapshots may add
context, but never grant permission to replace text outside tracked strokes.
"""

from __future__ import annotations

import time
from dataclasses import dataclass
from typing import Literal, Protocol

from .backend import KeyEvent
# Exported under these names: the engine and the platform field readers import them from here.
from .constants.text import FIELD_CONTEXT_MAX_CHARACTERS as CONTEXT_LIMIT
from .constants.text import (
    FIELD_CONTEXT_APPLICATION_MAX_CHARACTERS,
    FIELD_CONTEXT_FIELD_ID_MAX_CHARACTERS,
    FIELD_CONTEXT_SOURCE_MAX_CHARACTERS,
)
from .constants.timing import FIELD_CONTEXT_TTL_SECONDS as CONTEXT_TTL


__all__ = ["CONTEXT_LIMIT", "CONTEXT_TTL", "FieldContext", "FieldReader", "FieldRole", "InputContext"]

FieldRole = Literal["unknown", "text", "code", "search", "terminal", "password"]


@dataclass(frozen=True)
class FieldContext:
    application: str
    field_id: str
    before: str = ""
    after: str = ""
    role: FieldRole = "unknown"
    selection: bool = False
    sensitive: bool = False
    source: str = "observed"

    def bounded(self) -> FieldContext:
        private = self.sensitive or self.role == "password"
        return FieldContext(
            self.application[:FIELD_CONTEXT_APPLICATION_MAX_CHARACTERS],
            self.field_id[:FIELD_CONTEXT_FIELD_ID_MAX_CHARACTERS],
            "" if private else self.before[-CONTEXT_LIMIT:],
            "" if private else self.after[:CONTEXT_LIMIT], self.role,
            self.selection, private, self.source[:FIELD_CONTEXT_SOURCE_MAX_CHARACTERS],
        )

    def too_short_for(self, typed: str) -> bool:
        """Whether the read holds fewer characters than were just typed into the field.

        Such a read is not of the text the keys went into. VS Code with its screen-reader support
        off keeps the input it takes keys through empty and shows at most the key in flight: on the
        owner's laptop every read after `ghbdtn` held one character around the caret, the word was
        refused as typed into a changed field or into another word, and only `Pause` converted it
        (10.10.2026). The keys the engine saw are then the only account of the text.
        """

        return len(self.before) + len(self.after) < len(typed)


class FieldReader(Protocol):
    def read(self, application: str, window: int) -> FieldContext | None: ...


class InputContext:
    """One active field, no background history and no cross-window cache."""

    def __init__(self) -> None:
        self.application = ""
        self.window = 0
        self.text = ""
        self.updated_at = 0.0
        self.revision = 0

    def clear(self) -> None:
        self.text = ""
        self.updated_at = 0.0
        self.revision += 1

    def focus(self, application: str, window: int) -> None:
        if (application, window) != (self.application, self.window):
            self.clear()
            self.application, self.window = application, window

    def observe(self, event: KeyEvent) -> None:
        if not event.pressed or event.synthetic:
            return
        if event.control or event.alt or event.super_key:
            self.clear()
            return
        if event.key_name in {"Return", "KP_Enter", "Tab", "ISO_Left_Tab"}:
            if not event.deferred:
                self.clear()
            return
        if event.key_name in {
            "Left", "Right", "Up", "Down", "Home", "End", "Page_Up", "Page_Down",
            "Delete", "Insert", "Escape", "Pointer",
        }:
            self.clear()
            return
        now = time.monotonic()
        if now - self.updated_at > CONTEXT_TTL:
            self.text = ""
        if event.key_name == "BackSpace":
            self.text = self.text[:-1]
        elif event.character and (event.character.isprintable() or event.character.isspace()):
            self.text = (self.text + event.character)[-CONTEXT_LIMIT:]
        else:
            return
        self.updated_at = now
        self.revision += 1

    def before_word(self, original: str) -> str:
        if time.monotonic() - self.updated_at > CONTEXT_TTL:
            return ""
        # A normal boundary has already reached the editor; an intercepted
        # Enter has not. Only strip a single observed boundary, never search
        # backwards for an older occurrence of the same word.
        if original and self.text.endswith(original):
            return self.text[:-len(original)]
        if original and self.text[:-1].endswith(original):
            return self.text[:-len(original) - 1]
        return ""

    def withdraw(self, typed: str) -> bool:
        """Take back the text observed last, to be typed and observed again; False if it is not there."""

        if not self.text.endswith(typed):
            return False
        self.text = self.text[:len(self.text) - len(typed)]
        self.revision += 1
        return True

    def replace_suffix(self, original: str, replacement: str, boundary: str) -> None:
        old = original + boundary
        if old and self.text.endswith(old):
            self.text = (self.text[:-len(old)] + replacement + boundary)[-CONTEXT_LIMIT:]
            self.updated_at = time.monotonic()
            self.revision += 1
        else:
            self.clear()

    def snapshot(self, original: str) -> FieldContext:
        return FieldContext(
            self.application, str(self.window), before=self.before_word(original),
        )
