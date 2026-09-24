"""What KeySwitch reads around the caret on macOS, checked without a Mac.

The accessibility tree is reached only through a protocol, so the decisions that
matter - which fields are private, where the caret sits, how much text is
carried - are all verified here.
"""

from __future__ import annotations

import unittest
from unittest.mock import patch

from keyswitch.input_context import CONTEXT_LIMIT
from keyswitch.macos_context import (
    IDENTIFIER_ATTRIBUTE,
    ROLE_ATTRIBUTE,
    SEARCH_FIELD_SUBROLE,
    SECURE_TEXT_FIELD_SUBROLE,
    SELECTED_RANGE_ATTRIBUTE,
    SUBROLE_ATTRIBUTE,
    TEXT_AREA_ROLE,
    TEXT_FIELD_ROLE,
    VALUE_ATTRIBUTE,
    MacFieldReader,
)

ELEMENT = 0xF0C5
APPLICATION = "com.apple.TextEdit"
WINDOW = 42


class FakeAccessibility:
    def __init__(self, **attributes: object) -> None:
        self.attributes: dict[str, object] = {ROLE_ATTRIBUTE: TEXT_FIELD_ROLE}
        self.attributes.update(attributes)
        self.element = ELEMENT
        self.released: list[int] = []

    def focused_element(self) -> int:
        return self.element

    def string_attribute(self, element: int, name: str) -> str | None:
        value = self.attributes.get(name)
        return value if isinstance(value, str) else None

    def range_attribute(self, element: int, name: str) -> tuple[int, int] | None:
        value = self.attributes.get(name)
        return value if isinstance(value, tuple) else None

    def release(self, element: int) -> None:
        self.released.append(element)


def reader(**attributes: object) -> tuple[MacFieldReader, FakeAccessibility]:
    api = FakeAccessibility(**attributes)
    return MacFieldReader(api), api


