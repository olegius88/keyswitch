param(
    [string]$OutputDirectory = "",
    [string]$ModelDirectory = "",
    [string]$ModelLicense = ""
)

$ErrorActionPreference = "Stop"

function Invoke-NativeCommand {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Command,
        [Parameter(Mandatory = $true)]
        [string[]]$Arguments,
        [Parameter(Mandatory = $true)]
        [string]$FailureMessage
    )

    # Windows PowerShell 5.1 exposes a native program's stderr as the PowerShell
    # error stream. Tools such as Nuitka write normal progress messages there,
    # so ErrorActionPreference=Stop would abort an otherwise successful build.
    $PreviousErrorActionPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        & $Command @Arguments
        $NativeExitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $PreviousErrorActionPreference
    }

    if ($NativeExitCode -ne 0) {
        throw "$FailureMessage (exit code $NativeExitCode)"
    }
}

function Read-BoundedFileBytes {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,
        [Parameter(Mandatory = $true)]
        [long]$MaximumBytes,
        [Parameter(Mandatory = $true)]
        [int]$MinimumBytes,
        [Parameter(Mandatory = $true)]
        [string]$Label
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "$Label is missing: $Path"
    }
    $ResolvedPath = (Resolve-Path -LiteralPath $Path).Path
    $Stream = [System.IO.File]::Open(
        $ResolvedPath,
        [System.IO.FileMode]::Open,
        [System.IO.FileAccess]::Read,
        [System.IO.FileShare]::Read
    )
    try {
        $Length = $Stream.Length
        if ($Length -lt $MinimumBytes) {
            throw "$Label is shorter than $MinimumBytes bytes: $Path"
        }
        if ($Length -gt $MaximumBytes) {
            throw "$Label exceeds $MaximumBytes bytes: $Path"
        }
        if ($Length -gt [int]::MaxValue) {
            throw "$Label cannot be represented by the bounded reader: $Path"
        }
        $Buffer = [byte[]]::new([int]$Length)
        $Offset = 0
        while ($Offset -lt $Buffer.Length) {
            $Read = $Stream.Read($Buffer, $Offset, $Buffer.Length - $Offset)
            if ($Read -eq 0) {
                throw "$Label changed while it was being read: $Path"
            }
            $Offset += $Read
        }
        if ($Stream.ReadByte() -ne -1) {
            throw "$Label grew beyond its bounded snapshot: $Path"
        }
        return ,$Buffer
    }
    finally {
        $Stream.Dispose()
    }
}

function Read-BoundedJsonObject {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,
        [Parameter(Mandatory = $true)]
        [long]$MaximumBytes,
        [Parameter(Mandatory = $true)]
        [string]$Label
    )

    [byte[]]$Bytes = Read-BoundedFileBytes `
        -Path $Path `
        -MaximumBytes $MaximumBytes `
        -MinimumBytes 2 `
        -Label $Label
    $Utf8 = [System.Text.UTF8Encoding]::new($false, $true)
    try {
        $Text = $Utf8.GetString($Bytes)
        $Value = $Text | ConvertFrom-Json
    }
    catch {
        throw "$Label must contain bounded UTF-8 JSON: $Path ($($_.Exception.Message))"
    }
    if ($null -eq $Value -or $Value -isnot [System.Management.Automation.PSCustomObject]) {
        throw "$Label must contain a JSON object: $Path"
    }
    return $Value
}

function Get-BytesSha256 {
    param(
        [Parameter(Mandatory = $true)]
        [byte[]]$Bytes
    )

    $Hasher = [System.Security.Cryptography.SHA256]::Create()
    try {
        return ([System.BitConverter]::ToString($Hasher.ComputeHash($Bytes))).Replace("-", "").ToLowerInvariant()
    }
    finally {
        $Hasher.Dispose()
    }
}

function Get-VerifiedFrozenFileHash {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,
        [Parameter(Mandatory = $true)]
        [long]$ExpectedBytes,
        [Parameter(Mandatory = $true)]
        [string]$ExpectedSha256,
        [Parameter(Mandatory = $true)]
        [string]$Label
    )

    if ($ExpectedBytes -lt 1 -or $ExpectedBytes -gt 64MB) {
        throw "$Label has an invalid configured byte count: $ExpectedBytes"
    }
    if ($ExpectedSha256 -cnotmatch '^[0-9a-f]{64}$') {
        throw "$Label has an invalid configured SHA-256: $ExpectedSha256"
    }
    [byte[]]$Snapshot = Read-BoundedFileBytes `
        -Path $Path `
        -MaximumBytes $ExpectedBytes `
        -MinimumBytes ([int]$ExpectedBytes) `
        -Label $Label
    $ActualSha256 = Get-BytesSha256 -Bytes $Snapshot
    if ($ActualSha256 -cne $ExpectedSha256) {
        throw "$Label SHA-256 differs from config.json: $Path"
    }
    return $ActualSha256
}

$ModelContractValidator = @'
from __future__ import annotations

import hashlib
import json
import math
import struct
import sys
from pathlib import Path
from typing import Any


def fail(message: str) -> None:
    raise ValueError(message)


def bounded_bytes(path: Path, maximum_bytes: int, label: str) -> bytes:
    with path.open("rb") as stream:
        raw = stream.read(maximum_bytes + 1)
    if len(raw) > maximum_bytes:
        fail(f"{label} exceeds {maximum_bytes} bytes: {path}")
    return raw


def strict_json(path: Path, maximum_bytes: int, label: str) -> dict[str, Any]:
    raw = bounded_bytes(path, maximum_bytes, label)

    def unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        result: dict[str, Any] = {}
        for key, value in pairs:
            if key in result:
                fail(f"{label} contains duplicate key {key!r}")
            result[key] = value
        return result

    try:
        decoded = json.loads(
            raw.decode("utf-8"),
            object_pairs_hook=unique_object,
            parse_constant=lambda value: fail(
                f"{label} contains forbidden JSON constant {value}"
            ),
        )
    except (UnicodeDecodeError, json.JSONDecodeError, RecursionError) as error:
        raise ValueError(f"{label} must contain bounded UTF-8 JSON") from error
    if type(decoded) is not dict:
        fail(f"{label} must contain a JSON object")
    return decoded


KSLM_MAXIMUM_CONTAINER_BYTES = 14 * 1024 * 1024
KSLM_MAXIMUM_MANIFEST_BYTES = 1024 * 1024
KSLM_MAXIMUM_PAYLOAD_BYTES = 12 * 1024 * 1024
KSLM_MAXIMUM_FINGERPRINTS = 1 << 20
KSLM_SCHEMA = 4
KSLM_HEADER = struct.Struct("<4sHHIII32s")
DIRECTIONS = ("0>1", "1>0")
WILSON_INTERVAL_CONFIDENCE = 0.95
WILSON_95_Z_SCORE = 1.959963984540054
SELECTION_FALSE_POSITIVE_COMPARISONS = 12
SELECTION_PER_COMPARISON_CONFIDENCE = 0.9958333333333333
SELECTION_WILSON_Z_SCORE = 2.8652602385321333


