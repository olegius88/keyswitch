"""The selection hotkeys: the shared checks, the engine's handling and the Linux backends, with mocked native calls."""

from __future__ import annotations

import ctypes
import json
import sys
import tempfile
import unittest
from collections.abc import Callable
from pathlib import Path
from unittest.mock import MagicMock, patch

from engine_models import share_engine_models
from fixture_values.keys import (
    ALT_L_KEYCODE,
    ALTERNATE_COMMA_KEYCODE,
    A_KEYCODE,
    COMMA_KEYCODE,
    PAUSE_KEYCODE,
    SHIFT_L_KEYCODE,
    SPACE_KEYCODE,
    X11_LEFT_ARROW_KEYCODE,
)
from fixture_values.selection import (
    SELECTION_FIXTURE_EVENTS_PER_SHIFTED_TAP,
    SELECTION_FIXTURE_EVENTS_PER_TAP,
    SELECTION_FIXTURE_OTHER_WINDOW,
    SELECTION_FIXTURE_SCROLL_LOCK_KEYCODE,
    SELECTION_FIXTURE_SEVERAL_SELECTIONS,
    SELECTION_FIXTURE_SLASH_KEYCODE,
    SELECTION_FIXTURE_WINDOW,
)
from fixture_values.platform import XKB_LOCK_GROUP_GROUP_ARG_INDEX, XTEST_FAKE_KEY_EVENT_IS_PRESS_ARG_INDEX
from fixture_values.wayland import (
    WL_FIXTURE_LAYOUT_EN_INDEX,
    WL_FIXTURE_LAYOUT_RU_INDEX,
    WL_FIXTURE_WINDOW_ID,
)
from keyswitch.atspi_context import AtspiFieldReader, AtspiSelectionReader, _native_api
from keyswitch.backend import BackendProbe, FocusInfo, KeyDisposition, KeyEvent, LayoutSwitchUnconfirmed
from keyswitch.config import SettingsStore, default_settings
from keyswitch.constants.keyboard import ALT_MASK, CONTROL_MASK, LOCK_MASK, SHIFT_MASK
from keyswitch.constants.settings_defaults import (
    DEFAULT_SELECTION_CASE_HOTKEY,
    DEFAULT_SELECTION_LAYOUT_HOTKEY,
    DEFAULT_SELECTION_TRANSLITERATE_HOTKEY,
    DEFAULT_SETTINGS,
    HOTKEY_SELECTION_CASE,
    HOTKEY_SELECTION_LAYOUT,
    HOTKEY_SELECTION_TRANSLITERATE,
    MACOS_SELECTION_HOTKEY_DEFAULTS,
    SELECTION_HOTKEY_TITLES,
    SELECTION_HOTKEYS,
)
from keyswitch.constants.text import SELECTION_COMMAND_MAX_CHARACTERS
from keyswitch.constants.wayland import EVDEV_KEY_LEFT, EVDEV_KEY_LEFTSHIFT
from keyswitch.constants.x11 import LEFT_KEYSYM, SHIFT_L_KEYSYM, X11_MAX_KEYCODE, X11_MIN_KEYCODE
from keyswitch.engine import Hotkey, KeySwitchEngine
from keyswitch.settings_diagnostics import settings_snapshot
from keyswitch.history import HistoryStore
from keyswitch.selection_commands import (
    COMMAND_TITLES,
    REFUSAL_MESSAGES,
    SOURCE_ATSPI,
    PendingSelection,
    RefusalReason,
    SelectedText,
    SelectionBackend,
    SelectionPlan,
    SelectionRefused,
    caret_steps,
    checked_selection,
    plan_selection,
    typing_runs,
)
from keyswitch.wayland_backend import WaylandBackend, WaylandBackendError
from keyswitch.windows_ui_model import HOTKEY_SETTINGS
from keyswitch.x11_backend import X11Error, XkbStateRec
from test_wayland_backend import FakeWaylandAPI
from test_x11_backend import backend_with


def setUpModule() -> None:
    share_engine_models()


LAYOUTS = ["us", "ru"]


