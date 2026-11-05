//! The authored scenario families (`train_context_model.build_corpus`), reused as distinct old TRAIN situations, never
//! their evaluation labels (`historical_curriculum`).
use ks_cast::To as _;
use crate::captured::role_of;
use crate::corpus::read_json;
use crate::physical::{hex, pair};
use crate::rows::{ActionRow, field_of, has_word, word_at_start};
use crate::tools::{Lexicons, Tools};
use ks_context::context_model::{ContextAction, TECHNICAL_MARKS};
use ks_lang::detector::{DecideOptions, LanguageDetector};
use ks_lang::short_words::{natural_short_source_veto, trusted_short_word_decision, trusted_short_words};
use ks_text::pystr;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use ContextAction::{Convert, Keep, Suggest, Wait};

const DETERMINISTIC_CHOICE_HEX_DIGITS: usize = 8;
const CONTEXT_V1_SPLIT_BUCKET_COUNT: u64 = 10;
const CONTEXT_V1_TRAIN_SPLIT_BUCKETS: u64 = 8;
const SHORT_WORD_MAX_CHARACTERS: usize = 2;
const CONTEXT_V1_INSIDE_WORD_MIN_CHARACTERS: usize = 4;
const CONTEXT_V1_SLASH_WORDS_PER_FAMILY: usize = 30;
const NAMESPACE: &str = "keyswitch:context-v1:candidate3";
const FIELD_ROLES: [&str; 6] = ["text", "text", "unknown", "unknown", "unknown", "unknown"];
const TERMINAL_ROLES: [&str; 4] = ["code", "terminal", "unknown", "unknown"];
const LATIN_FIELD_ROLES: [&str; 2] = ["text", "text"];
const SLASH_ROLES: [&str; 4] = ["text", "code", "unknown", "unknown"];
const COLLISION_ENGLISH_FOLLOWING: [&str; 8] = ["is not ready yet", "can wait until tomorrow", "was right about it", "is", "can", "was", "the", "and"];
const TRIGGERS4: [&str; 4] = ["space", "pause", "enter", "punctuation"];

/// A row of the corpus: what the trainer reads of its evidence.
pub struct ScenarioRow {
    original: String,
    alternative: String,
    source_group: i64,
    before: String,
    after: String,
    role: &'static str,
    trigger: &'static str,
    action: ContextAction,
    family: String,
    split: &'static str,
    category: String,
}

fn family_split(signature: &str) -> &'static str {
    let digest = crate::physical::hex_digest(&format!("{NAMESPACE}:{signature}"));
    let bucket = u64::from_str_radix(digest.get(..DETERMINISTIC_CHOICE_HEX_DIGITS).unwrap_or_default(), 16).expect("hex digits") % CONTEXT_V1_SPLIT_BUCKET_COUNT;
    if bucket < CONTEXT_V1_TRAIN_SPLIT_BUCKETS { "train" } else { "development" }
}

fn other(word: &str, group: i64) -> String {
    let (source, target) = if group == 0 { ("us", "ru") } else { ("ru", "us") };
    pair().translate(word, source, target).expect("the pair of layouts")
}

fn list(payload: &Value, name: &str) -> Vec<String> {
    payload.get(name).and_then(Value::as_array).map(|items| items.iter().map(|item| item.as_str().expect("a scenario word").to_owned()).collect()).unwrap_or_default()
}

fn context_group_of(text: &str) -> Option<i64> {
    for character in pystr::casefold(text).chars().rev() {
        if ('а'..='я').contains(&character) || character == 'ё' {
            return Some(1);
        }
        if character.is_ascii_lowercase() {
            return Some(0);
        }
    }
    None
}

struct Builder<'a> {
    detector: &'a LanguageDetector,
    models: &'a Lexicons,
    taught: HashSet<String>,
    baselines: HashMap<(String, i64, &'static str, Option<i64>), bool>,
    rows: Vec<ScenarioRow>,
}

