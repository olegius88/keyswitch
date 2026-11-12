//! The text stays the user's. A person types sentences in two languages on a model of a desktop that behaves as the real
//! ones do: every key goes to the window as it is pressed, the engine learns of it through a queue and may be several keys
//! behind, a correction is typed over what the window already shows, keys pressed while it is typed are kept back and
//! typed after it, an Enter waits for its correction. The window remembers which key typed each character, so every
//! correction is checked as it lands: it may erase only the keys it types again, in the same order. What else must hold:
//! nothing is typed into a password field, nothing stays held once the person stops, and the engine ends up knowing the
//! layout the keyboard is in.
//!
//! The hook of the model answers a key as the Windows one does (`ks-win`, `answer_native` and `release_input`): what it
//! keeps back while a correction is typed or an Enter waits, which key-ups pass at once, and where a second Enter among
//! the keys typed again stops them. With `holds` off it is the hook of X11, which only watches: every key has reached the
//! window by the time the engine hears of it.
#![cfg(any(not(debug_assertions), coverage))]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ks_backend::constants::{ALT_MASK, CONTROL_MASK, SHIFT_MASK};
use ks_backend::{BackendError, BackendProbe, FieldContext, FieldReader, FieldRole, FocusInfo, Injection, InputBackend, KeyDisposition, KeyEvent, KeyFilter, KeyListener};
use ks_engine::engine::{CorrectionPlan, EngineCore, Strokes};
use ks_replay::harness::{Bundle, FakeClock, KEYS, PAUSE_KEYCODE};

/// An Enter or Tab kept back is answered well within this once every key is up.
const SLOW_ANSWER_SECONDS: f64 = 1.0;
const F5_KEYCODE: u32 = 171;
const SHIFT_KEYCODE: u32 = 150;
const CONTROL_KEYCODE: u32 = 137;
const ALT_KEYCODE: u32 = 164;
const BACKSPACE_KEYCODE: u32 = 122;
const RETURN_KEYCODE: u32 = 136;
const TAB_KEYCODE: u32 = 123;
const ESCAPE_KEYCODE: u32 = 109;
const LEFT_KEYCODE: u32 = 113;
const RIGHT_KEYCODE: u32 = 114;
const HOME_KEYCODE: u32 = 110;
const END_KEYCODE: u32 = 115;
const DELETE_KEYCODE: u32 = 119;
const MILLISECONDS_PER_SECOND: f64 = 1000.0;
/// How long the person stays away at the end of a walk: longer than every timer of the engine.
const SETTLE_SECONDS: f64 = 8.0;
const TIMER_STEP_SECONDS: f64 = 0.05;
const JOURNAL_LINES: usize = 60;
/// A typo correction may change this many keys of a word (two letters that changed places are two).
const TYPO_KEYS: usize = 2;

/// A small generator with a fixed start: the walk must be the same on every machine.
struct Dice(u64);

impl Dice {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, limit: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(limit).expect("small")).expect("small")
    }

    fn of<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    fn one_in(&mut self, odds: usize) -> bool {
        self.below(odds) == 0
    }

    /// Seconds from `least` to `most`, in steps of a millisecond.
    fn seconds(&mut self, least: f64, most: f64) -> f64 {
        let steps = ((most - least) * MILLISECONDS_PER_SECOND).round().max(1.0);
        let step = f64::from(u32::try_from(self.next() % 1_000_000).expect("small"));
        least + (step % steps) / MILLISECONDS_PER_SECOND
    }
}

/// The key that typed a character: its place on the keyboard and whether Shift was down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Origin {
    keycode: u32,
    shift: bool,
}

#[derive(Clone, Copy, Debug)]
struct Cell {
    character: char,
    /// None for text the person did not type key by key: it was pasted.
    origin: Option<Origin>,
}

/// A key as the keyboard sends it, before any layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Raw {
    pressed: bool,
    keycode: u32,
    /// The name of a key that types no text; empty for a text key.
    name: &'static str,
}

/// What a text key types in each layout, by its place and Shift.
fn characters_of(keycode: u32, shift: bool) -> Option<[char; 2]> {
    KEYS.iter().find(|key| key.keycode == keycode && key.shift == shift).map(|key| key.characters)
}

/// The key that types `character` in `layout`.
fn key_for(character: char, layout: usize) -> Option<Origin> {
    KEYS.iter().find(|key| key.characters[layout] == character).map(|key| Origin { keycode: key.keycode, shift: key.shift })
}

/// Everything the person does, in order.
#[derive(Clone, Copy, Debug)]
enum Act {
    Key(Raw),
    /// The person stops for so long.
    Wait(f64),
    /// A click in the field: the caret goes to the end or somewhere into the text.
    Click,
    /// The person switches the layout with the system's own keys.
    Layout,
    /// Another window comes to the front, with an empty field.
    Window,
    /// Another window with a password field.
    Password,
    /// Control+V: the window takes this text in at the caret.
    Paste(&'static str),
    /// From now on the reader shows the field this many characters behind the keys.
    Lag(usize),
    /// The next correction fails as it does when the window goes away under it: nothing is typed.
    Fail,
}

struct DeskState {
    cells: Vec<Cell>,
    caret: usize,
    password: bool,
    group: i32,
    window: i64,
    /// The hook can keep keys from the window (Windows, macOS, Wayland); false is X11, which only watches.
    holds: bool,
    holding: bool,
    held: VecDeque<Raw>,
    deferred: Option<Raw>,
    /// The keys that went down past the hook and have not come up.
    down: BTreeSet<u32>,
    /// The keys the hook kept from the window: their releases are kept too.
    consumed: BTreeSet<u32>,
    /// The keys that were down when an Enter was kept back: their releases belong before it.
    action_prior: BTreeSet<u32>,
    shift: bool,
    control: bool,
    alt: bool,
    /// What the person does next. A correction takes the next keys from here: they are pressed while it is typed.
    acts: VecDeque<Act>,
    dice: Dice,
    clipboard: &'static str,
    /// The engine reads the field, so it knows a password field for one.
    read: bool,
    typos: bool,
    reader_lag: usize,
    /// The field gives no answer at all, as UI Automation does at times.
    reader_silent: bool,
    failing: bool,
    /// Asking for the program in front stops whoever asks, as an error nobody foresaw does.
    panicking: bool,
    /// The same in the middle of a correction: the hook holds the keys by then.
    panicking_correction: bool,
    /// The hook cannot pass a kept Enter on or let it go: the call fails, the keys kept back are still typed again.
    failing_actions: bool,
    /// The text of the field each time an Enter reached the window.
    submitted: Vec<String>,
    listener: Option<KeyListener>,
    filter: Option<KeyFilter>,
    clock: FakeClock,
    violations: Vec<String>,
    journal: VecDeque<String>,
    /// The journal keeps every key and is not cut: one walk looked at closely.
    verbose: bool,
    injections: usize,
    typo_corrections: usize,
    deferred_actions: usize,
    dropped_actions: usize,
    held_keys: usize,
    replayed_keys: usize,
    rushed_keys: usize,
    /// The program in front.
    application: &'static str,
    /// A key the hook reports in a way no correction can type again.
    untypable: Option<u32>,
    /// When the Enter or Tab now kept back was kept back.
    deferred_at: f64,
}

impl DeskState {
    fn note(&mut self, line: String) {
        if self.journal.len() == JOURNAL_LINES && !self.verbose {
            self.journal.pop_front();
        }
        self.journal.push_back(line);
    }

    fn shown(&self) -> String {
        let mut text: String = self.cells[..self.caret].iter().map(|cell| cell.character).collect();
        text.push('|');
        text.extend(self.cells[self.caret..].iter().map(|cell| cell.character));
        text
    }

    fn violation(&mut self, text: String) {
        let line = format!("VIOLATION: {text}");
        self.note(line);
        self.violations.push(text);
    }

    fn stamp(&self) -> i64 {
        let milliseconds = (self.clock.now() * MILLISECONDS_PER_SECOND).round();
        format!("{milliseconds:.0}").parse().expect("a whole number of milliseconds")
    }

    fn another_field(&mut self, password: bool) {
        self.window += 1;
        self.cells.clear();
        self.caret = 0;
        self.password = password;
    }

    /// The event the hook makes of a key, in the layout of this moment, and the character the window gets from it.
    fn event(&self, raw: Raw) -> (KeyEvent, Option<char>) {
        let group = self.group;
        let state = (if self.shift { SHIFT_MASK } else { 0 }) | (if self.control { CONTROL_MASK } else { 0 }) | (if self.alt { ALT_MASK } else { 0 });
        let blank = || vec![String::new(), String::new()];
        let named = |name: &str, character: &str| KeyEvent {
            pressed: raw.pressed,
            keycode: raw.keycode,
            key_name: name.to_owned(),
            character: character.to_owned(),
            characters: blank(),
            group,
            state,
            timestamp: self.stamp(),
            ..KeyEvent::default()
        };
        if !raw.name.is_empty() {
            return (named(raw.name, ""), None);
        }
        let Some(characters) = characters_of(raw.keycode, self.shift) else {
            return (named("VoidSymbol", ""), None);
        };
        if self.control || self.alt {
            // A chord types nothing, and is named by the key's Latin letter in either layout.
            let name = characters[0].to_lowercase().to_string();
            return (named(&name, &name), None);
        }
        let typed = characters[usize::try_from(group).expect("one of two layouts")];
        let event = KeyEvent { characters: characters.iter().map(char::to_string).collect(), ..named(&typed.to_string(), &typed.to_string()) };
        (event, Some(typed))
    }

