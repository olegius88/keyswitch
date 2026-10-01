"""Switching rules the user added one by one in the rule window.

A rule says what to do with a typed word that meets its condition: convert it to
the other layout, or keep it as typed. Nothing is written here unless the user
pressed OK in the rule window: a manual conversion, an undo or a cancelled
correction teaches nothing on its own.
"""

from __future__ import annotations

import json
import threading
from dataclasses import dataclass
from pathlib import Path
from typing import Literal, TypedDict, cast

from .constants.detection import MAX_WORD_STROKES
from .constants.file_formats import (
    LEARNING_STORE_BACKUP_SUFFIX,
    LEARNING_STORE_SCHEMA_VERSION,
    LEGACY_RULE_CONFIRMATIONS_REQUIRED,
    USER_DATA_JSON_INDENT,
)
from .history import data_dir

RuleMatch = Literal["exact", "prefix", "contains"]
RuleAction = Literal["convert", "keep"]
# The order the rule window offers the conditions in, widest first. When several rules fit
# one word the most specific decides, so the position is also the rank: a whole word before
# a beginning, a beginning before a part.
RULE_MATCHES: tuple[RuleMatch, ...] = ("contains", "prefix", "exact")
RULE_ACTIONS: tuple[RuleAction, ...] = ("convert", "keep")


class InvalidRule(ValueError):
    """A rule the store refuses to keep."""


@dataclass(frozen=True)
class LearnedRule:
    """One rule as the rule window and the settings list show it.

    ``pattern`` is the text as typed in ``source_group``'s layout: the same keys
    typed in the other layout are a different word and need their own rule.
    ``target_group`` is the layout a ``convert`` rule switches to; a ``keep``
    rule names it too, as the layout the word is not taken to.
    """

    pattern: str
    source_group: int
    target_group: int
    action: RuleAction = "convert"
    match: RuleMatch = "exact"
    case_sensitive: bool = False

    @property
    def key(self) -> str:
        return self.pattern if self.case_sensitive else self.pattern.casefold()

    @property
    def identity(self) -> tuple[int, str, bool, str]:
        """Two rules with one identity cannot both exist: the later replaces the earlier."""

        return self.source_group, self.match, self.case_sensitive, self.key

    def matches(self, word: str) -> bool:
        text = word if self.case_sensitive else word.casefold()
        if self.match == "exact":
            return text == self.key
        if self.match == "prefix":
            return text.startswith(self.key)
        return self.key in text

    def may_match_continuation(self, prefix: str) -> bool:
        """Whether a word that begins with ``prefix`` can still meet this rule.

        A rule on a part of the word can be met by letters not typed yet, so it
        answers yes only once the typed letters already lead into the part.
        """

        text = prefix if self.case_sensitive else prefix.casefold()
        if self.match == "exact":
            return self.key.startswith(text)
        if self.match == "prefix":
            return self.key.startswith(text) or text.startswith(self.key)
        return self.key in text or any(
            self.key.startswith(text[start:]) for start in range(len(text))
        )

    def specificity(self) -> tuple[int, int, bool]:
        return RULE_MATCHES.index(self.match), len(self.pattern), self.action == "keep"


class _LearningData(TypedDict):
    schema_version: int
    rules: list[dict[str, object]]


def validate_rule(rule: LearnedRule) -> None:
    """Raise :class:`InvalidRule` for a rule that cannot work as intended."""

    if not rule.pattern or rule.pattern != rule.pattern.strip():
        raise InvalidRule("Сочетание букв не может быть пустым или начинаться и кончаться пробелом")
    if any(character.isspace() for character in rule.pattern):
        raise InvalidRule("Сочетание букв набирается одним словом, без пробелов")
    if not any(character.isalpha() for character in rule.pattern):
        raise InvalidRule("В сочетании нужна хотя бы одна буква")
    if len(rule.pattern) > MAX_WORD_STROKES:
        raise InvalidRule("Сочетание длиннее самого длинного слова, которое отслеживает KeySwitch")
    if rule.source_group == rule.target_group or min(rule.source_group, rule.target_group) < 0:
        raise InvalidRule("Правило должно связывать две разные раскладки")
    if rule.action not in RULE_ACTIONS or rule.match not in RULE_MATCHES:
        raise InvalidRule("Неизвестное условие или действие правила")


