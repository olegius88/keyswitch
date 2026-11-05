//! The public text the typing bench types and the tables counted from it: messages of ru.stackoverflow, Tatoeba and
//! UD Russian-Taiga (`mixed_typing messages`), how often words occur (`terms`, `alnum`, `short`), whose text the captured
//! files hold (`sources`), the messages of a class for one capture (`select_capture_messages`) and the fresh comments of
//! the developer-chat test (`build_devchat_messages`).
use ks_cast::To as _;
use crate::corpus::{checksum, read_json};
use crate::mixed::{jobs_recipe_messages, recipe_lines_above, recipe_previous_line};
use crate::physical::{hex, hex_digest, pair};
use flate2::Compression;
use flate2::read::MultiGzDecoder;
use flate2::write::GzEncoder;
use indexmap::IndexMap;
use ks_context::constants::{ALNUM_HEAD_MAX_CHARACTERS, OPENING_SHORT_MAX_LETTERS, RESUME_LEADING_SIGNS};
use ks_context::context_action_features::{dotted_question, letters_and_digits, sign_token_asked};
use ks_lang::short_words::TRUSTED_SINGLE_LETTER_WORDS;
use ks_lang::Resources;
use ks_text::{Indent, JsonOptions, json_dumps, pystr};
use regex::Regex;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

const MIN_TOKENS: usize = 3;
const MAX_TOKENS: usize = 24;
const SPLIT_HEX_DIGITS: usize = 8;
const SPLIT_BUCKET_COUNT: u64 = 100;
const TEST_SPLIT_BUCKETS: u64 = 10;
const DEVELOPMENT_SPLIT_BUCKETS: u64 = 20;
const CODE_LINE_MIN_CHARACTERS: usize = 10;
const CODE_LINE_MAX_CHARACTERS: usize = 100;
const TATOEBA_AUTHOR_COLUMN: usize = 3;
const TATOEBA_MIN_TOKENS: usize = 2;
const TATOEBA_MAX_TOKENS: usize = 12;
const LEIPZIG_WORDS_COLUMNS: usize = 3;
const SHORT_HEAD_LETTERS: [usize; 2] = [2, 3];
const TERM_FREQUENCY_LEAST: i64 = 5;
const TERM_EDGE_SIGNS: &str = ".,!?:;\"'()[]{}<>«»-";
const TRAILING_SIGNS: &str = ".,;:!?)";
const ALNUM_EDGE_SIGNS: &str = ".,;:!?()\"'«»";
const LAYOUT_ALIKE_SIGNS: &str = "-=!%*()_+";
const CHOICE_HEX_DIGITS: usize = 8;
const XZ_PRESET: u32 = 9;
const SPLITS: [&str; 3] = ["train", "dev", "test"];
const DEVCHAT_NAMESPACE: &str = "keyswitch:devchat-2026-10-15:question";
const DEVCHAT_DUMP_SHA256: &str = "e3f6eda2b3df7de1d3cc4a45ce2e69bb1e439b7736cd8d81e911129bd512dab9";
const DEVCHAT_CODE_SIGNS: &str = "\\{}|^$";
const DEVCHAT_PROSE_SIGNS: &str = ".,!?():-\"'";
const DEVCHAT_SIGN_RUN: usize = 3;
const SO_NAMESPACE: &str = "keyswitch:research-2026-09-29:ru_stackoverflow";
const TATOEBA_NAMESPACE: &str = "keyswitch:research-2026-09-29:tatoeba";
const ENTITIES: &str = include_str!("../data/html-entities.json");

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).expect("a valid pattern")
}

static CODE_FENCE: LazyLock<Regex> = LazyLock::new(|| regex(r"(?s)```.*?```"));
static CODE_FENCE_BODY: LazyLock<Regex> = LazyLock::new(|| regex(r"(?s)```[^\n]*\n(.*?)```"));
static INDENTED: LazyLock<Regex> = LazyLock::new(|| regex(r"(?m)^(?: {4}|\t).*$"));
static LINK: LazyLock<Regex> = LazyLock::new(|| regex(r"!?\[([^\]]*)\]\([^)]*\)"));
static URL: LazyLock<Regex> = LazyLock::new(|| regex(r"(?:https?://|www\.)[^\s\x1c-\x1f]+"));
static TAG: LazyLock<Regex> = LazyLock::new(|| regex(r"<[^>]+>"));
static PARAGRAPH: LazyLock<Regex> = LazyLock::new(|| regex(r"\n[\s\x1c-\x1f]*\n|\n"));
static CHARREF: LazyLock<Regex> = LazyLock::new(|| regex(r"&(#[0-9]+;?|#[xX][0-9a-fA-F]+;?|[^\t\n\x0c <&#;]{1,32};?)"));
static LATIN_PAIR: LazyLock<Regex> = LazyLock::new(|| regex(r"[A-Za-z]{2}"));
static DASH_TOKEN: LazyLock<Regex> = LazyLock::new(|| regex(r"^-{1,2}(?:[A-Za-z]+|[а-яёА-ЯЁ]+)[0-9]*$"));
static DIGIT_DOT_TOKEN: LazyLock<Regex> = LazyLock::new(|| regex(r"^[0-9]+[.,][0-9]*$"));
static DIGIT_LETTER_TOKEN: LazyLock<Regex> = LazyLock::new(|| regex(r"^[0-9]+б$"));
static SIGN_RUN: LazyLock<Regex> = LazyLock::new(|| regex(&format!(r"[!-/:-@\[-`{{-~]{{{DEVCHAT_SIGN_RUN},}}")));

struct Entities {
    named: HashMap<String, String>,
    invalid_charrefs: HashMap<u32, String>,
    invalid_codepoints: HashSet<u32>,
}

static HTML: LazyLock<Entities> = LazyLock::new(|| {
    let value: Value = serde_json::from_str(ENTITIES).expect("the entities table");
    let text = |value: &Value| value.as_str().unwrap_or("").to_owned();
    Entities {
        named: value["html5"].as_object().expect("names").iter().map(|(name, value)| (name.clone(), text(value))).collect(),
        invalid_charrefs: value["invalid_charrefs"].as_object().expect("charrefs").iter().map(|(name, value)| (name.parse().expect("a number"), text(value))).collect(),
        invalid_codepoints: value["invalid_codepoints"].as_array().expect("codepoints").iter().filter_map(Value::as_u64).map(ks_cast::To::to::<u32>).collect(),
    }
});

/// `html.unescape`.
pub fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    CHARREF
        .replace_all(text, |captures: &regex::Captures<'_>| {
            let reference = &captures[1];
            if let Some(number) = reference.strip_prefix('#') {
                let core = number.trim_end_matches(';');
                let value = match core.strip_prefix(['x', 'X']) {
                    Some(digits) => u64::from_str_radix(digits, 16),
                    None => core.parse::<u64>(),
                };
                let Ok(value) = value else { return "\u{fffd}".to_owned() };
                let Ok(code) = u32::try_from(value) else { return "\u{fffd}".to_owned() };
                if let Some(replacement) = HTML.invalid_charrefs.get(&code) {
                    return replacement.clone();
                }
                if (0xD800..=0xDFFF).contains(&code) || code > 0x0010_FFFF {
                    return "\u{fffd}".to_owned();
                }
                if HTML.invalid_codepoints.contains(&code) {
                    return String::new();
                }
                return char::from_u32(code).map_or_else(String::new, String::from);
            }
            if let Some(found) = HTML.named.get(reference) {
                return found.clone();
            }
            let characters: Vec<char> = reference.chars().collect();
            for end in (2..characters.len()).rev() {
                let name: String = characters[..end].iter().collect();
                if let Some(found) = HTML.named.get(&name) {
                    let rest: String = characters[end..].iter().collect();
                    return format!("{found}{rest}");
                }
            }
            format!("&{reference}")
        })
        .into_owned()
}