    /// The window takes a key in.
    fn type_into(&mut self, raw: Raw, typed: Option<char>) {
        if !raw.pressed {
            return;
        }
        match raw.name {
            "Return" => {
                let line = format!("the window takes Enter with «{}»", self.shown());
                self.note(line);
                self.submitted.push(self.cells.iter().map(|cell| cell.character).collect());
                self.cells.clear();
                self.caret = 0;
            }
            "Tab" => {
                let line = format!("the window takes Tab with «{}»: the next field", self.shown());
                self.note(line);
                self.another_field(false);
            }
            "BackSpace" => {
                if self.caret != 0 {
                    self.caret -= 1;
                    self.cells.remove(self.caret);
                }
            }
            "Delete" => {
                if self.caret < self.cells.len() {
                    self.cells.remove(self.caret);
                }
            }
            "Left" => self.caret = self.caret.saturating_sub(1),
            "Right" => self.caret = (self.caret + 1).min(self.cells.len()),
            "Home" => self.caret = 0,
            "End" => self.caret = self.cells.len(),
            "" if self.control && !self.alt && characters_of(raw.keycode, false).is_some_and(|characters| characters[0] == 'v') => {
                let pasted: Vec<Cell> = self.clipboard.chars().map(|character| Cell { character, origin: None }).collect();
                let caret = self.caret;
                self.caret += pasted.len();
                self.cells.splice(caret..caret, pasted);
                let line = format!("pasted, the window shows «{}»", self.shown());
                self.note(line);
            }
            _ => {
                if let Some(character) = typed {
                    let origin = Some(Origin { keycode: raw.keycode, shift: self.shift });
                    self.cells.insert(self.caret, Cell { character, origin });
                    self.caret += 1;
                }
            }
        }
    }

    fn track_modifiers(&mut self, raw: Raw) {
        match raw.name {
            "Shift_L" => self.shift = raw.pressed,
            "Control_L" => self.control = raw.pressed,
            "Alt_L" => self.alt = raw.pressed,
            _ => {}
        }
    }

    /// The keyboard hook, as the Windows one answers a key: a key of the person's, or one held back before and typed
    /// again now.
    fn hook(&mut self, raw: Raw, replayed: bool) {
        if self.holds && !replayed {
            let prior_release = !raw.pressed && self.action_prior.contains(&raw.keycode);
            let action_key = self.deferred.is_some() && self.consumed.contains(&raw.keycode);
            // A key-up of a key never seen going down is no keystroke to keep back.
            let orphan_release = !raw.pressed && !self.down.contains(&raw.keycode) && !self.held.iter().any(|item| item.pressed && item.keycode == raw.keycode);
            if self.holding && !prior_release && !action_key && !orphan_release {
                self.held.push_back(raw);
                self.held_keys += 1;
                if self.verbose {
                    self.note(format!("  kept back: {} {}", if raw.pressed { "down" } else { "up" }, raw.keycode));
                }
                return;
            }
        }
        if raw.pressed {
            self.down.insert(raw.keycode);
        } else {
            self.down.remove(&raw.keycode);
            self.action_prior.remove(&raw.keycode);
        }
        let (mut event, typed) = self.event(raw);
        // A repeat of a key the hook has kept from the window is the hook's alone to answer: the real ones do not tell
        // the engine of it (`repeated_answer` in the Windows, macOS and Wayland backends).
        let repeated_answer = self.holds && self.filter.is_some() && raw.pressed && self.consumed.contains(&raw.keycode);
        let disposition = match (&self.filter, self.holds) {
            (Some(_), true) if !raw.pressed => {
                if self.consumed.remove(&raw.keycode) { KeyDisposition::Consume } else { KeyDisposition::Pass }
            }
            (Some(_), true) if self.consumed.contains(&raw.keycode) => KeyDisposition::Consume,
            (Some(filter), true) => {
                let decision = filter(&event);
                if decision != KeyDisposition::Pass {
                    self.consumed.insert(raw.keycode);
                }
                decision
            }
            _ => KeyDisposition::Pass,
        };
        self.track_modifiers(raw);
        match disposition {
            KeyDisposition::Pass => self.type_into(raw, typed),
            KeyDisposition::Consume => {}
            KeyDisposition::Defer => {
                self.holding = true;
                self.deferred = Some(raw);
                self.action_prior = self.down.clone();
                self.deferred_actions += 1;
                self.deferred_at = self.clock.now();
                event.deferred = true;
                let line = format!("the hook keeps {} back for its correction, the window shows «{}»", raw.name, self.shown());
                self.note(line);
            }
        }
        if self.verbose {
            let line = format!(
                "  key {} {:?} {:?} layout {} {disposition:?}{}; the window shows «{}»",
                if raw.pressed { "down" } else { "up" },
                event.key_name,
                event.character,
                event.group,
                if replayed { " (typed again)" } else { "" },
                self.shown()
            );
            self.note(line);
        }
        if let Some(listener) = self.listener.clone().filter(|_| !repeated_answer) {
            listener(event);
        }
    }

    /// The keys kept back are typed again, in order. One of them may be the next Enter, kept back in its turn: then only
    /// the releases of the keys that were down before it follow, and the rest waits for that Enter.
    fn release_held(&mut self) -> i32 {
        if self.deferred.is_some() {
            return 0;
        }
        let mut count = 0;
        loop {
            let item = if self.deferred.is_some() {
                let Some(index) = self.held.iter().position(|item| !item.pressed && self.action_prior.contains(&item.keycode)) else {
                    break;
                };
                self.held.remove(index).expect("found above")
            } else if let Some(item) = self.held.pop_front() {
                item
            } else {
                break;
            };
            self.replayed_keys += 1;
            self.hook(item, true);
            count += 1;
        }
        if self.deferred.is_none() {
            self.holding = false;
        }
        count
    }

    /// The person goes on typing while a correction is typed: the next keys of the walk are pressed now, and the hook
    /// keeps them back.
    fn rush(&mut self) {
        if !self.holds || !self.dice.one_in(3) {
            return;
        }
        for _ in 0..1 + self.dice.below(5) {
            let Some(Act::Key(raw)) = self.acts.front().copied() else {
                return;
            };
            self.acts.pop_front();
            self.rushed_keys += 1;
            self.hook(raw, false);
        }
    }
}

/// The desktop: the window in front with its text field, the layout, and the hook between the keyboard and both.
struct Desk {
    state: Mutex<DeskState>,
}

impl Desk {
    fn new(mode: Walk, group: i32, acts: Vec<Act>, seed: u64, clock: &FakeClock, verbose: bool) -> Arc<Desk> {
        Arc::new(Desk {
            state: Mutex::new(DeskState {
                cells: Vec::new(),
                caret: 0,
                password: false,
                group,
                window: 1,
                holds: mode.holds,
                holding: false,
                held: VecDeque::new(),
                deferred: None,
                down: BTreeSet::new(),
                consumed: BTreeSet::new(),
                action_prior: BTreeSet::new(),
                shift: false,
                control: false,
                alt: false,
                acts: acts.into(),
                dice: Dice(seed),
                clipboard: "",
                read: mode.reads_field,
                typos: mode.typos,
                reader_lag: 0,
                reader_silent: false,
                failing: false,
                panicking: false,
                panicking_correction: false,
                failing_actions: false,
                submitted: Vec::new(),
                listener: None,
                filter: None,
                clock: clock.clone(),
                violations: Vec::new(),
                journal: VecDeque::new(),
                verbose,
                injections: 0,
                typo_corrections: 0,
                deferred_actions: 0,
                dropped_actions: 0,
                held_keys: 0,
                replayed_keys: 0,
                rushed_keys: 0,
                application: "editor",
                untypable: None,
                deferred_at: 0.0,
            }),
        })
    }

    fn lock(&self) -> MutexGuard<'_, DeskState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn key(&self, raw: Raw) {
        self.lock().hook(raw, false);
    }
}

fn origin_of(stroke: &KeyEvent) -> Origin {
    Origin { keycode: stroke.keycode, shift: stroke.shift() }
}

impl InputBackend for Desk {
    fn start(&self, listener: KeyListener) -> Result<(), BackendError> {
        self.lock().listener = Some(listener);
        Ok(())
    }

    fn set_key_filter(&self, predicate: Option<KeyFilter>) {
        self.lock().filter = predicate;
    }

    fn filters_keys(&self) -> bool {
        self.lock().holds
    }

    fn hold_input(&self) {
        let mut state = self.lock();
        if state.holds {
            state.holding = true;
        }
    }

    fn release_input(&self) -> Result<i32, BackendError> {
        Ok(self.lock().release_held())
    }

    fn complete_action(&self, deliver: bool) -> Result<i32, BackendError> {
        let mut state = self.lock();
        let Some(action) = state.deferred.take() else {
            return Ok(0);
        };
        state.action_prior.clear();
        // The hook holds every key behind an Enter kept back. The engine answers it once the keys are up and its
        // correction is made or given up, and a correction waits for its field for a fraction of a second at most.
        let waited = state.clock.now() - state.deferred_at;
        if waited >= SLOW_ANSWER_SECONDS && state.down.is_empty() {
            let text = format!("{} waited {waited:.2} s for its answer with every key up", action.name);
            state.violation(text);
        }
        if deliver {
            state.type_into(action, None);
        } else {
            state.dropped_actions += 1;
            let line = format!("{} is NOT passed on, the window shows «{}»", action.name, state.shown());
            state.note(line);
        }
        state.release_held();
        if state.failing_actions {
            return Err(BackendError::Failed("the window did not take the key".to_owned()));
        }
        Ok(if deliver { 2 } else { 0 })
    }

