"""Minimal StatusNotifierItem integration for XFCE/KDE-compatible panels."""

from __future__ import annotations

import os
from pathlib import Path
from typing import Callable

import dbus
import dbus.service
from dbus.mainloop.glib import DBusGMainLoop
from gi.repository import GLib


ITEM_INTERFACE = "org.kde.StatusNotifierItem"
PROPERTIES_INTERFACE = "org.freedesktop.DBus.Properties"
OBJECT_PATH = "/StatusNotifierItem"


class StatusNotifierItem(dbus.service.Object):
    def __init__(
        self,
        on_activate: Callable[[], None],
        on_toggle: Callable[[], None],
        icon_theme_path: Path,
    ) -> None:
        DBusGMainLoop(set_as_default=True)
        self._bus = dbus.SessionBus()
        self._bus_name = dbus.service.BusName(
            f"io.github.olegius88.KeySwitch.StatusNotifierItem.p{os.getpid()}",
            self._bus,
        )
        self._on_activate = on_activate
        self._on_toggle = on_toggle
        self._status = "Active"
        self._icon_name = "keyswitch"
        self._subtitle = "Автокоррекция включена"
        self._icon_theme_path = str(icon_theme_path)
        super().__init__(self._bus_name, OBJECT_PATH)
        watcher = self._bus.get_object("org.kde.StatusNotifierWatcher", "/StatusNotifierWatcher")
        dbus.Interface(watcher, "org.kde.StatusNotifierWatcher").RegisterStatusNotifierItem(
            OBJECT_PATH
        )

    def _properties(self) -> dict[str, object]:
        tooltip = dbus.Struct(
            (
                dbus.String("keyswitch"),
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
            "ItemIsMenu": dbus.Boolean(False),
            "Menu": dbus.ObjectPath("/NO_DBUSMENU"),
        }

    @dbus.service.method(PROPERTIES_INTERFACE, in_signature="ss", out_signature="v")
    def Get(self, interface: str, property_name: str):
        if interface != ITEM_INTERFACE:
            raise dbus.exceptions.DBusException("org.freedesktop.DBus.Error.UnknownInterface")
        properties = self._properties()
        if property_name not in properties:
            raise dbus.exceptions.DBusException("org.freedesktop.DBus.Error.UnknownProperty")
        return properties[property_name]

    @dbus.service.method(PROPERTIES_INTERFACE, in_signature="s", out_signature="a{sv}")
    def GetAll(self, interface: str):
        return self._properties() if interface == ITEM_INTERFACE else {}

    @dbus.service.method(PROPERTIES_INTERFACE, in_signature="ssv", out_signature="")
    def Set(self, _interface: str, _property_name: str, _value: object) -> None:
        raise dbus.exceptions.DBusException("org.freedesktop.DBus.Error.PropertyReadOnly")

    @dbus.service.method(ITEM_INTERFACE, in_signature="ii", out_signature="")
    def Activate(self, _x: int, _y: int) -> None:
        GLib.idle_add(self._on_activate)

    @dbus.service.method(ITEM_INTERFACE, in_signature="ii", out_signature="")
    def SecondaryActivate(self, _x: int, _y: int) -> None:
        GLib.idle_add(self._on_toggle)

    @dbus.service.method(ITEM_INTERFACE, in_signature="ii", out_signature="")
    def ContextMenu(self, _x: int, _y: int) -> None:
        GLib.idle_add(self._on_activate)

    @dbus.service.method(ITEM_INTERFACE, in_signature="is", out_signature="")
    def Scroll(self, _delta: int, _orientation: str) -> None:
        return None

    @dbus.service.signal(ITEM_INTERFACE, signature="s")
    def NewStatus(self, status: str) -> None:
        return None

    @dbus.service.signal(ITEM_INTERFACE, signature="")
    def NewIcon(self) -> None:
        return None

    @dbus.service.signal(ITEM_INTERFACE, signature="")
    def NewToolTip(self) -> None:
        return None

    def set_enabled(self, enabled: bool) -> None:
        # Keep the item visible while paused so it remains possible to resume
        # from the panel. The icon and tooltip communicate the paused state.
        self._status = "Active"
        self._icon_name = "keyswitch" if enabled else "keyswitch-paused"
        self._subtitle = "Автокоррекция включена" if enabled else "Автокоррекция на паузе"
        self.NewStatus(self._status)
        self.NewIcon()
        self.NewToolTip()

    def close(self) -> None:
        try:
            self.remove_from_connection()
        except LookupError:
            pass
