"""StatusNotifierItem and DBusMenu integration for desktop panels."""

from __future__ import annotations

import os
from pathlib import Path
from collections.abc import Callable, Iterable, Mapping

import dbus
import dbus.service
from dbus.mainloop.glib import DBusGMainLoop
from gi.repository import GLib

from .indicator import (
    alternate_layout_action_label,
    alternate_layout_group,
    layout_icon_name,
    layout_label,
    normalize_indicator_style,
)


ITEM_INTERFACE = "org.kde.StatusNotifierItem"
MENU_INTERFACE = "com.canonical.dbusmenu"
PROPERTIES_INTERFACE = "org.freedesktop.DBus.Properties"
OBJECT_PATH = "/StatusNotifierItem"
MENU_PATH = "/MenuBar"

MENU_LAYOUT = 1
MENU_SETTINGS = 2
MENU_SEPARATOR_PRIMARY = 3
MENU_AUTOSWITCH = 4
MENU_SOUND = 5
MENU_NOTIFICATIONS = 6
MENU_SEPARATOR_TOOLS = 7
MENU_HISTORY = 8
MENU_EXCEPTIONS = 9
MENU_ABOUT = 10
MENU_SEPARATOR_QUIT = 11
MENU_QUIT = 12
MENU_SWITCH_LAYOUT = 13

MENU_ITEM_IDS = (
    MENU_LAYOUT,
    MENU_SWITCH_LAYOUT,
    MENU_SETTINGS,
    MENU_SEPARATOR_PRIMARY,
    MENU_AUTOSWITCH,
    MENU_SOUND,
    MENU_NOTIFICATIONS,
    MENU_SEPARATOR_TOOLS,
    MENU_HISTORY,
    MENU_EXCEPTIONS,
    MENU_ABOUT,
    MENU_SEPARATOR_QUIT,
    MENU_QUIT,
)

TrayAction = Callable[[], bool | None]
MenuEvent = tuple[int, str, object, int]


