"""Create the deterministic Windows application icon used by native builds."""

from __future__ import annotations

import argparse
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont
import sys
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))
from keyswitch.constants.geometry import CENTERING_DIVISOR
from keyswitch.constants.release import (
    APP_ICON_BACKGROUND_BOX_PIXELS,
    APP_ICON_BACKGROUND_RADIUS_PIXELS,
    APP_ICON_BACKGROUND_RGBA,
    APP_ICON_BORDER_BOX_PIXELS,
    APP_ICON_BORDER_RADIUS_PIXELS,
    APP_ICON_BORDER_RGBA,
    APP_ICON_BORDER_WIDTH_PIXELS,
    APP_ICON_EXPORT_SIZES_PIXELS,
    APP_ICON_LABEL_FONT_SIZE_POINTS,
    APP_ICON_SIZE_PIXELS,
)


def create_icon(output: Path) -> None:
    size = APP_ICON_SIZE_PIXELS
    image = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)
    draw.rounded_rectangle(
        APP_ICON_BACKGROUND_BOX_PIXELS, radius=APP_ICON_BACKGROUND_RADIUS_PIXELS, fill=APP_ICON_BACKGROUND_RGBA
    )
    draw.rounded_rectangle(
        APP_ICON_BORDER_BOX_PIXELS,
        radius=APP_ICON_BORDER_RADIUS_PIXELS,
        outline=APP_ICON_BORDER_RGBA,
        width=APP_ICON_BORDER_WIDTH_PIXELS,
    )
    font: ImageFont.FreeTypeFont | ImageFont.ImageFont
    try:
        font = ImageFont.truetype("arialbd.ttf", APP_ICON_LABEL_FONT_SIZE_POINTS)
    except OSError:
        font = ImageFont.load_default()
    text = "KS"
    left, top, right, bottom = draw.textbbox((0, 0), text, font=font)
    width = right - left
    height = bottom - top
    draw.text(
        ((size - width) / CENTERING_DIVISOR, (size - height) / CENTERING_DIVISOR - top),
        text,
        font=font,
        fill="white",
    )
    output.parent.mkdir(parents=True, exist_ok=True)
    image.save(
        output,
        format="ICO",
        sizes=APP_ICON_EXPORT_SIZES_PIXELS,
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("output", type=Path)
    arguments = parser.parse_args(argv)
    create_icon(arguments.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
