"""The models every engine loads, loaded once per test process.

A KeySwitchEngine asks LanguageModel.load for both languages with the packaged supplement, and its
context policy parses the context artifact. The language models are cached already, but each load
normalises the supplement's words again (0.86 s a language), and the 7.5 MB context artifact is parsed
again (0.7 s): a module that builds an engine for every test spent most of its time there. The
application loads them as before; a test module that calls `share_engine_models()` from setUpModule
gives its engines the same models without the repeated work:

- LanguageModel.load with a language's packaged supplement returns the model the first such load
  returned, the very object the language model cache returns for it; any other call loads as before.
- ContextModel.try_load of the installed artifact returns a new ContextModel built from the one parsed
  first, so that no test sees what another changed in it; any other path loads as before.

A test that patches either loader itself still gets its own patch for as long as it holds it.
"""

from __future__ import annotations

import unittest
from collections.abc import Iterable
from functools import cache
from pathlib import Path
from unittest.mock import patch

from keyswitch.context_model import ARTIFACT_PATH, ContextModel
from keyswitch.language_model import LanguageModel
from keyswitch.lexicon_supplement import supplement_words

_load_language = LanguageModel.load
_load_context = ContextModel.try_load


@cache
def _language(locale: str) -> LanguageModel:
    return _load_language(locale, supplement_words(locale))


@cache
def _context() -> tuple[ContextModel | None, str]:
    return _load_context(ARTIFACT_PATH)


def shared_language(locale: str, extra_words: Iterable[str] = ()) -> LanguageModel:
    if extra_words is supplement_words(locale):
        return _language(locale)
    return _load_language(locale, extra_words)


def shared_context(path: Path = ARTIFACT_PATH) -> tuple[ContextModel | None, str]:
    if path != ARTIFACT_PATH:
        return _load_context(path)
    model, status = _context()
    if model is None:
        return model, status
    return ContextModel(model.weights, model.version, model.conversion_threshold,
                        feature_version=model.feature_version), status


def share_engine_models() -> None:
    """For the rest of the calling test module, give the engines it builds the shared models."""
    for patcher in (patch.object(LanguageModel, "load", side_effect=shared_language),
                    patch.object(ContextModel, "try_load", side_effect=shared_context)):
        patcher.start()
        unittest.addModuleCleanup(patcher.stop)