class SharedChecksTests(unittest.TestCase):

    def test_every_refusal_names_its_reason_and_its_message(self) -> None:
        for reason, message in REFUSAL_MESSAGES.items():
            refusal = SelectionRefused(reason)
            self.assertEqual((refusal.reason, str(refusal)), (reason, message))
        self.assertIn(str(SELECTION_COMMAND_MAX_CHARACTERS), REFUSAL_MESSAGES["too_long"])

    def test_a_selection_is_changed_only_when_it_is_one_line_and_not_too_long(self) -> None:
        self.assertEqual(checked_selection("ghbdtn vbh"), "ghbdtn vbh")
        for text, reason in (
            ("", "no_selection"),
            ("a" * (SELECTION_COMMAND_MAX_CHARACTERS + 1), "too_long"),
            ("first\nsecond", "line_breaks"),
            ("tab\there", "line_breaks"),
        ):
            with self.subTest(reason=reason), self.assertRaises(SelectionRefused) as caught:
                checked_selection(text)
            self.assertEqual(caught.exception.reason, reason)
        self.assertEqual(checked_selection("a" * SELECTION_COMMAND_MAX_CHARACTERS), "a" * SELECTION_COMMAND_MAX_CHARACTERS)

    def test_plans_move_the_layout_only_for_the_layout_command(self) -> None:
        self.assertEqual(plan_selection("case", "привет", 0, LAYOUTS), SelectionPlan("ПРИВЕТ"))
        self.assertEqual(plan_selection("transliterate", "Привет", 0, LAYOUTS), SelectionPlan("Privet"))
        self.assertEqual(plan_selection("layout", "ghbdtn", 0, LAYOUTS), SelectionPlan("привет", 0, 1))
        self.assertEqual(plan_selection("layout", "руддщ", 1, LAYOUTS), SelectionPlan("hello", 1, 0))
        # No letters and no known current layout: the English layout is taken as the one typed in.
        self.assertEqual(plan_selection("layout", "?", -1, LAYOUTS), SelectionPlan(",", 0, 1))
        self.assertTrue(SelectionPlan("b").changes("a"))
        self.assertFalse(SelectionPlan("a").changes("a"))

    def test_plans_refuse_what_they_cannot_type(self) -> None:
        with self.assertRaises(SelectionRefused) as caught:
            plan_selection("transliterate", "ь", 0, LAYOUTS)
        self.assertEqual(caught.exception.reason, "empty_result")
        with self.assertRaises(SelectionRefused) as caught:
            plan_selection("layout", "ghbdtn", 0, ["us"])
        self.assertEqual(caught.exception.reason, "no_layout")

    def test_a_result_is_selected_back_only_where_each_character_is_one_caret_step(self) -> None:
        self.assertEqual(caret_steps("привет мир"), len("привет мир"))
        self.assertIsNone(caret_steps("ok 😀"))
        self.assertIsNone(caret_steps("é"))
        self.assertIsNone(caret_steps("a‍b"))

    def test_typing_runs_stay_in_a_group_while_it_types_the_text(self) -> None:
        english = {"a": (A_KEYCODE, False), " ": (SPACE_KEYCODE, False), "?": (SELECTION_FIXTURE_SLASH_KEYCODE, True)}
        russian = {"ф": (A_KEYCODE, False), " ": (SPACE_KEYCODE, False), ",": (SELECTION_FIXTURE_SLASH_KEYCODE, True)}
        self.assertEqual(typing_runs("a ф,", [english, russian], 0), [
            (0, [(A_KEYCODE, False), (SPACE_KEYCODE, False)]),
            (1, [(A_KEYCODE, False), (SELECTION_FIXTURE_SLASH_KEYCODE, True)]),
        ])
        self.assertEqual(typing_runs(" a", [english, russian], 1), [
            (1, [(SPACE_KEYCODE, False)]), (0, [(A_KEYCODE, False)])])
        self.assertEqual(typing_runs(" ", [english, russian], -1), [(0, [(SPACE_KEYCODE, False)])])
        with self.assertRaises(SelectionRefused) as caught:
            typing_runs("a😀", [english, russian], 0)
        self.assertEqual(caught.exception.reason, "untypeable")

    def test_the_hotkeys_are_set_and_shown_in_both_windows(self) -> None:
        hotkeys = DEFAULT_SETTINGS["hotkeys"]
        assert isinstance(hotkeys, dict)
        self.assertEqual(
            {name: hotkeys[name] for name in SELECTION_HOTKEYS},
            {HOTKEY_SELECTION_LAYOUT: DEFAULT_SELECTION_LAYOUT_HOTKEY, HOTKEY_SELECTION_CASE: DEFAULT_SELECTION_CASE_HOTKEY,
             HOTKEY_SELECTION_TRANSLITERATE: DEFAULT_SELECTION_TRANSLITERATE_HOTKEY},
        )
        defaults = {value for name, value in hotkeys.items() if name not in SELECTION_HOTKEYS}
        self.assertFalse(defaults & {hotkeys[name] for name in SELECTION_HOTKEYS})
        specs = {spec.path: spec for spec in HOTKEY_SETTINGS}
        for name in SELECTION_HOTKEYS:
            self.assertEqual(specs[f"hotkeys.{name}"].title, SELECTION_HOTKEY_TITLES[name])
            self.assertEqual(specs[f"hotkeys.{name}"].kind, "text")

    def test_a_settings_file_from_before_the_hotkeys_gains_their_defaults(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "config.json"
            path.write_text(json.dumps({"hotkeys": {"convert_last": "F12"}}), encoding="utf-8")
            settings = SettingsStore(path)
        self.assertEqual(settings.get("hotkeys.convert_last"), "F12")
        self.assertEqual(settings.get(f"hotkeys.{HOTKEY_SELECTION_LAYOUT}"), DEFAULT_SELECTION_LAYOUT_HOTKEY)

    def test_macos_has_selection_hotkeys_a_mac_keyboard_can_press(self) -> None:
        mac = default_settings("darwin")["hotkeys"]
        assert isinstance(mac, dict)
        self.assertEqual({name: mac[name] for name in SELECTION_HOTKEYS}, MACOS_SELECTION_HOTKEY_DEFAULTS)
        self.assertFalse(
            set(MACOS_SELECTION_HOTKEY_DEFAULTS.values())
            & {value for name, value in mac.items() if name not in SELECTION_HOTKEYS})
        self.assertEqual(default_settings("linux"), DEFAULT_SETTINGS)
        self.assertEqual(default_settings(), default_settings(sys.platform))
        # Control+Option with a letter, as the macOS backend names the key: the letter of the English layout.
        letter = MACOS_SELECTION_HOTKEY_DEFAULTS[HOTKEY_SELECTION_CASE].rsplit("+", 1)[-1].casefold()
        self.assertTrue(Hotkey(MACOS_SELECTION_HOTKEY_DEFAULTS[HOTKEY_SELECTION_CASE]).matches(
            key(letter, A_KEYCODE, state=CONTROL_MASK | ALT_MASK)))

    def test_a_mac_settings_store_uses_and_restores_the_mac_defaults(self) -> None:
        path = f"hotkeys.{HOTKEY_SELECTION_CASE}"
        with tempfile.TemporaryDirectory() as directory, patch(
                "keyswitch.config.default_settings", side_effect=lambda: default_settings("darwin")):
            settings = SettingsStore(Path(directory) / "config.json")
            self.assertEqual(settings.get(path), MACOS_SELECTION_HOTKEY_DEFAULTS[HOTKEY_SELECTION_CASE])
            self.assertEqual(settings_snapshot(settings)["overrides"], {})
            settings.set(path, DEFAULT_SELECTION_CASE_HOTKEY)
            self.assertEqual(settings_snapshot(settings)["overrides"], {path: DEFAULT_SELECTION_CASE_HOTKEY})
            self.assertTrue(settings.restore_default(path))
            self.assertEqual(settings.get(path), MACOS_SELECTION_HOTKEY_DEFAULTS[HOTKEY_SELECTION_CASE])
            settings.set(path, DEFAULT_SELECTION_CASE_HOTKEY)
            settings.reset()
            self.assertEqual(settings.get(path), MACOS_SELECTION_HOTKEY_DEFAULTS[HOTKEY_SELECTION_CASE])
            self.assertEqual(settings.defaults(), default_settings("darwin"))


class FakeBackend:
    """The engine's backend contract, with no way to read a selection."""

    def __init__(self) -> None:
        self.group = 0
        self.application = "TestEditor"
        self.focus: FocusInfo | None = FocusInfo(SELECTION_FIXTURE_WINDOW)
        self.holds = 0
        self.releases = 0

    def active_application(self) -> str:
        return self.application

    def focused_window(self) -> FocusInfo | None:
        return self.focus

    def current_group(self) -> int:
        return self.group

    def switch_group(self, group: int) -> None:
        self.group = group

    def hold_input(self) -> None:
        self.holds += 1

    def release_input(self) -> int:
        self.releases += 1
        return 0

    def complete_action(self, deliver: bool) -> int:
        return 0

    def inject_correction(self, *args: object, **kwargs: object) -> int:
        raise AssertionError("a selection command types nothing through the word correction")

    def set_key_filter(self, predicate: Callable[[KeyEvent], KeyDisposition] | None) -> None:
        self.key_filter = predicate

    def start(self, listener: Callable[[KeyEvent], None]) -> None:
        self.listener = listener

    def stop(self) -> None:
        return None

    def close(self) -> None:
        return None

    def probe(self) -> BackendProbe:
        return BackendProbe(True, "x11", ":test", "1", "2", "1", self.group)


class FakeSelectionBackend(FakeBackend):
    """A backend with a selection to read and a record of what was typed over it."""

    def __init__(self) -> None:
        super().__init__()
        self.selected: SelectedText | SelectionRefused = SelectedText("ghbdtn", SOURCE_ATSPI)
        self.replace_error: Exception | None = None
        self.reads: list[tuple[str, int]] = []
        self.replaced: list[tuple[str, int | None]] = []

    def read_selection(self, application: str, window: int) -> SelectedText:
        self.reads.append((application, window))
        if isinstance(self.selected, SelectionRefused):
            raise self.selected
        return self.selected

    def replace_selection(self, selected: SelectedText, replacement: str, final_group: int | None) -> bool:
        if self.replace_error is not None:
            raise self.replace_error
        self.replaced.append((replacement, final_group))
        if final_group is not None:
            self.group = final_group
        return True


def key(name: str, keycode: int, *, pressed: bool = True, state: int = 0, character: str = "") -> KeyEvent:
    return KeyEvent(pressed, keycode, name, character, (character, character), 0, state, keycode)


class EngineSelectionTests(unittest.TestCase):

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.build(Path(self.temporary.name))

    def build(self, root: Path) -> None:
        root.mkdir(exist_ok=True)
        self.settings = SettingsStore(root / "config.json")
        self.history = HistoryStore(root / "history.jsonl")
        self.backend = FakeSelectionBackend()
        self.engine = KeySwitchEngine(self.settings, self.history, self.backend)

    def press(self, name: str, keycode: int, modifier: str, modifier_keycode: int, mask: int) -> None:
        """Press the modifier, then the key, and let the key and then the modifier come up."""

        typed = list(self.backend.replaced)
        self.engine._handle(key(modifier, modifier_keycode))
        self.engine._handle(key(name, keycode, state=mask))
        self.engine._handle(key(name, keycode, pressed=False, state=mask))
        self.assertEqual(self.backend.replaced, typed, "nothing is typed while a key of the hotkey is down")
        self.engine._handle(key(modifier, modifier_keycode, pressed=False, state=mask))

    def convert(self) -> None:
        self.press("Pause", PAUSE_KEYCODE, "Shift_L", SHIFT_L_KEYCODE, SHIFT_MASK)

    def test_the_layout_command_types_the_other_layout_and_switches_to_it(self) -> None:
        self.convert()
        self.assertEqual(self.backend.reads, [("TestEditor", SELECTION_FIXTURE_WINDOW)])
        self.assertEqual(self.backend.replaced, [("привет", 1)])
        self.assertEqual((self.backend.holds, self.backend.releases), (1, 1))
        self.assertEqual(self.engine.snapshot.current_group, 1)
        self.assertEqual(self.engine._manual_layout_group, 1)
        self.assertEqual(self.engine._manual_layout_source, "selection_layout")
        self.assertEqual(self.engine.snapshot.last_action, COMMAND_TITLES["layout"])
        self.assertIsNone(self.engine._selection_pending)

    def test_without_manual_layout_respect_the_switch_protects_nothing(self) -> None:
        self.settings.set("detection.respect_manual_layout", False)
        self.convert()
        self.assertEqual(self.backend.replaced, [("привет", 1)])
        self.assertIsNone(self.engine._manual_layout_group)

    def test_case_and_transliteration_keep_the_layout(self) -> None:
        self.backend.selected = SelectedText("привет", SOURCE_ATSPI)
        self.press("Pause", PAUSE_KEYCODE, "Alt_L", ALT_L_KEYCODE, ALT_MASK)
        self.press("Scroll_Lock", SELECTION_FIXTURE_SCROLL_LOCK_KEYCODE, "Alt_L", ALT_L_KEYCODE, ALT_MASK)
        self.assertEqual(self.backend.replaced, [("ПРИВЕТ", None), ("privet", None)])
        self.assertEqual(self.engine.snapshot.current_group, 0)
        self.assertEqual(self.engine.snapshot.last_action, COMMAND_TITLES["transliterate"])

    def test_a_selection_the_command_does_not_change_types_nothing(self) -> None:
        self.backend.selected = SelectedText("123", SOURCE_ATSPI)
        self.convert()
        self.assertEqual(self.backend.replaced, [])
        self.assertEqual(self.engine.snapshot.last_action, "Выделение не изменилось")

    def test_the_engine_refuses_where_no_correction_is_made(self) -> None:
        cases: list[tuple[RefusalReason, Callable[[], None]]] = [
            ("disabled", lambda: self.settings.set("enabled", False)),
            ("own_window", lambda: setattr(self.backend, "focus", FocusInfo(SELECTION_FIXTURE_WINDOW, own=True))),
            ("sensitive_field", self.sensitive_window),
            ("excluded_application", lambda: self.settings.set("exclusions.applications", ["testeditor"])),
        ]
        for reason, arrange in cases:
            with self.subTest(reason=reason):
                self.build(Path(self.temporary.name) / reason)
                arrange()
                self.convert()
                self.assertEqual(self.backend.reads, [])
                self.assertEqual(self.engine.snapshot.last_action, REFUSAL_MESSAGES[reason])

    def sensitive_window(self) -> None:
        self.engine._sensitive_context_window = SELECTION_FIXTURE_WINDOW
        self.engine._focus_window = SELECTION_FIXTURE_WINDOW

    def test_a_backend_without_the_selection_calls_is_reported(self) -> None:
        backend = FakeBackend()
        self.assertFalse(isinstance(backend, SelectionBackend))
        self.assertIsInstance(self.backend, SelectionBackend)
        engine = KeySwitchEngine(self.settings, self.history, backend)
        engine._run_selection_command("case")
        self.assertEqual(engine.snapshot.last_action, REFUSAL_MESSAGES["unsupported"])

    def test_without_a_focused_window_the_backend_finds_the_field_itself(self) -> None:
        self.backend.focus = None
        self.convert()
        self.assertEqual(self.backend.reads, [("TestEditor", 0)])

    def test_refusals_of_the_backend_and_of_the_shared_checks_reach_the_status_line(self) -> None:
        self.backend.selected = SelectionRefused("no_selection")
        self.convert()
        self.assertEqual(self.engine.snapshot.last_action, REFUSAL_MESSAGES["no_selection"])
        self.backend.selected = SelectedText("first\nsecond", SOURCE_ATSPI)
        self.convert()
        self.assertEqual(self.engine.snapshot.last_action, REFUSAL_MESSAGES["line_breaks"])
        self.assertEqual(self.backend.holds, self.backend.releases)
        self.assertEqual(self.backend.replaced, [])

    def test_a_failure_while_typing_is_an_error_and_an_unconfirmed_switch_is_still_the_engines(self) -> None:
        self.backend.replace_error = RuntimeError("typing failed")
        self.convert()
        self.assertEqual(self.engine.snapshot.last_error, "typing failed")
        self.assertEqual(self.engine.snapshot.last_action, "Ошибка замены выделения · проверьте текст в приложении")
        self.backend.replace_error = LayoutSwitchUnconfirmed("not confirmed", 1)
        self.convert()
        self.assertEqual(self.engine._engine_switch_group, 1)

    def test_keys_that_never_come_up_give_the_command_up(self) -> None:
        self.engine._handle(key("Shift_L", SHIFT_L_KEYCODE))
        self.engine._handle(key("Pause", PAUSE_KEYCODE, state=SHIFT_MASK))
        pending = self.engine._selection_pending
        assert pending is not None
        self.assertEqual(pending, PendingSelection("layout", pending.deadline))
        self.engine._expire_manual_correction()
        self.assertIsNotNone(self.engine._selection_pending)
        with patch("keyswitch.engine.time.monotonic", return_value=pending.deadline):
            self.engine._expire_manual_correction()
        self.assertIsNone(self.engine._selection_pending)
        self.assertEqual(self.engine.snapshot.last_action, REFUSAL_MESSAGES["release_timeout"])
        self.engine._advance_selection_command()
        self.assertEqual(self.backend.reads, [])

    def test_the_word_being_typed_is_let_go_when_a_selection_hotkey_is_pressed(self) -> None:
        self.engine._handle(KeyEvent(True, A_KEYCODE, "a", "a", ("a", "ф"), 0, 0, A_KEYCODE))
        self.assertTrue(self.engine._strokes)
        self.engine._handle(key("Shift_L", SHIFT_L_KEYCODE))
        self.engine._handle(key("Pause", PAUSE_KEYCODE, state=SHIFT_MASK))
        self.assertEqual(self.engine._strokes, [])

    def test_the_log_gets_lengths_and_never_the_text(self) -> None:
        self.settings.set("diagnostics.technical_logging", True)
        with self.assertLogs("keyswitch.engine", level="INFO") as logs:
            self.convert()
        records = [line for line in logs.output if '"event":"selection_command"' in line]
        self.assertEqual(len(records), 1)
        self.assertNotIn("ghbdtn", records[0])
        self.assertNotIn("привет", records[0])
        payload = json.loads(records[0].split("TECHNICAL ", 1)[1])
        self.assertEqual(
            (payload["outcome"], payload["selected_characters"], payload["replacement_characters"], payload["source"]),
            ("applied", len("ghbdtn"), len("привет"), SOURCE_ATSPI),
        )

    def test_only_the_layouts_with_models_are_named(self) -> None:
        self.settings.set("detection.layouts", ["us", "ru", "de"])
        self.assertEqual(self.engine._layout_names(), LAYOUTS)


def atspi_api() -> MagicMock:
    api = MagicMock()
    api.init.return_value = 0
    desktop = api.get_desktop.return_value
    desktop.get_child_count.return_value = 1
    application = desktop.get_child_at_index.return_value
    application.get_name.return_value = "chat"
    application.get_process_id.return_value = 0
    application.get_state_set.return_value.contains.return_value = False
    application.get_child_count.return_value = 1
    node = application.get_child_at_index.return_value
    node.get_child_count.return_value = 0
    node.get_state_set.return_value.contains.return_value = True
    node.get_role.return_value = "text"
    text = node.get_text_iface.return_value
    text.get_n_selections.return_value = 1
    span = api.Text.get_selection.return_value
    span.start_offset, span.end_offset = len("до "), len("до ghbdtn")
    api.Text.get_text.return_value = "ghbdtn"
    return api


class AtspiSelectionTests(unittest.TestCase):

    def setUp(self) -> None:
        _native_api.cache_clear()
        self.addCleanup(_native_api.cache_clear)
        self.api = atspi_api()
        self.node = self.api.get_desktop.return_value.get_child_at_index.return_value.get_child_at_index.return_value
        self.text = self.node.get_text_iface.return_value
        self.reader = AtspiFieldReader(self.api)

    def refusal(self) -> str:
        with self.assertRaises(SelectionRefused) as caught:
            self.reader.read_selection("chat", SELECTION_FIXTURE_WINDOW)
        return caught.exception.reason

    def test_the_selection_of_the_focused_editable_field(self) -> None:
        self.assertEqual(self.reader.read_selection("chat", SELECTION_FIXTURE_WINDOW), SelectedText("ghbdtn", SOURCE_ATSPI))
        self.api.Text.get_text.assert_called_once_with(self.text, len("до "), len("до ghbdtn"))
        self.api.Text.get_selection.assert_called_once_with(self.text, 0)

    def test_a_long_selection_is_read_one_character_past_the_bound(self) -> None:
        span = self.api.Text.get_selection.return_value
        span.start_offset, span.end_offset = 0, SELECTION_COMMAND_MAX_CHARACTERS + SELECTION_COMMAND_MAX_CHARACTERS
        self.reader.read_selection("chat", SELECTION_FIXTURE_WINDOW)
        self.api.Text.get_text.assert_called_once_with(self.text, 0, SELECTION_COMMAND_MAX_CHARACTERS + 1)

    def test_what_is_not_a_selection_to_change(self) -> None:
        self.node.get_role.return_value = self.api.Role.PASSWORD_TEXT
        self.assertEqual(self.refusal(), "sensitive_field")
        self.node.get_role.return_value = "text"
        self.api.Text.get_text.return_value = "•••"
        self.assertEqual(self.refusal(), "sensitive_field")
        self.api.Text.get_text.return_value = "ghbdtn"
        span = self.api.Text.get_selection.return_value
        span.end_offset = span.start_offset
        self.assertEqual(self.refusal(), "no_selection")
        self.text.get_n_selections.return_value = 0
        self.assertEqual(self.refusal(), "no_selection")
        self.text.get_n_selections.return_value = SELECTION_FIXTURE_SEVERAL_SELECTIONS
        self.assertEqual(self.refusal(), "multiple_selection")
        self.node.get_state_set.return_value.contains.side_effect = lambda state: state is self.api.StateType.FOCUSED
        self.assertEqual(self.refusal(), "read_only")
        self.node.get_text_iface.return_value = None
        self.assertEqual(self.refusal(), "unsupported")
        self.node.get_state_set.return_value.contains.side_effect = None
        self.node.get_state_set.return_value.contains.return_value = False
        self.assertEqual(self.refusal(), "unsupported")

    def test_the_reader_of_the_backends_is_made_once_and_reports_a_broken_bus_as_unsupported(self) -> None:
        with patch("keyswitch.atspi_context.AtspiFieldReader") as factory:
            factory.return_value.read_selection.return_value = SelectedText("x", SOURCE_ATSPI)
            reader = AtspiSelectionReader(lambda window: window)
            self.assertEqual(reader.read_selection("chat", SELECTION_FIXTURE_WINDOW), SelectedText("x", SOURCE_ATSPI))
            reader.read_selection("chat", SELECTION_FIXTURE_WINDOW)
            factory.assert_called_once()
            factory.return_value.read_selection.side_effect = SelectionRefused("read_only")
            with self.assertRaises(SelectionRefused) as caught:
                reader.read_selection("chat", SELECTION_FIXTURE_WINDOW)
            self.assertEqual(caught.exception.reason, "read_only")
            factory.return_value.read_selection.side_effect = RuntimeError("no bus")
            with self.assertRaises(SelectionRefused) as caught:
                reader.read_selection("chat", SELECTION_FIXTURE_WINDOW)
            self.assertEqual(caught.exception.reason, "unsupported")
            factory.return_value.write_selection.return_value = True
            self.assertTrue(reader.write_selection(SelectedText("x", SOURCE_ATSPI), "y"))
        self.assertIsNone(AtspiSelectionReader().write_selection(SelectedText("x", SOURCE_ATSPI), "y"))


class AtspiEditableTextTests(unittest.TestCase):
    """The selection written through EditableText, with a mocked AT-SPI."""

    def setUp(self) -> None:
        self.api = atspi_api()
        self.node = self.api.get_desktop.return_value.get_child_at_index.return_value.get_child_at_index.return_value
        self.text = self.node.get_text_iface.return_value
        self.editable = self.node.get_editable_text_iface.return_value
        self.api.EditableText.delete_text.return_value = True
        self.api.EditableText.insert_text.return_value = True
        self.api.Text.set_selection.return_value = True
        self.reader = AtspiFieldReader(self.api)
        self.selected = self.reader.read_selection("chat", SELECTION_FIXTURE_WINDOW)

    def refusal(self, selected: SelectedText | None = None) -> str:
        with self.assertRaises(SelectionRefused) as caught:
            self.reader.write_selection(selected or self.selected, "привет — «мир»")
        return caught.exception.reason

    def test_any_character_is_written_over_the_selection_and_selected_again(self) -> None:
        replacement = "привет — «мир»"
        self.assertTrue(self.reader.write_selection(self.selected, replacement))
        start, end = len("до "), len("до ghbdtn")
        self.api.EditableText.delete_text.assert_called_once_with(self.editable, start, end)
        self.api.EditableText.insert_text.assert_called_once_with(
            self.editable, start, replacement, len(replacement.encode("utf-8")))
        self.api.Text.set_selection.assert_called_once_with(self.text, 0, start, start + len(replacement))
        self.assertEqual(self.refusal(), "focus_changed", "a written selection is not written twice")

    def test_a_field_without_editable_text_is_typed_into_instead(self) -> None:
        self.node.get_editable_text_iface.return_value = None
        self.assertIsNone(self.reader.write_selection(self.selected, "привет"))
        self.api.EditableText.delete_text.assert_not_called()

    def test_a_selection_that_changed_since_it_was_read_is_left_alone(self) -> None:
        self.assertEqual(self.refusal(SelectedText("other", SOURCE_ATSPI)), "focus_changed")
        self.api.Text.get_text.return_value = "changed"
        self.assertEqual(self.refusal(), "focus_changed")
        self.api.Text.get_text.return_value = "ghbdtn"
        span = self.api.Text.get_selection.return_value
        span.start_offset = 0
        self.assertEqual(self.refusal(), "focus_changed")
        span.start_offset = len("до ")
        self.node.get_state_set.return_value.contains.return_value = False
        self.assertEqual(self.refusal(), "focus_changed")
        self.text.get_n_selections.return_value = 0
        self.assertEqual(self.refusal(), "focus_changed")
        self.api.EditableText.delete_text.assert_not_called()
        with self.assertRaises(SelectionRefused) as caught:
            AtspiFieldReader(self.api).write_selection(self.selected, "привет")
        self.assertEqual(caught.exception.reason, "focus_changed")

    def test_a_refused_delete_changes_nothing_and_a_refused_insert_puts_the_selection_back(self) -> None:
        self.api.EditableText.delete_text.return_value = False
        self.assertEqual(self.refusal(), "not_settable")
        self.api.EditableText.delete_text.return_value = True
        self.api.EditableText.insert_text.side_effect = [False, True]
        self.reader.read_selection("chat", SELECTION_FIXTURE_WINDOW)
        with self.assertRaisesRegex(RuntimeError, "возвращён"):
            self.reader.write_selection(self.selected, "привет")
        self.api.EditableText.insert_text.assert_called_with(
            self.editable, len("до "), "ghbdtn", len("ghbdtn".encode("utf-8")))
        self.api.EditableText.insert_text.side_effect = [False, False]
        self.reader.read_selection("chat", SELECTION_FIXTURE_WINDOW)
        with self.assertRaisesRegex(RuntimeError, "не вернулся"):
            self.reader.write_selection(self.selected, "привет")


class WrittenSource:
    """A selection source whose field takes the text itself (``written`` is True or False) or not at all (None)."""

    def __init__(self, written: bool | None) -> None:
        self.written = written
        self.writes: list[tuple[SelectedText, str]] = []

    def read_selection(self, application: str, window: int) -> SelectedText:
        return SelectedText("ghbdtn", SOURCE_ATSPI)

    def write_selection(self, selected: SelectedText, replacement: str) -> bool | None:
        self.writes.append((selected, replacement))
        return self.written


# A fake keyboard of two groups: each key's plain and shifted character in the English and the Russian group.
KEYBOARD: dict[int, tuple[tuple[str, str], tuple[str, str]]] = {
    A_KEYCODE: (("a", "A"), ("ф", "Ф")),
    COMMA_KEYCODE: ((",", "<"), ("б", "Б")),
    ALTERNATE_COMMA_KEYCODE: ((".", ">"), ("ю", "Ю")),
    SELECTION_FIXTURE_SLASH_KEYCODE: (("/", "?"), (".", ",")),
    SPACE_KEYCODE: ((" ", " "), (" ", " ")),
}


def keyboard_character(keycode: int, group: int, state: int) -> str:
    keys = KEYBOARD.get(keycode)
    return "" if keys is None else keys[group][bool(state & SHIFT_MASK)]


class X11SelectionTests(unittest.TestCase):

    def setUp(self) -> None:
        self.backend, self.libraries = backend_with()
        self.backend._control = 1
        self.state = {"group": 0, "locked_mods": 0, "result": 0}

        def get_state(_display: int, _device: int, pointer: ctypes._CData | ctypes._CArgObject | int) -> int:
            record = ctypes.cast(pointer, ctypes.POINTER(XkbStateRec)).contents
            record.group = self.state["group"]
            record.locked_mods = self.state["locked_mods"]
            return self.state["result"]

        self.libraries.x11.XkbGetState.side_effect = get_state
        self.libraries.x11.XKeysymToKeycode.side_effect = lambda _display, keysym: {
            SHIFT_L_KEYSYM: SHIFT_L_KEYCODE, LEFT_KEYSYM: X11_LEFT_ARROW_KEYCODE}.get(keysym, 0)
        patcher = patch.object(self.backend, "_character_for_keycode", side_effect=keyboard_character)
        self.lookup = patcher.start()
        self.addCleanup(patcher.stop)

    def typed(self) -> list[tuple[int, bool]]:
        return [(call.args[1], bool(call.args[XTEST_FAKE_KEY_EVENT_IS_PRESS_ARG_INDEX]))
                for call in self.libraries.xtst.XTestFakeKeyEvent.call_args_list]

    def groups(self) -> list[int]:
        return [call.args[XKB_LOCK_GROUP_GROUP_ARG_INDEX] for call in self.libraries.x11.XkbLockGroup.call_args_list]

    def test_each_run_is_typed_in_its_group_and_the_result_selected_back(self) -> None:
        self.assertTrue(self.backend.replace_selection(SelectedText("f&", SOURCE_ATSPI), "фA?,", 1))

        def shifted(keycode: int) -> list[tuple[int, bool]]:
            return [(SHIFT_L_KEYCODE, True), (keycode, True), (keycode, False), (SHIFT_L_KEYCODE, False)]

        left = [(X11_LEFT_ARROW_KEYCODE, True), (X11_LEFT_ARROW_KEYCODE, False)]
        self.assertEqual(self.typed(), [
            (A_KEYCODE, True), (A_KEYCODE, False), *shifted(A_KEYCODE), *shifted(SELECTION_FIXTURE_SLASH_KEYCODE),
            (COMMA_KEYCODE, True), (COMMA_KEYCODE, False),
            (SHIFT_L_KEYCODE, True), *left * len("фA?,"), (SHIFT_L_KEYCODE, False),
        ])
        # The Russian run, the English one, then the layout the command leaves.
        self.assertEqual(self.groups(), [1, 0, 1])
        self.libraries.x11.XSync.assert_called_once_with(1, 0)
        lookups = {call.args[0] for call in self.lookup.call_args_list}
        self.assertEqual((min(lookups), max(lookups)), (X11_MIN_KEYCODE, X11_MAX_KEYCODE))

    def test_caps_lock_turns_shift_over_for_letters_only_and_the_group_comes_back(self) -> None:
        self.state["locked_mods"] = LOCK_MASK
        self.state["group"] = 1
        with patch("keyswitch.x11_backend.caret_steps", return_value=None):
            self.assertFalse(self.backend.replace_selection(SelectedText("x", SOURCE_ATSPI), "Ф.", None))
        self.assertEqual(self.typed(), [
            (A_KEYCODE, True), (A_KEYCODE, False),
            (SELECTION_FIXTURE_SLASH_KEYCODE, True), (SELECTION_FIXTURE_SLASH_KEYCODE, False),
        ])
        self.assertEqual(self.groups(), [1, 1])
        self.assertEqual(len(self.backend._expected), SELECTION_FIXTURE_EVENTS_PER_TAP * len("Ф."))

    def test_nothing_is_typed_when_a_character_has_no_key(self) -> None:
        with self.assertRaises(SelectionRefused) as caught:
            self.backend.replace_selection(SelectedText("x", SOURCE_ATSPI), "a😀", 0)
        self.assertEqual(caught.exception.reason, "untypeable")
        self.assertEqual(self.typed(), [])

    def test_failures_leave_no_expected_keys_behind(self) -> None:
        self.libraries.x11.XkbLockGroup.return_value = 0
        with self.assertRaisesRegex(X11Error, "XKB-группу"):
            self.backend.replace_selection(SelectedText("x", SOURCE_ATSPI), "a", 0)
        self.assertFalse(self.backend._expected)
        self.libraries.x11.XkbLockGroup.return_value = 1
        self.libraries.xtst.XTestFakeKeyEvent.return_value = 0
        with self.assertRaisesRegex(X11Error, "XTest"):
            self.backend.replace_selection(SelectedText("x", SOURCE_ATSPI), "a", 0)
        self.assertEqual(self.libraries.xtst.XTestGrabControl.call_args_list[-1].args, (1, 0))

    def test_a_closed_display_an_unreadable_state_and_missing_keys_are_errors(self) -> None:
        self.state["result"] = 1
        with self.assertRaisesRegex(X11Error, "состояние клавиатуры"):
            self.backend.replace_selection(SelectedText("x", SOURCE_ATSPI), "a", 0)
        self.state["result"] = 0
        self.libraries.x11.XKeysymToKeycode.side_effect = lambda _display, _keysym: 0
        with self.assertRaisesRegex(X11Error, "Shift/Left"):
            self.backend.replace_selection(SelectedText("x", SOURCE_ATSPI), "a", 0)
        self.backend._control = None
        with self.assertRaisesRegex(X11Error, "не запущен"):
            self.backend.replace_selection(SelectedText("x", SOURCE_ATSPI), "a", 0)

    def test_a_field_with_editable_text_takes_the_text_and_only_the_layout_moves(self) -> None:
        source = WrittenSource(True)
        self.backend._selection_source = source
        self.assertTrue(self.backend.replace_selection(SelectedText("ghbdtn", SOURCE_ATSPI), "привет — мир", 1))
        self.backend._selection_source = WrittenSource(False)
        self.assertFalse(self.backend.replace_selection(SelectedText("ghbdtn", SOURCE_ATSPI), "привет", None))
        self.assertEqual(self.typed(), [])
        self.assertEqual(self.groups(), [1])
        self.assertEqual(source.writes, [(SelectedText("ghbdtn", SOURCE_ATSPI), "привет — мир")])

    def test_without_editable_text_the_result_is_typed(self) -> None:
        self.backend._selection_source = WrittenSource(None)
        self.assertTrue(self.backend.replace_selection(SelectedText("f", SOURCE_ATSPI), "ф", 1))
        self.assertIn((A_KEYCODE, True), self.typed())
        with self.assertRaises(SelectionRefused) as caught:
            self.backend.replace_selection(SelectedText("x", SOURCE_ATSPI), "—", None)
        self.assertEqual(caught.exception.reason, "untypeable")

    def test_the_selection_is_read_through_at_spi(self) -> None:
        with patch("keyswitch.atspi_context.AtspiSelectionReader") as reader:
            reader.return_value.read_selection.return_value = SelectedText("ghbdtn", SOURCE_ATSPI)
            self.assertEqual(self.backend.read_selection("chat", SELECTION_FIXTURE_WINDOW).text, "ghbdtn")
            self.backend.read_selection("chat", SELECTION_FIXTURE_WINDOW)
        reader.assert_called_once_with(self.backend.window_process_id)


class WaylandSelectionTests(unittest.TestCase):

    def setUp(self) -> None:
        self.api = FakeWaylandAPI()
        self.api.record_editor = True
        self.backend = WaylandBackend(self.api)

    def test_the_result_is_typed_in_its_layout_selected_back_and_the_layout_moved(self) -> None:
        self.assertTrue(self.backend.replace_selection(SelectedText("ghbdtn", SOURCE_ATSPI), "Фа ф", 1))
        self.assertEqual("".join(self.api.editor_text), "Фа ф")
        self.assertEqual(self.api.current_index, WL_FIXTURE_LAYOUT_RU_INDEX)
        reselect = self.api.posted[-1]
        self.assertEqual((reselect[0], reselect[-1]), ((True, EVDEV_KEY_LEFTSHIFT), (False, EVDEV_KEY_LEFTSHIFT)))
        self.assertEqual(reselect.count((True, EVDEV_KEY_LEFT)), len("Фа ф"))
        self.assertEqual(
            len(self.api.posted[0]),
            SELECTION_FIXTURE_EVENTS_PER_SHIFTED_TAP + len("а ф") * SELECTION_FIXTURE_EVENTS_PER_TAP,
        )

    def test_runs_of_two_layouts_switch_between_them_and_come_back(self) -> None:
        with patch("keyswitch.wayland_backend.time.sleep"):
            self.assertTrue(self.backend.replace_selection(SelectedText("x", SOURCE_ATSPI), "a ф", None))
        self.assertEqual("".join(self.api.editor_text), "a ф")
        self.assertEqual(self.api.switch_requests, [WL_FIXTURE_LAYOUT_RU_INDEX, WL_FIXTURE_LAYOUT_EN_INDEX])

    def test_caps_lock_is_read_into_the_keys_and_an_unknown_layout_is_left_alone(self) -> None:
        self.api.caps_lock = True
        self.api.current_index = len(self.api.names)
        with patch("keyswitch.wayland_backend.caret_steps", return_value=None):
            self.assertFalse(self.backend.replace_selection(SelectedText("x", SOURCE_ATSPI), "aA", None))
        self.assertEqual("".join(self.api.editor_text), "aA")

    def test_a_key_that_reached_the_program_or_another_window_stops_the_typing(self) -> None:
        self.backend.hold_input()
        self.backend._hold_broken = True
        with self.assertRaises(WaylandBackendError):
            self.backend.replace_selection(SelectedText("x", SOURCE_ATSPI), "a", None)
        self.backend._hold_broken = False
        self.api.window = SELECTION_FIXTURE_OTHER_WINDOW
        with self.assertRaises(SelectionRefused) as caught:
            self.backend.replace_selection(SelectedText("x", SOURCE_ATSPI), "a", None)
        self.assertEqual(caught.exception.reason, "focus_changed")
        self.assertEqual(self.api.editor_text, [])

    def test_a_field_with_editable_text_takes_the_text_and_only_the_layout_moves(self) -> None:
        self.backend._selection_source = WrittenSource(False)
        self.assertFalse(self.backend.replace_selection(SelectedText("ghbdtn", SOURCE_ATSPI), "«привет»", 1))
        self.assertEqual(self.api.posted, [])
        self.assertEqual(self.api.current_index, WL_FIXTURE_LAYOUT_RU_INDEX)
        self.backend._selection_source = WrittenSource(True)
        self.assertTrue(self.backend.replace_selection(SelectedText("ghbdtn", SOURCE_ATSPI), "привет", None))
        self.assertEqual(self.api.switch_requests, [WL_FIXTURE_LAYOUT_RU_INDEX])
        self.backend.hold_input()
        self.api.window = SELECTION_FIXTURE_OTHER_WINDOW
        with self.assertRaises(SelectionRefused):
            self.backend.replace_selection(SelectedText("ghbdtn", SOURCE_ATSPI), "привет", None)

    def test_without_editable_text_the_result_is_typed(self) -> None:
        self.backend._selection_source = WrittenSource(None)
        self.assertTrue(self.backend.replace_selection(SelectedText("ghbdtn", SOURCE_ATSPI), "ф", None))
        self.assertEqual("".join(self.api.editor_text), "ф")

    def test_a_password_window_is_refused_before_any_read(self) -> None:
        self.api.password_windows.add(WL_FIXTURE_WINDOW_ID)
        with self.assertRaises(SelectionRefused) as caught:
            self.backend.read_selection("editor", WL_FIXTURE_WINDOW_ID)
        self.assertEqual(caught.exception.reason, "sensitive_field")
        self.api.password_windows.clear()
        with patch("keyswitch.atspi_context.AtspiSelectionReader") as reader:
            reader.return_value.read_selection.return_value = SelectedText("ghbdtn", SOURCE_ATSPI)
            self.assertEqual(self.backend.read_selection("editor", 0).text, "ghbdtn")
            self.backend.read_selection("editor", WL_FIXTURE_WINDOW_ID)
        reader.assert_called_once_with(self.backend.window_process_id)


if __name__ == "__main__":
    unittest.main()
