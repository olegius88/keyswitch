"""Pure protocol framing and state parsers for Wayland desktop compositors.

Every function here is free of live sockets and D-Bus connections so GNOME Shell,
KDE Plasma 6 (KWin) and sway wire formats are verified deterministically.
"""

from __future__ import annotations

from collections import OrderedDict
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
import json
import os
import struct
from typing import cast

from .constants.geometry import CENTERING_DIVISOR
from .constants.wayland import (
    KDE_WINDOW_ID_CACHE_LIMIT,
    KDE_WINDOW_ID_START,
    SWAY_IPC_HEADER_BYTES,
)

SWAY_IPC_MAGIC = b"i3-ipc"
SWAY_IPC_HEADER = struct.Struct("=6sII")

GNOME_EXTENSION_UUID = "keyswitch@io.github.olegius88.KeySwitch"
GNOME_DBUS_NAME = "org.gnome.Shell"
GNOME_DBUS_PATH = "/io/github/olegius88/KeySwitch/Gnome"
GNOME_DBUS_INTERFACE = "io.github.olegius88.KeySwitch.Gnome"
GNOME_EXTENSIONS_BUS_NAME = "org.gnome.Shell.Extensions"
GNOME_EXTENSIONS_PATH = "/org/gnome/Shell/Extensions"
GNOME_EXTENSIONS_INTERFACE = "org.gnome.Shell.Extensions"

KDE_KEYBOARD_BUS_NAME = "org.kde.keyboard"
KDE_KEYBOARD_PATH = "/Layouts"
KDE_KEYBOARD_INTERFACE = "org.kde.KeyboardLayouts"

KWIN_BUS_NAME = "org.kde.KWin"
KWIN_SCRIPTING_PATH = "/Scripting"
KWIN_SCRIPTING_INTERFACE = "org.kde.kwin.Scripting"
KWIN_SCRIPT_INTERFACE = "org.kde.kwin.Script"


@dataclass(frozen=True)
class DesktopWindowInfo:
    """Focused top-level window metadata reported by a Wayland compositor."""

    window_id: int
    application: str
    pid: int
    x: int = 0
    y: int = 0
    width: int = 0
    height: int = 0
    # The keyboard is in a field that hides what is typed, which the compositor knows and AT-SPI
    # cannot tell (a password prompt of GNOME Shell).
    password: bool = False

    def center(self) -> tuple[int, int] | None:
        if self.width <= 0 or self.height <= 0:
            return None
        return (
            self.x + self.width // CENTERING_DIVISOR,
            self.y + self.height // CENTERING_DIVISOR,
        )


@dataclass(frozen=True)
class DesktopSnapshot:
    """Snapshot of layouts, active window, pointer and lock state."""

    layouts: tuple[str, ...]
    current_layout: int
    window: DesktopWindowInfo | None = None
    pointer: tuple[int, int] | None = None
    locked: bool = False
    isolated_layout: bool = False


def wayland_session_active(environ: Mapping[str, str] | None = None) -> bool:
    """Whether the current Linux session runs on Wayland rather than X11."""

    env = os.environ if environ is None else environ
    gdk = env.get("GDK_BACKEND", "").strip().casefold()
    if gdk == "x11":
        return False
    if gdk == "wayland":
        return True
    if env.get("WAYLAND_DISPLAY", "").strip():
        return True
    return (
        env.get("XDG_SESSION_TYPE", "").strip().casefold() == "wayland"
        and not env.get("DISPLAY", "").strip()
    )