    fn stop(&self) {}

    fn close(&self) {}

    fn current_group(&self) -> Result<i32, BackendError> {
        Ok(self.lock().group)
    }

    fn switch_group(&self, group: i32) -> Result<(), BackendError> {
        let mut state = self.lock();
        state.group = group;
        let line = format!("the engine switches the layout to {group}");
        state.note(line);
        Ok(())
    }

    fn active_application(&self) -> String {
        let state = self.lock();
        assert!(!state.panicking, "the backend failed in a way nobody foresaw");
        state.application.to_owned()
    }

    fn types_again(&self, stroke: &KeyEvent) -> bool {
        self.lock().untypable != Some(stroke.keycode)
    }

    fn focused_window(&self) -> Option<FocusInfo> {
        Some(FocusInfo { window: self.lock().window, own: false, isolated_layout: false })
    }

    fn probe(&self) -> BackendProbe {
        BackendProbe { available: true, session_type: "x11".to_owned(), current_group: self.lock().group, ..BackendProbe::default() }
    }

    fn inject_correction(&self, injection: Injection<'_>) -> Result<i32, BackendError> {
        let mut state = self.lock();
        let Injection { strokes, target_group, boundary, source_group, late, trailing, kept_tail, erased } = injection;
        assert!(!state.panicking_correction, "the backend failed in the middle of a correction");
        if state.failing {
            state.failing = false;
            state.note("a correction fails: the window does not take it".to_owned());
            state.rush();
            state.release_held();
            return Err(BackendError::Failed("the window is gone".to_owned()));
        }
        let final_group = if kept_tail == 0 { target_group } else { source_group.unwrap_or_else(|| strokes[0].group) };
        let converted = strokes.len() - kept_tail;
        // What is typed, in the order it is typed: the characters and the key.
        let mut typed: Vec<(String, Origin)> = Vec::new();
        for (index, stroke) in strokes.iter().enumerate() {
            let group = if index < converted { target_group } else { final_group };
            typed.push((stroke.character_for(group).to_owned(), origin_of(stroke)));
        }
        for stroke in trailing.iter().chain(boundary) {
            typed.push((stroke.character.clone(), origin_of(stroke)));
        }
        for stroke in late {
            typed.push((stroke.character_for(final_group).to_owned(), origin_of(stroke)));
        }
        let word = erased.unwrap_or(strokes.len());
        let count = word + trailing.len() + usize::from(boundary.is_some()) + late.len();
        let typed_text: String = typed.iter().map(|(character, _)| character.as_str()).collect();
        if state.password && state.read {
            let text = format!("a correction types «{typed_text}» into a password field");
            state.violation(text);
        }
        if count > state.caret {
            let text = format!("a correction erases {count} characters and types «{typed_text}» where the window has «{}»", state.shown());
            state.violation(text);
            state.release_held();
            return Err(BackendError::Failed("the correction erases more than the field holds".to_owned()));
        }
        let start = state.caret - count;
        let erased_cells: Vec<Cell> = state.cells[start..state.caret].to_vec();
        let erased_text: String = erased_cells.iter().map(|cell| cell.character).collect();
        // What follows the word is typed again key for key. So is the word, unless a typo was corrected in it: then it
        // is the person's own word, and at most a letter or two of it changed.
        let tail_kept = erased_cells.iter().skip(word).map(|cell| cell.origin).eq(typed.iter().skip(strokes.len()).map(|(_, origin)| Some(*origin)));
        let own_word = erased_cells.iter().take(word).all(|cell| cell.origin.is_some());
        let changed = if erased.is_some() {
            TYPO_KEYS
        } else {
            erased_cells.iter().zip(&typed).take(word).filter(|(cell, (_, origin))| cell.origin != Some(*origin)).count()
        };
        let allowed = if state.typos { TYPO_KEYS } else { 0 };
        if !tail_kept || !own_word || changed > allowed {
            let text = format!("a correction erases «{erased_text}» and types «{typed_text}»: other keys than the person pressed; the window had «{}»", state.shown());
            state.violation(text);
        }
        if changed != 0 {
            state.typo_corrections += 1;
        }
        let cells: Vec<Cell> = typed.iter().flat_map(|(characters, origin)| characters.chars().map(|character| Cell { character, origin: Some(*origin) })).collect();
        let caret = state.caret;
        let added = cells.len();
        state.cells.splice(start..caret, cells);
        state.caret = start + added;
        state.group = final_group;
        state.injections += 1;
        let seconds = state.dice.seconds(0.01, 0.12);
        state.clock.advance(seconds);
        let line = format!("a correction erases «{erased_text}» and types «{typed_text}», layout {final_group}, {} late; the window shows «{}»", late.len(), state.shown());
        state.note(line);
        // The late keys come back through the hook as keys of the person's, and so the engine hears of them again.
        if let Some(listener) = state.listener.clone() {
            for stroke in late {
                let character = stroke.character_for(final_group).to_owned();
                let pressed = KeyEvent { pressed: true, key_name: character.clone(), character, group: final_group, timestamp: state.stamp(), ..stroke.clone() };
                listener(pressed.clone());
                listener(KeyEvent { pressed: false, ..pressed });
            }
        }
        state.rush();
        Ok(state.release_held())
    }
}

/// The field as an accessibility reader shows it: sometimes a few characters behind the keys.
struct DeskReader {
    desk: Arc<Desk>,
}

impl FieldReader for DeskReader {
    fn read(&self, application: &str, _window: i64) -> Option<FieldContext> {
        let state = self.desk.lock();
        if state.reader_silent {
            return None;
        }
        let mut field = FieldContext::new(application, state.window.to_string());
        "uia".clone_into(&mut field.source);
        if state.password {
            field.role = FieldRole::Password;
            field.sensitive = true;
            return Some(field);
        }
        let seen = state.caret.saturating_sub(state.reader_lag);
        field.before = state.cells[..seen].iter().map(|cell| cell.character).collect();
        if state.reader_lag == 0 {
            field.after = state.cells[state.caret..].iter().map(|cell| cell.character).collect();
        }
        field.role = FieldRole::Text;
        Some(field)
    }

    fn status(&self) -> String {
        "available".to_owned()
    }
}

const RUSSIAN: &[&str] = &[
    "привет", "мир", "как", "дела", "что", "это", "не", "на", "я", "в", "и", "с", "по", "да", "нет", "все", "уже", "там", "тут", "так", "вот", "мне", "тебе", "надо", "можно",
    "сегодня", "завтра", "вчера", "хорошо", "плохо", "спасибо", "пожалуйста", "работа", "проект", "задача", "ошибка", "сервер", "файл", "папка", "текст", "слово", "клавиша",
    "раскладка", "программа", "человек", "время", "день", "ночь", "утро", "вечер", "если", "когда", "потом", "сейчас", "очень", "быстро", "медленно", "сделать", "посмотреть",
    "написать", "проверить", "исправить", "запустить", "обновить", "удалить", "добавить", "найти", "хочу", "буду", "был", "была", "они", "она", "он", "мы", "вы", "ты", "кто",
    "где", "зачем", "почему", "ещё", "или", "но", "а", "же", "бы", "ли", "под", "над", "без", "для", "про", "после", "перед", "через", "между", "объект", "съезд", "жизнь",
    "хлеб", "южный", "эхо", "щётка", "ёлка", "подъезд", "бюджет", "жюри", "экзамен", "хэш", "ёж",
];

const ENGLISH: &[&str] = &[
    "hello", "world", "the", "and", "is", "it", "to", "of", "in", "a", "i", "ok", "yes", "no", "not", "for", "with", "this", "that", "from", "have", "what", "when", "where",
    "today", "work", "project", "task", "error", "server", "file", "folder", "text", "word", "key", "layout", "program", "time", "day", "night", "fast", "slow", "make", "check",
    "write", "run", "update", "delete", "add", "find", "git", "commit", "push", "pull", "merge", "branch", "docker", "npm", "node", "rust", "cargo", "python", "linux", "windows",
    "api", "url", "json", "http", "sql", "css", "html", "bug", "fix", "test", "build", "deploy", "release", "code", "review", "issue", "token", "user", "login", "admin", "config",
    "queue", "cache", "index", "query", "table", "value", "string", "number", "array", "object", "class", "function", "return", "import", "export", "async", "await", "x", "y", "z",
];

/// What stands between two words, as the person means it in each language.
const JOINTS: &[&str] = &[" ", " ", " ", " ", " ", " ", " ", " ", ", ", ". ", "? ", "! ", ": ", "; ", " - ", "-", ".", ",", "/", " (", ") ", "\" ", " \"", "'", "_"];

const PASTED: &[&str] = &["https://example.org/a?b=1", "готовый текст", "word", " ", "2026-11-11", "две строки"];

const NAVIGATION: &[(u32, &str)] = &[(HOME_KEYCODE, "Home"), (END_KEYCODE, "End"), (RIGHT_KEYCODE, "Right"), (DELETE_KEYCODE, "Delete"), (ESCAPE_KEYCODE, "Escape")];

struct Typist {
    acts: Vec<Act>,
    /// The key that is still down and comes up after the next press (rollover typing).
    lingering: Option<Raw>,
    shift: bool,
}

