"""The orthotactic model's gram tables packed into two arrays: the same answers in a small part of the memory.

`OrthoModel.load` keeps each channel's log probabilities and backoff weights as dicts of strings to floats: about a
million grams of up to five characters, 101 MB of objects and 189 MB resident after loading (03.11.2026), from a file
of 14 MB. Packed, a gram is its characters translated to one byte each and read as one integer; the sorted integers
and their values sit in two arrays and a lookup is a binary search. The values are the very floats the dict held, so
every score is the same bit for bit. `ortho_model.py` stays as it is (the ortho-v1 seal pins its bytes): its
`_Channel` asks a table for `.get(gram)` and `.get(context, 0.0)` alone, and the packed table answers the same.
"""

from __future__ import annotations

import struct
from array import array
from bisect import bisect_left
from collections.abc import Iterator, Mapping

from .constants.packing import (
    PACKED_GRAM_BITS_PER_CODE,
    PACKED_GRAM_KEY_BYTES,
    PACKED_GRAM_LAST_CODE,
    PACKED_GRAM_MAX_CHARACTERS,
    PACKED_GRAM_MIN_ENTRIES,
)
from .ortho_model import OrthoModel

_ABSENT = "\x00"


class PackedGrams(Mapping[str, float]):
    """A read-only table of grams to floats, keyed by the grams' characters packed into integers."""

    def __init__(self, alphabet: str, keys: array[int], values: array[float]) -> None:
        # Every character the table holds has a code from 1; every other Latin-1 character the zero code, and any
        # other character fails to encode, so a gram with a character the table never holds is no key of it.
        self._encode = {ord(character): chr(code) for code, character in enumerate(alphabet, 1)}
        self._encode.update({code: _ABSENT for code in range(PACKED_GRAM_LAST_CODE + 1)
                             if chr(code) not in alphabet})
        self._decode = {code: character for code, character in enumerate(alphabet, 1)}
        self._keys = keys
        self._values = values

    @classmethod
    def pack(cls, table: Mapping[str, float]) -> PackedGrams | None:
        """The table packed, or None where packing does not fit (too many characters, too long or an empty gram).

        A million grams are packed at start-up, so the work is done on one buffer rather than gram by gram: each gram
        padded with zero characters to the key's width, the whole buffer translated and encoded at once and read as
        big-endian 64-bit integers, which are the keys `_key` gives, since a gram's padding is the key's leading zero
        bytes. A zero character in a gram would read as padding: such a table is not packed.
        """

        alphabet = "".join(sorted(set("".join(table))))
        if (len(table) < PACKED_GRAM_MIN_ENTRIES or len(alphabet) > PACKED_GRAM_LAST_CODE or _ABSENT in alphabet
                or max(map(len, table)) > PACKED_GRAM_MAX_CHARACTERS or not min(map(len, table))):
            return None
        packed = cls(alphabet, array("q"), array("d"))
        buffer = "".join([gram.rjust(PACKED_GRAM_KEY_BYTES, _ABSENT) for gram in table])
        keys = struct.unpack(f">{len(table)}q", buffer.translate(packed._encode).encode("latin-1"))
        values = array("d", table.values())
        order = sorted(range(len(keys)), key=keys.__getitem__)
        packed._keys.extend(map(keys.__getitem__, order))
        packed._values.extend(map(values.__getitem__, order))
        return packed

    def _key(self, gram: str) -> int | None:
        try:
            data = gram.translate(self._encode).encode("latin-1")
        except UnicodeEncodeError:
            return None
        if len(data) > PACKED_GRAM_MAX_CHARACTERS or _ABSENT.encode("latin-1") in data:
            return None
        return int.from_bytes(data, "big")

    def get(self, gram: str, default: float | None = None) -> float | None:  # type: ignore[override]
        key = self._key(gram)
        if key is None or not key:
            return default
        index = bisect_left(self._keys, key)
        if index < len(self._keys) and self._keys[index] == key:
            return self._values[index]
        return default

    def __getitem__(self, gram: str) -> float:
        found = self.get(gram)
        if found is None:
            raise KeyError(gram)
        return found

    def __iter__(self) -> Iterator[str]:
        for key in self._keys:
            length = -(-key.bit_length() // PACKED_GRAM_BITS_PER_CODE)
            yield key.to_bytes(length, "big").decode("latin-1").translate(self._decode)

    def __len__(self) -> int:
        return len(self._keys)


def pack_channels(model: OrthoModel | None) -> None:
    """Pack the gram tables of every channel of `model` in place; a table packing does not fit stays a dict."""

    if model is None:
        return
    for channel in {id(channel): channel for channel in (*model.channels.values(), *model.source_channels.values())}.values():
        for name in ("logprob", "backoff"):
            packed = PackedGrams.pack(getattr(channel, name))
            if packed is not None:
                setattr(channel, name, packed)