class ReadingTests(unittest.TestCase):
    FIRST_WORD_LENGTH = 6
    TEXT_AREA_WORD_LENGTH = 5
    OVERSIZED_TEXT_MULTIPLIER = 3
    MIDPOINT_DIVISOR = 2

    def test_the_text_is_split_at_the_caret(self) -> None:
        field, api = reader(**{VALUE_ATTRIBUTE: "привет мир", SELECTED_RANGE_ATTRIBUTE: (self.FIRST_WORD_LENGTH, 0)})
        context = field.read(APPLICATION, WINDOW)
        assert context is not None
        self.assertEqual(context.before, "привет")
        self.assertEqual(context.after, " мир")
        self.assertFalse(context.selection)
        self.assertEqual(context.role, "text")
        self.assertEqual(api.released, [ELEMENT])

    def test_a_selection_is_reported_and_left_out_of_both_sides(self) -> None:
        field, _api = reader(**{VALUE_ATTRIBUTE: "привет мир", SELECTED_RANGE_ATTRIBUTE: (0, self.FIRST_WORD_LENGTH)})
        context = field.read(APPLICATION, WINDOW)
        assert context is not None
        self.assertTrue(context.selection)
        self.assertEqual(context.before, "")
        self.assertEqual(context.after, " мир")

    def test_a_password_field_is_named_and_never_read(self) -> None:
        """Its text must not be fetched at all, not fetched and then dropped."""

        field, api = reader(**{
            SUBROLE_ATTRIBUTE: SECURE_TEXT_FIELD_SUBROLE,
            VALUE_ATTRIBUTE: "секрет",
            SELECTED_RANGE_ATTRIBUTE: (self.FIRST_WORD_LENGTH, 0),
        })
        with patch.object(api, "string_attribute", wraps=api.string_attribute) as read:
            context = field.read(APPLICATION, WINDOW)
        assert context is not None
        self.assertEqual(context.role, "password")
        self.assertTrue(context.sensitive)
        self.assertEqual(context.before, "")
        self.assertNotIn(VALUE_ATTRIBUTE, [call.args[1] for call in read.call_args_list])

    def test_a_search_field_is_told_apart_from_ordinary_text(self) -> None:
        field, _api = reader(**{
            SUBROLE_ATTRIBUTE: SEARCH_FIELD_SUBROLE,
            VALUE_ATTRIBUTE: "запрос", SELECTED_RANGE_ATTRIBUTE: (self.FIRST_WORD_LENGTH, 0),
        })
        context = field.read(APPLICATION, WINDOW)
        assert context is not None
        self.assertEqual(context.role, "search")

    def test_a_text_area_counts_as_text(self) -> None:
        field, _api = reader(**{
            ROLE_ATTRIBUTE: TEXT_AREA_ROLE,
            VALUE_ATTRIBUTE: "абзац", SELECTED_RANGE_ATTRIBUTE: (self.TEXT_AREA_WORD_LENGTH, 0),
        })
        context = field.read(APPLICATION, WINDOW)
        assert context is not None
        self.assertEqual(context.role, "text")

    def test_only_a_bounded_amount_of_text_travels(self) -> None:
        text = "я" * (CONTEXT_LIMIT * self.OVERSIZED_TEXT_MULTIPLIER)
        field, _api = reader(**{
            VALUE_ATTRIBUTE: text, SELECTED_RANGE_ATTRIBUTE: (len(text) // self.MIDPOINT_DIVISOR, 0)})
        context = field.read(APPLICATION, WINDOW)
        assert context is not None
        self.assertEqual(len(context.before), CONTEXT_LIMIT)
        self.assertEqual(len(context.after), CONTEXT_LIMIT)

    def test_the_identifier_is_used_when_the_field_offers_one(self) -> None:
        field, _api = reader(**{
            IDENTIFIER_ATTRIBUTE: "search-box",
            VALUE_ATTRIBUTE: "", SELECTED_RANGE_ATTRIBUTE: (0, 0)})
        context = field.read(APPLICATION, WINDOW)
        assert context is not None
        self.assertEqual(context.field_id, "search-box")

    def test_a_field_without_an_identifier_falls_back_to_its_role(self) -> None:
        field, _api = reader(**{VALUE_ATTRIBUTE: "", SELECTED_RANGE_ATTRIBUTE: (0, 0)})
        context = field.read(APPLICATION, WINDOW)
        assert context is not None
        self.assertEqual(context.field_id, TEXT_FIELD_ROLE)


class RefusalTests(unittest.TestCase):
    """Silence is the right answer more often than a guess is."""

    OUT_OF_BOUNDS_CARET_INDEX = 99
    NEGATIVE_RANGE_LENGTH = -3

    def test_nothing_focused_reads_as_no_context(self) -> None:
        field, api = reader()
        api.element = 0
        self.assertIsNone(field.read(APPLICATION, WINDOW))
        self.assertEqual(api.released, [])

    def test_a_field_of_an_unknown_kind_is_not_mistaken_for_an_empty_one(self) -> None:
        """An empty answer would tell the engine the field is genuinely empty."""

        field, _api = reader(**{ROLE_ATTRIBUTE: "AXButton"})
        self.assertIsNone(field.read(APPLICATION, WINDOW))

    def test_a_field_that_will_not_give_its_text_is_refused(self) -> None:
        field, _api = reader(**{SELECTED_RANGE_ATTRIBUTE: (0, 0)})
        self.assertIsNone(field.read(APPLICATION, WINDOW))

    def test_a_field_without_a_caret_is_refused(self) -> None:
        field, _api = reader(**{VALUE_ATTRIBUTE: "текст"})
        self.assertIsNone(field.read(APPLICATION, WINDOW))

    def test_a_caret_outside_the_text_is_refused(self) -> None:
        """The application changed the text after it computed the range."""

        field, _api = reader(**{VALUE_ATTRIBUTE: "коротко", SELECTED_RANGE_ATTRIBUTE: (self.OUT_OF_BOUNDS_CARET_INDEX, 0)})
        self.assertIsNone(field.read(APPLICATION, WINDOW))

    def test_a_negative_range_is_refused(self) -> None:
        field, _api = reader(**{VALUE_ATTRIBUTE: "текст", SELECTED_RANGE_ATTRIBUTE: (-1, 0)})
        self.assertIsNone(field.read(APPLICATION, WINDOW))
        field, _api = reader(**{VALUE_ATTRIBUTE: "текст", SELECTED_RANGE_ATTRIBUTE: (0, self.NEGATIVE_RANGE_LENGTH)})
        self.assertIsNone(field.read(APPLICATION, WINDOW))

    def test_the_element_is_released_even_when_nothing_is_returned(self) -> None:
        field, api = reader(**{ROLE_ATTRIBUTE: "AXButton"})
        field.read(APPLICATION, WINDOW)
        self.assertEqual(api.released, [ELEMENT])

    def test_closing_the_reader_keeps_nothing_open(self) -> None:
        field, _api = reader()
        field.close()

    def test_the_real_accessibility_layer_is_used_when_none_is_supplied(self) -> None:
        import sys
        import types

        module = types.ModuleType("keyswitch.macos_native")
        sentinel = FakeAccessibility()
        module.CtypesAccessibilityAPI = lambda: sentinel  # type: ignore[attr-defined]
        with patch.dict(sys.modules, {"keyswitch.macos_native": module}):
            self.assertIs(MacFieldReader()._api, sentinel)


if __name__ == "__main__":
    unittest.main()


class StatusShapeTests(unittest.TestCase):
    """The shape both platforms report autostart in."""

    def test_a_report_carries_every_reason_it_may_not_work(self) -> None:
        from keyswitch.system_model import AutostartStatus

        status = AutostartStatus("/Applications/KeySwitch.app", False, True)
        self.assertEqual(status.as_dict(), {
            "command": "/Applications/KeySwitch.app",
            "blocked_by_windows": False,
            "target_missing": True,
            "effective": False,
        })
