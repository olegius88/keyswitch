"""Blind physical-family reconciliation; no prediction or test-quality scoring."""

from __future__ import annotations

import argparse
import ast
from collections import Counter, defaultdict
from collections.abc import Callable, Iterable, Iterator, Mapping, Sequence
from dataclasses import asdict, dataclass, replace
from functools import lru_cache
import gzip
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import sys
from typing import Protocol, cast
import unicodedata

from context_physical_keys import KEYS, translated
from freeze_context_action_corpus import (
    CorpusRow, Sentence, SurfaceToken, Union, WORDS, canonical, checksum, digest,
    row_identifier, typo_variants,
)
from keyswitch.layouts import LayoutPair
from keyswitch.short_words import TRUSTED_SHORT_WORDS
from keyswitch.constants.model_protocol import ACTIVE_SPLITS, ALL_SPLITS, FITTING_SPLITS
from keyswitch.constants.corpus import (
    ADD_CALL_AFTER_ARG_INDEX,
    ADD_CALL_BEFORE_ARG_INDEX,
    ADD_CALL_MIN_ARGS,
    PHYSICAL_ALIAS_CACHE_SIZE,
    UNKNOWN_FAMILY_CALL_CONTEXTS_ARG_INDEX,
    UNKNOWN_FAMILY_CALL_MIN_ARGS,
)

ROOT = Path(__file__).resolve().parents[1]
PAIR = LayoutPair()
POSITIONS = tuple({key.characters[group]: key.keycode for key in reversed(KEYS)} for group in (0, 1))


class FrozenAPI(Protocol):
    physical: Callable[[str], str]

    def source_inventory(self, source_root: Path) -> tuple[list[tuple[str, Path]], list[dict[str, object]]]: ...
    def choose_sentences(self, paths: Sequence[tuple[str, Path]], namespace: str, max_documents: int, max_sentences: int) -> tuple[set[tuple[str, str, str]], dict[str, object]]: ...
    def read_conllu(self, path: Path, source: str) -> Iterator[Sentence]: ...
    def exposed_families(self, repository: Path, additional: Sequence[Path] = ()) -> tuple[set[str], dict[str, str]]: ...
    def partition(self, sentences: Sequence[Sentence], namespace: str, exposed: set[str]) -> tuple[list[CorpusRow], dict[str, object]]: ...


def read_object(path: Path) -> dict[str, object]:
    value: object = json.loads(path.read_bytes())
    if not isinstance(value, dict) or any(not isinstance(key, str) for key in value):
        raise ValueError("expected JSON object")
    return cast(dict[str, object], value)


def frozen_api(directory: Path, manifest: Mapping[str, object]) -> FrozenAPI:
    source = directory / "generator-source.py"
    if checksum(source) != manifest.get("generator_sha256"):
        raise ValueError("frozen generator checksum mismatch")
    spec = importlib.util.spec_from_file_location("_frozen_context_family_reconciliation", source)
    if spec is None or spec.loader is None:
        raise ValueError("frozen generator unavailable")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return cast(FrozenAPI, module)


@lru_cache(maxsize=PHYSICAL_ALIAS_CACHE_SIZE)
def physical_aliases(form: str) -> frozenset[str]:
    """Hash every supported whole-layout or script-guided physical reading."""
    text = unicodedata.normalize("NFC", form).casefold().replace("’", "'")
    aliases: set[str] = set()
    for initial in (0, 1):
        for guided in (False, True):
            group = initial
            positions: list[int] = []
            for char in text:
                if guided:
                    if "a" <= char <= "z":
                        group = 0
                    elif "а" <= char <= "я" or char == "ё":
                        group = 1
                    if char not in POSITIONS[group] and char in POSITIONS[1 - group]:
                        group = 1 - group
                position = POSITIONS[group].get(char)
                if position is None:
                    break
                positions.append(position)
            else:
                aliases.add(digest("physical-key-positions-v1:" + ",".join(map(str, positions))))
    # An unsupported historical spelling still reserves its exact normalized
    # form; unknown glyphs are never erased from family identities.
    aliases.add(digest("normalized-literal-v1:" + text))
    return frozenset(aliases)