def verify_kslm_packaging_bounds(path: Path) -> bytes:
    with path.open("rb") as stream:
        data = stream.read(KSLM_MAXIMUM_CONTAINER_BYTES + 1)
    if len(data) > KSLM_MAXIMUM_CONTAINER_BYTES:
        fail("KSLM container exceeds the 14 MiB packaging limit")
    if len(data) < KSLM_HEADER.size:
        fail("KSLM header is truncated")
    magic, schema, flags, manifest_length, payload_length, _crc, _digest = (
        KSLM_HEADER.unpack_from(data)
    )
    if magic != b"KSLM":
        fail("KSLM magic is invalid")
    if schema != KSLM_SCHEMA:
        fail("KSLM schema is unsupported")
    if flags != 0:
        fail("KSLM header flags are unsupported")
    if not 2 <= manifest_length <= KSLM_MAXIMUM_MANIFEST_BYTES:
        fail("KSLM embedded manifest exceeds the 1 MiB packaging limit")
    if not 0 < payload_length <= KSLM_MAXIMUM_PAYLOAD_BYTES:
        fail("KSLM payload exceeds the 12 MiB packaging limit")
    if len(data) != KSLM_HEADER.size + manifest_length + payload_length:
        fail("KSLM header lengths do not match the complete container")
    try:
        embedded = json.loads(
            data[
                KSLM_HEADER.size : KSLM_HEADER.size + manifest_length
            ].decode("utf-8")
        )
    except (UnicodeDecodeError, json.JSONDecodeError, RecursionError) as error:
        raise ValueError("KSLM embedded manifest is invalid") from error
    if type(embedded) is not dict:
        fail("KSLM embedded manifest must be an object")
    fingerprints = embedded.get("supported_fingerprint_count")
    dimension = embedded.get("dimension")
    if (
        type(fingerprints) is not int
        or not 0 <= fingerprints <= KSLM_MAXIMUM_FINGERPRINTS
    ):
        fail("KSLM fingerprint count exceeds the 2^20 packaging limit")
    if type(dimension) is not int or dimension <= 0:
        fail("KSLM dimension is invalid")
    if payload_length != (dimension * 2) + (fingerprints * 8):
        fail("KSLM payload shape does not match its embedded manifest")
    return data


def exact_object(value: Any, path: str, fields: set[str]) -> dict[str, Any]:
    if type(value) is not dict:
        fail(f"{path} must be an object")
    if set(value) != fields:
        fail(
            f"{path} fields mismatch: expected {sorted(fields)}, "
            f"got {sorted(value)}"
        )
    return value


def exact_true(value: Any, path: str) -> None:
    if value is not True:
        fail(f"{path} must be exact true")


def exact_sha256(value: Any, path: str) -> str:
    if (
        type(value) is not str
        or len(value) != 64
        or any(character not in "0123456789abcdef" for character in value)
    ):
        fail(f"{path} must be a lowercase SHA-256")
    return value


def finite_number(value: Any, path: str) -> float:
    if type(value) not in (int, float) or not math.isfinite(float(value)):
        fail(f"{path} must be a finite number")
    return float(value)


def nonnegative_integer(value: Any, path: str) -> int:
    if type(value) is not int or value < 0:
        fail(f"{path} must be a non-negative integer")
    return value


def fraction(value: Any, path: str) -> float:
    parsed = finite_number(value, path)
    if not 0.0 <= parsed <= 1.0:
        fail(f"{path} must be in [0, 1]")
    return parsed


METRIC_FIELDS = {
    "true_positive",
    "false_negative",
    "true_negative",
    "false_positive",
    "precision",
    "recall",
    "specificity",
    "false_positive_rate",
    "false_positive_rate_upper_95",
    "negative_samples",
}
CHECK_FIELDS = {
    "positive_samples",
    "negative_samples",
    "precision",
    "recall",
    "specificity",
    "false_positive_rate_upper_bound",
}
LIMIT_FIELDS = {
    "minimum_precision",
    "minimum_recall",
    "minimum_specificity",
    "maximum_false_positive_rate_upper_bound",
}
FALSE_POSITIVE_BOUND_FIELDS = {
    "method",
    "multiplicity_correction",
    "familywise_confidence",
    "comparisons",
    "per_comparison_confidence",
    "z_score",
    "upper",
}


def metric_payload(value: Any, path: str) -> dict[str, Any]:
    metric = exact_object(value, path, METRIC_FIELDS)
    for field in (
        "true_positive",
        "false_negative",
        "true_negative",
        "false_positive",
        "negative_samples",
    ):
        nonnegative_integer(metric[field], f"{path}.{field}")
    if metric["negative_samples"] != (
        metric["true_negative"] + metric["false_positive"]
    ):
        fail(f"{path}.negative_samples is inconsistent")
    for field in METRIC_FIELDS - {
        "true_positive",
        "false_negative",
        "true_negative",
        "false_positive",
        "negative_samples",
    }:
        fraction(metric[field], f"{path}.{field}")
    return metric


def wilson_upper_bound(successes: int, samples: int, z_score: float) -> float:
    if samples < 0 or successes < 0 or successes > samples:
        fail("invalid binomial counts in signed quality evidence")
    if not math.isfinite(z_score) or z_score <= 0.0:
        fail("invalid Wilson z-score in Windows packaging policy")
    if samples == 0:
        return 1.0
    proportion = successes / samples
    z_squared = z_score * z_score
    denominator = 1.0 + z_squared / samples
    centre = proportion + z_squared / (2.0 * samples)
    radius = z_score * math.sqrt(
        proportion * (1.0 - proportion) / samples
        + z_squared / (4.0 * samples * samples)
    )
    return min(1.0, (centre + radius) / denominator)


def binary_gate(
    value: Any,
    path: str,
    *,
    selection_familywise: bool,
) -> dict[str, Any]:
    gate = exact_object(
        value,
        path,
        {"passed", "checks", "actual", "false_positive_bound", "limits"},
    )
    exact_true(gate["passed"], f"{path}.passed")
    checks = exact_object(gate["checks"], f"{path}.checks", CHECK_FIELDS)
    for field in CHECK_FIELDS:
        exact_true(checks[field], f"{path}.checks.{field}")
    actual = metric_payload(gate["actual"], f"{path}.actual")
    bound = exact_object(
        gate["false_positive_bound"],
        f"{path}.false_positive_bound",
        FALSE_POSITIVE_BOUND_FIELDS,
    )
    limits = exact_object(gate["limits"], f"{path}.limits", LIMIT_FIELDS)
    for field in LIMIT_FIELDS:
        fraction(limits[field], f"{path}.limits.{field}")
    expected_comparisons = (
        SELECTION_FALSE_POSITIVE_COMPARISONS if selection_familywise else 1
    )
    expected_per_comparison_confidence = (
        SELECTION_PER_COMPARISON_CONFIDENCE
        if selection_familywise
        else WILSON_INTERVAL_CONFIDENCE
    )
    expected_z_score = (
        SELECTION_WILSON_Z_SCORE
        if selection_familywise
        else WILSON_95_Z_SCORE
    )
    if bound["method"] != "wilson_score_upper_endpoint":
        fail(f"{path}.false_positive_bound.method is unsupported")
    expected_correction = "bonferroni" if selection_familywise else "none"
    if bound["multiplicity_correction"] != expected_correction:
        fail(f"{path}.false_positive_bound correction is inconsistent")
    if fraction(
        bound["familywise_confidence"],
        f"{path}.false_positive_bound.familywise_confidence",
    ) != WILSON_INTERVAL_CONFIDENCE:
        fail(f"{path}.false_positive_bound familywise confidence differs")
    comparisons = nonnegative_integer(
        bound["comparisons"], f"{path}.false_positive_bound.comparisons"
    )
    if comparisons != expected_comparisons:
        fail(f"{path}.false_positive_bound comparison count differs")
    if fraction(
        bound["per_comparison_confidence"],
        f"{path}.false_positive_bound.per_comparison_confidence",
    ) != expected_per_comparison_confidence:
        fail(f"{path}.false_positive_bound confidence differs")
    if finite_number(
        bound["z_score"], f"{path}.false_positive_bound.z_score"
    ) != expected_z_score:
        fail(f"{path}.false_positive_bound z-score differs")
    upper = fraction(bound["upper"], f"{path}.false_positive_bound.upper")
    expected_upper = wilson_upper_bound(
        actual["false_positive"],
        actual["negative_samples"],
        expected_z_score,
    )
    if not math.isclose(upper, expected_upper, rel_tol=0.0, abs_tol=1e-15):
        fail(f"{path}.false_positive_bound upper endpoint is inconsistent")
    if upper > limits["maximum_false_positive_rate_upper_bound"]:
        fail(f"{path}.false_positive_bound exceeds its signed limit")
    return gate


def exact_trigger_map(value: Any, path: str, triggers: tuple[str, ...]) -> dict[str, Any]:
    return exact_object(value, path, set(triggers))


