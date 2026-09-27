#!/usr/bin/env python3
"""Drop truncated forms from the packaged Russian lexicon supplement, reproducibly.

The supplement (src/keyswitch/resources/lexicon-supplement-ru_RU.json) lists forms of a subtitle
frequency list that the onboard lexicon lacks. Subtitles cut words at line ends, so the list also
holds word beginnings such as `клу`, `которы`, `други`. The engine loads every form as a known word
with the synthetic fallback frequency, so a word like `которых` typed in the wrong layout reads two
ways at once (`которых`, `которы` + `[`) and the boundary model has to abstain on it.

Rule: a form is dropped when the reference Hunspell dictionary (model/intent_v1/sources/hunspell,
pinned by the intent configuration) accepts neither the form nor its capitalised spelling (a proper
name is spelled capitalised there), and the form is a proper prefix of a word of the onboard lexicon
(ru_RU.lm unigrams plus the built-in fallback words). Every other form is kept, slang and names the
dictionary does not know included.

The receipt beside the supplement records the digests of every input and of the output, the rule,
the counts and the dropped forms, so `--verify` can check the packaged supplement without its
predecessor: the kept and dropped forms together are exactly the recorded input words, every dropped
form meets the rule and no kept form does.

Usage: filter_lexicon_supplement.py --input OLD.json [--output NEW.json] [--receipt RECEIPT.json]
       [--removed-list LIST.tsv]
       filter_lexicon_supplement.py --verify
"""
from __future__ import annotations

import argparse
import bisect
import hashlib
import json
import os
import sys
from collections.abc import Iterable, Mapping, Sequence
from pathlib import Path
from typing import Protocol, cast

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))
from keyswitch.constants.file_formats import REPORT_JSON_INDENT  # noqa: E402
from keyswitch.language_model import LOCALE_FALLBACKS, LanguageModel  # noqa: E402

LOCALE = "ru_RU"
SUPPLEMENT = ROOT / "src/keyswitch/resources/lexicon-supplement-ru_RU.json"
RECEIPT = ROOT / "src/keyswitch/resources/lexicon-supplement-ru_RU.receipt.json"
CONFIG = ROOT / "model/intent_v1/config.json"
HUNSPELL_ROOT = ROOT / "model/intent_v1/sources/hunspell"
NAME = "opensubtitles-2018-ru-full-min10-outside-onboard-v4"
RULE = ("drop a form when the reference Hunspell ru_RU dictionary accepts neither the form nor its "
        "capitalised spelling and the form is a proper prefix of a word of the onboard ru_RU lexicon "
        "(ru_RU.lm unigrams plus the built-in fallback words); keep every other form")
SCOPE_ADDITION = (" Filtered by tools/filter_lexicon_supplement.py: forms the reference Hunspell dictionary "
                  "does not accept (neither as written nor capitalised) that are proper prefixes of an onboard "
                  "word are dropped, as subtitle line breaks cut words there; the receipt beside this file "
                  "lists them.")


class Speller(Protocol):
    def check(self, word: str) -> bool: ...


def canonical(value: object) -> bytes:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8") + b"\n"


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def words_digest(words: Iterable[str]) -> str:
    return sha256("".join(word + "\n" for word in sorted(words)).encode("utf-8"))


def accepted(speller: Speller, form: str) -> bool:
    return speller.check(form) or speller.check(form[:1].upper() + form[1:])


def prefix_owner(onboard: Sequence[str], form: str) -> str | None:
    """The first onboard word (sorted) that the form begins without being it, if any."""
    index = bisect.bisect_right(onboard, form)
    if index < len(onboard) and onboard[index].startswith(form) and onboard[index] != form:
        return onboard[index]
    return None


def split_forms(forms: Iterable[str], onboard: Sequence[str], speller: Speller) -> tuple[list[str], dict[str, str]]:
    """Kept forms, and each dropped form with the onboard word it begins."""
    kept: list[str] = []
    dropped: dict[str, str] = {}
    for form in forms:
        owner = prefix_owner(onboard, form)
        if owner is not None and not accepted(speller, form):
            dropped[form] = owner
        else:
            kept.append(form)
    return kept, dropped


