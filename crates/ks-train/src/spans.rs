//! Bounded non-test frames captured from physical engine boundaries (`context_action_spans`): a wrong focus after a
//! correct anchor of the same split, typed through the real engine, whose every question to the model is recorded.
use ks_cast::To as _;
use crate::corpus::{CorpusRow, TRAIN, DEVELOPMENT, CALIBRATION};
use crate::curricula::Counts;
use crate::physical::{PhysicalKey, hex_digest, key_of, sha256};
use crate::rows::is_alpha;
use crate::replay::{Capture, Mode, Plan, Replayer};
use ks_context::context_model::{ContextAction, ContextEvidence};
use ks_text::pystr;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};

const VARIANTS: [(&str, &str); 6] = [("", " "), (".", " "), (",", "  "), (";", "   "), ("...", "  "), ("!", "   ")];
const SPAN_ANCHOR_MAX_CHARACTERS: usize = 24;
const SPAN_ANCHOR_MIN_CHARACTERS: usize = 3;
const SPAN_MAXIMUM_FAMILIES: i64 = 128;
const SPAN_ORIGINAL_MAX_CHARACTERS: usize = 64;
const LAYOUT_GROUP_COUNT: usize = 2;

#[derive(Clone, Debug, PartialEq)]
pub struct SpanFrame {
    pub evidence: ContextEvidence,
    pub action: ContextAction,
    pub parent_family: String,
    pub source_identifier: String,
    pub sequence_id: String,
    pub sample_weight: f64,
}

#[derive(Default)]
pub struct SpanCurriculum {
    pub frames: Vec<SpanFrame>,
    pub counts: Option<Counts>,
}

fn rank(purpose: &str, identity: &str) -> [u8; 32] {
    sha256(&format!("context-action-spans-v1:{purpose}:{identity}"))
}

/// `_keys`: the first key that types each character in this layout.
fn keys_of(text: &str, group: i64) -> Option<Vec<PhysicalKey>> {
    text.chars().map(|character| key_of(character, group.to::<usize>()).cloned()).collect()
}

fn body(text: &str, group: i64) -> Vec<u32> {
    keys_of(text, group).expect("unrepresentable span sequence").iter().map(|key| key.keycode).collect()
}

fn select(rows: &[CorpusRow], maximum: usize) -> (Vec<&CorpusRow>, [Vec<&CorpusRow>; 2]) {
    let mut ordered: Vec<&CorpusRow> = rows.iter().collect();
    ordered.sort_by_cached_key(|row| (rank("source", &row.identifier), row.identifier.clone()));
    let mut families: HashMap<&str, &CorpusRow> = HashMap::new();
    let mut anchors: [Vec<&CorpusRow>; 2] = [Vec::new(), Vec::new()];
    for row in ordered {
        let Some(group) = row.group.filter(|group| *group == 0 || *group == 1) else { continue };
        let length = pystr::len(&row.original);
        if !row.layout_representable || !(1..=SPAN_ORIGINAL_MAX_CHARACTERS).contains(&length) || !row.original.chars().any(is_alpha) || keys_of(&row.original, group).is_none() {
            continue;
        }
        families.entry(&row.family).or_insert(row);
        if pystr::isalpha(&row.original) && (SPAN_ANCHOR_MIN_CHARACTERS..=SPAN_ANCHOR_MAX_CHARACTERS).contains(&length) {
            anchors[group.to::<usize>()].push(row);
        }
    }
    let mut ranked: Vec<&CorpusRow> = families.into_values().collect();
    ranked.sort_by_cached_key(|row| (rank("family", &row.family), row.family.clone()));
    let mut selected = Vec::new();
    let mut per_group = [0_usize; 2];
    for row in ranked {
        let group = row.group.expect("a selected row has a group").to::<usize>();
        if anchors[group].is_empty() || per_group[group] >= maximum / LAYOUT_GROUP_COUNT {
            continue;
        }
        per_group[group] += 1;
        selected.push(row);
    }
    (selected, anchors)
}

/// `_label`: what a captured question's answer is, given the text that should stand by then.
fn label(capture: &Capture, expected: &str) -> Option<ContextAction> {
    if capture.observed == expected {
        return Some(ContextAction::Keep);
    }
    let item = &capture.evidence;
    let suffix = format!("{}{}", item.literal_tail, item.boundary_text);
    let anchor = format!("{}{suffix}", item.original);
    if anchor.is_empty() || !capture.observed.ends_with(&anchor) {
        return None;
    }
    let converted = format!("{}{}{suffix}", capture.observed.get(..capture.observed.len() - anchor.len()).unwrap_or_default(), item.alternative);
    (converted == expected).then_some(ContextAction::Convert)
}

