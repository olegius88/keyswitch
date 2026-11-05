//! A new corpus from a frozen one: a test-only holdout from unread public sources (`freeze_context_action_holdout`) and
//! fitting splits extended with short utterances (`freeze_context_action_fitting`). No model, dictionary score or text of
//! any test row takes part in selection, and no test text is printed. Nothing is overwritten.
use ks_cast::To as _;
use crate::corpus::{CorpusRow, FITTING_SPLITS, checksum, load_rows, read_json};
use crate::physical::{expanded_aliases, hex, hex_digest, physical};
use flate2::Compression;
use flate2::read::MultiGzDecoder;
use flate2::write::GzEncoder;
use ks_context::context_action_features::words_of;
use ks_text::{pystr, ucd};
use serde_json::{Map, Value, json};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};

const CORPUS_BEFORE_WINDOW_CHARACTERS: usize = 96;
const CORPUS_AFTER_WINDOW_CHARACTERS: usize = 64;
const CORPUS_LITERAL_TAIL_MAX_CHARACTERS: usize = 64;
const COMMANDS_PER_ALIAS_FAMILY: usize = 2;
pub const DEFAULT_MAX_SENTENCES_PER_DOCUMENT: usize = 12;
pub const HOLDOUT_MAX_DOCUMENTS_PER_SOURCE: usize = 8000;
pub const FITTING_DEFAULT_MAX_DOCUMENTS_PER_SOURCE: usize = 12000;
const MAX_TATOEBA_SENTENCE_CHARACTERS: usize = 400;
const TATOEBA_SAMPLE_PER_MILLE: u64 = 40;
const PER_MILLE_SCALE: u64 = 1000;
const FITTING_SHARES: [(u64, &str); 3] = [(78, "train"), (89, "development"), (100, "calibration")];
const SPLIT_BUCKET_COUNT: u64 = 100;
const DETERMINISTIC_DRAW_HEX_DIGITS: usize = 16;
const TYPO_SOURCE_MIN_CHARACTERS: usize = 4;
const TYPO_SOURCE_MAX_CHARACTERS: usize = 64;
const TRANSPOSE_TAIL_MARGIN_CHARACTERS: usize = 2;
const COMMAND_NAME_MIN_CHARACTERS: usize = 3;
const COMMAND_NAME_MAX_CHARACTERS: usize = 16;
const PRIOR_TEST: &str = "prior-accessed-test";
const ALIAS_SCOPE: &str = "expanded physical aliases of each held-out row's form, lemmas and typo variants";
const TATOEBA_BASE: &str = "https://downloads.tatoeba.org/exports/per_language/";
const CAPTURED_TATOEBA: &str = "model/context_v1/captured/tatoeba-authors.tsv.xz";
const PINS: [(&str, &str); 12] = [
    ("UD_Russian-GSD", "9acc9d677327043bd416fcc89e4b3407c620d885"),
    ("UD_English-GUM", "b58e74bc22d17220c9198864c253a50a897bf27f"),
    ("UD_Russian-PUD", "1228be1571bca125ac19c922ef52ae71d3cdbb83"),
    ("UD_English-PUD", "e173a1be1b442faf34e7d5a502189ad5d9d1e197"),
    ("UD_English-LinES", "07a998d3fe0fa4e2bc6aaf651692615f89458a1a"),
    ("UD_Russian-SynTagRus", "6377522610550b696fcc70d39074d2ce03da0e7b"),
    ("UD_English-ParTUT", "9cb91499ada4e284dfad3ea69b3b1dd10d2c516a"),
    ("UD_English-GENTLE", "93a5069df8ded256c3e038938b9ea17baf75c73c"),
    ("UD_Russian-Poetry", "f23cc60d67743d3229731dca5059650f1ec41532"),
    ("UD_English-LittlePrince", "a1936378cd57c9cda6cb57da938b164760a42529"),
    ("UD_English-CTeTex", "3208ccc0be5c003d8357ad52ca2b359ae31eb6a4"),
    ("UD_English-Atis", "dce409dd1b526745ab0980360ed2b9f803e93531"),
];
const SID_CONTENTS_URL: &str = "https://deb.debian.org/debian/dists/sid/main/Contents-amd64.gz";
/// Debian-style Contents indexes the freezer accepts: (Contents URL, `InRelease` URL, path in `InRelease`, source, document prefix, description).
const DEB_SOURCES: [(&str, &str, &str, &str, &str, &str); 2] = [
    (SID_CONTENTS_URL, "https://deb.debian.org/debian/dists/sid/InRelease", "main/Contents-amd64.gz", "Debian-sid-main-amd64", "debian-package-component:", "Debian sid main amd64 Contents"),
    ("https://archive.ubuntu.com/ubuntu/dists/noble/Contents-amd64.gz", "https://archive.ubuntu.com/ubuntu/dists/noble/InRelease", "Contents-amd64.gz", "Ubuntu-noble-amd64", "ubuntu-package-component:", "Ubuntu noble amd64 Contents, all components"),
];
const ARCH_MIRROR: &str = "https://geo.mirror.pkgbuild.com/";
const ARCH_REPOSITORIES: [&str; 2] = ["core", "extra"];
const ALPINE_SOURCES: [(&str, &str, &str, &str); 2] = [
    ("https://dl-cdn.alpinelinux.org/alpine/v3.21/main/x86_64/", "Alpine-x86_64", "alpine-package-component:", "Alpine Linux v3.21 main x86_64 package index"),
    ("https://dl-cdn.alpinelinux.org/alpine/v3.21/community/x86_64/", "Alpine-community-x86_64", "alpine-community-package-component:", "Alpine Linux v3.21 community x86_64 package index"),
];
const RPM_SOURCES: [(&str, &str, &str, &str); 3] = [
    ("https://dl.fedoraproject.org/pub/fedora/linux/releases/43/Everything/x86_64/os/", "Fedora-x86_64", "fedora-package-component:", "Fedora 43 Everything x86_64 package index"),
    ("https://download.opensuse.org/tumbleweed/repo/oss/", "openSUSE-x86_64", "opensuse-package-component:", "openSUSE Tumbleweed oss x86_64 package index"),
    ("https://dl.fedoraproject.org/pub/fedora/linux/development/rawhide/Everything/x86_64/os/", "Fedora-rawhide-x86_64", "fedora-rawhide-package-component:", "Fedora Rawhide Everything x86_64 package index"),
];

#[derive(Clone, Debug)]
pub struct SurfaceToken {
    pub identifier: String,
    pub form: String,
    pub lemmas: Vec<String>,
    upos: String,
    features: String,
    misc: String,
}

#[derive(Clone, Debug)]
pub struct Sentence {
    identifier: String,
    document: String,
    source: String,
    filename: String,
    text: Option<String>,
    pub tokens: Vec<SurfaceToken>,
}

#[derive(Default)]
pub(crate) struct Union {
    pub(crate) parents: HashMap<String, String>,
}

impl Union {
    pub(crate) fn find(&mut self, value: &str) -> String {
        let mut current = value.to_owned();
        self.parents.entry(current.clone()).or_insert_with(|| current.clone());
        let mut path = Vec::new();
        while self.parents[&current] != current {
            let next = self.parents[&current].clone();
            path.push(current);
            current = next;
        }
        for previous in path {
            self.parents.insert(previous, current.clone());
        }
        current
    }

    pub(crate) fn join(&mut self, left: &str, right: &str) {
        let (a, b) = (self.find(left), self.find(right));
        if a != b {
            if a > b {
                self.parents.insert(a, b);
            } else {
                self.parents.insert(b, a);
            }
        }
    }
}

pub(crate) fn row_identifier(sentence: &Sentence, token: &SurfaceToken) -> String {
    format!("{}:{}:{}:{}", sentence.source, sentence.filename, sentence.identifier, token.identifier)
}

/// `family_aliases`: surface and every annotated lemma are joined, including multiword components.
pub(crate) fn family_aliases(token: &SurfaceToken, identifier: &str) -> Vec<String> {
    let mut values: BTreeSet<String> = BTreeSet::new();
    values.insert(physical(&token.form));
    values.extend(token.lemmas.iter().map(|lemma| physical(lemma)));
    values.extend(crate::corpus::typo_variants(&token.form, identifier).iter().map(|variant| physical(variant)));
    values.into_iter().collect()
}

fn misc_value<'a>(misc: &'a str, name: &str) -> Option<&'a str> {
    // `dict(part.split("=", 1) ...)`: the last of equal names.
    misc.split('|').filter_map(|part| part.split_once('=')).rfind(|pair| pair.0 == name).map(|pair| pair.1)
}

fn unescape_space(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\\' {
            let replacement = match characters.peek() {
                Some('s') => Some(' '),
                Some('t') => Some('\t'),
                Some('n') => Some('\n'),
                Some('r') => Some('\r'),
                Some('p') => Some('|'),
                Some('\\') => Some('\\'),
                _ => None,
            };
            if let Some(replacement) = replacement {
                result.push(replacement);
                characters.next();
                continue;
            }
        }
        result.push(character);
    }
    result
}

fn is_digits(text: &str) -> bool {
    pystr::isdigit(text)
}

/// `surface_tokens`.
fn surface_tokens(lines: &[Vec<String>]) -> Result<Vec<SurfaceToken>, String> {
    let members: HashMap<i64, &Vec<String>> = lines.iter().filter(|parts| is_digits(&parts[0])).map(|parts| (ks_text::py_int(&parts[0]).expect("digits").to::<i64>(), parts)).collect();
    let mut result = Vec::new();
    let mut covered: BTreeSet<i64> = BTreeSet::new();
    for parts in lines {
        let identifier = &parts[0];
        if identifier.contains('.') {
            continue;
        }
        let lemmas: Vec<String> = if identifier.contains('-') {
            let (begin, end) = identifier.split_once('-').ok_or("invalid CoNLL-U range")?;
            let (begin, end) = (begin.parse::<i64>().map_err(|error| error.to_string())?, end.parse::<i64>().map_err(|error| error.to_string())?);
            if begin > end || (begin..=end).any(|index| covered.contains(&index) || !members.contains_key(&index)) {
                return Err("invalid or overlapping CoNLL-U multiword token".to_owned());
            }
            covered.extend(begin..=end);
            (begin..=end).map(|index| if members[&index][2] == "_" { members[&index][1].clone() } else { members[&index][2].clone() }).collect()
        } else if covered.contains(&identifier.parse::<i64>().map_err(|error| error.to_string())?) {
            continue;
        } else if parts[2] != "_" {
            vec![parts[2].clone()]
        } else {
            vec![parts[1].clone()]
        };
        result.push(SurfaceToken { identifier: identifier.clone(), form: parts[1].clone(), lemmas, upos: parts[3].clone(), features: parts[5].clone(), misc: parts[9].clone() });
    }
    Ok(result)
}

