//! Bounded candidates for an unexecuted next-word conversion (`context_lookahead_curriculum`), and the curricula built on
//! them. The original first-word annotation supplies the label; a baseline proposal for the next word only establishes
//! an entry condition.
use ks_cast::To as _;
use crate::captured::group_of_word;
use crate::corpus::CorpusRow;
use crate::curricula::{Counts, Curriculum};
use crate::evidence::evidence;
use crate::physical::{hex_digest, physical, reading, sha256, translated};
use crate::rows::{ActionRow, application_of, has_word, public_field, word_at_start};
use crate::tools::{Lexicons, Tools};
use ks_context::context_action_features::extract_action_features;
use ks_context::context_model::{AfterOrigin, ContextAction, ContextEvidence};
use ks_lang::detector::{DecideOptions, LanguageDetector};
use ks_lang::intent_model::CorrectionTrigger;
use ks_lang::ortho_model::OrthoModel;
use ks_lang::word_decision::{AutomaticOptions, automatic_word_decision};
use ks_text::{fsum, py_round_ndigits, pystr};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};

const LOOKAHEAD_ANCHOR_MAX_CHARACTERS: usize = 64;
const LOOKAHEAD_ANCHOR_MIN_CHARACTERS: usize = 3;
const LOOKAHEAD_MAXIMUM_FAMILIES_LIMIT: i64 = 4096;
const LOOKAHEAD_MAXIMUM_SEEDS_PER_FAMILY: i64 = 128;
const PLANNED_VARIANT_MASS_DIVISOR: f64 = 2.0;
const LEXICAL_PAIR_ANCHOR_VARIANTS: usize = 3;
const IDENTIFIER_SUFFIX_SEGMENTS: usize = 2;
const MASS_REPORT_DECIMALS: i64 = 4;
const ACTION_SHORT_WORD_MAX_CHARACTERS: usize = 2;
const ACTION_DEFERRED_WORD_MAX_CHARACTERS: usize = 3;

struct Seed {
    identifier: String,
    parent_family: String,
    evidence: ContextEvidence,
    action: ContextAction,
    category: String,
    sample_weight: f64,
    next_words: String,
    planned_action: Option<ContextAction>,
}

#[derive(Clone)]
pub struct Anchor {
    identifier: String,
    text: String,
    group: i64,
}

struct Frame {
    source: usize,
    evidence: ContextEvidence,
    action: ContextAction,
    sample_weight: f64,
    planned_anchor: Option<String>,
}

struct Built {
    frames: Vec<Frame>,
    counts: Counts,
    mass: BTreeMap<String, f64>,
}

fn rank(purpose: &str, identifier: &str) -> [u8; 32] {
    sha256(&format!("context-lookahead-v1:{purpose}:{identifier}"))
}

/// `_fingerprint`: what tells two frames apart to the model. The Python hashes the features' JSON; the bits of the
/// floats tell the same values apart.
fn fingerprint(tools: &Tools, item: &ContextEvidence) -> String {
    let features = extract_action_features(item, &tools.tables).expect("the features of a frame");
    let mut sorted: Vec<(&String, &f64)> = features.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    let mut text = String::new();
    for (name, value) in sorted {
        text.push_str(name);
        text.push('=');
        text.push_str(&value.to_bits().to_string());
        text.push(';');
    }
    hex_digest(&text)
}

