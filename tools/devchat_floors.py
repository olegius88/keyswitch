"""The floors of the developer-chat test (tools/evaluate_devchat_test.py), computed from its counts alone.

Shared by the test, which writes them, and the receipt verifier (tools/verify_context_action_model.py), which
recomputes them from the counts the receipt publishes: no import of the engine or the bench, so the public packaging
gate can run them.
"""

from __future__ import annotations

import math
from collections.abc import Mapping
from typing import Final

from keyswitch.constants.corpus import DEVCHAT_MIN_NET_MARGIN, DEVCHAT_SIGNIFICANCE

CORRECT: Final = "correct"
RESTORING_MODES: Final = ("forget_ru", "forget_en", "start_wrong")
MODES: Final = (CORRECT, *RESTORING_MODES)
CANDIDATE: Final = "candidate"
RELEASED: Final = "released"
BASELINE: Final = "baseline"
PAIRS: Final = (CANDIDATE, RELEASED, BASELINE)
FLOORS: Final = ("every_message_typed", "f1_better_than_baseline", "f3_not_worse_than_released")
Table = Mapping[str, Mapping[str, int]]


def corruptions(table: Table) -> int:
    return table[CORRECT]["n"] - table[CORRECT]["ok"]


def net(table: Table) -> int:
    """Messages restored in the three modes that need a restoration, less the ones typed right and changed."""

    return sum(table[mode]["ok"] for mode in RESTORING_MODES) - corruptions(table)


def loss_chance(losses: int, gains: int) -> float:
    """P(at least `losses` losses among the losses + gains messages two pairs disagree on, were a loss as likely as a
    gain): the one-sided binomial test F3 reads per mode."""

    total = losses + gains
    return sum(math.comb(total, count) for count in range(losses, total + 1)) / (1 << total)


def floors(tables: Mapping[str, Table], paired: Mapping[str, Mapping[str, int]]) -> dict[str, bool]:
    """F1 against the baseline pair, F3 against the released pair, and every message typed by every pair."""

    candidate, released, baseline = (tables[name] for name in PAIRS)
    errors = sum(table[mode]["errors"] for table in tables.values() for mode in MODES if mode in table)
    return {
        "every_message_typed": errors == 0,
        "f1_better_than_baseline": (net(candidate) >= net(baseline) + DEVCHAT_MIN_NET_MARGIN
                                    and corruptions(candidate) <= corruptions(baseline)),
        "f3_not_worse_than_released": (
            all(loss_chance(paired[mode]["losses"], paired[mode]["gains"]) >= DEVCHAT_SIGNIFICANCE for mode in RESTORING_MODES)
            and sum(candidate[mode]["ok"] for mode in RESTORING_MODES) >= sum(released[mode]["ok"] for mode in RESTORING_MODES)
            and corruptions(candidate) <= corruptions(released)),
    }
