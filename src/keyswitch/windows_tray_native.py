"""Native Windows notification-area implementation backed by pystray/Pillow."""

from __future__ import annotations

from collections.abc import Callable
from pathlib import Path

import pystray
from pystray._util import win32
from PIL import Image, ImageDraw, ImageFont

from .tray_model import MenuEntry, menu_entries
from .windows_tray import (
    WindowsTrayActions,
    WindowsTrayState,
    menu_activation_message,
)


ICON_SIZE = 64


def _menu_items(
    actions: WindowsTrayActions,
    state: Callable[[], WindowsTrayState],
) -> tuple[pystray.MenuItem, ...]:
    """Render the shared menu description as pystray items.

    Every label, switch and enabled flag is read again when the menu opens, so
    the items follow the state instead of the moment they were built. The shape
    of the menu - how many lines and where the separators fall - does not
    depend on the state, so it is settled once here.
    """

    def entry_at(index: int) -> MenuEntry:
        return menu_entries(state(), actions)[index]

    def item(index: int, template: MenuEntry) -> pystray.MenuItem:
        def run(_icon: object, _item: object) -> None:
            action = entry_at(index).action
            if action is not None:
                action()

        return pystray.MenuItem(
            lambda _item: entry_at(index).label,
            run if template.action is not None else None,
            enabled=lambda _item: entry_at(index).enabled,
            # A checkbox appears whenever this is not None, so a plain line
            # has to pass None rather than a callable answering False.
            checked=(lambda _item: bool(entry_at(index).checked))
            if template.checked is not None else None,
            default=template.default,
        )

    return tuple(
        pystray.Menu.SEPARATOR if template.separator else item(index, template)
        for index, template in enumerate(menu_entries(state(), actions))
    )


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
        menu = pystray.Menu(*_menu_items(actions, state))
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
        flag_indicator = (
            state.indicator_style == "flags" and state.group in (0, 1)
        )
        if flag_indicator:
            cls._draw_flag(draw, state.group)
        else:
            if not state.enabled:
                background = (105, 105, 105, 255)
            elif state.group == 1:
                background = (194, 42, 55, 255)
            else:
                background = (27, 92, 180, 255)
            draw.rounded_rectangle((2, 2, 61, 61), radius=13, fill=background)
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
        left, top, right, bottom = 0, 0, ICON_SIZE - 1, ICON_SIZE - 1
        if group == 1:
            first_edge = ICON_SIZE // 3
            second_edge = ICON_SIZE * 2 // 3
            draw.rectangle((left, top, right, first_edge - 1), fill="white")
            draw.rectangle(
                (left, first_edge, right, second_edge - 1),
                fill=(0, 57, 166, 255),
            )
            draw.rectangle(
                (left, second_edge, right, bottom),
                fill=(213, 43, 30, 255),
            )
            return
        for index in range(13):
            stripe_top = index * ICON_SIZE // 13
            stripe_bottom = (index + 1) * ICON_SIZE // 13 - 1
            color = (178, 34, 52, 255) if index % 2 == 0 else "white"
            draw.rectangle(
                (left, stripe_top, right, stripe_bottom),
                fill=color,
            )
        canton_right = ICON_SIZE * 2 // 5 - 1
        canton_bottom = ICON_SIZE * 7 // 13 - 1
        draw.rectangle(
            (left, top, canton_right, canton_bottom),
            fill=(60, 59, 110, 255),
        )
        for row in range(5):
            stars_in_row = 4 if row % 2 == 0 else 3
            offset = 3 if stars_in_row == 4 else 6
            for column in range(stars_in_row):
                center_x = offset + column * 6
                center_y = 3 + row * 6
                draw.ellipse(
                    (center_x - 1, center_y - 1, center_x + 1, center_y + 1),
                    fill="white",
                )
