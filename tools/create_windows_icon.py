"""Create the deterministic Windows application icon used by native builds."""

from __future__ import annotations

import argparse
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ICON_SIZE_PIXELS = 256
BADGE_BACKGROUND_BOX_PIXELS = (8, 8, 247, 247)
BADGE_BACKGROUND_RADIUS_PIXELS = 56
BADGE_BACKGROUND_COLOR = (35, 92, 190, 255)
BADGE_BORDER_BOX_PIXELS = (31, 31, 224, 224)
BADGE_BORDER_RADIUS_PIXELS = 40
BADGE_BORDER_COLOR = (255, 255, 255, 70)
BADGE_BORDER_WIDTH_PIXELS = 5
LABEL_FONT_SIZE_POINTS = 86
TEXT_CENTERING_DIVISOR = 2
ICON_EXPORT_SIZES_PIXELS = ((16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256))


def create_icon(output: Path) -> None:
    size = ICON_SIZE_PIXELS
    image = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)
    draw.rounded_rectangle(
        BADGE_BACKGROUND_BOX_PIXELS, radius=BADGE_BACKGROUND_RADIUS_PIXELS, fill=BADGE_BACKGROUND_COLOR
    )
    draw.rounded_rectangle(
        BADGE_BORDER_BOX_PIXELS,
        radius=BADGE_BORDER_RADIUS_PIXELS,
        outline=BADGE_BORDER_COLOR,
        width=BADGE_BORDER_WIDTH_PIXELS,
    )
    font: ImageFont.FreeTypeFont | ImageFont.ImageFont
    try:
        font = ImageFont.truetype("arialbd.ttf", LABEL_FONT_SIZE_POINTS)
    except OSError:
        font = ImageFont.load_default()
    text = "KS"
    left, top, right, bottom = draw.textbbox((0, 0), text, font=font)
    width = right - left
    height = bottom - top
    draw.text(
        ((size - width) / TEXT_CENTERING_DIVISOR, (size - height) / TEXT_CENTERING_DIVISOR - top),
        text,
        font=font,
        fill="white",
    )
    output.parent.mkdir(parents=True, exist_ok=True)
    image.save(
        output,
        format="ICO",
        sizes=ICON_EXPORT_SIZES_PIXELS,
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("output", type=Path)
    arguments = parser.parse_args(argv)
    create_icon(arguments.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
