#!/usr/bin/env python3
"""Train the key-space orthotactic model by counting, then seal before testing.

Counting is the whole algorithm, which is why the result replays byte for byte:
there is no optimiser, no shuffling and no floating-point accumulation order to
preserve beyond a fixed iteration over sorted keys.

The model answers "was this layout wrong?" without a dictionary. Two channels:

* a character model per language over the physical key sequence, so one sequence
  has exactly two readings and their ratio is a Bayes factor over one space;
* a counted case channel, because the strongest genuine-Russian sequences under
  that ratio are abbreviations, and an abbreviation is written in capitals.

Generation 3 changes the evidence, not the model form, and each change answers a
measurement (model/ortho_v1/README.md):

* The character counts come from the letters of every train token in both layouts
  (`ortho_corpus.word_core`) and from the frozen Onboard word lists, so a short
  command or an unusual Russian word is judged against a language, not against a
  few thousand phrases.
* Every group outside the test counts; there is no calibration split. The
  thresholds are fixed in the configuration, chosen on development draws that
  never saw the test groups, because the old rule - the worst calibration negative
  plus a margin - followed a single sample and moved by tens of nats between draws.
* The test measures the population the engine actually serves the model
  (`population`): the token as the engine cuts it, keys of the us/ru pair, a word
  the layout's dictionary does not know, and a replacement that is a word.

The feature model (artifact schema 2) adds `ortho_model.token_features` to the score
with a weight in nats per direction. The weights are fitted out of fold: the train
groups fall into `feature_folds` folds by the letter core of the token, the character
models are counted without one fold, the population of that fold is scored, and a
logistic regression over [character score, features] on all folds gives each
feature's weight relative to the character score. The final character models count
all of train. Every step iterates in a fixed order, so `verify` replays it byte for
byte.

Generation 6 counts a second channel for Russian, the one the model uses when a token
was typed in Russian (`ortho_model`, `source_models`): the train tokens and the Onboard
list as always, with the frozen web word forms (`ortho_web_lexicon`) merged into the
list. Slang on a borrowed stem and the name of a layout are then sequences the model has
seen, so it keeps them; the channel that asks whether Latin keys were meant as Russian
never sees the web forms, because each odd sequence they admit would argue for turning
English into Russian. The configuration names the scripts (`web_source_scripts`).

Generation 8 ties the weight of a rare feature to the feature that carries the same
evidence (`FEATURE_PARENTS`, `fitted_weights`): a feature seen on fewer than
`feature_min_types` training types of a direction is not fitted on its own - a weight from
a handful of examples follows the examples, and from none it is nothing - so `.dist`
keeps the evidence of `Я-то` whichever groups a split puts in train.

Generation 9 adds the feature `extra_key` (artifact schema 3, `ortho_model.one_key_extra`): the
reading in the target language is no word but becomes one without one of its letters - a stray
key (`фпривет`). The web forms of the source channel are the lowercase words and the lowercase
compounds with a hyphen (`рен-тв`), not the abbreviations in capitals. The snapshot is correctly
spelled, so the feature fit also sees synthetic positives (`synthetic_population`): a share of the
train words typed in the other layout with one stray key, drawn by `ortho_corpus.extra_key_variant`;
they touch nothing but the feature fit.

Generation 10 reads a stretch both ways (artifact schema 4, `stretch_runs` in the configuration,
`ortho_model.stretch_readings`): a run of at least that many identical keys is cut to two as before
and also read once, each reading is judged whole - characters and features - and the one that argues
less for the other layout is kept. The feature fit takes the features of the reading the model keeps
(`OrthoScore.reading`). Schema 4 also weighs `source_word` - the reading in the layout typed is a word
of its dictionary - which only a reading without its stretch can show; it is rare in train and takes
the weight of `parts_source`.

Generation 11 steadies the weights of rare features (`fitted_weights`): the configuration may name
features whose weight both directions share (`feature_shared`, fitted on both directions' samples at
once by `shared_weight`), features always tied to their parent (`feature_tied`, whatever their
support in one training) and a prior for the fit (`feature_prior` = "firth": Firth's correction in
`logistic`). The standardised L2 penalty barely touches a rare binary column, so without them such a
weight was close to an unpenalised estimate and followed whichever groups a split put in train.

Generation 12 builds artifact schema 5 (`hyphen_stretch` in the configuration): the reading without
its stretch also drops a hyphen between two copies of one key (`ortho_model.drop_hyphen_stretches`,
`Ти-ише`), compound evidence on both sides cancels (`В-к` / `D-r`), and the features `name_unknown`
(a capitalised token whose replacement is no word of its language; its weight is shared by both
directions), `target_unknown` and `source_plausibility` are counted. A feature the configuration names
in `feature_omitted` is not fitted and weighs nothing; the artifact carries the centres of
`source_plausibility` (`feature_centres`, the out-of-fold mean per direction) either way. The configuration
may also require of a compound in a script a half of some length (`compound_min_letters`, copied into the
artifact): the English dictionary knows every letter and more than half of all two-letter strings, so two
such halves are English words by chance. Columns the configuration lists as separated (`feature_separated`,
per direction: columns whose rarer label has fewer than `feature_min_types` types in the whole snapshot)
carry a log-F(1,1) prior (`logistic`): without it the penalised estimate of a column that only positives
carry is set by the hardest few of them.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import statistics
import sys
import tempfile
from collections import Counter
from collections.abc import Sequence
from pathlib import Path
from typing import Final, cast

sys.path.insert(0, str(Path(__file__).resolve().parent))

import ortho_corpus  # noqa: E402
import ortho_known  # noqa: E402
import ortho_prose_lexicon  # noqa: E402
import ortho_web_lexicon  # noqa: E402

from keyswitch.constants.file_formats import HEXADECIMAL_BASE, REPORT_JSON_INDENT, VERSION_HASH_CHARACTERS  # noqa: E402
from keyswitch.constants.model_protocol import SEALED_BEFORE_TEST, TEST, TRAIN  # noqa: E402
from keyswitch.constants.ortho import (  # noqa: E402
    ORTHO_EXTRA_KEY_ARTIFACT_SCHEMA_VERSION,
    ORTHO_PLAUSIBILITY_ARTIFACT_SCHEMA_VERSION,
    ORTHO_REPLACEMENT_ARTIFACT_SCHEMA_VERSION,
    ORTHO_STRETCH_ARTIFACT_SCHEMA_VERSION,
    ORTHO_V1_CONFIG_SCHEMA_VERSION,
    ORTHO_V1_EVIDENCE_SCHEMA_VERSION,
    ORTHO_V1_FIRTH_SCORE_CENTRE,
    ORTHO_V1_LOGISTIC_ITERATIONS,
    ORTHO_V1_LOGISTIC_LOGIT_BOUND,
    ORTHO_V1_LOGISTIC_TOLERANCE,
    ORTHO_V1_PRINTED_REPORT_MAX_CHARACTERS,
    ORTHO_V1_RECALL_DECIMALS,
    ORTHO_V1_REPORTED_SCORE_DECIMALS,
    ORTHO_V1_REPORTED_WORST_FALSE,
    ORTHO_V1_SEALED_THRESHOLD_DECIMALS,
    ORTHO_V1_SHAPE_PSEUDO_COUNT,
    ORTHO_V1_UNIGRAM_PSEUDO_COUNT,
)
from keyswitch.constants.training import DETERMINISTIC_CHOICE_HEX_DIGITS  # noqa: E402
from keyswitch.context_policy import word_shaped  # noqa: E402
from keyswitch.identifier_lexicon import RESOURCE_PATH as IDENTIFIERS, IdentifierLexicon  # noqa: E402
from keyswitch.ortho_model import (  # noqa: E402
    BOS, CONTINUOUS_FEATURE, EOS, FEATURES, SCHEMA_FEATURES, SCRIPTS, SHAPES, TARGET_FEATURE, KnownWord, OrthoEvidence,
    OrthoModel,
    collapse_runs, token_features,
)
from keyswitch.value_provenance import CONSTANTS_PACKAGE, pin_values  # noqa: E402

ROOT: Final[Path] = Path(__file__).resolve().parents[1]
DIRECTORY: Final[Path] = ROOT / "model/ortho_v1"
CONFIG: Final[Path] = DIRECTORY / "config.json"
CANDIDATE: Final[Path] = DIRECTORY / "candidate.json"
SEAL: Final[Path] = DIRECTORY / "seal.json"
REPORT: Final[Path] = DIRECTORY / "report.json"
ARTIFACT: Final[Path] = ROOT / "src/keyswitch/resources/models/ortho_v1.json"
# Names this generation's seal and draws its feature folds. Generation 1 was
# "keyswitch:ortho-v1:candidate1", generation 2 "keyswitch:ortho-v1:candidate2" (rejected before its
# test), generation 3 "keyswitch:ortho-v1:candidate3" (sealed, held in reserve, never tested),
# generation 4 "keyswitch:ortho-v1:candidate4" (rejected by its test), generation 5
# "keyswitch:ortho-v1:candidate5" (sealed, superseded before its test), generation 6
# "keyswitch:ortho-v1:candidate6" (rejected by its test), generations 7 and 8 "keyswitch:ortho-v1:candidate7"
# and "keyswitch:ortho-v1:candidate8" (never fitted: their development failed); generations 9 to 12
# "keyswitch:ortho-v1:candidate9" to "candidate16"; the candidate in the tree is that of "candidate15".
NAMESPACE: Final[str] = "keyswitch:ortho-v1:candidate15"
# The code that counts, seals and tests. The seal pins its bytes and, since its numbers moved into
# keyswitch.constants, the values it imports from there as well.
CODE: Final[tuple[str, ...]] = (
    "tools/train_ortho_model.py", "tools/ortho_corpus.py", "tools/ortho_known.py",
    "tools/ortho_web_lexicon.py", "tools/ortho_prose_lexicon.py", "tools/wikisource_text.py", "src/keyswitch/ortho_model.py",
)
Label = tuple[str, str, str]
# A rare feature shares the weight of the feature that carries the same evidence (`fitted_weights`):
# a dot and an English word (`.dist`), two dictionary halves around a hyphen (`Я-то`) and a word with
# one stray key (`фпривет`) all say that the reading in the target language is made of words of its
# dictionary up to punctuation or one extra key.
FEATURE_PARENTS: Final[dict[str, str]] = {"dot_word": "parts_target", "extra_key": "parts_target",
                                           "source_word": "parts_source"}
# A feature the configuration names in `feature_shared` weighs the same in both directions
# (`shared_weight`), and the seal records it so; `feature_prior` = FIRTH_PRIOR fits with Firth's correction.
SHARED_TIE: Final[str] = "shared by both directions"
FIRTH_PRIOR: Final[str] = "firth"


def canonical(value: object) -> bytes:
    return (json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(",", ":")) + "\n").encode()


def checksum(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def config() -> dict[str, object]:
    value: object = json.loads(CONFIG.read_bytes())
    if not isinstance(value, dict) or value.get("schema_version") != ORTHO_V1_CONFIG_SCHEMA_VERSION:
        raise ValueError("incompatible orthotactic configuration")
    return cast(dict[str, object], value)


def counts(rows: list[dict[str, object]], order: int, collapse: int | None = None) -> tuple[
        dict[str, Counter[str]], dict[str, Counter[str]], dict[str, Counter[str]]]:
    """Character n-gram counts per script, and the case shapes of prose and of abbreviations.

    Text of a language reaches a model in two shapes, one per layout it could have been typed in:
    the trailing comma of `French,` is a letter in Russian and a stripped symbol in English. Both
    are things a user produces while writing that language, so each distinct letter core of a train
    token (`ortho_corpus.word_core` in either layout) is counted, with the token's occurrences and
    case shapes. A word-list row adds the characters of its keys and no shape: a word list carries
    no case. This is exactly the counting the development draws measured (NOTES of stage O, 9.3).
    With `collapse` every run of one key longer than that is cut first, as the model cuts it when it
    scores (`ortho_model.collapse_runs`).
    """

    counted: list[tuple[str, str, dict[str, int], bool]] = []
    for row in rows:
        script, keys = str(row["script"]), str(row["keys"])
        shape_counts = cast(dict[str, int], row["shapes"])
        if row["split"] == TRAIN:
            counted.extend((script, sequence, shape_counts, True)
                           for sequence in sorted({ortho_corpus.word_core(keys, layout) for layout in SCRIPTS})
                           if sequence)
        elif row["split"] == ortho_corpus.LEXICON:
            counted.append((script, keys, shape_counts, False))
    if collapse is not None:
        counted = [(script, collapse_runs(sequence, collapse), shape_counts, cased)
                   for script, sequence, shape_counts, cased in counted]
    grams: dict[str, Counter[str]] = {script: Counter() for script in SCRIPTS}
    shapes: dict[str, Counter[str]] = {script: Counter() for script in SCRIPTS}
    acronyms: dict[str, Counter[str]] = {script: Counter() for script in SCRIPTS}
    upper_keys: dict[str, set[str]] = {script: set() for script in SCRIPTS}
    for script, sequence, shape_counts, cased in counted:
        if cased and shape_counts.get("upper"):
            upper_keys[script].add(sequence)
    for script, sequence, shape_counts, cased in counted:
        weight = sum(shape_counts.values())
        padded = BOS * (order - 1) + sequence + EOS
        for index in range(order - 1, len(padded)):
            for size in range(1, order + 1):
                grams[script][padded[index - size + 1:index + 1]] += weight
        if not cased:
            continue
        target = acronyms if sequence in upper_keys[script] else shapes
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
                probability = ((count + ORTHO_V1_UNIGRAM_PSEUDO_COUNT)
                               / (unigram_total + ORTHO_V1_UNIGRAM_PSEUDO_COUNT * alphabet))
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
        total = sum(counter.values()) + ORTHO_V1_SHAPE_PSEUDO_COUNT * len(SHAPES)
        result[script] = {
            shape: quantise(math.log((counter.get(shape, 0) + ORTHO_V1_SHAPE_PSEUDO_COUNT) / total), scale)
            for shape in SHAPES
        }
    return result


def channel_payload(gram_counts: Counter[str], settings: dict[str, object]) -> dict[str, object]:
    """One fitted character channel, quantised as the artifact stores it."""

    order = cast(int, settings["order"])
    scale = cast(int, settings["scale"])
    alphabet = len({gram for gram in gram_counts if len(gram) == 1})
    logprob, backoff, uniform = fit_channel(gram_counts, order, float(cast(float, settings["discount"])), alphabet)
    names = sorted(logprob)
    return {
        "grams": "\n".join(names),
        "logprob": [quantise(logprob[name], scale) for name in names],
        "backoff": [[name, quantise(backoff[name], scale)] for name in sorted(backoff)],
        "uniform": quantise(uniform, scale),
    }


def source_rows(rows: list[dict[str, object]], web: Sequence[dict[str, object]]) -> list[dict[str, object]]:
    """What the source channel of a web script counts: the train tokens, and that script's word lists
    merged with the web forms, a key weighing the larger of its two weights."""

    scripts = {str(row["script"]) for row in web}
    lists: dict[tuple[str, str], int] = {}
    listed = [row for row in rows if row["split"] == ortho_corpus.LEXICON and row["script"] in scripts]
    for row in [*listed, *web]:
        key = (str(row["script"]), str(row["keys"]))
        lists[key] = max(lists.get(key, 0), sum(cast(dict[str, int], row["shapes"]).values()))
    return [row for row in rows if row["split"] == TRAIN] + [
        {"split": ortho_corpus.LEXICON, "script": script, "keys": keys, "shapes": {"lower": weight}}
        for (script, keys), weight in sorted(lists.items())
    ]


def char_payload(rows: list[dict[str, object]], settings: dict[str, object],
                 web: Sequence[dict[str, object]] = ()) -> dict[str, object]:
    """The counted weights: character models and case channels, no features, no thresholds.

    With `web` rows (the frozen web forms of a script) the artifact also carries that script's
    source channel (`ortho_model`, `source_models`): the train tokens and the word lists as always,
    with the web forms merged into the lists (`source_rows`). The other channels never see them.
    """

    order = cast(int, settings["order"])
    scale = cast(int, settings["scale"])
    collapse = cast("int | None", settings.get("collapse_runs"))
    gram_counts, prose, acronym = counts(rows, order, collapse)
    payload: dict[str, object] = {
        "schema_version": 1, "order": order, "scale": scale,
        "models": {script: channel_payload(gram_counts[script], settings) for script in SCRIPTS},
        "prose_shape": shape_channel(prose, scale),
        "acronym_shape": shape_channel(acronym, scale),
    }
    if web:
        sourced, _prose, _acronym = counts(source_rows(rows, web), order, collapse)
        payload["source_models"] = {script: channel_payload(sourced[script], settings)
                                    for script in sorted({str(row["script"]) for row in web})}
    return payload


def scope(settings: dict[str, object]) -> dict[str, object]:
    """The artifact's run collapse, the case shapes it does not score and, with a schema 4 artifact, the
    runs it also reads once (`stretch_runs`), from the configuration; the schema follows. A schema 5
    artifact also carries the configuration's `compound_min_letters`; `target_plausibility` makes it schema 6."""

    result: dict[str, object] = {"collapse_runs": settings["collapse_runs"], "unscored_shapes": settings["unscored_shapes"],
                                 "schema_version": ORTHO_EXTRA_KEY_ARTIFACT_SCHEMA_VERSION}
    if settings.get("stretch_runs") is not None:
        schema = ORTHO_REPLACEMENT_ARTIFACT_SCHEMA_VERSION if settings.get("hyphen_stretch") else ORTHO_STRETCH_ARTIFACT_SCHEMA_VERSION
        result.update({"stretch_runs": settings["stretch_runs"], "schema_version": schema})
    if settings.get("target_plausibility"):
        if result["schema_version"] != ORTHO_REPLACEMENT_ARTIFACT_SCHEMA_VERSION:
            raise ValueError("the plausibility of the replacement needs the schema 5 readings")
        result["schema_version"] = ORTHO_PLAUSIBILITY_ARTIFACT_SCHEMA_VERSION
    elif settings.get("feature_gated"):
        raise ValueError("only a schema 6 artifact gates features")
    if settings.get("compound_min_letters"):
        if cast(int, result["schema_version"]) < ORTHO_REPLACEMENT_ARTIFACT_SCHEMA_VERSION:
            raise ValueError("only a schema 5 artifact requires a length of a compound's half")
        result["compound_min_letters"] = settings["compound_min_letters"]
    return result