def selection_trigger_map(
    value: Any,
    path: str,
    triggers: tuple[str, ...],
    maximum_false_positives: int,
) -> dict[str, Any]:
    result = exact_trigger_map(value, path, triggers)
    for trigger in triggers:
        item_path = f"{path}.{trigger}"
        item = exact_object(
            result[trigger],
            item_path,
            {
                "passed",
                "logits",
                "overall",
                "typos",
                "false_positive_budget",
            },
        )
        exact_true(item["passed"], f"{item_path}.passed")
        logits = exact_object(
            item["logits"], f"{item_path}.logits", set(DIRECTIONS)
        )
        for direction in DIRECTIONS:
            finite_number(
                logits[direction], f"{item_path}.logits.{direction}"
            )
        overall = binary_gate(
            item["overall"], f"{item_path}.overall", selection_familywise=True
        )
        typos = binary_gate(
            item["typos"], f"{item_path}.typos", selection_familywise=True
        )
        budget_path = f"{item_path}.false_positive_budget"
        budget = exact_object(
            item["false_positive_budget"],
            budget_path,
            {
                "passed",
                "checks",
                "actual",
                "maximum_false_positives_per_trigger",
            },
        )
        exact_true(budget["passed"], f"{budget_path}.passed")
        checks = exact_object(
            budget["checks"], f"{budget_path}.checks", {"overall", "typos"}
        )
        exact_true(checks["overall"], f"{budget_path}.checks.overall")
        exact_true(checks["typos"], f"{budget_path}.checks.typos")
        actual = exact_object(
            budget["actual"],
            f"{budget_path}.actual",
            {"overall_false_positives", "typo_false_positives"},
        )
        overall_false_positives = nonnegative_integer(
            actual["overall_false_positives"],
            f"{budget_path}.actual.overall_false_positives",
        )
        typo_false_positives = nonnegative_integer(
            actual["typo_false_positives"],
            f"{budget_path}.actual.typo_false_positives",
        )
        if budget["maximum_false_positives_per_trigger"] != maximum_false_positives:
            fail(f"{budget_path} maximum differs from training config")
        if (
            overall_false_positives != overall["actual"]["false_positive"]
            or typo_false_positives != typos["actual"]["false_positive"]
            or overall_false_positives > maximum_false_positives
            or typo_false_positives > maximum_false_positives
        ):
            fail(f"{budget_path} is inconsistent with signed metrics")
    return result


def binary_trigger_map(
    value: Any, path: str, triggers: tuple[str, ...]
) -> dict[str, Any]:
    result = exact_trigger_map(value, path, triggers)
    for trigger in triggers:
        binary_gate(
            result[trigger], f"{path}.{trigger}", selection_familywise=False
        )
    return result


def context_gate(
    value: Any,
    path: str,
    triggers: tuple[str, ...],
    profiles: tuple[Any, ...],
    *,
    selection_familywise: bool,
) -> dict[str, Any]:
    gate = exact_object(
        value,
        path,
        {"passed", "all_profiles_present", "expected_profiles", "profiles"},
    )
    exact_true(gate["passed"], f"{path}.passed")
    exact_true(gate["all_profiles_present"], f"{path}.all_profiles_present")
    expected_names = sorted(profile.name for profile in profiles)
    if type(gate["expected_profiles"]) is not list or gate["expected_profiles"] != expected_names:
        fail(f"{path}.expected_profiles must contain every fixed profile")
    profile_map = exact_object(
        gate["profiles"], f"{path}.profiles", set(expected_names)
    )
    profile_by_name = {profile.name: profile for profile in profiles}
    for name in expected_names:
        profile_path = f"{path}.profiles.{name}"
        item = exact_object(
            profile_map[name],
            profile_path,
            {"passed", "delta", "group_selector", "all_triggers_present", "per_trigger"},
        )
        exact_true(item["passed"], f"{profile_path}.passed")
        exact_true(
            item["all_triggers_present"],
            f"{profile_path}.all_triggers_present",
        )
        expected = profile_by_name[name]
        if finite_number(item["delta"], f"{profile_path}.delta") != expected.delta:
            fail(f"{profile_path}.delta does not match the fixed profile")
        if item["group_selector"] != expected.group_selector:
            fail(f"{profile_path}.group_selector does not match the fixed profile")
        per_trigger = exact_trigger_map(
            item["per_trigger"], f"{profile_path}.per_trigger", triggers
        )
        for trigger in triggers:
            trigger_path = f"{profile_path}.per_trigger.{trigger}"
            trigger_item = exact_object(
                per_trigger[trigger], trigger_path, {"passed", "overall", "typos"}
            )
            exact_true(trigger_item["passed"], f"{trigger_path}.passed")
            binary_gate(
                trigger_item["overall"],
                f"{trigger_path}.overall",
                selection_familywise=selection_familywise,
            )
            binary_gate(
                trigger_item["typos"],
                f"{trigger_path}.typos",
                selection_familywise=selection_familywise,
            )
    return gate


def threshold_evidence(
    value: Any,
    triggers: tuple[str, ...],
    profiles: tuple[Any, ...],
    maximum_false_positives: int,
) -> dict[str, Any]:
    path = "manifest.threshold_selection_gate_breakdown"
    gate = exact_object(
        value,
        path,
        {"passed", "all_triggers_present", "per_trigger", "neutral", "context_stress"},
    )
    exact_true(gate["passed"], f"{path}.passed")
    exact_true(gate["all_triggers_present"], f"{path}.all_triggers_present")
    per_trigger = selection_trigger_map(
        gate["per_trigger"],
        f"{path}.per_trigger",
        triggers,
        maximum_false_positives,
    )
    neutral = exact_object(
        gate["neutral"],
        f"{path}.neutral",
        {"passed", "all_triggers_present", "per_trigger"},
    )
    exact_true(neutral["passed"], f"{path}.neutral.passed")
    exact_true(
        neutral["all_triggers_present"],
        f"{path}.neutral.all_triggers_present",
    )
    neutral_triggers = selection_trigger_map(
        neutral["per_trigger"],
        f"{path}.neutral.per_trigger",
        triggers,
        maximum_false_positives,
    )
    if neutral_triggers != per_trigger:
        fail(f"{path}.neutral.per_trigger must equal the signed root evidence")
    context_gate(
        gate["context_stress"],
        f"{path}.context_stress",
        triggers,
        profiles,
        selection_familywise=True,
    )
    return gate


def veto_result(value: Any, path: str, *, includes_passed: bool) -> dict[str, Any]:
    fields = {
        "raw_logit",
        "positive_samples",
        "vetoed_positive_samples",
        "false_negative_rate",
    }
    if includes_passed:
        fields.add("passed")
    result = exact_object(value, path, fields)
    if includes_passed:
        exact_true(result["passed"], f"{path}.passed")
    finite_number(result["raw_logit"], f"{path}.raw_logit")
    positives = nonnegative_integer(result["positive_samples"], f"{path}.positive_samples")
    vetoed = nonnegative_integer(
        result["vetoed_positive_samples"], f"{path}.vetoed_positive_samples"
    )
    if positives == 0 or vetoed > positives:
        fail(f"{path} must contain a non-empty, consistent positive sample")
    rate = fraction(result["false_negative_rate"], f"{path}.false_negative_rate")
    if not math.isclose(rate, vetoed / positives, rel_tol=0.0, abs_tol=1e-15):
        fail(f"{path}.false_negative_rate is inconsistent")
    return result


