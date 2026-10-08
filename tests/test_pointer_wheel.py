"""A scroll moves the view, not the caret: the wheel is no click.

Every backend reported the wheel as a pointer event, and the engine discards the word being typed
and every word before it on one. The Windows log of 08.10.2026 counts 141 such events a minute while
a page was scrolled; a word begun before a scroll was dropped, and the next one was judged with no
words before it.
"""

from __future__ import annotations

import unittest

from keyswitch.constants.macos import EVENT_LEFT_MOUSE_DOWN, EVENT_SCROLL_WHEEL, POINTER_EVENTS
from keyswitch.constants.windows import POINTER_INVALIDATING_MESSAGES, WHEEL_MESSAGES, WM_LBUTTONDOWN
from keyswitch.constants.x11 import X11_BUTTON_PRESS, X11_WHEEL_BUTTONS
from fixture_values.keys import X11_PRIMARY_BUTTON
from test_x11_backend import backend_with, payload


class PointerWheelTests(unittest.TestCase):
    def test_x11_reports_a_click_and_no_scroll(self) -> None:
        backend, _libraries = backend_with()
        click = backend._decode_event(payload(X11_BUTTON_PRESS, keycode=X11_PRIMARY_BUTTON))
        assert click is not None
        self.assertEqual(click.key_name, "Pointer")
        for button in sorted(X11_WHEEL_BUTTONS):
            with self.subTest(button=button):
                self.assertIsNone(backend._decode_event(payload(X11_BUTTON_PRESS, keycode=button)))

    def test_windows_and_macos_watch_the_buttons_and_not_the_wheel(self) -> None:
        self.assertIn(WM_LBUTTONDOWN, POINTER_INVALIDATING_MESSAGES)
        self.assertFalse(POINTER_INVALIDATING_MESSAGES & WHEEL_MESSAGES)
        self.assertIn(EVENT_LEFT_MOUSE_DOWN, POINTER_EVENTS)
        self.assertNotIn(EVENT_SCROLL_WHEEL, POINTER_EVENTS)


if __name__ == "__main__":
    unittest.main()
