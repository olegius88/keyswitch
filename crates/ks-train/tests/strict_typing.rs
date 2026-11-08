//! What no lint sees of the untyped JSON tree and of text used as an enum, counted in every tracked source file of the
//! crates and held to the numbers in `tests/data/strict_typing.json`.
//!
//! Clippy refuses `serde_json::Value`, `json!` and the tree's accessors (clippy.toml), but not `v["key"]` on a tree whose
//! type was only inferred, nor `mode == "early"` on a string that stands for one of a few values. This test counts, in
//! `src` outside `#[cfg(test)]` items and in the whole of every other file:
//!
//! - `value_tokens`: `serde_json::` paths, except `serde_json::Number` (a number as the file wrote it), and the names
//!   `Value`, `Map` and `json!` where the file imports them from `serde_json`;
//! - `string_key_index`: an index by a text literal: `x["key"]`, `f()["key"]`, `x?["key"]`, `x["a"]["b"]`; not
//!   `vec!["a"]`, `&["a"]`, `[("a", 1)]` or `in ["a"]`, which are arrays;
//! - `literal_compares`: `x == "text"` or `a.b != "text"`, a name compared with a literal of two characters or more;
//! - `expect_roots`: a crate root that carries a temporary `expect` of a `clippy::disallowed_*` lint.
//!
//! A crate's totals leave out the modules listed under `exceptions`: only these may carry a module-level `expect` of
//! those lints, with the reason the list gives; their own lines are still counted, under their own file. No item may
//! carry such an `expect`. The record must equal what is measured: a count that rose fails with the file and its lines, a
//! count that fell fails until the record says so. On a difference the measured record is written next to the test's
//! other output, ready to copy over the file once the change is meant.
use ks_text::{Indent, JsonOptions, json};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

/// The record, by its path in the crate.
const RECORD: &str = "tests/data/strict_typing.json";

/// The names a file can import from `serde_json` that this test counts.
const UNTYPED_NAMES: [&str; 3] = ["Value", "Map", "json"];

/// Words that may stand before `[` when it opens an array, not an index.
const KEYWORDS: [&str; 30] = [
    "as", "async", "box", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "fn", "for", "if", "impl",
    "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "static", "struct", "super", "while", "yield",
];

/// The first words of an item; an attribute on anything else (a field, an arm, a statement) ends at a comma too.
const ITEM_WORDS: [&str; 15] =
    ["pub", "mod", "fn", "use", "impl", "struct", "enum", "const", "static", "type", "trait", "macro_rules", "extern", "unsafe", "async"];

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Ident(String),
    /// A text literal, plain or raw, as the program sees it.
    Text(String),
    /// `==`
    Equal,
    /// `!=`
    NotEqual,
    /// `::`
    PathSeparator,
    Punct(char),
    /// A number, a character, a byte string, a lifetime: nothing this test looks into.
    Other,
}

#[derive(Debug, Clone)]
struct Lexed {
    token: Token,
    line: usize,
}

struct Cursor {
    chars: Vec<char>,
    at: usize,
    line: usize,
}

impl Cursor {
    fn peek(&self, ahead: usize) -> Option<char> {
        self.chars.get(self.at + ahead).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek(0)?;
        self.at += 1;
        if c == '\n' {
            self.line += 1;
        }
        Some(c)
    }

    fn eat_while(&mut self, keep: impl Fn(char) -> bool) -> String {
        let mut eaten = String::new();
        while let Some(c) = self.peek(0).filter(|&c| keep(c)) {
            eaten.push(c);
            self.bump();
        }
        eaten
    }

    /// The rest of a quoted literal after its opening quote, escapes decoded.
    fn quoted(&mut self) -> String {
        let mut text = String::new();
        while let Some(c) = self.bump() {
            match c {
                '"' => break,
                '\\' => match self.bump() {
                    Some('n') => text.push('\n'),
                    Some('t') => text.push('\t'),
                    Some('r') => text.push('\r'),
                    Some('0') => text.push('\0'),
                    Some('x') => {
                        let code: String = [self.bump(), self.bump()].into_iter().flatten().collect();
                        text.push(char::from_u32(u32::from_str_radix(&code, 16).unwrap_or(0)).unwrap_or('\0'));
                    }
                    Some('u') => {
                        let code = self.eat_while(|c| c != '}');
                        self.bump();
                        let code = code.trim_start_matches('{');
                        text.push(char::from_u32(u32::from_str_radix(code, 16).unwrap_or(0)).unwrap_or('\0'));
                    }
                    Some('\n') => {
                        self.eat_while(char::is_whitespace);
                    }
                    Some(other) => text.push(other),
                    None => break,
                },
                other => text.push(other),
            }
        }
        text
    }

    /// A raw literal from its first `#` or quote: the text up to the quote followed by as many `#` as opened it.
    fn raw(&mut self) -> String {
        let hashes = self.eat_while(|c| c == '#').len();
        self.bump();
        let mut text = String::new();
        while let Some(c) = self.bump() {
            if c == '"' && (0..hashes).all(|ahead| self.peek(ahead) == Some('#')) {
                for _ in 0..hashes {
                    self.bump();
                }
                break;
            }
            text.push(c);
        }
        text
    }

    /// Whether a raw literal starts here: hashes, then a quote.
    fn raw_ahead(&self) -> bool {
        let mut ahead = 0;
        while self.peek(ahead) == Some('#') {
            ahead += 1;
        }
        self.peek(ahead) == Some('"')
    }

