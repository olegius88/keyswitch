#!/usr/bin/env python3
"""Measure what this machine computes, not what it calls itself.

The intent model used to be sealed against the interpreter's *name*: the full
`sys.version` string, the libc version, the platform triple. That is a proxy,
and a brittle one. On 2026-09-09 an `apt upgrade` rebuilt python3.14 without
changing the language version; `sys.version` gained a new build date, the sealed
candidate hash moved, and the release replay failed - while every weight, every
threshold and every calibration constant reproduced byte for byte. The name
changed and the results did not.

This probe replaces the name with the results. It exercises the primitives the
trainer and the runtime actually depend on, captures each answer as its exact
bit image, and reduces the stream to a digest. An interpreter that computes the
same answers produces the same digest whatever it calls itself; one that
computes different answers produces a different digest and can say which
primitive moved.

What it is NOT. The probe is evidence, not proof. The space of a double is 2**64
wide and these grids are finite: a libm that diverges only off the grid will
pass here and still change the weights. Only a full replay settles that, which
is why the probe never gets a vote on identity - it is recorded as provenance
and used to explain a divergence, never to certify its absence. The one cell
that IS exhaustive is `unicode`, which walks every code point, because the
runtime normalizes every token it sees (src/keyswitch/intent_model.py:164-168)
and a Unicode database bump is a real and silent behavioural change.

Cells, and why each one is here:

* `float_arithmetic` - the FTRL update written out as the trainer writes it
  (tools/train_intent_model.py:4275-4285, 4334-4349), including the cancelling
  difference `sqrt(new_n) - sqrt(old_n)` that carries the learning rate.
* `libm` - exactly the functions the trainer calls, at the frequency it calls
  them: sqrt (7 sites), nextafter (3), log1p (2), exp, log, floor, ceil.
* `builtin_sum` - summation order and any compensation, which decides the last
  bits of every dot product.
* `float_text` - repr/hex/round/json, the boundary where a float becomes the
  manifest and comes back.
* `unicode` - casefold and NFC/NFD over every code point.
* `hashing` - UTF-8 encoding and the FNV-1a mixing loop
  (src/keyswitch/intent_model.py:171-181), plus struct packing, which is how a
  feature becomes a bucket.
* `ordering` - sort stability, which fixes the order rows reach the model.
* `integers` - the exact integer operations quantisation relies on.
* `random_stream` - `random.Random(seed + epoch).shuffle`
  (tools/train_intent_model.py:4859), which fixes the epoch's row order.

No expected value appears anywhere in this file. The probe reports what this
machine does; comparing that against a recorded run is the caller's business.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import random
import struct
import sys
import unicodedata
from collections.abc import Iterator, Sequence
from pathlib import Path
from typing import Callable, Final, TypedDict

SCHEMA_VERSION: Final[int] = 1


class CellMeasurement(TypedDict):
    """One primitive's answer: the digest, plus digests per bucket.

    The buckets exist so a divergence can be pointed at rather than merely
    announced: `libm/exp` says more than a moved 64-hex string.
    """

    sha256: str
    entries: int
    buckets: dict[str, str]


class Measurement(TypedDict, total=False):
    """What this machine computes. `fork_check` appears only when asked for."""

    schema_version: int
    probe_sha256: str
    cells: dict[str, CellMeasurement]
    fork_check: str

# Operands chosen to sit on the places doubles misbehave: subnormals, the
# exponent boundary, values whose difference cancels, and ordinary magnitudes
# from real training. Not a random sample - a list of known-awkward inputs.
_PROBE_DOUBLES: Final[tuple[float, ...]] = (
    0.0,
    -0.0,
    5e-324,
    2.2250738585072014e-308,
    1e-300,
    1e-15,
    0.1,
    0.5,
    1.0 - 2**-53,
    1.0,
    1.0 + 2**-52,
    1.5,
    2.0,
    3.0,
    10.0,
    1e15,
    1e15 + 1.0,
    2**53 - 1.0,
    float(2**53),
    1e300,
    1.7976931348623157e308,
)

assert all(type(value) is float for value in _PROBE_DOUBLES), (
    "probe operands must be doubles; an int here would silently probe integer "
    "arithmetic instead"
)

_PROBE_STRINGS: Final[tuple[str, ...]] = (
    "",
    "a",
    "A",
    "ß",           # sharp s: casefolds to two characters
    "İ",           # dotted capital I: locale-sensitive elsewhere
    "ı",           # dotless i
    "Å",     # A + combining ring, composes under NFC
    "Å",           # precomposed angstrom sign source
    "Å",           # angstrom sign, normalises to the above
    "й",           # Cyrillic short i, composed
    "й",     # its decomposition
    "\U0001d11e",       # astral: treble clef
    "\U0001f600",       # astral: emoji
    "ﬃ",           # ffi ligature
    "ẞ",           # capital sharp s
    "ȩ́",    # two combining marks, canonical ordering matters
)


def _double(value: float) -> bytes:
    """The exact IEEE-754 image, which also separates -0.0 from 0.0.

    Mirrors `_same_double` in the trainer (tools/train_intent_model.py:4547).
    """

    return struct.pack("<d", value)


def _integer(value: int) -> bytes:
    return value.to_bytes(17, "little", signed=True)


def _text(value: str) -> bytes:
    return value.encode("utf-8", "surrogatepass")


# --------------------------------------------------------------------------
# Cells. Each yields (label, exact bit image) and must be pure and ordered.
# --------------------------------------------------------------------------


def _cell_float_arithmetic() -> Iterator[tuple[str, bytes]]:
    """The FTRL update, written as the trainer writes it."""

    alpha, beta, l1, l2 = 0.1, 1.0, 0.5, 0.01
    for old_n in _PROBE_DOUBLES:
        if not math.isfinite(old_n) or old_n < 0.0:
            continue
        for gradient in _PROBE_DOUBLES:
            if not math.isfinite(gradient):
                continue
            new_n = old_n + gradient * gradient
            if not math.isfinite(new_n):
                continue
            # The cancelling difference that carries the learning rate.
            sigma = (math.sqrt(new_n) - math.sqrt(old_n)) / alpha
            yield f"sigma:{old_n!r}:{gradient!r}", _double(sigma)
            weight = gradient * 0.5
            z_value = gradient - sigma * weight
            yield f"z:{old_n!r}:{gradient!r}", _double(z_value)
            if abs(z_value) <= l1:
                continue
            sign = -1.0 if z_value < 0.0 else 1.0
            denominator = (beta + math.sqrt(new_n)) / alpha + l2
            yield (
                f"weight:{old_n!r}:{gradient!r}",
                _double(-(z_value - sign * l1) / denominator),
            )


def _cell_libm() -> Iterator[tuple[str, bytes]]:
    """Exactly the libm surface the trainer touches."""

    unary: tuple[tuple[str, Callable[[float], float]], ...] = (
        ("sqrt", math.sqrt),
        ("exp", math.exp),
        ("log", math.log),
        ("log1p", math.log1p),
        ("floor", lambda value: float(math.floor(value))),
        ("ceil", lambda value: float(math.ceil(value))),
    )
    for name, function in unary:
        for value in _PROBE_DOUBLES:
            try:
                result = function(value)
            except (ValueError, OverflowError) as error:
                yield f"{name}:{value!r}", _text(type(error).__name__)
                continue
            yield f"{name}:{value!r}", _double(result)
    for left in _PROBE_DOUBLES:
        for right in _PROBE_DOUBLES:
            yield f"nextafter:{left!r}:{right!r}", _double(math.nextafter(left, right))


def _cell_builtin_sum() -> Iterator[tuple[str, bytes]]:
    """Summation order, which decides the last bits of every dot product."""

    sequences: tuple[tuple[str, tuple[float, ...]], ...] = (
        ("cancel", (1e16, 1.0, -1e16)),
        ("cancel_reversed", (-1e16, 1.0, 1e16)),
        ("many_small", (1.0,) + (1e-16,) * 64),
        ("many_small_reversed", (1e-16,) * 64 + (1.0,)),
        ("alternating", tuple(
            (1.0 if index % 2 else -1.0) * (1.0 + index * 1e-13)
            for index in range(128)
        )),
        ("subnormal", (5e-324,) * 32),
        ("mixed", _PROBE_DOUBLES),
    )
    def guarded(
        compute: Callable[[tuple[float, ...]], float], values: tuple[float, ...]
    ) -> bytes:
        """Record what the machine does, including refusing.

        `math.fsum` raises on an intermediate overflow that plain `sum` absorbs
        into an infinity. Which of the two a build does is exactly the kind of
        behaviour worth digesting, so the exception is recorded rather than
        allowed to abort the measurement.
        """

        try:
            return _double(compute(values))
        except (OverflowError, ValueError) as error:
            return _text(type(error).__name__)

    def straight(values: tuple[float, ...]) -> float:
        total = 0.0
        for value in values:
            total += value
        return total

    summations: tuple[tuple[str, Callable[[tuple[float, ...]], float]], ...] = (
        ("sum", sum),
        ("sum_start", lambda values: sum(values, 0.0)),
        ("fsum", math.fsum),
        ("loop", straight),
    )
    for name, values in sequences:
        for label, summation in summations:
            yield f"{label}:{name}", guarded(summation, values)


def _cell_float_text() -> Iterator[tuple[str, bytes]]:
    """The boundary where a float becomes the manifest and comes back."""

    for value in _PROBE_DOUBLES:
        yield f"repr:{value!r}", _text(repr(value))
        yield f"hex:{value!r}", _text(value.hex())
        yield f"json:{value!r}", _text(json.dumps(value))
        yield f"roundtrip:{value!r}", _double(float(repr(value)))
        yield f"from_hex:{value!r}", _double(float.fromhex(value.hex()))
        for digits in (0, 1, 6, 15):
            yield f"round:{value!r}:{digits}", _double(round(value, digits))


def _cell_unicode() -> Iterator[tuple[str, bytes]]:
    """Every code point. The runtime normalises every token it sees."""

    for point in range(0x110000):
        character = chr(point)
        folded = character.casefold()
        composed = unicodedata.normalize("NFC", character)
        decomposed = unicodedata.normalize("NFD", character)
        if folded == character and composed == character and decomposed == character:
            # Identity under all three: the overwhelming majority. Recording it
            # per code point would bloat the stream without adding evidence,
            # but its membership in this class is itself evidence, so the plane
            # digest still moves if a future database changes it.
            yield f"U+{point:04X}", b"\x00"
            continue
        yield f"U+{point:04X}:fold", _text(folded)
        yield f"U+{point:04X}:nfc", _text(composed)
        yield f"U+{point:04X}:nfd", _text(decomposed)
        yield f"U+{point:04X}:cat", _text(unicodedata.category(character))
        yield f"U+{point:04X}:ccc", _integer(unicodedata.combining(character))
    for value in _PROBE_STRINGS:
        yield f"str:fold:{value!r}", _text(value.casefold())
        for form in ("NFC", "NFD", "NFKC", "NFKD"):
            yield f"str:{form}:{value!r}", _text(unicodedata.normalize(form, value))
        # Exactly what normalize_token does (intent_model.py:164-168).
        yield (
            f"str:token:{value!r}",
            _text(unicodedata.normalize(
                "NFC", unicodedata.normalize("NFC", value).casefold()
            )),
        )


def _cell_hashing() -> Iterator[tuple[str, bytes]]:
    """How a feature becomes a bucket."""

    prime = 0x100000001B3
    mask = (1 << 64) - 1
    for value in _PROBE_STRINGS:
        encoded = _text(value)
        yield f"utf8:{value!r}", encoded
        result = 0xCBF29CE484222325
        for byte in encoded:
            result ^= byte
            result = (result * prime) & mask
        yield f"fnv1a64:{value!r}", _integer(result)
    for number in _PROBE_DOUBLES:
        yield f"pack_le:{number!r}", struct.pack("<d", number)
        yield f"pack_be:{number!r}", struct.pack(">d", number)
        # A double beyond the single-precision range is not representable;
        # record the refusal rather than clamping it into a different answer.
        try:
            packed = struct.pack("<f", number)
        except OverflowError:
            packed = _text("OverflowError")
        yield f"pack_f32:{number!r}", packed
    yield "byteorder", _text(sys.byteorder)
    yield "sha256:empty", hashlib.sha256(b"").digest()
    yield "sha256:probe", hashlib.sha256(b"keyswitch-environment-probe").digest()


def _cell_ordering() -> Iterator[tuple[str, bytes]]:
    """Sort stability, which fixes the order rows reach the model."""

    pairs = tuple((index % 5, index) for index in range(64))
    yield "stable", _text(repr(sorted(pairs, key=lambda item: item[0])))
    yield "reverse", _text(repr(sorted(pairs, key=lambda item: item[0], reverse=True)))
    yield "strings", _text(repr(sorted(_PROBE_STRINGS)))
    yield "strings_fold", _text(repr(sorted(_PROBE_STRINGS, key=str.casefold)))
    finite = tuple(value for value in _PROBE_DOUBLES if math.isfinite(value))
    yield "doubles", _text(repr(sorted(finite)))
    yield "min_max", _double(min(finite)) + _double(max(finite))
    yield "dict_order", _text(repr(list({value: None for value in _PROBE_STRINGS})))


def _cell_integers() -> Iterator[tuple[str, bytes]]:
    """The integer operations quantisation relies on."""

    values = (-(2**70), -(2**63), -7, -1, 0, 1, 7, 2**31, 2**63 - 1, 2**70)
    for left in values:
        yield f"str:{left}", _text(str(left))
        yield f"bits:{left}", _integer(left.bit_length())
        for right in values:
            if right == 0:
                continue
            yield f"floordiv:{left}:{right}", _integer(left // right)
            yield f"mod:{left}:{right}", _integer(left % right)
            quotient, remainder = divmod(left, right)
            yield f"divmod:{left}:{right}", _integer(quotient) + _integer(remainder)
        yield f"shift:{left}", _integer(left >> 3) + _integer(left << 3)
    yield "pow_mod", _integer(pow(3, 2**20, 2**61 - 1))
    for value in (0.0, 0.5, 1.5, 2.5, -0.5, -1.5, 1e15 + 0.5):
        yield f"round_int:{value!r}", _integer(round(value))
        yield f"trunc:{value!r}", _integer(math.trunc(value))


def _cell_random_stream() -> Iterator[tuple[str, bytes]]:
    """The Mersenne Twister stream that fixes each epoch's row order."""

    for seed in (0, 1, 20260909, 2**31 - 1):
        for epoch in (0, 1, 7):
            generator = random.Random(seed + epoch)
            indices = list(range(64))
            generator.shuffle(indices)
            yield f"shuffle:{seed}:{epoch}", _text(repr(indices))
            generator = random.Random(seed + epoch)
            yield f"random:{seed}:{epoch}", b"".join(
                _double(generator.random()) for _ in range(8)
            )
            generator = random.Random(seed + epoch)
            yield f"getrandbits:{seed}:{epoch}", _integer(generator.getrandbits(64))
            generator = random.Random(seed + epoch)
            yield f"sample:{seed}:{epoch}", _text(
                repr(generator.sample(range(1000), 16))
            )


