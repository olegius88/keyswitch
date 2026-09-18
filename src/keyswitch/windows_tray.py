"""Windows tray controller: the shared model plus the one Win32 peculiarity."""

from __future__ import annotations

from .tray_model import TrayAction, TrayActions, TrayAdapter, TrayController, TrayState

# The names the Windows frontend has always used, now pointing at the shared
# model rather than at copies of it.
TrayAction = TrayAction
WindowsTrayActions = TrayActions
WindowsTrayAdapter = TrayAdapter
WindowsTrayState = TrayState


def menu_activation_message(
    message: int,
    primary_click_message: int,
    menu_click_message: int,
) -> int:
    """Map a primary click to the native popup-menu notification."""

    return menu_click_message if message == primary_click_message else message


def _native_adapter() -> WindowsTrayAdapter:
    from .windows_tray_native import PystrayWindowsAdapter

    return PystrayWindowsAdapter()


class WindowsTray(TrayController):
    def __init__(
        self,
        actions: WindowsTrayActions,
        adapter: WindowsTrayAdapter | None = None,
    ) -> None:
        super().__init__(actions, adapter or _native_adapter())
