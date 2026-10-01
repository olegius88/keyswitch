"""GTK learning prompt and accessibility-anchor tests."""

from __future__ import annotations

import os
import unittest
from types import SimpleNamespace
from typing import ClassVar
from unittest.mock import Mock, patch

import dbus
import gi

gi.require_version("Atspi", "2.0")
gi.require_version("Gdk", "4.0")
gi.require_version("GdkX11", "4.0")
gi.require_version("Gtk", "4.0")
from gi.repository import Atspi, Gdk, GdkX11, GLib, Gtk

from keyswitch import learning_prompt as prompt_module
from keyswitch.backend import ScreenAnchor
from keyswitch.engine import LearningPrompt
from keyswitch.learning import LearnedRule
from keyswitch.learning_prompt import LearningPromptWindow, RuleEditorWindow, focused_caret_anchor
from keyswitch.rule_editor import PROMPT_TITLE, RuleDraft
from keyswitch.constants.geometry import CENTERING_DIVISOR
from keyswitch.constants.learning_prompt import (
    LEARNING_PROMPT_ANCHOR_GAP_PIXELS,
    LEARNING_PROMPT_GTK_WIDTH_PIXELS,
)
from fixture_values.counts import (
    LEARNING_PROMPT_CHILD_COUNT_WITH_NULL_SLOT,
    LEARNING_PROMPT_DISMISS_CALLS_AFTER_CLOSE_WITH_PROMPT,
    LEARNING_PROMPT_DISMISS_CALLS_AFTER_ESCAPE_AND_STRAY_KEY,
    LEARNING_PROMPT_FOCUSED_CARET_OFFSET,
)
from fixture_values.keys import (
    A_KEYCODE,
    ESCAPE_KEYCODE,
    LEARNING_PROMPT_FIXTURE_WINDOW_ID,
    RETURN_KEYCODE,
)
from fixture_values.ui import (
    LEARNING_PROMPT_CARET_ONLY_ANCHOR_X,
    LEARNING_PROMPT_CARET_ONLY_ANCHOR_Y,
    LEARNING_PROMPT_CARET_OVERRIDE_ANCHOR_X,
    LEARNING_PROMPT_CARET_OVERRIDE_ANCHOR_Y,
    LEARNING_PROMPT_CHILD_RECT_HEIGHT,
    LEARNING_PROMPT_CHILD_RECT_WIDTH,
    LEARNING_PROMPT_CHILD_RECT_X,
    LEARNING_PROMPT_CHILD_RECT_Y,
    LEARNING_PROMPT_FALLBACK_ANCHOR_WINDOW,
    LEARNING_PROMPT_FALLBACK_ANCHOR_X,
    LEARNING_PROMPT_FALLBACK_ANCHOR_Y,
    LEARNING_PROMPT_FIRST_SCREEN_ANCHOR_WINDOW,
    LEARNING_PROMPT_FIRST_SCREEN_ANCHOR_X,
    LEARNING_PROMPT_FIRST_SCREEN_ANCHOR_Y,
    LEARNING_PROMPT_FIXTURE_WINDOW_HEIGHT_PIXELS,
    LEARNING_PROMPT_FOCUSED_RECT_HEIGHT,
    LEARNING_PROMPT_FOCUSED_RECT_WIDTH,
    LEARNING_PROMPT_FOCUSED_RECT_X,
    LEARNING_PROMPT_FOCUSED_RECT_Y,
)


DISPLAY_AVAILABLE = bool(os.environ.get("DISPLAY")) and Gtk.init_check()

# Accessible-text character-extents fixtures; expected anchors below are derived
# from these, the same way focused_caret_anchor() derives them from the real ones.
FOCUSED_RECT = SimpleNamespace(
    x=LEARNING_PROMPT_FOCUSED_RECT_X,
    y=LEARNING_PROMPT_FOCUSED_RECT_Y,
    width=LEARNING_PROMPT_FOCUSED_RECT_WIDTH,
    height=LEARNING_PROMPT_FOCUSED_RECT_HEIGHT,
)
CHILD_RECT = SimpleNamespace(
    x=LEARNING_PROMPT_CHILD_RECT_X,
    y=LEARNING_PROMPT_CHILD_RECT_Y,
    width=LEARNING_PROMPT_CHILD_RECT_WIDTH,
    height=LEARNING_PROMPT_CHILD_RECT_HEIGHT,
)

