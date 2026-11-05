//! The base of a context corpus, from pinned public sources (`freeze_context_action_corpus`,
//! `reconcile_context_action_corpus`, `context_technical_corpus`, `merge_context_action_corpora`,
//! `prefix_exposure_inventory`). No model, dictionary score or text of a test row takes part in selection, and no test
//! text is printed. `freeze` and `technical` write rows; `reconcile` closes the physical families over them; `merge` joins
//! the two without reading a test row; `inventory` lists what the prefix model has seen.
use ks_cast::To as _;
use crate::corpus::{CorpusRow, checksum, read_json, typo_variants};
use crate::evaluate::{select_rows, sequence_plan};
use crate::freeze::{Command, Sentence, SurfaceToken, Union, family_aliases, read_commands, read_conllu, row_identifier, select_sentences, sentence_rows, source_inventory};
use crate::physical::{expanded_aliases, hex, hex_digest, translated};
use flate2::Compression;
use flate2::read::MultiGzDecoder;
use flate2::write::GzEncoder;
use ks_context::context_action_features::words_of;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};

const ALL_SPLITS: [&str; 5] = ["train", "development", "calibration", "test", "quarantine"];
const ACTIVE_SPLITS: [&str; 4] = ["train", "development", "calibration", "test"];
const FITTING_SPLITS: [&str; 3] = ["train", "development", "calibration"];
pub const BASE_PINS: [(&str, &str); 3] = [("UD_Russian-Taiga", "d1e48cd29c1f19b6aad1aec361d9f724db764f66"), ("UD_English-EWT", "b7711cce01cdd4f5fcc0a8199b8a50d951b16c0c"), ("UD_Russian-SynTagRus", "6377522610550b696fcc70d39074d2ce03da0e7b")];
pub const DEFAULT_MAX_DOCUMENTS_PER_SOURCE: usize = 2_000;
pub const DEFAULT_MAX_SENTENCES_PER_DOCUMENT: usize = 12;
const SPLIT_BUCKET_COUNT: u64 = 100;
const TRAIN_CEILING: u64 = 70;
const DEVELOPMENT_CEILING: u64 = 80;
const CALIBRATION_CEILING: u64 = 90;
const DRAW_HEX_DIGITS: usize = 16;
const MAJORITY_DIVISOR: usize = 2;
const COMMANDS_PER_ALIAS_FAMILY: usize = 2;
const TYPO_SOURCE_MIN_CHARACTERS: usize = 4;
const TYPO_SOURCE_MAX_CHARACTERS: usize = 64;
const TRANSPOSE_TAIL_MARGIN_CHARACTERS: usize = 2;
const COMMAND_NAME_MIN_CHARACTERS: usize = 3;
const COMMAND_NAME_MAX_CHARACTERS: usize = 16;
const TECHNICAL_NAMESPACE: &str = "keyswitch:context-action:debian-trixie-20260912-v1";
const ALIAS_SCOPE: &str = "expanded physical aliases of each held-out row's form, lemmas and typo variants";
pub const HISTORICAL_CODE_FORMS: &str = "model/context_v1/historical-code-forms.json";
const FREEZER_SOURCE: &str = "crates/ks-train/src/base_corpus.rs";
const LAYOUT_TABLE: &str = "crates/ks-text/src/layouts.rs";
const MEMBERSHIP_FIELDS: [&str; 3] = ["row_ids_sha256", "family_ids_sha256", "document_ids_sha256"];
const PREFIX_SPLITS: [&str; 3] = ["train", "development", "calibration"];

fn canonical(value: &Value) -> Vec<u8> {
    crate::artifact::canonical(value)
}

/// `assigned_split`.
fn assigned_split(namespace: &str, key: &str) -> &'static str {
    let digest = hex_digest(&format!("{namespace}:{key}"));
    let bucket = u64::from_str_radix(digest.get(..DRAW_HEX_DIGITS).unwrap_or_default(), 16).expect("a hexadecimal digest") % SPLIT_BUCKET_COUNT;
    if bucket < TRAIN_CEILING {
        "train"
    } else if bucket < DEVELOPMENT_CEILING {
        "development"
    } else if bucket < CALIBRATION_CEILING {
        "calibration"
    } else {
        "test"
    }
}

/// `partition`: documents hashed into splits; what would cross them, or was seen before, into quarantine.
fn partition(sentences: &[Sentence], namespace: &str, exposed: &HashSet<String>) -> Result<(Vec<CorpusRow>, Value), String> {
    let mut families = Union::default();
    for sentence in sentences {
        for token in &sentence.tokens {
            let aliases = family_aliases(token, &row_identifier(sentence, token));
            for alias in &aliases {
                families.join(&aliases[0], alias);
            }
        }
    }
    let exposed_aliases: Vec<&String> = exposed.iter().filter(|alias| families.parents.contains_key(*alias)).collect();
    let exposed_roots: BTreeSet<String> = exposed_aliases.into_iter().map(|alias| families.find(alias)).collect();
    let mut rows: Vec<CorpusRow> = Vec::new();
    for sentence in sentences {
        rows.extend(sentence_rows(sentence, &mut families));
    }
    let identifiers: HashSet<&str> = rows.iter().map(|row| row.identifier.as_str()).collect();
    if identifiers.len() != rows.len() {
        return Err("duplicate source row identifiers".to_owned());
    }
    let mut full = Union::default();
    let documents: BTreeSet<String> = rows.iter().map(|row| row.document.clone()).collect();
    for row in &rows {
        full.join(&format!("document:{}", row.document), &format!("family:{}", row.family));
    }
    let mut component_documents: HashMap<String, usize> = HashMap::new();
    for document in &documents {
        *component_documents.entry(full.find(&format!("document:{document}"))).or_default() += 1;
    }
    let giant = component_documents.values().copied().max().unwrap_or(0);
    let conflict_mode = !documents.is_empty() && giant > 1.max(documents.len() / MAJORITY_DIVISOR);
    let mut assignment: HashMap<String, &'static str> = HashMap::new();
    for document in &documents {
        let key = if conflict_mode { format!("document:{document}") } else { full.find(&format!("document:{document}")) };
        assignment.insert(document.clone(), assigned_split(namespace, &key));
    }
    let mut splits_by_family: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
    for row in &rows {
        splits_by_family.entry(row.family.clone()).or_default().insert(assignment[&row.document]);
    }
    let exposed_ids: HashSet<String> = exposed_roots.iter().map(|root| hex_digest(root)).collect();
    let mut reasons_count: BTreeMap<String, i64> = BTreeMap::new();
    for row in &mut rows {
        let mut split = assignment[&row.document];
        let mut reasons = Vec::new();
        if conflict_mode && splits_by_family[&row.family].len() > 1 {
            reasons.push("family-crosses-document-splits".to_owned());
        }
        if split != "train" && exposed_ids.contains(&row.family) {
            reasons.push("family-previously-exposed".to_owned());
        }
        if row.alignment != "exact-text-comment" {
            reasons.push(row.alignment.clone());
        }
        if !reasons.is_empty() {
            split = "quarantine";
            for reason in &reasons {
                *reasons_count.entry(reason.clone()).or_default() += 1;
            }
        }
        row.split = split.to_owned();
        row.quarantine_reasons = reasons;
    }
    rows.sort_by(|a, b| a.identifier.cmp(&b.identifier));
    let summary = json!({"mode": if conflict_mode { "document-hash-with-family-conflict-quarantine" } else { "whole-document-lexeme-components" }, "documents": documents.len(), "union_components": component_documents.len(), "largest_component_documents": giant, "family_components": splits_by_family.len(), "exposed_family_components": exposed_ids.len(), "quarantine_reasons": reasons_count, "policy": "70/10/10/10 by SHA256; conflicting rows retained in quarantine; no model or dictionary scores"});
    Ok((rows, summary))
}

/// `row_aliases`: every physical alias of a row's word: its form, its lemmas and its typo variants.
fn row_aliases(row: &CorpusRow) -> BTreeSet<String> {
    let mut aliases = BTreeSet::new();
    let mut values: Vec<String> = vec![row.original.clone()];
    values.extend(row.lemma.split(' ').map(str::to_owned));
    values.extend(typo_variants(&row.original, &row.identifier));
    for value in values {
        if !value.is_empty() {
            aliases.extend(expanded_aliases(&value));
        }
    }
    aliases
}