fn eligible(tools: &Tools, seed: &Seed) -> bool {
    let item = &seed.evidence;
    if !tools.deferred_isolated(&item.original, &item.alternative, item.source_group)
        || !pystr::isalpha(&item.original)
        || !pystr::isalpha(&item.alternative)
        || item.trigger != "space"
        || item.boundary_text != " "
        || !item.literal_tail.is_empty()
        || item.after_origin == AfterOrigin::PlannedNextConversion
    {
        return false;
    }
    match translated(&item.original, item.source_group) {
        Ok(other) if other == item.alternative => {}
        _ => return false,
    }
    if seed.action == ContextAction::Convert && seed.category == "legacy_short_lookahead" {
        return !pystr::strip(&item.field.after).is_empty();
    }
    let planned = seed.planned_action.unwrap_or(seed.action);
    if planned == ContextAction::Convert && seed.category == "natural_short_lookahead" {
        // A natural typing frame knows its own continuation; the left context may be empty.
        return item.field.after.is_empty() && !pystr::strip(&seed.next_words).is_empty();
    }
    if planned == ContextAction::Convert && seed.category == "lexical_short_pair" {
        return item.field.after.is_empty() && pystr::strip(&item.field.before).is_empty();
    }
    planned == ContextAction::Keep && (seed.category.starts_with("legacy_") || seed.category == "natural_surface") && !pystr::strip(&item.field.before).is_empty() && item.field.after.is_empty()
}

fn anchor_word(text: &str) -> bool {
    (LOOKAHEAD_ANCHOR_MIN_CHARACTERS..=LOOKAHEAD_ANCHOR_MAX_CHARACTERS).contains(&pystr::len(text)) && pystr::isalpha(text)
}

