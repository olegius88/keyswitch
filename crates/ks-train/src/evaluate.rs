//! A sealed candidate measured against the frozen baseline pair by typing whole sequences through the engine
//! (`evaluate_context_action_sequences`). The test split is read here and nowhere else: only after the access is
//! claimed in the ledger, once for a test, and never for a test that shares a row, a family or a document with a test
//! read before.
use ks_cast::To as _;
use crate::corpus::{CorpusRow, DEVELOPMENT, checksum, load_rows, read_json};
use crate::physical::{PhysicalKey, hex, hex_digest, key_of};
use crate::replay::{Mode, Plan, Replayer, prefix_indexes};
use crate::sequence_report::{AllPairs, Block, Case, EarlyOff, FirstBlock, FirstGates, Gated, Gates, Documents, InstalledReference, PairCounts, Profile, Profiles, Report, Selection, Unsupported};
use crate::tools::{Lexicons, Tools, reference_models};
use ks_context::VersionedPrefixModel;
use ks_context::context_model::ContextModel;
use ks_engine::engine::EngineCore;
use ks_lang::Resources;
use ks_lang::ortho_model::{OrthoModel, artifact_path};
use ks_text::pystr;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const CORPUS_BEFORE_WINDOW_CHARACTERS: usize = 96;
const CORPUS_AFTER_WINDOW_CHARACTERS: usize = 64;
const SEQUENCE_DOCUMENT_CAP_PER_GROUP: usize = 512;
const MINIMUM_SEQUENCE_DOCUMENTS_PER_GROUP: i64 = 32;
pub const PROFILES: [&str; 2] = ["portable", "reference_hunspell"];
pub const BASELINE: &str = "model/context_v3/baseline-context-v1.json";
pub const PREFIX_BASELINE: &str = "model/prefix_v2/baseline-prefix-v1.json";
const SEALED_BEFORE_TEST: &str = "sealed-before-test";
/// The file of the audited sequence protocols, every version a receipt may name.
pub const PROTOCOLS: &str = "model/context_v3/protocols.json";
/// The version of the sequence protocol this tree evaluates by.
pub const SEQUENCE_PROTOCOL_VERSION: i64 = 5;
/// The version whose blocks stand at the top of the protocols file, where receipts of that version were checked; later
/// versions stand under `versions`, by number.
pub const TOP_LEVEL_PROTOCOL_VERSION: i64 = 4;
/// The version from which the default mode also types each focus first in a text of its own (`first_plan`), and the
/// default mode's restoration floor is taken from those plans.
pub const FIRST_PLANS_VERSION: i64 = 5;
/// The tree's installed pair, replayed in the default mode as a reference of how early the switch comes; never gated.
pub const INSTALLED: &str = "installed";
pub const INSTALLED_CONTEXT: &str = "resources/models/context_policy_v1.json";
pub const INSTALLED_PREFIX: &str = "resources/models/prefix_policy_v1.json";
/// The kind of correction an early switch makes: the mode of the engine's plan.
pub const EARLY_KIND: &str = "early";
/// The bins of the earliness histogram: the keys from one to this many each, then one bin for every later key.
pub const SWITCH_KEY_HISTOGRAM_KEYS: usize = 8;
/// The bin of every key after the last numbered one.
pub const SWITCH_KEY_HISTOGRAM_LATER: &str = "9+";
/// The names of a pair in each settings mode, with its context and prefix models.
const PAIRS: [(Mode, &[(&str, &str, &str)]); 2] = [
    (Mode::EarlyOff, &[("baseline", "baseline", "baseline"), ("candidate", "candidate", "candidate")]),
    (Mode::Default, &[("baseline", "baseline", "baseline"), ("candidate", "candidate", "candidate"), ("candidate_context_baseline_prefix", "candidate", "baseline"), (INSTALLED, INSTALLED, INSTALLED)]),
];
const MEMBERSHIP_IDENTITY_FIELDS: [&str; 3] = ["row_ids_sha256", "family_ids_sha256", "document_ids_sha256"];

/// `script_group`: `"a" <= character.lower() <= "z"`, and the same for the Russian letters.
pub(crate) fn script_group(character: char) -> Option<i64> {
    let lowered = pystr::lower(character.encode_utf8(&mut [0_u8; 4]));
    let text = lowered.as_str();
    if ("a"..="z").contains(&text) {
        return Some(0);
    }
    if ("а"..="я").contains(&text) || text == "ё" {
        return Some(1);
    }
    None
}

fn is_alpha(character: char) -> bool {
    crate::rows::is_alpha(character)
}

/// `trimmed_window`: at a full window clipped inside letters, only that outer letter fragment goes.
fn trimmed_window(row: &CorpusRow) -> (String, String, usize, usize) {
    let mut before: Vec<char> = row.before.chars().collect();
    let mut after: Vec<char> = row.after.chars().collect();
    let (mut left, mut right) = (0, 0);
    if before.len() == CORPUS_BEFORE_WINDOW_CHARACTERS && is_alpha(before[0]) {
        while left < before.len() && is_alpha(before[left]) {
            left += 1;
        }
        before.drain(..left);
    }
    if after.len() == CORPUS_AFTER_WINDOW_CHARACTERS && is_alpha(after[after.len() - 1]) {
        while right < after.len() && is_alpha(after[after.len() - 1 - right]) {
            right += 1;
        }
        after.truncate(after.len() - right);
    }
    (before.into_iter().collect(), after.into_iter().collect(), left, right)
}

/// `sequence_plan`.
pub fn sequence_plan(row: &CorpusRow, initially_wrong: bool) -> Plan {
    let declared = row.group.filter(|group| *group == 0 || *group == 1);
    let focus_letter = declared.and_then(|group| row.original.chars().position(|character| script_group(character) == Some(group)));
    if initially_wrong {
        assert!(declared.is_some(), "mixed or unclassified focus has no unique wrong layout");
        assert!(focus_letter.is_some(), "wrong intervention requires a focus letter in the declared group");
    }
    let (before, after, left, right) = trimmed_window(row);
    let mut intended = format!("{before}{}", row.original).chars().find_map(script_group).or(row.group).filter(|group| *group != 0).unwrap_or(0);
    let mut keys: Vec<PhysicalKey> = Vec::new();
    let mut unsupported: BTreeSet<String> = BTreeSet::new();
    let mut first = true;
    let mut intervention_event = None;
    for (phase, text) in [("before", before.as_str()), ("focus", row.original.as_str()), ("after", after.as_str())] {
        for (index, character) in text.chars().enumerate() {
            let mut explicit: Option<i64> = None;
            let group = script_group(character);
            if first || group.is_some_and(|value| value != intended) {
                intended = group.unwrap_or(intended);
                explicit = Some(intended);
                first = false;
            }
            if let Some(declared) = declared.filter(|_| phase == "focus") {
                if index == 0 {
                    intended = declared;
                    explicit = Some(declared);
                }
                if Some(index) == focus_letter {
                    intended = declared;
                    explicit = Some(if initially_wrong { 1 - declared } else { declared });
                }
            }
            let Some(chosen) = key_in_layout(character, &mut intended, &mut explicit) else {
                unsupported.insert(format!("U+{:04X}", u32::from(character)));
                continue;
            };
            if phase == "focus" && Some(index) == focus_letter && initially_wrong {
                intervention_event = Some(keys.len() + 1);
            }
            keys.push(chosen);
        }
    }
    let expected = format!("{before}{}{after}", row.original);
    Plan { identifier: row.identifier.clone(), document: row.document.clone(), group: row.group, initially_wrong, expected, keys, unsupported: unsupported.into_iter().collect(), trimmed_left: left, trimmed_right: right, expected_final_group: intended, intervention_event, initial_group: 0, focus_letters: None }
}

/// The key of `character` in the layout the text is typed in, selecting `explicit` if one is set; a glyph only the other
/// layout has is typed there, and its key selects that layout. None for a glyph neither layout has.
fn key_in_layout(character: char, intended: &mut i64, explicit: &mut Option<i64>) -> Option<PhysicalKey> {
    let mut candidate = key_of(character, intended.to::<usize>());
    if candidate.is_none() {
        candidate = key_of(character, (1 - *intended).to::<usize>());
        if candidate.is_some() {
            *intended = 1 - *intended;
            *explicit = Some(*intended);
        }
    }
    let mut chosen = candidate?.clone();
    chosen.explicit_group = explicit.map(ks_cast::To::to::<i32>);
    Some(chosen)
}