fn text_of(character: char) -> String {
    character.to_string()
}

fn is_space(character: char) -> bool {
    pystr::isspace(&text_of(character))
}

/// `\w` of a Python pattern over text.
fn is_word(character: char) -> bool {
    character == '_' || pystr::isalnum(&text_of(character))
}

fn is_cyrillic_letter(character: char) -> bool {
    ('а'..='я').contains(&character) || ('А'..='Я').contains(&character) || character == 'ё' || character == 'Ё'
}

/// `(?:^|\s)@[\w.\-]+`, each replaced by a space.
fn without_mentions(text: &str) -> String {
    let characters: Vec<char> = text.chars().collect();
    let body = |character: char| is_word(character) || character == '.' || character == '-';
    let mut output = String::with_capacity(text.len());
    let mut index = 0;
    while index < characters.len() {
        let start = if index == 0 && characters[0] == '@' {
            Some(0)
        } else if is_space(characters[index]) && characters.get(index + 1) == Some(&'@') {
            Some(index + 1)
        } else {
            None
        };
        if let Some(at) = start
            && characters.get(at + 1).is_some_and(|character| body(*character))
        {
            let mut end = at + 1;
            while end < characters.len() && body(characters[end]) {
                end += 1;
            }
            output.push(' ');
            index = end;
            continue;
        }
        output.push(characters[index]);
        index += 1;
    }
    output
}

/// `text.translate(TYPEABLE)`.
fn typeable(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '«' | '»' | '“' | '”' | '„' => output.push('"'),
            '’' | '‘' => output.push('\''),
            '—' | '–' => output.push('-'),
            '…' => output.push_str("..."),
            '\u{a0}' | '\u{2009}' | '\u{202f}' => output.push(' '),
            other => output.push(other),
        }
    }
    output
}

/// `clean`.
fn clean(text: &str) -> String {
    let text = CODE_FENCE.replace_all(text, " ");
    let text = INDENTED.replace_all(&text, " ");
    let text = LINK.replace_all(&text, "$1");
    let text = URL.replace_all(&text, " ");
    let unescaped = unescape(&text);
    let text = TAG.replace_all(&unescaped, " ");
    let text = without_mentions(&text);
    let text = text.replace('`', "").replace("**", "").replace("__", " ");
    typeable(&text)
}

fn token_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || is_cyrillic_letter(character) || ".,!?:;-()'\"/_=+*<>[]{}#@$%&|\\~^`".contains(character)
}

fn has_cyrillic(text: &str) -> bool {
    text.chars().any(is_cyrillic_letter)
}

fn has_latin(text: &str) -> bool {
    text.chars().any(|character| character.is_ascii_alphabetic())
}

fn typeable_tokens(tokens: &[&str]) -> bool {
    tokens.iter().all(|token| !token.is_empty() && token.chars().all(token_character) && !(has_cyrillic(token) && has_latin(token)))
}

/// `SENTENCE.split`: at the white space after a sentence's last sign.
fn sentences(text: &str) -> Vec<String> {
    let characters: Vec<char> = text.chars().collect();
    let mut parts = Vec::new();
    let (mut start, mut index) = (0, 0);
    while index < characters.len() {
        if is_space(characters[index]) && index > 0 && matches!(characters[index - 1], '.' | '!' | '?') {
            parts.push(characters[start..index].iter().collect());
            while index < characters.len() && is_space(characters[index]) {
                index += 1;
            }
            start = index;
        } else {
            index += 1;
        }
    }
    parts.push(characters[start..].iter().collect());
    parts
}

/// `post_messages`: messages of one post: typeable, with Cyrillic, of a chat message's length.
pub fn post_messages(text: &str) -> Vec<Vec<String>> {
    let cleaned = clean(text);
    let mut output = Vec::new();
    for paragraph in PARAGRAPH.split(&cleaned) {
        let paragraph = pystr::strip(pystr::lstrip_chars(pystr::strip(paragraph), "#>*- "));
        if paragraph.is_empty() {
            continue;
        }
        let parts = if pystr::split(paragraph).len() <= MAX_TOKENS { vec![paragraph.to_owned()] } else { sentences(paragraph) };
        for part in &parts {
            let tokens = pystr::split(part);
            if (MIN_TOKENS..=MAX_TOKENS).contains(&tokens.len()) && typeable_tokens(&tokens) && tokens.iter().any(|token| has_cyrillic(token)) {
                output.push(tokens.iter().map(|token| (*token).to_owned()).collect());
            }
        }
    }
    output
}

fn code_lines(markdown: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for captures in CODE_FENCE_BODY.captures_iter(markdown) {
        lines.extend(captures[1].split('\n').map(str::to_owned));
    }
    lines.extend(markdown.split('\n').filter_map(|line| line.strip_prefix("    ")).map(str::to_owned));
    lines
        .into_iter()
        .filter_map(|line| {
            let stripped = pystr::strip(&line);
            let length = stripped.chars().count();
            let printable = !stripped.is_empty() && stripped.chars().all(|character| (' '..='~').contains(&character));
            ((CODE_LINE_MIN_CHARACTERS..=CODE_LINE_MAX_CHARACTERS).contains(&length) && printable && LATIN_PAIR.is_match(&line)).then(|| stripped.to_owned())
        })
        .collect()
}

fn split_of(namespace: &str, identifier: &str) -> &'static str {
    let digest = hex_digest(&format!("{namespace}:{identifier}"));
    let bucket = u64::from_str_radix(digest.get(..SPLIT_HEX_DIGITS).unwrap_or_default(), 16).expect("a digest") % SPLIT_BUCKET_COUNT;
    if bucket < TEST_SPLIT_BUCKETS {
        "test"
    } else if bucket < DEVELOPMENT_SPLIT_BUCKETS {
        "dev"
    } else {
        "train"
    }
}

/// Lines as a text-mode read gives them: `\r\n` and `\r` are `\n`.
fn universal_lines(text: &str) -> Vec<String> {
    text.replace("\r\n", "\n").replace('\r', "\n").split_inclusive('\n').map(|line| line.strip_suffix('\n').unwrap_or(line).to_owned()).collect()
}

struct GzWriter {
    encoder: GzEncoder<std::fs::File>,
}

impl GzWriter {
    fn create(path: &Path) -> Result<GzWriter, String> {
        Ok(GzWriter { encoder: GzEncoder::new(std::fs::File::create(path).map_err(|error| format!("{}: {error}", path.display()))?, Compression::new(9)) })
    }

    fn line(&mut self, line: &str) -> Result<(), String> {
        self.encoder.write_all(line.as_bytes()).and_then(|()| self.encoder.write_all(b"\n")).map_err(|error| error.to_string())
    }