def detect_desktop_kind(
    environ: Mapping[str, str],
    *,
    has_gnome: bool = False,
    has_kde: bool = False,
    sway_socket: str = "",
) -> str:
    """Identify the running Wayland compositor family.

    A desktop's own service on the session bus outranks SWAYSOCK: sway exports it into the user's
    systemd environment, and a GNOME or Plasma session started after a sway one inherits the dead
    socket (seen in a VM, 27.10.2026). ``sway_socket`` is a socket that exists.
    """

    desktop = (
        f"{environ.get('XDG_CURRENT_DESKTOP', '')}:{environ.get('DESKTOP_SESSION', '')}"
        .casefold()
    )
    if has_gnome:
        return "gnome"
    if has_kde:
        return "kde"
    if sway_socket:
        return "sway"
    if "gnome" in desktop or "ubuntu" in desktop:
        return "gnome"
    if "kde" in desktop or "plasma" in desktop:
        return "kde"
    if "sway" in desktop:
        return "sway"
    return "generic"


def normalize_application_id(raw: str) -> str:
    """Clean desktop identifiers (strip ``.desktop`` and Mutter's untracked ``window:N``)."""

    cleaned = raw.strip()
    if not cleaned or cleaned.startswith("window:"):
        return ""
    if cleaned.casefold().endswith(".desktop"):
        cleaned = cleaned[: -len(".desktop")]
    return cleaned


def _int_field(mapping: Mapping[str, object], key: str, default: int = 0) -> int:
    value = mapping.get(key, default)
    if isinstance(value, bool):
        return default
    if isinstance(value, (int, float)):
        return int(value)
    return default


def _str_field(mapping: Mapping[str, object], key: str) -> str:
    value = mapping.get(key, "")
    return value if isinstance(value, str) else ""


def encode_sway_ipc(message_type: int, payload: str = "") -> bytes:
    """Pack one ``i3-ipc`` request frame."""

    body = payload.encode("utf-8")
    return SWAY_IPC_HEADER.pack(SWAY_IPC_MAGIC, len(body), message_type) + body


def decode_sway_ipc_header(header: bytes) -> tuple[int, int]:
    """Unpack ``(payload_length, message_type)`` from a 14-byte ``i3-ipc`` header."""

    if len(header) != SWAY_IPC_HEADER_BYTES:
        raise ValueError("Invalid sway IPC header length")
    magic, length, message_type = SWAY_IPC_HEADER.unpack(header)
    if magic != SWAY_IPC_MAGIC:
        raise ValueError("Invalid sway IPC magic")
    return int(length), int(message_type)


def parse_sway_inputs(payload: object) -> tuple[tuple[str, ...], int]:
    """Extract keyboard layout names and active index from sway ``GET_INPUTS``."""

    if not isinstance(payload, list):
        return (), 0
    fallback: tuple[tuple[str, ...], int] = ((), 0)
    for item in payload:
        if not isinstance(item, dict) or item.get("type") != "keyboard":
            continue
        entry = cast(Mapping[str, object], item)
        raw_names = entry.get("xkb_layout_names")
        if not isinstance(raw_names, list):
            continue
        names = tuple(name for name in raw_names if isinstance(name, str) and name)
        if not names:
            continue
        active = _int_field(entry, "xkb_active_layout_index", 0)
        if not 0 <= active < len(names):
            active = 0
        if len(names) > 1:
            return names, active
        if not fallback[0]:
            fallback = (names, active)
    return fallback


