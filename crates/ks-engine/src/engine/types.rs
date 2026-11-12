//! The values the engine passes around: plans, snapshots, words waiting for their neighbours.

use crate::constants::{HOTKEY_KEY_ALIASES, HOTKEY_MESSAGE_KEY_PLACE, HOTKEY_MODIFIER_KEYS, HOTKEY_NO_KEY_MESSAGE, HOTKEY_SEVERAL_KEYS_MESSAGE, HOTKEY_UNKNOWN_KEY_MESSAGE};
use ks_backend::{FieldContext, KeyEvent};
use ks_config::learning::RuleAction;
use ks_lang::detector::DetectionDecision;
use ks_text::pystr;
use std::sync::Arc;

/// A key as the engine holds it. The Python compares some of them by identity (`id(stroke)`, `is`): `Arc::ptr_eq`
/// answers that, and `==` compares the values, as for a dataclass.
pub type Stroke = Arc<KeyEvent>;
pub type Strokes = Vec<Stroke>;
pub type PlanRef = Arc<CorrectionPlan>;
/// `dict[int, str]` in its order.
pub type Alternatives = Vec<(i64, String)>;

pub const MODIFIER_KEYS: [&str; 12] = [
    "Shift_L", "Shift_R", "Control_L", "Control_R", "Alt_L", "Alt_R", "Meta_L", "Meta_R", "Super_L", "Super_R", "ISO_Level3_Shift",
    "Caps_Lock",
];
pub const NAVIGATION_KEYS: [&str; 12] =
    ["Left", "Right", "Up", "Down", "Home", "End", "Page_Up", "Page_Down", "Escape", "Delete", "Insert", "Pointer"];
pub const PUNCTUATION: &str = ".,!?;:()[]{}—–-…\"«»";
// Keys that answer the learning prompt: while it is shown they belong to
// KeySwitch, not to the text being typed.
pub const PROMPT_KEYS: [&str; 3] = ["Return", "KP_Enter", "Escape"];
pub const WORD_BOUNDARY_KEYS: [&str; 4] = ["space", "Return", "Tab", "ISO_Left_Tab"];
pub const ACTION_BOUNDARY_KEYS: [&str; 4] = ["Return", "KP_Enter", "Tab", "ISO_Left_Tab"];
pub const WORD_JOINERS: [&str; 5] = ["'", "’", "-", "‐", "‑"];

#[inline]
pub(crate) fn is_modifier_key(name: &str) -> bool {
    MODIFIER_KEYS.contains(&name)
}

#[inline]
pub(crate) fn is_navigation_key(name: &str) -> bool {
    NAVIGATION_KEYS.contains(&name)
}

#[inline]
pub(crate) fn is_action_boundary_key(name: &str) -> bool {
    ACTION_BOUNDARY_KEYS.contains(&name)
}

/// What a lone tap of a modifier does: the setting the key is chosen in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TapAction {
    /// `detection.layout_switch_key`: the layout switches.
    SwitchLayout,
    /// `detection.convert_last_key`: what a press of the conversion hotkey does.
    ConvertLast,
}

/// A lone press of the layout switch key (`detection.layout_switch_key`) or of the conversion key
/// (`detection.convert_last_key`), from its first press until it comes up or anything else happens (`layout_tap`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutTap {
    pub keycode: u32,
    /// `monotonic()` at the first press: repeats of the held key leave it.
    pub pressed_at: f64,
    /// The layout the press came with.
    pub group: i64,
    /// The setting the key was chosen in when it went down.
    pub action: TapAction,
}

/// What the focus probe found: another window, or one whose layout is moot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FocusChange {
    pub changed: bool,
    pub ignore_layout: bool,
}

