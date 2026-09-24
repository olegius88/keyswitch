"""Win32 API numbers and Windows-specific limits."""

from __future__ import annotations

from typing import Final

WINDOWS_MAX_PATH_CHARACTERS: Final = 260
MB_ICONERROR: Final = 0x10
# DOS header offset of e_lfanew, the pointer to the PE signature.
DOS_HEADER_PE_OFFSET_FIELD: Final = 0x3C
PE_SIGNATURE_BYTES: Final = 4
COFF_FILE_HEADER_BYTES: Final = 20
# Byte offset of the Subsystem field within the optional header.
OPTIONAL_HEADER_SUBSYSTEM_OFFSET: Final = 68
WINDOWS_SUBSYSTEM_GUI: Final = 2
WINDOWS_SUBSYSTEM_CUI: Final = 3
# Where the optional header starts relative to the PE signature.
PE_SIGNATURE_AND_COFF_HEADER_BYTES: Final = PE_SIGNATURE_BYTES + COFF_FILE_HEADER_BYTES
