#!/usr/bin/env python3
"""Verify that a strict intent-model report belongs to the current tree.

The strict evaluator takes about half an hour, so packaging and the release
pipeline may reuse a report that was produced earlier in the same release
contour.  Reuse must stay fail-closed: the report is accepted only when every
gate passed and every hash it recorded (artifact, config, frozen sources and
the complete model toolchain) still equals the file that is present now.  A
report that predates any change to those files is rejected, and the caller has
to run the evaluator again.

The check depends only on the standard library so it can run from
``packaging/build-deb.sh``, from ``tools/release_pipeline.py`` and from CI.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import Final


PROJECT_ROOT: Final[Path] = Path(__file__).resolve().parents[1]
REPORT_LIMIT_BYTES: Final[int] = 8 * 1024 * 1024
MANIFEST_LIMIT_BYTES: Final[int] = 1024 * 1024
ARTIFACT_LIMIT_BYTES: Final[int] = 14 * 1024 * 1024
SHA256_PATTERN: Final[re.Pattern[str]] = re.compile(r"^[0-9a-f]{64}$")
CURRENT_PATTERN: Final[re.Pattern[str]] = re.compile(r"current=([0-9a-f]{64})")

# Mirrors the mapping enforced by packaging/build-windows.ps1 and by the
# release pipeline; the preseal receipt path is derived from the registry.
TOOLCHAIN_PATHS: Final[Mapping[str, str]] = {
    "toolchain_trainer_sha256": "tools/train_intent_model.py",
    "toolchain_runtime_sha256": "src/keyswitch/intent_model.py",
    "toolchain_detector_sha256": "src/keyswitch/detector.py",
    "toolchain_protected_tokens_sha256": "src/keyswitch/resources/protected_tokens.txt",
    "toolchain_layouts_sha256": "src/keyswitch/layouts.py",
    "toolchain_language_model_sha256": "src/keyswitch/language_model.py",
    "toolchain_evaluator_sha256": "tools/evaluate_intent_model.py",
    "toolchain_preseal_generator_sha256": "tools/preseal_intent_holdout.py",
    "toolchain_development_freezer_sha256": "tools/freeze_intent_development_corpus.py",
}
SOURCE_PATHS: Final[Mapping[str, str]] = {
    "english_source_sha256": "model/intent_v1/sources/en_US.lm",
    "russian_source_sha256": "model/intent_v1/sources/ru_RU.lm",
}
REQUIRED_PROVENANCE: Final[frozenset[str]] = frozenset(
    {
        "artifact_sha256",
        "config_sha256",
        "dataset_sha256",
        "split_namespace",
        "sealed_evaluation",
        "sealed_candidate_sha256",
        "build_provenance_sha256",
        "model_version",
        "toolchain_preseal_receipt_sha256",
        *TOOLCHAIN_PATHS,
        *SOURCE_PATHS,
    }
)


class ReportRejected(Exception):
    """The report cannot stand in for a fresh strict evaluation."""


def sha256_bytes(payload: bytes) -> str:
    return hashlib.sha256(payload).hexdigest()


def read_bounded(path: Path, limit: int, label: str) -> bytes:
    try:
        with path.open("rb") as stream:
            payload = stream.read(limit + 1)
    except OSError as error:
        raise ReportRejected(f"{label} is unreadable: {error}") from error
    if len(payload) > limit:
        raise ReportRejected(f"{label} exceeds {limit} bytes")
    return payload


def load_object(path: Path, limit: int, label: str) -> dict[str, object]:
    try:
        payload = json.loads(read_bounded(path, limit, label).decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ReportRejected(f"{label} is not valid JSON: {error}") from error
    return as_object(payload, label)


def as_object(value: object, label: str) -> dict[str, object]:
    if not isinstance(value, dict):
        raise ReportRejected(f"{label} must be a JSON object")
    result: dict[str, object] = {}
    for key, item in value.items():
        if not isinstance(key, str):
            raise ReportRejected(f"{label} has a non-string key")
        result[key] = item
    return result


def as_sha256(value: object, label: str) -> str:
    if not isinstance(value, str) or SHA256_PATTERN.match(value) is None:
        raise ReportRejected(f"{label} is not a SHA-256 digest")
    return value


def current_digest(detail: object, label: str) -> str:
    """Extract the ``current=<sha256>`` digest a provenance entry recorded."""

    if not isinstance(detail, str):
        raise ReportRejected(f"{label} detail is not a string")
    match = CURRENT_PATTERN.search(detail)
    if match is None:
        raise ReportRejected(f"{label} detail lacks a current digest")
    return match.group(1)


def receipt_path(manifest: Mapping[str, object], project_root: Path) -> Path:
    sealed = as_object(manifest.get("sealed_evaluation"), "manifest.sealed_evaluation")
    registry = sealed.get("registry_path")
    if not isinstance(registry, str):
        raise ReportRejected("manifest.sealed_evaluation.registry_path is missing")
    match = re.search(r"seal-registry-v(\d+)\.json$", registry)
    if match is None:
        raise ReportRejected(f"unexpected registry path {registry!r}")
    return project_root / "model" / "intent_v1" / f"holdout-v{match.group(1)}-preseal.json"


def verify_report(
    *,
    report_path: Path,
    artifact_path: Path,
    manifest_path: Path,
    config_path: Path,
    project_root: Path = PROJECT_ROOT,
) -> dict[str, object]:
    """Return a short summary, or raise ``ReportRejected`` with the reason."""

    report = load_object(report_path, REPORT_LIMIT_BYTES, "strict report")
    manifest = load_object(manifest_path, MANIFEST_LIMIT_BYTES, "model manifest")
    artifact_sha256 = sha256_bytes(
        read_bounded(artifact_path, ARTIFACT_LIMIT_BYTES, "KSLM artifact")
    )
    config_sha256 = sha256_bytes(read_bounded(config_path, MANIFEST_LIMIT_BYTES, "config"))

    if report.get("strict_passed") is not True:
        raise ReportRejected("strict_passed is not true")
    gates = as_object(report.get("strict_gates"), "strict_gates")
    if not gates:
        raise ReportRejected("strict_gates is empty")
    failed = sorted(name for name, value in gates.items() if value is not True)
    if failed:
        raise ReportRejected("failed strict gates: " + ", ".join(failed))

    model = as_object(report.get("model"), "report.model")
    if model.get("checksum") != artifact_sha256:
        raise ReportRejected("report.model.checksum differs from the current artifact")
    expected_version = manifest.get("artifact_model_version")
    if not isinstance(expected_version, str) or model.get("version") != expected_version:
        raise ReportRejected("report.model.version differs from the manifest")
    if manifest.get("artifact_sha256") != artifact_sha256:
        raise ReportRejected("manifest.artifact_sha256 differs from the current artifact")
    if manifest.get("config_sha256") != config_sha256:
        raise ReportRejected("manifest.config_sha256 differs from the current config")
    performance = as_object(report.get("performance"), "report.performance")
    if performance.get("deterministic_predictions") is not True:
        raise ReportRejected("report did not record deterministic predictions")

    provenance_raw = report.get("provenance")
    if not isinstance(provenance_raw, list):
        raise ReportRejected("report.provenance is not a list")
    entries: dict[str, dict[str, object]] = {}
    for item in provenance_raw:
        entry = as_object(item, "provenance entry")
        name = entry.get("name")
        if not isinstance(name, str):
            raise ReportRejected("provenance entry lacks a name")
        if entry.get("passed") is not True:
            raise ReportRejected(f"provenance check did not pass: {name}")
        entries[name] = entry
    missing = sorted(REQUIRED_PROVENANCE - entries.keys())
    if missing:
        raise ReportRejected("provenance lacks required checks: " + ", ".join(missing))

    if entries["artifact_sha256"].get("detail") != artifact_sha256:
        raise ReportRejected("provenance artifact_sha256 differs from the current artifact")
    if entries["config_sha256"].get("detail") != config_sha256:
        raise ReportRejected("provenance config_sha256 differs from the current config")
    if entries["split_namespace"].get("detail") != manifest.get("split_namespace"):
        raise ReportRejected("provenance split_namespace differs from the manifest")

    toolchain = as_object(manifest.get("toolchain"), "manifest.toolchain")
    file_checks = dict(TOOLCHAIN_PATHS)
    verified: dict[str, str] = {}
    for name, relative in file_checks.items():
        recorded = current_digest(entries[name].get("detail"), name)
        actual = sha256_bytes(
            read_bounded(project_root / relative, REPORT_LIMIT_BYTES, relative)
        )
        manifest_field = name.removeprefix("toolchain_")
        if recorded != actual:
            raise ReportRejected(f"{relative} changed since the report was produced")
        if toolchain.get(manifest_field) != actual:
            raise ReportRejected(f"manifest.toolchain.{manifest_field} differs from {relative}")
        verified[relative] = actual
    receipt = receipt_path(manifest, project_root)
    recorded_receipt = current_digest(
        entries["toolchain_preseal_receipt_sha256"].get("detail"),
        "toolchain_preseal_receipt_sha256",
    )
    actual_receipt = sha256_bytes(
        read_bounded(receipt, REPORT_LIMIT_BYTES, "preseal receipt")
    )
    if recorded_receipt != actual_receipt or toolchain.get("preseal_receipt_sha256") != actual_receipt:
        raise ReportRejected("preseal receipt changed since the report was produced")
    verified[str(receipt.relative_to(project_root))] = actual_receipt
    for name, relative in SOURCE_PATHS.items():
        recorded_source = as_sha256(entries[name].get("detail"), name)
        actual_source = sha256_bytes(
            read_bounded(project_root / relative, ARTIFACT_LIMIT_BYTES, relative)
        )
        if recorded_source != actual_source:
            raise ReportRejected(f"{relative} changed since the report was produced")
        verified[relative] = actual_source

    return {
        "report": str(report_path),
        "report_sha256": sha256_bytes(read_bounded(report_path, REPORT_LIMIT_BYTES, "report")),
        "model_version": expected_version,
        "artifact_sha256": artifact_sha256,
        "gate_count": len(gates),
        "verified_files": len(verified),
    }


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument(
        "--artifact",
        type=Path,
        default=PROJECT_ROOT / "src" / "keyswitch" / "resources" / "models" / "layout_intent_v1.ksm",
    )
    parser.add_argument(
        "--manifest", type=Path, default=PROJECT_ROOT / "model" / "intent_v1" / "manifest.json"
    )
    parser.add_argument(
        "--config", type=Path, default=PROJECT_ROOT / "model" / "intent_v1" / "config.json"
    )
    parser.add_argument("--project-root", type=Path, default=PROJECT_ROOT)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    arguments = build_parser().parse_args(list(argv) if argv is not None else None)
    try:
        summary = verify_report(
            report_path=Path(str(arguments.report)),
            artifact_path=Path(str(arguments.artifact)),
            manifest_path=Path(str(arguments.manifest)),
            config_path=Path(str(arguments.config)),
            project_root=Path(str(arguments.project_root)).resolve(),
        )
    except ReportRejected as error:
        print(f"strict report rejected: {error}", file=sys.stderr)
        return 1
    print(json.dumps(summary, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
