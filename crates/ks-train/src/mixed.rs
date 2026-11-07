//! Public text typed the way a person switches layouts, through the engine (`tools/mixed_typing.py`): the questions the
//! engine asks the model on the way (`capture`), the final text against the intended one (`evaluate`), and the sealed test
//! of developer chat built on it (`tools/evaluate_devchat_test.py`). No message is printed or written by a read of that test.
use ks_cast::To as _;
use crate::physical::{hex, hex_digest};
use crate::tools::Options;
use flate2::read::MultiGzDecoder;
use ks_backend::constants::SHIFT_MASK;
use ks_backend::{FieldContext, FieldReader, FieldRole, KeyEvent};
use ks_config::config::SettingsStore;
use ks_config::history::HistoryStore;
use ks_context::context_action_features as features;
use ks_context::context_model::{ContextEvidence, ContextModel, ContextPrediction, BeforeOrigin};
use ks_context::input_context::InputContext;
use ks_context::ContextPolicy;
use ks_engine::engine::{EngineCore, EngineParts};
use ks_lang::{LanguageModel, Resources};
use ks_lang::ortho_model::{OrthoModel, artifact_path};
use ks_replay::harness::{Backend, FakeClock};
use ks_text::{json_dumps, JsonOptions, pystr};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

pub const TYPING_MODES: [&str; 4] = ["correct", "forget_ru", "forget_en", "start_wrong"];
const SLIP_MODE: &str = "slip";
const LETTER_SLIP_MODE: &str = "slip_letter";
const LETTER_SLIP_BACK_MODE: &str = "slip_letter_back";
const UNPROTECTED_MODE: &str = "unprotected";
const US_SIGNS_MODE: &str = "us_signs";
const EDIT_MODE: &str = "insert";
const US_ROWS: [&str; 3] = ["qwertyuiop[]", "asdfghjkl;'", "zxcvbnm,./"];
const RU_ROWS: [&str; 3] = ["йцукенгшщзхъ", "фывапролджэ", "ячсмитьбю."];
const TYPO_KINDS: [&str; 4] = ["missed", "swapped", "doubled", "neighbour"];
const MIXED_DRAW_HEX_DIGITS: usize = 8;
const MIXED_DRAW_RESOLUTION: u64 = 10000;
const MIXED_EDIT_CLOCK_START_SECONDS: f64 = 1000.0;
const MIXED_EDIT_KEY_INTERVAL_SECONDS: f64 = 0.08;
const MIXED_EDIT_LENGTH_SHIFT: u32 = 8;
const MIXED_EDIT_MAX_REMOVED_LETTERS: u64 = 2;
const MIXED_EDIT_PAUSE_SECONDS: f64 = 2.0;
const MIXED_EDIT_PLACE_SHIFT: u32 = 12;
const MIXED_EDIT_TWO_LETTERS_FROM: usize = 5;
const MIXED_EDIT_WORD_MIN_LETTERS: usize = 4;
const MIXED_PREFIX_CODE_LINES: usize = 200_000;
const MIXED_PREFIX_KINDS: u64 = 2;
const MIXED_TYPO_HEX_DIGITS: usize = 12;
const MIXED_TYPO_MIN_LETTERS: usize = 3;
const MIXED_TYPO_NEIGHBOUR_SHIFT: u32 = 16;
const MIXED_TYPO_PLACE_SHIFT: u32 = 4;
const MIXED_CAPTURE_XZ_PRESET: u32 = 9;
const EDITOR_REPLAY_FIRST_KEY_SERIAL: u32 = 100;
const CONTEXT_LIMIT: i64 = 512;
const ACTION_FEATURE_AFTER_CONTEXT_CHARACTERS: i64 = 128;
const MIXED_EVALUATE_RUSSIAN_SHARE: f64 = 0.25;

fn draw_with(text: &str, digits: usize) -> u64 {
    u64::from_str_radix(hex_digest(text).get(..digits).unwrap_or_default(), 16).expect("hex digits")
}

fn draw(text: &str) -> u64 {
    draw_with(text, MIXED_DRAW_HEX_DIGITS)
}

fn russian_letter(character: char) -> bool {
    ('а'..='я').contains(&character) || ('А'..='Я').contains(&character) || character == 'ё' || character == 'Ё'
}

/// `script`: the layout a token's letters belong to.
fn script(token: &str) -> Option<i64> {
    if token.chars().any(russian_letter) {
        Some(1)
    } else if token.chars().any(|character| character.is_ascii_alphabetic()) {
        Some(0)
    } else {
        None
    }
}

fn is_alpha(character: char) -> bool {
    crate::rows::is_alpha(character)
}

/// A token, its intended layout, the layout selected before it and whether that change slipped in unseen.
type Step = (String, i64, Option<i64>, bool);

/// `plan`: the starting layout and the steps of a message.
fn plan(tokens: &[&str], mode: &str, choice: u64) -> Option<(i64, Vec<Step>)> {
    let mut intended: Vec<i64> = Vec::new();
    let mut current: Option<i64> = None;
    for token in tokens {
        if let Some(group) = script(token) {
            current = Some(group);
        }
        intended.push(current.unwrap_or(-1));
    }
    let first = intended.iter().copied().find(|group| *group >= 0)?;
    for group in &mut intended {
        if *group < 0 {
            *group = first;
        }
    }
    let transitions: Vec<usize> = (1..tokens.len()).filter(|index| intended[*index] != intended[*index - 1]).collect();
    if matches!(mode, SLIP_MODE | LETTER_SLIP_MODE | LETTER_SLIP_BACK_MODE) {
        let trusted = ks_lang::short_words::TRUSTED_SINGLE_LETTER_WORDS;
        let slips: Vec<usize> = (1..tokens.len())
            .filter(|index| {
                let token = tokens[*index];
                intended[*index - 1] == 1
                    && intended[*index] == 1
                    && script(token) == Some(1)
                    && (mode == SLIP_MODE || trusted.contains(&pystr::casefold(token).as_str()))
                    && (mode != LETTER_SLIP_BACK_MODE || index + 1 < tokens.len())
            })
            .collect();
        if slips.is_empty() {
            return None;
        }
        let slip = slips[(choice % slips.len().to::<u64>()).to::<usize>()];
        let back = if mode == LETTER_SLIP_BACK_MODE { Some(slip + 1) } else { None };
        let steps = tokens
            .iter()
            .enumerate()
            .map(|(index, token)| {
                let selected = if index == slip {
                    Some(1 - intended[index])
                } else if transitions.contains(&index) || back == Some(index) {
                    Some(intended[index])
                } else {
                    None
                };
                ((*token).to_owned(), intended[index], selected, index == slip)
            })
            .collect();
        return Some((first, steps));
    }
    let options: Vec<i64> = match mode {
        "forget_ru" => transitions.iter().filter(|index| intended[**index] == 1).map(|index| (*index).to::<i64>()).collect(),
        "forget_en" => transitions.iter().filter(|index| intended[**index] == 0).map(|index| (*index).to::<i64>()).collect(),
        "start_wrong" => vec![0],
        _ => vec![-1],
    };
    if options.is_empty() {
        return None;
    }
    let forgotten = options[(choice % options.len().to::<u64>()).to::<usize>()];
    let start = if mode == "start_wrong" { 1 - first } else { first };
    let steps = tokens
        .iter()
        .enumerate()
        .map(|(index, token)| {
            let selected = (transitions.contains(&index) && index.to::<i64>() != forgotten).then_some(intended[index]);
            ((*token).to_owned(), intended[index], selected, false)
        })
        .collect();
    Some((start, steps))
}

/// `_neighbours`: the letters beside a key.
fn neighbours(character: char) -> Vec<char> {
    let lower: Vec<char> = pystr::lower(character.encode_utf8(&mut [0_u8; 4])).chars().collect();
    for rows in [US_ROWS, RU_ROWS] {
        for row in rows {
            let letters: Vec<char> = row.chars().collect();
            let found = letters.windows(lower.len().max(1)).position(|window| window == lower.as_slice());
            if let Some(index) = found {
                let mut near: Vec<char> = Vec::new();
                if index > 0 {
                    near.push(letters[index - 1]);
                }
                if let Some(next) = letters.get(index + 1) {
                    near.push(*next);
                }
                near.retain(|character| is_alpha(*character));
                let joined: String = near.iter().collect();
                if pystr::isupper(character.encode_utf8(&mut [0_u8; 4])) {
                    return pystr::upper(&joined).chars().collect();
                }
                return near;
            }
        }
    }
    Vec::new()
}

/// `typo`: one typo among a token's letters.
fn typo(token: &str, seed: &str, kinds: &[String], least: usize) -> String {
    let letters: Vec<char> = token.chars().collect();
    let positions: Vec<usize> = letters.iter().enumerate().filter(|(_, character)| is_alpha(**character)).map(|(index, _)| index).collect();
    if positions.len() < least {
        return token.to_owned();
    }
    let digest = draw_with(seed, MIXED_TYPO_HEX_DIGITS);
    let kind = kinds[(digest % kinds.len().to::<u64>()).to::<usize>()].as_str();
    let at = positions[((digest >> MIXED_TYPO_PLACE_SHIFT) % positions.len().to::<u64>()).to::<usize>()];
    let join = |parts: &[&[char]]| -> String { parts.iter().flat_map(|part| part.iter()).collect() };
    if kind == "missed" && positions.len() > least.max(MIXED_TYPO_MIN_LETTERS) {
        return join(&[&letters[..at], &letters[at + 1..]]);
    }
    if kind == "swapped" && at + 1 < letters.len() && is_alpha(letters[at + 1]) {
        return join(&[&letters[..at], &letters[at + 1..=at + 1], &letters[at..=at], &letters[at + 2..]]);
    }
    if kind == "doubled" {
        return join(&[&letters[..=at], &letters[at..=at], &letters[at + 1..]]);
    }
    let near = neighbours(letters[at]);
    if !near.is_empty() && kind == "extra" {
        let chosen = [near[((digest >> MIXED_TYPO_NEIGHBOUR_SHIFT) % near.len().to::<u64>()).to::<usize>()]];
        return join(&[&letters[..=at], &chosen, &letters[at + 1..]]);
    }
    if !near.is_empty() {
        let chosen = [near[((digest >> MIXED_TYPO_NEIGHBOUR_SHIFT) % near.len().to::<u64>()).to::<usize>()]];
        return join(&[&letters[..at], &chosen, &letters[at + 1..]]);
    }
    join(&[&letters[..=at], &letters[at..=at], &letters[at + 1..]])
}