def char_model(payload: dict[str, object], settings: dict[str, object]) -> OrthoModel:
    """Load counted weights as a scoring model: its scope, no feature weight, thresholds unused."""

    scoped = scope(settings)
    names = SCHEMA_FEATURES[cast(int, scoped["schema_version"])]
    if cast(int, scoped["schema_version"]) >= ORTHO_REPLACEMENT_ARTIFACT_SCHEMA_VERSION:
        scoped["feature_centres"] = {script: 0 for script in SCRIPTS}
    if scoped["schema_version"] == ORTHO_PLAUSIBILITY_ARTIFACT_SCHEMA_VERSION:
        scoped["target_centres"] = {script: 0 for script in SCRIPTS}
    scored = {**payload, **scoped,
              "features": {script: {name: 0 for name in names} for script in SCRIPTS},
              "version": "ortho-v1-" + "0" * VERSION_HASH_CHARACTERS,
              "minimum_length": settings["minimum_length"], "thresholds": {script: 0 for script in SCRIPTS}}
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "counts.json"
        path.write_bytes(canonical(scored))
        return OrthoModel.load(path)


def fold_of(row: dict[str, object], folds: int) -> int:
    """The feature fold of a train row: its letter core decides, so a word and its punctuated forms stay together."""

    script, keys = str(row["script"]), str(row["keys"])
    core = ortho_corpus.word_core(keys, script)
    digest = hashlib.sha256(f"{NAMESPACE}:fold:{script}:{core}".encode()).hexdigest()
    return int(digest[:DETERMINISTIC_CHOICE_HEX_DIGITS], HEXADECIMAL_BASE) % folds


