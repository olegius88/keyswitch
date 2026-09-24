#!/usr/bin/env python3
"""Export public aggregate evidence and verify an accepted context-v3 artifact."""
from __future__ import annotations

import argparse
import ast
from collections.abc import Mapping, Sequence
import hashlib
import json
import math
from pathlib import Path
import re
from typing import cast

from keyswitch.constants.model_protocol import PROFILES, SEALED_BEFORE_TEST
from keyswitch.context_model import ARTIFACT_PATH, ContextModel
from keyswitch.prefix_model import ARTIFACT as PREFIX_ARTIFACT_PATH
from keyswitch.prefix_schema import VersionedPrefixModel
from keyswitch.constants.file_formats import (
    FULL_REPORT_JSON_LIMIT_BYTES,
    HASH_CHUNK_BYTES,
    METADATA_JSON_LIMIT_BYTES,
    MODEL_ARTIFACT_JSON_LIMIT_BYTES,
    VERSION_HASH_CHARACTERS,
)
from keyswitch.constants.models import CONTEXT_ACTION_FEATURE_VERSION
from keyswitch.constants.training import (
    CONTEXT_ACTION_GATE_POLICY,
    CONTEXT_ACTION_PROTOCOL_PHYSICAL_KEY_PERIOD_MS,
    CONTEXT_ACTION_SEQUENCE_PROTOCOL_VERSION,
    MINIMUM_SEQUENCE_DOCUMENTS_PER_GROUP,
    SEQUENCE_DOCUMENT_CAP_PER_GROUP,
    SEQUENCE_MAXIMUM_SELECTED_ROWS,
    SIMULATED_KEY_DOWN_SECONDS,
    SIMULATED_KEY_UP_SECONDS,
    SIMULATED_WORD_IDLE_SECONDS,
    TRIMMED_LEFT_FIELD,
    TRIMMED_RIGHT_FIELD,
    VERSION_MODULE_STATEMENT_COUNT,
)
from keyswitch.constants.units import MILLISECONDS_PER_SECOND

ROOT = Path(__file__).resolve().parents[1]
RECEIPT = ROOT / "model/context_v3/release-receipt.json"
BASELINE = "model/context_v3/baseline-context-v1.json"
PREFIX_BASELINE = "model/prefix_v2/baseline-prefix-v1.json"
INSTALLED_MODELS = ("context_policy_v1.json", "prefix_policy_v1.json")
RECIPE = "model/context_v3/recipe.json"
# model/context_v2/candidate-seal.json and report.json: rejected experiment.
REJECTED_ARTIFACT = "55f0d735de8751f3f414d569b6e1eb2bbb294303d8fe01f2fde66c3fadecfeb1"
COUNTS = frozenset({"rows", "initially_correct", "initially_wrong", "preserved_correct", "exactly_restored",
                    "correct_text_corruptions", "length_mismatches", "injections", "execution_errors",
                    "correction_layout_mismatches", "final_layout_mismatches"})
CALIBRATION = frozenset({"rows", "convert_rows", "converted_correctly", "false_conversions", "conversion_recall"})
SCOPE = "Sealed private-corpus test aggregates and current artifact/provenance verification; fresh-fit reproducibility, reviewed human-intent labels and native OS execution are not asserted."
AUDITED_SEQUENCE_PROTOCOL_SHA256 = "59b9e6e9d4ca222fb732d9c59d9aac68b6081867fdbfe119f3aa7f7547636fab"
PROTOCOL: dict[str, object] = {
    "version": CONTEXT_ACTION_SEQUENCE_PROTOCOL_VERSION,
    "document_cap_per_group": SEQUENCE_DOCUMENT_CAP_PER_GROUP, "profiles": list(PROFILES),
    "window": "bounded_sentence", "backend": "simulated_editor",
    "physical_key_period_ms": CONTEXT_ACTION_PROTOCOL_PHYSICAL_KEY_PERIOD_MS,
    "trim_clipped_token_edges": True, "native_execution_verified": False,
    "test_access": SEALED_BEFORE_TEST, "fresh_fit_reproducibility_verified": False,
    "source_protocol_sha256": AUDITED_SEQUENCE_PROTOCOL_SHA256,
    "wrong_intervention": "first_declared_group_letter_after_literal_focus_prefix",
    "settings_modes": ["early_off", "default"],
    "default_key_down_ms": round(SIMULATED_KEY_DOWN_SECONDS * MILLISECONDS_PER_SECOND),
    "default_key_up_ms": round(SIMULATED_KEY_UP_SECONDS * MILLISECONDS_PER_SECOND),
    "default_word_idle_ms": round(SIMULATED_WORD_IDLE_SECONDS * MILLISECONDS_PER_SECOND),
    "models": "explicit_context_and_prefix_pairs",
    "acceptance": "candidate_pair_against_baseline_pair_in_both_modes_and_profiles",
    "restoration_gate": "net_restorations_exactly_restored_minus_correct_text_corruptions_at_least_baseline",
    "identifier_lexicon": "packaged_identifiers_json_as_lexical_evidence_for_both_readings",
}
FIELDS = frozenset({"schema_version", "feature_version", "model_version", "conversion_threshold",
                    "artifact_sha256", "weights_sha256", "baseline_path", "baseline_sha256",
                    "recipe_path", "recipe_sha256", "candidate_seal_sha256", "full_test_report_sha256",
                    "corpus_manifest_sha256", "provenance", "calibration", "test", "gate_policy",
                    "protocol", "scope", "quality_gates_passed",
                    "prefix_model_version", "prefix_feature_version", "prefix_conversion_threshold",
                    "prefix_artifact_sha256", "prefix_weights_sha256", "prefix_candidate_seal_sha256",
                    "prefix_baseline_path", "prefix_baseline_sha256"})