impl Builder<'_> {
    /// `baseline_decision`: exactly what `engine.decide_word` hands to the policy.
    fn baseline_for(&mut self, word: &str, group: i64, before: &str, trigger: &'static str) -> bool {
        let context_group = context_group_of(before);
        let key = (word.to_owned(), group, trigger, context_group);
        if let Some(found) = self.baselines.get(&key) {
            return *found;
        }
        let alternate = other(word, group);
        let alternatives = [(1 - group, alternate)];
        let options = DecideOptions { trigger: crate::evidence::trigger_of(trigger), ..DecideOptions::default() };
        let decision = self.detector.decide(word, &alternatives, group, &options).expect("both lexicons are given");
        let mut decision = natural_short_source_veto(decision, context_group);
        if !decision.should_convert {
            let over = trusted_short_word_decision(self.detector, word, &alternatives, group, &[], &[], true, context_group).expect("both lexicons are given");
            if let Some(over) = over {
                decision = over;
            }
        }
        self.baselines.insert(key, decision.should_convert);
        decision.should_convert
    }

    fn add(&mut self, word: &str, group: i64, before: &str, after: &str, role: &'static str, trigger: &'static str, action: ContextAction, category: &str) {
        let alternate = other(word, group);
        let signature = pystr::casefold(word).min(pystr::casefold(&alternate));
        let split = if self.taught.contains(&signature) { "train" } else { family_split(&signature) };
        self.rows.push(ScenarioRow {
            original: word.to_owned(),
            alternative: alternate,
            source_group: group,
            before: before.to_owned(),
            after: after.to_owned(),
            role,
            trigger,
            action,
            family: signature,
            split,
            category: category.to_owned(),
        });
    }

    fn known(&self, group: i64, word: &str) -> bool {
        self.models[group.to::<usize>()].score(word).known
    }
}

