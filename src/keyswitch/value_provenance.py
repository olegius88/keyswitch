"""Pin the named values a set of source files uses, rather than the files that hold them.

A seal that hashes the bytes of its toolchain files stops seeing a value once the value
moves into `keyswitch.constants`: the file keeps `from keyswitch.constants.training
import THRESHOLD` byte for byte while the threshold changes. Hashing the whole constants
modules instead would pin too much - a UI range in `settings_defaults` would break the
seal of a model that never reads it. So this module pins exactly what the files import:

1. Every given file is parsed (never imported) and every name it takes from a
   `keyswitch.constants` module is collected: `from keyswitch.constants.X import NAME`,
   the relative forms `from .constants.X import NAME` (a file in `keyswitch/`) and
   `from .X import NAME` (a file in `keyswitch/constants/`), and a module bound by
   `import keyswitch.constants.X as m` or `from keyswitch.constants import X` and read
   as `m.NAME`. A form that hides which names it reads - `import *`, or the module object
   itself passed on or read by `getattr` - is refused rather than guessed at.
2. The constants modules are executed from their source files under the given source
   root, apart from the package this process imported: the values come from the tree
   being verified, never from cached bytecode, and a mirror of the tree can be checked
   without touching `sys.modules`. If this process already imported the same file, its
   values must agree with the file, so the pinned value is the value in use.
3. Each value is written in a canonical form that does not depend on the run, the hash
   seed or the platform, and the digest is SHA-256 over canonical JSON of the sorted
   `module.NAME` -> canonical value mapping.

Values are resolved, so a constant derived from other constants
(`KSLM_MAX_CONTAINER_BYTES = 14 * BYTES_PER_MEBIBYTE`) is pinned by its result, and the
constants that a constants module imports need no entry of their own: a change of
`BYTES_PER_MEBIBYTE` moves every pinned value computed from it and nothing else.

Canonical forms, each a two-item list `[tag, payload]`:

* `None`, `bool`, `int`, `float`: the exact type name and `repr()`. `repr()` of a float
  is the shortest string that reads back to the same double when
  `sys.float_repr_style` is "short" (CPython formats it itself, not through the C
  library); another style is refused.
* `str`: the text itself, escaped to ASCII by the JSON encoder. `repr()` is not used
  for text: it escapes exactly the characters `str.isprintable()` rejects, and that
  depends on the Unicode database of the interpreter.
* `tuple`, `list`: the items in order; `dict`: the `[key, value]` pairs in insertion
  order; `set`, `frozenset`: the items sorted by their canonical JSON, so iteration
  order under a hash seed does not matter.
* Anything else, including subclasses of these types (an `IntEnum`, a named tuple, a
  `Path`), raises `NonCanonicalValue`, so a value without a portable form cannot slip
  into a digest silently.

The digest algorithm itself is not part of the digest: a changed algorithm computes a
different digest for the same values, and every verifier then refuses the recorded
one - loudly, which is the direction a seal has to fail in.
"""

from __future__ import annotations

import ast
import builtins
import hashlib
import importlib.util
import json
import sys
from collections.abc import Iterable, Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from types import MappingProxyType, ModuleType
from typing import Final, TypeAlias

CONSTANTS_PACKAGE: Final = "keyswitch.constants"
_CONSTANTS_PREFIX: Final = CONSTANTS_PACKAGE + "."
_TOP_PACKAGE: Final = CONSTANTS_PACKAGE.partition(".")[0]
_SHORTEST_FLOAT_REPR: Final = "short"
_SCALAR_TYPES: Final = (type(None), bool, int, float)
_SEQUENCE_TYPES: Final = (tuple, list)
_SET_TYPES: Final = (set, frozenset)
_JSON_SEPARATORS: Final = (",", ":")

CanonicalValue: TypeAlias = list[object]


class ValueProvenanceError(ValueError):
    """The values a set of files uses cannot be listed or resolved exactly."""


class NonCanonicalValue(ValueProvenanceError):
    """A value has no canonical form that is the same on every run and platform."""


@dataclass(frozen=True)
class PinnedValues:
    """The constants a set of files imports, in canonical form, and their digest."""

    sha256: str
    values: Mapping[str, object]


def canonical_json(value: object) -> bytes:
    """ASCII JSON with sorted keys and no whitespace: the bytes the digest is taken over."""

    return json.dumps(
        value,
        ensure_ascii=True,
        sort_keys=True,
        separators=_JSON_SEPARATORS,
        allow_nan=False,
    ).encode("ascii")