#[derive(Clone, Debug)]
pub struct CorrectionPlan {
    pub strokes: Strokes,
    pub boundary: Option<Stroke>,
    pub source_group: i64,
    pub target_group: i64,
    pub original: String,
    pub replacement: String,
    pub confidence: f64,
    pub application: String,
    pub automatic: bool,
    // boundary | pause | manual | undo | early | symbols | late_stroke
    // | mention_shown | mention_hidden | context_phrase | context_word | sign | case | typo
    pub mode: &'static str,
    pub context_field: String,
    // Literal punctuation before `boundary`, not replayed in the new layout.
    pub trailing: Strokes,
    // A symbol typed in front of the word that this application reads as the
    // start of a mention. It is judged with the word, not on its own, so it is
    // kept out of `strokes` until the correction runs (execute_correction).
    pub head: Strokes,
    // A word converted at a space: its baseline decision and field, so the model can be
    // asked about it again once the next word shows which way that one went
    // (revert_with_next_word).
    pub revisit: Option<Arc<(DetectionDecision, Option<FieldContext>)>>,
    // The boundary sign was folded into `strokes` and is replayed in the new layout
    // (_replay_sign_with_word): the word ended there, though no boundary follows it.
    pub sign_replayed: bool,
    // A phrase whose last word the early switch converted while it was typed shows letters
    // of both layouts, and `original` is that text. Undo types every key in `source_group`;
    // this is the text it leaves there, the keys as typed.
    pub typed: String,
    // The last strokes of `strokes` that stay in `source_group`: a waiting word converted alone
    // before its next word, which is typed again as it was (decide_with_kept_neighbour). The
    // layout stays `source_group`.
    pub kept_tail: usize,
    // The last strokes of `strokes` typed in `target_group` already, in a layout the user picked by hand for them
    // (decide_before_picked_word): converted with the rest they stay as typed, and the plans that put the text back
    // (undo, Pause right after) keep them as their `kept_tail`, typed again in the layout they were typed in.
    pub picked_tail: usize,
    // The keys on screen that `strokes` replace when they are other keys: a typo correction
    // (mode "typo", typo_plan) types the corrected word over the keys typed, and its undo types
    // those back. Empty: `strokes` are the keys on screen, as for every other correction.
    pub replaced_strokes: Strokes,
    // The keys of the text this plan replaces, when they are not `strokes`: a case fix types the keys
    // of the word with one letter's Shift changed (fix_double_capitals), and undo types these back.
    // Empty when the plan replaces `strokes` themselves, as every conversion does.
    pub replaced: Strokes,
}

impl CorrectionPlan {
    /// The positional fields; `automatic` True and `mode` "boundary" are the defaults of the dataclass.
    #[must_use]
    pub fn new(
        strokes: Strokes, boundary: Option<Stroke>, source_group: i64, target_group: i64, original: String, replacement: String,
        confidence: f64, application: &str,
    ) -> CorrectionPlan {
        CorrectionPlan {
            strokes,
            boundary,
            source_group,
            target_group,
            original,
            replacement,
            confidence,
            application: application.to_owned(),
            automatic: true,
            mode: "boundary",
            context_field: String::new(),
            trailing: Vec::new(),
            head: Vec::new(),
            revisit: None,
            sign_replayed: false,
            typed: String::new(),
            kept_tail: 0,
            picked_tail: 0,
            replaced_strokes: Vec::new(),
            replaced: Vec::new(),
        }
    }

    /// The plan changes the case of letters in their own layout: a case fix, or its undo.
    #[must_use]
    pub fn case_fix(&self) -> bool {
        !self.replaced.is_empty()
    }