    /// A character literal or a lifetime, from its quote.
    fn quote(&mut self) {
        self.bump();
        if self.peek(0) == Some('\\') {
            self.bump();
            self.bump();
            self.eat_while(|c| c != '\'');
            self.bump();
        } else if self.peek(1) == Some('\'') {
            self.bump();
            self.bump();
        } else {
            self.eat_while(is_ident_char);
        }
    }

    fn word(&mut self) -> Token {
        let word = self.eat_while(is_ident_char);
        match (word.as_str(), self.peek(0)) {
            ("r", Some('"' | '#')) if self.raw_ahead() => Token::Text(self.raw()),
            ("br" | "cr", Some('"' | '#')) if self.raw_ahead() => {
                self.raw();
                Token::Other
            }
            ("r", Some('#')) => {
                self.bump();
                Token::Ident(self.eat_while(is_ident_char))
            }
            ("b" | "c", Some('"')) => {
                self.bump();
                self.quoted();
                Token::Other
            }
            ("b", Some('\'')) => {
                self.quote();
                Token::Other
            }
            _ => Token::Ident(word),
        }
    }

    fn block_comment(&mut self) {
        let mut depth = 0usize;
        while let Some(c) = self.bump() {
            if c == '/' && self.peek(0) == Some('*') {
                self.bump();
                depth += 1;
            } else if c == '*' && self.peek(0) == Some('/') {
                self.bump();
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
        }
    }
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// The tokens of a Rust source, comments dropped, each with its line.
fn lex(source: &str) -> Vec<Lexed> {
    let mut cursor = Cursor { chars: source.chars().collect(), at: 0, line: 1 };
    let mut tokens = Vec::new();
    while let Some(c) = cursor.peek(0) {
        let line = cursor.line;
        let token = match (c, cursor.peek(1)) {
            (c, _) if c.is_whitespace() => {
                cursor.bump();
                continue;
            }
            ('/', Some('/')) => {
                cursor.eat_while(|c| c != '\n');
                continue;
            }
            ('/', Some('*')) => {
                cursor.block_comment();
                continue;
            }
            ('"', _) => {
                cursor.bump();
                Token::Text(cursor.quoted())
            }
            ('\'', _) => {
                cursor.quote();
                Token::Other
            }
            (c, _) if c.is_ascii_digit() => {
                cursor.eat_while(is_ident_char);
                Token::Other
            }
            (c, _) if is_ident_char(c) => cursor.word(),
            (first, second) => {
                cursor.bump();
                match (first, second) {
                    ('=' | '!', Some('=')) | (':', Some(':')) => {
                        cursor.bump();
                        match first {
                            '=' => Token::Equal,
                            '!' => Token::NotEqual,
                            _ => Token::PathSeparator,
                        }
                    }
                    _ => Token::Punct(first),
                }
            }
        };
        tokens.push(Lexed { token, line });
    }
    tokens
}

/// What one file holds, by line.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Scan {
    value_tokens: Vec<usize>,
    string_key_index: Vec<usize>,
    literal_compares: Vec<usize>,
    /// Inner attributes that expect a `clippy::disallowed_*` lint: the line and the reason.
    module_expects: Vec<(usize, String)>,
    /// Outer attributes that do: an exception on one item.
    item_expects: Vec<usize>,
    /// The files that `#[cfg(test)]` module declarations name with `#[path]`, relative to this file's folder.
    test_modules: Vec<String>,
}

fn ident(lexed: Option<&Lexed>) -> Option<&str> {
    match lexed.map(|lexed| &lexed.token) {
        Some(Token::Ident(word)) => Some(word),
        _ => None,
    }
}

fn is(lexed: Option<&Lexed>, token: &Token) -> bool {
    lexed.is_some_and(|lexed| lexed.token == *token)
}

/// The index just past the `]` that closes the `[` at `open`.
fn closing(tokens: &[Lexed], open: usize) -> usize {
    let mut depth = 0usize;
    for (index, lexed) in tokens.iter().enumerate().skip(open) {
        if lexed.token == Token::Punct('[') {
            depth += 1;
        } else if lexed.token == Token::Punct(']') {
            depth -= 1;
            if depth == 0 {
                return index + 1;
            }
        }
    }
    tokens.len()
}

/// The index just past the item, field, arm or statement that starts at `from`.
fn item_end(tokens: &[Lexed], from: usize) -> usize {
    let item = ident(tokens.get(from)).is_some_and(|word| ITEM_WORDS.contains(&word));
    let mut depth = 0usize;
    for (index, lexed) in tokens.iter().enumerate().skip(from) {
        let Token::Punct(c) = lexed.token else { continue };
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                if depth == 0 {
                    return index;
                }
                depth -= 1;
                if depth == 0 && c == '}' {
                    return index + 1;
                }
            }
            ';' if depth == 0 => return index + 1,
            ',' if depth == 0 && !item => return index + 1,
            _ => {}
        }
    }
    tokens.len()
}

/// Whether the tokens inside an attribute's brackets expect a `clippy::disallowed_*` lint, and the reason they give.
fn disallowed_expect(inside: &[Lexed]) -> Option<String> {
    let expects = inside.iter().any(|lexed| lexed.token == Token::Ident("expect".to_owned()));
    let disallowed = inside.windows(3).any(|three| {
        ident(three.first()) == Some("clippy")
            && is(three.get(1), &Token::PathSeparator)
            && ident(three.get(2)).is_some_and(|name| name.starts_with("disallowed_"))
    });
    if !(expects && disallowed) {
        return None;
    }
    let reason = inside.windows(3).find_map(|three| match (ident(three.first()), &three[1].token, &three[2].token) {
        (Some("reason"), Token::Punct('='), Token::Text(reason)) => Some(reason.clone()),
        _ => None,
    });
    Some(reason.unwrap_or_default())
}

