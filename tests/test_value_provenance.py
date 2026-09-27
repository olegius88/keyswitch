"""Pinning the values a toolchain imports from keyswitch.constants, not the files that hold them."""

from __future__ import annotations

import enum
import json
import math
import os
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path
from typing import NamedTuple
from unittest import mock

import keyswitch.constants.units as real_units
from keyswitch.value_provenance import (
    CONSTANTS_PACKAGE,
    NonCanonicalValue,
    ValueProvenanceError,
    canonical_json,
    canonical_value,
    changed_values,
    pin_values,
    resolve_values,
    used_constants,
    values_sha256,
)

PROJECT_ROOT = Path(__file__).resolve().parents[1]

# Enough distinct strings that two hash seeds iterate a frozenset of them differently.
SEED_SENSITIVE_NAMES = tuple(f"name-{letter}" for letter in "abcdefghijklmnopqrstuvwxyz")

UNITS_SOURCE = '''
    """Units of a fixture tree."""

    from __future__ import annotations

    from typing import Final

    BYTES_PER_KIB: Final = 1024
'''

SAMPLE_SOURCE = f'''
    """Values of a fixture tree."""

    from __future__ import annotations

    from typing import Final

    from .units import BYTES_PER_KIB

    LIMIT: Final = 4 * BYTES_PER_KIB
    RATIO: Final = 0.1
    NAMES: Final = frozenset({SEED_SENSITIVE_NAMES!r})
    TABLE: Final = {{"b": (1, 2.5), "a": [None, True]}}
    UNUSED: Final = 7
'''

DERIVED_SOURCE = '''
    """A constants module that reads its siblings in both relative forms."""

    from __future__ import annotations

    from typing import Final

    from . import units
    from .sample import RATIO

    HALF_RATIO: Final = RATIO / 2
    KIB: Final = units.BYTES_PER_KIB
'''


class Tree:
    """A throwaway checkout: `src/keyswitch/constants/` plus files that read it."""

    def __init__(self, root: Path) -> None:
        self.root = root
        self.source_root = root / "src"
        self.constants = self.source_root / "keyswitch" / "constants"
        self.write("src/keyswitch/__init__.py", "")
        self.write("src/keyswitch/constants/__init__.py", '"""Fixture constants."""\n')
        self.write("src/keyswitch/constants/units.py", UNITS_SOURCE)
        self.write("src/keyswitch/constants/sample.py", SAMPLE_SOURCE)

    def write(self, relative: str, source: str) -> Path:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(textwrap.dedent(source).lstrip(), "utf-8")
        return path

    def replace(self, relative: str, old: str, new: str) -> None:
        path = self.root / relative
        text = path.read_text("utf-8")
        if old not in text:
            raise AssertionError(f"{old!r} is not in {relative}")
        path.write_text(text.replace(old, new), "utf-8")

    def used(self, *paths: Path) -> frozenset[str]:
        return used_constants(paths, source_root=self.source_root)

    def pin(self, *paths: Path) -> tuple[str, dict[str, object]]:
        pinned = pin_values(paths, source_root=self.source_root)
        return pinned.sha256, dict(pinned.values)


def qualified(module: str, name: str) -> str:
    return f"{CONSTANTS_PACKAGE}.{module}.{name}"


