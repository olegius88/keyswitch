"""The selection hotkeys on Windows: UI Automation, the clipboard fallback and Unicode typing, with mocked Win32 calls."""

from __future__ import annotations

import struct
import unittest
from unittest.mock import MagicMock, patch

from fixture_values.keys import FIELD_READER_MATCHING_PROCESS_ID, FIELD_READER_MISMATCHED_PROCESS_ID, WINDOWS_FAKE_HWND
from fixture_values.platform import RUSSIAN_HKL
from fixture_values.selection import (
    SELECTION_FIXTURE_CLIPBOARD_BYTES,
    SELECTION_FIXTURE_CLIPBOARD_FORMAT,
    SELECTION_FIXTURE_OTHER_WINDOW,
    SELECTION_FIXTURE_SEQUENCE_BEFORE,
    SELECTION_FIXTURE_SEVERAL_SELECTIONS,
    SELECTION_FIXTURE_WINDOW,
)
from keyswitch.constants.text import SELECTION_COMMAND_MAX_CHARACTERS
from keyswitch.constants.windows import (
    UIA_EDIT_CONTROL_TYPE_ID,
    UIA_EMBEDDED_OBJECT_CHARACTER,
    UIA_IS_READ_ONLY_ATTRIBUTE_ID,
    VK_CONTROL,
    VK_INSERT,
    VK_LEFT,
    VK_SCROLL,
    VK_SHIFT,
)
from keyswitch.selection_commands import SOURCE_CLIPBOARD, SOURCE_UIA, SelectedText, SelectionRefused
from keyswitch.windows_backend import (
    ClipboardContents,
    NativeInput,
    WindowsBackend,
    WindowsBackendError,
    WindowsSelectionAPI,
    _uia_selection_reader,
    key_name,
)
from keyswitch.windows_context import WindowsFieldReader
from test_windows_backend import FakeWindowsAPI

COPY = (
    NativeInput(True, virtual_key=VK_CONTROL), NativeInput(True, virtual_key=VK_INSERT, extended=True),
    NativeInput(False, virtual_key=VK_INSERT, extended=True), NativeInput(False, virtual_key=VK_CONTROL),
)
SAVED = ClipboardContents(((SELECTION_FIXTURE_CLIPBOARD_FORMAT, SELECTION_FIXTURE_CLIPBOARD_BYTES),))


class FakeClipboardAPI(FakeWindowsAPI):
    """A Windows with a clipboard: Ctrl+C puts ``copied`` on it after ``copy_delay`` looks at the sequence."""

    def __init__(self) -> None:
        super().__init__()
        self.window_classes: dict[int, str] = {}
        self.caret = True
        self.caret_windows: list[int] = []
        self.sequence = SELECTION_FIXTURE_SEQUENCE_BEFORE
        self.saved = SAVED
        self.copied: str | None = "ghbdtn"
        self.copies = True
        self.copy_delay = 0
        self.copy_pending = False
        self.busy_reads = 0
        self.busy_texts = 0
        self.restores = True
        self.written: list[ClipboardContents] = []

    def send_inputs(self, inputs: tuple[NativeInput, ...]) -> int:
        self.copy_pending = self.copy_pending or (inputs == COPY and self.copies)
        return super().send_inputs(inputs)

    def window_class(self, window: int) -> str:
        return self.window_classes.get(window, "Notepad")

    def text_caret(self, window: int) -> bool:
        self.caret_windows.append(window)
        return self.caret

    def clipboard_sequence(self) -> int:
        if self.copy_pending and self.copy_delay:
            self.copy_delay -= 1
        elif self.copy_pending:
            self.copy_pending = False
            self.sequence += 1
        return self.sequence

    def read_clipboard(self, limit_bytes: int) -> ClipboardContents | None:
        if self.busy_reads:
            self.busy_reads -= 1
            return None
        return self.saved

    def clipboard_text(self, limit_characters: int) -> str | None:
        self.text_limit = limit_characters
        if self.busy_texts:
            self.busy_texts -= 1
            return None
        return self.copied

    def write_clipboard(self, contents: ClipboardContents) -> bool:
        self.written.append(contents)
        return self.restores


class FakeReader:
    def __init__(self, answer: SelectedText | Exception | None, editable: bool | Exception = False) -> None:
        self.answer = answer
        self.editable = editable
        self.windows: list[int] = []

    def read_selection(self, window: int) -> SelectedText | None:
        self.windows.append(window)
        if isinstance(self.answer, Exception):
            raise self.answer
        return self.answer

    def focused_editable(self, window: int) -> bool:
        if isinstance(self.editable, Exception):
            raise self.editable
        return self.editable