/// `test_membership` of the original freeze and of a reconciliation.
fn test_membership(rows: &[CorpusRow], namespace: &str) -> Value {
    let held: Vec<&CorpusRow> = rows.iter().filter(|row| row.split == "test").collect();
    let mut row_ids: Vec<String> = held.iter().map(|row| hex_digest(&row.identifier)).collect();
    row_ids.sort();
    let families: BTreeSet<&str> = held.iter().map(|row| row.family.as_str()).collect();
    let documents: BTreeSet<String> = held.iter().map(|row| hex_digest(&row.document)).collect();
    let mut aliases: BTreeSet<String> = BTreeSet::new();
    for row in &held {
        aliases.extend(row_aliases(row));
    }
    json!({"namespace": namespace, "scope": "prospective sequence holdout; not globally unseen lexicon", "row_ids_sha256": row_ids, "family_ids_sha256": families, "document_ids_sha256": documents, "alias_sha256": aliases, "alias_scope": ALIAS_SCOPE})
}

fn write_split(path: &Path, rows: &[&CorpusRow]) -> Result<Value, String> {
    let mut hasher = Sha256::new();
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    for row in rows {
        let line = row.line();
        hasher.update(&line);
        encoder.write_all(&line).map_err(|error| error.to_string())?;
    }
    std::fs::write(path, encoder.finish().map_err(|error| error.to_string())?).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(json!({"path": path.file_name().map(|name| name.to_string_lossy().into_owned()), "sha256": checksum(path)?, "content_sha256": hex(&hasher.finalize()), "rows": rows.len()}))
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    std::fs::write(path, canonical(value)).map_err(|error| format!("{}: {error}", path.display()))
}

fn begin(output: &Path) -> Result<(), String> {
    if output.exists() {
        return Err("refusing to overwrite a frozen corpus directory".to_owned());
    }
    std::fs::create_dir_all(output).map_err(|error| format!("{}: {error}", output.display()))
}

fn generator(root: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(root.join(FREEZER_SOURCE)).map_err(|error| format!("{FREEZER_SOURCE}: {error}"))
}

/// The sentences a base is made of, read as `audit` and `freeze` read them.
fn base_sentences(source_root: &Path, namespace: &str, max_documents: usize, max_sentences: usize) -> Result<(Vec<Sentence>, Value, Vec<Value>), String> {
    let (paths, sources) = source_inventory(source_root, &BASE_PINS)?;
    let mut sentences = Vec::new();
    for (source, path) in &paths {
        sentences.extend(read_conllu(path, source)?);
    }
    let (selected, sampling) = select_sentences(sentences, namespace, max_documents, max_sentences, false)?;
    Ok((selected, sampling, sources))
}

/// `freeze_context_action_corpus`.
pub fn freeze_base(root: &Path, source_root: &Path, output: &Path, namespace: &str, extra_exposure: &[PathBuf], max_documents: usize, max_sentences: usize) -> Result<Value, String> {
    if max_documents < 1 || max_sentences < 1 {
        return Err("document and sentence bounds must be positive".to_owned());
    }
    let (exposed, provenance, _) = crate::freeze::exposed_families(root, extra_exposure)?;
    let (sentences, sampling, sources) = base_sentences(source_root, namespace, max_documents, max_sentences)?;
    let (rows, partitioning) = partition(&sentences, namespace, &exposed)?;
    begin(output)?;
    let source = generator(root)?;
    std::fs::write(output.join("generator-source.rs"), &source).map_err(|error| error.to_string())?;
    let mut files = Map::new();
    for split in ALL_SPLITS {
        let selected: Vec<&CorpusRow> = rows.iter().filter(|row| row.split == split).collect();
        files.insert(split.to_owned(), write_split(&output.join(format!("{split}.jsonl.gz")), &selected)?);
    }
    let membership_path = output.join("test-membership.json");
    write_json(&membership_path, &test_membership(&rows, namespace))?;
    let mut sorted_exposed: Vec<&String> = exposed.iter().collect();
    sorted_exposed.sort();
    let joined: Vec<&str> = sorted_exposed.iter().map(|text| text.as_str()).collect();
    let manifest = json!({
        "schema_version": 1, "namespace": namespace, "label": "keep",
        "scope": "prospective document/physical-lemma sequence holdout; not globally unseen lexicon or verified human language intent",
        "token_language": "surface alphabet determines RU/Latin group; foreign Latin text is not asserted to be English; mixed/und group is null",
        "context": "within the same sentence only; before<=96 and after<=64 characters; punctuation/whitespace preserved",
        "sources": sources, "sampling": sampling, "partitioning": partitioning,
        "exposure_inputs": provenance,
        "exposed_physical_families": exposed.len(),
        "exposed_membership_sha256": hex_digest(&joined.join("\n")),
        "typo_policy": "only exported typo_variants(original,identifier); all three aliases joined before splitting; old context-v1 scenarios quarantine every internal deletion/duplication/transposition",
        "splits": files, "test_membership_sha256": checksum(&membership_path)?,
        "generator_sha256": hex(&Sha256::digest(&source)),
        "generator_source": "generator-source.rs",
        "layout_table_sha256": checksum(&root.join(LAYOUT_TABLE))?,
        "compression": "gzip mtime=0; raw content SHA256 is independent of deflate implementation",
        "test_policy": "sealed before model scoring; loader reads only the explicitly named split; quarantine is never a training split",
        "limitations": ["UD text is naturally occurring, not reviewed keyboard-intent ground truth", "unmarked source documents are conservatively grouped by file; anonymous newdoc boundaries remain local to that file", "past morphology, dictionaries, private experiments and pretrained models are not exhaustively inventoried", "identical documents under different upstream IDs and paraphrases are not detected"],
    });
    write_json(&output.join("manifest.json"), &manifest)?;
    Ok(manifest)
}

fn family_values(token: &SurfaceToken, identifier: &str) -> Vec<String> {
    let mut values = vec![token.form.clone()];
    values.extend(token.lemmas.iter().cloned());
    values.extend(typo_variants(&token.form, identifier));
    values
}

fn historical_code_forms(root: &Path) -> Result<Vec<String>, String> {
    let forms = read_json(&root.join(HISTORICAL_CODE_FORMS))?;
    forms.as_array().ok_or("historical code forms must be a list")?.iter().map(|form| form.as_str().map(str::to_owned).ok_or_else(|| "a historical form must be text".to_owned())).collect()
}

/// `sequence_document_counts`: the evaluator's model-blind selection and physical representability.
fn sequence_document_counts(rows: &[CorpusRow]) -> Value {
    let held: Vec<CorpusRow> = rows.iter().filter(|row| row.split == "test").cloned().collect();
    let selected = select_rows(&held);
    let mut supported: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
    let mut unsupported: BTreeMap<String, i64> = BTreeMap::new();
    let name_of = |group: Option<i64>| group.map_or_else(|| "None".to_owned(), |group| group.to_string());
    for row in &selected {
        let name = name_of(row.group);
        let mut representable = sequence_plan(row, false).unsupported.is_empty();
        if matches!(row.group, Some(0 | 1)) {
            let has_letter = row.original.chars().any(|character| crate::evaluate::script_group(character) == row.group);
            representable = representable && has_letter && sequence_plan(row, true).unsupported.is_empty();
        }
        if representable {
            supported.entry(name).or_default().insert(row.document.as_str());
        } else {
            *unsupported.entry(name).or_default() += 1;
        }
    }
    let by_group: Map<String, Value> = [Some(0), Some(1), None].into_iter().map(|group| { let name = name_of(group); let count = supported.get(&name).map_or(0, BTreeSet::len); (name, json!(count)) }).collect();
    json!({"selected_rows": selected.len(), "screenable_documents_by_group": by_group, "unsupported_selected_by_group": unsupported, "scope": "physical representability only; no model scoring"})
}

/// `Reconciled`.
struct Reconciled {
    rows: Vec<CorpusRow>,
    sidecar: Value,
    summary: Map<String, Value>,
}

fn count_by_split(rows: &[CorpusRow]) -> BTreeMap<String, i64> {
    let mut counts = BTreeMap::new();
    for row in rows {
        *counts.entry(row.split.clone()).or_default() += 1;
    }
    counts
}