def _rule_from_json(value: object) -> LearnedRule | None:
    if not isinstance(value, dict):
        return None
    pattern, source, target = value.get("pattern"), value.get("source_group"), value.get("target_group")
    action, match, case = value.get("action"), value.get("match"), value.get("case_sensitive", False)
    if (
        not isinstance(pattern, str) or type(source) is not int or type(target) is not int
        or action not in RULE_ACTIONS or match not in RULE_MATCHES or not isinstance(case, bool)
    ):
        return None
    rule = LearnedRule(
        pattern, source, target, cast(RuleAction, action), cast(RuleMatch, match), case
    )
    try:
        validate_rule(rule)
    except InvalidRule:
        return None
    return rule


def _rule_to_json(rule: LearnedRule) -> dict[str, object]:
    return {
        "pattern": rule.pattern,
        "source_group": rule.source_group,
        "target_group": rule.target_group,
        "action": rule.action,
        "match": rule.match,
        "case_sensitive": rule.case_sensitive,
    }


def _legacy_identity(key: object) -> tuple[int, str] | None:
    if not isinstance(key, str):
        return None
    group, separator, word = key.partition(":")
    if not separator or not group.isdigit():
        return None
    return int(group), word


def migrate_legacy(payload: dict[str, object], confirmations_required: int) -> list[LearnedRule]:
    """Rules from a learning file of schema 2 and older.

    Such a file kept a counter per word and a list of forbidden directions. A word
    whose counter had reached the threshold acted as a rule and stays one, as a
    whole-word "convert" rule; a forbidden direction becomes a whole-word "keep"
    rule. A counter still short of the threshold never acted and is not carried
    over: the old versions wrote one on every manual conversion, without asking.
    """

    rules: list[LearnedRule] = []
    stored_rules = payload.get("rules")
    for key, value in (stored_rules.items() if isinstance(stored_rules, dict) else ()):
        identity = _legacy_identity(key)
        if identity is None or not isinstance(value, dict):
            continue
        target, confirmations = value.get("target_group"), value.get("confirmations", 0)
        if type(target) is not int or type(confirmations) is not int:
            continue
        if confirmations < max(1, confirmations_required):
            continue
        rules.append(LearnedRule(identity[1], identity[0], target))
    stored_rejections = payload.get("rejections")
    for key, value in (stored_rejections.items() if isinstance(stored_rejections, dict) else ()):
        identity = _legacy_identity(key)
        if identity is None or not isinstance(value, list):
            continue
        for target in value:
            if type(target) is int:
                rules.append(LearnedRule(identity[1], identity[0], target, "keep"))
    valid: dict[tuple[int, str, bool, str], LearnedRule] = {}
    for rule in rules:
        try:
            validate_rule(rule)
        except InvalidRule:
            continue
        current = valid.get(rule.identity)
        # A word both forced and forbidden ended up untouched by the old engine
        # (the forbidden direction removed the rule): the "keep" rule says that.
        if current is None or rule.action == "keep":
            valid[rule.identity] = rule
    return list(valid.values())


