"""The selection hotkeys on macOS: kAXSelectedTextAttribute read and written, checked without a Mac."""

from __future__ import annotations

import sys
import types
import unittest
from unittest.mock import patch

from fixture_values.platform import MACOS_FAKE_AX_ELEMENT
from fixture_values.selection import (
    SELECTION_FIXTURE_CLIPBOARD_BYTES,
    SELECTION_FIXTURE_OTHER_WINDOW,
    SELECTION_FIXTURE_SEQUENCE_BEFORE,
    SELECTION_FIXTURE_RANGE_START,
    SELECTION_FIXTURE_REPLACEMENT_UNITS,
)
from keyswitch.constants.macos import MAC_VK_ANSI_V, MAC_VK_COMMAND, MAC_VK_LEFT_ARROW, MAC_VK_SHIFT, UTF16_CODE_UNIT_BYTES
from keyswitch.constants.text import CLIPBOARD_SNAPSHOT_MAX_BYTES
from keyswitch.constants.timing import MACOS_PASTE_RESTORE_DELAY_SECONDS
from keyswitch.macos_backend import MacBackend, MacBackendError, MacPasteboardAPI, NativeInput, PasteboardContents
from keyswitch.macos_context import (
    CARET_AFTER,
    ROLE_ATTRIBUTE,
    SECURE_TEXT_FIELD_SUBROLE,
    SELECTED_RANGE_ATTRIBUTE,
    SELECTED_TEXT_ATTRIBUTE,
    SELECTION_KEPT,
    SELECTION_UNKNOWN,
    SOURCE,
    SUBROLE_ATTRIBUTE,
    TEXT_AREA_ROLE,
    MacSelectionAccess,
    _native_selection_api,
)
from keyswitch.selection_commands import SelectedText, SelectionRefused
from test_macos_backend import RUSSIAN, FakeMacAPI


class FakeSelectionAccessibility:
    """A focused text area with a selection; ``ranges`` are what AXSelectedTextRange answers, read by read."""

    def __init__(self) -> None:
        self.element = MACOS_FAKE_AX_ELEMENT
        self.attributes: dict[str, str | None] = {ROLE_ATTRIBUTE: TEXT_AREA_ROLE, SELECTED_TEXT_ATTRIBUTE: "ghbdtn"}
        self.settable = True
        self.writes: list[str] = []
        self.write_accepted = True
        self.ranges: list[tuple[int, int] | None] = [(SELECTION_FIXTURE_RANGE_START, len("ghbdtn"))]
        self.released: list[int] = []

    def focused_element(self) -> int:
        return self.element

    def string_attribute(self, element: int, name: str) -> str | None:
        return self.attributes.get(name)

    def range_attribute(self, element: int, name: str) -> tuple[int, int] | None:
        self.range_name = name
        return self.ranges.pop(0) if self.ranges else None

    def release(self, element: int) -> None:
        self.released.append(element)

    def attribute_settable(self, element: int, name: str) -> bool:
        return self.settable and name == SELECTED_TEXT_ATTRIBUTE

    def set_string_attribute(self, element: int, name: str, value: str) -> bool:
        self.writes.append(value)
        return self.write_accepted


def after_write(api: FakeSelectionAccessibility, where: tuple[int, int] | None) -> None:
    api.ranges = [(SELECTION_FIXTURE_RANGE_START, len("ghbdtn")), where]