class TreeTestCase(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.tree = Tree(Path(directory.name))


class ImportFormsTests(TreeTestCase):
    def test_absolute_imports_and_bound_modules_are_followed(self) -> None:
        tool = self.tree.write(
            "tools/tool.py",
            '''
            import os
            import keyswitch.constants.units
            import keyswitch.constants.sample as sample_module
            from os import path
            from keyswitch import constants
            from keyswitch.constants import sample
            from keyswitch.constants.sample import LIMIT as limit

            print(os, path, limit, keyswitch.__doc__, len("x").real)
            print(keyswitch.constants.units.BYTES_PER_KIB, sample_module.RATIO)
            print(sample.NAMES.union(()), constants.sample.TABLE["a"], unknown.attribute)
            ''',
        )
        self.assertEqual(
            self.tree.used(tool),
            {
                qualified("sample", "LIMIT"),
                qualified("sample", "NAMES"),
                qualified("sample", "RATIO"),
                qualified("sample", "TABLE"),
                qualified("units", "BYTES_PER_KIB"),
            },
        )

    def test_relative_imports_resolve_against_the_file_package(self) -> None:
        inside = self.tree.write(
            "src/keyswitch/user.py",
            '''
            from . import constants as named
            from .constants.sample import TABLE

            print(TABLE, named.sample.LIMIT)
            ''',
        )
        derived = self.tree.write("src/keyswitch/constants/derived.py", DERIVED_SOURCE)
        self.assertEqual(
            self.tree.used(inside, derived),
            {
                qualified("sample", "TABLE"),
                qualified("sample", "LIMIT"),
                qualified("sample", "RATIO"),
                qualified("units", "BYTES_PER_KIB"),
            },
        )

    def test_forms_that_hide_the_names_are_refused(self) -> None:
        refused = {
            "star from a module": "from keyswitch.constants.sample import *\n",
            "star from the package": "from keyswitch.constants import *\n",
            "module object": "from keyswitch.constants import sample\nprint(sample)\n",
            "package object": "import keyswitch.constants\nprint(keyswitch.constants)\n",
        }
        for label, source in refused.items():
            with self.subTest(form=label):
                path = self.tree.write("tools/refused.py", source)
                with self.assertRaisesRegex(ValueProvenanceError, "refused.py"):
                    self.tree.used(path)

    def test_relative_import_needs_a_package(self) -> None:
        for relative in ("tools/relative.py", "src/relative.py", "src/keyswitch/beyond.py"):
            with self.subTest(file=relative):
                source = "from ..outside import NAME\n" if "beyond" in relative else "from .x import NAME\n"
                path = self.tree.write(relative, source)
                with self.assertRaisesRegex(ValueProvenanceError, Path(relative).name):
                    self.tree.used(path)

    def test_unreadable_or_unparsable_files_are_refused(self) -> None:
        with self.assertRaisesRegex(ValueProvenanceError, "is unreadable"):
            self.tree.used(self.tree.root / "tools/absent.py")
        broken = self.tree.write("tools/broken.py", "def (:\n")
        with self.assertRaisesRegex(ValueProvenanceError, "does not parse"):
            self.tree.used(broken)


class DigestTests(TreeTestCase):
    def reader(self) -> Path:
        return self.tree.write(
            "tools/reader.py",
            "from keyswitch.constants.sample import LIMIT, NAMES, RATIO, TABLE\n",
        )

    def test_values_are_resolved_and_derived_constants_follow_their_inputs(self) -> None:
        digest, values = self.tree.pin(self.reader())
        self.assertEqual(values[qualified("sample", "LIMIT")], ["int", "4096"])
        self.assertEqual(values[qualified("sample", "RATIO")], ["float", "0.1"])
        self.assertEqual(
            values[qualified("sample", "NAMES")],
            ["frozenset", [["str", name] for name in sorted(SEED_SENSITIVE_NAMES)]],
        )
        self.assertEqual(digest, values_sha256(values))
        self.tree.replace("src/keyswitch/constants/units.py", "= 1024", "= 1000")
        changed_digest, changed = self.tree.pin(self.reader())
        self.assertNotEqual(changed_digest, digest)
        self.assertEqual(changed_values(values, changed), (qualified("sample", "LIMIT"),))

    def test_a_changed_used_value_changes_the_digest(self) -> None:
        digest, values = self.tree.pin(self.reader())
        self.tree.replace("src/keyswitch/constants/sample.py", "RATIO: Final = 0.1", "RATIO: Final = 0.2")
        changed_digest, changed = self.tree.pin(self.reader())
        self.assertNotEqual(changed_digest, digest)
        self.assertEqual(changed_values(values, changed), (qualified("sample", "RATIO"),))

    def test_a_changed_unused_value_in_the_same_module_does_not(self) -> None:
        digest, _values = self.tree.pin(self.reader())
        self.tree.replace("src/keyswitch/constants/sample.py", "UNUSED: Final = 7", "UNUSED: Final = 8")
        self.assertEqual(self.tree.pin(self.reader())[0], digest)

    def test_file_order_and_repetition_do_not_matter(self) -> None:
        other = self.tree.write("tools/other.py", "from keyswitch.constants.units import BYTES_PER_KIB\n")
        reader = self.reader()
        self.assertEqual(self.tree.pin(reader, other), self.tree.pin(other, reader, other))

    def test_the_digest_does_not_depend_on_the_hash_seed(self) -> None:
        reader = self.reader()
        program = textwrap.dedent(
            """
            import json, sys
            from pathlib import Path
            from keyswitch.value_provenance import pin_values
            pinned = pin_values([Path(sys.argv[1])], source_root=Path(sys.argv[2]))
            print(json.dumps([pinned.sha256, list(frozenset(json.loads(sys.argv[3])))]))
            """
        )
        digests: list[str] = []
        orders: list[list[str]] = []
        for seed in ("0", "1"):
            completed = subprocess.run(
                [
                    sys.executable,
                    "-c",
                    program,
                    str(reader),
                    str(self.tree.source_root),
                    json.dumps(SEED_SENSITIVE_NAMES),
                ],
                capture_output=True,
                text=True,
                check=True,
                env={**os.environ, "PYTHONHASHSEED": seed, "PYTHONPATH": str(PROJECT_ROOT / "src")},
            )
            digest, order = json.loads(completed.stdout)
            digests.append(digest)
            orders.append(order)
        self.assertNotEqual(orders[0], orders[1], "the seeds must iterate the set differently")
        self.assertEqual(digests, [self.tree.pin(reader)[0]] * len(digests))

    def test_changed_values_reports_added_removed_and_changed_names(self) -> None:
        self.assertEqual(
            changed_values({"a": ["int", "1"], "b": ["int", "1"]}, {"b": ["int", "0"], "c": ["int", "1"]}),
            ("a", "b", "c"),
        )
        self.assertEqual(changed_values({"a": ["int", "1"]}, {"a": ["int", "1"]}), ())


class ResolutionTests(TreeTestCase):
    def test_constants_modules_import_siblings_both_ways_and_stay_isolated(self) -> None:
        derived = self.tree.write("src/keyswitch/constants/derived.py", DERIVED_SOURCE)
        reader = self.tree.write(
            "tools/reader.py",
            "from keyswitch.constants.derived import HALF_RATIO, KIB\nfrom keyswitch.constants.units import BYTES_PER_KIB\n",
        )
        _digest, values = self.tree.pin(reader, derived)
        self.assertEqual(values[qualified("derived", "KIB")], ["int", "1024"])
        self.assertEqual(values[qualified("derived", "HALF_RATIO")], ["float", "0.05"])
        self.assertNotIn("keyswitch.constants.derived", sys.modules)

    def test_missing_modules_and_names_are_refused(self) -> None:
        cases = {
            "from keyswitch.constants.absent import NAME\n": "has no source file",
            "from keyswitch.constants.sample import ABSENT\n": "is not defined",
        }
        for source, message in cases.items():
            with self.subTest(source=source):
                path = self.tree.write("tools/missing.py", source)
                with self.assertRaisesRegex(ValueProvenanceError, message):
                    self.tree.pin(path)
        for name in (f"{CONSTANTS_PACKAGE}.sample.inner.NAME", "keyswitch.other.NAME", f"{CONSTANTS_PACKAGE}.NAME"):
            with self.subTest(name=name):
                with self.assertRaises(ValueProvenanceError):
                    resolve_values([name], source_root=self.tree.source_root)

    def test_constants_modules_that_escape_the_tree_or_fail_are_refused(self) -> None:
        cases = {
            "from keyswitch.constants.units import BYTES_PER_KIB\nVALUE = 1\n": "by its absolute name",
            "from ..outside import NAME\nVALUE = 1\n": "only its sibling constants modules",
            "from .absent import NAME\nVALUE = 1\n": "has no source file",
            "from .units import ABSENT\nVALUE = 1\n": "failed to execute",
            "VALUE = 1 / 0\n": "failed to execute",
        }
        reader = self.tree.write("tools/reader.py", "from keyswitch.constants.escaping import VALUE\n")
        for source, message in cases.items():
            with self.subTest(source=source):
                self.tree.write("src/keyswitch/constants/escaping.py", source)
                with self.assertRaisesRegex(ValueProvenanceError, message):
                    self.tree.pin(reader)

    def test_a_non_canonical_constant_cannot_slip_in(self) -> None:
        self.tree.write(
            "src/keyswitch/constants/paths.py",
            "from pathlib import Path\nLOCATION = Path('x')\n",
        )
        reader = self.tree.write("tools/reader.py", "from keyswitch.constants.paths import LOCATION\n")
        with self.assertRaisesRegex(NonCanonicalValue, "LOCATION: a pathlib"):
            self.tree.pin(reader)


class ImportedValueAgreementTests(unittest.TestCase):
    """The pinned value must be the one this process uses when it imported the same file."""

    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.reader = Path(directory.name) / "reader.py"
        self.reader.write_text("from keyswitch.constants.units import BYTES_PER_MEBIBYTE\n", "utf-8")

    def pin(self) -> dict[str, object]:
        return dict(pin_values([self.reader], source_root=PROJECT_ROOT / "src").values)

    def test_the_imported_module_agrees_with_its_file(self) -> None:
        self.assertEqual(
            self.pin(),
            {qualified("units", "BYTES_PER_MEBIBYTE"): ["int", repr(real_units.BYTES_PER_MEBIBYTE)]},
        )

    def test_an_imported_value_that_left_its_file_is_refused(self) -> None:
        with mock.patch.object(real_units, "BYTES_PER_MEBIBYTE", real_units.BYTES_PER_MEBIBYTE + 1):
            with self.assertRaisesRegex(ValueProvenanceError, "differs from .*units.py"):
                self.pin()

    def test_another_tree_is_not_compared_with_this_process(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            tree = Tree(Path(directory))
            tree.write(
                "src/keyswitch/constants/units.py",
                "BYTES_PER_MEBIBYTE = 1\n",
            )
            pinned = pin_values([self.reader], source_root=tree.source_root)
        self.assertEqual(dict(pinned.values), {qualified("units", "BYTES_PER_MEBIBYTE"): ["int", "1"]})


class Colour(enum.IntEnum):
    RED = 1


class Point(NamedTuple):
    x: int


class Label(str):
    pass


class CanonicalFormTests(unittest.TestCase):
    def test_scalars_carry_their_exact_type_and_repr(self) -> None:
        self.assertEqual(canonical_value(None, "v"), ["NoneType", "None"])
        self.assertEqual(canonical_value(True, "v"), ["bool", "True"])
        self.assertEqual(canonical_value(1, "v"), ["int", "1"])
        self.assertEqual(canonical_value(1.0, "v"), ["float", "1.0"])
        self.assertNotEqual(canonical_value(0.0, "v"), canonical_value(-0.0, "v"))
        self.assertEqual(canonical_value(math.inf, "v"), ["float", "inf"])
        self.assertNotEqual(canonical_value(True, "v"), canonical_value(1, "v"))

    def test_text_is_kept_verbatim_and_written_as_ascii_json(self) -> None:
        self.assertEqual(canonical_value("ё͸", "v"), ["str", "ё͸"])
        self.assertEqual(canonical_json(canonical_value("ё", "v")), b'["str","\\u0451"]')

    def test_containers_keep_their_kind_and_order(self) -> None:
        self.assertNotEqual(canonical_value((1,), "v"), canonical_value([1], "v"))
        self.assertNotEqual(canonical_value({"a": 1, "b": 0}, "v"), canonical_value({"b": 0, "a": 1}, "v"))
        self.assertEqual(
            canonical_value({"a": (None,)}, "v"),
            ["dict", [[["str", "a"], ["tuple", [["NoneType", "None"]]]]]],
        )
        self.assertEqual(canonical_value({"b", "a"}, "v"), ["set", [["str", "a"], ["str", "b"]]])
        self.assertNotEqual(canonical_value({"a"}, "v"), canonical_value(frozenset({"a"}), "v"))

    def test_values_without_a_portable_form_are_refused(self) -> None:
        for value in (Path("x"), Colour.RED, Point(0), Label("x"), b"x", object(), (1, object())):
            with self.subTest(value=value):
                with self.assertRaises(NonCanonicalValue):
                    canonical_value(value, "v")

    def test_floats_need_the_shortest_round_trip_repr(self) -> None:
        with mock.patch.object(sys, "float_repr_style", "legacy"):
            with self.assertRaisesRegex(ValueProvenanceError, "shortest"):
                canonical_value(1.0, "v")
            self.assertEqual(canonical_value(1, "v"), ["int", "1"])


if __name__ == "__main__":
    unittest.main()