/// Counts what a source holds. `in_src`: the file is under `src`, where `#[cfg(test)]` items are tests and left out.
fn scan(source: &str, in_src: bool) -> Scan {
    let tokens = lex(source);
    let mut found = Scan::default();
    let mut counted = Vec::with_capacity(tokens.len());
    let mut index = 0;
    while index < tokens.len() {
        let at = |ahead: usize| tokens.get(index + ahead);
        if is(at(0), &Token::Punct('#')) && is(at(1), &Token::Punct('!')) && is(at(2), &Token::Punct('[')) {
            let end = closing(&tokens, index + 2);
            if let Some(reason) = disallowed_expect(tokens.get(index + 3..end.saturating_sub(1)).unwrap_or_default()) {
                found.module_expects.push((tokens[index].line, reason));
            }
            index = end;
            continue;
        }
        if is(at(0), &Token::Punct('#')) && is(at(1), &Token::Punct('[')) {
            let mut attributes = Vec::new();
            let mut end = index;
            while is(tokens.get(end), &Token::Punct('#')) && is(tokens.get(end + 1), &Token::Punct('[')) {
                let close = closing(&tokens, end + 1);
                attributes.push(tokens.get(end + 2..close.saturating_sub(1)).unwrap_or_default());
                end = close;
            }
            let test = attributes.iter().any(|inside| {
                let words: Vec<&Token> = inside.iter().map(|lexed| &lexed.token).collect();
                words == [&Token::Ident("cfg".to_owned()), &Token::Punct('('), &Token::Ident("test".to_owned()), &Token::Punct(')')]
            });
            if test && in_src {
                let item = item_end(&tokens, end);
                let declaration: Vec<&Token> = tokens[end..item].iter().map(|lexed| &lexed.token).collect();
                let module_file = matches!(declaration.as_slice(), [.., Token::Ident(word), Token::Ident(_), Token::Punct(';')] if word == "mod");
                let path = attributes.iter().find_map(|inside| match inside {
                    [Lexed { token: Token::Ident(word), .. }, Lexed { token: Token::Punct('='), .. }, Lexed { token: Token::Text(path), .. }] if word == "path" => Some(path.clone()),
                    _ => None,
                });
                if let (true, Some(path)) = (module_file, path) {
                    found.test_modules.push(path);
                }
                index = item;
                continue;
            }
            for inside in &attributes {
                if disallowed_expect(inside).is_some() {
                    found.item_expects.push(tokens[index].line);
                }
            }
            index = end;
            continue;
        }
        counted.push(&tokens[index]);
        index += 1;
    }
    count(&counted, &mut found);
    found
}

/// The untyped names a file imports from `serde_json`, under the names it uses, and the tokens of those imports.
fn imports(tokens: &[&Lexed]) -> (BTreeMap<String, &'static str>, BTreeSet<usize>) {
    let mut names = BTreeMap::new();
    let mut inside = BTreeSet::new();
    for (start, lexed) in tokens.iter().enumerate() {
        let leading = usize::from(is(tokens.get(start + 1).copied(), &Token::PathSeparator));
        if lexed.token != Token::Ident("use".to_owned()) || ident(tokens.get(start + 1 + leading).copied()) != Some("serde_json") {
            continue;
        }
        let end = (start..tokens.len()).find(|&index| tokens[index].token == Token::Punct(';')).unwrap_or(tokens.len());
        inside.extend(start..end);
        for index in start..end {
            let Some(name) = ident(tokens.get(index).copied()) else { continue };
            let Some(&untyped) = UNTYPED_NAMES.iter().find(|&&untyped| untyped == name) else { continue };
            let alias = match (ident(tokens.get(index + 1).copied()), ident(tokens.get(index + 2).copied())) {
                (Some("as"), Some(alias)) => alias,
                _ => name,
            };
            names.insert(alias.to_owned(), untyped);
        }
    }
    (names, inside)
}

fn count(tokens: &[&Lexed], found: &mut Scan) {
    let (names, imports) = imports(tokens);
    let at = |index: usize| tokens.get(index).copied();
    for (index, lexed) in tokens.iter().enumerate() {
        let before = index.checked_sub(1).and_then(at);
        let after = at(index + 1);
        match &lexed.token {
            Token::Ident(word) if word == "serde_json" => {
                if is(after, &Token::PathSeparator) && ident(at(index + 2)) != Some("Number") {
                    found.value_tokens.push(lexed.line);
                }
            }
            Token::Ident(word) if names.contains_key(word) && !imports.contains(&index) => {
                let qualified = is(before, &Token::PathSeparator) || is(before, &Token::Punct('.'));
                let called = names[word] != "json" || is(after, &Token::Punct('!'));
                if !qualified && called {
                    found.value_tokens.push(lexed.line);
                }
            }
            Token::Punct('[') => {
                let indexed = match before.map(|lexed| &lexed.token) {
                    Some(Token::Ident(word)) => !KEYWORDS.contains(&word.as_str()),
                    Some(Token::Punct(')' | ']' | '?')) => true,
                    _ => false,
                };
                let key = matches!(at(index + 1).map(|lexed| &lexed.token), Some(Token::Text(_)));
                if indexed && key && is(at(index + 2), &Token::Punct(']')) {
                    found.string_key_index.push(lexed.line);
                }
            }
            Token::Equal | Token::NotEqual => {
                let named = ident(before).is_some_and(|word| !KEYWORDS.contains(&word));
                let literal = matches!(after.map(|lexed| &lexed.token), Some(Token::Text(text)) if text.chars().count() >= 2);
                if named && literal {
                    found.literal_compares.push(lexed.line);
                }
            }
            Token::Ident(_) | Token::Text(_) | Token::PathSeparator | Token::Punct(_) | Token::Other => {}
        }
    }
}

