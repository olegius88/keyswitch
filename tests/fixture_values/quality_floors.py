"""Quality floors the installed artifacts must stay above.

Every number here is the level the shipped pair and the frozen engine replays reached
on 03.10.2026. `tests/test_quality_ratchet.py` fails when an installed artifact falls
below a floor. A floor only moves up: when a new pair does better, raise the floor in the
same change; lowering one is a decision a pull request has to state, never a side effect
of a retrained model or an edited test. The model protocol gates a candidate against the
frozen context-v1 + prefix-v1 pair; these floors gate it against the best pair shipped.
"""

from __future__ import annotations

from typing import Final

# Sealed test of the installed context-v3 + prefix-v2 pair (release receipt), per profile.
# A candidate must corrupt no correctly typed row and keep every correct row as typed.
RECEIPT_MAX_CORRUPTIONS: Final = 0
# Restorations beyond the frozen baseline pair, by settings mode (early switch on / off).
RECEIPT_MIN_RESTORATION_MARGIN: Final[dict[str, int]] = {"default": 5, "early_off": 7}
# No replay may fail to execute, change a word's length or land a correction in the wrong layout.
RECEIPT_MAX_EXECUTION_ERRORS: Final = 0
RECEIPT_MAX_LENGTH_MISMATCHES: Final = 0
RECEIPT_MAX_CORRECTION_LAYOUT_MISMATCHES: Final = 0
# Calibration split of the installed pair.
RECEIPT_MIN_CALIBRATION_RECALL: Final = 0.975
RECEIPT_MAX_CALIBRATION_FALSE_CONVERSIONS: Final[dict[str, int]] = {"portable": 13, "reference_hunspell": 14}
# Serving thresholds are part of the contract: a lower threshold converts on weaker evidence.
RECEIPT_MIN_CONVERSION_THRESHOLD: Final = 0.99
RECEIPT_MIN_PREFIX_CONVERSION_THRESHOLD: Final = 0.985

# Frozen prefix engine replay (model/prefix_v1/engine-report.json), every profile.
PREFIX_MIN_EXACT: Final = 268
PREFIX_MIN_RESTORED: Final = 127
PREFIX_MIN_EARLY_RESTORED: Final = 112
PREFIX_MAX_CHANGED_CORRECT: Final = 19
PREFIX_MAX_LENGTH_MISMATCHES: Final = 0

# Frozen boundary engine replay (model/boundary_v1/engine-regression.json), active variant.
BOUNDARY_MIN_EXACT: Final = 18
BOUNDARY_MAX_CHANGED_CORRECT: Final = 0
BOUNDARY_MAX_INJECTIONS_BEFORE_HARD_BOUNDARY: Final = 0
BOUNDARY_MAX_LENGTH_MISMATCHES: Final = 0

# Authored expectations the installed pair decides differently and the suite discloses as
# expected failures (tests/disclosed_regressions.py). Each new disclosure is a conscious
# decision recorded here, never a quiet way past a failing test.
MAX_DISCLOSED_EXPECTED_FAILURES: Final = 4
