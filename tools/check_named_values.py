"""Every value lives once, in a constants folder; everything else imports it.

The owner's rule (AGENTS.md): a number written in place hides what it means, and the
same meaning spelled in two places lets one of them be changed without the other - a
wait that lapsed after ten seconds next to a context kept for forty-five is how
`tot привет` came about. So:

1. Outside the constants folders no number other than 0 and 1 appears (-1 is 1 with a
   minus). The folders are `src/keyswitch/constants/` for the application and its
   tools, `tests/fixture_values/` for data only the tests use, and the same pair under
   `apps/logcourier/`.
2. Inside a constants folder a number appears only in a module-level constant.
3. A constant name is declared once across the folders of its application: the same
   name in two modules is two places for one meaning.
4. Outside the folders no constant is declared from numbers or from another constant:
   a second name for a value is exactly the duplicate this rule removes.
5. Outside the folders, in `src/`, `tools/` and `apps/`, a text does not spell a value:
   the constant changes and the sentence keeps the old number. A string literal that is
   not a docstring (nor another bare string statement, nor an f-string format spec) may
   not contain a number followed by a unit (секунд..., сек, минут..., мин, час..., дн...,
   день, МБ, МиБ, КБ, КиБ, ГБ, ГиБ, байт..., символ..., букв..., слов..., %, мс, ms, s,
   second(s), minute(s), hour(s), day(s), MB, MiB, KB, KiB, GB, GiB, byte(s),
   character(s), letter(s), word(s); a space or hyphen may stand between them), a range
   ("4–12", "4—12", "0..20", "от 1 до 80", "from 2 to 128", "between 0.25 and 8",
   "3 through 16"), a bound ("до 50", "не больше 50", "up to 50", "at least 5000",
   "at most 12"), a power ("2^20") or a Russian number word before a unit ("шесть
   часов"). Such a number comes from its constant through an f-string, and a Russian
   noun agrees with it through `keyswitch.russian_text.quantity` (LogCourier has its
   own `logcourier.russian_text`). Only these shapes are read as values, so a path, a
   version (0.16.2), a date, a hash (SHA-256), a name (X11, Win32), a sample of input
   (-1001234567890) or a numbered step ("1. Создайте") is not one. None of a unit ("0
   bytes", "0%"), all of a share ("100%"), and 0 or 1 as a bound or at both ends of a
   range say "none", "all" or "one" and need no name, as in rule 1. English number
   words are not read: the audited sequence protocol spells "four letters", and its
   bytes are pinned. Tests are left out: a test that pins wording spells it on purpose.

Files whose bytes are pinned by accepted seals are listed in `PENDING_RESEAL`. They are
reported but do not fail the check: changing a single byte of them makes the seal of a
shipped model unverifiable, so they follow the rule in the cycle that seals their model
again and leave the list then. The list can only shrink; a listed file that no longer
breaks the rule fails the check, so it is not left on the list by mistake.

Usage: check_named_values.py [--pending]
"""

from __future__ import annotations

import ast
import re
import sys
from collections.abc import Iterator, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Final

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src"))
from keyswitch.constants.repository import NEUTRAL_NUMBERS, NEUTRAL_PERCENTS

ROOT: Final = Path(__file__).resolve().parent.parent
SCANNED_ROOTS: Final = ("src", "tests", "tools", "apps")
# Each application's constants folders; names must be unique across the folders of one.
CONSTANTS_FOLDERS: Final = {
    "keyswitch": ("src/keyswitch/constants", "tests/fixture_values"),
    "logcourier": ("apps/logcourier/src/logcourier/constants", "apps/logcourier/tests/fixture_values"),
}
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
    "src/keyswitch/short_words.py",
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
    message: str

    def __str__(self) -> str:
        return f"{self.path}:{self.line}: {self.message}"


# Rule 5: where texts are read, and the shapes in which a text spells a value.
TEXT_ROOTS: Final = ("src/", "tools/", "apps/")
_UNITS: Final = "|".join((
    r"секунд\w*", "сек", r"минут\w*", "мин", "час(?:а|ов|у|ам|ами|ах|ом)?", "дн(?:я|ей|и|ям|ями|ях)", "день",
    "МБ", "МиБ", "КБ", "КиБ", "ГБ", "ГиБ", r"байт\w*", r"символ\w*", "букв(?:а|ы|у|ой|ам|ами|ах|е)?",
    "слов(?:о|а|у|ом|ам|ами|ах|е)?", "мс", "ms", "s", "seconds?", "minutes?", "hours?", "days?",
    "MB", "MiB", "KB", "KiB", "GB", "GiB", "bytes?", "characters?", "letters?", "words?", "%",
))
_RUSSIAN_NUMBER_WORDS: Final = "|".join((
    "дв(?:а|е|ух|умя)", "тр(?:и|[её]х|емя)", "четыр(?:е|[её]х|ьмя)", "(?:пят|шест|сем|девят|десят)(?:ь|и|ью)",
    "восем(?:ь|ью)", "восьми",
    "(?:один|две|три|четыр|пят|шест|сем|восем|девят)надцат(?:ь|и|ью)", "(?:два|три)дцат(?:ь|и|ью)",
    "сорок(?:а)?", "(?:пять|шесть|семь|восемь)десят", "(?:пяти|шести|семи|восьми)десяти", "девяност[оа]",
    "сто", "ста", "двести", "триста", "четыреста", "(?:пять|шесть|семь|восемь|девять)сот", "тысяч(?:а|и|у|ей)?",
    "полтор[аы]",
))