/// `read_conllu`.
pub(crate) fn read_conllu(path: &Path, source: &str) -> Result<Vec<Sentence>, String> {
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let mut document = format!("{source}:unmarked-file:{name}");
    let (mut document_serial, mut sentence_serial) = (0, 0);
    let mut comments: HashMap<String, String> = HashMap::new();
    let mut tokens: Vec<Vec<String>> = Vec::new();
    let mut sentences = Vec::new();
    let text = std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut complete = |comments: &HashMap<String, String>, tokens: &[Vec<String>], document: &str, sentences: &mut Vec<Sentence>| -> Result<(), String> {
        sentence_serial += 1;
        sentences.push(Sentence {
            identifier: comments.get("sent_id").cloned().unwrap_or_else(|| format!("{name}:sentence:{sentence_serial}")),
            document: document.to_owned(),
            source: source.to_owned(),
            filename: name.clone(),
            text: comments.get("text").cloned(),
            tokens: surface_tokens(tokens)?,
        });
        Ok(())
    };
    for raw in text.split('\n') {
        let line = raw.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            if !tokens.is_empty() {
                complete(&comments, &tokens, &document, &mut sentences)?;
            }
            comments.clear();
            tokens.clear();
            continue;
        }
        if let Some(rest) = line.strip_prefix('#') {
            let stripped = pystr::strip(rest);
            let (comment, value) = match stripped.split_once(" = ") {
                Some((comment, value)) => (comment, Some(value)),
                None => (stripped, None),
            };
            if matches!(comment, "newdoc" | "newdoc id" | "newdoc_id") {
                document_serial += 1;
                document = match value {
                    Some(value) => format!("{source}:{value}"),
                    None => format!("{source}:{name}:anonymous:{document_serial}"),
                };
            }
            comments.insert(comment.to_owned(), value.unwrap_or("").to_owned());
            continue;
        }
        let columns: Vec<String> = line.split('\t').map(str::to_owned).collect();
        if columns.len() != 10 {
            return Err(format!("CoNLL-U row must have ten columns: {name}"));
        }
        tokens.push(columns);
    }
    if !tokens.is_empty() {
        complete(&comments, &tokens, &document, &mut sentences)?;
    }
    Ok(sentences)
}

fn char_is_space(character: char) -> bool {
    pystr::isspace(character.encode_utf8(&mut [0_u8; 4]))
}

fn char_is_alnum(character: char) -> bool {
    pystr::isalnum(character.encode_utf8(&mut [0_u8; 4]))
}

/// `aligned_text`: the sentence's text with the span of each token, in characters, and how they were found.
fn aligned_text(sentence: &Sentence) -> (Vec<char>, Vec<(usize, usize)>, &'static str) {
    if let Some(text) = &sentence.text {
        let characters: Vec<char> = text.chars().collect();
        let mut position = 0;
        let mut spans = Vec::new();
        let mut whole = true;
        for token in &sentence.tokens {
            while position < characters.len() && char_is_space(characters[position]) {
                position += 1;
            }
            let form: Vec<char> = token.form.chars().collect();
            if !characters[position..].starts_with(&form) {
                whole = false;
                break;
            }
            spans.push((position, position + form.len()));
            position += form.len();
        }
        if whole && characters[position..].iter().all(|character| char_is_space(*character)) {
            return (characters, spans, "exact-text-comment");
        }
    }
    let mut text: Vec<char> = Vec::new();
    let mut spans = Vec::new();
    for (index, token) in sentence.tokens.iter().enumerate() {
        if index == 0 {
            text.extend(unescape_space(misc_value(&token.misc, "SpacesBefore").unwrap_or("")).chars());
        }
        let start = text.len();
        text.extend(token.form.chars());
        spans.push((start, text.len()));
        if let Some(after) = misc_value(&token.misc, "SpacesAfter") {
            text.extend(unescape_space(after).chars());
        } else if misc_value(&token.misc, "SpaceAfter") != Some("No") && index + 1 < sentence.tokens.len() {
            text.push(' ');
        }
    }
    (text, spans, if sentence.text.is_none() { "misc-reconstructed-no-text" } else { "text-alignment-conflict" })
}

/// `token_language`.
fn token_language(form: &str) -> (&'static str, Option<i64>, bool) {
    let folded = pystr::casefold(form);
    let russian = |character: char| ('а'..='я').contains(&character) || character == 'ё';
    let ru = folded.chars().any(russian);
    let en = folded.chars().any(|character| character.is_ascii_lowercase());
    let (language, group) = match (ru, en) {
        (true, true) => ("mixed", None),
        (true, false) => ("ru", Some(1)),
        (false, true) => ("en", Some(0)),
        (false, false) => ("und", None),
    };
    let supported = group.is_some() && folded.chars().all(|character| character.is_ascii() || russian(character)) && !form.chars().any(char_is_space);
    (language, group, supported)
}

fn collected(characters: &[char]) -> String {
    characters.iter().collect()
}

/// `sentence_rows`.
pub(crate) fn sentence_rows(sentence: &Sentence, families: &mut Union) -> Vec<CorpusRow> {
    let (text, spans, alignment) = aligned_text(sentence);
    let mut rows = Vec::new();
    for (index, (token, (start, end))) in sentence.tokens.iter().zip(spans.iter().copied()).enumerate() {
        let previous = if index > 0 { spans[index - 1].1 } else { 0 };
        let following = spans.get(index + 1).map_or(text.len(), |span| span.0);
        let mut tail_end = end;
        while tail_end < text.len() && tail_end - end < CORPUS_LITERAL_TAIL_MAX_CHARACTERS && !char_is_alnum(text[tail_end]) {
            tail_end += 1;
        }
        let (language, group, representable) = token_language(&token.form);
        rows.push(CorpusRow {
            identifier: row_identifier(sentence, token),
            original: token.form.clone(),
            group,
            before: collected(&text[start.saturating_sub(CORPUS_BEFORE_WINDOW_CHARACTERS)..start]),
            after: collected(&text[end..(end + CORPUS_AFTER_WINDOW_CHARACTERS).min(text.len())]),
            lemma: token.lemmas.join(" "),
            family: hex_digest(&families.find(&physical(&token.form))),
            document: sentence.document.clone(),
            language: language.to_owned(),
            source: sentence.source.clone(),
            source_file: sentence.filename.clone(),
            source_sentence: sentence.identifier.clone(),
            source_token: token.identifier.clone(),
            spacing: collected(&text[end..following]),
            space_before: collected(&text[previous..start]),
            literal_tail: collected(&text[end..tail_end]),
            upos: token.upos.clone(),
            features: token.features.clone(),
            misc: token.misc.clone(),
            alignment: alignment.to_owned(),
            layout_representable: representable,
            split: String::new(),
            quarantine_reasons: Vec::new(),
        });
    }
    rows
}

/// `TATOEBA_TOKEN`: `[^\W_]+(?:['’\-][^\W_]+)*|[^\w\s]|_`, in characters.
fn tatoeba_tokens(text: &str) -> Vec<String> {
    let characters: Vec<char> = text.chars().collect();
    let word = |character: char| ucd::re_w(character) && character != '_';
    let mut tokens = Vec::new();
    let mut position = 0;
    while position < characters.len() {
        let character = characters[position];
        if word(character) {
            let mut end = position;
            while end < characters.len() && word(characters[end]) {
                end += 1;
            }
            while end + 1 < characters.len() && matches!(characters[end], '\'' | '’' | '-') && word(characters[end + 1]) {
                end += 1;
                while end < characters.len() && word(characters[end]) {
                    end += 1;
                }
            }
            tokens.push(collected(&characters[position..end]));
            position = end;
        } else if character == '_' || (!ucd::re_w(character) && !char_is_space(character)) {
            tokens.push(character.to_string());
            position += 1;
        } else {
            position += 1;
        }
    }
    tokens
}

fn tatoeba_sentence(identifier: &str, language: &str, text: &str, source: &str, filename: &str) -> Sentence {
    let tokens = tatoeba_tokens(text)
        .into_iter()
        .enumerate()
        .map(|(index, form)| {
            let alnum = form.chars().next().is_some_and(char_is_alnum);
            SurfaceToken { identifier: (index + 1).to_string(), lemmas: vec![form.clone()], upos: if alnum { "X" } else { "PUNCT" }.to_owned(), features: "_".to_owned(), misc: "_".to_owned(), form }
        })
        .collect();
    Sentence { identifier: format!("tatoeba:{language}:{identifier}"), document: format!("{source}:sentence:{identifier}"), source: source.to_owned(), filename: filename.to_owned(), text: Some(text.to_owned()), tokens }
}

fn draw(text: &str) -> u64 {
    u64::from_str_radix(hex_digest(text).get(..DETERMINISTIC_DRAW_HEX_DIGITS).unwrap_or_default(), 16).expect("hex digits")
}

/// `captured_tatoeba_identifiers`: the Tatoeba sentences behind the captured training questions.
fn captured_tatoeba_identifiers(root: &Path) -> Result<HashSet<String>, String> {
    let file = std::fs::File::open(root.join(CAPTURED_TATOEBA)).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    xz2::read::XzDecoder::new(std::io::BufReader::new(file)).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    let text = String::from_utf8(bytes).map_err(|error| error.to_string())?;
    Ok(text.split_inclusive('\n').filter(|line| line.chars().next().is_some_and(|character| pystr::isdigit(character.encode_utf8(&mut [0_u8; 4])))).map(|line| line.split('\t').next().unwrap_or("").to_owned()).collect())
}

/// `read_tatoeba`: one export, thinned deterministically by identifier before selection.
fn read_tatoeba(path: &Path, source: &str, language: &str, namespace: &str, excluded: &HashSet<String>) -> Result<Vec<Sentence>, String> {
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut text = String::new();
    bzip2_rs::DecoderReader::new(std::io::BufReader::new(file)).read_to_string(&mut text).map_err(|error| format!("{name}: {error}"))?;
    let mut sentences = Vec::new();
    for raw in text.split('\n') {
        let line = raw.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            continue;
        }
        let columns: Vec<&str> = line.split('\t').collect();
        if columns.len() != 3 || !is_digits(columns[0]) {
            return Err(format!("Tatoeba export line must be id, language, text: {name}"));
        }
        let (identifier, declared) = (columns[0], columns[1]);
        if declared != language {
            return Err(format!("Tatoeba export {name} holds a {declared} sentence"));
        }
        if excluded.contains(identifier) {
            continue;
        }
        let joined = pystr::split(columns[2]).join(" ");
        if joined.is_empty() || pystr::len(&joined) > MAX_TATOEBA_SENTENCE_CHARACTERS {
            continue;
        }
        if draw(&format!("{namespace}:tatoeba:{identifier}")) % PER_MILLE_SCALE >= TATOEBA_SAMPLE_PER_MILLE {
            continue;
        }
        sentences.push(tatoeba_sentence(identifier, language, &joined, source, &name));
    }
    Ok(sentences)
}

/// `verified_tatoeba_source`: exports named by a TLS receipt whose per-file size and SHA-256 match the payloads.
fn verified_tatoeba_source(directory: &Path, languages: &[String]) -> Result<(Vec<(String, String, PathBuf)>, BTreeMap<String, String>, Value), String> {
    let receipt_path = directory.join("source-receipt.json");
    let receipt = read_json(&receipt_path)?;
    let licence_named = receipt["licence"].as_str().is_some_and(|licence| licence.contains("CC BY"));
    if !receipt["files"].is_object() || receipt["tls_certificate_verification"] != json!(true) || receipt["base_url"] != json!(TATOEBA_BASE) || !licence_named {
        return Err("source receipt does not identify verified TLS Tatoeba downloads under CC BY".to_owned());
    }
    let mut result = Vec::new();
    let mut provenance = BTreeMap::new();
    provenance.insert(receipt_path.display().to_string(), checksum(&receipt_path)?);
    for language in languages {
        let name = format!("{language}_sentences.tsv.bz2");
        let entry = &receipt["files"][&name];
        let path = directory.join(&name);
        let size = std::fs::metadata(&path).map_err(|error| error.to_string())?.len();
        if entry["status"] != json!(200) || entry["language"] != json!(language) || entry["url"] != json!(format!("{TATOEBA_BASE}{language}/{name}")) || entry["sha256"] != json!(checksum(&path)?) || entry["bytes"] != json!(size) {
            return Err(format!("Tatoeba export receipt mismatch: {name}"));
        }
        provenance.insert(path.display().to_string(), checksum(&path)?);
        result.push((format!("Tatoeba-{language}"), language.clone(), path));
    }
    Ok((result, provenance, json!({"source": "Tatoeba per-language sentence exports", "receipt": receipt, "verification": "HTTPS TLS and per-file SHA-256 from the receipt; no upstream signature exists"})))
}

