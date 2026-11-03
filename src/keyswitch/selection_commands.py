"""The selection hotkeys: what they ask of a platform, and what every platform shares.

A selection hotkey reads the text selected in the focused field, changes it (selection_text) and types the result over
it, as typing over a selection replaces it in any editor. The engine decides whether a command may run at all
(KeySwitch on, not its own window, not an excluded program or a password field) and when (every key of the hotkey
released, as for Pause). A backend that offers the two calls below reads and replaces; one that does not is reported
as unable to, and nothing is typed.
"""

from __future__ import annotations

import unicodedata
from collections.abc import Sequence
from dataclasses import dataclass
from typing import Final, Literal, Protocol, runtime_checkable

from .constants.settings_defaults import (
    HOTKEY_SELECTION_CASE,
    HOTKEY_SELECTION_LAYOUT,
    HOTKEY_SELECTION_TRANSLITERATE,
)
from .constants.text import (
    BASIC_MULTILINGUAL_PLANE_MAX_CODEPOINT,
    SELECTION_COMMAND_MAX_CHARACTERS,
)
from .russian_text import CHARACTERS, quantity
from .selection_text import convert_layout, cycle_case, transliterate

# Where the selected text was read; the log names it.
SOURCE_UIA: Final = "uia"
SOURCE_CLIPBOARD: Final = "clipboard"
SOURCE_ATSPI: Final = "atspi"

RefusalReason = Literal[
    "disabled", "own_window", "excluded_application", "sensitive_field", "unsupported", "no_selection",
    "multiple_selection", "read_only", "too_long", "line_breaks", "untypeable", "terminal", "clipboard_busy",
    "clipboard_unrestorable", "not_settable", "focus_changed", "release_timeout", "no_layout", "empty_result",
    "no_caret",
]
# What the status line says when a selection hotkey changes nothing; the reason itself goes into the log.
REFUSAL_MESSAGES: Final[dict[RefusalReason, str]] = {
    "disabled": "Выделение не изменено: KeySwitch на паузе",
    "own_window": "Выделение в окне KeySwitch не меняется",
    "excluded_application": "Выделение не изменено: программа в исключениях",
    "sensitive_field": "Выделение в поле пароля не меняется",
    "unsupported": "Выделение не прочитано: программа не сообщает выделенный текст",
    "no_selection": "Нет выделенного текста",
    "multiple_selection": "Несколько выделений сразу не меняются",
    "read_only": "Выделенный текст нельзя изменить",
    "too_long": f"Выделение не изменено: предел — {quantity(SELECTION_COMMAND_MAX_CHARACTERS, CHARACTERS)}",
    "line_breaks": "Выделение из нескольких строк не меняется: перевод строки набирался бы как Enter",
    "untypeable": "Выделение не изменено: в нём есть знаки, которых нет на раскладках EN и RU",
    "terminal": "В терминале выделение не заменяется",
    "clipboard_busy": "Выделение не прочитано: буфер обмена занят другой программой",
    "clipboard_unrestorable": "Выделение не прочитано: буфер обмена хранит данные, которые нельзя вернуть",
    "not_settable": "Поле не даёт заменить выделение",
    "focus_changed": "Выделение не изменено: окно сменилось",
    "release_timeout": "Выделение не изменено: не получено отпускание клавиш",
    "no_layout": "Выделение не изменено: раскладка EN/RU не найдена",
    "empty_result": "Выделение не изменено: от него ничего не осталось бы",
    "no_caret": "Выделение не прочитано: в окне нет поля ввода с курсором",
}

SelectionCommand = Literal["layout", "case", "transliterate"]
# The command of each selection hotkey, by its name under `hotkeys`.
COMMANDS: Final[dict[str, SelectionCommand]] = {
    HOTKEY_SELECTION_LAYOUT: "layout",
    HOTKEY_SELECTION_CASE: "case",
    HOTKEY_SELECTION_TRANSLITERATE: "transliterate",
}
# What the status line calls each command once it has changed the selection.
COMMAND_TITLES: Final[dict[SelectionCommand, str]] = {
    "layout": "Раскладка выделения изменена",
    "case": "Регистр выделения изменён",
    "transliterate": "Выделение транслитерировано",
}
# The characters a selection may hold besides printable ones: none. A line break or a tab typed over a selection is
# Enter or Tab to the program, which sends a chat message or leaves the field.
_LINE_BREAKS: Final = frozenset("\r\n\t\v\f\x85\u2028\u2029")
# Unicode categories of characters the caret does not step over on their own (combining marks and format characters
# such as the zero-width joiner of an emoji sequence): a selection with them is not selected back key by key.
_JOINED_CATEGORIES: Final = frozenset(("Mn", "Me", "Cf"))


