"""Sizes, colours and flag geometry of the tray icon."""

from __future__ import annotations

from typing import Final

# Side of the square Windows tray icon image.
TRAY_ICON_SIZE_PIXELS: Final = 64
# Letter badge (the non-flag indicator style).
BADGE_INSET_PIXELS: Final = 2
BADGE_EDGE_PIXELS: Final = 61
BADGE_CORNER_RADIUS_PIXELS: Final = 13
BADGE_FONT_SIZE_PIXELS: Final = 27
# Indices into PIL's (left, top, right, bottom) textbbox tuple.
TEXTBBOX_RIGHT_INDEX: Final = 2
TEXTBBOX_BOTTOM_INDEX: Final = 3
BADGE_EN_RGBA: Final = (27, 92, 180, 255)
BADGE_RU_RGBA: Final = (194, 42, 55, 255)
BADGE_DISABLED_RGBA: Final = (105, 105, 105, 255)
# The diagonal slash drawn over a disabled badge or flag.
DISABLED_SLASH_NEAR_PIXELS: Final = 12
DISABLED_SLASH_FAR_PIXELS: Final = 52
DISABLED_SLASH_WIDTH_PIXELS: Final = 7
DISABLED_SLASH_RGBA: Final = (255, 255, 255, 235)
# The modulus of every even/odd stripe and star-row check of the flag drawing.
ALTERNATE_EVERY_OTHER: Final = 2
# Russian flag (group 1, see LAYOUT_LABELS): three equal horizontal bands, white/blue/red.
RUSSIAN_FLAG_BAND_COUNT: Final = 3
RUSSIAN_FLAG_SECOND_BAND_NUMERATOR: Final = 2
RUSSIAN_FLAG_BLUE_RGBA: Final = (0, 57, 166, 255)
RUSSIAN_FLAG_RED_RGBA: Final = (213, 43, 30, 255)
# US flag (any other group): 13 stripes, a canton, and a 5-row star field.
US_FLAG_STRIPE_COUNT: Final = 13
US_FLAG_STRIPE_RED_RGBA: Final = (178, 34, 52, 255)
US_FLAG_CANTON_WIDTH_NUMERATOR: Final = 2
US_FLAG_CANTON_WIDTH_DENOMINATOR: Final = 5
US_FLAG_CANTON_HEIGHT_STRIPES: Final = 7
US_FLAG_CANTON_RGBA: Final = (60, 59, 110, 255)
US_FLAG_STAR_ROWS: Final = 5
US_FLAG_STARS_PER_LONG_ROW: Final = 4
US_FLAG_STARS_PER_SHORT_ROW: Final = 3
US_FLAG_STAR_OFFSET_LONG_ROW_PIXELS: Final = 3
US_FLAG_STAR_OFFSET_SHORT_ROW_PIXELS: Final = 6
US_FLAG_STAR_GRID_SPACING_PIXELS: Final = 6
US_FLAG_STAR_TOP_MARGIN_PIXELS: Final = 3
