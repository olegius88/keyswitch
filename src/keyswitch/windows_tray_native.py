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
from .constants.geometry import CENTERING_DIVISOR
from .constants.tray_icon import (
    ALTERNATE_EVERY_OTHER,
    BADGE_CORNER_RADIUS_PIXELS,
    BADGE_DISABLED_RGBA,
    BADGE_EDGE_PIXELS,
    BADGE_EN_RGBA,
    BADGE_FONT_SIZE_PIXELS,
    BADGE_INSET_PIXELS,
    BADGE_RU_RGBA,
    DISABLED_SLASH_FAR_PIXELS,
    DISABLED_SLASH_NEAR_PIXELS,
    DISABLED_SLASH_RGBA,
    DISABLED_SLASH_WIDTH_PIXELS,
    RUSSIAN_FLAG_BAND_COUNT,
    RUSSIAN_FLAG_BLUE_RGBA,
    RUSSIAN_FLAG_RED_RGBA,
    RUSSIAN_FLAG_SECOND_BAND_NUMERATOR,
    TEXTBBOX_BOTTOM_INDEX,
    TEXTBBOX_RIGHT_INDEX,
    TRAY_ICON_SIZE_PIXELS,
    US_FLAG_CANTON_HEIGHT_STRIPES,
    US_FLAG_CANTON_RGBA,
    US_FLAG_CANTON_WIDTH_DENOMINATOR,
    US_FLAG_CANTON_WIDTH_NUMERATOR,
    US_FLAG_STARS_PER_LONG_ROW,
    US_FLAG_STARS_PER_SHORT_ROW,
    US_FLAG_STAR_GRID_SPACING_PIXELS,
    US_FLAG_STAR_OFFSET_LONG_ROW_PIXELS,
    US_FLAG_STAR_OFFSET_SHORT_ROW_PIXELS,
    US_FLAG_STAR_ROWS,
    US_FLAG_STAR_TOP_MARGIN_PIXELS,
    US_FLAG_STRIPE_COUNT,
    US_FLAG_STRIPE_RED_RGBA,
)


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
        image = Image.new("RGBA", (TRAY_ICON_SIZE_PIXELS, TRAY_ICON_SIZE_PIXELS), (0, 0, 0, 0))
        draw = ImageDraw.Draw(image)
        flag_indicator = (
            state.indicator_style == "flags" and state.group in (0, 1)
        )
        if flag_indicator:
            cls._draw_flag(draw, state.group)
        else:
            if not state.enabled:
                background = BADGE_DISABLED_RGBA
            elif state.group == 1:
                background = BADGE_RU_RGBA
            else:
                background = BADGE_EN_RGBA
            draw.rounded_rectangle(
                (BADGE_INSET_PIXELS, BADGE_INSET_PIXELS, BADGE_EDGE_PIXELS, BADGE_EDGE_PIXELS),
                radius=BADGE_CORNER_RADIUS_PIXELS,
                fill=background,
            )
            text = state.label if state.group >= 0 else "?"
            font = cls._font(BADGE_FONT_SIZE_PIXELS)
            bounds = draw.textbbox((0, 0), text, font=font)
            width = bounds[TEXTBBOX_RIGHT_INDEX] - bounds[0]
            height = bounds[TEXTBBOX_BOTTOM_INDEX] - bounds[1]
            draw.text(
                (
                    (TRAY_ICON_SIZE_PIXELS - width) / CENTERING_DIVISOR,
                    (TRAY_ICON_SIZE_PIXELS - height) / CENTERING_DIVISOR - bounds[1],
                ),
                text,
                font=font,
                fill="white",
            )
        if not state.enabled:
            draw.line(
                (
                    DISABLED_SLASH_NEAR_PIXELS, DISABLED_SLASH_FAR_PIXELS,
                    DISABLED_SLASH_FAR_PIXELS, DISABLED_SLASH_NEAR_PIXELS,
                ),
                fill=DISABLED_SLASH_RGBA,
                width=DISABLED_SLASH_WIDTH_PIXELS,
            )
        return image

    @staticmethod
    def _draw_flag(draw: ImageDraw.ImageDraw, group: int) -> None:
        left, top, right, bottom = 0, 0, TRAY_ICON_SIZE_PIXELS - 1, TRAY_ICON_SIZE_PIXELS - 1
        if group == 1:
            first_edge = TRAY_ICON_SIZE_PIXELS // RUSSIAN_FLAG_BAND_COUNT
            second_edge = TRAY_ICON_SIZE_PIXELS * RUSSIAN_FLAG_SECOND_BAND_NUMERATOR // RUSSIAN_FLAG_BAND_COUNT
            draw.rectangle((left, top, right, first_edge - 1), fill="white")
            draw.rectangle(
                (left, first_edge, right, second_edge - 1),
                fill=RUSSIAN_FLAG_BLUE_RGBA,
            )
            draw.rectangle(
                (left, second_edge, right, bottom),
                fill=RUSSIAN_FLAG_RED_RGBA,
            )
            return
        for index in range(US_FLAG_STRIPE_COUNT):
            stripe_top = index * TRAY_ICON_SIZE_PIXELS // US_FLAG_STRIPE_COUNT
            stripe_bottom = (index + 1) * TRAY_ICON_SIZE_PIXELS // US_FLAG_STRIPE_COUNT - 1
            color = US_FLAG_STRIPE_RED_RGBA if index % ALTERNATE_EVERY_OTHER == 0 else "white"
            draw.rectangle(
                (left, stripe_top, right, stripe_bottom),
                fill=color,
            )
        canton_right = TRAY_ICON_SIZE_PIXELS * US_FLAG_CANTON_WIDTH_NUMERATOR // US_FLAG_CANTON_WIDTH_DENOMINATOR - 1
        canton_bottom = TRAY_ICON_SIZE_PIXELS * US_FLAG_CANTON_HEIGHT_STRIPES // US_FLAG_STRIPE_COUNT - 1
        draw.rectangle(
            (left, top, canton_right, canton_bottom),
            fill=US_FLAG_CANTON_RGBA,
        )
        for row in range(US_FLAG_STAR_ROWS):
            stars_in_row = US_FLAG_STARS_PER_LONG_ROW if row % ALTERNATE_EVERY_OTHER == 0 else US_FLAG_STARS_PER_SHORT_ROW
            offset = (
                US_FLAG_STAR_OFFSET_LONG_ROW_PIXELS
                if stars_in_row == US_FLAG_STARS_PER_LONG_ROW
                else US_FLAG_STAR_OFFSET_SHORT_ROW_PIXELS
            )
            for column in range(stars_in_row):
                center_x = offset + column * US_FLAG_STAR_GRID_SPACING_PIXELS
                center_y = US_FLAG_STAR_TOP_MARGIN_PIXELS + row * US_FLAG_STAR_GRID_SPACING_PIXELS
                draw.ellipse(
                    (center_x - 1, center_y - 1, center_x + 1, center_y + 1),
                    fill="white",
                )
