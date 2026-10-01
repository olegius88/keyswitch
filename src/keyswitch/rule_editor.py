"""What the rule window and the rule offer show, apart from any toolkit.

The Tk window (Windows, macOS) and the GTK window (Linux) draw the same form:
the letters, the condition, case sensitivity and the action. This module fills
the form from the offer a double press of the conversion hotkey made, checks it
and turns it into a :class:`~keyswitch.learning.LearnedRule`.
"""

from __future__ import annotations

from dataclasses import dataclass

from .engine import LearningPrompt
from .indicator import layout_label
from .layouts import RU_KEYS, US_KEYS, LayoutPair
from .learning import InvalidRule, LearnedRule, RuleAction, RuleMatch, validate_rule

WINDOW_TITLE = "Правило переключения"
PATTERN_LABEL = "Сочетание букв, как оно набирается:"
CONDITION_TITLE = "Условие"
CONDITION_LEAD = "Набираемое на клавиатуре слово должно:"
EXTRA_LEAD = "Дополнительно:"
CASE_LABEL = "учитывать регистр"
ACTION_TITLE = "Действие"
MATCH_LABELS: dict[RuleMatch, str] = {
    "contains": "содержать данное сочетание букв",
    "prefix": "начинаться с данных букв",
    "exact": "совпадать с данным сочетанием",
}
ACTION_LABELS: dict[RuleAction, str] = {
    "convert": "Переводить слово в другую раскладку",
    "keep": "Не переводить слово в другую раскладку",
}
# How the settings list names a condition in one short phrase.
MATCH_SHORT_LABELS: dict[RuleMatch, str] = {
    "contains": "содержит",
    "prefix": "начинается с",
    "exact": "совпадает",
}
PROMPT_TITLE = "Добавить правило переключения?"
PROMPT_HINT = "Enter - открыть правило    Esc - закрыть"
PROMPT_EMPTY = "Слова нет: откроется пустое правило"
_LATIN_LETTERS = frozenset(character for character in US_KEYS if character.isalpha())
_CYRILLIC_LETTERS = frozenset(character for character in RU_KEYS if character.isalpha())


@dataclass(frozen=True)
class RuleDraft:
    """The state of the form; ``editing`` is the stored rule it was opened for."""

    pattern: str = ""
    match: RuleMatch = "exact"
    case_sensitive: bool = False
    action: RuleAction = "convert"
    application: str = ""
    editing: LearnedRule | None = None


def draft_from_prompt(prompt: LearningPrompt) -> RuleDraft:
    """The form as the offer fills it: the word, as a whole, with its action."""

    return RuleDraft(prompt.original, action=prompt.action, application=prompt.application)


def draft_from_rule(rule: LearnedRule) -> RuleDraft:
    return RuleDraft(rule.pattern, rule.match, rule.case_sensitive, rule.action, editing=rule)


def pattern_group(pattern: str) -> int | None:
    """The layout the letters were typed in: Latin is EN, Cyrillic is RU.

    Punctuation and digits say nothing; letters of both alphabets cannot come
    from one layout, so such a pattern has no layout at all.
    """

    letters = {character.casefold() for character in pattern if character.isalpha()}
    latin = bool(letters & _LATIN_LETTERS)
    cyrillic = bool(letters & _CYRILLIC_LETTERS)
    if latin == cyrillic or letters - _LATIN_LETTERS - _CYRILLIC_LETTERS:
        return None
    return 0 if latin else 1


def other_layout_text(pattern: str, group: int) -> str:
    """The same keys as the other layout prints them."""

    pair = LayoutPair()
    return pair.translate(pattern, "us", "ru") if group == 0 else pair.translate(pattern, "ru", "us")


def build_rule(draft: RuleDraft) -> LearnedRule:
    """The rule the form describes; :class:`InvalidRule` says what to fix."""

    pattern = draft.pattern.strip()
    group = pattern_group(pattern)
    if not any(character.isalpha() for character in pattern):
        raise InvalidRule("В сочетании нужна хотя бы одна буква")
    if group is None:
        raise InvalidRule("Наберите сочетание буквами одной раскладки: латиницей или кириллицей")
    # The other of the two layouts the program works with.
    rule = LearnedRule(pattern, group, 1 - group, draft.action, draft.match, draft.case_sensitive)
    validate_rule(rule)
    return rule


def draft_problem(draft: RuleDraft) -> str:
    """What stops the form from becoming a rule, or an empty string."""

    try:
        build_rule(draft)
    except InvalidRule as error:
        return str(error)
    return ""


def layout_hint(draft: RuleDraft) -> str:
    """One line under the letters: which layout they are typed in and what the other prints."""

    pattern = draft.pattern.strip()
    group = pattern_group(pattern)
    if group is None:
        return ""
    return (
        f"Набрано в раскладке {layout_label(group)}; "
        f"в {layout_label(1 - group)} это «{other_layout_text(pattern, group)}»"
    )


def condition_text(rule: LearnedRule) -> str:
    case = ", с учётом регистра" if rule.case_sensitive else ""
    return MATCH_SHORT_LABELS[rule.match] + case


def action_text(rule: LearnedRule) -> str:
    if rule.action == "keep":
        return "не переводить"
    return f"переводить в {layout_label(rule.target_group)}"


def prompt_word_line(prompt: LearningPrompt) -> str:
    """The middle line of the offer: the word and what the rule would do with it."""

    if not prompt.original:
        return PROMPT_EMPTY
    if prompt.action == "keep":
        return f"«{prompt.original}»: не переводить"
    return f"«{prompt.original}» → «{prompt.replacement}»: переводить"