def canonical_value(value: object, label: str) -> CanonicalValue:
    """Write one value in its run- and platform-independent form (see the module doc)."""

    kind = type(value)
    if kind in _SCALAR_TYPES:
        if kind is float and sys.float_repr_style != _SHORTEST_FLOAT_REPR:
            raise ValueProvenanceError(
                f"{label}: this interpreter does not print floats in their shortest "
                "round-trip form, so their text is not a portable identity"
            )
        return [kind.__name__, repr(value)]
    if isinstance(value, str) and kind is str:
        return [kind.__name__, value]
    if isinstance(value, _SEQUENCE_TYPES) and kind in _SEQUENCE_TYPES:
        return [
            kind.__name__,
            [
                canonical_value(item, f"{label}[{index}]")
                for index, item in enumerate(value)
            ],
        ]
    if isinstance(value, dict) and kind is dict:
        return [
            kind.__name__,
            [
                [
                    canonical_value(key, f"{label} key {key!r}"),
                    canonical_value(item, f"{label}[{key!r}]"),
                ]
                for key, item in value.items()
            ],
        ]
    if isinstance(value, _SET_TYPES) and kind in _SET_TYPES:
        items = [canonical_value(item, f"{label} item") for item in value]
        return [kind.__name__, sorted(items, key=canonical_json)]
    raise NonCanonicalValue(
        f"{label}: a {kind.__module__}.{kind.__qualname__} has no canonical form; "
        "use None, bool, int, float, str, or a tuple, list, dict, set or frozenset of them"
    )


def values_sha256(values: Mapping[str, object]) -> str:
    """SHA-256 over canonical JSON of the `module.NAME` -> canonical value mapping."""

    return hashlib.sha256(canonical_json(dict(values))).hexdigest()


def changed_values(
    recorded: Mapping[str, object], current: Mapping[str, object]
) -> tuple[str, ...]:
    """Names added, removed or given another value between two resolved mappings."""

    return tuple(
        sorted(
            name
            for name in recorded.keys() | current.keys()
            if name not in recorded
            or name not in current
            or recorded[name] != current[name]
        )
    )


def _read_source(path: Path) -> bytes:
    try:
        return path.read_bytes()
    except OSError as error:
        raise ValueProvenanceError(f"{path} is unreadable: {error}") from error


def _package_of(path: Path, source_root: Path) -> str | None:
    """The package a file belongs to, which relative imports are resolved against."""

    try:
        relative = path.resolve().relative_to(source_root.resolve())
    except ValueError:
        return None
    return ".".join(relative.parent.parts) or None


def _import_base(node: ast.ImportFrom, package: str | None, path: Path) -> str:
    try:
        return importlib.util.resolve_name(
            "." * node.level + (node.module or ""), package
        )
    except ImportError as error:
        raise ValueProvenanceError(f"{path}:{node.lineno}: {error}") from error


def _within_constants(module: str) -> bool:
    return module.startswith(_CONSTANTS_PREFIX)


def _dotted(node: ast.expr) -> str | None:
    """`a.b.c` for an attribute chain rooted at a name, otherwise None."""

    parts: list[str] = []
    while isinstance(node, ast.Attribute):
        parts.append(node.attr)
        node = node.value
    if not isinstance(node, ast.Name):
        return None
    parts.append(node.id)
    return ".".join(reversed(parts))


def _attribute_references(
    tree: ast.Module, aliases: Mapping[str, str], path: Path
) -> set[str]:
    """Names read through a bound constants module, as in `models.NAME`."""

    inner = {
        id(node.value) for node in ast.walk(tree) if isinstance(node, ast.Attribute)
    }
    found: set[str] = set()
    for node in ast.walk(tree):
        if id(node) in inner or not isinstance(node, (ast.Attribute, ast.Name)):
            continue
        dotted = _dotted(node)
        if dotted is None:
            continue
        root, _, rest = dotted.partition(".")
        target = aliases.get(root)
        if target is None:
            continue
        expanded = ".".join(part for part in (target, rest) if part)
        if expanded != CONSTANTS_PACKAGE and not _within_constants(expanded):
            continue
        module, _, remainder = expanded.removeprefix(_CONSTANTS_PREFIX).partition(".")
        name = remainder.partition(".")[0]
        if expanded == CONSTANTS_PACKAGE or not name:
            raise ValueProvenanceError(
                f"{path}:{node.lineno}: {expanded} is used as an object, so the "
                "values read from it cannot be listed; import the names instead"
            )
        found.add(f"{_CONSTANTS_PREFIX}{module}.{name}")
    return found


def _file_references(path: Path, source_root: Path) -> set[str]:
    try:
        tree = ast.parse(_read_source(path), filename=str(path))
    except SyntaxError as error:
        raise ValueProvenanceError(f"{path} does not parse: {error}") from error
    package = _package_of(path, source_root)
    names: set[str] = set()
    aliases: dict[str, str] = {}
    for node in ast.walk(tree):
        if isinstance(node, ast.ImportFrom):
            base = _import_base(node, package, path)
            for alias in node.names:
                imported = f"{base}.{alias.name}"
                if not (
                    base == CONSTANTS_PACKAGE
                    or imported == CONSTANTS_PACKAGE
                    or _within_constants(base)
                ):
                    continue
                if alias.name == "*":
                    raise ValueProvenanceError(
                        f"{path}:{node.lineno}: 'from {base} import *' hides which "
                        "values the file uses"
                    )
                if _within_constants(base):
                    names.add(imported)
                else:
                    aliases[alias.asname or alias.name] = imported
        elif isinstance(node, ast.Import):
            for alias in node.names:
                if alias.name != CONSTANTS_PACKAGE and not _within_constants(alias.name):
                    continue
                if alias.asname:
                    aliases[alias.asname] = alias.name
                else:
                    aliases[_TOP_PACKAGE] = _TOP_PACKAGE
    if aliases:
        names.update(_attribute_references(tree, aliases, path))
    return names


