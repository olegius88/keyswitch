"""Find numbers the code spells out instead of naming.

Every number in the code, the tests and the tools, other than 0 and 1 (and -1,
which Python reads as 1 with a minus), is declared once as a named constant - an
UPPER_CASE name assigned at module or class level - and used by that name. A
number written in place hides what it means and lets two places that must agree
drift apart without anything failing: a wait that lapsed after ten seconds while
the text around it stayed context for forty-five is how `tot привет` came about.

Files whose bytes are pinned by accepted seals are listed in `PENDING_RESEAL`.
They are reported but do not fail the check: changing a single byte of them makes
the seal of a shipped model unverifiable, so they are brought under the rule in
the cycle that seals their model again, and taken off the list then. The list can
only shrink; a listed file that no longer holds such a number fails the check, so
it is not left on the list by mistake.

Usage: check_named_values.py [--pending]
"""

from __future__ import annotations

import ast
import sys
from collections.abc import Iterator, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Final

ROOT: Final = Path(__file__).resolve().parents[1]
SCANNED_ROOTS: Final = ("src", "tests", "tools", "apps")
# The additive and multiplicative identities: an empty count, a first index, a
# single step. Everything else says something and has a name.
NEUTRAL_NUMBERS: Final = frozenset({0, 1})
PENDING_RESEAL: Final = frozenset({
    "src/keyswitch/boundary_model.py",
    "src/keyswitch/boundary_policy.py",
    "src/keyswitch/context_model.py",
    "src/keyswitch/detector.py",
    "src/keyswitch/early_switch.py",
    "src/keyswitch/input_context.py",
    "src/keyswitch/intent_model.py",
    "src/keyswitch/language_model.py",
    "src/keyswitch/ortho_model.py",
    "src/keyswitch/prefix_model.py",
    "src/keyswitch/spellcheck.py",
    "tests/test_input_integrity.py",
    "tools/boundary_v2_corpus.py",
    "tools/context_corpus.py",
    "tools/context_evidence.py",
    "tools/context_frames.py",
    "tools/context_optimizer.py",
    "tools/environment_probe.py",
    "tools/evaluate_context_engine.py",
    "tools/evaluate_intent_model.py",
    "tools/freeze_intent_development_corpus.py",
    "tools/ortho_corpus.py",
    "tools/ortho_v2_corpus.py",
    "tools/ortho_v2_known.py",
    "tools/ortho_v2_verified.py",
    "tools/prefix_corpus.py",
    "tools/preseal_intent_holdout.py",
    "tools/train_boundary_model.py",
    "tools/train_boundary_v2.py",
    "tools/train_context_model.py",
    "tools/train_context_v2.py",
    "tools/train_intent_model.py",
    "tools/train_ortho_model.py",
    "tools/train_ortho_v2.py",
    "tools/train_prefix_model.py",
})


@dataclass(frozen=True)
class Finding:
    path: str
    line: int
    value: object

    def __str__(self) -> str:
        return f"{self.path}:{self.line}: {self.value!r}"


def _names_a_constant(node: ast.stmt) -> bool:
    if isinstance(node, ast.Assign):
        targets: list[ast.expr] = list(node.targets)
    elif isinstance(node, ast.AnnAssign):
        targets = [node.target]
    else:
        return False
    return all(isinstance(target, ast.Name) and target.id.isupper() for target in targets)


def _constant_values(body: Sequence[ast.stmt]) -> Iterator[ast.AST]:
    """Every node inside an UPPER_CASE assignment at module or class level."""

    for statement in body:
        if _names_a_constant(statement):
            yield from ast.walk(statement)
        elif isinstance(statement, ast.ClassDef):
            yield from _constant_values(statement.body)


def findings(path: Path) -> list[Finding]:
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    named = {id(node) for node in _constant_values(tree.body)}
    relative = path.relative_to(ROOT).as_posix()
    return [
        Finding(relative, node.lineno, node.value)
        for node in ast.walk(tree)
        if isinstance(node, ast.Constant)
        and isinstance(node.value, (int, float, complex))
        and not isinstance(node.value, bool)
        and node.value not in NEUTRAL_NUMBERS
        and id(node) not in named
    ]


def scanned_files(root: Path = ROOT) -> list[Path]:
    """Python files of the scanned trees, leaving out hidden directories (`.venv`, `.local`)."""

    return sorted(
        path for name in SCANNED_ROOTS for path in (root / name).rglob("*.py")
        if not any(part.startswith(".") for part in path.relative_to(root).parts)
    )


def main(argv: Sequence[str] | None = None) -> int:
    arguments = list(sys.argv[1:] if argv is None else argv)
    pending, violations = [], []
    for path in scanned_files():
        found = findings(path)
        relative = path.relative_to(ROOT).as_posix()
        if relative in PENDING_RESEAL:
            pending.append((relative, len(found)))
        else:
            violations.extend(found)
    cleaned = [name for name, count in pending if not count]
    for finding in violations:
        print(finding)
    for name in cleaned:
        print(f"{name}: holds no unnamed number any more; take it off PENDING_RESEAL")
    if "--pending" in arguments:
        for name, count in pending:
            print(f"pending reseal: {name}: {count}")
    return int(bool(violations or cleaned))


if __name__ == "__main__":
    raise SystemExit(main())