def parse_sway_focused_window(tree: object) -> DesktopWindowInfo | None:
    """Find the focused container in sway ``GET_TREE``."""

    if not isinstance(tree, dict):
        return None
    stack: list[Mapping[str, object]] = [cast(Mapping[str, object], tree)]
    while stack:
        node = stack.pop()
        pid = _int_field(node, "pid", 0)
        focused = node.get("focused") is True
        if focused and pid > 0:
            con_id = _int_field(node, "id", 0)
            if con_id <= 0:
                return None
            app_id = normalize_application_id(_str_field(node, "app_id"))
            if not app_id:
                props = node.get("window_properties")
                if isinstance(props, dict):
                    typed_props = cast(Mapping[str, object], props)
                    app_id = normalize_application_id(
                        _str_field(typed_props, "class")
                        or _str_field(typed_props, "instance")
                    )
            # The client area: `rect` is the container with its borders and title bar, and
            # `window_rect` is where the client sits inside it.
            rect_obj = node.get("rect")
            rect = cast(Mapping[str, object], rect_obj) if isinstance(rect_obj, dict) else {}
            inner_obj = node.get("window_rect")
            inner = cast(Mapping[str, object], inner_obj) if isinstance(inner_obj, dict) else {}
            return DesktopWindowInfo(
                window_id=con_id,
                application=app_id,
                pid=pid,
                x=_int_field(rect, "x") + _int_field(inner, "x"),
                y=_int_field(rect, "y") + _int_field(inner, "y"),
                width=_int_field(inner, "width") or _int_field(rect, "width"),
                height=_int_field(inner, "height") or _int_field(rect, "height"),
            )
        for key in ("floating_nodes", "nodes"):
            children = node.get(key)
            if isinstance(children, list):
                for child in reversed(children):
                    if isinstance(child, dict):
                        stack.append(cast(Mapping[str, object], child))
    return None


def find_sway_window_by_pid(
    tree: object, pid: int, *, title: str = ""
) -> int | None:
    """Locate a sway container ``con_id`` owned by ``pid`` (and optional ``title``)."""

    if not isinstance(tree, dict) or pid <= 0:
        return None
    stack: list[Mapping[str, object]] = [cast(Mapping[str, object], tree)]
    fallback: int | None = None
    while stack:
        node = stack.pop()
        if _int_field(node, "pid", 0) == pid:
            con_id = _int_field(node, "id", 0)
            if con_id > 0:
                if not title or _str_field(node, "name") == title:
                    return con_id
                if fallback is None:
                    fallback = con_id
        for key in ("floating_nodes", "nodes"):
            children = node.get(key)
            if isinstance(children, list):
                for child in reversed(children):
                    if isinstance(child, dict):
                        stack.append(cast(Mapping[str, object], child))
    return fallback


def sway_popup_move_command(con_id: int, x: int, y: int) -> str:
    return f"[con_id={con_id}] floating enable, border none, move position {x} {y}"


def sway_focus_command(con_id: int) -> str:
    return f"[con_id={con_id}] focus"


def sway_switch_layout_command(index: int) -> str:
    return f"input type:keyboard xkb_switch_layout {index}"


def parse_gnome_state(raw_json: str) -> DesktopSnapshot:
    """Decode the JSON state snapshot returned by the GNOME Shell extension."""

    data = json.loads(raw_json)
    if not isinstance(data, dict):
        raise ValueError("GNOME state payload must be a JSON object")
    entry = cast(Mapping[str, object], data)
    raw_layouts = entry.get("layouts")
    layouts: tuple[str, ...] = ()
    if isinstance(raw_layouts, list):
        layouts = tuple(item for item in raw_layouts if isinstance(item, str) and item)
    # -1: the extension has not seen the current source applied yet; the layout is unknown till then.
    current = _int_field(entry, "current_layout", -1)
    if not 0 <= current < len(layouts):
        current = -1
    window: DesktopWindowInfo | None = None
    win_obj = entry.get("window")
    if isinstance(win_obj, dict):
        win_map = cast(Mapping[str, object], win_obj)
        win_id = _int_field(win_map, "id", 0)
        if win_id > 0:
            app_name = (
                normalize_application_id(_str_field(win_map, "wm_class"))
                or normalize_application_id(_str_field(win_map, "app_id"))
                or normalize_application_id(_str_field(win_map, "wm_class_instance"))
            )
            window = DesktopWindowInfo(
                window_id=win_id,
                application=app_name,
                pid=_int_field(win_map, "pid", 0),
                x=_int_field(win_map, "x"),
                y=_int_field(win_map, "y"),
                width=_int_field(win_map, "width"),
                height=_int_field(win_map, "height"),
                password=win_map.get("password") is True,
            )
    pointer: tuple[int, int] | None = None
    ptr_obj = entry.get("pointer")
    if isinstance(ptr_obj, dict):
        ptr_map = cast(Mapping[str, object], ptr_obj)
        if "x" in ptr_map and "y" in ptr_map:
            pointer = (_int_field(ptr_map, "x"), _int_field(ptr_map, "y"))
    return DesktopSnapshot(
        layouts=layouts,
        current_layout=current,
        window=window,
        pointer=pointer,
        locked=entry.get("locked") is True,
        isolated_layout=entry.get("per_window_layout") is True,
    )