/// `reconcile_rows`: physical closure over the original families; test keeps only what no fit has seen.
fn reconcile_rows(sentences: &[Sentence], rows: &[CorpusRow], historical: &HashSet<String>) -> Result<Reconciled, String> {
    let mut union = Union::default();
    let by_id: HashMap<&str, &CorpusRow> = rows.iter().map(|row| (row.identifier.as_str(), row)).collect();
    let mut values_by_id: HashMap<String, Vec<String>> = HashMap::new();
    for sentence in sentences {
        for token in &sentence.tokens {
            let identifier = row_identifier(sentence, token);
            let row = by_id.get(identifier.as_str()).ok_or("a source token without its row")?;
            let values = family_values(token, &identifier);
            let anchor = format!("family:{}", row.family);
            union.find(&anchor);
            for value in &values {
                for alias in expanded_aliases(value) {
                    union.join(&anchor, &format!("alias:{alias}"));
                }
            }
            values_by_id.insert(identifier, values);
        }
    }
    if values_by_id.len() != by_id.len() {
        return Err("source alias inventory does not cover original rows".to_owned());
    }
    fn matching_roots<'a>(union: &mut Union, forms: impl IntoIterator<Item = &'a str>, roots: &mut BTreeSet<String>) {
        for form in forms {
            for alias in expanded_aliases(form) {
                let key = format!("alias:{alias}");
                if union.parents.contains_key(&key) {
                    roots.insert(union.find(&key));
                }
            }
        }
    }
    let mut historical_roots = BTreeSet::new();
    matching_roots(&mut union, historical.iter().map(String::as_str), &mut historical_roots);
    let mut seen_roots = historical_roots.clone();
    for row in rows {
        if !FITTING_SPLITS.contains(&row.split.as_str()) {
            continue;
        }
        seen_roots.insert(union.find(&format!("family:{}", row.family)));
        // Prior fit saw context words too, including tokens whose focus row was quarantined. A future drop cannot erase that.
        let mut forms: Vec<&str> = values_by_id[&row.identifier].iter().map(String::as_str).collect();
        forms.extend(words_of(&row.before));
        forms.extend(words_of(&row.after));
        matching_roots(&mut union, forms, &mut seen_roots);
    }
    let mut splits_by_root: HashMap<String, BTreeSet<&str>> = HashMap::new();
    for row in rows {
        if ACTIVE_SPLITS.contains(&row.split.as_str()) {
            splits_by_root.entry(union.find(&format!("family:{}", row.family))).or_default().insert(row.split.as_str());
        }
    }
    let conflicts: HashSet<&String> = splits_by_root.iter().filter(|(_, splits)| splits.len() > 1).map(|(root, _)| root).collect();
    let mut test_roots: BTreeSet<String> = BTreeSet::new();
    for row in rows.iter().filter(|row| row.split == "test") {
        test_roots.insert(union.find(&format!("family:{}", row.family)));
    }
    let contaminated: BTreeSet<&String> = test_roots.intersection(&seen_roots).collect();
    let mut changed: Vec<CorpusRow> = Vec::with_capacity(rows.len());
    let mut removed: Vec<Value> = Vec::new();
    let original_counts = count_by_split(rows);
    for row in rows {
        let root = union.find(&format!("family:{}", row.family));
        let mut reasons = row.quarantine_reasons.clone();
        if ACTIVE_SPLITS.contains(&row.split.as_str()) {
            if conflicts.contains(&root) {
                reasons.push("shift-closure-crosses-original-splits".to_owned());
            }
            if row.split != "train" && historical_roots.contains(&root) {
                reasons.push("shift-closure-historical-exposure".to_owned());
            }
            if row.split == "test" && contaminated.contains(&root) {
                reasons.push("shift-closure-prior-fit-exposure".to_owned());
            }
            if row.split != "test" && matches!(row.group, Some(0 | 1)) && row.layout_representable {
                let group = row.group.expect("a group");
                let mut forms = vec![row.original.clone()];
                forms.extend(typo_variants(&row.original, &row.identifier));
                if forms.iter().any(|value| translated(value, group).is_err()) {
                    reasons.push("unsupported-exact-physical-glyph".to_owned());
                }
            }
        }
        let mut next = row.clone();
        if row.split != "quarantine" && !reasons.is_empty() {
            let unique: BTreeSet<String> = reasons.into_iter().collect();
            let sorted: Vec<String> = unique.into_iter().collect();
            removed.push(json!({"row_sha256": hex_digest(&row.identifier), "previous_split": row.split, "closure_sha256": hex_digest(&root), "reasons": sorted}));
            "quarantine".clone_into(&mut next.split);
            next.quarantine_reasons = sorted;
        } else if FITTING_SPLITS.contains(&row.split.as_str()) {
            next.family = hex_digest(&root);
        }
        changed.push(next);
    }
    let mut retained: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for row in &changed {
        if ACTIVE_SPLITS.contains(&row.split.as_str()) {
            let old = by_id[row.identifier.as_str()];
            retained.entry(row.split.clone()).or_default().insert(union.find(&format!("family:{}", old.family)));
        }
    }
    let empty = BTreeSet::new();
    for (index, left) in ACTIVE_SPLITS.iter().enumerate() {
        for right in &ACTIVE_SPLITS[index + 1..] {
            if retained.get(*left).unwrap_or(&empty).intersection(retained.get(*right).unwrap_or(&empty)).next().is_some() {
                return Err("remaining partitions share physical closure".to_owned());
            }
        }
    }
    let retained_test = retained.get("test").unwrap_or(&empty);
    if retained_test.intersection(&seen_roots).next().is_some() {
        return Err("remaining test shares previously exposed physical closure".to_owned());
    }
    let names: Vec<String> = union.parents.keys().cloned().collect();
    let mut components: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut aliases: BTreeMap<String, String> = BTreeMap::new();
    for name in names {
        let component = hex_digest(&union.find(&name));
        if let Some(family) = name.strip_prefix("family:") {
            components.entry(component).or_default().push(family.to_owned());
        } else if let Some(alias) = name.strip_prefix("alias:") {
            aliases.insert(alias.to_owned(), component);
        }
    }
    for families in components.values_mut() {
        families.sort();
    }
    let counts = count_by_split(&changed);
    let name_of = |group: Option<i64>| group.map_or_else(|| "None".to_owned(), |group| group.to_string());
    let documents: Map<String, Value> = [Some(0), Some(1), None].into_iter().map(|group| { let set: BTreeSet<&str> = changed.iter().filter(|row| row.split == "test" && row.group == group).map(|row| row.document.as_str()).collect(); (name_of(group), json!(set.len())) }).collect();
    let removed_counts: Map<String, Value> = ACTIVE_SPLITS.iter().map(|split| ((*split).to_owned(), json!(original_counts.get(*split).unwrap_or(&0) - counts.get(*split).unwrap_or(&0)))).collect();
    let mut test_contaminated_rows = 0;
    for row in rows.iter().filter(|row| row.split == "test") {
        if contaminated.contains(&union.find(&format!("family:{}", row.family))) {
            test_contaminated_rows += 1;
        }
    }
    let mut summary = Map::new();
    summary.insert("original_counts".to_owned(), json!(original_counts));
    summary.insert("remaining_counts".to_owned(), json!(counts));
    summary.insert("removed_counts".to_owned(), Value::Object(removed_counts));
    summary.insert("test_contaminated_rows".to_owned(), json!(test_contaminated_rows));
    summary.insert("test_contaminated_components".to_owned(), json!(contaminated.len()));
    summary.insert("remaining_test_documents_by_group".to_owned(), Value::Object(documents));
    summary.insert("cross_split_components".to_owned(), json!(conflicts.len()));
    summary.insert("prior_exposed_components".to_owned(), json!(seen_roots.len()));
    summary.insert("remaining_cross_split_overlap".to_owned(), json!(0));
    summary.insert("remaining_test_prior_exposure_overlap".to_owned(), json!(0));
    summary.insert("replacements".to_owned(), json!(0));
    summary.insert("scope".to_owned(), json!("blind physical-family isolation; no scoring, predictions, test examples or quality claims"));
    summary.insert("physical_selection".to_owned(), sequence_document_counts(&changed));
    let closures = |roots: &BTreeSet<String>| { let digests: BTreeSet<String> = roots.iter().map(|root| hex_digest(root)).collect(); json!(digests) };
    let sidecar = json!({"schema_version": 1, "family_policy": "physical positions, Shift ignored; old aliases plus declared exact and old-layout variants", "components": components, "alias_sha256_to_closure_sha256": aliases, "prior_exposed_closure_sha256": closures(&seen_roots), "parent_test_closure_sha256": closures(&test_roots), "retained_test_closure_sha256": closures(retained_test), "removed": removed});
    Ok(Reconciled { rows: changed, sidecar, summary })
}

