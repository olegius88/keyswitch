"""DBusMenu item ids and protocol version of the Linux tray menu."""

from __future__ import annotations

from typing import Final

# DBusMenu item ids of the Linux tray menu.
MENU_LAYOUT: Final = 1
MENU_SETTINGS: Final = 2
MENU_SEPARATOR_PRIMARY: Final = 3
MENU_AUTOSWITCH: Final = 4
MENU_SOUND: Final = 5
MENU_NOTIFICATIONS: Final = 6
MENU_SEPARATOR_TOOLS: Final = 7
MENU_HISTORY: Final = 8
MENU_EXCEPTIONS: Final = 9
MENU_ABOUT: Final = 10
MENU_SEPARATOR_QUIT: Final = 11
MENU_QUIT: Final = 12
MENU_SWITCH_LAYOUT: Final = 13
# The DBusMenu interface version this implementation speaks.
DBUSMENU_INTERFACE_VERSION: Final = 3
MENU_ITEM_IDS: Final = (MENU_LAYOUT, MENU_SWITCH_LAYOUT, MENU_SETTINGS, MENU_SEPARATOR_PRIMARY, MENU_AUTOSWITCH, MENU_SOUND, MENU_NOTIFICATIONS, MENU_SEPARATOR_TOOLS, MENU_EXCEPTIONS, MENU_HISTORY, MENU_ABOUT, MENU_SEPARATOR_QUIT, MENU_QUIT)