class SelectionAccessTests(unittest.TestCase):

    def setUp(self) -> None:
        self.api = FakeSelectionAccessibility()
        self.access = MacSelectionAccess(self.api)

    def refused(self, call: str) -> str:
        """The reason the call is refused; the element it took is released either way."""

        released = len(self.api.released)
        with self.assertRaises(SelectionRefused) as caught:
            if call == "read":
                self.access.read()
            else:
                self.access.replace("ghbdtn", "привет")
        self.assertEqual(len(self.api.released), released + (1 if self.api.element else 0))
        return caught.exception.reason

    def test_the_selected_text_of_a_field_that_lets_it_be_written(self) -> None:
        self.assertEqual(self.access.read(), SelectedText("ghbdtn", SOURCE))
        self.assertEqual(self.api.released, [MACOS_FAKE_AX_ELEMENT])

    def test_a_selection_is_read_even_where_it_cannot_be_written(self) -> None:
        self.api.settable = False
        self.assertEqual(self.access.read(), SelectedText("ghbdtn", SOURCE))

    def test_fields_whose_selection_is_left_alone(self) -> None:
        self.api.attributes[SELECTED_TEXT_ATTRIBUTE] = ""
        self.assertEqual(self.refused("read"), "no_selection")
        self.api.attributes[SELECTED_TEXT_ATTRIBUTE] = None
        self.assertEqual(self.refused("read"), "unsupported")
        self.api.attributes[SUBROLE_ATTRIBUTE] = SECURE_TEXT_FIELD_SUBROLE
        self.assertEqual(self.refused("read"), "sensitive_field")
        self.api.attributes[ROLE_ATTRIBUTE] = "AXButton"
        self.assertEqual(self.refused("read"), "unsupported")
        self.api.attributes[ROLE_ATTRIBUTE] = None
        self.assertEqual(self.refused("read"), "unsupported")
        self.api.element = 0
        self.assertEqual(self.refused("read"), "unsupported")

    def test_writing_says_where_the_selection_stands_afterwards(self) -> None:
        after_write(self.api, (SELECTION_FIXTURE_RANGE_START, SELECTION_FIXTURE_REPLACEMENT_UNITS))
        self.assertEqual(self.access.replace("ghbdtn", "привет"), SELECTION_KEPT)
        after_write(self.api, (SELECTION_FIXTURE_RANGE_START + SELECTION_FIXTURE_REPLACEMENT_UNITS, 0))
        self.assertEqual(self.access.replace("ghbdtn", "привет"), CARET_AFTER)
        after_write(self.api, (0, 0))
        self.assertEqual(self.access.replace("ghbdtn", "привет"), SELECTION_UNKNOWN)
        self.api.ranges = []
        self.assertEqual(self.access.replace("ghbdtn", "привет"), SELECTION_UNKNOWN)
        self.assertEqual(set(self.api.writes), {"привет"})
        self.assertEqual(self.api.range_name, SELECTED_RANGE_ATTRIBUTE)

    def test_a_field_that_changed_or_refuses_the_write_is_left_as_it_is(self) -> None:
        self.api.settable = False
        self.assertEqual(self.refused("replace"), "not_settable")
        self.assertEqual(self.api.writes, [])
        self.api.settable = True
        self.api.write_accepted = False
        self.assertEqual(self.refused("replace"), "not_settable")
        self.api.attributes[SELECTED_TEXT_ATTRIBUTE] = "other"
        self.assertEqual(self.refused("replace"), "focus_changed")
        self.api.element = 0
        self.assertEqual(self.refused("replace"), "focus_changed")
        self.assertEqual(self.api.writes, ["привет"])

    def test_the_ranges_around_a_paste(self) -> None:
        self.assertEqual(self.access.selected_range("ghbdtn"), (SELECTION_FIXTURE_RANGE_START, len("ghbdtn")))
        self.assertIsNone(self.access.caret_range())
        with self.assertRaises(SelectionRefused):
            self.access.selected_range("other")
        self.api.element = 0
        with self.assertRaises(SelectionRefused):
            self.access.selected_range("ghbdtn")
        self.assertIsNone(self.access.caret_range())

    def test_the_native_calls_are_made_on_macos_only(self) -> None:
        native = types.ModuleType("keyswitch.macos_native")
        setattr(native, "CtypesAccessibilityAPI", lambda: self.api)
        with patch.dict(sys.modules, {"keyswitch.macos_native": native}):
            self.assertIs(_native_selection_api(), self.api)
            self.assertEqual(MacSelectionAccess().read().text, "ghbdtn")