FIRST_SCREEN_ANCHOR = ScreenAnchor(
    LEARNING_PROMPT_FIRST_SCREEN_ANCHOR_X,
    LEARNING_PROMPT_FIRST_SCREEN_ANCHOR_Y,
    LEARNING_PROMPT_FIRST_SCREEN_ANCHOR_WINDOW,
)
CARET_OVERRIDE_WITH_BACKEND_ANCHOR = ScreenAnchor(
    LEARNING_PROMPT_CARET_OVERRIDE_ANCHOR_X, LEARNING_PROMPT_CARET_OVERRIDE_ANCHOR_Y
)
CARET_ONLY_ANCHOR = ScreenAnchor(
    LEARNING_PROMPT_CARET_ONLY_ANCHOR_X, LEARNING_PROMPT_CARET_ONLY_ANCHOR_Y
)
FALLBACK_ONLY_ANCHOR = ScreenAnchor(
    LEARNING_PROMPT_FALLBACK_ANCHOR_X,
    LEARNING_PROMPT_FALLBACK_ANCHOR_Y,
    LEARNING_PROMPT_FALLBACK_ANCHOR_WINDOW,
)


class FakePromptBackend:
    def __init__(self) -> None:
        self.anchor: ScreenAnchor | None = FIRST_SCREEN_ANCHOR
        self.positions: list[tuple[int, int, int]] = []
        self.restored: list[int | None] = []

    def input_anchor(self) -> ScreenAnchor | None:
        return self.anchor

    def position_window(self, window: int, x: int, y: int) -> bool:
        self.positions.append((window, x, y))
        return True

    def restore_window(self, window: int | None) -> bool:
        self.restored.append(window)
        return window is not None


