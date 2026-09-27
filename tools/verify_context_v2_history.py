"""Audited historical context-v2 identity, without asserting current engine quality."""
from __future__ import annotations

from array import array
import ast
from collections.abc import Callable, Iterable, Iterator, Sequence
import dataclasses
import hashlib
import importlib.util
import itertools
import json
import math
from pathlib import Path
import sys
import tempfile
from types import ModuleType
from typing import cast
sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))
from historical_sources import CONTEXT_V2, ROOT as ROOT, checksum as checksum
from keyswitch.input_context import FieldContext, FieldRole
from keyswitch.layouts import LayoutPair

# Every source the evidence pins outside model/context_v2 is checked against its archived copy
# (tools/historical_sources.py); none is read from the live tree.
ARCHIVE = CONTEXT_V2.directory
SOURCES = CONTEXT_V2.sources
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
HISTORY = "model/context_v2"
CURRENT_MODEL = "src/keyswitch/context_model.py"
# The corpus receipt records the digest of the tool that built the phrase split.
CORPUS_BUILDER = "tools/context_corpus.py"
# The shipped artifact is compared too while it declares feature schema 2.
INSTALLED_ARTIFACT = "src/keyswitch/resources/models/context_policy_v1.json"
# Both feature-2 models of the historical evaluation: the frozen v1 baseline and the rejected
# candidate (the largest artifact, loaded last).
HISTORICAL_ARTIFACTS = ("baseline-context-v1.json", "candidate.json")
PROBE_SEED = "model/context_v2/compatibility/feature-two-probes.json"
# SHA-256 of the canonical feature-2 probe plan built from PROBE_SEED and the archived source:
# the evidence corpus, the score vectors and the loader payloads. A changed, shortened or
# reordered plan fails closed instead of silently comparing less.
FEATURE_TWO_PLAN_SHA256 = "d990630577f8e6eb3622ac0b77fd7665184cff1f028021f6d4c6a36d6111e740"
# The live evidence class also carries serving fields the archive never had. They are
# varied across the corpus: feature schema 2 must ignore them.
SERVING_ONLY_EVIDENCE = ("model_probability", "model_threshold", "ortho_score", "ortho_threshold",
                         "literal_tail", "boundary_text", "after_origin", "source_identifier", "target_identifier")
COMPARISON_MODULE_PREFIX = "keyswitch._context_v2_history_"
_OUTSIDE = "outside feature schema 2"
_OUTSIDE_AS_FEATURE_TWO = "outside feature schema 2, loaded as feature 2: "
_MISSING = object()


class FeatureTwoChanged(ValueError):
    """The live feature-2 numerical path no longer reproduces the archived one."""


def verify_sources(value: object, root: Path = ROOT) -> None:
    """Every pin resolves to model/context_v2 or to an archived copy with that exact digest."""

    CONTEXT_V2.verify(value, root)


def _named(tree: ast.AST, name: str) -> ast.AST:
    for node in ast.walk(tree):
        if isinstance(node, (ast.FunctionDef, ast.ClassDef)) and node.name == name:
            return node
        if isinstance(node, ast.Assign) and any(isinstance(target, ast.Name) and target.id == name for target in node.targets):
            return node
        if isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name) and node.target.id == name:
            return node
    raise ValueError("feature-2 compatibility declaration missing: " + name)


def load_context_module(source: bytes, origin: Path, label: str) -> ModuleType:
    """Execute one context_model.py source as a private module beside the live package.

    Its relative imports resolve to the live `keyswitch` package: the archive imports
    only `.input_context` (FieldContext, the input container whose archived bytes the
    historical engine report pins), the live file also its serving-only siblings. The module is in
    `sys.modules` only while its classes are created, because dataclasses look their
    module up there; no live module is replaced and no bytecode is written anywhere.
    """

    name = COMPARISON_MODULE_PREFIX + label
    spec = importlib.util.spec_from_loader(name, loader=None, origin=str(origin))
    if spec is None or name in sys.modules:
        raise FeatureTwoChanged("feature-2 comparison module cannot be isolated: " + label)
    module = importlib.util.module_from_spec(spec)
    module.__file__ = str(origin)
    sys.modules[name] = module
    try:
        exec(compile(source, str(origin), "exec", dont_inherit=True), module.__dict__)
    except Exception as error:
        raise FeatureTwoChanged(f"feature-2 module cannot be loaded: {label}: {type(error).__name__}") from error
    finally:
        del sys.modules[name]
    return module