/// `sentence_documents`: a treebank without document markers is sampled by sentence.
fn sentence_documents(sentences: &mut [Sentence]) {
    for sentence in sentences {
        if sentence.document == format!("{}:unmarked-file:{}", sentence.source, sentence.filename) {
            sentence.document = format!("{}:sentence:{}:{}", sentence.source, sentence.filename, sentence.identifier);
        }
    }
}

const LATIN_LOOKALIKES: &str = "ABCEHKMOPTXaceopxy";
const CYRILLIC_LOOKALIKES: &str = "АВСЕНКМОРТХасеорху";
const SAME_KEY_LOOKALIKES: &str = "CcСс";

fn letter_runs(text: &str) -> Vec<&str> {
    text.split(|character: char| !(character.is_ascii_alphabetic() || ('А'..='я').contains(&character) || character == 'Ё' || character == 'ё')).filter(|run| !run.is_empty()).collect()
}

/// `lookalike_typo`: a sentence whose text no writer types: a word in look-alike letters of the other script.
fn lookalike_typo(sentence: &Sentence) -> bool {
    let is_alpha = |character: &char| crate::rows::is_alpha(*character);
    let words: Vec<&str> = sentence.tokens.iter().map(|token| token.form.as_str()).filter(|form| form.chars().any(crate::rows::is_alpha)).collect();
    let mut scripts = Vec::new();
    for form in &words {
        for run in letter_runs(form) {
            let latin: Vec<char> = run.chars().filter(char::is_ascii).collect();
            let cyrillic: Vec<char> = run.chars().filter(|character| !character.is_ascii()).collect();
            if !latin.is_empty() && !cyrillic.is_empty() && (latin.iter().all(|character| LATIN_LOOKALIKES.contains(*character)) || cyrillic.iter().all(|character| CYRILLIC_LOOKALIKES.contains(*character))) {
                return true;
            }
        }
        let letters: Vec<char> = form.chars().filter(is_alpha).collect();
        let latin_letters = letters.iter().any(char::is_ascii);
        let cyrillic_letters = letters.iter().any(|character| crate::rows::cyrillic_letter(*character));
        scripts.push(match (latin_letters, cyrillic_letters) {
            (true, false) => "latin",
            (false, true) => "cyrillic",
            _ => "other",
        });
    }
    for (index, form) in words.iter().enumerate() {
        if !form.chars().filter(is_alpha).all(|character| SAME_KEY_LOOKALIKES.contains(character)) {
            continue;
        }
        let count = |script: &str| scripts.iter().enumerate().filter(|(position, value)| *position != index && **value == script).count();
        let (own, other) = if scripts[index] == "latin" { ("latin", "cyrillic") } else { ("cyrillic", "latin") };
        if count(other) > count(own) {
            return true;
        }
    }
    false
}

/// `select_holdout_sentences`: sentences by hash in documents by hash.
pub(crate) fn select_sentences(sentences: Vec<Sentence>, namespace: &str, max_documents: usize, max_sentences: usize, policy: bool) -> Result<(Vec<Sentence>, Value), String> {
    let mut sources: BTreeMap<String, BTreeMap<String, Vec<([u8; 32], usize)>>> = BTreeMap::new();
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    let mut seen: HashSet<(String, String, String)> = HashSet::new();
    for (index, sentence) in sentences.iter().enumerate() {
        if !seen.insert((sentence.source.clone(), sentence.filename.clone(), sentence.identifier.clone())) {
            return Err("duplicate holdout sentence identity".to_owned());
        }
        *counts.entry(sentence.source.clone()).or_insert(0) += 1;
        // The hash read as an integer: the larger `-rank` the smaller the bytes.
        let rank: [u8; 32] = Sha256::digest(format!("{namespace}:sample:{}:{}", sentence.document, sentence.identifier).as_bytes()).into();
        sources.entry(sentence.source.clone()).or_default().entry(sentence.document.clone()).or_default().push((rank, index));
    }
    let mut chosen: Vec<usize> = Vec::new();
    let mut selected_documents = Map::new();
    let mut available_documents = Map::new();
    for (source, documents) in &mut sources {
        available_documents.insert(source.clone(), json!(documents.len()));
        let mut ranked: Vec<String> = documents.keys().cloned().collect();
        ranked.sort_by_cached_key(|document| hex_digest(&format!("{namespace}:document-sample:{document}")));
        ranked.truncate(max_documents);
        selected_documents.insert(source.clone(), json!(ranked.len()));
        for document in ranked {
            let entries = documents.get_mut(&document).expect("a ranked document");
            entries.sort_unstable();
            chosen.extend(entries.iter().take(max_sentences).map(|entry| entry.1));
        }
    }
    let chosen_set: HashSet<usize> = chosen.into_iter().collect();
    let mut selected: Vec<Sentence> = sentences.into_iter().enumerate().filter(|(index, _)| chosen_set.contains(index)).map(|(_, sentence)| sentence).collect();
    selected.sort_by(|a, b| (&a.source, &a.filename, &a.identifier).cmp(&(&b.source, &b.filename, &b.identifier)));
    let mut report = json!({"available_documents": available_documents, "available_sentences": counts, "selected_documents": selected_documents, "selected_sentences": selected.len(), "max_documents_per_source": max_documents, "max_sentences_per_document": max_sentences});
    if policy {
        report["policy"] = json!("every selected document is a TEST candidate; no train/development/calibration assignment");
    }
    Ok((selected, report))
}

/// What a new word must not be: known to the base, to a test read before, to the prefix curriculum or to earlier exposure.
pub struct Exclusions {
    base_aliases: HashSet<String>,
    prior_test_aliases: HashSet<String>,
    prefix_aliases: HashSet<String>,
    prior_test_families: HashSet<String>,
    exposed_physical: HashSet<String>,
    historical_aliases: HashSet<String>,
    lexicon: HashSet<String>,
    provenance: BTreeMap<String, String>,
}

fn string_items(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => items.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
        Value::Object(table) => table.keys().cloned().collect(),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => Vec::new(),
    }
}

/// The ledger's records of accessed tests, in file order.
fn ledger_memberships(ledger: &Path) -> Result<Vec<(PathBuf, Value)>, String> {
    let mut records = Vec::new();
    if !ledger.is_dir() {
        return Ok(records);
    }
    let mut paths: Vec<PathBuf> = std::fs::read_dir(ledger).map_err(|error| error.to_string())?.filter_map(|entry| entry.ok().map(|entry| entry.path())).collect();
    paths.retain(|path| path.to_string_lossy().ends_with(".access.json"));
    paths.sort();
    for path in paths {
        let record = read_json(&path)?;
        if record["test_membership"].is_object() {
            records.push((path, record["test_membership"].clone()));
        }
    }
    Ok(records)
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).display().to_string()
}