    fn close(self) -> Result<(), String> {
        self.encoder.finish().map(|_| ()).map_err(|error| error.to_string())
    }
}

fn gz_text(path: &Path) -> Result<String, String> {
    let mut text = String::new();
    MultiGzDecoder::new(std::fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?).read_to_string(&mut text).map_err(|error| error.to_string())?;
    Ok(text)
}

fn bz2_text(path: &Path) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut text = String::new();
    bzip2_rs::DecoderReader::new(BufReader::new(file)).read_to_string(&mut text).map_err(|error| error.to_string())?;
    Ok(text)
}

fn row_line(value: &Value) -> String {
    json_dumps(value, &JsonOptions { ensure_ascii: false, ..JsonOptions::default() })
}

fn mix_of(tokens: &[&str]) -> &'static str {
    if tokens.iter().any(|token| has_latin(token)) { "mixed" } else { "russian" }
}

fn count_up(counts: &mut BTreeMap<String, i64>, key: String) {
    *counts.entry(key).or_insert(0) += 1;
}

/// `build_taiga`: sentences of UD Russian-Taiga by its own parts.
pub fn build_taiga(taiga: &Path, out: &Path) -> Result<BTreeMap<String, i64>, String> {
    let mut counts = BTreeMap::new();
    let mut train: Vec<PathBuf> = std::fs::read_dir(taiga).map_err(|error| error.to_string())?.filter_map(|entry| entry.ok().map(|entry| entry.path())).filter(|path| {
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        name.starts_with("ru_taiga-ud-train-") && name.ends_with(".conllu")
    }).collect();
    train.sort();
    for lower in [false, true] {
        let name = if lower { "taiga-chat" } else { "taiga" };
        let kind = name.replace('-', "_");
        for (part, files) in [("train", train.clone()), ("dev", vec![taiga.join("ru_taiga-ud-dev.conllu")]), ("test", vec![taiga.join("ru_taiga-ud-test.conllu")])] {
            let mut number = 0;
            let mut writer = GzWriter::create(&out.join(format!("{name}-{part}.jsonl.gz")))?;
            for path in files {
                let mut sentence_id = String::new();
                for line in universal_lines(&std::fs::read_to_string(&path).map_err(|error| error.to_string())?) {
                    if let Some(rest) = line.strip_prefix("# sent_id = ") {
                        pystr::strip(rest).clone_into(&mut sentence_id);
                    }
                    let Some(rest) = line.strip_prefix("# text = ") else { continue };
                    number += 1;
                    let translated = typeable(rest);
                    let mut joined = pystr::split(&translated).join(" ");
                    if lower {
                        pystr::strip(pystr::rstrip_chars(&pystr::lower(&joined), ".!?…")).clone_into(&mut joined);
                    }
                    let tokens = pystr::split(&joined);
                    if !(MIN_TOKENS..=MAX_TOKENS).contains(&tokens.len()) || !typeable_tokens(&tokens) || !tokens.iter().any(|token| has_cyrillic(token)) {
                        continue;
                    }
                    writer.line(&row_line(&json!({"qid": number, "sid": sentence_id, "kind": kind, "mix": mix_of(&tokens), "text": joined})))?;
                    count_up(&mut counts, format!("{name}:{part}"));
                }
            }
            writer.close()?;
        }
    }
    Ok(counts)
}

/// `BARE_SIGNS.sub(" ", text)`: `[,.!?…:;"()]|(?<!\w)-|-(?!\w)`.
fn without_bare_signs(text: &str) -> String {
    let characters: Vec<char> = text.chars().collect();
    let mut output = String::with_capacity(text.len());
    for (index, character) in characters.iter().enumerate() {
        let sign = ",.!?…:;\"()".contains(*character)
            || (*character == '-' && (index == 0 || !is_word(characters[index - 1]) || !characters.get(index + 1).is_some_and(|next| is_word(*next))));
        output.push(if sign { ' ' } else { *character });
    }
    output
}

fn tatoeba_rows(tatoeba: &Path, language: &str) -> Result<Vec<(String, String)>, String> {
    let mut rows = Vec::new();
    for line in universal_lines(&bz2_text(&tatoeba.join(format!("{language}_sentences.tsv.bz2")))?) {
        let mut fields = line.split('\t');
        let (Some(identifier), Some(_), Some(text)) = (fields.next(), fields.next(), fields.next()) else { continue };
        rows.push((identifier.to_owned(), text.to_owned()));
    }
    Ok(rows)
}

/// `build_tatoeba_bare`: sentences as chat lines are often typed: lowercase, no signs, only words of the language.
pub fn build_tatoeba_bare(tatoeba: &Path, out: &Path) -> Result<BTreeMap<String, i64>, String> {
    let mut counts = BTreeMap::new();
    let mut files: HashMap<&str, GzWriter> = SPLITS.iter().map(|part| Ok((*part, GzWriter::create(&out.join(format!("tatoeba-bare-{part}.jsonl.gz")))?))).collect::<Result<_, String>>()?;
    for (language, cyrillic) in [("rus", true), ("eng", false)] {
        for (identifier, text) in tatoeba_rows(tatoeba, language)? {
            let lowered = without_bare_signs(&pystr::lower(&typeable(&text)));
            let tokens = pystr::split(&lowered);
            let word = |token: &&str| !token.is_empty() && token.chars().all(|character| if cyrillic { is_cyrillic_letter(character) } else { character.is_ascii_alphabetic() });
            if !(TATOEBA_MIN_TOKENS..=TATOEBA_MAX_TOKENS).contains(&tokens.len()) || !tokens.iter().all(word) {
                continue;
            }
            let part = split_of(&format!("{TATOEBA_NAMESPACE}:{language}"), &identifier);
            let number: i64 = identifier.parse().map_err(|_| "a sentence id")?;
            files.get_mut(part).expect("a part").line(&row_line(&json!({"qid": number, "kind": "tatoeba_bare", "mix": if cyrillic { "russian" } else { "mixed" }, "text": tokens.join(" ")})))?;
            count_up(&mut counts, format!("tatoeba-bare:{part}"));
        }
    }
    for (_, writer) in files {
        writer.close()?;
    }
    Ok(counts)
}

fn zstd_lines(path: &Path, mut each: impl FnMut(&Value) -> Result<(), String>) -> Result<(), String> {
    let file = std::fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let decoder = ruzstd::decoding::StreamingDecoder::new(BufReader::new(file)).map_err(|error| format!("{error:?}"))?;
    for line in BufReader::with_capacity(1 << 20, decoder).split(b'\n') {
        let line = line.map_err(|error| error.to_string())?;
        let record: Value = serde_json::from_slice(&line).map_err(|error| error.to_string())?;
        each(&record)?;
    }
    Ok(())
}

fn or_empty<'a>(value: &'a Value, name: &str) -> &'a str {
    value.get(name).and_then(Value::as_str).unwrap_or("")
}

fn list<'a>(value: &'a Value, name: &str) -> &'a [Value] {
    value.get(name).and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// `build_messages`: the three styles of the sources, split by thread or sentence id.