/// `build_corpus` of the packaged scenarios, with the reference lexicons.
pub fn build_corpus(tools: &Tools, detector: &LanguageDetector, models: &Lexicons) -> Result<Vec<ScenarioRow>, String> {
    let payload = read_json(&tools.root.join("model/context_v1/scenarios.json"))?;
    if payload.get("schema_version").and_then(Value::as_i64) != Some(1) {
        return Err("invalid scenarios".to_owned());
    }
    let mut taught = HashSet::new();
    for word in list(&payload, "short_russian") {
        let wrong = other(&word, 1);
        taught.insert(pystr::casefold(&word).min(pystr::casefold(&wrong)));
    }
    let waits_alone: HashSet<String> = list(&payload, "short_russian_not_opening").iter().map(|word| pystr::casefold(word)).collect();
    let mut b = Builder { detector, models, taught, baselines: HashMap::new(), rows: Vec::new() };

    let russian_words_list = list(&payload, "russian");
    for word in &russian_words_list {
        let wrong = other(word, 1);
        for stray in ["q", "w", "p"] {
            for typed in [format!("{stray}{wrong}"), format!("{wrong}{stray}")] {
                for before in ["", "я думаю что "] {
                    for role in FIELD_ROLES {
                        for trigger in ["space", "enter"] {
                            b.add(&typed, 0, before, "", role, trigger, Convert, "stray_key_wrong");
                        }
                    }
                }
            }
        }
        for after in ["этого достаточно", "сегодня всё получилось", "is not ready yet", "was right about it"] {
            for before in ["", "я думаю что "] {
                for role in FIELD_ROLES {
                    b.add(word, 1, before, after, role, "space", Keep, "russian_correct_after");
                    b.add(&wrong, 0, before, after, role, "space", Convert, "russian_wrong_after");
                }
            }
        }
    }

    let english_next = ["is", "can", "was", "the", "and"];
    let russian_next = ["привет", "сегодня", "давай", "спасибо", "хорошо"];
    let english_list = list(&payload, "english");
    for (group, family, other_next, own_next) in [(1_i64, &russian_words_list, english_next, russian_next), (0, &english_list, russian_next, english_next)] {
        for word in family {
            let wrong = other(word, group);
            if b.known(1 - group, &wrong) {
                continue;
            }
            for (following, own) in other_next.iter().zip(own_next.iter()) {
                for role in FIELD_ROLES {
                    b.add(&wrong, 1 - group, "", following, role, "space", Convert, "revisit_stays");
                    b.add(word, group, "", own, role, "space", Keep, "revisit_stays_correct");
                }
            }
        }
    }

    for (name, group) in [("russian", 1_i64), ("english", 0), ("short_russian", 1), ("russian_chat", 1), ("short_english", 0), ("russian_hard", 1)] {
        for word in list(&payload, name) {
            let wrong = other(&word, group);
            let mut contexts: Vec<&str> = if group == 1 { vec!["", "я думаю что ", "подскажи пожалуйста ", "мы обсуждали это вчера "] } else { vec!["", "I think that ", "could you please ", "we discussed this yesterday "] };
            if name == "short_russian" {
                contexts.extend(["но ", "мне кажется ", "давай ", "сегодня "]);
                contexts.extend(["слушай ", "кстати ", "вот ", "короче ", "хорошо ", "спасибо ", "понятно ", "ладно ", "смотри ", "знаешь ", "а ", "ну ", "да ", "вообще ", "наверное ", "конечно "]);
            }
            let collision = matches!(name, "short_russian" | "short_english") || b.known(1 - group, &wrong);
            for before in &contexts {
                for role in FIELD_ROLES {
                    for trigger in TRIGGERS4 {
                        let mut action = Convert;
                        let opens = name == "short_russian" && !waits_alone.contains(&pystr::casefold(&word));
                        if matches!(name, "short_russian" | "russian_chat" | "short_english") && collision && before.is_empty() && !b.baseline_for(&wrong, 1 - group, "", trigger) {
                            action = if opens {
                                Convert
                            } else if matches!(trigger, "enter" | "punctuation") {
                                Suggest
                            } else {
                                Wait
                            };
                        }
                        b.add(&wrong, 1 - group, before, "", role, trigger, action, &format!("{name}_wrong"));
                        b.add(&word, group, before, "", role, trigger, Keep, &format!("{name}_correct"));
                    }
                }
            }
            if matches!(name, "short_russian" | "short_english") {
                for after in ["этого достаточно", "сегодня всё получилось", "is not ready yet", "was right about it"] {
                    let befores: Vec<&str> = if group == 1 { vec!["", contexts[1]] } else { vec![""] };
                    for before in befores {
                        for role in FIELD_ROLES {
                            b.add(&word, group, before, after, role, "space", Keep, "short_correct_after");
                        }
                    }
                }
            }
            if name == "short_russian" {
                for following in COLLISION_ENGLISH_FOLLOWING.iter().filter(|word| !word.contains(' ')) {
                    for role in FIELD_ROLES {
                        b.add(&wrong, 0, "", following, role, "space", Keep, "short_revisit");
                    }
                }
                for following in ["этого достаточно", "следующего сообщения", "сегодня всё получилось", "завтра продолжим", "меня всё устраивает", "нас это не касается", "тебя ждут в офисе", "него другое мнение", "вас получилось лучше", "них уже есть решение"] {
                    let first = pystr::split(following)[0];
                    for role in FIELD_ROLES {
                        b.add(&wrong, 0, "", following, role, "space", Convert, "short_lookahead");
                        b.add(&wrong, 0, "", first, role, "space", Convert, "short_lookahead");
                        b.add(&wrong, 0, "", first, role, "space", Convert, "short_lookahead");
                    }
                }
            } else if name == "short_english" {
                for following in ["is not ready yet", "can wait until tomorrow", "will be fine", "was right about it", "should know that", "have seen this before"] {
                    let first = pystr::split(following)[0];
                    for role in FIELD_ROLES {
                        b.add(&wrong, 1, "", following, role, "space", Convert, "short_lookahead");
                        b.add(&wrong, 1, "", first, role, "space", Convert, "short_lookahead");
                        b.add(&wrong, 1, "", first, role, "space", Convert, "short_lookahead");
                    }
                }
                for following in COLLISION_ENGLISH_FOLLOWING.iter().filter(|word| !word.contains(' ')) {
                    for role in FIELD_ROLES {
                        b.add(&wrong, 1, "", following, role, "space", Convert, "short_english_revisit");
                    }
                }
            } else if name == "russian_chat" && collision {
                for following in ["в", "не", "раз", "тест", "работает", "можно", "будет", "немного", "надо", "есть"] {
                    for role in FIELD_ROLES {
                        b.add(&wrong, 0, "", following, role, "space", Convert, "chat_lookahead");
                        b.add(&wrong, 0, "", following, role, "space", Convert, "chat_lookahead");
                        b.add(&wrong, 0, "", "", role, "space", Wait, "chat_lookahead_alone");
                    }
                }
            }
            // A code field must not override the actual language of its comments.
            b.add(&wrong, 1 - group, &format!("// {}", contexts[1]), "", "code", "space", Convert, "code_comment");
            // A legitimate English insertion inside Russian prose is not a layout error.
            if group == 0 {
                b.add(&word, 0, "в сообщении написано ", "", "text", "space", Keep, "mixed_prose");
            }
        }
    }

    let latin_fields = list(&payload, "latin_fields");
    let latin_tails = list(&payload, "latin_tails");
    let mut russian_words: Vec<String> = Vec::new();
    for name in ["russian_chat", "short_russian", "russian"] {
        russian_words.extend(list(&payload, name));
    }
    let english_words = list(&payload, "english");
    let count = russian_words.len().min(english_words.len());
    for field in &latin_fields {
        for tail in &latin_tails {
            let before = format!("{field}{tail}");
            for role in LATIN_FIELD_ROLES {
                let words = russian_words[..count].iter().map(|word| (word, 1_i64)).chain(english_words[..count].iter().map(|word| (word, 0_i64)));
                for (word, group) in words {
                    b.add(word, group, &before, "", role, "space", Keep, "latin_field_correct");
                    let wrong = other(word, group);
                    if pystr::len(word) > SHORT_WORD_MAX_CHARACTERS && !b.known(1 - group, &wrong) {
                        b.add(&wrong, 1 - group, &before, "", role, "space", Convert, "latin_field_wrong");
                        if group == 1 {
                            b.add(word, 1, &before, "", role, "pause", Keep, "latin_field_correct");
                            b.add(&wrong, 0, &before, "", role, "pause", Convert, "latin_field_wrong");
                        }
                    }
                    if group == 1 && !tail.is_empty() && pystr::len(word) > SHORT_WORD_MAX_CHARACTERS && !b.known(0, &wrong) {
                        b.add(word, 1, &before, "сегодня всё получилось", role, "pause", Keep, "latin_field_correct_after");
                        b.add(&wrong, 0, &before, "сегодня всё получилось", role, "pause", Convert, "latin_field_wrong_after");
                    }
                }
            }
        }
    }

    for word in list(&payload, "russian_collisions") {
        for before in ["я думаю что ", "подскажи пожалуйста ", "мы обсуждали это вчера "] {
            for role in FIELD_ROLES {
                for trigger in TRIGGERS4 {
                    b.add(&word, 1, before, "", role, trigger, Keep, "collision_correct");
                }
            }
        }
        for field in &latin_fields {
            for tail in &latin_tails {
                for role in LATIN_FIELD_ROLES {
                    b.add(&word, 1, &format!("{field}{tail}"), "", role, "space", Keep, "collision_latin_field");
                }
            }
        }
        for before in ["", "I think that ", "we discussed this yesterday "] {
            for following in COLLISION_ENGLISH_FOLLOWING {
                for role in ["text", "unknown"] {
                    b.add(&word, 1, before, following, role, "space", Convert, "collision_english_after");
                    if !following.contains(' ') {
                        b.add(&word, 1, before, following, role, "space", Convert, "collision_english_after");
                    }
                }
            }
        }
    }

    // The runtime's curated trusted short-word list decides a handful of two-letter tokens on its own, in both directions.
    for target_group in [0_i64, 1] {
        let mut trusted: Vec<&str> = trusted_short_words(target_group);
        trusted.sort_unstable();
        for word in trusted {
            let wrong = other(word, target_group);
            let phrases: [&str; 3] = if target_group == 1 { ["", "я думаю что ", "мы обсуждали это вчера "] } else { ["", "I think that ", "we discussed this yesterday "] };
            for before in phrases {
                for role in FIELD_ROLES {
                    for trigger in TRIGGERS4 {
                        let converts = b.baseline_for(&wrong, 1 - target_group, before, trigger);
                        let opening = target_group == 1 && !waits_alone.contains(&pystr::casefold(word));
                        let action = if converts || !before.is_empty() || opening {
                            Convert
                        } else if matches!(trigger, "enter" | "punctuation") {
                            Suggest
                        } else {
                            Wait
                        };
                        b.add(&wrong, 1 - target_group, before, "", role, trigger, action, "trusted_short_wrong");
                        b.add(word, target_group, before, "", role, trigger, Keep, "trusted_short_correct");
                    }
                }
            }
        }
    }

    let technical_contexts = ["запусти ", "введи команду ", "const value = ", "return ", "$ ", "the command is "];
    for token in list(&payload, "technical") {
        let wrong = other(&token, 0);
        let letters = crate::rows::alpha_only(&wrong);
        let stays = pystr::len(&token) > SHORT_WORD_MAX_CHARACTERS && b.known(1, &letters);
        let marked = |text: &str| text.chars().any(|character| TECHNICAL_MARKS.contains(character));
        let mut befores: Vec<&str> = Vec::new();
        if stays || marked(&token) {
            befores.push("");
        }
        befores.extend(technical_contexts);
        for before in befores {
            for role in TERMINAL_ROLES {
                for trigger in TRIGGERS4 {
                    b.add(&token, 0, before, "", role, trigger, Keep, "technical");
                }
            }
        }
        if pystr::len(&token) > SHORT_WORD_MAX_CHARACTERS && !stays && !marked(&wrong) {
            let mut befores = vec![""];
            befores.extend(technical_contexts);
            for before in befores {
                for role in TERMINAL_ROLES {
                    for trigger in TRIGGERS4 {
                        b.add(&wrong, 1, before, "", role, trigger, Convert, "technical_wrong");
                    }
                }
            }
        }
    }

    let unknown_families: [(&str, i64, &[&str], &[&str]); 4] = [
        ("technical_terms", 0, &["", "запусти ", "далее запускается процесс ", "$ "], &TERMINAL_ROLES),
        ("dotted", 0, &["", "открой файл ", "нужно поправить ", "$ cat "], &TERMINAL_ROLES),
        ("english_unknown", 0, &["", "I think that ", "could you please ", "в сообщении написано "], &FIELD_ROLES),
        ("russian_unknown", 1, &["", "я думаю что ", "нужно срочно ", "мы обсуждали это вчера ", "подскажи пожалуйста "], &FIELD_ROLES),
    ];
    for (name, group, contexts, roles) in unknown_families {
        for token in list(&payload, name) {
            let wrong = other(&token, group);
            for before in contexts {
                for role in roles {
                    for trigger in TRIGGERS4 {
                        let mut action = Convert;
                        if pystr::len(&token) <= SHORT_WORD_MAX_CHARACTERS && before.is_empty() && name != "russian_unknown" && !b.baseline_for(&wrong, 1 - group, "", trigger) {
                            action = if matches!(trigger, "enter" | "punctuation") { Suggest } else { Wait };
                        }
                        b.add(&wrong, 1 - group, before, "", role, trigger, action, &format!("{name}_wrong"));
                        b.add(&token, group, before, "", role, trigger, Keep, &format!("{name}_correct"));
                    }
                }
            }
        }
    }

    let short_unknown = list(&payload, "short_russian_unknown");
    for word in &short_unknown {
        for before in ["я думаю что ", "нужно срочно ", "мы обсуждали это вчера ", "подскажи пожалуйста "] {
            for role in FIELD_ROLES {
                for trigger in TRIGGERS4 {
                    b.add(word, 1, before, "", role, trigger, Keep, "short_russian_unknown_correct");
                }
            }
        }
    }
    let short_russian = list(&payload, "short_russian");
    let short_english = list(&payload, "short_english");
    for field in &latin_fields {
        for tail in &latin_tails {
            let before = format!("{field}{tail}");
            for role in LATIN_FIELD_ROLES {
                for word in &short_unknown {
                    b.add(word, 1, &before, "", role, "space", Keep, "latin_field_short_unknown");
                }
                if tail.is_empty() {
                    for word in &short_russian {
                        let wrong = other(word, 1);
                        let action = if pystr::len(word) == 1 { Wait } else { Convert };
                        b.add(&wrong, 0, field, "", role, "space", action, "latin_field_short_wrong");
                    }
                }
                for word in &short_english {
                    b.add(word, 0, &before, "", role, "space", Keep, "latin_field_short_correct");
                    if tail.is_empty() {
                        let wrong = other(word, 0);
                        b.add(&wrong, 1, field, "", role, "space", Convert, "latin_field_short_wrong");
                    }
                }
            }
        }
    }

    let inside_contexts = list(&payload, "inside_contexts");
    let inside_after = payload.get("inside_after").cloned().unwrap_or(json!({}));
    for (name, group) in [("russian", 1_i64), ("russian_hard", 1), ("russian_unknown", 1), ("english", 0), ("technical_terms", 0)] {
        let tail_after = inside_after.get(if group == 1 { "russian" } else { "english" }).and_then(Value::as_str).unwrap_or("").to_owned();
        for word in list(&payload, name) {
            if pystr::len(&word) < CONTEXT_V1_INSIDE_WORD_MIN_CHARACTERS {
                continue;
            }
            let wrong = other(&word, group);
            for before in &inside_contexts {
                for after in ["", tail_after.as_str()] {
                    for trigger in ["pause", "space"] {
                        b.add(&wrong, 1 - group, before, after, "text", trigger, Convert, "inside_wrong");
                    }
                }
            }
        }
    }

    let heads = payload.get("slash_heads").cloned().unwrap_or(json!({}));
    let mut prefixes: Vec<String> = Vec::new();
    for name in ["russian", "english"] {
        prefixes.extend(list(&heads, name));
    }
    for (name, group) in [("russian", 1_i64), ("english", 0), ("technical_terms", 0)] {
        let words = list(&payload, name);
        let limit = if name == "technical_terms" { words.len() } else { words.len().min(CONTEXT_V1_SLASH_WORDS_PER_FAMILY) };
        for word in &words[..limit] {
            let wrong = other(word, group);
            for before in &prefixes {
                for role in SLASH_ROLES {
                    b.add(&wrong, 1 - group, before, "", role, "space", Convert, "slash_wrong");
                    b.add(word, group, before, "", role, "space", Keep, "slash_correct");
                }
            }
        }
    }
    Ok(b.rows)
}