/// `build_lookahead_curriculum`: original plus labelled, unexecuted planned-context variants.
fn build(tools: &Tools, seeds: &[Seed], anchors: &[Anchor], detector: &LanguageDetector, maximum_families: i64, seeds_per_family: i64) -> Built {
    assert!((1..=LOOKAHEAD_MAXIMUM_FAMILIES_LIMIT).contains(&maximum_families) && (1..=LOOKAHEAD_MAXIMUM_SEEDS_PER_FAMILY).contains(&seeds_per_family), "lookahead budgets");
    for seed in seeds {
        assert!(!seed.identifier.is_empty() && !seed.parent_family.is_empty(), "lookahead requires source and parent identities");
        assert!(seed.sample_weight.is_finite() && seed.sample_weight > 0.0, "lookahead requires finite positive mass");
    }
    let mut counts = Counts::default();
    counts.add_by("input_seeds", seeds.len().to::<i64>());
    counts.add_by("input_anchors", anchors.len().to::<i64>());
    let mut sorted_anchors: Vec<&Anchor> = anchors.iter().collect();
    sorted_anchors.sort_by(|a, b| a.identifier.cmp(&b.identifier));
    let mut available: [Vec<&Anchor>; 2] = [Vec::new(), Vec::new()];
    for anchor in sorted_anchors {
        if !anchor_word(&anchor.text) || translated(&anchor.text, anchor.group).is_err() {
            counts.add("skipped_anchor");
            continue;
        }
        available[anchor.group.to::<usize>()].push(anchor);
    }
    let mut families: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut labels: HashMap<String, HashSet<ContextAction>> = HashMap::new();
    for (index, seed) in seeds.iter().enumerate() {
        labels.entry(fingerprint(tools, &seed.evidence)).or_default().insert(seed.action);
        if eligible(tools, seed) {
            families.entry(&seed.parent_family).or_default().push(index);
        }
    }
    let mut family_names: Vec<&str> = families.keys().copied().collect();
    family_names.sort_by_cached_key(|family| (rank("family", family), (*family).to_owned()));
    family_names.truncate(maximum_families.to::<usize>());
    let mut selected: Vec<usize> = Vec::new();
    for family in &family_names {
        // Interleave the existing actions so many KEEP contexts cannot hide all positive lookahead seeds.
        let mut members = families[family].clone();
        members.sort_by_cached_key(|index| (rank("seed", &seeds[*index].identifier), seeds[*index].identifier.clone()));
        let mut action_index: HashMap<ContextAction, i64> = HashMap::new();
        let mut ranked: Vec<(i64, [u8; 32], &str, usize)> = Vec::new();
        for index in members {
            let seed = &seeds[index];
            let position = action_index.entry(seed.action).or_insert(0);
            ranked.push((*position, rank("seed", &seed.identifier), &seed.identifier, index));
            *position += 1;
        }
        ranked.sort_unstable();
        selected.extend(ranked.into_iter().take(seeds_per_family.to::<usize>()).map(|entry| entry.3));
    }
    counts.add_by("selected_families", family_names.len().to::<i64>());
    counts.add_by("selected_seeds", selected.len().to::<i64>());
    let mut proposals: HashMap<usize, (ContextEvidence, String)> = HashMap::new();
    for index in selected {
        let seed = &seeds[index];
        let item = &seed.evidence;
        let group = 1 - item.source_group;
        let planned_label = seed.planned_action.unwrap_or(seed.action);
        let words = if item.field.after.is_empty() { seed.next_words.as_str() } else { item.field.after.as_str() };
        let continuation = pystr::split(words);
        let mut choices: Vec<&Anchor> = available[group.to::<usize>()].clone();
        if planned_label == ContextAction::Convert && !continuation.is_empty() {
            choices.retain(|anchor| anchor.text == continuation[0]);
        }
        if choices.is_empty() {
            counts.add("skipped_missing_anchor");
            continue;
        }
        let purpose = format!("anchor:{}", seed.identifier);
        let anchor = choices.into_iter().min_by_key(|anchor| (rank(&purpose, &anchor.identifier), anchor.identifier.clone())).expect("a choice");
        let next_original = reading(&anchor.text, group);
        let previous = [(item.source_group, item.original.clone()), (group, item.alternative.clone())];
        let targets = [(group, anchor.text.clone())];
        let proposed = automatic_word_decision(
            detector,
            &next_original,
            &targets,
            item.source_group,
            &AutomaticOptions {
                decide: DecideOptions { previous_words: &previous, context_group: Some(item.source_group), trigger: CorrectionTrigger::Space, ..DecideOptions::default() },
                ..AutomaticOptions::default()
            },
        )
        .expect("both lexicons are given");
        if !proposed.should_convert {
            counts.add("skipped_next_not_proposed");
            continue;
        }
        // The engine re-decides a waiting word with its planned next word as the only language context.
        let only = [(group, anchor.text.clone())];
        let alternatives = [(group, item.alternative.clone())];
        let contextual = automatic_word_decision(
            detector,
            &item.original,
            &alternatives,
            item.source_group,
            &AutomaticOptions {
                decide: DecideOptions { previous_words: &only, context_group: Some(group), trigger: CorrectionTrigger::Space, ..DecideOptions::default() },
                ..AutomaticOptions::default()
            },
        )
        .expect("both lexicons are given");
        let mut candidate = item.clone();
        candidate.field.after.clone_from(&anchor.text);
        candidate.after_origin = AfterOrigin::PlannedNextConversion;
        candidate.baseline_convert = contextual.should_convert;
        candidate.model_probability = contextual.model_probability;
        candidate.model_threshold = contextual.model_threshold;
        labels.entry(fingerprint(tools, &candidate)).or_default().insert(planned_label);
        proposals.insert(index, (candidate, anchor.identifier.clone()));
    }
    let mut order: Vec<usize> = (0..seeds.len()).collect();
    order.sort_by(|a, b| seeds[*a].identifier.cmp(&seeds[*b].identifier));
    let mut frames = Vec::new();
    let mut mass: BTreeMap<String, f64> = BTreeMap::new();
    for index in order {
        let seed = &seeds[index];
        let mut proposal = proposals.remove(&index);
        if proposal.as_ref().is_some_and(|found| labels[&fingerprint(tools, &found.0)].len() != 1) {
            counts.add("skipped_label_conflict");
            proposal = None;
        }
        let weight = if proposal.is_none() { seed.sample_weight } else { seed.sample_weight / PLANNED_VARIANT_MASS_DIVISOR };
        frames.push(Frame { source: index, evidence: seed.evidence.clone(), action: seed.action, sample_weight: weight, planned_anchor: None });
        *mass.entry(format!("{}:{}", seed.evidence.after_origin.as_str(), seed.action.as_str())).or_insert(0.0) += weight;
        if let Some((item, anchor)) = proposal {
            let planned_label = seed.planned_action.unwrap_or(seed.action);
            counts.add(format!("planned_{}", planned_label.as_str()));
            *mass.entry(format!("{}:{}", item.after_origin.as_str(), planned_label.as_str())).or_insert(0.0) += weight;
            frames.push(Frame { source: index, evidence: item, action: planned_label, sample_weight: weight, planned_anchor: Some(anchor) });
        }
    }
    Built { frames, counts, mass }
}