class StatusNotifierMenu(dbus.service.Object):
    """Small dynamic menu exported with the canonical DBusMenu protocol."""

    def __init__(
        self,
        bus_name: dbus.service.BusName,
        actions: Mapping[int, TrayAction],
        icon_theme_path: Path,
    ) -> None:
        self._actions = dict(actions)
        self._icon_theme_path = str(icon_theme_path)
        self._revision = 1
        self._enabled = True
        self._sound_enabled = False
        self._notifications_enabled = True
        self._group = -1
        self._layout_icon = "keyswitch"
        super().__init__(bus_name, MENU_PATH)

    def _menu_properties(self) -> dict[str, object]:
        return {
            "Version": dbus.UInt32(3),
            "TextDirection": dbus.String("ltr"),
            "Status": dbus.String("normal"),
            "IconThemePath": dbus.Array([self._icon_theme_path], signature="s"),
        }

    @dbus.service.method(PROPERTIES_INTERFACE, in_signature="ss", out_signature="v")
    def Get(self, interface: str, property_name: str) -> object:
        if interface != MENU_INTERFACE:
            raise dbus.exceptions.DBusException(
                "org.freedesktop.DBus.Error.UnknownInterface"
            )
        properties = self._menu_properties()
        if property_name not in properties:
            raise dbus.exceptions.DBusException(
                "org.freedesktop.DBus.Error.UnknownProperty"
            )
        return properties[property_name]

    @dbus.service.method(PROPERTIES_INTERFACE, in_signature="s", out_signature="a{sv}")
    def GetAll(self, interface: str) -> dict[str, object]:
        return self._menu_properties() if interface == MENU_INTERFACE else {}

    @dbus.service.method(PROPERTIES_INTERFACE, in_signature="ssv", out_signature="")
    def Set(self, _interface: str, _property_name: str, _value: object) -> None:
        raise dbus.exceptions.DBusException(
            "org.freedesktop.DBus.Error.PropertyReadOnly"
        )

    def _item_properties(self, item_id: int) -> dict[str, object]:
        if item_id == 0:
            return {"children-display": dbus.String("submenu")}
        if item_id not in MENU_ITEM_IDS:
            raise dbus.exceptions.DBusException(
                "org.freedesktop.DBus.Error.InvalidArgs"
            )
        if item_id in {
            MENU_SEPARATOR_PRIMARY,
            MENU_SEPARATOR_TOOLS,
            MENU_SEPARATOR_QUIT,
        }:
            return {"type": dbus.String("separator")}

        labels = {
            MENU_LAYOUT: (
                f"Текущая раскладка: {layout_label(self._group)}"
                if self._group >= 0
                else "Текущая раскладка определяется"
            ),
            MENU_SWITCH_LAYOUT: alternate_layout_action_label(self._group),
            MENU_SETTINGS: "Настройки KeySwitch…",
            MENU_AUTOSWITCH: "Автопереключение",
            MENU_SOUND: "Звуковые эффекты",
            MENU_NOTIFICATIONS: "Уведомления об исправлениях",
            MENU_HISTORY: "История исправлений…",
            MENU_EXCEPTIONS: "Программы-исключения…",
            MENU_ABOUT: "О программе…",
            MENU_QUIT: "Выход",
        }
        icons = {
            MENU_LAYOUT: self._layout_icon,
            MENU_SWITCH_LAYOUT: "input-keyboard-symbolic",
            MENU_SETTINGS: "preferences-system-symbolic",
            MENU_AUTOSWITCH: "input-keyboard-symbolic",
            MENU_SOUND: "audio-volume-high-symbolic",
            MENU_NOTIFICATIONS: "preferences-system-notifications-symbolic",
            MENU_HISTORY: "document-open-recent-symbolic",
            MENU_EXCEPTIONS: "application-x-executable-symbolic",
            MENU_ABOUT: "help-about-symbolic",
            MENU_QUIT: "application-exit-symbolic",
        }
        properties: dict[str, object] = {
            "label": dbus.String(labels[item_id]),
            "icon-name": dbus.String(icons[item_id]),
        }
        if item_id == MENU_LAYOUT:
            properties["enabled"] = dbus.Boolean(False)
        elif item_id == MENU_SWITCH_LAYOUT:
            properties["enabled"] = dbus.Boolean(
                alternate_layout_group(self._group) is not None
                and item_id in self._actions
            )
        if item_id in {MENU_AUTOSWITCH, MENU_SOUND, MENU_NOTIFICATIONS}:
            states = {
                MENU_AUTOSWITCH: self._enabled,
                MENU_SOUND: self._sound_enabled,
                MENU_NOTIFICATIONS: self._notifications_enabled,
            }
            properties["toggle-type"] = dbus.String("checkmark")
            properties["toggle-state"] = dbus.Int32(1 if states[item_id] else 0)
        if item_id not in self._actions and item_id not in {
            MENU_LAYOUT,
            MENU_SWITCH_LAYOUT,
        }:
            properties["enabled"] = dbus.Boolean(False)
        return properties

    @staticmethod
    def _filtered_properties(
        properties: dict[str, object], requested: list[str]
    ) -> dbus.Dictionary[str, object]:
        if requested:
            properties = {
                name: value for name, value in properties.items() if name in requested
            }
        return dbus.Dictionary(properties, signature="sv")

    def _layout(
        self,
        item_id: int,
        recursion_depth: int,
        requested: list[str],
        *,
        variant: bool,
    ) -> dbus.Struct:
        properties = self._filtered_properties(
            self._item_properties(item_id), requested
        )
        children: list[dbus.Struct] = []
        if item_id == 0 and recursion_depth != 0:
            children = [
                self._layout(child_id, 0, requested, variant=True)
                for child_id in MENU_ITEM_IDS
            ]
        return dbus.Struct(
            (
                dbus.Int32(item_id),
                properties,
                dbus.Array(children, signature="v"),
            ),
            signature="ia{sv}av",
            variant_level=1 if variant else 0,
        )

    @dbus.service.method(
        MENU_INTERFACE, in_signature="iias", out_signature="u(ia{sv}av)"
    )
    def GetLayout(
        self, parent_id: int, recursion_depth: int, property_names: list[str]
    ) -> tuple[dbus.UInt32, dbus.Struct]:
        return (
            dbus.UInt32(self._revision),
            self._layout(
                int(parent_id),
                int(recursion_depth),
                property_names,
                variant=False,
            ),
        )

    @dbus.service.method(
        MENU_INTERFACE, in_signature="aias", out_signature="a(ia{sv})"
    )
    def GetGroupProperties(
        self, item_ids: list[int], property_names: list[str]
    ) -> dbus.Array[dbus.Struct]:
        selected = (
            [int(item_id) for item_id in item_ids]
            if item_ids
            else [0, *MENU_ITEM_IDS]
        )
        result: list[dbus.Struct] = []
        for item_id in selected:
            try:
                properties = self._item_properties(item_id)
            except dbus.exceptions.DBusException:
                continue
            result.append(
                dbus.Struct(
                    (
                        dbus.Int32(item_id),
                        self._filtered_properties(properties, property_names),
                    ),
                    signature="ia{sv}",
                )
            )
        return dbus.Array(result, signature="(ia{sv})")

    @dbus.service.method(MENU_INTERFACE, in_signature="is", out_signature="v")
    def GetProperty(self, item_id: int, property_name: str) -> object:
        properties = self._item_properties(int(item_id))
        defaults: dict[str, object] = {
            "type": dbus.String("standard"),
            "label": dbus.String(""),
            "enabled": dbus.Boolean(True),
            "visible": dbus.Boolean(True),
            "icon-name": dbus.String(""),
            "toggle-type": dbus.String(""),
            "toggle-state": dbus.Int32(-1),
            "children-display": dbus.String(""),
        }
        if property_name in properties:
            return properties[property_name]
        if property_name in defaults:
            return defaults[property_name]
        raise dbus.exceptions.DBusException(
            "org.freedesktop.DBus.Error.UnknownProperty"
        )

    def _dispatch(self, item_id: int, event_id: str) -> bool:
        known = item_id == 0 or item_id in MENU_ITEM_IDS
        if event_id != "clicked":
            return known
        callback = self._actions.get(item_id)
        if callback is not None:
            GLib.idle_add(callback)
        return known

    @dbus.service.method(MENU_INTERFACE, in_signature="isvu", out_signature="")
    def Event(
        self, item_id: int, event_id: str, _data: object, _timestamp: int
    ) -> None:
        self._dispatch(int(item_id), event_id)

    @dbus.service.method(MENU_INTERFACE, in_signature="a(isvu)", out_signature="ai")
    def EventGroup(
        self, events: Iterable[MenuEvent]
    ) -> dbus.Array[dbus.Int32]:
        errors = [
            dbus.Int32(int(item_id))
            for item_id, event_id, _data, _timestamp in events
            if not self._dispatch(int(item_id), event_id)
        ]
        return dbus.Array(errors, signature="i")

    @dbus.service.method(MENU_INTERFACE, in_signature="i", out_signature="b")
    def AboutToShow(self, _item_id: int) -> bool:
        return False

    @dbus.service.method(MENU_INTERFACE, in_signature="ai", out_signature="aiai")
    def AboutToShowGroup(
        self, item_ids: list[int]
    ) -> tuple[dbus.Array[dbus.Int32], dbus.Array[dbus.Int32]]:
        errors = [
            dbus.Int32(int(item_id))
            for item_id in item_ids
            if int(item_id) != 0 and int(item_id) not in MENU_ITEM_IDS
        ]
        return (
            dbus.Array[dbus.Int32]([], signature="i"),
            dbus.Array(errors, signature="i"),
        )

    @dbus.service.signal(MENU_INTERFACE, signature="a(ia{sv})a(ias)")
    def ItemsPropertiesUpdated(self, _updated: object, _removed: object) -> None:
        return None

    @dbus.service.signal(MENU_INTERFACE, signature="ui")
    def LayoutUpdated(self, _revision: int, _parent: int) -> None:
        return None

    @dbus.service.signal(MENU_INTERFACE, signature="iu")
    def ItemActivationRequested(self, _item_id: int, _timestamp: int) -> None:
        return None

    def request_open(self) -> None:
        self.ItemActivationRequested(dbus.Int32(0), dbus.UInt32(0))

    def _notify(self, item_id: int, properties: dict[str, object]) -> None:
        self._revision += 1
        updated = dbus.Array(
            [
                dbus.Struct(
                    (
                        dbus.Int32(item_id),
                        dbus.Dictionary(properties, signature="sv"),
                    ),
                    signature="ia{sv}",
                )
            ],
            signature="(ia{sv})",
        )
        self.ItemsPropertiesUpdated(
            updated,
            dbus.Array([], signature="(ias)"),
        )

    def set_indicator_state(self, enabled: bool, group: int, icon_name: str) -> None:
        group_changed = self._group != group
        if group_changed or self._layout_icon != icon_name:
            self._group = group
            self._layout_icon = icon_name
            label = (
                f"Текущая раскладка: {layout_label(group)}"
                if group >= 0
                else "Текущая раскладка определяется"
            )
            self._notify(
                MENU_LAYOUT,
                {
                    "label": dbus.String(label),
                    "icon-name": dbus.String(icon_name),
                },
            )
        if group_changed:
            self._notify(
                MENU_SWITCH_LAYOUT,
                {
                    "label": dbus.String(alternate_layout_action_label(group)),
                    "enabled": dbus.Boolean(
                        alternate_layout_group(group) is not None
                        and MENU_SWITCH_LAYOUT in self._actions
                    ),
                },
            )
        if self._enabled != enabled:
            self._enabled = enabled
            self._notify(
                MENU_AUTOSWITCH,
                {"toggle-state": dbus.Int32(1 if enabled else 0)},
            )

    def set_sound_enabled(self, enabled: bool) -> None:
        if self._sound_enabled == enabled:
            return
        self._sound_enabled = enabled
        self._notify(
            MENU_SOUND, {"toggle-state": dbus.Int32(1 if enabled else 0)}
        )

    def set_notifications_enabled(self, enabled: bool) -> None:
        if self._notifications_enabled == enabled:
            return
        self._notifications_enabled = enabled
        self._notify(
            MENU_NOTIFICATIONS,
            {"toggle-state": dbus.Int32(1 if enabled else 0)},
        )

    def close(self) -> None:
        try:
            self.remove_from_connection()
        except LookupError:
            pass