def _number(name: str, *, after_separator: bool = False) -> str:
    """A written number that is not part of a word, a version, a date or a hyphenated name.

    The second number of a range follows the range's own separator ("0..20"), so a dot
    before it does not make it part of a version.
    """

    before = "" if after_separator else r"(?<![\w.,\-])"
    return before + rf"(?P<{name}>\d+(?:[.,]\d+)?)(?![\w:^]|[.,]\d)"


_UNIT_VALUE: Final = re.compile(
    r"(?<![\w.,\-])(?P<value>\d+(?:[.,]\d+)?)[ \u00a0\u202f\-]?(?P<unit>" + _UNITS + r")(?!\w)")
_RANGES: Final = (
    re.compile(_number("low") + r"[ \u00a0]*(?:–|—|\.\.)[ \u00a0]*" + _number("high", after_separator=True)),
    re.compile(r"(?<!\w)(?i:от|from|between)\s+" + _number("low") + r"\s+(?i:до|to|and)\s+" + _number("high")),
    re.compile(_number("low") + r"\s+through\s+" + _number("high")),
)
_BOUND: Final = re.compile(
    r"(?<!\w)(?i:до|не более|не менее|не больше|не меньше|up to|at least|at most|no more than|no less than"
    r"|no fewer than)\s+" + _number("value"))
_POWER: Final = re.compile(r"(?<![\w.,\-])\d+\^\d+")
_RUSSIAN_NUMBER_WORD_UNIT: Final = re.compile(
    r"(?<!\w)(?i:" + _RUSSIAN_NUMBER_WORDS + r")[ \u00a0]+(?:" + _UNITS + r")(?!\w)")


def _value(text: str) -> float:
    return float(text.replace(",", "."))


def text_values(text: str) -> list[str]:
    """The fragments of a text that spell a value (rule 5), in order and without repeats."""

    found = [match.group(0) for match in _UNIT_VALUE.finditer(text)
             if not (_value(match["value"]) == 0
                     or (match["unit"] == "%" and _value(match["value"]) in NEUTRAL_PERCENTS))]
    found.extend(match.group(0) for pattern in _RANGES for match in pattern.finditer(text)
                 if not {_value(match["low"]), _value(match["high"])} <= NEUTRAL_NUMBERS)
    found.extend(match.group(0) for match in _BOUND.finditer(text) if _value(match["value"]) not in NEUTRAL_NUMBERS)
    found.extend(match.group(0) for pattern in (_POWER, _RUSSIAN_NUMBER_WORD_UNIT) for match in pattern.finditer(text))
    return list(dict.fromkeys(found))


def reads_texts(relative: str) -> bool:
    """Rule 5 reads the application, the tools and the other applications, not tests or constants."""

    return (relative.startswith(TEXT_ROOTS) and "tests" not in Path(relative).parts
            and not in_constants_folder(relative))


def _not_texts(tree: ast.Module) -> set[int]:
    """Docstrings, other bare string statements and f-string format specs."""

    skipped: set[int] = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Expr) and isinstance(node.value, ast.Constant):
            skipped.add(id(node.value))
        elif isinstance(node, ast.FormattedValue) and node.format_spec is not None:
            skipped.update(id(sub) for sub in ast.walk(node.format_spec))
    return skipped


def text_findings(tree: ast.Module, relative: str) -> list[Finding]:
    skipped = _not_texts(tree)
    found: list[Finding] = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Constant) and isinstance(node.value, str) and id(node) not in skipped:
            values = text_values(node.value)
            if values:
                found.append(Finding(relative, node.lineno, f"text spells {', '.join(map(repr, values))}; "
                                     "format it from the constant it describes"))
    return found


def in_constants_folder(relative: str) -> bool:
    return any(relative.startswith(folder + "/") for folders in CONSTANTS_FOLDERS.values() for folder in folders)


def _is_number(node: ast.AST) -> bool:
    return (isinstance(node, ast.Constant) and isinstance(node.value, (int, float, complex))
            and not isinstance(node.value, bool))


def _numbers(node: ast.AST) -> Iterator[ast.Constant]:
    return (sub for sub in ast.walk(node) if isinstance(sub, ast.Constant) and _is_number(sub))


def _constant_targets(node: ast.stmt) -> list[str]:
    if isinstance(node, ast.Assign):
        targets: list[ast.expr] = list(node.targets)
    elif isinstance(node, ast.AnnAssign) and node.value is not None:
        targets = [node.target]
    else:
        return []
    names = [target.id for target in targets if isinstance(target, ast.Name)]
    return names if names and all(name.isupper() for name in names) else []


def _declared_value(node: ast.stmt) -> ast.expr | None:
    return node.value if isinstance(node, (ast.Assign, ast.AnnAssign)) else None