fn with_typos(text: &str, rate: f64, seed: &str, kinds: &[String], least: usize) -> String {
    let largest = (16_u64.pow(MIXED_DRAW_HEX_DIGITS.to::<u32>()) - 1).to::<f64>();
    pystr::split(text)
        .iter()
        .enumerate()
        .map(|(index, token)| if (draw(&format!("{seed}:{index}"))).to::<f64>() / largest < rate { typo(token, &format!("{seed}:{index}:t"), kinds, least) } else { (*token).to_owned() })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `edit_point`: a deterministic gap inside one word of the message: (start, length, layout of the word).
fn edit_point(text: &str) -> Option<(usize, usize, i64)> {
    let mut words: Vec<(usize, Vec<char>, i64)> = Vec::new();
    let mut position = 0;
    for word in text.split(' ') {
        let letters: Vec<char> = word.chars().collect();
        if let Some(group) = script(word).filter(|_| letters.len() >= MIXED_EDIT_WORD_MIN_LETTERS && pystr::isalpha(word)) {
            words.push((position, letters.clone(), group));
        }
        position += letters.len() + 1;
    }
    if words.is_empty() {
        return None;
    }
    let digest = draw(text);
    let (offset, word, group) = &words[(digest % words.len().to::<u64>()).to::<usize>()];
    let length = if word.len() >= MIXED_EDIT_TWO_LETTERS_FROM { 1 + ((digest >> MIXED_EDIT_LENGTH_SHIFT) % MIXED_EDIT_MAX_REMOVED_LETTERS).to::<usize>() } else { 1 };
    let inner = 1 + ((digest >> MIXED_EDIT_PLACE_SHIFT) % (word.len() - length - 1).to::<u64>()).to::<usize>();
    Some((offset + inner, length, *group))
}

/// `Reader`: the field as a field reader reports it: the text before the caret, and after it when editing.
struct Reader {
    backend: Arc<Backend>,
    after: bool,
}

impl FieldReader for Reader {
    fn read(&self, application: &str, _window: i64) -> Option<FieldContext> {
        let text: String = self.backend.text().iter().collect();
        let mut field = FieldContext::new(application, "1");
        field.role = FieldRole::Text;
        if self.after {
            let caret = self.backend.caret().to::<i64>();
            pystr::slice(pystr::slice(&text, None, Some(caret)), Some(-CONTEXT_LIMIT), None).clone_into(&mut field.before);
            pystr::slice(pystr::slice(&text, Some(caret), None), None, Some(ACTION_FEATURE_AFTER_CONTEXT_CHARACTERS)).clone_into(&mut field.after);
            "uia".clone_into(&mut field.source);
        } else {
            pystr::slice(&text, Some(-CONTEXT_LIMIT), None).clone_into(&mut field.before);
            "native".clone_into(&mut field.source);
        }
        Some(field)
    }

    fn status(&self) -> String {
        "available".to_owned()
    }
}

/// `PhysicalSession.key`: the event of a key named by its US character.
struct Keys {
    serial: u32,
}

fn us_to_ru(us: &str) -> String {
    let special = match us {
        "/" => Some("."),
        "?" => Some(","),
        "@" => Some("\""),
        "#" => Some("№"),
        "$" => Some(";"),
        "^" => Some(":"),
        "&" => Some("?"),
        "~" => Some("Ё"),
        "{" => Some("Х"),
        "}" => Some("Ъ"),
        ":" => Some("Ж"),
        "\"" => Some("Э"),
        "<" => Some("Б"),
        ">" => Some("Ю"),
        "|" => Some("/"),
        _ => None,
    };
    special.map_or_else(|| crate::physical::pair().translate(us, "us", "ru").expect("the pair of layouts"), str::to_owned)
}

impl Keys {
    fn key(&mut self, us: &str, group: i32) -> KeyEvent {
        self.serial += 1;
        let ru = us_to_ru(us);
        let character = if group == 0 { us.to_owned() } else { ru.clone() };
        // Every key types one character (`keys_for` gives one key per character), so `us` and its reading are never empty.
        let shift = pystr::isupper(us) || "~!@#$%^&*()_+{}|:\"<>?".contains(us);
        let name = if pystr::isspace(&character) { "space".to_owned() } else { us.to_owned() };
        KeyEvent { pressed: true, keycode: self.serial, key_name: name, character, characters: vec![us.to_owned(), ru], group, state: if shift { SHIFT_MASK } else { 0 }, timestamp: i64::from(self.serial), ..KeyEvent::default() }
    }
}

/// The keys that print each character in the Russian layout: `_state["ru_keys"]`.
fn russian_keys() -> HashMap<String, String> {
    let mut inverse = HashMap::new();
    for code in u32::from('!')..=u32::from('~') {
        let us = char::from_u32(code).expect("an ASCII character").to_string();
        inverse.entry(us_to_ru(&us)).or_insert(us);
    }
    inverse
}

fn keys_for(token: &str, group: i64, inverse: &HashMap<String, String>) -> Option<Vec<String>> {
    if group == 0 {
        return Some(token.chars().map(|character| character.to_string()).collect());
    }
    token.chars().map(|character| inverse.get(&character.to_string()).cloned()).collect()
}

/// A job: (index, text, mode, choice, the text above it).
#[derive(Clone)]
pub struct Job {
    pub index: usize,
    pub text: String,
    pub mode: String,
    pub choice: u64,
    pub prefix: String,
}

#[derive(Default)]
struct Spy {
    typed: Vec<(String, i64, i64)>,
    rows: Vec<Value>,
    edit: bool,
    fields: Vec<&'static str>,
}

/// The evidence's columns a captured row may hold, by name: `fields_of` names only these.
const EVIDENCE_COLUMNS: [(&str, fn(&ContextEvidence) -> Value); 17] = [
    ("original", |item| json!(item.original)),
    ("alternative", |item| json!(item.alternative)),
    ("source_group", |item| json!(item.source_group)),
    ("trigger", |item| json!(item.trigger)),
    ("baseline_convert", |item| json!(item.baseline_convert)),
    ("source_known", |item| json!(item.source_known)),
    ("target_known", |item| json!(item.target_known)),
    ("score_delta", |item| json!(item.score_delta)),
    ("literal_tail", |item| json!(item.literal_tail)),
    ("boundary_text", |item| json!(item.boundary_text)),
    ("after_origin", |item| json!(item.after_origin.as_str())),
    ("before_origin", |item| json!(item.before_origin.as_str())),
    ("source_typo", |item| json!(item.source_typo)),
    ("target_typo", |item| json!(item.target_typo)),
    ("source_opening", |item| json!(item.source_opening)),
    ("target_opening", |item| json!(item.target_opening)),
    ("inside", |item| json!(item.inside)),
];

fn captured_row(item: &ContextEvidence, label: &str, fields: &[&'static str]) -> Value {
    let mut row = Vec::new();
    for name in fields {
        let (_, value) = EVIDENCE_COLUMNS.iter().find(|column| column.0 == *name).expect("a column of the evidence");
        row.push(value(item));
    }
    row.extend([json!(item.field.before), json!(item.field.after), json!(item.field.role.as_str()), json!(label)]);
    Value::Array(row)
}

/// One worker's engine pieces: a session is built on them per message.
pub struct Typist {
    resources: Arc<Resources>,
    models: Vec<Arc<LanguageModel>>,
    shared: ks_context::context_policy::SharedOrtho,
    model_path: PathBuf,
    capturing: bool,
    policy: Option<ContextPolicy>,
    spy: Arc<Mutex<Spy>>,
    folder: PathBuf,
    inverse: HashMap<String, String>,
}

/// What every worker of a bench shares: the engine's dictionaries and the orthotactic model bound to them.
pub struct Bench {
    resources: Arc<Resources>,
    models: Vec<Arc<LanguageModel>>,
    shared: ks_context::context_policy::SharedOrtho,
}

/// The context policy of a typist: the model of `model_path`, its answers watched by the spy when capturing.
fn policy_of(resources: &Arc<Resources>, shared: &ks_context::context_policy::SharedOrtho, model_path: &Path, spy: &Arc<Mutex<Spy>>, capturing: bool) -> Result<ContextPolicy, String> {
    let models_dir = resources.resource_dir().join("models");
    let mut policy = ContextPolicy::with_ortho(resources, None, shared.clone(), None);
    let load = || ContextModel::load_artifact(model_path, &models_dir).map_err(|error| format!("{}: {}", model_path.display(), error.name));
    let mut model = load()?;
    if capturing {
        let inner = load()?;
        let handle = spy.clone();
        model.answer = Some(Arc::new(move |item: &ContextEvidence| -> ContextPrediction {
            let result = inner.predict(item);
            let mut state = handle.lock().unwrap_or_else(PoisonError::into_inner);
            if state.edit {
                if item.inside {
                    let row = captured_row(item, "convert", &state.fields);
                    state.rows.push(row);
                }
                return result;
            }
            let core = pystr::casefold(&item.original);
            let hit = state.typed.iter().rev().find(|(text, group, _)| !core.is_empty() && text.contains(&core) && item.source_group == *group).map(|(_, group, intended)| group != intended);
            if let Some(converted) = hit {
                let row = captured_row(item, if converted { "convert" } else { "keep" }, &state.fields);
                state.rows.push(row);
            }
            result
        }));
    }
    policy.model = Some(model);
    Ok(policy)
}

impl Bench {
    pub fn new(resource_dir: &Path) -> Result<Bench, String> {
        let resources: Arc<Resources> = Resources::with_speller(resource_dir.to_path_buf(), ks_hunspell::hunspell_speller(ks_config::history::data_dir()));
        let mut models = Vec::new();
        for locale in crate::tools::LOCALES {
            models.push(resources.engine_language_model(locale).map_err(|error| error.name.to_string())?);
        }
        let known = EngineCore::known_of(&models);
        let shared = match OrthoModel::load(&artifact_path(resources.resource_dir()), Some(resources.packaged_identifier()), known, Some(resources.clone())) {
            Ok(model) => {
                let version = model.version.clone();
                (Some(Arc::new(model)), version)
            }
            Err(error) => (None, format!("unavailable: {}", error.name)),
        };
        Ok(Bench { resources, models, shared })
    }

    fn typist(&self, model_path: &Path, worker: usize, fields: Vec<&'static str>, capturing: bool) -> Result<Typist, String> {
        let spy = Arc::new(Mutex::new(Spy { fields, ..Spy::default() }));
        let policy = policy_of(&self.resources, &self.shared, model_path, &spy, capturing)?;
        let folder = std::env::temp_dir().join(format!("ks-mixed-{}-{worker}", std::process::id()));
        std::fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
        Ok(Typist {
            resources: self.resources.clone(),
            models: self.models.clone(),
            shared: self.shared.clone(),
            model_path: model_path.to_path_buf(),
            capturing,
            policy: Some(policy),
            spy,
            folder,
            inverse: russian_keys(),
        })
    }
}

struct Session {
    core: EngineCore,
    backend: Arc<Backend>,
    keys: Keys,
    clock: FakeClock,
    settings: Arc<SettingsStore>,
}

/// What one message typed in one mode came to: what failed comes back as text, as `safe_evaluate` keeps its kind.
type Typed<T> = Result<Option<T>, String>;

impl Typist {
    /// `_session`: an engine on an editor; typing respects manual layout changes, editing does not.
    fn session(&mut self, start: i64, editing: bool) -> Result<Session, String> {
        let settings = Arc::new(SettingsStore::new(Some(self.folder.join("settings.json"))));
        let mut changes = vec![
            ("detection.early_switch", json!(false)),
            ("detection.respect_manual_layout", json!(false)),
            ("detection.context_read_field", json!(false)),
            ("detection.learning", json!(false)),
            ("general.keep_history", json!(false)),
            ("diagnostics.technical_logging", json!(false)),
        ];
        if !editing {
            changes.push(("detection.respect_manual_layout", json!(true)));
        }
        changes.push(("detection.context_read_field", json!(true)));
        for (name, value) in changes {
            settings.set(name, value, false).expect("in memory");
        }
        let backend = Arc::new(Backend::new("Telegram", start.to::<i32>(), Vec::new()));
        let clock = FakeClock::new();
        if editing {
            clock.advance(MIXED_EDIT_CLOCK_START_SECONDS - clock.now());
        }
        let mut policy = self.policy.take().expect("one engine at a time");
        policy.stream = InputContext::with_clock(clock.read());
        policy.reader = None;
        let (intent_model, intent_model_status) = self.resources.intent_model();
        let made = EngineCore::from_parts(EngineParts {
            settings: settings.clone(),
            history: Arc::new(HistoryStore::new(Some(self.folder.join("history.jsonl")), 200)),
            backend: backend.clone(),
            learning: None,
            backend_label: "editor".to_owned(),
            models: self.models.clone(),
            intent_model,
            intent_model_status,
            protected_tokens: self.resources.protected_tokens(),
            context_policy: policy,
            boundary_model: self.resources.boundary_policy(),
            prefix_model: None,
            prefix_indexes: Vec::new(),
            resource_dir: self.resources.resource_dir().to_path_buf(),
            clock: clock.read(),
            platform_reader: None,
        });
        match made {
            Ok(core) => Ok(Session { core, backend, keys: Keys { serial: EDITOR_REPLAY_FIRST_KEY_SERIAL }, clock, settings }),
            Err(error) => {
                // The engine that was not made took the policy with it: the next message is typed on a new one.
                self.policy = Some(policy_of(&self.resources, &self.shared, &self.model_path, &self.spy, self.capturing)?);
                Err(error.to_string())
            }
        }
    }

    fn finish(&mut self, session: Session) -> String {
        self.policy = Some(session.core.context_policy);
        let _ = std::fs::remove_file(self.folder.join("history.jsonl"));
        session.backend.text().iter().collect()
    }

    fn send(session: &mut Session, event: KeyEvent) -> Result<(), String> {
        session.backend.type_key(&event);
        session.core.handle_inner(Arc::new(event)).map_err(|error| error.to_string())
    }

    fn tap(session: &mut Session, event: KeyEvent) -> Result<(), String> {
        let released = KeyEvent { pressed: false, ..event.clone() };
        Self::send(session, event)?;
        Self::send(session, released)
    }

    /// `_type_message`: one message in one mode; the final text, or None if it cannot be typed.
    fn type_message(&mut self, job: &Job) -> Typed<String> {
        let tokens = pystr::split(&job.text);
        let Some((start, steps)) = plan(&tokens, &job.mode, job.choice) else { return Ok(None) };
        let mut session = self.session(start, false)?;
        if matches!(job.mode.as_str(), SLIP_MODE | LETTER_SLIP_MODE | LETTER_SLIP_BACK_MODE) {
            // A layout the engine did not select is no switch by hand that protects the next word.
            session.settings.set("detection.respect_manual_layout", json!(false), false).expect("in memory");
            session.settings.set("detection.early_switch", json!(false), false).expect("in memory");
        } else if job.mode == UNPROTECTED_MODE {
            session.settings.set("detection.respect_manual_layout", json!(false), false).expect("in memory");
        }
        session.core.context_policy.reader = Some(Arc::new(Reader { backend: session.backend.clone(), after: false }));
        if !job.prefix.is_empty() {
            let characters: Vec<char> = job.prefix.chars().collect();
            let caret = characters.len();
            session.backend.set_text(characters);
            session.backend.set_caret(caret);
        }
        let typed = self.type_steps(&mut session, &job.mode, steps);
        // The engine pieces come back whatever happened to the message.
        let text = self.finish(session);
        Ok(typed?.then_some(text))
    }

    /// The tokens of a plan typed one after another: false when a token has no keys in its layout.
    fn type_steps(&self, session: &mut Session, mode: &str, steps: Vec<Step>) -> Result<bool, String> {
        for (token, intended, selected, slipped) in steps {
            if let Some(selected) = selected.filter(|value| i64::from(session.backend.group()) != *value) {
                session.backend.set_group(selected.to::<i32>());
                if !slipped {
                    session.core.observe_group(selected, "hotkey");
                }
            }
            let Some(mut keys) = keys_for(&token, intended, &self.inverse) else { return Ok(false) };
            let mut meant = intended;
            if mode == US_SIGNS_MODE && intended == 1 && token.chars().any(|character| pystr::isdigit(character.encode_utf8(&mut [0_u8; 4]))) {
                let signed: Vec<String> = token.chars().zip(keys.iter()).map(|(character, key)| if matches!(character, '.' | ',') { character.to_string() } else { key.clone() }).collect();
                // The sign typed on the key of the English layout means that key's reading there: `2ю` is meant `2.`.
                meant = if signed == keys { intended } else { 1 - intended };
                keys = signed;
            }
            let group = session.backend.group();
            let typed: String = keys.iter().map(|key| session.keys.key(key, group).character).collect();
            self.spy.lock().unwrap_or_else(PoisonError::into_inner).typed.push((pystr::casefold(&typed), i64::from(group), meant));
            keys.push(" ".to_owned());
            for key in keys {
                let group = session.backend.group();
                let event = session.keys.key(&key, group);
                Self::tap(session, event)?;
            }
        }
        Ok(true)
    }

    /// `_edit_message`: letters removed from inside a word are typed back after a click in the other layout.
    fn edit_message(&mut self, job: &Job) -> Typed<String> {
        let Some((start, length, own)) = edit_point(&job.text) else { return Ok(None) };
        let letters: Vec<char> = job.text.chars().collect();
        let piece: String = letters[start..start + length].iter().collect();
        let Some(keys) = keys_for(&piece, own, &self.inverse) else { return Ok(None) };
        let mut session = self.session(1 - own, true)?;
        session.core.context_policy.reader = Some(Arc::new(Reader { backend: session.backend.clone(), after: true }));
        let prefix: Vec<char> = job.prefix.chars().collect();
        let caret = prefix.len() + start;
        session.backend.set_text(prefix.into_iter().chain(letters[..start].iter().copied()).chain(letters[start + length..].iter().copied()).collect());
        session.backend.set_caret(caret);
        let edited = Self::type_back(&mut session, keys);
        let text = self.finish(session);
        edited.map(|()| Some(text))
    }

    /// A click into the gap, the letters typed into it and the pause after them.
    fn type_back(session: &mut Session, keys: Vec<String>) -> Result<(), String> {
        let group = session.backend.group();
        let pointer = KeyEvent { pressed: true, keycode: 0, key_name: "Pointer".to_owned(), character: String::new(), characters: vec![String::new(), String::new()], group, state: 0, timestamp: 0, ..KeyEvent::default() };
        let released = KeyEvent { pressed: false, ..pointer.clone() };
        session.core.handle_inner(Arc::new(pointer)).map_err(|error| error.to_string())?;
        session.core.handle_inner(Arc::new(released)).map_err(|error| error.to_string())?;
        for key in keys {
            session.clock.advance(MIXED_EDIT_KEY_INTERVAL_SECONDS);
            let group = session.backend.group();
            let event = session.keys.key(&key, group);
            Self::tap(session, event)?;
        }
        session.clock.advance(MIXED_EDIT_PAUSE_SECONDS);
        session.core.maybe_correct_after_pause(None).map_err(|error| error.to_string())
    }

    /// `_capture`: the questions of one job. A message the engine fails on stops the capture, as it stops the Python's.
    fn capture(&mut self, job: &Job) -> Vec<Value> {
        {
            let mut state = self.spy.lock().unwrap_or_else(PoisonError::into_inner);
            state.typed.clear();
            state.rows.clear();
            state.edit = job.mode == EDIT_MODE;
        }
        let typed = if job.mode == EDIT_MODE { self.edit_message(job) } else { self.type_message(job) }.expect("the engine types the message");
        let mut state = self.spy.lock().unwrap_or_else(PoisonError::into_inner);
        if job.mode != EDIT_MODE && typed.is_none() {
            state.rows.clear();
        }
        std::mem::take(&mut state.rows)
    }

    /// `_evaluate`: the final text against the intended one, None if it cannot be typed, or what the engine failed on.
    pub fn evaluate(&mut self, job: &Job) -> Typed<(String, String, bool)> {
        self.spy.lock().unwrap_or_else(PoisonError::into_inner).typed.clear();
        let got = if job.mode == EDIT_MODE { self.edit_message(job) } else { self.type_message(job) }?;
        Ok(got.map(|got| {
            let expected = format!("{}{}{}", job.prefix, job.text, if job.mode == EDIT_MODE { "" } else { " " });
            let same = got == expected;
            (expected, got, same)
        }))
    }
}

fn gz_lines(path: &Path) -> Result<Vec<String>, String> {
    let file = std::fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    std::io::BufReader::new(MultiGzDecoder::new(file)).lines().collect::<Result<_, _>>().map_err(|error| error.to_string())
}

fn string_list(value: &Value) -> Vec<String> {
    value.as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect()
}

/// `recipe_messages`: the recipe's messages by seed, mixed ones first, as typed (typos included), with their ids.
fn recipe_messages(messages: &Path, recipe: &Value) -> Result<Vec<(String, i64)>, String> {
    let seed = recipe["seed"].as_str().ok_or("a recipe without a seed")?;
    let kinds: BTreeSet<String> = string_list(&recipe["kinds"]).into_iter().collect();
    let mut pool: Vec<(String, String, String, i64)> = Vec::new();
    for line in gz_lines(messages)? {
        let row: Value = serde_json::from_str(&line).map_err(|error| error.to_string())?;
        if row["kind"].as_str().is_some_and(|kind| kinds.contains(kind)) {
            let text = row["text"].as_str().unwrap_or("").to_owned();
            pool.push((hex_digest(&format!("{seed}:{text}")), text, row["mix"].as_str().unwrap_or("").to_owned(), row["qid"].as_i64().unwrap_or(0)));
        }
    }
    pool.sort();
    let count = recipe.int("count").to::<usize>();
    let share = recipe.float("russian_share");
    let mut chosen: Vec<(String, i64)> = pool.iter().filter(|entry| entry.2 == "mixed").take(count).map(|entry| (entry.1.clone(), entry.3)).collect();
    chosen.extend(pool.iter().filter(|entry| entry.2 == "russian").take((count.to::<f64>() * share).to::<usize>()).map(|entry| (entry.1.clone(), entry.3)));
    let rate = recipe.float_or("typo_rate", 0.0);
    let kinds: Vec<String> = recipe.get("typo_kinds").map_or_else(|| TYPO_KINDS.iter().map(|kind| (*kind).to_owned()).collect(), string_list);
    let least = (recipe.int_or("typo_min_letters", MIXED_TYPO_MIN_LETTERS.to::<i64>())).to::<usize>();
    if kinds.iter().any(|kind| !TYPO_KINDS.contains(&kind.as_str()) && kind != "extra") || least < 1 {
        return Err(format!("unknown typo kinds or length: {kinds:?}, {least}"));
    }
    if rate != 0.0 {
        chosen = chosen.into_iter().map(|(text, qid)| (with_typos(&text, rate, &format!("{seed}:typo:{text}"), &kinds, least), qid)).collect();
    }
    Ok(chosen)
}

/// `lines_above`: messages of another pool a recipe puts on the line above its own, by seed.
fn lines_above(messages: &Path, recipe: &Value) -> Result<Vec<String>, String> {
    let Some(pool) = recipe.get("lines_above").filter(|pool| pool.as_object().is_some_and(|table| !table.is_empty())) else { return Ok(Vec::new()) };
    let kinds: BTreeSet<String> = string_list(&pool["kinds"]).into_iter().collect();
    let (seed, mix) = (pool["seed"].as_str().unwrap_or(""), pool["mix"].as_str().unwrap_or(""));
    let path = messages.parent().ok_or("messages outside a folder")?.join(pool["messages"].as_str().unwrap_or(""));
    let mut chosen: Vec<(String, String)> = Vec::new();
    for line in gz_lines(&path)? {
        let row: Value = serde_json::from_str(&line).map_err(|error| error.to_string())?;
        if row["kind"].as_str().is_some_and(|kind| kinds.contains(kind)) && row["mix"].as_str() == Some(mix) {
            let text = row["text"].as_str().unwrap_or("").to_owned();
            chosen.push((hex_digest(&format!("{seed}:{text}")), text));
        }
    }
    chosen.sort();
    Ok(chosen.into_iter().take(pool.int("count").to::<usize>()).map(|entry| entry.1).collect())
}

/// `recipe_messages`, for the tool that says whose text a recipe types.
pub(crate) fn jobs_recipe_messages(messages: &Path, recipe: &Value) -> Result<Vec<(String, i64)>, String> {
    recipe_messages(messages, recipe)
}

/// `lines_above` with the ids of the pool's messages, and how many code lines a recipe has to draw from.
pub(crate) fn recipe_lines_above(messages: &Path, recipe: &Value, code_path: Option<&Path>) -> Result<(Vec<(String, i64)>, usize), String> {
    let mut above = Vec::new();
    if let Some(pool) = recipe.get("lines_above").filter(|pool| pool.as_object().is_some_and(|table| !table.is_empty())) {
        let kinds: BTreeSet<String> = string_list(&pool["kinds"]).into_iter().collect();
        let (seed, mix) = (pool["seed"].as_str().unwrap_or(""), pool["mix"].as_str().unwrap_or(""));
        let path = messages.parent().ok_or("messages outside a folder")?.join(pool["messages"].as_str().unwrap_or(""));
        let mut chosen: Vec<(String, String, i64)> = Vec::new();
        for line in gz_lines(&path)? {
            let row: Value = serde_json::from_str(&line).map_err(|error| error.to_string())?;
            if row["kind"].as_str().is_some_and(|kind| kinds.contains(kind)) && row["mix"].as_str() == Some(mix) {
                let text = row["text"].as_str().unwrap_or("").to_owned();
                chosen.push((hex_digest(&format!("{seed}:{text}")), text, row["qid"].as_i64().unwrap_or(0)));
            }
        }
        chosen.sort();
        above = chosen.into_iter().take(pool.int("count").to::<usize>()).map(|entry| (entry.1, entry.2)).collect();
    }
    let code = match code_path {
        Some(path) if recipe.float_or("prefix_rate", 0.0) != 0.0 => gz_lines(path)?.len().min(MIXED_PREFIX_CODE_LINES),
        _ => 0,
    };
    Ok((above, code))
}

/// `previous_line`: what stands on the line above a message: ("code", line), ("message", index) or ("above", index).
pub(crate) fn recipe_previous_line(recipe: &Value, text: &str, index: usize, messages: usize, code_lines: usize, lines_above: usize) -> Option<(&'static str, usize)> {
    let value = draw(&format!("{}:prefix:{text}", recipe["seed"].as_str().unwrap_or("")));
    if (value % MIXED_DRAW_RESOLUTION).to::<f64>() >= recipe.float_or("prefix_rate", 0.0) * MIXED_DRAW_RESOLUTION.to::<f64>() {
        return None;
    }
    let half = value / MIXED_PREFIX_KINDS;
    if code_lines != 0 && value.is_multiple_of(MIXED_PREFIX_KINDS) {
        return Some(("code", (half % code_lines.to::<u64>()).to::<usize>()));
    }
    if lines_above != 0 {
        return Some(("above", (half % lines_above.to::<u64>()).to::<usize>()));
    }
    Some(("message", ((half + index.to::<u64>() + 1) % messages.to::<u64>()).to::<usize>()))
}

/// `jobs_for`: the recipe's messages, typos and previous lines, and one job per mode.
pub fn jobs_for(messages: &Path, recipe: &Value, code_path: Option<&Path>) -> Result<Vec<Job>, String> {
    let everyone: Vec<String> = recipe_messages(messages, recipe)?.into_iter().map(|entry| entry.0).collect();
    let prefix_rate = recipe.float_or("prefix_rate", 0.0);
    let code: Vec<String> = match code_path {
        Some(path) if prefix_rate != 0.0 => gz_lines(path)?.into_iter().take(MIXED_PREFIX_CODE_LINES).collect(),
        _ => Vec::new(),
    };
    let above_pool = lines_above(messages, recipe)?;
    let modes: Vec<String> = if recipe.get("edit").and_then(Value::as_bool) == Some(true) {
        vec![EDIT_MODE.to_owned()]
    } else {
        recipe.get("modes").map_or_else(|| TYPING_MODES.iter().map(|mode| (*mode).to_owned()).collect(), string_list)
    };
    let known = [EDIT_MODE, SLIP_MODE, LETTER_SLIP_MODE, LETTER_SLIP_BACK_MODE, UNPROTECTED_MODE, US_SIGNS_MODE];
    if modes.iter().any(|mode| !known.contains(&mode.as_str()) && !TYPING_MODES.contains(&mode.as_str())) {
        return Err(format!("unknown typing modes: {modes:?}"));
    }
    let seed = recipe["seed"].as_str().unwrap_or("");
    let mut jobs = Vec::new();
    for (index, text) in everyone.iter().enumerate() {
        let value = draw(&format!("{seed}:prefix:{text}"));
        let mut prefix = String::new();
        if (value % MIXED_DRAW_RESOLUTION).to::<f64>() < prefix_rate * MIXED_DRAW_RESOLUTION.to::<f64>() {
            let half = value / MIXED_PREFIX_KINDS;
            let line = if !code.is_empty() && value.is_multiple_of(MIXED_PREFIX_KINDS) {
                &code[(half % code.len().to::<u64>()).to::<usize>()]
            } else if !above_pool.is_empty() {
                &above_pool[(half % above_pool.len().to::<u64>()).to::<usize>()]
            } else {
                &everyone[((half + index.to::<u64>() + 1) % everyone.len().to::<u64>()).to::<usize>()]
            };
            prefix = format!("{line}\n");
        }
        let choice = draw(&format!("{seed}:choice:{text}"));
        for mode in &modes {
            jobs.push(Job { index, text: text.clone(), mode: mode.clone(), choice, prefix: prefix.clone() });
        }
    }
    Ok(jobs)
}

/// Results of jobs in their order, by `workers` typists.
fn run_jobs<T: Send>(jobs: &[Job], workers: usize, make: &(dyn Fn(usize) -> Result<Typist, String> + Sync), work: &(dyn Fn(&mut Typist, &Job) -> T + Sync)) -> Result<Vec<T>, String> {
    let next = AtomicUsize::new(0);
    let done: Mutex<Vec<Option<T>>> = Mutex::new((0..jobs.len()).map(|_| None).collect());
    let failure: Mutex<Option<String>> = Mutex::new(None);
    std::thread::scope(|scope| {
        for worker in 0..workers.max(1) {
            let (next, done, failure) = (&next, &done, &failure);
            scope.spawn(move || {
                let mut typist = match make(worker) {
                    Ok(typist) => typist,
                    Err(error) => {
                        *failure.lock().expect("a worker finished") = Some(error);
                        return;
                    }
                };
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(job) = jobs.get(index) else { break };
                    let result = work(&mut typist, job);
                    done.lock().expect("a worker finished")[index] = Some(result);
                }
                let _ = std::fs::remove_dir_all(&typist.folder);
            });
        }
    });
    if let Some(error) = failure.into_inner().expect("the workers finished") {
        return Err(error);
    }
    Ok(done.into_inner().expect("the workers finished").into_iter().map(|entry| entry.expect("every job ran")).collect())
}

fn fields_of(schema: i64) -> Option<Vec<&'static str>> {
    let mut fields = vec!["original", "alternative", "source_group", "trigger", "baseline_convert", "source_known", "target_known", "score_delta", "literal_tail", "boundary_text", "after_origin", "source_typo", "target_typo", "source_opening", "target_opening", "inside"];
    match schema {
        1 => Some(fields),
        2 => {
            let place = fields.iter().position(|name| *name == "after_origin").expect("the column") + 1;
            fields.insert(place, "before_origin");
            Some(fields)
        }
        _ => None,
    }
}