def solve(matrix: list[list[float]], vector: list[float]) -> list[float]:
    """Gaussian elimination with partial pivoting, in a fixed order."""

    size = len(vector)
    rows = [row[:] + [vector[index]] for index, row in enumerate(matrix)]
    for column in range(size):
        pivot = max(range(column, size), key=lambda index: abs(rows[index][column]))
        rows[column], rows[pivot] = rows[pivot], rows[column]
        for index in range(size):
            if index != column and rows[column][column]:
                factor = rows[index][column] / rows[column][column]
                for position in range(column, size + 1):
                    rows[index][position] -= factor * rows[column][position]
    return [rows[index][size] / rows[index][index] for index in range(size)]


def probability_of(logit: float) -> float:
    bounded = max(-ORTHO_V1_LOGISTIC_LOGIT_BOUND, min(ORTHO_V1_LOGISTIC_LOGIT_BOUND, logit))
    return 1.0 / (1.0 + math.exp(-bounded))


def logistic(samples: list[list[float]], labels: list[int], l2: float, firth: bool = False,
             separated: Sequence[int] = ()) -> list[float]:
    """Coefficients (on the raw scale) of a logistic regression with an L2 penalty, by Newton steps.

    The columns are standardised first, so one penalty means the same for every column; the
    intercept is not penalised. With `firth` the score carries Firth's correction - the penalty of
    Jeffreys' prior, half the log-determinant of the information (Firth 1993, Biometrika 80:27-38) -
    which keeps an estimate finite and shrinks it toward zero where a rare column nearly separates the
    labels (Heinze & Schemper 2002, Stat Med 21:2409-2419): each sample adds its leverage times
    (probability - 1/2) to the gradient. `separated` names columns (indices into a sample) whose raw
    coefficient b carries a log-F(1,1) prior besides (Greenland & Mansournia 2015, Stat Med 34:3133-3143):
    the penalty log(1 + e^b) - b/2, the likelihood of one pseudo-record of that column alone with half a
    success in one trial - centred on zero whatever the other columns do, and finite where only one label
    carries the column. (Jeffreys' prior restricted to such a column is not: its mode is where the column's
    rows are least certain, so a column only positives carry, whose rows the character score already
    predicts, was pulled below zero.)
    """

    width = len(samples[0])
    mean = [statistics.fmean(sample[column] for sample in samples) for column in range(width)]
    spread = [statistics.pstdev([sample[column] for sample in samples]) or 1.0 for column in range(width)]
    standard = [[1.0] + [(sample[column] - mean[column]) / spread[column] for column in range(width)]
                for sample in samples]
    weights = [0.0] * (width + 1)
    for _iteration in range(ORTHO_V1_LOGISTIC_ITERATIONS):
        gradient = [l2 * weight if index else 0.0 for index, weight in enumerate(weights)]
        hessian = [[l2 if row == column and row else 0.0 for column in range(width + 1)] for row in range(width + 1)]
        probabilities = [probability_of(sum(w * x for w, x in zip(weights, row, strict=True))) for row in standard]
        for row, label, probability in zip(standard, labels, probabilities, strict=True):
            residual, curvature = probability - label, probability * (1.0 - probability)
            for first in range(width + 1):
                gradient[first] += residual * row[first]
                for second in range(first, width + 1):
                    hessian[first][second] += curvature * row[first] * row[second]
        for first in range(width + 1):
            for second in range(first):
                hessian[first][second] = hessian[second][first]
        for column in separated:
            prior = probability_of(weights[column + 1] / spread[column])
            gradient[column + 1] += (prior - ORTHO_V1_FIRTH_SCORE_CENTRE) / spread[column]
            hessian[column + 1][column + 1] += prior * (1.0 - prior) / (spread[column] * spread[column])
        if firth:
            inverse = [solve(hessian, [float(index == column) for index in range(width + 1)]) for column in range(width + 1)]
            for row, probability in zip(standard, probabilities, strict=True):
                spread_row = [sum(inverse[first][second] * row[second] for second in range(width + 1))
                              for first in range(width + 1)]
                leverage = probability * (1.0 - probability) * sum(x * y for x, y in zip(row, spread_row, strict=True))
                for first in range(width + 1):
                    gradient[first] += leverage * (probability - ORTHO_V1_FIRTH_SCORE_CENTRE) * row[first]
        step = solve(hessian, gradient)
        weights = [weight - change for weight, change in zip(weights, step, strict=True)]
        if max(abs(change) for change in step) < ORTHO_V1_LOGISTIC_TOLERANCE:
            break
    return [weights[column + 1] / spread[column] for column in range(width)]