fn mass_report(mass: &BTreeMap<String, f64>) -> Value {
    let mut map = Map::new();
    for (name, value) in mass {
        map.insert(name.clone(), json!(value));
    }
    Value::Object(map)
}

fn total(rows: &[ActionRow]) -> f64 {
    fsum(&rows.iter().map(|row| row.sample_weight).collect::<Vec<_>>()).expect("finite masses")
}

/// The row of a frame: its seed's row, at the frame's context and mass.
fn replaced(row: &ActionRow, frame: &Frame) -> ActionRow {
    let mut result = row.clone();
    if let Some(anchor) = &frame.planned_anchor {
        result.identifier = format!("{}:planned:{anchor}", row.identifier);
    }
    result.field = frame.evidence.field.clone();
    result.sample_weight = frame.sample_weight;
    result.after_origin = frame.evidence.after_origin;
    result
}

/// `legacy_lookahead_rows`: bounded old TRAIN frames replaced with equal-mass planned variants.
pub fn legacy_lookahead_rows(tools: &Tools, rows: Vec<ActionRow>, detector: &LanguageDetector, ortho: Option<&OrthoModel>, maximum_families: i64, seeds_per_family: i64) -> Curriculum {
    let chosen = |row: &ActionRow| {
        row.category.starts_with("legacy_")
            && tools.lookahead_focus(&row.original, row.group)
            && row.trigger == "space"
            && row.boundary_text == " "
            && row.literal_tail.is_empty()
            && !row.field.sensitive
            && !row.field.selection
            && row.field.role != ks_backend::FieldRole::Password
    };
    // `{row.identifier: row}`: the last of equal identifiers, at the place of the first.
    let mut places: HashMap<&str, usize> = HashMap::new();
    let mut selected: Vec<&ActionRow> = Vec::new();
    for row in &rows {
        if chosen(row) {
            if let Some(place) = places.get(row.identifier.as_str()) { selected[*place] = row } else {
                places.insert(&row.identifier, selected.len());
                selected.push(row);
            }
        }
    }
    let mut sorted: Vec<&ActionRow> = rows.iter().collect();
    sorted.sort_by(|a, b| a.identifier.cmp(&b.identifier));
    let mut anchors: Vec<Anchor> = Vec::new();
    let mut seen: HashSet<(String, i64)> = HashSet::new();
    for row in sorted {
        if !row.category.starts_with("legacy_") {
            continue;
        }
        let Some(text) = word_at_start(pystr::lstrip(&row.field.after)) else { continue };
        if !anchor_word(text) {
            continue;
        }
        let group = group_of_word(text);
        if translated(text, group).is_err() || !seen.insert((text.to_owned(), group)) {
            continue;
        }
        anchors.push(Anchor { identifier: format!("{}:first-after", row.identifier), text: text.to_owned(), group });
    }
    let seeds: Vec<Seed> = selected
        .iter()
        .map(|row| Seed {
            identifier: row.identifier.clone(),
            parent_family: row.parent_family.clone(),
            evidence: evidence(tools, row, detector, ortho, None),
            action: row.action,
            category: row.category.clone(),
            sample_weight: row.sample_weight,
            next_words: String::new(),
            planned_action: None,
        })
        .collect();
    let curriculum = build(tools, &seeds, &anchors, detector, maximum_families, seeds_per_family);
    let chosen_rows: Vec<ActionRow> = selected.iter().map(|row| (*row).clone()).collect();
    let selected_names: HashSet<&str> = chosen_rows.iter().map(|row| row.identifier.as_str()).collect();
    let mut result: Vec<ActionRow> = rows.iter().filter(|row| !selected_names.contains(row.identifier.as_str())).cloned().collect();
    for frame in &curriculum.frames {
        result.push(replaced(&chosen_rows[frame.source], frame));
    }
    let report = json!({"counts": curriculum.counts.report(), "mass_by_origin_action": mass_report(&curriculum.mass), "input_mass": total(&rows), "output_mass": total(&result),
        "scope": "Old TRAIN annotations and first words of their existing right contexts; no new split or candidate context-model labels."});
    (result, report)
}