/// `CAPTURED_QUESTIONS`: the classes a capture recipe may keep alone, by the head that answers them.
fn captured_question(name: &str, row: &[Value], fields: &[&'static str], tables: &ks_context::context_model::TermTables) -> Result<bool, String> {
    // The text before and after the word follow the fields; each other name asked for is a field of both schemas.
    let at = |field: &str| -> &str { row[fields.iter().position(|name| *name == field).expect("a field of the schema")].as_str().unwrap_or("") };
    let (before, after) = (row[fields.len()].as_str().unwrap_or(""), row[fields.len() + 1].as_str().unwrap_or(""));
    let origin = crate::captured::after_origin_of(at("after_origin"));
    let (original, alternative, trigger) = (at("original"), at("alternative"), at("trigger"));
    let left_field = !fields.contains(&"before_origin") || at("before_origin") == BeforeOrigin::Field.as_str();
    Ok(match name {
        "variable" => features::variable_question(original, alternative, before, trigger),
        "dotted" => features::dotted_question(original, alternative),
        "alnum" => features::alnum_question(original, alternative, before, tables).map_err(|error| format!("{error:?}"))?,
        "hypothesis" => fields.contains(&"before_origin") && at("before_origin") == BeforeOrigin::PlannedPreviousReading.as_str(),
        "short" => left_field && features::short_question(original, before, tables).map_err(|error| format!("{error:?}"))?,
        "slip" => left_field && features::slip_question(original, before, tables),
        "sign" => features::sign_question(original, alternative, before, after, trigger, origin),
        "resume" => features::resume_question(original, alternative, before, after, origin),
        "cyrletter" => features::cyrletter_question(original, alternative, before, trigger),
        "dash" => features::dash_question(original, alternative, trigger),
        "digitdot" => features::digit_dot_question(original, alternative, trigger),
        "quote" => features::quote_question(original, alternative, trigger),
        _ => return Err(format!("no class of questions {name}")),
    })
}

fn compact(value: &Value) -> String {
    json_dumps(value, &JsonOptions { ensure_ascii: false, separators: Some((",".to_owned(), ":".to_owned())), ..JsonOptions::default() })
}

/// `capture`: the questions the engine asks while each source's recipe is typed, labelled from what the person meant.
pub fn capture(resource_dir: &Path, manifest: &Path, messages: &Path, model: &Path, only: Option<&str>, verify: bool, workers: usize) -> Result<Value, String> {
    let mut payload = crate::corpus::read_json(manifest)?;
    let fields = fields_of(payload.int("schema_version")).ok_or("invalid captured question manifest")?;
    let bench = Bench::new(resource_dir)?;
    let tables = ks_context::context_model::TermTables::load(&resource_dir.join("models")).map_err(|error| error.name.to_string())?;
    let parent = manifest.parent().ok_or("a manifest outside a folder")?.to_path_buf();
    let mut results = Map::new();
    for source in payload["sources"].as_array_mut().ok_or("a manifest without sources")? {
        let file = source["file"].as_str().unwrap_or("").to_owned();
        if only.is_some_and(|name| name != file) {
            continue;
        }
        let recipe = source["recipe"].clone();
        let code = recipe.get("code").and_then(Value::as_str).map(|name| messages.join(name));
        let jobs = jobs_for(&messages.join(recipe["messages"].as_str().ok_or("a recipe without messages")?), &recipe, code.as_deref())?;
        let labels: Option<BTreeSet<String>> = recipe.get("labels").filter(|value| value.as_array().is_some_and(|items| !items.is_empty())).map(|value| string_list(value).into_iter().collect());
        let question = recipe.get("questions").and_then(Value::as_str);
        let make = |worker: usize| bench.typist(model, worker, fields.clone(), true);
        let rows = run_jobs(&jobs, workers, &make, &|typist: &mut Typist, job: &Job| typist.capture(job))?;
        let mut seen: BTreeMap<String, i64> = BTreeMap::new();
        for row in rows.into_iter().flatten() {
            let items = row.as_array().ok_or("a captured row is a list")?;
            let label = items[items.len() - 1].as_str().unwrap_or("");
            let keep = items[0] != items[1] && labels.as_ref().is_none_or(|labels| labels.contains(label));
            let keep = keep && question.map_or(Ok(true), |name| captured_question(name, items, &fields, &tables))?;
            if keep {
                *seen.entry(compact(&row)).or_insert(0) += 1;
            }
        }
        let mut sorted: Vec<(&String, &i64)> = seen.iter().collect();
        sorted.sort();
        let mut content = String::new();
        for (row, count) in sorted {
            content.push_str(&format!("[{count},{}\n", row.get(1..).unwrap_or_default()));
        }
        let digest = hex(&Sha256::digest(content.as_bytes()));
        let path = parent.join(&file);
        let total: i64 = seen.values().sum();
        if verify {
            let mut existing = Vec::new();
            xz2::read::XzDecoder::new(std::fs::File::open(&path).map_err(|error| error.to_string())?).read_to_end(&mut existing).map_err(|error| error.to_string())?;
            if hex(&Sha256::digest(&existing)) != digest {
                return Err(format!("captured questions differ: {file}"));
            }
        } else {
            let mut encoder = xz2::write::XzEncoder::new(Vec::new(), MIXED_CAPTURE_XZ_PRESET);
            encoder.write_all(content.as_bytes()).map_err(|error| error.to_string())?;
            let data = encoder.finish().map_err(|error| error.to_string())?;
            std::fs::write(&path, &data).map_err(|error| error.to_string())?;
            source["sha256"] = json!(hex(&Sha256::digest(&data)));
            source["rows"] = json!(total);
            source["unique"] = json!(seen.len());
        }
        results.insert(file, json!({"rows": total, "unique": seen.len(), "content_sha256": digest}));
    }
    if !verify {
        // The trainer checks every file against the SHA-256 recorded here.
        let options = JsonOptions { ensure_ascii: false, indent: Some(ks_text::Indent::Spaces(1)), ..JsonOptions::default() };
        std::fs::write(manifest, json_dumps(&payload, &options) + "\n").map_err(|error| error.to_string())?;
    }
    Ok(Value::Object(results))
}

/// `evaluate`: final text against intended text, per typing mode.
pub fn evaluate(resource_dir: &Path, messages: &Path, recipe: &Value, code: Option<&Path>, model: &Path, output: &Path, workers: usize) -> Result<Value, String> {
    let jobs = jobs_for(messages, recipe, code)?;
    let bench = Bench::new(resource_dir)?;
    let make = |worker: usize| bench.typist(model, worker, Vec::new(), false);
    // A message the engine fails on stops the run, as it stops the Python's.
    let rows = run_jobs(&jobs, workers, &make, &|typist: &mut Typist, job: &Job| typist.evaluate(job).expect("the engine types the message"))?;
    let mut summary: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    let mut listed = Vec::new();
    for (job, row) in jobs.iter().zip(rows) {
        match row {
            None => listed.push(json!({"index": job.index, "mode": job.mode, "skipped": true})),
            Some((expected, got, ok)) => {
                let entry = summary.entry(job.mode.clone()).or_insert((0, 0));
                entry.0 += 1;
                entry.1 += i64::from(ok);
                listed.push(json!({"index": job.index, "mode": job.mode, "expected": expected, "got": got, "ok": ok}));
            }
        }
    }
    let table: Map<String, Value> = summary.iter().map(|(mode, (n, ok))| (mode.clone(), json!({"n": n, "ok": ok}))).collect();
    let options = JsonOptions { ensure_ascii: false, indent: Some(ks_text::Indent::Spaces(1)), ..JsonOptions::default() };
    std::fs::write(output, json_dumps(&json!({"summary": table, "rows": listed}), &options) + "\n").map_err(|error| error.to_string())?;
    Ok(json!({"summary": table}))
}

// ---- the sealed test of developer chat ------------------------------------------------------------------------------

const DEVCHAT_NAMESPACE: &str = "keyswitch:devchat-2026-10-15:question";
const DEVCHAT_SEED: &str = "devchat-v1";
const DEVCHAT_MIN_NET_MARGIN: i64 = 1;
const DEVCHAT_SIGNIFICANCE: f64 = 0.05;
const BENCH: &str = "mixed_typing: the field read, a layout chosen by hand respected, the early switch off; four typing modes";
const RESTORING_MODES: [&str; 3] = ["forget_ru", "forget_en", "start_wrong"];

#[derive(Clone)]
struct Member {
    text: String,
    question: String,
    mix: String,
}

fn content_checksum(path: &Path) -> Result<String, String> {
    let mut bytes = Vec::new();
    MultiGzDecoder::new(std::fs::File::open(path).map_err(|error| error.to_string())?).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    Ok(hex(&Sha256::digest(&bytes)))
}

fn load_members(path: &Path) -> Result<Vec<Member>, String> {
    let mut members = Vec::new();
    for line in gz_lines(path)? {
        let row: Value = serde_json::from_str(&line).map_err(|error| error.to_string())?;
        // `str(row["question"])`.
        let question = match &row["question"] {
            Value::String(text) => text.clone(),
            other @ (Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) |
Value::Object(_)) => other.to_string(),
        };
        members.push(Member { text: row["text"].as_str().unwrap_or("").to_owned(), question: hex_digest(&format!("{DEVCHAT_NAMESPACE}:{question}")), mix: row["mix"].as_str().unwrap_or("").to_owned() });
    }
    Ok(members)
}