/// `exposed_families`: the physical forms of everything the repository has shown a model before, and the forms as written.
pub(crate) fn exposed_families(root: &Path, additional: &[PathBuf]) -> Result<(HashSet<String>, BTreeMap<String, String>, HashSet<String>), String> {
    let mut paths = vec![root.join("model/context_v1/scenarios.json")];
    let mut holdouts: Vec<PathBuf> = std::fs::read_dir(root.join("model/context_v1")).map_err(|error| error.to_string())?.filter_map(|entry| entry.ok().map(|entry| entry.path())).filter(|path| path.file_name().is_some_and(|name| name.to_string_lossy().starts_with("holdout") && name.to_string_lossy().ends_with(".json"))).collect();
    holdouts.sort();
    paths.extend(holdouts);
    paths.push(root.join("model/context_v2/sources/lexical-evidence.json.gz"));
    paths.push(root.join("model/context_v2/sources/tatoeba-cc0-en-ru.tsv.gz"));
    paths.extend_from_slice(additional);
    let mut exposed = Exposed::default();
    let mut provenance = BTreeMap::new();
    #[derive(Default)]
    struct Exposed {
        physical: HashSet<String>,
        forms: HashSet<String>,
    }
    impl Exposed {
        fn insert(&mut self, form: &str) {
            self.physical.insert(physical(form));
            self.forms.insert(form.to_owned());
        }
    }
    fn add(text: &str, expand: bool, exposed: &mut Exposed) {
        exposed.insert(text);
        for token in words_of(text) {
            exposed.insert(token);
            if expand {
                let letters: Vec<char> = token.chars().collect();
                if (TYPO_SOURCE_MIN_CHARACTERS..=TYPO_SOURCE_MAX_CHARACTERS).contains(&letters.len()) {
                    for index in 1..letters.len() - 1 {
                        let mut deleted = letters.clone();
                        deleted.remove(index);
                        exposed.insert(&collected(&deleted));
                        let mut doubled = letters.clone();
                        doubled.insert(index, letters[index]);
                        exposed.insert(&collected(&doubled));
                    }
                    for index in 1..letters.len() - TRANSPOSE_TAIL_MARGIN_CHARACTERS {
                        let mut swapped = letters.clone();
                        swapped.swap(index, index + 1);
                        exposed.insert(&collected(&swapped));
                    }
                }
            }
        }
    }
    fn walk(value: &Value, expand: bool, exposed: &mut Exposed) {
        match value {
            Value::String(text) => add(text, expand, exposed),
            Value::Array(items) => items.iter().for_each(|child| walk(child, expand, exposed)),
            Value::Object(table) => {
                for (key, child) in table {
                    if key.starts_with('[') {
                        if let Ok(Value::Array(parts)) = serde_json::from_str::<Value>(key)
                            && let Some(text) = parts.get(1).and_then(Value::as_str) {
                                add(text, expand, exposed);
                            }
                    } else if matches!(key.as_str(), "word" | "original" | "alternative" | "text" | "before" | "after" | "expected") || child.is_array() || child.is_object() {
                        walk(child, expand, exposed);
                    }
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }
    let mut done: BTreeSet<PathBuf> = BTreeSet::new();
    for path in paths {
        if !done.insert(path.clone()) {
            continue;
        }
        if !path.is_file() {
            return Err(format!("exposure input is absent: {}", path.display()));
        }
        provenance.insert(relative(root, &path), checksum(&path)?);
        let expand = path.parent().and_then(Path::file_name).is_some_and(|name| name == "context_v1");
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let raw = std::fs::read(&path).map_err(|error| error.to_string())?;
        let bytes = if Path::new(&name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("gz")) {
            let mut unpacked = Vec::new();
            MultiGzDecoder::new(raw.as_slice()).read_to_end(&mut unpacked).map_err(|error| error.to_string())?;
            unpacked
        } else {
            raw
        };
        if name.ends_with(".tsv.gz") {
            for line in String::from_utf8(bytes).map_err(|error| error.to_string())?.lines() {
                let fields: Vec<&str> = line.split('\t').collect();
                if fields.len() != 4 {
                    return Err("invalid prior context TSV".to_owned());
                }
                add(fields[2], expand, &mut exposed);
            }
        } else {
            walk(&serde_json::from_slice(&bytes).map_err(|error| error.to_string())?, expand, &mut exposed);
        }
    }
    Ok((exposed.physical, provenance, exposed.forms))
}

fn update_aliases(set: &mut HashSet<String>, form: &str) {
    set.extend(expanded_aliases(form));
}

/// `load_exclusions`.
pub fn load_exclusions(root: &Path, base: &Path, ud_origin: &Path, technical_origin: &Path, prefix_inventory: &Path, lexicon: &Path, extra_exposure: &[PathBuf], ledger: &Path) -> Result<Exclusions, String> {
    let mut provenance = BTreeMap::new();
    let closure_path = ud_origin.join("physical-family-closure.json");
    let closure = read_json(&closure_path)?;
    let alias_map = closure["alias_sha256_to_closure_sha256"].as_object().ok_or("invalid UD physical alias sidecar")?;
    let retained: HashSet<String> = string_items(&closure["retained_test_closure_sha256"]).into_iter().collect();
    let mut base_aliases: HashSet<String> = alias_map.keys().cloned().collect();
    let mut prior_test_aliases: HashSet<String> = alias_map.iter().filter(|(_, member)| member.as_str().is_some_and(|member| retained.contains(member))).map(|(alias, _)| alias.clone()).collect();
    let families_path = technical_origin.join("family-aliases.json");
    let families = read_json(&families_path)?;
    for record in families["families"].as_object().ok_or("invalid technical family sidecar")?.values() {
        let aliases = string_items(&record["aliases_sha256"]);
        if record["split"] == json!("test") {
            prior_test_aliases.extend(aliases.iter().cloned());
        }
        base_aliases.extend(aliases);
    }
    let prefix_aliases: HashSet<String> = string_items(&read_json(prefix_inventory)?["aliases_sha256"]).into_iter().collect();
    let membership = read_json(&base.join("test-membership.json"))?;
    if read_json(&base.join("manifest.json"))?["test_membership_sha256"] != json!(checksum(&base.join("test-membership.json"))?) {
        return Err("base test membership differs from its manifest".to_owned());
    }
    let mut prior_test_families: HashSet<String> = string_items(&membership["family_ids_sha256"]).into_iter().collect();
    prior_test_aliases.extend(string_items(&membership["alias_sha256"]));
    for (path, record) in ledger_memberships(ledger)? {
        prior_test_families.extend(string_items(&record["family_ids_sha256"]));
        prior_test_aliases.extend(string_items(&record["alias_sha256"]));
        provenance.insert(path.display().to_string(), checksum(&path)?);
    }
    let (exposed, sources, _) = exposed_families(root, extra_exposure)?;
    let forms_path = root.join("model/context_v3/historical-code-forms.json");
    let mut historical: BTreeSet<String> = string_items(&read_json(&forms_path)?["forms"]).into_iter().collect();
    historical.extend(exposed.iter().cloned());
    let mut historical_aliases = HashSet::new();
    for form in &historical {
        update_aliases(&mut historical_aliases, form);
    }
    let lexicon_names: HashSet<String> = string_items(&read_json(lexicon)?["identifiers"]).into_iter().collect();
    provenance.extend(sources);
    for path in [closure_path, families_path, prefix_inventory.to_path_buf(), base.join("manifest.json"), base.join("test-membership.json"), lexicon.to_path_buf(), forms_path, ud_origin.join("manifest.json"), technical_origin.join("manifest.json")] {
        provenance.insert(relative(root, &path), checksum(&path)?);
    }
    Ok(Exclusions { base_aliases, prior_test_aliases, prefix_aliases, prior_test_families, exposed_physical: exposed, historical_aliases, lexicon: lexicon_names, provenance })
}

fn meets(hashes: &BTreeSet<String>, set: &HashSet<String>) -> bool {
    hashes.iter().any(|hash| set.contains(hash))
}

/// `alias_reasons`.
fn alias_reasons(hashes: &BTreeSet<String>, forms: &BTreeSet<String>, family: &str, exclusions: &Exclusions) -> Vec<String> {
    let mut reasons = Vec::new();
    if meets(hashes, &exclusions.prior_test_aliases) || exclusions.prior_test_families.contains(family) {
        reasons.push("prior-accessed-test-family".to_owned());
    }
    if meets(hashes, &exclusions.base_aliases) {
        reasons.push("family-in-base-corpus".to_owned());
    }
    if meets(hashes, &exclusions.prefix_aliases) {
        reasons.push("prefix-curriculum-prior-exposure".to_owned());
    }
    if meets(forms, &exclusions.exposed_physical) || meets(hashes, &exclusions.historical_aliases) {
        reasons.push("family-previously-exposed".to_owned());
    }
    reasons
}

struct Families {
    physical_by_family: HashMap<String, BTreeSet<String>>,
    hashes_by_family: BTreeMap<String, BTreeSet<String>>,
}

/// The rows of sentences with their families joined, and every alias each family holds.
fn family_rows(sentences: &[Sentence]) -> Result<(Vec<CorpusRow>, Families), String> {
    let mut families = Union::default();
    for sentence in sentences {
        for token in &sentence.tokens {
            let aliases = family_aliases(token, &row_identifier(sentence, token));
            for alias in &aliases {
                families.join(&aliases[0], alias);
            }
        }
    }
    let rows: Vec<CorpusRow> = sentences.iter().flat_map(|sentence| sentence_rows(sentence, &mut families)).collect();
    let distinct: HashSet<&str> = rows.iter().map(|row| row.identifier.as_str()).collect();
    if distinct.len() != rows.len() {
        return Err("duplicate row identifiers".to_owned());
    }
    let mut out = Families { physical_by_family: HashMap::new(), hashes_by_family: BTreeMap::new() };
    for sentence in sentences {
        for token in &sentence.tokens {
            let identifier = row_identifier(sentence, token);
            let family = hex_digest(&families.find(&physical(&token.form)));
            let mut values: Vec<String> = vec![token.form.clone()];
            values.extend(token.lemmas.iter().cloned());
            values.extend(crate::corpus::typo_variants(&token.form, &identifier));
            let forms = out.physical_by_family.entry(family.clone()).or_default();
            for value in &values {
                forms.insert(physical(value));
                forms.extend(words_of(value).into_iter().map(physical));
            }
            let hashes = out.hashes_by_family.entry(family).or_default();
            for value in &values {
                hashes.extend(expanded_aliases(value));
            }
        }
    }
    Ok((rows, out))
}

fn count_into(counts: &mut BTreeMap<String, i64>, reasons: &[String]) {
    for reason in reasons {
        *counts.entry(reason.clone()).or_insert(0) += 1;
    }
}

/// `ud_holdout_rows`.
fn holdout_rows(sentences: &[Sentence], exclusions: &Exclusions) -> Result<(Vec<CorpusRow>, Value), String> {
    let (rows, families) = family_rows(sentences)?;
    let mut reasons_by_family: HashMap<&str, Vec<String>> = HashMap::new();
    for (family, hashes) in &families.hashes_by_family {
        reasons_by_family.insert(family, alias_reasons(hashes, &families.physical_by_family[family], family, exclusions));
    }
    let mut result = Vec::with_capacity(rows.len());
    let mut reasons_count = BTreeMap::new();
    for mut row in rows {
        let mut reasons = reasons_by_family.get(row.family.as_str()).cloned().unwrap_or_default();
        if row.alignment != "exact-text-comment" {
            reasons.push(row.alignment.clone());
        }
        if reasons.is_empty() { "test" } else { "quarantine" }.clone_into(&mut row.split);
        count_into(&mut reasons_count, &reasons);
        row.quarantine_reasons = reasons;
        result.push(row);
    }
    result.sort_by(|a, b| a.identifier.cmp(&b.identifier));
    let held = |row: &&CorpusRow| row.split == "test";
    let test_families: BTreeSet<&str> = result.iter().filter(held).map(|row| row.family.as_str()).collect();
    let test_documents: BTreeSet<&str> = result.iter().filter(held).map(|row| row.document.as_str()).collect();
    let report = json!({"rows": result.len(), "families": families.hashes_by_family.len(), "test_rows": result.iter().filter(held).count(), "test_families": test_families.len(), "test_documents": test_documents.len(), "quarantine_reasons": reasons_count});
    Ok((result, report))
}

#[derive(Clone)]
pub struct Command {
    pub name: String,
    pub paths: Vec<String>,
    pub owners: Vec<String>,
}

fn commands_from(paths: BTreeMap<String, BTreeSet<String>>) -> Vec<Command> {
    paths.into_iter().map(|(name, owners)| Command { name, paths: Vec::new(), owners: owners.into_iter().collect() }).collect()
}

/// `COMMAND_PATH`: `(?:usr/)?s?bin/([a-z]{3,16})`, full match.
fn command_of_path(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("usr/").unwrap_or(path);
    let rest = rest.strip_prefix('s').unwrap_or(rest);
    let name = rest.strip_prefix("bin/")?;
    command_name(name).then_some(name)
}

/// `[a-z]{3,16}`.
fn command_name(name: &str) -> bool {
    (COMMAND_NAME_MIN_CHARACTERS..=COMMAND_NAME_MAX_CHARACTERS).contains(&name.len()) && name.bytes().all(|byte| byte.is_ascii_lowercase())
}

fn package_section(item: &str) -> bool {
    // `(?:[a-z0-9+.-]+/){1,2}[a-z0-9][a-z0-9+.-]*`.
    let parts: Vec<&str> = item.split('/').collect();
    let plain = |text: &str| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'+' | b'.' | b'-'));
    (parts.len() == 2 || parts.len() == 3) && parts.iter().all(|part| plain(part)) && parts[parts.len() - 1].bytes().next().is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
}

/// `read_commands`: the executables a Contents index lists.
pub(crate) fn read_commands(contents: &Path) -> Result<Vec<Command>, String> {
    let file = std::fs::File::open(contents).map_err(|error| error.to_string())?;
    let mut owners: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut paths: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for line in std::io::BufReader::new(MultiGzDecoder::new(file)).lines() {
        let line = line.map_err(|error| error.to_string())?;
        let trimmed = pystr::rstrip(&line);
        let Some(place) = trimmed.rfind(char::is_whitespace) else { continue };
        let (path, packages) = (pystr::rstrip(trimmed.get(..place).unwrap_or_default()), trimmed.get(place..).unwrap_or_default().trim_start());
        let Some(name) = command_of_path(path) else { continue };
        let items: Vec<&str> = packages.split(',').collect();
        if !items.iter().all(|item| package_section(item)) {
            return Err("invalid package ownership in selected Contents row".to_owned());
        }
        paths.entry(name.to_owned()).or_default().insert(path.to_owned());
        owners.entry(name.to_owned()).or_default().extend(items.iter().map(|item| (*item).to_owned()));
    }
    let mut commands = commands_from(owners);
    for command in &mut commands {
        command.paths = paths.remove(&command.name).unwrap_or_default().into_iter().collect();
    }
    Ok(commands)
}

/// The entries of a tar archive: (name, bytes).
fn tar_entries(bytes: &[u8]) -> Vec<(String, &[u8])> {
    let mut entries = Vec::new();
    let mut position = 0;
    while position + 512 <= bytes.len() {
        let header = &bytes[position..position + 512];
        if header.iter().all(|byte| *byte == 0) {
            break;
        }
        let text = |range: std::ops::Range<usize>| -> String { String::from_utf8_lossy(&header[range]).trim_end_matches('\0').to_owned() };
        let name = format!("{}{}", { let prefix = text(345..500); if prefix.is_empty() || !header[257..262].eq(b"ustar") { String::new() } else { format!("{prefix}/") } }, text(0..100));
        let size = usize::from_str_radix(text(124..136).trim(), 8).unwrap_or(0);
        let kind = header[156];
        position += 512;
        if kind == 0 || kind == b'0' {
            entries.push((name, &bytes[position..(position + size).min(bytes.len())]));
        }
        position += size.div_ceil(512) * 512;
    }
    entries
}

fn gunzip(path: &Path) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    MultiGzDecoder::new(file).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    Ok(bytes)
}