/// `build_span_curriculum`: attainable labels captured without dictionary or candidate predictions.
pub fn build_span_curriculum(replayer: &mut Replayer, rows: &[CorpusRow], maximum_families: i64, expected_split: &str) -> SpanCurriculum {
    assert!((LAYOUT_GROUP_COUNT.to::<i64>()..=SPAN_MAXIMUM_FAMILIES).contains(&maximum_families), "span family budget");
    assert!(maximum_families % LAYOUT_GROUP_COUNT.to::<i64>() == 0, "span family budget");
    assert!([TRAIN, DEVELOPMENT, CALIBRATION].contains(&expected_split), "span curriculum never accepts the test split");
    assert!(rows.iter().all(|row| row.split == expected_split), "span curriculum accepts only rows from the declared split");
    assert!(rows.iter().all(|row| row.quarantine_reasons.is_empty()), "span curriculum accepts only rows from the declared split");
    let (selected, anchors) = select(rows, maximum_families.to::<usize>());
    let mut counts = Counts::default();
    counts.add_by("input_rows", rows.len().to::<i64>());
    counts.add_by("selected_families", selected.len().to::<i64>());
    let mut frames: Vec<SpanFrame> = Vec::new();
    for row in selected {
        let group = row.group.expect("a selected row has a group");
        counts.add(format!("selected_group_{group}"));
        let purpose = format!("anchor:{}", row.family);
        let anchor = anchors[group.to::<usize>()].iter().min_by_key(|other| (rank(&purpose, &other.identifier), other.identifier.clone())).expect("an anchor");
        let parent_body = body(&row.original, group);
        for (variant, (tail, spacing)) in VARIANTS.into_iter().enumerate() {
            let before = format!("{}{spacing}", anchor.original);
            for wrong in [false, true] {
                let source = if wrong { 1 - group } else { group };
                let prefix_keys = keys_of(&before, group).expect("unrepresentable span sequence");
                let focus_keys = keys_of(&row.original, group).expect("unrepresentable span sequence");
                let tail_keys = keys_of(&format!("{tail} "), source).expect("unrepresentable span sequence");
                let prefix_length = prefix_keys.len();
                let mut keys = prefix_keys;
                keys.extend(focus_keys);
                keys.extend(tail_keys);
                keys[0].explicit_group = Some(group.to::<i32>());
                keys[prefix_length].explicit_group = Some(source.to::<i32>());
                let expected = format!("{before}{}{tail} ", row.original);
                let expected_characters: Vec<char> = expected.chars().collect();
                let identity = hex_digest(&format!("{}:{variant}:{}", row.identifier, i32::from(wrong)));
                let plan = Plan::typed(identity.clone(), row.document.clone(), group, expected.clone(), keys, source);
                let outcome = replayer.replay(&plan, Mode::EarlyOff);
                assert!(outcome.error.is_none(), "span capture must complete without corrections: {expected:?} gave {:?}", outcome.error);
                assert!(outcome.corrections.is_empty(), "span capture must complete without corrections: {expected:?} gave {:?}", outcome.error);
                let records = replayer.take_records();
                counts.add("sequences");
                let focus: Vec<Capture> = records.into_iter().filter(|record| record.event > prefix_length).collect();
                counts.add_by("no_focus_call", i64::from(focus.is_empty()));
                for record in focus {
                    counts.add("focus_calls");
                    let item = &record.evidence;
                    if body(&item.original, item.source_group) != parent_body {
                        counts.add("skipped_nonparent_body");
                        continue;
                    }
                    let so_far: String = expected_characters[..record.event].iter().collect();
                    let Some(action) = label(&record, &so_far) else {
                        counts.add("skipped_inexpressible");
                        continue;
                    };
                    counts.add(format!("action_{}", action.as_str()));
                    counts.add_by("tail_nonempty", i64::from(!item.literal_tail.is_empty()));
                    counts.add_by("repeated_space_before", i64::from(item.field.before.ends_with("  ")));
                    frames.push(SpanFrame { evidence: record.evidence, action, parent_family: row.family.clone(), source_identifier: row.identifier.clone(), sequence_id: identity.clone(), sample_weight: 0.0 });
                }
            }
        }
    }
    let mut family_counts: BTreeMap<String, u64> = BTreeMap::new();
    for frame in &frames {
        *family_counts.entry(frame.parent_family.clone()).or_insert(0) += 1;
    }
    for frame in &mut frames {
        frame.sample_weight = 1.0 / family_counts[&frame.parent_family].to::<f64>();
    }
    counts.add_by("retained_families", family_counts.len().to::<i64>());
    counts.add_by("frames", frames.len().to::<i64>());
    SpanCurriculum { frames, counts: Some(counts) }
}

impl SpanCurriculum {
    pub fn report(&self) -> Value {
        self.counts.as_ref().map_or(json!({}), Counts::report)
    }
}
