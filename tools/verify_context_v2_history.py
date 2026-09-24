"""Audited historical context-v2 identity, without asserting current engine quality."""
from __future__ import annotations

import ast
from copy import deepcopy
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
from typing import cast
import sys
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))
from keyswitch.constants.file_formats import HASH_CHUNK_BYTES

ROOT = Path(__file__).resolve().parents[1]
ARCHIVE = "model/context_v2/compatibility/generation-sources"
GENERATION_REF = "80534ec"
SCOPE = "Historical rejected context-v2 evidence and feature-2 numeric compatibility; no new fit, new holdout, native execution or current-engine acceptance."
ANCHORS = {
    "candidate.json": "55f0d735de8751f3f414d569b6e1eb2bbb294303d8fe01f2fde66c3fadecfeb1",
    "candidate-seal.json": "03878d1d8fcc0a79295b5f63acfa06f097a2b99931a1f98eb6060c7d0370a53a",
    "report.json": "be7ff355ddcbe0cb84e0fdbb53a62a6bafdbeb2509d589ded16ec1c82c38918e",
    "engine-report.json": "1849b11451ceda5b64da8a24eb37c7b6532d838273521ece25dab92afef618cb",
    "corpus-receipt.json": "cf88d7fe395d09ff6fafa8b3950cd18e1e7a07d781af9199e23e15fcff57cd5b",
    "lexical-receipt.json": "f0b89d49b707ac7163207ed273de0eb9d45fa04da2fb6735b430c18fcd058a94",
    "baseline-context-v1.json": "80a1b8c97a095413298e4287763bb675ea8dd9218dc6ca78d371ae7a1b9efd93",
}
SOURCES = {
    "src/keyswitch/context_model.py": "760c0d10ef73167ee8c601b806a75262e2ad3a118fde425e252e11cfde5e0491",
    "src/keyswitch/context_policy.py": "c77bc72cf0f4e6da13d260cf426e2af68ee404d42556c8f2239da80f15ad8ae2",
    "src/keyswitch/engine.py": "25eda8ad37def9160492245d8a5c55d0924a0fbbdc81829d55e629437ea3dbf9",
}
# Reviewed feature-version dispatch additions; feature-2 weight decoding stays exact.
LOADER_AST_SHA256 = {
    "__init__": "8bddcce338377c33d1787e4d02aeae6d2bd2e382606955cd822b26b2265dce64",
    "load": "5b1aecb0d6c8df0d6a7ccdcb135d151b46b29707346d528496e1275f98d8ab9b",
}


def checksum(path: Path) -> str:
    result = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(HASH_CHUNK_BYTES), b""):
            result.update(block)
    return result.hexdigest()


def normalized_provenance(value: object) -> dict[str, str]:
    if not isinstance(value, dict) or not value:
        raise ValueError("missing historical source pins")
    result: dict[str, str] = {}
    for key, digest in value.items():
        if not isinstance(key, str) or not isinstance(digest, str) or not re.fullmatch(r"[a-f0-9]{64}", digest):
            raise ValueError("invalid historical source pin")
        name = key.replace("\\", "/")
        # Pins are repository-relative POSIX paths whatever the platform: on
        # Windows Path("/outside.py") has no drive and is not absolute.
        path = PurePosixPath(name)
        if (path.is_absolute() or ".." in path.parts or path.as_posix() != name
                or name in result or ":" in name):
            raise ValueError("invalid or duplicate historical source path")
        result[name] = digest
    return result


def verify_sources(value: object, root: Path = ROOT) -> None:
    for relative, digest in normalized_provenance(value).items():
        source = root / relative
        if relative in SOURCES:
            if SOURCES[relative] != digest:
                raise ValueError("unapproved historical source identity")
            source = root / ARCHIVE / Path(relative).name
        if not source.resolve().is_relative_to(root.resolve()) or checksum(source) != digest:
            raise ValueError("historical source changed: " + relative)


def _named(tree: ast.AST, name: str) -> ast.AST:
    for node in ast.walk(tree):
        if isinstance(node, (ast.FunctionDef, ast.ClassDef)) and node.name == name:
            return node
        if isinstance(node, ast.Assign) and any(isinstance(target, ast.Name) and target.id == name for target in node.targets):
            return node
        if isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name) and node.target.id == name:
            return node
    raise ValueError("feature-2 compatibility declaration missing: " + name)


