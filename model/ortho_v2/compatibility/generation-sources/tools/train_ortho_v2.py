#!/usr/bin/env python3
"""Train key-space orthotactics v2 by counting, then seal before testing.

Counting is still the whole algorithm, so the result replays byte for byte.
What changed against v1 is the evidence and the decision rule, and each change
answers a measurement rather than a preference:

* **The frozen lexicons are counted.** Russian had a third of the English
  material in the phrase snapshot, and that deficit - not the model form - was
  what made short Russian tokens unreachable. The lists enter the character
  counts only; they are lowercased, so as negatives they would raise the
  threshold until a real command such as ``htop`` stopped converting.
* **The threshold is one per language, and its margin is measured rather than
  chosen.** Thresholds per token length were tried and dropped: each length
  bucket held only 72 to 517 negatives, and the worst of so few sat eight to ten
  nats below the worst on independent text, so the buckets were measuring
  sampling noise rather than a prior. Pooled per language the same estimate
  rests on thousands. The margin above the observed maximum is the shortfall of
  that maximum, estimated by splitting the calibration negatives in half and
  measuring how far one half's maximum falls from the other's.
* **The licence stops where the dictionary starts.** This model is consulted
  only for tokens the current layout's dictionary does not know. `руку` is an
  ordinary Russian word whose keys spell the ordinary English word `here`; no
  character model can separate them, and pretending otherwise cost a threshold
  high enough to silence everything else.
* **A gate family is reserved.** Sixteen percent of key sequences contribute no
  character to any count and are scored exactly once, here.

Which population may compare the two models is not obvious, and getting it wrong
flatters one of them. The installed model counted 12,287 of those gate sequences,
because they were only ever withheld from THIS candidate; scoring both there asks
one model about text it memorised and the other about text it has never seen. The
test split leans the other way: some of its evaluations are sequences the
candidate learned from the lexicons and the installed model never saw. The
promotion decision therefore rests on the intersection - sequences in a gate
family that the installed model did not count either - where neither side has an
advantage. The two larger populations are still measured and published, the gate
against an absolute ceiling and the test split against the installed behaviour,
but neither of them decides alone.
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import math
import sys
from collections import Counter
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Final, cast

sys.path.insert(0, str(Path(__file__).resolve().parent))

import ortho_v2_corpus as corpus  # noqa: E402
import ortho_v2_verified as verified  # noqa: E402

from keyswitch.context_policy import word_shaped  # noqa: E402
from keyswitch.ortho_model import (  # noqa: E402
    BOS, EOS, SCRIPTS, SHAPES,
)

ROOT: Final[Path] = Path(__file__).resolve().parents[1]
DIRECTORY: Final[Path] = ROOT / "model/ortho_v2"
CONFIG: Final[Path] = DIRECTORY / "config.json"
CANDIDATE: Final[Path] = DIRECTORY / "candidate.json"
SEAL: Final[Path] = DIRECTORY / "seal.json"
REPORT: Final[Path] = DIRECTORY / "report.json"
ARTIFACT: Final[Path] = ROOT / "src/keyswitch/resources/models/ortho_v2.json"
BASELINE: Final[Path] = ROOT / "model/ortho_v1/candidate.json"
BASELINE_CORPUS: Final[Path] = ROOT / "model/ortho_v1/tokens.jsonl.gz"
NAMESPACE: Final[str] = "keyswitch:ortho-v2:candidate1"
COUNTED_SPLITS: Final[frozenset[str]] = frozenset({"train", "lexicon"})


def canonical(value: object) -> bytes:
    return (json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":")) + "\n").encode()


def checksum(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def config() -> dict[str, object]:
    value: object = json.loads(CONFIG.read_bytes())
    if not isinstance(value, dict) or value.get("schema_version") != 1:
        raise ValueError("incompatible orthotactic configuration")
    return cast(dict[str, object], value)


def bucket_of(keys: str, buckets: Sequence[int]) -> int:
    """The length bucket a token falls in; the last bucket is open-ended."""

    length = len(keys)
    for value in buckets:
        if length <= value:
            return value
    return buckets[-1]


def counts(rows: list[dict[str, object]], order: int) -> tuple[
        dict[str, Counter[str]], dict[str, Counter[str]], dict[str, Counter[str]]]:
    """Character n-gram counts, prose shapes and abbreviation shapes per script.

    A gate-family sequence is skipped here and nowhere else: that is what makes
    its later evaluation a statement about sequences the model never saw.
    """

    grams: dict[str, Counter[str]] = {script: Counter() for script in SCRIPTS}
    shapes: dict[str, Counter[str]] = {script: Counter() for script in SCRIPTS}
    acronyms: dict[str, Counter[str]] = {script: Counter() for script in SCRIPTS}
    upper_keys: dict[str, set[str]] = {script: set() for script in SCRIPTS}
    for row in rows:
        if row["split"] != "train" or corpus.gate_family(str(row["keys"])):
            continue
        shape_counts = cast(dict[str, int], row["shapes"])
        if shape_counts.get("upper"):
            upper_keys[str(row["script"])].add(str(row["keys"]))
    for row in rows:
        if row["split"] not in COUNTED_SPLITS or corpus.gate_family(str(row["keys"])):
            continue
        script, keys = str(row["script"]), str(row["keys"])
        shape_counts = cast(dict[str, int], row["shapes"])
        weight = sum(shape_counts.values())
        # Text of this language reaches a model in two shapes, one per layout it
        # could have been typed in: the trailing comma of `French,` is a letter
        # in Russian and a stripped symbol stroke in English. Both are things a
        # user produces while writing this language, so both are counted. The
        # counting shape is the word core, not what the engine would serve: a
        # word the source wrapped in quotation marks is still evidence about
        # which letter sequences the language produces.
        for sequence in {corpus.word_core(keys, layout) for layout in SCRIPTS}:
            if not sequence:
                continue
            padded = BOS * (order - 1) + sequence + EOS
            for index in range(order - 1, len(padded)):
                for size in range(1, order + 1):
                    grams[script][padded[index - size + 1:index + 1]] += weight
        if row["split"] != "train":
            # A word list carries no case, so it may say what sequences exist
            # but nothing about how an abbreviation is written.
            continue
        target = acronyms if keys in upper_keys[script] else shapes
        for name, value in shape_counts.items():
            target[script][name] += value
    return grams, shapes, acronyms


def fit_channel(gram_counts: Counter[str], order: int, discount: float,
                alphabet: int) -> tuple[dict[str, float], dict[str, float], float]:
    """Interpolated absolute discounting, stored ARPA-shaped for lookup."""

    context_total: Counter[str] = Counter()
    context_types: Counter[str] = Counter()
    for gram, count in sorted(gram_counts.items()):
        if len(gram) == 1:
            continue
        context_total[gram[:-1]] += count
        context_types[gram[:-1]] += 1
    uniform = -math.log(alphabet)
    unigram_total = sum(count for gram, count in gram_counts.items() if len(gram) == 1)
    logprob: dict[str, float] = {}
    backoff: dict[str, float] = {}
    for size in range(1, order + 1):
        for gram in sorted(gram for gram in gram_counts if len(gram) == size):
            count = gram_counts[gram]
            if size == 1:
                probability = (count + 0.5) / (unigram_total + 0.5 * alphabet)
                logprob[gram] = math.log(probability)
                continue
            context = gram[:-1]
            total = context_total[context]
            weight = discount * context_types[context] / total
            lower = math.exp(logprob[gram[1:]]) if gram[1:] in logprob else math.exp(uniform)
            probability = max(count - discount, 0.0) / total + weight * lower
            logprob[gram] = math.log(probability)
            backoff[context] = math.log(weight)
    return logprob, backoff, uniform


def quantise(value: float, scale: int) -> int:
    return int(round(value * scale))


def shape_channel(observed: dict[str, Counter[str]], scale: int) -> dict[str, object]:
    """P(shape | reading), smoothed so an unseen shape is unlikely, not impossible."""

    result: dict[str, object] = {}
    for script in SCRIPTS:
        counter = observed[script]
        total = sum(counter.values()) + 0.5 * len(SHAPES)
        result[script] = {
            shape: quantise(math.log((counter.get(shape, 0) + 0.5) / total), scale)
            for shape in SHAPES
        }
    return result


def build(rows: list[dict[str, object]], settings: dict[str, object]) -> dict[str, object]:
    order = cast(int, settings["order"])
    discount = float(cast(float, settings["discount"]))
    scale = cast(int, settings["scale"])
    gram_counts, prose, acronym = counts(rows, order)
    models: dict[str, object] = {}
    for script in SCRIPTS:
        alphabet = len({gram for gram in gram_counts[script] if len(gram) == 1})
        logprob, backoff, uniform = fit_channel(gram_counts[script], order, discount, alphabet)
        names = sorted(logprob)
        models[script] = {
            "grams": "\n".join(names),
            "logprob": [quantise(logprob[name], scale) for name in names],
            "backoff": [[name, quantise(backoff[name], scale)] for name in sorted(backoff)],
            "uniform": quantise(uniform, scale),
        }
    payload: dict[str, object] = {
        "schema_version": 1, "order": order, "scale": scale,
        "models": models,
        "prose_shape": shape_channel(prose, scale),
        "acronym_shape": shape_channel(acronym, scale),
    }
    digest = hashlib.sha256(canonical(payload)).hexdigest()
    payload["version"] = f"ortho-v2-{digest[:12]}"
    payload["weights_sha256"] = digest
    return payload


def baseline_counted() -> frozenset[str]:
    """Key sequences the installed model counted, read from its frozen corpus."""

    with gzip.open(BASELINE_CORPUS, "rt", encoding="utf-8") as handle:
        return frozenset(
            str(row["keys"]) for row in map(json.loads, handle) if row["split"] == "train"
        )


def believable(keys: str, direction: str, trusted: dict[str, frozenset[str]],
               attested: frozenset[str]) -> bool:
    """REJECTED. Kept only so the rejection is legible; never call this.

    The idea was that a negative label may be discounted when the corpus cannot
    stand behind it: the sentence around it is not verifiably monolingual, or the
    snapshot shows the token once, or the token reads as an ordinary word of the
    other language. Each condition had a real token behind it - `пдфку` in a
    Russian sentence is the English word `glare` typed in the wrong layout.

    Applied, it dropped the threshold far enough to look like two points of free
    accuracy, and a pre-release review drove the engine and found the truth: of
    the nineteen Russian sequences the lower threshold newly converted, exactly
    ONE was genuine wrong-layout text. The other eighteen were ordinary Russian -
    `гифка`, `Ютуб`, `флуд`, `какбэ`, `тыща` - turned into `ubarf`, `Юne,`,
    `akel`, `rfr,'`, `nsof`. The filter had not removed untrustworthy labels; it
    had removed the hard negatives, and the measured "no false conversions" was
    taken on a population emptied of the errors.

    The lesson is worth more than the code: excluding a label does not make the
    token safe to convert, it only hides it from the metric. A negative
    population must hold every token the engine can present, whatever one thinks
    of its label. A wrong label such as `пдфку` costs recall; excluding a real
    word costs the user's text, and that is the worse error by far.
    """

    raise NotImplementedError("see the docstring: this filter corrupts ordinary words")


def attested_keys(rows: list[dict[str, object]]) -> frozenset[str]:
    """Key sequences the snapshot shows more than once.

    A token the snapshot shows once cannot be checked against anything:
    `каеффф`, `фнафер`, `qeṛṛeḍ` and `áñez` are Kabyle, Berber and garbage that
    inherited a language label from the sentence around them, and 58.9% of the
    negatives are of that kind. Such a label is not ground truth, so converting
    one is not evidence of harm; converting something the snapshot shows
    repeatedly is.
    """

    occurrences: Counter[str] = Counter()
    for row in rows:
        if row["split"] == "lexicon":
            continue
        weight = sum(cast(dict[str, int], row["shapes"]).values())
        # A sequence without punctuation is served identically in both layouts;
        # counting it once per direction would make a single occurrence look
        # like two and call an unverifiable label attested.
        served: set[str] = set()
        for direction in SCRIPTS:
            served.update(corpus.served(str(row["keys"]), direction))
        for keys in served:
            occurrences[keys] += weight
    return frozenset(keys for keys, count in occurrences.items() if count > 1)


def candidate_counted(rows: list[dict[str, object]]) -> frozenset[str]:
    """Key sequences this candidate counted, by the same rule `counts` applies."""

    return frozenset(
        str(row["keys"]) for row in rows
        if row["split"] in COUNTED_SPLITS and not corpus.gate_family(str(row["keys"]))
    )


def evidence_rows(rows: list[dict[str, object]], split: str | None, *, gate: bool,
                  seen: frozenset[str] | None = None) -> list[tuple[str, str, str, int]]:
    """(script, whole keys, shape, weight) for the requested population.

    The keys are stored whole; which part of them a direction actually sees is
    decided in `measure`, because the engine strips a different edge depending
    on the layout the user is typing in.

    Lexicon rows never appear: they are lowercased, so an abbreviation there has
    lost the capitals that protect it, and scoring them as negatives would set a
    threshold no real typing can justify.
    """

    result: list[tuple[str, str, str, int]] = []
    for row in rows:
        keys = str(row["keys"])
        if row["split"] == "lexicon" or corpus.gate_family(keys) != gate:
            continue
        if split is not None and row["split"] != split:
            continue
        if seen is not None and keys in seen:
            continue
        shape_counts = cast(dict[str, int], row["shapes"])
        for shape in sorted(shape_counts):
            result.append((str(row["script"]), keys, shape, shape_counts[shape]))
    return result


def governed(keys: str, shape: str, minimum_length: int, direction: str) -> bool:
    """Only what the engine actually hands the model reaches an evaluation.

    The certified code guard already refuses tokens with digits, path or address
    characters, ALL-CAPS and camelCase, so a realistic capitalised abbreviation
    never reaches this model. Measuring on a population the runtime excludes
    would flatter the result.

    What is left must also be a word in the layout being judged, which
    `word_shaped` decides on the text the user sees. Punctuation caught between
    letters set every threshold in the calibration split - `и"ю`, `хэ)б`,
    `зы-ы-ырк` - and silenced ordinary commands. The runtime applies the same
    predicate to the same string, so the population here is the population there.
    """

    if len(keys) < minimum_length or shape == "upper":
        return False
    if not word_shaped(corpus.visible(keys, direction)):
        return False
    return not any(character.isdigit() or character in "_/\\=:@" for character in keys)


def measure(model: CountedModel, samples: list[tuple[str, str, str, int]],
            minimum_length: int, trusted: dict[str, frozenset[str]] | None = None,
            attested: frozenset[str] | None = None) -> list[tuple[str, float, int, str, str, int]]:
    """Score each token in BOTH directions, keeping its length bucket.

    The score depends only on the keys, never on the label: for a user typing in
    layout d it is log P_other(keys) - log P_d(keys). What the label decides is
    whether that token is a negative for direction d (its own script) or a
    positive (the other script, typed with the layout wrong).
    """

    scored: list[tuple[str, float, int, str, str, int]] = []
    for script, whole, shape, weight in samples:
        for direction in SCRIPTS:
            # One key sequence can reach the model as several tokens: the engine
            # commits a word at every key that writes punctuation in the layout
            # being typed, so `"ably"` is one token and `word!more` is two.
              for keys in corpus.served(whole, direction):
                    if not governed(keys, shape, minimum_length, direction):
                        continue
                    if corpus.dictionary_known(direction, keys):
                        # The dictionary of the current layout knows this token, so
                        # the word models decide it and this one is never consulted.
                        continue

                    scored.append((f"{script}|{direction}",
                                   model.score(keys, shape, direction),
                                   weight, keys, shape, len(keys)))
    return scored


def lexical_samples(minimum_length: int) -> list[tuple[str, str, str, int]]:
    """A harder, separate track: the frozen word lists, held-out families only.

    These lists are lowercased, so an abbreviation such as РСФСР appears without
    the capitals that would protect it in real typing. That makes this a stress
    track, deliberately harsher than what a user can produce; it is reported
    separately and never sets a threshold. Only gate families appear, so nothing
    here contributed a character to the counts.
    """

    result: list[tuple[str, str, str, int]] = []
    for (script, keys), _weight in sorted(corpus.lexicon_rows().items()):
        if corpus.gate_family(keys) and governed(keys, "lower", minimum_length, script):
            result.append((script, keys, "lower", 1))
    return result


def thresholds_from(scored: list[tuple[str, float, int, str, str, int]],
                    settings: dict[str, object]) -> dict[str, float]:
    """The worst label this corpus can stand behind, plus a fixed headroom.

    Estimating the shortfall of the maximum by resampling was tried while the
    negatives still held Kabyle, garbage and wrong-layout text: the tail was a
    few isolated points, so the estimate ran to thirteen and seventeen nats and
    swallowed every gain. Held to labels that survive `believable`, the tail is
    ordinary and a constant headroom is enough - the same three-to-five nats v1
    used, and the measurement behind the choice is in the model card.
    """

    margin = float(cast(float, settings["margin_nats"]))
    result: dict[str, float] = {}
    for script in SCRIPTS:
        negatives = [total for label, total, *_rest in scored if label == f"{script}|{script}"]
        if not negatives:
            raise ValueError(f"calibration split has no believable {script} negatives")
        result[script] = max(negatives) + margin
    return result


def outcome(scored: list[tuple[str, float, int, str, str, int]],
            thresholds: dict[str, float], buckets: Sequence[int],
            attested: frozenset[str] = frozenset()) -> dict[str, object]:
    """False conversions and recall, counted by type, occurrence and length."""

    counted: Counter[str] = Counter()
    worst: dict[str, list[tuple[float, str, str]]] = {script: [] for script in SCRIPTS}
    for label, total, weight, keys, shape, _length in scored:
        script, direction = label.split("|")
        bucket = bucket_of(keys, buckets)
        converts = total > thresholds[direction]
        if script == direction:
            # The user was in this token's own layout: converting corrupts text.
            counted[f"{direction}:negative_types"] += 1
            counted[f"{direction}:negative_occurrences"] += weight
            if converts:
                counted[f"{direction}:false_types"] += 1
                counted[f"{direction}:false_occurrences"] += weight
                if weight > 1:
                    counted[f"{direction}:false_repeated_types"] += 1
                if keys in attested:
                    # The snapshot shows this sequence more than once, so its
                    # label can be checked and this is a real error.
                    counted[f"{direction}:false_attested_types"] += 1
                worst[direction].append((total, keys, shape))
        else:
            # The token belongs to the other language: the layout was wrong.
            counted[f"{direction}:positive_types"] += 1
            counted[f"{direction}:positive_occurrences"] += weight
            counted[f"{direction}:positive_types_len{bucket}"] += 1
            if converts:
                counted[f"{direction}:recalled_types"] += 1
                counted[f"{direction}:recalled_occurrences"] += weight
                counted[f"{direction}:recalled_types_len{bucket}"] += 1
    report: dict[str, object] = {"counts": dict(sorted(counted.items()))}
    report["worst_false"] = {
        script: [{"score": round(value, 4), "keys": keys, "shape": shape}
                 for value, keys, shape in sorted(items, reverse=True)[:10]]
        for script, items in worst.items()
    }
    return report


@dataclass(frozen=True)
class Totals:
    """What a promotion decision is allowed to look at."""

    negative_types: int
    negative_occurrences: int
    false_types: int
    false_occurrences: int
    false_repeated_types: int
    false_attested_types: int
    positive_types: int
    recall: float

    @property
    def false_per_10000(self) -> float:
        return 10_000 * self.false_occurrences / max(1, self.negative_occurrences)


def totals(result: dict[str, object]) -> Totals:
    counts_map = cast(dict[str, int], result["counts"])

    def summed(name: str) -> int:
        return sum(counts_map.get(f"{script}:{name}", 0) for script in SCRIPTS)

    positives = summed("positive_types")
    return Totals(summed("negative_types"), summed("negative_occurrences"),
                  summed("false_types"), summed("false_occurrences"),
                  summed("false_repeated_types"), summed("false_attested_types"),
                  positives, summed("recalled_types") / max(1, positives))


def promoted(result: Totals, baseline: Totals, promotion: dict[str, object],
             prefix: str) -> bool:
    """Beat what is installed today, on labels this corpus can stand behind.

    Both halves matter. A candidate must convert nothing that a verified,
    attested, unambiguous label says to leave alone - that is zero, not a rate,
    because these are the labels that survived every check. And it must recover
    more than the installed model does on the same rows, by a margin large enough
    not to be noise.
    """

    return (result.negative_types >= cast(int, promotion[f"minimum_{prefix}negatives"])
            and result.false_types == 0
            and result.recall >= baseline.recall + float(
                cast(float, promotion["minimum_recall_gain"])))


class CountedModel:
    """A counted artifact, scored here rather than through the runtime loader.

    This pipeline measures; it does not ship. The installed model and every
    candidate are read by these few lines, so the two are compared by exactly the
    same code and neither depends on which schema the runtime happens to accept.
    """

    def __init__(self, path: Path) -> None:
        payload = cast(dict[str, object], json.loads(path.read_bytes()))
        if payload.get("schema_version") != 1:
            raise ValueError("unexpected counted artifact")
        scale = cast(int, payload["scale"])
        self.order = cast(int, payload["order"])
        self.version = cast(str, payload["version"])
        models = cast(dict[str, dict[str, object]], payload["models"])
        self.logprob: dict[str, dict[str, float]] = {}
        self.backoff: dict[str, dict[str, float]] = {}
        self.uniform: dict[str, float] = {}
        for script in SCRIPTS:
            channel = models[script]
            names = cast(str, channel["grams"]).split("\n")
            self.logprob[script] = {
                name: value / scale
                for name, value in zip(names, cast(list[int], channel["logprob"]), strict=True)
            }
            self.backoff[script] = {
                cast(str, name): cast(int, value) / scale
                for name, value in cast(list[list[object]], channel["backoff"])
            }
            self.uniform[script] = cast(int, channel["uniform"]) / scale
        shapes = {name: cast(dict[str, dict[str, int]], payload[name])
                  for name in ("prose_shape", "acronym_shape")}
        self.shape = {
            name: {script: {shape: value / scale for shape, value in table.items()}
                   for script, table in tables.items()}
            for name, tables in shapes.items()
        }
        self.thresholds = {script: cast(dict[str, int], payload["thresholds"])[script] / scale
                           for script in SCRIPTS}

    def _conditional(self, script: str, gram: str) -> float:
        accumulated = 0.0
        while True:
            found = self.logprob[script].get(gram)
            if found is not None:
                return accumulated + found
            if len(gram) == 1:
                return accumulated + self.uniform[script]
            accumulated += self.backoff[script].get(gram[:-1], 0.0)
            gram = gram[1:]

    def _sequence(self, script: str, keys: str) -> float:
        padded = BOS * (self.order - 1) + keys + EOS
        return sum(self._conditional(script, padded[index - self.order + 1:index + 1])
                   for index in range(self.order - 1, len(padded)))

    def score(self, keys: str, shape: str, source: str) -> float:
        """Nats in favour of reading these keys in the other layout."""

        target = "ru" if source == "en" else "en"
        shape = shape if shape in SHAPES else "lower"
        return (self._sequence(target, keys) - self._sequence(source, keys)
                - (self.shape["acronym_shape"][source][shape]
                   - self.shape["prose_shape"][source][shape]))

    def converts(self, keys: str, shape: str, source: str) -> bool:
        return self.score(keys, shape, source) > self.thresholds[source]


def baseline_outcome(model: CountedModel, samples: list[tuple[str, str, str, int]],
                     minimum_length: int, *, guards: bool = False,
                     attested: frozenset[str] = frozenset(),
                     trusted: dict[str, frozenset[str]] | None = None) -> Totals:
    """The installed model's numbers on exactly the rows the candidate saw.

    With `guards` the two runtime refusals this release adds are applied as well,
    which isolates what the weights alone changed. Without them this is the
    behaviour a user has today, and that is what promotion is judged against.
    """

    counted: Counter[str] = Counter()
    for script, whole, shape, weight in samples:
        for direction in SCRIPTS:
            for keys in corpus.served(whole, direction):
                if len(keys) < minimum_length or shape == "upper":
                    continue
                if any(character.isdigit() or character in "_/\\=:@" for character in keys):
                    continue
                if guards and (not governed(keys, shape, minimum_length, direction)
                               or corpus.dictionary_known(direction, keys)):
                    continue

                converts = model.converts(keys, shape, direction)
                if script == direction:
                    counted[f"{direction}:negative_types"] += 1
                    counted[f"{direction}:negative_occurrences"] += weight
                    if converts:
                        counted[f"{direction}:false_types"] += 1
                        counted[f"{direction}:false_occurrences"] += weight
                        if weight > 1:
                            counted[f"{direction}:false_repeated_types"] += 1
                else:
                    counted[f"{direction}:positive_types"] += 1
                    counted[f"{direction}:positive_occurrences"] += weight
                    if converts:
                        counted[f"{direction}:recalled_types"] += 1
                        counted[f"{direction}:recalled_occurrences"] += weight
    return totals({"counts": dict(counted)})


def provenance() -> dict[str, str]:
    paths = (ROOT / "tools/train_ortho_v2.py", ROOT / "tools/ortho_v2_corpus.py",
             ROOT / "src/keyswitch/ortho_model.py", corpus.TOKENS, corpus.RECEIPT,
             CONFIG, BASELINE, BASELINE_CORPUS, corpus.KNOWN_EVIDENCE,
             ROOT / "model/ortho_v2/sources/known-receipt.json",
             verified.LABELS, verified.RECEIPT)
    return {path.relative_to(ROOT).as_posix(): checksum(path) for path in paths}


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("fit", "test", "verify", "promote"))
    arguments = parser.parse_args(argv)
    settings = config()
    buckets = cast(list[int], settings["length_buckets"])
    rows = corpus.load_tokens()
    if checksum(corpus.TOKENS) != cast(str, cast(dict[str, object], json.loads(
            corpus.RECEIPT.read_bytes()))["tokens_sha256"]):
        raise ValueError("key-space corpus does not match its receipt")
    minimum = cast(int, settings["minimum_length"])
    if arguments.command == "fit":
        if CANDIDATE.exists() or SEAL.exists():
            raise ValueError("candidate already exists; do not refit over sealed evidence")
        payload = build(rows, settings)
        # The thresholds ship inside the artifact: the runtime has no other file
        # to read them from. They are computed on calibration only, after the
        # weights are fixed, so the model identity above does not depend on them.
        payload["minimum_length"] = minimum
        payload["thresholds"] = {script: 0 for script in SCRIPTS}
        CANDIDATE.write_bytes(canonical(payload))
        model = CountedModel(CANDIDATE)
        # Both non-gate development splits set the thresholds. One of them alone
        # left buckets whose worst negative was an artefact seen once, and the
        # threshold then answered to that artefact rather than to typing.
        calibration = measure(model, evidence_rows(rows, "calibration", gate=False)
                              + evidence_rows(rows, "development", gate=False), minimum)
        thresholds = thresholds_from(calibration, settings)
        payload["thresholds"] = {
            script: quantise(thresholds[script], cast(int, settings["scale"]))
            for script in SCRIPTS
        }
        CANDIDATE.write_bytes(canonical(payload))
        seal = {
            "schema_version": 1, "stage": "sealed-before-test", "namespace": NAMESPACE,
            "model_version": payload["version"], "candidate_sha256": checksum(CANDIDATE),
            "thresholds": {script: round(value, 6) for script, value in thresholds.items()},
            "config": settings, "provenance": provenance(),
            "calibration_rows": len(calibration),
        }
        SEAL.write_bytes(canonical(seal))
        print(json.dumps(seal, ensure_ascii=False, indent=2))
        return 0
    seal = cast(dict[str, object], json.loads(SEAL.read_bytes()))
    if seal.get("provenance") != provenance() or seal.get("candidate_sha256") != checksum(CANDIDATE):
        raise ValueError("candidate or provenance changed after the seal")
    model = CountedModel(CANDIDATE)
    thresholds = {script: float(value)
                  for script, value in cast(dict[str, float], seal["thresholds"]).items()}
    if arguments.command == "test":
        if REPORT.exists():
            raise ValueError("test already observed; re-scoring it would not be an independent test")
        seen = baseline_counted() | candidate_counted(rows)
        attested = attested_keys(rows)
        trusted = verified.load()
        gate_rows = evidence_rows(rows, None, gate=True)
        unseen_rows = evidence_rows(rows, None, gate=True, seen=seen)
        test_rows = evidence_rows(rows, "test", gate=False)
        gate = outcome(measure(model, gate_rows, minimum), thresholds, buckets, attested)
        unseen = outcome(measure(model, unseen_rows, minimum), thresholds, buckets, attested)
        held = outcome(measure(model, test_rows, minimum), thresholds, buckets, attested)
        lexical = outcome(measure(model, lexical_samples(minimum), minimum), thresholds, buckets)
        promotion = cast(dict[str, object], settings["promotion"])
        gate_totals, unseen_totals, held_totals = totals(gate), totals(unseen), totals(held)
        legacy = CountedModel(BASELINE)
        gate_baseline = baseline_outcome(legacy, gate_rows, minimum, attested=attested)
        unseen_baseline = baseline_outcome(legacy, unseen_rows, minimum, attested=attested)
        test_baseline = baseline_outcome(legacy, test_rows, minimum, attested=attested)
        gate_guarded = baseline_outcome(legacy, gate_rows, minimum, guards=True, attested=attested)
        test_guarded = baseline_outcome(legacy, test_rows, minimum, guards=True, attested=attested)
        passed = (promoted(unseen_totals, unseen_baseline, promotion, "unseen_")
                  and promoted(gate_totals, gate_baseline, promotion, "gate_")
                  and promoted(held_totals, test_baseline, promotion, "test_"))
        payload = {
            "schema_version": 1, "model_version": seal["model_version"],
            "seal_sha256": checksum(SEAL), "candidate_sha256": checksum(CANDIDATE),
            "thresholds": seal["thresholds"],
            "gate_families": gate, "gate_recall": round(gate_totals.recall, 6),
            "gate_false_per_10000": round(gate_totals.false_per_10000, 4),
            "mutually_unseen": unseen,
            "mutually_unseen_recall": round(unseen_totals.recall, 6),
            "mutually_unseen_false_per_10000": round(unseen_totals.false_per_10000, 4),
            "mutually_unseen_scope": (
                "gate families the installed model did not count either; the only "
                "population where neither model can be answering from memory"
            ),
            "corpus_test": held, "recall": round(held_totals.recall, 6),
            "test_false_per_10000": round(held_totals.false_per_10000, 4),
            "lexical_stress_track": lexical,
            "baseline": {
                "model_version": legacy.version,
                "scope": "the installed model as users run it today, on the same rows",
                "gate": {"false_per_10000": round(gate_baseline.false_per_10000, 4),
                         "false_repeated_types": gate_baseline.false_repeated_types,
                         "false_attested_types": gate_baseline.false_attested_types,
                         "recall": round(gate_baseline.recall, 6)},
                "test": {"false_per_10000": round(test_baseline.false_per_10000, 4),
                         "false_repeated_types": test_baseline.false_repeated_types,
                         "false_attested_types": test_baseline.false_attested_types,
                         "recall": round(test_baseline.recall, 6)},
                "mutually_unseen": {
                    "false_per_10000": round(unseen_baseline.false_per_10000, 4),
                    "false_repeated_types": unseen_baseline.false_repeated_types,
                    "false_attested_types": unseen_baseline.false_attested_types,
                    "recall": round(unseen_baseline.recall, 6)},
            },
            "baseline_behind_new_guards": {
                "scope": ("the same weights with this release's two refusals applied, "
                          "reported so the weights and the guards can be told apart"),
                "gate": {"false_per_10000": round(gate_guarded.false_per_10000, 4),
                         "recall": round(gate_guarded.recall, 6)},
                "test": {"false_per_10000": round(test_guarded.false_per_10000, 4),
                         "recall": round(test_guarded.recall, 6)},
            },
            "promotion_passed": passed,
            "not_promoted_because": (
                None if passed else
                "no candidate built on this corpus has beaten the installed model "
                "while satisfying the rule; the weights separate 99.7% of wanted "
                "conversions at an ideal threshold, but the corpus cannot fix one: "
                "its negative tail is mislabelled wrong-layout text and unverifiable "
                "foreign strings, so the margin swallows the gain"
            ),
            "evidence_scope": (
                "synthetic wrong-layout renderings of frozen CC0 corpus tokens, shaped "
                "as the engine hands them over; the gate families contributed no "
                "character to any count; the lexical track is lowercased and therefore "
                "harsher than real typing"
            ),
        }
        REPORT.write_bytes(canonical(payload))
        print(json.dumps(payload, ensure_ascii=False, indent=2)[:4000])
        return 0 if passed else 1
    if arguments.command == "verify":
        replay = build(rows, settings)
        replay["minimum_length"] = minimum
        stored = cast(dict[str, object], json.loads(CANDIDATE.read_bytes()))
        replay["thresholds"] = stored["thresholds"]
        if canonical(replay) != CANDIDATE.read_bytes():
            raise ValueError("orthotactic candidate is not reproducible")
        print(json.dumps({"model_version": seal["model_version"], "reproducible": True}))
        return 0
    report = cast(dict[str, object], json.loads(REPORT.read_bytes()))
    if report.get("promotion_passed") is not True or report.get("candidate_sha256") != checksum(CANDIDATE):
        raise ValueError("refusing to promote a candidate without a passing report")
    ARTIFACT.write_bytes(CANDIDATE.read_bytes())
    print(json.dumps({"installed": seal["model_version"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