class AccessibilityAnchorTests(unittest.TestCase):
    def test_accessibility_bus_detection_is_safe(self) -> None:
        with (
            patch.dict(os.environ, {"GTK_A11Y": "none"}),
            patch.object(dbus, "SessionBus") as session_bus,
        ):
            self.assertFalse(prompt_module._accessibility_bus_available())
        session_bus.assert_not_called()

        bus = Mock()
        bus.name_has_owner.return_value = True
        with (
            patch.dict(os.environ, {"GTK_A11Y": ""}),
            patch.object(dbus, "SessionBus", return_value=bus),
        ):
            self.assertTrue(prompt_module._accessibility_bus_available())
        bus.list_activatable_names.assert_not_called()

        bus.name_has_owner.return_value = False
        bus.list_activatable_names.return_value = ["org.example.Service", "org.a11y.Bus"]
        with (
            patch.dict(os.environ, {"GTK_A11Y": ""}),
            patch.object(dbus, "SessionBus", return_value=bus),
        ):
            self.assertTrue(prompt_module._accessibility_bus_available())
        bus.list_activatable_names.return_value = []
        with (
            patch.dict(os.environ, {"GTK_A11Y": ""}),
            patch.object(dbus, "SessionBus", return_value=bus),
        ):
            self.assertFalse(prompt_module._accessibility_bus_available())
        with (
            patch.dict(os.environ, {"GTK_A11Y": ""}),
            patch.object(
                dbus,
                "SessionBus",
                side_effect=RuntimeError("session bus unavailable"),
            ),
        ):
            self.assertFalse(prompt_module._accessibility_bus_available())

    def test_no_desktop_empty_tree_and_failure_return_no_anchor(self) -> None:
        with (
            patch.object(prompt_module, "_accessibility_bus_available", return_value=False),
            patch.object(Atspi, "get_desktop_count") as desktop_count,
        ):
            self.assertIsNone(focused_caret_anchor())
        desktop_count.assert_not_called()

        with (
            patch.object(prompt_module, "_accessibility_bus_available", return_value=True),
            patch.object(Atspi, "get_desktop_count", return_value=0),
        ):
            self.assertIsNone(focused_caret_anchor())

        root = Mock()
        root.get_state_set.return_value.contains.return_value = False
        root.get_text_iface.return_value = None
        root.get_child_count.return_value = 0
        with (
            patch.object(prompt_module, "_accessibility_bus_available", return_value=True),
            patch.object(Atspi, "get_desktop_count", return_value=1),
            patch.object(Atspi, "get_desktop", return_value=root),
        ):
            self.assertIsNone(focused_caret_anchor())

        with patch.object(
            prompt_module, "_accessibility_bus_available", return_value=True
        ), patch.object(
            Atspi, "get_desktop_count", side_effect=RuntimeError("a11y unavailable")
        ):
            self.assertIsNone(focused_caret_anchor())

    def test_focused_text_caret_handles_start_and_later_offsets(self) -> None:
        root = Mock()
        root.get_state_set.return_value.contains.return_value = True
        text = Mock()
        root.get_text_iface.return_value = text
        text.get_caret_offset.return_value = LEARNING_PROMPT_FOCUSED_CARET_OFFSET
        text.get_character_extents.return_value = FOCUSED_RECT
        with (
            patch.object(prompt_module, "_accessibility_bus_available", return_value=True),
            patch.object(Atspi, "get_desktop_count", return_value=1),
            patch.object(Atspi, "get_desktop", return_value=root),
        ):
            self.assertEqual(
                focused_caret_anchor(),
                ScreenAnchor(FOCUSED_RECT.x + FOCUSED_RECT.width, FOCUSED_RECT.y + FOCUSED_RECT.height),
            )
            text.get_caret_offset.return_value = 0
            self.assertEqual(
                focused_caret_anchor(), ScreenAnchor(FOCUSED_RECT.x, FOCUSED_RECT.y + FOCUSED_RECT.height)
            )

    def test_tree_traversal_skips_null_children_and_has_a_safety_limit(self) -> None:
        child = Mock()
        child.get_state_set.return_value.contains.return_value = True
        child_text = Mock()
        child.get_text_iface.return_value = child_text
        child_text.get_caret_offset.return_value = 1
        child_text.get_character_extents.return_value = CHILD_RECT
        root = Mock()
        root.get_state_set.return_value.contains.return_value = False
        root.get_text_iface.return_value = None
        root.get_child_count.return_value = LEARNING_PROMPT_CHILD_COUNT_WITH_NULL_SLOT
        root.get_child_at_index.side_effect = [None, child]
        with (
            patch.object(prompt_module, "_accessibility_bus_available", return_value=True),
            patch.object(Atspi, "get_desktop_count", return_value=1),
            patch.object(Atspi, "get_desktop", return_value=root),
        ):
            self.assertEqual(
                focused_caret_anchor(), ScreenAnchor(CHILD_RECT.x + CHILD_RECT.width, CHILD_RECT.y + CHILD_RECT.height)
            )

        loop = Mock()
        loop.get_state_set.return_value.contains.return_value = False
        loop.get_text_iface.return_value = None
        loop.get_child_count.return_value = 1
        loop.get_child_at_index.return_value = loop
        with (
            patch.object(prompt_module, "_accessibility_bus_available", return_value=True),
            patch.object(Atspi, "get_desktop_count", return_value=1),
            patch.object(Atspi, "get_desktop", return_value=loop),
        ):
            self.assertIsNone(focused_caret_anchor())