/// `validate_original`: the rows rebuilt here are the ones the parent corpus holds.
fn validate_original(directory: &Path, manifest: &Value, rows: &[CorpusRow]) -> Result<(), String> {
    for split in ALL_SPLITS {
        let record = &manifest["splits"][split];
        if record["path"].as_str() != Some(&format!("{split}.jsonl.gz")) || record["sha256"].as_str() != Some(&checksum(&directory.join(format!("{split}.jsonl.gz")))?) {
            return Err("original compressed split changed".to_owned());
        }
        let mut hasher = Sha256::new();
        let mut count = 0;
        for row in rows.iter().filter(|row| row.split == split) {
            hasher.update(row.line());
            count += 1;
        }
        if record["rows"].as_u64() != Some(count) || record["content_sha256"].as_str() != Some(&hex(&hasher.finalize())) {
            return Err("reconstructed original split differs".to_owned());
        }
    }
    let membership = directory.join("test-membership.json");
    if manifest["test_membership_sha256"].as_str() != Some(&checksum(&membership)?) {
        return Err("original test membership changed".to_owned());
    }
    if canonical(&test_membership(rows, manifest["namespace"].as_str().unwrap_or(""))) != std::fs::read(&membership).map_err(|error| error.to_string())? {
        return Err("reconstructed test membership differs".to_owned());
    }
    Ok(())
}

/// `reconcile_context_action_corpus`: the report, and with `output` the derivative. A failure names only its kind: no
/// source sentence, row identifier or test text reaches the output of this tool.
pub fn reconcile(root: &Path, directory: &Path, source_root: &Path, report: &Path, output: Option<&Path>) -> Result<Value, String> {
    let manifest = read_json(&directory.join("manifest.json"))?;
    let namespace = manifest["namespace"].as_str().ok_or("a namespace")?.to_owned();
    let sampling = &manifest["sampling"];
    let bound = |name: &str| sampling[name].as_u64().map(ks_cast::To::to::<usize>).ok_or_else(|| format!("sampling.{name}"));
    let (sentences, metadata, sources) = base_sentences(source_root, &namespace, bound("max_documents_per_source")?, bound("max_sentences_per_document")?)?;
    if Value::Array(sources) != manifest["sources"] {
        return Err("pinned source inventory differs from original freeze".to_owned());
    }
    if &metadata != sampling {
        return Err("source sampling changed".to_owned());
    }
    let inputs: Vec<PathBuf> = manifest["exposure_inputs"].as_object().ok_or("exposure inputs")?.keys().map(|name| root.join(name)).collect();
    let (exposed, provenance, mut originals) = crate::freeze::exposed_families(root, &inputs)?;
    if json!(provenance) != manifest["exposure_inputs"] {
        return Err("historical exposure input changed".to_owned());
    }
    let mut sorted_exposed: Vec<&str> = exposed.iter().map(String::as_str).collect();
    sorted_exposed.sort_unstable();
    if manifest["exposed_membership_sha256"].as_str() != Some(hex_digest(&sorted_exposed.join("\n")).as_str()) {
        return Err("historical exposure reconstruction differs".to_owned());
    }
    originals.extend(historical_code_forms(root)?);
    let (rows, partitioning) = partition(&sentences, &namespace, &exposed)?;
    if partitioning != manifest["partitioning"] {
        return Err("original partitioning changed".to_owned());
    }
    validate_original(directory, &manifest, &rows)?;
    let mut result = reconcile_rows(&sentences, &rows, &originals)?;
    result.summary.insert("parent_manifest_sha256".to_owned(), json!(checksum(&directory.join("manifest.json"))?));
    result.summary.insert("parent_test_membership_sha256".to_owned(), manifest["test_membership_sha256"].clone());
    result.summary.insert("namespace".to_owned(), json!(namespace));
    result.summary.insert("original_reconstruction_verified".to_owned(), json!(true));
    let summary = Value::Object(result.summary.clone());
    if let Some(parent) = report.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    write_json(report, &summary)?;
    if let Some(output) = output {
        write_derivative(root, directory, output, &result, &manifest, &namespace)?;
    }
    Ok(summary)
}

fn write_derivative(root: &Path, directory: &Path, output: &Path, result: &Reconciled, manifest: &Value, namespace: &str) -> Result<(), String> {
    begin(output)?;
    let mut files = Map::new();
    for split in ALL_SPLITS {
        let selected: Vec<&CorpusRow> = result.rows.iter().filter(|row| row.split == split).collect();
        let path = output.join(format!("{split}.jsonl.gz"));
        let mut hasher = Sha256::new();
        for row in &selected {
            hasher.update(row.line());
        }
        if split == "test" && manifest["splits"][split]["content_sha256"].as_str() == Some(hex(&hasher.clone().finalize()).as_str()) {
            std::fs::copy(directory.join(format!("{split}.jsonl.gz")), &path).map_err(|error| error.to_string())?;
            files.insert(split.to_owned(), json!({"path": format!("{split}.jsonl.gz"), "sha256": checksum(&path)?, "content_sha256": hex(&hasher.finalize()), "rows": selected.len()}));
        } else {
            files.insert(split.to_owned(), write_split(&path, &selected)?);
        }
    }
    let membership_path = output.join("test-membership.json");
    write_json(&membership_path, &test_membership(&result.rows, namespace))?;
    let closure_path = output.join("physical-family-closure.json");
    write_json(&closure_path, &result.sidecar)?;
    let generator_name = manifest["generator_source"].as_str().unwrap_or("generator-source.rs");
    std::fs::copy(directory.join(generator_name), output.join(generator_name)).map_err(|error| error.to_string())?;
    let source = generator(root)?;
    std::fs::write(output.join("reconciliation-source.rs"), &source).map_err(|error| error.to_string())?;
    let mut updated = manifest.clone();
    updated["splits"] = Value::Object(files);
    updated["test_membership_sha256"] = json!(checksum(&membership_path)?);
    updated["parent_manifest_sha256"] = result.summary["parent_manifest_sha256"].clone();
    updated["parent_test_membership_sha256"] = manifest["test_membership_sha256"].clone();
    updated["generator_role"] = json!("unchanged historical freezer; derivative generated by reconciliation-source.rs");
    updated["reconciliation"] = json!({"summary": result.summary, "closure_sha256": checksum(&closure_path)?, "source_sha256": hex(&Sha256::digest(&source)), "physical_keys_sha256": checksum(&root.join("crates/ks-train/src/physical.rs"))?, "test_policy": "parent test minus all formally exposed closure; no replacements or new namespace"});
    write_json(&output.join("manifest.json"), &updated)
}

fn command_identifier(name: &str) -> String {
    format!("debian-trixie-main-amd64:command:{name}")
}

fn command_aliases(command: &Command) -> BTreeSet<String> {
    let mut aliases = expanded_aliases(&command.name);
    for form in typo_variants(&command.name, &command_identifier(&command.name)) {
        aliases.extend(expanded_aliases(&form));
    }
    aliases
}

fn package_of(owner: &str) -> &str {
    owner.rsplit('/').next().unwrap_or(owner)
}

/// `Partitioned`.
struct Partitioned {
    rows: Vec<CorpusRow>,
    commands: Vec<Command>,
    aliases: BTreeMap<String, BTreeSet<String>>,
    summary: Value,
}