CELLS: Final[tuple[tuple[str, Callable[[], Iterator[tuple[str, bytes]]]], ...]] = (
    ("float_arithmetic", _cell_float_arithmetic),
    ("libm", _cell_libm),
    ("builtin_sum", _cell_builtin_sum),
    ("float_text", _cell_float_text),
    ("unicode", _cell_unicode),
    ("hashing", _cell_hashing),
    ("ordering", _cell_ordering),
    ("integers", _cell_integers),
    ("random_stream", _cell_random_stream),
)
CELL_NAMES: Final[frozenset[str]] = frozenset(name for name, _ in CELLS)


def _bucket_of(cell: str, label: str) -> str:
    """Group a cell's entries so a divergence can be localised.

    `unicode` is bucketed by plane because it has more than a million entries;
    every other cell is bucketed by the operation named before the first colon.
    """

    if cell == "unicode" and label.startswith("U+"):
        point = int(label[2:].split(":", 1)[0], 16)
        return f"plane-{point >> 16:02X}"
    return label.split(":", 1)[0]


def measure_cell(cell: str) -> CellMeasurement:
    """Digest one cell, plus a digest per bucket for localisation."""

    generator = dict(CELLS)[cell]
    overall = hashlib.sha256()
    buckets: dict[str, "hashlib._Hash"] = {}
    entries = 0
    for label, image in generator():
        record = label.encode("utf-8") + b"\x00" + image + b"\n"
        overall.update(record)
        buckets.setdefault(_bucket_of(cell, label), hashlib.sha256()).update(record)
        entries += 1
    return {
        "sha256": overall.hexdigest(),
        "entries": entries,
        "buckets": {
            name: digest.hexdigest()
            for name, digest in sorted(buckets.items())
        },
    }