class LearningStore:
    """Persist the user's switching rules in ``learning.json``.

    ``legacy_confirmations`` is the threshold an older learning file was used
    with; it only decides which of its counters were rules when the file is
    converted to the current schema.
    """

    def __init__(
        self, path: Path | None = None, *, legacy_confirmations: int = LEGACY_RULE_CONFIRMATIONS_REQUIRED
    ) -> None:
        self.path = path or data_dir() / "learning.json"
        self._lock = threading.RLock()
        self._rules: list[LearnedRule] = []
        self._legacy_confirmations = legacy_confirmations
        self.migrated = 0
        self.load()

    def load(self) -> None:
        with self._lock:
            self._rules = []
            if not self.path.is_file():
                return
            try:
                raw = self.path.read_bytes()
                payload = json.loads(raw.decode("utf-8"))
            except (OSError, ValueError):
                return
            if not isinstance(payload, dict):
                return
            stored = payload.get("rules")
            if isinstance(stored, list):
                for value in stored:
                    rule = _rule_from_json(value)
                    if rule is not None:
                        self._insert(rule)
                return
            if not isinstance(stored, dict) and not isinstance(payload.get("rejections"), dict):
                return
            for rule in migrate_legacy(payload, self._legacy_confirmations):
                self._insert(rule)
            self.migrated = len(self._rules)
            backup = self.path.with_name(self.path.name + LEARNING_STORE_BACKUP_SUFFIX)
            try:
                if not backup.exists():
                    backup.write_bytes(raw)
                self.save()
            except OSError:
                return

    def save(self) -> None:
        with self._lock:
            data: _LearningData = {
                "schema_version": LEARNING_STORE_SCHEMA_VERSION,
                "rules": [_rule_to_json(rule) for rule in self._rules],
            }
            self.path.parent.mkdir(parents=True, exist_ok=True)
            temporary = self.path.with_suffix(".json.tmp")
            temporary.write_text(
                json.dumps(data, ensure_ascii=False, indent=USER_DATA_JSON_INDENT) + "\n",
                encoding="utf-8",
            )
            temporary.replace(self.path)

    def _insert(self, rule: LearnedRule) -> None:
        self._rules = [item for item in self._rules if item.identity != rule.identity]
        self._rules.append(rule)

    def rules(self) -> tuple[LearnedRule, ...]:
        """Every rule, in the order the settings list shows them."""

        with self._lock:
            stored = tuple(self._rules)
        return tuple(sorted(
            stored,
            key=lambda rule: (rule.pattern.casefold(), rule.source_group, rule.match, rule.case_sensitive),
        ))

    def count(self) -> int:
        with self._lock:
            return len(self._rules)

    def add_rule(self, rule: LearnedRule) -> LearnedRule:
        """Keep a rule; one with the same pattern and condition is replaced."""

        validate_rule(rule)
        with self._lock:
            self._insert(rule)
            self.save()
        return rule

    def replace_rule(self, old: LearnedRule, new: LearnedRule) -> LearnedRule:
        """Store an edited rule in place of the one the user opened."""

        validate_rule(new)
        with self._lock:
            self._rules = [item for item in self._rules if item != old]
            self._insert(new)
            self.save()
        return new

    def remove_rule(self, rule: LearnedRule) -> bool:
        """Forget one rule; True when it was there."""

        with self._lock:
            remaining = [item for item in self._rules if item != rule]
            if len(remaining) == len(self._rules):
                return False
            self._rules = remaining
            self.save()
            return True

    def clear(self) -> None:
        with self._lock:
            self._rules = []
            self.save()

    def match(self, source_group: int, word: str) -> LearnedRule | None:
        """The rule that decides about ``word`` typed in ``source_group``, if any."""

        if not word:
            return None
        with self._lock:
            fitting = [
                rule for rule in self._rules
                if rule.source_group == source_group and rule.matches(word)
            ]
        return max(fitting, key=LearnedRule.specificity, default=None)

    def forced_target(self, source_group: int, word: str) -> int | None:
        rule = self.match(source_group, word)
        return rule.target_group if rule is not None and rule.action == "convert" else None

    def keeps(self, source_group: int, word: str) -> bool:
        rule = self.match(source_group, word)
        return rule is not None and rule.action == "keep"

    def keeps_continuation(self, source_group: int, prefix: str) -> bool:
        """Whether a "keep" rule may still apply once the word started by ``prefix`` ends."""

        with self._lock:
            return any(
                rule.action == "keep" and rule.source_group == source_group
                and rule.may_match_continuation(prefix)
                for rule in self._rules
            )