/// `partition_commands`: package edges join whole alias families before any exclusion or cap.
fn partition_commands(commands: Vec<Command>, reserved: &BTreeSet<String>) -> Result<Partitioned, String> {
    let mut records = commands;
    records.sort_by(|a, b| a.name.cmp(&b.name));
    let unique: BTreeSet<&str> = records.iter().map(|item| item.name.as_str()).collect();
    if unique.len() != records.len() || records.iter().any(|item| item.owners.is_empty()) {
        return Err("commands must have unique names and nonempty package owners".to_owned());
    }
    let aliases: BTreeMap<String, BTreeSet<String>> = records.iter().map(|item| (item.name.clone(), command_aliases(item))).collect();
    let mut family_union = Union::default();
    for item in &records {
        let anchor = format!("command:{}", item.name);
        family_union.find(&anchor);
        for alias in &aliases[&item.name] {
            family_union.join(&anchor, &format!("alias:{alias}"));
        }
    }
    let families: BTreeMap<String, String> = records.iter().map(|item| (item.name.clone(), hex_digest(&family_union.find(&format!("command:{}", item.name))))).collect();
    let blocked: BTreeSet<&String> = records.iter().filter(|item| aliases[&item.name].iter().any(|alias| reserved.contains(alias))).map(|item| &families[&item.name]).collect();
    let mut package_union = Union::default();
    for item in &records {
        let anchor = format!("family:{}", families[&item.name]);
        for owner in &item.owners {
            package_union.join(&anchor, &format!("package:{}", package_of(owner)));
        }
    }
    let documents: BTreeMap<String, String> = records.iter().map(|item| (item.name.clone(), format!("debian-package-component:{}", hex_digest(&package_union.find(&format!("family:{}", families[&item.name])))))).collect();
    let mut members: BTreeMap<&String, Vec<&String>> = BTreeMap::new();
    for item in &records {
        members.entry(&families[&item.name]).or_default().push(&item.name);
    }
    let mut retained: HashSet<&String> = HashSet::new();
    for names in members.values() {
        let mut ordered = names.clone();
        ordered.sort_by_cached_key(|name| hex_digest(&format!("{TECHNICAL_NAMESPACE}:family-cap:{}", command_identifier(name))));
        retained.extend(ordered.into_iter().take(COMMANDS_PER_ALIAS_FAMILY));
    }
    let mut rows = Vec::with_capacity(records.len());
    for item in &records {
        let family = &families[&item.name];
        let mut reasons = Vec::new();
        if blocked.contains(family) {
            reasons.push("reserved-physical-alias".to_owned());
        }
        if !retained.contains(&item.name) {
            reasons.push("fixed-hash-family-cap".to_owned());
        }
        let split = if reasons.is_empty() { assigned_split(TECHNICAL_NAMESPACE, &documents[&item.name]) } else { "quarantine" };
        let identifier = command_identifier(&item.name);
        rows.push(CorpusRow { identifier: identifier.clone(), original: item.name.clone(), group: Some(0), before: String::new(), after: String::new(), lemma: item.name.clone(), family: family.clone(), document: documents[&item.name].clone(), language: "en".to_owned(), source: "Debian-trixie-main-amd64".to_owned(), source_file: "Contents-amd64.gz".to_owned(), source_sentence: identifier, source_token: "1".to_owned(), spacing: " ".to_owned(), space_before: String::new(), literal_tail: String::new(), upos: "X".to_owned(), features: "_".to_owned(), misc: "_".to_owned(), alignment: "declared-command-name".to_owned(), layout_representable: true, split: split.to_owned(), quarantine_reasons: reasons });
    }
    let by_name: HashMap<&str, &Command> = records.iter().map(|item| (item.name.as_str(), item)).collect();
    let mut split_aliases: BTreeMap<&str, BTreeSet<&String>> = BTreeMap::new();
    let mut split_packages: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for row in &rows {
        if ACTIVE_SPLITS.contains(&row.split.as_str()) {
            split_aliases.entry(row.split.as_str()).or_default().extend(aliases[&row.original].iter());
            split_packages.entry(row.split.as_str()).or_default().extend(by_name[row.original.as_str()].owners.iter().map(|owner| package_of(owner)));
        }
    }
    let (mut alias_overlap, mut package_overlap) = (0, 0);
    for (index, left) in ACTIVE_SPLITS.iter().enumerate() {
        for right in &ACTIVE_SPLITS[index + 1..] {
            if let (Some(a), Some(b)) = (split_aliases.get(left), split_aliases.get(right)) {
                alias_overlap += a.intersection(b).count();
            }
            if let (Some(a), Some(b)) = (split_packages.get(left), split_packages.get(right)) {
                package_overlap += a.intersection(b).count();
            }
        }
    }
    let reserved_overlap: usize = split_aliases.values().map(|values| values.iter().filter(|alias| reserved.contains(**alias)).count()).sum();
    if alias_overlap != 0 || package_overlap != 0 || reserved_overlap != 0 {
        return Err("technical corpus partition leakage".to_owned());
    }
    let mut component_sizes: HashMap<&String, usize> = HashMap::new();
    for document in documents.values() {
        *component_sizes.entry(document).or_default() += 1;
    }
    let per_split = |measure: &dyn Fn(&CorpusRow) -> String| -> Map<String, Value> { ALL_SPLITS.iter().map(|split| { let set: BTreeSet<String> = rows.iter().filter(|row| row.split == *split).map(measure).collect(); ((*split).to_owned(), json!(set.len())) }).collect() };
    let rows_by_split: Map<String, Value> = ALL_SPLITS.iter().map(|split| ((*split).to_owned(), json!(rows.iter().filter(|row| row.split == *split).count()))).collect();
    let mut reason_counts: BTreeMap<&str, i64> = BTreeMap::new();
    for reason in rows.iter().flat_map(|row| row.quarantine_reasons.iter()) {
        *reason_counts.entry(reason).or_default() += 1;
    }
    let summary = json!({"mode": "joint-package-and-physical-family-component-hash", "split_probabilities": {"train": 0.7, "development": 0.1, "calibration": 0.1, "test": 0.1}, "commands": records.len(), "families": members.len(), "package_components": component_sizes.len(), "largest_component_commands": component_sizes.values().copied().max().unwrap_or(0), "reserved_families": blocked.len(), "family_cap": COMMANDS_PER_ALIAS_FAMILY, "rows_by_split": rows_by_split, "documents_by_split": per_split(&|row| row.document.clone()), "families_by_split": per_split(&|row| row.family.clone()), "quarantine_reasons": reason_counts, "alias_split_overlap": alias_overlap, "package_split_overlap": package_overlap, "reserved_alias_overlap": reserved_overlap});
    drop(by_name);
    Ok(Partitioned { rows, commands: records, aliases, summary })
}

/// `all_internal_edits`.
fn all_internal_edits(forms: &[String]) -> BTreeSet<String> {
    let mut result: BTreeSet<String> = forms.iter().cloned().collect();
    let tokens: BTreeSet<String> = forms.iter().flat_map(|form| words_of(form)).map(str::to_owned).collect();
    for word in &tokens {
        result.insert(word.clone());
        let letters: Vec<char> = word.chars().collect();
        if !(TYPO_SOURCE_MIN_CHARACTERS..=TYPO_SOURCE_MAX_CHARACTERS).contains(&letters.len()) {
            continue;
        }
        for index in 1..letters.len() - 1 {
            let mut deleted = letters.clone();
            deleted.remove(index);
            result.insert(deleted.into_iter().collect());
            let mut doubled = letters.clone();
            doubled.insert(index, letters[index]);
            result.insert(doubled.into_iter().collect());
        }
        for index in 1..letters.len() - TRANSPOSE_TAIL_MARGIN_CHARACTERS {
            // `word[:index] + word[index + 1] + word[index] + word[index + 2:]`.
            let mut swapped = letters.clone();
            swapped.swap(index, index + 1);
            result.insert(swapped.into_iter().collect());
        }
    }
    result
}

/// `verified_source`: the Contents index against its receipt and its `InRelease`.
fn verified_debian_source(directory: &Path) -> Result<(PathBuf, BTreeMap<String, String>, Value), String> {
    let receipt_path = directory.join("source-receipt.json");
    let receipt = read_json(&receipt_path)?;
    let (contents, release) = (directory.join("Contents-amd64.gz"), directory.join("InRelease"));
    let (data, release_data) = (&receipt["contents"], &receipt["release"]);
    if data["status"] != json!(200) || release_data["status"] != json!(200) || receipt["tls_certificate_verification"] != json!(true) || data["url"] != json!("https://deb.debian.org/debian/dists/trixie/main/Contents-amd64.gz") || release_data["url"] != json!("https://deb.debian.org/debian/dists/trixie/InRelease") {
        return Err("source receipt does not identify verified TLS Debian downloads".to_owned());
    }
    let size = std::fs::metadata(&contents).map_err(|error| error.to_string())?.len();
    if receipt["contents_sha256"].as_str() != Some(&checksum(&contents)?) || receipt["contents_bytes"].as_u64() != Some(size) || receipt["release_sha256"].as_str() != Some(&checksum(&release)?) {
        return Err("Debian source checksum mismatch".to_owned());
    }
    let expected = [receipt["contents_sha256"].as_str().unwrap_or("").to_owned(), receipt["contents_bytes"].to_string(), "main/Contents-amd64.gz".to_owned()];
    let text = std::fs::read_to_string(&release).map_err(|error| error.to_string())?;
    let (mut section, mut found) = (false, false);
    for line in ks_text::pystr::splitlines(&text, false) {
        if line == "SHA256:" {
            section = true;
        } else if section && !line.is_empty() && !line.starts_with(' ') {
            section = false;
        } else if section && ks_text::pystr::split(line) == expected.iter().map(String::as_str).collect::<Vec<_>>() {
            found = true;
        }
    }
    if !found {
        return Err("Contents checksum absent from InRelease SHA256 section".to_owned());
    }
    let mut provenance = BTreeMap::new();
    for path in [&receipt_path, &contents, &release] {
        provenance.insert(path.display().to_string(), checksum(path)?);
    }
    Ok((contents, provenance, json!({"source": "Debian trixie main amd64 Contents", "receipt": receipt, "verification": "HTTPS TLS and Contents SHA256 against downloaded InRelease; OpenPGP signature not verified by this generator"})))
}

