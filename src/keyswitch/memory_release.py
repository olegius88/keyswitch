"""Handing the C heap's free pages back to the system once the models are built."""

from __future__ import annotations

import ctypes
import sys


def _on_linux() -> bool:
    # A call, not a constant: strict mypy for another platform would read the rest of the function as unreachable.
    return sys.platform.startswith("linux")


def release_free_memory() -> bool:
    """Ask glibc to return the free pages of its heap (malloc_trim); whether it was asked.

    Building the engine parses about 50 MB of model files into objects and drops most of the temporary ones, and
    later the gram tables the packed tables replace (packed_grams). The C library keeps the freed pages for the process:
    after the orthotactic model was packed, the process held 99 MB where malloc_trim left 54 MB (03.11.2026). Other C
    libraries (musl, Windows, macOS) have no such call, and the process keeps what they keep.
    """

    if not _on_linux():
        return False
    try:
        trim = ctypes.CDLL(None).malloc_trim
    except (OSError, AttributeError):
        return False
    trim.argtypes = [ctypes.c_size_t]
    trim.restype = ctypes.c_int
    trim(0)
    return True
