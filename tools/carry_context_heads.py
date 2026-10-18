"""The frozen base for a fit that refits some heads of a shipped action pair: the pair with those heads' weights removed.

Every head fitted from zero moves with the contexts its curriculum draws and the corpus it is fitted on: the code of
0.57.0 fitted again on corpus v66, eleven TRAIN rows short of v65, lost four hyphenated words of calibration, and a fit
of the first-term head's change lost two of them and a word of the owner's typing, all in heads that change did not
touch (18.10.2026). A fit onto the shipped pair keeps every weight of every other head byte for byte
(carried_prefixes in tools/train_context_action_model.py) and fits only the heads named here, so a candidate differs
from the shipped pair in those heads' classes alone. The output is an action model of its own version; the recipe names
it under `frozen_base` with its SHA-256 and the pair it was made from in the scope.

    python3 tools/carry_context_heads.py --pair src/keyswitch/resources/models/context_policy_v1.json --refit first \\
        --output model/context_v3/frozen-base-754513f80498.json
"""
from __future__ import annotations

import argparse
import hashlib
import json
import sys
from collections.abc import Iterable, Mapping
from pathlib import Path
from typing import cast

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from keyswitch.constants.file_formats import VERSION_HASH_CHARACTERS  # noqa: E402
from keyswitch.context_model import ContextModel  # noqa: E402
from train_context_action_model import HEAD_PREFIXES, canonical, checksum  # noqa: E402


def carried(pair: Mapping[str, object], refit: Iterable[str]) -> dict[str, object]:
    """The payload of `pair` without the weights of the heads named in `refit`, with its own checksum and version."""

    prefixes = tuple(HEAD_PREFIXES[name] for name in refit)
    if not prefixes:
        raise ValueError("name a head to fit again")
    weights = cast(dict[str, list[float]], pair["weights"])
    if any(not any(name.startswith(prefix) for name in weights) for prefix in prefixes):
        raise ValueError("the pair has no weights for a head named to fit again")
    kept = {name: values for name, values in weights.items() if not name.startswith(prefixes)}
    digest = hashlib.sha256(json.dumps(kept, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode()).hexdigest()
    return {**pair, "weights": kept, "weights_sha256": digest, "version": "context-v3-" + digest[:VERSION_HASH_CHARACTERS]}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--pair", type=Path, required=True, help="the shipped action model")
    parser.add_argument("--refit", action="append", required=True, choices=sorted(HEAD_PREFIXES),
                        help="a head the fit fits again (repeatable)")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    pair = cast(dict[str, object], json.loads(args.pair.read_text(encoding="utf-8")))
    source = ContextModel.load(args.pair)
    args.output.write_bytes(canonical(carried(pair, args.refit)))
    base = ContextModel.load(args.output)
    print(json.dumps({"artifact": str(args.output), "sha256": checksum(args.output), "model_version": base.version,
                      "pair": source.version, "refit": args.refit, "weights": len(base.weights)}, indent=1))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