impl Typist {
    fn press(&mut self, dice: &mut Dice, keycode: u32, name: &'static str) {
        let down = Raw { pressed: true, keycode, name };
        let up = Raw { pressed: false, ..down };
        self.acts.push(Act::Key(down));
        if let Some(lingering) = self.lingering.take() {
            self.acts.push(Act::Key(lingering));
        }
        if name.is_empty() && dice.one_in(40) {
            // The key stays down long enough to repeat.
            for _ in 0..1 + dice.below(4) {
                self.acts.push(Act::Key(down));
            }
        }
        if name.is_empty() && dice.one_in(4) {
            self.lingering = Some(up);
        } else {
            self.acts.push(Act::Key(up));
        }
    }

    fn settle(&mut self) {
        if let Some(lingering) = self.lingering.take() {
            self.acts.push(Act::Key(lingering));
        }
    }

    fn set_shift(&mut self, wanted: bool) {
        if self.shift != wanted {
            self.settle();
            self.acts.push(Act::Key(Raw { pressed: wanted, keycode: SHIFT_KEYCODE, name: "Shift_L" }));
            self.shift = wanted;
        }
    }

    /// The keys of `text` as someone who means `layout` presses them, whatever layout is on.
    fn text(&mut self, dice: &mut Dice, text: &str, layout: usize) {
        for character in text.chars() {
            let Some(origin) = key_for(character, layout).or_else(|| key_for(character, 1 - layout)) else {
                continue;
            };
            self.set_shift(origin.shift);
            self.press(dice, origin.keycode, "");
        }
        self.set_shift(false);
    }

    fn named(&mut self, dice: &mut Dice, keycode: u32, name: &'static str) {
        self.settle();
        self.press(dice, keycode, name);
    }

    /// Control+Alt and a letter: the undo hotkey.
    fn chord(&mut self, dice: &mut Dice, letter: char) {
        self.settle();
        let origin = key_for(letter, 0).expect("a Latin letter");
        self.acts.push(Act::Key(Raw { pressed: true, keycode: CONTROL_KEYCODE, name: "Control_L" }));
        self.acts.push(Act::Key(Raw { pressed: true, keycode: ALT_KEYCODE, name: "Alt_L" }));
        self.press(dice, origin.keycode, "");
        self.settle();
        self.acts.push(Act::Key(Raw { pressed: false, keycode: ALT_KEYCODE, name: "Alt_L" }));
        self.acts.push(Act::Key(Raw { pressed: false, keycode: CONTROL_KEYCODE, name: "Control_L" }));
    }
}

fn capitalised(word: &str) -> String {
    let mut letters = word.chars();
    letters.next().map_or_else(String::new, |first| first.to_uppercase().chain(letters).collect())
}

/// One walk of the person: sentences in two languages with what people do between the words.
fn script(dice: &mut Dice) -> Vec<Act> {
    let mut typist = Typist { acts: Vec::new(), lingering: None, shift: false };
    let mut language = dice.below(2);
    for _ in 0..3 + dice.below(4) {
        let words = 1 + dice.below(7);
        for position in 0..words {
            if dice.one_in(4) {
                language = 1 - language;
            }
            let word = *dice.of(if language == 1 { RUSSIAN } else { ENGLISH });
            let word = match dice.below(30) {
                0 => word.to_uppercase(),
                1..=3 => capitalised(word),
                _ if position == 0 && dice.one_in(2) => capitalised(word),
                _ => word.to_owned(),
            };
            typist.text(dice, &word, language);
            match dice.below(60) {
                0..=3 => typist.acts.push(Act::Wait(dice.seconds(0.2, 3.0))),
                4..=6 => typist.named(dice, PAUSE_KEYCODE, "Pause"),
                7..=8 => {
                    typist.named(dice, PAUSE_KEYCODE, "Pause");
                    typist.named(dice, PAUSE_KEYCODE, "Pause");
                }
                9 => typist.chord(dice, 'z'),
                10..=12 => {
                    for _ in 0..1 + dice.below(4) {
                        typist.named(dice, BACKSPACE_KEYCODE, "BackSpace");
                    }
                }
                13 => typist.acts.push(Act::Click),
                14 => typist.acts.push(Act::Layout),
                15 => {
                    for _ in 0..1 + dice.below(3) {
                        typist.named(dice, LEFT_KEYCODE, "Left");
                    }
                }
                16..=17 => {
                    let (keycode, name) = *dice.of(NAVIGATION);
                    typist.named(dice, keycode, name);
                }
                18 => {
                    typist.settle();
                    typist.acts.push(Act::Paste(dice.of(PASTED)));
                }
                19 => typist.acts.push(Act::Lag(dice.below(4))),
                20 => typist.acts.push(Act::Fail),
                _ => {}
            }
            if position + 1 < words {
                let joint = *dice.of(JOINTS);
                typist.text(dice, joint, language);
            }
        }
        match dice.below(16) {
            0..=6 => typist.named(dice, RETURN_KEYCODE, "Return"),
            7..=8 => {
                typist.text(dice, ". ", language);
                typist.named(dice, RETURN_KEYCODE, "Return");
            }
            9 => typist.named(dice, TAB_KEYCODE, "Tab"),
            10 => {
                typist.settle();
                typist.acts.push(Act::Window);
            }
            11 => {
                typist.settle();
                typist.acts.push(Act::Password);
            }
            _ => typist.text(dice, ". ", language),
        }
        if dice.one_in(3) {
            typist.acts.push(Act::Wait(dice.seconds(0.5, 6.0)));
        }
    }
    typist.settle();
    typist.acts
}

fn drain(core: &mut EngineCore) {
    while core.shared.qsize() != 0 {
        core.turn(0.0);
    }
    // The queue is empty: this turn is the timers'.
    core.turn(0.0);
}

fn idle(core: &mut EngineCore, clock: &FakeClock, seconds: f64) {
    let mut left = seconds;
    while left > 0.0 {
        clock.advance(TIMER_STEP_SECONDS.min(left));
        left -= TIMER_STEP_SECONDS;
        drain(core);
    }
}

/// What one walk found: what broke, and how much of the engine it exercised.
#[derive(Default)]
struct Outcome {
    violations: Vec<String>,
    injections: usize,
    typo_corrections: usize,
    deferred: usize,
    dropped: usize,
    rushed: usize,
}

#[derive(Clone, Copy)]
struct Walk {
    holds: bool,
    reads_field: bool,
    typos: bool,
    /// The engine may be behind even when the person clicks, changes the window or the layout.
    stalls: bool,
}

fn walk(bundle: &mut Bundle, seed: u64, mode: Walk, verbose: bool) -> Outcome {
    let mut dice = Dice(0x9e37_79b9_7f4a_7c15 ^ seed.wrapping_mul(0xd6e8_feb8_6659_fd93));
    let clock = FakeClock::new();
    let acts = script(&mut dice);
    let group = i32::try_from(dice.below(2)).expect("small");
    let desk = Desk::new(mode, group, acts, dice.next() | 1, &clock, verbose);
    let mut core = engine_over(bundle, &desk, mode, &clock);
    let mut executed = 0;
    loop {
        let Some(act) = desk.lock().acts.pop_front() else {
            break;
        };
        if !matches!(act, Act::Key(_)) && !mode.stalls {
            // What takes a person a moment finds the engine caught up.
            drain(&mut core);
        }
        match act {
            Act::Key(raw) => {
                clock.advance(dice.seconds(0.02, 0.22));
                desk.key(raw);
            }
            Act::Wait(seconds) => {
                drain(&mut core);
                idle(&mut core, &clock, seconds);
            }
            Act::Click => {
                let mut state = desk.lock();
                state.caret = if dice.one_in(2) { dice.below(state.cells.len() + 1) } else { state.cells.len() };
                let (group, stamp) = (state.group, state.stamp());
                let line = format!("a click, the window shows «{}»", state.shown());
                state.note(line);
                let listener = state.listener.clone().expect("started");
                drop(state);
                listener(KeyEvent { pressed: true, key_name: "Pointer".to_owned(), characters: vec![String::new(), String::new()], group, timestamp: stamp, ..KeyEvent::default() });
            }
            Act::Layout => {
                let mut state = desk.lock();
                state.group = 1 - state.group;
                let line = format!("the person switches the layout to {}", state.group);
                state.note(line);
            }
            Act::Window | Act::Password => {
                let mut state = desk.lock();
                let password = matches!(act, Act::Password);
                state.another_field(password);
                state.note(if password { "another window, a password field".to_owned() } else { "another window".to_owned() });
            }
            Act::Paste(text) => {
                desk.lock().clipboard = text;
                let letter = key_for('v', 0).expect("a Latin letter").keycode;
                for raw in [
                    Raw { pressed: true, keycode: CONTROL_KEYCODE, name: "Control_L" },
                    Raw { pressed: true, keycode: letter, name: "" },
                    Raw { pressed: false, keycode: letter, name: "" },
                    Raw { pressed: false, keycode: CONTROL_KEYCODE, name: "Control_L" },
                ] {
                    clock.advance(dice.seconds(0.02, 0.1));
                    desk.key(raw);
                }
            }
            Act::Lag(behind) => desk.lock().reader_lag = behind,
            Act::Fail => desk.lock().failing = true,
        }
        // The engine's worker: most of the time it keeps up, sometimes it is a key or several behind.
        match dice.below(20) {
            0..=13 => drain(&mut core),
            14..=17 => {
                core.turn(0.0);
            }
            _ => {}
        }
        let plans = core.executed.as_ref().map_or(&[][..], Vec::as_slice);
        let mut state = desk.lock();
        for plan in &plans[executed..] {
            let line = format!("    the plan was {} ({}): «{}» to «{}»", plan.mode, if plan.automatic { "the engine's own" } else { "asked for" }, plan.original, plan.replacement);
            state.note(line);
        }
        executed = plans.len();
        if !state.violations.is_empty() {
            let found = std::mem::take(&mut state.violations);
            let journal: Vec<String> = state.journal.iter().cloned().collect();
            drop(state);
            return finish(bundle, core, &desk, vec![format!("{}\n    {}", found[0], journal.join("\n    "))]);
        }
    }
    drain(&mut core);
    idle(&mut core, &clock, SETTLE_SECONDS);
    let mut found = Vec::new();
    {
        let state = desk.lock();
        let journal: Vec<String> = state.journal.iter().cloned().collect();
        if let Some(text) = state.violations.first() {
            found.push(format!("{text}\n    {}", journal.join("\n    ")));
        }
        if state.holding || !state.held.is_empty() || state.deferred.is_some() {
            found.push(format!(
                "the keyboard stays held after the person stopped: holding {}, {} keys kept back, an action waiting: {}\n    {}",
                state.holding,
                state.held.len(),
                state.deferred.is_some(),
                journal.join("\n    ")
            ));
        }
        if state.held_keys != state.replayed_keys {
            found.push(format!("keys were kept back and not all typed again: {} and {}\n    {}", state.held_keys, state.replayed_keys, journal.join("\n    ")));
        }
        if core.shared.current_group() != i64::from(state.group) {
            found.push(format!("the engine ends up with another layout than the keyboard: {} and {}\n    {}", core.shared.current_group(), state.group, journal.join("\n    ")));
        }
        if verbose {
            println!("{}", journal.join("\n"));
        }
    }
    finish(bundle, core, &desk, found)
}

