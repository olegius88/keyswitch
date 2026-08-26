"""Pure layout-indicator mapping shared by the tray and tests."""

from __future__ import annotations


INDICATOR_STYLES = ("letters", "flags")
LAYOUT_LABELS = {0: "EN", 1: "RU"}


def normalize_indicator_style(value: object) -> str:
    style = str(value)
    return style if style in INDICATOR_STYLES else "letters"


def layout_label(group: int) -> str:
    return LAYOUT_LABELS.get(group, "—")


def layout_icon_name(style: object, group: int) -> str:
    normalized = normalize_indicator_style(style)
    icons = {
        "letters": {0: "keyswitch-en", 1: "keyswitch-ru"},
        "flags": {0: "keyswitch-flag-us", 1: "keyswitch-flag-ru"},
    }
    return icons[normalized].get(group, "keyswitch")