def shared_weight(parts: dict[str, list[tuple[float, float, int]]], firth: bool = False,
                  prior: dict[str, float] | None = None) -> float:
    """One weight in nats for a feature both directions share, from both directions' samples at once.

    `parts` holds, per direction, (linear predictor without intercept, the feature's column scaled by the
    direction's coefficient of the character score, label) for every sample, as the direction's own fit
    left them. Newton's method finds the maximum of the joint likelihood over each direction's intercept
    and the one shared weight - with `firth`, of the likelihood with Firth's correction, as `logistic`
    does; a weight nothing supports is zero. With `prior` ({direction: the scale of its column, its
    coefficient of the character score}) the weight carries a log-F(1,1) prior, the one pseudo-record of
    `logistic` split evenly between the directions: half a record of each, with the column at its scale.
    """

    directions = sorted(parts)
    size = len(directions) + 1
    estimate = [0.0] * size
    for _iteration in range(ORTHO_V1_LOGISTIC_ITERATIONS):
        gradient = [0.0] * size
        hessian = [[0.0] * size for _row in range(size)]
        seen: list[tuple[list[float], float]] = []
        for position, direction in enumerate(directions):
            for predictor, column, label in parts[direction]:
                row = [float(index == position) for index in range(size - 1)] + [column]
                probability = probability_of(estimate[position] + predictor + estimate[-1] * column)
                residual, curvature = probability - label, probability * (1.0 - probability)
                for first in range(size):
                    gradient[first] += residual * row[first]
                    for second in range(size):
                        hessian[first][second] += curvature * row[first] * row[second]
                seen.append((row, probability))
        if not hessian[-1][-1]:
            return 0.0
        for scale in (prior or {}).values():
            share = 1.0 / len(cast(dict[str, float], prior))
            pseudo = probability_of(estimate[-1] * scale)
            gradient[-1] += share * (pseudo - ORTHO_V1_FIRTH_SCORE_CENTRE) * scale
            hessian[-1][-1] += share * pseudo * (1.0 - pseudo) * scale * scale
        if firth:
            inverse = [solve(hessian, [float(index == column) for index in range(size)]) for column in range(size)]
            for row, probability in seen:
                leverage = probability * (1.0 - probability) * sum(
                    row[first] * inverse[first][second] * row[second] for first in range(size) for second in range(size))
                for first in range(size):
                    gradient[first] += leverage * (probability - ORTHO_V1_FIRTH_SCORE_CENTRE) * row[first]
        step = solve(hessian, gradient)
        estimate = [value - change for value, change in zip(estimate, step, strict=True)]
        if max(abs(change) for change in step) < ORTHO_V1_LOGISTIC_TOLERANCE:
            break
    return estimate[-1]