/// `natural_anchors`: the first words of a split's own right contexts, one anchor per word and language.
#[must_use]
pub fn natural_anchors(source_rows: &[CorpusRow]) -> Vec<Anchor> {
    let mut sorted: Vec<&CorpusRow> = source_rows.iter().collect();
    sorted.sort_by(|a, b| a.identifier.cmp(&b.identifier));
    let mut anchors = Vec::new();
    let mut seen: HashSet<(String, i64)> = HashSet::new();
    for source in sorted {
        let Some(text) = word_at_start(pystr::lstrip(&source.after)) else { continue };
        if !anchor_word(text) {
            continue;
        }
        let group = group_of_word(text);
        if translated(text, group).is_err() || !seen.insert((text.to_owned(), group)) {
            continue;
        }
        anchors.push(Anchor { identifier: format!("{}:first-after", source.identifier), text: text.to_owned(), group });
    }
    anchors
}

/// `row.identifier.rsplit(":", 2)[0]`.
fn source_identifier(identifier: &str) -> &str {
    let mut end = identifier.len();
    for _ in 0..IDENTIFIER_SUFFIX_SEGMENTS {
        match identifier[..end].rfind(':') {
            Some(place) => end = place,
            None => break,
        }
    }
    &identifier[..end]
}

/// `natural_lookahead_rows`: bounded natural short-space frames replaced with equal-mass original/planned variants.
#[expect(clippy::too_many_arguments)]
pub fn natural_lookahead_rows(tools: &Tools, rows: Vec<ActionRow>, source_rows: &[CorpusRow], detector: &LanguageDetector, ortho: Option<&OrthoModel>, maximum_families: i64, seeds_per_family: i64) -> Curriculum {
    let by_identifier: HashMap<&str, &CorpusRow> = source_rows.iter().map(|row| (row.identifier.as_str(), row)).collect();
    let mut places: HashMap<String, usize> = HashMap::new();
    let mut selected: Vec<(ActionRow, String)> = Vec::new();
    for row in &rows {
        if !matches!(row.category.as_str(), "layout_intervention" | "natural_surface")
            || !tools.lookahead_focus(&row.original, row.group)
            || row.trigger != "space"
            || row.boundary_text != " "
            || !row.literal_tail.is_empty()
            || row.field.sensitive
            || row.field.selection
            || row.field.role == ks_backend::FieldRole::Password
            || !row.field.after.is_empty()
        {
            continue;
        }
        let Some(source) = by_identifier.get(source_identifier(&row.identifier)) else { continue };
        let entry = if row.category == "layout_intervention" {
            (row.clone(), source.after.clone())
        } else if !pystr::strip(&row.field.before).is_empty() {
            (row.clone(), String::new())
        } else {
            continue;
        };
        if let Some(place) = places.get(&row.identifier) { selected[*place] = entry } else {
            places.insert(row.identifier.clone(), selected.len());
            selected.push(entry);
        }
    }
    let anchors = natural_anchors(source_rows);
    let seeds: Vec<Seed> = selected
        .iter()
        .map(|(row, next_words)| {
            let intervention = row.category == "layout_intervention";
            Seed {
                identifier: row.identifier.clone(),
                parent_family: format!("natural:{}", by_identifier[source_identifier(&row.identifier)].document),
                evidence: evidence(tools, row, detector, ortho, None),
                action: row.action,
                category: if intervention { "natural_short_lookahead".to_owned() } else { row.category.clone() },
                sample_weight: row.sample_weight,
                next_words: next_words.clone(),
                planned_action: Some(if intervention { ContextAction::Convert } else { ContextAction::Keep }),
            }
        })
        .collect();
    let curriculum = build(tools, &seeds, &anchors, detector, maximum_families, seeds_per_family);
    let mut result: Vec<ActionRow> = rows.iter().filter(|row| !places.contains_key(&row.identifier)).cloned().collect();
    for frame in &curriculum.frames {
        let mut row = replaced(&selected[frame.source].0, frame);
        row.action = frame.action;
        result.push(row);
    }
    let report = json!({"counts": curriculum.counts.report(), "mass_by_origin_action": mass_report(&curriculum.mass), "input_mass": total(&rows), "output_mass": total(&result),
        "scope": "Natural short typing frames of one split paired with the first word of their own sentence's continuation; no cross-split anchors and no candidate context-model labels."});
    (result, report)
}

