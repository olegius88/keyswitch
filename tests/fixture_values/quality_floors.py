"""Quality floors the installed artifacts must stay above.

Every number here is the level the shipped pair and the frozen engine replays reached
on 04.10.2026. `tests/test_quality_ratchet.py` fails when an installed artifact falls
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
# Raised for the corpus v19 pair (04.10.2026): on test v19, 13 rows restored exactly against the
# baseline's 13 with the early switch on and 177 against 167 with it off, no correct row corrupted
# by either. Test v18 had given 15 against 16 and 190 against 183, and test v16 26 against 21: a
# margin measured on one test set is not one of another, and a new set that gives less is a
# lowering its pull request states.
RECEIPT_MIN_RESTORATION_MARGIN: Final[dict[str, int]] = {"default": 0, "early_off": 10}
# No replay may fail to execute, change a word's length or land a correction in the wrong layout.
RECEIPT_MAX_EXECUTION_ERRORS: Final = 0
RECEIPT_MAX_LENGTH_MISMATCHES: Final = 0
RECEIPT_MAX_CORRECTION_LAYOUT_MISMATCHES: Final = 0
# Calibration split of the installed pair.
RECEIPT_MIN_CALIBRATION_RECALL: Final = 0.976
RECEIPT_MAX_CALIBRATION_FALSE_CONVERSIONS: Final[dict[str, int]] = {"portable": 7, "reference_hunspell": 7}
# Serving thresholds are part of the contract: a lower threshold converts on weaker evidence.
RECEIPT_MIN_CONVERSION_THRESHOLD: Final = 0.99
RECEIPT_MIN_PREFIX_CONVERSION_THRESHOLD: Final = 0.985

# Frozen prefix engine replay (model/prefix_v1/engine-report.json), every profile.
PREFIX_MIN_EXACT: Final = 268
PREFIX_MIN_RESTORED: Final = 128
PREFIX_MIN_EARLY_RESTORED: Final = 123
# Raised from 19 for the corpus v18 pair (04.10.2026): one more of the one documented class, a
# Russian word typed in the English layout after a `$` prompt that the replay labels a command to
# keep. The word decision converts it with the prefix model and without it alike.
PREFIX_MAX_CHANGED_CORRECT: Final = 20
PREFIX_MAX_LENGTH_MISMATCHES: Final = 0

# Frozen boundary engine replay (model/boundary_v1/engine-regression.json), active variant.
BOUNDARY_MIN_EXACT: Final = 18
BOUNDARY_MAX_CHANGED_CORRECT: Final = 0
BOUNDARY_MAX_INJECTIONS_BEFORE_HARD_BOUNDARY: Final = 0
BOUNDARY_MAX_LENGTH_MISMATCHES: Final = 0

# Authored expectations the installed pair decides differently and the suite discloses as
# expected failures (tests/disclosed_regressions.py). Each new disclosure is a conscious
# decision recorded here, never a quiet way past a failing test.
MAX_DISCLOSED_EXPECTED_FAILURES: Final = 3