/// `context_technical_corpus`: command families from a checksum-verified Debian Contents, the base holdout protected by its
/// hashed sidecar alone.
pub fn freeze_technical(root: &Path, source_directory: &Path, ud_corpus: &Path, extra_exposure: &[PathBuf], output: Option<&Path>, preview_report: &Path) -> Result<Value, String> {
    let (contents, mut provenance, source_metadata) = verified_debian_source(source_directory)?;
    let records = read_commands(&contents)?;
    let candidates: BTreeSet<String> = records.iter().flat_map(command_aliases).collect();
    let manifest_path = ud_corpus.join("manifest.json");
    let ud_manifest = read_json(&manifest_path)?;
    let sidecar_path = ud_corpus.join("physical-family-closure.json");
    if ud_manifest["reconciliation"]["closure_sha256"].as_str() != Some(&checksum(&sidecar_path)?) {
        return Err("UD physical sidecar checksum mismatch".to_owned());
    }
    let sidecar = read_json(&sidecar_path)?;
    let ud_aliases: HashSet<&str> = sidecar["alias_sha256_to_closure_sha256"].as_object().ok_or("invalid UD physical alias sidecar")?.keys().map(String::as_str).collect();
    provenance.insert(manifest_path.display().to_string(), checksum(&manifest_path)?);
    provenance.insert(sidecar_path.display().to_string(), checksum(&sidecar_path)?);
    let (exposed, sources, _) = crate::freeze::exposed_families(root, extra_exposure)?;
    provenance.extend(sources);
    let historical_forms = historical_code_forms(root)?;
    let historical = all_internal_edits(&historical_forms);
    for file in [HISTORICAL_CODE_FORMS, "crates/ks-lang/src/short_words.rs", FREEZER_SOURCE, "crates/ks-train/src/freeze.rs", "crates/ks-train/src/physical.rs", LAYOUT_TABLE] {
        provenance.insert(file.to_owned(), checksum(&root.join(file))?);
    }
    let mut reserved: BTreeSet<String> = candidates.iter().filter(|alias| ud_aliases.contains(alias.as_str())).cloned().collect();
    let candidate_ud_matches = reserved.len();
    let mut forms: BTreeSet<&str> = exposed.iter().map(String::as_str).collect();
    forms.extend(historical.iter().map(String::as_str));
    let mut historical_matches: BTreeSet<String> = BTreeSet::new();
    for form in &forms {
        historical_matches.extend(expanded_aliases(form).into_iter().filter(|alias| candidates.contains(alias)));
    }
    reserved.extend(historical_matches.iter().cloned());
    let reservations = json!({"ud_aliases_reserved": ud_aliases.len(), "candidate_ud_alias_matches": candidate_ud_matches, "historical_forms": forms.len(), "candidate_historical_alias_matches": historical_matches.len(), "ud_scope": "all sidecar aliases including quarantine; no UD split text opened", "historical_scope": "existing exposed_families plus declared historical context literals and short words; all their internal edits"});
    let result = partition_commands(records, &reserved)?;
    let metadata = json!({"source": source_metadata, "reservations": reservations});
    let mut report = json!({"namespace": TECHNICAL_NAMESPACE, "partitioning": result.summary, "reservations": reservations, "provenance": provenance, "test_scored": false, "old_ud_split_text_read": false});
    if let Some(output) = output {
        let manifest = write_technical(root, &result, output, &provenance, &metadata)?;
        report["manifest_sha256"] = json!(checksum(&output.join("manifest.json"))?);
        report["test_membership_sha256"] = manifest["test_membership_sha256"].clone();
    }
    write_json(preview_report, &report)?;
    Ok(json!({"partitioning": result.summary, "test_scored": false}))
}

fn write_technical(root: &Path, partitioned: &Partitioned, output: &Path, provenance: &BTreeMap<String, String>, metadata: &Value) -> Result<Value, String> {
    begin(output)?;
    let source = generator(root)?;
    std::fs::write(output.join("generator-source.rs"), &source).map_err(|error| error.to_string())?;
    let mut files = Map::new();
    for split in ALL_SPLITS {
        let selected: Vec<&CorpusRow> = partitioned.rows.iter().filter(|row| row.split == split).collect();
        files.insert(split.to_owned(), write_split(&output.join(format!("{split}.jsonl.gz")), &selected)?);
    }
    let held: Vec<&CorpusRow> = partitioned.rows.iter().filter(|row| row.split == "test").collect();
    let mut row_ids: Vec<String> = held.iter().map(|row| hex_digest(&row.identifier)).collect();
    row_ids.sort();
    let families: BTreeSet<&str> = held.iter().map(|row| row.family.as_str()).collect();
    let documents: BTreeSet<String> = held.iter().map(|row| hex_digest(&row.document)).collect();
    let membership_path = output.join("test-membership.json");
    write_json(&membership_path, &json!({"namespace": TECHNICAL_NAMESPACE, "scope": "declared command families and deduplicated binary-package components", "row_ids_sha256": row_ids, "family_ids_sha256": families, "document_ids_sha256": documents}))?;
    let command_path = output.join("command-provenance.json");
    let commands: Vec<Value> = partitioned.commands.iter().map(|item| json!({"name": item.name, "paths": item.paths, "owners": item.owners})).collect();
    write_json(&command_path, &json!({"commands": commands, "aliases_sha256_by_command": partitioned.aliases}))?;
    let mut by_family: BTreeMap<&str, Vec<&CorpusRow>> = BTreeMap::new();
    for row in &partitioned.rows {
        by_family.entry(&row.family).or_default().push(row);
    }
    let mut family_records = Map::new();
    for (family, rows) in by_family {
        let active: Vec<&&CorpusRow> = rows.iter().filter(|row| ACTIVE_SPLITS.contains(&row.split.as_str())).collect();
        if active.is_empty() {
            continue;
        }
        let splits: BTreeSet<&str> = active.iter().map(|row| row.split.as_str()).collect();
        if splits.len() != 1 {
            return Err("family spans multiple active splits".to_owned());
        }
        let aliases: BTreeSet<&String> = rows.iter().flat_map(|row| partitioned.aliases[&row.original].iter()).collect();
        let document_ids: BTreeSet<String> = active.iter().map(|row| hex_digest(&row.document)).collect();
        family_records.insert(family.to_owned(), json!({"split": active[0].split, "aliases_sha256": aliases, "document_ids_sha256": document_ids}));
    }
    let family_path = output.join("family-aliases.json");
    write_json(&family_path, &json!({"schema_version": 1, "families": family_records}))?;
    let name = format!("[a-z]{{{COMMAND_NAME_MIN_CHARACTERS},{COMMAND_NAME_MAX_CHARACTERS}}}");
    let manifest = json!({
        "schema_version": 1, "namespace": TECHNICAL_NAMESPACE, "label": "keep", "splits": files,
        "scope": "declared command-family/package holdout; not globally unseen lexicon or verified human intent",
        "normalization": format!("exact case-sensitive (usr/)?s?bin/{name}; command basename deduplicated; all paths and owners retained"),
        "context": "empty before/after; downstream action_rows adds its existing disclosed mixed contexts and declared layout interventions",
        "token_language": "ASCII command alphabet uses group 0; not an assertion of natural English prose",
        "partitioning": partitioned.summary, "source_metadata": metadata, "provenance": provenance,
        "generator_sha256": hex(&Sha256::digest(&source)), "generator_source": "generator-source.rs",
        "command_provenance_sha256": checksum(&command_path)?, "test_membership_sha256": checksum(&membership_path)?, "family_aliases_sha256": checksum(&family_path)?,
        "typo_policy": "all exported typo_variants(original,identifier) joined using expanded_aliases before package grouping and splitting",
        "test_policy": "no scoring or test-text output; exact split loader; test requires explicit allow_test; quarantine never used for training",
        "compression": "gzip mtime=0; content SHA256 independent of deflate implementation",
        "limitations": ["Contents lists shipped paths, not executable permissions or human command intent", "binary-package ownership is grouped; shared source packages and renamed forks are not identified", format!("only lowercase ASCII command names of {COMMAND_NAME_MIN_CHARACTERS} through {COMMAND_NAME_MAX_CHARACTERS} characters are in this supplement"), "existing dictionaries and pretrained model vocabularies are not globally inventoried", "an intended command typo is labeled by the existing action_rows spelling policy, not human-reviewed commands"],
    });
    write_json(&output.join("manifest.json"), &manifest)?;
    Ok(manifest)
}

