//! What the rule window and the rule offer show, apart from any toolkit.
//!
//! The windows draw the same form: the letters, the condition, case sensitivity and the action. This module fills the
//! form from the offer a double press of the conversion hotkey made, checks it and turns it into a `LearnedRule`.

use crate::engine::LearningPrompt;
use ks_config::indicator::layout_label;
use ks_config::learning::{validate_rule, InvalidRule, LearnedRule, RuleAction, RuleMatch};
use ks_text::layouts::{LayoutPair, RU_KEYS, US_KEYS};
use ks_text::pystr;

pub const WINDOW_TITLE: &str = "Правило переключения";
pub const PATTERN_LABEL: &str = "Сочетание букв, как оно набирается:";
pub const CONDITION_TITLE: &str = "Условие";
pub const CONDITION_LEAD: &str = "Набираемое на клавиатуре слово должно:";
pub const EXTRA_LEAD: &str = "Дополнительно:";
pub const CASE_LABEL: &str = "учитывать регистр";
pub const ACTION_TITLE: &str = "Действие";
pub const PROMPT_TITLE: &str = "Добавить правило переключения?";
pub const PROMPT_HINT: &str = "Enter - открыть правило    Esc - закрыть";
pub const PROMPT_EMPTY: &str = "Слова нет: откроется пустое правило";

pub fn match_label(rule_match: RuleMatch) -> &'static str {
    match rule_match {
        RuleMatch::Contains => "содержать данное сочетание букв",
        RuleMatch::Prefix => "начинаться с данных букв",
        RuleMatch::Exact => "совпадать с данным сочетанием",
    }
}

pub fn action_label(action: RuleAction) -> &'static str {
    match action {
        RuleAction::Convert => "Переводить слово в другую раскладку",
        RuleAction::Keep => "Не переводить слово в другую раскладку",
    }
}

/// How the settings list names a condition in one short phrase.
pub fn match_short_label(rule_match: RuleMatch) -> &'static str {
    match rule_match {
        RuleMatch::Contains => "содержит",
        RuleMatch::Prefix => "начинается с",
        RuleMatch::Exact => "совпадает",
    }
}

/// The state of the form; `editing` is the stored rule it was opened for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleDraft {
    pub pattern: String,
    pub r#match: RuleMatch,
    pub case_sensitive: bool,
    pub action: RuleAction,
    pub application: String,
    pub editing: Option<LearnedRule>,
}

impl Default for RuleDraft {
    fn default() -> Self {
        RuleDraft { pattern: String::new(), r#match: RuleMatch::Exact, case_sensitive: false, action: RuleAction::Convert, application: String::new(), editing: None }
    }
}

/// The form as the offer fills it: the word, as a whole, with its action.
pub fn draft_from_prompt(prompt: &LearningPrompt) -> RuleDraft {
    RuleDraft { pattern: prompt.original.clone(), action: prompt.action, application: prompt.application.clone(), ..RuleDraft::default() }
}

pub fn draft_from_rule(rule: &LearnedRule) -> RuleDraft {
    RuleDraft {
        pattern: rule.pattern.clone(),
        r#match: rule.r#match,
        case_sensitive: rule.case_sensitive,
        action: rule.action,
        application: String::new(),
        editing: Some(rule.clone()),
    }
}

fn letters_of(keys: &str) -> Vec<String> {
    keys.chars().filter(|key| ks_text::ucd::isalpha(*key)).map(|key| key.to_string()).collect()
}

/// The layout the letters were typed in: Latin is EN, Cyrillic is RU.
///
/// Punctuation and digits say nothing; letters of both alphabets cannot come from one layout, so such a pattern has no
/// layout at all.
pub fn pattern_group(pattern: &str) -> Option<i64> {
    let (latin_letters, cyrillic_letters) = (letters_of(US_KEYS), letters_of(RU_KEYS));
    let letters: Vec<String> = pattern.chars().filter(|character| ks_text::ucd::isalpha(*character)).map(|character| pystr::casefold(&character.to_string())).collect();
    let latin = letters.iter().any(|letter| latin_letters.contains(letter));
    let cyrillic = letters.iter().any(|letter| cyrillic_letters.contains(letter));
    if latin == cyrillic || letters.iter().any(|letter| !latin_letters.contains(letter) && !cyrillic_letters.contains(letter)) {
        return None;
    }
    Some(if latin { 0 } else { 1 })
}

/// The same keys as the other layout prints them.
pub fn other_layout_text(pattern: &str, group: i64) -> String {
    let pair = LayoutPair::default();
    let (source, target) = if group == 0 { ("us", "ru") } else { ("ru", "us") };
    pair.translate(pattern, source, target).expect("the us/ru pair translates")
}

/// The rule the form describes; `InvalidRule` says what to fix.
pub fn build_rule(draft: &RuleDraft) -> Result<LearnedRule, InvalidRule> {
    let pattern = pystr::strip(&draft.pattern);
    let group = pattern_group(pattern);
    if !pattern.chars().any(ks_text::ucd::isalpha) {
        return Err(InvalidRule("В сочетании нужна хотя бы одна буква".to_owned()));
    }
    let Some(group) = group else {
        return Err(InvalidRule("Наберите сочетание буквами одной раскладки: латиницей или кириллицей".to_owned()));
    };
    // The other of the two layouts the program works with.
    let rule = LearnedRule { action: draft.action, r#match: draft.r#match, case_sensitive: draft.case_sensitive, ..LearnedRule::new(pattern, group, 1 - group) };
    validate_rule(&rule)?;
    Ok(rule)
}

/// What stops the form from becoming a rule, or an empty string.
pub fn draft_problem(draft: &RuleDraft) -> String {
    build_rule(draft).err().map_or_else(String::new, |error| error.0)
}

/// One line under the letters: which layout they are typed in and what the other prints.
pub fn layout_hint(draft: &RuleDraft) -> String {
    let pattern = pystr::strip(&draft.pattern);
    let Some(group) = pattern_group(pattern) else {
        return String::new();
    };
    format!("Набрано в раскладке {}; в {} это «{}»", layout_label(group), layout_label(1 - group), other_layout_text(pattern, group))
}

pub fn condition_text(rule: &LearnedRule) -> String {
    format!("{}{}", match_short_label(rule.r#match), if rule.case_sensitive { ", с учётом регистра" } else { "" })
}

pub fn action_text(rule: &LearnedRule) -> String {
    if rule.action == RuleAction::Keep {
        return "не переводить".to_owned();
    }
    format!("переводить в {}", layout_label(rule.target_group))
}

/// The middle line of the offer: the word and what the rule would do with it.
pub fn prompt_word_line(prompt: &LearningPrompt) -> String {
    if prompt.original.is_empty() {
        return PROMPT_EMPTY.to_owned();
    }
    if prompt.action == RuleAction::Keep {
        return format!("«{}»: не переводить", prompt.original);
    }
    format!("«{}» → «{}»: переводить", prompt.original, prompt.replacement)
}