def backend(api: FakeWindowsAPI, reader: FakeReader | None = None) -> WindowsBackend:
    result = WindowsBackend(api)
    result._selection_reader_factory = lambda: reader if reader is not None else FakeReader(None)
    return result


class WindowsReadingTests(unittest.TestCase):

    def setUp(self) -> None:
        self.api = FakeClipboardAPI()
        sleep = patch("keyswitch.windows_backend.time.sleep")
        sleep.start()
        self.addCleanup(sleep.stop)

    def refusal(self, subject: WindowsBackend) -> str:
        with self.assertRaises(SelectionRefused) as caught:
            subject.read_selection("Notepad", SELECTION_FIXTURE_WINDOW)
        return caught.exception.reason

    def test_scroll_lock_has_a_name_a_hotkey_can_use(self) -> None:
        self.assertEqual(key_name(VK_SCROLL), "Scroll_Lock")

    def test_ui_automation_answers_first_and_nothing_is_copied(self) -> None:
        reader = FakeReader(SelectedText("ghbdtn", SOURCE_UIA))
        subject = backend(self.api, reader)
        self.assertEqual(subject.read_selection("Notepad", SELECTION_FIXTURE_WINDOW), SelectedText("ghbdtn", SOURCE_UIA))
        subject.read_selection("Notepad", 0)
        self.assertEqual(reader.windows, [SELECTION_FIXTURE_WINDOW, WINDOWS_FAKE_HWND])
        self.assertEqual(self.api.sent, [])

    def test_a_console_is_left_alone(self) -> None:
        self.api.window_classes[WINDOWS_FAKE_HWND] = "ConsoleWindowClass"
        self.assertEqual(self.refusal(backend(self.api)), "terminal")
        self.assertEqual(self.api.sent, [])

    def test_a_refusal_of_ui_automation_stands(self) -> None:
        self.assertEqual(self.refusal(backend(self.api, FakeReader(SelectionRefused("read_only")))), "read_only")

    def test_without_ui_automation_the_selection_is_copied_and_the_clipboard_given_back(self) -> None:
        for reader in (FakeReader(None), FakeReader(RuntimeError("provider gone"))):
            with self.subTest(reader=reader.answer):
                self.api.written.clear()
                self.api.sent.clear()
                self.api.sequence = SELECTION_FIXTURE_SEQUENCE_BEFORE
                selected = backend(self.api, reader).read_selection("Notepad", SELECTION_FIXTURE_WINDOW)
                self.assertEqual(selected, SelectedText("ghbdtn", SOURCE_CLIPBOARD))
                self.assertEqual(self.api.sent, [COPY])
                self.assertEqual(self.api.written, [SAVED])
                self.assertEqual(self.api.text_limit, SELECTION_COMMAND_MAX_CHARACTERS + 1)

    def test_a_reader_that_cannot_be_built_is_tried_again_next_time(self) -> None:
        subject = WindowsBackend(self.api)
        attempts: list[str] = []

        def unavailable() -> FakeReader:
            attempts.append("built")
            raise OSError("no UI Automation")

        subject._selection_reader_factory = unavailable
        subject.read_selection("Notepad", SELECTION_FIXTURE_WINDOW)
        subject.read_selection("Notepad", SELECTION_FIXTURE_WINDOW)
        self.assertEqual(attempts, ["built", "built"])
        reader = FakeReader(SelectedText("x", SOURCE_UIA))
        subject._selection_reader_factory = lambda: reader
        subject.read_selection("Notepad", SELECTION_FIXTURE_WINDOW)
        subject._selection_reader_factory = unavailable
        subject.read_selection("Notepad", SELECTION_FIXTURE_WINDOW)
        self.assertEqual(reader.windows, [SELECTION_FIXTURE_WINDOW, SELECTION_FIXTURE_WINDOW])

    def test_the_default_reader_is_ui_automation(self) -> None:
        with patch("keyswitch.windows_context.WindowsFieldReader") as reader:
            self.assertIs(_uia_selection_reader(), reader.return_value)

    def test_a_windows_without_the_clipboard_calls_cannot_read_it(self) -> None:
        plain = FakeWindowsAPI()
        self.assertFalse(isinstance(plain, WindowsSelectionAPI))
        self.assertIsInstance(self.api, WindowsSelectionAPI)
        self.assertEqual(self.refusal(backend(plain)), "unsupported")

    def test_without_a_caret_or_an_edit_field_nothing_is_copied(self) -> None:
        self.api.caret = False

        def unavailable() -> FakeReader:
            raise OSError("no UI Automation")

        for reader in (FakeReader(None), FakeReader(None, editable=RuntimeError("provider gone")), None):
            with self.subTest(reader=reader and reader.editable):
                subject = backend(self.api, reader)
                if reader is None:
                    subject._selection_reader_factory = unavailable
                self.assertEqual(self.refusal(subject), "no_caret")
        self.assertEqual(self.api.sent, [])
        self.assertEqual(set(self.api.caret_windows), {WINDOWS_FAKE_HWND})

    def test_an_edit_field_ui_automation_names_is_copied_without_a_caret(self) -> None:
        self.api.caret = False
        selected = backend(self.api, FakeReader(None, editable=True)).read_selection("Notepad", SELECTION_FIXTURE_WINDOW)
        self.assertEqual(selected, SelectedText("ghbdtn", SOURCE_CLIPBOARD))
        self.assertEqual(self.api.sent, [COPY])

    def test_a_busy_clipboard_is_asked_again(self) -> None:
        self.api.busy_reads = 1
        self.api.busy_texts = 1
        self.api.copy_delay = 1
        self.assertEqual(backend(self.api).read_selection("Notepad", 0).text, "ghbdtn")

    def test_a_clipboard_that_cannot_be_kept_or_read_is_not_borrowed(self) -> None:
        with patch("keyswitch.windows_backend.SELECTION_CLIPBOARD_TIMEOUT_SECONDS", 0.0):
            self.api.busy_reads = SELECTION_FIXTURE_SEVERAL_SELECTIONS
            self.assertEqual(self.refusal(backend(self.api)), "clipboard_busy")
            self.api.busy_reads = 0
            self.api.saved = ClipboardContents((), complete=False)
            self.assertEqual(self.refusal(backend(self.api)), "clipboard_unrestorable")
            self.assertEqual(self.api.sent, [])

    def test_no_copy_in_time_means_no_selection_and_an_untouched_clipboard(self) -> None:
        self.api.copies = False
        with patch("keyswitch.windows_backend.SELECTION_CLIPBOARD_TIMEOUT_SECONDS", 0.0):
            self.assertEqual(self.refusal(backend(self.api)), "no_selection")
        self.assertEqual(self.api.written, [])

    def test_a_copy_that_cannot_be_read_still_gives_the_clipboard_back(self) -> None:
        self.api.busy_texts = SELECTION_FIXTURE_SEVERAL_SELECTIONS
        with patch("keyswitch.windows_backend.SELECTION_CLIPBOARD_TIMEOUT_SECONDS", 0.0):
            self.assertEqual(self.refusal(backend(self.api)), "clipboard_busy")
        self.assertEqual(self.api.written, [SAVED])

    def test_a_clipboard_that_could_not_be_given_back_is_an_error(self) -> None:
        self.api.restores = False
        with patch("keyswitch.windows_backend.SELECTION_CLIPBOARD_TIMEOUT_SECONDS", 0.0):
            with self.assertRaisesRegex(WindowsBackendError, "Буфер обмена не удалось вернуть"):
                backend(self.api).read_selection("Notepad", 0)


