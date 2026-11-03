"""Packed gram tables answer what the dicts answered, bit for bit, and leave tables packing does not fit alone."""

from __future__ import annotations

import struct
import sys
import unittest
from pathlib import Path
from types import SimpleNamespace
from typing import cast
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))

from keyswitch.constants.packing import PACKED_GRAM_MAX_CHARACTERS, PACKED_GRAM_MIN_ENTRIES  # noqa: E402
from keyswitch.memory_release import release_free_memory  # noqa: E402
from keyswitch.ortho_model import OrthoModel, _Channel  # noqa: E402
from keyswitch.packed_grams import PackedGrams, pack_channels  # noqa: E402

sys.path.insert(0, str(ROOT / "tests"))
from fixture_values.ortho import (  # noqa: E402
    PACKED_FIXTURE_BACKOFF,
    PACKED_FIXTURE_LETTERS,
    PACKED_FIXTURE_ORDER,
    PACKED_FIXTURE_WIDE_FIRST_CODE,
)


def bits(value: float | None) -> bytes | None:
    return None if value is None else struct.pack("<d", value)


class PackedGramsTests(unittest.TestCase):
    def test_every_gram_of_the_shipped_model_answers_the_same_float(self) -> None:
        model, _status = OrthoModel.try_load()
        assert model is not None
        channels = [*model.channels.values(), *model.source_channels.values()]
        tables = [(dict(channel.logprob), dict(channel.backoff)) for channel in channels]
        pack_channels(model)
        for channel, (logprob, backoff) in zip(channels, tables, strict=True):
            for table, packed in ((logprob, channel.logprob), (backoff, channel.backoff)):
                self.assertIsInstance(packed, PackedGrams)
                self.assertEqual(len(packed), len(table))
                self.assertEqual(set(packed), set(table))
                self.assertTrue(all(bits(packed.get(gram)) == bits(value) for gram, value in table.items()))
                self.assertEqual(bits(packed[next(iter(table))]), bits(table[next(iter(table))]))
        words = ("ghbdtn", "hello", "rkfdbfnehf", "z", "jbdcbnbt", "щука", "naïve")
        fresh, _status = OrthoModel.try_load()
        assert fresh is not None
        for script in fresh.channels:
            for word in words:
                self.assertEqual(bits(model.channels[script].score(word)), bits(fresh.channels[script].score(word)))
        pack_channels(None)

    def test_a_gram_the_table_never_holds_is_absent(self) -> None:
        table = {f"{chr(ord('a') + index % PACKED_FIXTURE_LETTERS)}{index}": float(index) for index in range(PACKED_GRAM_MIN_ENTRIES)}
        packed = PackedGrams.pack(table)
        assert packed is not None
        for gram in ("", "zz", "à", "中", "a\u0000", "a" * (PACKED_GRAM_MAX_CHARACTERS + 1)):
            self.assertIsNone(packed.get(gram))
            self.assertEqual(packed.get(gram, 0.0), 0.0)
        with self.assertRaises(KeyError):
            packed["zz"]

    def test_a_table_packing_does_not_fit_stays_as_it_is(self) -> None:
        small = {"ab": 1.0}
        self.assertIsNone(PackedGrams.pack(small))
        many = {f"g{index}": float(index) for index in range(PACKED_GRAM_MIN_ENTRIES)}
        self.assertIsNone(PackedGrams.pack({**many, "x" * (PACKED_GRAM_MAX_CHARACTERS + 1): 0.0}))
        self.assertIsNone(PackedGrams.pack({**many, "": 0.0}))
        # A zero character would read as the padding of a shorter gram.
        self.assertIsNone(PackedGrams.pack({**many, "a\x00": 0.0}))
        wide = {chr(PACKED_FIXTURE_WIDE_FIRST_CODE + index): float(index) for index in range(PACKED_GRAM_MIN_ENTRIES)}
        self.assertIsNone(PackedGrams.pack(wide))

    def test_a_channel_whose_tables_do_not_pack_keeps_its_dicts(self) -> None:
        channel = _Channel(PACKED_FIXTURE_ORDER, {"ab": 1.0}, {"a": PACKED_FIXTURE_BACKOFF}, 0.0)
        pack_channels(cast(OrthoModel, SimpleNamespace(channels={"en": channel}, source_channels={"ru": channel})))
        self.assertEqual((channel.logprob, channel.backoff), ({"ab": 1.0}, {"a": PACKED_FIXTURE_BACKOFF}))


class MemoryReleaseTests(unittest.TestCase):
    def test_only_glibc_on_linux_is_asked(self) -> None:
        with patch("keyswitch.memory_release.sys.platform", "win32"):
            self.assertFalse(release_free_memory())
        with patch("keyswitch.memory_release.ctypes.CDLL", side_effect=OSError("no libc")):
            self.assertFalse(release_free_memory())
        with patch("keyswitch.memory_release.sys.platform", "linux"):
            self.assertTrue(release_free_memory())


if __name__ == "__main__":
    unittest.main()