class FakePasteboardMac(FakeMacAPI):
    """A Mac with a pasteboard: ``put_text`` puts the result there, ``changed`` stands for a copy made meanwhile."""

    def __init__(self) -> None:
        super().__init__()
        self.saved = PasteboardContents(((("public.utf8-plain-text", SELECTION_FIXTURE_CLIPBOARD_BYTES),),))
        self.change_count = SELECTION_FIXTURE_SEQUENCE_BEFORE
        self.puts: list[str] = []
        self.put_accepted = True
        self.changed = False
        self.restores: list[PasteboardContents] = []
        self.restore_accepted = True

    def pasteboard_change_count(self) -> int:
        return self.change_count + (1 if self.changed else 0)

    def read_pasteboard(self, limit_bytes: int) -> PasteboardContents:
        self.limit = limit_bytes
        return self.saved

    def write_pasteboard(self, contents: PasteboardContents) -> bool:
        self.restores.append(contents)
        return self.restore_accepted

    def put_text(self, text: str) -> int | None:
        self.puts.append(text)
        return self.change_count if self.put_accepted else None


PASTE = [
    NativeInput(True, MAC_VK_COMMAND), NativeInput(True, MAC_VK_ANSI_V, command=True),
    NativeInput(False, MAC_VK_ANSI_V, command=True), NativeInput(False, MAC_VK_COMMAND),
]


class MacPasteTests(unittest.TestCase):

    def setUp(self) -> None:
        self.mac = FakePasteboardMac()
        self.backend = MacBackend(self.mac)
        self.api = FakeSelectionAccessibility()
        self.api.settable = False
        # Read by replace before it refuses, by the paste before and after the keys.
        self.api.ranges = [
            (SELECTION_FIXTURE_RANGE_START, len("ghbdtn")), (SELECTION_FIXTURE_RANGE_START, len("ghbdtn")),
            (SELECTION_FIXTURE_RANGE_START + SELECTION_FIXTURE_REPLACEMENT_UNITS, 0),
        ]
        self.backend._selection_access = MacSelectionAccess(self.api)
        sleep = patch("keyswitch.macos_backend.time.sleep")
        self.sleep = sleep.start()
        self.addCleanup(sleep.stop)

    def replace(self) -> bool:
        return self.backend.replace_selection(SelectedText("ghbdtn", SOURCE), "привет", 1)

    def test_a_field_that_cannot_be_written_gets_a_paste_and_the_pasteboard_back(self) -> None:
        self.assertTrue(self.replace())
        self.assertEqual(self.mac.puts, ["привет"])
        left = [NativeInput(True, MAC_VK_LEFT_ARROW, shift=True), NativeInput(False, MAC_VK_LEFT_ARROW, shift=True)]
        self.assertEqual(self.mac.posted, [
            *PASTE, NativeInput(True, MAC_VK_SHIFT), *left * len("привет"), NativeInput(False, MAC_VK_SHIFT)])
        self.sleep.assert_called_once_with(MACOS_PASTE_RESTORE_DELAY_SECONDS)
        self.assertEqual(self.mac.restores, [self.mac.saved])
        self.assertEqual(self.mac.limit, CLIPBOARD_SNAPSHOT_MAX_BYTES)
        self.assertEqual(self.mac.current, RUSSIAN)
        self.assertEqual(self.api.writes, [])

    def test_a_pasteboard_copied_to_meanwhile_is_left_to_the_user(self) -> None:
        self.mac.changed = True
        self.replace()
        self.assertEqual(self.mac.restores, [])

    def test_a_pasteboard_that_cannot_be_kept_is_not_borrowed(self) -> None:
        self.mac.saved = PasteboardContents((), complete=False)
        with self.assertRaises(SelectionRefused) as caught:
            self.replace()
        self.assertEqual(caught.exception.reason, "clipboard_unrestorable")
        self.assertEqual((self.mac.puts, self.mac.posted), ([], []))

    def test_a_selection_gone_before_the_paste_is_left_alone(self) -> None:
        self.api.ranges = [(SELECTION_FIXTURE_RANGE_START, len("ghbdtn"))]

        def deselect(element: int, name: str) -> tuple[int, int] | None:
            self.api.attributes[SELECTED_TEXT_ATTRIBUTE] = "other"
            return None

        self.api.range_attribute = deselect  # type: ignore[method-assign]
        with self.assertRaises(SelectionRefused) as caught:
            self.replace()
        self.assertEqual(caught.exception.reason, "focus_changed")
        self.assertEqual(self.mac.puts, [])

    def test_a_refused_put_or_restore_is_an_error_and_the_pasteboard_is_given_back(self) -> None:
        self.mac.put_accepted = False
        with self.assertRaisesRegex(MacBackendError, "не положила"):
            self.replace()
        self.assertEqual((self.mac.posted, self.mac.restores), ([], [self.mac.saved]))
        self.setUp()
        self.mac.restore_accepted = False
        with self.assertRaisesRegex(MacBackendError, "не удалось вернуть"):
            self.replace()

    def test_a_selection_that_changed_is_not_pasted_over(self) -> None:
        self.api.attributes[SELECTED_TEXT_ATTRIBUTE] = "other"
        with self.assertRaises(SelectionRefused) as caught:
            self.replace()
        self.assertEqual(caught.exception.reason, "focus_changed")
        self.assertEqual((self.mac.puts, self.mac.posted), ([], []))

    def test_a_mac_without_the_pasteboard_calls_leaves_the_field_alone(self) -> None:
        backend = MacBackend(FakeMacAPI())
        backend._selection_access = MacSelectionAccess(self.api)
        with self.assertRaises(SelectionRefused) as caught:
            backend.replace_selection(SelectedText("ghbdtn", SOURCE), "привет", None)
        self.assertEqual(caught.exception.reason, "not_settable")
        self.assertIsInstance(self.mac, MacPasteboardAPI)


