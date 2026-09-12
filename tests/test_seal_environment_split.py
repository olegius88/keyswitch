"""The v21 boundary: code and data are identity, the machine is provenance.

Until v21 the sealed candidate hash covered seven strings naming the build
machine, so an `apt upgrade` that rebuilt Python without changing a single
computed number was refused as a changed candidate. These tests hold the new
arrangement in place from both sides: the environment must stay out of the
identity, and the sealed test must still be answerable exactly once now that it
no longer relies on the environment to keep the count.
"""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from collections.abc import Mapping
from dataclasses import asdict, fields
from pathlib import Path
from typing import cast

PROJECT_ROOT = Path(__file__).resolve().parents[1]
for candidate in (
    PROJECT_ROOT / "tools",
    PROJECT_ROOT / "src",
    Path(__file__).resolve().parent,
):
    if str(candidate) not in sys.path:
        sys.path.insert(0, str(candidate))

import environment_probe  # noqa: E402
import evaluate_intent_model as evaluator  # noqa: E402
import train_intent_model as trainer  # noqa: E402

from test_intent_training import config  # noqa: E402

# Names that describe a machine rather than an input. None of them may appear
# in anything that reaches sealed_candidate_sha256.
ENVIRONMENT_FIELD_NAMES = frozenset(
    {
        "python_implementation",
        "python_version",
        "python_build",
        "system",
        "machine",
        "libc",
        "byteorder",
        "compiler",
        "ftrl_kernel",
    }
)


def outcome_manifest(**changes: object) -> dict[str, object]:
    payload: dict[str, object] = {
        "sealed_test": {"passed": True, "per_trigger": {"space": 0.99}},
        "sealed_test_typos": {"passed": True, "per_trigger": {}},
        "sealed_test_context_stress": {"passed": True},
        "safety": {"collisions": 0},
        "veto": {"threshold": -1.25},
        "unrelated": {"not": "part of the sealed answer"},
    }
    payload.update(changes)
    return payload


class IdentityExcludesTheMachine(unittest.TestCase):
    def test_snapshot_names_no_environment(self) -> None:
        names = {field.name for field in fields(trainer.TrainingToolchainSnapshot)}
        self.assertEqual(names & ENVIRONMENT_FIELD_NAMES, set())
        self.assertEqual(
            names,
            {"config_sha256"}
            | {field for field, _ in evaluator._TOOLCHAIN_CODE_PATHS},
        )

    def test_snapshot_is_code_and_config_only(self) -> None:
        snapshot = trainer.capture_toolchain_snapshot("0" * 64)
        for name, value in asdict(snapshot).items():
            with self.subTest(field=name):
                self.assertRegex(value, r"^[0-9a-f]{64}$", name)

    def test_the_probe_file_is_certified(self) -> None:
        """The readings stay out of the identity; the file does not.

        A probe that could be weakened without anyone noticing would be worse
        than no probe: it would look like evidence while measuring nothing.
        """

        self.assertIn(
            "environment_probe_sha256",
            {field for field, _ in evaluator._TOOLCHAIN_CODE_PATHS},
        )
        snapshot = trainer.capture_toolchain_snapshot("0" * 64)
        self.assertEqual(
            snapshot.environment_probe_sha256,
            trainer.sha256_file(trainer.ENVIRONMENT_PROBE_PATH),
        )

    def test_a_manifest_that_smuggles_the_machine_back_is_rejected(self) -> None:
        """The invariant has to be machine-checked, not merely intended.

        "It is only provenance, let us record the libc version here" is how the
        old failure would come back, and it would go unnoticed until the next
        distribution upgrade.
        """

        honest = {
            "config_sha256": "0" * 64,
            **{field: "0" * 64 for field, _ in evaluator._TOOLCHAIN_CODE_PATHS},
        }
        self.assertTrue(evaluator._toolchain_code_hashes(honest))
        for name in sorted(ENVIRONMENT_FIELD_NAMES):
            with self.subTest(field=name):
                with self.assertRaises(ValueError):
                    evaluator._toolchain_code_hashes({**honest, name: "x"})

    def test_dropping_a_code_digest_is_rejected_too(self) -> None:
        honest = {
            "config_sha256": "0" * 64,
            **{field: "0" * 64 for field, _ in evaluator._TOOLCHAIN_CODE_PATHS},
        }
        del honest["environment_probe_sha256"]
        with self.assertRaises(ValueError):
            evaluator._toolchain_code_hashes(honest)


