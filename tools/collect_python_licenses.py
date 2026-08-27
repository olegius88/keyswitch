"""Collect licenses for Python components bundled in the Windows runtime."""

from __future__ import annotations

import argparse
import importlib.metadata
import re
import sys
from pathlib import Path


LICENSE_PREFIXES = ("license", "copying", "notice", "authors")


def _safe_name(value: str) -> str:
    return re.sub(r"[^0-9A-Za-z._-]+", "-", value).strip("-")


def collect_distribution(output: Path, name: str) -> int:
    distribution = importlib.metadata.distribution(name)
    files = distribution.files or ()
    candidates = sorted(
        (
            item
            for item in files
            if Path(str(item)).name.casefold().startswith(LICENSE_PREFIXES)
        ),
        key=lambda item: (len(Path(str(item)).parts), str(item).casefold()),
    )
    copied = 0
    seen: set[bytes] = set()
    for item in candidates:
        source = Path(str(distribution.locate_file(item)))
        if not source.is_file():
            continue
        payload = source.read_bytes()
        if payload in seen:
            continue
        seen.add(payload)
        copied += 1
        destination = output / (
            f"LICENSE.{_safe_name(distribution.metadata['Name'] or name)}."
            f"{copied}.{_safe_name(source.name)}"
        )
        destination.write_bytes(payload)
    if copied == 0:
        raise RuntimeError(f"No license file found for {name}")
    return copied


def collect_python_runtime(output: Path) -> int:
    candidates = (
        Path(sys.base_prefix) / "LICENSE.txt",
        Path(sys.base_prefix) / "LICENSE",
        Path(sys.base_prefix) / "Doc" / "license.rst",
        Path(sys.base_prefix) / "tcl" / "tcl8.6" / "license.terms",
        Path(sys.base_prefix) / "tcl" / "tk8.6" / "license.terms",
        Path("/usr/share/doc/python3/copyright"),
    )
    copied = 0
    for source in candidates:
        if not source.is_file():
            continue
        copied += 1
        destination = output / f"LICENSE.Python-runtime.{copied}.{source.name}"
        destination.write_bytes(source.read_bytes())
    return copied


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("output", type=Path)
    parser.add_argument("distributions", nargs="+")
    arguments = parser.parse_args(argv)
    arguments.output.mkdir(parents=True, exist_ok=True)
    for name in arguments.distributions:
        collect_distribution(arguments.output, name)
    if collect_python_runtime(arguments.output) == 0:
        raise RuntimeError("No Python runtime license file found")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