/// `read_arch_commands`: `usr/bin` entries of every package of pacman `.files` databases.
fn read_arch_commands(databases: &[(String, PathBuf)]) -> Result<Vec<Command>, String> {
    let mut owners: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (repository, database) in databases {
        let bytes = gunzip(database)?;
        let entries: HashMap<String, &[u8]> = tar_entries(&bytes).into_iter().collect();
        let mut names: Vec<&String> = entries.keys().filter(|name| name.ends_with("/files")).collect();
        names.sort();
        for name in names {
            let directory = name.rsplit_once('/').map_or("", |pair| pair.0);
            let description = entries.get(&format!("{directory}/desc")).ok_or_else(|| format!("package files without a description: {directory}"))?;
            let description = String::from_utf8_lossy(description);
            let lines: Vec<&str> = description.lines().collect();
            let package = lines.iter().position(|line| *line == "%NAME%").and_then(|place| lines.get(place + 1)).ok_or_else(|| format!("package description without a name: {directory}"))?;
            for line in String::from_utf8_lossy(entries[name]).lines() {
                if let Some(command) = command_of_path(line) {
                    owners.entry(command.to_owned()).or_default().insert(format!("{repository}/{package}"));
                }
            }
        }
    }
    Ok(commands_from(owners))
}

/// `read_alpine_commands`: every package lists what it provides as `cmd:name`.
fn read_alpine_commands(archive: &Path) -> Result<Vec<Command>, String> {
    let bytes = gunzip(archive)?;
    let entries: HashMap<String, &[u8]> = tar_entries(&bytes).into_iter().collect();
    let index = entries.get("APKINDEX").ok_or("Alpine index archive has no APKINDEX member")?;
    let mut owners: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut package = String::new();
    for line in String::from_utf8_lossy(index).lines() {
        if let Some(name) = line.strip_prefix("P:") {
            pystr::strip(name).clone_into(&mut package);
        } else if let (Some(provided), false) = (line.strip_prefix("p:"), package.is_empty()) {
            for token in pystr::split(provided) {
                let Some(name) = token.strip_prefix("cmd:") else { continue };
                let name = name.split('=').next().unwrap_or("");
                if command_name(name) {
                    owners.entry(name.to_owned()).or_default().insert(format!("alpine/{package}"));
                }
            }
        }
    }
    Ok(commands_from(owners))
}