    /// `dataclasses.replace(plan, ...)`.
    pub fn replaced_with(&self, change: impl FnOnce(&mut CorrectionPlan)) -> PlanRef {
        let mut plan = self.clone();
        change(&mut plan);
        Arc::new(plan)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EngineSnapshot {
    pub running: bool,
    pub enabled: bool,
    pub backend: String,
    pub current_group: i64,
    pub current_word: String,
    pub correction_count: i64,
    pub last_action: String,
    pub last_error: String,
    pub context_action: String,
    pub context_model: String,
}

impl Default for EngineSnapshot {
    fn default() -> Self {
        EngineSnapshot {
            running: false,
            enabled: true,
            backend: "остановлен".to_owned(),
            current_group: -1,
            current_word: String::new(),
            correction_count: 0,
            last_action: "Ожидание ввода".to_owned(),
            last_error: String::new(),
            context_action: String::new(),
            context_model: String::new(),
        }
    }
}

/// The keyword arguments of `update`.
#[derive(Default)]
pub struct Update {
    pub running: Option<bool>,
    pub enabled: Option<bool>,
    pub backend: Option<String>,
    pub current_group: Option<i64>,
    pub current_word: Option<String>,
    pub correction_count: Option<i64>,
    pub last_action: Option<String>,
    pub last_error: Option<String>,
    pub context_action: Option<String>,
    pub context_model: Option<String>,
}

#[derive(Clone, Debug)]
pub struct LanguageContext {
    pub group: i64,
    pub words: Alternatives,
    pub updated_at: f64,
}

/// The letters that already stood right before and after the caret when a word began there.
#[derive(Clone, Debug)]
pub struct InsertionPoint {
    pub head: String,
    pub tail: String,
}

impl InsertionPoint {
    #[must_use]
    pub fn inside_word(&self) -> bool {
        !self.head.is_empty() || !self.tail.is_empty()
    }
}

#[derive(Clone, Debug)]
pub struct WaitingContextWord {
    pub plan: PlanRef,
    pub decision: DetectionDecision,
    pub field: FieldContext,
    pub window: i64,
    pub deadline: f64,
    pub diagnostic_id: i64,
    // A word the model only suggested converting is asked again when its neighbour
    // arrives, never at a pause: a pause brings no new context to decide with.
    pub settles_on_pause: bool,
    // The detector's verdict on the word at its own boundary: the baseline the model saw there,
    // and the one it is asked with again beside a kept next word.
    pub baseline: Option<DetectionDecision>,
    // The word was converted at its boundary and waits only in case the next word decides it again (start_context_wait's
    // fallback): nothing is left to convert when no next word comes.
    pub converted: bool,
}

// The baseline reason of the question a converted word is asked again (revert_with_next_word).
pub const REVISIT_REASON: &str = "слово после соседа, переведённого обратно";
// The words of a line read by the stranded-letter rule (stranded_letter): signs that end a
// sentence, and the two scripts a word may be written in.
pub const SENTENCE_END_SIGNS: &str = ".!?…";

/// An applied conversion of a word that the next word may still take back.
#[derive(Clone, Debug)]
pub struct ProvisionalWord {
    pub plan: PlanRef,
    pub window: i64,
    pub deadline: f64,
}

/// A word left as typed at a space that the next word may still take along.
///
/// `baseline` is the detector's decision, or None for a token that was not decided at all.
/// `segmentation_open` marks one of those whose last keys may be letters or signs: the
/// boundary model could not split it (`dc\``: `всё` or `вс` and a backtick), or split a
/// sign off a word that only reads as one with it (`t\``: `её`). A token the code guard,
/// an exclusion or a rule kept from segmentation is undecided without being open.
#[derive(Clone, Debug)]
pub struct KeptWord {
    pub plan: PlanRef,
    pub baseline: Option<DetectionDecision>,
    pub field: Option<FieldContext>,
    pub window: i64,
    pub deadline: f64,
    pub segmentation_open: bool,
}

/// The rule a double press of the conversion hotkey offers to set up.
///
/// ``original`` is the word as typed in ``source_group`` and ``replacement`` the same
/// keys in ``target_group``; ``action`` is what the rule would do with it. An empty
/// ``original`` offers an empty rule window: there was no word to start from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LearningPrompt {
    pub source_group: i64,
    pub target_group: i64,
    pub original: String,
    pub replacement: String,
    pub application: String,
    pub action: RuleAction,
}

/// A correction waiting for its field to show the word it replaces (`waits_for_field`).
#[derive(Clone, Debug)]
pub struct FieldWait {
    pub plan: PlanRef,
    pub learning_prompt: Option<LearningPrompt>,
    pub deadline: f64,
    /// The keys the engine had answered when the wait began (`EngineCore::presses_handled`), and how many of the keys
    /// typed after the word were letters of the next one by then (`rolled_after`): see `typed_on_since`.
    pub presses: u64,
    pub rolled: usize,
}

/// What a press of the conversion hotkey did: the plan it scheduled, or a layout toggle.
#[derive(Clone, Debug)]
pub enum ConvertOutcome {
    Plan(PlanRef),
    Switch,
}

/// A plan `execute_correction` carried out, as it was asked to run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutedPlan {
    pub mode: &'static str,
    pub original: String,
    pub replacement: String,
    pub automatic: bool,
}

const HOTKEY_CTRL: u8 = 1;
const HOTKEY_ALT: u8 = 2;
const HOTKEY_SHIFT: u8 = 4;
const HOTKEY_SUPER: u8 = 8;