def reference_inputs() -> tuple[list[str], Speller, dict[str, object]]:
    """The onboard vocabulary and the reference dictionary, each checked against the intent pins."""
    config = cast(dict[str, object], json.loads(CONFIG.read_bytes()))
    languages = cast(dict[str, dict[str, object]], cast(dict[str, object], config["sources"])["languages"])
    hunspell = cast(dict[str, dict[str, object]], cast(dict[str, object], config["external_evaluation"])["hunspell"])
    lexicon = ROOT / str(languages[LOCALE]["path"])
    dictionary, affix = HUNSPELL_ROOT / f"{LOCALE}.dic", HUNSPELL_ROOT / f"{LOCALE}.aff"
    pins: dict[str, dict[str, str]] = {"onboard_lexicon": {"path": lexicon.relative_to(ROOT).as_posix(), "sha256": sha256(lexicon.read_bytes())},
            "hunspell_dictionary": {"path": dictionary.relative_to(ROOT).as_posix(), "sha256": sha256(dictionary.read_bytes())},
            "hunspell_affix": {"path": affix.relative_to(ROOT).as_posix(), "sha256": sha256(affix.read_bytes())}}
    if (pins["onboard_lexicon"]["sha256"] != languages[LOCALE]["sha256"]
            or pins["hunspell_dictionary"]["sha256"] != hunspell[LOCALE]["dictionary_sha256"]
            or pins["hunspell_affix"]["sha256"] != hunspell[LOCALE]["affix_sha256"]):
        raise ValueError("reference lexicon or dictionary differs from the intent configuration")
    os.environ["KEYSWITCH_HUNSPELL_PATH"] = str(HUNSPELL_ROOT)
    from keyswitch.spellcheck import HunspellDictionary
    speller = HunspellDictionary(LOCALE)
    if not speller.available or Path(speller.source).resolve() != dictionary.resolve():
        raise ValueError("the reference Hunspell dictionary is not the one loaded")
    frequencies, _ = LanguageModel._read_arpa(lexicon)
    onboard = set(frequencies) | {LanguageModel.normalize(word) for word in LOCALE_FALLBACKS[LOCALE]}
    return sorted(onboard), speller, dict(pins)


def build(payload: Mapping[str, object], kept: list[str], dropped: Mapping[str, str]) -> dict[str, object]:
    selection = dict(cast(dict[str, object], payload["selection"]))
    selection["dropped_truncations"] = len(dropped)
    selection["selected"] = len(kept)
    return {**payload, "name": NAME, "scope": str(payload["scope"]) + SCOPE_ADDITION, "selection": selection,
            "words": kept}


def receipt(input_raw: bytes, input_words: list[str], output_raw: bytes, kept: list[str], dropped: dict[str, str],
            pins: dict[str, object]) -> dict[str, object]:
    return {"schema_version": 1, "tool": "tools/filter_lexicon_supplement.py", "rule": RULE,
            "input": {"sha256": sha256(input_raw), "words": len(input_words), "words_sha256": words_digest(input_words)},
            "output": {"path": SUPPLEMENT.relative_to(ROOT).as_posix(), "sha256": sha256(output_raw), "words": len(kept),
                       "words_sha256": words_digest(kept)},
            "references": pins, "dropped": {"count": len(dropped), "forms": sorted(dropped)}}


def verify() -> dict[str, object]:
    """The packaged supplement is the recorded output, and the recorded split follows the rule."""
    record = cast(dict[str, object], json.loads(RECEIPT.read_bytes()))
    raw = SUPPLEMENT.read_bytes()
    output = cast(dict[str, object], record["output"])
    if sha256(raw) != output["sha256"]:
        raise ValueError("the packaged supplement is not the recorded output")
    onboard, speller, pins = reference_inputs()
    if record["references"] != pins:
        raise ValueError("the reference lexicon or dictionary changed since the filter ran")
    kept = cast(list[str], json.loads(raw)["words"])
    forms = cast(list[str], cast(dict[str, object], record["dropped"])["forms"])
    source = cast(dict[str, object], record["input"])
    if words_digest([*kept, *forms]) != source["words_sha256"] or len(kept) + len(forms) != source["words"]:
        raise ValueError("kept and dropped forms are not the recorded input")
    retained, removed = split_forms(kept, onboard, speller)
    if removed:
        raise ValueError("a kept form meets the drop rule: " + next(iter(removed)))
    _, confirmed = split_forms(forms, onboard, speller)
    if sorted(confirmed) != sorted(forms):
        raise ValueError("a dropped form does not meet the rule")
    return {"verified": True, "kept": len(retained), "dropped": len(forms)}


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--input", type=Path, help="the supplement to filter")
    parser.add_argument("--output", type=Path, default=SUPPLEMENT)
    parser.add_argument("--receipt", type=Path, default=RECEIPT)
    parser.add_argument("--removed-list", type=Path, help="also write the dropped forms with the word each begins")
    parser.add_argument("--verify", action="store_true")
    args = parser.parse_args(argv)
    if args.verify:
        print(json.dumps(verify(), ensure_ascii=False))
        return 0
    if args.input is None:
        parser.error("--input is required unless --verify is given")
    input_raw = args.input.read_bytes()
    payload = cast(dict[str, object], json.loads(input_raw))
    words = cast(list[str], payload["words"])
    if words != sorted(set(words)) or payload.get("locale") != LOCALE:
        raise ValueError("the input is not a sorted Russian supplement")
    if payload.get("name") == NAME:
        raise ValueError("the input is already filtered")
    onboard, speller, pins = reference_inputs()
    kept, dropped = split_forms(words, onboard, speller)
    output_raw = canonical(build(payload, kept, dropped))
    args.output.write_bytes(output_raw)
    args.receipt.write_bytes(json.dumps(receipt(input_raw, words, output_raw, kept, dropped, pins),
                                        ensure_ascii=False, sort_keys=True, indent=REPORT_JSON_INDENT).encode("utf-8") + b"\n")
    if args.removed_list is not None:
        args.removed_list.write_text("".join(f"{form}\t{owner}\n" for form, owner in sorted(dropped.items())),
                                     encoding="utf-8")
    print(json.dumps({"input": len(words), "kept": len(kept), "dropped": len(dropped),
                      "output_sha256": sha256(output_raw)}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
