"""Select an epoch on development actions; calibration sets the serving threshold."""

from __future__ import annotations

import math
from array import array
from bisect import bisect_left
from collections.abc import Mapping, Sequence
from dataclasses import dataclass

from keyswitch.context_model import ACTIONS
from keyswitch.constants.training import PROBABILITY_SUM_TOLERANCE


@dataclass(frozen=True)
class EpochSelection:
    threshold: float
    minimum_recall: float
    pooled_recall: float
    loss: float
    by_profile: dict[str, dict[str, int | float]]
    net_benefit: int = 0
    minimum_net_benefit: int = 0

    @property
    def rank(self) -> tuple[float, float, float, float, float]:
        """Worst profile first, then the pooled balance: a model is judged by what it nets.

        Until 17.09.2026 the rank was recall alone under a zero-false-conversion budget.
        That budget is what forced class-wide vetoes into the runtime - without them no
        threshold reached zero and the fit degenerated into converting nothing. A model that
        decides is weighed by how much it repairs against how much it breaks.
        """
        return (self.minimum_net_benefit, self.net_benefit, self.pooled_recall, -self.loss, -self.threshold)


def assess_epoch(
    predictions: Mapping[str, tuple[array[float], array[int]]], *,
    thresholds: Sequence[float], loss: float, minimum_net_benefit: int = 1,
) -> EpochSelection | None:
    """Rank development operating points by what they net after the runtime support checks.

    The threshold here is a development operating point, not an exported
    decision. After choosing weights, the trainer calibrates their threshold
    on its separate calibration split. No calibration or test outcome belongs
    in this selection. Exact rank ties retain the earlier epoch in the caller.

    An epoch qualifies when every profile repairs more than it breaks by at least
    ``minimum_net_benefit`` conversions; among those the one with the best balance wins.
    """
    if not predictions or not thresholds:
        raise ValueError("epoch selection requires development profiles and thresholds")
    if not math.isfinite(loss) or loss < 0:
        raise ValueError("invalid development loss")
    if type(minimum_net_benefit) is not int or minimum_net_benefit < 0:
        raise ValueError("invalid net-benefit floor")
    if any(type(value) not in (float, int) or not math.isfinite(value) or not 0 <= value <= 1
           for value in thresholds):
        raise ValueError("invalid development threshold")
    observations: dict[str, tuple[list[float], list[float], int, int]] = {}
    for profile, (values, labels) in predictions.items():
        if not labels or len(values) != len(labels) * len(ACTIONS):
            raise ValueError("invalid development prediction dimensions")
        positives: list[float] = []
        negatives: list[float] = []
        possible = 0
        for row, label in enumerate(labels):
            scores = values[row * len(ACTIONS):row * len(ACTIONS) + len(ACTIONS)]
            if label not in range(len(ACTIONS)) or any(not math.isfinite(value) or not 0 <= value <= 1 for value in scores):
                raise ValueError("invalid development labels or probabilities")
            if not math.isclose(sum(scores), 1.0, rel_tol=0.0, abs_tol=PROBABILITY_SUM_TOLERANCE):
                raise ValueError("development probabilities are not normalized")
            possible += int(label == 1)
            if max(range(len(ACTIONS)), key=scores.__getitem__) == 1:
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
        per_profile_net = [int(row["converted_correctly"]) - int(row["false_conversions"])
                           for row in by_profile.values()]
        if min(per_profile_net) < minimum_net_benefit:
            continue
        candidate = EpochSelection(
            float(threshold), min(row["conversion_recall"] for row in by_profile.values()),
            sum(row["converted_correctly"] for row in by_profile.values()) /
            sum(row["convert_rows"] for row in by_profile.values()), loss, by_profile,
            net_benefit=sum(per_profile_net), minimum_net_benefit=min(per_profile_net),
        )
        if best is None or candidate.rank > best.rank:
            best = candidate
    return best