def quality_evidence(
    value: Any,
    triggers: tuple[str, ...],
    profiles: tuple[Any, ...],
) -> dict[str, Any]:
    path = "manifest.quality_gate_breakdown"
    gate = exact_object(
        value,
        path,
        {
            "passed",
            "all_triggers_present",
            "sealed_test",
            "sealed_test_typos",
            "sealed_test_context_stress",
            "safety",
            "veto",
        },
    )
    exact_true(gate["passed"], f"{path}.passed")
    exact_true(gate["all_triggers_present"], f"{path}.all_triggers_present")
    for field in ("sealed_test", "sealed_test_typos"):
        section_path = f"{path}.{field}"
        section = exact_object(
            gate[field], section_path, {"passed", "per_trigger"}
        )
        exact_true(section["passed"], f"{section_path}.passed")
        binary_trigger_map(section["per_trigger"], f"{section_path}.per_trigger", triggers)
    context_gate(
        gate["sealed_test_context_stress"],
        f"{path}.sealed_test_context_stress",
        triggers,
        profiles,
        selection_familywise=False,
    )
    safety = exact_object(
        gate["safety"],
        f"{path}.safety",
        {"passed", "actual_guard_failures", "maximum_guard_failures"},
    )
    exact_true(safety["passed"], f"{path}.safety.passed")
    actual_failures = nonnegative_integer(
        safety["actual_guard_failures"], f"{path}.safety.actual_guard_failures"
    )
    maximum_failures = nonnegative_integer(
        safety["maximum_guard_failures"], f"{path}.safety.maximum_guard_failures"
    )
    if actual_failures > maximum_failures:
        fail(f"{path}.safety exceeds its failure limit")
    veto = exact_object(
        gate["veto"],
        f"{path}.veto",
        {"passed", "selection", "sealed_test", "maximum_false_negative_rate"},
    )
    exact_true(veto["passed"], f"{path}.veto.passed")
    maximum_veto_rate = fraction(
        veto["maximum_false_negative_rate"],
        f"{path}.veto.maximum_false_negative_rate",
    )
    for field in ("selection", "sealed_test"):
        result = veto_result(
            veto[field], f"{path}.veto.{field}", includes_passed=True
        )
        if result["false_negative_rate"] > maximum_veto_rate:
            fail(f"{path}.veto.{field} exceeds its failure limit")
    return gate


def main() -> None:
    if len(sys.argv) != 5:
        fail("expected project, config, manifest and artifact paths")
    project, config_path, manifest_path, artifact_path = map(Path, sys.argv[1:])
    sys.path.insert(0, str(project / "tools"))
    sys.path.insert(0, str(project / "src"))

    from keyswitch.intent_model import (
        MAX_CONTAINER_BYTES,
        MAX_MANIFEST_BYTES,
        MAX_PAYLOAD_BYTES,
        MAX_SUPPORTED_FINGERPRINTS,
        SCHEMA_VERSION,
        LinearNgramModel,
        TRIGGERS,
        stable_sigmoid,
    )
    from train_intent_model import (
        CONTEXT_STRESS_PROFILES,
        gate_policy_payload,
        load_training_config_snapshot,
    )

    config, config_digest = load_training_config_snapshot(config_path)
    runtime_limits = (
        SCHEMA_VERSION,
        MAX_CONTAINER_BYTES,
        MAX_MANIFEST_BYTES,
        MAX_PAYLOAD_BYTES,
        MAX_SUPPORTED_FINGERPRINTS,
    )
    packaging_limits = (
        KSLM_SCHEMA,
        KSLM_MAXIMUM_CONTAINER_BYTES,
        KSLM_MAXIMUM_MANIFEST_BYTES,
        KSLM_MAXIMUM_PAYLOAD_BYTES,
        KSLM_MAXIMUM_FINGERPRINTS,
    )
    if runtime_limits != packaging_limits:
        fail("KSLM runtime and Windows packaging bounds differ")
    artifact_bytes = verify_kslm_packaging_bounds(artifact_path)
    if type(config.schema_version) is not int or config.schema_version != 13:
        fail("Windows packaging requires training config schema 13")
    manifest = strict_json(manifest_path, 1024 * 1024, "intent manifest")
    if type(manifest.get("schema_version")) is not int or manifest["schema_version"] != 1:
        fail("intent manifest schema must be exact integer 1")

    if manifest.get("config_sha256") != config_digest:
        fail("intent manifest config_sha256 does not match config.json")
    if manifest.get("gate_policy") != gate_policy_payload(config):
        fail("intent manifest gate_policy does not match config.json")
    exact_true(manifest.get("quality_gates_passed"), "manifest.quality_gates_passed")

    sealed = exact_object(
        manifest.get("sealed_evaluation"),
        "manifest.sealed_evaluation",
        {
            "schema_version",
            "split_namespace",
            "candidate_sha256",
            "registry_path",
            "candidate_dataset_sha256",
            "config_sha256",
            "registry_sha256",
        },
    )
    if sealed["schema_version"] != 1:
        fail("manifest.sealed_evaluation schema must be exact integer 1")
    for field in (
        "candidate_sha256",
        "candidate_dataset_sha256",
        "config_sha256",
        "registry_sha256",
    ):
        exact_sha256(sealed[field], f"manifest.sealed_evaluation.{field}")
    if (
        sealed["split_namespace"] != config.sealed_evaluation.split_namespace
        or sealed["registry_path"] != config.sealed_evaluation.registry_path
        or sealed["config_sha256"] != config_digest
    ):
        fail("manifest sealed evaluation differs from config.json")
    registry_relative = Path(config.sealed_evaluation.registry_path)
    if registry_relative.is_absolute() or ".." in registry_relative.parts:
        fail("sealed registry path must be repository-relative")
    project_root = project.resolve()
    registry_path = (project_root / registry_relative).resolve()
    try:
        registry_path.relative_to(project_root)
    except ValueError as error:
        raise ValueError("sealed registry escapes the repository") from error
    registry_bytes = bounded_bytes(
        registry_path, 16 * 1024, "immutable seal registry"
    )
    if hashlib.sha256(registry_bytes).hexdigest() != sealed["registry_sha256"]:
        fail("immutable seal registry SHA-256 differs from the manifest")
    registry = strict_json(
        registry_path, 16 * 1024, "immutable seal registry"
    )
    expected_registry = {
        "schema_version": sealed["schema_version"],
        "split_namespace": sealed["split_namespace"],
        "candidate_sha256": sealed["candidate_sha256"],
        "config_sha256": sealed["config_sha256"],
        "candidate_dataset_sha256": sealed["candidate_dataset_sha256"],
    }
    if registry != expected_registry:
        fail("immutable seal registry differs from the signed manifest receipt")

    toolchain = manifest.get("toolchain")
    if type(toolchain) is not dict:
        fail("manifest.toolchain must be an object")
    toolchain_paths = {
        "trainer_sha256": "tools/train_intent_model.py",
        "runtime_sha256": "src/keyswitch/intent_model.py",
        "detector_sha256": "src/keyswitch/detector.py",
        "protected_tokens_sha256": "src/keyswitch/resources/protected_tokens.txt",
        "layouts_sha256": "src/keyswitch/layouts.py",
        "language_model_sha256": "src/keyswitch/language_model.py",
        "evaluator_sha256": "tools/evaluate_intent_model.py",
        "preseal_generator_sha256": "tools/preseal_intent_holdout.py",
        "development_freezer_sha256": "tools/freeze_intent_development_corpus.py",
        "preseal_receipt_sha256": "model/intent_v1/holdout-v20-preseal.json",
    }
    for field, relative_path in toolchain_paths.items():
        expected_digest = exact_sha256(
            toolchain.get(field), f"manifest.toolchain.{field}"
        )
        actual_digest = hashlib.sha256(
            bounded_bytes(
                project_root / relative_path,
                8 * 1024 * 1024,
                f"model toolchain file {relative_path}",
            )
        ).hexdigest()
        if actual_digest != expected_digest:
            fail(f"model toolchain file differs from manifest: {relative_path}")

    triggers = tuple(TRIGGERS)
    if len(triggers) * 2 != SELECTION_FALSE_POSITIVE_COMPARISONS:
        fail("Windows packaging selection comparison count differs from runtime")
    profiles = tuple(CONTEXT_STRESS_PROFILES)
    threshold_gate = threshold_evidence(
        manifest.get("threshold_selection_gate_breakdown"),
        triggers,
        profiles,
        config.selection_maximum_false_positives_per_trigger,
    )
    quality_gate = quality_evidence(
        manifest.get("quality_gate_breakdown"), triggers, profiles
    )

    thresholds = exact_trigger_map(manifest.get("thresholds"), "manifest.thresholds", triggers)
    selected_margins: set[float] = set()
    for trigger in triggers:
        path = f"manifest.thresholds.{trigger}"
        item = exact_object(
            thresholds[trigger],
            path,
            {"global_logit_margin", "logits", "confidences", "selection_metrics", "selection_typo_metrics"},
        )
        margin = finite_number(
            item["global_logit_margin"], f"{path}.global_logit_margin"
        )
        if margin < 0.0 or margin > config.threshold_logit_margin_cap:
            fail(f"{path}.global_logit_margin is outside the signed cap")
        selected_margins.add(margin)
        logits = exact_object(item["logits"], f"{path}.logits", set(DIRECTIONS))
        confidences = exact_object(
            item["confidences"], f"{path}.confidences", set(DIRECTIONS)
        )
        for direction in DIRECTIONS:
            logit = finite_number(
                logits[direction], f"{path}.logits.{direction}"
            )
            confidence = fraction(
                confidences[direction], f"{path}.confidences.{direction}"
            )
            if not math.isclose(
                confidence, stable_sigmoid(logit), rel_tol=0.0, abs_tol=1e-15
            ):
                fail(
                    f"{path}.confidences.{direction} is inconsistent with its logit"
                )
        selection_metrics = metric_payload(
            item["selection_metrics"], f"{path}.selection_metrics"
        )
        selection_typo_metrics = metric_payload(
            item["selection_typo_metrics"], f"{path}.selection_typo_metrics"
        )
        gate_item = threshold_gate["per_trigger"][trigger]
        if gate_item["logits"] != item["logits"]:
            fail(f"{path}.logits differ from threshold gate evidence")
        if gate_item["overall"]["actual"] != selection_metrics:
            fail(f"{path}.selection_metrics differs from threshold gate evidence")
        if gate_item["typos"]["actual"] != selection_typo_metrics:
            fail(f"{path}.selection_typo_metrics differs from threshold gate evidence")
    if len(selected_margins) != 1:
        fail("manifest thresholds must share one global logit margin")

    for field, quality_field in (
        ("sealed_test", "sealed_test"),
        ("sealed_test_typos", "sealed_test_typos"),
    ):
        metrics = exact_trigger_map(manifest.get(field), f"manifest.{field}", triggers)
        for trigger in triggers:
            actual = metric_payload(metrics[trigger], f"manifest.{field}.{trigger}")
            gate_actual = quality_gate[quality_field]["per_trigger"][trigger]["actual"]
            if actual != gate_actual:
                fail(f"manifest.{field}.{trigger} differs from quality gate evidence")

    raw_context = exact_object(
        manifest.get("sealed_test_context_stress"),
        "manifest.sealed_test_context_stress",
        {profile.name for profile in profiles},
    )
    quality_context = quality_gate["sealed_test_context_stress"]["profiles"]
    for profile in profiles:
        path = f"manifest.sealed_test_context_stress.{profile.name}"
        raw_profile = exact_object(raw_context[profile.name], path, {"overall", "typos"})
        for field in ("overall", "typos"):
            raw_metrics = exact_trigger_map(
                raw_profile[field], f"{path}.{field}", triggers
            )
            for trigger in triggers:
                actual = metric_payload(
                    raw_metrics[trigger], f"{path}.{field}.{trigger}"
                )
                gate_actual = quality_context[profile.name]["per_trigger"][trigger][field]["actual"]
                if actual != gate_actual:
                    fail(f"{path}.{field}.{trigger} differs from quality gate evidence")

    raw_veto = exact_object(
        manifest.get("veto"), "manifest.veto", {"selection", "sealed_test"}
    )
    for field in ("selection", "sealed_test"):
        raw_result = veto_result(
            raw_veto[field], f"manifest.veto.{field}", includes_passed=False
        )
        quality_result = dict(quality_gate["veto"][field])
        quality_result.pop("passed")
        if raw_result != quality_result:
            fail(f"manifest.veto.{field} differs from quality gate evidence")

    artifact_digest = hashlib.sha256(artifact_bytes).hexdigest()
    if manifest.get("artifact_sha256") != artifact_digest:
        fail("intent artifact differs from its manifest")
    model = LinearNgramModel.load(artifact_path)
    if model.checksum != artifact_digest:
        fail("loaded intent model checksum differs from the artifact")
    if model.model_version != manifest.get("artifact_model_version"):
        fail("loaded intent model version differs from the manifest")
    signed_manifest = {
        key: value
        for key, value in manifest.items()
        if key not in {"artifact_sha256", "artifact_model_version"}
    }
    if model.metadata != signed_manifest:
        fail("KSLM metadata does not contain the complete signed quality evidence")
    if (
        model.dimension != config.dimension
        or model.fnv_seed != config.feature_hash_seed
        or model.membership_seed != config.membership_hash_seed
    ):
        fail("KSLM feature schema differs from config.json")
    for trigger in triggers:
        for direction in DIRECTIONS:
            if (
                model.threshold_logits[trigger][direction]
                != thresholds[trigger]["logits"][direction]
            ):
                fail(
                    f"KSLM threshold logit differs for {trigger}/{direction}"
                )
    if model.veto_threshold != raw_veto["selection"]["raw_logit"]:
        fail("KSLM veto threshold differs from selection evidence")
    calibration = exact_object(
        manifest.get("calibration"),
        "manifest.calibration",
        {
            "schema_version",
            "method",
            "provenance",
            "sample_count",
            "positive_count",
            "by_direction",
        },
    )
    if (
        calibration["schema_version"] != 1
        or calibration["method"]
        != "independent-platt-by-layout-direction"
    ):
        fail("signed calibration method is unsupported")
    directional = exact_object(
        calibration["by_direction"],
        "manifest.calibration.by_direction",
        {"0>1", "1>0"},
    )
    for direction in ("0>1", "1>0"):
        signed = exact_object(
            directional[direction],
            f"manifest.calibration.by_direction.{direction}",
            {"slope", "intercept", "sample_count", "positive_count", "provenance"},
        )
        runtime = model.platt_calibration[direction]
        if runtime.scale != signed["slope"] or runtime.bias != signed["intercept"]:
            fail(f"KSLM calibration differs for direction {direction}")
    training = manifest.get("training")
    if type(training) is not dict or model.bias != training.get("bias"):
        fail("KSLM bias differs from signed training evidence")
    print(model.model_version)