def measure(cells: Sequence[str] | None = None) -> Measurement:
    names = [name for name, _ in CELLS] if cells is None else list(cells)
    unknown = sorted(set(names) - CELL_NAMES)
    if unknown:
        raise ValueError(f"unknown probe cells: {', '.join(unknown)}")
    measured = {name: measure_cell(name) for name in names}
    combined = hashlib.sha256()
    for name in sorted(measured):
        combined.update(name.encode("utf-8") + b"\x00")
        combined.update(measured[name]["sha256"].encode("ascii") + b"\n")
    return {
        "schema_version": SCHEMA_VERSION,
        "probe_sha256": combined.hexdigest(),
        "cells": measured,
    }


def _forked_measurement(cells: Sequence[str]) -> Measurement | None:
    """Repeat the measurement in a forked child, the way workers are made.

    The trainer fingerprints rows in forked worker processes. If a primitive
    behaved differently after fork, every parallel phase would be suspect, so
    the disagreement matters more than either value on its own. Returns None
    where fork is unavailable (Windows), which is not a failure: the platform
    simply cannot be checked this way.
    """

    import multiprocessing

    if "fork" not in multiprocessing.get_all_start_methods():
        return None
    context = multiprocessing.get_context("fork")
    parent_end, child_end = context.Pipe(duplex=False)

    def child() -> None:
        child_end.send(measure(cells))
        child_end.close()

    process = context.Process(target=child)
    process.start()
    child_end.close()
    try:
        payload: Measurement = parent_end.recv()
    finally:
        parent_end.close()
        process.join()
    return payload


