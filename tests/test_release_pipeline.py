"""Tests for the unattended release pipeline orchestrator."""

from __future__ import annotations

import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

TOOLS_PATH = str(Path(__file__).resolve().parents[1] / "tools")
if TOOLS_PATH not in sys.path:
    sys.path.insert(0, TOOLS_PATH)

import release_pipeline as pipeline  # noqa: E402


def _options(**overrides: object) -> pipeline.Options:
    base: dict[str, object] = {
        "profile": "release",
        "only": (),
        "skip": (),
        "start_from": "",
        "replays": 2,
        "replay_dir": None,
        "replay_strict": False,
        "strict_report": None,
        "workers": 0,
        "jobs": 2,
        "memory_reserve_mib": 1024,
        "timeout_scale": 1.0,
        "fail_fast": False,
        "pipeline_root": Path("/nonexistent"),
    }
    base.update(overrides)
    return pipeline.Options(
        profile=str(base["profile"]),
        only=tuple(str(item) for item in _sequence(base["only"])),
        skip=tuple(str(item) for item in _sequence(base["skip"])),
        start_from=str(base["start_from"]),
        replays=int(str(base["replays"])),
        replay_dir=base["replay_dir"] if isinstance(base["replay_dir"], Path) else None,
        replay_strict=bool(base["replay_strict"]),
        strict_report=(
            base["strict_report"] if isinstance(base["strict_report"], Path) else None
        ),
        workers=int(str(base["workers"])),
        jobs=int(str(base["jobs"])),
        memory_reserve_mib=int(str(base["memory_reserve_mib"])),
        timeout_scale=float(str(base["timeout_scale"])),
        fail_fast=bool(base["fail_fast"]),
        pipeline_root=Path(str(base["pipeline_root"])),
    )


def _sequence(value: object) -> tuple[object, ...]:
    if isinstance(value, tuple):
        return tuple(value)
    return ()


class PhaseGraphTests(unittest.TestCase):
    def test_dependencies_exist_and_precede_their_dependents(self) -> None:
        order = [spec.name for spec in pipeline.PHASES]
        for spec in pipeline.PHASES:
            for dependency in spec.depends:
                self.assertIn(dependency, pipeline.PHASE_BY_NAME)
                self.assertLess(order.index(dependency), order.index(spec.name))

    def test_profiles_are_closed_under_dependencies(self) -> None:
        for name, members in pipeline.PROFILES.items():
            for member in members:
                self.assertIn(member, pipeline.PHASE_BY_NAME, name)
                for dependency in pipeline.PHASE_BY_NAME[member].depends:
                    self.assertIn(dependency, members, f"{name}: {member} needs {dependency}")

    def test_release_profile_covers_every_phase_once(self) -> None:
        self.assertEqual(
            list(pipeline.PROFILES["release"]),
            [spec.name for spec in pipeline.PHASES],
        )

    def test_checklist_refers_to_known_phases(self) -> None:
        for _label, names in pipeline.CHECKLIST:
            for name in names:
                self.assertIn(name, pipeline.PHASE_BY_NAME)

    def test_every_phase_declares_positive_budgets(self) -> None:
        for spec in pipeline.PHASES:
            self.assertGreater(spec.timeout_seconds, 0, spec.name)
            self.assertGreater(spec.expected_seconds, 0, spec.name)
            self.assertGreater(spec.memory_mib, 0, spec.name)


