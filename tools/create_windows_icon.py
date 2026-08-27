"""Create the deterministic Windows application icon used by native builds."""

from __future__ import annotations

import argparse
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


def create_icon(output: Path) -> None:
    size = 256
    image = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)
    draw.rounded_rectangle((8, 8, 247, 247), radius=56, fill=(35, 92, 190, 255))
    draw.rounded_rectangle(
        (31, 31, 224, 224),
        radius=40,
        outline=(255, 255, 255, 70),
        width=5,
    )
    font: ImageFont.FreeTypeFont | ImageFont.ImageFont
    try:
        font = ImageFont.truetype("arialbd.ttf", 86)
    except OSError:
        font = ImageFont.load_default()
    text = "KS"
    bounds = draw.textbbox((0, 0), text, font=font)
    width = bounds[2] - bounds[0]
    height = bounds[3] - bounds[1]
    draw.text(
        ((size - width) / 2, (size - height) / 2 - bounds[1]),
        text,
        font=font,
        fill="white",
    )
    output.parent.mkdir(parents=True, exist_ok=True)
    image.save(
        output,
        format="ICO",
        sizes=((16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)),
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("output", type=Path)
    arguments = parser.parse_args(argv)
    create_icon(arguments.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