def _project_two(body: list[ast.stmt], support: ast.expr | None = None) -> list[ast.stmt]:
    """Remove only branches provably unreachable for self.feature_version == 2."""
    guard = ast.dump(ast.parse("self.feature_version == 3", mode="eval").body)
    call = ast.dump(ast.parse("self.supports_features(features)", mode="eval").body)
    result: list[ast.stmt] = []
    for node in body:
        if isinstance(node, ast.If):
            test = node.test.values[0] if isinstance(node.test, ast.BoolOp) and isinstance(node.test.op, ast.And) else node.test
            if ast.dump(test) == guard:
                result.extend(_project_two(node.orelse, support))
                continue
        if isinstance(node, ast.Assign) and support is not None and ast.dump(node.value) == call:
            node = deepcopy(node)
            node.value = deepcopy(support)
        result.append(node)
    return result


def _ast_checksum(node: ast.AST) -> str:
    """Ignore source coordinates and empty optional AST fields across Python versions."""
    def normalized(value: object) -> object:
        if isinstance(value, ast.AST):
            return [type(value).__name__, [[name, normalized(child)] for name, child in ast.iter_fields(value)
                                          if child is not None and child != []]]
        if isinstance(value, list):
            return [normalized(item) for item in value]
        return [type(value).__name__, repr(value)]
    encoded = json.dumps(normalized(node), ensure_ascii=False, separators=(",", ":")).encode()
    return hashlib.sha256(encoded).hexdigest()


def verify_feature_two(current: str, historical: str) -> None:
    """Conservatively compare the feature-2 numerical path with archived source."""
    old, new = ast.parse(historical), ast.parse(current)
    for name in ("ACTIONS", "FEATURE_VERSION", "MAX_ARTIFACT_BYTES", "MAX_FEATURES", "_WORDS", "_normalized",
                 "ContextPrediction", "extract_context_features", "softmax"):
        if ast.dump(_named(old, name)) != ast.dump(_named(new, name)):
            raise ValueError("feature-2 numerical declaration changed: " + name)
    for name, expected in LOADER_AST_SHA256.items():
        previous = _named(_named(old, "ContextModel"), name)
        current_method = _named(_named(new, "ContextModel"), name)
        if ast.dump(previous) != ast.dump(current_method) and _ast_checksum(current_method) != expected:
            raise ValueError("feature-2 numerical loader changed: " + name)
    old_predict = cast(ast.FunctionDef, _named(_named(old, "ContextModel"), "predict"))
    new_predict = deepcopy(cast(ast.FunctionDef, _named(_named(new, "ContextModel"), "predict")))
    support: ast.expr | None = None
    if any(isinstance(node, ast.FunctionDef) and node.name == "supports_features" for node in ast.walk(new)):
        method = cast(ast.FunctionDef, _named(new, "supports_features"))
        body = _project_two(method.body)
        if body and isinstance(body[0], ast.Expr) and isinstance(body[0].value, ast.Constant) and isinstance(body[0].value.value, str):
            body = body[1:]
        if len(body) != 1 or not isinstance(body[0], ast.Return) or body[0].value is None:
            raise ValueError("feature-2 support semantics cannot be verified")
        support = body[0].value
    new_predict.body = _project_two(new_predict.body, support)
    if ast.dump(old_predict) != ast.dump(new_predict):
        raise ValueError("feature-2 numerical prediction changed")


def verify_anchors(directory: Path, root: Path = ROOT) -> dict[str, object]:
    for name, expected in ANCHORS.items():
        if checksum(directory / name) != expected:
            raise ValueError("historical context-v2 anchor changed: " + name)
    verify_sources(SOURCES, root)
    verify_feature_two((root / "src/keyswitch/context_model.py").read_text(encoding="utf-8"),
                       (root / ARCHIVE / "context_model.py").read_text(encoding="utf-8"))
    for name, field in (("candidate-seal.json", "provenance"), ("lexical-receipt.json", "source_hashes"),
                        ("engine-report.json", "provenance")):
        value: object = json.loads((directory / name).read_bytes())
        if not isinstance(value, dict):
            raise ValueError("invalid historical evidence object")
        verify_sources(value.get(field), root)
    return {"historical_evidence_verified": True, "current_runtime_verified": False,
            "feature_two_compatible": True, "scope": SCOPE, "generation_ref": GENERATION_REF}