/// `Streamed`.
#[derive(Clone, PartialEq, Eq)]
struct Streamed {
    sha256: String,
    content_sha256: String,
    rows: u64,
    raw_bytes: u64,
}

/// `inspect_gzip`: complete JSONL lines counted as bytes, never deserialized.
fn inspect_gzip(path: &Path, mut raw: Option<&mut Sha256>) -> Result<Streamed, String> {
    let file = std::fs::File::open(path).map_err(|_| "missing or empty gzip source".to_owned())?;
    let mut reader = std::io::BufReader::new(MultiGzDecoder::new(file));
    let mut hasher = Sha256::new();
    let (mut rows, mut size) = (0, 0);
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line).map_err(|_| "invalid gzip source".to_owned())? == 0 {
            break;
        }
        if line.last() != Some(&b'\n') || line.iter().all(u8::is_ascii_whitespace) {
            return Err("source JSONL must contain complete nonempty lines".to_owned());
        }
        hasher.update(&line);
        if let Some(raw) = raw.as_mut() {
            raw.update(&line);
        }
        rows += 1;
        size += line.len().to::<u64>();
    }
    Ok(Streamed { sha256: checksum(path)?, content_sha256: hex(&hasher.finalize()), rows, raw_bytes: size })
}

/// `Origin`.
struct Origin {
    directory: PathBuf,
    manifest: Value,
    namespace: String,
    membership: BTreeMap<&'static str, Vec<String>>,
    files: BTreeMap<&'static str, Streamed>,
    pins: BTreeMap<PathBuf, String>,
}

fn hash_list(value: &Value) -> Result<Vec<String>, String> {
    let items = value.as_array().ok_or("expected metadata hash list")?;
    let result: Vec<String> = items.iter().map(|item| item.as_str().map(str::to_owned).ok_or_else(|| "invalid metadata SHA256".to_owned())).collect::<Result<_, _>>()?;
    let unique: BTreeSet<&String> = result.iter().collect();
    if unique.len() != result.len() {
        return Err("duplicate hashes".to_owned());
    }
    Ok(result)
}

fn verify_origin(directory: &Path) -> Result<Origin, String> {
    let manifest_path = directory.join("manifest.json");
    let manifest = read_json(&manifest_path)?;
    let namespace = manifest["namespace"].as_str().filter(|text| !text.is_empty()).ok_or("invalid corpus manifest identity")?.to_owned();
    if manifest["schema_version"] != json!(1) || manifest["label"] != json!("keep") {
        return Err("invalid corpus manifest identity".to_owned());
    }
    let mut pins = BTreeMap::from([(manifest_path.clone(), checksum(&manifest_path)?)]);
    let mut files = BTreeMap::new();
    for split in ALL_SPLITS {
        let record = &manifest["splits"][split];
        if record["path"].as_str() != Some(&format!("{split}.jsonl.gz")) {
            return Err("unexpected source split filename".to_owned());
        }
        let path = directory.join(format!("{split}.jsonl.gz"));
        let observed = inspect_gzip(&path, None)?;
        if record["sha256"].as_str() != Some(&observed.sha256) {
            return Err("source compressed checksum mismatch".to_owned());
        }
        if record["content_sha256"].as_str() != Some(&observed.content_sha256) {
            return Err("source raw content checksum mismatch".to_owned());
        }
        if record["rows"].as_u64() != Some(observed.rows) {
            return Err("source streamed row count mismatch".to_owned());
        }
        pins.insert(path, observed.sha256.clone());
        files.insert(split, observed);
    }
    if files.values().map(|item| item.rows).sum::<u64>() == 0 {
        return Err("empty source corpus".to_owned());
    }
    let generator_name = manifest["generator_source"].as_str().unwrap_or("generator-source.py");
    let generator = directory.join(generator_name);
    if manifest["generator_sha256"].as_str() != Some(&checksum(&generator)?) {
        return Err("origin generator checksum mismatch".to_owned());
    }
    pins.insert(generator.clone(), checksum(&generator)?);
    if let Some(reconciliation) = manifest["reconciliation"]["source_sha256"].as_str() {
        let reconciler = [directory.join("reconciliation-source.rs"), directory.join("reconciliation-source.py")].into_iter().find(|path| path.is_file()).ok_or("origin reconciliation generator is absent")?;
        if reconciliation != checksum(&reconciler)? {
            return Err("origin reconciliation generator checksum mismatch".to_owned());
        }
        pins.insert(reconciler.clone(), checksum(&reconciler)?);
    }
    let membership_path = directory.join("test-membership.json");
    if manifest["test_membership_sha256"].as_str() != Some(&checksum(&membership_path)?) {
        return Err("test membership checksum mismatch".to_owned());
    }
    let membership_value = read_json(&membership_path)?;
    if membership_value["namespace"].as_str() != Some(&namespace) {
        return Err("test membership namespace mismatch".to_owned());
    }
    let mut membership = BTreeMap::new();
    for field in MEMBERSHIP_FIELDS {
        membership.insert(field, hash_list(&membership_value[field])?);
    }
    if (membership["row_ids_sha256"].len()).to::<u64>() != files["test"].rows {
        return Err("test membership row count mismatch".to_owned());
    }
    if files["test"].rows != 0 && membership.values().any(Vec::is_empty) {
        return Err("nonempty test requires family and document membership".to_owned());
    }
    pins.insert(membership_path.clone(), checksum(&membership_path)?);
    Ok(Origin { directory: directory.to_path_buf(), manifest, namespace, membership, files, pins })
}

fn verify_aliases(base: &Origin, extension: &Origin) -> Result<(Value, BTreeMap<PathBuf, String>), String> {
    let base_path = base.directory.join("physical-family-closure.json");
    if base.manifest["reconciliation"]["closure_sha256"].as_str() != Some(&checksum(&base_path)?) {
        return Err("base physical sidecar checksum mismatch".to_owned());
    }
    let closure = read_json(&base_path)?;
    let base_aliases = closure["alias_sha256_to_closure_sha256"].as_object().filter(|table| !table.is_empty()).ok_or("empty base physical alias sidecar")?;
    let extension_path = extension.directory.join("family-aliases.json");
    if extension.manifest["family_aliases_sha256"].as_str() != Some(&checksum(&extension_path)?) {
        return Err("technical family sidecar checksum mismatch".to_owned());
    }
    let sidecar = read_json(&extension_path)?;
    if sidecar["schema_version"] != json!(1) {
        return Err("unsupported technical family sidecar".to_owned());
    }
    let families = sidecar["families"].as_object().ok_or("expected corpus metadata object")?;
    let mut aliases_seen: HashMap<String, (&String, &str)> = HashMap::new();
    let mut documents_seen: HashMap<String, &str> = HashMap::new();
    let (mut test_families, mut test_documents): (BTreeSet<&String>, BTreeSet<String>) = Default::default();
    for (family, record) in families {
        let split = record["split"].as_str().filter(|split| ACTIVE_SPLITS.contains(split)).ok_or("technical family has invalid retained split")?;
        let aliases = hash_list(&record["aliases_sha256"])?;
        let documents = hash_list(&record["document_ids_sha256"])?;
        if aliases.is_empty() || documents.is_empty() {
            return Err("technical family requires aliases and documents".to_owned());
        }
        for alias in aliases {
            if base_aliases.contains_key(&alias) {
                return Err("technical family overlaps base physical alias".to_owned());
            }
            if aliases_seen.get(&alias).is_some_and(|seen| *seen != (family, split)) {
                return Err("technical alias belongs to multiple families or splits".to_owned());
            }
            aliases_seen.insert(alias, (family, split));
        }
        for document in documents {
            if documents_seen.get(&document).is_some_and(|seen| *seen != split) {
                return Err("technical document spans multiple splits".to_owned());
            }
            if split == "test" {
                test_documents.insert(document.clone());
            }
            documents_seen.insert(document, split);
        }
        if split == "test" {
            test_families.insert(family);
        }
    }
    let declared_families: BTreeSet<&String> = extension.membership["family_ids_sha256"].iter().collect();
    if test_families != declared_families {
        return Err("technical sidecar test family membership mismatch".to_owned());
    }
    let declared_documents: BTreeSet<String> = extension.membership["document_ids_sha256"].iter().cloned().collect();
    if test_documents != declared_documents {
        return Err("technical sidecar test document membership mismatch".to_owned());
    }
    for field in MEMBERSHIP_FIELDS {
        let left: BTreeSet<&String> = base.membership[field].iter().collect();
        if extension.membership[field].iter().any(|item| left.contains(item)) {
            return Err(format!("origin test membership overlap: {field}"));
        }
    }
    let audit = json!({"base_aliases": base_aliases.len(), "technical_retained_families": families.len(), "technical_aliases": aliases_seen.len(), "base_technical_alias_overlap": 0, "technical_cross_split_alias_overlap": 0, "technical_cross_split_document_overlap": 0, "test_membership_origin_overlap": 0, "scope": "source metadata hashes and all physical aliases; no model scoring or test row parsing"});
    Ok((audit, BTreeMap::from([(base_path.clone(), checksum(&base_path)?), (extension_path.clone(), checksum(&extension_path)?)])))
}