/// `lexical_short_pairs`: three-letter words of the pinned lexicons whose other reading is a word or a command too.
fn lexical_short_pairs(tools: &Tools, models: &Lexicons) -> Result<Vec<(String, String, i64)>, String> {
    let mut supplement: Vec<HashSet<String>> = Vec::new();
    for locale in crate::tools::LOCALES {
        let words = tools.resources.supplement_words(locale).map_err(|error| error.name.to_string())?;
        supplement.push(words.iter().map(str::to_owned).collect());
    }
    let real = |text: &str, group: i64| {
        let known = models[group.to::<usize>()].score(text).known;
        (!supplement[group.to::<usize>()].contains(text) && known) || (tools.plausible_reading(text, group) && !known)
    };
    let mut pairs = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for group in 0..2_i64 {
        // The table is sorted by word.
        for word in models[group.to::<usize>()].frequencies.words().iter() {
            let length = pystr::len(word);
            if length <= ACTION_SHORT_WORD_MAX_CHARACTERS || length > ACTION_DEFERRED_WORD_MAX_CHARACTERS || !pystr::isalpha(word) || supplement[group.to::<usize>()].contains(word) {
                continue;
            }
            let Ok(alternate) = translated(word, group) else { continue };
            if !real(&alternate, 1 - group) || !tools.deferred_isolated(word, &alternate, group) || tools.is_refused(&[word, &alternate]) {
                continue;
            }
            // Both lexicons name the same physical pair; it enters once, in the reading met first.
            if !seen.insert(physical(word)) {
                continue;
            }
            pairs.push((word.to_owned(), alternate, group));
        }
    }
    Ok(pairs)
}