def expanded_aliases(form: str) -> set[str]:
    result: set[str] = set()
    for value in (form, PAIR.translate(form, "us", "ru"), PAIR.translate(form, "ru", "us")):
        result.update(physical_aliases(value))
    for group in (0, 1):
        try:
            result.update(physical_aliases(translated(form, group)))
        except ValueError:
            continue
    return result


def family_values(token: SurfaceToken, identifier: str) -> tuple[str, ...]:
    return (token.form, *token.lemmas, *typo_variants(token.form, identifier))


def historical_code_forms(source: str) -> set[str]:
    """Read declared context literals; never execute the historical scorer."""
    forms = {word for words in TRUSTED_SHORT_WORDS.values() for word in words}

    def include(expression: ast.AST) -> None:
        for part in ast.walk(expression):
            if isinstance(part, ast.Constant) and isinstance(part.value, str):
                forms.add(part.value)
                forms.update(WORDS.findall(part.value))

    for node in ast.walk(ast.parse(source)):
        if isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name) and node.target.id in {"contexts", "phrases"}:
            if node.value is not None:
                include(node.value)
        elif isinstance(node, ast.Assign) and any(isinstance(target, ast.Name) and target.id in {"contexts", "phrases"} for target in node.targets):
            include(node.value)
        elif isinstance(node, ast.AugAssign) and isinstance(node.target, ast.Name) and node.target.id == "contexts":
            include(node.value)
        elif isinstance(node, ast.For) and isinstance(node.target, ast.Name) and node.target.id in {"before", "following"}:
            include(node.iter)
        elif isinstance(node, ast.Call) and isinstance(node.func, ast.Name):
            if node.func.id == "add" and len(node.args) >= ADD_CALL_MIN_ARGS:
                include(node.args[ADD_CALL_BEFORE_ARG_INDEX])
                include(node.args[ADD_CALL_AFTER_ARG_INDEX])
            elif node.func.id == "unknown_family" and len(node.args) >= UNKNOWN_FAMILY_CALL_MIN_ARGS:
                include(node.args[UNKNOWN_FAMILY_CALL_CONTEXTS_ARG_INDEX])
    return forms


def sequence_document_counts(rows: Sequence[CorpusRow]) -> dict[str, object]:
    """Count the evaluator's model-blind selection and physical representability."""
    from evaluate_context_action_sequences import select_rows, sequence_plan

    selected = select_rows([row for row in rows if row.split == "test"])
    supported: dict[str, set[str]] = defaultdict(set)
    unsupported: Counter[str] = Counter()
    for row in selected:
        group = str(row.group)
        try:
            plans = [sequence_plan(row, False)]
            if row.group in (0, 1):
                plans.append(sequence_plan(row, True))
            representable = all(not plan.unsupported for plan in plans)
        except ValueError:
            representable = False
        if representable:
            supported[group].add(row.document)
        else:
            unsupported[group] += 1
    return {"selected_rows": len(selected), "screenable_documents_by_group": {str(group): len(supported[str(group)]) for group in (0, 1, None)},
            "unsupported_selected_by_group": dict(unsupported), "scope": "physical representability only; no model scoring"}


def test_membership(rows: Sequence[CorpusRow], namespace: str) -> dict[str, object]:
    held = [row for row in rows if row.split == "test"]
    return {"namespace": namespace, "scope": "prospective sequence holdout; not globally unseen lexicon",
            "row_ids_sha256": sorted(digest(row.identifier) for row in held),
            "family_ids_sha256": sorted({row.family for row in held}),
            "document_ids_sha256": sorted({digest(row.document) for row in held})}