/// The engine over a desk, with the settings the program ships with but those the walk names.
fn engine_over(bundle: &mut Bundle, desk: &Arc<Desk>, mode: Walk, clock: &FakeClock) -> EngineCore {
    let reader: Option<Arc<dyn FieldReader>> = mode.reads_field.then(|| -> Arc<dyn FieldReader> { Arc::new(DeskReader { desk: desk.clone() }) });
    let mut core = bundle.core_over(desk.clone(), reader, clock, mode.typos);
    core.settings.set("detection.context_read_field", mode.reads_field.into(), false).expect("in memory");
    core.apply_commands();
    desk.start(core.shared.key_listener()).expect("the model starts");
    desk.set_key_filter(Some(core.shared.key_filter()));
    core
}

fn finish(bundle: &mut Bundle, core: EngineCore, desk: &Desk, violations: Vec<String>) -> Outcome {
    bundle.take_back(core);
    let state = desk.lock();
    Outcome {
        violations,
        injections: state.injections,
        typo_corrections: state.typo_corrections,
        deferred: state.deferred_actions,
        dropped: state.dropped_actions,
        rushed: state.rushed_keys,
    }
}

const WALKS: u64 = 600;

fn walks(mode: Walk, name: &str) {
    let mut bundle = Bundle::load(None);
    let mut failures: BTreeMap<String, (u64, String, usize)> = BTreeMap::new();
    let mut total = Outcome::default();
    let mut dropped_at: Vec<u64> = Vec::new();
    for seed in 1..=WALKS {
        let outcome = walk(&mut bundle, seed, mode, false);
        total.injections += outcome.injections;
        total.typo_corrections += outcome.typo_corrections;
        total.deferred += outcome.deferred;
        total.dropped += outcome.dropped;
        total.rushed += outcome.rushed;
        if outcome.dropped != 0 && dropped_at.len() < 12 {
            dropped_at.push(seed);
        }
        for violation in outcome.violations {
            // One line per kind of failure: the first words of it, without the text it happened in.
            let kind: String = violation.split(['«', ':']).next().unwrap_or_default().trim().to_owned();
            let entry = failures.entry(kind).or_insert((seed, violation, 0));
            entry.2 += 1;
        }
    }
    bundle.clean();
    println!(
        "{name}: {WALKS} walks, {} corrections ({} of typos), {} Enters and Tabs kept back ({} not passed on), {} keys pressed while a correction was typed",
        total.injections, total.typo_corrections, total.deferred, total.dropped, total.rushed
    );
    println!("{name}: the first walks with an Enter or Tab not passed on: {dropped_at:?}");
    for (kind, (seed, example, count)) in &failures {
        println!("{name}: {count} walks: {kind}\n  first at seed {seed}:\n    {example}");
    }
    assert!(failures.is_empty(), "{name}: {} kinds of failure", failures.len());
    // A walk that corrects nothing checks nothing.
    assert!(total.injections > usize::try_from(WALKS).expect("small"), "{name}: only {} corrections", total.injections);
}

/// `KS_INTEGRITY_WALK=<mode letters>:<seed>` walks one seed with every key in the journal and prints it; the letters are
/// h (the hook keeps keys back), f (the field is read), t (typo corrections), s (the engine may stall anywhere). With
/// `KS_INTEGRITY_LOG=<file>` the engine's technical log goes to that file.
#[test]
fn one_walk_looked_at_closely() {
    let Some(asked) = std::env::var_os("KS_INTEGRITY_WALK") else {
        return;
    };
    let asked = asked.to_string_lossy().into_owned();
    let (letters, seed) = asked.split_once(':').expect("<mode letters>:<seed>");
    let mode = Walk { holds: letters.contains('h'), reads_field: letters.contains('f'), typos: letters.contains('t'), stalls: letters.contains('s') };
    let log = std::env::var_os("KS_INTEGRITY_LOG").map(std::path::PathBuf::from);
    let mut bundle = Bundle::load(log.as_deref());
    let outcome = walk(&mut bundle, seed.parse().expect("a seed"), mode, true);
    bundle.clean();
    for violation in &outcome.violations {
        println!("{violation}");
    }
    println!("{} corrections, {} violations", outcome.injections, outcome.violations.len());
}

#[test]
fn the_text_stays_the_users_where_the_hook_only_watches() {
    walks(Walk { holds: false, reads_field: false, typos: false, stalls: false }, "watching hook");
}

#[test]
fn the_text_stays_the_users_where_the_hook_only_watches_and_the_field_is_read() {
    walks(Walk { holds: false, reads_field: true, typos: true, stalls: false }, "watching hook, field read, typos");
}

#[test]
fn the_text_stays_the_users_where_the_hook_keeps_keys_back() {
    walks(Walk { holds: true, reads_field: false, typos: false, stalls: false }, "holding hook");
}

#[test]
fn the_text_stays_the_users_where_the_hook_keeps_keys_back_and_the_field_is_read() {
    walks(Walk { holds: true, reads_field: true, typos: true, stalls: false }, "holding hook, field read, typos");
}

#[test]
fn the_text_stays_the_users_when_the_engine_stalls() {
    walks(Walk { holds: true, reads_field: true, typos: false, stalls: true }, "holding hook, field read, a stalling engine");
}

/// One desk and its engine for a scene typed by hand: every key finds the engine caught up, unless the scene says otherwise.
struct Scene {
    desk: Arc<Desk>,
    core: EngineCore,
    clock: FakeClock,
}

impl Scene {
    fn new(bundle: &mut Bundle, holds: bool, reads_field: bool) -> Scene {
        let clock = FakeClock::new();
        let mode = Walk { holds, reads_field, typos: false, stalls: false };
        let desk = Desk::new(mode, 0, Vec::new(), 1, &clock, true);
        let core = engine_over(bundle, &desk, mode, &clock);
        Scene { desk, core, clock }
    }

    fn raw(&mut self, raw: Raw) {
        self.clock.advance(0.06);
        self.desk.key(raw);
        drain(&mut self.core);
    }

    fn tap(&mut self, keycode: u32, name: &'static str) {
        self.raw(Raw { pressed: true, keycode, name });
        self.raw(Raw { pressed: false, keycode, name });
    }

    /// The keys of `text` as someone who means `layout` presses them.
    fn text(&mut self, text: &str, layout: usize) {
        for character in text.chars() {
            let origin = key_for(character, layout).expect("a key of the keyboard");
            if origin.shift {
                self.raw(Raw { pressed: true, keycode: SHIFT_KEYCODE, name: "Shift_L" });
            }
            self.tap(origin.keycode, "");
            if origin.shift {
                self.raw(Raw { pressed: false, keycode: SHIFT_KEYCODE, name: "Shift_L" });
            }
        }
    }

    fn idle(&mut self, seconds: f64) {
        idle(&mut self.core, &self.clock, seconds);
    }

    /// A click at the end of the text.
    fn click(&mut self) {
        let (group, stamp, listener) = {
            let state = self.desk.lock();
            (state.group, state.stamp(), state.listener.clone().expect("started"))
        };
        self.clock.advance(0.06);
        listener(KeyEvent { pressed: true, key_name: "Pointer".to_owned(), characters: vec![String::new(), String::new()], group, timestamp: stamp, ..KeyEvent::default() });
        drain(&mut self.core);
    }

    fn shown(&self) -> String {
        self.desk.lock().shown()
    }