class ProvenanceDescribesTheMachine(unittest.TestCase):
    def test_provenance_carries_what_the_snapshot_gave_up(self) -> None:
        provenance = asdict(trainer.capture_environment_provenance())
        self.assertEqual(
            ENVIRONMENT_FIELD_NAMES - set(provenance),
            set(),
            "the sidecar must keep every field the identity dropped",
        )
        self.assertEqual(provenance["python_build"], " ".join(sys.version.split()))
        self.assertIn(provenance["ftrl_kernel"], {"native", "python"})

    def test_provenance_carries_a_probe_reading(self) -> None:
        provenance = trainer.capture_environment_provenance()
        probe = provenance.environment_probe
        self.assertEqual(
            probe["probe_sha256"], environment_probe.measure()["probe_sha256"]
        )
        cells = cast(Mapping[str, object], probe["cells"])
        self.assertEqual(
            set(cells), {name for name, _ in environment_probe.CELLS}
        )

    def test_a_stable_machine_passes_the_stability_check(self) -> None:
        provenance = trainer.capture_environment_provenance()
        trainer.verify_environment_stability(provenance)

    def test_a_machine_that_moves_mid_run_is_refused_by_name(self) -> None:
        """A library swapped under a running trainer invalidates the run.

        Nothing produced across that boundary can be trusted, whatever the
        gates say afterwards, so this is a refusal rather than a note.
        """

        provenance = trainer.capture_environment_provenance()
        moved = json.loads(json.dumps(provenance.environment_probe))
        moved["probe_sha256"] = "0" * 64
        moved["cells"]["libm"]["sha256"] = "0" * 64
        with self.assertRaisesRegex(RuntimeError, "libm"):
            trainer.verify_environment_stability(
                trainer.TrainingEnvironmentProvenance(
                    **{**asdict(provenance), "environment_probe": moved}
                )
            )


