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

#[must_use]
pub fn match_label(rule_match: RuleMatch) -> &'static str {
    match rule_match {
        RuleMatch::Contains => "содержать данное сочетание букв",
        RuleMatch::Prefix => "начинаться с данных букв",
        RuleMatch::Exact => "совпадать с данным сочетанием",
    }
}

#[must_use]
pub fn action_label(action: RuleAction) -> &'static str {
    match action {
        RuleAction::Convert => "Переводить слово в другую раскладку",
        RuleAction::Keep => "Не переводить слово в другую раскладку",
    }
}

/// How the settings list names a condition in one short phrase.
#[must_use]
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
#[must_use]
pub fn draft_from_prompt(prompt: &LearningPrompt) -> RuleDraft {
    RuleDraft { pattern: prompt.original.clone(), action: prompt.action, application: prompt.application.clone(), ..RuleDraft::default() }
}

#[must_use]
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
#[must_use]
pub fn pattern_group(pattern: &str) -> Option<i64> {
    let (latin_letters, cyrillic_letters) = (letters_of(US_KEYS), letters_of(RU_KEYS));
    let letters: Vec<String> = pattern.chars().filter(|character| ks_text::ucd::isalpha(*character)).map(|character| pystr::casefold(&character.to_string())).collect();
    let latin = letters.iter().any(|letter| latin_letters.contains(letter));
    let cyrillic = letters.iter().any(|letter| cyrillic_letters.contains(letter));
    if latin == cyrillic || letters.iter().any(|letter| !latin_letters.contains(letter) && !cyrillic_letters.contains(letter)) {
        return None;
    }
    Some(i64::from(!latin))
}

/// The same keys as the other layout prints them.
#[must_use]
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

/// One line under the letters: which layout they are typed in and what the other prints.
#[must_use]
pub fn layout_hint(draft: &RuleDraft) -> String {
    let pattern = pystr::strip(&draft.pattern);
    let Some(group) = pattern_group(pattern) else {
        return String::new();
    };
    format!("Набрано в раскладке {}; в {} это «{}»", layout_label(group), layout_label(1 - group), other_layout_text(pattern, group))
}

#[must_use]
pub fn condition_text(rule: &LearnedRule) -> String {
    format!("{}{}", match_short_label(rule.r#match), if rule.case_sensitive { ", с учётом регистра" } else { "" })
}

#[must_use]
pub fn action_text(rule: &LearnedRule) -> String {
    if rule.action == RuleAction::Keep {
        return "не переводить".to_owned();
    }
    format!("переводить в {}", layout_label(rule.target_group))
}

/// The middle line of the offer: the word and what the rule would do with it.
#[must_use]
pub fn prompt_word_line(prompt: &LearningPrompt) -> String {
    if prompt.original.is_empty() {
        return PROMPT_EMPTY.to_owned();
    }
    if prompt.action == RuleAction::Keep {
        return format!("«{}»: не переводить", prompt.original);
    }
    format!("«{}» → «{}»: переводить", prompt.original, prompt.replacement)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    fn draft(pattern: &str) -> RuleDraft {
        RuleDraft { pattern: pattern.to_owned(), r#match: RuleMatch::Exact, case_sensitive: false, action: RuleAction::Convert, application: String::new(), editing: None }
    }

    #[test]
    fn a_pattern_is_of_one_layout_and_has_a_letter() {
        assert_eq!(pattern_group("ghbdtn"), Some(0));
        assert_eq!(pattern_group("привет"), Some(1));
        assert_eq!(pattern_group("123"), None);
        assert_eq!(pattern_group("приvет"), None);
        assert_eq!(other_layout_text("ghbdtn", 0), "привет");
        assert_eq!(other_layout_text("привет", 1), "ghbdtn");
        assert!(build_rule(&draft("123")).is_err());
        assert!(build_rule(&draft("приvет")).is_err());
        assert_eq!(build_rule(&draft(" ghbdtn ")).unwrap().target_group, 1);
        assert_eq!(layout_hint(&draft("123")), "");
        assert!(layout_hint(&draft("ghbdtn")).contains("привет"));
    }

    #[test]
    fn every_label_of_a_rule() {
        for rule_match in [RuleMatch::Contains, RuleMatch::Prefix, RuleMatch::Exact] {
            assert!(!match_short_label(rule_match).is_empty() && !match_label(rule_match).is_empty());
        }
        assert!(!action_label(RuleAction::Keep).is_empty() && !action_label(RuleAction::Convert).is_empty());
        let mut rule = LearnedRule::new("ghbdtn", 0, 1);
        assert!(action_text(&rule).starts_with("переводить"));
        assert_eq!(draft_from_rule(&rule).pattern, "ghbdtn");
        rule.case_sensitive = true;
        assert!(condition_text(&rule).contains("с учётом"));
        rule.action = RuleAction::Keep;
        assert_eq!(action_text(&rule), "не переводить");
    }

    #[test]
    fn the_offer_reads_as_what_it_would_do() {
        let mut prompt = LearningPrompt { source_group: 0, target_group: 1, original: "ghbdtn".to_owned(), replacement: "привет".to_owned(), application: String::new(), action: RuleAction::Convert };
        assert!(prompt_word_line(&prompt).ends_with("переводить"));
        prompt.action = RuleAction::Keep;
        assert!(prompt_word_line(&prompt).ends_with("не переводить"));
        assert_eq!(draft_from_prompt(&prompt).pattern, "ghbdtn");
        prompt.original.clear();
        assert_eq!(prompt_word_line(&prompt), PROMPT_EMPTY);
    }
}