class KdeWindowIdMap:
    """Bounded bidirectional mapping between KWin ``internalId`` UUIDs and ``int`` handles."""

    def __init__(self, limit: int = KDE_WINDOW_ID_CACHE_LIMIT) -> None:
        self._limit = limit
        self._next_id = KDE_WINDOW_ID_START
        self._by_uuid: OrderedDict[str, int] = OrderedDict()
        self._by_handle: dict[int, str] = {}

    def intern(self, uuid_text: str) -> int:
        key = uuid_text.strip()
        if not key:
            return 0
        existing = self._by_uuid.get(key)
        if existing is not None:
            self._by_uuid.move_to_end(key)
            return existing
        while len(self._by_uuid) >= self._limit:
            oldest_uuid, oldest_handle = self._by_uuid.popitem(last=False)
            self._by_handle.pop(oldest_handle, None)
        handle = self._next_id
        self._next_id += 1
        self._by_uuid[key] = handle
        self._by_handle[handle] = key
        return handle

    def resolve(self, handle: int) -> str:
        return self._by_handle.get(handle, "")


def parse_kde_layouts(items: Sequence[tuple[str, str, str]]) -> tuple[str, ...]:
    """Convert ``org.kde.KeyboardLayouts.getLayoutsList`` ``(layout, variant, label)`` tuples."""

    result: list[str] = []
    for layout, variant, _label in items:
        code = layout.strip()
        if not code:
            continue
        sub = variant.strip()
        result.append(f"{code}+{sub}" if sub else code)
    return tuple(result)


def parse_kwin_report(
    raw_json: str, id_map: KdeWindowIdMap
) -> tuple[DesktopWindowInfo | None, tuple[int, int] | None]:
    """Decode a focus/pointer JSON report pushed by the KWin 6 helper script."""

    data = json.loads(raw_json)
    if not isinstance(data, dict):
        raise ValueError("KWin report must be a JSON object")
    entry = cast(Mapping[str, object], data)
    window: DesktopWindowInfo | None = None
    win_obj = entry.get("window")
    if isinstance(win_obj, dict):
        win_map = cast(Mapping[str, object], win_obj)
        handle = id_map.intern(_str_field(win_map, "id"))
        if handle > 0:
            app_name = (
                normalize_application_id(_str_field(win_map, "resourceClass"))
                or normalize_application_id(_str_field(win_map, "desktopFileName"))
                or normalize_application_id(_str_field(win_map, "resourceName"))
            )
            window = DesktopWindowInfo(
                window_id=handle,
                application=app_name,
                pid=_int_field(win_map, "pid", 0),
                x=_int_field(win_map, "x"),
                y=_int_field(win_map, "y"),
                width=_int_field(win_map, "width"),
                height=_int_field(win_map, "height"),
            )
    pointer: tuple[int, int] | None = None
    ptr_obj = entry.get("cursor")
    if isinstance(ptr_obj, dict):
        ptr_map = cast(Mapping[str, object], ptr_obj)
        if "x" in ptr_map and "y" in ptr_map:
            pointer = (_int_field(ptr_map, "x"), _int_field(ptr_map, "y"))
    return window, pointer