@dataclasses.dataclass(frozen=True)
class Payload:
    """One loader probe: a JSON artifact, optionally padded with whitespace to a size bound."""

    label: str
    content: bytes
    padding: int
    # Declares exactly feature schema 2 or is no object at all: both loaders must agree
    # exactly. Any other payload may be rejected or loaded as another schema by the current
    # loader, never loaded as a feature-2 model the archive did not produce identically.
    feature_two: bool
    # Megabytes to parse: loaded after the cheaper checks, so a changed source fails fast.
    bulky: bool = False


@dataclasses.dataclass(frozen=True)
class ProbePlan:
    feature_version: int
    evidence: tuple[tuple[dict[str, object], dict[str, object]], ...]
    scores: tuple[tuple[float, ...], ...]
    payloads: tuple[Payload, ...]
    digest: str


def _literals(node: ast.AST) -> list[float]:
    """Magnitudes of the numbers written in one archived function: its bounds and scales."""

    return sorted({abs(float(sub.value)) for sub in ast.walk(node) if isinstance(sub, ast.Constant)
                   and isinstance(sub.value, (int, float)) and not isinstance(sub.value, bool)})


def _around(values: Iterable[float]) -> list[int]:
    """Each whole magnitude with its neighbours: where a slice or a bound can move by one."""

    wholes = {int(value) for value in values if value.is_integer()}
    return sorted({size for whole in wholes for size in (whole - 1, whole, whole + 1) if size >= 0})


def _edges(values: Iterable[float]) -> list[float]:
    """Each magnitude with both signs and its neighbouring floats, then the IEEE extremes."""

    found: dict[str, float] = {}
    for value in values:
        for signed in (value, -value):
            for probe in (math.nextafter(signed, -math.inf), signed, math.nextafter(signed, math.inf)):
                found.setdefault(probe.hex(), probe)
    for probe in (math.inf, -math.inf, math.nan, sys.float_info.max, -sys.float_info.max,
                  sys.float_info.min, math.ulp(0.0)):
        found.setdefault(probe.hex(), probe)
    return list(found.values())


def _pick(values: Sequence[str], position: int) -> str:
    return values[position % len(values)]