/// `merge_context_action_corpora`: byte-exact gzip members of both origins, test JSON never parsed.
pub fn merge(root: &Path, base_directory: &Path, extension_directory: &Path, output: &Path) -> Result<Value, String> {
    if output.exists() {
        return Err("refusing to overwrite combined corpus".to_owned());
    }
    let (base, extension) = (verify_origin(base_directory)?, verify_origin(extension_directory)?);
    let (audit, sidecars) = verify_aliases(&base, &extension)?;
    let source = generator(root)?;
    let mut pins: BTreeMap<PathBuf, String> = base.pins.clone();
    pins.extend(extension.pins.clone());
    pins.extend(sidecars);
    pins.insert(root.join(FREEZER_SOURCE), hex(&Sha256::digest(&source)));
    let mut membership = Map::new();
    membership.insert("namespace".to_owned(), json!(base.namespace));
    membership.insert("scope".to_owned(), json!("unchanged original UD membership plus separately frozen command-family/package membership"));
    for field in MEMBERSHIP_FIELDS {
        let mut joined = base.membership[field].clone();
        joined.extend(extension.membership[field].iter().cloned());
        joined.sort();
        membership.insert(field.to_owned(), json!(joined));
    }
    std::fs::create_dir_all(output.join("origins")).map_err(|error| error.to_string())?;
    let mut origins = Vec::new();
    for (name, origin) in [("base", &base), ("technical", &extension)] {
        let saved = output.join(format!("origins/{name}-manifest.json"));
        std::fs::copy(origin.directory.join("manifest.json"), &saved).map_err(|error| error.to_string())?;
        origins.push(json!({"name": name, "namespace": origin.namespace, "manifest": format!("origins/{name}-manifest.json"), "manifest_sha256": checksum(&saved)?, "generator_sha256": origin.manifest["generator_sha256"], "test_membership_sha256": origin.manifest["test_membership_sha256"], "splits": origin.manifest["splits"]}));
    }
    std::fs::write(output.join("generator-source.rs"), &source).map_err(|error| error.to_string())?;
    let mut files = Map::new();
    for split in ALL_SPLITS {
        let name = format!("{split}.jsonl.gz");
        let destination = output.join(&name);
        let mut raw = Sha256::new();
        let (mut rows, mut bytes) = (0, 0);
        let mut joined = Vec::new();
        for origin in [&base, &extension] {
            let path = origin.directory.join(&name);
            let current = inspect_gzip(&path, Some(&mut raw))?;
            if current != origin.files[split] {
                return Err("source changed during corpus composition".to_owned());
            }
            let mut member = Vec::new();
            std::fs::File::open(&path).and_then(|mut file| file.read_to_end(&mut member)).map_err(|error| error.to_string())?;
            if hex(&Sha256::digest(&member)) != current.sha256 {
                return Err("copied source bytes differ from verified source".to_owned());
            }
            joined.extend_from_slice(&member);
            rows += current.rows;
            bytes += current.raw_bytes;
        }
        std::fs::write(&destination, &joined).map_err(|error| error.to_string())?;
        let combined = inspect_gzip(&destination, None)?;
        let digest = hex(&raw.finalize());
        if combined.content_sha256 != digest || combined.rows != rows || combined.raw_bytes != bytes {
            return Err("combined gzip stream differs from source streams".to_owned());
        }
        files.insert(split.to_owned(), json!({"path": name, "sha256": checksum(&destination)?, "content_sha256": digest, "rows": rows, "raw_bytes": bytes, "gzip_members_from": ["base", "technical"]}));
    }
    if membership["row_ids_sha256"].as_array().map_or(0, Vec::len).to::<u64>() != files["test"]["rows"].as_u64().unwrap_or(0) {
        return Err("combined test membership row count mismatch".to_owned());
    }
    let membership_path = output.join("test-membership.json");
    write_json(&membership_path, &Value::Object(membership))?;
    for (path, expected) in &pins {
        if &checksum(path)? != expected {
            return Err("source or generator changed while merging corpora".to_owned());
        }
    }
    let rows_by_split: Map<String, Value> = ALL_SPLITS.iter().map(|split| ((*split).to_owned(), files[*split]["rows"].clone())).collect();
    let provenance: Map<String, Value> = pins.iter().map(|(path, value)| (path.display().to_string(), json!(value))).collect();
    let manifest = json!({
        "schema_version": 1, "namespace": base.namespace, "label": "keep", "splits": files,
        "scope": "union of unchanged frozen UD and command-family/package partitions; not new independent evidence",
        "origins": origins, "alias_audit": audit,
        "partitioning": {"mode": "unchanged-source-split-concatenation", "replacements": 0, "rows_by_split": rows_by_split},
        "generator_source": "generator-source.rs", "generator_sha256": hex(&Sha256::digest(&source)),
        "test_membership_sha256": checksum(&membership_path)?,
        "provenance": provenance,
        "compression": "byte-exact base gzip members followed by byte-exact technical gzip members; no recompression",
        "test_policy": "test JSON never parsed or printed during merge; only opaque byte-stream checks and hashed membership read; future scoring requires a new final recipe/seal and joint gate",
        "limitations": ["source counts and byte hashes are verified, not the semantic correctness of labels or row JSON", "source alias sidecars attest declared physical families, not globally unseen lexicons or human intent", "original source-specific reconciliation and evaluation statistics remain only in origin manifests", "union membership adds an unscored technical holdout and does not erase any prior UD exposure", "cryptographic source hashes provide consistency, not signed provenance or a new quality evaluation"],
    });
    write_json(&output.join("manifest.json"), &manifest)?;
    Ok(manifest)
}

/// `prefix_exposure_inventory`: the physical aliases of every word the prefix model's fitting inputs expose. Its test split
/// is not opened: its words trained nothing.
pub fn prefix_inventory(root: &Path, corpus: &Path, output: &Path) -> Result<Value, String> {
    if output.exists() {
        return Err("refusing to overwrite an exposure inventory".to_owned());
    }
    let (mut by_split, mut by_scope): (BTreeMap<&str, BTreeSet<String>>, BTreeMap<&str, BTreeSet<String>>) = Default::default();
    let mut provenance: BTreeMap<String, String> = BTreeMap::new();
    let name_of = |path: &Path| path.strip_prefix(root).unwrap_or(path).display().to_string();
    for split in PREFIX_SPLITS {
        let path = corpus.join(format!("{split}.jsonl.gz"));
        provenance.insert(name_of(&path), checksum(&path)?);
        let file = std::fs::File::open(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        for line in std::io::BufReader::new(MultiGzDecoder::new(file)).lines() {
            let row: Value = serde_json::from_str(&line.map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
            for scope in ["text", "family"] {
                let value = row[scope].as_str().filter(|text| !text.is_empty()).ok_or_else(|| format!("prefix row without a {scope}"))?;
                let aliases = expanded_aliases(value);
                by_split.entry(split).or_default().extend(aliases.iter().cloned());
                by_scope.entry(scope).or_default().extend(aliases);
            }
        }
    }
    let manifest = corpus.join("corpus.json");
    provenance.insert(name_of(&manifest), checksum(&manifest)?);
    let aliases: BTreeSet<&String> = by_split.values().flatten().collect();
    let mut counts = Map::new();
    counts.insert("aliases".to_owned(), json!(aliases.len()));
    for (split, values) in &by_split {
        counts.insert((*split).to_owned(), json!(values.len()));
    }
    let result = json!({"schema_version": 1, "kind": "actual-prefix-model-input-exposure", "test_json_opened": false, "model_scoring": false, "scope": "expanded physical aliases of every typed word and whole-word family in the prefix-v1 train, development and calibration splits, the inputs of prefix-v2 training; the prefix test split was not opened", "scopes": {"text": "typed word form", "family": "whole-word physical family"}, "aliases_sha256": aliases, "aliases_by_split": by_split, "aliases_by_scope": by_scope, "counts": counts, "provenance": provenance});
    write_json(output, &result)?;
    Ok(json!({"output": output.display().to_string(), "counts": result["counts"]}))
}