def fitted_weights(samples: dict[str, list[list[float]]], labels: dict[str, list[int]],
                   settings: dict[str, object], names: tuple[str, ...] = FEATURES,
                   ties: dict[str, dict[str, str]] | None = None) -> dict[str, dict[str, float]]:
    """Each feature's weight in nats per direction from the out-of-fold samples [char score, *names].

    A feature that fires on fewer than `feature_min_types` samples of a direction and has a parent
    (`FEATURE_PARENTS`) shares the parent's weight: the regression sees one column, the sum of the
    two indicators - the runtime adds both weights, so the two agree even where both fire - and both
    get the fitted weight. A feature the configuration names in `feature_tied` shares it always,
    whatever its support: a tie that follows the support of one training flips between trainings.
    A feature named in `feature_omitted` is not fitted and weighs nothing. A column the configuration
    lists for a direction in `feature_separated` (a fitted feature, with whatever is tied to it) carries a
    log-F(1,1) prior (`logistic`; a shared feature listed for both directions, in `shared_weight`). A feature
    the configuration names in `feature_copied` is left out of the
    regression and takes its parent's weight as fitted without it: the corpus holds no example of
    what it stands for, so the regression would learn only its coincidences (`extra_key`: a stray key
    never occurs in the snapshot's correctly spelled text). `ties`, when given, receives
    {direction: {feature: parent}} for both kinds.
    """

    minimum = int(cast(int, settings.get("feature_min_types", 0)))
    copied_names = set(cast(list[str], settings.get("feature_copied", [])))
    always_tied = set(cast(list[str], settings.get("feature_tied", [])))
    omitted = set(cast(list[str], settings.get("feature_omitted", [])))
    shared = [name for name in cast(list[str], settings.get("feature_shared", [])) if name in names and name not in omitted]
    firth = settings.get("feature_prior") == FIRTH_PRIOR
    raw_separated = settings.get("feature_separated", {})
    if not isinstance(raw_separated, dict) or not set(raw_separated) <= set(SCRIPTS):
        raise ValueError("feature_separated must list columns per direction")
    separated = cast(dict[str, list[str]], raw_separated)
    result: dict[str, dict[str, float]] = {}
    scales: dict[str, float] = {}
    parts: dict[str, dict[str, list[tuple[float, float, int]]]] = {name: {} for name in shared}
    for direction in SCRIPTS:
        rows = samples[direction]
        copied = {name: parent for name, parent in FEATURE_PARENTS.items()
                  if name in copied_names and name in names and parent in names and name not in shared}
        tied = {rare: parent for rare, parent in FEATURE_PARENTS.items()
                if rare in names and parent in names and rare not in copied and rare not in shared
                and (rare in always_tied or sum(1 for row in rows if row[1 + names.index(rare)]) < minimum)}
        kept = [name for name in names if name not in tied and name not in copied and name not in shared and name not in omitted]
        columns = [[names.index(name)] + [names.index(rare) for rare, parent in tied.items() if parent == name]
                   for name in kept]
        matrix = [[row[0]] + [sum(row[1 + index] for index in column) for column in columns] for row in rows]
        listed = [name for name in separated.get(direction, []) if name not in shared]
        if not set(listed) <= set(kept):
            raise ValueError("a separated column must be a feature fitted on its own column")
        coefficients = logistic(matrix, labels[direction], float(cast(float, settings["feature_l2"])), firth,
                                [1 + kept.index(name) for name in listed])
        scales[direction] = coefficients[0]
        if coefficients[0] <= 0:
            raise ValueError("the character score does not predict a wrong layout; refusing the feature fit")
        fitted = {name: coefficients[index + 1] / coefficients[0] for index, name in enumerate(kept)}
        fitted.update({rare: fitted[parent] for rare, parent in {**tied, **copied}.items()})
        result[direction] = {name: fitted.get(name, 0.0) for name in FEATURES}
        for name in shared:
            parts[name][direction] = [(sum(coefficient * value for coefficient, value in zip(coefficients, entry, strict=True)),
                                       coefficients[0] * row[1 + names.index(name)], label)
                                      for entry, row, label in zip(matrix, rows, labels[direction], strict=True)]
        if ties is not None:
            ties[direction] = dict(sorted({**tied, **copied, **{name: SHARED_TIE for name in shared}}.items()))
    for name in shared:
        listed_in = {direction for direction in SCRIPTS if name in separated.get(direction, [])}
        if listed_in not in (set(), set(SCRIPTS)):
            raise ValueError("a shared feature is separated in both directions or in none")
        weight = shared_weight(parts[name], firth, scales if listed_in else None)
        for direction in SCRIPTS:
            result[direction][name] = weight
    return result