/// The counts of a file, or the totals of a crate.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Counts {
    #[serde(default, skip_serializing_if = "is_zero")]
    value_tokens: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    string_key_index: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    literal_compares: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    expect_roots: usize,
}

#[expect(clippy::trivially_copy_pass_by_ref, reason = "serde hands the field over by reference")]
fn is_zero(count: &usize) -> bool {
    *count == 0
}

const KINDS: [&str; 4] = ["value_tokens", "string_key_index", "literal_compares", "expect_roots"];

impl Counts {
    fn get(&self, kind: &str) -> usize {
        match kind {
            "value_tokens" => self.value_tokens,
            "string_key_index" => self.string_key_index,
            "literal_compares" => self.literal_compares,
            _ => self.expect_roots,
        }
    }

}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CrateRecord {
    /// The crate's counts outside the exception modules.
    value_tokens: usize,
    string_key_index: usize,
    literal_compares: usize,
    expect_roots: usize,
    /// Every file with a count, by its path in the crate, the exception modules too.
    files: BTreeMap<String, Counts>,
}

impl CrateRecord {
    fn totals(&self) -> Counts {
        Counts {
            value_tokens: self.value_tokens,
            string_key_index: self.string_key_index,
            literal_compares: self.literal_compares,
            expect_roots: self.expect_roots,
        }
    }

