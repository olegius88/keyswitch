"""Unit tests for pure Wayland desktop detection, IPC framing, and state parsers."""

from __future__ import annotations

import json
import os
import unittest
from unittest.mock import patch

from fixture_values.wayland import (
    WL_FIXTURE_BORDER_PIXELS,
    WL_FIXTURE_FLOAT_COORD,
    WL_FIXTURE_OUT_OF_RANGE_LAYOUT_INDEX,
    WL_FIXTURE_POINTER_X_PIXELS,
    WL_FIXTURE_POINTER_Y_PIXELS,
    WL_FIXTURE_POPUP_X_PIXELS,
    WL_FIXTURE_POPUP_Y_PIXELS,
    WL_FIXTURE_PROCESS_ID,
    WL_FIXTURE_SAMPLE_IPC_TYPE,
    WL_FIXTURE_SECOND_PROCESS_ID,
    WL_FIXTURE_SECOND_WINDOW_ID,
    WL_FIXTURE_SHORT_HEADER_BYTES,
    WL_FIXTURE_SMALL_CACHE_LIMIT,
    WL_FIXTURE_THIRD_WINDOW_ID,
    WL_FIXTURE_TITLE_BAR_PIXELS,
    WL_FIXTURE_WINDOW_HEIGHT_PIXELS,
    WL_FIXTURE_WINDOW_ID,
    WL_FIXTURE_WINDOW_WIDTH_PIXELS,
    WL_FIXTURE_WINDOW_X_PIXELS,
    WL_FIXTURE_WINDOW_Y_PIXELS,
)
from keyswitch.constants.geometry import CENTERING_DIVISOR
from keyswitch.constants.learning_prompt import LEARNING_PROMPT_WINDOW_TITLE
from keyswitch.constants.wayland import KDE_WINDOW_ID_START, SWAY_IPC_HEADER_BYTES
from keyswitch.wayland_desktop import (
    DesktopSnapshot,
    DesktopWindowInfo,
    KdeWindowIdMap,
    SWAY_IPC_HEADER,
    build_kwin_action_script,
    build_kwin_watcher_script,
    decode_sway_ipc_header,
    detect_desktop_kind,
    encode_sway_ipc,
    find_sway_window_by_pid,
    normalize_application_id,
    parse_gnome_state,
    parse_kde_layouts,
    parse_kwin_report,
    parse_sway_focused_window,
    parse_sway_inputs,
    sway_focus_command,
    sway_popup_move_command,
    sway_switch_layout_command,
    wayland_session_active,
)