fn loss_chance(losses: i64, gains: i64) -> f64 {
    // A one-sided binomial test: the terms are exact as far as a double holds them, which the floor's margin does not reach.
    let total = losses + gains;
    let mut combination = 1.0_f64;
    let mut sum = 0.0;
    for count in 0..=total {
        if count >= losses {
            sum += combination;
        }
        combination = combination * (total - count).to::<f64>() / (count + 1).to::<f64>();
    }
    sum / 2_f64.powi(total.to::<i32>())
}

type Table = BTreeMap<String, (i64, i64, i64)>;

fn corruptions(table: &Table) -> i64 {
    let row = &table["correct"];
    row.0 - row.1
}

fn net(table: &Table) -> i64 {
    RESTORING_MODES.iter().map(|mode| table[*mode].1).sum::<i64>() - corruptions(table)
}

/// `safe_evaluate`: a message in a mode typed right or wrong, not typed at all, or failed on by the engine.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Skipped,
    Typed(bool),
    Failed,
}

/// A pair's result per message and mode.
type Rows = Vec<(usize, String, Outcome)>;

fn typed_rows(bench: &Bench, model: &Path, texts: &[String], workers: usize) -> Result<Rows, String> {
    let mut jobs = Vec::new();
    for (index, text) in texts.iter().enumerate() {
        let choice = draw(&format!("{DEVCHAT_SEED}:choice:{text}"));
        for mode in TYPING_MODES {
            jobs.push(Job { index, text: text.clone(), mode: mode.to_owned(), choice, prefix: String::new() });
        }
    }
    let make = |worker: usize| bench.typist(model, worker, Vec::new(), false);
    let results = run_jobs(&jobs, workers, &make, &|typist: &mut Typist, job: &Job| match typist.evaluate(job) {
        Ok(Some(entry)) => Outcome::Typed(entry.2),
        Ok(None) => Outcome::Skipped,
        // What went wrong counts by its kind alone, never with the text it held.
        Err(_) => Outcome::Failed,
    })?;
    Ok(jobs.iter().zip(results).map(|(job, result)| (job.index, job.mode.clone(), result)).collect())
}