/// `lexical_short_pair_rows`: a deferred three-letter word taught to follow its converted neighbour.
#[expect(clippy::too_many_arguments)]
pub fn lexical_short_pair_rows(tools: &Tools, rows: Vec<ActionRow>, source_rows: &[CorpusRow], detector: &LanguageDetector, ortho: Option<&OrthoModel>, maximum_families: i64, seeds_per_family: i64, models: &Lexicons) -> Result<Curriculum, String> {
    let pairs = lexical_short_pairs(tools, models)?;
    let mut seeds = Vec::new();
    let mut originals: Vec<ActionRow> = Vec::new();
    for (word, alternate, group) in &pairs {
        let family = format!("lexical:{}", hex_digest(&physical(word)));
        for (member, member_group) in [(word, *group), (alternate, 1 - *group)] {
            for variant in 0..LEXICAL_PAIR_ANCHOR_VARIANTS {
                let identifier = format!("lexical:{member_group}:{member}:{variant}");
                let field = public_field(application_of(&identifier), "", "");
                let row = ActionRow::new(identifier.as_str(), member.as_str(), member_group, field, "space", ContextAction::Wait, "lexical_short_pair", " ").weighing(1.0 / LEXICAL_PAIR_ANCHOR_VARIANTS.to::<f64>());
                seeds.push(Seed {
                    identifier,
                    parent_family: family.clone(),
                    evidence: evidence(tools, &row, detector, ortho, None),
                    action: ContextAction::Wait,
                    category: "lexical_short_pair".to_owned(),
                    sample_weight: row.sample_weight,
                    next_words: String::new(),
                    planned_action: Some(ContextAction::Convert),
                });
                originals.push(row);
            }
        }
    }
    let anchors = natural_anchors(source_rows);
    let curriculum = build(tools, &seeds, &anchors, detector, maximum_families, seeds_per_family);
    let input_mass = total(&rows);
    let mut result = rows;
    for frame in &curriculum.frames {
        let mut row = replaced(&originals[frame.source], frame);
        row.action = frame.action;
        result.push(row);
    }
    let mut words: Vec<String> = pairs.iter().map(|(word, alternate, _)| format!("{word}/{alternate}")).collect();
    words.sort();
    let by_direction = |group: i64| pairs.iter().filter(|pair| pair.2 == group).count();
    let report = json!({"pairs": pairs.len(), "pairs_by_direction": {"0": by_direction(0), "1": by_direction(1)}, "words": words,
        "counts": curriculum.counts.report(), "mass_by_origin_action": mass_report(&curriculum.mass), "input_mass": input_mass, "output_mass": total(&result),
        "scope": "TRAIN only: three-letter words of the pinned lexicons and the shipped identifier index whose other reading is a real word or command too, standing alone with the deferred label and with planned variants labelled convert; words of this corpus's test and of every accessed test are refused by their aliases."});
    Ok((result, report))
}

fn rounded_table(table: &BTreeMap<(i64, usize), f64>) -> Value {
    let mut map = Map::new();
    for ((group, length), value) in table {
        map.insert(format!("{group}:{length}"), json!(py_round_ndigits(*value, MASS_REPORT_DECIMALS).expect("a finite mass")));
    }
    Value::Object(map)
}

/// `balance_planned_mass`: planned lookahead frames given the mass of the situation they resolve.
pub fn balance_planned_mass(tools: &Tools, rows: Vec<ActionRow>) -> Curriculum {
    let mut isolated: BTreeMap<(i64, usize), f64> = BTreeMap::new();
    let mut planned: BTreeMap<(i64, usize), f64> = BTreeMap::new();
    for row in &rows {
        if !tools.lookahead_focus(&row.original, row.group) {
            continue;
        }
        let key = (row.group, pystr::len(&row.original));
        if row.after_origin == AfterOrigin::PlannedNextConversion {
            *planned.entry(key).or_insert(0.0) += row.sample_weight;
        } else if row.after_origin == AfterOrigin::None && matches!(row.action, ContextAction::Wait | ContextAction::Suggest) && !has_word(&row.field.before) && !has_word(&row.field.after) {
            *isolated.entry(key).or_insert(0.0) += row.sample_weight;
        }
    }
    let mut scale: BTreeMap<(i64, usize), f64> = BTreeMap::new();
    for (key, value) in &planned {
        if *value > 0.0 && isolated.get(key).copied().unwrap_or(0.0) > 0.0 {
            scale.insert(*key, isolated[key] / value);
        }
    }
    let input_mass = total(&rows);
    let mut result = rows;
    for row in &mut result {
        let key = (row.group, pystr::len(&row.original));
        if row.after_origin == AfterOrigin::PlannedNextConversion
            && let Some(factor) = scale.get(&key) {
                row.sample_weight *= factor;
            }
    }
    let output_mass = total(&result);
    let report = json!({"policy": "planned frames per (direction, focus length) carry the total mass of the isolated deferred frames of the same class",
        "isolated_mass": rounded_table(&isolated), "planned_mass_before": rounded_table(&planned), "scale": rounded_table(&scale), "input_mass": input_mass, "output_mass": output_mass});
    (result, report)
}
