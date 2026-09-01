"""Pure layout-indicator mapping shared by the tray and tests."""

from __future__ import annotations


INDICATOR_STYLES = ("letters", "flags")
LAYOUT_LABELS = {0: "EN", 1: "RU"}
LAYOUT_NAMES = {0: "английский (EN)", 1: "русский (RU)"}


def normalize_indicator_style(value: object) -> str:
    style = str(value)
    return style if style in INDICATOR_STYLES else "letters"


def layout_label(group: int) -> str:
    return LAYOUT_LABELS.get(group, "—")


def alternate_layout_group(group: int) -> int | None:
    """Return the other member of the supported EN/RU layout pair."""

    return 1 - group if group in LAYOUT_LABELS else None


def alternate_layout_action_label(group: int) -> str:
    target = alternate_layout_group(group)
    return (
        f"Переключить на {LAYOUT_NAMES[target]}"
        if target is not None
        else "Переключить язык"
    )


def layout_icon_name(style: object, group: int) -> str:
    normalized = normalize_indicator_style(style)
    icons = {
        "letters": {0: "keyswitch-en", 1: "keyswitch-ru"},
        "flags": {0: "keyswitch-flag-us", 1: "keyswitch-flag-ru"},
    }
    return icons[normalized].get(group, "keyswitch")