    /// Nothing went wrong on the way, and the keyboard is the person's again.
    fn ends_clean(self, bundle: &mut Bundle) {
        bundle.take_back(self.core);
        let state = self.desk.lock();
        let journal: Vec<String> = state.journal.iter().cloned().collect();
        assert!(state.violations.is_empty(), "{:?}\n{}", state.violations, journal.join("\n"));
        assert!(!state.holding && state.held.is_empty() && state.deferred.is_none(), "the keyboard stays held\n{}", journal.join("\n"));
    }
}

/// X11 cannot keep the Enter that answers the rule prompt from the window: the window has had it, and the word before it
/// is over. `Djn`, Pause twice, Enter and a word of six letters: Pause converted nine keys on a line of six.
#[test]
fn an_enter_the_watching_hook_could_not_keep_ends_the_word_under_the_prompt() {
    let mut bundle = Bundle::load(None);
    // A backend says so only if it cannot keep a key back: the model editor of the replays is not asked to.
    assert!(ks_replay::harness::Backend::new("editor", 0, Vec::new()).filters_keys());
    let mut scene = Scene::new(&mut bundle, false, false);
    scene.text("Вот", 1);
    scene.tap(PAUSE_KEYCODE, "Pause");
    assert_eq!(scene.shown(), "Вот|");
    scene.tap(PAUSE_KEYCODE, "Pause");
    assert!(scene.core.shared.learning_prompt().is_some(), "the second press offers a rule");
    scene.tap(RETURN_KEYCODE, "Return");
    assert!(scene.core.shared.learning_prompt().is_none(), "Enter answers the prompt");
    scene.text("строка", 1);
    assert_eq!(scene.shown(), "строка|");
    scene.tap(PAUSE_KEYCODE, "Pause");
    assert_eq!(scene.shown(), "cnhjrf|");
    scene.ends_clean(&mut bundle);
    // Escape closes the prompt and types nothing: the word goes on.
    let mut scene = Scene::new(&mut bundle, false, false);
    scene.text("Вот", 1);
    scene.tap(PAUSE_KEYCODE, "Pause");
    scene.tap(PAUSE_KEYCODE, "Pause");
    assert!(scene.core.shared.learning_prompt().is_some());
    scene.tap(ESCAPE_KEYCODE, "Escape");
    assert!(scene.core.shared.learning_prompt().is_none());
    assert_eq!(scene.shown(), "Вот|");
    scene.ends_clean(&mut bundle);
    bundle.clean();
}

/// A correction that waits for a field read late takes the letters typed on with it, and gives way to any other key: a
/// bracket before the next word stands in the window where the correction counts none.
#[test]
fn a_correction_that_waits_for_its_field_gives_way_to_other_keys() {
    let mut bundle = Bundle::load(None);
    // The letters of the next word alone: the correction is made, and they are typed again after it.
    let mut scene = Scene::new(&mut bundle, true, true);
    scene.desk.lock().reader_lag = 2;
    scene.text("перед ", 1);
    assert_eq!(scene.shown(), "gthtl |", "the field is behind: the correction waits");
    scene.text("о", 1);
    scene.desk.lock().reader_lag = 1;
    scene.idle(0.2);
    assert_eq!(scene.shown(), "перед о|");
    scene.ends_clean(&mut bundle);
    // A bracket and a letter: the word stays as typed.
    let mut scene = Scene::new(&mut bundle, true, true);
    scene.desk.lock().reader_lag = 2;
    scene.text("перед ", 1);
    scene.text("(о", 1);
    scene.idle(1.0);
    assert_eq!(scene.shown(), "gthtl (j|");
    scene.ends_clean(&mut bundle);
    // A Backspace: the same.
    let mut scene = Scene::new(&mut bundle, true, true);
    scene.desk.lock().reader_lag = 2;
    scene.text("перед ", 1);
    scene.tap(BACKSPACE_KEYCODE, "BackSpace");
    scene.desk.lock().reader_lag = 0;
    scene.idle(1.0);
    assert_eq!(scene.shown(), "gthtl|");
    scene.ends_clean(&mut bundle);
    bundle.clean();
}

/// An Enter waits with its correction for a field that shows its text a moment late, and follows the correction.
#[test]
fn an_enter_waits_for_the_field_with_its_correction() {
    let mut bundle = Bundle::load(None);
    let mut scene = Scene::new(&mut bundle, true, true);
    scene.desk.lock().reader_lag = 1;
    scene.text("нет", 1);
    scene.tap(RETURN_KEYCODE, "Return");
    assert_eq!(scene.shown(), "ytn|", "the field is behind: the correction and its Enter wait");
    assert!(scene.desk.lock().deferred.is_some());
    // A key-up the hook lets through meanwhile (Windows sends one of Ctrl to a window whose layout it switches) is no
    // reason to send the Enter ahead of its correction.
    scene.raw(Raw { pressed: false, keycode: CONTROL_KEYCODE, name: "Control_L" });
    assert!(scene.desk.lock().deferred.is_some());
    scene.desk.lock().reader_lag = 0;
    scene.idle(0.1);
    assert_eq!(scene.desk.lock().submitted, ["нет"]);
    assert_eq!(scene.desk.lock().dropped_actions, 0);
    scene.ends_clean(&mut bundle);
    // A field that never shows the word: nothing is typed over it, and the Enter goes on with the text as it stands.
    let mut scene = Scene::new(&mut bundle, true, true);
    scene.desk.lock().reader_lag = 1;
    scene.text("нет", 1);
    scene.tap(RETURN_KEYCODE, "Return");
    assert!(scene.desk.lock().submitted.is_empty(), "the Enter waits for as long as the field may still show the word");
    scene.idle(3.0);
    assert_eq!(scene.desk.lock().injections, 0);
    assert_eq!(scene.desk.lock().submitted, ["ytn"]);
    assert_eq!(scene.desk.lock().dropped_actions, 0);
    scene.ends_clean(&mut bundle);
    // A field that gives no answer as the Enter comes up: the same, at once.
    let mut scene = Scene::new(&mut bundle, true, true);
    scene.text("нет", 1);
    scene.raw(Raw { pressed: true, keycode: RETURN_KEYCODE, name: "Return" });
    scene.desk.lock().reader_silent = true;
    scene.raw(Raw { pressed: false, keycode: RETURN_KEYCODE, name: "Return" });
    assert_eq!(scene.desk.lock().injections, 0);
    assert_eq!(scene.desk.lock().submitted, ["ytn"]);
    assert_eq!(scene.desk.lock().dropped_actions, 0);
    scene.ends_clean(&mut bundle);
    // A word with a key that types two characters at once, which no key types again: the same.
    let mut scene = Scene::new(&mut bundle, true, false);
    scene.core.settings.set("enabled", false.into(), false).expect("in memory");
    scene.core.apply_commands();
    scene.text("нет", 1);
    scene.tap(END_KEYCODE, "End");
    scene.core.settings.set("enabled", true.into(), false).expect("in memory");
    scene.core.apply_commands();
    let keycode = key_for('н', 1).expect("a key of the keyboard").keycode;
    let key = scene.desk.lock().event(Raw { pressed: true, keycode, name: "" }).0;
    let odd: Strokes = vec![Arc::new(KeyEvent { characters: vec!["ab".to_owned(), "ab".to_owned()], ..key })];
    scene.core.pending = Some(Arc::new(CorrectionPlan { mode: "before_action", ..CorrectionPlan::new(odd, None, 0, 1, "ytn".to_owned(), "нет".to_owned(), 0.9, "editor") }));
    scene.tap(RETURN_KEYCODE, "Return");
    assert_eq!(scene.desk.lock().injections, 0);
    assert_eq!(scene.desk.lock().submitted, ["ytn"]);
    assert_eq!(scene.desk.lock().dropped_actions, 0);
    scene.ends_clean(&mut bundle);
    bundle.clean();
}

/// A switch planned on the first letters of a word is looked at again before the Enter that ends the word, as it is
/// before any other key comes up: made as planned, it deleted the last letters and typed the first ones over them.
#[test]
fn an_enter_does_not_bring_a_switch_planned_on_the_first_letters_alone() {
    let mut bundle = Bundle::load(None);
    let mut scene = Scene::new(&mut bundle, true, false);
    // The word stands in the window, and the engine has finished with it.
    scene.core.settings.set("enabled", false.into(), false).expect("in memory");
    scene.core.apply_commands();
    scene.text("привет", 1);
    scene.tap(END_KEYCODE, "End");
    scene.core.settings.set("enabled", true.into(), false).expect("in memory");
    // The journal says why the plan was given up.
    scene.core.settings.set("diagnostics.technical_logging", true.into(), false).expect("in memory");
    scene.core.apply_commands();
    assert_eq!(scene.shown(), "ghbdtn|");
    let first: Strokes = "прив"
        .chars()
        .map(|character| {
            let keycode = key_for(character, 1).expect("a key of the keyboard").keycode;
            Arc::new(scene.desk.lock().event(Raw { pressed: true, keycode, name: "" }).0)
        })
        .collect();
    scene.core.pending = Some(Arc::new(CorrectionPlan { mode: "early", ..CorrectionPlan::new(first, None, 0, 1, "ghbd".to_owned(), "прив".to_owned(), 0.9, "editor") }));
    scene.tap(RETURN_KEYCODE, "Return");
    assert_eq!(scene.desk.lock().injections, 0);
    assert_eq!(scene.desk.lock().submitted, ["ghbdtn"]);
    scene.ends_clean(&mut bundle);
    // A plan that still names the letters as they stand is made before the Enter, as any other: here the undo of a
    // switch, with the rest of the token left alone.
    let mut scene = Scene::new(&mut bundle, true, false);
    scene.core.settings.set("detection.early_switch", false.into(), false).expect("in memory");
    scene.core.apply_commands();
    scene.text("прив", 1);
    assert_eq!(scene.shown(), "ghbd|");
    scene.core.untracked_token = true;
    let typed = scene.core.strokes.clone();
    scene.core.pending = Some(Arc::new(CorrectionPlan { mode: "early_undo", automatic: false, ..CorrectionPlan::new(typed, None, 0, 1, "ghbd".to_owned(), "прив".to_owned(), 0.9, "editor") }));
    scene.tap(RETURN_KEYCODE, "Return");
    assert_eq!(scene.desk.lock().submitted, ["прив"]);
    scene.ends_clean(&mut bundle);
    bundle.clean();
}

