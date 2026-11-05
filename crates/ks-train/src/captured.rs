//! The questions the engine asked while public text was typed the way a person types it (`tools/mixed_typing.py capture`),
//! pinned by SHA-256 in their manifests, and the curricula read from them.
use ks_cast::To as _;
use crate::corpus::{checksum, read_json};
use crate::curricula::{Counts, Curriculum, curated};
use crate::physical::{hex, variant_choice};
use crate::rows::{ActionRow, APPLICATIONS, cyrillic_letter, field_of, has_word, word_at_start};
use crate::tools::{Options, Tools};
use ks_backend::FieldRole;
use ks_context::context_action_features as features;
use ks_context::context_model::{ACTIONS, AfterOrigin, BeforeOrigin, ContextAction, ContextEvidence};
use ks_text::pystr;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use ContextAction::{Convert, Keep, Suggest, Wait};

const PLANNED_CONTEXT_WORD_MAX_CHARACTERS: usize = 3;
const CAPTURED_COLUMNS: [&str; 21] = [
    "count", "original", "alternative", "source_group", "trigger", "baseline_convert", "source_known", "target_known", "score_delta", "literal_tail", "boundary_text",
    "after_origin", "source_typo", "target_typo", "source_opening", "target_opening", "inside", "before", "after", "role", "label",
];

#[derive(Clone, Debug)]
pub struct CapturedSource {
    pub path: PathBuf,
    pub weight: f64,
    pub balanced: bool,
    pub columns: Vec<String>,
}

fn columns_of(version: i64) -> Option<Vec<String>> {
    let mut columns: Vec<String> = CAPTURED_COLUMNS.iter().map(|name| (*name).to_owned()).collect();
    match version {
        1 => Some(columns),
        2 => {
            let place = columns.iter().position(|name| name == "after_origin").expect("the column") + 1;
            columns.insert(place, "before_origin".to_owned());
            Some(columns)
        }
        _ => None,
    }
}