if __name__ == "__main__":
    main()
'@

$ProjectDirectory = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$FrozenModelDirectory = Join-Path $ProjectDirectory "model\intent_v1\sources"
if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path $ProjectDirectory "dist"
}
if (-not $ModelDirectory) {
    $ModelDirectory = $FrozenModelDirectory
}
if (-not $ModelLicense) {
    $ModelLicense = Join-Path $FrozenModelDirectory "COPYRIGHT.onboard-data"
}
$IntentModel = Join-Path $ProjectDirectory "src\keyswitch\resources\models\layout_intent_v1.ksm"
$IntentManifest = Join-Path $ProjectDirectory "model\intent_v1\manifest.json"
$IntentConfig = Join-Path $ProjectDirectory "model\intent_v1\config.json"
$IntentModelMaximumBytes = 14MB

$StrictUtf8 = [System.Text.UTF8Encoding]::new($false, $true)
[byte[]]$PyProjectBytes = Read-BoundedFileBytes `
    -Path (Join-Path $ProjectDirectory "pyproject.toml") `
    -MaximumBytes 1MB `
    -MinimumBytes 1 `
    -Label "pyproject.toml"
[byte[]]$ModuleBytes = Read-BoundedFileBytes `
    -Path (Join-Path $ProjectDirectory "src\keyswitch\__init__.py") `
    -MaximumBytes 64KB `
    -MinimumBytes 1 `
    -Label "keyswitch package version module"
try {
    $PyProject = $StrictUtf8.GetString($PyProjectBytes)
    $Module = $StrictUtf8.GetString($ModuleBytes)
}
catch {
    throw "KeySwitch version sources must be valid UTF-8"
}
$VersionMatch = [regex]::Match($PyProject, '(?m)^version = "([^"]+)"\r?$')
$ModuleMatch = [regex]::Match($Module, '(?m)^__version__ = "([^"]+)"\r?$')
if (-not $VersionMatch.Success -or -not $ModuleMatch.Success) {
    throw "Cannot read KeySwitch version"
}
$Version = $VersionMatch.Groups[1].Value
if ($Version -ne $ModuleMatch.Groups[1].Value) {
    throw "Version mismatch between pyproject.toml and keyswitch.__init__"
}
if ($Version -notmatch '^\d+\.\d+\.\d+$') {
    throw "Windows packaging requires a three-part numeric version"
}