@unittest.skipUnless(DISPLAY_AVAILABLE, "GTK display is required")
class LearningPromptWindowTests(unittest.TestCase):
    application: ClassVar[Gtk.Application]

    @classmethod
    def setUpClass(cls) -> None:
        cls.application = Gtk.Application(
            application_id="io.github.olegius88.KeySwitchLearningPromptTests"
        )
        cls.application.register(None)

    def setUp(self) -> None:
        self.backend = FakePromptBackend()
        self.confirm = Mock(return_value=True)
        self.dismiss = Mock(return_value=True)
        self.window = LearningPromptWindow(
            self.application,
            self.backend,
            self.confirm,
            self.dismiss,
        )
        self.prompt = LearningPrompt(0, 1, "hello", "руддщ", "Editor")

    def tearDown(self) -> None:
        self.window.destroy()

    def test_build_show_anchor_fallback_and_hide(self) -> None:
        self.assertEqual(self.window.question.get_label(), PROMPT_TITLE)
        self.assertIn("Enter", self.window.hint.get_label())
        with (
            patch.object(
                prompt_module,
                "focused_caret_anchor",
                return_value=CARET_OVERRIDE_WITH_BACKEND_ANCHOR,
            ),
            patch.object(GLib, "idle_add") as idle,
            patch.object(self.window, "present") as present,
            patch.object(self.window, "grab_focus") as focus,
        ):
            self.window.show_prompt(self.prompt)
        self.assertEqual(
            self.window.anchor,
            ScreenAnchor(
                CARET_OVERRIDE_WITH_BACKEND_ANCHOR.x, CARET_OVERRIDE_WITH_BACKEND_ANCHOR.y, FIRST_SCREEN_ANCHOR.window
            ),
        )
        self.assertEqual(self.window.word.get_text(), "«hello» → «руддщ»: переводить")
        present.assert_called_once_with()
        focus.assert_called_once_with()
        idle.assert_called_once_with(self.window._position_above_anchor)

        self.backend.anchor = None
        with (
            patch.object(
                prompt_module,
                "focused_caret_anchor",
                return_value=CARET_ONLY_ANCHOR,
            ),
            patch.object(GLib, "idle_add"),
            patch.object(self.window, "present"),
            patch.object(self.window, "grab_focus"),
        ):
            self.window.show_prompt(self.prompt)
        self.assertEqual(self.window.anchor, ScreenAnchor(CARET_ONLY_ANCHOR.x, CARET_ONLY_ANCHOR.y, None))

        self.backend.anchor = FALLBACK_ONLY_ANCHOR
        with (
            patch.object(prompt_module, "focused_caret_anchor", return_value=None),
            patch.object(GLib, "idle_add"),
            patch.object(self.window, "present"),
            patch.object(self.window, "grab_focus"),
        ):
            self.window.show_prompt(self.prompt)
        self.assertEqual(self.window.anchor, FALLBACK_ONLY_ANCHOR)
        self.window.hide_prompt()
        self.assertIsNone(self.window.prompt)
        self.assertIsNone(self.window.anchor)
        self.assertEqual(self.backend.restored, [FALLBACK_ONLY_ANCHOR.window])
        self.window.hide_prompt()
        self.assertEqual(self.backend.restored, [FALLBACK_ONLY_ANCHOR.window])

    def test_positioning_handles_missing_and_x11_surfaces(self) -> None:
        self.window.anchor = None
        with patch.object(self.window, "get_surface", return_value=Mock()):
            self.assertFalse(self.window._position_above_anchor())
        self.window.anchor = FIRST_SCREEN_ANCHOR
        with patch.object(self.window, "get_surface", return_value=None):
            self.assertFalse(self.window._position_above_anchor())

        surface = Mock()
        with (
            patch.object(self.window, "get_surface", return_value=surface),
            patch.object(self.window, "get_width", return_value=LEARNING_PROMPT_GTK_WIDTH_PIXELS),
            patch.object(self.window, "get_height", return_value=LEARNING_PROMPT_FIXTURE_WINDOW_HEIGHT_PIXELS),
            patch.object(
                GdkX11.X11Surface,
                "get_xid",
                return_value=LEARNING_PROMPT_FIXTURE_WINDOW_ID,
            ),
        ):
            self.assertFalse(self.window._position_above_anchor())
        self.assertEqual(
            self.backend.positions,
            [(
                LEARNING_PROMPT_FIXTURE_WINDOW_ID,
                FIRST_SCREEN_ANCHOR.x - LEARNING_PROMPT_GTK_WIDTH_PIXELS // CENTERING_DIVISOR,
                FIRST_SCREEN_ANCHOR.y - LEARNING_PROMPT_FIXTURE_WINDOW_HEIGHT_PIXELS - LEARNING_PROMPT_ANCHOR_GAP_PIXELS,
            )],
        )

    def test_keyboard_and_close_paths(self) -> None:
        controller = Mock()
        state = Gdk.ModifierType(0)
        self.assertFalse(
            self.window._on_key_pressed(controller, Gdk.KEY_Return, RETURN_KEYCODE, state)
        )
        self.window.prompt = self.prompt
        self.assertTrue(
            self.window._on_key_pressed(controller, Gdk.KEY_Return, RETURN_KEYCODE, state)
        )
        self.confirm.assert_called_once_with(self.prompt)
        self.assertTrue(
            self.window._on_key_pressed(controller, Gdk.KEY_Escape, ESCAPE_KEYCODE, state)
        )
        self.assertFalse(
            self.window._on_key_pressed(controller, Gdk.KEY_a, A_KEYCODE, state)
        )
        self.assertEqual(self.dismiss.call_count, LEARNING_PROMPT_DISMISS_CALLS_AFTER_ESCAPE_AND_STRAY_KEY)

        self.window.prompt = None
        self.assertTrue(self.window._on_close_request(self.window))
        self.window.prompt = self.prompt
        self.assertTrue(self.window._on_close_request(self.window))
        self.assertEqual(self.dismiss.call_count, LEARNING_PROMPT_DISMISS_CALLS_AFTER_CLOSE_WITH_PROMPT)


