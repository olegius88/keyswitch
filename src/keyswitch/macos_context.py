"""Read a bounded caret range through the macOS accessibility tree.

Called on the engine worker, never inside the event tap: a slow answer there
would have the system switch the tap off. The accessibility tree is reached only
through :class:`AccessibilityAPI`, so what this module decides - which fields are
private, where the caret sits, how much text travels - is verified without a Mac.

The attribute and role names are the documented values of Apple's constants,
read from ``AXAttributeConstants.h`` and ``AXRoleConstants.h``.
"""

from __future__ import annotations

from typing import Protocol

from .constants.macos import UTF16_CODE_UNIT_BYTES
from .input_context import CONTEXT_LIMIT, FieldContext, FieldRole

ROLE_ATTRIBUTE = "AXRole"
SUBROLE_ATTRIBUTE = "AXSubrole"
VALUE_ATTRIBUTE = "AXValue"
SELECTED_RANGE_ATTRIBUTE = "AXSelectedTextRange"
IDENTIFIER_ATTRIBUTE = "AXIdentifier"

TEXT_FIELD_ROLE = "AXTextField"
TEXT_AREA_ROLE = "AXTextArea"
COMBO_BOX_ROLE = "AXComboBox"
SECURE_TEXT_FIELD_SUBROLE = "AXSecureTextField"
SEARCH_FIELD_SUBROLE = "AXSearchField"

# What the engine calls the kind of field it is correcting in.
ROLES: dict[str, FieldRole] = {
    TEXT_FIELD_ROLE: "text",
    TEXT_AREA_ROLE: "text",
    COMBO_BOX_ROLE: "text",
}
SOURCE = "ax"


class AccessibilityAPI(Protocol):
    """The handful of accessibility calls the reader needs."""

    def focused_element(self) -> int: ...

    def string_attribute(self, element: int, name: str) -> str | None: ...

    def range_attribute(self, element: int, name: str) -> tuple[int, int] | None: ...

    def release(self, element: int) -> None: ...


def _native_api() -> AccessibilityAPI:
    from .macos_native import CtypesAccessibilityAPI

    return CtypesAccessibilityAPI()


def _character_index(text: str, units: int) -> int | None:
    """Where in ``text`` an offset counted in UTF-16 code units falls.

    AXSelectedTextRange counts as NSString does: an emoji or another character
    outside the basic plane is two units. Slicing the Python string with that
    number moved the caret one character right for each such character before it.
    None when the offset lies past the text or inside such a character.
    """

    encoded = text.encode("utf-16-le")
    if units * UTF16_CODE_UNIT_BYTES > len(encoded):
        return None
    try:
        return len(encoded[: units * UTF16_CODE_UNIT_BYTES].decode("utf-16-le"))
    except UnicodeDecodeError:
        return None


class MacFieldReader:
    """What stands on either side of the caret in the focused field."""

    def __init__(self, api: AccessibilityAPI | None = None) -> None:
        self._api = api if api is not None else _native_api()

    def read(self, application: str, window: int) -> FieldContext | None:
        element = self._api.focused_element()
        if not element:
            return None
        try:
            return self._read(application, element)
        finally:
            self._api.release(element)

    def _read(self, application: str, element: int) -> FieldContext | None:
        role = self._api.string_attribute(element, ROLE_ATTRIBUTE) or ""
        if role not in ROLES:
            # Anything else - a button, a list, a web view we cannot read - is
            # reported as no context rather than as an empty field, which the
            # engine would take for a field that is genuinely empty.
            return None
        field_id = self._api.string_attribute(element, IDENTIFIER_ATTRIBUTE) or role
        subrole = self._api.string_attribute(element, SUBROLE_ATTRIBUTE) or ""
        if subrole == SECURE_TEXT_FIELD_SUBROLE:
            # A password field is named and then left alone: its text is never
            # read, not even to be thrown away afterwards.
            return FieldContext(application, field_id, role="password", sensitive=True,
                                source=SOURCE).bounded()
        value = self._api.string_attribute(element, VALUE_ATTRIBUTE)
        if value is None:
            return None
        caret = self._api.range_attribute(element, SELECTED_RANGE_ATTRIBUTE)
        if caret is None:
            return None
        location, length = caret
        start = _character_index(value, location) if location >= 0 and length >= 0 else None
        end = _character_index(value, location + length) if start is not None else None
        if start is None or end is None:
            # A range the application computed against text it has since changed.
            return None
        kind: FieldRole = "search" if subrole == SEARCH_FIELD_SUBROLE else ROLES[role]
        return FieldContext(
            application,
            field_id,
            before=value[:start][-CONTEXT_LIMIT:],
            after=value[end:][:CONTEXT_LIMIT],
            role=kind,
            selection=length > 0,
            source=SOURCE,
        ).bounded()

    def close(self) -> None:
        """Nothing is kept open: every read starts from the focused element."""