/// The text between each `<tag ...>` and `</tag>` at the positions the scan reaches them.
fn xml_text(raw: &str) -> String {
    raw.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// `read_fedora_commands`: executables of a `primary.xml` index, whose filter keeps `bin` paths.
fn read_fedora_commands(primary: &Path) -> Result<Vec<Command>, String> {
    let file = std::fs::File::open(primary).map_err(|error| error.to_string())?;
    let mut decoder = ruzstd::decoding::StreamingDecoder::new(std::io::BufReader::new(file)).map_err(|error| error.to_string())?;
    let mut text = String::new();
    decoder.read_to_string(&mut text).map_err(|error| error.to_string())?;
    let mut owners: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for block in text.split("<package ").skip(1) {
        let block = block.split("</package>").next().unwrap_or("");
        let name = block.find("<name>").and_then(|start| block.get(start + 6..).unwrap_or_default().find("</name>").map(|end| pystr::strip(block.get(start + 6..start + 6 + end).unwrap_or_default()).to_owned())).unwrap_or_default();
        let mut rest = block;
        while let Some(start) = rest.find("<file") {
            let after = rest.get(start..).unwrap_or_default();
            let Some(open) = after.find('>') else { break };
            let Some(end) = after.find("</file>") else { break };
            if open < end {
                let path = xml_text(pystr::strip(after.get(open + 1..end).unwrap_or_default()));
                if let Some(command) = command_of_path(path.trim_start_matches('/')) {
                    owners.entry(command.to_owned()).or_default().insert(format!("fedora/{name}"));
                }
            }
            rest = after.get(end + 7..).unwrap_or_default();
        }
    }
    Ok(commands_from(owners))
}

/// `technical_holdout_rows`: commands absent from the shipped lexicon, as a test-only technical holdout.
fn technical_holdout_rows(commands: &[Command], exclusions: &Exclusions, namespace: &str, source: &str, source_file: &str, document_prefix: &str) -> (Vec<CorpusRow>, Value) {
    let identifier_of = |name: &str| format!("{}:command:{name}", source.to_lowercase());
    let mut records: Vec<&Command> = commands.iter().filter(|item| !exclusions.lexicon.contains(&item.name)).collect();
    records.sort_by(|a, b| a.name.cmp(&b.name));
    let mut family_union = Union::default();
    // `command_identifier` of the technical corpus names the typo variants of a command, whatever its source.
    let aliases: HashMap<&str, BTreeSet<String>> = records
        .iter()
        .map(|item| {
            let mut set = expanded_aliases(&item.name);
            for variant in crate::corpus::typo_variants(&item.name, &format!("debian-trixie-main-amd64:command:{}", item.name)) {
                set.extend(expanded_aliases(&variant));
            }
            (item.name.as_str(), set)
        })
        .collect();
    for item in &records {
        let anchor = format!("command:{}", item.name);
        family_union.find(&anchor);
        for alias in &aliases[item.name.as_str()] {
            family_union.join(&anchor, &format!("alias:{alias}"));
        }
    }
    let families: HashMap<&str, String> = records.iter().map(|item| (item.name.as_str(), hex_digest(&family_union.find(&format!("command:{}", item.name))))).collect();
    let mut package_union = Union::default();
    for item in &records {
        let anchor = format!("family:{}", families[item.name.as_str()]);
        for owner in &item.owners {
            package_union.join(&anchor, &format!("package:{}", owner.rsplit('/').next().unwrap_or(owner)));
        }
    }
    let documents: HashMap<&str, String> = records.iter().map(|item| (item.name.as_str(), format!("{document_prefix}{}", hex_digest(&package_union.find(&format!("family:{}", families[item.name.as_str()])))))).collect();
    let mut members: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for item in &records {
        members.entry(&families[item.name.as_str()]).or_default().push(&item.name);
    }
    let mut retained: HashSet<&str> = HashSet::new();
    for names in members.values() {
        let mut ranked = names.clone();
        ranked.sort_by_cached_key(|name| hex_digest(&format!("{namespace}:family-cap:{}", identifier_of(name))));
        retained.extend(ranked.into_iter().take(COMMANDS_PER_ALIAS_FAMILY));
    }
    let mut family_hashes: HashMap<&str, BTreeSet<String>> = HashMap::new();
    for item in &records {
        family_hashes.entry(&families[item.name.as_str()]).or_default().extend(aliases[item.name.as_str()].iter().cloned());
    }
    let mut rows = Vec::new();
    let mut reasons_count = BTreeMap::new();
    for item in &records {
        let family = &families[item.name.as_str()];
        let forms: BTreeSet<String> = std::iter::once(physical(&item.name)).collect();
        let mut reasons = alias_reasons(&family_hashes[family.as_str()], &forms, family, exclusions);
        if !retained.contains(item.name.as_str()) {
            reasons.push("fixed-hash-family-cap".to_owned());
        }
        count_into(&mut reasons_count, &reasons);
        rows.push(CorpusRow {
            identifier: identifier_of(&item.name),
            original: item.name.clone(),
            group: Some(0),
            before: String::new(),
            after: String::new(),
            lemma: item.name.clone(),
            family: family.clone(),
            document: documents[item.name.as_str()].clone(),
            language: "en".to_owned(),
            source: source.to_owned(),
            source_file: source_file.to_owned(),
            source_sentence: identifier_of(&item.name),
            source_token: "1".to_owned(),
            spacing: " ".to_owned(),
            space_before: String::new(),
            literal_tail: String::new(),
            upos: "X".to_owned(),
            features: "_".to_owned(),
            misc: "_".to_owned(),
            alignment: "declared-command-name".to_owned(),
            layout_representable: true,
            split: if reasons.is_empty() { "test" } else { "quarantine" }.to_owned(),
            quarantine_reasons: reasons,
        });
    }
    let held = |row: &&CorpusRow| row.split == "test";
    let test_families: BTreeSet<&str> = rows.iter().filter(held).map(|row| row.family.as_str()).collect();
    let test_documents: BTreeSet<&str> = rows.iter().filter(held).map(|row| row.document.as_str()).collect();
    let report = json!({"commands_in_index": commands.len(), "commands_outside_lexicon": records.len(), "families": members.len(), "test_rows": rows.iter().filter(held).count(),
        "test_families": test_families.len(), "test_documents": test_documents.len(), "family_cap": COMMANDS_PER_ALIAS_FAMILY, "quarantine_reasons": reasons_count});
    (rows, report)
}

/// What a technical source gives: its commands, and the labels and provenance of its rows.
struct Technical {
    commands: Vec<Command>,
    source: String,
    source_file: &'static str,
    document_prefix: String,
    provenance: BTreeMap<String, String>,
    metadata: Value,
}

/// `verified_sid_source`: a Contents index named by a TLS receipt, its checksum in the downloaded `InRelease`.
fn sid_source(directory: &Path) -> Result<Technical, String> {
    let receipt_path = directory.join("source-receipt.json");
    let receipt = read_json(&receipt_path)?;
    let (contents, release) = (directory.join("Contents-amd64.gz"), directory.join("InRelease"));
    let url = receipt["contents"]["url"].as_str().unwrap_or("");
    let known = DEB_SOURCES.iter().find(|entry| entry.0 == url).filter(|entry| receipt["release"]["url"] == json!(entry.1));
    let Some((contents_url, _, listed, source, prefix, description)) = known.copied().filter(|_| receipt["contents"]["status"] == json!(200) && receipt["release"]["status"] == json!(200) && receipt["tls_certificate_verification"] == json!(true)) else {
        return Err("source receipt does not identify verified TLS downloads of a known Contents index".to_owned());
    };
    let size = std::fs::metadata(&contents).map_err(|error| error.to_string())?.len();
    if receipt["contents_sha256"] != json!(checksum(&contents)?) || receipt["contents_bytes"] != json!(size) || receipt["release_sha256"] != json!(checksum(&release)?) {
        return Err("Contents index checksum mismatch".to_owned());
    }
    let expected = [receipt["contents_sha256"].as_str().unwrap_or("").to_owned(), receipt["contents_bytes"].to_string(), listed.to_owned()];
    let (mut section, mut found) = (false, false);
    for line in std::fs::read_to_string(&release).map_err(|error| error.to_string())?.lines() {
        if line == "SHA256:" {
            section = true;
        } else if section && !line.is_empty() && !line.starts_with(' ') {
            section = false;
        } else if section && pystr::split(line) == expected.iter().map(String::as_str).collect::<Vec<_>>() {
            found = true;
        }
    }
    if !found {
        return Err("Contents checksum absent from InRelease SHA256 section".to_owned());
    }
    let signed = receipt["signature"]["verified"] == json!(true);
    let mut provenance = BTreeMap::new();
    for path in [&receipt_path, &contents, &release] {
        provenance.insert(path.display().to_string(), checksum(path)?);
    }
    let verification = format!("HTTPS TLS and Contents SHA256 against downloaded InRelease; {}", if signed { "InRelease OpenPGP signature verified by the fetcher against the local archive keyring" } else { "OpenPGP signature not verified by this generator" });
    Ok(Technical { commands: read_commands(&contents)?, source: source.to_owned(), source_file: "Contents-amd64.gz", document_prefix: prefix.to_owned(), provenance, metadata: json!({"source": description, "contents_url": contents_url, "receipt": receipt, "verification": verification}) })
}

fn arch_source(directory: &Path) -> Result<Technical, String> {
    let receipt_path = directory.join("source-receipt.json");
    let receipt = read_json(&receipt_path)?;
    if receipt["tls_certificate_verification"] != json!(true) || receipt["mirror"] != json!(ARCH_MIRROR) {
        return Err("source receipt does not identify verified TLS Arch Linux downloads".to_owned());
    }
    let mut paths = Vec::new();
    let mut provenance = BTreeMap::new();
    provenance.insert(receipt_path.display().to_string(), checksum(&receipt_path)?);
    for repository in ARCH_REPOSITORIES {
        let entry = &receipt["databases"][repository];
        let path = directory.join(format!("{repository}.files"));
        let size = std::fs::metadata(&path).map_err(|error| error.to_string())?.len();
        if entry["download"]["status"] != json!(200) || entry["download"]["url"] != json!(format!("{ARCH_MIRROR}{repository}/os/x86_64/{repository}.files")) || entry["file"] != json!(format!("{repository}.files")) {
            return Err(format!("source receipt does not identify the {repository} files database download"));
        }
        if entry["sha256"] != json!(checksum(&path)?) || entry["bytes"] != json!(size) {
            return Err(format!("Arch {repository} files database checksum mismatch"));
        }
        provenance.insert(path.display().to_string(), checksum(&path)?);
        paths.push((repository.to_owned(), path));
    }
    Ok(Technical { commands: read_arch_commands(&paths)?, source: "Arch-x86_64".to_owned(), source_file: "core.files+extra.files", document_prefix: "arch-package-component:".to_owned(), provenance,
        metadata: json!({"source": "Arch Linux core and extra x86_64 package file databases", "receipt": receipt, "verification": "HTTPS TLS and SHA-256 of the downloaded databases; the mirror publishes no detached signature for .files"}) })
}

fn alpine_source(directory: &Path) -> Result<Technical, String> {
    let receipt_path = directory.join("source-receipt.json");
    let receipt = read_json(&receipt_path)?;
    let archive = directory.join("APKINDEX.tar.gz");
    let base_url = receipt["base_url"].as_str().unwrap_or("");
    let Some((_, source, prefix, description)) = ALPINE_SOURCES.iter().find(|entry| entry.0 == base_url).copied().filter(|_| receipt["archive"]["status"] == json!(200) && receipt["tls_certificate_verification"] == json!(true) && receipt["archive"]["url"] == json!(format!("{base_url}APKINDEX.tar.gz"))) else {
        return Err("source receipt does not identify a verified TLS Alpine download".to_owned());
    };
    let size = std::fs::metadata(&archive).map_err(|error| error.to_string())?.len();
    if receipt["archive"]["sha256"] != json!(checksum(&archive)?) || receipt["archive"]["bytes"] != json!(size) {
        return Err("Alpine index checksum mismatch".to_owned());
    }
    let mut provenance = BTreeMap::new();
    for path in [&receipt_path, &archive] {
        provenance.insert(path.display().to_string(), checksum(path)?);
    }
    Ok(Technical { commands: read_alpine_commands(&archive)?, source: source.to_owned(), source_file: "APKINDEX.tar.gz", document_prefix: prefix.to_owned(), provenance,
        metadata: json!({"source": description, "base_url": base_url, "receipt": receipt, "verification": "HTTPS TLS and the archive SHA-256 from the receipt; the detached RSA signature was not verified by this generator"}) })
}

fn rpm_source(directory: &Path) -> Result<Technical, String> {
    let receipt_path = directory.join("source-receipt.json");
    let receipt = read_json(&receipt_path)?;
    let (repomd, primary) = (directory.join("repomd.xml"), directory.join("primary.xml.zst"));
    let base_url = receipt["base_url"].as_str().unwrap_or("");
    let Some((_, source, prefix, description)) = RPM_SOURCES.iter().find(|entry| entry.0 == base_url).copied().filter(|_| receipt["tls_certificate_verification"] == json!(true) && receipt["repomd"]["status"] == json!(200)) else {
        return Err("source receipt does not identify verified TLS RPM index downloads".to_owned());
    };
    if receipt["repomd_sha256"] != json!(checksum(&repomd)?) || receipt["primary"]["sha256"] != json!(checksum(&primary)?) {
        return Err("RPM index checksum mismatch".to_owned());
    }
    let algorithm = receipt["primary"].get("checksum_type").and_then(Value::as_str).unwrap_or("sha256");
    let declared = receipt["primary"].get("checksum").or_else(|| receipt["primary"].get("sha256")).and_then(Value::as_str).ok_or("RPM index receipt names an unknown checksum")?;
    let payload = std::fs::read(&primary).map_err(|error| error.to_string())?;
    let actual = match algorithm {
        "sha256" => hex(&Sha256::digest(&payload)),
        "sha512" => hex(&Sha512::digest(&payload)),
        _ => return Err("RPM index receipt names an unknown checksum".to_owned()),
    };
    if actual != declared {
        return Err("RPM index payload does not match the checksum the index names".to_owned());
    }
    let href = receipt["primary"].get("href").and_then(Value::as_str).unwrap_or("");
    let text = std::fs::read_to_string(&repomd).map_err(|error| error.to_string())?;
    if !text.contains(&format!("href=\"{href}\"")) || !text.contains(&format!(">{declared}<")) {
        return Err("primary metadata is not the one repomd.xml names".to_owned());
    }
    let mut provenance = BTreeMap::new();
    for path in [&receipt_path, &repomd, &primary] {
        provenance.insert(path.display().to_string(), checksum(path)?);
    }
    Ok(Technical { commands: read_fedora_commands(&primary)?, source: source.to_owned(), source_file: "primary.xml.zst", document_prefix: prefix.to_owned(), provenance,
        metadata: json!({"source": description, "base_url": base_url, "receipt": receipt, "verification": format!("HTTPS TLS and the primary {algorithm} from repomd.xml; OpenPGP signature not verified by this generator")}) })
}

/// `holdout_inventory`: pinned treebank files of the new sources, verified by SHA-256 and git blob hash.
pub(crate) fn source_inventory(source_root: &Path, pins: &[(&str, &str)]) -> Result<(Vec<(String, PathBuf)>, Vec<Value>), String> {
    let mut paths = Vec::new();
    let mut metadata = Vec::new();
    for (source, commit) in pins {
        let directory = source_root.join(source);
        let mut pin = read_json(&directory.join("pin.json"))?;
        if pin["commit"] != json!(commit) || pin["repository"] != json!(format!("https://github.com/UniversalDependencies/{source}")) {
            return Err(format!("holdout source repository or pinned commit differs: {source}"));
        }
        for item in pin["files"].as_array().filter(|items| !items.is_empty()).ok_or_else(|| format!("invalid pinned holdout inventory: {source}"))? {
            let name = item["path"].as_str().filter(|name| name.ends_with(".conllu") || Path::new(name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md")) || Path::new(name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("txt"))).ok_or_else(|| format!("invalid pinned holdout filename: {source}"))?;
            let path = directory.join(Path::new(name).file_name().ok_or("a pinned file without a name")?);
            let content = std::fs::read(&path).map_err(|error| error.to_string())?;
            if item["sha256"] != json!(hex(&Sha256::digest(&content))) {
                return Err(format!("pinned holdout checksum differs: {source}/{name}"));
            }
            let mut blob = Sha1::new();
            blob.update(format!("blob {}\0", content.len()).as_bytes());
            blob.update(&content);
            if item["sha"] != json!(hex(&blob.finalize())) {
                return Err(format!("holdout content disagrees with pinned Git tree blob: {name}"));
            }
            if name.ends_with(".conllu") {
                paths.push(((*source).to_owned(), path));
            }
        }
        pin["pin_metadata_sha256"] = json!(checksum(&directory.join("pin.json"))?);
        metadata.push(pin);
    }
    paths.sort_by(|a, b| (&a.0, a.1.file_name()).cmp(&(&b.0, b.1.file_name())));
    Ok((paths, metadata))
}

/// `row_aliases`: every physical alias of a row's word: its form, its lemmas and its typo variants.
fn row_aliases(row: &CorpusRow) -> BTreeSet<String> {
    let mut aliases = BTreeSet::new();
    let mut values: Vec<String> = vec![row.original.clone()];
    values.extend(row.lemma.split(' ').map(str::to_owned));
    values.extend(crate::corpus::typo_variants(&row.original, &row.identifier));
    for value in values {
        if !value.is_empty() {
            aliases.extend(expanded_aliases(&value));
        }
    }
    aliases
}

fn membership_of(rows: &[&CorpusRow]) -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>) {
    let mut row_ids: Vec<String> = rows.iter().map(|row| hex_digest(&row.identifier)).collect();
    row_ids.sort();
    let families: BTreeSet<&str> = rows.iter().map(|row| row.family.as_str()).collect();
    let documents: BTreeSet<String> = rows.iter().map(|row| hex_digest(&row.document)).collect();
    let mut aliases: BTreeSet<String> = BTreeSet::new();
    for row in rows {
        aliases.extend(row_aliases(row));
    }
    (row_ids, families.iter().map(|family| (*family).to_owned()).collect(), documents.into_iter().collect(), aliases.into_iter().collect())
}

fn gzip_member(rows: &[&CorpusRow]) -> Result<Vec<u8>, String> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    for row in rows {
        encoder.write_all(&row.line()).map_err(|error| error.to_string())?;
    }
    encoder.finish().map_err(|error| error.to_string())
}

/// The raw content's hash and row count of a split file, for its record.
fn split_record(path: &Path, expected: u64) -> Result<Value, String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut reader = std::io::BufReader::new(MultiGzDecoder::new(file));
    let mut hasher = Sha256::new();
    let mut count = 0_u64;
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line).map_err(|error| error.to_string())? == 0 {
            break;
        }
        hasher.update(&line);
        count += 1;
    }
    if count != expected {
        return Err(format!("assembled split row count mismatch: {} ({count} against {expected})", path.display()));
    }
    Ok(json!({"path": path.file_name().map(|name| name.to_string_lossy().into_owned()), "sha256": checksum(path)?, "content_sha256": hex(&hasher.finalize()), "rows": count}))
}

fn begin_output(base: &Path, output: &Path, generator: &str) -> Result<Value, String> {
    if output.exists() {
        return Err("refusing to overwrite a corpus directory".to_owned());
    }
    let resolved_base = base.canonicalize().map_err(|error| error.to_string())?;
    let parent = output.parent().map(Path::to_path_buf).unwrap_or_default();
    std::fs::create_dir_all(&parent).map_err(|error| error.to_string())?;
    let resolved = parent.canonicalize().map_err(|error| error.to_string())?.join(output.file_name().ok_or("an output without a name")?);
    if resolved == resolved_base || resolved.starts_with(&resolved_base) {
        return Err("derivative must not be inside the base corpus".to_owned());
    }
    let manifest = read_json(&base.join("manifest.json"))?;
    std::fs::create_dir_all(output.join("origins")).map_err(|error| error.to_string())?;
    std::fs::write(output.join("generator-source.rs"), generator).map_err(|error| error.to_string())?;
    std::fs::copy(base.join("manifest.json"), output.join("origins/base-manifest.json")).map_err(|error| error.to_string())?;
    std::fs::copy(base.join("test-membership.json"), output.join("origins/base-test-membership.json")).map_err(|error| error.to_string())?;
    Ok(manifest)
}

/// The arguments both freezes take.
pub struct FreezeRequest<'a> {
    pub root: &'a Path,
    pub base: &'a Path,
    pub ud_origin: &'a Path,
    pub technical_origin: &'a Path,
    pub prefix_inventory: &'a Path,
    pub lexicon: &'a Path,
    pub ledger: &'a Path,
    pub namespace: &'a str,
    pub output: Option<&'a Path>,
    pub report: Option<&'a Path>,
    pub extra_exposure: Vec<PathBuf>,
    pub tatoeba_directory: Option<&'a Path>,
    pub tatoeba_languages: Vec<String>,
    pub max_documents: usize,
    pub max_sentences_per_document: usize,
}

