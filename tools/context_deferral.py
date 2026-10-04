"""Which isolated reading the context-action corpus defers instead of labelling.

A word with no word on either side has an observable intent label only when the
two readings of its keys differ in plausibility. One letter is deferred whatever
it is (``train_context_action_model.action_rows`` keeps the one curated-letter
exception); two and three letters are deferred only when both readings are
plausible - the portable lexicon of the reading's layout knows it, or the
shipped identifier index does - because then `tot` is as much `еще` as it is
English, `ты` as much `ns`, and `зум` as much the Debian command `pev` as it is
a chat word, and only a neighbour can tell. One plausible reading decides at
once: `rjn` is only `кот`, `lf` is only `да`, `три` is only Russian, and
`зь2`/`pm2` or `гш`/`ui`, plausible neither way, are left to the model's own
evidence.

Until 04.10.2026 every two-letter word alone was deferred too. Replayed on the
owner's own typing (0.38.0), a one-word message was then never corrected: the
model answered "wait" for `jr`, `гш`, `зщ`, `еп`, and the message ended before
any neighbour came - 22 of the 113 words it left in the wrong layout, 13 of
them marked by the owner by hand.

The engine still holds every one- or two-letter word for its next word
(``held_for_next_word``): the lookahead curriculum seeds and the planned-mass
balance ask that, not the isolated label.

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
from keyswitch.context_policy import shared_identifiers
from keyswitch.identifier_lexicon import IdentifierLexicon
from keyswitch.language_model import LanguageModel
from reference_lexicon import reference_models


_PLAUSIBILITY: tuple[dict[int, LanguageModel], IdentifierLexicon | None] | None = None


def plausible_reading(text: str, group: int) -> bool:
    """Whether a reading could be meant as typed: a known word of its layout or an identifier."""

    global _PLAUSIBILITY
    if _PLAUSIBILITY is None:
        _PLAUSIBILITY = reference_models(False), shared_identifiers()
    models, identifiers = _PLAUSIBILITY
    if models[group].score(text).known:
        return True
    return identifiers is not None and identifiers.contains(text)


def deferred_isolated(original: str, alternate: str, group: int) -> bool:
    """Whether an isolated reading has no observable intent label and is deferred.

    Symmetric in its two members: both readings of one physical word get the
    same answer, so natural and intervention frames of one moment agree.
    """

    length = len(original)
    if length == 1:
        return True
    if length <= ACTION_DEFERRED_WORD_MAX_CHARACTERS:
        return plausible_reading(original, group) and plausible_reading(alternate, 1 - group)
    return False


def held_for_next_word(original: str, alternate: str, group: int) -> bool:
    """Whether the engine may hold a word for its next word: every one or two letters, and three
    letters whose two readings are both plausible. The lookahead curriculum's seeds and focus."""

    if 0 < len(original) <= ACTION_SHORT_WORD_MAX_CHARACTERS:
        return True
    return deferred_isolated(original, alternate, group)


def lookahead_focus(original: str, group: int) -> bool:
    """Whether a word is one the engine may hold for its next word: a deferrable reading."""

    try:
        alternate = translated(original, group)
    except ValueError:
        return False
    return held_for_next_word(original, alternate, group)