@unittest.skipUnless(DISPLAY_AVAILABLE, "GTK display is required")
class RuleEditorWindowTests(unittest.TestCase):
    application: ClassVar[Gtk.Application]

    @classmethod
    def setUpClass(cls) -> None:
        cls.application = Gtk.Application(application_id="io.github.olegius88.KeySwitchRuleEditorTests")
        cls.application.register(None)

    def setUp(self) -> None:
        self.answers: list[str] = []
        self.drafts: list[RuleDraft] = []
        self.closed = Mock()
        self.focused = Mock()
        self.editing = LearnedRule("tot", 0, 1, "keep", "prefix", True)
        self.window = RuleEditorWindow(
            self.application, RuleDraft("tot", "prefix", True, "keep", "Telegram", self.editing),
            self.accept, self.closed, self.focused,
        )

    def accept(self, draft: RuleDraft) -> str:
        self.drafts.append(draft)
        return self.answers.pop(0) if self.answers else ""

    def test_the_form_starts_from_the_draft_and_reads_back_the_choices(self) -> None:
        self.assertEqual(self.window.current_draft(), RuleDraft("tot", "prefix", True, "keep", "Telegram", self.editing))
        self.assertEqual(self.window.hint.get_text(), "Набрано в раскладке EN; в RU это «еще»")
        self.window.matches["contains"].set_active(True)
        self.window.actions["convert"].set_active(True)
        self.window.case_sensitive.set_active(False)
        self.window.entry.set_text("еще")
        self.assertEqual(self.window.hint.get_text(), "Набрано в раскладке RU; в EN это «tot»")
        self.assertEqual(
            self.window.current_draft(), RuleDraft("еще", "contains", False, "convert", "Telegram", self.editing)
        )
        self.window.destroy()

    def test_ok_shows_what_to_fix_or_closes(self) -> None:
        self.answers.append("В сочетании нужна хотя бы одна буква")
        self.window.entry.emit("activate")
        self.assertEqual(self.window.problem.get_text(), "В сочетании нужна хотя бы одна буква")
        self.closed.assert_not_called()
        # Editing the letters clears the old message.
        self.window.entry.set_text("tota")
        self.assertEqual(self.window.problem.get_text(), "")
        self.window.confirm()
        self.closed.assert_called_once_with()
        self.focused.assert_called_with(False)
        self.assertEqual(len(self.drafts), len(("first", "second")))

    def test_escape_cancel_and_closing_the_window_do_not_store_anything(self) -> None:
        state = Gdk.ModifierType(0)
        self.assertFalse(self.window._on_key_pressed(Mock(), Gdk.KEY_a, A_KEYCODE, state))
        self.assertTrue(self.window._on_key_pressed(Mock(), Gdk.KEY_Escape, ESCAPE_KEYCODE, state))
        self.closed.assert_called_once_with()
        # Losing or gaining the keyboard is passed on.
        self.focused.reset_mock()
        second = RuleEditorWindow(self.application, RuleDraft(), self.accept, self.closed, self.focused)
        second.notify("is-active")
        self.focused.assert_called_once_with(second.is_active())
        self.assertTrue(second._on_close_request(second))
        third = RuleEditorWindow(self.application, RuleDraft(), self.accept, self.closed, self.focused)
        next(
            widget for widget in _descendants(third)
            if isinstance(widget, Gtk.Button) and widget.get_label() == "Отмена"
        ).emit("clicked")
        fourth = RuleEditorWindow(self.application, RuleDraft("tot"), self.accept, self.closed, self.focused)
        next(
            widget for widget in _descendants(fourth)
            if isinstance(widget, Gtk.Button) and widget.get_label() == "OK"
        ).emit("clicked")
        self.assertEqual(self.closed.call_count, len(("escape", "close", "cancel", "ok")))
        self.assertEqual([draft.pattern for draft in self.drafts], ["tot"])


def _descendants(widget: Gtk.Widget) -> list[Gtk.Widget]:
    found: list[Gtk.Widget] = []
    child = widget.get_first_child()
    while child is not None:
        found.append(child)
        found.extend(_descendants(child))
        child = child.get_next_sibling()
    return found


if __name__ == "__main__":
    unittest.main()
