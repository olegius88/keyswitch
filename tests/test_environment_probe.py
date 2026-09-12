"""The environment probe measures results, so its tests check results.

The probe exists because the seal used to bind to the interpreter's name. Three
properties make it a usable replacement, and each is tested here rather than
argued: it is deterministic, it agrees with itself across a fork, and a change
to one primitive moves exactly the cells that use that primitive. The last is
the one that matters - a probe whose digest moves for everything is no more
informative than the build string it replaced.
"""

from __future__ import annotations

import json
import math
import random
import subprocess
import sys
import tempfile
import unicodedata
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))

import environment_probe as probe  # noqa: E402

CELL_NAMES = tuple(name for name, _ in probe.CELLS)


class ProbeShape(unittest.TestCase):
    def test_every_cell_produces_entries_and_buckets(self) -> None:
        for name in CELL_NAMES:
            with self.subTest(cell=name):
                cell = probe.measure_cell(name)
                self.assertGreater(cell["entries"], 0)
                self.assertTrue(cell["buckets"])
                self.assertEqual(len(cell["sha256"]), 64)

    def test_unicode_cell_is_exhaustive(self) -> None:
        # The runtime normalises every token it sees, so this is the one cell
        # that must not be a sample: one entry per code point, no fewer.
        cell = probe.measure_cell("unicode")
        self.assertGreaterEqual(cell["entries"], 0x110000)
        planes = {name for name in cell["buckets"] if name.startswith("plane-")}
        self.assertEqual(len(planes), 17)

    def test_measure_rejects_an_unknown_cell(self) -> None:
        with self.assertRaises(ValueError):
            probe.measure(["libm", "no_such_cell"])

    def test_combined_digest_covers_every_cell(self) -> None:
        full = probe.measure()
        partial = probe.measure(["libm"])
        self.assertNotEqual(full["probe_sha256"], partial["probe_sha256"])
        self.assertEqual(set(full["cells"]), set(CELL_NAMES))

    def test_probe_operands_are_doubles(self) -> None:
        # An int here would silently probe integer arithmetic instead.
        for value in probe._PROBE_DOUBLES:
            self.assertIs(type(value), float)

    def test_source_carries_no_expected_digest(self) -> None:
        """The probe reports what this machine does; it never grades it.

        A baked-in digest would turn the probe into an assertion about one
        machine, which is precisely the mistake it was written to undo.
        """

        source = (ROOT / "tools/environment_probe.py").read_text(encoding="utf-8")
        measured = probe.measure()
        digests = {measured["probe_sha256"]} | {
            cell["sha256"] for cell in measured["cells"].values()
        }
        for digest in digests:
            self.assertNotIn(digest, source)
            self.assertNotIn(digest[:16], source)


class ProbeDeterminism(unittest.TestCase):
    def test_repeated_measurement_is_identical(self) -> None:
        self.assertEqual(
            probe.measure(["libm", "float_arithmetic", "random_stream"]),
            probe.measure(["libm", "float_arithmetic", "random_stream"]),
        )

    def test_digest_ignores_the_hash_seed(self) -> None:
        """Dict and set iteration must not leak into the measurement."""

        digests = set()
        for seed in ("0", "1", "12345"):
            completed = subprocess.run(
                [sys.executable, str(ROOT / "tools/environment_probe.py"),
                 "--cells", "hashing", "ordering", "integers"],
                capture_output=True, check=True,
                env={"PATH": "/usr/bin:/bin", "PYTHONHASHSEED": seed},
            )
            digests.add(json.loads(completed.stdout)["probe_sha256"])
        self.assertEqual(len(digests), 1)

    def test_a_forked_child_agrees_with_its_parent(self) -> None:
        """Feature extraction runs in forked workers; disagreement there would
        make every parallel phase suspect."""

        cells = ["libm", "float_arithmetic", "hashing"]
        forked = probe._forked_measurement(cells)
        if forked is None:
            self.skipTest("this platform cannot fork")
        self.assertEqual(forked["probe_sha256"], probe.measure(cells)["probe_sha256"])


class ProbeLocalisation(unittest.TestCase):
    """A broken primitive must move its own cells and no others."""

    baseline_digests: dict[str, str]

    @classmethod
    def setUpClass(cls) -> None:
        # Measured once: the exhaustive Unicode walk dominates the runtime, and
        # a baseline that moved between tests would make them meaningless.
        cls.baseline_digests = {
            name: probe.measure_cell(name)["sha256"] for name in CELL_NAMES
        }

    def baseline(self) -> dict[str, str]:
        return dict(self.baseline_digests)

    def moved_cells(self, before: dict[str, str]) -> list[str]:
        return sorted(
            name for name in CELL_NAMES
            if probe.measure_cell(name)["sha256"] != before[name]
        )

    def test_a_one_ulp_sqrt_moves_the_cells_that_call_sqrt(self) -> None:
        before = self.baseline()
        genuine = math.sqrt
        drifted = lambda value: math.nextafter(genuine(value), math.inf)
        with mock.patch.object(math, "sqrt", drifted):
            moved = self.moved_cells(before)
        # The FTRL update is written around sqrt, so both cells must notice.
        self.assertEqual(moved, ["float_arithmetic", "libm"])
        # And the fault must not outlive the patch, or every later test lies.
        self.assertEqual(self.moved_cells(before), [])

    def test_a_one_ulp_exp_moves_only_libm(self) -> None:
        before = self.baseline()
        genuine = math.exp
        drifted = lambda value: math.nextafter(genuine(value), math.inf)
        with mock.patch.object(math, "exp", drifted):
            moved = self.moved_cells(before)
        # The FTRL cell never calls exp, so it must stay put.
        self.assertEqual(moved, ["libm"])

    def test_a_changed_normalisation_moves_only_unicode(self) -> None:
        before = self.baseline()
        genuine = unicodedata.normalize
        swapped = lambda form, text: genuine(
            "NFD" if form == "NFC" else form, text
        )
        with mock.patch.object(unicodedata, "normalize", swapped):
            moved = self.moved_cells(before)
        self.assertEqual(moved, ["unicode"])

    def test_a_changed_shuffle_moves_only_the_random_stream(self) -> None:
        before = self.baseline()

        def reversed_shuffle(self: random.Random, sequence: list[int]) -> None:
            sequence.reverse()

        with mock.patch.object(random.Random, "shuffle", reversed_shuffle):
            moved = self.moved_cells(before)
        self.assertEqual(moved, ["random_stream"])