class WaylandDesktopDetectionTests(unittest.TestCase):
    def test_window_center_and_application_id_normalization(self) -> None:
        empty = DesktopWindowInfo(WL_FIXTURE_WINDOW_ID, "app", WL_FIXTURE_PROCESS_ID)
        self.assertIsNone(empty.center())

        sized = DesktopWindowInfo(
            WL_FIXTURE_WINDOW_ID,
            "app",
            WL_FIXTURE_PROCESS_ID,
            x=WL_FIXTURE_WINDOW_X_PIXELS,
            y=WL_FIXTURE_WINDOW_Y_PIXELS,
            width=WL_FIXTURE_WINDOW_WIDTH_PIXELS,
            height=WL_FIXTURE_WINDOW_HEIGHT_PIXELS,
        )
        self.assertEqual(
            sized.center(),
            (
                WL_FIXTURE_WINDOW_X_PIXELS + WL_FIXTURE_WINDOW_WIDTH_PIXELS // CENTERING_DIVISOR,
                WL_FIXTURE_WINDOW_Y_PIXELS + WL_FIXTURE_WINDOW_HEIGHT_PIXELS // CENTERING_DIVISOR,
            ),
        )

        self.assertEqual(normalize_application_id("   "), "")
        self.assertEqual(normalize_application_id("window:12"), "")
        self.assertEqual(
            normalize_application_id("org.gnome.TextEditor.DESKTOP"),
            "org.gnome.TextEditor",
        )
        self.assertEqual(normalize_application_id("kitty"), "kitty")

    def test_wayland_session_active_honors_gdk_override_and_session_vars(self) -> None:
        self.assertFalse(wayland_session_active({"GDK_BACKEND": "x11", "WAYLAND_DISPLAY": "wayland-0"}))
        self.assertTrue(wayland_session_active({"GDK_BACKEND": "wayland"}))
        self.assertTrue(wayland_session_active({"WAYLAND_DISPLAY": "wayland-0"}))
        self.assertTrue(wayland_session_active({"XDG_SESSION_TYPE": "wayland"}))
        self.assertFalse(wayland_session_active({"XDG_SESSION_TYPE": "wayland", "DISPLAY": ":0"}))
        self.assertFalse(wayland_session_active({}))

        with patch.dict(os.environ, {"GDK_BACKEND": "wayland"}, clear=True):
            self.assertTrue(wayland_session_active())

    def test_detect_desktop_kind_distinguishes_sway_gnome_kde_and_generic(self) -> None:
        self.assertEqual(detect_desktop_kind({}, sway_socket="/run/sway.sock"), "sway")
        # A dead socket sway left in the user's environment is no sway session: the caller passes
        # only a live one, and a desktop's own bus service outranks it.
        self.assertEqual(detect_desktop_kind({"SWAYSOCK": "/run/sway.sock"}), "generic")
        self.assertEqual(detect_desktop_kind({}, has_gnome=True, sway_socket="/run/sway.sock"), "gnome")
        self.assertEqual(detect_desktop_kind({}, has_kde=True, sway_socket="/run/sway.sock"), "kde")
        self.assertEqual(detect_desktop_kind({}, has_gnome=True), "gnome")
        self.assertEqual(detect_desktop_kind({"XDG_CURRENT_DESKTOP": "ubuntu:GNOME"}), "gnome")
        self.assertEqual(detect_desktop_kind({}, has_kde=True), "kde")
        self.assertEqual(detect_desktop_kind({"DESKTOP_SESSION": "plasma"}), "kde")
        self.assertEqual(detect_desktop_kind({"XDG_CURRENT_DESKTOP": "sway"}), "sway")
        self.assertEqual(detect_desktop_kind({"XDG_CURRENT_DESKTOP": "wlroots"}), "generic")


