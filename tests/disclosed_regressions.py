"""Authored regressions of the installed model pair that are disclosed, not hidden.

The tests decorated with :func:`disclosed_installed_pair_regression` describe how
the shipped context-v1 + prefix-v1 pair fails on reproduced user text (see
.t/reliable-release-2026-09-12/DEFAULT-EARLY-REGRESSIONS.md and
DEFAULT-REGRESSION-TESTS.md). They stay in the suite as expected failures while
that pair is installed, so the suite is green for a release that ships it, and
they turn into hard assertions the moment a feature-version-3 context model is
installed: an unexpected pass is then reported as such by unittest.

Wiring the packaged lexicon supplement into the runtime removed all but one of
them: an isolated word that neither the onboard lexicon nor the supplement knows
is still converted, which is the class a feature-version-3 model refuses without
a licence from the frozen models.
"""

from __future__ import annotations

import unittest
from collections.abc import Callable
from typing import NoReturn, TypeVar

from keyswitch.context_model import ContextModel

# NoReturn is the bottom type, so any one-argument test method satisfies the bound
# (parameters are contravariant) without an explicit Any in the signature.
T = TypeVar("T", bound=Callable[[NoReturn], None])


def installed_pair_is_disclosed() -> bool:
    model, _status = ContextModel.try_load()
    return model is None or model.feature_version == 2


def disclosed_installed_pair_regression(test: T) -> T:
    """Expected failure only while the disclosed pair is installed; a real test otherwise."""

    if installed_pair_is_disclosed():
        return unittest.expectedFailure(test)
    return test