fn boundaries(trigger: &str) -> &'static [&'static str] {
    match trigger {
        "space" => &[" "],
        "pause" => &[""],
        "enter" => &["", "\n"],
        "tab" => &["", "\t"],
        "punctuation" => &["."],
        _ => panic!("unsupported historical action trigger"),
    }
}

/// `historical_curriculum`: distinct old TRAIN situations, equal mass for each represented action of a parent.
pub fn historical_curriculum(tools: &Tools, detector: &LanguageDetector, models: &Lexicons) -> Result<Vec<ActionRow>, String> {
    let curriculum = build_corpus(tools, detector, models)?;
    let mut result: Vec<ActionRow> = Vec::new();
    let mut selected: HashSet<[u8; 32]> = HashSet::new();
    let mut parents: Vec<usize> = Vec::new();
    let mut parent_ids: HashMap<(String, String, &'static str), usize> = HashMap::new();
    let mut variants: HashMap<(usize, ContextAction), i64> = HashMap::new();
    let mut actions: HashMap<usize, HashSet<ContextAction>> = HashMap::new();
    for row in &curriculum {
        if row.split != "train" {
            continue;
        }
        let mut afters: Vec<&str> = vec![row.after.as_str()];
        if !row.after.is_empty()
            && let Some(completed) = word_at_start(pystr::lstrip(&row.after))
                && completed != row.after {
                    afters.push(completed);
                }
        for after in afters {
            for boundary in boundaries(row.trigger) {
                let key: [u8; 32] = {
                    let bytes = crate::artifact::canonical(&json!([row.family, row.category, row.action.as_str(), row.original, row.source_group, row.before, after, "", role_of(row.role).as_str(), row.trigger, boundary]));
                    Sha256::digest(&bytes).into()
                };
                if !selected.insert(key) {
                    continue;
                }
                let next = parent_ids.len();
                let parent = *parent_ids.entry((row.family.clone(), row.category.clone(), row.trigger)).or_insert(next);
                let mut action = row.action;
                if action == Keep && tools.deferred_isolated(&row.original, &row.alternative, row.source_group) && !has_word(&row.before) && !has_word(after) {
                    // A correct reading with no neighbouring word has no observable intent label either.
                    action = if matches!(row.trigger, "enter" | "tab" | "punctuation") { Suggest } else { Wait };
                }
                *variants.entry((parent, action)).or_insert(0) += 1;
                actions.entry(parent).or_default().insert(action);
                let field = field_of("", "training-field", &row.before, after, role_of(row.role));
                let mut frame = ActionRow::new(format!("legacy-train:{}", hex(&key)), row.original.as_str(), row.source_group, field, row.trigger, action, format!("legacy_{}", row.category), *boundary);
                frame.parent_family.clone_from(&row.family);
                parents.push(parent);
                result.push(frame);
            }
        }
    }
    // Keep the fixed parent budget and equal mass for each represented action.
    for (frame, parent) in result.iter_mut().zip(parents) {
        frame.sample_weight = 1.0 / (actions[&parent].len().to::<f64>() * (variants[&(parent, frame.action)]).to::<f64>());
    }
    Ok(result)
}