class SelectionRefused(RuntimeError):
    """The selection is left as it is, for a reason the log and the status line name."""

    def __init__(self, reason: RefusalReason) -> None:
        super().__init__(REFUSAL_MESSAGES[reason])
        self.reason: RefusalReason = reason


@dataclass(frozen=True)
class SelectedText:
    """The selected text and where it was read (SOURCE_*)."""

    text: str
    source: str


class SelectionSource(Protocol):
    """Something that reads the selection for a backend (the AT-SPI reader of the Linux backends) and, where the field
    lets it, writes over it: ``write_selection`` says whether the result is selected again, or None when the field
    cannot be written that way and the backend types instead."""

    def read_selection(self, application: str, window: int) -> SelectedText: ...

    def write_selection(self, selected: SelectedText, replacement: str) -> bool | None: ...


@runtime_checkable
class SelectionBackend(Protocol):
    """The two calls a backend offers for the selection hotkeys.

    ``read_selection`` raises SelectionRefused when there is nothing it may change. ``replace_selection`` types
    ``replacement`` over the selection, leaves the layout in ``final_group`` (or where it was, for None), and says
    whether it selected the result again so that the next press acts on it.
    """

    def read_selection(self, application: str, window: int) -> SelectedText: ...

    def replace_selection(self, selected: SelectedText, replacement: str, final_group: int | None) -> bool: ...


@dataclass(frozen=True)
class PendingSelection:
    """A selection hotkey pressed and waiting for its keys to come up; given up at ``deadline``."""

    command: SelectionCommand
    deadline: float


@dataclass(frozen=True)
class SelectionPlan:
    """What a command makes of the selection, and the layout the keyboard is left in (None: where it was)."""

    replacement: str
    source_group: int | None = None
    target_group: int | None = None

    def changes(self, text: str) -> bool:
        return self.replacement != text


def checked_selection(text: str) -> str:
    """The selected text, or SelectionRefused for a selection the hotkeys do not change."""

    if not text:
        raise SelectionRefused("no_selection")
    if len(text) > SELECTION_COMMAND_MAX_CHARACTERS:
        raise SelectionRefused("too_long")
    if any(character in _LINE_BREAKS for character in text):
        raise SelectionRefused("line_breaks")
    return text


def plan_selection(
    command: SelectionCommand, text: str, current_group: int, layout_names: list[str],
) -> SelectionPlan:
    """What ``command`` makes of ``text``. Only the layout command moves the keyboard: to the layout it converted to."""

    if command == "case":
        return SelectionPlan(cycle_case(text))
    if command == "transliterate":
        # Only a selection of soft signs comes out empty (ICAO leaves Ь out); typing nothing would replace nothing.
        replacement = transliterate(text)
        if not replacement:
            raise SelectionRefused("empty_result")
        return SelectionPlan(replacement)
    current = layout_names[current_group] if 0 <= current_group < len(layout_names) else ""
    conversion = convert_layout(text, current)
    if conversion.source not in layout_names or conversion.target not in layout_names:
        raise SelectionRefused("no_layout")
    return SelectionPlan(
        conversion.text, layout_names.index(conversion.source), layout_names.index(conversion.target))


# A key that types a character in a layout group: its code, and whether Shift goes with it.
KeyChoice = tuple[int, bool]


def typing_runs(
    text: str, tables: Sequence[dict[str, KeyChoice]], start_group: int,
) -> list[tuple[int, list[KeyChoice]]]:
    """The keys that type ``text``, run by run in one layout group, for a backend that types keys, not characters.

    ``tables`` give each group's key for each character it types. A character is typed in the group the previous one
    was typed in when that group has it (spaces, digits and signs on the same key in both layouts do not switch), and
    otherwise in the first group that has it. A character no group types makes the whole text SelectionRefused, before
    a single key is typed.
    """

    runs: list[tuple[int, list[KeyChoice]]] = []
    group = start_group
    for character in text:
        if not 0 <= group < len(tables) or character not in tables[group]:
            group = next((index for index, table in enumerate(tables) if character in table), -1)
            if group < 0:
                raise SelectionRefused("untypeable")
        if not runs or runs[-1][0] != group:
            runs.append((group, []))
        runs[-1][1].append(tables[group][character])
    return runs


def caret_steps(text: str) -> int | None:
    """How many presses of Shift+Left select ``text`` back from its end, or None where that cannot be counted.

    The caret steps over one character a press, but over a combining mark together with its letter and over an
    emoji sequence as a whole, and some editors count a character above the Basic Multilingual Plane as two; such a
    result is left unselected rather than selected wrong.
    """

    if any(
        ord(character) > BASIC_MULTILINGUAL_PLANE_MAX_CODEPOINT
        or unicodedata.category(character) in _JOINED_CATEGORIES
        for character in text
    ):
        return None
    return len(text)
