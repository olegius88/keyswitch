"""Select an epoch on development actions; calibration sets the serving threshold."""

from __future__ import annotations

import math
from array import array
from bisect import bisect_left
from collections.abc import Mapping, Sequence
from dataclasses import dataclass


@dataclass(frozen=True)
class EpochSelection:
    threshold: float
    minimum_recall: float
    pooled_recall: float
    loss: float
    by_profile: dict[str, dict[str, int | float]]

    @property
    def rank(self) -> tuple[float, float, float, float]:
        return self.minimum_recall, self.pooled_recall, -self.loss, -self.threshold


def assess_epoch(
    predictions: Mapping[str, tuple[array[float], array[int]]], *,
    thresholds: Sequence[float], loss: float, maximum_false: int = 0,
) -> EpochSelection | None:
    """Rank zero-budget development frontiers after the runtime support checks.

    The threshold here is a development operating point, not an exported
    decision. After choosing weights, the trainer calibrates their threshold
    on its separate calibration split. No calibration or test outcome belongs
    in this selection. Exact rank ties retain the earlier epoch in the caller.
    """
    if not predictions or not thresholds:
        raise ValueError("epoch selection requires development profiles and thresholds")
    if not math.isfinite(loss) or loss < 0:
        raise ValueError("invalid development loss")
    if type(maximum_false) is not int or maximum_false < 0:
        raise ValueError("invalid false-conversion budget")
    if any(type(value) not in (float, int) or not math.isfinite(value) or not 0 <= value <= 1
           for value in thresholds):
        raise ValueError("invalid development threshold")
    observations: dict[str, tuple[list[float], list[float], int, int]] = {}
    for profile, (values, labels) in predictions.items():
        if not labels or len(values) != len(labels) * 4:
            raise ValueError("invalid development prediction dimensions")
        positives: list[float] = []
        negatives: list[float] = []
        possible = 0
        for row, label in enumerate(labels):
            scores = values[row * 4:row * 4 + 4]
            if label not in (0, 1, 2, 3) or any(not math.isfinite(value) or not 0 <= value <= 1 for value in scores):
                raise ValueError("invalid development labels or probabilities")
            if not math.isclose(sum(scores), 1.0, rel_tol=0.0, abs_tol=1e-8):
                raise ValueError("development probabilities are not normalized")
            possible += int(label == 1)
            if max(range(4), key=scores.__getitem__) == 1:
                (positives if label == 1 else negatives).append(scores[1])
        if not possible:
            raise ValueError("development profile has no conversion targets")
        observations[profile] = sorted(positives), sorted(negatives), possible, len(labels)
    best: EpochSelection | None = None
    for threshold in sorted(set(thresholds)):
        by_profile: dict[str, dict[str, int | float]] = {}
        for profile, (positives, negatives, possible, rows) in observations.items():
            true = len(positives) - bisect_left(positives, threshold)
            false = len(negatives) - bisect_left(negatives, threshold)
            by_profile[profile] = {"rows": rows, "converted_correctly": true,
                                   "false_conversions": false, "convert_rows": possible,
                                   "conversion_recall": true / possible}
        if any(row["false_conversions"] > maximum_false for row in by_profile.values()):
            continue
        candidate = EpochSelection(
            float(threshold), min(row["conversion_recall"] for row in by_profile.values()),
            sum(row["converted_correctly"] for row in by_profile.values()) /
            sum(row["convert_rows"] for row in by_profile.values()), loss, by_profile,
        )
        if best is None or candidate.rank > best.rank:
            best = candidate
    return best