pub fn build_messages(so: &Path, tatoeba: &Path, out: &Path) -> Result<BTreeMap<String, i64>, String> {
    std::fs::create_dir_all(out).map_err(|error| error.to_string())?;
    let mut counts = BTreeMap::new();
    let open = |pattern: &str| -> Result<HashMap<&'static str, GzWriter>, String> { SPLITS.iter().map(|part| Ok((*part, GzWriter::create(&out.join(pattern.replace('*', part)))?))).collect() };
    let (mut handles, mut code, mut code_ids) = (open("so-*.jsonl.gz")?, open("code-*.txt.gz")?, open("code-*.qids.txt.gz")?);
    let (mut seen, mut seen_code): (HashSet<String>, HashSet<String>) = (HashSet::new(), HashSet::new());
    zstd_lines(so, |record| {
        let question = record["question_id"].as_i64().ok_or("a question id")?;
        let part = split_of(SO_NAMESPACE, &question.to_string());
        let mut pieces: Vec<(&str, &str)> = vec![("title", or_empty(record, "title"))];
        pieces.extend(list(record, "comments").iter().map(|comment| ("comment", or_empty(comment, "text"))));
        pieces.push(("prose", or_empty(record, "text_markdown")));
        let mut markdowns = vec![or_empty(record, "text_markdown")];
        for answer in list(record, "answers") {
            pieces.push(("prose", or_empty(answer, "text_markdown")));
            pieces.extend(list(answer, "comments").iter().map(|comment| ("comment", or_empty(comment, "text"))));
            markdowns.push(or_empty(answer, "text_markdown"));
        }
        for (kind, text) in pieces {
            for tokens in post_messages(text) {
                let joined = tokens.join(" ");
                if seen.insert(joined.clone()) {
                    let references: Vec<&str> = tokens.iter().map(String::as_str).collect();
                    handles.get_mut(part).expect("a part").line(&row_line(&json!({"qid": question, "kind": kind, "mix": mix_of(&references), "text": joined})))?;
                    count_up(&mut counts, format!("so:{part}"));
                }
            }
        }
        for markdown in markdowns {
            for line in code_lines(markdown) {
                if seen_code.insert(line.clone()) {
                    code.get_mut(part).expect("a part").line(&line)?;
                    code_ids.get_mut(part).expect("a part").line(&question.to_string())?;
                    count_up(&mut counts, format!("code:{part}"));
                }
            }
        }
        Ok(())
    })?;
    for writer in handles.into_values().chain(code.into_values()).chain(code_ids.into_values()) {
        writer.close()?;
    }
    for lower in [false, true] {
        let name = if lower { "tatoeba-chat" } else { "tatoeba" };
        let kind = name.replace('-', "_");
        let mut files: HashMap<&str, GzWriter> = SPLITS.iter().map(|part| Ok((*part, GzWriter::create(&out.join(format!("{name}-{part}.jsonl.gz")))?))).collect::<Result<_, String>>()?;
        for (language, cyrillic) in [("rus", true), ("eng", false)] {
            for (identifier, text) in tatoeba_rows(tatoeba, language)? {
                let translated = typeable(&text);
                let stripped = pystr::strip(&translated);
                let tokens = pystr::split(stripped);
                if !(TATOEBA_MIN_TOKENS..=TATOEBA_MAX_TOKENS).contains(&tokens.len()) || !typeable_tokens(&tokens) {
                    continue;
                }
                if tokens.iter().any(|token| if cyrillic { has_latin(token) } else { has_cyrillic(token) }) || !tokens.iter().any(|token| if cyrillic { has_cyrillic(token) } else { has_latin(token) }) {
                    continue;
                }
                let part = split_of(&format!("{TATOEBA_NAMESPACE}:{language}"), &identifier);
                let mut joined = tokens.join(" ");
                if lower {
                    pystr::strip(pystr::rstrip_chars(&pystr::lower(&joined), ".!?…")).clone_into(&mut joined);
                    if pystr::split(&joined).len() < TATOEBA_MIN_TOKENS {
                        continue;
                    }
                }
                let number: i64 = identifier.parse().map_err(|_| "a sentence id")?;
                files.get_mut(part).expect("a part").line(&row_line(&json!({"qid": number, "kind": kind, "mix": if cyrillic { "russian" } else { "mixed" }, "text": joined})))?;
                count_up(&mut counts, format!("{name}:{part}"));
            }
        }
        for (_, writer) in files {
            writer.close()?;
        }
    }
    Ok(counts)
}

/// `build_terms`: how often plain words occur in the training parts, in the order they first occur.
pub fn build_terms(messages: &Path, known: impl Fn(&str) -> bool) -> Result<Value, String> {
    let (mut latin, mut cyrillic, mut english, mut russian): (IndexMap<String, i64>, IndexMap<String, i64>, IndexMap<String, i64>, IndexMap<String, i64>) = Default::default();
    for name in ["so-train.jsonl.gz", "tatoeba-train.jsonl.gz", "taiga-train.jsonl.gz"] {
        for line in gz_text(&messages.join(name))?.lines() {
            let row: Value = serde_json::from_str(line).map_err(|error| error.to_string())?;
            let english_sentence = name.starts_with("tatoeba") && row["mix"] != json!("russian");
            for token in pystr::split(row["text"].as_str().ok_or("a text")?) {
                let core = pystr::lower(pystr::strip_chars(token, TERM_EDGE_SIGNS));
                if !core.is_empty() && core.chars().all(|character| character.is_ascii_lowercase()) {
                    if name.starts_with("so") {
                        *latin.entry(core).or_insert(0) += 1;
                    } else if english_sentence {
                        *english.entry(core).or_insert(0) += 1;
                    }
                } else if !core.is_empty() && core.chars().all(|character| ('а'..='я').contains(&character) || character == 'ё') {
                    *russian.entry(core.clone()).or_insert(0) += 1;
                    if !name.starts_with("taiga") {
                        *cyrillic.entry(core).or_insert(0) += 1;
                    }
                }
            }
        }
    }
    let frequent = |counts: &IndexMap<String, i64>, keep: &dyn Fn(&str) -> bool| -> Value {
        Value::Object(counts.iter().filter(|(word, count)| **count >= TERM_FREQUENCY_LEAST && keep(word)).map(|(word, count)| (word.clone(), json!(count))).collect())
    };
    let all = |_: &str| true;
    Ok(json!({"latin": frequent(&latin, &all), "cyrillic": frequent(&cyrillic, &|word| !known(word)), "english": frequent(&english, &all), "russian": frequent(&russian, &all)}))
}

/// The question of every developer-chat message: none of them is typed for training.
fn excluded_questions(members: &[PathBuf]) -> Result<HashSet<i64>, String> {
    let mut questions = HashSet::new();
    for path in members {
        for line in gz_text(path)?.lines() {
            let row: Value = serde_json::from_str(line).map_err(|error| error.to_string())?;
            questions.insert(row["question"].as_i64().ok_or("a question")?);
        }
    }
    Ok(questions)
}