def validate_original(directory: Path, manifest: Mapping[str, object], rows: Sequence[CorpusRow]) -> None:
    records = cast(dict[str, dict[str, object]], manifest["splits"])
    for split in ALL_SPLITS:
        record = records[split]
        if record["path"] != split + ".jsonl.gz" or checksum(directory / str(record["path"])) != record["sha256"]:
            raise ValueError("original compressed split changed")
        selected = [row for row in rows if row.split == split]
        digestor = hashlib.sha256()
        for row in selected:
            digestor.update(canonical(asdict(row)))
        if len(selected) != record["rows"] or digestor.hexdigest() != record["content_sha256"]:
            raise ValueError("reconstructed original split differs")
    membership = directory / "test-membership.json"
    if checksum(membership) != manifest["test_membership_sha256"]:
        raise ValueError("original test membership changed")
    if canonical(test_membership(rows, str(manifest["namespace"]))) != membership.read_bytes():
        raise ValueError("reconstructed test membership differs")


@dataclass
class Reconciled:
    rows: list[CorpusRow]
    sidecar: dict[str, object]
    summary: dict[str, object]


def reconcile_rows(sentences: Sequence[Sentence], rows: Sequence[CorpusRow], historical: Iterable[str]) -> Reconciled:
    union = Union()
    by_id = {row.identifier: row for row in rows}
    values_by_id: dict[str, tuple[str, ...]] = {}
    for sentence in sentences:
        for token in sentence.tokens:
            identifier = row_identifier(sentence, token)
            row = by_id[identifier]
            values = family_values(token, identifier)
            values_by_id[identifier] = values
            anchor = "family:" + row.family
            union.find(anchor)
            for value in values:
                for alias in expanded_aliases(value):
                    union.join(anchor, "alias:" + alias)
    if set(values_by_id) != set(by_id):
        raise ValueError("source alias inventory does not cover original rows")

    def matching_roots(forms: Iterable[str]) -> set[str]:
        roots: set[str] = set()
        for form in forms:
            for alias in expanded_aliases(form):
                key = "alias:" + alias
                if key in union.parents:
                    roots.add(union.find(key))
        return roots

    historical_roots = matching_roots(historical)
    seen_roots = set(historical_roots)
    for row in rows:
        if row.split not in FITTING_SPLITS:
            continue
        seen_roots.add(union.find("family:" + row.family))
        # Prior fit saw context words too, including tokens whose focus row was
        # quarantined. A future drop cannot erase that earlier exposure.
        seen_roots.update(matching_roots((*values_by_id[row.identifier],
                                         *WORDS.findall(row.before), *WORDS.findall(row.after))))
    splits_by_root: dict[str, set[str]] = defaultdict(set)
    for row in rows:
        if row.split in ACTIVE_SPLITS:
            splits_by_root[union.find("family:" + row.family)].add(row.split)
    conflicts = {root for root, splits in splits_by_root.items() if len(splits) > 1}
    test_roots = {union.find("family:" + row.family) for row in rows if row.split == "test"}
    contaminated = test_roots & seen_roots
    changed: list[CorpusRow] = []
    removed: list[dict[str, object]] = []
    original_counts = Counter(row.split for row in rows)
    for row in rows:
        root = union.find("family:" + row.family)
        reasons = list(row.quarantine_reasons)
        if row.split in ACTIVE_SPLITS:
            if root in conflicts:
                reasons.append("shift-closure-crosses-original-splits")
            if row.split != "train" and root in historical_roots:
                reasons.append("shift-closure-historical-exposure")
            if row.split == "test" and root in contaminated:
                reasons.append("shift-closure-prior-fit-exposure")
            if row.split != "test" and row.group in (0, 1) and row.layout_representable:
                try:
                    for value in (row.original, *typo_variants(row.original, row.identifier)):
                        translated(value, row.group)
                except ValueError:
                    reasons.append("unsupported-exact-physical-glyph")
        if row.split != "quarantine" and reasons:
            removed.append({"row_sha256": digest(row.identifier), "previous_split": row.split,
                            "closure_sha256": digest(root), "reasons": sorted(set(reasons))})
            changed.append(replace(row, split="quarantine", quarantine_reasons=tuple(sorted(set(reasons)))))
        elif row.split in FITTING_SPLITS:
            changed.append(replace(row, family=digest(root)))
        else:
            changed.append(row)
    retained_roots: dict[str, set[str]] = defaultdict(set)
    for row in changed:
        if row.split in ACTIVE_SPLITS:
            old = by_id[row.identifier]
            retained_roots[row.split].add(union.find("family:" + old.family))
    for index, left in enumerate(ACTIVE_SPLITS):
        for right in ACTIVE_SPLITS[index + 1:]:
            if retained_roots[left] & retained_roots[right]:
                raise ValueError("remaining partitions share physical closure")
    if retained_roots["test"] & seen_roots:
        raise ValueError("remaining test shares previously exposed physical closure")
    components: dict[str, list[str]] = defaultdict(list)
    aliases: dict[str, str] = {}
    for name in union.parents:
        component = digest(union.find(name))
        if name.startswith("family:"):
            components[component].append(name.removeprefix("family:"))
        else:
            aliases[name.removeprefix("alias:")] = component
    counts = Counter(row.split for row in changed)
    documents = {str(group): len({row.document for row in changed if row.split == "test" and row.group == group})
                 for group in (0, 1, None)}
    summary: dict[str, object] = {"original_counts": dict(original_counts), "remaining_counts": dict(counts),
        "removed_counts": {split: original_counts[split] - counts[split] for split in ACTIVE_SPLITS},
        "test_contaminated_rows": sum(row.split == "test" and union.find("family:" + row.family) in contaminated for row in rows),
        "test_contaminated_components": len(contaminated), "remaining_test_documents_by_group": documents,
        "cross_split_components": len(conflicts), "prior_exposed_components": len(seen_roots),
        "remaining_cross_split_overlap": 0, "remaining_test_prior_exposure_overlap": 0, "replacements": 0,
        "scope": "blind physical-family isolation; no scoring, predictions, test examples or quality claims",
        "physical_selection": sequence_document_counts(changed)}
    sidecar: dict[str, object] = {"schema_version": 1, "family_policy": "physical positions, Shift ignored; old aliases plus declared exact and old-layout variants",
        "components": {key: sorted(value) for key, value in sorted(components.items())},
        "alias_sha256_to_closure_sha256": dict(sorted(aliases.items())),
        "prior_exposed_closure_sha256": sorted(digest(root) for root in seen_roots),
        "parent_test_closure_sha256": sorted(digest(root) for root in test_roots),
        "retained_test_closure_sha256": sorted(digest(root) for root in retained_roots["test"]),
        "removed": removed}
    return Reconciled(changed, sidecar, summary)


