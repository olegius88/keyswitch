#!/usr/bin/env python3
"""Write ``model/intent_v1/rejection-vN.json`` from the actual strict report.

A candidate whose sealed test was opened but whose independent strict
evaluation failed must leave an immutable, fact-only receipt behind (runbook
section 9, cookbook section 6.9).  Every value below is read from the
published manifest, the seal registry and the strict report; nothing is typed
in by hand, so the receipt cannot disagree with the evidence it describes.
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
MODEL_DIRECTORY: Final[Path] = PROJECT_ROOT / "model" / "intent_v1"
LIMIT_BYTES: Final[int] = 64 * 1024 * 1024
FAILURE_SECTIONS: Final[Mapping[str, str]] = {
    "fallback_regression": "model_vs_fallback",
    "unknown_typo_false_positives": "lexical_disjoint_unknown_typos",
    "unknown_typo_recall": "lexical_disjoint_unknown_typos",
    "sealed_test": "sealed_test",
    "sealed_test_context_stress": "sealed_test_context_stress",
    "production_context_ensemble": "production_context_ensemble",
    "safety": "safety",
    "veto": "veto",
    "load_latency": "performance",
    "inference_latency": "performance",
}


def sha256_file(path: Path) -> str:
    with path.open("rb") as stream:
        payload = stream.read(LIMIT_BYTES + 1)
    if len(payload) > LIMIT_BYTES:
        raise ValueError(f"{path} exceeds {LIMIT_BYTES} bytes")
    return hashlib.sha256(payload).hexdigest()


def load_object(path: Path) -> dict[str, object]:
    payload = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(payload, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return {str(key): value for key, value in payload.items()}


def as_str(value: object, label: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{label} must be a string")
    return value


def compact(value: object, depth: int = 0) -> object:
    """Keep scalar evidence and small mappings, drop bulky per-row payloads."""

    if isinstance(value, dict):
        if depth >= 3:
            return {"omitted_keys": sorted(str(key) for key in value)}
        return {str(key): compact(item, depth + 1) for key, item in value.items()}
    if isinstance(value, list):
        return value if len(value) <= 16 else {"omitted_items": len(value)}
    return value


def build_receipt(
    *,
    version: int,
    strict_report: Path,
    manifest_path: Path,
    registry_path: Path,
    artifact_path: Path,
    test_report_path: Path,
    evaluator_path: Path,
    remediation: str,
) -> dict[str, object]:
    manifest = load_object(manifest_path)
    registry = load_object(registry_path)
    report = load_object(strict_report)
    gates = report.get("strict_gates")
    if not isinstance(gates, dict):
        raise ValueError("strict report lacks strict_gates")
    failed = sorted(str(name) for name, value in gates.items() if value is not True)
    if not failed:
        raise ValueError("strict report passed every gate; nothing to reject")
    if report.get("strict_passed") is not False:
        raise ValueError("strict report does not record strict_passed=false")
    split_namespace = as_str(manifest.get("split_namespace"), "split_namespace")
    if not re.search(rf":intent-v{version}:", split_namespace):
        raise ValueError(f"manifest namespace {split_namespace!r} is not v{version}")
    failure: dict[str, object] = {"failed_strict_gates": failed}
    for gate in failed:
        section = FAILURE_SECTIONS.get(gate)
        if section is not None and section in report:
            failure[section] = compact(report[section])
    return {
        "schema_version": 1,
        "phase": "strict_" + failed[0],
        "decision": "rejected",
        "reason": ",".join(failed),
        "artifact_published": True,
        "internal_quality_gates_passed": manifest.get("quality_gates_passed") is True,
        "independent_external_holdout_evaluated": (
            "lexical_disjoint_unknown_typos" in report
        ),
        "model_version": as_str(manifest.get("artifact_model_version"), "model version"),
        "artifact_sha256": sha256_file(artifact_path),
        "config_sha256": as_str(manifest.get("config_sha256"), "config sha"),
        "candidate_sha256": as_str(registry.get("candidate_sha256"), "candidate sha"),
        "candidate_dataset_sha256": as_str(
            registry.get("candidate_dataset_sha256"), "candidate dataset sha"
        ),
        "seal_registry_sha256": sha256_file(registry_path),
        "manifest_sha256": sha256_file(manifest_path),
        "internal_report_sha256": sha256_file(test_report_path),
        "evaluator_sha256": sha256_file(evaluator_path),
        "strict_report_sha256": sha256_file(strict_report),
        "failure": failure,
        "remediation": remediation,
    }


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", type=int, required=True, help="candidate number N")
    parser.add_argument("--strict-report", type=Path, required=True)
    parser.add_argument("--output", type=Path, default=None)
    parser.add_argument(
        "--remediation",
        default=(
            "Treat the disclosed sealed and external results as development "
            "evidence only. Rotate the sealed split, registry, hard-negative "
            "source and independent holdout namespaces before the next candidate "
            "and evaluate that candidate exactly once."
        ),
    )
    arguments = parser.parse_args(list(argv) if argv is not None else None)
    version = int(arguments.version)
    output = Path(str(arguments.output)) if arguments.output else (
        MODEL_DIRECTORY / f"rejection-v{version}.json"
    )
    if output.exists():
        print(f"refusing to overwrite the existing receipt {output}", file=sys.stderr)
        return 1
    try:
        receipt = build_receipt(
            version=version,
            strict_report=Path(str(arguments.strict_report)),
            manifest_path=MODEL_DIRECTORY / "manifest.json",
            registry_path=MODEL_DIRECTORY / f"seal-registry-v{version}.json",
            artifact_path=PROJECT_ROOT
            / "src"
            / "keyswitch"
            / "resources"
            / "models"
            / "layout_intent_v1.ksm",
            test_report_path=MODEL_DIRECTORY / "test-report.json",
            evaluator_path=PROJECT_ROOT / "tools" / "evaluate_intent_model.py",
            remediation=str(arguments.remediation),
        )
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"cannot write the rejection receipt: {error}", file=sys.stderr)
        return 1
    output.write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n", "utf-8")
    print(output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