/// `build_alnum`: whole tokens of letters and digits in the SO training messages, by script.
pub fn build_alnum(messages: &Path, members: &[PathBuf]) -> Result<Value, String> {
    let left_out = excluded_questions(members)?;
    let (mut latin, mut cyrillic): (BTreeMap<String, i64>, BTreeMap<String, i64>) = Default::default();
    for line in gz_text(&messages.join("so-train.jsonl.gz"))?.lines() {
        let row: Value = serde_json::from_str(line).map_err(|error| error.to_string())?;
        if left_out.contains(&row["qid"].as_i64().ok_or("a question")?) {
            continue;
        }
        for token in pystr::split(row["text"].as_str().ok_or("a text")?) {
            let core = pystr::casefold(pystr::strip_chars(token, TERM_EDGE_SIGNS));
            if core.chars().count() <= ALNUM_HEAD_MAX_CHARACTERS && letters_and_digits(&core) {
                *(if pystr::isascii(&core) { &mut latin } else { &mut cyrillic }).entry(core).or_insert(0) += 1;
            }
        }
    }
    let frequent = |counts: BTreeMap<String, i64>| -> Value { Value::Object(counts.into_iter().filter(|(_, count)| *count >= TERM_FREQUENCY_LEAST).map(|(token, count)| (token, json!(count))).collect()) };
    Ok(json!({"cyrillic": frequent(cyrillic), "latin": frequent(latin)}))
}

/// `build_short`: tokens of two or three Russian letters in the Leipzig word lists unpacked under the directory.
pub fn build_short(leipzig: &Path) -> Result<Value, String> {
    let mut paths: Vec<PathBuf> = Vec::new();
    for outer in std::fs::read_dir(leipzig).map_err(|error| error.to_string())?.flatten() {
        if !outer.file_name().to_string_lossy().starts_with("x-") || !outer.path().is_dir() {
            continue;
        }
        for inner in std::fs::read_dir(outer.path()).map_err(|error| error.to_string())?.flatten() {
            if inner.path().is_dir() {
                for file in std::fs::read_dir(inner.path()).map_err(|error| error.to_string())?.flatten() {
                    if file.file_name().to_string_lossy().ends_with("-words.txt") {
                        paths.push(file.path());
                    }
                }
            }
        }
    }
    paths.sort();
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    for path in paths {
        let text = String::from_utf8_lossy(&std::fs::read(&path).map_err(|error| error.to_string())?).into_owned();
        for line in universal_lines(&text) {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() < LEIPZIG_WORDS_COLUMNS {
                continue;
            }
            let word = pystr::casefold(parts[1]);
            if SHORT_HEAD_LETTERS.contains(&word.chars().count()) && word.chars().all(|character| ('а'..='я').contains(&character) || character == 'ё') {
                *counts.entry(word).or_insert(0) += parts[2].trim().parse::<i64>().map_err(|_| "a count")?;
            }
        }
    }
    Ok(json!({"cyrillic": Value::Object(counts.into_iter().filter(|(_, count)| *count >= TERM_FREQUENCY_LEAST).map(|(token, count)| (token, json!(count))).collect())}))
}

/// `build_sources`: whose text the captured files hold, written next to the manifest for attribution.
pub fn build_sources(manifest: &Path, messages: &Path, tatoeba: &Path) -> Result<Value, String> {
    let payload = read_json(manifest)?;
    let (mut questions, mut sentences): (BTreeSet<i64>, BTreeSet<i64>) = Default::default();
    let mut treebank: BTreeSet<String> = BTreeSet::new();
    for source in payload["sources"].as_array().ok_or("sources")? {
        let recipe = &source["recipe"];
        let name = recipe["messages"].as_str().ok_or("a recipe's messages")?;
        let file = messages.join(name);
        let code = recipe.get("code").and_then(Value::as_str).filter(|code| !code.is_empty()).map(|code| messages.join(code));
        let chosen = jobs_recipe_messages(&file, recipe)?;
        let (above, code_lines) = recipe_lines_above(&file, recipe, code.as_deref())?;
        let mut ids: HashSet<i64> = chosen.iter().map(|entry| entry.1).collect();
        let mut lines: HashSet<usize> = HashSet::new();
        for (index, (text, _)) in chosen.iter().enumerate() {
            match recipe_previous_line(recipe, text, index, chosen.len(), code_lines, above.len()) {
                Some(("code", line)) => {
                    lines.insert(line);
                }
                Some(("above", line)) => {
                    ids.insert(above[line].1);
                }
                _ => {}
            }
        }
        if let (false, Some(code)) = (lines.is_empty(), &code) {
            let path = code.with_file_name(code.file_name().map(|name| name.to_string_lossy().replace(".txt.gz", ".qids.txt.gz")).unwrap_or_default());
            for (number, line) in gz_text(&path)?.lines().enumerate() {
                if lines.contains(&number) {
                    questions.insert(line.parse().map_err(|_| "a question")?);
                }
            }
        }
        if name.starts_with("taiga") {
            let mut names = HashMap::new();
            for line in gz_text(&file)?.lines() {
                let row: Value = serde_json::from_str(line).map_err(|error| error.to_string())?;
                names.insert(row["qid"].as_i64().ok_or("a sentence")?, row["sid"].as_str().unwrap_or("").to_owned());
            }
            treebank.extend(ids.iter().filter_map(|number| names.get(number).cloned()));
        } else if name.starts_with("tatoeba") {
            sentences.extend(ids);
        } else {
            questions.extend(ids);
        }
    }
    let mut authors: BTreeMap<i64, String> = BTreeMap::new();
    for language in ["rus", "eng"] {
        for line in universal_lines(&bz2_text(&tatoeba.join(format!("{language}_sentences_detailed.tsv.bz2")))?) {
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() > TATOEBA_AUTHOR_COLUMN {
                let number: i64 = fields[0].parse().map_err(|_| "a sentence id")?;
                if sentences.contains(&number) {
                    authors.insert(number, fields[TATOEBA_AUTHOR_COLUMN].to_owned());
                }
            }
        }
    }
    if authors.len() != sentences.len() {
        return Err("a typed sentence is missing from the detailed export".to_owned());
    }
    let folder = manifest.parent().ok_or("a manifest outside a folder")?;
    let write = |name: &str, bytes: &[u8]| std::fs::write(folder.join(name), bytes).map_err(|error| error.to_string());
    let mut text = "# ru.stackoverflow.com questions whose messages or code lines the captured files hold;\n# https://ru.stackoverflow.com/questions/<id> names the authors of the question, its answers and comments.\n".to_owned();
    for question in &questions {
        text.push_str(&format!("{question}\n"));
    }
    write("so-questions.txt", text.as_bytes())?;
    let mut text = "# Tatoeba sentences the captured files hold and their authors (\\N: no owner);\n# https://tatoeba.org/sentences/show/<id>\n".to_owned();
    for (sentence, author) in &authors {
        text.push_str(&format!("{sentence}\t{author}\n"));
    }
    let mut encoder = xz2::write::XzEncoder::new(Vec::new(), XZ_PRESET);
    encoder.write_all(text.as_bytes()).map_err(|error| error.to_string())?;
    write("tatoeba-authors.tsv.xz", &encoder.finish().map_err(|error| error.to_string())?)?;
    if !treebank.is_empty() {
        let mut text = "# UD Russian-Taiga r2.18 sentences the captured files hold, by `sent_id`;\n# https://github.com/UniversalDependencies/UD_Russian-Taiga names their sources.\n".to_owned();
        for name in &treebank {
            text.push_str(&format!("{name}\n"));
        }
        write("taiga-sentences.txt", text.as_bytes())?;
    }
    let unique: HashSet<&String> = authors.values().collect();
    Ok(json!({"questions": questions.len(), "sentences": sentences.len(), "authors": unique.len(), "taiga": treebank.len()}))
}