def _ulp_distance(left: float, right: float) -> int | None:
    """How many representable doubles separate two values."""

    if not (math.isfinite(left) and math.isfinite(right)):
        return None

    def ordered(value: float) -> int:
        # Map the sign-magnitude bit pattern onto a monotone integer line, so
        # that subtracting two of them counts the doubles between them.
        bits = int(struct.unpack("<q", struct.pack("<d", value))[0])
        return bits if bits >= 0 else -(bits & 0x7FFFFFFFFFFFFFFF) - 1

    return abs(ordered(left) - ordered(right))


def explain(cell: str, recorded: Measurement | None) -> int:
    """Print one cell's entries, and where it parts company with a record."""

    if cell not in CELL_NAMES:
        raise ValueError(f"unknown probe cell: {cell}")
    current = measure_cell(cell)
    print(f"cell {cell}: {current['sha256']} ({current['entries']} entries)")
    if recorded is None:
        for name, digest in sorted(current["buckets"].items()):
            print(f"  {name}: {digest}")
        return 0
    cells = recorded.get("cells", {})
    if cell not in cells:
        print(f"  the record carries no {cell} cell; nothing to compare")
        return 1
    previous = cells[cell]
    if previous["sha256"] == current["sha256"]:
        print("  identical to the record; this cell did not move")
        return 0
    print(f"  record:  {previous['sha256']}")
    print("  buckets that differ:")
    recorded_buckets = previous["buckets"]
    current_buckets = current["buckets"]
    moved = [
        name
        for name in sorted(set(recorded_buckets) | set(current_buckets))
        if recorded_buckets.get(name) != current_buckets.get(name)
    ]
    for name in moved:
        print(
            f"    {name}: record={recorded_buckets.get(name, '(absent)')} "
            f"host={current_buckets.get(name, '(absent)')}"
        )
    if not moved:
        print("    none - the entry count or ordering changed instead")
    print(
        "\n  The record carries digests, not values, so the differing entries "
        "themselves\n  are not in it. Run this same command on both machines "
        "and diff the output;\n  use --ulp <hex> <hex> on any two packed "
        "doubles to size the gap."
    )
    return 1


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--cells", nargs="+", metavar="CELL",
        help="measure only these cells (default: all)",
    )
    parser.add_argument(
        "--explain", metavar="CELL",
        help="print one cell's bucket digests instead of the whole measurement",
    )
    parser.add_argument(
        "--against", type=Path, metavar="FILE",
        help="a recorded measurement to compare --explain against",
    )
    parser.add_argument(
        "--ulp", nargs=2, metavar=("A", "B"),
        help="report how many doubles separate two little-endian hex images",
    )
    parser.add_argument(
        "--check-fork", action="store_true",
        help="also measure in a forked child and fail if the two disagree",
    )
    arguments = parser.parse_args(argv)

    if arguments.ulp:
        left, right = (
            struct.unpack("<d", bytes.fromhex(item))[0] for item in arguments.ulp
        )
        distance = _ulp_distance(left, right)
        print(f"{left!r} vs {right!r}: {'non-finite' if distance is None else distance} ulp")
        return 0

    cells = arguments.cells or [name for name, _ in CELLS]
    if arguments.explain:
        recorded: Measurement | None = None
        if arguments.against is not None:
            loaded = json.loads(arguments.against.read_bytes())
            # The sidecar nests the measurement; a bare probe run does not.
            if "cells" not in loaded and "environment_probe" in loaded:
                loaded = loaded["environment_probe"]
            recorded = loaded
        return explain(arguments.explain, recorded)

    payload = measure(cells)
    if arguments.check_fork:
        forked = _forked_measurement(cells)
        if forked is None:
            payload["fork_check"] = "unavailable"
        elif forked["probe_sha256"] != payload["probe_sha256"]:
            moved = sorted(
                name
                for name, cell in payload["cells"].items()
                if forked["cells"][name]["sha256"] != cell["sha256"]
            )
            raise SystemExit(
                "the forked child computed different answers than its parent: "
                + ", ".join(moved)
            )
        else:
            payload["fork_check"] = "agrees"
    print(json.dumps(payload, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