def feature_weights(rows: list[dict[str, object]], settings: dict[str, object],
                    known: dict[str, frozenset[str]], identifier: IdentifierLexicon,
                    names: tuple[str, ...] = FEATURES,
                    web: Sequence[dict[str, object]] = (),
                    ties: dict[str, dict[str, str]] | None = None,
                    centres: dict[str, float] | None = None,
                    target_centres: dict[str, float] | None = None) -> dict[str, dict[str, float]]:
    """Each feature's weight in nats per direction, fitted out of fold on train (see the module doc).

    `names` limits the fit to some of the features (the others weigh nothing); the development
    draws compare subsets this way, the trainer fits them all. Rare features are tied to their
    parent (`fitted_weights`). The continuous features, `source_plausibility` and `target_plausibility`,
    enter the fit as measured; `centres` and `target_centres`, when given, receive their means per
    direction, from which the artifact measures them. A feature the configuration names in `feature_gated`
    counts only where the source plausibility lies below its centre, as the artifact scores it.
    """

    folds = cast(int, settings["feature_folds"])
    minimum = cast(int, settings["minimum_length"])
    lookup: KnownWord = lambda script, word: word in known[script]  # noqa: E731
    samples: dict[str, list[list[float]]] = {script: [] for script in SCRIPTS}
    labels: dict[str, list[int]] = {script: [] for script in SCRIPTS}
    train = [row for row in rows if row["split"] == TRAIN]
    lexicon = [row for row in rows if row["split"] == ortho_corpus.LEXICON]
    for fold in range(folds):
        model = char_model(char_payload([row for row in train if fold_of(row, folds) != fold] + lexicon,
                                        settings, web), settings)
        held = [row for row in train if fold_of(row, folds) == fold]
        real = population(held, TRAIN, known, minimum)
        synthetic = {label: negative for label, negative in synthetic_population(held, known, minimum).items()
                     if label not in real}
        for (direction, keys, shape), negative in [*sorted(real.items()), *sorted(synthetic.items())]:
            scored = model.score(OrthoEvidence(keys, shape, direction))
            if not scored.supported:
                continue     # outside the model's scope: never licensed, so nothing to weigh
            # The features of the reading the model keeps (schema 4 reads a stretch both ways).
            values: dict[str, float] = dict(token_features(scored.reading, direction, identifier=identifier.contains,
                                                           known=lookup, shape=shape, symmetric=model.hyphens,
                                                           compound_min_letters=model.compound_min_letters))
            values[CONTINUOUS_FEATURE] = scored.source_logprob / (len(scored.reading) + 1)
            values[TARGET_FEATURE] = scored.target_logprob / (len(scored.reading) + 1)
            samples[direction].append([scored.total] + [float(values[name]) for name in names])
            labels[direction].append(0 if negative else 1)
    for feature, receiver in ((CONTINUOUS_FEATURE, centres), (TARGET_FEATURE, target_centres)):
        if feature not in names:
            continue
        column = 1 + names.index(feature)
        measured = {direction: statistics.fmean(sample[column] for sample in drawn) if drawn else 0.0
                    for direction, drawn in samples.items()}
        if receiver is not None:
            receiver.update(measured)
        if feature == CONTINUOUS_FEATURE:
            # A gated feature counts only for a reading less plausible than the centre, as the artifact scores it.
            for name in cast(list[str], settings.get("feature_gated", [])):
                gated = 1 + names.index(name)
                for direction, drawn in samples.items():
                    for sample in drawn:
                        if sample[column] >= measured[direction]:
                            sample[gated] = 0.0
    return fitted_weights(samples, labels, settings, names, ties)


