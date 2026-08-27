"""Native Windows notification-area implementation backed by pystray/Pillow."""

from __future__ import annotations

from collections.abc import Callable
from pathlib import Path

import pystray
from pystray._util import win32
from PIL import Image, ImageDraw, ImageFont

from .windows_tray import (
    WindowsTrayActions,
    WindowsTrayState,
    menu_activation_message,
)


ICON_SIZE = 64


class LeftClickMenuIcon(pystray.Icon):
    """Make the primary click open the same complete menu as the right click."""

    def _on_notify(self, wparam: int, lparam: int) -> None:
        message = menu_activation_message(
            lparam,
            win32.WM_LBUTTONUP,
            win32.WM_RBUTTONUP,
        )
        super()._on_notify(wparam, message)


class PystrayWindowsAdapter:
    def __init__(self) -> None:
        self._icon: pystray.Icon | None = None
        self._state: Callable[[], WindowsTrayState] | None = None

    def start(
        self,
        actions: WindowsTrayActions,
        state: Callable[[], WindowsTrayState],
    ) -> None:
        self._state = state
        menu = pystray.Menu(
            pystray.MenuItem(
                lambda _item: f"Текущая раскладка: {state().label}",
                None,
                enabled=False,
            ),
            pystray.MenuItem(
                "Настройки KeySwitch…",
                lambda _icon, _item: actions.show_settings(),
                default=True,
            ),
            pystray.Menu.SEPARATOR,
            pystray.MenuItem(
                "Автопереключение",
                lambda _icon, _item: actions.toggle_engine(),
                checked=lambda _item: state().enabled,
            ),
            pystray.MenuItem(
                "Звуковые эффекты",
                lambda _icon, _item: actions.toggle_sound(),
                checked=lambda _item: state().sound_enabled,
            ),
            pystray.MenuItem(
                "Уведомления об исправлениях",
                lambda _icon, _item: actions.toggle_notifications(),
                checked=lambda _item: state().notifications_enabled,
            ),
            pystray.Menu.SEPARATOR,
            pystray.MenuItem(
                "История исправлений…",
                lambda _icon, _item: actions.show_history(),
            ),
            pystray.MenuItem(
                "Программы-исключения…",
                lambda _icon, _item: actions.show_exclusions(),
            ),
            pystray.MenuItem(
                "О программе…",
                lambda _icon, _item: actions.show_about(),
            ),
            pystray.Menu.SEPARATOR,
            pystray.MenuItem(
                "Выход",
                lambda _icon, _item: actions.quit_application(),
            ),
        )
        self._icon = LeftClickMenuIcon(
            "keyswitch",
            self._render(state()),
            "KeySwitch",
            menu,
        )
        self._icon.run_detached()

    def update(self, state: WindowsTrayState) -> None:
        icon = self._icon
        if icon is None:
            return
        icon.icon = self._render(state)
        icon.title = f"KeySwitch — {state.label}"
        icon.update_menu()

    def notify(self, title: str, message: str) -> None:
        if self._icon is not None:
            self._icon.notify(message, title)

    def close(self) -> None:
        icon, self._icon = self._icon, None
        if icon is not None:
            icon.stop()

    @staticmethod
    def _font(size: int) -> ImageFont.FreeTypeFont | ImageFont.ImageFont:
        candidates = (
            Path(r"C:\Windows\Fonts\segoeuib.ttf"),
            Path(r"C:\Windows\Fonts\arialbd.ttf"),
        )
        for path in candidates:
            try:
                return ImageFont.truetype(str(path), size)
            except OSError:
                continue
        return ImageFont.load_default()

    @classmethod
    def _render(cls, state: WindowsTrayState) -> Image.Image:
        image = Image.new("RGBA", (ICON_SIZE, ICON_SIZE), (0, 0, 0, 0))
        draw = ImageDraw.Draw(image)
        if not state.enabled:
            background = (105, 105, 105, 255)
        elif state.group == 1:
            background = (194, 42, 55, 255)
        else:
            background = (27, 92, 180, 255)
        draw.rounded_rectangle((2, 2, 61, 61), radius=13, fill=background)
        if state.indicator_style == "flags" and state.group in (0, 1):
            cls._draw_flag(draw, state.group)
        else:
            text = state.label if state.group >= 0 else "?"
            font = cls._font(27)
            bounds = draw.textbbox((0, 0), text, font=font)
            width = bounds[2] - bounds[0]
            height = bounds[3] - bounds[1]
            draw.text(
                ((ICON_SIZE - width) / 2, (ICON_SIZE - height) / 2 - bounds[1]),
                text,
                font=font,
                fill="white",
            )
        if not state.enabled:
            draw.line((12, 52, 52, 12), fill=(255, 255, 255, 235), width=7)
        return image

    @staticmethod
    def _draw_flag(draw: ImageDraw.ImageDraw, group: int) -> None:
        left, top, right, bottom = 9, 16, 55, 48
        if group == 1:
            height = (bottom - top) // 3
            draw.rectangle((left, top, right, top + height), fill="white")
            draw.rectangle(
                (left, top + height, right, top + height * 2),
                fill=(33, 71, 156, 255),
            )
            draw.rectangle(
                (left, top + height * 2, right, bottom),
                fill=(213, 43, 30, 255),
            )
            return
        stripe_height = max(2, (bottom - top) // 7)
        for index in range(7):
            color = (190, 10, 48, 255) if index % 2 == 0 else "white"
            draw.rectangle(
                (
                    left,
                    top + index * stripe_height,
                    right,
                    min(bottom, top + (index + 1) * stripe_height),
                ),
                fill=color,
            )
        draw.rectangle((left, top, left + 20, top + 18), fill=(0, 40, 104, 255))