/// Enter and Tab are kept back only for a correction that may come before them: with nothing typed since the last one
/// they reach the window as they are pressed and repeat while held, and a program the user excluded has them as its own.
#[test]
fn an_enter_with_nothing_before_it_is_the_windows_own() {
    let mut bundle = Bundle::load(None);
    let mut scene = Scene::new(&mut bundle, true, false);
    scene.text("нет", 1);
    scene.tap(RETURN_KEYCODE, "Return");
    assert_eq!(scene.desk.lock().submitted, ["нет"]);
    assert_eq!(scene.desk.lock().deferred_actions, 1, "the Enter after a word waits for its correction");
    // A held Enter: every repeat of it goes to the window as it comes.
    for _ in 0..3 {
        scene.raw(Raw { pressed: true, keycode: RETURN_KEYCODE, name: "Return" });
    }
    assert_eq!(scene.desk.lock().submitted, ["нет", "", "", ""]);
    scene.raw(Raw { pressed: false, keycode: RETURN_KEYCODE, name: "Return" });
    scene.tap(TAB_KEYCODE, "Tab");
    assert_eq!(scene.desk.lock().deferred_actions, 1);
    // A click types nothing: the Enter after it is the window's own too.
    scene.click();
    scene.raw(Raw { pressed: true, keycode: RETURN_KEYCODE, name: "Return" });
    assert_eq!(scene.desk.lock().deferred_actions, 1);
    scene.raw(Raw { pressed: false, keycode: RETURN_KEYCODE, name: "Return" });
    // The next word has its Enter kept back again, and held, that Enter comes once: after its correction.
    scene.text("да", 1);
    scene.raw(Raw { pressed: true, keycode: RETURN_KEYCODE, name: "Return" });
    scene.raw(Raw { pressed: true, keycode: RETURN_KEYCODE, name: "Return" });
    scene.raw(Raw { pressed: false, keycode: RETURN_KEYCODE, name: "Return" });
    assert_eq!(scene.desk.lock().deferred_actions, 2);
    assert_eq!(scene.desk.lock().submitted.last().map(String::as_str), Some("да"));
    assert_eq!(scene.desk.lock().dropped_actions, 0);
    scene.ends_clean(&mut bundle);
    // A program in the user's exclusions: nothing of it is kept back, and nothing in it is corrected.
    let mut scene = Scene::new(&mut bundle, true, false);
    scene.core.settings.set("exclusions.applications", serde_json::json!(["Editor"]), false).expect("in memory");
    scene.core.apply_commands();
    scene.text("нет", 1);
    scene.tap(RETURN_KEYCODE, "Return");
    assert_eq!(scene.desk.lock().deferred_actions, 0);
    assert_eq!(scene.desk.lock().submitted, ["ytn"]);
    // Taken off the list, it is a program like any other.
    scene.core.settings.set("exclusions.applications", serde_json::json!([]), false).expect("in memory");
    scene.core.apply_commands();
    scene.text("нет", 1);
    scene.tap(RETURN_KEYCODE, "Return");
    assert_eq!(scene.desk.lock().deferred_actions, 1);
    assert_eq!(scene.desk.lock().submitted, ["ytn", "нет"]);
    scene.ends_clean(&mut bundle);
    // An excluded program that has just come to the front (Alt+Tab to a game), its Tab held down at once: the hook
    // learns of the program from the first key the engine takes there, so it keeps this Tab back, and the engine passes
    // it on as it takes it, not when the key comes up. The second Tab is the program's own.
    let mut scene = Scene::new(&mut bundle, true, false);
    scene.core.settings.set("exclusions.applications", serde_json::json!(["game"]), false).expect("in memory");
    scene.core.apply_commands();
    scene.text("нет", 1);
    scene.tap(RETURN_KEYCODE, "Return");
    assert_eq!(scene.desk.lock().submitted, ["нет"]);
    scene.raw(Raw { pressed: true, keycode: ALT_KEYCODE, name: "Alt_L" });
    {
        let mut state = scene.desk.lock();
        state.another_field(false);
        state.application = "game";
    }
    scene.raw(Raw { pressed: false, keycode: ALT_KEYCODE, name: "Alt_L" });
    scene.idle(1.0);
    let field = scene.desk.lock().window;
    scene.raw(Raw { pressed: true, keycode: TAB_KEYCODE, name: "Tab" });
    assert_eq!(scene.desk.lock().deferred_actions, 2, "the hook does not know the program yet");
    assert_ne!(scene.desk.lock().window, field, "the Tab reaches the program while its key is still down");
    scene.idle(2.5);
    scene.raw(Raw { pressed: false, keycode: TAB_KEYCODE, name: "Tab" });
    let field = scene.desk.lock().window;
    scene.raw(Raw { pressed: true, keycode: TAB_KEYCODE, name: "Tab" });
    assert_ne!(scene.desk.lock().window, field);
    scene.raw(Raw { pressed: false, keycode: TAB_KEYCODE, name: "Tab" });
    assert_eq!(scene.desk.lock().deferred_actions, 2);
    assert_eq!(scene.desk.lock().dropped_actions, 0);
    scene.ends_clean(&mut bundle);
    bundle.clean();
}

/// An Enter pressed while the correction of an earlier word still waits for its field is answered as soon as that wait
/// ends, whichever way: it is no key typed after the word, and nothing else would answer it before its two seconds ran
/// out, with every key typed meanwhile held behind it.
#[test]
fn an_enter_is_answered_when_the_wait_of_an_earlier_word_ends() {
    let mut bundle = Bundle::load(None);
    // The field catches up: the word is corrected, and the Enter follows.
    let mut scene = Scene::new(&mut bundle, true, true);
    scene.desk.lock().reader_lag = 2;
    scene.text("нет ", 1);
    assert_eq!(scene.shown(), "ytn |", "the field is behind: the correction of the word waits");
    assert!(scene.core.field_wait.is_some());
    scene.tap(RETURN_KEYCODE, "Return");
    scene.desk.lock().reader_lag = 0;
    scene.idle(0.5);
    assert_eq!(scene.desk.lock().submitted, ["нет "]);
    scene.ends_clean(&mut bundle);
    // A word typed on that the engine keeps: the wait is given up, and the Enter goes on at once.
    let mut scene = Scene::new(&mut bundle, true, true);
    scene.desk.lock().reader_lag = 2;
    scene.text("нет ", 1);
    assert!(scene.core.field_wait.is_some());
    // Typed without a gap: the engine is behind, and the wait is not tried again meanwhile.
    for (pressed, keycode, name) in [(true, key_for('o', 0), ""), (false, key_for('o', 0), ""), (true, key_for('k', 0), ""), (false, key_for('k', 0), "")] {
        scene.desk.key(Raw { pressed, keycode: keycode.expect("a key of the keyboard").keycode, name });
    }
    scene.desk.key(Raw { pressed: true, keycode: RETURN_KEYCODE, name: "Return" });
    scene.desk.key(Raw { pressed: false, keycode: RETURN_KEYCODE, name: "Return" });
    scene.desk.lock().reader_lag = 0;
    drain(&mut scene.core);
    scene.idle(0.5);
    assert_eq!(scene.desk.lock().submitted, ["ytn ok"]);
    scene.ends_clean(&mut bundle);
    bundle.clean();
}