/// The bit of a modifier as a hotkey of the settings names it, the name casefolded; None for the name of a key.
fn hotkey_modifier(piece: &str) -> Option<u8> {
    match piece {
        "ctrl" | "control" => Some(HOTKEY_CTRL),
        "alt" => Some(HOTKEY_ALT),
        "shift" => Some(HOTKEY_SHIFT),
        "super" | "meta" => Some(HOTKEY_SUPER),
        _ => None,
    }
}

/// The names the key of a hotkey goes by, casefolded: its own and those `HOTKEY_KEY_ALIASES` gives it.
fn key_names(key: &str) -> impl Iterator<Item = &str> {
    let aliases = HOTKEY_KEY_ALIASES.iter().find(|(name, _)| *name == key).map_or(&[][..], |(_, aliases)| *aliases);
    std::iter::once(key).chain(aliases.iter().copied())
}

/// What stops a hotkey typed into the settings from ever firing (`Hotkey::problem`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HotkeyProblem {
    /// No key besides the modifiers, or a modifier key in the key's place: `Ctrl`, `Ctrl+Alt`, `Shift_L`. A modifier key
    /// alone is no hotkey: it would fire, where it fired at all, on every press that begins `Ctrl+C`.
    NoKey,
    /// More keys than one besides the modifiers: `Ctrl+A+B`.
    SeveralKeys,
    /// A key by a name this system's keyboard has none of, as it was typed: `Левый Ctrl`, `Pausee`.
    UnknownKey(String),
}

impl HotkeyProblem {
    /// The problem in the words the settings window shows.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            HotkeyProblem::NoKey => HOTKEY_NO_KEY_MESSAGE.to_owned(),
            HotkeyProblem::SeveralKeys => HOTKEY_SEVERAL_KEYS_MESSAGE.to_owned(),
            HotkeyProblem::UnknownKey(key) => HOTKEY_UNKNOWN_KEY_MESSAGE.replace(HOTKEY_MESSAGE_KEY_PLACE, key),
        }
    }
}

/// A hotkey of the settings. The set of modifiers is held as bits, `control` and `meta` under their other names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hotkey {
    modifiers: u8,
    pub key: String,
}

impl Hotkey {
    #[must_use]
    pub fn new(value: &str) -> Hotkey {
        let text = value.replace('<', "").replace('>', "+");
        let mut modifiers = 0;
        let mut key = String::new();
        for piece in text.split('+') {
            if pystr::strip(piece).is_empty() {
                continue;
            }
            let piece = pystr::casefold(pystr::strip(piece));
            match hotkey_modifier(&piece) {
                Some(modifier) => modifiers |= modifier,
                None => key = piece,
            }
        }
        Hotkey { modifiers, key }
    }

    /// What stops `value` from ever firing as a hotkey, for the settings window to say before it is stored; None for a
    /// hotkey that can fire. `key_exists` says whether the keyboard of this system has a key of a name, given as a hotkey
    /// holds it (casefolded). The window never stored an empty hotkey and does not ask about one.
    pub fn problem(value: &str, key_exists: impl Fn(&str) -> bool) -> Option<HotkeyProblem> {
        let text = value.replace('<', "").replace('>', "+");
        let mut keys = text.split('+').map(pystr::strip).filter(|piece| !piece.is_empty() && hotkey_modifier(&pystr::casefold(piece)).is_none());
        let Some(key) = keys.next() else { return Some(HotkeyProblem::NoKey) };
        if keys.next().is_some() {
            return Some(HotkeyProblem::SeveralKeys);
        }
        let name = pystr::casefold(key);
        if HOTKEY_MODIFIER_KEYS.contains(&name.as_str()) {
            return Some(HotkeyProblem::NoKey);
        }
        if key_names(&name).any(&key_exists) { None } else { Some(HotkeyProblem::UnknownKey(key.to_owned())) }
    }

    #[must_use]
    pub fn matches(&self, event: &KeyEvent) -> bool {
        if !event.pressed || self.key.is_empty() {
            return false;
        }
        let actual = (u8::from(event.control()) * HOTKEY_CTRL)
            | (u8::from(event.alt()) * HOTKEY_ALT)
            | (u8::from(event.shift()) * HOTKEY_SHIFT)
            | (u8::from(event.super_key()) * HOTKEY_SUPER);
        if actual != self.modifiers {
            return false;
        }
        let key_name = pystr::casefold(&event.key_name);
        key_names(&self.key).any(|name| name == key_name)
    }
}