class SwayProtocolTests(unittest.TestCase):
    def test_sway_ipc_framing_and_commands(self) -> None:
        frame = encode_sway_ipc(WL_FIXTURE_SAMPLE_IPC_TYPE, "hello")
        length, msg_type = decode_sway_ipc_header(frame[:SWAY_IPC_HEADER_BYTES])
        self.assertEqual((length, msg_type), (len(b"hello"), WL_FIXTURE_SAMPLE_IPC_TYPE))
        self.assertEqual(frame[SWAY_IPC_HEADER_BYTES:], b"hello")

        with self.assertRaisesRegex(ValueError, "header length"):
            decode_sway_ipc_header(b"x" * WL_FIXTURE_SHORT_HEADER_BYTES)
        bad_magic = SWAY_IPC_HEADER.pack(b"badipc", 0, 0)
        with self.assertRaisesRegex(ValueError, "magic"):
            decode_sway_ipc_header(bad_magic)

        self.assertEqual(
            sway_popup_move_command(
                WL_FIXTURE_WINDOW_ID,
                WL_FIXTURE_POPUP_X_PIXELS,
                WL_FIXTURE_POPUP_Y_PIXELS,
            ),
            f"[con_id={WL_FIXTURE_WINDOW_ID}] floating enable, border none, move position "
            f"{WL_FIXTURE_POPUP_X_PIXELS} {WL_FIXTURE_POPUP_Y_PIXELS}",
        )
        self.assertEqual(
            sway_focus_command(WL_FIXTURE_WINDOW_ID),
            f"[con_id={WL_FIXTURE_WINDOW_ID}] focus",
        )
        self.assertEqual(
            sway_switch_layout_command(1),
            "input type:keyboard xkb_switch_layout 1",
        )

    def test_parse_sway_inputs_prefers_multilayout_and_falls_back_safely(self) -> None:
        self.assertEqual(parse_sway_inputs("invalid"), ((), 0))
        payload: list[object] = [
            "skip",
            {"type": "pointer"},
            {"type": "keyboard", "xkb_layout_names": "bad"},
            {"type": "keyboard", "xkb_layout_names": ["", 0]},
            {
                "type": "keyboard",
                "xkb_layout_names": ["English (US)"],
                "xkb_active_layout_index": True,
            },
            {
                "type": "keyboard",
                "xkb_layout_names": ["Ignored single"],
                "xkb_active_layout_index": 0,
            },
        ]
        self.assertEqual(parse_sway_inputs(payload), (("English (US)",), 0))

        payload.append(
            {
                "type": "keyboard",
                "xkb_layout_names": ["English (US)", "Russian"],
                "xkb_active_layout_index": WL_FIXTURE_OUT_OF_RANGE_LAYOUT_INDEX,
            }
        )
        self.assertEqual(parse_sway_inputs(payload), (("English (US)", "Russian"), 0))

        valid_multi = [
            {
                "type": "keyboard",
                "xkb_layout_names": ["English (US)", "Russian"],
                "xkb_active_layout_index": 1,
            }
        ]
        self.assertEqual(parse_sway_inputs(valid_multi), (("English (US)", "Russian"), 1))

    def test_parse_sway_tree_focus_and_pid_lookup(self) -> None:
        self.assertIsNone(parse_sway_focused_window([]))
        self.assertIsNone(parse_sway_focused_window({"focused": True, "pid": WL_FIXTURE_PROCESS_ID, "id": 0}))

        tree_wayland = {
            "id": 1,
            "nodes": [
                "not-a-dict",
                {
                    "id": WL_FIXTURE_WINDOW_ID,
                    "pid": WL_FIXTURE_PROCESS_ID,
                    "focused": True,
                    "app_id": "foot.desktop",
                    "rect": {
                        "x": WL_FIXTURE_FLOAT_COORD,
                        "y": "bad",
                        "width": WL_FIXTURE_WINDOW_WIDTH_PIXELS,
                        "height": WL_FIXTURE_WINDOW_HEIGHT_PIXELS,
                    },
                    # The client sits inside its container: the window's own area is reported.
                    "window_rect": {
                        "x": WL_FIXTURE_BORDER_PIXELS,
                        "y": WL_FIXTURE_TITLE_BAR_PIXELS,
                        "width": WL_FIXTURE_POPUP_X_PIXELS,
                        "height": WL_FIXTURE_POPUP_Y_PIXELS,
                    },
                },
            ],
            "floating_nodes": "not-a-list",
        }
        focused = parse_sway_focused_window(tree_wayland)
        self.assertEqual(
            focused,
            DesktopWindowInfo(
                window_id=WL_FIXTURE_WINDOW_ID,
                application="foot",
                pid=WL_FIXTURE_PROCESS_ID,
                x=int(WL_FIXTURE_FLOAT_COORD) + WL_FIXTURE_BORDER_PIXELS,
                y=WL_FIXTURE_TITLE_BAR_PIXELS,
                width=WL_FIXTURE_POPUP_X_PIXELS,
                height=WL_FIXTURE_POPUP_Y_PIXELS,
            ),
        )

        tree_xwayland = {
            "id": 1,
            "floating_nodes": [
                {
                    "id": WL_FIXTURE_SECOND_WINDOW_ID,
                    "pid": WL_FIXTURE_SECOND_PROCESS_ID,
                    "focused": True,
                    "app_id": 0,
                    "window_properties": {"instance": "xterm"},
                    "rect": None,
                }
            ],
        }
        self.assertEqual(
            parse_sway_focused_window(tree_xwayland),
            DesktopWindowInfo(
                window_id=WL_FIXTURE_SECOND_WINDOW_ID,
                application="xterm",
                pid=WL_FIXTURE_SECOND_PROCESS_ID,
            ),
        )

        tree_no_props = {
            "id": WL_FIXTURE_THIRD_WINDOW_ID,
            "pid": WL_FIXTURE_PROCESS_ID,
            "focused": True,
            "window_properties": "none",
        }
        self.assertEqual(
            parse_sway_focused_window(tree_no_props),
            DesktopWindowInfo(
                window_id=WL_FIXTURE_THIRD_WINDOW_ID,
                application="",
                pid=WL_FIXTURE_PROCESS_ID,
            ),
        )
        self.assertIsNone(parse_sway_focused_window({"id": 1, "nodes": []}))

        self.assertIsNone(find_sway_window_by_pid([], WL_FIXTURE_PROCESS_ID))
        self.assertIsNone(find_sway_window_by_pid({}, 0))
        lookup_tree = {
            "id": 1,
            "nodes": [
                "skip",
                {"id": 0, "pid": WL_FIXTURE_PROCESS_ID, "name": "Ignored"},
                {"id": WL_FIXTURE_WINDOW_ID, "pid": WL_FIXTURE_PROCESS_ID, "name": "Main"},
                {"id": WL_FIXTURE_SECOND_WINDOW_ID, "pid": WL_FIXTURE_PROCESS_ID, "name": "Other"},
                {"id": WL_FIXTURE_THIRD_WINDOW_ID, "pid": WL_FIXTURE_PROCESS_ID, "name": "Popup"},
            ],
            "floating_nodes": None,
        }
        self.assertEqual(
            find_sway_window_by_pid(lookup_tree, WL_FIXTURE_PROCESS_ID, title="Popup"),
            WL_FIXTURE_THIRD_WINDOW_ID,
        )
        self.assertEqual(
            find_sway_window_by_pid(lookup_tree, WL_FIXTURE_PROCESS_ID, title="Missing"),
            WL_FIXTURE_WINDOW_ID,
        )
        self.assertEqual(
            find_sway_window_by_pid(lookup_tree, WL_FIXTURE_PROCESS_ID),
            WL_FIXTURE_WINDOW_ID,
        )


