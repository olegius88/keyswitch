"""Whether the settings window is drawn dark: the user's choice, or the system's.

Tk follows no system appearance of its own, so the window asks the system when
the user leaves the choice to it (``appearance.theme`` is ``system``). Each
platform answers in its own words, read here; the call that reads them belongs
to the platform (DesktopServices.system_dark_theme).
"""

from __future__ import annotations

import logging
from collections.abc import Callable


LOGGER = logging.getLogger(__name__)
# macOS names the dark appearance in the global AppleInterfaceStyle default; in
# the light one the default is not set at all.
MACOS_DARK_INTERFACE_STYLE = "Dark"


def dark_theme(theme: str, system_dark: Callable[[], bool]) -> bool:
    """Whether the window is drawn dark for the ``appearance.theme`` setting ``theme``.

    An unknown value reads as the system's choice, as the settings windows show it.
    The system is only asked when it decides, and an answer it cannot give is light.
    """

    if theme in {"light", "dark"}:
        return theme == "dark"
    try:
        return system_dark()
    except Exception:
        LOGGER.warning("Системная тема не определена; окно остаётся светлым", exc_info=True)
        return False


def dark_from_apps_use_light_theme(value: int | None) -> bool:
    """Windows: the AppsUseLightTheme value of the user's Personalize key, None when it is absent.

    0 there asks apps to be dark and 1 light; the darkdetect library reads the
    value the same way. It is a best effort, so anything but 0, and no value at
    all, is light.
    """

    return value == 0


def dark_from_interface_style(style: str) -> bool:
    """macOS: the value of the global AppleInterfaceStyle default, empty when it is not set."""

    return style == MACOS_DARK_INTERFACE_STYLE