def build(rows: list[dict[str, object]], settings: dict[str, object], known: dict[str, frozenset[str]],
          identifier: IdentifierLexicon, web: Sequence[dict[str, object]] = (),
          ties: dict[str, dict[str, str]] | None = None) -> dict[str, object]:
    """The complete artifact: counted weights, feature weights, then the fixed thresholds."""

    scale = cast(int, settings["scale"])
    payload = {**char_payload(rows, settings, web), **scope(settings)}
    names = SCHEMA_FEATURES[cast(int, payload["schema_version"])]
    centres: dict[str, float] = {}
    target_centres: dict[str, float] = {}
    payload["features"] = {
        direction: {name: quantise(weights[name], scale) for name in names}
        for direction, weights in feature_weights(rows, settings, known, identifier, names, web=web, ties=ties,
                                                  centres=centres, target_centres=target_centres).items()
    }
    if cast(int, payload["schema_version"]) >= ORTHO_REPLACEMENT_ARTIFACT_SCHEMA_VERSION:
        payload["feature_centres"] = {script: quantise(centres[script], scale) for script in SCRIPTS}
    if payload["schema_version"] == ORTHO_PLAUSIBILITY_ARTIFACT_SCHEMA_VERSION:
        payload["target_centres"] = {script: quantise(target_centres[script], scale) for script in SCRIPTS}
        if settings.get("feature_gated"):
            payload["gated_features"] = sorted(cast(list[str], settings["feature_gated"]))
    digest = hashlib.sha256(canonical(payload)).hexdigest()
    payload["version"] = f"ortho-v1-{digest[:VERSION_HASH_CHARACTERS]}"
    payload["weights_sha256"] = digest
    # The thresholds ship inside the artifact: the runtime has no other file to read them from.
    # They are configuration, not fitted, so the model identity above does not depend on them.
    payload["minimum_length"] = settings["minimum_length"]
    thresholds = cast(dict[str, float], settings["thresholds_nats"])
    payload["thresholds"] = {script: quantise(float(thresholds[script]), scale) for script in SCRIPTS}
    return payload


def population(rows: list[dict[str, object]], split: str, known: dict[str, frozenset[str]],
               minimum_length: int) -> dict[Label, bool]:
    """(direction, served keys, shape) -> is it a negative, for what the engine serves the model.

    Each rule repeats the runtime:
    * `ortho_corpus.served` - how `KeySwitchEngine` cuts the keys into the word it analyses;
    * keys of the us/ru pair only - `ContextPolicy._orthotactic` asks about those two groups;
    * shorter than `minimum_length` - refused by `_orthotactic`;
    * not `word_shaped` in the layout being typed - refused by `_orthotactic`;
    * known to that layout's dictionary - refused by `_orthotactic` (`source_score.known`), read
      from the frozen verdict of `tools/ortho_known.py`;
    * a word whose other reading is not a word - the conversion is withdrawn by
      `ContextPolicy._spelling_a_word`, so it can neither harm nor help.
    A sequence the snapshot shows both as a negative and as a positive for one direction is a
    negative: the label that protects text wins, and no label is dropped.
    """

    labels: dict[Label, bool] = {}
    for row in rows:
        if row["split"] != split:
            continue
        script, keys = str(row["script"]), str(row["keys"])
        shapes = sorted(cast(dict[str, int], row["shapes"]))
        for direction in SCRIPTS:
            for served in ortho_corpus.served(keys, direction):
                if len(served) < minimum_length or not set(served) <= ortho_corpus.KEYS:
                    continue
                original = ortho_corpus.visible(served, direction)
                if not word_shaped(original) or original in known[direction]:
                    continue
                replacement = ortho_corpus.visible(served, ortho_corpus.other(direction)).removeprefix(".")
                if original.isalpha() and not (replacement.isalpha() or word_shaped(replacement)):
                    continue
                negative = script == direction
                for shape in shapes:
                    label = (direction, served, shape)
                    labels[label] = labels.get(label, False) or negative
    return labels


def synthetic_population(rows: list[dict[str, object]], known: dict[str, frozenset[str]],
                         minimum_length: int) -> dict[Label, bool]:
    """Positive types for the feature fit only: train words typed in the other layout with a stray key.

    `ortho_corpus.extra_key_variant` draws the variant; the engine's rules of `population` apply to
    it as typed in the other layout, and every case shape of the source type is kept. A variant never
    enters the character counts, a held-out part or the test population.
    """

    labels: dict[Label, bool] = {}
    for row in rows:
        if row["split"] != TRAIN:
            continue
        script = str(row["script"])
        synthetic = ortho_corpus.extra_key_variant(script, str(row["keys"]))
        if synthetic is None:
            continue
        typed = {**row, "script": script, "keys": synthetic}
        for (direction, served, shape), _negative in population([typed], TRAIN, known, minimum_length).items():
            if direction != script:
                labels[(direction, served, shape)] = False
    return labels


def outcome(model: OrthoModel, labels: dict[Label, bool]) -> dict[str, object]:
    """False conversions and recall, counted by type."""

    counted: Counter[str] = Counter()
    worst: dict[str, list[tuple[float, str, str]]] = {script: [] for script in SCRIPTS}
    for (direction, keys, shape), negative in sorted(labels.items()):
        scored = model.score(OrthoEvidence(keys, shape, direction))
        total = scored.total
        converts = scored.supported and total > model.thresholds[direction]
        if negative:
            # The user was in this token's own layout: converting corrupts text.
            counted[f"{direction}:negative_types"] += 1
            if converts:
                counted[f"{direction}:false_types"] += 1
                worst[direction].append((total, keys, shape))
        else:
            # The token belongs to the other language: the layout was wrong.
            counted[f"{direction}:positive_types"] += 1
            if converts:
                counted[f"{direction}:recalled_types"] += 1
    report: dict[str, object] = {"counts": dict(sorted(counted.items()))}
    report["worst_false"] = {
        script: [{"score": round(value, ORTHO_V1_REPORTED_SCORE_DECIMALS), "keys": keys, "shape": shape}
                 for value, keys, shape in sorted(items, reverse=True)[:ORTHO_V1_REPORTED_WORST_FALSE]]
        for script, items in worst.items()
    }
    return report