class TheSealedAnswerIsClaimedOnce(unittest.TestCase):
    """Removing the machine from the identity gave up a side effect.

    Before v21 a second look at the sealed test from another interpreter was
    refused by `claim_sealed_evaluation`, because the environment was part of
    the candidate hash. It no longer is, so the answer itself is now what gets
    consumed exactly once.
    """

    def test_the_digest_covers_the_sealed_sections_and_nothing_else(self) -> None:
        base = trainer.sealed_outcome_sha256(outcome_manifest())
        self.assertEqual(
            base,
            trainer.sealed_outcome_sha256(
                outcome_manifest(unrelated={"something": "else"})
            ),
            "a section outside the sealed answer must not move the digest",
        )
        for section in trainer.SEALED_OUTCOME_SECTIONS:
            with self.subTest(section=section):
                self.assertNotEqual(
                    base,
                    trainer.sealed_outcome_sha256(
                        outcome_manifest(**{section: {"changed": True}})
                    ),
                )

    def test_a_missing_section_is_refused_rather_than_skipped(self) -> None:
        incomplete = outcome_manifest()
        del incomplete["safety"]
        with self.assertRaisesRegex(ValueError, "safety"):
            trainer.sealed_outcome_sha256(incomplete)

    def claim(self, root: Path, *, outcome: str, candidate: str) -> None:
        trainer.claim_sealed_outcome(
            config=config(),
            outcome_sha256=outcome,
            candidate_sha256=candidate,
            repository_root=root,
        )

    def prepared_root(self, temporary: str) -> Path:
        root = Path(temporary)
        (root / "model/intent_v1").mkdir(parents=True)
        return root

    def test_an_identical_rerun_is_allowed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = self.prepared_root(temporary)
            self.claim(root, outcome="a" * 64, candidate="b" * 64)
            self.claim(root, outcome="a" * 64, candidate="b" * 64)

    def test_a_different_answer_from_the_same_candidate_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = self.prepared_root(temporary)
            self.claim(root, outcome="a" * 64, candidate="b" * 64)
            with self.assertRaisesRegex(RuntimeError, "different answer"):
                self.claim(root, outcome="c" * 64, candidate="b" * 64)

    def test_a_different_candidate_is_told_to_rotate(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = self.prepared_root(temporary)
            self.claim(root, outcome="a" * 64, candidate="b" * 64)
            with self.assertRaisesRegex(RuntimeError, "rotate split_namespace"):
                self.claim(root, outcome="c" * 64, candidate="d" * 64)

    def test_the_ledger_records_digests_and_no_metric(self) -> None:
        """The ledger must not become a way to read the sealed test."""

        with tempfile.TemporaryDirectory() as temporary:
            root = self.prepared_root(temporary)
            self.claim(root, outcome="a" * 64, candidate="b" * 64)
            path = trainer.sealed_outcome_path(config(), repository_root=root)
            record = json.loads(path.read_bytes())
            self.assertEqual(
                set(record),
                {
                    "schema_version",
                    "split_namespace",
                    "candidate_sha256",
                    "sealed_outcome_sha256",
                },
            )
            for value in record.values():
                self.assertIsInstance(value, (str, int))

    def test_the_ledger_sits_beside_the_registry_it_complements(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = self.prepared_root(temporary)
            registry = trainer.sealed_registry_path(config(), repository_root=root)
            outcome = trainer.sealed_outcome_path(config(), repository_root=root)
            self.assertEqual(registry.parent, outcome.parent)
            self.assertNotEqual(registry.name, outcome.name)


class RefusalsSayWhichKindOfDivergence(unittest.TestCase):
    """The old message always advised rotating, which misled three times in four."""

    expected = {
        "split_namespace": "ns:v20",
        "candidate_sha256": "b" * 64,
        "config_sha256": "c" * 64,
        "candidate_dataset_sha256": "d" * 64,
    }

    def refusal(self, **changes: object) -> str:
        previous = {**self.expected, **changes}
        return trainer._sealed_claim_refusal(
            json.dumps(previous).encode("utf-8"), self.expected
        )

    def test_a_foreign_namespace_is_named_as_such(self) -> None:
        self.assertIn("wrong registry file", self.refusal(split_namespace="ns:v19"))

    def test_a_changed_dataset_advises_rotation(self) -> None:
        message = self.refusal(candidate_dataset_sha256="9" * 64)
        self.assertIn("rotate split_namespace", message)
        self.assertIn("candidate_dataset_sha256", message)

    def test_identical_inputs_with_different_weights_points_at_the_probe(self) -> None:
        message = self.refusal(candidate_sha256="a" * 64)
        self.assertIn("environment divergence", message)
        self.assertIn("environment_probe", message)

    def test_an_unreadable_ledger_is_not_mistaken_for_a_candidate(self) -> None:
        message = trainer._sealed_claim_refusal(b'{"truncated"', self.expected)
        self.assertIn("unreadable", message)


class TheEvaluationReadsFrozenDictionaries(unittest.TestCase):
    """A dictionary is an input to the evaluation, so it is frozen like one."""

    def test_the_evaluator_points_at_the_frozen_root(self) -> None:
        import os

        self.assertEqual(
            os.environ.get("KEYSWITCH_HUNSPELL_PATH"),
            str(evaluator.FROZEN_HUNSPELL_ROOT),
        )

    def test_every_frozen_dictionary_is_present_and_checksummed(self) -> None:
        sums = (
            PROJECT_ROOT / "model/intent_v1/sources/SHA256SUMS"
        ).read_text(encoding="utf-8")
        for name in ("en_US.aff", "en_US.dic", "ru_RU.aff", "ru_RU.dic"):
            with self.subTest(dictionary=name):
                self.assertTrue(
                    (evaluator.FROZEN_HUNSPELL_ROOT / name).is_file()
                )
                self.assertIn(f"hunspell/{name}", sums)

    def test_the_report_shows_frozen_against_system(self) -> None:
        section = evaluator.environment_section()
        dictionaries = cast(Mapping[str, object], section["hunspell_dictionaries"])
        self.assertEqual(
            set(dictionaries),
            {"en_US.aff", "en_US.dic", "ru_RU.aff", "ru_RU.dic"},
        )
        probe = cast(Mapping[str, object], section["probe"])
        self.assertEqual(
            probe["probe_sha256"],
            environment_probe.measure()["probe_sha256"],
        )


if __name__ == "__main__":
    unittest.main()