/// `cyrillic` of the class predicates: `"а" <= char.casefold() <= "я" or char.casefold() == "ё"`.
fn cyrillic(character: char) -> bool {
    let folded = pystr::casefold(&text_of(character));
    ("а"..="я").contains(&folded.as_str()) || folded == "ё"
}

fn latin_letter(character: char) -> bool {
    let lowered = pystr::lower(&text_of(character));
    ("a"..="z").contains(&lowered.as_str())
}

fn is_alpha(character: char) -> bool {
    pystr::isalpha(&text_of(character))
}

fn trusted_letter(text: &str) -> bool {
    TRUSTED_SINGLE_LETTER_WORDS.contains(&pystr::casefold(text).as_str())
}

fn variable_token(previous: &str, token: &str) -> bool {
    let core = pystr::rstrip_chars(token, TRAILING_SIGNS);
    if core.chars().count() != 1 || !pystr::isalpha(core) || previous.is_empty() || !previous.chars().all(cyrillic) {
        return false;
    }
    pystr::isascii(core) || (core.chars().all(cyrillic) && !trusted_letter(core))
}

fn translate(text: &str, source: &str, target: &str) -> String {
    pair().translate(text, source, target).expect("known layouts")
}

fn cyrletter_latin_token(previous: &str, token: &str) -> bool {
    let core = pystr::rstrip_chars(token, TRAILING_SIGNS);
    variable_token(previous, token) && pystr::isascii(core) && trusted_letter(&translate(core, "us", "ru"))
}

fn dash_token(_previous: &str, token: &str) -> bool {
    DASH_TOKEN.is_match(pystr::rstrip_chars(token, TRAILING_SIGNS))
}

fn russian_word(text: &str) -> bool {
    text.chars().any(cyrillic) && !text.chars().any(latin_letter)
}

fn digit_dot_token(previous: &str, token: &str) -> bool {
    russian_word(previous) && DIGIT_DOT_TOKEN.is_match(token)
}

fn digit_letter_token(previous: &str, token: &str) -> bool {
    russian_word(previous) && DIGIT_LETTER_TOKEN.is_match(&pystr::casefold(pystr::rstrip_chars(token, TRAILING_SIGNS)))
}

fn dotted_token(_previous: &str, token: &str) -> bool {
    let core = pystr::rstrip_chars(pystr::lstrip_chars(token, "("), ",;:)!?");
    if pystr::isascii(core) {
        return dotted_question(core, &translate(core, "us", "ru").replace('/', ".").replace('?', ","));
    }
    dotted_question(&translate(&core.replace('.', "/").replace(',', "?"), "ru", "us"), core)
}

fn latin_dotted_token(previous: &str, token: &str) -> bool {
    pystr::isascii(token) && dotted_token(previous, token)
}

fn russian_dotted_token(previous: &str, token: &str) -> bool {
    !pystr::isascii(token) && dotted_token(previous, token)
}

fn alnum_token(_previous: &str, token: &str) -> bool {
    let core = pystr::strip_chars(token, ALNUM_EDGE_SIGNS);
    let other = if pystr::isascii(core) { translate(core, "us", "ru") } else { translate(core, "ru", "us") };
    core.chars().count() <= ALNUM_HEAD_MAX_CHARACTERS && letters_and_digits(core) && letters_and_digits(&other)
}

fn latin_alnum_token(previous: &str, token: &str) -> bool {
    pystr::isascii(pystr::strip_chars(token, ALNUM_EDGE_SIGNS)) && alnum_token(previous, token)
}

fn russian_alnum_token(previous: &str, token: &str) -> bool {
    !pystr::isascii(pystr::strip_chars(token, ALNUM_EDGE_SIGNS)) && alnum_token(previous, token)
}

fn opening_short(text: &str) -> bool {
    pystr::split(text).first().is_some_and(|first| pystr::isalpha(first) && first.chars().count() <= OPENING_SHORT_MAX_LETTERS)
}

fn resumes(text: &str) -> bool {
    let tokens = pystr::split(text);
    if tokens.len() < 3 {
        return false;
    }
    for index in 1..tokens.len() - 1 {
        let token = tokens[index];
        let word = if token.chars().next().is_some_and(|first| RESUME_LEADING_SIGNS.contains(first)) { pystr::slice(token, Some(1), None) } else { token };
        if pystr::isalpha(word) && word.chars().count() <= OPENING_SHORT_MAX_LETTERS && tokens[index + 1].chars().any(cyrillic) {
            let before = tokens[..index].join(" ");
            if before.chars().rfind(|character| is_alpha(*character)).is_some_and(|last| last.is_ascii()) {
                return true;
            }
        }
    }
    false
}

fn letter_slippable(text: &str) -> bool {
    let tokens = pystr::split(text);
    tokens.windows(2).any(|pair| trusted_letter(pair[1]) && pair[0].chars().any(cyrillic) && !pair[0].chars().any(latin_letter))
}

fn quote_opener(text: &str) -> bool {
    pystr::split(text).first().is_some_and(|first| {
        let mut characters = first.chars();
        characters.next() == Some('"') && characters.next().is_some_and(cyrillic)
    })
}

fn slippable(text: &str) -> bool {
    pystr::split(text).windows(2).any(|pair| pair[0].chars().filter(|character| is_alpha(*character)).all(cyrillic) && pair[0].chars().any(cyrillic) && pair[1].chars().any(cyrillic) && !pair[1].chars().any(latin_letter))
}

fn reads_alike(token: &str) -> bool {
    !token.is_empty() && token.chars().all(|character| pystr::isdigit(&text_of(character)) || LAYOUT_ALIKE_SIGNS.contains(character))
}

fn formula(text: &str) -> bool {
    let tokens = pystr::split(text);
    (0..tokens.len().saturating_sub(1)).any(|index| {
        let previous = if index == 0 { "" } else { tokens[index - 1] };
        tokens[index].chars().count() == 1 && variable_token(previous, tokens[index]) && reads_alike(tokens[index + 1]) && sign_token_asked(tokens[index + 1])
    })
}

type TokenClass = fn(&str, &str) -> bool;

fn message_class(name: &str) -> Option<fn(&str) -> bool> {
    Some(match name {
        "opening-short" => opening_short,
        "slippable" => slippable,
        "formula" => formula,
        "resume" => resumes,
        "cyrletter-russian" => letter_slippable,
        "quote-opener" => quote_opener,
        _ => return None,
    })
}

fn token_class(name: &str) -> Option<TokenClass> {
    Some(match name {
        "variable" => variable_token,
        "dotted" => dotted_token,
        "dotted-latin" => latin_dotted_token,
        "dotted-russian" => russian_dotted_token,
        "alnum-latin" => latin_alnum_token,
        "alnum-russian" => russian_alnum_token,
        "cyrletter-latin" => cyrletter_latin_token,
        "dash" => dash_token,
        "digitdot" => digit_dot_token,
        "digitdot-letter" => digit_letter_token,
        _ => return None,
    })
}

