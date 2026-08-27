"""Stateful Windows tray controller with a replaceable native boundary."""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from typing import Protocol

from .indicator import layout_label, normalize_indicator_style


TrayAction = Callable[[], None]


def menu_activation_message(
    message: int,
    primary_click_message: int,
    menu_click_message: int,
) -> int:
    """Map a primary click to the native popup-menu notification."""

    return menu_click_message if message == primary_click_message else message


@dataclass(frozen=True)
class WindowsTrayActions:
    show_settings: TrayAction
    toggle_engine: TrayAction
    toggle_sound: TrayAction
    toggle_notifications: TrayAction
    show_history: TrayAction
    show_exclusions: TrayAction
    show_about: TrayAction
    quit_application: TrayAction


@dataclass(frozen=True)
class WindowsTrayState:
    group: int = -1
    enabled: bool = True
    sound_enabled: bool = False
    notifications_enabled: bool = True
    indicator_style: str = "letters"

    @property
    def label(self) -> str:
        return layout_label(self.group)


class WindowsTrayAdapter(Protocol):
    def start(
        self,
        actions: WindowsTrayActions,
        state: Callable[[], WindowsTrayState],
    ) -> None: ...

    def update(self, state: WindowsTrayState) -> None: ...

    def notify(self, title: str, message: str) -> None: ...

    def close(self) -> None: ...


def _native_adapter() -> WindowsTrayAdapter:
    from .windows_tray_native import PystrayWindowsAdapter

    return PystrayWindowsAdapter()


class WindowsTray:
    def __init__(
        self,
        actions: WindowsTrayActions,
        adapter: WindowsTrayAdapter | None = None,
    ) -> None:
        self._adapter = adapter or _native_adapter()
        self._state = WindowsTrayState()
        self._closed = False
        self._adapter.start(actions, lambda: self._state)
        self._adapter.update(self._state)

    @property
    def state(self) -> WindowsTrayState:
        return self._state

    def _publish(self, state: WindowsTrayState) -> None:
        if self._closed:
            return
        self._state = state
        self._adapter.update(self._state)

    def set_layout(self, group: int) -> None:
        self._publish(
            WindowsTrayState(
                group,
                self._state.enabled,
                self._state.sound_enabled,
                self._state.notifications_enabled,
                self._state.indicator_style,
            )
        )

    def set_enabled(self, enabled: bool) -> None:
        self._publish(
            WindowsTrayState(
                self._state.group,
                enabled,
                self._state.sound_enabled,
                self._state.notifications_enabled,
                self._state.indicator_style,
            )
        )

    def set_sound_enabled(self, enabled: bool) -> None:
        self._publish(
            WindowsTrayState(
                self._state.group,
                self._state.enabled,
                enabled,
                self._state.notifications_enabled,
                self._state.indicator_style,
            )
        )

    def set_notifications_enabled(self, enabled: bool) -> None:
        self._publish(
            WindowsTrayState(
                self._state.group,
                self._state.enabled,
                self._state.sound_enabled,
                enabled,
                self._state.indicator_style,
            )
        )

    def set_indicator_style(self, style: object) -> None:
        self._publish(
            WindowsTrayState(
                self._state.group,
                self._state.enabled,
                self._state.sound_enabled,
                self._state.notifications_enabled,
                normalize_indicator_style(style),
            )
        )

    def notify(self, title: str, message: str) -> None:
        if not self._closed:
            self._adapter.notify(title, message)

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        self._adapter.close()