class ProbeExplain(unittest.TestCase):
    def record(self, cells: list[str]) -> probe.Measurement:
        return probe.measure(cells)

    def test_an_identical_record_reports_no_movement(self) -> None:
        self.assertEqual(probe.explain("libm", self.record(["libm"])), 0)

    def test_a_missing_record_is_not_silently_a_pass(self) -> None:
        self.assertEqual(probe.explain("libm", self.record(["ordering"])), 1)

    def test_a_moved_cell_is_reported_with_its_bucket(self) -> None:
        recorded = self.record(["libm"])
        libm = recorded["cells"]["libm"]
        libm["buckets"]["exp"] = "0" * 64
        libm["sha256"] = "0" * 64
        self.assertEqual(probe.explain("libm", recorded), 1)

    def test_explain_rejects_an_unknown_cell(self) -> None:
        with self.assertRaises(ValueError):
            probe.explain("no_such_cell", None)

    def test_explain_without_a_record_lists_buckets(self) -> None:
        self.assertEqual(probe.explain("ordering", None), 0)


class ProbeUlp(unittest.TestCase):
    def test_neighbouring_doubles_are_one_ulp_apart(self) -> None:
        value = 0.1
        self.assertEqual(probe._ulp_distance(value, math.nextafter(value, math.inf)), 1)

    def test_zero_signs_are_adjacent(self) -> None:
        self.assertEqual(probe._ulp_distance(0.0, -0.0), 1)

    def test_non_finite_has_no_distance(self) -> None:
        self.assertIsNone(probe._ulp_distance(1.0, math.inf))
        self.assertIsNone(probe._ulp_distance(math.nan, 1.0))

    def test_distance_is_symmetric(self) -> None:
        left, right = 1.0, math.nextafter(math.nextafter(1.0, math.inf), math.inf)
        self.assertEqual(probe._ulp_distance(left, right), 2)
        self.assertEqual(probe._ulp_distance(right, left), 2)


class ProbeCommandLine(unittest.TestCase):
    def run_probe(self, *argv: str) -> subprocess.CompletedProcess[bytes]:
        return subprocess.run(
            [sys.executable, str(ROOT / "tools/environment_probe.py"), *argv],
            capture_output=True, env={"PATH": "/usr/bin:/bin"},
        )

    def test_default_run_emits_every_cell(self) -> None:
        completed = self.run_probe("--cells", "ordering", "integers")
        self.assertEqual(completed.returncode, 0)
        payload = json.loads(completed.stdout)
        self.assertEqual(payload["schema_version"], probe.SCHEMA_VERSION)
        self.assertEqual(set(payload["cells"]), {"ordering", "integers"})

    def test_fork_check_is_reported(self) -> None:
        completed = self.run_probe("--cells", "ordering", "--check-fork")
        self.assertEqual(completed.returncode, 0)
        self.assertIn(
            json.loads(completed.stdout)["fork_check"], {"agrees", "unavailable"}
        )

    def test_ulp_subcommand_sizes_a_gap(self) -> None:
        import struct

        left = struct.pack("<d", 1.0).hex()
        right = struct.pack("<d", math.nextafter(1.0, math.inf)).hex()
        completed = self.run_probe("--ulp", left, right)
        self.assertEqual(completed.returncode, 0)
        self.assertIn("1 ulp", completed.stdout.decode())

    def test_explain_against_a_file(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            record = Path(directory) / "environment.json"
            record.write_text(
                json.dumps(probe.measure(["ordering"])), encoding="utf-8"
            )
            completed = self.run_probe(
                "--explain", "ordering", "--against", str(record)
            )
            self.assertEqual(completed.returncode, 0)
            self.assertIn("did not move", completed.stdout.decode())

    def test_explain_accepts_a_wrapped_record(self) -> None:
        """The sidecar nests the measurement under `environment_probe`."""

        with tempfile.TemporaryDirectory() as directory:
            record = Path(directory) / "build-environment.json"
            record.write_text(
                json.dumps({"environment_probe": probe.measure(["ordering"])}),
                encoding="utf-8",
            )
            completed = self.run_probe(
                "--explain", "ordering", "--against", str(record)
            )
            self.assertEqual(completed.returncode, 0)
            self.assertIn("did not move", completed.stdout.decode())


if __name__ == "__main__":
    unittest.main()