$IntentConfigObject = Read-BoundedJsonObject `
    -Path $IntentConfig `
    -MaximumBytes 64KB `
    -Label "intent-model training config"
if ($IntentConfigObject.schema_version -is [bool] -or $IntentConfigObject.schema_version -ne 13) {
    throw "Windows packaging requires intent-model training config schema 13"
}
$EnglishSourcePolicy = $IntentConfigObject.sources.languages.en_US
$RussianSourcePolicy = $IntentConfigObject.sources.languages.ru_RU
$LicenseSourcePolicy = $IntentConfigObject.sources.license_evidence
$HardNegativeSourcePolicy = $IntentConfigObject.hard_negative_development.source
if ($EnglishSourcePolicy.path -cne "model/intent_v1/sources/en_US.lm") {
    throw "English frozen-source path differs from the packaging contract"
}
if ($RussianSourcePolicy.path -cne "model/intent_v1/sources/ru_RU.lm") {
    throw "Russian frozen-source path differs from the packaging contract"
}
if ($LicenseSourcePolicy.path -cne "model/intent_v1/sources/COPYRIGHT.onboard-data") {
    throw "Frozen license-evidence path differs from the packaging contract"
}
if ($HardNegativeSourcePolicy.path -cne "model/intent_v1/unknown-typo-development-v20.json") {
    throw "Frozen hard-negative source path differs from the packaging contract"
}

$HardNegativeSource = Join-Path $ProjectDirectory "model\intent_v1\unknown-typo-development-v20.json"
$null = Get-VerifiedFrozenFileHash `
    -Path $HardNegativeSource `
    -ExpectedBytes ([long]$HardNegativeSourcePolicy.bytes) `
    -ExpectedSha256 ([string]$HardNegativeSourcePolicy.sha256) `
    -Label "frozen hard-negative development corpus"

$EnglishModel = Join-Path $ModelDirectory "en_US.lm"
$RussianModel = Join-Path $ModelDirectory "ru_RU.lm"
$FrozenEnglishModel = Join-Path $FrozenModelDirectory "en_US.lm"
$FrozenRussianModel = Join-Path $FrozenModelDirectory "ru_RU.lm"
$FrozenEnglishHash = Get-VerifiedFrozenFileHash `
    -Path $FrozenEnglishModel `
    -ExpectedBytes ([long]$EnglishSourcePolicy.bytes) `
    -ExpectedSha256 ([string]$EnglishSourcePolicy.sha256) `
    -Label "frozen English language model"
$FrozenRussianHash = Get-VerifiedFrozenFileHash `
    -Path $FrozenRussianModel `
    -ExpectedBytes ([long]$RussianSourcePolicy.bytes) `
    -ExpectedSha256 ([string]$RussianSourcePolicy.sha256) `
    -Label "frozen Russian language model"
$ExpectedLanguageModelHashes = @{
    en_US = $FrozenEnglishHash
    ru_RU = $FrozenRussianHash
}
$SelectedEnglishHash = Get-VerifiedFrozenFileHash `
    -Path $EnglishModel `
    -ExpectedBytes ([long]$EnglishSourcePolicy.bytes) `
    -ExpectedSha256 ([string]$EnglishSourcePolicy.sha256) `
    -Label "selected English language model"
$SelectedRussianHash = Get-VerifiedFrozenFileHash `
    -Path $RussianModel `
    -ExpectedBytes ([long]$RussianSourcePolicy.bytes) `
    -ExpectedSha256 ([string]$RussianSourcePolicy.sha256) `
    -Label "selected Russian language model"
if (($SelectedEnglishHash -cne $ExpectedLanguageModelHashes.en_US) -or ($SelectedRussianHash -cne $ExpectedLanguageModelHashes.ru_RU)) {
    throw "Selected language models differ from the frozen training sources"
}

$FrozenModelLicense = Join-Path $FrozenModelDirectory "COPYRIGHT.onboard-data"
$FrozenLicenseHash = Get-VerifiedFrozenFileHash `
    -Path $FrozenModelLicense `
    -ExpectedBytes ([long]$LicenseSourcePolicy.bytes) `
    -ExpectedSha256 ([string]$LicenseSourcePolicy.sha256) `
    -Label "frozen language-model license"
$SelectedLicenseHash = Get-VerifiedFrozenFileHash `
    -Path $ModelLicense `
    -ExpectedBytes ([long]$LicenseSourcePolicy.bytes) `
    -ExpectedSha256 ([string]$LicenseSourcePolicy.sha256) `
    -Label "selected language-model license"
if ($SelectedLicenseHash -cne $FrozenLicenseHash) {
    throw "Language-model license differs from the frozen training evidence"
}
[byte[]]$ModelCopyrightBytes = Read-BoundedFileBytes `
    -Path $ModelLicense `
    -MaximumBytes ([long]$LicenseSourcePolicy.bytes) `
    -MinimumBytes ([int]$LicenseSourcePolicy.bytes) `
    -Label "selected language-model license"
try {
    $ModelCopyrightText = $StrictUtf8.GetString($ModelCopyrightBytes)
}
catch {
    throw "Language-model license is not valid UTF-8: $ModelLicense"
}
foreach ($AttributionLine in @(
    "Files: models/*",
    "Copyright: 2013, 2014, marmuta <marmvta@gmail.com>",
    "  2011, 2012, Francesco Fumanti <francesco.fumanti@gmx.net>",
    "License: GPL-3+"
)) {
    if (-not $ModelCopyrightText.Contains($AttributionLine)) {
        throw "Onboard model copyright is missing required attribution: $AttributionLine"
    }
}
$FrozenChecksumsPath = Join-Path $FrozenModelDirectory "SHA256SUMS"
[byte[]]$FrozenChecksumsBytes = Read-BoundedFileBytes `
    -Path $FrozenChecksumsPath `
    -MaximumBytes 4KB `
    -MinimumBytes 1 `
    -Label "frozen-source SHA256SUMS"
try {
    $FrozenChecksumsText = $StrictUtf8.GetString($FrozenChecksumsBytes)
}
catch {
    throw "Frozen-source SHA256SUMS is not valid UTF-8: $FrozenChecksumsPath"
}
$ExpectedChecksumsText = (
    "{0}  en_US.lm`n{1}  ru_RU.lm`n{2}  COPYRIGHT.onboard-data`n" -f `
    $EnglishSourcePolicy.sha256,
    $RussianSourcePolicy.sha256,
    $LicenseSourcePolicy.sha256
)
if ($FrozenChecksumsText -cne $ExpectedChecksumsText) {
    throw "Frozen-source SHA256SUMS differs from config.json"
}

[byte[]]$IntentModelBytes = Read-BoundedFileBytes `
    -Path $IntentModel `
    -MaximumBytes $IntentModelMaximumBytes `
    -MinimumBytes 4 `
    -Label "bundled intent model"