class GnomeAndKdeParserTests(unittest.TestCase):
    def test_parse_gnome_state_handles_full_and_partial_payloads(self) -> None:
        with self.assertRaisesRegex(ValueError, "JSON object"):
            parse_gnome_state("[]")

        full = json.dumps({
            "layouts": ["us", "", "ru"],
            "current_layout": WL_FIXTURE_OUT_OF_RANGE_LAYOUT_INDEX,
            "window": {
                "id": WL_FIXTURE_WINDOW_ID,
                "wm_class": "window:1",
                "app_id": "org.gnome.Console.desktop",
                "pid": WL_FIXTURE_PROCESS_ID,
                "x": WL_FIXTURE_WINDOW_X_PIXELS,
                "y": WL_FIXTURE_WINDOW_Y_PIXELS,
                "width": WL_FIXTURE_WINDOW_WIDTH_PIXELS,
                "height": WL_FIXTURE_WINDOW_HEIGHT_PIXELS,
            },
            "pointer": {"x": WL_FIXTURE_POINTER_X_PIXELS, "y": WL_FIXTURE_POINTER_Y_PIXELS},
            "locked": True,
            "per_window_layout": True,
        })
        self.assertEqual(
            parse_gnome_state(full),
            DesktopSnapshot(
                layouts=("us", "ru"),
                current_layout=-1,
                window=DesktopWindowInfo(
                    window_id=WL_FIXTURE_WINDOW_ID,
                    application="org.gnome.Console",
                    pid=WL_FIXTURE_PROCESS_ID,
                    x=WL_FIXTURE_WINDOW_X_PIXELS,
                    y=WL_FIXTURE_WINDOW_Y_PIXELS,
                    width=WL_FIXTURE_WINDOW_WIDTH_PIXELS,
                    height=WL_FIXTURE_WINDOW_HEIGHT_PIXELS,
                ),
                pointer=(WL_FIXTURE_POINTER_X_PIXELS, WL_FIXTURE_POINTER_Y_PIXELS),
                locked=True,
                isolated_layout=True,
            ),
        )

        minimal = json.dumps({
            "layouts": "bad",
            "window": {"id": 0},
            "pointer": {"x": WL_FIXTURE_POINTER_X_PIXELS},
        })
        self.assertEqual(parse_gnome_state(minimal), DesktopSnapshot(layouts=(), current_layout=-1))

        no_objects = json.dumps({"layouts": ["us", "ru"], "current_layout": 1})
        self.assertEqual(
            parse_gnome_state(no_objects),
            DesktopSnapshot(layouts=("us", "ru"), current_layout=1),
        )
        # GNOME Shell's own password prompt has the keyboard: the window says so.
        prompt = json.dumps({"window": {"id": WL_FIXTURE_WINDOW_ID, "app_id": "org.gnome.Shell", "password": True}})
        window = parse_gnome_state(prompt).window
        self.assertTrue(window is not None and window.password)
        # The source is chosen but mutter has not applied its group yet: no layout to report.
        applying = json.dumps({"layouts": ["us", "ru"], "current_layout": -1})
        self.assertEqual(
            parse_gnome_state(applying),
            DesktopSnapshot(layouts=("us", "ru"), current_layout=-1),
        )

    def test_kde_window_id_map_layouts_report_and_script_builders(self) -> None:
        id_map = KdeWindowIdMap(limit=WL_FIXTURE_SMALL_CACHE_LIMIT)
        self.assertEqual(id_map.intern("   "), 0)
        first = id_map.intern("uuid-a")
        second = id_map.intern("uuid-b")
        self.assertEqual(first, KDE_WINDOW_ID_START)
        self.assertEqual(second, KDE_WINDOW_ID_START + 1)
        self.assertEqual(id_map.intern("uuid-a"), first)
        third = id_map.intern("uuid-c")
        self.assertEqual(id_map.resolve(first), "uuid-a")
        self.assertEqual(id_map.resolve(second), "")
        self.assertEqual(id_map.resolve(third), "uuid-c")

        self.assertEqual(
            parse_kde_layouts((("us", "", "English"), ("", "winkeys", "Blank"), ("ru", "phonetic", "Russian"))),
            ("us", "ru+phonetic"),
        )

        with self.assertRaisesRegex(ValueError, "JSON object"):
            parse_kwin_report("[]", id_map)

        win, ptr = parse_kwin_report(
            json.dumps({
                "window": {
                    "id": "uuid-kwrite",
                    "resourceClass": "",
                    "desktopFileName": "org.kde.kwrite.desktop",
                    "resourceName": "kwrite",
                    "pid": WL_FIXTURE_PROCESS_ID,
                    "x": WL_FIXTURE_WINDOW_X_PIXELS,
                    "y": WL_FIXTURE_WINDOW_Y_PIXELS,
                    "width": WL_FIXTURE_WINDOW_WIDTH_PIXELS,
                    "height": WL_FIXTURE_WINDOW_HEIGHT_PIXELS,
                },
                "cursor": {"x": WL_FIXTURE_POINTER_X_PIXELS, "y": WL_FIXTURE_POINTER_Y_PIXELS},
            }),
            id_map,
        )
        assert win is not None
        self.assertEqual(win.application, "org.kde.kwrite")
        self.assertEqual(ptr, (WL_FIXTURE_POINTER_X_PIXELS, WL_FIXTURE_POINTER_Y_PIXELS))

        self.assertEqual(
            parse_kwin_report(json.dumps({"window": {"id": ""}, "cursor": {"x": 0}}), id_map),
            (None, None),
        )
        self.assertEqual(parse_kwin_report("{}", id_map), (None, None))

        watcher = build_kwin_watcher_script(":1.42", "/Watcher", "io.test.Watcher")
        self.assertIn("workspace.windowActivated.connect(ksPack);", watcher)
        self.assertIn('":1.42"', watcher)

        empty_action = build_kwin_action_script()
        self.assertEqual(empty_action.strip(), "var list = workspace.windowList();")

        full_action = build_kwin_action_script(
            move_pid=WL_FIXTURE_PROCESS_ID,
            move_title=LEARNING_PROMPT_WINDOW_TITLE,
            x=WL_FIXTURE_POPUP_X_PIXELS,
            y=WL_FIXTURE_POPUP_Y_PIXELS,
            activate_uuid="uuid-kwrite",
            report_target=(":1.42", "/Watcher", "io.test.Watcher"),
        )
        self.assertIn("popup.keepAbove = true;", full_action)
        self.assertIn('"uuid-kwrite"', full_action)
        self.assertIn("callDBus", full_action)


if __name__ == "__main__":
    unittest.main()