class SelectionTests(unittest.TestCase):
    def test_profile_selection_keeps_declaration_order(self) -> None:
        self.assertEqual(
            pipeline.select_phases(_options(profile="quick")),
            pipeline.PROFILES["quick"],
        )

    def test_only_skip_and_from_combine(self) -> None:
        selected = pipeline.select_phases(
            _options(
                profile="release",
                only=("environment", "coverage", "typecheck", "build-deb"),
                skip=("typecheck",),
                start_from="coverage",
            )
        )
        self.assertEqual(selected, ("coverage", "build-deb"))

    def test_unknown_names_are_usage_errors(self) -> None:
        with self.assertRaises(pipeline.UsageError):
            pipeline.select_phases(_options(only=("no-such-phase",)))
        with self.assertRaises(pipeline.UsageError):
            pipeline.select_phases(_options(start_from="no-such-phase"))
        with self.assertRaises(pipeline.UsageError):
            pipeline.select_phases(_options(skip=tuple(pipeline.PROFILES["quick"]), profile="quick"))

    def test_command_line_round_trips_through_to_argv(self) -> None:
        parser = pipeline.build_parser()
        first = pipeline.options_from(
            parser.parse_args(
                [
                    "run",
                    "--profile",
                    "app",
                    "--only",
                    "coverage,typecheck",
                    "--replays",
                    "1",
                    "--replay-strict",
                    "--jobs",
                    "3",
                    "--memory-reserve-mib",
                    "512",
                    "--fail-fast",
                ]
            )
        )
        second = pipeline.options_from(parser.parse_args(["run", *first.to_argv()]))
        self.assertEqual(first, second)
        self.assertEqual(first.only, ("coverage", "typecheck"))
        self.assertTrue(first.fail_fast)

    def test_invalid_replay_and_job_counts_are_rejected(self) -> None:
        parser = pipeline.build_parser()
        with self.assertRaises(pipeline.UsageError):
            pipeline.options_from(parser.parse_args(["run", "--replays", "3"]))
        with self.assertRaises(pipeline.UsageError):
            pipeline.options_from(parser.parse_args(["run", "--jobs", "0"]))


class ReportingTests(unittest.TestCase):
    def test_changelog_sections_collect_entries_per_heading(self) -> None:
        sections = pipeline.changelog_sections(
            "# Changelog\n\n## Unreleased\n\n- one\n- two\n\n## 0.6.1 — 2026-09-02\n\n- three\n"
        )
        self.assertEqual(sections["Unreleased"], ["- one", "- two"])
        self.assertEqual(sections["0.6.1 — 2026-09-02"], ["- three"])

    def test_checklist_rows_derive_verdicts_from_phase_statuses(self) -> None:
        rows = dict(
            pipeline.checklist_rows(
                {
                    "model-inputs": "passed",
                    "model-preseal-replay": "skipped",
                    "typecheck": "failed",
                    "coverage": "running",
                }
            )
        )
        self.assertEqual(rows["Registry, manifest and test-report agree"], "passed")
        self.assertEqual(
            rows["Preseal receipt is model-blind and reproducible"],
            "passed (some phases skipped)",
        )
        self.assertEqual(rows["Strict typing"], "FAILED")
        self.assertEqual(rows["100% line and branch coverage"], "running")
        self.assertEqual(rows["Detector quality gates"], "not run")
        self.assertIn("Windows installer verifier and smoke", rows)

    def test_summary_markdown_lists_failures_with_log_tail(self) -> None:
        state: dict[str, object] = {
            "pipeline": {
                "status": "failed",
                "profile": "quick",
                "run_dir": "/tmp/run",
                "started_at": "2026-09-02T00:00:00Z",
                "finished_at": "2026-09-02T00:01:00Z",
                "jobs": 2,
                "memory_reserve_mib": 1024,
                "git": {"branch": "main", "head": "abc", "dirty_files": 0},
                "not_covered_on_this_host": ["Windows job"],
            },
            "phases": [
                {
                    "name": "typecheck",
                    "status": "failed",
                    "duration_seconds": 12.5,
                    "observed_peak_rss_mib": 300,
                    "error": "mypy | failed",
                    "log": "/tmp/run/phases/01-typecheck.log",
                    "log_tail": ["error: boom"],
                    "facts": {},
                    "notes": ["note one"],
                },
                {
                    "name": "coverage",
                    "status": "passed",
                    "duration_seconds": 3661,
                    "observed_peak_rss_mib": 400,
                    "error": None,
                    "log": None,
                    "log_tail": [],
                    "facts": {"tests": 346},
                    "notes": [],
                },
            ],
        }
        markdown = pipeline.render_summary_markdown(state)
        self.assertIn("# KeySwitch release pipeline: FAILED", markdown)
        self.assertIn("| 1 | typecheck | failed | 12s | 300 MiB | mypy / failed |", markdown)
        self.assertIn("| 2 | coverage | passed | 1h01m01s |", markdown)
        self.assertIn("### typecheck: mypy | failed", markdown)
        self.assertIn("error: boom", markdown)
        self.assertIn("- note: note one", markdown)
        self.assertIn('"tests": 346', markdown)
        self.assertIn("- Windows job", markdown)

    def test_format_duration(self) -> None:
        self.assertEqual(pipeline.format_duration(None), "-")
        self.assertEqual(pipeline.format_duration(59), "59s")
        self.assertEqual(pipeline.format_duration(61), "1m01s")
        self.assertEqual(pipeline.format_duration(3600), "1h00m00s")