/// The letters a word opens with: the run of letters from its start.
#[must_use]
pub fn opening_letters(text: &str) -> usize {
    text.chars().take_while(|character| is_alpha(*character)).count()
}

/// `first_plan` (protocol 5): the focus typed first, in a text of its own: no window before it, the window after it
/// trimmed as `sequence_plan` trims it. The editor opens in the layout the first key is typed in, the declared one for a
/// correct plan and the other one for a wrong plan, and that key selects no layout: a wrong plan's first letter is wrong
/// with no manual choice behind it. A later key selects a layout where the text changes script or has a glyph only the
/// other layout has, as in `sequence_plan`. None for a row whose focus does not open with a letter of its declared layout.
#[must_use]
pub fn first_plan(row: &CorpusRow, initially_wrong: bool) -> Option<Plan> {
    let declared = row.group.filter(|group| *group == 0 || *group == 1)?;
    if row.original.chars().next().and_then(script_group) != Some(declared) {
        return None;
    }
    let (_, after, _, right) = trimmed_window(row);
    let mut intended = declared;
    let mut keys: Vec<PhysicalKey> = Vec::new();
    let mut unsupported: BTreeSet<String> = BTreeSet::new();
    for character in row.original.chars().chain(after.chars()) {
        let mut explicit: Option<i64> = None;
        if let Some(group) = script_group(character).filter(|group| *group != intended) {
            intended = group;
            explicit = Some(group);
        }
        match key_in_layout(character, &mut intended, &mut explicit) {
            Some(chosen) => keys.push(chosen),
            None => {
                unsupported.insert(format!("U+{:04X}", u32::from(character)));
            }
        }
    }
    let initial_group = if initially_wrong { 1 - declared } else { declared };
    Some(Plan {
        identifier: row.identifier.clone(),
        document: row.document.clone(),
        group: row.group,
        initially_wrong,
        expected: format!("{}{after}", row.original),
        keys,
        unsupported: unsupported.into_iter().collect(),
        trimmed_left: 0,
        trimmed_right: right,
        expected_final_group: intended,
        intervention_event: initially_wrong.then_some(1),
        initial_group: initial_group.to::<i32>(),
        focus_letters: Some(opening_letters(&row.original)),
    })
}

/// `select_rows`: hash-ranked documents and one hash-ranked focus per document and group.
#[must_use]
pub fn select_rows(rows: &[CorpusRow]) -> Vec<&CorpusRow> {
    let mut selected = Vec::new();
    for group in [Some(0), Some(1), None] {
        let mut documents: BTreeMap<&str, Vec<&CorpusRow>> = BTreeMap::new();
        for row in rows {
            if row.group == group {
                documents.entry(&row.document).or_default().push(row);
            }
        }
        let mut ranked: Vec<&str> = documents.keys().copied().collect();
        ranked.sort_by_cached_key(|name| (hex_digest(&format!("context-sequence:document:{name}")), (*name).to_owned()));
        for document in ranked.into_iter().take(SEQUENCE_DOCUMENT_CAP_PER_GROUP) {
            let focus = documents[document].iter().min_by_key(|row| (hex_digest(&format!("context-sequence:focus:{}", row.identifier)), row.identifier.clone())).expect("a document holds rows");
            selected.push(*focus);
        }
    }
    selected
}

/// `score_pair`: the cases of every plan a pair replayed, with the counts they give.
fn score_pair(plans: &[Plan], replayer: &mut Replayer, mode: Mode) -> (PairCounts, Vec<Case>) {
    let results: Vec<Case> = plans.iter().filter(|plan| plan.unsupported.is_empty()).map(|plan| Case::new(plan, &replayer.replay(plan, mode))).collect();
    (PairCounts::of_cases(&results), results)
}

fn profile_gates(candidate: &PairCounts, baseline: &PairCounts, documents: &Documents) -> Gates {
    Gates {
        correct_text_preserved: candidate.correct_text_corruptions <= baseline.correct_text_corruptions,
        length_preserved: candidate.length_mismatches == 0,
        net_restorations_at_least_baseline: candidate.exactly_restored - candidate.correct_text_corruptions >= baseline.exactly_restored - baseline.correct_text_corruptions,
        enough_documents_us: documents.us >= MINIMUM_SEQUENCE_DOCUMENTS_PER_GROUP,
        enough_documents_ru: documents.ru >= MINIMUM_SEQUENCE_DOCUMENTS_PER_GROUP,
        execution_succeeded: candidate.execution_errors == 0 && baseline.execution_errors == 0,
        correction_layouts_match: candidate.correction_layout_mismatches == 0,
    }
}

/// One correction as the earliness reads it: the key it came at, the layout it typed, the kind of plan that made it.
#[derive(Clone, Copy, Debug)]
pub struct Switch<'a> {
    pub event: usize,
    pub target_group: i64,
    pub kind: &'a str,
}

/// How early the switch came for a pair (protocol 5). A wrong first plan is counted by K, the key of its first correction
/// into the declared layout (the first key is 1, so K is the number of keys typed when it came), against L, the letters
/// of the word typed first; and by whether that word is right in the final text. Early switches in correctly typed text
/// are counted at the start of a text (correct first plans) and inside one (correct plans of protocol 4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Earliness {
    /// K < L: before the word's last letter.
    pub inside_word: i64,
    /// K = L: at its last letter.
    pub last_letter: i64,
    /// K = L + 1: at the key after it.
    pub boundary_key: i64,
    /// K > L + 1.
    pub later: i64,
    /// No correction typed the declared layout.
    pub never: i64,
    /// The final text up to its first space is the expected text up to its first space.
    pub first_word_correct: i64,
    /// The first correction into the declared layout is of kind early and came by the word's last letter (K <= L), and the
    /// first word is wrong all the same: a right early switch undone later.
    pub started_right_ended_wrong: i64,
    /// K summed over the rows whose first word is right, and their number: the mean key of a switch that ended right.
    pub switch_key_sum: i64,
    pub switch_key_rows: i64,
    /// K of those rows, a bin for each key up to `SWITCH_KEY_HISTOGRAM_KEYS`, then one bin for the rest.
    pub switch_key_histogram: BTreeMap<String, i64>,
    /// The kind of the correction that gives K, over every row that has one.
    pub first_correction_kinds: BTreeMap<String, i64>,
    /// The early corrections of the correct first plans: false early switches at the start of a text.
    pub correct_first_plans: FirstFalseEarly,
    /// The early corrections of the correct protocol-4 plans of the default mode: false early switches inside a text.
    pub correct_sequence_plans: SequenceFalseEarly,
}

/// The early corrections of the texts typed as meant, at the start of a text. A false early switch is a correction of kind
/// early whose target is not the row's layout; one whose target is the row's layout brings the text back after another
/// layer switched it away, and is a return, not a false switch.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FirstFalseEarly {
    /// Rows with a false early switch.
    pub false_early: i64,
    /// Of those, the rows whose first false early switch came by the last letter of the word typed first.
    pub false_early_on_focus: i64,
    /// Of those, the rows whose final text is exact: the switch was undone.
    pub false_early_flicker: i64,
    /// Of those, the rows whose final text is not exact.
    pub false_early_corrupted: i64,
    /// Rows with an early correction into the row's layout: a return after another layer's switch, not a false switch.
    pub early_returns: i64,
}

/// The same inside a text (the correct plans of protocol 4): the plan does not say how many letters the word in focus has,
/// so there is no count on it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SequenceFalseEarly {
    /// Rows with a false early switch.
    pub false_early: i64,
    /// Of those, the rows whose final text is exact.
    pub false_early_flicker: i64,
    /// Of those, the rows whose final text is not exact.
    pub false_early_corrupted: i64,
    /// Rows with an early correction into the row's layout.
    pub early_returns: i64,
}