/// `captured_sources`: the question files with their weights, each checked against its pinned SHA-256.
pub fn captured_sources(manifest: &Path) -> Result<Vec<CapturedSource>, String> {
    let payload = read_json(manifest)?;
    let columns = payload.get("schema_version").and_then(Value::as_i64).and_then(columns_of).ok_or("invalid captured question manifest")?;
    let listed: Vec<&str> = payload.get("columns").and_then(Value::as_array).map(|items| items.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    if listed != columns.iter().map(String::as_str).collect::<Vec<_>>() {
        return Err("invalid captured question manifest".to_owned());
    }
    let parent = manifest.parent().ok_or("a manifest outside a folder")?;
    let mut sources = Vec::new();
    for source in payload.get("sources").and_then(Value::as_array).into_iter().flatten() {
        let path = parent.join(source["file"].as_str().ok_or("a source without a file")?);
        if Value::String(checksum(&path)?) != source["sha256"] {
            return Err(format!("captured questions changed: {}", path.display()));
        }
        let weight = source["weight"].as_f64().ok_or("a source without a weight")?;
        if !weight.is_finite() || weight <= 0.0 {
            return Err(format!("invalid weight or balance for {}", path.display()));
        }
        sources.push(CapturedSource { path, weight, balanced: source.get("balance").and_then(Value::as_bool).unwrap_or(true), columns: columns.clone() });
    }
    Ok(sources)
}

#[must_use]
pub fn role_of(name: &str) -> FieldRole {
    match name {
        "text" => FieldRole::Text,
        "code" => FieldRole::Code,
        "search" => FieldRole::Search,
        "terminal" => FieldRole::Terminal,
        "password" => FieldRole::Password,
        "unknown" => FieldRole::Unknown,
        _ => panic!("no field role {name}"),
    }
}

#[must_use]
pub fn after_origin_of(name: &str) -> AfterOrigin {
    match name {
        "none" => AfterOrigin::None,
        "field" => AfterOrigin::Field,
        "planned_next_conversion" => AfterOrigin::PlannedNextConversion,
        "kept_next_word" => AfterOrigin::KeptNextWord,
        _ => panic!("no origin {name}"),
    }
}

fn before_origin_of(name: &str) -> BeforeOrigin {
    match name {
        "field" => BeforeOrigin::Field,
        "planned_previous_reading" => BeforeOrigin::PlannedPreviousReading,
        _ => panic!("no origin {name}"),
    }
}

/// `captured_rows`: each distinct captured question once, with its label index and how often it was asked.
pub fn captured_rows(source: &CapturedSource) -> Result<Vec<(ContextEvidence, usize)>, String> {
    let file = std::fs::File::open(&source.path).map_err(|error| error.to_string())?;
    let mut text = Vec::new();
    xz2::read::XzDecoder::new(std::io::BufReader::new(file)).read_to_end(&mut text).map_err(|error| format!("{}: {error}", source.path.display()))?;
    let positions: HashMap<&str, usize> = source.columns.iter().enumerate().map(|(index, name)| (name.as_str(), index)).collect();
    let mut rows = Vec::new();
    for line in text.lines() {
        let line = line.map_err(|error| error.to_string())?;
        let values: Value = serde_json::from_str(&line).map_err(|error| error.to_string())?;
        let values = values.as_array().ok_or("a captured row is a list")?;
        if values.len() != source.columns.len() {
            return Err("a captured row with other columns".to_owned());
        }
        let value = |name: &str| &values[positions[name]];
        let text_of = |name: &str| value(name).as_str().expect("a text column");
        let flag = |name: &str| value(name).as_bool().expect("a flag column");
        // No application name, as in every scenario row: the model does not read it.
        let field = field_of("", "captured", text_of("before"), text_of("after"), role_of(text_of("role")));
        let mut item = ContextEvidence::new(text_of("original"), text_of("alternative"), value("source_group").as_i64().expect("a group"), field);
        item.trigger = text_of("trigger").to_owned();
        item.baseline_convert = flag("baseline_convert");
        item.source_known = flag("source_known");
        item.target_known = flag("target_known");
        item.score_delta = value("score_delta").as_f64().expect("a score");
        item.literal_tail = text_of("literal_tail").to_owned();
        item.boundary_text = text_of("boundary_text").to_owned();
        item.after_origin = after_origin_of(text_of("after_origin"));
        if let Some(index) = positions.get("before_origin") {
            item.before_origin = before_origin_of(values[*index].as_str().expect("a text column"));
        }
        item.source_typo = flag("source_typo");
        item.target_typo = flag("target_typo");
        item.source_opening = flag("source_opening");
        item.target_opening = flag("target_opening");
        item.inside = flag("inside");
        let label = ACTIONS.iter().position(|action| action.as_str() == text_of("label")).expect("a label");
        rows.push((item.finished(), label));
    }
    Ok(rows)
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
}

fn canonical_digest(value: &Value) -> [u8; 32] {
    Sha256::digest(crate::artifact::canonical(value)).into()
}

fn static_trigger(name: &str) -> &'static str {
    crate::evidence::trigger_of(name).as_str()
}

fn cyrillic_any(text: &str) -> bool {
    text.chars().any(cyrillic_letter)
}