def used_constants(paths: Iterable[Path], *, source_root: Path) -> frozenset[str]:
    """Every `keyswitch.constants.<module>.<NAME>` the files import or read."""

    names: set[str] = set()
    for path in paths:
        names.update(_file_references(path, source_root))
    return frozenset(names)


class _IsolatedConstants:
    """Constants modules executed from their source files, apart from `sys.modules`.

    A constants module may import the standard library and its sibling constants
    modules, relatively. An absolute import of the application package would fetch the
    module this process already imported - perhaps from another tree - so it is refused.
    """

    def __init__(self, source_root: Path) -> None:
        self.directory = source_root.joinpath(*CONSTANTS_PACKAGE.split("."))
        self._modules: dict[str, ModuleType] = {}
        self._builtins = {**vars(builtins), "__import__": self._import}

    def module(self, name: str) -> ModuleType:
        loaded = self._modules.get(name)
        if loaded is not None:
            return loaded
        path = self.directory / f"{name}.py"
        if "." in name or not path.is_file():
            raise ValueProvenanceError(
                f"{_CONSTANTS_PREFIX}{name} has no source file in {self.directory}"
            )
        module = ModuleType(f"{_CONSTANTS_PREFIX}{name}")
        module.__file__ = str(path)
        module.__dict__["__builtins__"] = self._builtins
        self._modules[name] = module
        source = _read_source(path)
        try:
            exec(compile(source, str(path), "exec", dont_inherit=True), module.__dict__)
        except ValueProvenanceError:
            raise
        except Exception as error:
            raise ValueProvenanceError(f"{path} failed to execute: {error!r}") from error
        return module

    def _import(
        self,
        name: str,
        module_globals: Mapping[str, object] | None = None,
        module_locals: Mapping[str, object] | None = None,
        fromlist: Sequence[str] | None = (),
        level: int = 0,
    ) -> ModuleType:
        if not level:
            if name == _TOP_PACKAGE or name.startswith(_TOP_PACKAGE + "."):
                raise ValueProvenanceError(
                    f"a constants module imports {name} by its absolute name; "
                    "constants modules import their siblings relatively"
                )
            return builtins.__import__(
                name, module_globals, module_locals, fromlist, level
            )
        if level != 1:
            raise ValueProvenanceError(
                "a constants module may import only its sibling constants modules"
            )
        if name:
            return self.module(name)
        package = ModuleType(CONSTANTS_PACKAGE)
        for item in fromlist or ():
            setattr(package, item, self.module(item))
        return package


def _agree_with_imported(qualified: str, source: Path, canonical: CanonicalValue) -> None:
    """Refuse a pin that differs from the value this process imported from the same file."""

    module_name, _, name = qualified.rpartition(".")
    imported = sys.modules.get(module_name)
    location = getattr(imported, "__file__", None)
    if not isinstance(location, str) or Path(location).resolve() != source.resolve():
        return
    if (
        not hasattr(imported, name)
        or canonical_value(getattr(imported, name), qualified) != canonical
    ):
        raise ValueProvenanceError(
            f"{qualified} in this process differs from {source}: the file changed "
            "after it was imported, so the pinned value is not the value in use"
        )


def resolve_values(
    names: Iterable[str], *, source_root: Path
) -> dict[str, CanonicalValue]:
    """Canonical values of `keyswitch.constants.<module>.<NAME>` under a source root."""

    constants = _IsolatedConstants(source_root)
    values: dict[str, CanonicalValue] = {}
    for qualified in sorted(names):
        module_name, _, name = qualified.rpartition(".")
        if not _within_constants(module_name):
            raise ValueProvenanceError(f"{qualified} is not a constant of {CONSTANTS_PACKAGE}")
        module = constants.module(module_name.removeprefix(_CONSTANTS_PREFIX))
        if name not in module.__dict__:
            raise ValueProvenanceError(f"{qualified} is not defined in {module.__file__}")
        canonical = canonical_value(module.__dict__[name], qualified)
        _agree_with_imported(qualified, Path(str(module.__file__)), canonical)
        values[qualified] = canonical
    return values


def pin_values(paths: Iterable[Path], *, source_root: Path) -> PinnedValues:
    """The constants the files use, resolved under `source_root`, and their digest.

    `source_root` is the directory that holds the `keyswitch` package (`src/` of a
    checkout). Files outside it are read too; they only cannot import relatively.
    """

    values = resolve_values(
        used_constants(paths, source_root=source_root), source_root=source_root
    )
    return PinnedValues(values_sha256(values), MappingProxyType(dict(values)))