impl Default for Earliness {
    fn default() -> Earliness {
        let bins = (1..=SWITCH_KEY_HISTOGRAM_KEYS).map(|key| key.to_string()).chain(std::iter::once(SWITCH_KEY_HISTOGRAM_LATER.to_owned()));
        Earliness {
            inside_word: 0,
            last_letter: 0,
            boundary_key: 0,
            later: 0,
            never: 0,
            first_word_correct: 0,
            started_right_ended_wrong: 0,
            switch_key_sum: 0,
            switch_key_rows: 0,
            switch_key_histogram: bins.map(|bin| (bin, 0)).collect(),
            first_correction_kinds: BTreeMap::new(),
            correct_first_plans: FirstFalseEarly::default(),
            correct_sequence_plans: SequenceFalseEarly::default(),
        }
    }
}

/// A text up to its first space.
fn first_word(text: &str) -> &str {
    text.split(|character: char| pystr::isspace(character.encode_utf8(&mut [0_u8; 4]))).next().unwrap_or_default()
}

/// What the early corrections of a text typed as meant, in the layout `group`, say: the key of the first one that went away
/// from that layout (a false switch), and whether one came back into it (a return). A row of no layout (`None`) has none to
/// come back into: every early correction of it goes away.
fn early_corrections(group: Option<i64>, corrections: &[Switch<'_>]) -> (Option<usize>, bool) {
    let early = || corrections.iter().filter(|item| item.kind == EARLY_KIND);
    (early().find(|item| Some(item.target_group) != group).map(|item| item.event), early().any(|item| Some(item.target_group) == group))
}

impl Earliness {
    /// The bin of key `key` in the histogram.
    #[must_use]
    pub fn bin(key: usize) -> String {
        if key <= SWITCH_KEY_HISTOGRAM_KEYS { key.to_string() } else { SWITCH_KEY_HISTOGRAM_LATER.to_owned() }
    }

    /// A wrong first plan whose word of `letters` letters belongs to the layout `declared`.
    pub fn add_wrong(&mut self, declared: i64, letters: usize, expected: &str, actual: &str, corrections: &[Switch<'_>]) {
        let right = first_word(actual) == first_word(expected);
        self.first_word_correct += i64::from(right);
        if let Some(switch) = corrections.iter().find(|item| item.target_group == declared) {
            let key = switch.event;
            let class = match key.cmp(&letters) {
                std::cmp::Ordering::Less => &mut self.inside_word,
                std::cmp::Ordering::Equal => &mut self.last_letter,
                std::cmp::Ordering::Greater if key == letters + 1 => &mut self.boundary_key,
                std::cmp::Ordering::Greater => &mut self.later,
            };
            *class += 1;
            *self.first_correction_kinds.entry(switch.kind.to_owned()).or_insert(0) += 1;
            if right {
                self.switch_key_sum += key.to::<i64>();
                self.switch_key_rows += 1;
                *self.switch_key_histogram.entry(Earliness::bin(key)).or_insert(0) += 1;
            }
        } else {
            self.never += 1;
        }
        let first_into_declared = corrections.iter().find(|item| item.target_group == declared);
        if !right && first_into_declared.is_some_and(|item| item.kind == EARLY_KIND && item.event <= letters) {
            self.started_right_ended_wrong += 1;
        }
    }

    /// A correct first plan of a row of the layout `group`, whose word typed first has `letters` letters; `exact` is whether
    /// its final text is the expected one.
    pub fn add_correct_first(&mut self, group: i64, letters: usize, exact: bool, corrections: &[Switch<'_>]) {
        let (away, returned) = early_corrections(Some(group), corrections);
        let block = &mut self.correct_first_plans;
        block.early_returns += i64::from(returned);
        if let Some(event) = away {
            block.false_early += 1;
            block.false_early_on_focus += i64::from(event <= letters);
            block.false_early_flicker += i64::from(exact);
            block.false_early_corrupted += i64::from(!exact);
        }
    }

    /// A correct plan of protocol 4 in the default mode, of a row of the layout `group` (a row may have none).
    pub fn add_correct_sequence(&mut self, group: Option<i64>, exact: bool, corrections: &[Switch<'_>]) {
        let (away, returned) = early_corrections(group, corrections);
        let block = &mut self.correct_sequence_plans;
        block.early_returns += i64::from(returned);
        if away.is_some() {
            block.false_early += 1;
            block.false_early_flicker += i64::from(exact);
            block.false_early_corrupted += i64::from(!exact);
        }
    }
}

/// The corrections of a case as the earliness reads them.
fn switches(case: &Case) -> Result<Vec<Switch<'_>>, String> {
    case.corrections.iter().map(|item| Ok(Switch { event: item.event, target_group: item.target_group, kind: item.kind.as_deref().ok_or("a case correction without its kind")? })).collect()
}

/// `earliness`: a pair's earliness from its cases: those of its first plans, and those of its protocol-4 plans in the
/// default mode.
pub fn earliness(first: &[Case], sequence: &[Case]) -> Result<Earliness, String> {
    let mut total = Earliness::default();
    for case in first {
        let corrections = switches(case)?;
        if case.initially_wrong {
            let declared = case.group.ok_or("a wrong first case without its layout")?;
            let letters = case.focus_letters.and_then(|letters| usize::try_from(letters).ok()).ok_or("a first case without the letters of its word")?;
            total.add_wrong(declared, letters, &case.expected, &case.actual, &corrections);
        } else {
            let group = case.group.ok_or("a correct first case without its layout")?;
            let letters = case.focus_letters.and_then(|letters| usize::try_from(letters).ok()).ok_or("a first case without the letters of its word")?;
            total.add_correct_first(group, letters, case.exact, &corrections);
        }
    }
    for case in sequence.iter().filter(|case| !case.initially_wrong) {
        total.add_correct_sequence(case.group, case.exact, &switches(case)?);
    }
    Ok(total)
}

/// The models of an evaluation.
pub struct Pairs {
    pub candidate: PathBuf,
    pub baseline: PathBuf,
    pub prefix_candidate: PathBuf,
    pub prefix_baseline: PathBuf,
    /// The tree's installed pair: the default mode's reference.
    pub installed: PathBuf,
    pub prefix_installed: PathBuf,
}

/// The plans of an evaluation: protocol 4's for every selected row, and the first plans of protocol 5.
pub struct SequencePlans<'a> {
    pub sequence: Vec<Plan>,
    /// For each selected US or RU row that protocol 4 types and whose focus opens with a letter of its layout: the
    /// correct plan, then the wrong one.
    pub first: Vec<Plan>,
    /// The US or RU rows protocol 4 types whose focus opens otherwise: they have no first plans.
    pub first_ineligible: Vec<&'a str>,
}

/// `sequence_plans`.
#[must_use]
pub fn sequence_plans<'a>(selected: &[&'a CorpusRow]) -> SequencePlans<'a> {
    let mut plans = SequencePlans { sequence: Vec::new(), first: Vec::new(), first_ineligible: Vec::new() };
    for &row in selected {
        let correct = sequence_plan(row, false);
        let typed = correct.unsupported.is_empty();
        plans.sequence.push(correct);
        if matches!(row.group, Some(0 | 1)) {
            plans.sequence.push(sequence_plan(row, true));
            if typed {
                if let (Some(right), Some(wrong)) = (first_plan(row, false), first_plan(row, true)) {
                    // Every glyph of a first plan stands in its row's window, which protocol 4 types.
                    assert!(right.unsupported.is_empty(), "a first plan types part of a window that is typed whole");
                    plans.first.push(right);
                    plans.first.push(wrong);
                } else {
                    plans.first_ineligible.push(row.identifier.as_str());
                }
            }
        }
    }
    plans
}

/// Which plans a task replays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Sequence,
    First,
}

/// One pair's replay of one part of the plans, in one profile and settings mode.
#[derive(Clone, Copy)]
struct Task<'a> {
    profile: usize,
    mode: Mode,
    context: &'a str,
    prefix: &'a str,
    part: Part,
}