/// `captured_source_curriculum`: the frames of one captured question file and its report.
pub fn captured_source_curriculum(tools: &Tools, source: &CapturedSource, options: &Value) -> Result<Curriculum, String> {
    let budget = options.int("maximum_rows_per_source");
    let convert_share = options.get("convert_share").filter(|value| !value.is_null()).map(|_| options.float("convert_share"));
    let keep_unknown_share = options.float("keep_unknown_share");
    let unknown_keep_groups: Vec<i64> = options["unknown_keep_groups"].as_array().ok_or("unknown_keep_groups")?.iter().filter_map(Value::as_i64).collect();
    let weight_scale = options.float("weight_scale");
    let minimum_characters = options.int("minimum_word_characters").to::<usize>();
    let excluded: Vec<&str> = options["excluded_file_markers"].as_array().ok_or("excluded_file_markers")?.iter().filter_map(Value::as_str).collect();
    let name = file_name(&source.path);
    if excluded.iter().any(|marker| name.contains(marker)) {
        return Ok((Vec::new(), json!({"excluded": true})));
    }
    let mut ranked: [Vec<([u8; 32], ContextEvidence)>; 2] = [Vec::new(), Vec::new()];
    let mut skipped_short = 0;
    for (item, label) in captured_rows(source)? {
        let action = ACTIONS[label];
        if !matches!(action, Keep | Convert) {
            continue;
        }
        if pystr::len(&item.original) < minimum_characters {
            // A lone letter is left out, as the corpus leaves it out.
            skipped_short += 1;
            continue;
        }
        if crate::physical::translated(&item.original, item.source_group).is_err() {
            continue;
        }
        let key = canonical_digest(&json!(["captured", name, item.original, item.source_group, item.trigger, item.literal_tail, item.boundary_text, item.after_origin.as_str(), item.field.before, item.field.after, item.field.role.as_str()]));
        ranked[usize::from(action == Convert)].push((key, item));
    }
    let [mut keeps_ranked, mut converts] = ranked;
    let available = (keeps_ranked.len(), converts.len());
    let share = convert_share.unwrap_or(available.1.to::<f64>() / ((available.0 + available.1).max(1)).to::<f64>());
    converts.sort_by_key(|a| a.0);
    converts.truncate((budget.to::<f64>() * share).to::<usize>());
    // Correctly typed words no lexicon knows are the keep answers a uniform sample nearly never draws.
    let keep_budget = budget.to::<usize>() - converts.len();
    keeps_ranked.sort_by_key(|a| a.0);
    let unknown: Vec<usize> = keeps_ranked
        .iter()
        .enumerate()
        .filter(|(_, entry)| unknown_keep_groups.contains(&entry.1.source_group) && !tools.plausible_reading(&entry.1.original, entry.1.source_group))
        .map(|(index, _)| index)
        .collect();
    let chosen_unknown: Vec<usize> = unknown.iter().copied().take((keep_budget.to::<f64>() * keep_unknown_share).to::<usize>()).collect();
    let taken: HashSet<usize> = chosen_unknown.iter().copied().collect();
    let mut keeps: Vec<usize> = chosen_unknown.clone();
    keeps.extend((0..keeps_ranked.len()).filter(|index| !taken.contains(index)).take(keep_budget - chosen_unknown.len()));
    let stem = source.path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    let category = format!("captured_{}", name.split('.').next().unwrap_or("").replace('-', "_"));
    let weight = source.weight * weight_scale;
    let mut counts = Counts::default();
    let mut result = Vec::new();
    let mut emit = |key: &[u8; 32], item: &ContextEvidence, action: ContextAction| {
        let identifier = format!("captured:{stem}:{}", hex(key));
        let application = APPLICATIONS[variant_choice(&identifier, "application", APPLICATIONS.len())];
        let field = field_of(application, "captured", &item.field.before, &item.field.after, item.field.role);
        let mut origin = item.after_origin;
        if origin == AfterOrigin::PlannedNextConversion && pystr::len(&item.original) > PLANNED_CONTEXT_WORD_MAX_CHARACTERS {
            origin = AfterOrigin::Field;
        }
        if origin == AfterOrigin::PlannedNextConversion && word_at_start(pystr::lstrip(&item.field.after)).is_none() {
            // The engine plans a next word only once one was typed.
            counts.add("skipped_planned_without_word");
            return;
        }
        let mut labelled = action;
        if origin != AfterOrigin::PlannedNextConversion
            && !curated(&item.original, &item.alternative)
            && !has_word(&item.field.before)
            && !has_word(&item.field.after)
            && tools.deferred_isolated(&item.original, &item.alternative, item.source_group)
        {
            labelled = if matches!(item.trigger.as_str(), "enter" | "tab" | "punctuation") { Suggest } else { Wait };
        }
        counts.add(labelled.as_str());
        let mut row = ActionRow::new(identifier, item.original.as_str(), item.source_group, field, static_trigger(&item.trigger), labelled, category.as_str(), item.boundary_text.as_str()).weighing(weight).originating(origin);
        row.literal_tail.clone_from(&item.literal_tail);
        result.push(row);
    };
    for (key, item) in &converts {
        emit(key, item, Convert);
    }
    for index in keeps {
        let (key, item) = &keeps_ranked[index];
        emit(key, item, Keep);
    }
    let report = json!({"available": {"keep": available.0, "convert": available.1}, "skipped_short": skipped_short, "unknown_keep_available": unknown.len(),
        "unknown_keep_chosen": chosen_unknown.len(), "chosen": counts.report(), "weight": weight});
    Ok((result, report))
}

