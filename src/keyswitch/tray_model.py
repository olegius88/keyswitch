"""What a tray icon or a menu bar item shows, and what it can be asked to do.

Both platforms present the same thing - the current layout, whether correcting
is on, and the handful of switches behind it - so the state, the actions and the
controller live here and each platform supplies only the adapter that draws it.
"""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass, replace
from typing import Protocol

from .indicator import (
    alternate_layout_action_label,
    alternate_layout_group,
    layout_label,
    normalize_indicator_style,
)

TrayAction = Callable[[], None]

# The one description of the menu both platforms draw. Keeping the wording here
# rather than in each adapter is what stops the two from drifting apart.
CURRENT_LAYOUT_PREFIX = "Текущая раскладка"
SETTINGS_LABEL = "Настройки KeySwitch…"
AUTO_SWITCH_LABEL = "Автопереключение"
SOUND_LABEL = "Звуковые эффекты"
NOTIFICATIONS_LABEL = "Уведомления об исправлениях"
HISTORY_LABEL = "История исправлений…"
EXCLUSIONS_LABEL = "Программы-исключения…"
ABOUT_LABEL = "О программе…"
QUIT_LABEL = "Выход"


@dataclass(frozen=True)
class TrayActions:
    show_settings: TrayAction
    switch_layout: TrayAction
    toggle_engine: TrayAction
    toggle_sound: TrayAction
    toggle_notifications: TrayAction
    show_history: TrayAction
    show_exclusions: TrayAction
    show_about: TrayAction
    quit_application: TrayAction


@dataclass(frozen=True)
class TrayState:
    group: int = -1
    enabled: bool = True
    sound_enabled: bool = False
    notifications_enabled: bool = True
    indicator_style: str = "letters"

    @property
    def label(self) -> str:
        return layout_label(self.group)

    @property
    def alternate_layout_label(self) -> str:
        return alternate_layout_action_label(self.group)

    @property
    def can_switch_layout(self) -> bool:
        return alternate_layout_group(self.group) is not None


class TrayAdapter(Protocol):
    def start(
        self,
        actions: TrayActions,
        state: Callable[[], TrayState],
    ) -> None: ...

    def update(self, state: TrayState) -> None: ...

    def notify(self, title: str, message: str) -> None: ...

    def close(self) -> None: ...


class TrayController:
    """Hold the state and tell the adapter whenever it changes."""

    def __init__(self, actions: TrayActions, adapter: TrayAdapter) -> None:
        self._adapter = adapter
        self._state = TrayState()
        self._closed = False
        self._adapter.start(actions, lambda: self._state)
        self._adapter.update(self._state)

    @property
    def state(self) -> TrayState:
        return self._state

    def _publish(self, **changes: object) -> None:
        if self._closed:
            return
        self._state = replace(self._state, **changes)  # type: ignore[arg-type]
        self._adapter.update(self._state)

    def set_layout(self, group: int) -> None:
        self._publish(group=group)

    def set_enabled(self, enabled: bool) -> None:
        self._publish(enabled=enabled)

    def set_sound_enabled(self, enabled: bool) -> None:
        self._publish(sound_enabled=enabled)

    def set_notifications_enabled(self, enabled: bool) -> None:
        self._publish(notifications_enabled=enabled)

    def set_indicator_style(self, style: object) -> None:
        self._publish(indicator_style=normalize_indicator_style(style))

    def notify(self, title: str, message: str) -> None:
        if not self._closed:
            self._adapter.notify(title, message)

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        self._adapter.close()


@dataclass(frozen=True)
class MenuEntry:
    """One line of the menu, already resolved for the state it was built from.

    ``action`` of ``None`` marks a line that only reports something, and
    ``checked`` of ``None`` a line that is not a switch.
    """

    label: str = ""
    action: TrayAction | None = None
    checked: bool | None = None
    enabled: bool = True
    separator: bool = False
    default: bool = False


SEPARATOR = MenuEntry(separator=True, enabled=False)


def menu_entries(state: TrayState, actions: TrayActions) -> tuple[MenuEntry, ...]:
    """The menu as it should look right now."""

    return (
        MenuEntry(f"{CURRENT_LAYOUT_PREFIX}: {state.label}", enabled=False),
        MenuEntry(state.alternate_layout_label, actions.switch_layout,
                  enabled=state.can_switch_layout),
        MenuEntry(SETTINGS_LABEL, actions.show_settings, default=True),
        SEPARATOR,
        MenuEntry(AUTO_SWITCH_LABEL, actions.toggle_engine, checked=state.enabled),
        MenuEntry(SOUND_LABEL, actions.toggle_sound, checked=state.sound_enabled),
        MenuEntry(NOTIFICATIONS_LABEL, actions.toggle_notifications,
                  checked=state.notifications_enabled),
        SEPARATOR,
        MenuEntry(HISTORY_LABEL, actions.show_history),
        MenuEntry(EXCLUSIONS_LABEL, actions.show_exclusions),
        MenuEntry(ABOUT_LABEL, actions.show_about),
        SEPARATOR,
        MenuEntry(QUIT_LABEL, actions.quit_application),
    )