/// `score_sequences`: every pair replayed in both settings modes, and the first plans in the default mode; acceptance
/// needs every gate in every mode and profile.
pub fn score_sequences(root: &Path, resources: &Arc<Resources>, tools: &Tools, spelling: &Lexicons, rows: &[CorpusRow], pairs: &Pairs, jobs: usize) -> Result<Replayed, String> {
    let selected = select_rows(rows);
    let SequencePlans { sequence: plans, first: first_plans, first_ineligible } = sequence_plans(&selected);
    let distinct = |group: Option<i64>| -> i64 {
        let documents: BTreeSet<&str> = plans.iter().filter(|plan| plan.group == group && plan.unsupported.is_empty()).map(|plan| plan.document.as_str()).collect();
        documents.len().to::<i64>()
    };
    let documents = Documents { us: distinct(Some(0)), ru: distinct(Some(1)), none: distinct(None) };
    let selection = Selection {
        source_ids: selected.iter().map(|row| row.identifier.clone()).collect(),
        selected_rows: selected.len().to::<i64>(),
        replayed_documents: documents.clone(),
        trimmed_rows: plans.iter().filter(|plan| !plan.initially_wrong && (plan.trimmed_left != 0 || plan.trimmed_right != 0)).count().to::<i64>(),
        unsupported: plans.iter().filter(|plan| !plan.unsupported.is_empty() && !plan.initially_wrong).map(|plan| Unsupported { identifier: plan.identifier.clone(), codepoints: plan.unsupported.clone() }).collect(),
        first_ineligible: first_ineligible.iter().map(|identifier| (*identifier).to_owned()).collect(),
    };
    let first = &tools.portable;
    let known = EngineCore::known_of(first);
    let shared = match OrthoModel::load(&artifact_path(resources.resource_dir()), Some(resources.packaged_identifier()), known, Some(resources.clone())) {
        Ok(model) => {
            let version = model.version.clone();
            (Some(Arc::new(model)), version)
        }
        Err(error) => (None, format!("unavailable: {}", error.name)),
    };
    let models_dir = resources.resource_dir().join("models");
    let load_context = |path: &Path| ContextModel::load_artifact(path, &models_dir).map_err(|error| format!("{}: {}", path.display(), error.name));
    let load_prefix = |path: &Path| VersionedPrefixModel::load(path).map(Arc::new).map_err(|error| format!("{}: {}", path.display(), error.name));
    let (prefix_candidate, prefix_baseline, prefix_installed) = (load_prefix(&pairs.prefix_candidate)?, load_prefix(&pairs.prefix_baseline)?, load_prefix(&pairs.prefix_installed)?);
    let indexes = [prefix_indexes(first), prefix_indexes(spelling)];
    // Tasks are listed in the order the report is put together, and run longest first, so that no worker is left with a
    // long one at the end.
    let mut tasks: Vec<Task<'_>> = Vec::new();
    for profile in 0..PROFILES.len() {
        for (mode, listed) in PAIRS {
            for (_, context, prefix) in listed {
                tasks.push(Task { profile, mode, context, prefix, part: Part::Sequence });
                if mode == Mode::Default {
                    tasks.push(Task { profile, mode, context, prefix, part: Part::First });
                }
            }
        }
    }
    let plans_of = |part: Part| if part == Part::First { &first_plans } else { &plans };
    let keys_of = |part: Part| plans_of(part).iter().filter(|plan| plan.unsupported.is_empty()).map(|plan| plan.keys.len()).sum::<usize>();
    let mut order: Vec<usize> = (0..tasks.len()).collect();
    order.sort_by_cached_key(|index| std::cmp::Reverse(keys_of(tasks[*index].part)));
    let next = AtomicUsize::new(0);
    let done: Mutex<Vec<Option<Scored>>> = Mutex::new((0..tasks.len()).map(|_| None).collect());
    let failure: Mutex<Option<String>> = Mutex::new(None);
    std::thread::scope(|scope| {
        for worker in 0..jobs.max(1).min(tasks.len()) {
            let (tasks, order, next, done, failure, plans_of, shared, indexes, load_context) = (&tasks, &order, &next, &done, &failure, &plans_of, &shared, &indexes, &load_context);
            let (prefix_candidate, prefix_baseline, prefix_installed) = (&prefix_candidate, &prefix_baseline, &prefix_installed);
            scope.spawn(move || {
                while let Some(index) = order.get(next.fetch_add(1, Ordering::Relaxed)).copied() {
                    let task = tasks[index];
                    let path = match task.context {
                        "candidate" => &pairs.candidate,
                        INSTALLED => &pairs.installed,
                        _ => &pairs.baseline,
                    };
                    let model = match load_context(path) {
                        Ok(model) => model,
                        Err(error) => {
                            *failure.lock().expect("a worker finished") = Some(error);
                            break;
                        }
                    };
                    let prefix_model = match task.prefix {
                        "candidate" => prefix_candidate.clone(),
                        INSTALLED => prefix_installed.clone(),
                        _ => prefix_baseline.clone(),
                    };
                    let models = if task.profile == 0 { first } else { spelling };
                    let folder = std::env::temp_dir().join(format!("ks-evaluate-{}-{worker}", std::process::id()));
                    let mut replayer = Replayer::new(tools, models, shared.clone(), folder.clone(), model, Some(prefix_model), indexes[task.profile].clone());
                    let scored = score_pair(plans_of(task.part), &mut replayer, task.mode);
                    let _ = std::fs::remove_dir_all(folder);
                    done.lock().expect("a worker finished")[index] = Some(scored);
                }
            });
        }
    });
    if let Some(error) = failure.into_inner().expect("the workers finished") {
        return Err(error);
    }
    let mut scored = done.into_inner().expect("the workers finished").into_iter();
    let mut next_scored = || scored.next().flatten().ok_or("a task was not scored");
    let mut all_passed = true;
    let mut profiles = Vec::new();
    for _ in PROFILES {
        let (mut control, mut sequences, mut firsts): (Replays<'_>, Replays<'_>, Replays<'_>) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
        for (mode, listed) in PAIRS {
            for (name, _, _) in listed {
                let replayed = next_scored()?;
                if mode == Mode::Default {
                    sequences.insert(name, replayed);
                    firsts.insert(name, next_scored()?);
                } else {
                    control.insert(name, replayed);
                }
            }
        }
        let early_off = {
            let (baseline, candidate) = (pulled(&mut control, "baseline")?, pulled(&mut control, "candidate")?);
            let gates = profile_gates(&candidate.0, &baseline.0, &documents);
            all_passed = all_passed && gates.all_held();
            EarlyOff { counts: Gated { baseline: baseline.0, candidate: candidate.0 }, cases: Gated { baseline: baseline.1, candidate: candidate.1 }, gates }
        };
        // The earliness of each pair reads its first plans and its protocol-4 plans of the default mode.
        let early = AllPairs::build(|name| earliness(&firsts[name].1, &sequences[name].1))?;
        let replayed = AllPairs::build(|name| pulled(&mut firsts, name))?;
        let gates = FirstGates::of(&replayed.candidate.0, &replayed.baseline.0);
        all_passed = all_passed && gates.all_held();
        let first = FirstBlock { counts: replayed.map(|(counts, _)| counts.clone()), earliness: early, cases: replayed.map(|(_, cases)| cases.clone()), gates };
        let (baseline, candidate, installed) = (pulled(&mut sequences, "baseline")?, pulled(&mut sequences, "candidate")?, pulled(&mut sequences, INSTALLED)?);
        let gates = profile_gates(&candidate.0, &baseline.0, &documents);
        all_passed = all_passed && gates.all_held();
        let ablation: BTreeMap<String, Block> = sequences.into_iter().map(|(name, (counts, cases))| (name.to_owned(), Block { counts, cases })).collect();
        profiles.push(Profile {
            counts: Gated { baseline: baseline.0, candidate: candidate.0 },
            cases: Gated { baseline: baseline.1, candidate: candidate.1 },
            gates,
            ablation: (!ablation.is_empty()).then_some(ablation),
            installed: Some(Block { counts: installed.0, cases: installed.1 }),
            first: Some(first),
            early_off,
        });
    }
    let _ = root;
    let [portable, reference_hunspell]: [Profile; 2] = profiles.try_into().map_err(|_| "a profile was not scored")?;
    Ok(Replayed { selection, profiles: Profiles { portable, reference_hunspell }, promotion_passed: all_passed })
}

/// What `score_sequences` makes of an evaluation: what was selected, every profile's blocks, and whether every gate held.
pub struct Replayed {
    pub selection: Selection,
    pub profiles: Profiles,
    pub promotion_passed: bool,
}

/// The counts and the cases a pair's replay of some plans gave.
type Scored = (PairCounts, Vec<Case>);

/// The scores of the pairs of a block, by name.
type Replays<'a> = BTreeMap<&'a str, Scored>;

/// The scores of a pair, taken out of its block.
fn pulled(block: &mut Replays<'_>, name: &str) -> Result<Scored, String> {
    block.remove(name).ok_or_else(|| format!("{name} was not scored"))
}

fn hash_list(value: &Value, name: &str) -> Result<Vec<String>, String> {
    let items = value.as_array().filter(|items| !items.is_empty()).ok_or_else(|| format!("invalid test membership: {name}"))?;
    let mut entries = Vec::new();
    for item in items {
        let text = item.as_str().filter(|text| text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))).ok_or_else(|| format!("invalid test membership: {name}"))?;
        entries.push(text.to_owned());
    }
    if entries.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err("test membership must be sorted and unique".to_owned());
    }
    Ok(entries)
}