/// The technical source of a holdout: exactly one.
pub enum TechnicalSource<'a> {
    Sid(&'a Path),
    Arch(&'a Path),
    Rpm(&'a Path),
    Alpine(&'a Path),
}

/// `freeze_context_action_holdout.main`: new treebanks and commands become the TEST of a derivative corpus whose fitting splits are the base's, byte for byte.
pub fn freeze_holdout(request: &FreezeRequest<'_>, source_root: &Path, treebanks: &[String], technical: &TechnicalSource<'_>) -> Result<Value, String> {
    let pins: Vec<(&str, &str)> = if treebanks == ["none"] {
        Vec::new()
    } else if treebanks.is_empty() {
        PINS.to_vec()
    } else {
        let mut chosen = Vec::new();
        for name in treebanks {
            chosen.push(*PINS.iter().find(|pin| pin.0 == name).ok_or("unpinned treebank requested")?);
        }
        chosen
    };
    let (paths, sources) = if pins.is_empty() { (Vec::new(), Vec::new()) } else { source_inventory(source_root, &pins)? };
    let exclusions = load_exclusions(request.root, request.base, request.ud_origin, request.technical_origin, request.prefix_inventory, request.lexicon, &request.extra_exposure, request.ledger)?;
    let mut sentences = Vec::new();
    for (source, path) in &paths {
        sentences.extend(read_conllu(path, source)?);
    }
    sentence_documents(&mut sentences);
    let mut tatoeba_provenance = BTreeMap::new();
    let mut tatoeba_receipt = None;
    if let Some(directory) = request.tatoeba_directory {
        if request.tatoeba_languages.is_empty() {
            return Err("--tatoeba-directory needs at least one --tatoeba-language".to_owned());
        }
        let (exports, provenance, _metadata) = verified_tatoeba_source(directory, &request.tatoeba_languages)?;
        let captured = captured_tatoeba_identifiers(request.root)?;
        tatoeba_provenance = provenance;
        tatoeba_provenance.insert(CAPTURED_TATOEBA.to_owned(), checksum(&request.root.join(CAPTURED_TATOEBA))?);
        for (label, language, path) in exports {
            sentences.extend(read_tatoeba(&path, &label, &language, request.namespace, &captured)?);
        }
        tatoeba_receipt = Some(tatoeba_provenance[&directory.join("source-receipt.json").display().to_string()].clone());
    } else if !request.tatoeba_languages.is_empty() {
        return Err("--tatoeba-language needs --tatoeba-directory".to_owned());
    }
    let total = sentences.len();
    let typed: Vec<Sentence> = sentences.into_iter().filter(|sentence| !lookalike_typo(sentence)).collect();
    let dropped = total - typed.len();
    let (selected, mut sampling) = select_sentences(typed, request.namespace, request.max_documents, request.max_sentences_per_document, true)?;
    sampling["lookalike_typo_sentences_dropped"] = json!(dropped);
    let (mut ud_rows, ud_summary) = holdout_rows(&selected, &exclusions)?;
    drop(selected);
    let source = match technical {
        TechnicalSource::Sid(directory) => sid_source(directory)?,
        TechnicalSource::Arch(directory) => arch_source(directory)?,
        TechnicalSource::Rpm(directory) => rpm_source(directory)?,
        TechnicalSource::Alpine(directory) => alpine_source(directory)?,
    };
    let (mut technical_rows, technical_summary) = technical_holdout_rows(&source.commands, &exclusions, request.namespace, &source.source, source.source_file, &source.document_prefix);
    let mut provenance = exclusions.provenance.clone();
    provenance.extend(source.provenance.clone());
    provenance.extend(tatoeba_provenance);
    if let Some(receipt) = tatoeba_receipt {
        sampling["tatoeba"] = json!({"languages": request.tatoeba_languages, "per_mille": TATOEBA_SAMPLE_PER_MILLE, "receipt_sha256": receipt});
    }
    for pin in &sources {
        provenance.insert(format!("holdout-source:{}", pin["repository"].as_str().unwrap_or("")), pin["pin_metadata_sha256"].as_str().unwrap_or("").to_owned());
    }
    let ledger = ledger_memberships(request.ledger)?;
    let prior_rows: HashSet<String> = ledger.iter().flat_map(|(_, record)| string_items(&record["row_ids_sha256"])).collect();
    let prior_documents: HashSet<String> = ledger.iter().flat_map(|(_, record)| string_items(&record["document_ids_sha256"])).collect();
    let mut repeated = 0;
    for row in ud_rows.iter_mut().chain(technical_rows.iter_mut()) {
        if row.split == "test" && (prior_rows.contains(&hex_digest(&row.identifier)) || prior_documents.contains(&hex_digest(&row.document))) {
            "quarantine".clone_into(&mut row.split);
            row.quarantine_reasons.push("prior-accessed-test-row".to_owned());
            repeated += 1;
        }
    }
    let held: Vec<&CorpusRow> = ud_rows.iter().chain(technical_rows.iter()).filter(|row| row.split == "test").collect();
    let (row_ids, family_ids, document_ids, aliases) = membership_of(&held);
    let mut overlap: BTreeMap<&str, usize> = BTreeMap::new();
    for (name, mine) in [("row_ids_sha256", &row_ids), ("family_ids_sha256", &family_ids), ("document_ids_sha256", &document_ids)] {
        let mine: HashSet<&String> = mine.iter().collect();
        let mut total = 0;
        for (_, record) in &ledger {
            total += string_items(&record[name]).iter().filter(|entry| mine.contains(entry)).count();
        }
        if !ledger.is_empty() {
            overlap.insert(name, total);
        }
    }
    let metadata = json!({"sources": sources, "sampling": sampling, "ud": ud_summary, "technical": technical_summary, "sid_source": source.metadata, "repeated_rows_quarantined": repeated, "prior_access_overlap": overlap,
        "exclusion_counts": {"base_aliases": exclusions.base_aliases.len(), "prior_test_aliases": exclusions.prior_test_aliases.len(), "prefix_aliases": exclusions.prefix_aliases.len(),
            "prior_test_families": exclusions.prior_test_families.len(), "exposed_physical": exclusions.exposed_physical.len(), "historical_aliases": exclusions.historical_aliases.len(), "lexicon": exclusions.lexicon.len()}});
    let mut report = json!({"namespace": request.namespace, "test_scored": false, "test_text_printed": false});
    if let (Some(table), Some(extra)) = (report.as_object_mut(), metadata.as_object()) {
        table.extend(extra.clone());
    }
    if overlap.values().any(|count| *count != 0) {
        return Err(format!("holdout overlaps a previously accessed test: {overlap:?}"));
    }
    if let Some(output) = request.output {
        let generator = include_str!("freeze.rs");
        let base_manifest = begin_output(request.base, output, generator)?;
        let mut files = Map::new();
        for split in FITTING_SPLITS {
            let record = &base_manifest["splits"][split];
            let source_path = request.base.join(record["path"].as_str().ok_or("a split without a path")?);
            if record["sha256"] != json!(checksum(&source_path)?) {
                return Err(format!("base split checksum mismatch: {split}"));
            }
            std::fs::copy(&source_path, output.join(format!("{split}.jsonl.gz"))).map_err(|error| error.to_string())?;
            files.insert(split.to_owned(), record.clone());
        }
        let quarantine_record = &base_manifest["splits"]["quarantine"];
        let quarantine_source = request.base.join(quarantine_record["path"].as_str().ok_or("a split without a path")?);
        if quarantine_record["sha256"] != json!(checksum(&quarantine_source)?) {
            return Err("base quarantine checksum mismatch".to_owned());
        }
        // The previously accessed test rows, relabelled programmatically; their text is never printed.
        let mut prior = load_rows(request.base, "test")?;
        for row in &mut prior {
            "quarantine".clone_into(&mut row.split);
            row.quarantine_reasons = vec![PRIOR_TEST.to_owned()];
        }
        let pick = |rows: &[CorpusRow], split: &str| -> Result<(Vec<u8>, u64), String> {
            let chosen: Vec<&CorpusRow> = rows.iter().filter(|row| row.split == split).collect();
            Ok((gzip_member(&chosen)?, chosen.len().to::<u64>()))
        };
        let test_path = output.join("test.jsonl.gz");
        let (first, first_count) = pick(&ud_rows, "test")?;
        let (second, second_count) = pick(&technical_rows, "test")?;
        std::fs::write(&test_path, [first, second].concat()).map_err(|error| error.to_string())?;
        files.insert("test".to_owned(), split_record(&test_path, first_count + second_count)?);
        let quarantine_path = output.join("quarantine.jsonl.gz");
        let base_quarantine = std::fs::read(&quarantine_source).map_err(|error| error.to_string())?;
        let prior_refs: Vec<&CorpusRow> = prior.iter().collect();
        let prior_member = gzip_member(&prior_refs)?;
        let (ud_member, ud_count) = pick(&ud_rows, "quarantine")?;
        let (technical_member, technical_count) = pick(&technical_rows, "quarantine")?;
        std::fs::write(&quarantine_path, [base_quarantine, prior_member, ud_member, technical_member].concat()).map_err(|error| error.to_string())?;
        let expected = quarantine_record["rows"].as_u64().ok_or("a split without a row count")? + prior.len().to::<u64>() + ud_count + technical_count;
        files.insert("quarantine".to_owned(), split_record(&quarantine_path, expected)?);
        let membership = json!({"namespace": request.namespace, "scope": "test-only holdout extension; families known to the base corpus, its accessed test, the prefix curriculum or declared exposure are quarantined",
            "row_ids_sha256": row_ids, "family_ids_sha256": family_ids, "document_ids_sha256": document_ids, "alias_sha256": aliases, "alias_scope": ALIAS_SCOPE});
        let membership_path = output.join("test-membership.json");
        std::fs::write(&membership_path, crate::artifact::canonical(&membership)).map_err(|error| error.to_string())?;
        let mut partitioning = json!({"mode": "test-only-holdout-extension"});
        if let (Some(table), Some(extra)) = (partitioning.as_object_mut(), metadata.as_object()) {
            table.extend(extra.clone());
        }
        let manifest = json!({"schema_version": 1, "namespace": request.namespace, "label": "keep", "splits": files,
            "scope": "holdout extension: base train/development/calibration byte-identical; TEST from new treebanks and commands outside the shipped identifier lexicon; not globally unseen lexicon or verified human intent",
            "token_language": "surface alphabet determines RU/Latin group; ASCII command alphabet uses group 0",
            "context": format!("within the same sentence only; before<={CORPUS_BEFORE_WINDOW_CHARACTERS} and after<={CORPUS_AFTER_WINDOW_CHARACTERS} characters; commands have empty before/after"),
            "partitioning": partitioning,
            "origins": {"base_manifest_sha256": checksum(&request.base.join("manifest.json"))?, "base_test_membership_sha256": checksum(&request.base.join("test-membership.json"))?, "base_namespace": base_manifest["namespace"], "base_test_rows_quarantined": prior.len()},
            "provenance": provenance, "generator_sha256": hex(&Sha256::digest(generator.as_bytes())), "generator_source": "generator-source.rs", "test_membership_sha256": checksum(&membership_path)?,
            "compression": "gzip members; base members copied byte-for-byte; raw content SHA256 is independent of deflate implementation",
            "test_policy": "sealed before model scoring; loader reads only the explicitly named split; quarantine is never a training split; the base TEST is quarantined because it was already accessed",
            "limitations": ["UD text is naturally occurring, not reviewed keyboard-intent ground truth", "technical rows measure commands absent from the shipped lexicon: the unknown-identifier regime", "a treebank without document markers is sampled by sentence", "past morphology, dictionaries, private experiments and pretrained models are not exhaustively inventoried"]});
        std::fs::write(output.join("manifest.json"), crate::artifact::canonical(&manifest)).map_err(|error| error.to_string())?;
        report["manifest_sha256"] = json!(checksum(&output.join("manifest.json"))?);
        report["test_membership_sha256"] = manifest["test_membership_sha256"].clone();
        report["splits"] = manifest["splits"].clone();
    }
    if let Some(path) = request.report {
        std::fs::write(path, crate::artifact::canonical(&report)).map_err(|error| error.to_string())?;
    }
    Ok(json!({"ud": report["ud"], "technical": report["technical"], "prior_access_overlap": report["prior_access_overlap"], "written": request.output.is_some()}))
}

/// `fitting_split`: which fitting split a new family belongs to, deterministically.
fn fitting_split(namespace: &str, family: &str) -> &'static str {
    let bucket = draw(&format!("{namespace}:fitting:{family}")) % SPLIT_BUCKET_COUNT;
    FITTING_SHARES.iter().find(|share| bucket < share.0).map(|share| share.1).expect("the shares cover every bucket")
}

/// `freeze_context_action_fitting.main`: fitting splits extended with short utterances, the test left alone.
pub fn freeze_fitting(request: &FreezeRequest<'_>) -> Result<Value, String> {
    let output = request.output.ok_or("a fitting freeze needs an output")?;
    let exclusions = load_exclusions(request.root, request.base, request.ud_origin, request.technical_origin, request.prefix_inventory, request.lexicon, &request.extra_exposure, request.ledger)?;
    let directory = request.tatoeba_directory.ok_or("a fitting freeze needs the Tatoeba exports")?;
    let (exports, tatoeba_provenance, mut tatoeba_metadata) = verified_tatoeba_source(directory, &request.tatoeba_languages)?;
    let captured = captured_tatoeba_identifiers(request.root)?;
    let mut provenance = exclusions.provenance.clone();
    provenance.extend(tatoeba_provenance);
    provenance.insert(CAPTURED_TATOEBA.to_owned(), checksum(&request.root.join(CAPTURED_TATOEBA))?);
    tatoeba_metadata["captured_training_sentences_excluded"] = json!(captured.len());
    let mut sentences = Vec::new();
    for (label, language, path) in exports {
        sentences.extend(read_tatoeba(&path, &label, &language, request.namespace, &captured)?);
    }
    sentence_documents(&mut sentences);
    let total = sentences.len();
    let typed: Vec<Sentence> = sentences.into_iter().filter(|sentence| !lookalike_typo(sentence)).collect();
    let dropped = total - typed.len();
    let (selected, mut sampling) = select_sentences(typed, request.namespace, request.max_documents, request.max_sentences_per_document, true)?;
    sampling["lookalike_typo_sentences_dropped"] = json!(dropped);
    // What the base fits and what its test holds, by the aliases of the words.
    let mut alias_splits: HashMap<String, String> = HashMap::new();
    let mut contested: HashSet<String> = HashSet::new();
    for split in FITTING_SPLITS {
        for row in load_rows(request.base, split)? {
            for alias in row_aliases(&row) {
                let held = alias_splits.entry(alias.clone()).or_insert_with(|| row.split.clone());
                if *held != row.split {
                    contested.insert(alias);
                }
            }
        }
    }
    let membership = read_json(&request.base.join("test-membership.json"))?;
    if read_json(&request.base.join("manifest.json"))?["test_membership_sha256"] != json!(checksum(&request.base.join("test-membership.json"))?) {
        return Err("base test membership differs from its manifest".to_owned());
    }
    let base_test_families: HashSet<String> = string_items(&membership["family_ids_sha256"]).into_iter().collect();
    let base_test_rows: HashSet<String> = string_items(&membership["row_ids_sha256"]).into_iter().collect();
    let base_test_documents: HashSet<String> = string_items(&membership["document_ids_sha256"]).into_iter().collect();
    if membership.get("alias_sha256").is_none() {
        return Err("a base whose test membership names no words is not supported".to_owned());
    }
    let base_test_aliases: HashSet<String> = string_items(&membership["alias_sha256"]).into_iter().collect();
    let ledger = ledger_memberships(request.ledger)?;
    let ledger_families: HashSet<String> = ledger.iter().flat_map(|(_, record)| string_items(&record["family_ids_sha256"])).collect();
    let mut test_rows: HashSet<String> = ledger.iter().flat_map(|(_, record)| string_items(&record["row_ids_sha256"])).collect();
    let mut test_documents: HashSet<String> = ledger.iter().flat_map(|(_, record)| string_items(&record["document_ids_sha256"])).collect();
    let mut test_aliases: HashSet<String> = ledger.iter().flat_map(|(_, record)| string_items(&record["alias_sha256"])).collect();
    let without = ledger.iter().filter(|(_, record)| record.get("alias_sha256").is_none()).count();
    test_rows.extend(base_test_rows.iter().cloned());
    test_documents.extend(base_test_documents.iter().cloned());
    test_aliases.extend(base_test_aliases.iter().cloned());
    let (rows, families) = family_rows(&selected)?;
    drop(selected);
    let mut reasons_by_family: HashMap<&str, Vec<String>> = HashMap::new();
    let mut split_by_family: HashMap<&str, String> = HashMap::new();
    for (family, hashes) in &families.hashes_by_family {
        // Being in the base corpus is what this extension is for; being in anyone's test is not.
        let mut reasons: Vec<String> = alias_reasons(hashes, &families.physical_by_family[family], family, &exclusions).into_iter().filter(|reason| reason != "family-in-base-corpus").collect();
        if (base_test_families.contains(family) || ledger_families.contains(family) || meets(hashes, &test_aliases)) && !reasons.iter().any(|reason| reason == "prior-accessed-test-family") {
            reasons.push("prior-accessed-test-family".to_owned());
        }
        // A word the base fits follows its split; one the base fits in two splits cannot be fitted in one and measured in another.
        let splits: BTreeSet<&String> = hashes.iter().filter_map(|alias| alias_splits.get(alias)).collect();
        if meets(hashes, &contested) || splits.len() > 1 {
            reasons.push("base-split-conflict".to_owned());
        } else if let Some(split) = splits.into_iter().next() {
            split_by_family.insert(family, split.clone());
        }
        reasons_by_family.insert(family, reasons);
    }
    let mut result = Vec::with_capacity(rows.len());
    let mut reasons_count = BTreeMap::new();
    let mut split_counts: BTreeMap<String, i64> = BTreeMap::new();
    for mut row in rows {
        let mut reasons = reasons_by_family.get(row.family.as_str()).cloned().unwrap_or_default();
        if row.alignment != "exact-text-comment" {
            reasons.push(row.alignment.clone());
        }
        // The row and document digests can tell whether a sealed test has seen this row; a family identifier cannot.
        if test_rows.contains(&hex_digest(&row.identifier)) {
            reasons.push("prior-accessed-test-row".to_owned());
        }
        if test_documents.contains(&hex_digest(&row.document)) {
            reasons.push("prior-accessed-test-document".to_owned());
        }
        let split = if reasons.is_empty() { split_by_family.get(row.family.as_str()).cloned().unwrap_or_else(|| fitting_split(request.namespace, &row.family).to_owned()) } else { "quarantine".to_owned() };
        count_into(&mut reasons_count, &reasons);
        *split_counts.entry(split.clone()).or_insert(0) += 1;
        row.split = split;
        row.quarantine_reasons = reasons;
        result.push(row);
    }
    result.sort_by(|a, b| a.identifier.cmp(&b.identifier));
    let kept: Vec<&CorpusRow> = result.iter().filter(|row| row.split != "quarantine").collect();
    let added_families: BTreeSet<&str> = kept.iter().map(|row| row.family.as_str()).collect();
    let summary = json!({"rows": result.len(), "families": families.hashes_by_family.len(), "added_rows": kept.len(), "added_families": added_families.len(), "added_by_split": split_counts,
        "followed_base_split": kept.iter().filter(|row| split_by_family.contains_key(row.family.as_str())).count(), "quarantine_reasons": reasons_count});
    let refused_rows: HashSet<&String> = test_rows.iter().collect();
    let metadata = json!({"sampling": sampling, "fitting": summary, "source": tatoeba_metadata, "base_alias_splits": alias_splits.len(), "base_contested_aliases": contested.len(), "base_test_families": base_test_families.len(), "base_test_alias_source": "membership",
        "ledger_families": ledger_families.len(), "ledger_records_without_aliases": without, "refused_test_rows": refused_rows.len(), "refused_test_documents": test_documents.len(), "refused_test_aliases": test_aliases.len()});
    let generator = include_str!("freeze.rs");
    let base_manifest = begin_output(request.base, output, generator)?;
    let mut files = Map::new();
    let test_record = &base_manifest["splits"]["test"];
    let test_source = request.base.join(test_record["path"].as_str().ok_or("a split without a path")?);
    if test_record["sha256"] != json!(checksum(&test_source)?) {
        return Err("base split checksum mismatch: test".to_owned());
    }
    std::fs::copy(&test_source, output.join("test.jsonl.gz")).map_err(|error| error.to_string())?;
    files.insert("test".to_owned(), test_record.clone());
    for split in FITTING_SPLITS.iter().copied().chain(std::iter::once("quarantine")) {
        let record = &base_manifest["splits"][split];
        let source_path = request.base.join(record["path"].as_str().ok_or("a split without a path")?);
        if record["sha256"] != json!(checksum(&source_path)?) {
            return Err(format!("base split checksum mismatch: {split}"));
        }
        let new_rows: Vec<&CorpusRow> = result.iter().filter(|row| row.split == split).collect();
        let path = output.join(format!("{split}.jsonl.gz"));
        let bytes = [std::fs::read(&source_path).map_err(|error| error.to_string())?, gzip_member(&new_rows)?].concat();
        std::fs::write(&path, bytes).map_err(|error| error.to_string())?;
        files.insert(split.to_owned(), split_record(&path, record["rows"].as_u64().ok_or("a split without a row count")? + new_rows.len().to::<u64>())?);
    }
    std::fs::copy(request.base.join("test-membership.json"), output.join("test-membership.json")).map_err(|error| error.to_string())?;
    let manifest = json!({"schema_version": base_manifest["schema_version"], "namespace": request.namespace, "kind": "fitting-extension", "base_namespace": base_manifest["namespace"], "base_manifest_sha256": checksum(&request.base.join("manifest.json"))?,
        "test_membership_sha256": checksum(&output.join("test-membership.json"))?, "splits": files, "provenance": provenance, "metadata": metadata, "generator_sha256": hex(&Sha256::digest(generator.as_bytes()))});
    std::fs::write(output.join("manifest.json"), crate::artifact::canonical(&manifest)).map_err(|error| error.to_string())?;
    let mut report = json!({"manifest_sha256": checksum(&output.join("manifest.json"))?, "splits": manifest["splits"]});
    if let (Some(table), Some(extra)) = (report.as_object_mut(), metadata.as_object()) {
        table.extend(extra.clone());
    }
    if let Some(path) = request.report {
        let options = ks_text::JsonOptions { ensure_ascii: false, sort_keys: true, indent: Some(ks_text::Indent::Spaces(1)), ..ks_text::JsonOptions::default() };
        std::fs::write(path, ks_text::json_dumps(&report, &options) + "\n").map_err(|error| error.to_string())?;
    }
    Ok(json!({"written": true, "manifest_sha256": report["manifest_sha256"], "added_by_split": metadata["fitting"]["added_by_split"], "added_rows": metadata["fitting"]["added_rows"], "quarantine_reasons": metadata["fitting"]["quarantine_reasons"]}))
}