fn counts(rows: &Rows, chosen: &[Member]) -> Table {
    let mut result = Table::new();
    for (index, mode, outcome) in rows {
        // One the mode does not apply to counts nowhere; one a pair could not type counts as an error.
        if *outcome == Outcome::Skipped {
            continue;
        }
        for name in [mode.clone(), format!("{mode}:{}", chosen[*index].mix)] {
            let entry = result.entry(name).or_insert((0, 0, 0));
            entry.0 += 1;
            entry.1 += i64::from(*outcome == Outcome::Typed(true));
            entry.2 += i64::from(*outcome == Outcome::Failed);
        }
    }
    result
}

fn table_value(table: &Table) -> Value {
    let map: Map<String, Value> = table.iter().map(|(name, (n, ok, errors))| (name.clone(), json!({"n": n, "ok": ok, "errors": errors}))).collect();
    Value::Object(map)
}

/// `evaluate`'s reading of the chosen texts, the three pairs typing them and the floors on what they typed: the results,
/// the discordance and the floors, or what stopped it by its kind alone, never with a word of what it was reading.
fn typed_floors(request: &DevchatRequest<'_>, files: &[(PathBuf, PathBuf, String)], chosen: &[Member]) -> Result<[Value; 3], &'static str> {
    // The texts are read only after the claim; a member's text is its hash in the members file.
    let wanted: HashSet<&str> = chosen.iter().map(|member| member.text.as_str()).collect();
    let mut found: HashMap<String, String> = HashMap::new();
    for (messages, _, _) in files {
        for line in gz_lines(messages).map_err(|_| "ReadError")? {
            let row: Value = serde_json::from_str(&line).map_err(|_| "ReadError")?;
            let text = row["text"].as_str().unwrap_or("").to_owned();
            let key = hex_digest(&text);
            if wanted.contains(key.as_str()) {
                found.insert(key, text);
            }
        }
    }
    if found.len() != wanted.len() {
        // `Refusal("the messages file lacks members the draw chose")`.
        return Err("Refusal");
    }
    let texts: Vec<String> = chosen.iter().map(|member| found[&member.text].clone()).collect();
    let bench = Bench::new(request.resource_dir).map_err(|_| "TypingError")?;
    let candidate_rows = typed_rows(&bench, request.candidate, &texts, request.workers).map_err(|_| "TypingError")?;
    let released_rows = typed_rows(&bench, request.released, &texts, request.workers).map_err(|_| "TypingError")?;
    let baseline_rows = typed_rows(&bench, request.baseline, &texts, request.workers).map_err(|_| "TypingError")?;
    let tables = [counts(&candidate_rows, chosen), counts(&released_rows, chosen), counts(&baseline_rows, chosen)];
    if tables.iter().any(|table| TYPING_MODES.iter().any(|mode| !table.contains_key(*mode))) {
        // A mode no chosen message is typed in has no count to compare: the floors of the Python raise a KeyError.
        return Err("KeyError");
    }
    let theirs: HashMap<(usize, &str), Outcome> = released_rows.iter().map(|(index, mode, outcome)| ((*index, mode.as_str()), *outcome)).collect();
    let mut paired: BTreeMap<&str, (i64, i64)> = TYPING_MODES.iter().map(|mode| (*mode, (0, 0))).collect();
    let right = Outcome::Typed(true);
    for (index, mode, outcome) in &candidate_rows {
        let other = theirs[&(*index, mode.as_str())];
        let entry = paired.get_mut(mode.as_str()).expect("a mode");
        entry.0 += i64::from(*outcome == right && other != right);
        entry.1 += i64::from(*outcome != right && other == right);
    }
    let errors: i64 = tables.iter().flat_map(|table| TYPING_MODES.iter().map(|mode| table[*mode].2)).sum();
    let (candidate_table, released_table, baseline_table) = (&tables[0], &tables[1], &tables[2]);
    let restored = |table: &Table| RESTORING_MODES.iter().map(|mode| table[*mode].1).sum::<i64>();
    let floors = json!({
        "every_message_typed": errors == 0,
        "f1_better_than_baseline": net(candidate_table) >= net(baseline_table) + DEVCHAT_MIN_NET_MARGIN && corruptions(candidate_table) <= corruptions(baseline_table),
        "f3_not_worse_than_released": RESTORING_MODES.iter().all(|mode| loss_chance(paired[mode].1, paired[mode].0) >= DEVCHAT_SIGNIFICANCE) && restored(candidate_table) >= restored(released_table) && corruptions(candidate_table) <= corruptions(released_table),
    });
    let results = json!({"candidate": table_value(candidate_table), "released": table_value(released_table), "baseline": table_value(baseline_table)});
    let paired_value: Map<String, Value> = paired.iter().map(|(mode, (gains, losses))| ((*mode).to_owned(), json!({"gains": gains, "losses": losses}))).collect();
    Ok([results, Value::Object(paired_value), floors])
}

pub struct DevchatRequest<'a> {
    pub root: &'a Path,
    pub resource_dir: &'a Path,
    pub split: &'a str,
    pub directory: &'a Path,
    pub candidate: &'a Path,
    pub released: &'a Path,
    pub baseline: &'a Path,
    pub output: &'a Path,
    pub ledger: &'a Path,
    pub sequences_ledger: &'a Path,
    pub workers: usize,
}

