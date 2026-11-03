#!/usr/bin/env python3
"""Measure the typo correction (keyswitch.typo_correction) on development text and on the owner's typing.

Three measurements, all deterministic (a draw is a hash of the place it is made for):

- `synthetic`: one word in TYPO_EVALUATION_SAMPLE_MODULUS of the development texts (letters only, at least
  TYPO_MIN_LETTERS of them, lowercase or capitalised at the start of a sentence) gets one slip, the four operations in
  equal shares - no source we could open gives their shares for Russian typing. Named by the operation that undoes
  it: `transposition` swaps two neighbouring letters, `deletion` undoes a letter typed in excess (a neighbour of a
  letter beside it, or one of them doubled), `insertion` undoes a letter left out, `substitution` undoes a letter
  typed with a neighbouring key. A typo is corrected right (to the word it came from), corrected wrong, or left, with
  the reason. The texts split in two halves by message: the decision parameters are tuned on the first
  (`--tune`: the most right corrections with at most TYPO_EVALUATION_MAX_WRONG_SHARE of them wrong) and reported on
  the second.
- `clean`: every word of the development texts is judged as typed; each correction is a false one unless the text
  held a typo (they do: the words are listed for review).
- `field`: every word of the owner's typing batteries (.t/field/manifest.json), in the reading the owner kept, is
  judged; a correction of a word the owner later erased and typed again as the correction is a real typo fixed.

Texts: the development splits only (`*-dev`), never a `*-test` file. Lists of words go to the private output folder.

Usage: evaluate_typo_correction.py synthetic|clean|field|all --messages DIR --manifest PATH --out DIR [--tune]
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import itertools
import json
import math
import re
import sys
import tempfile
from collections import Counter
from collections.abc import Callable, Iterator, Mapping, Sequence
from dataclasses import dataclass, field
from datetime import datetime
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))
sys.path.insert(0, str(ROOT / "tools"))
from keyswitch.constants.corpus import DETERMINISTIC_DRAW_HEX_DIGITS  # noqa: E402
from keyswitch.constants.file_formats import HEXADECIMAL_BASE, REPORT_JSON_INDENT  # noqa: E402
from keyswitch.constants.typo import (  # noqa: E402
    TYPO_EVALUATION_VOCABULARY_MIN_COUNTS,
    TYPO_VOCABULARY_MAX_RECALL_LOSS,
    TYPO_CANDIDATE_MIN_COUNT,
    TYPO_ACCEPTED_FORUM_WORDS_PER_FALSE_CORRECTION,
    TYPO_ACCEPTED_PROSE_WORDS_PER_FALSE_CORRECTION,
    TYPO_EVALUATION_DELETION_LOG_PROBABILITIES,
    TYPO_EVALUATION_INSERTION_LOG_PROBABILITIES,
    TYPO_EVALUATION_MAX_WRONG_SHARE,
    TYPO_EVALUATION_MIN_COUNTS,
    TYPO_EVALUATION_RATIOS,
    TYPO_EVALUATION_RETYPE_WINDOW_SECONDS,
    TYPO_EVALUATION_SAMPLE_MODULUS,
    TYPO_EVALUATION_SUBSTITUTION_LOG_PROBABILITIES,
    TYPO_EVALUATION_WILSON_Z,
    TYPO_MIN_LETTERS,
    TYPO_TRANSPOSED_LETTERS,
    TYPO_TRANSPOSITION_LOG_PROBABILITY,
)
from keyswitch.context_model import _term_frequency  # noqa: E402
from keyswitch.context_policy import shared_identifiers  # noqa: E402
from keyswitch.engine import PUNCTUATION, WORD_JOINERS  # noqa: E402
from keyswitch.layouts import LayoutPair  # noqa: E402
from keyswitch.personal_words import PersonalWords  # noqa: E402
from keyswitch.typo_correction import (  # noqa: E402
    OPERATIONS,
    Candidate,
    Operation,
    TypoCorrector,
    TypoJudgement,
    TypoLexicon,
    TypoPolicy,
    known_elsewhere,
)

WORD = re.compile(r"[A-Za-zА-Яа-яЁё]+")
SENTENCE_END = ".!?…"
EDGE_SIGNS = ".,!?:;\"'()«»…"
# The signs that end a word in the engine without joining it to the next one.
WORD_ENDS = frozenset(PUNCTUATION - WORD_JOINERS)
PROSE_FILES = ("tatoeba-dev", "taiga-dev")
FORUM_FILES = ("so-dev",)
CODE_FILE = "code-dev.txt.gz"
PAIR = LayoutPair()
LAYOUT = {"en_US": "us", "ru_RU": "ru"}
GROUP_LOCALES = ("en_US", "ru_RU")
# The synthetic typos split by message: the parameters are tuned on the first half and reported on the second.
HALVES = ("tune", "report")


def draw(*parts: object) -> int:
    digest = hashlib.sha256(":".join(map(str, parts)).encode("utf-8")).hexdigest()
    return int(digest[:DETERMINISTIC_DRAW_HEX_DIGITS], HEXADECIMAL_BASE)


def locale_of(token: str) -> str | None:
    if all("a" <= character.casefold() <= "z" for character in token):
        return "en_US"
    if all(character.casefold() in "абвгдеёжзийклмнопрстуфхцчшщъыьэюя" for character in token):
        return "ru_RU"
    return None


def other_reading(text: str, locale: str) -> dict[str, str]:
    other = GROUP_LOCALES[1 - GROUP_LOCALES.index(locale)]
    return {other: PAIR.translate(text, LAYOUT[locale], LAYOUT[other])}


def words_of(text: str) -> Iterator[tuple[int, str, bool]]:
    """The plain words of a text as the engine ends them: their index, the word, and whether a capital may open it
    there (the start of the text or of a sentence).

    The engine ends a word at a space, Enter, Tab or a sign of engine.PUNCTUATION; any other sign typed beside it
    (`=`, `_`, `/`, a digit, a joining hyphen or apostrophe) makes it part of a longer token, which is not corrected.
    """

    for index, match in enumerate(WORD.finditer(text)):
        before = text[match.start() - 1] if match.start() else " "
        after = text[match.end()] if match.end() < len(text) else " "
        if not all(edge.isspace() or edge in WORD_ENDS for edge in (before, after)):
            continue
        preceding = text[:match.start()].rstrip()
        yield index, match.group(), not preceding or preceding[-1] in SENTENCE_END


def messages(directory: Path, name: str) -> Iterator[tuple[str, str]]:
    with gzip.open(directory / f"{name}.jsonl.gz", "rt", encoding="utf-8") as handle:
        for line in handle:
            item = json.loads(line)
            yield str(item["qid"]), str(item["text"])


class Knowledge:
    """What the engine knows from elsewhere (typo_correction.known_elsewhere), with the frozen reference lexicons."""

    def __init__(self) -> None:
        from reference_lexicon import reference_models

        models = reference_models(False)
        self.lexicons = [models[group].frequencies for group in sorted(models)]
        self.terms = _term_frequency()
        self.identifiers = shared_identifiers()

    def __call__(self, text: str) -> bool:
        return known_elsewhere(text, lexicons=self.lexicons, terms=self.terms, identifiers=self.identifiers,
                               excluded=())


def make_typo(word: str, operation: Operation, seed: int, neighbours: Mapping[str, str]) -> str | None:
    """One slip in a lowercase word that the operation undoes, chosen by `seed`; None if the word allows none."""

    letters = len(word)
    if operation == "transposition":
        places = [index for index in range(letters - 1) if word[index] != word[index + 1]]
        if not places:
            return None
        place = places[seed % len(places)]
        return word[:place] + word[place + 1] + word[place] + word[place + TYPO_TRANSPOSED_LETTERS:]
    if operation == "insertion":
        place = seed % letters
        return word[:place] + word[place + 1:]
    if operation == "substitution":
        place = seed % letters
        options = neighbours.get(word[place], "")
        if not options:
            return None
        return word[:place] + options[(seed // letters) % len(options)] + word[place + 1:]
    place = seed % (letters + 1)
    around = [word[index] for index in (place - 1, place) if 0 <= index < letters]
    extra = sorted(set(around) | {near for letter in around for near in neighbours.get(letter, "")})
    return word[:place] + extra[(seed // (letters + 1)) % len(extra)] + word[place:]


@dataclass
class Sample:
    half: int
    locale: str
    operation: Operation
    word: str
    typo: str
    weight: int
    capital: bool
    gate: str
    candidates: list[Candidate] = field(default_factory=list)


def synthetic_samples(directory: Path, corrector: TypoCorrector, knowledge: Callable[[str], bool]) -> list[Sample]:
    samples: list[Sample] = []
    for name in (*PROSE_FILES, *FORUM_FILES):
        for qid, text in messages(directory, name):
            half = draw("half", name, qid) % len(HALVES)
            for index, token, capital_allowed in words_of(text):
                locale = locale_of(token)
                lower = token.casefold()
                if locale is None or len(token) < TYPO_MIN_LETTERS or not (token == lower or capital_allowed):
                    continue
                if token[1:] != lower[1:]:
                    continue
                seed = draw("typo", name, qid, index)
                if seed % TYPO_EVALUATION_SAMPLE_MODULUS:
                    continue
                weight = corrector.lexicons[locale].weight(lower)
                if weight is None:
                    continue
                operation = OPERATIONS[(seed // TYPO_EVALUATION_SAMPLE_MODULUS) % len(OPERATIONS)]
                typo = make_typo(lower, operation, seed // TYPO_EVALUATION_SAMPLE_MODULUS // len(OPERATIONS),
                                 corrector.neighbours[locale])
                if typo is None or typo == lower:
                    continue
                capital = token != lower
                typed = typo[:1].upper() + typo[1:] if capital else typo
                judgement = corrector.judge(typed, locale, other_reading(typed, locale), capital_allowed=True,
                                            known=knowledge)
                sample = Sample(half, locale, operation, lower, typo, weight, capital, judgement.reason)
                if judgement.reason in ("no_candidate", "rare", "ambiguous", "corrected"):
                    sample.gate = ""
                    sample.candidates = corrector.candidates(typo, locale)
                samples.append(sample)
    return samples


def score(samples: Sequence[Sample], corrector: TypoCorrector, half: int | None) -> dict[str, object]:
    """Right, wrong and left by locale and operation, over the samples whose word the policy may correct to."""

    minimum = corrector.policy.minimum_weight
    table: dict[str, Counter[str]] = {}
    for sample in samples:
        if (half is not None and sample.half != half) or sample.weight < minimum:
            continue
        outcome = sample.gate
        if not outcome:
            judgement = corrector.decide(sample.candidates, sample.capital)
            outcome = judgement.reason
            if judgement.choice is not None and outcome == "corrected":
                outcome = "right" if judgement.choice.word.casefold() == sample.word else "wrong"
        for key in (f"{sample.locale}:{sample.operation}", sample.locale, "all"):
            table.setdefault(key, Counter())[outcome] += 1
    report: dict[str, object] = {}
    for key, counts in sorted(table.items()):
        total = sum(counts.values())
        corrected = counts["right"] + counts["wrong"]
        report[key] = {
            "samples": total, "right": counts["right"], "wrong": counts["wrong"],
            "right_share": counts["right"] / total if total else 0.0,
            "wrong_share_of_corrections": counts["wrong"] / corrected if corrected else 0.0,
            "left": {reason: count for reason, count in sorted(counts.items()) if reason not in ("right", "wrong")},
            # Typos that are words of the list themselves: out of a non-word corrector's reach by design.
            "real_word_typos": counts["occurs"],
        }
    return report


def channels() -> Iterator[tuple[tuple[Operation, float], ...]]:
    for deletion, insertion, substitution in itertools.product(
            TYPO_EVALUATION_DELETION_LOG_PROBABILITIES, TYPO_EVALUATION_INSERTION_LOG_PROBABILITIES,
            TYPO_EVALUATION_SUBSTITUTION_LOG_PROBABILITIES):
        yield (("transposition", TYPO_TRANSPOSITION_LOG_PROBABILITY), ("deletion", deletion),
               ("insertion", insertion), ("substitution", substitution))


def wilson_upper(wrong: int, total: int) -> float:
    """The upper bound of the Wilson score interval of the share `wrong / total`."""

    if not total:
        return 1.0
    z_squared = TYPO_EVALUATION_WILSON_Z * TYPO_EVALUATION_WILSON_Z
    share = wrong / total
    twice = total + total
    centre = share + z_squared / twice
    spread = TYPO_EVALUATION_WILSON_Z * math.sqrt(share * (1 - share) / total + z_squared / (twice * twice))
    return (centre + spread) / (1 + z_squared / total)


def tune(samples: Sequence[Sample], corrector: TypoCorrector) -> tuple[TypoPolicy, list[dict[str, object]]]:
    """The policy with the most right corrections on the tuning half whose wrong share stays within the acceptance.

    The slip probabilities decide which candidate is best and by what margin; the count floor and the ratio only
    accept or refuse that choice. So each set of probabilities is scored once, and every floor and ratio read off it.
    The choice is made at the design's count floor (TYPO_CANDIDATE_MIN_COUNT); the other floors are recorded. A policy
    qualifies when the upper bound of its wrong share (wilson_upper) is within TYPO_EVALUATION_MAX_WRONG_SHARE.
    """

    tried: list[dict[str, object]] = []
    best: tuple[int, TypoPolicy] | None = None
    tuning = [sample for sample in samples if sample.half == 0 and not sample.gate]
    population = Counter(sample.weight for sample in samples if sample.half == 0)
    for channel in channels():
        trial = TypoCorrector(corrector.lexicons, TypoPolicy(log_probabilities=channel))
        choices: list[tuple[int, int, float, bool]] = []
        for sample in tuning:
            judgement = trial.decide(sample.candidates, sample.capital)
            if judgement.choice is not None:
                choices.append((sample.weight, judgement.choice.weight, judgement.choice.margin,
                                judgement.choice.word.casefold() == sample.word))
        for minimum, ratio in itertools.product(TYPO_EVALUATION_MIN_COUNTS, TYPO_EVALUATION_RATIOS):
            policy = TypoPolicy(minimum, ratio, channel)
            floor, threshold = policy.minimum_weight, math.log(ratio)
            taken = [right for weight, chosen, margin, right in choices
                     if weight >= floor and chosen >= floor and margin >= threshold]
            right = sum(taken)
            wrong = len(taken) - right
            upper = wilson_upper(wrong, len(taken))
            feasible = upper <= TYPO_EVALUATION_MAX_WRONG_SHARE
            in_scope = sum(count for weight, count in population.items() if weight >= floor)
            tried.append({"policy": policy_record(policy), "samples": in_scope, "right": right, "wrong": wrong,
                          "wrong_share_upper_bound": upper, "feasible": feasible})
            if feasible and minimum == TYPO_CANDIDATE_MIN_COUNT and (best is None or right > best[0]):
                best = (right, policy)
    if best is None:
        raise SystemExit("no policy keeps the wrong corrections within the allowed share")
    return best[1], tried


def vocabulary_floor(arguments: argparse.Namespace) -> dict[str, object]:
    """The count of the technical vocabulary (TYPO_VOCABULARY_MIN_COUNT), chosen on the development splits.

    For each candidate count: how many corrections of the clean development texts the vocabulary would leave as typed
    (`--clean`: the clean-corrections.tsv of lists built without it), and how many synthetic typos of each operation no
    list holds that it would make known (the recall it costs, at most). The chosen count is the least one at which no
    operation loses more than TYPO_VOCABULARY_MAX_RECALL_LOSS of those typos.
    """

    import build_typo_frequency as builder

    with gzip.open(arguments.train_messages, "rt", encoding="utf-8") as messages_file, \
            gzip.open(arguments.train_code, "rt", encoding="utf-8") as code_file:
        counts = builder.vocabulary_counts(messages_file, code_file)
    lexicons: dict[str, TypoLexicon] = {}
    with tempfile.TemporaryDirectory() as directory:
        for source in builder.SOURCES:
            path = Path({"ru_RU": arguments.ru, "en_US": arguments.en}[source.locale])
            built = builder.build(source, path.read_text(encoding="utf-8").splitlines(), source.sha256)
            target = Path(directory) / f"{source.locale}.bin"
            target.write_bytes(built.file)
            lexicons[source.locale] = TypoLexicon.load(target, source.locale)
    corrector = TypoCorrector(lexicons)

    def known(text: str, floor: int) -> bool:
        return any(table[text.casefold()] >= floor for table in counts.values())

    removed: dict[int, Counter[str]] = {floor: Counter() for floor in TYPO_EVALUATION_VOCABULARY_MIN_COUNTS}
    with arguments.clean.open(encoding="utf-8") as handle:
        for line in itertools.islice(handle, 1, None):
            corpus, _, locale, _, word, *_ = line.split("\t")
            reading = next(iter(other_reading(word, locale).values()))
            for floor in removed:
                if known(word, floor) or known(reading, floor):
                    removed[floor][f"{corpus}:{locale}"] += 1
    lost: dict[int, Counter[str]] = {floor: Counter() for floor in TYPO_EVALUATION_VOCABULARY_MIN_COUNTS}
    typos: Counter[str] = Counter()
    for name in (*PROSE_FILES, *FORUM_FILES):
        for qid, text in messages(arguments.messages, name):
            for index, token, capital_allowed in words_of(text):
                locale = locale_of(token)
                lower = token.casefold()
                if (locale is None or len(token) < TYPO_MIN_LETTERS or not (token == lower or capital_allowed)
                        or token[1:] != lower[1:]):
                    continue
                seed = draw("typo", name, qid, index)
                weight = corrector.lexicons[locale].weight(lower)
                if seed % TYPO_EVALUATION_SAMPLE_MODULUS or weight is None or weight < corrector.policy.minimum_weight:
                    continue
                operation = OPERATIONS[(seed // TYPO_EVALUATION_SAMPLE_MODULUS) % len(OPERATIONS)]
                typo = make_typo(lower, operation, seed // TYPO_EVALUATION_SAMPLE_MODULUS // len(OPERATIONS),
                                 corrector.neighbours[locale])
                if (typo is None or typo == lower or len(typo) < TYPO_MIN_LETTERS
                        or corrector.lexicons[locale].occurs(typo)):
                    continue
                key = f"{locale}:{operation}"
                typos[key] += 1
                reading = next(iter(other_reading(typo, locale).values()))
                for floor in lost:
                    if known(typo, floor) or known(reading, floor):
                        lost[floor][key] += 1
    table = {str(floor): {"clean_corrections_left_as_typed": dict(removed[floor]),
                          "synthetic_typos_lost": {key: lost[floor][key] / typos[key] for key in sorted(typos)}}
             for floor in TYPO_EVALUATION_VOCABULARY_MIN_COUNTS}
    chosen = min(floor for floor in TYPO_EVALUATION_VOCABULARY_MIN_COUNTS
                 if all(lost[floor][key] <= TYPO_VOCABULARY_MAX_RECALL_LOSS * typos[key] for key in typos))
    return {"synthetic_typos_no_list_holds": dict(typos), "floors": table, "chosen": chosen}


def policy_record(policy: TypoPolicy) -> dict[str, object]:
    return {"candidate_min_count": policy.candidate_min_count, "posterior_ratio": policy.posterior_ratio,
            "log_probabilities": dict(policy.log_probabilities)}


@dataclass
class CleanTally:
    words: Counter[str] = field(default_factory=Counter)
    eligible: Counter[str] = field(default_factory=Counter)
    corrections: Counter[str] = field(default_factory=Counter)


def judge_cached(corrector: TypoCorrector, knowledge: Callable[[str], bool],
                 cache: dict[tuple[str, bool], TypoJudgement], token: str, locale: str,
                 capital_allowed: bool) -> TypoJudgement:
    key = (token, capital_allowed)
    if key not in cache:
        cache[key] = corrector.judge(token, locale, other_reading(token, locale), capital_allowed=capital_allowed,
                                     known=knowledge)
    return cache[key]


def clean(directory: Path, corrector: TypoCorrector, knowledge: Callable[[str], bool], out: Path) -> dict[str, object]:
    tally = CleanTally()
    cache: dict[tuple[str, bool], TypoJudgement] = {}
    rows: list[str] = []
    sources: list[tuple[str, Iterator[tuple[str, str]]]] = [
        (name, messages(directory, name)) for name in (*PROSE_FILES, *FORUM_FILES)]
    sources.append(("code-dev", code_lines(directory / CODE_FILE)))
    for name, texts in sources:
        for qid, text in texts:
            for _, token, capital_allowed in words_of(text):
                locale = locale_of(token)
                if locale is None:
                    continue
                kind = "code" if name == "code-dev" else "forum" if name in FORUM_FILES else "prose"
                key = f"{name}:{locale}"
                tally.words[key] += 1
                judgement = judge_cached(corrector, knowledge, cache, token, locale, capital_allowed)
                if judgement.reason == "shape":
                    continue
                tally.eligible[key] += 1
                if judgement.reason == "corrected":
                    assert judgement.choice is not None
                    tally.corrections[key] += 1
                    rows.append("\t".join((name, kind, locale, qid, token, judgement.choice.word,
                                           judgement.choice.edit.operation, str(judgement.count_order),
                                           f"{judgement.choice.margin:.2f}", text.replace("\t", " ").replace("\n", " "))))
    (out / "clean-corrections.tsv").write_text(
        "corpus\tkind\tlocale\tqid\tword\tcorrection\toperation\tcount_order\tmargin\ttext\n" + "\n".join(rows) + "\n",
        encoding="utf-8")
    report: dict[str, object] = {}
    for key in sorted(tally.words):
        words, corrections = tally.words[key], tally.corrections[key]
        report[key] = {"words": words, "eligible": tally.eligible[key], "corrections": corrections,
                       "per_10000_words": corrections * TYPO_ACCEPTED_PROSE_WORDS_PER_FALSE_CORRECTION / words}
    report["accepted"] = {"prose_words_per_false_correction": TYPO_ACCEPTED_PROSE_WORDS_PER_FALSE_CORRECTION,
                          "forum_words_per_false_correction": TYPO_ACCEPTED_FORUM_WORDS_PER_FALSE_CORRECTION,
                          "code": 0}
    return report


def code_lines(path: Path) -> Iterator[tuple[str, str]]:
    with gzip.open(path, "rt", encoding="utf-8") as handle:
        for number, line in enumerate(handle, 1):
            yield str(number), line.rstrip("\n")


def device_streams(root: Path, batteries: Mapping[str, str]) -> dict[str, list[tuple[str, dict[str, object]]]]:
    """The streams of every battery by device (the battery's name without its version digits: `pc`, `nout`), in
    typing order, each stream once."""

    devices: dict[str, list[tuple[str, dict[str, object]]]] = {}
    seen: set[tuple[str, str]] = set()
    for name, relative in batteries.items():
        for line in filter(None, (root / relative).read_text(encoding="utf-8").splitlines()):
            stream = json.loads(line)
            identity = (str(stream["t"]), str(stream["app"]))
            if identity not in seen:
                seen.add(identity)
                devices.setdefault(name.rstrip("0123456789"), []).append((name, stream))
    for streams in devices.values():
        streams.sort(key=lambda entry: str(entry[1]["t"]))
    return devices


def field_words(manifest: Path, corrector: TypoCorrector, knowledge: Callable[[str], bool], out: Path) -> dict[str, object]:
    """Every word the owner ended at a boundary, judged in typing order on each device, with the personal dictionary
    growing as it would in use: a word the corrector looked for candidates for and the owner left as typed counts, and
    at PERSONAL_WORD_MIN_COUNT it is the owner's own and never corrected again. A correction of a word the owner kept
    so often that it became theirs is a word of their vocabulary changed: harmful."""

    # The manifest lies in .t/field/ of the repository, and names the batteries from the repository's root.
    root = manifest.resolve().parent.parent.parent
    batteries = json.loads(manifest.read_text(encoding="utf-8"))["batteries"]
    cache: dict[tuple[str, bool], TypoJudgement] = {}
    rows: list[tuple[str, str, tuple[str, ...]]] = []
    counts: Counter[str] = Counter()
    vocabulary: dict[str, set[str]] = {}
    with tempfile.TemporaryDirectory() as directory:
        for device, streams in device_streams(root, batteries).items():
            personal = PersonalWords(Path(directory) / f"{device}.json", persist=lambda: False)
            mine = vocabulary.setdefault(device, set())
            for index, (name, stream) in enumerate(streams):
                following = streams[index + 1][1] if index + 1 < len(streams) else None
                retyped = retyped_words(stream, following)
                previous = ""
                items = stream["items"]
                assert isinstance(items, list)
                for item in items:
                    kept = item["original"] if item["intended_group"] == item["group"] else item["other"]
                    token = kept.strip(EDGE_SIGNS)
                    capital_allowed = not previous or previous.rstrip()[-1:] in SENTENCE_END
                    previous = kept
                    counts["words"] += 1
                    if item["trigger"] == "pause":
                        # Judged at a pause, the word may still be typed on (`добав` of `добавь`): the correction
                        # waits for a boundary.
                        counts["pause_items"] += 1
                        continue
                    locale = locale_of(token) if token else None
                    if locale is None:
                        continue
                    if token in personal:
                        counts["personal"] += 1
                        continue
                    judgement = judge_cached(corrector, knowledge, cache, token, locale, capital_allowed)
                    if judgement.reason == "shape":
                        continue
                    counts["eligible"] += 1
                    erased = item.get("excluded") == "erased"
                    if not erased and judgement.reason in ("no_candidate", "rare", "ambiguous", "corrected"):
                        if personal.keep(token):
                            mine.add(token.casefold())
                    if judgement.reason != "corrected":
                        continue
                    assert judgement.choice is not None
                    counts["corrections"] += 1
                    mark = ("erased_retyped_as_correction" if erased and judgement.choice.word.casefold() in retyped
                            else "erased_retyped_otherwise" if erased and retyped else "erased" if erased else "kept")
                    counts[mark] += 1
                    rows.append((device, token.casefold(), (
                        device, name, str(stream["app"]), item["t"], token, judgement.choice.word,
                        judgement.choice.edit.operation, str(judgement.count_order), f"{judgement.choice.margin:.2f}",
                        mark, " ".join(sorted(retyped)))))
    harmful = [fields for device, word, fields in rows if word in vocabulary[device]]
    counts["corrections_of_words_that_became_the_owner_s"] = len(harmful)
    header = "device\tbattery\tapp\ttime\tword\tcorrection\toperation\tcount_order\tmargin\tmark\tretyped_next\n"
    (out / "field-corrections.tsv").write_text(header + "".join("\t".join(row) + "\n" for _, _, row in rows),
                                               encoding="utf-8")
    (out / "field-harmful.tsv").write_text(header + "".join("\t".join(row) + "\n" for row in harmful),
                                           encoding="utf-8")
    return dict(counts)


def retyped_words(stream: Mapping[str, object], following: Mapping[str, object] | None) -> set[str]:
    """The words of the stream right after an erasing one, in the same application and soon after it."""

    if following is None or following.get("app") != stream.get("app") or not stream.get("deleted_chars"):
        return set()
    items = following.get("items")
    closed = stream.get("closed_ts")
    if not isinstance(items, list) or not items or not isinstance(closed, (int, float)):
        return set()
    started = datetime.strptime(str(items[0]["t"]), "%Y-%m-%d %H:%M:%S,%f").timestamp()
    if started - closed >= TYPO_EVALUATION_RETYPE_WINDOW_SECONDS:
        return set()
    words: set[str] = set()
    for item in items:
        kept = item["original"] if item["intended_group"] == item["group"] else item["other"]
        words.add(str(kept).strip(EDGE_SIGNS).casefold())
    return words


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("measurement", choices=("synthetic", "clean", "field", "all", "vocabulary"))
    parser.add_argument("--messages", type=Path, default=ROOT / ".t/research-2026-09-29/messages-tool")
    parser.add_argument("--manifest", type=Path, default=ROOT / ".t/field/manifest.json")
    parser.add_argument("--out", type=Path, default=ROOT / ".t/typo")
    parser.add_argument("--tune", action="store_true", help="tune the policy on the first half of the synthetic typos")
    parser.add_argument("--clean", type=Path, help="vocabulary: clean-corrections.tsv of lists without the vocabulary")
    parser.add_argument("--ru", type=Path, help="vocabulary: the Russian full frequency list")
    parser.add_argument("--en", type=Path, help="vocabulary: the English full frequency list")
    parser.add_argument("--train-messages", type=Path, help="vocabulary: so-train.jsonl.gz")
    parser.add_argument("--train-code", type=Path, help="vocabulary: code-train.txt.gz")
    parser.add_argument("--min-count", type=int, help="measure another least count of a corrected word")
    parser.add_argument("--ratio", type=float, help="measure another posterior ratio")
    parser.add_argument("--log-probabilities", help="measure other slip probabilities: deletion,insertion,substitution")
    arguments = parser.parse_args(argv)
    arguments.out.mkdir(parents=True, exist_ok=True)
    corrector = TypoCorrector.packaged(GROUP_LOCALES)
    shipped = corrector.policy
    channel = dict(shipped.log_probabilities)
    if arguments.log_probabilities:
        deletion, insertion, substitution = (float(value) for value in arguments.log_probabilities.split(","))
        channel.update({"deletion": deletion, "insertion": insertion, "substitution": substitution})
    policy = TypoPolicy(arguments.min_count or shipped.candidate_min_count, arguments.ratio or shipped.posterior_ratio,
                        tuple((operation, channel[operation]) for operation in OPERATIONS))
    corrector = TypoCorrector(corrector.lexicons, policy)
    knowledge = Knowledge()
    report: dict[str, object] = {"policy": policy_record(corrector.policy)}
    if arguments.measurement == "vocabulary":
        report["vocabulary"] = vocabulary_floor(arguments)
    if arguments.measurement in ("synthetic", "all"):
        samples = synthetic_samples(arguments.messages, corrector, knowledge)
        print(f"synthetic samples: {len(samples)}", file=sys.stderr, flush=True)
        if arguments.tune:
            tuned, tried = tune(samples, corrector)
            (arguments.out / "synthetic-tuning.json").write_text(
                json.dumps(tried, ensure_ascii=False, indent=REPORT_JSON_INDENT) + "\n", encoding="utf-8")
            report["tuned_policy"] = policy_record(tuned)
            report["tuned_on_tuning_half"] = score(samples, TypoCorrector(corrector.lexicons, tuned), 0)
            report["tuned_on_report_half"] = score(samples, TypoCorrector(corrector.lexicons, tuned), 1)
        report["synthetic_tuning_half"] = score(samples, corrector, 0)
        report["synthetic_report_half"] = score(samples, corrector, 1)
    if arguments.measurement in ("clean", "all"):
        report["clean"] = clean(arguments.messages, corrector, knowledge, arguments.out)
    if arguments.measurement in ("field", "all"):
        report["field"] = field_words(arguments.manifest, corrector, knowledge, arguments.out)
    (arguments.out / f"report-{arguments.measurement}.json").write_text(
        json.dumps(report, ensure_ascii=False, indent=REPORT_JSON_INDENT) + "\n", encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=REPORT_JSON_INDENT))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