    fn count(&mut self, counts: &Counts) {
        self.value_tokens += counts.value_tokens;
        self.string_key_index += counts.string_key_index;
        self.literal_compares += counts.literal_compares;
        self.expect_roots += counts.expect_roots;
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    /// The modules that may carry a module-level `expect` of a `clippy::disallowed_*` lint, by their path in the
    /// repository, with the reason their attribute gives.
    exceptions: BTreeMap<String, String>,
    crates: BTreeMap<String, CrateRecord>,
}

/// One file as measured: its crate, its path in the crate, what it holds and its text, for the lines of a report.
struct Measured {
    krate: String,
    in_crate: String,
    scan: Scan,
    counts: Counts,
    text: String,
}

/// Whether a file is the root of a target: a library, a program, a test, an example, a benchmark or a build script.
fn is_crate_root(in_crate: &str) -> bool {
    let parts: Vec<&str> = in_crate.split('/').collect();
    matches!(
        parts.as_slice(),
        ["src", "lib.rs" | "main.rs"]
            | ["build.rs"]
            | ["src", "bin", _]
            | ["src", "bin", _, "main.rs"]
            | ["tests" | "examples" | "benches", _]
            | ["examples" | "benches", _, "main.rs"]
    )
}

/// Measures the sources (path in the repository, text): every file but those that are tests declared from `src`.
fn measure(sources: &[(String, String)]) -> BTreeMap<String, Measured> {
    let mut measured = BTreeMap::new();
    let mut tests = BTreeSet::new();
    for (path, text) in sources {
        let Some((krate, in_crate)) = path.strip_prefix("crates/").and_then(|rest| rest.split_once('/')) else { continue };
        let scan = scan(text, in_crate.starts_with("src/"));
        let folder = path.rsplit_once('/').map_or("", |(folder, _)| folder);
        tests.extend(scan.test_modules.iter().map(|module| format!("{folder}/{module}")));
        let counts = Counts {
            value_tokens: scan.value_tokens.len(),
            string_key_index: scan.string_key_index.len(),
            literal_compares: scan.literal_compares.len(),
            expect_roots: usize::from(is_crate_root(in_crate) && !scan.module_expects.is_empty()),
        };
        let file = Measured { krate: krate.to_owned(), in_crate: in_crate.to_owned(), scan, counts, text: text.clone() };
        measured.insert(path.clone(), file);
    }
    measured.retain(|path, _| !tests.contains(path));
    measured
}

/// The record that the measured files make, with the module-level exceptions they carry.
fn record_of(measured: &BTreeMap<String, Measured>) -> Record {
    let mut record = Record::default();
    for (path, file) in measured {
        if !is_crate_root(&file.in_crate)
            && let Some((_, reason)) = file.scan.module_expects.first()
        {
            record.exceptions.insert(path.clone(), reason.clone());
        }
    }
    for (path, file) in measured {
        let entry = record.crates.entry(file.krate.clone()).or_default();
        if file.counts != Counts::default() {
            entry.files.insert(file.in_crate.clone(), file.counts);
        }
        if !record.exceptions.contains_key(path) {
            entry.count(&file.counts);
        }
    }
    record
}

/// The lines of a file that hold one kind of count, each once.
fn lines_of(file: &Measured, kind: &str) -> Vec<usize> {
    let mut lines = match kind {
        "value_tokens" => file.scan.value_tokens.clone(),
        "string_key_index" => file.scan.string_key_index.clone(),
        "literal_compares" => file.scan.literal_compares.clone(),
        _ => file.scan.module_expects.iter().map(|(line, _)| *line).collect(),
    };
    lines.dedup();
    lines
}

/// Every difference between the record and the files, as lines of a report.
fn differences(recorded: &Record, measured: &BTreeMap<String, Measured>) -> Vec<String> {
    let found = record_of(measured);
    let mut problems = Vec::new();
    for (path, file) in measured {
        let listed = recorded.exceptions.get(path);
        for (line, reason) in &file.scan.module_expects {
            if is_crate_root(&file.in_crate) {
                continue;
            }
            match listed {
                None => problems.push(format!("{path}:{line}: a module-level exception that is not in the list of exceptions")),
                Some(listed) if file.scan.module_expects.iter().all(|(_, reason)| reason != listed) => {
                    problems.push(format!("{path}:{line}: the exception says {reason:?}, the list says {listed:?}"));
                }
                Some(_) => {}
            }
        }
        if listed.is_none() {
            for line in &file.scan.item_expects {
                problems.push(format!("{path}:{line}: an exception on one item: move the untyped code into a module that says why"));
            }
        }
    }
    for path in recorded.exceptions.keys() {
        if !found.exceptions.contains_key(path) {
            problems.push(format!("{path}: listed as an exception, but carries no module-level expect of a disallowed lint"));
        }
    }
    let crates: BTreeSet<&String> = recorded.crates.keys().chain(found.crates.keys()).collect();
    let empty = CrateRecord::default();
    for krate in crates {
        let (was, now) = (recorded.crates.get(krate).unwrap_or(&empty), found.crates.get(krate).unwrap_or(&empty));
        let files: BTreeSet<&String> = was.files.keys().chain(now.files.keys()).collect();
        for in_crate in files {
            let (before, after) = (was.files.get(in_crate).copied().unwrap_or_default(), now.files.get(in_crate).copied().unwrap_or_default());
            let path = format!("crates/{krate}/{in_crate}");
            for kind in KINDS {
                let (before, after) = (before.get(kind), after.get(kind));
                if after > before {
                    problems.push(format!("{path}: {kind} rose from {before} to {after}:"));
                    let file = &measured[&path];
                    let text: Vec<&str> = file.text.lines().collect();
                    for line in lines_of(file, kind) {
                        problems.push(format!("  {path}:{line}: {}", text.get(line - 1).map_or("", |text| text.trim())));
                    }
                } else if after < before {
                    problems.push(format!("{path}: {kind} fell from {before} to {after}: write it down in {RECORD}"));
                }
            }
        }
        for kind in KINDS {
            let (before, after) = (was.totals().get(kind), now.totals().get(kind));
            if before != after {
                problems.push(format!("{krate}: the record's {kind} is {before}, the files hold {after}"));
            }
        }
    }
    problems
}

/// The record as the file holds it.
fn record_text(record: &Record) -> String {
    json::dumps(record, &JsonOptions { ensure_ascii: false, indent: Some(Indent::Spaces(1)), sort_keys: true, ..JsonOptions::default() }) + "\n"
}

/// The differences between the record's text and the files, its own form included.
fn problems_of(text: &str, measured: &BTreeMap<String, Measured>) -> Vec<String> {
    let recorded: Record = json::loads(text).expect("the record has the shape of a record");
    let mut problems = differences(&recorded, measured);
    if text != record_text(&recorded) {
        problems.push(format!("{RECORD} is not written the way this test writes it"));
    }
    problems
}

/// The tracked Rust sources of the crates with their text. The record counts what git tracks, so the test needs a git
/// checkout: outside one it fails rather than pass without having counted.
fn tracked_sources(root: &Path) -> Vec<(String, String)> {
    let output = Command::new("git").current_dir(root).args(["ls-files", "-z", "--", "crates"]).output().expect("git runs");
    assert!(output.status.success(), "git ls-files: {}", String::from_utf8_lossy(&output.stderr));
    let listing = String::from_utf8(output.stdout).expect("tracked paths are UTF-8");
    listing
        .split('\0')
        .filter(|path| Path::new(path).extension().is_some_and(|extension| extension == "rs"))
        .map(|path| (path.to_owned(), std::fs::read_to_string(root.join(path)).expect("a tracked source is UTF-8 text")))
        .collect()
}

#[test]
fn no_untyped_json_or_text_enum_is_added_and_every_one_removed_is_written_down() {
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let measured = measure(&tracked_sources(&crate_root.join("../..")));
    let problems = problems_of(&std::fs::read_to_string(crate_root.join(RECORD)).expect("the record"), &measured);
    // Written on every run, so that the record a change needs is at hand.
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("strict_typing.json");
    std::fs::write(&out, record_text(&record_of(&measured))).expect("the measured record is written");
    assert!(problems.is_empty(), "{}\nThe measured record is in {}: copy it over {RECORD} once the change is meant.", problems.join("\n"), out.display());
}

#[cfg(test)]
mod counting {
    use super::*;

    fn counts(source: &str) -> (usize, usize, usize) {
        let found = scan(source, true);
        (found.value_tokens.len(), found.string_key_index.len(), found.literal_compares.len())
    }