fn private_record(path: &Path, value: &Value, exclusive: bool) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::create_dir_all(path.parent().ok_or("a record outside a folder")?).map_err(|error| error.to_string())?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).mode(0o600);
    if exclusive {
        options.create_new(true);
    } else {
        options.create(true).truncate(true);
    }
    let mut file = options.open(path).map_err(|error| error.to_string())?;
    let format = JsonOptions { ensure_ascii: false, sort_keys: true, indent: Some(ks_text::Indent::Spaces(1)), ..JsonOptions::default() };
    file.write_all((json_dumps(value, &format) + "\n").as_bytes()).map_err(|error| error.to_string())
}

fn read_anything(access: &Value) -> bool {
    access["test_membership"]["messages"].as_array().is_some_and(|items| !items.is_empty())
}

/// `claim`: a read of the test recorded before any of its text is opened. The ledger is read again under its lock, so
/// that of two reads that race only one claims: a read that repeats the candidate's or overlaps an earlier one is refused.
fn claim(ledger: &Path, identity: &Value, candidate: &str) -> Result<String, String> {
    let key = hex(&Sha256::digest(json_dumps(&identity["test_membership"], &JsonOptions { sort_keys: true, ..JsonOptions::default() }).as_bytes()));
    std::fs::create_dir_all(ledger).map_err(|error| error.to_string())?;
    let lock = ledger.join(".access-lock");
    std::fs::OpenOptions::new().write(true).create_new(true).open(&lock).map_err(|_| "the dev-chat test ledger is locked; no test was read".to_owned())?;
    let claimed = claim_locked(ledger, identity, candidate, &key);
    let _ = std::fs::remove_file(&lock);
    claimed.map(|()| key)
}

/// The part of `claim` that runs while the lock is held.
fn claim_locked(ledger: &Path, identity: &Value, candidate: &str, key: &str) -> Result<(), String> {
    for (_, prior) in ledger_records(ledger, ".access.json")? {
        if prior["pairs"]["candidate"]["sha256"].as_str() == Some(candidate) && read_anything(&prior) {
            return Err("this candidate read the dev-chat test already".to_owned());
        }
        for field in ["messages", "questions"] {
            let earlier: HashSet<String> = string_list(&prior["test_membership"][field]).into_iter().collect();
            if string_list(&identity["test_membership"][field]).iter().any(|entry| earlier.contains(entry)) {
                return Err("the draw overlaps messages or questions an earlier read took".to_owned());
            }
        }
    }
    private_record(&ledger.join(format!("{key}.access.json")), identity, true)
}

fn ledger_records(ledger: &Path, suffix: &str) -> Result<Vec<(PathBuf, Value)>, String> {
    let mut records = Vec::new();
    if ledger.is_dir() {
        let mut paths: Vec<PathBuf> = std::fs::read_dir(ledger).map_err(|error| error.to_string())?.filter_map(|entry| entry.ok().map(|entry| entry.path())).collect();
        paths.retain(|path| path.to_string_lossy().ends_with(suffix));
        paths.sort();
        for path in paths {
            let value = crate::corpus::read_json(&path)?;
            records.push((path, value));
        }
    }
    Ok(records)
}

