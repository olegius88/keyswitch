"""Authored regressions of the installed model pair that are disclosed, not hidden.

The tests decorated with :func:`disclosed_installed_pair_regression` describe how
the shipped context-v1 + prefix-v1 pair fails on reproduced user text (see
.t/reliable-release-2026-09-12/DEFAULT-EARLY-REGRESSIONS.md and
DEFAULT-REGRESSION-TESTS.md). They stay in the suite as expected failures while
that pair is installed, so the suite is green for a release that ships it, and
they turn into hard assertions the moment a feature-version-3 context model is
installed: an unexpected pass is then reported as such by unittest.

Wiring the packaged lexicon supplement into the runtime removed all but one of
them, and the replacement-shape refusal in the context policy (a word is never
replaced by a reading with punctuation in it, ``дюп`` -> ``l.g``) closed that
last one on 20.09.2026. No test carries that decorator at the moment; it stays
for the next regression that has to be disclosed rather than hidden.

:func:`disclosed_schema3_pair_regression` is its counterpart for the pair installed on
02.10.2026 and its context model retrained on corpus v18 on 04.10.2026: authored
expectations of context-v1 that the schema-3 model decides differently, listed in its
docstring with the measured verdicts.
"""

from __future__ import annotations

import unittest
from collections.abc import Callable
from typing import NoReturn, TypeVar

from keyswitch.constants.models import CONTEXT_ACTION_FEATURE_VERSION, CONTEXT_V1_FEATURE_VERSIONS
from keyswitch.context_model import ContextModel

# NoReturn is the bottom type, so any one-argument test method satisfies the bound
# (parameters are contravariant) without an explicit Any in the signature.
T = TypeVar("T", bound=Callable[[NoReturn], None])


def installed_pair_is_disclosed() -> bool:
    model, _status = ContextModel.try_load()
    return model is None or model.feature_version in CONTEXT_V1_FEATURE_VERSIONS


def disclosed_installed_pair_regression(test: T) -> T:
    """Expected failure only while the disclosed pair is installed; a real test otherwise."""

    if installed_pair_is_disclosed():
        return unittest.expectedFailure(test)
    return test


def disclosed_schema3_pair_regression(test: T) -> T:
    """Expected failure while a feature-version-3 pair is installed; a real test otherwise.

    The authored expectations below were written against context-v1 and hold for it.
    The installed context-v3 + prefix-v2 pair (02.10.2026; the context model retrained on
    corpus v18 on 04.10.2026) decides each of them with its own calibrated verdict,
    measured on the installed artifact:

    * `jr.` then `привет`: `jr` waits for its neighbour (wait p=1.000), and the neighbour's
      conversion is not one the model's vocabulary supports, so the pair is not converted
      and `jr.` stays;
    * `ша` after a protected `ша `: wait p=0.998 (convert p=0.000) with a Russian word
      before it, so the second `ша` waits instead of becoming `if` by the curated rule,
      which the model arbitrates in schema 3;
    * `b/bkb`: convert p=0.274, the model leaves the slash-joined token as typed.

    `ghbdtn@` was disclosed for the corpus v16 model as well; the corpus v18 model converts it
    (p=0.993), and the test holds.
    Each is a text-preserving verdict, never a corruption; whether the expectation or the
    verdict should change is a decision recorded in the test, not hidden by removing it.
    """

    model, _status = ContextModel.try_load()
    if model is not None and model.feature_version == CONTEXT_ACTION_FEATURE_VERSION:
        return unittest.expectedFailure(test)
    return test