    #[test]
    fn untyped_json_counts_where_the_file_imports_it() {
        assert_eq!(counts("use serde_json::Value;"), (1, 0, 0));
        assert_eq!(counts("use serde_json::{Map, Value, json};\nfn f(v: &Value) -> Map<String, Value> { json!({}) }"), (5, 0, 0));
        assert_eq!(counts("fn f() -> serde_json::Value { serde_json::json!(null) }"), (2, 0, 0));
        assert_eq!(counts("use ::serde_json::Value;"), (1, 0, 0));
        assert_eq!(counts("use serde_json::Value as Json;\nfn f(j: Json) -> Json { j }"), (3, 0, 0));
        // The number as the file wrote it is no union.
        assert_eq!(counts("use serde_json::Number;\nfn f(n: serde_json::Number) -> Number { n }"), (0, 0, 0));
        // Without the import, `Value` is another type, `json` a variable, `ValueError` a name of Python's.
        assert_eq!(counts("enum Value { A }\nfn f() -> Value { Value::A }\n// serde_json::Value\nconst T: &str = \"serde_json::Value\";"), (0, 0, 0));
        assert_eq!(counts("use serde_json::json;\nfn f(json: u8) -> u8 { json + 1 }\nfn g() { json!(1); }"), (2, 0, 0));
        assert_eq!(counts("use serde_json::Value;\nfn f(v: &Value) { let x = other::Value::new(); v.Value; }\n/// ValueError"), (2, 0, 0));
    }

