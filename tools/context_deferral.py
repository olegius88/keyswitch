"""Which isolated reading the context-action corpus defers instead of labelling.

A word with no word on either side has an observable intent label only when the
two readings of its keys differ in plausibility. One or two letters are deferred
whatever they are (``train_context_action_model.action_rows`` keeps the one
curated-letter exception); three letters are deferred only when both readings
are plausible - the portable lexicon of the reading's layout knows it, or the
shipped identifier index does - because then `tot` is as much `еще` as it is
English and `зум` is as much the Debian command `pev` as it is a chat word, and
only a neighbour can tell. One plausible reading decides at once: `rjn` is only
`кот`, `три` is only Russian, and `зь2`/`pm2`, plausible neither way, are left
to the model's own evidence.

Labels never depend on the fitting profile: the reference Hunspell profile adds
spell-known forms the shipped engine has no dictionary for, and one frame must
carry one label in both profiles. The shared trainer, the lookahead curriculum
and the planned-mass balance all ask this module, so a focus word is "short"
the same way everywhere.
"""

from __future__ import annotations

from context_physical_keys import translated
from keyswitch.constants.training import (
    ACTION_DEFERRED_WORD_MAX_CHARACTERS,
    ACTION_SHORT_WORD_MAX_CHARACTERS,
)
from keyswitch.context_model import _term_frequency
from keyswitch.context_policy import shared_identifiers
from keyswitch.identifier_lexicon import IdentifierLexicon
from keyswitch.language_model import LanguageModel
from reference_lexicon import reference_models


_PLAUSIBILITY: tuple[dict[int, LanguageModel], IdentifierLexicon | None] | None = None


def _plausibility() -> tuple[dict[int, LanguageModel], IdentifierLexicon | None]:
    global _PLAUSIBILITY
    if _PLAUSIBILITY is None:
        _PLAUSIBILITY = reference_models(False), shared_identifiers()
    return _PLAUSIBILITY


def plausible_reading(text: str, group: int) -> bool:
    """Whether a reading could be meant as typed: a known word of its layout or an identifier."""

    models, identifiers = _plausibility()
    if models[group].score(text).known:
        return True
    return identifiers is not None and identifiers.contains(text)


def deferred_isolated(original: str, alternate: str, group: int) -> bool:
    """Whether an isolated reading has no observable intent label and is deferred.

    Symmetric in its two members: both readings of one physical word get the
    same answer, so natural and intervention frames of one moment agree.
    """

    length = len(original)
    if 0 < length <= ACTION_SHORT_WORD_MAX_CHARACTERS:
        return True
    if ACTION_SHORT_WORD_MAX_CHARACTERS < length <= ACTION_DEFERRED_WORD_MAX_CHARACTERS:
        return plausible_reading(original, group) and plausible_reading(alternate, 1 - group)
    return False


def counted_reading(original: str, alternate: str, group: int, *,
                    minimum: int, other_maximum: int, dominance: float) -> int | None:
    """The layout group a lone token's counts settle, or None when they settle nothing.

    Both readings of a deferred lone word may be no word of the portable lexicon and still differ in
    how often text uses them: `ui` is in Russian technical text 4752 times and its Russian keys `гш`
    never, while `тз` (744) outweighs its Latin keys `np` (80). The term frequency table the model
    reads (schema 7: `latin` for Latin tokens, `cyrillic` and `russian` for Cyrillic ones) settles a
    reading counted at least `minimum` times and `dominance` times as often as the other, whose count
    stays under `other_maximum` and which no lexicon knows: then the other reading is the same keys
    typed in the wrong layout. Two counted readings (`ns` 195, `ты`) or a word of the lexicon on the
    weaker side (`лун` for `key`) settle nothing; only a neighbour can tell. Symmetric in its two
    members, as deferred_isolated is.
    """

    table = _term_frequency()
    latin, cyrillic = (original, alternate) if group == 0 else (alternate, original)
    latin, cyrillic = latin.casefold(), cyrillic.casefold()
    counts = (table["latin"].get(latin, 0), max(table["cyrillic"].get(cyrillic, 0), table["russian"].get(cyrillic, 0)))
    models = _plausibility()[0]
    for strong in (0, 1):
        weak = 1 - strong
        if (counts[strong] >= minimum and counts[strong] >= dominance * counts[weak] and counts[weak] < other_maximum
                and not models[weak].score((latin, cyrillic)[weak]).known):
            return strong
    return None


def lookahead_focus(original: str, group: int) -> bool:
    """Whether a word is one the engine may hold for its next word: a deferrable reading."""

    try:
        alternate = translated(original, group)
    except ValueError:
        return False
    return deferred_isolated(original, alternate, group)