def build_kwin_watcher_script(
    bus_name: str, object_path: str, interface: str
) -> str:
    """Generate the KWin 6 JavaScript listener that pushes active-window updates."""

    dest = json.dumps(bus_name)
    path = json.dumps(object_path)
    iface = json.dumps(interface)
    return (
        "function ksPack(w) {\n"
        "    var win = null;\n"
        "    if (w) {\n"
        "        var g = w.clientGeometry || w.frameGeometry;\n"
        "        win = {\n"
        "            id: String(w.internalId || ''),\n"
        "            resourceClass: String(w.resourceClass || ''),\n"
        "            resourceName: String(w.resourceName || ''),\n"
        "            desktopFileName: String(w.desktopFileName || ''),\n"
        "            pid: Number(w.pid || 0),\n"
        "            x: Math.round(g.x),\n"
        "            y: Math.round(g.y),\n"
        "            width: Math.round(g.width),\n"
        "            height: Math.round(g.height)\n"
        "        };\n"
        "    }\n"
        "    var payload = JSON.stringify({\n"
        "        window: win,\n"
        "        cursor: {x: Math.round(workspace.cursorPos.x), y: Math.round(workspace.cursorPos.y)}\n"
        "    });\n"
        f"    callDBus({dest}, {path}, {iface}, 'Report', payload);\n"
        "}\n"
        "workspace.windowActivated.connect(ksPack);\n"
        "ksPack(workspace.activeWindow);\n"
    )


def build_kwin_action_script(
    *,
    move_pid: int = 0,
    move_title: str = "",
    x: int = 0,
    y: int = 0,
    activate_uuid: str = "",
    report_target: tuple[str, str, str] | None = None,
) -> str:
    """Generate a one-shot KWin 6 script to position a popup, restore focus, or query cursor."""

    lines = ["var list = workspace.windowList();"]
    if move_pid > 0:
        title_json = json.dumps(move_title)
        lines.extend((
            f"var targetTitle = {title_json};",
            "var fallbackPopup = null;",
            "var chosenPopup = null;",
            "for (var i = 0; i < list.length; i++) {",
            "    var cand = list[i];",
            f"    if (Number(cand.pid || 0) === {move_pid}) {{",
            "        if (targetTitle && String(cand.caption || '') === targetTitle) {",
            "            chosenPopup = cand;",
            "            break;",
            "        }",
            "        if (!fallbackPopup) fallbackPopup = cand;",
            "    }",
            "}",
            "var popup = chosenPopup || fallbackPopup;",
            "if (popup) {",
            "    popup.keepAbove = true;",
            "    popup.frameGeometry = {",
            f"        x: {x},",
            f"        y: {y},",
            "        width: popup.frameGeometry.width,",
            "        height: popup.frameGeometry.height",
            "    };",
            "}",
        ))
    if activate_uuid:
        uuid_json = json.dumps(activate_uuid)
        lines.extend((
            "for (var j = 0; j < list.length; j++) {",
            f"    if (String(list[j].internalId || '') === {uuid_json}) {{",
            "        workspace.activeWindow = list[j];",
            "        break;",
            "    }",
            "}",
        ))
    if report_target is not None:
        dest, path, iface = (json.dumps(part) for part in report_target)
        lines.extend((
            "var active = workspace.activeWindow;",
            "var g = active ? (active.clientGeometry || active.frameGeometry) : null;",
            "var win = active ? {",
            "    id: String(active.internalId || ''),",
            "    resourceClass: String(active.resourceClass || ''),",
            "    resourceName: String(active.resourceName || ''),",
            "    desktopFileName: String(active.desktopFileName || ''),",
            "    pid: Number(active.pid || 0),",
            "    x: Math.round(g.x),",
            "    y: Math.round(g.y),",
            "    width: Math.round(g.width),",
            "    height: Math.round(g.height)",
            "} : null;",
            "var out = JSON.stringify({",
            "    window: win,",
            "    cursor: {x: Math.round(workspace.cursorPos.x), y: Math.round(workspace.cursorPos.y)}",
            "});",
            f"callDBus({dest}, {path}, {iface}, 'Report', out);",
        ))
    return "\n".join(lines) + "\n"