def canonical(value: object) -> bytes:
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"), allow_nan=False) + "\n").encode()


def checksum(path: Path) -> str:
    result = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(HASH_CHUNK_BYTES), b""):
            result.update(block)
    return result.hexdigest()


def mapping(value: object, label: str, fields: frozenset[str] | None = None) -> dict[str, object]:
    if not isinstance(value, dict) or any(not isinstance(key, str) for key in value):
        raise ValueError("invalid object: " + label)
    result = cast(dict[str, object], value)
    if fields is not None and set(result) != fields:
        raise ValueError("missing or extra fields: " + label)
    return result


def unique_pairs(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for name, value in pairs:
        if name in result:
            raise ValueError("duplicate JSON field: " + name)
        result[name] = value
    return result


def read_object(path: Path, limit: int = METADATA_JSON_LIMIT_BYTES) -> dict[str, object]:
    with path.open("rb") as stream:
        raw = stream.read(limit + 1)
    if len(raw) > limit:
        raise ValueError("oversized evidence metadata")
    value: object = json.loads(raw, object_pairs_hook=unique_pairs)
    return mapping(value, "metadata")


def integer(value: object, label: str) -> int:
    if type(value) is not int or value < 0:
        raise ValueError("invalid count: " + label)
    return value


def sha(value: object) -> str:
    if not isinstance(value, str) or re.fullmatch(r"[a-f0-9]{64}", value) is None:
        raise ValueError("invalid SHA256")
    return value


def public_path(root: Path, name: str) -> Path:
    path = Path(name)
    resolved = (root / path).resolve()
    target_parts = resolved.relative_to(root.resolve()).parts if resolved.is_relative_to(root.resolve()) else ()
    if (not re.fullmatch(r"[A-Za-z0-9_./-]+", name) or path.is_absolute() or path.as_posix() != name
            or path.parts[0] not in {"src", "tools", "tests", "model"}
            or any(part.startswith(".") for part in path.parts)
            or not target_parts or target_parts[0] not in {"src", "tools", "tests", "model"}
            or any(part.startswith(".") for part in target_parts)):
        raise ValueError("nonpublic provenance path")
    return root / path


# The release process rewrites the version literal in every release, and that file holds
# nothing else. Pinning it would tie a model's evidence to the release number it happened
# to be sealed under: a pair could never ship in a later version without a new sealed test,
# which is a test spent on a string. It is excluded and its shape is checked instead, so
# nothing can be hidden in the one file the receipt does not fingerprint.
VERSION_MODULE = "src/keyswitch/__init__.py"


def version_module_holds_only_a_version(root: Path) -> bool:
    """True when the excluded module is a docstring and one string assignment, nothing else."""
    try:
        body = ast.parse((root / VERSION_MODULE).read_text(encoding="utf-8")).body
    except (OSError, SyntaxError, ValueError):
        return False
    if (len(body) != VERSION_MODULE_STATEMENT_COUNT or not isinstance(body[0], ast.Expr)
            or not isinstance(body[0].value, ast.Constant)):
        return False
    assignment = body[1]
    return (isinstance(body[0].value.value, str) and isinstance(assignment, ast.Assign)
            and len(assignment.targets) == 1 and isinstance(assignment.targets[0], ast.Name)
            and assignment.targets[0].id == "__version__"
            and isinstance(assignment.value, ast.Constant) and isinstance(assignment.value.value, str))


def required_provenance(root: Path) -> set[str]:
    paths = {path.relative_to(root).as_posix() for path in (root / "src/keyswitch").rglob("*.py")}
    paths.discard(VERSION_MODULE)
    paths.update(path.relative_to(root).as_posix() for path in (root / "src/keyswitch/resources/models").iterdir()
                 if path.is_file() and path.name not in INSTALLED_MODELS)
    paths.update({BASELINE, PREFIX_BASELINE, RECIPE, "model/intent_v1/config.json", "model/context_v1/scenarios.json",
                  "src/keyswitch/resources/protected_tokens.txt", "src/keyswitch/resources/identifiers.json",
                  "src/keyswitch/resources/lexicon-supplement-ru_RU.json"})
    paths.update("tools/" + name for name in ("train_context_action_model.py", "freeze_context_action_corpus.py",
                 "evaluate_context_action_sequences.py", "context_evidence.py", "reference_lexicon.py", "context_optimizer.py",
                 "context_optimizer.c", "train_context_model.py", "context_physical_keys.py",
                 "reconcile_context_action_corpus.py", "context_action_spans.py"))
    paths.update("tests/" + name for name in ("test_input_integrity.py", "test_engine_behaviour.py",
                 "test_windows_backend.py", "test_x11_backend.py", "test_language_intent_regressions.py",
                 "test_input_sequence_matrix.py", "test_context_policy.py", "test_context_action_spans.py"))
    paths.update("model/intent_v1/sources/" + locale + ".lm" for locale in ("en_US", "ru_RU"))
    paths.update("model/intent_v1/sources/hunspell/" + locale + suffix
                 for locale in ("en_US", "ru_RU") for suffix in (".dic", ".aff"))
    return paths


def calibration_counts(value: object) -> dict[str, object]:
    """The calibration a receipt publishes must show a balance, not an absence of error.

    This read the same zero-false-conversion rule as the evaluator until 17.09.2026, and
    it has to keep reading the same one: the published gate policy names a minimum net
    benefit per profile, and a verifier that silently asked for more than the policy says
    would reject exactly the candidates the policy accepts.
    """

    row = mapping(value, "calibration counts", CALIBRATION)
    counts = {name: integer(row[name], name) for name in CALIBRATION - {"conversion_recall"}}
    recall = row["conversion_recall"]
    minimum = cast(int, CONTEXT_ACTION_GATE_POLICY["minimum_calibration_net_benefit"])
    floor = cast(float, CONTEXT_ACTION_GATE_POLICY["minimum_calibration_conversion_recall"])
    if (type(recall) not in (float, int) or not math.isfinite(cast(float, recall))
            or not 0 < counts["convert_rows"] <= counts["rows"]
            or counts["converted_correctly"] > counts["convert_rows"]
            or recall != counts["converted_correctly"] / counts["convert_rows"]
            or counts["false_conversions"] > counts["converted_correctly"]
            or counts["converted_correctly"] - counts["false_conversions"] < minimum
            or recall < floor):
        raise ValueError("calibration gate failed")
    return row


def validate_calibration(value: object) -> None:
    total = mapping(value, "calibration", CALIBRATION | {"by_profile"})
    aggregate = calibration_counts({key: total[key] for key in CALIBRATION})
    profiles = mapping(total["by_profile"], "calibration profiles", frozenset(PROFILES))
    counts = [calibration_counts(profiles[name]) for name in PROFILES]
    if any(aggregate[key] != sum(cast(int, row[key]) for row in counts) for key in CALIBRATION - {"conversion_recall"}):
        raise ValueError("calibration aggregate mismatch")


def sequence_counts(value: object, documents: Mapping[str, int]) -> dict[str, int]:
    row = mapping(value, "sequence counts", COUNTS)
    counts = {name: integer(row[name], name) for name in COUNTS}
    wrong = documents["0"] + documents["1"]
    correct = wrong + documents["None"]
    if (counts["rows"] != wrong + correct or counts["initially_correct"] != correct
            or counts["initially_wrong"] != wrong or counts["exactly_restored"] > wrong
            or counts["exactly_restored"] > counts["injections"]
            or counts["preserved_correct"] + counts["correct_text_corruptions"] != correct
            or any(counts[key] > counts["rows"] for key in ("length_mismatches", "execution_errors", "final_layout_mismatches"))
            or counts["correction_layout_mismatches"] > counts["injections"]):
        raise ValueError("inconsistent sequence counts")
    return counts


def computed_gates(candidate: Mapping[str, int], baseline: Mapping[str, int], documents: Mapping[str, int]) -> dict[str, bool]:
    """Recompute the published gates from the published counts, by the published policy.

    `correct_text_preserved` asked for zero corruptions until 18.09.2026, while the
    gate policy this receipt carries - and the evaluator that produced the counts -
    say `sequence_corruptions_at_most_baseline`. A verifier stricter than the policy
    it prints refuses candidates the policy accepts, and does it silently.
    """

    return {"correct_text_preserved": (candidate["correct_text_corruptions"] <= baseline["correct_text_corruptions"]
                                       if CONTEXT_ACTION_GATE_POLICY["sequence_corruptions_at_most_baseline"]
                                       else candidate["correct_text_corruptions"] == 0),
            "length_preserved": candidate["length_mismatches"] == 0,
            "net_restorations_at_least_baseline": (candidate["exactly_restored"] - candidate["correct_text_corruptions"]
                                                  >= baseline["exactly_restored"] - baseline["correct_text_corruptions"]),
            "enough_documents_us": documents["0"] >= MINIMUM_SEQUENCE_DOCUMENTS_PER_GROUP,
            "enough_documents_ru": documents["1"] >= MINIMUM_SEQUENCE_DOCUMENTS_PER_GROUP,
            "execution_succeeded": candidate["execution_errors"] == baseline["execution_errors"] == 0,
            "correction_layouts_match": candidate["correction_layout_mismatches"] == 0}


def validate_test(value: object) -> None:
    test = mapping(value, "test", frozenset({"documents", "selected_rows", "trimmed_rows", "unsupported_rows", "profiles"}))
    raw_documents = mapping(test["documents"], "documents", frozenset({"0", "1", "None"}))
    documents = {name: integer(count, "documents") for name, count in raw_documents.items()}
    selected = integer(test["selected_rows"], "selected_rows")
    if (any(count > SEQUENCE_DOCUMENT_CAP_PER_GROUP for count in documents.values()) or selected > SEQUENCE_MAXIMUM_SELECTED_ROWS
            or selected != sum(documents.values()) + integer(test["unsupported_rows"], "unsupported_rows")
            or integer(test["trimmed_rows"], "trimmed_rows") > selected):
        raise ValueError("inconsistent sequence selection")
    profiles = mapping(test["profiles"], "test profiles", frozenset(PROFILES))
    for name in PROFILES:
        profile = mapping(profiles[name], "test profile", frozenset({"counts", "gates", "early_off"}))
        validate_gated_block(profile, documents)
        validate_gated_block(mapping(profile["early_off"], "early-off control", frozenset({"counts", "gates"})), documents)


def validate_gated_block(block: Mapping[str, object], documents: Mapping[str, int]) -> None:
    counts = mapping(block["counts"], "compared models", frozenset({"candidate", "baseline"}))
    candidate, baseline = (sequence_counts(counts[key], documents) for key in ("candidate", "baseline"))
    gates = computed_gates(candidate, baseline, documents)
    claimed = mapping(block["gates"], "sequence gates", frozenset(gates))
    if not all(gates.values()) or any(claimed[key] is not expected for key, expected in gates.items()):
        raise ValueError("sequence gates failed or contradicted by counts")


def validate_receipt(receipt: object, artifact: Path = ARTIFACT_PATH, root: Path = ROOT,
                     prefix_artifact: Path = PREFIX_ARTIFACT_PATH) -> dict[str, object]:
    value = mapping(receipt, "receipt", FIELDS)
    if (type(value["schema_version"]) is not int or value["schema_version"] != 1
            or type(value["feature_version"]) is not int or value["feature_version"] != CONTEXT_ACTION_FEATURE_VERSION
            or value["quality_gates_passed"] is not True or value["scope"] != SCOPE
            or canonical(value["protocol"]) != canonical(PROTOCOL)
            or canonical(value["gate_policy"]) != canonical(CONTEXT_ACTION_GATE_POLICY)
            or value["baseline_path"] != BASELINE or value["recipe_path"] != RECIPE
            or value["prefix_baseline_path"] != PREFIX_BASELINE):
        raise ValueError("unsupported receipt identity or policy")
    for name in FIELDS:
        if name.endswith("_sha256"):
            sha(value[name])
    actual = checksum(artifact)
    if actual == REJECTED_ARTIFACT or actual != value["artifact_sha256"]:
        raise ValueError("unaccepted or changed context artifact")
    model = ContextModel.load(artifact)
    payload = read_object(artifact, MODEL_ARTIFACT_JSON_LIMIT_BYTES)
    if (type(value["conversion_threshold"]) not in (int, float)
            or model.feature_version != CONTEXT_ACTION_FEATURE_VERSION
            or model.version != "context-v3-" + sha(value["weights_sha256"])[:VERSION_HASH_CHARACTERS]
            or value["model_version"] != model.version or value["conversion_threshold"] != model.conversion_threshold
            or payload.get("weights_sha256") != value["weights_sha256"]):
        raise ValueError("context weights, version or threshold mismatch")
    prefix_payload = read_object(prefix_artifact, MODEL_ARTIFACT_JSON_LIMIT_BYTES)
    prefix = VersionedPrefixModel.load(prefix_artifact)
    if (checksum(prefix_artifact) != value["prefix_artifact_sha256"]
            or type(value["prefix_feature_version"]) is not int or prefix.feature_version != value["prefix_feature_version"]
            or type(value["prefix_conversion_threshold"]) not in (int, float)
            or prefix.model.conversion_threshold != value["prefix_conversion_threshold"]
            or prefix.version != f"prefix-v{prefix.feature_version}-" + sha(value["prefix_weights_sha256"])[:VERSION_HASH_CHARACTERS]
            or value["prefix_model_version"] != prefix.version
            or prefix_payload.get("weights_sha256") != value["prefix_weights_sha256"]):
        raise ValueError("prefix weights, version or threshold mismatch")
    hashes = mapping(value["provenance"], "provenance")
    if not required_provenance(root) <= hashes.keys() or VERSION_MODULE in hashes:
        raise ValueError("missing required provenance")
    if not version_module_holds_only_a_version(root):
        raise ValueError("the unpinned version module carries more than a version")
    for name, expected in hashes.items():
        if checksum(public_path(root, name)) != sha(expected):
            raise ValueError("context provenance mismatch: " + name)
    if hashes.get(PREFIX_BASELINE) != value["prefix_baseline_sha256"]:
        raise ValueError("prefix baseline mismatch")
    recipe = read_object(root / RECIPE)
    if (hashes.get(BASELINE) != value["baseline_sha256"] or hashes.get(RECIPE) != value["recipe_sha256"]
            or type(recipe.get("schema_version")) is not int or recipe["schema_version"] != 1
            or type(recipe.get("feature_version")) is not int or recipe["feature_version"] != CONTEXT_ACTION_FEATURE_VERSION
            or recipe.get("profiles") != list(PROFILES)
            or canonical(recipe.get("gate_policy")) != canonical(CONTEXT_ACTION_GATE_POLICY)):
        raise ValueError("baseline, recipe or gate policy mismatch")
    validate_calibration(value["calibration"])
    validate_test(value["test"])
    return value


def verify(root: Path = ROOT, receipt_path: Path = RECEIPT, artifact: Path = ARTIFACT_PATH,
           prefix_artifact: Path = PREFIX_ARTIFACT_PATH) -> dict[str, object]:
    return validate_receipt(read_object(receipt_path), artifact, root, prefix_artifact)


def case_evidence(raw: object) -> tuple[tuple[str, bool], tuple[object, ...], dict[str, int]]:
    case = mapping(raw, "case")
    for field in ("identifier", "document", "expected", "actual"):
        if not isinstance(case.get(field), str):
            raise ValueError("invalid case text or identity")
    identifier, document = cast(str, case["identifier"]), cast(str, case["document"])
    expected, actual = cast(str, case["expected"]), cast(str, case["actual"])
    group = case.get("group")
    if group is not None and (type(group) is not int or group not in (0, 1)):
        raise ValueError("invalid case group")
    wrong = case.get("initially_wrong")
    if type(wrong) is not bool or wrong and group is None:
        raise ValueError("invalid case intervention mode")
    final, intended = case.get("final_group"), case.get("expected_final_group")
    if any(type(value) is not int or value not in (0, 1) for value in (final, intended)):
        raise ValueError("invalid case final layout")
    error = case.get("error")
    if "error" not in case or error is not None and not isinstance(error, str):
        raise ValueError("invalid case execution error")
    exact, length, layout = error is None and actual == expected, len(actual) != len(expected), final == intended
    if (case.get("exact") is not exact or case.get("length_mismatch") is not length
            or case.get("final_layout_matches") is not layout):
        raise ValueError("case outcome contradicts actual text or layout")
    intervention = case.get("intervention")
    if wrong:
        event = mapping(intervention, "case intervention")
        number = integer(event.get("event"), "case intervention event")
        source, target = event.get("source_group"), event.get("target_group")
        if (not 1 <= number <= len(expected) or type(source) is not int or source not in (0, 1)
                or type(target) is not int or target != group or source == target
                or event.get("expected") != expected[number - 1]
                or not isinstance(event.get("observed"), str) or len(cast(str, event["observed"])) != 1
                or event["observed"] == event["expected"]):
            raise ValueError("case does not show an effective wrong intervention")
    elif intervention is not None:
        raise ValueError("correct case unexpectedly has a wrong intervention")
    corrections = case.get("corrections")
    if not isinstance(corrections, list) or wrong and exact and not corrections:
        raise ValueError("invalid case corrections or restoration without injection")
    layout_errors = 0
    for raw_correction in corrections:
        correction = mapping(raw_correction, "case correction")
        if (any(type(correction.get(key)) is not int or correction[key] not in (0, 1)
                for key in ("source_group", "target_group", "actual_group"))
                or not 1 <= integer(correction.get("event"), "case correction event") <= len(expected)):
            raise ValueError("invalid case correction layout or event")
        for text, caret in (("before", "caret_before"), ("after", "caret_after")):
            if (not isinstance(correction.get(text), str)
                    or integer(correction.get(caret), "case correction caret") > len(cast(str, correction[text]))):
                raise ValueError("invalid case correction text or caret")
        layout_errors += int(correction["actual_group"] != correction["target_group"])
    counts = {"rows": 1, "initially_correct": int(not wrong), "initially_wrong": int(wrong),
        "preserved_correct": int(not wrong and exact), "exactly_restored": int(wrong and exact),
        "correct_text_corruptions": int(not wrong and not exact), "length_mismatches": int(length),
        "injections": len(corrections), "execution_errors": int(error is not None),
        "correction_layout_mismatches": layout_errors, "final_layout_mismatches": int(not layout)}
    inputs = (document, group, expected, integer(case.get("trimmed_left"), "case left trim"),
              integer(case.get("trimmed_right"), "case right trim"), intended)
    return (identifier, wrong), inputs, counts


def case_block_inputs(block: Mapping[str, object], selected: set[str], omitted: set[str],
                      document_hashes: set[str]) -> dict[tuple[str, bool], tuple[object, ...]]:
    """Recompute one compared block's counts from its private cases; return the shared inputs."""
    all_cases = mapping(block.get("cases"), "model cases", frozenset({"baseline", "candidate"}))
    all_counts = mapping(block.get("counts"), "case counts", frozenset({"baseline", "candidate"}))
    reference: dict[tuple[str, bool], tuple[object, ...]] | None = None
    for model in ("baseline", "candidate"):
        rows = all_cases[model]
        if not isinstance(rows, list):
            raise ValueError("missing case rows")
        inputs: dict[tuple[str, bool], tuple[object, ...]] = {}
        counts = dict.fromkeys(COUNTS, 0)
        for raw in rows:
            key, values, outcome = case_evidence(raw)
            if key in inputs or key[0] not in selected - omitted:
                raise ValueError("duplicate, omitted or unselected case")
            if hashlib.sha256(cast(str, values[0]).encode()).hexdigest() not in document_hashes:
                raise ValueError("case document is outside immutable test membership")
            inputs[key] = values
            for field, count in outcome.items():
                counts[field] += count
        if canonical(all_counts[model]) != canonical(counts):
            raise ValueError("claimed counts differ from actual cases")
        if reference is not None and reference != inputs:
            raise ValueError("compared models or profiles have different case inputs")
        reference = inputs
    assert reference is not None
    return reference


def validate_full_report(report: Mapping[str, object], identity: Mapping[str, object]) -> None:
    """Recompute aggregates from private cases without reading corpus text."""
    selection = mapping(report.get("selection"), "case selection")
    identifiers = selection.get("source_ids")
    if (not isinstance(identifiers, list) or any(not isinstance(name, str) or not name for name in identifiers)
            or len(set(identifiers)) != len(identifiers)
            or len(identifiers) != integer(selection.get("selected_rows"), "selected case rows")):
        raise ValueError("case selection IDs are missing, duplicated or inconsistent")
    selected = set(cast(list[str], identifiers))
    membership = mapping(identity.get("test_membership"), "case membership")
    row_hashes = set(cast(list[str], membership["row_ids_sha256"]))
    document_hashes = set(cast(list[str], membership["document_ids_sha256"]))
    if any(hashlib.sha256(name.encode()).hexdigest() not in row_hashes for name in selected):
        raise ValueError("selected case is outside immutable test membership")
    unsupported = selection.get("unsupported")
    if not isinstance(unsupported, list):
        raise ValueError("invalid unsupported case selection")
    omitted = [mapping(item, "unsupported case").get("identifier") for item in unsupported]
    if (any(not isinstance(name, str) for name in omitted) or len(set(omitted)) != len(omitted)
            or not set(omitted) <= selected):
        raise ValueError("unsupported case IDs are duplicated or outside selection")
    reference: dict[tuple[str, bool], tuple[object, ...]] | None = None
    profiles = mapping(report.get("profiles"), "case profiles", frozenset(PROFILES))
    for name in PROFILES:
        profile = mapping(profiles[name], "case profile")
        for block in (profile, mapping(profile.get("early_off"), "early-off control block")):
            inputs = case_block_inputs(block, selected, set(cast(list[str], omitted)), document_hashes)
            if reference is not None and reference != inputs:
                raise ValueError("compared models, modes or profiles have different case inputs")
            reference = inputs
    assert reference is not None
    correct = {name: values for (name, wrong), values in reference.items() if not wrong}
    expected_keys = {(name, wrong) for name, values in correct.items()
                     for wrong in ((False, True) if values[1] in (0, 1) else (False,))}
    if (set(correct) != selected - set(omitted) or set(reference) != expected_keys
            or any(values != correct[name] for (name, wrong), values in reference.items() if wrong)):
        raise ValueError("case coverage or paired correct/wrong inputs differ")
    documents = {str(group): len({values[0] for values in correct.values() if values[1] == group})
                 for group in (0, 1, None)}
    if (sum(documents.values()) != len(correct)
            or canonical(documents) != canonical(selection.get("replayed_documents"))):
        raise ValueError("case document counts or one-focus-per-document policy differ")
    trimmed = sum(bool(values[TRIMMED_LEFT_FIELD] or values[TRIMMED_RIGHT_FIELD]) for values in correct.values())
    if not trimmed <= integer(selection.get("trimmed_rows"), "trimmed case rows") <= trimmed + len(omitted):
        raise ValueError("case trim count differs from selection")


def export_receipt(artifact: Path, seal_path: Path, report_path: Path, corpus: Path, output: Path, *,
                   prefix_artifact: Path, prefix_seal_path: Path) -> dict[str, object]:
    # The public packaging gate above has no evaluator/EditorBackend imports.
    import evaluate_context_action_sequences as evaluator

    if hashlib.sha256(canonical(evaluator.PROTOCOL)).hexdigest() != AUDITED_SEQUENCE_PROTOCOL_SHA256:
        raise ValueError("sequence protocol changed; public receipt protocol must be reviewed")
    seal = evaluator.validate_candidate_seal(artifact, seal_path, corpus)
    evaluator.validate_prefix_seal(prefix_artifact, prefix_seal_path, require_calibration=True)
    identity = evaluator.evaluation_identity(artifact, seal_path, corpus, prefix_artifact, prefix_seal_path)
    report = read_object(report_path, FULL_REPORT_JSON_LIMIT_BYTES)
    if (type(report.get("schema_version")) is not int or report["schema_version"] != 1 or report.get("split") != "test"
            or canonical(report.get("identity")) != canonical(identity)
            or canonical(report.get("protocol")) != canonical(evaluator.PROTOCOL)
            or canonical(report.get("gate_policy")) != canonical(CONTEXT_ACTION_GATE_POLICY)
            or report.get("promotion_passed") is not True or "execution_error" in report):
        raise ValueError("report is not an accepted current sealed test")
    key = evaluator.membership_key(identity)
    if ((evaluator.LEDGER_ROOT / (key + ".access.json")).read_bytes() != canonical(identity)
            or (evaluator.LEDGER_ROOT / (key + ".outcome.json")).read_bytes() != report_path.read_bytes()):
        raise ValueError("immutable access or outcome does not confirm report")
    validate_full_report(report, identity)
    hashes = dict(mapping(seal.get("provenance"), "seal provenance"))
    for name, fingerprint in mapping(identity.get("runtime"), "runtime provenance").items():
        if name in hashes and hashes[name] != fingerprint:
            raise ValueError("seal and runtime provenance disagree")
        hashes[name] = fingerprint
    hashes.pop(VERSION_MODULE, None)
    calibration = mapping(seal.get("calibration"), "calibration")
    by_profile = mapping(calibration.get("by_profile"), "calibration profiles", frozenset(PROFILES))
    public_calibration = {name: calibration[name] for name in CALIBRATION}
    public_calibration["by_profile"] = {profile: {name: mapping(by_profile[profile], "calibration profile")[name]
                                                  for name in CALIBRATION} for profile in PROFILES}
    selection = mapping(report.get("selection"), "selection")
    unsupported = selection.get("unsupported")
    if not isinstance(unsupported, list):
        raise ValueError("missing unsupported selection aggregate")
    profiles = mapping(report.get("profiles"), "profiles", frozenset(PROFILES))
    public_profiles = {}
    for profile in PROFILES:
        private = mapping(profiles[profile], "profile")
        control = mapping(private.get("early_off"), "early-off control")
        public_profiles[profile] = {"counts": private["counts"], "gates": private["gates"],
                                    "early_off": {"counts": control["counts"], "gates": control["gates"]}}
    payload = read_object(artifact, MODEL_ARTIFACT_JSON_LIMIT_BYTES)
    prefix_payload = read_object(prefix_artifact, MODEL_ARTIFACT_JSON_LIMIT_BYTES)
    prefix = VersionedPrefixModel.load(prefix_artifact)
    receipt: dict[str, object] = {
        "schema_version": 1, "feature_version": CONTEXT_ACTION_FEATURE_VERSION, "model_version": seal["model_version"],
        "conversion_threshold": seal["conversion_threshold"], "artifact_sha256": checksum(artifact),
        "weights_sha256": payload["weights_sha256"], "baseline_path": BASELINE,
        "baseline_sha256": identity["baseline_sha256"], "recipe_path": RECIPE, "recipe_sha256": hashes[RECIPE],
        "prefix_model_version": prefix.version, "prefix_feature_version": prefix.feature_version,
        "prefix_conversion_threshold": prefix.model.conversion_threshold,
        "prefix_artifact_sha256": identity["prefix_candidate_sha256"],
        "prefix_weights_sha256": prefix_payload["weights_sha256"],
        "prefix_candidate_seal_sha256": identity["prefix_candidate_seal_sha256"],
        "prefix_baseline_path": PREFIX_BASELINE, "prefix_baseline_sha256": identity["prefix_baseline_sha256"],
        "candidate_seal_sha256": checksum(seal_path), "full_test_report_sha256": checksum(report_path),
        "corpus_manifest_sha256": identity["corpus_manifest_sha256"], "provenance": hashes,
        "calibration": public_calibration, "test": {"documents": selection["replayed_documents"],
        "selected_rows": selection["selected_rows"], "trimmed_rows": selection["trimmed_rows"],
        "unsupported_rows": len(unsupported), "profiles": public_profiles},
        "gate_policy": CONTEXT_ACTION_GATE_POLICY, "protocol": PROTOCOL, "scope": SCOPE, "quality_gates_passed": True,
    }
    validate_receipt(receipt, artifact, ROOT, prefix_artifact)
    evaluator.immutable_record(output, receipt)
    return receipt


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--verify", action="store_true")
    mode.add_argument("--export", action="store_true")
    parser.add_argument("--artifact", type=Path, default=ARTIFACT_PATH)
    parser.add_argument("--prefix-artifact", type=Path, default=PREFIX_ARTIFACT_PATH)
    parser.add_argument("--receipt", type=Path, default=RECEIPT)
    parser.add_argument("--seal", type=Path)
    parser.add_argument("--prefix-seal", type=Path)
    parser.add_argument("--report", type=Path)
    parser.add_argument("--corpus", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    if args.export:
        if not all((args.seal, args.report, args.corpus, args.output, args.prefix_seal)):
            parser.error("--export requires --seal, --prefix-seal, --report, --corpus and --output")
        receipt = export_receipt(args.artifact, args.seal, args.report, args.corpus, args.output,
                                 prefix_artifact=args.prefix_artifact, prefix_seal_path=args.prefix_seal)
    else:
        receipt = verify(receipt_path=args.receipt, artifact=args.artifact, prefix_artifact=args.prefix_artifact)
    print(json.dumps({"model_version": receipt["model_version"], "artifact_sha256": receipt["artifact_sha256"],
                      "quality_gates_passed": receipt["quality_gates_passed"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