/// A word, its space and Enter typed faster than the engine takes them: the Enter the hook keeps back is in the engine's
/// queue when the word is about to be corrected. It has not reached the window and the hook holds every key after it, so
/// the word is corrected and the Enter follows. With letters typed between the space and the Enter the correction is
/// given up: typed again, they would come back to the engine behind that Enter, though they stand before it.
#[test]
fn an_enter_already_in_the_queue_does_not_stop_the_correction_before_it() {
    let mut bundle = Bundle::load(None);
    let space = key_for(' ', 0).expect("the space bar").keycode;
    for behind in [false, true] {
        let mut scene = Scene::new(&mut bundle, true, false);
        scene.text("нет", 1);
        scene.raw(Raw { pressed: true, keycode: space, name: "" });
        for raw in [Raw { pressed: false, keycode: space, name: "" }, Raw { pressed: true, keycode: RETURN_KEYCODE, name: "Return" }, Raw { pressed: false, keycode: RETURN_KEYCODE, name: "Return" }] {
            scene.clock.advance(0.03);
            scene.desk.key(raw);
            if !behind {
                drain(&mut scene.core);
            }
        }
        drain(&mut scene.core);
        scene.idle(1.0);
        assert_eq!(scene.desk.lock().submitted, ["нет "], "the engine a key behind: {behind}");
        assert_eq!(scene.desk.lock().dropped_actions, 0);
        // The line after the Enter is the engine's as it is the window's.
        scene.text("да ", 1);
        scene.idle(1.0);
        assert_eq!(scene.shown(), "да |");
        scene.ends_clean(&mut bundle);
    }
    // Letters between the space and the Enter, all of it in the queue: the word stays as typed, the Enter goes on, and
    // the next line is corrected as any other.
    let mut scene = Scene::new(&mut bundle, true, false);
    scene.text("нет", 1);
    scene.raw(Raw { pressed: true, keycode: space, name: "" });
    let ok = [key_for('o', 0), key_for('k', 0)].map(|key| key.expect("a key of the keyboard").keycode);
    for raw in [
        Raw { pressed: false, keycode: space, name: "" },
        Raw { pressed: true, keycode: ok[0], name: "" },
        Raw { pressed: false, keycode: ok[0], name: "" },
        Raw { pressed: true, keycode: ok[1], name: "" },
        Raw { pressed: false, keycode: ok[1], name: "" },
        Raw { pressed: true, keycode: RETURN_KEYCODE, name: "Return" },
        Raw { pressed: false, keycode: RETURN_KEYCODE, name: "Return" },
    ] {
        scene.clock.advance(0.03);
        scene.desk.key(raw);
    }
    drain(&mut scene.core);
    scene.idle(1.0);
    assert_eq!(scene.desk.lock().submitted, ["ytn ok"]);
    assert_eq!(scene.desk.lock().dropped_actions, 0);
    scene.text("да ", 1);
    scene.idle(1.0);
    assert_eq!(scene.shown(), "да |");
    scene.ends_clean(&mut bundle);
    // The same with a capital among the letters: the Shift pressed for it has stopped the correction already.
    let mut scene = Scene::new(&mut bundle, true, false);
    scene.text("нет", 1);
    scene.raw(Raw { pressed: true, keycode: space, name: "" });
    for raw in [
        Raw { pressed: false, keycode: space, name: "" },
        Raw { pressed: true, keycode: ok[0], name: "" },
        Raw { pressed: false, keycode: ok[0], name: "" },
        Raw { pressed: true, keycode: SHIFT_KEYCODE, name: "Shift_L" },
        Raw { pressed: true, keycode: ok[1], name: "" },
        Raw { pressed: false, keycode: ok[1], name: "" },
        Raw { pressed: false, keycode: SHIFT_KEYCODE, name: "Shift_L" },
        Raw { pressed: true, keycode: RETURN_KEYCODE, name: "Return" },
        Raw { pressed: false, keycode: RETURN_KEYCODE, name: "Return" },
    ] {
        scene.clock.advance(0.03);
        scene.desk.key(raw);
    }
    drain(&mut scene.core);
    scene.idle(1.0);
    assert_eq!(scene.desk.lock().submitted, ["ytn oK"]);
    assert_eq!(scene.desk.lock().dropped_actions, 0);
    scene.ends_clean(&mut bundle);
    bundle.clean();
}

/// An Enter the hook kept back and passed on is an Enter the window has had: what came before it says nothing of the
/// word typed after it. A key the engine does not follow (F5) used to keep that word as typed.
#[test]
fn an_enter_passed_on_ends_what_was_typed_before_it() {
    let mut bundle = Bundle::load(None);
    for holds in [false, true] {
        let mut scene = Scene::new(&mut bundle, holds, false);
        scene.tap(F5_KEYCODE, "F5");
        scene.tap(RETURN_KEYCODE, "Return");
        scene.text("привет ", 1);
        scene.idle(1.0);
        assert_eq!(scene.shown(), "привет |", "the hook keeps keys back: {holds}");
        scene.ends_clean(&mut bundle);
    }
    bundle.clean();
}

/// A correction refused before anything is typed lets the Enter kept back for it go on: a key-up that reached the queue
/// behind the Enter's own (Windows sends one of Ctrl to a window whose layout it switches), and a key no correction can
/// type again.
#[test]
fn an_enter_goes_on_when_its_correction_is_refused_untyped() {
    let mut bundle = Bundle::load(None);
    let mut scene = Scene::new(&mut bundle, true, false);
    scene.text("нет", 1);
    scene.raw(Raw { pressed: true, keycode: RETURN_KEYCODE, name: "Return" });
    scene.clock.advance(0.06);
    scene.desk.key(Raw { pressed: false, keycode: RETURN_KEYCODE, name: "Return" });
    scene.desk.key(Raw { pressed: false, keycode: CONTROL_KEYCODE, name: "Control_L" });
    drain(&mut scene.core);
    assert_eq!(scene.desk.lock().submitted, ["ytn"]);
    assert_eq!(scene.desk.lock().dropped_actions, 0);
    scene.ends_clean(&mut bundle);
    // A key the hook reported without what it takes to type it again: its word stays as typed, at a space and at Enter.
    let mut scene = Scene::new(&mut bundle, true, false);
    scene.desk.lock().untypable = Some(key_for('н', 1).expect("a key of the keyboard").keycode);
    scene.text("нет ", 1);
    scene.idle(1.0);
    assert_eq!(scene.shown(), "ytn |");
    scene.text("нет", 1);
    scene.tap(RETURN_KEYCODE, "Return");
    assert_eq!(scene.desk.lock().injections, 0);
    assert_eq!(scene.desk.lock().submitted, ["ytn ytn"]);
    assert_eq!(scene.desk.lock().dropped_actions, 0);
    assert_eq!(scene.core.shared.snapshot().last_error, "");
    scene.ends_clean(&mut bundle);
    bundle.clean();
}

/// An engine that fails in the middle of a correction lets go of the keys the hook holds for it: nothing else would, and
/// with every key held the engine hears of none again.
#[test]
fn a_correction_that_fails_midway_does_not_keep_the_keyboard() {
    let mut bundle = Bundle::load(None);
    let mut scene = Scene::new(&mut bundle, true, false);
    // The word is corrected at its space, not while it is typed.
    scene.core.settings.set("detection.early_switch", false.into(), false).expect("in memory");
    scene.core.apply_commands();
    scene.text("привет", 1);
    scene.desk.lock().panicking_correction = true;
    let space = key_for(' ', 0).expect("the space bar").keycode;
    scene.raw(Raw { pressed: true, keycode: space, name: "" });
    scene.desk.key(Raw { pressed: false, keycode: space, name: "" });
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drain(&mut scene.core)));
    assert!(failed.is_err(), "the turn stops in the correction");
    assert!(scene.desk.lock().holding, "the hook holds the keys for the correction");
    scene.desk.lock().panicking_correction = false;
    scene.core.recover_from_error(&ks_engine::engine::EngineError("the backend failed".to_owned()));
    scene.text("мир", 1);
    assert_eq!(scene.shown(), "ghbdtn vbh|");
    scene.ends_clean(&mut bundle);
    bundle.clean();
}

/// An Enter or Tab the hook keeps back is always answered: the keyboard is never left held for it.
#[test]
fn an_enter_nobody_claims_does_not_keep_the_keyboard() {
    let mut bundle = Bundle::load(None);
    // A hotkey of the program's own set to Tab alone: the hook does not keep it back.
    let mut scene = Scene::new(&mut bundle, true, false);
    scene.core.settings.set("hotkeys.undo", "Tab".into(), false).expect("in memory");
    scene.core.apply_commands();
    scene.core.shared.refresh_hook_settings();
    scene.tap(TAB_KEYCODE, "Tab");
    assert_eq!(scene.desk.lock().deferred_actions, 0);
    scene.ends_clean(&mut bundle);
    // The rule window took the keyboard between the hook's answer and the engine's turn: the Enter goes on.
    let mut scene = Scene::new(&mut bundle, true, false);
    scene.desk.key(Raw { pressed: true, keycode: RETURN_KEYCODE, name: "Return" });
    scene.core.shared.set_rule_editor_open(true);
    drain(&mut scene.core);
    scene.core.shared.set_rule_editor_open(false);
    scene.raw(Raw { pressed: false, keycode: RETURN_KEYCODE, name: "Return" });
    assert_eq!(scene.desk.lock().submitted, [""]);
    scene.ends_clean(&mut bundle);
    // The same with a hook that fails to pass it on: the failure is the engine's to report, the keys are let go.
    let mut scene = Scene::new(&mut bundle, true, false);
    scene.desk.lock().failing_actions = true;
    scene.desk.key(Raw { pressed: true, keycode: RETURN_KEYCODE, name: "Return" });
    scene.core.shared.set_rule_editor_open(true);
    drain(&mut scene.core);
    scene.core.shared.set_rule_editor_open(false);
    scene.raw(Raw { pressed: false, keycode: RETURN_KEYCODE, name: "Return" });
    scene.ends_clean(&mut bundle);
    // The engine fails on the Enter itself, as on an error nobody foresaw: the worker goes on and lets the keys go.
    for failing_actions in [false, true] {
        let mut scene = Scene::new(&mut bundle, true, false);
        scene.desk.lock().panicking = true;
        scene.desk.lock().failing_actions = failing_actions;
        scene.desk.key(Raw { pressed: true, keycode: RETURN_KEYCODE, name: "Return" });
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| scene.core.turn(0.0)));
        assert!(failed.is_err(), "the turn stops on the backend's failure");
        scene.desk.lock().panicking = false;
        scene.core.recover_from_error(&ks_engine::engine::EngineError("the backend failed".to_owned()));
        scene.raw(Raw { pressed: false, keycode: RETURN_KEYCODE, name: "Return" });
        scene.ends_clean(&mut bundle);
    }
    bundle.clean();
}
