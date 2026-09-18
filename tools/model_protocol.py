"""Values the training and verification protocol agrees on, declared once.

A trainer writes a seal, an evaluator reads it and a public verifier checks it.
The three only agree because they spell the same words: the stage a seal is in,
the names of the corpus splits, the names of the lexical profiles. Spelled out
separately in each module, one typo does not raise anything - it quietly turns a
check into a different check, or drops a split from a sum that still looks whole.

This module therefore owns those words and nothing else. It imports nothing, so
the public receipt verifier can use it without pulling the evaluator or the
engine in behind it.
"""

from __future__ import annotations

from typing import Final

# A seal is written before its test is opened, and says so. A candidate that
# failed its own pre-test gates carries the second value and is never promoted.
SEALED_BEFORE_TEST: Final = "sealed-before-test"
REJECTED_BEFORE_TEST: Final = "rejected-before-test"

TRAIN: Final = "train"
DEVELOPMENT: Final = "development"
CALIBRATION: Final = "calibration"
TEST: Final = "test"
QUARANTINE: Final = "quarantine"

# What a model may be fitted and calibrated on, what may additionally be counted,
# and everything a frozen corpus directory holds.
FITTING_SPLITS: Final = (TRAIN, DEVELOPMENT, CALIBRATION)
ACTIVE_SPLITS: Final = (*FITTING_SPLITS, TEST)
ALL_SPLITS: Final = (*ACTIVE_SPLITS, QUARANTINE)

# Portable is the shipped lexicon alone; the other adds the reference Hunspell
# dictionaries, so a result that only holds on one machine's dictionaries shows.
PORTABLE: Final = "portable"
REFERENCE_HUNSPELL: Final = "reference_hunspell"
PROFILES: Final = (PORTABLE, REFERENCE_HUNSPELL)