class MacBackendSelectionTests(unittest.TestCase):

    def setUp(self) -> None:
        self.mac = FakeMacAPI()
        self.backend = MacBackend(self.mac)
        self.api = FakeSelectionAccessibility()
        self.backend._selection_access = MacSelectionAccess(self.api)

    def test_the_selection_is_read_through_the_accessibility_tree(self) -> None:
        self.assertEqual(self.backend.read_selection("TextEdit", 0), SelectedText("ghbdtn", SOURCE))
        backend = MacBackend(self.mac)
        with patch("keyswitch.macos_backend.MacSelectionAccess") as access:
            backend.read_selection("TextEdit", 0)
            backend.read_selection("TextEdit", 0)
        access.assert_called_once_with()

    def test_a_caret_after_the_result_is_turned_into_a_selection_and_the_layout_moved(self) -> None:
        after_write(self.api, (SELECTION_FIXTURE_RANGE_START + SELECTION_FIXTURE_REPLACEMENT_UNITS, 0))
        self.assertTrue(self.backend.replace_selection(SelectedText("ghbdtn", SOURCE), "привет", 1))
        left = [NativeInput(True, MAC_VK_LEFT_ARROW, shift=True), NativeInput(False, MAC_VK_LEFT_ARROW, shift=True)]
        self.assertEqual(self.mac.posted, [NativeInput(True, MAC_VK_SHIFT), *left * len("привет"), NativeInput(False, MAC_VK_SHIFT)])
        self.assertEqual(self.mac.current, RUSSIAN)

    def test_a_kept_selection_needs_no_keys_and_an_unknown_one_gets_none(self) -> None:
        after_write(self.api, (SELECTION_FIXTURE_RANGE_START, SELECTION_FIXTURE_REPLACEMENT_UNITS))
        self.assertTrue(self.backend.replace_selection(SelectedText("ghbdtn", SOURCE), "привет", None))
        after_write(self.api, (0, 0))
        self.assertFalse(self.backend.replace_selection(SelectedText("ghbdtn", SOURCE), "привет", None))
        after_write(self.api, (SELECTION_FIXTURE_RANGE_START + len("😀".encode("utf-16-le")) // UTF16_CODE_UNIT_BYTES, 0))
        self.assertFalse(self.backend.replace_selection(SelectedText("ghbdtn", SOURCE), "😀", None))
        self.assertEqual(self.mac.posted, [])

    def test_another_window_leaves_the_selection_alone(self) -> None:
        self.backend.hold_input()
        self.mac.window = SELECTION_FIXTURE_OTHER_WINDOW
        with self.assertRaises(SelectionRefused) as caught:
            self.backend.replace_selection(SelectedText("ghbdtn", SOURCE), "привет", None)
        self.assertEqual(caught.exception.reason, "focus_changed")
        self.assertEqual(self.api.writes, [])


if __name__ == "__main__":
    unittest.main()