TABLE_BUILDERS: Final = frozenset({"frozenset", "tuple", "set"})


CONSTANTS_MODULE_PREFIXES: Final = ("keyswitch.constants", "logcourier.constants", "fixture_values", ".constants")


def _imported_values(tree: ast.Module) -> set[str]:
    """Names a module imports from a constants folder."""

    names: set[str] = set()
    for node in tree.body:
        if isinstance(node, ast.ImportFrom):
            module = "." * node.level + (node.module or "")
            if module.startswith(CONSTANTS_MODULE_PREFIXES):
                names.update(alias.asname or alias.name for alias in node.names)
    return names


def _built_from_values(node: ast.expr, values: set[str]) -> bool:
    """Numbers and names of values, joined by arithmetic or collected into tables.

    Such an expression is itself a value and belongs in a constants folder; a table of
    classes, functions, strings or objects, or an empty cache, is not.
    """

    allowed = (ast.Constant, ast.Name, ast.Attribute, ast.UnaryOp, ast.BinOp, ast.Tuple, ast.List,
               ast.Set, ast.Dict, ast.Load, ast.operator, ast.unaryop, ast.Call)
    leaves = 0
    for sub in ast.walk(node):
        if not isinstance(sub, allowed):
            return False
        if isinstance(sub, ast.Call):
            if not (isinstance(sub.func, ast.Name) and sub.func.id in TABLE_BUILDERS and not sub.keywords):
                return False
        elif isinstance(sub, ast.Constant):
            if not _is_number(sub):
                return False
            leaves += 1
        elif isinstance(sub, ast.Name) and sub.id not in TABLE_BUILDERS:
            if sub.id not in values:
                return False
            leaves += 1
        elif isinstance(sub, ast.Attribute):
            return False
    return leaves > 0


def _is_enumeration(node: ast.ClassDef) -> bool:
    """Members of an Enum are its values' names, not constants to move."""

    return any((isinstance(base, ast.Name) and base.id.endswith(("Enum", "Flag")))
               or (isinstance(base, ast.Attribute) and base.attr.endswith(("Enum", "Flag"))) for base in node.bases)


def _class_bodies(body: Sequence[ast.stmt]) -> Iterator[Sequence[ast.stmt]]:
    yield body
    for statement in body:
        if isinstance(statement, ast.ClassDef) and not _is_enumeration(statement):
            yield from _class_bodies(statement.body)


def findings(path: Path, root: Path = ROOT) -> list[Finding]:
    relative = path.relative_to(root).as_posix()
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    found: list[Finding] = []
    if in_constants_folder(relative):
        declared = {id(sub) for statement in tree.body if _constant_targets(statement) for sub in ast.walk(statement)}
        found.extend(Finding(relative, node.lineno, f"{node.value!r} outside a module-level constant")
                     for node in _numbers(tree) if id(node) not in declared)
        return found
    found.extend(Finding(relative, node.lineno, f"{node.value!r} belongs in a constants folder")
                 for node in _numbers(tree) if node.value not in NEUTRAL_NUMBERS)
    if reads_texts(relative):
        found.extend(text_findings(tree, relative))
    values = _imported_values(tree)
    for body in _class_bodies(tree.body):
        for statement in body:
            value = _declared_value(statement)
            targets = _constant_targets(statement)
            if targets and value is not None and _built_from_values(value, values):
                values.update(targets)
                found.append(Finding(relative, statement.lineno,
                                     f"{', '.join(targets)} is a value; declare it in a constants folder"))
    return found


def duplicate_names(root: Path = ROOT) -> list[Finding]:
    """Constant names declared in more than one module of an application's constants folders."""

    found: list[Finding] = []
    for folders in CONSTANTS_FOLDERS.values():
        seen: dict[str, str] = {}
        for folder in folders:
            for path in sorted((root / folder).glob("*.py")):
                relative = path.relative_to(root).as_posix()
                for statement in ast.parse(path.read_text(encoding="utf-8")).body:
                    for name in _constant_targets(statement):
                        if name in seen and seen[name] != relative:
                            found.append(Finding(relative, statement.lineno, f"{name} is also declared in {seen[name]}"))
                        seen.setdefault(name, relative)
    return found


def scanned_files(root: Path = ROOT) -> list[Path]:
    """Python files of the scanned trees, leaving out hidden directories (`.venv`, `.local`)."""

    return sorted(
        path for name in SCANNED_ROOTS for path in (root / name).rglob("*.py")
        if not any(part.startswith(".") for part in path.relative_to(root).parts)
    )


def main(argv: Sequence[str] | None = None) -> int:
    arguments = list(sys.argv[1:] if argv is None else argv)
    pending: list[tuple[str, int]] = []
    violations: list[Finding] = duplicate_names()
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
        print(f"{name}: follows the rule now; take it off PENDING_RESEAL")
    if "--pending" in arguments:
        for name, count in pending:
            print(f"pending reseal: {name}: {count}")
    return int(bool(violations or cleaned))


if __name__ == "__main__":
    raise SystemExit(main())