/// `captured_head_curriculum`: a head's frames of one split: the questions about tokens of its class.
pub fn captured_head_curriculum(tools: &Tools, split: &str, options: &Value, question: &dyn Fn(&ContextEvidence) -> bool, category: &str) -> Result<Curriculum, String> {
    let named: Vec<(String, String)> = options
        .get("captured")
        .and_then(|table| table.get(split))
        .and_then(Value::as_array)
        .map(|items| items.iter().map(|pair| (pair[0].as_str().unwrap_or("").to_owned(), pair[1].as_str().unwrap_or("").to_owned())).collect())
        .unwrap_or_default();
    if named.is_empty() {
        return Ok((Vec::new(), json!({"frames": 0, "scope": "not used"})));
    }
    let weight = options.float("sample_weight");
    let convert_weight = options.float_or("convert_weight", 1.0);
    let token_cap = options.int_or("token_cap", 0);
    let mut rows = Vec::new();
    let mut counts = Counts::default();
    for (manifest, name) in &named {
        let sources = captured_sources(&tools.root.join(manifest))?;
        let source = sources.iter().find(|item| file_name(&item.path) == *name).ok_or_else(|| format!("no captured file {name}"))?;
        for (item, label) in captured_rows(source)? {
            let action = ACTIONS[label];
            if !matches!(action, Keep | Convert) || !question(&item) {
                continue;
            }
            let mut origin = item.after_origin;
            if origin == AfterOrigin::PlannedNextConversion && pystr::len(&item.original) > PLANNED_CONTEXT_WORD_MAX_CHARACTERS {
                origin = AfterOrigin::Field;
            }
            if origin == AfterOrigin::KeptNextWord && !features::kept_context_reachable(&item.original, &item.trigger, &item.boundary_text, &item.field.after) {
                // A next word kept as typed that the engine passes no further: no such question is asked.
                counts.add("skipped_unreachable_kept");
                continue;
            }
            if origin == AfterOrigin::PlannedNextConversion && word_at_start(pystr::lstrip(&item.field.after)).is_none() {
                counts.add("skipped_planned_without_word");
                continue;
            }
            // The left context's origin joins the key only where it is a hypothesis.
            let mut key = vec![
                json!(category), json!(name), json!(item.original), json!(item.source_group), json!(item.trigger), json!(item.literal_tail), json!(item.boundary_text),
                json!(origin.as_str()), json!(item.field.before), json!(item.field.after), json!(item.field.role.as_str()),
            ];
            if item.before_origin != BeforeOrigin::Field {
                key.push(json!(item.before_origin.as_str()));
            }
            let identifier = format!("{category}:{}", hex(&canonical_digest(&Value::Array(key))));
            let application = APPLICATIONS[variant_choice(&identifier, "application", APPLICATIONS.len())];
            let field = field_of(application, "captured", &item.field.before, &item.field.after, item.field.role);
            counts.add(format!("{name}:{}", action.as_str()));
            let mass = weight * if action == Convert { convert_weight } else { 1.0 };
            let mut row = ActionRow::new(identifier, item.original.as_str(), item.source_group, field, static_trigger(&item.trigger), action, category, item.boundary_text.as_str()).weighing(mass).originating(origin);
            row.literal_tail.clone_from(&item.literal_tail);
            row.before_origin = item.before_origin;
            rows.push(row);
        }
    }
    let mut report = json!({"frames": rows.len(), "by_file": counts.report(), "sample_weight": weight});
    if token_cap != 0 {
        // At most token_cap frames of one token with one answer, in the order of their identifiers.
        let mut taken: BTreeMap<(String, &'static str), i64> = BTreeMap::new();
        let total = rows.len();
        rows.sort_by(|a, b| a.identifier.cmp(&b.identifier));
        rows.retain(|row| {
            let count = taken.entry((pystr::casefold(&row.original), row.action.as_str())).or_insert(0);
            *count += 1;
            *count <= token_cap
        });
        report["frames"] = json!(rows.len());
        report["capped"] = json!(total - rows.len());
        report["token_cap"] = json!(token_cap);
    }
    if convert_weight != 1.0 {
        report["convert_weight"] = json!(convert_weight);
    }
    Ok((rows, report))
}

/// `any cyrillic` of `natural_anchors`.
#[must_use]
pub fn group_of_word(text: &str) -> i64 {
    i64::from(cyrillic_any(text))
}
