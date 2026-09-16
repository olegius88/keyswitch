"""Minimal stub for the part of :mod:`compression.zstd` this repository uses.

Python 3.14 added the module (PEP 784); the project type-checks against 3.10, whose
typeshed does not describe it. Only the file object is declared, with the two modes
and the three methods the holdout tooling needs.
"""

from os import PathLike
from types import TracebackType
from typing import Literal

class ZstdFile:
    def __init__(
        self,
        file: str | bytes | PathLike[str] | PathLike[bytes],
        mode: Literal["rb", "wb"] = "rb",
    ) -> None: ...
    def read(self, size: int = -1) -> bytes: ...
    def write(self, data: bytes) -> int: ...
    def close(self) -> None: ...
    def readable(self) -> bool: ...
    def readinto(self, buffer: bytearray | memoryview) -> int: ...
    def __enter__(self) -> ZstdFile: ...
    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc_value: BaseException | None,
        traceback: TracebackType | None,
    ) -> None: ...