fn holds(text: &str, class: TokenClass) -> bool {
    let tokens = pystr::split(text);
    tokens.iter().enumerate().any(|(index, token)| class(if index == 0 { "" } else { tokens[index - 1] }, token))
}

fn capture_split(question: i64, development_share: f64) -> &'static str {
    let digest = hex_digest(&format!("capture-split:{question}"));
    let draw = u64::from_str_radix(digest.get(..CHOICE_HEX_DIGITS).unwrap_or_default(), 16).expect("a digest").to::<f64>() / 16_f64.powi(CHOICE_HEX_DIGITS.to::<i32>());
    if draw < development_share { "dev" } else { "train" }
}

fn listed_ids(path: &Path) -> Result<HashSet<i64>, String> {
    let mut text = String::new();
    xz2::read::XzDecoder::new(std::fs::File::open(path).map_err(|error| error.to_string())?).read_to_string(&mut text).map_err(|error| error.to_string())?;
    Ok(text.lines().filter(|line| line.starts_with(|first: char| pystr::isdigit(&text_of(first)))).filter_map(|line| line.split('\t').next()?.parse().ok()).collect())
}

pub struct SelectRequest<'a> {
    pub messages: &'a Path,
    pub members: &'a [PathBuf],
    pub name: &'a str,
    pub development_share: f64,
    pub out: &'a Path,
    pub prefix: &'a str,
    pub only: Option<&'a Path>,
}

/// `select`: the messages of a class, in two files by question.
pub fn select(request: &SelectRequest<'_>) -> Result<Value, String> {
    let (message, token) = (message_class(request.name), token_class(request.name));
    if message.is_none() && token.is_none() {
        return Err(format!("unknown class: {}", request.name));
    }
    let other = match request.name {
        "dotted-russian" => token_class("dotted-latin"),
        "alnum-russian" => token_class("alnum-latin"),
        _ => None,
    };
    let left_out = excluded_questions(request.members)?;
    let allowed = request.only.map(listed_ids).transpose()?;
    std::fs::create_dir_all(request.out).map_err(|error| error.to_string())?;
    let name_of = |split: &str| format!("{}-{}-{split}.jsonl.gz", request.prefix, request.name);
    let mut handles: HashMap<&str, GzWriter> = ["train", "dev"].iter().map(|split| Ok((*split, GzWriter::create(&request.out.join(name_of(split)))?))).collect::<Result<_, String>>()?;
    let (mut train, mut dev, mut dropped) = (0, 0, 0);
    for line in gz_text(request.messages)?.lines() {
        let row: Value = serde_json::from_str(line).map_err(|error| error.to_string())?;
        let text = row["text"].as_str().ok_or("a text")?;
        if message.is_some_and(|class| !class(text)) || token.is_some_and(|class| !holds(text, class)) || other.is_some_and(|class| holds(text, class)) {
            continue;
        }
        let question = row["qid"].as_i64().ok_or("a question")?;
        if allowed.as_ref().is_some_and(|ids| !ids.contains(&question)) {
            continue;
        }
        if left_out.contains(&question) {
            dropped += 1;
            continue;
        }
        let split = capture_split(question, request.development_share);
        handles.get_mut(split).expect("a split").line(&row_line(&row))?;
        if split == "dev" {
            dev += 1;
        } else {
            train += 1;
        }
    }
    for (_, writer) in handles {
        writer.close()?;
    }
    Ok(json!({"class": request.name, "messages": {"train": train, "dev": dev, "left_out": dropped}, "files": {"train": name_of("train"), "dev": name_of("dev")}, "excluded_questions": left_out.len()}))
}

fn text_key(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
}

fn code_like(token: &str) -> bool {
    token.chars().any(|character| DEVCHAT_CODE_SIGNS.contains(character)) || SIGN_RUN.find_iter(token).any(|run| run.as_str().chars().any(|character| !DEVCHAT_PROSE_SIGNS.contains(character)))
}

fn lookalike(tokens: &[&str]) -> bool {
    for (index, token) in tokens.iter().enumerate() {
        let neighbours: Vec<&str> = [index.checked_sub(1).map(|at| tokens[at]), tokens.get(index + 1).copied()].into_iter().flatten().collect();
        if neighbours.is_empty() {
            continue;
        }
        if matches!(*token, "c" | "C") && neighbours.iter().all(|word| has_cyrillic(word)) {
            return true;
        }
        if matches!(*token, "с" | "С") && neighbours.iter().all(|word| has_latin(word) && !has_cyrillic(word)) {
            return true;
        }
    }
    false
}

fn messages_of(text: &str) -> Vec<String> {
    post_messages(&TAG.replace_all(text, " ")).into_iter().map(|tokens| tokens.join(" ")).collect()
}

fn devchat_split(question: i64) -> &'static str {
    let digest = hex_digest(&format!("{DEVCHAT_NAMESPACE}:{question}"));
    if u64::from_str_radix(digest.get(..8).unwrap_or_default(), 16).expect("a digest").is_multiple_of(2) { "dev" } else { "test" }
}

fn content_checksum(path: &Path) -> Result<String, String> {
    let mut bytes = Vec::new();
    MultiGzDecoder::new(std::fs::File::open(path).map_err(|error| error.to_string())?).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    Ok(hex(&Sha256::digest(&bytes)))
}