$IntentMagic = [System.Text.Encoding]::ASCII.GetString(
    $IntentModelBytes,
    0,
    4
)
if ($IntentMagic -ne "KSLM") {
    throw "Bundled intent model has an invalid header: $IntentModel"
}
$IntentManifestObject = Read-BoundedJsonObject `
    -Path $IntentManifest `
    -MaximumBytes 1MB `
    -Label "intent-model commit manifest"
[byte[]]$IntentConfigBytes = Read-BoundedFileBytes `
    -Path $IntentConfig `
    -MaximumBytes 64KB `
    -MinimumBytes 2 `
    -Label "intent-model training config"
$IntentConfigHash = Get-BytesSha256 -Bytes $IntentConfigBytes
if ($IntentManifestObject.config_sha256 -ne $IntentConfigHash) {
    throw "Intent-model config differs from the config hash in its commit manifest"
}
$IntentHash = Get-BytesSha256 -Bytes $IntentModelBytes
if ($IntentManifestObject.artifact_sha256 -ne $IntentHash) {
    throw "Intent-model artifact differs from its commit manifest"
}
$BuildDirectory = Join-Path $ProjectDirectory "build\windows"
if (Test-Path $BuildDirectory) {
    Remove-Item -Recurse -Force $BuildDirectory
}
New-Item -ItemType Directory -Force $BuildDirectory | Out-Null
$ModelContractValidatorPath = Join-Path `
    $BuildDirectory `
    "validate_intent_model_contract.py"
[System.IO.File]::WriteAllText(
    $ModelContractValidatorPath,
    $ModelContractValidator,
    $StrictUtf8
)
$env:PYTHONPATH = Join-Path $ProjectDirectory "src"
Invoke-NativeCommand `
    -Command "python" `
    -Arguments @((Join-Path $ProjectDirectory "tools\verify_context_model.py")) `
    -FailureMessage "Context model provenance or quality gate failed"
# Generate the OS type library before freezing. The runtime must not need
# writable installation files or a Python compiler to open accessibility.
Invoke-NativeCommand `
    -Command "python" `
    -Arguments @("-c", "import comtypes.client; comtypes.client.GetModule('UIAutomationCore.dll')") `
    -FailureMessage "UI Automation interface generation failed"
try {
    Invoke-NativeCommand `
        -Command "python" `
        -Arguments @(
            $ModelContractValidatorPath,
            $ProjectDirectory,
            $IntentConfig,
            $IntentManifest,
            $IntentModel
        ) `
        -FailureMessage "Intent-model schema, provenance or quality evidence validation failed"
}
finally {
    Remove-Item `
        -LiteralPath $ModelContractValidatorPath `
        -Force `
        -ErrorAction SilentlyContinue
}

$NativeOutput = Join-Path $BuildDirectory "native"
$Icon = Join-Path $BuildDirectory "keyswitch.ico"
$EntryPoint = Join-Path $ProjectDirectory "packaging\keyswitch_windows_entry.py"
$NativeDistribution = Join-Path $NativeOutput "keyswitch_windows_entry.dist"
$Executable = Join-Path $NativeDistribution "KeySwitch.exe"

New-Item -ItemType Directory -Force $NativeOutput, $OutputDirectory | Out-Null

Invoke-NativeCommand `
    -Command "python" `
    -Arguments @((Join-Path $ProjectDirectory "tools\create_windows_icon.py"), $Icon) `
    -FailureMessage "Windows icon generation failed"

$env:PYTHONPATH = (Join-Path $ProjectDirectory "src")
$NuitkaArguments = @(
    "-m", "nuitka",
    "--mode=standalone",
    "--lto=no",
    "--msvc=latest",
    "--assume-yes-for-downloads",
    "--enable-plugin=tk-inter",
    "--windows-console-mode=disable",
    "--windows-icon-from-ico=$Icon",
    "--company-name=Oleg Shevchuk",
    "--product-name=KeySwitch",
    "--file-description=Automatic EN/RU keyboard layout correction",
    "--file-version=$Version.0",
    "--product-version=$Version.0",
    "--copyright=GNU GPL-3.0-or-later",
    "--output-dir=$NativeOutput",
    "--output-filename=KeySwitch.exe",
    "--include-module=keyswitch.windows_ui",
    "--include-module=keyswitch.windows_native",
    "--include-module=keyswitch.windows_instance_native",
    "--include-module=keyswitch.windows_registry",
    "--include-module=keyswitch.windows_tray_native",
    "--include-module=keyswitch.intent_model",
    "--include-module=keyswitch.windows_context",
    "--include-package=comtypes",
    "--include-package=comtypes.gen",
    "--nofollow-import-to=comtypes.test",
    "--nofollow-import-to=keyswitch.atspi_context",
    "--include-module=pystray._base",
    "--include-module=pystray._util",
    "--include-module=pystray._util.win32",
    "--include-module=pystray._win32",
    "--include-package=PIL",
    # Includes the in-package KSLM; the post-build SHA-256 check verifies it.
    "--include-package-data=keyswitch",
    "--include-data-files=$(Join-Path $ModelDirectory 'en_US.lm')=keyswitch/resources/models/en_US.lm",
    "--include-data-files=$(Join-Path $ModelDirectory 'ru_RU.lm')=keyswitch/resources/models/ru_RU.lm",
    "--nofollow-import-to=keyswitch.app",
    "--nofollow-import-to=keyswitch.ui",
    "--nofollow-import-to=keyswitch.tray",
    "--nofollow-import-to=keyswitch.x11_backend",
    "--nofollow-import-to=pystray._appindicator",
    "--nofollow-import-to=pystray._darwin",
    "--nofollow-import-to=pystray._dummy",
    "--nofollow-import-to=pystray._gtk",
    "--nofollow-import-to=pystray._xorg",
    "--no-progressbar",
    "--report=$(Join-Path $NativeOutput 'compilation-report.xml')",
    $EntryPoint
)
Invoke-NativeCommand `
    -Command "python" `
    -Arguments $NuitkaArguments `
    -FailureMessage "Nuitka Windows build failed"
if (-not (Test-Path $Executable -PathType Leaf)) {
    throw "Nuitka did not produce $Executable"
}

$UnexpectedPython = Get-ChildItem $NativeDistribution -Recurse -File | Where-Object {
    $_.Extension -in @(".py", ".pyc", ".pyo")
}
if ($UnexpectedPython) {
    throw "Native distribution contains Python source or bytecode"
}

$ProductVersion = (Get-Item $Executable).VersionInfo.ProductVersion
if ($ProductVersion -ne "$Version.0") {
    throw "Unexpected executable product version: $ProductVersion"
}

$BundledIntentModel = Join-Path $NativeDistribution "keyswitch\resources\models\layout_intent_v1.ksm"
$BundledContextModel = Join-Path $NativeDistribution "keyswitch\resources\models\context_policy_v1.json"
$ContextReport = Read-BoundedJsonObject `
    -Path (Join-Path $ProjectDirectory "model\context_v1\report.json") `
    -MaximumBytes 1MB -Label "context-model quality report"
[byte[]]$BundledContextBytes = Read-BoundedFileBytes `
    -Path $BundledContextModel -MaximumBytes 8MB -MinimumBytes 2 `
    -Label "bundled contextual model"
if ((Get-BytesSha256 -Bytes $BundledContextBytes) -cne $ContextReport.artifact_sha256) {
    throw "Native distribution contains a different contextual model"
}
[byte[]]$BundledIntentModelBytes = Read-BoundedFileBytes `
    -Path $BundledIntentModel `
    -MaximumBytes $IntentModelMaximumBytes `
    -MinimumBytes 4 `
    -Label "native-distribution intent model"
$BundledIntentMagic = [System.Text.Encoding]::ASCII.GetString(
    $BundledIntentModelBytes,
    0,
    4
)
if ($BundledIntentMagic -ne "KSLM") {
    throw "Bundled native intent model has an invalid header"
}
if ((Get-BytesSha256 -Bytes $BundledIntentModelBytes) -cne $IntentHash) {
    throw "Native distribution contains a different intent model"
}
$BundledEnglishModel = Join-Path $NativeDistribution "keyswitch\resources\models\en_US.lm"
$BundledRussianModel = Join-Path $NativeDistribution "keyswitch\resources\models\ru_RU.lm"
$BundledEnglishHash = Get-VerifiedFrozenFileHash `
    -Path $BundledEnglishModel `
    -ExpectedBytes ([long]$EnglishSourcePolicy.bytes) `
    -ExpectedSha256 ([string]$EnglishSourcePolicy.sha256) `
    -Label "native-distribution English language model"
$BundledRussianHash = Get-VerifiedFrozenFileHash `
    -Path $BundledRussianModel `
    -ExpectedBytes ([long]$RussianSourcePolicy.bytes) `
    -ExpectedSha256 ([string]$RussianSourcePolicy.sha256) `
    -Label "native-distribution Russian language model"
if (($BundledEnglishHash -cne $ExpectedLanguageModelHashes.en_US) -or ($BundledRussianHash -cne $ExpectedLanguageModelHashes.ru_RU)) {
    throw "Native distribution contains different frozen language models"
}

$PostBuildDiagnosticsPath = Join-Path $BuildDirectory "post-build-diagnostics.json"
$PostBuildDiagnosticsErrorPath = Join-Path $BuildDirectory "post-build-diagnostics.stderr.txt"
$PreviousIntentModelOverride = $env:KEYSWITCH_INTENT_MODEL_PATH
$PreviousConfigDirectory = $env:KEYSWITCH_CONFIG_DIR
$PreviousDataDirectory = $env:KEYSWITCH_DATA_DIR
try {
    Remove-Item Env:KEYSWITCH_INTENT_MODEL_PATH -ErrorAction SilentlyContinue
    $env:KEYSWITCH_CONFIG_DIR = Join-Path $BuildDirectory "post-build-config"
    $env:KEYSWITCH_DATA_DIR = Join-Path $BuildDirectory "post-build-data"
    New-Item -ItemType Directory -Force $env:KEYSWITCH_CONFIG_DIR, $env:KEYSWITCH_DATA_DIR | Out-Null
    $PostBuildDiagnosticsProcess = Start-Process `
        -FilePath $Executable `
        -ArgumentList "--diagnose" `
        -RedirectStandardOutput $PostBuildDiagnosticsPath `
        -RedirectStandardError $PostBuildDiagnosticsErrorPath `
        -PassThru
    if (-not $PostBuildDiagnosticsProcess.WaitForExit(30000)) {
        $PostBuildDiagnosticsProcess.Kill()
        $PostBuildDiagnosticsProcess.WaitForExit()
        throw "Post-build executable diagnostics timed out after 30 seconds"
    }
}
finally {
    if ($null -eq $PreviousIntentModelOverride) {
        Remove-Item Env:KEYSWITCH_INTENT_MODEL_PATH -ErrorAction SilentlyContinue
    }
    else {
        $env:KEYSWITCH_INTENT_MODEL_PATH = $PreviousIntentModelOverride
    }
    if ($null -eq $PreviousConfigDirectory) {
        Remove-Item Env:KEYSWITCH_CONFIG_DIR -ErrorAction SilentlyContinue
    }
    else {
        $env:KEYSWITCH_CONFIG_DIR = $PreviousConfigDirectory
    }
    if ($null -eq $PreviousDataDirectory) {
        Remove-Item Env:KEYSWITCH_DATA_DIR -ErrorAction SilentlyContinue
    }
    else {
        $env:KEYSWITCH_DATA_DIR = $PreviousDataDirectory
    }
}
if ($PostBuildDiagnosticsProcess.ExitCode -notin @(0, 1)) {
    try {
        [byte[]]$PostBuildDiagnosticsErrorBytes = Read-BoundedFileBytes `
            -Path $PostBuildDiagnosticsErrorPath `
            -MaximumBytes 1MB `
            -MinimumBytes 0 `
            -Label "post-build diagnostics stderr"
        $PostBuildDiagnosticsError = $StrictUtf8.GetString($PostBuildDiagnosticsErrorBytes)
    }
    catch {
        $PostBuildDiagnosticsError = "<unavailable or oversized diagnostics stderr>"
    }
    throw "Post-build executable diagnostics failed with exit code $($PostBuildDiagnosticsProcess.ExitCode): $PostBuildDiagnosticsError"
}
$PostBuildDiagnostics = Read-BoundedJsonObject `
    -Path $PostBuildDiagnosticsPath `
    -MaximumBytes 1MB `
    -Label "post-build executable diagnostics"
if ($PostBuildDiagnostics.context_model.available -ne $true -or $PostBuildDiagnostics.context_model.status -cne $ContextReport.model_version) {
    throw "Post-build executable cannot load its exact contextual model"
}
if ($PostBuildDiagnostics.context_field_access.available -ne $true) {
    throw "Post-build executable cannot initialize its bundled UI Automation bridge"
}
if ($PostBuildDiagnostics.intent_model.available -isnot [bool] -or -not $PostBuildDiagnostics.intent_model.available) {
    throw "Post-build executable cannot load its bundled intent model: $($PostBuildDiagnostics.intent_model.error)"
}
if ($PostBuildDiagnostics.intent_model.checksum -ne $IntentHash) {
    throw "Post-build executable loaded an unexpected intent-model checksum"
}
if ($PostBuildDiagnostics.intent_model.version -ne $IntentManifestObject.artifact_model_version) {
    throw "Post-build executable loaded an unexpected intent-model version"
}
$ResolvedDiagnosticModelPath = [System.IO.Path]::GetFullPath([string]$PostBuildDiagnostics.intent_model.path)
$ResolvedBundledIntentModel = [System.IO.Path]::GetFullPath($BundledIntentModel)
if (-not $ResolvedDiagnosticModelPath.Equals($ResolvedBundledIntentModel, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Post-build executable did not load the exact in-distribution intent model"
}

$LicenseDirectory = Join-Path $NativeDistribution "licenses"
New-Item -ItemType Directory -Force $LicenseDirectory | Out-Null
Invoke-NativeCommand `
    -Command "python" `
    -Arguments @(
        (Join-Path $ProjectDirectory "tools\collect_python_licenses.py"),
        $LicenseDirectory,
        "Nuitka",
        "comtypes",
        "pystray",
        "Pillow",
        "six"
    ) `
    -FailureMessage "Third-party license collection failed"