def audit(directory: Path, source_root: Path) -> tuple[Reconciled, dict[str, object]]:
    manifest = read_object(directory / "manifest.json")
    api = frozen_api(directory, manifest)
    if checksum(ROOT / "src/keyswitch/layouts.py") != manifest["layout_table_sha256"]:
        raise ValueError("original layout mapper changed")
    paths, sources = api.source_inventory(source_root)
    if sources != manifest["sources"]:
        raise ValueError("pinned source inventory differs from original freeze")
    exposure = cast(dict[str, str], manifest["exposure_inputs"])
    for name, expected in exposure.items():
        path = ROOT / name
        if not path.resolve().is_relative_to(ROOT) or checksum(path) != expected:
            raise ValueError("historical exposure input changed")
    originals: set[str] = set()
    physical = api.physical

    def capture(form: str) -> str:
        originals.add(form)
        return physical(form)

    api.physical = capture
    try:
        exposed, provenance = api.exposed_families(ROOT, [ROOT / name for name in exposure])
    finally:
        api.physical = physical
    if provenance != exposure or digest("\n".join(sorted(exposed))) != manifest["exposed_membership_sha256"]:
        raise ValueError("historical exposure reconstruction differs")
    originals.update(historical_code_forms((ROOT / "tools/train_context_model.py").read_text(encoding="utf-8")))
    sampling = cast(dict[str, int], manifest["sampling"])
    chosen, metadata = api.choose_sentences(paths, str(manifest["namespace"]), sampling["max_documents_per_source"], sampling["max_sentences_per_document"])
    if metadata != manifest["sampling"]:
        raise ValueError("source sampling changed")
    sentences = [sentence for source, path in paths for sentence in api.read_conllu(path, source)
                 if (source, path.name, sentence.identifier) in chosen]
    rows, partitioning = api.partition(sentences, str(manifest["namespace"]), exposed)
    if partitioning != manifest["partitioning"]:
        raise ValueError("original partitioning changed")
    validate_original(directory, manifest, rows)
    result = reconcile_rows(sentences, rows, originals)
    result.summary.update({"parent_manifest_sha256": checksum(directory / "manifest.json"),
                           "parent_test_membership_sha256": manifest["test_membership_sha256"],
                           "namespace": manifest["namespace"], "original_reconstruction_verified": True})
    return result, manifest