/// `build_devchat_messages`: messages of fresh comments, by the question they discuss; a members file beside each split.
pub fn build_devchat(root: &Path, comments: &Path, dump: &Path, out: &Path, earlier_files: &[PathBuf]) -> Result<Value, String> {
    if checksum(dump)? != DEVCHAT_DUMP_SHA256 {
        return Err("the 2023 dump differs from the pinned one".to_owned());
    }
    let mut known: HashSet<String> = HashSet::new();
    zstd_lines(dump, |record| {
        let mut add = |text: &str| known.extend(messages_of(text).iter().map(|message| text_key(message)));
        add(or_empty(record, "text_markdown"));
        for answer in list(record, "answers") {
            add(or_empty(answer, "text_markdown"));
        }
        for comment in list(record, "comments") {
            add(or_empty(comment, "text"));
        }
        for answer in list(record, "answers") {
            for comment in list(answer, "comments") {
                add(or_empty(comment, "text"));
            }
        }
        Ok(())
    })?;
    let posts = comments.join("posts/question-of-post.json");
    let question_of: HashMap<i64, i64> = read_json(&posts)?.as_object().ok_or("posts")?.iter().filter_map(|(post, question)| Some((post.parse().ok()?, question.as_i64()?))).collect();
    let mut files: Vec<PathBuf> = std::fs::read_dir(comments).map_err(|error| error.to_string())?.flatten().map(|entry| entry.path()).filter(|path| {
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        name.starts_with("comments-") && name.ends_with(".json.gz")
    }).collect();
    files.sort();
    let mut earlier: HashSet<String> = HashSet::new();
    for path in earlier_files {
        for line in gz_text(path)?.lines() {
            let row: Value = serde_json::from_str(line).map_err(|error| error.to_string())?;
            earlier.insert(row["text"].as_str().unwrap_or("").to_owned());
        }
    }
    std::fs::create_dir_all(out).map_err(|error| error.to_string())?;
    let mut handles: HashMap<(&str, &str), GzWriter> = HashMap::new();
    for split in ["dev", "test"] {
        for kind in ["messages", "members"] {
            handles.insert((split, kind), GzWriter::create(&out.join(format!("devchat-{split}.{kind}.jsonl.gz")))?);
        }
    }
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    let mut seen: HashSet<String> = HashSet::new();
    for path in &files {
        let batch: Value = serde_json::from_str(&gz_text(path)?).map_err(|error| error.to_string())?;
        for comment in batch.as_array().ok_or("comments")? {
            let post = match &comment["post_id"] {
                Value::String(text) => text.parse().map_err(|_| "a post")?,
                value @ Value::Null | value @ Value::Bool(_) | value @ Value::Number(_) | value @ Value::Array(_) | value @ Value::Object(_) => value.as_i64().ok_or("a post")?,
            };
            let question = question_of.get(&post).copied().unwrap_or(post);
            for text in messages_of(or_empty(comment, "body")) {
                if !seen.insert(text.clone()) {
                    continue;
                }
                let tokens = pystr::split(&text);
                let key = text_key(&text);
                let reason = if tokens.iter().any(|token| code_like(token)) {
                    Some("code")
                } else if known.contains(&key) {
                    Some("dump")
                } else if earlier.contains(&key) {
                    Some("earlier")
                } else if lookalike(&tokens) {
                    Some("lookalike")
                } else {
                    None
                };
                if let Some(reason) = reason {
                    count_up(&mut counts, format!("dropped:{reason}"));
                    continue;
                }
                let split = devchat_split(question);
                let mix = mix_of(&tokens);
                handles.get_mut(&(split, "messages")).expect("a file").line(&row_line(&json!({"qid": question, "kind": "comment", "mix": mix, "text": text})))?;
                handles.get_mut(&(split, "members")).expect("a file").line(&json_dumps(&json!({"text": key, "question": question, "mix": mix}), &JsonOptions::default()))?;
                count_up(&mut counts, format!("{split}:{mix}"));
            }
        }
    }
    for (_, writer) in handles {
        writer.close()?;
    }
    let mut written: Vec<PathBuf> = std::fs::read_dir(out).map_err(|error| error.to_string())?.flatten().map(|entry| entry.path()).filter(|path| {
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        name.starts_with("devchat-") && name.ends_with(".jsonl.gz")
    }).collect();
    written.sort();
    let hashes = |paths: &[PathBuf], hash: &dyn Fn(&Path) -> Result<String, String>| -> Result<Map<String, Value>, String> {
        paths.iter().map(|path| Ok((path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default(), Value::String(hash(path)?)))).collect()
    };
    let receipt = json!({"namespace": DEVCHAT_NAMESPACE, "split": "by question", "counts": counts, "content_sha256": hashes(&written, &content_checksum)?, "comments_sha256": hashes(&files, &checksum)?,
        "posts_sha256": checksum(&posts)?, "dump_sha256": DEVCHAT_DUMP_SHA256, "builder_sha256": checksum(&root.join("crates/ks-train/src/messages.rs"))?, "earlier_members_sha256": hashes(earlier_files, &content_checksum)?});
    std::fs::write(out.join("receipt.json"), json_dumps(&receipt, &JsonOptions { indent: Some(Indent::Spaces(1)), ..JsonOptions::default() }) + "\n").map_err(|error| error.to_string())?;
    Ok(json!(counts))
}

/// The commands of this module.
pub fn command(arguments: &[String], root: &Path, resources: &std::sync::Arc<Resources>) -> Result<Value, String> {
    let option = |name: &str| arguments.iter().position(|argument| argument == name).and_then(|place| arguments.get(place + 1)).map(PathBuf::from);
    let required = |name: &str| option(name).ok_or_else(|| format!("{name} is required"));
    let repeated = |name: &str| -> Vec<PathBuf> { arguments.iter().enumerate().filter(|(_, argument)| *argument == name).filter_map(|(place, _)| arguments.get(place + 1)).map(PathBuf::from).collect() };
    let indented = JsonOptions { ensure_ascii: false, sort_keys: true, indent: Some(Indent::Spaces(2)), ..JsonOptions::default() };
    let write_table = |table: &Value, path: &Path| -> Result<Value, String> {
        std::fs::write(path, json_dumps(table, &indented) + "\n").map_err(|error| error.to_string())?;
        Ok(Value::Object(table.as_object().ok_or("a table")?.iter().map(|(name, words)| (name.clone(), json!(words.as_object().map_or(0, Map::len)))).collect()))
    };
    match arguments.first().map(String::as_str) {
        Some("messages") => {
            let out = required("--out")?;
            let mut built = BTreeMap::new();
            if let (Some(so), Some(tatoeba)) = (option("--so"), option("--tatoeba")) {
                built.extend(build_messages(&so, &tatoeba, &out)?);
            }
            if let Some(tatoeba) = option("--tatoeba") {
                std::fs::create_dir_all(&out).map_err(|error| error.to_string())?;
                built.extend(build_tatoeba_bare(&tatoeba, &out)?);
            }
            if let Some(taiga) = option("--taiga") {
                std::fs::create_dir_all(&out).map_err(|error| error.to_string())?;
                built.extend(build_taiga(&taiga, &out)?);
            }
            Ok(json!(built))
        }
        Some("terms") => {
            let russian = resources.engine_language_model("ru_RU").map_err(|error| error.name.to_string())?;
            let table = build_terms(&required("--messages")?, |word| russian.score(word).known)?;
            std::fs::write(required("--out")?, json_dumps(&table, &JsonOptions { ensure_ascii: false, ..JsonOptions::default() })).map_err(|error| error.to_string())?;
            Ok(Value::Object(table.as_object().ok_or("a table")?.iter().map(|(name, words)| (name.clone(), json!(words.as_object().map_or(0, Map::len)))).collect()))
        }
        Some("alnum") => write_table(&build_alnum(&required("--messages")?, &repeated("--members"))?, &required("--out")?),
        Some("short") => write_table(&build_short(&required("--leipzig")?)?, &required("--out")?),
        Some("sources") => build_sources(&required("--manifest")?, &required("--messages")?, &required("--tatoeba")?),
        Some("select") => {
            let share: f64 = arguments.iter().position(|argument| argument == "--development-share").and_then(|place| arguments.get(place + 1)).ok_or("--development-share is required")?.parse().map_err(|_| "--development-share is a number")?;
            let name = arguments.iter().position(|argument| argument == "--class").and_then(|place| arguments.get(place + 1)).ok_or("--class is required")?;
            let prefix = arguments.iter().position(|argument| argument == "--prefix").and_then(|place| arguments.get(place + 1)).map_or("so", String::as_str);
            let only = option("--only-ids");
            select(&SelectRequest { messages: &required("--messages")?, members: &repeated("--members"), name, development_share: share, out: &required("--out")?, prefix, only: only.as_deref() })
        }
        Some("devchat-messages") => build_devchat(root, &required("--comments")?, &required("--dump")?, &required("--out")?, &repeated("--earlier")),
        other => Err(format!("unknown command: {other:?}")),
    }
}