Copy-Item (Join-Path $ProjectDirectory "LICENSE") (Join-Path $LicenseDirectory "LICENSE.KeySwitch.txt")
Copy-Item (Join-Path $ProjectDirectory "README.en.md") (Join-Path $NativeDistribution "README.en.md")
Copy-Item (Join-Path $ProjectDirectory "README.md") (Join-Path $NativeDistribution "README.md")
$BundledModelLicense = Join-Path $LicenseDirectory "COPYRIGHT.onboard-data.txt"
Copy-Item $ModelLicense $BundledModelLicense
$BundledLicenseHash = Get-VerifiedFrozenFileHash `
    -Path $BundledModelLicense `
    -ExpectedBytes ([long]$LicenseSourcePolicy.bytes) `
    -ExpectedSha256 ([string]$LicenseSourcePolicy.sha256) `
    -Label "native-distribution language-model license"
if ($BundledLicenseHash -cne $FrozenLicenseHash) {
    throw "Native distribution contains different language-model license evidence"
}

$ZipPath = Join-Path $OutputDirectory "KeySwitch-$Version-windows-x64.zip"
if (Test-Path $ZipPath) {
    Remove-Item -Force $ZipPath
}
Compress-Archive -Path (Join-Path $NativeDistribution "*") -DestinationPath $ZipPath -CompressionLevel Optimal

$IsccCommand = Get-Command ISCC.exe -ErrorAction SilentlyContinue
$IsccPath = if ($IsccCommand) { $IsccCommand.Source } else { "" }
if (-not $IsccPath) {
    $DefaultIscc = Join-Path ${env:ProgramFiles(x86)} "Inno Setup 6\ISCC.exe"
    if (Test-Path $DefaultIscc -PathType Leaf) {
        $IsccPath = $DefaultIscc
    }
}
if (-not $IsccPath) {
    throw "Inno Setup 6 compiler (ISCC.exe) is required"
}

$InstallerScript = Join-Path $ProjectDirectory "packaging\windows\KeySwitch.iss"
Invoke-NativeCommand `
    -Command $IsccPath `
    -Arguments @(
        "/DMyAppVersion=$Version",
        "/DSourceDir=$NativeDistribution",
        "/DOutputDir=$OutputDirectory",
        "/DSetupIcon=$Icon",
        $InstallerScript
    ) `
    -FailureMessage "Inno Setup build failed"

$Installer = Join-Path $OutputDirectory "KeySwitch-Setup-$Version-x64.exe"
if (-not (Test-Path $Installer -PathType Leaf)) {
    throw "Inno Setup did not produce $Installer"
}

Write-Host "Built $ZipPath"
Write-Host "Built $Installer"