class WindowsTypingTests(unittest.TestCase):

    def setUp(self) -> None:
        self.api = FakeClipboardAPI()
        self.backend = backend(self.api)

    def test_the_result_is_typed_as_characters_selected_back_and_the_layout_moved(self) -> None:
        self.assertTrue(self.backend.replace_selection(SelectedText("ghbdtn", SOURCE_UIA), "пр", 1))
        characters = [
            item for character in "пр" for item in (
                NativeInput(True, scan_code=ord(character), unicode=True),
                NativeInput(False, scan_code=ord(character), unicode=True),
            )
        ]
        left = [NativeInput(True, virtual_key=VK_LEFT, extended=True), NativeInput(False, virtual_key=VK_LEFT, extended=True)]
        self.assertEqual(self.api.sent, [tuple([
            *characters, NativeInput(True, virtual_key=VK_SHIFT), *left, *left, NativeInput(False, virtual_key=VK_SHIFT),
        ])])
        self.assertEqual(self.api.requests, [RUSSIAN_HKL])

    def test_a_character_beyond_the_basic_plane_goes_as_its_two_units_and_is_not_selected_back(self) -> None:
        self.assertFalse(self.backend.replace_selection(SelectedText("x", SOURCE_UIA), "😀", None))
        pressed = [item.scan_code for item in self.api.sent[0] if item.pressed]
        self.assertEqual(pressed, [unit for (unit,) in struct.iter_unpack("<H", "😀".encode("utf-16-le"))])
        self.assertTrue(all(item.unicode for item in self.api.sent[0]))
        self.assertEqual(self.api.requests, [])

    def test_another_window_or_a_short_send_changes_nothing_more(self) -> None:
        self.backend.hold_input()
        self.api.foreground = SELECTION_FIXTURE_OTHER_WINDOW
        with self.assertRaises(SelectionRefused) as caught:
            self.backend.replace_selection(SelectedText("x", SOURCE_UIA), "y", None)
        self.assertEqual(caught.exception.reason, "focus_changed")
        self.assertEqual(self.api.sent, [])
        self.backend.release_input()
        self.api.send_count = 0
        with self.assertRaises(WindowsBackendError):
            self.backend.replace_selection(SelectedText("x", SOURCE_UIA), "y", None)