/// `test_membership`.
fn test_membership(corpus: &Path) -> Result<Value, String> {
    let manifest = read_json(&corpus.join("manifest.json"))?;
    let path = corpus.join("test-membership.json");
    if manifest.get("test_membership_sha256") != Some(&Value::String(checksum(&path)?)) {
        return Err("test membership differs from manifest".to_owned());
    }
    let membership = read_json(&path)?;
    let mut result = Map::new();
    for field in MEMBERSHIP_IDENTITY_FIELDS.iter().copied().chain(membership.get("alias_sha256").map(|_| "alias_sha256")) {
        result.insert(field.to_owned(), json!(hash_list(&membership[field], field)?));
    }
    Ok(Value::Object(result))
}

/// What must not change between the seal and the end of a replay.
pub struct Identity {
    pub value: Value,
    pub key: String,
}

pub fn evaluation_identity(request: &EvaluationRequest<'_>) -> Result<Identity, String> {
    let root = request.root;
    let membership = test_membership(request.corpus)?;
    let key = hex(&Sha256::digest(crate::artifact::canonical(&membership)));
    let runtime = crate::fit::provenance(root, &root.join("model/context_v3/recipe.json"))?;
    let value = json!({"schema_version": 1, "artifact_sha256": checksum(request.artifact)?, "candidate_seal_sha256": checksum(request.seal)?, "corpus_manifest_sha256": checksum(&request.corpus.join("manifest.json"))?,
        "test_membership": membership, "baseline_sha256": checksum(&root.join(BASELINE))?, "prefix_candidate_sha256": checksum(request.prefix)?, "prefix_candidate_seal_sha256": checksum(request.prefix_seal)?,
        "prefix_baseline_path": PREFIX_BASELINE, "prefix_baseline_sha256": checksum(&root.join(PREFIX_BASELINE))?, "settings_modes": ["early_off", "default"], "runtime": runtime, "protocol": protocol(root)?});
    Ok(Identity { value, key })
}

/// The blocks of a version of the sequence protocol: `sequence`, `receipt`, `scope` and the audited hash of `sequence`.
/// Version 4 stands at the top of the file, where the receipts of that version were checked; a later one under
/// `versions`. None when the file has no such version, or the block there names another.
#[must_use]
pub fn protocol_set(protocols: &Value, version: i64) -> Option<&Value> {
    let set = if version == TOP_LEVEL_PROTOCOL_VERSION { protocols } else { &protocols["versions"][version.to_string()] };
    (set["sequence"]["version"].as_i64() == Some(version)).then_some(set)
}

/// The sequence block of a version, in the words of the audited protocol, whose bytes its hash pins.
pub fn audited_sequence(protocols: &Value, version: i64) -> Result<Value, String> {
    let set = protocol_set(protocols, version).ok_or_else(|| format!("no sequence protocol of version {version}"))?;
    let sequence = set["sequence"].clone();
    if set["audited_sequence_protocol_sha256"].as_str() != Some(hex(&Sha256::digest(crate::artifact::canonical(&sequence))).as_str()) {
        return Err("sequence protocol changed; public receipt protocol must be reviewed".to_owned());
    }
    Ok(sequence)
}

/// The protocol a report states: the current version's.
pub fn protocol(root: &Path) -> Result<Value, String> {
    audited_sequence(&read_json(&root.join(PROTOCOLS))?, SEQUENCE_PROTOCOL_VERSION)
}

/// `immutable_record`: create a record, or verify the exact existing bytes.
fn immutable_record<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<(), String> {
    let encoded = crate::artifact::canonical_of(value);
    if path.exists() {
        if std::fs::read(path).map_err(|error| error.to_string())? != encoded {
            return Err(format!("immutable test record differs: {}", path.display()));
        }
        return Ok(());
    }
    std::fs::create_dir_all(path.parent().ok_or("a record outside a folder")?).map_err(|error| error.to_string())?;
    let pending = path.with_extension("pending");
    std::fs::write(&pending, &encoded).map_err(|error| error.to_string())?;
    let linked = std::fs::hard_link(&pending, path);
    let _ = std::fs::remove_file(&pending);
    linked.map_err(|error| error.to_string())
}

/// `claim_test_access`: a test is read once, and never when it shares anything with a test read before.
fn claim_test_access(ledger: &Path, identity: &Identity) -> Result<(), String> {
    std::fs::create_dir_all(ledger).map_err(|error| error.to_string())?;
    let lock = ledger.join(".access-lock");
    std::fs::OpenOptions::new().write(true).create_new(true).open(&lock).map_err(|_| "test access ledger is locked; no test was read".to_owned())?;
    let claimed = (|| -> Result<(), String> {
        let membership = &identity.value["test_membership"];
        let mine = format!("{}.access.json", identity.key);
        let mut paths: Vec<PathBuf> = std::fs::read_dir(ledger).map_err(|error| error.to_string())?.filter_map(|entry| entry.ok().map(|entry| entry.path())).collect();
        paths.sort();
        for path in paths {
            let name = path.file_name().map_or_default(|name| name.to_string_lossy().into_owned());
            if !name.ends_with(".access.json") || name == mine {
                continue;
            }
            let record = read_json(&path)?;
            for field in MEMBERSHIP_IDENTITY_FIELDS {
                let earlier: BTreeSet<&str> = record["test_membership"][field].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                if membership[field].as_array().into_iter().flatten().filter_map(Value::as_str).any(|entry| earlier.contains(entry)) {
                    return Err("test overlaps previously accessed rows, families or documents".to_owned());
                }
            }
        }
        immutable_record(&ledger.join(mine), &identity.value)
    })();
    let _ = std::fs::remove_file(&lock);
    claimed
}

/// The calibration a candidate carries must show a balance, not an absence of error (`validate_calibration`).
#[expect(clippy::float_cmp, reason = "the exact double is the meaning here, as it is in the Python this was checked against")]
fn validate_calibration(value: &Value) -> Result<(), String> {
    let profiles = value.get("by_profile").and_then(Value::as_object).ok_or("calibration must contain both declared lexical profiles")?;
    if profiles.len() != PROFILES.len() || PROFILES.iter().any(|name| !profiles.contains_key(*name)) {
        return Err("calibration must contain both declared lexical profiles".to_owned());
    }
    let count = |row: &Value, field: &str| row.get(field).and_then(Value::as_i64).filter(|number| *number >= 0).ok_or_else(|| format!("calibration.{field} must be a nonnegative integer"));
    let mut sums = [0_i64; 4];
    let fields = ["rows", "false_conversions", "converted_correctly", "convert_rows"];
    for (name, row) in std::iter::once(("aggregate", value)).chain(PROFILES.iter().map(|name| (*name, &profiles[*name]))) {
        let numbers: Vec<i64> = fields.iter().map(|field| count(row, field)).collect::<Result<_, _>>()?;
        let (rows, wrong, right, possible) = (numbers[0], numbers[1], numbers[2], numbers[3]);
        let recall = row.get("conversion_recall").and_then(Value::as_f64).unwrap_or(f64::NAN);
        if possible == 0 || rows < possible || right > possible || recall != right.to::<f64>() / possible.to::<f64>() || wrong > right || right - wrong < 1 {
            return Err(format!("calibration gate failed: {name}"));
        }
        if name == "aggregate" {
            sums = [0; 4];
        } else {
            for (sum, number) in sums.iter_mut().zip(&numbers) {
                *sum += number;
            }
        }
    }
    for (field, sum) in fields.iter().zip(sums) {
        if count(value, field)? != sum {
            return Err("calibration aggregate does not match its profiles".to_owned());
        }
    }
    Ok(())
}

