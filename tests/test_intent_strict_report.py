"""Tests for the fail-closed reuse check of strict intent-model reports."""

from __future__ import annotations

import hashlib
import json
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

TOOLS_PATH = str(Path(__file__).resolve().parents[1] / "tools")
if TOOLS_PATH not in sys.path:
    sys.path.insert(0, TOOLS_PATH)

import verify_intent_strict_report as verifier  # noqa: E402

PROJECT_ROOT = Path(__file__).resolve().parents[1]
ARTIFACT = PROJECT_ROOT / "src/keyswitch/resources/models/layout_intent_v1.ksm"
MANIFEST = PROJECT_ROOT / "model/intent_v1/manifest.json"
CONFIG = PROJECT_ROOT / "model/intent_v1/config.json"


def _sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _manifest() -> dict[str, object]:
    payload = json.loads(MANIFEST.read_text("utf-8"))
    assert isinstance(payload, dict)
    return {str(key): value for key, value in payload.items()}


def _toolchain(manifest: dict[str, object]) -> dict[str, object]:
    toolchain = manifest["toolchain"]
    assert isinstance(toolchain, dict)
    return {str(key): value for key, value in toolchain.items()}


def _synthetic_report() -> dict[str, object]:
    """Build a passing report whose hashes match the repository files."""

    manifest = _manifest()
    toolchain = _toolchain(manifest)
    provenance: list[dict[str, object]] = [
        {"name": "artifact_sha256", "passed": True, "detail": _sha256(ARTIFACT)},
        {"name": "config_sha256", "passed": True, "detail": _sha256(CONFIG)},
        {"name": "dataset_sha256", "passed": True, "detail": manifest["dataset_sha256"]},
        {"name": "split_namespace", "passed": True, "detail": manifest["split_namespace"]},
        {"name": "sealed_evaluation", "passed": True, "detail": "ok"},
        {"name": "sealed_candidate_sha256", "passed": True, "detail": "ok"},
        {"name": "build_provenance_sha256", "passed": True, "detail": "ok"},
        {"name": "model_version", "passed": True, "detail": "ok"},
    ]
    for name, relative in verifier.TOOLCHAIN_PATHS.items():
        digest = _sha256(PROJECT_ROOT / relative)
        provenance.append(
            {"name": name, "passed": True, "detail": f"current={digest}, manifest={digest}"}
        )
    receipt = verifier.receipt_path(manifest, PROJECT_ROOT)
    receipt_digest = _sha256(receipt)
    provenance.append(
        {
            "name": "toolchain_preseal_receipt_sha256",
            "passed": True,
            "detail": f"current={receipt_digest}, manifest={receipt_digest}",
        }
    )
    for name, relative in verifier.SOURCE_PATHS.items():
        provenance.append(
            {"name": name, "passed": True, "detail": _sha256(PROJECT_ROOT / relative)}
        )
    return {
        "strict_passed": True,
        "strict_gates": {"provenance": True, "safety": True, "sealed_test": True},
        "model": {
            "version": manifest["artifact_model_version"],
            "checksum": _sha256(ARTIFACT),
        },
        "performance": {"deterministic_predictions": True},
        "provenance": provenance,
        "toolchain_digest_sample": toolchain.get("trainer_sha256"),
    }


class StrictReportVerifierTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.report_path = self.root / "strict.json"

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def _write(self, report: dict[str, object]) -> Path:
        self.report_path.write_text(json.dumps(report), "utf-8")
        return self.report_path

    def _verify(self, report: dict[str, object], project_root: Path = PROJECT_ROOT) -> dict[str, object]:
        return verifier.verify_report(
            report_path=self._write(report),
            artifact_path=ARTIFACT,
            manifest_path=MANIFEST,
            config_path=CONFIG,
            project_root=project_root,
        )

    def _assert_rejected(self, report: dict[str, object], pattern: str) -> None:
        with self.assertRaisesRegex(verifier.ReportRejected, pattern):
            self._verify(report)

    def test_report_bound_to_the_current_tree_is_accepted(self) -> None:
        summary = self._verify(_synthetic_report())
        self.assertEqual(summary["artifact_sha256"], _sha256(ARTIFACT))
        self.assertEqual(summary["gate_count"], 3)
        self.assertEqual(summary["verified_files"], 12)
        self.assertEqual(summary["model_version"], _manifest()["artifact_model_version"])

    def test_failed_or_missing_gates_are_rejected(self) -> None:
        report = _synthetic_report()
        gates = report["strict_gates"]
        assert isinstance(gates, dict)
        gates["safety"] = False
        self._assert_rejected(report, "failed strict gates: safety")
        report["strict_gates"] = {}
        self._assert_rejected(report, "strict_gates is empty")
        report["strict_passed"] = False
        self._assert_rejected(report, "strict_passed is not true")

    def test_foreign_artifact_or_version_is_rejected(self) -> None:
        report = _synthetic_report()
        model = report["model"]
        assert isinstance(model, dict)
        model["checksum"] = "0" * 64
        self._assert_rejected(report, "checksum differs from the current artifact")
        report = _synthetic_report()
        model = report["model"]
        assert isinstance(model, dict)
        model["version"] = "intent-v1-000000000000"
        self._assert_rejected(report, "version differs from the manifest")

    def test_non_deterministic_or_failed_provenance_is_rejected(self) -> None:
        report = _synthetic_report()
        performance = report["performance"]
        assert isinstance(performance, dict)
        performance["deterministic_predictions"] = False
        self._assert_rejected(report, "deterministic predictions")
        report = _synthetic_report()
        provenance = report["provenance"]
        assert isinstance(provenance, list)
        first = provenance[0]
        assert isinstance(first, dict)
        first["passed"] = False
        self._assert_rejected(report, "provenance check did not pass")
        report = _synthetic_report()
        provenance = report["provenance"]
        assert isinstance(provenance, list)
        provenance.pop()
        self._assert_rejected(report, "lacks required checks")

    def test_stale_toolchain_hash_is_rejected(self) -> None:
        report = _synthetic_report()
        provenance = report["provenance"]
        assert isinstance(provenance, list)
        for entry in provenance:
            assert isinstance(entry, dict)
            if entry["name"] == "toolchain_trainer_sha256":
                entry["detail"] = "current=" + "1" * 64
        self._assert_rejected(report, "train_intent_model.py changed")

    def test_changed_toolchain_file_on_disk_is_rejected(self) -> None:
        """A report is stale once any hashed toolchain file changes."""

        mirror = self.root / "tree"
        for relative in (
            *verifier.TOOLCHAIN_PATHS.values(),
            *verifier.SOURCE_PATHS.values(),
            "model/intent_v1",
        ):
            source = PROJECT_ROOT / relative
            target = mirror / relative
            if source.is_dir():
                shutil.copytree(source, target, dirs_exist_ok=True)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(source, target)
        detector = mirror / "src/keyswitch/detector.py"
        detector.write_text(detector.read_text("utf-8") + "\n# drift\n", "utf-8")
        with self.assertRaisesRegex(verifier.ReportRejected, "detector.py changed"):
            self._verify(_synthetic_report(), project_root=mirror)

    def test_command_line_reports_the_reason_and_exit_status(self) -> None:
        report = _synthetic_report()
        report["strict_passed"] = False
        path = self._write(report)
        self.assertEqual(verifier.main(["--report", str(path)]), 1)
        self.assertEqual(verifier.main(["--report", str(self._write(_synthetic_report()))]), 0)
        self.assertEqual(verifier.main(["--report", str(self.root / "absent.json")]), 1)


class DebBuildReuseContractTests(unittest.TestCase):
    def test_build_deb_reuses_only_verified_reports(self) -> None:
        script = (PROJECT_ROOT / "packaging/build-deb.sh").read_text("utf-8")
        self.assertIn('reusable_strict_report="${KEYSWITCH_INTENT_STRICT_REPORT:-}"', script)
        self.assertIn("tools/verify_intent_strict_report.py", script)
        self.assertIn("Reusable strict report is not bound to the current tree", script)
        self.assertIn("exit 1", script.split("Reusable strict report is not bound")[1][:200])
        for workflow in ("tests.yml", "release.yml"):
            text = (PROJECT_ROOT / ".github/workflows" / workflow).read_text("utf-8")
            self.assertIn("--report build/keyswitch-intent-strict.json", text)
            self.assertIn(
                "KEYSWITCH_INTENT_STRICT_REPORT: build/keyswitch-intent-strict.json", text
            )


if __name__ == "__main__":
    unittest.main()