class JsonHelperTests(unittest.TestCase):
    def test_lookup_walks_nested_objects_and_reports_missing_fields(self) -> None:
        document: dict[str, object] = {"a": {"b": {"c": 1}}}
        self.assertEqual(pipeline.lookup(document, "a", "b", "c"), 1)
        with self.assertRaisesRegex(pipeline.PhaseFailure, "missing JSON field: a.b.d"):
            pipeline.lookup(document, "a", "b", "d")
        with self.assertRaisesRegex(pipeline.PhaseFailure, "must be a JSON object"):
            pipeline.lookup(document, "a", "b", "c", "e")

    def test_scalar_coercions_reject_wrong_types(self) -> None:
        self.assertEqual(pipeline.as_int(3, "n"), 3)
        with self.assertRaises(pipeline.PhaseFailure):
            pipeline.as_int(True, "n")
        with self.assertRaises(pipeline.PhaseFailure):
            pipeline.as_bool(1, "flag")
        with self.assertRaises(pipeline.PhaseFailure):
            pipeline.as_str(None, "text")
        self.assertIsNone(pipeline.optional_number(True))
        self.assertEqual(pipeline.optional_number(2), 2.0)

    def test_load_json_object_enforces_bounds_and_shape(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "payload.json"
            path.write_text("[1, 2]", "utf-8")
            with self.assertRaisesRegex(pipeline.PhaseFailure, "must be a JSON object"):
                pipeline.load_json_object(path, "payload")
            path.write_text("{not json", "utf-8")
            with self.assertRaisesRegex(pipeline.PhaseFailure, "not valid JSON"):
                pipeline.load_json_object(path, "payload")
            with self.assertRaisesRegex(pipeline.PhaseFailure, "is missing"):
                pipeline.load_json_object(Path(directory) / "absent.json", "payload")

    def test_write_json_atomic_replaces_the_target(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "state.json"
            pipeline.write_json_atomic(path, {"value": 1})
            pipeline.write_json_atomic(path, {"value": 2})
            self.assertEqual(json.loads(path.read_text("utf-8")), {"value": 2})
            self.assertFalse((Path(directory) / "state.json.tmp").exists())


class ModelArtifactTests(unittest.TestCase):
    def test_bundled_kslm_matches_manifest_identity(self) -> None:
        bounds = pipeline.kslm_bounds(pipeline.MODEL_ARTIFACT)
        manifest = pipeline.load_json_object(pipeline.MODEL_MANIFEST, "manifest")
        self.assertEqual(bounds["schema"], 4)
        self.assertEqual(bounds["embedded_model_version"], manifest["artifact_model_version"])
        self.assertEqual(bounds["bytes"], pipeline.MODEL_ARTIFACT.stat().st_size)

    def test_model_identity_resolves_versioned_receipt_from_registry(self) -> None:
        identity = pipeline.model_identity()
        self.assertTrue(identity.registry_path.name.startswith("seal-registry-v"))
        self.assertTrue(identity.receipt_path.name.startswith("holdout-v"))
        self.assertEqual(
            identity.receipt_path.name.split("-")[1],
            identity.registry_path.stem.split("-")[-1],
        )
        self.assertTrue(identity.artifact_version.startswith("intent-v1-"))


@unittest.skipUnless(Path("/proc/self/stat").exists(), "requires Linux /proc")
class ProcessAccountingTests(unittest.TestCase):
    def test_session_memory_counts_the_current_session(self) -> None:
        self.assertEqual(pipeline.session_rss_mib([]), 0)
        getsid = getattr(os, "getsid", None)
        self.assertIsNotNone(getsid)
        if getsid is None:
            raise AssertionError("os.getsid is unavailable")
        self.assertGreater(pipeline.session_rss_mib([int(getsid(0))]), 0)

    def test_process_command_lines_include_this_interpreter(self) -> None:
        lines = pipeline.process_command_lines()
        self.assertTrue(any("python" in line for line in lines))


if __name__ == "__main__":
    unittest.main()