fn check_pins(root: &Path, hashes: &Value, label: &str) -> Result<(), String> {
    for (name, expected) in hashes.as_object().ok_or("the seal names no provenance")? {
        let path = Path::new(name);
        if path.is_absolute() || name.contains("..") {
            return Err(format!("{label} provenance must use repository-relative paths"));
        }
        if Some(checksum(&root.join(path))?.as_str()) != expected.as_str() {
            return Err(format!("{label} provenance changed: {name}"));
        }
    }
    Ok(())
}

/// `validate_candidate_seal`: done before any test byte is read.
pub fn validate_candidate_seal(root: &Path, artifact: &Path, seal_path: &Path, corpus: &Path, models_dir: &Path) -> Result<Value, String> {
    let seal = read_json(seal_path)?;
    if seal.get("schema_version").and_then(Value::as_i64) != Some(1) {
        return Err("unsupported candidate seal".to_owned());
    }
    if seal.get("stage").and_then(Value::as_str) != Some(SEALED_BEFORE_TEST) || seal.get("test_accessed").and_then(Value::as_bool) != Some(false) {
        return Err("candidate is not sealed before test".to_owned());
    }
    if seal.get("artifact_sha256") != Some(&Value::String(checksum(artifact)?)) {
        return Err("candidate artifact differs from seal".to_owned());
    }
    if seal.get("corpus_manifest_sha256") != Some(&Value::String(checksum(&corpus.join("manifest.json"))?)) {
        return Err("corpus differs from candidate seal".to_owned());
    }
    let recipe = read_json(&root.join("model/context_v3/recipe.json"))?;
    if crate::artifact::canonical(&seal["recipe"]) != crate::artifact::canonical(&recipe) {
        return Err("recipe differs from candidate seal".to_owned());
    }
    if crate::artifact::canonical(&seal["gate_policy"]) != crate::artifact::canonical(&recipe["gate_policy"]) || recipe["profiles"] != json!(PROFILES) {
        return Err("candidate gate policy differs from required policy".to_owned());
    }
    check_pins(root, &seal["provenance"], "candidate")?;
    validate_calibration(&seal["calibration"])?;
    let model = ContextModel::load_artifact(artifact, models_dir).map_err(|error| error.name.to_string())?;
    if seal.get("model_version").and_then(Value::as_str) != Some(model.version.as_str()) || seal.get("conversion_threshold").and_then(Value::as_f64) != Some(model.conversion_threshold) {
        return Err("candidate model identity differs from seal".to_owned());
    }
    Ok(seal)
}

/// `validate_prefix_seal`: calibration is mandatory before a sealed test.
pub fn validate_prefix_seal(root: &Path, artifact: &Path, seal_path: &Path, require_calibration: bool) -> Result<Value, String> {
    let seal = read_json(seal_path)?;
    if seal.get("schema_version").and_then(Value::as_i64) != Some(1) {
        return Err("unsupported prefix seal".to_owned());
    }
    if seal.get("promotion_accepted").and_then(Value::as_bool) != Some(false) || seal.get("independent_test_evaluated").and_then(Value::as_bool) != Some(false) {
        return Err("prefix seal must not claim promotion or an evaluated test".to_owned());
    }
    if require_calibration && seal.get("calibration_passed").and_then(Value::as_bool) != Some(true) {
        return Err("prefix candidate did not pass calibration".to_owned());
    }
    if seal.get("candidate_sha256") != Some(&Value::String(checksum(artifact)?)) {
        return Err("prefix artifact differs from seal".to_owned());
    }
    check_pins(root, &seal["provenance"], "prefix")?;
    let model = VersionedPrefixModel::load(artifact).map_err(|error| error.name.to_string())?;
    if seal.get("model_version").and_then(Value::as_str) != Some(model.version()) || seal.get("threshold").and_then(Value::as_f64) != Some(model.model.conversion_threshold) {
        return Err("prefix model identity differs from seal".to_owned());
    }
    Ok(seal)
}

pub struct EvaluationRequest<'a> {
    pub root: &'a Path,
    pub resource_dir: &'a Path,
    pub artifact: &'a Path,
    pub seal: &'a Path,
    pub prefix: &'a Path,
    pub prefix_seal: &'a Path,
    pub corpus: &'a Path,
    pub split: &'a str,
    pub output: &'a Path,
    pub ledger: &'a Path,
    pub jobs: usize,
    /// A development diagnostic of models that were not fitted by this tree: the seals are not read.
    pub unsealed: bool,
}

/// The inputs of a replay are still the ones its identity named before it started.
pub fn unchanged_since(request: &EvaluationRequest<'_>, identity: &Identity) -> Result<(), String> {
    if evaluation_identity(request)?.value != identity.value {
        return Err("evaluation inputs changed during sequence replay".to_owned());
    }
    Ok(())
}

/// The installed pair of a tree, by the digests of its two models: the reference a replay compares with.
pub fn installed_reference(root: &Path) -> Result<InstalledReference, String> {
    Ok(InstalledReference { context_sha256: checksum(&root.join(INSTALLED_CONTEXT))?, prefix_sha256: checksum(&root.join(INSTALLED_PREFIX))? })
}

/// The installed pair of a tree is still the one a replay started with.
pub fn installed_unchanged_since(root: &Path, reference: &InstalledReference) -> Result<(), String> {
    if installed_reference(root)? != *reference {
        return Err("the installed pair changed during sequence replay".to_owned());
    }
    Ok(())
}