    #[test]
    fn a_text_key_index_counts_and_an_array_does_not() {
        assert_eq!(counts(r#"fn f() { v["key"]; v["a"]["b"]; f()["k"]; x?["k"]; v["Cargo.lock"]; v[r"raw"]; self["s"]; }"#), (0, 8, 0));
        assert_eq!(counts(r#"fn f() { vec!["a"]; &["a"]; [("a", 1)]; for x in ["a"] {} let a = ["a"]; m[name]; v[0]; f(["a"]); }"#), (0, 0, 0));
        assert_eq!(counts(r#"fn f() { match s { ["a"] => {} } return ["a"]; x = ["a", "b"]; v[b"k"]; }"#), (0, 0, 0));
        assert_eq!(counts("fn f() { let s = r#\"v[\"k\"]\"#; /* v[\"k\"] /* nested */ v[\"k\"] */ }"), (0, 0, 0));
    }

    #[test]
    fn a_name_compared_with_a_text_of_two_characters_counts() {
        assert_eq!(counts(r#"fn f() { plan.mode == "early"; x != "ab"; x == "я!"; x == r"ab"; *x == "ab"; }"#), (0, 0, 5));
        assert_eq!(counts(r#"fn f() { c == " "; s == ""; c == "\n"; c == "\u{44f}"; x.as_str() == "ab"; "ab" == x; x == 'a'; }"#), (0, 0, 0));
        assert_eq!(counts(r#"fn f() { if x <= "ab" {} if return == "ab" {} x == b"ab"; }"#), (0, 0, 0));
        assert_eq!(counts("fn f() { x == \"a\\\n   b\"; x == \"\\x41\\t\"; }"), (0, 0, 2));
    }

    #[test]
    fn test_items_in_src_are_left_out_and_tests_counted_whole() {
        let source = "#[cfg(test)]\nuse std::sync::Mutex;\nfn f() { v[\"a\"]; }\n#[cfg(test)]\n#[cfg_attr(coverage_nightly, coverage(off))]\nmod tests {\n    fn g() { v[\"b\"]; }\n}\n";
        assert_eq!(scan(source, true).string_key_index, vec![3]);
        assert_eq!(scan(source, false).string_key_index, vec![3, 7]);
        let source = "struct S {\n    #[cfg(test)]\n    a: Vec<u8>,\n    b: u8,\n}\nfn f() { match x { #[cfg(test)] A => v[\"a\"], B => v[\"b\"] } }\n#[cfg(test)] impl<A, B> X for Y { fn g() { v[\"c\"]; } }\nfn h() { v[\"d\"]; }";
        assert_eq!(scan(source, true).string_key_index, vec![6, 8]);
        let source = "#[cfg(test)]\n#[path = \"x_tests.rs\"]\nmod tests;\n#[cfg(test)]\nfn helper() { v[\"a\"] }\n#[cfg(all(test, unix))]\nfn other() { v[\"b\"]; }\n#[cfg(test)]\nmod inline;\n#[path = \"real.rs\"]\nmod real;";
        let found = scan(source, true);
        assert_eq!((found.test_modules, found.string_key_index), (vec!["x_tests.rs".to_owned()], vec![7]));
        // An attribute that runs to the end of the text, and an item that does.
        assert_eq!(scan("#[cfg(test)]\nfn f() { v[\"a\"]", true), Scan::default());
        assert_eq!(scan("#[derive(Debug", true), Scan::default());
    }

    #[test]
    fn exceptions_are_read_with_their_reasons() {
        let source = "//! A module.\n#![expect(clippy::disallowed_types, reason = \"the tree is the format\")]\n#![cfg_attr(test, expect(clippy::disallowed_macros, reason = \"tests\"))]\n#![expect(clippy::panic, reason = \"other\")]\n#[expect(clippy::disallowed_methods, reason = \"one item\")]\nfn f() {}\n#[cfg_attr(windows, expect(clippy::disallowed_methods))]\nfn g() {}\n#[expect(clippy::panic, reason = \"x\")]\nfn h() {}";
        let found = scan(source, true);
        assert_eq!(found.module_expects, vec![(2, "the tree is the format".to_owned()), (3, "tests".to_owned())]);
        assert_eq!(found.item_expects, vec![5, 7]);
        assert_eq!(disallowed_expect(&lex("allow(clippy::disallowed_types)")), None);
        assert_eq!(disallowed_expect(&lex("expect(clippy::disallowed_types)")), Some(String::new()));
    }

    #[test]
    fn literals_lifetimes_and_characters_do_not_end_or_start_text() {
        let source = "fn f<'a>(x: &'a str) -> char { let q = '\"'; let e = '\\''; let u = '\\u{44f}'; let b = b'\"'; let s = b\"v[\\\"k\\\"]\"; let r = br#\"v[\"k\"]\"#; let c = c\"x\"; let i = r#type; 'outer: loop {} v[\"k\"]; q }";
        assert_eq!(counts(source), (0, 1, 0));
        assert_eq!(lex("r#type").first().map(|lexed| lexed.token.clone()), Some(Token::Ident("type".to_owned())));
        assert_eq!(lex("\"a\\\"b\\\\\\0\\r\\q").first().map(|lexed| lexed.token.clone()), Some(Token::Text("a\"b\\\0\rq".to_owned())));
        assert_eq!(lex("\"\\").first().map(|lexed| lexed.token.clone()), Some(Token::Text(String::new())));
        assert_eq!(lex("r##\"a\"#b\"##").first().map(|lexed| lexed.token.clone()), Some(Token::Text("a\"#b".to_owned())));
        assert_eq!(lex("1.5e-7 0x1F").iter().filter(|lexed| lexed.token == Token::Other).count(), 4);
        let lines: Vec<usize> = lex("a\n/* b\n c */ d\n\"e\nf\" g").iter().map(|lexed| lexed.line).collect();
        assert_eq!(lines, vec![1, 3, 4, 5]);
        let tokens = |source: &str| lex(source).into_iter().map(|lexed| lexed.token).collect::<Vec<_>>();
        assert_eq!(tokens("\"abc"), [Token::Text("abc".to_owned())]);
        assert_eq!(tokens("a /* b/c d*e */ f /* open"), [Token::Ident("a".to_owned()), Token::Ident("f".to_owned())]);
        assert_eq!(tokens("br\"a\" cr#\"b\"# cr\"c\" br#\"d\"#"), [Token::Other, Token::Other, Token::Other, Token::Other]);
        assert_eq!(tokens("r#\"open"), [Token::Text("open".to_owned())]);
        assert_eq!(tokens("br#x"), [Token::Ident("br".to_owned()), Token::Punct('#'), Token::Ident("x".to_owned())]);
    }

    #[test]
    fn what_only_looks_like_an_attribute_or_an_index_is_read_as_code() {
        // Not attributes: a lone `#!`, a `#` alone, a `#` after an attribute; brackets nested inside one.
        assert_eq!(counts("#!x\n# x\n#[a] # b\nfn f() { v[\"a\"]; }"), (0, 1, 0));
        assert_eq!(scan("#[cfg_attr(a, b([1]))]\nfn f() { v[\"a\"]; }", true).string_key_index, vec![2]);
        // A test field that ends its struct, a test import renamed, a test module with another attribute.
        let source = "struct S {\n    #[cfg(test)]\n    a: u8\n}\n#[cfg(test)]\nuse a as b;\n#[cfg(test)]\n#[doc = \"x\"]\nmod m;\nfn f() { v[\"a\"]; }";
        let found = scan(source, true);
        assert_eq!((found.test_modules.len(), found.string_key_index), (0, vec![10]));
        assert_eq!(counts("extern crate serde_json;\nfn f() { v[\"a\", \"b\"]; }"), (0, 0, 0));
        assert_eq!(disallowed_expect(&lex("expect(clippy, clippy::panic)")), None);
    }

    fn sources(files: &[(&str, &str)]) -> Vec<(String, String)> {
        files.iter().map(|(path, text)| ((*path).to_owned(), (*text).to_owned())).collect()
    }

    #[test]
    fn crate_roots_and_test_modules_are_told_apart() {
        for root in ["src/lib.rs", "src/main.rs", "build.rs", "src/bin/a.rs", "src/bin/a/main.rs", "tests/a.rs", "examples/a.rs", "benches/a/main.rs"] {
            assert!(is_crate_root(root), "{root}");
        }
        for module in ["src/a.rs", "src/bin/trace/run.rs", "tests/support/mod.rs", "src/a/mod.rs"] {
            assert!(!is_crate_root(module), "{module}");
        }
        let tree = sources(&[
            ("crates/a/src/lib.rs", "#![expect(clippy::disallowed_types, reason = \"temporary\")]\n#[cfg(test)]\n#[path = \"x_tests.rs\"]\nmod tests;"),
            ("crates/a/src/x_tests.rs", "use serde_json::Value;"),
            ("crates/a/tests/t.rs", "use serde_json::Value;"),
            ("README.md", "not a crate"),
        ]);
        let measured = measure(&tree);
        assert_eq!(measured.keys().collect::<Vec<_>>(), ["crates/a/src/lib.rs", "crates/a/tests/t.rs"]);
        let record = record_of(&measured);
        assert_eq!(record.crates.get("a").map(CrateRecord::totals), Some(Counts { value_tokens: 1, expect_roots: 1, ..Counts::default() }));
    }

    #[test]
    fn every_difference_is_named_with_its_file_and_lines() {
        let tree = sources(&[
            ("crates/a/src/lib.rs", "#![expect(clippy::disallowed_types, reason = \"temporary\")]\nmod listed;\nmod unlisted;"),
            ("crates/a/src/listed.rs", "#![expect(clippy::disallowed_types, reason = \"the format\")]\nuse serde_json::Value;\n#[expect(clippy::disallowed_methods, reason = \"inside\")]\nfn f() {}"),
            ("crates/a/src/unlisted.rs", "#![expect(clippy::disallowed_types, reason = \"sneaked\")]\nuse serde_json::Value;\nfn f(v: &Value) { v[\"k\"]; }\n#[expect(clippy::disallowed_methods)]\nfn g() {}"),
            ("crates/b/src/lib.rs", "fn f() { x == \"ab\"; }"),
        ]);
        let measured = measure(&tree);
        let exact = record_of(&measured);
        assert_eq!(exact.exceptions.len(), 2);
        assert!(differences(&exact, &measured).is_empty());
        let mut fixed = exact.clone();
        fixed.exceptions.remove("crates/a/src/unlisted.rs");
        let problems = differences(&fixed, &measured);
        assert!(problems.contains(&"crates/a/src/unlisted.rs:1: a module-level exception that is not in the list of exceptions".to_owned()), "{problems:#?}");
        assert!(problems.contains(&"crates/a/src/unlisted.rs:4: an exception on one item: move the untyped code into a module that says why".to_owned()), "{problems:#?}");
        assert!(!problems.iter().any(|problem| problem.starts_with("crates/a/src/listed.rs:3")), "{problems:#?}");

        let mut wrong = exact.clone();
        wrong.exceptions.insert("crates/a/src/listed.rs".to_owned(), "another reason".to_owned());
        wrong.exceptions.insert("crates/a/src/gone.rs".to_owned(), "gone".to_owned());
        let problems = differences(&wrong, &measured);
        assert!(problems.contains(&"crates/a/src/listed.rs:1: the exception says \"the format\", the list says \"another reason\"".to_owned()), "{problems:#?}");
        assert!(problems.contains(&"crates/a/src/gone.rs: listed as an exception, but carries no module-level expect of a disallowed lint".to_owned()), "{problems:#?}");

        let mut lower = exact.clone();
        let files = &mut lower.crates.get_mut("a").expect("crate a").files;
        let unlisted = files.get_mut("src/unlisted.rs").expect("the file");
        (unlisted.value_tokens, unlisted.string_key_index) = (1, 0);
        files.insert("src/old.rs".to_owned(), Counts { literal_compares: 2, ..Counts::default() });
        let b = lower.crates.get_mut("b").expect("crate b");
        b.literal_compares = 0;
        b.files.get_mut("src/lib.rs").expect("the file").literal_compares = 0;
        lower.crates.insert("c".to_owned(), CrateRecord::default());
        let problems = differences(&lower, &measured);
        assert_eq!(
            problems,
            [
                "crates/a/src/old.rs: literal_compares fell from 2 to 0: write it down in tests/data/strict_typing.json",
                "crates/a/src/unlisted.rs: value_tokens rose from 1 to 2:",
                "  crates/a/src/unlisted.rs:2: use serde_json::Value;",
                "  crates/a/src/unlisted.rs:3: fn f(v: &Value) { v[\"k\"]; }",
                "crates/a/src/unlisted.rs: string_key_index rose from 0 to 1:",
                "  crates/a/src/unlisted.rs:3: fn f(v: &Value) { v[\"k\"]; }",
                "crates/b/src/lib.rs: literal_compares rose from 0 to 1:",
                "  crates/b/src/lib.rs:1: fn f() { x == \"ab\"; }",
                "b: the record's literal_compares is 0, the files hold 1",
            ]
        );
        let mut root = exact;
        root.crates.get_mut("a").expect("crate a").files.get_mut("src/lib.rs").expect("the root").expect_roots = 0;
        root.crates.get_mut("a").expect("crate a").expect_roots = 0;
        let problems = differences(&root, &measured);
        assert_eq!(problems, ["crates/a/src/lib.rs: expect_roots rose from 0 to 1:", "  crates/a/src/lib.rs:1: #![expect(clippy::disallowed_types, reason = \"temporary\")]", "a: the record's expect_roots is 0, the files hold 1"]);
    }

    #[test]
    fn the_record_reads_back_from_its_own_text() {
        let measured = measure(&sources(&[("crates/a/src/lib.rs", "fn f() { x == \"ab\"; v[\"k\"]; }")]));
        let record = record_of(&measured);
        let text = record_text(&record);
        assert_eq!(json::loads::<Record>(&text).expect("the record"), record);
        assert!(json::loads::<Record>(r#"{"exceptions": {}, "crates": {}, "other": 1}"#).is_err());
        assert!(problems_of(&text, &measured).is_empty());
        let compact = json::dumps(&record, &JsonOptions::default());
        assert_eq!(problems_of(&compact, &measured), ["tests/data/strict_typing.json is not written the way this test writes it"]);
        let tracked = tracked_sources(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
        assert!(tracked.iter().any(|(path, _)| path.ends_with("ks-train/tests/strict_typing.rs")));
        assert_eq!(text, "{\n \"crates\": {\n  \"a\": {\n   \"expect_roots\": 0,\n   \"files\": {\n    \"src/lib.rs\": {\n     \"literal_compares\": 1,\n     \"string_key_index\": 1\n    }\n   },\n   \"literal_compares\": 1,\n   \"string_key_index\": 1,\n   \"value_tokens\": 0\n  }\n },\n \"exceptions\": {}\n}\n");
    }
}