def web_rows(settings: dict[str, object]) -> list[dict[str, object]]:
    """Word-list rows of the frozen web forms (`web_source_scripts`) and of the frozen public-domain prose
    forms (`prose_source_scripts`), for the scripts whose source channel counts them; `source_rows` merges a
    key's rows to its larger weight."""

    rows: list[dict[str, object]] = []
    for key, tool, name in (("web_source_scripts", ortho_web_lexicon, "web"), ("prose_source_scripts", ortho_prose_lexicon, "prose")):
        scripts = cast(list[str], settings.get(key, []))
        if not scripts:
            continue
        if scripts != ["ru"]:
            raise ValueError(f"the {name} lexicon holds Russian forms only")
        recorded = cast(dict[str, object], json.loads(tool.RECEIPT.read_bytes()))
        if recorded.get("forms_sha256") != checksum(tool.FORMS):
            raise ValueError(f"{name} lexicon does not match its receipt")
        rows.extend({"split": ortho_corpus.LEXICON, "script": "ru", "keys": ortho_corpus.to_keys(form),
                     "shapes": {"lower": weight}} for form, weight in sorted(tool.load(tool.FORMS).items()))
    return rows


def provenance() -> dict[str, str]:
    code = tuple(ROOT / name for name in CODE)
    paths = (*code, ortho_corpus.TOKENS, ortho_corpus.RECEIPT, ortho_known.EVIDENCE,
             ortho_known.RECEIPT, ortho_web_lexicon.FORMS, ortho_web_lexicon.RECEIPT, ortho_prose_lexicon.FORMS,
             ortho_prose_lexicon.RECEIPT, CONFIG, IDENTIFIERS)
    pins = {path.relative_to(ROOT).as_posix(): checksum(path) for path in paths}
    pins[CONSTANTS_PACKAGE] = pin_values(code, source_root=ROOT / "src").sha256
    return pins


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("fit", "test", "verify", "promote"))
    arguments = parser.parse_args(argv)
    settings = config()
    rows = ortho_corpus.load_tokens()
    if checksum(ortho_corpus.TOKENS) != cast(str, cast(dict[str, object], json.loads(
            ortho_corpus.RECEIPT.read_bytes()))["tokens_sha256"]):
        raise ValueError("key-space corpus does not match its receipt")
    if checksum(ortho_known.EVIDENCE) != cast(str, cast(dict[str, object], json.loads(
            ortho_known.RECEIPT.read_bytes()))["evidence_sha256"]):
        raise ValueError("dictionary evidence does not match its receipt")
    known = ortho_known.load()
    identifier = IdentifierLexicon.load(IDENTIFIERS)
    frozen: KnownWord = lambda script, word: word in known[script]  # noqa: E731
    web = web_rows(settings)
    if arguments.command == "fit":
        if CANDIDATE.exists() or SEAL.exists():
            raise ValueError("candidate already exists; do not refit over sealed evidence")
        ties: dict[str, dict[str, str]] = {}
        payload = build(rows, settings, known, identifier, web, ties)
        CANDIDATE.write_bytes(canonical(payload))
        model = OrthoModel.load(CANDIDATE, identifier=identifier.contains, known=frozen)
        seal = {
            "schema_version": ORTHO_V1_EVIDENCE_SCHEMA_VERSION, "stage": SEALED_BEFORE_TEST, "namespace": NAMESPACE,
            "model_version": payload["version"], "candidate_sha256": checksum(CANDIDATE),
            "thresholds": {script: round(value, ORTHO_V1_SEALED_THRESHOLD_DECIMALS)
                           for script, value in model.thresholds.items()},
            "feature_weights_nats": {script: {name: round(weight, ORTHO_V1_SEALED_THRESHOLD_DECIMALS)
                                              for name, weight in weights.items()}
                                     for script, weights in model.features.items()},
            "feature_ties": ties,
            "config": settings, "provenance": provenance(),
        }
        SEAL.write_bytes(canonical(seal))
        print(json.dumps(seal, ensure_ascii=False, indent=REPORT_JSON_INDENT))
        return 0
    seal = cast(dict[str, object], json.loads(SEAL.read_bytes()))
    if seal.get("provenance") != provenance() or seal.get("candidate_sha256") != checksum(CANDIDATE):
        raise ValueError("candidate or provenance changed after the seal")
    model = OrthoModel.load(CANDIDATE, identifier=identifier.contains, known=frozen)
    if arguments.command == "test":
        if REPORT.exists():
            raise ValueError("test already observed; re-scoring it would not be an independent test")
        minimum = cast(int, settings["minimum_length"])
        corpus = outcome(model, population(rows, TEST, known, minimum))
        promotion = cast(dict[str, object], settings["promotion"])
        counts_map = cast(dict[str, int], corpus["counts"])

        def summed(name: str) -> int:
            return sum(counts_map.get(f"{script}:{name}", 0) for script in SCRIPTS)

        negatives, false_total, positives = summed("negative_types"), summed("false_types"), summed("positive_types")
        recall = summed("recalled_types") / max(1, positives)
        passed = (negatives >= cast(int, promotion["minimum_test_negatives"])
                  and false_total <= cast(int, promotion["maximum_test_false_conversions"])
                  and recall >= float(cast(float, promotion["minimum_recall"])))
        payload = {
            "schema_version": ORTHO_V1_EVIDENCE_SCHEMA_VERSION, "model_version": seal["model_version"],
            "seal_sha256": checksum(SEAL), "candidate_sha256": checksum(CANDIDATE),
            "thresholds": seal["thresholds"], "corpus_test": corpus,
            "recall": round(recall, ORTHO_V1_RECALL_DECIMALS),
            "promotion_passed": passed,
            "evidence_scope": (
                "synthetic wrong-layout renderings of frozen CC0 corpus tokens, cut as the engine cuts "
                "them and limited to what it serves the model: keys of the us/ru pair, word-shaped, "
                "unknown to the layout's dictionary, with a word as the replacement; one type is "
                "(direction, keys, case shape)"
            ),
        }
        REPORT.write_bytes(canonical(payload))
        print(json.dumps(payload, ensure_ascii=False, indent=REPORT_JSON_INDENT)[:ORTHO_V1_PRINTED_REPORT_MAX_CHARACTERS])
        return 0 if passed else 1
    if arguments.command == "verify":
        if canonical(build(rows, settings, known, identifier, web)) != CANDIDATE.read_bytes():
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