/// `evaluate`: the report of a development diagnostic, or of the sealed test, read once.
pub fn evaluate(request: &EvaluationRequest<'_>) -> Result<Report, String> {
    let split = request.split;
    if split != DEVELOPMENT && split != "test" {
        return Err("sequence evaluator accepts development or sealed test only".to_owned());
    }
    if request.unsealed && split == "test" {
        return Err("a sealed test needs seals".to_owned());
    }
    let (root, models_dir) = (request.root, request.resource_dir.join("models"));
    if !request.unsealed {
        validate_candidate_seal(root, request.artifact, request.seal, request.corpus, &models_dir)?;
        validate_prefix_seal(root, request.prefix, request.prefix_seal, split == "test")?;
    }
    let identity = if request.unsealed {
        Identity { value: json!({"unsealed": true}), key: String::new() }
    } else {
        evaluation_identity(request)?
    };
    if split == "test" {
        claim_test_access(request.ledger, &identity)?;
    }
    // No test file, decompression, model scoring or row selection occurs above the access claim.
    let resources: Arc<Resources> = Resources::with_speller(request.resource_dir.to_path_buf(), ks_hunspell::hunspell_speller(ks_config::history::data_dir()));
    let portable = reference_models(root, &resources, false)?;
    let spelling = reference_models(root, &resources, true)?;
    let tools = Tools::new(root, resources.clone(), portable, BTreeSet::new())?;
    let rows = load_rows(request.corpus, split)?;
    let pairs = Pairs {
        candidate: request.artifact.to_path_buf(),
        baseline: root.join(BASELINE),
        prefix_candidate: request.prefix.to_path_buf(),
        prefix_baseline: root.join(PREFIX_BASELINE),
        installed: root.join(INSTALLED_CONTEXT),
        prefix_installed: root.join(INSTALLED_PREFIX),
    };
    // The installed pair is a reference, not an input the seal or the test membership names: the report says which one
    // it was, and the pair must not change while it is replayed.
    let reference = installed_reference(root)?;
    let gate_policy = read_json(&root.join("model/context_v3/recipe.json"))?["gate_policy"].clone();
    let mut report = Report {
        schema_version: 1,
        split: split.to_owned(),
        identity: Report::carry(&identity.value)?,
        protocol: Report::carry(&protocol(root)?)?,
        gate_policy: Report::carry(&gate_policy)?,
        promotion_passed: false,
        installed_reference: Some(reference.clone()),
        selection: None,
        profiles: None,
        execution_error: None,
        scope: None,
    };
    let scored = score_sequences(root, &resources, &tools, &spelling, &rows, &pairs, request.jobs);
    let write_report = |report: &Report| -> Result<(), String> {
        std::fs::create_dir_all(request.output.parent().ok_or("a report outside a folder")?).map_err(|error| error.to_string())?;
        std::fs::write(request.output, crate::artifact::canonical_of(report)).map_err(|error| error.to_string())
    };
    match scored {
        Ok(replayed) => {
            report.selection = Some(replayed.selection);
            report.profiles = Some(replayed.profiles);
            report.promotion_passed = replayed.promotion_passed;
        }
        Err(error) => {
            report.execution_error = Some(error.clone());
            if split == "test" {
                immutable_record(&request.ledger.join(format!("{}.outcome.json", identity.key)), &report)?;
            }
            write_report(&report)?;
            return Err(error);
        }
    }
    if !request.unsealed {
        unchanged_since(request, &identity)?;
    }
    installed_unchanged_since(root, &reference)?;
    if split == "test" {
        immutable_record(&request.ledger.join(format!("{}.outcome.json", identity.key)), &report)?;
    } else {
        report.promotion_passed = false;
        report.scope = Some("development diagnostic; cannot promote a model".to_owned());
    }
    write_report(&report)?;
    Ok(report)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::sequence_report::CorrectionRecord;

    #[test]
    fn a_replay_that_failed_to_run_fails_its_gate_whichever_pair_it_was() {
        let documents = Documents { us: 40, ru: 40, none: 0 };
        let clean = PairCounts::default();
        let failed = PairCounts { execution_errors: 1, ..PairCounts::default() };
        assert!(profile_gates(&clean, &clean, &documents).execution_succeeded);
        for (candidate, baseline) in [(&failed, &clean), (&clean, &failed)] {
            assert!(!profile_gates(candidate, baseline, &documents).execution_succeeded);
        }
    }

    fn switch(event: usize, target_group: i64, kind: &str) -> Switch<'_> {
        Switch { event, target_group, kind }
    }

    /// The class of each key a switch can come at, for a Russian word of six letters typed first in the English layout.
    #[test]
    fn a_wrong_first_word_is_counted_by_the_key_its_switch_came_at() {
        let mut total = Earliness::default();
        for (key, kind) in [(4, "early"), (6, "pause"), (7, "boundary"), (9, "context_word")] {
            total.add_wrong(1, 6, "привет мир", "привет мир", &[switch(key, 1, kind)]);
        }
        total.add_wrong(1, 6, "привет мир", "ghbdtn мир", &[]);
        // A correction into the other layout first: K is the first one into the declared layout.
        total.add_wrong(1, 6, "привет мир", "привет мир", &[switch(2, 0, "undo"), switch(12, 1, "boundary")]);
        assert_eq!((total.inside_word, total.last_letter, total.boundary_key, total.later, total.never), (1, 1, 1, 2, 1));
        assert_eq!((total.first_word_correct, total.switch_key_rows, total.switch_key_sum), (5, 5, 4 + 6 + 7 + 9 + 12));
        let histogram: Vec<i64> = total.switch_key_histogram.values().copied().collect();
        assert_eq!(total.switch_key_histogram.len(), SWITCH_KEY_HISTOGRAM_KEYS + 1);
        assert_eq!(histogram.iter().sum::<i64>(), 5);
        assert_eq!((total.switch_key_histogram["4"], total.switch_key_histogram["6"], total.switch_key_histogram["7"], total.switch_key_histogram[SWITCH_KEY_HISTOGRAM_LATER]), (1, 1, 1, 2));
        assert_eq!(total.first_correction_kinds, BTreeMap::from([("boundary".to_owned(), 2), ("context_word".to_owned(), 1), ("early".to_owned(), 1), ("pause".to_owned(), 1)]));
        assert_eq!(total.started_right_ended_wrong, 0);
    }

    /// An early switch that began the word right and a word that ended wrong; the word is the text up to its first space.
    #[test]
    fn a_word_an_early_switch_began_and_its_end_spoiled_is_counted_apart() {
        let mut total = Earliness::default();
        total.add_wrong(1, 6, "Фарадж сказал", "Фарад; сказал", &[switch(4, 1, "early")]);
        // Wrong after an early switch that came only after the word: not an earliness error of that kind.
        total.add_wrong(1, 6, "Фарадж сказал", "Фарад; сказал", &[switch(7, 1, "early")]);
        // Wrong after a switch of another kind inside the word.
        total.add_wrong(1, 6, "Фарадж сказал", "Фарад; сказал", &[switch(4, 1, "pause")]);
        // The first word right, the rest wrong: the first word counts as right.
        total.add_wrong(1, 6, "Фарадж сказал", "Фарадж crfpfk", &[switch(4, 1, "early")]);
        assert_eq!((total.started_right_ended_wrong, total.first_word_correct, total.switch_key_rows), (1, 1, 1));
        assert_eq!((total.inside_word, total.boundary_key), (3, 1));
        // A whole text of one word.
        assert_eq!(first_word("слово"), "слово");
        assert_eq!(first_word(""), "");
    }

    /// A Russian word of six letters typed first, in the English layout the editor opens in: the row's layout is 1, the
    /// other is 0.
    #[test]
    fn an_early_switch_away_from_the_layout_of_text_typed_right_is_a_false_one() {
        let mut total = Earliness::default();
        // Away and undone by the end: a flicker, on the word in focus (key 4 of six letters).
        total.add_correct_first(1, 6, true, &[switch(4, 0, "early"), switch(6, 1, "pause")]);
        // Away and left, after the word: spoiled, not on the word in focus.
        total.add_correct_first(1, 6, false, &[switch(8, 0, "early")]);
        // Away at the word's last letter: on it. Only the first false switch of a row counts, once.
        total.add_correct_first(1, 6, false, &[switch(6, 0, "early"), switch(9, 0, "early")]);
        // Not an early correction, or none: nothing.
        total.add_correct_first(1, 6, true, &[switch(7, 0, "boundary")]);
        total.add_correct_first(1, 6, true, &[]);
        assert_eq!(total.correct_first_plans, FirstFalseEarly { false_early: 3, false_early_on_focus: 2, false_early_flicker: 1, false_early_corrupted: 2, early_returns: 0 });
        assert_eq!(total.correct_sequence_plans, SequenceFalseEarly::default());
        assert_eq!(Earliness::bin(1), "1");
        assert_eq!(Earliness::bin(SWITCH_KEY_HISTOGRAM_KEYS + 1), SWITCH_KEY_HISTOGRAM_LATER);
    }

    /// Another layer turned the text away (a context switch on a pause), and an early correction put it back: a return.
    #[test]
    fn an_early_correction_back_into_the_layout_of_the_row_is_a_return_and_no_false_switch() {
        let mut total = Earliness::default();
        total.add_correct_first(1, 6, true, &[switch(6, 0, "pause"), switch(9, 1, "early")]);
        // A return and a false switch in one row count in both.
        total.add_correct_first(1, 6, false, &[switch(4, 0, "early"), switch(9, 1, "early")]);
        assert_eq!(total.correct_first_plans, FirstFalseEarly { false_early: 1, false_early_on_focus: 1, false_early_flicker: 0, false_early_corrupted: 1, early_returns: 2 });
        total.add_correct_sequence(Some(1), true, &[switch(30, 0, "context_word"), switch(31, 1, "early")]);
        total.add_correct_sequence(Some(1), true, &[switch(30, 1, "early"), switch(31, 1, "early")]);
        assert_eq!(total.correct_sequence_plans, SequenceFalseEarly { false_early: 0, false_early_flicker: 0, false_early_corrupted: 0, early_returns: 2 });
    }

    /// Inside a text: false switches by their final text, no count on a word in focus.
    #[test]
    fn a_false_early_switch_inside_a_text_is_split_by_its_final_text() {
        let mut total = Earliness::default();
        total.add_correct_sequence(Some(0), true, &[switch(30, 1, "early"), switch(31, 0, "early")]);
        total.add_correct_sequence(Some(0), false, &[switch(30, 1, "early")]);
        total.add_correct_sequence(Some(0), false, &[switch(30, 1, "pause")]);
        assert_eq!(total.correct_sequence_plans, SequenceFalseEarly { false_early: 2, false_early_flicker: 1, false_early_corrupted: 1, early_returns: 1 });
        assert_eq!(total.correct_first_plans, FirstFalseEarly::default());
    }

    /// A right early switch that the end of the word undid is the only one of its kind: the first correction into the
    /// declared layout must be an early one, by the word's last letter.
    #[test]
    fn a_right_early_switch_undone_at_the_boundary_is_counted_apart() {
        let mut total = Earliness::default();
        // Early at the fourth key, the first word spoiled at the boundary.
        total.add_wrong(1, 6, "Фарадж сказал", "Фарад; сказал", &[switch(4, 1, "early"), switch(7, 0, "context_word"), switch(8, 1, "boundary")]);
        // The first correction into the declared layout is a pause one; a later early one does not make it a right start.
        total.add_wrong(1, 6, "Фарадж сказал", "Фарад; сказал", &[switch(6, 1, "pause"), switch(6, 1, "early")]);
        // The early switch away from the declared layout does not start right.
        total.add_wrong(1, 6, "Фарадж сказал", "Фарад; сказал", &[switch(3, 0, "early"), switch(8, 1, "boundary")]);
        assert_eq!(total.started_right_ended_wrong, 1);
    }

    fn case(wrong: bool, letters: Option<i64>, actual: &str, corrections: Vec<CorrectionRecord>) -> Case {
        Case {
            identifier: "a".to_owned(),
            document: "d".to_owned(),
            group: Some(1),
            initially_wrong: wrong,
            expected: "привет мир".to_owned(),
            trimmed_left: 0,
            trimmed_right: 0,
            focus_letters: letters,
            mode: "default".to_owned(),
            actual: actual.to_owned(),
            exact: actual == "привет мир",
            length_mismatch: false,
            final_group: 1,
            expected_final_group: 1,
            final_layout_matches: true,
            corrections,
            intervention: None,
            error: None,
        }
    }

    fn correction(event: usize, target_group: i64, kind: Option<&str>) -> CorrectionRecord {
        CorrectionRecord { event, before: String::new(), after: String::new(), source_group: 0, target_group, actual_group: target_group, caret_before: 0, caret_after: 0, kind: kind.map(str::to_owned) }
    }

    /// The earliness a report states is read from its cases: the same numbers as from the corrections themselves.
    #[test]
    fn the_earliness_of_a_pair_is_read_from_its_cases() {
        let early = || vec![correction(4, 1, Some("early"))];
        let away = || vec![correction(4, 0, Some("early"))];
        let first = [case(true, Some(6), "привет мир", early()), case(false, Some(6), "привет мир", away()), case(false, Some(6), "ghbdtn", away())];
        let sequence = [case(false, None, "привет мир", away()), case(false, None, "привет мир", early()), case(true, None, "привет мир", early())];
        let read = earliness(&first, &sequence).expect("cases that hold");
        let mut expected = Earliness::default();
        expected.add_wrong(1, 6, "привет мир", "привет мир", &[switch(4, 1, "early")]);
        expected.add_correct_first(1, 6, true, &[switch(4, 0, "early")]);
        expected.add_correct_first(1, 6, false, &[switch(4, 0, "early")]);
        expected.add_correct_sequence(Some(1), true, &[switch(4, 0, "early")]);
        expected.add_correct_sequence(Some(1), true, &[switch(4, 1, "early")]);
        assert_eq!(read, expected);
        assert_eq!((read.correct_first_plans.false_early, read.correct_sequence_plans.false_early, read.correct_sequence_plans.early_returns), (2, 1, 1));
        // A case that does not say what the earliness needs.
        let refused = |first: &[Case], sequence: &[Case], words: &str| {
            let said = earliness(first, sequence).expect_err(words);
            assert!(said.contains(words), "{words}: {said}");
        };
        refused(&[case(true, None, "привет мир", early())], &[], "letters of its word");
        refused(&[case(true, Some(-1), "привет мир", early())], &[], "letters of its word");
        refused(&[case(true, Some(6), "привет мир", vec![correction(4, 1, None)])], &[], "without its kind");
        refused(&[], &[case(false, None, "привет мир", vec![correction(4, 1, None)])], "without its kind");
        let ungrouped = Case { group: None, ..case(true, Some(1), "привет мир", Vec::new()) };
        refused(&[ungrouped], &[], "without its layout");
        refused(&[Case { group: None, ..case(false, Some(1), "привет мир", Vec::new()) }], &[], "correct first case without its layout");
        refused(&[case(false, None, "привет мир", away())], &[], "letters of its word");
        // A row of no layout has no layout to be returned to: its early corrections all go away.
        let none = Case { group: None, ..case(false, None, "привет мир", vec![correction(4, 1, Some("early"))]) };
        let read = earliness(&[], &[none]).expect("a case of no layout");
        assert_eq!(read.correct_sequence_plans, SequenceFalseEarly { false_early: 1, false_early_flicker: 1, false_early_corrupted: 0, early_returns: 0 });
    }

    #[test]
    fn the_first_plans_are_gated_by_their_balance_their_corruptions_and_their_lengths() {
        let counts = |restored: i64, spoiled: i64, lengths: i64| PairCounts { exactly_restored: restored, correct_text_corruptions: spoiled, length_mismatches: lengths, ..PairCounts::default() };
        assert!(FirstGates::of(&counts(10, 1, 0), &counts(9, 1, 0)).all_held());
        assert!(FirstGates::of(&counts(10, 1, 0), &counts(10, 1, 0)).all_held());
        let fewer = FirstGates::of(&counts(9, 1, 0), &counts(10, 1, 0));
        assert_eq!((fewer.net_restorations_at_least_baseline, fewer.correct_text_preserved), (false, true));
        // More restored does not buy more corruption.
        let spoiling = FirstGates::of(&counts(20, 2, 0), &counts(10, 1, 0));
        assert_eq!((spoiling.net_restorations_at_least_baseline, spoiling.correct_text_preserved), (true, false));
        assert!(!FirstGates::of(&counts(10, 1, 1), &counts(9, 1, 0)).length_preserved);
    }

    /// Version 4 stands at the top of the file, version 5 under `versions`; each answers for its own hash.
    #[test]
    fn a_protocol_is_found_by_its_version() {
        let tree = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let protocols = read_json(&tree.join(PROTOCOLS)).expect("the protocols");
        for version in [TOP_LEVEL_PROTOCOL_VERSION, SEQUENCE_PROTOCOL_VERSION] {
            let set = protocol_set(&protocols, version).expect("a version the tree keeps");
            assert_eq!(set["receipt"]["version"], json!(version));
            assert_eq!(audited_sequence(&protocols, version).expect("audited")["version"], json!(version));
        }
        assert_eq!(protocol(&tree).expect("the current protocol")["version"], json!(SEQUENCE_PROTOCOL_VERSION));
        assert!(protocol_set(&protocols, 3).is_none() && protocol_set(&protocols, 6).is_none());
        assert!(audited_sequence(&protocols, 6).unwrap_err().contains("no sequence protocol of version 6"));
        // A block that names another version than the one it stands under, and one whose words changed after their audit.
        let mut moved = protocols.clone();
        moved["versions"]["5"]["sequence"]["version"] = json!(4);
        assert!(protocol_set(&moved, SEQUENCE_PROTOCOL_VERSION).is_none());
        let mut changed = protocols;
        changed["versions"]["5"]["sequence"]["window"] = json!("other");
        assert!(audited_sequence(&changed, SEQUENCE_PROTOCOL_VERSION).unwrap_err().contains("protocol changed"));
        assert!(audited_sequence(&changed, TOP_LEVEL_PROTOCOL_VERSION).is_ok());
    }
}
