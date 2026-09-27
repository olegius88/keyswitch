"""Optional Hunspell integration used as morphological dictionary evidence.

The application talks to the stable Hunspell C API through ``ctypes``.  This
keeps the runtime small and avoids a Python extension dependency while still
recognising inflected Russian and English forms from the system dictionaries.
"""

from __future__ import annotations

import codecs
import ctypes
import ctypes.util
import os
import threading
from pathlib import Path

from .history import data_dir


DEFAULT_DICTIONARY_ROOTS = (
    Path("/usr/share/hunspell"),
    Path("/usr/share/myspell/dicts"),
    Path("/usr/local/share/hunspell"),
)


class HunspellDictionary:
    """Small, thread-safe owner of one Hunspell dictionary handle."""

    _library: ctypes.CDLL | None = None
    _library_attempted = False
    _library_lock = threading.Lock()

    def __init__(self, locale: str) -> None:
        self.locale = locale
        self.source = ""
        self._handle: int | None = None
        self._encoding = "utf-8"
        self._lock = threading.Lock()
        library = self._load_library()
        dictionary = self._find_dictionary(locale)
        if library is None or dictionary is None:
            return
        affix_path, dictionary_path = dictionary
        handle = library.Hunspell_create(
            os.fsencode(affix_path),
            os.fsencode(dictionary_path),
        )
        if not handle:
            return
        self._handle = handle
        self.source = str(dictionary_path)
        encoding = library.Hunspell_get_dic_encoding(handle)
        if encoding:
            candidate = encoding.decode("ascii", "replace")
            try:
                codecs.lookup(candidate)
                self._encoding = candidate
            except LookupError:
                self._encoding = "utf-8"

    @property
    def available(self) -> bool:
        return self._handle is not None

    def check(self, word: str) -> bool:
        normalized = word.strip()
        if not self._handle or not normalized or len(normalized) > 128:
            return False
        try:
            encoded = normalized.encode(self._encoding)
        except UnicodeEncodeError:
            return False
        library = self.__class__._library
        if library is None:
            return False
        with self._lock:
            return bool(library.Hunspell_spell(self._handle, encoded))

    def close(self) -> None:
        handle, self._handle = self._handle, None
        library = self.__class__._library
        if handle and library is not None:
            library.Hunspell_destroy(handle)

    def __del__(self) -> None:
        self.close()

    @classmethod
    def _load_library(cls) -> ctypes.CDLL | None:
        with cls._library_lock:
            if cls._library_attempted:
                return cls._library
            cls._library_attempted = True
            name = ctypes.util.find_library("hunspell-1.7") or ctypes.util.find_library(
                "hunspell"
            )
            if not name:
                return None
            try:
                library = ctypes.CDLL(name)
                library.Hunspell_create.argtypes = [ctypes.c_char_p, ctypes.c_char_p]
                library.Hunspell_create.restype = ctypes.c_void_p
                library.Hunspell_destroy.argtypes = [ctypes.c_void_p]
                library.Hunspell_destroy.restype = None
                library.Hunspell_spell.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
                library.Hunspell_spell.restype = ctypes.c_int
                library.Hunspell_get_dic_encoding.argtypes = [ctypes.c_void_p]
                library.Hunspell_get_dic_encoding.restype = ctypes.c_char_p
            except (AttributeError, OSError):
                return None
            cls._library = library
            return library

    @staticmethod
    def _dictionary_roots() -> tuple[Path, ...]:
        override = os.environ.get("KEYSWITCH_HUNSPELL_PATH", "")
        custom = tuple(Path(item) for item in override.split(os.pathsep) if item)
        xdg_data = os.environ.get("XDG_DATA_HOME")
        xdg_root = Path(xdg_data) if xdg_data else Path.home() / ".local" / "share"
        user_roots = (data_dir() / "dictionaries", xdg_root / "hunspell")
        return custom + user_roots + DEFAULT_DICTIONARY_ROOTS

    @classmethod
    def _find_dictionary(cls, locale: str) -> tuple[Path, Path] | None:
        variants = (locale, locale.replace("-", "_"), locale.split("_", 1)[0])
        for root in cls._dictionary_roots():
            for variant in dict.fromkeys(variants):
                affix = root / f"{variant}.aff"
                dictionary = root / f"{variant}.dic"
                if affix.is_file() and dictionary.is_file():
                    return affix, dictionary
        return None