class StatusNotifierItem(dbus.service.Object):
    def __init__(
        self,
        on_activate: TrayAction,
        on_toggle: TrayAction,
        icon_theme_path: Path,
        *,
        on_switch_layout: TrayAction | None = None,
        on_sound_toggle: TrayAction | None = None,
        on_notifications_toggle: TrayAction | None = None,
        on_history: TrayAction | None = None,
        on_exceptions: TrayAction | None = None,
        on_about: TrayAction | None = None,
        on_quit: TrayAction | None = None,
    ) -> None:
        DBusGMainLoop(set_as_default=True)
        self._bus = dbus.SessionBus()
        self._bus_name = dbus.service.BusName(
            f"io.github.olegius88.KeySwitch.StatusNotifierItem.p{os.getpid()}",
            self._bus,
        )
        self._on_toggle = on_toggle
        self._status = "Active"
        self._icon_name = "keyswitch"
        self._subtitle = "Раскладка определяется"
        self._enabled = True
        self._group = -1
        self._indicator_style = "letters"
        self._sound_enabled = False
        self._notifications_enabled = True
        self._icon_theme_path = str(icon_theme_path)
        actions: dict[int, TrayAction] = {
            MENU_SETTINGS: on_activate,
            MENU_AUTOSWITCH: on_toggle,
        }
        optional_actions: dict[int, TrayAction | None] = {
            MENU_SWITCH_LAYOUT: on_switch_layout,
            MENU_SOUND: on_sound_toggle,
            MENU_NOTIFICATIONS: on_notifications_toggle,
            MENU_HISTORY: on_history,
            MENU_EXCEPTIONS: on_exceptions,
            MENU_ABOUT: on_about,
            MENU_QUIT: on_quit,
        }
        actions.update(
            {
                item_id: callback
                for item_id, callback in optional_actions.items()
                if callback is not None
            }
        )
        self._menu = StatusNotifierMenu(self._bus_name, actions, icon_theme_path)
        super().__init__(self._bus_name, OBJECT_PATH)
        watcher = self._bus.get_object(
            "org.kde.StatusNotifierWatcher", "/StatusNotifierWatcher"
        )
        dbus.Interface(
            watcher, "org.kde.StatusNotifierWatcher"
        ).RegisterStatusNotifierItem(OBJECT_PATH)

    def _properties(self) -> dict[str, object]:
        tooltip = dbus.Struct(
            (
                dbus.String(self._icon_name),
                dbus.Array([], signature="(iiay)"),
                dbus.String("KeySwitch"),
                dbus.String(self._subtitle),
            ),
            signature="sa(iiay)ss",
        )
        return {
            "Category": dbus.String("ApplicationStatus"),
            "Id": dbus.String("keyswitch"),
            "Title": dbus.String("KeySwitch"),
            "Status": dbus.String(self._status),
            "WindowId": dbus.UInt32(0),
            "IconName": dbus.String(self._icon_name),
            "IconThemePath": dbus.String(self._icon_theme_path),
            "AttentionIconName": dbus.String("keyswitch-paused"),
            "OverlayIconName": dbus.String(""),
            "ToolTip": tooltip,
            "ItemIsMenu": dbus.Boolean(True),
            "Menu": dbus.ObjectPath(MENU_PATH),
        }

    @dbus.service.method(PROPERTIES_INTERFACE, in_signature="ss", out_signature="v")
    def Get(self, interface: str, property_name: str) -> object:
        if interface != ITEM_INTERFACE:
            raise dbus.exceptions.DBusException(
                "org.freedesktop.DBus.Error.UnknownInterface"
            )
        properties = self._properties()
        if property_name not in properties:
            raise dbus.exceptions.DBusException(
                "org.freedesktop.DBus.Error.UnknownProperty"
            )
        return properties[property_name]

    @dbus.service.method(PROPERTIES_INTERFACE, in_signature="s", out_signature="a{sv}")
    def GetAll(self, interface: str) -> dict[str, object]:
        return self._properties() if interface == ITEM_INTERFACE else {}

    @dbus.service.method(PROPERTIES_INTERFACE, in_signature="ssv", out_signature="")
    def Set(self, _interface: str, _property_name: str, _value: object) -> None:
        raise dbus.exceptions.DBusException(
            "org.freedesktop.DBus.Error.PropertyReadOnly"
        )

    @dbus.service.method(ITEM_INTERFACE, in_signature="ii", out_signature="")
    def Activate(self, _x: int, _y: int) -> None:
        self._menu.request_open()

    @dbus.service.method(ITEM_INTERFACE, in_signature="ii", out_signature="")
    def SecondaryActivate(self, _x: int, _y: int) -> None:
        GLib.idle_add(self._on_toggle)

    @dbus.service.method(ITEM_INTERFACE, in_signature="ii", out_signature="")
    def ContextMenu(self, _x: int, _y: int) -> None:
        self._menu.request_open()

    @dbus.service.method(ITEM_INTERFACE, in_signature="is", out_signature="")
    def Scroll(self, _delta: int, _orientation: str) -> None:
        return None

    @dbus.service.signal(ITEM_INTERFACE, signature="s")
    def NewStatus(self, _status: str) -> None:
        return None

    @dbus.service.signal(ITEM_INTERFACE, signature="")
    def NewIcon(self) -> None:
        return None

    @dbus.service.signal(ITEM_INTERFACE, signature="")
    def NewToolTip(self) -> None:
        return None

    def set_enabled(self, enabled: bool) -> None:
        if self._enabled == enabled:
            return
        self._enabled = enabled
        self._refresh()

    def set_layout(self, group: int) -> None:
        if self._group == group:
            return
        self._group = group
        self._refresh()

    def set_indicator_style(self, style: object) -> None:
        normalized = normalize_indicator_style(style)
        if self._indicator_style == normalized:
            return
        self._indicator_style = normalized
        self._refresh()

    def set_sound_enabled(self, enabled: bool) -> None:
        self._sound_enabled = enabled
        self._menu.set_sound_enabled(enabled)

    def set_notifications_enabled(self, enabled: bool) -> None:
        self._notifications_enabled = enabled
        self._menu.set_notifications_enabled(enabled)

    def _refresh(self) -> None:
        # Keep the item visible while paused so it remains possible to resume
        # from the panel. The current layout stays visible in both states.
        self._status = "Active"
        self._icon_name = layout_icon_name(self._indicator_style, self._group)
        label = layout_label(self._group)
        state = (
            "автокоррекция включена"
            if self._enabled
            else "автокоррекция на паузе"
        )
        self._subtitle = (
            f"{label} · {state}" if label != "—" else state.capitalize()
        )
        self._menu.set_indicator_state(
            self._enabled, self._group, self._icon_name
        )
        self.NewStatus(self._status)
        self.NewIcon()
        self.NewToolTip()

    def close(self) -> None:
        self._menu.close()
        try:
            self.remove_from_connection()
        except LookupError:
            pass