class WindowsUiAutomationTests(unittest.TestCase):

    def setUp(self) -> None:
        self.automation = MagicMock()
        self.element = self.automation.GetFocusedElement.return_value
        self.element.CurrentProcessId = FIELD_READER_MATCHING_PROCESS_ID
        self.element.CurrentIsPassword = False
        pattern = self.element.GetCurrentPattern.return_value.QueryInterface.return_value
        self.ranges = pattern.GetSelection.return_value
        self.ranges.Length = 1
        self.selection = self.ranges.GetElement.return_value
        self.selection.CompareEndpoints.return_value = 1
        self.selection.GetAttributeValue.return_value = object()
        self.selection.GetText.return_value = "ghbdtn"
        self.reader = WindowsFieldReader(self.automation, object())
        process = patch.object(WindowsFieldReader, "_process_for_window", return_value=FIELD_READER_MATCHING_PROCESS_ID)
        process.start()
        self.addCleanup(process.stop)

    def refusal(self) -> str:
        with self.assertRaises(SelectionRefused) as caught:
            self.reader.read_selection(SELECTION_FIXTURE_WINDOW)
        return caught.exception.reason

    def test_the_selected_text_of_the_focused_field(self) -> None:
        self.assertEqual(self.reader.read_selection(SELECTION_FIXTURE_WINDOW), SelectedText("ghbdtn", SOURCE_UIA))
        self.selection.GetText.assert_called_once_with(SELECTION_COMMAND_MAX_CHARACTERS + 1)
        self.selection.GetAttributeValue.assert_called_once_with(UIA_IS_READ_ONLY_ATTRIBUTE_ID)

    def test_an_edit_field_of_the_window_is_named_editable(self) -> None:
        self.element.CurrentControlType = UIA_EDIT_CONTROL_TYPE_ID
        self.assertTrue(self.reader.focused_editable(SELECTION_FIXTURE_WINDOW))
        self.element.CurrentControlType = UIA_EDIT_CONTROL_TYPE_ID + 1
        self.assertFalse(self.reader.focused_editable(SELECTION_FIXTURE_WINDOW))
        self.element.CurrentControlType = UIA_EDIT_CONTROL_TYPE_ID
        self.element.CurrentProcessId = FIELD_READER_MISMATCHED_PROCESS_ID
        self.assertFalse(self.reader.focused_editable(SELECTION_FIXTURE_WINDOW))

    def test_where_ui_automation_cannot_say_the_clipboard_is_asked(self) -> None:
        self.element.GetCurrentPattern.return_value = None
        self.assertIsNone(self.reader.read_selection(SELECTION_FIXTURE_WINDOW))
        self.element.CurrentProcessId = FIELD_READER_MISMATCHED_PROCESS_ID
        self.assertIsNone(self.reader.read_selection(SELECTION_FIXTURE_WINDOW))

    def test_what_is_not_a_selection_to_type_over(self) -> None:
        self.selection.GetText.return_value = "a" + UIA_EMBEDDED_OBJECT_CHARACTER
        self.assertEqual(self.refusal(), "untypeable")
        self.selection.GetAttributeValue.return_value = self.automation.ReservedMixedAttributeValue
        self.assertEqual(self.refusal(), "read_only")
        self.selection.GetAttributeValue.return_value = True
        self.assertEqual(self.refusal(), "read_only")
        self.selection.CompareEndpoints.return_value = 0
        self.assertEqual(self.refusal(), "no_selection")
        self.ranges.Length = 0
        self.assertEqual(self.refusal(), "no_selection")
        self.ranges.Length = SELECTION_FIXTURE_SEVERAL_SELECTIONS
        self.assertEqual(self.refusal(), "multiple_selection")
        self.element.CurrentIsPassword = True
        self.element.GetCurrentPattern.reset_mock()
        self.assertEqual(self.refusal(), "sensitive_field")
        self.element.GetCurrentPattern.assert_not_called()


if __name__ == "__main__":
    unittest.main()