/// `evaluate_devchat_test.evaluate`: development read freely, the test once, each with its floors.
pub fn devchat(request: &DevchatRequest<'_>) -> Result<Value, String> {
    let split = request.split;
    let directory = request.directory;
    let batches = crate::corpus::read_json(&request.root.join("model/context_v3/devchat-test-batches.json"))?;
    let first_pin = batches[if split == "test" { "test_content_sha256" } else { "development_content_sha256" }].as_str().unwrap_or("").to_owned();
    let mut files: Vec<(PathBuf, PathBuf, String)> = vec![(directory.join(format!("devchat-{split}.messages.jsonl.gz")), directory.join(format!("devchat-{split}.members.jsonl.gz")), first_pin)];
    if split == "test" {
        for batch in batches["batches"].as_array().into_iter().flatten() {
            let name = batch["name"].as_str().unwrap_or("");
            files.push((directory.join(format!("{name}.messages.jsonl.gz")), directory.join(format!("{name}.members.jsonl.gz")), batch["content_sha256"].as_str().unwrap_or("").to_owned()));
        }
    }
    for (messages, _, pin) in &files {
        if &content_checksum(messages)? != pin {
            return Err(format!("the {split} messages differ from the pinned ones"));
        }
    }
    let pair_of = |path: &Path| -> Result<Value, String> {
        Ok(json!({"path": path.display().to_string(), "sha256": crate::corpus::checksum(path)?, "model_version": crate::corpus::read_json(path)?["version"]}))
    };
    let pairs = json!({"candidate": pair_of(request.candidate)?, "released": pair_of(request.released)?, "baseline": pair_of(request.baseline)?});
    let candidate = pairs["candidate"]["sha256"].as_str().unwrap_or("").to_owned();
    let development = request.ledger.join(format!("{candidate}.dev.json"));
    let same_models = |recorded: &Value| -> bool {
        ["candidate", "released", "baseline"].iter().all(|name| recorded[*name]["sha256"] == pairs[*name]["sha256"] && recorded[*name]["model_version"] == pairs[*name]["model_version"])
    };
    let mut pins = crate::fit::provenance(request.root, &request.root.join("model/context_v3/recipe.json"))?;
    // Every message file of the folder, whichever split is read: a development result and the test it allows pin the same.
    let mut folder: Vec<PathBuf> = std::fs::read_dir(directory).map_err(|error| error.to_string())?.flatten().map(|entry| entry.path()).filter(|path| {
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        name.starts_with("devchat-") && name.ends_with(".jsonl.gz")
    }).collect();
    folder.sort();
    for path in folder {
        pins.insert(format!("messages/{}", path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()), content_checksum(&path)?);
    }
    pins.insert("model/context_v3/devchat-test-batches.json".to_owned(), crate::corpus::checksum(&request.root.join("model/context_v3/devchat-test-batches.json"))?);
    let pins = serde_json::to_value(pins).map_err(|error| error.to_string())?;
    let accesses = ledger_records(request.ledger, ".access.json")?;
    if split == "test" {
        for (path, prior) in &accesses {
            if prior["pairs"]["candidate"]["sha256"].as_str() == Some(candidate.as_str()) && read_anything(prior) {
                let result = PathBuf::from(path.to_string_lossy().replace(".access.json", ".result.json"));
                if result.is_file() {
                    return crate::corpus::read_json(&result);
                }
                return Err("this candidate read the dev-chat test already and no result was recorded".to_owned());
            }
        }
        if !development.is_file() {
            return Err("no development result of this candidate is recorded; the test is not read".to_owned());
        }
        let recorded = crate::corpus::read_json(&development)?;
        if !recorded["floors"].as_object().is_some_and(|table| table.values().all(|value| value == &json!(true))) {
            return Err("the candidate's development result holds no gain; the test is not read".to_owned());
        }
        if recorded["runtime"] != pins || !same_models(&recorded["pairs"]) {
            return Err("the runtime or the pairs differ from the candidate's development result".to_owned());
        }
        let passed = ledger_records(request.sequences_ledger, ".access.json")?.iter().any(|(path, identity)| {
            let outcome = PathBuf::from(path.to_string_lossy().replace(".access.json", ".outcome.json"));
            identity["artifact_sha256"].as_str() == Some(candidate.as_str())
                && crate::corpus::read_json(&outcome).is_ok_and(|report| report["split"] == json!("test") && report["promotion_passed"] == json!(true))
        });
        if !passed {
            return Err("the candidate passed no Tatoeba test; the dev-chat test is not read".to_owned());
        }
    }
    // The members of every file, a text a batch repeats counted once.
    let mut seen: HashSet<String> = HashSet::new();
    let mut members: Vec<Member> = Vec::new();
    for (_, path, _) in &files {
        for member in load_members(path)? {
            if seen.insert(member.text.clone()) {
                members.push(member);
            }
        }
    }
    let (mut taken_messages, mut taken_questions): (HashSet<String>, HashSet<String>) = (HashSet::new(), HashSet::new());
    if split == "test" {
        for (_, prior) in &accesses {
            taken_messages.extend(string_list(&prior["test_membership"]["messages"]));
            taken_questions.extend(string_list(&prior["test_membership"]["questions"]));
        }
    }
    let mut ordered: Vec<(String, &Member)> = members.iter().filter(|member| !taken_messages.contains(&member.text) && !taken_questions.contains(&member.question)).map(|member| (hex_digest(&format!("{DEVCHAT_SEED}:{}", member.text)), member)).collect();
    ordered.sort_by(|a, b| (&a.0, &a.1.text).cmp(&(&b.0, &b.1.text)));
    let test_count = batches["test_count"].as_u64().unwrap_or(0).to::<usize>();
    let limit = if split == "test" { test_count } else { usize::MAX };
    let mixed: Vec<Member> = ordered.iter().filter(|entry| entry.1.mix == "mixed").take(limit).map(|entry| entry.1.clone()).collect();
    let share = (mixed.len().to::<f64>() * MIXED_EVALUATE_RUSSIAN_SHARE).to::<usize>();
    let russian: Vec<Member> = ordered.iter().filter(|entry| entry.1.mix == "russian").take(share).map(|entry| entry.1.clone()).collect();
    let mixed_count = mixed.len();
    let chosen: Vec<Member> = mixed.into_iter().chain(russian).collect();
    if split == "test" && mixed_count < test_count {
        return Err("the test holds fewer fresh messages than a read takes; a new batch is needed and no test was read".to_owned());
    }
    let membership_messages: BTreeSet<&str> = chosen.iter().map(|member| member.text.as_str()).collect();
    let membership_questions: BTreeSet<&str> = chosen.iter().map(|member| member.question.as_str()).collect();
    let membership = json!({"messages": membership_messages, "questions": membership_questions});
    let batches: Vec<&String> = files.iter().map(|entry| &entry.2).collect();
    let identity = json!({"protocol": "devchat-messages-v1", "split": split, "messages_sha256": if batches.len() == 1 { json!(batches[0]) } else { json!(batches) }, "seed": DEVCHAT_SEED, "russian_share": MIXED_EVALUATE_RUSSIAN_SHARE, "pairs": pairs, "bench": BENCH, "test_membership": membership});
    let key = if split == "test" {
        claim(request.ledger, &identity, &candidate)?
    } else {
        candidate.clone()
    };
    let mut result = json!({"split": split, "access": key, "messages": chosen.len(), "pairs": pairs, "bench": BENCH});
    // Whatever stops the read after the claim is recorded as its kind alone, so that no read of the test goes unrecorded.
    match typed_floors(request, &files, &chosen) {
        Ok([results, discordance, floors]) => {
            result["results"] = results;
            result["discordance"] = discordance;
            result["floors"] = floors;
        }
        Err(kind) => {
            result["error"] = json!(kind);
            result["floors"] = json!({"every_message_typed": false});
        }
    }
    if split == "dev" {
        result["runtime"] = pins;
        private_record(&development, &result, false)?;
    } else {
        private_record(&request.ledger.join(format!("{key}.result.json")), &result, true)?;
    }
    private_record(request.output, &result, false)?;
    Ok(result)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    fn kinds(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn the_letters_beside_a_key_are_the_letters_of_its_row() {
        assert_eq!(neighbours('s'), ['a', 'd']);
        assert_eq!(neighbours('q'), ['w']);
        assert_eq!(neighbours('S'), ['A', 'D']);
        // A sign beside a letter is no neighbour, and a key of signs has none.
        assert_eq!(neighbours('p'), ['o']);
        assert!(neighbours(']').is_empty());
        assert_eq!(neighbours('ъ'), ['х']);
        assert!(neighbours('1').is_empty());
        assert!(neighbours('ё').is_empty());
    }

    #[test]
    fn a_typo_is_of_the_kind_drawn_and_falls_back_where_it_cannot_be() {
        // Too few letters: the token as it is.
        assert_eq!(typo("12", "seed", &kinds(&["missed"]), 1), "12");
        assert_eq!(typo("дом", "seed", &kinds(&["doubled"]), 4), "дом");
        // A letter left out only of a word long enough.
        assert_eq!(pystr::len(&typo("слово", "seed", &kinds(&["missed"]), 1)), 4);
        // Three letters are too few to lose one: a neighbour stands in instead.
        let three = typo("дом", "seed", &kinds(&["missed"]), 1);
        assert_eq!(pystr::len(&three), 3);
        assert_ne!(three, "дом");
        // A swap needs a letter after the letter drawn; otherwise a neighbour stands in.
        assert_eq!(typo("a1", "seed", &kinds(&["swapped"]), 1), "s1");
        assert_eq!(typo("a", "seed", &kinds(&["swapped"]), 1), "s");
        let swaps: BTreeSet<String> = (0..32).map(|seed| typo("ab", &seed.to_string(), &kinds(&["swapped"]), 1)).collect();
        assert!(swaps.contains("ba") && swaps.len() > 1, "{swaps:?}");
        assert_eq!(pystr::len(&typo("дом", "seed", &kinds(&["doubled"]), 1)), 4);
        // An extra letter is a neighbour; a key with no neighbours doubles instead.
        assert_eq!(typo("a", "seed", &kinds(&["extra"]), 1), "as");
        assert_eq!(typo("ё", "seed", &kinds(&["extra"]), 1), "ёё");
        assert_eq!(typo("ё", "seed", &kinds(&["neighbour"]), 1), "ёё");
        assert_eq!(typo("a", "seed", &kinds(&["neighbour"]), 1), "s");
    }

    #[test]
    fn typos_fall_on_the_words_their_rate_draws() {
        let all = kinds(&["doubled"]);
        assert_eq!(with_typos("один два", 0.0, "seed", &all, 1), "один два");
        let every = with_typos("один два", 1.0, "seed", &all, 1);
        assert_eq!(pystr::len(&every), pystr::len("один два") + 2);
    }

    #[test]
    fn a_plan_needs_a_word_of_a_layout_and_a_change_to_make() {
        assert!(plan(&["123", "456"], "correct", 0).is_none());
        // No word of the layout a slip lands in.
        assert!(plan(&["hello", "world"], SLIP_MODE, 0).is_none());
        // No one-letter word to slip on, or none with a word after it.
        assert!(plan(&["привет", "мир"], LETTER_SLIP_MODE, 0).is_none());
        assert!(plan(&["привет", "и"], LETTER_SLIP_BACK_MODE, 0).is_none());
        let (start, steps) = plan(&["привет", "и", "мир"], LETTER_SLIP_BACK_MODE, 0).expect("a plan");
        assert_eq!(start, 1);
        assert_eq!(steps[1], ("и".to_owned(), 1, Some(0), true));
        assert_eq!(steps[2], ("мир".to_owned(), 1, Some(1), false));
        // A layout to forget needs a change to it.
        assert!(plan(&["привет", "мир"], "forget_en", 0).is_none());
        let (start, _) = plan(&["42", "привет"], "start_wrong", 0).expect("a plan");
        assert_eq!(start, 0);
    }

    #[test]
    fn a_gap_is_cut_only_inside_a_word_of_letters() {
        assert!(edit_point("да нет ok 1234").is_none());
        let (start, length, group) = edit_point("про длинные слова").expect("a gap");
        assert!(length >= 1 && start > 0 && group == 1);
    }

    #[test]
    fn a_loss_is_as_likely_as_the_binomial_says() {
        assert!((loss_chance(0, 0) - 1.0).abs() < 1e-12);
        assert!((loss_chance(1, 0) - 0.5).abs() < 1e-12);
        assert!((loss_chance(5, 0) - 1.0 / 32.0).abs() < 1e-12);
        assert!((loss_chance(0, 3) - 1.0).abs() < 1e-12);
    }

    fn repository() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// A fresh folder of this test program.
    fn folder_of(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("ks-mixed-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("a folder");
        folder
    }

    fn gz_file(path: &Path, lines: &[String]) {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        for line in lines {
            encoder.write_all(line.as_bytes()).expect("in memory");
            encoder.write_all(b"\n").expect("in memory");
        }
        std::fs::write(path, encoder.finish().expect("in memory")).expect("a file");
    }

    fn ledger_of(name: &str, records: &[Value]) -> PathBuf {
        let folder = folder_of(&format!("claim-{name}"));
        for (index, record) in records.iter().enumerate() {
            std::fs::write(folder.join(format!("{index}.access.json")), record.to_string()).expect("a record");
        }
        folder
    }

    fn earlier(candidate: &str, messages: &[&str], questions: &[&str]) -> Value {
        json!({"pairs": {"candidate": {"sha256": candidate}}, "test_membership": {"messages": messages, "questions": questions}})
    }

    #[test]
    fn a_claim_reads_the_ledger_again_under_its_lock() {
        let identity = earlier("mine", &["m1", "m2"], &["q1", "q2"]);
        let refused = |name: &str, records: &[Value], words: &str| {
            let ledger = ledger_of(name, records);
            let said = claim(&ledger, &identity, "mine").expect_err(words);
            assert!(said.contains(words), "{words}: {said}");
            assert!(!ledger.join(".access-lock").exists());
            assert_eq!(ledger_records(&ledger, ".access.json").expect("the ledger").len(), records.len());
            let _ = std::fs::remove_dir_all(&ledger);
        };
        // Records no earlier look at the ledger saw: a read of this candidate, one that took a message, one a question.
        refused("repeat", &[earlier("mine", &["m9"], &["q9"])], "this candidate read the dev-chat test already");
        refused("messages", &[earlier("other", &[], &[]), earlier("other", &["m2"], &["q9"])], "the draw overlaps");
        refused("questions", &[earlier("other", &["m9"], &["q1"])], "the draw overlaps");
        let broken = ledger_of("broken", &[]);
        std::fs::write(broken.join("0.access.json"), b"not json").expect("a record");
        assert!(claim(&broken, &identity, "mine").is_err());
        assert!(!broken.join(".access-lock").exists());
        // Somebody else holds the lock, and it stays theirs.
        let locked = ledger_of("locked", &[]);
        std::fs::write(locked.join(".access-lock"), b"").expect("a lock");
        assert!(claim(&locked, &identity, "mine").expect_err("locked").contains("is locked"));
        assert!(locked.join(".access-lock").exists());
        // A record of this candidate that read nothing, and a read of other messages: the claim is written.
        let clean = ledger_of("clean", &[earlier("mine", &[], &[]), earlier("other", &["m9"], &["q9"])]);
        let key = claim(&clean, &identity, "mine").expect("claimed");
        assert!(clean.join(format!("{key}.access.json")).is_file());
        assert!(!clean.join(".access-lock").exists());
        assert!(claim(&clean, &identity, "mine").expect_err("claimed once").contains("read the dev-chat test already"));
        for folder in [broken, locked, clean] {
            let _ = std::fs::remove_dir_all(folder);
        }
    }

    #[test]
    fn a_pool_above_gives_only_lines_of_its_kind_and_mix() {
        let folder = folder_of("above");
        let messages = folder.join("messages.jsonl.gz");
        gz_file(&messages, &[json!({"kind": "comment", "text": "привет мир", "mix": "mixed", "qid": 1}).to_string()]);
        let pool = [
            json!({"kind": "note", "text": "строка сверху", "mix": "russian", "qid": 5}),
            json!({"kind": "other", "text": "другой вид", "mix": "russian", "qid": 6}),
            json!({"kind": "note", "text": "другая смесь", "mix": "mixed", "qid": 7}),
        ];
        gz_file(&folder.join("above.jsonl.gz"), &pool.iter().map(Value::to_string).collect::<Vec<_>>());
        let code = folder.join("code.txt.gz");
        gz_file(&code, &["let x = 1;".to_owned()]);
        let recipe = json!({"seed": "s", "lines_above": {"messages": "above.jsonl.gz", "kinds": ["note"], "mix": "russian", "seed": "a", "count": 5}});
        assert_eq!(lines_above(&messages, &recipe).expect("the pool"), ["строка сверху"]);
        let (above, code_lines) = recipe_lines_above(&messages, &recipe, Some(&code)).expect("the pool");
        assert_eq!(above, [("строка сверху".to_owned(), 5)]);
        // Code stands above a message only in a recipe that puts lines above it.
        assert_eq!(code_lines, 0);
        let mut drawn = recipe.clone();
        drawn["prefix_rate"] = json!(0.5);
        assert_eq!(recipe_lines_above(&messages, &drawn, Some(&code)).expect("the pool").1, 1);
        assert_eq!(recipe_lines_above(&messages, &json!({}), None).expect("no pool"), (Vec::new(), 0));
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn a_reader_reports_the_field_and_is_always_there() {
        let reader = Reader { backend: Arc::new(Backend::new("Telegram", 0, "ab".chars().collect())), after: true };
        assert_eq!(reader.status(), "available");
        assert_eq!(reader.read("Telegram", 0).expect("a field").source, "uia");
    }

    #[test]
    fn a_bench_without_its_orthotactic_model_says_why() {
        let real = repository().join("resources");
        let resources = folder_of("no-ortho");
        for entry in std::fs::read_dir(&real).expect("the resources").flatten() {
            if entry.file_name() != "models" {
                std::os::unix::fs::symlink(entry.path(), resources.join(entry.file_name())).expect("a link");
            }
        }
        std::fs::create_dir_all(resources.join("models")).expect("a folder");
        for entry in std::fs::read_dir(real.join("models")).expect("the models").flatten() {
            if entry.file_name() != "ortho_v1.json" {
                std::os::unix::fs::symlink(entry.path(), resources.join("models").join(entry.file_name())).expect("a link");
            }
        }
        let bench = Bench::new(&resources).expect("a bench");
        assert!(bench.shared.1.starts_with("unavailable"), "{}", bench.shared.1);
        let _ = std::fs::remove_dir_all(resources);
    }

    #[test]
    fn a_question_is_captured_only_of_a_word_that_was_typed() {
        let root = repository();
        let bench = Bench::new(&root.join("resources")).expect("a bench");
        let typist = bench.typist(&root.join("resources/models/context_policy_v1.json"), 0, fields_of(2).expect("the fields"), true).expect("a typist");
        let answer = typist.policy.as_ref().and_then(|policy| policy.model.as_ref()).and_then(|model| model.answer.clone()).expect("the spy");
        let mut field = FieldContext::new("Telegram", "1");
        "привет ".clone_into(&mut field.before);
        let asked = |original: &str, alternative: &str, inside: bool| {
            let mut item = ContextEvidence::new(original, alternative, 0, field.clone());
            item.inside = inside;
            item
        };
        typist.spy.lock().expect("the spy").typed.push(("ghbdtn".to_owned(), 0, 1));
        // No word, and a word typed in the layout that was not meant.
        let _ = answer(&asked("", "", false));
        assert!(typist.spy.lock().expect("the spy").rows.is_empty());
        let _ = answer(&asked("ghbdtn", "привет", false));
        let rows = std::mem::take(&mut typist.spy.lock().expect("the spy").rows);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].as_array().and_then(|row| row.last()).and_then(Value::as_str), Some("convert"));
        // While a word is edited only the questions about its inside are kept.
        typist.spy.lock().expect("the spy").edit = true;
        let _ = answer(&asked("ghbdtn", "привет", false));
        let _ = answer(&asked("ghbdtn", "привет", true));
        assert_eq!(typist.spy.lock().expect("the spy").rows.len(), 1);
        let _ = std::fs::remove_dir_all(&typist.folder);
    }

    #[test]
    fn a_message_the_engine_fails_on_is_an_error_and_the_next_one_is_typed() {
        let root = repository();
        let bench = Bench::new(&root.join("resources")).expect("a bench");
        let mut typist = bench.typist(&root.join("resources/models/context_policy_v1.json"), 7, Vec::new(), false).expect("a typist");
        let job = |text: &str, mode: &str| Job { index: 0, text: text.to_owned(), mode: mode.to_owned(), choice: 0, prefix: String::new() };
        // No word to cut letters out of, and letters no key of the Russian layout types: nothing is edited.
        assert_eq!(typist.evaluate(&job("да нет", EDIT_MODE)), Ok(None));
        assert_eq!(typist.evaluate(&job("кіііііі", EDIT_MODE)), Ok(None));
        // A learning file the engine refuses to read: no engine is made, to type or to edit.
        std::fs::write(typist.folder.join("learning.json"), r#"{"rules": {"²:слово": {}}}"#).expect("a file");
        assert!(typist.evaluate(&job("привет мир", "start_wrong")).is_err());
        assert!(typist.evaluate(&job("привет большой мир", EDIT_MODE)).is_err());
        std::fs::remove_file(typist.folder.join("learning.json")).expect("the file goes");
        assert!(typist.evaluate(&job("привет мир", "start_wrong")).is_ok_and(|row| row.is_some()));
        let _ = std::fs::remove_dir_all(&typist.folder);
    }
}