def write_derivative(directory: Path, output: Path, result: Reconciled, manifest: Mapping[str, object]) -> None:
    if output.exists():
        raise ValueError("refusing to overwrite a corpus directory")
    if output.resolve() == directory.resolve() or output.resolve().is_relative_to(directory.resolve()):
        raise ValueError("derivative must not be inside original corpus")
    if checksum(directory / "manifest.json") != result.summary["parent_manifest_sha256"]:
        raise ValueError("parent changed after reconciliation")
    output.mkdir(parents=True)
    files: dict[str, dict[str, object]] = {}
    records = cast(dict[str, dict[str, object]], manifest["splits"])
    for split in ALL_SPLITS:
        selected = [row for row in result.rows if row.split == split]
        path = output / (split + ".jsonl.gz")
        digestor = hashlib.sha256()
        for row in selected:
            digestor.update(canonical(asdict(row)))
        if split == "test" and digestor.hexdigest() == records[split]["content_sha256"]:
            shutil.copyfile(directory / path.name, path)
        else:
            with path.open("wb") as destination, gzip.GzipFile(fileobj=destination, filename="", mode="wb", mtime=0) as stream:
                for row in selected:
                    stream.write(canonical(asdict(row)))
        files[split] = {"path": path.name, "sha256": checksum(path), "content_sha256": digestor.hexdigest(), "rows": len(selected)}
    (output / "test-membership.json").write_bytes(canonical(test_membership(result.rows, str(manifest["namespace"]))))
    (output / "physical-family-closure.json").write_bytes(canonical(result.sidecar))
    shutil.copyfile(directory / "generator-source.py", output / "generator-source.py")
    (output / "reconciliation-source.py").write_bytes(Path(__file__).read_bytes())
    updated = {**manifest, "splits": files, "test_membership_sha256": checksum(output / "test-membership.json"),
        "parent_manifest_sha256": result.summary["parent_manifest_sha256"],
        "parent_test_membership_sha256": manifest["test_membership_sha256"],
        "generator_role": "unchanged historical freezer; derivative generated by reconciliation-source.py",
        "reconciliation": {"summary": result.summary, "closure_sha256": checksum(output / "physical-family-closure.json"),
                           "source_sha256": checksum(output / "reconciliation-source.py"),
                           "physical_keys_sha256": checksum(ROOT / "tools/context_physical_keys.py"),
                           "test_policy": "parent test minus all formally exposed closure; no replacements or new namespace"}}
    (output / "manifest.json").write_bytes(canonical(updated))


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--source-root", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        if args.report.resolve().is_relative_to(args.corpus.resolve()) or args.report.resolve().is_relative_to(args.source_root.resolve()):
            raise ValueError("report must not overwrite corpus or source inputs")
        result, manifest = audit(args.corpus, args.source_root)
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_bytes(canonical(result.summary))
        print(json.dumps(result.summary, sort_keys=True), flush=True)
        if args.output is not None:
            write_derivative(args.corpus, args.output, result, manifest)
    except PermissionError as error:
        print(json.dumps({"status": "access-denied", "operation": "blind physical-family reconciliation", "path": error.filename}), flush=True)
        return 1
    except Exception as error:
        # Never put a source sentence, row ID or third-party parser exception
        # into the observer's tool output.
        print(json.dumps({"status": "blind-audit-failed", "error_type": type(error).__name__}), flush=True)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