def _text(alphabet: str, size: int) -> str:
    return (alphabet * (size // len(alphabet) + 1))[:size]


def _word(alphabet: str, position: int) -> str:
    """A distinct letters-only word for every position below the square of the alphabet."""

    letters = [char for char in alphabet if char.isalpha()]
    return letters[position % len(letters)] + letters[position // len(letters) % len(letters)]


def _strings(seed: dict[str, object], key: str) -> list[str]:
    value = seed.get(key)
    if not isinstance(value, list) or not value or not all(isinstance(item, str) for item in value):
        raise ValueError("invalid feature-2 probe seed: " + key)
    return cast(list[str], value)


def _weights_sha256(weights: object) -> str:
    """The artifact checksum exactly as the archived loader computes it."""

    return hashlib.sha256(json.dumps(weights, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()).hexdigest()


def _declares_feature_two(content: bytes, feature_version: int) -> bool:
    try:
        payload: object = json.loads(content)
    except ValueError:
        return True
    if not isinstance(payload, dict):
        return True
    declared: object = payload.get("feature_version")
    return type(declared) is int and declared == feature_version


def _evidence_corpus(seed: dict[str, object], text_sizes: list[int], deltas: list[float]) -> list[dict[str, object]]:
    """Scenario words in both layouts over rotating contexts, then one probe per archived bound."""

    words, texts, alphabets = _strings(seed, "words"), _strings(seed, "texts"), _strings(seed, "alphabets")
    applications, roles, triggers = _strings(seed, "applications"), _strings(seed, "roles"), _strings(seed, "triggers")
    raw_fields = seed.get("fields")
    if not isinstance(raw_fields, list) or not raw_fields or not all(
            isinstance(item, dict) and set(item) == {"before", "after"} and all(isinstance(text, str) for text in item.values())
            for item in raw_fields):
        raise ValueError("invalid feature-2 probe seed: fields")
    fields = cast(list[dict[str, str]], raw_fields)
    flags = list(itertools.product((False, True), (False, True), (False, True)))
    pair = LayoutPair()
    records: list[dict[str, object]] = []

    def add(original: str, alternative: str, group: int, before: str, after: str, application: str,
            delta: float | None = None) -> None:
        position = len(records)
        baseline, source_known, target_known = flags[position % len(flags)]
        flag = bool(position & 1)
        share = (position % len(texts)) / len(texts)
        role = _pick(roles, position)
        records.append({
            "original": original, "alternative": alternative, "source_group": group,
            "trigger": _pick(triggers, position), "baseline_convert": baseline, "source_known": source_known,
            "target_known": target_known, "score_delta": deltas[position % len(deltas)] if delta is None else delta,
            "field": {"application": application, "field_id": "probe-" + role, "before": before, "after": after, "role": role},
            "extra": {"model_probability": share, "model_threshold": share, "ortho_score": share, "ortho_threshold": share,
                      "literal_tail": _pick(texts, position), "boundary_text": _pick(texts, position + flag),
                      "after_origin": "planned_next_conversion" if flag else "field",
                      "source_identifier": flag, "target_identifier": not flag},
        })

    def layouts(word: str) -> tuple[str, str]:
        own = "ru" if any("а" <= char <= "я" or char == "ё" for char in word.casefold()) else "us"
        return own, "us" if own == "ru" else "ru"

    for index, word in enumerate(words):
        own, other = layouts(word)
        typed = pair.translate(word, own, other)
        for original, alternative, layout in ((typed, word, other), (word, typed, own)):
            context = fields[len(records) % len(fields)]
            add(original, alternative, int(layout == "ru"), context["before"], context["after"],
                _pick(applications, len(records) + index))
    for size in text_sizes:
        for alphabet in alphabets:
            text = _text(alphabet, size)
            spaced = " ".join(_word(alphabet, position) for position in range(size))
            dotted = ".".join(_word(alphabet, position) for position in range(size))
            word = _pick(words, size)
            context = fields[size % len(fields)]
            application = _pick(applications, size)
            add(text, word, int(bool(size & 1)), context["before"], context["after"], application)
            add(word, text, int(not size & 1), context["before"], context["after"], application)
            for before, after in ((text, context["after"]), (spaced, context["after"]),
                                  (context["before"], text), (context["before"], spaced)):
                add(word, text[::-1], int(bool(size & 1)), before, after, application)
            add(word, text, int(bool(size & 1)), context["before"], context["after"], text)
            add(word, text, int(bool(size & 1)), context["before"], context["after"], dotted)
    for index, delta in enumerate(deltas):
        word = _pick(words, index)
        own, other = layouts(word)
        context = fields[index % len(fields)]
        add(pair.translate(word, own, other), word, int(other == "ru"), context["before"], context["after"],
            _pick(applications, index), delta)
    for index, text in enumerate(texts):
        word = _pick(words, index)
        context = fields[index % len(fields)]
        application = _pick(applications, index)
        add(text, word, int(bool(index & 1)), context["before"], context["after"], application)
        add(word, text, int(not index & 1), context["before"], context["after"], application)
        add(word, word, int(bool(index & 1)), text + context["before"], context["after"], application)
        add(word, word, int(not index & 1), context["before"] + text, text + context["after"], application)
        add(word, word, int(bool(index & 1)), context["before"], context["after"], text)
    return records


def _score_vectors(values: list[float], width: int) -> list[tuple[float, ...]]:
    vectors: list[tuple[float, ...]] = []
    for start, value in enumerate(values):
        vectors.append(tuple(values[(start + offset) % len(values)] for offset in range(width)))
        vectors.append((value,) * width)
        vectors.append((value,))
    return vectors


def _loader_payloads(archived: ModuleType, load_sizes: list[int], values: list[float], baseline: bytes) -> list[Payload]:
    """Valid and invalid artifacts at every archived loader bound, plus a tampered historical baseline."""

    actions = [str(action) for action in archived.ACTIONS]
    feature_version = int(archived.FEATURE_VERSION)
    zeros = [0.0] * len(actions)
    prefix, other_prefix = "context-v1-", "context-v3-"
    base: dict[str, object] = {
        "actions": actions, "conversion_threshold": float(archived.ContextModel({}, prefix + "probe").conversion_threshold),
        "feature_version": feature_version, "version": prefix + "probe",
        "weights": {"bias": [float(index) for index in range(len(actions))]},
    }
    base["weights_sha256"] = _weights_sha256(base["weights"])
    base_bytes = json.dumps(base, ensure_ascii=False, separators=(",", ":")).encode()
    payloads: list[Payload] = []

    def raw(label: str, content: bytes, padding: int = 0, bulky: bool = False) -> None:
        payloads.append(Payload(label, content, padding, _declares_feature_two(content, feature_version), bulky))

    def payload(label: str, *, sign: bool = True, bulky: bool = False, **changes: object) -> None:
        value = {**base, **changes}
        if sign:
            value["weights_sha256"] = _weights_sha256(value["weights"])
        value = {key: item for key, item in value.items() if item is not _MISSING}
        raw(label, json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode(), bulky=bulky)

    payload("base")
    for size in load_sizes:
        payload(f"name {size}", weights={"n" * size: zeros})
        payload(f"values {size}", weights={"bias": [0.0] * size})
        payload(f"version {size}", version=_text(prefix + "probe", size))
        for whole in (size, -size):
            payload(f"weight int {whole}", weights={"bias": [whole, *zeros[1:]]})
            payload(f"threshold int {whole}", conversion_threshold=whole)
    for number in values:
        payload(f"weight {number.hex()}", weights={"bias": [number, *zeros[1:]]})
        payload(f"threshold {number.hex()}", conversion_threshold=number)
    for label, special in (("true", True), ("null", None), ("text", "1"), ("list", [])):
        payload("weight " + label, weights={"bias": [special, *zeros[1:]]})
        payload("threshold " + label, conversion_threshold=special)
    declared_versions: tuple[tuple[str, object], ...] = (
        ("int", feature_version), ("float", float(feature_version)), ("text", str(feature_version)), ("true", True),
        ("null", None), ("previous", feature_version - 1), ("next", feature_version + 1), ("missing", _MISSING))
    for label, declared in declared_versions:
        for version_prefix in (prefix, other_prefix):
            payload(f"feature_version {label} {version_prefix}", feature_version=declared, version=version_prefix + "probe")
    for label, version in (("null", None), ("number", 1), ("list", [prefix + "probe"]), ("upper", "Context-v1-probe"),
                           ("space", " " + prefix + "probe"), ("other", "context-v2-probe"), ("missing", _MISSING)):
        payload("version " + label, version=version)
    for label, changed in (("reversed", actions[::-1]), ("short", actions[:-1]), ("extra", [*actions, "extra"]),
                           ("null", None), ("text", actions[0]), ("missing", _MISSING)):
        payload("actions " + label, actions=changed)
    for label, weights in (("empty", {}), ("list", []), ("null", None), ("text", "bias"), ("values text", {"bias": "0"}),
                           ("values null", {"bias": None}), ("unsorted", {"b": zeros, "a": zeros}),
                           ("non-ascii", {"before:word:привет": zeros, "app:telegram": zeros, "char:ё": zeros})):
        payload("weights " + label, weights=weights)
    payload("weights missing", sign=False, weights=_MISSING)
    signed = cast(str, base["weights_sha256"])
    for label, digest in (("wrong", hashlib.sha256(b"").hexdigest()), ("upper", signed.upper()), ("null", None),
                          ("missing", _MISSING)):
        payload("checksum " + label, sign=False, weights_sha256=digest)
    for label, content in (("list", b"[]"), ("null", b"null"), ("number", b"0"), ("string", b'"x"'), ("empty", b""),
                           ("truncated", base_bytes[:-1]), ("invalid utf-8", b"\xff"),
                           ("bom", "\ufeff".encode() + base_bytes),
                           ("duplicate threshold", base_bytes[:-1] + b',"conversion_threshold":true}')):
        raw("raw " + label, content)
    historical: object = json.loads(baseline)
    if not isinstance(historical, dict) or not isinstance(historical.get("weights"), dict) or not historical["weights"]:
        raise ValueError("invalid historical feature-2 baseline")
    weights = cast(dict[str, list[float]], dict(historical["weights"]))
    first = next(iter(weights))
    weights[first] = [math.nextafter(float(weights[first][0]), math.inf), *weights[first][1:]]
    raw("baseline tampered", json.dumps({**historical, "weights": weights}, ensure_ascii=False).encode(), bulky=True)
    raw("baseline resigned", json.dumps({**historical, "weights": weights, "weights_sha256": _weights_sha256(weights)},
                                        ensure_ascii=False).encode(), bulky=True)
    limit = int(archived.MAX_FEATURES)
    for count in (limit, limit + 1):
        payload(f"features {count}", bulky=True, weights={f"f{index}": zeros for index in range(count)})
    size = int(archived.MAX_ARTIFACT_BYTES)
    for total in (size, size + 1):
        raw(f"bytes {total}", base_bytes, total - len(base_bytes), bulky=True)
    return payloads


def build_plan(archived: ModuleType, historical: bytes, seed_bytes: bytes, baseline: bytes) -> ProbePlan:
    """The frozen feature-2 probe set: every number it uses comes from the archived source."""

    tree = ast.parse(historical)
    model = _named(tree, "ContextModel")
    extract, load, init = _literals(_named(tree, "extract_context_features")), _literals(_named(model, "load")), _literals(_named(model, "__init__"))
    seed: object = json.loads(seed_bytes)
    if not isinstance(seed, dict) or seed.get("schema_version") != 1:
        raise ValueError("invalid feature-2 probe seed")
    records = _evidence_corpus(cast(dict[str, object], seed), _around(extract), _edges(extract))
    scores = _score_vectors(_edges(extract + load + init), len(archived.ACTIONS))
    payloads = _loader_payloads(archived, _around(load), _edges(load + init), baseline)
    document = {"records": records, "scores": [[value.hex() for value in vector] for vector in scores],
                "payloads": [[item.label, hashlib.sha256(item.content).hexdigest(), item.padding, item.feature_two,
                              item.bulky] for item in payloads]}
    digest = hashlib.sha256(json.dumps(document, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    evidence: list[tuple[dict[str, object], dict[str, object]]] = []
    for record in records:
        field = cast(dict[str, str], record["field"])
        arguments = {name: value for name, value in record.items() if name not in ("field", "extra")}
        arguments["field"] = FieldContext(field["application"], field["field_id"], field["before"], field["after"],
                                          cast(FieldRole, field["role"]))
        evidence.append((arguments, cast(dict[str, object], record["extra"])))
    return ProbePlan(int(archived.FEATURE_VERSION), tuple(evidence), tuple(scores), tuple(payloads), digest)


def _attempt(function: Callable[[], object], describe: Callable[[object], str] = repr) -> tuple[object | None, str]:
    """The value or the exception: an exception is behaviour too, compared by type and message."""

    try:
        value = function()
    except Exception as error:
        return None, f"raises {type(error).__name__}: {error}"
    return value, describe(value)


def _describe_model(model: object, feature_version: int) -> str:
    """Exact identity of a loaded model: names in order, every weight bit, value types."""

    weights = cast(dict[str, tuple[float, ...]], getattr(model, "weights"))
    digest = hashlib.sha256("\0".join(weights).encode("utf-8", "surrogatepass"))
    digest.update(array("d", itertools.chain.from_iterable(weights.values())).tobytes())
    types = sorted({type(values).__name__ for values in weights.values()}
                   | {type(value).__name__ for values in weights.values() for value in values})
    return (f"model {digest.hexdigest()} {types} {getattr(model, 'version')!r} "
            f"{getattr(model, 'conversion_threshold')!r} {getattr(model, 'feature_version', feature_version)!r}")


def _observe(module: ModuleType, plan: ProbePlan, payloads: Sequence[Path],
             artifacts: Sequence[tuple[str, Path, bool]], reference: bool) -> Iterator[tuple[str, str]]:
    """Everything feature schema 2 computes, in a fixed order, as exact text.

    `repr` spells every float exactly (it round-trips), and a dict keeps its order: the
    order of the features is the order in which scores are summed.
    """

    feature_version = plan.feature_version
    for name in ("ACTIONS", "FEATURE_VERSION", "MAX_ARTIFACT_BYTES", "MAX_FEATURES"):
        yield "declaration " + name, _attempt(lambda: (type(getattr(module, name)).__name__, getattr(module, name)))[1]
    yield "ContextPrediction fields", _attempt(lambda: [field.name for field in dataclasses.fields(module.ContextPrediction)])[1]
    for index, vector in enumerate(plan.scores):
        yield f"softmax #{index}", _attempt(lambda: module.softmax(list(vector)))[1]
    names = {field.name for field in dataclasses.fields(module.ContextEvidence)}
    evidence = [module.ContextEvidence(**arguments, **{name: value for name, value in extra.items() if name in names})
                for arguments, extra in plan.evidence]
    for index, item in enumerate(evidence):
        yield f"extract_context_features #{index}", _attempt(lambda: module.extract_context_features(item))[1]

    def describe(model: object) -> str:
        return _describe_model(model, feature_version)

    def load(path: Path, declared_feature_two: bool) -> tuple[object | None, str]:
        model, text = _attempt(lambda: module.ContextModel.load(path), describe)
        if declared_feature_two:
            return model, text
        if model is not None and getattr(model, "feature_version", feature_version) == feature_version:
            return None, _OUTSIDE_AS_FEATURE_TWO + text
        return None, _OUTSIDE

    def predictions(label: str, model: object, indices: Iterable[int]) -> Iterator[tuple[int, str, object | None, str]]:
        for index in indices:
            prediction, text = _attempt(lambda: getattr(model, "predict")(evidence[index]))
            yield index, f"{label} predict #{index}", prediction, text

    empty, text = _attempt(lambda: module.ContextModel({}, "context-v1-empty"), describe)
    yield "constructor defaults", text
    if empty is not None:
        # Equal scores: the first action wins the tie.
        for _index, step, _prediction, text in predictions("empty model", empty, range(len(module.ACTIONS))):
            yield step, text
    loads = list(zip(plan.payloads, payloads, strict=True))
    for payload, path in loads:
        if not payload.bulky:
            yield "load " + payload.label, load(path, payload.feature_two)[1]
    for label, path, declared_feature_two in artifacts:
        model, text = load(path, declared_feature_two)
        if reference and declared_feature_two and model is None:
            raise ValueError("historical feature-2 artifact cannot be loaded: " + label)
        yield "load " + label, text
        if model is None:
            continue
        convert = module.ACTIONS.index("convert")
        selected: dict[int, float] = {}
        for index, step, prediction, text in predictions(label, model, range(len(evidence))):
            yield step, text
            probabilities = cast(tuple[float, ...], getattr(prediction, "probabilities", ()))
            if probabilities and max(range(len(probabilities)), key=probabilities.__getitem__) == convert:
                selected[index] = probabilities[convert]
        if not selected:
            continue
        # A threshold equal to an observed conversion probability, and the next float
        # above it, pins the strictness of the threshold comparison.
        for index in dict.fromkeys((min(selected, key=selected.__getitem__), max(selected, key=selected.__getitem__))):
            for threshold in (selected[index], math.nextafter(selected[index], math.inf)):
                name = f"{label} threshold {threshold.hex()}"
                bounded, text = _attempt(lambda: module.ContextModel(
                    getattr(model, "weights"), getattr(model, "version"), threshold), lambda value: "constructed")
                yield name, text
                if bounded is not None:
                    for _index, step, _prediction, text in predictions(name, bounded, (index,)):
                        yield step, text
    for payload, path in loads:
        if payload.bulky:
            yield "load " + payload.label, load(path, payload.feature_two)[1]


def _compare(expected: Sequence[tuple[str, str]], observed: Iterable[tuple[str, str]]) -> None:
    """Exact equality, with one reviewed asymmetry for payloads outside feature schema 2.

    There the current loader may reject, or load as another feature schema, what the
    archive loaded as feature 2 (it rejects `"feature_version": 2.0`); it may never load
    such a payload as a feature-2 model the archive rejected or loaded differently.
    """

    for reference, current in itertools.zip_longest(expected, observed):
        if reference == current or (reference is not None and current is not None and reference[0] == current[0]
                                    and reference[1].startswith(_OUTSIDE_AS_FEATURE_TWO) and current[1] == _OUTSIDE):
            continue
        label = reference[0] if reference is not None else cast(tuple[str, str], current)[0]
        raise FeatureTwoChanged("feature-2 numerical behaviour changed: " + label)


def _digest(*parts: bytes) -> str:
    result = hashlib.sha256()
    for part in parts:
        result.update(hashlib.sha256(part).digest())
    return result.hexdigest()


# Results are pure functions of the bytes they are keyed by, so one process computes
# the archived reference once and repeats no verification of identical sources.
_REFERENCES: dict[str, tuple[ProbePlan, tuple[tuple[str, str], ...]]] = {}
_VERIFIED: set[str] = set()


def verify_feature_two(current: Path, historical: Path, directory: Path | None = None, root: Path = ROOT) -> None:
    """Fail unless the current feature-2 numerical path behaves exactly as the archived one.

    Both sources are executed side by side and compared on declarations, softmax,
    feature extraction, artifact loading and prediction over the pinned probe plan and
    the real feature-2 artifacts: the two historical models and the installed one while
    it declares feature schema 2. Source text is not compared, so moving numbers into
    named constants or reformatting passes; any numerical difference fails.
    """

    directory = root / HISTORY if directory is None else directory
    baseline, candidate = HISTORICAL_ARTIFACTS
    artifacts = [(baseline, directory / baseline), ("installed", root / INSTALLED_ARTIFACT), (candidate, directory / candidate)]
    current_bytes, historical_bytes, seed_bytes = current.read_bytes(), historical.read_bytes(), (root / PROBE_SEED).read_bytes()
    artifact_bytes = [path.read_bytes() for _label, path in artifacts]
    reference_key = _digest(historical_bytes, seed_bytes, *artifact_bytes)
    key = _digest(current_bytes, reference_key.encode())
    if key in _VERIFIED:
        return
    with tempfile.TemporaryDirectory(prefix="keyswitch-feature-two-") as temporary:
        folder = Path(temporary)
        cached = _REFERENCES.get(reference_key)
        if cached is None:
            archived: ModuleType | None = load_context_module(historical_bytes, historical, "archived")
            plan = build_plan(cast(ModuleType, archived), historical_bytes, seed_bytes, artifact_bytes[0])
        else:
            archived, plan = None, cached[0]
        if plan.digest != FEATURE_TWO_PLAN_SHA256:
            raise FeatureTwoChanged("feature-2 probe plan changed: " + plan.digest)
        payload_paths: list[Path] = []
        for index, payload in enumerate(plan.payloads):
            path = folder / f"payload-{index}.json"
            with path.open("wb") as target:
                target.write(payload.content)
                target.write(b" " * payload.padding)
            payload_paths.append(path)
        # The artifacts are loaded from copies of exactly the bytes the cache key hashed.
        artifact_paths: list[tuple[str, Path, bool]] = []
        for (label, source), content in zip(artifacts, artifact_bytes, strict=True):
            path = folder / ("artifact-" + source.name)
            path.write_bytes(content)
            artifact_paths.append((label, path, _declares_feature_two(content, plan.feature_version)))
        try:
            if archived is None:
                expected = cast(tuple[ProbePlan, tuple[tuple[str, str], ...]], cached)[1]
            else:
                expected = tuple(_observe(archived, plan, payload_paths, artifact_paths, reference=True))
                _REFERENCES[reference_key] = plan, expected
            module = load_context_module(current_bytes, current, "current")
            _compare(expected, _observe(module, plan, payload_paths, artifact_paths, reference=False))
        except FeatureTwoChanged:
            raise
        except Exception as error:
            raise FeatureTwoChanged(f"feature-2 behaviour cannot be observed: {type(error).__name__}: {error}") from error
    _VERIFIED.add(key)


def verify_anchors(directory: Path, root: Path = ROOT) -> dict[str, object]:
    for name, expected in ANCHORS.items():
        if checksum(directory / name) != expected:
            raise ValueError("historical context-v2 anchor changed: " + name)
    verify_sources(SOURCES, root)
    for name, field in (("candidate-seal.json", "provenance"), ("lexical-receipt.json", "source_hashes"),
                        ("engine-report.json", "provenance"), ("corpus-receipt.json", "builder_sha256")):
        value: object = json.loads((directory / name).read_bytes())
        if not isinstance(value, dict):
            raise ValueError("invalid historical evidence object")
        pins = {CORPUS_BUILDER: value.get(field)} if field == "builder_sha256" else value.get(field)
        verify_sources(pins, root)
    # Last, once every pin holds: the behavioural comparison is the slowest check.
    verify_feature_two(root / CURRENT_MODEL, CONTEXT_V2.path(CURRENT_MODEL, root), directory, root)
    return {"historical_evidence_verified": True, "current_runtime_verified": False,
            "feature_two_compatible": True, "scope": SCOPE, "generation_ref": GENERATION_REF}
