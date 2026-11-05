//! The public aggregate evidence of an accepted pair and its verification (`verify_context_action_model`): every count
//! recomputed from the private cases of the sealed test before it is published, every gate recomputed from the counts.
use ks_cast::To as _;
use crate::corpus::{checksum, read_json};
use crate::evaluate::{EvaluationRequest, PROFILES, evaluation_identity, validate_candidate_seal, validate_prefix_seal};
use crate::fit::readable;
use crate::physical::hex_digest;
use ks_context::VersionedPrefixModel;
use ks_context::context_model::ContextModel;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;

const OUTSIDE_LONE_WORD: &str = "outside_lone_word";
const DEVCHAT: &str = "devchat_test";
const REJECTED_ARTIFACT: &str = "55f0d735de8751f3f414d569b6e1eb2bbb294303d8fe01f2fde66c3fadecfeb1";
const BASELINE: &str = "model/context_v3/baseline-context-v1.json";
const PREFIX_BASELINE: &str = "model/prefix_v2/baseline-prefix-v1.json";
const RECIPE: &str = "model/context_v3/recipe.json";
const COUNTS: [&str; 11] = ["rows", "initially_correct", "initially_wrong", "preserved_correct", "exactly_restored", "correct_text_corruptions", "length_mismatches", "injections", "execution_errors", "correction_layout_mismatches", "final_layout_mismatches"];
const CALIBRATION: [&str; 5] = ["rows", "convert_rows", "converted_correctly", "false_conversions", "conversion_recall"];
const SEQUENCE_DOCUMENT_CAP_PER_GROUP: i64 = 512;
const SEQUENCE_MAXIMUM_SELECTED_ROWS: i64 = 3 * SEQUENCE_DOCUMENT_CAP_PER_GROUP;
const MINIMUM_SEQUENCE_DOCUMENTS_PER_GROUP: i64 = 32;
const DEVCHAT_MIN_NET_MARGIN: i64 = 1;
const RECEIPT_FIELDS: [&str; 29] = [
    "schema_version", "feature_version", "model_version", "conversion_threshold", "artifact_sha256", "weights_sha256", "baseline_path", "baseline_sha256", "recipe_path", "recipe_sha256",
    "candidate_seal_sha256", "full_test_report_sha256", "corpus_manifest_sha256", "provenance", "calibration", "test", "gate_policy", "protocol", "scope", "quality_gates_passed",
    "prefix_model_version", "prefix_feature_version", "prefix_conversion_threshold", "prefix_artifact_sha256", "prefix_weights_sha256", "prefix_candidate_seal_sha256", "prefix_baseline_path",
    "prefix_baseline_sha256", "devchat_test",
];
const DEVCHAT_SIGNIFICANCE: f64 = 0.05;
const MODES: [&str; 4] = ["correct", "forget_ru", "forget_en", "start_wrong"];
const RESTORING_MODES: [&str; 3] = ["forget_ru", "forget_en", "start_wrong"];

fn fields_exact(value: &Value, label: &str, fields: &[&str]) -> Result<(), String> {
    let table = value.as_object().ok_or_else(|| format!("invalid object: {label}"))?;
    if table.len() != fields.len() || fields.iter().any(|name| !table.contains_key(*name)) {
        return Err(format!("missing or extra fields: {label}"));
    }
    Ok(())
}

fn count(value: &Value, label: &str) -> Result<i64, String> {
    value.as_i64().filter(|number| *number >= 0).ok_or_else(|| format!("invalid count: {label}"))
}

fn sha(value: &Value) -> Result<&str, String> {
    value.as_str().filter(|text| text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))).ok_or_else(|| "invalid SHA256".to_owned())
}

fn calibration_counts(row: &Value, policy: &Value) -> Result<(), String> {
    let number = |name: &str| count(&row[name], name);
    let (rows, possible, right, wrong) = (number("rows")?, number("convert_rows")?, number("converted_correctly")?, number("false_conversions")?);
    let recall = row["conversion_recall"].as_f64().filter(|value| value.is_finite()).ok_or("calibration gate failed")?;
    if possible <= 0
        || possible > rows
        || right > possible
        || recall != right.to::<f64>() / possible.to::<f64>()
        || wrong > right
        || right - wrong < policy["minimum_calibration_net_benefit"].as_i64().unwrap_or(1)
        || recall < policy["minimum_calibration_conversion_recall"].as_f64().unwrap_or(0.0)
    {
        return Err("calibration gate failed".to_owned());
    }
    Ok(())
}

fn validate_calibration_block(block: &Value, label: &str, policy: &Value) -> Result<(), String> {
    calibration_counts(block, policy)?;
    let profiles = &block["by_profile"];
    fields_exact(profiles, label, &PROFILES)?;
    for name in PROFILES {
        calibration_counts(&profiles[name], policy)?;
    }
    for name in ["rows", "convert_rows", "converted_correctly", "false_conversions"] {
        let sum: i64 = PROFILES.iter().map(|profile| profiles[*profile][name].as_i64().unwrap_or(0)).sum();
        if block[name].as_i64() != Some(sum) {
            return Err(format!("{label} aggregate mismatch"));
        }
    }
    Ok(())
}

fn public_calibration_block(block: &Value) -> Value {
    let mut public = Map::new();
    for name in CALIBRATION {
        public.insert(name.to_owned(), block[name].clone());
    }
    let mut profiles = Map::new();
    for profile in PROFILES {
        let mut row = Map::new();
        for name in CALIBRATION {
            row.insert(name.to_owned(), block["by_profile"][profile][name].clone());
        }
        profiles.insert(profile.to_owned(), Value::Object(row));
    }
    public.insert("by_profile".to_owned(), Value::Object(profiles));
    Value::Object(public)
}

fn sequence_counts(value: &Value, documents: &BTreeMap<String, i64>) -> Result<BTreeMap<&'static str, i64>, String> {
    fields_exact(value, "sequence counts", &COUNTS)?;
    let mut counts = BTreeMap::new();
    for name in COUNTS {
        counts.insert(name, count(&value[name], name)?);
    }
    let wrong = documents["0"] + documents["1"];
    let correct = wrong + documents["None"];
    if counts["rows"] != wrong + correct
        || counts["initially_correct"] != correct
        || counts["initially_wrong"] != wrong
        || counts["exactly_restored"] > wrong
        || counts["exactly_restored"] > counts["injections"]
        || counts["preserved_correct"] + counts["correct_text_corruptions"] != correct
        || ["length_mismatches", "execution_errors", "final_layout_mismatches"].iter().any(|key| counts[key] > counts["rows"])
        || counts["correction_layout_mismatches"] > counts["injections"]
    {
        return Err("inconsistent sequence counts".to_owned());
    }
    Ok(counts)
}

fn validate_gated_block(block: &Value, documents: &BTreeMap<String, i64>) -> Result<(), String> {
    let candidate = sequence_counts(&block["counts"]["candidate"], documents)?;
    let baseline = sequence_counts(&block["counts"]["baseline"], documents)?;
    let gates = json!({
        "correct_text_preserved": candidate["correct_text_corruptions"] <= baseline["correct_text_corruptions"],
        "length_preserved": candidate["length_mismatches"] == 0,
        "net_restorations_at_least_baseline": candidate["exactly_restored"] - candidate["correct_text_corruptions"] >= baseline["exactly_restored"] - baseline["correct_text_corruptions"],
        "enough_documents_us": documents["0"] >= MINIMUM_SEQUENCE_DOCUMENTS_PER_GROUP,
        "enough_documents_ru": documents["1"] >= MINIMUM_SEQUENCE_DOCUMENTS_PER_GROUP,
        "execution_succeeded": candidate["execution_errors"] == 0 && baseline["execution_errors"] == 0,
        "correction_layouts_match": candidate["correction_layout_mismatches"] == 0,
    });
    if !gates.as_object().is_some_and(|table| table.values().all(|value| value == &json!(true))) || block["gates"] != gates {
        return Err("sequence gates failed or contradicted by counts".to_owned());
    }
    Ok(())
}

fn validate_test(test: &Value) -> Result<(), String> {
    fields_exact(test, "test", &["documents", "selected_rows", "trimmed_rows", "unsupported_rows", "profiles"])?;
    let mut documents = BTreeMap::new();
    for name in ["0", "1", "None"] {
        documents.insert(name.to_owned(), count(&test["documents"][name], "documents")?);
    }
    let selected = count(&test["selected_rows"], "selected_rows")?;
    if documents.values().any(|value| *value > SEQUENCE_DOCUMENT_CAP_PER_GROUP)
        || selected > SEQUENCE_MAXIMUM_SELECTED_ROWS
        || selected != documents.values().sum::<i64>() + count(&test["unsupported_rows"], "unsupported_rows")?
        || count(&test["trimmed_rows"], "trimmed_rows")? > selected
    {
        return Err("inconsistent sequence selection".to_owned());
    }
    fields_exact(&test["profiles"], "test profiles", &PROFILES)?;
    for name in PROFILES {
        let profile = &test["profiles"][name];
        validate_gated_block(profile, &documents)?;
        validate_gated_block(&profile["early_off"], &documents)?;
    }
    Ok(())
}

fn loss_chance(losses: i64, gains: i64) -> f64 {
    let total = losses + gains;
    let mut combination = 1.0_f64;
    let mut sum = 0.0;
    for number in 0..=total {
        if number >= losses {
            sum += combination;
        }
        combination = combination * (total - number).to::<f64>() / (number + 1).to::<f64>();
    }
    sum / 2_f64.powi(total.to::<i32>())
}

fn validate_devchat(block: &Value, receipt: &Value) -> Result<(), String> {
    for name in ["messages_sha256", "result_sha256", "access"] {
        sha(&block[name])?;
    }
    let pairs = &block["pairs"];
    if pairs["candidate"]["sha256"] != receipt["artifact_sha256"] || pairs["candidate"]["model_version"] != receipt["model_version"] || pairs["baseline"]["sha256"] != receipt["baseline_sha256"] {
        return Err("the developer-chat test compared other pairs".to_owned());
    }
    let results = &block["results"];
    let ok = |pair: &str, mode: &str| results[pair][mode]["ok"].as_i64().unwrap_or(0);
    let n = |pair: &str, mode: &str| results[pair][mode]["n"].as_i64().unwrap_or(0);
    let corruptions = |pair: &str| n(pair, "correct") - ok(pair, "correct");
    let restored = |pair: &str| RESTORING_MODES.iter().map(|mode| ok(pair, mode)).sum::<i64>();
    let errors: i64 = ["candidate", "released", "baseline"].iter().flat_map(|pair| MODES.iter().map(move |mode| results[*pair][*mode]["errors"].as_i64().unwrap_or(0))).sum();
    let f1 = restored("candidate") - corruptions("candidate") >= restored("baseline") - corruptions("baseline") + DEVCHAT_MIN_NET_MARGIN && corruptions("candidate") <= corruptions("baseline");
    let chance = |mode: &str| loss_chance(block["discordance"][mode]["losses"].as_i64().unwrap_or(0), block["discordance"][mode]["gains"].as_i64().unwrap_or(0));
    let f3 = RESTORING_MODES.iter().all(|mode| chance(mode) >= DEVCHAT_SIGNIFICANCE) && restored("candidate") >= restored("released") && corruptions("candidate") <= corruptions("released");
    let held = json!({"every_message_typed": errors == 0, "f1_better_than_baseline": f1, "f3_not_worse_than_released": f3});
    if errors != 0 || !f1 || !f3 || block["floors"] != held {
        return Err("developer-chat floors failed or contradicted by counts".to_owned());
    }
    Ok(())
}

/// What a receipt's provenance must name at least: what the program and the fit are made of.
fn required_provenance(root: &Path) -> Result<BTreeSet<String>, String> {
    let current = crate::fit::provenance(root, &root.join(RECIPE))?;
    Ok(current.into_keys().collect())
}

/// `validate_receipt`.
pub fn validate_receipt(receipt: &Value, artifact: &Path, root: &Path, prefix_artifact: &Path, models_dir: &Path) -> Result<(), String> {
    validate_evidence(receipt, artifact, root, prefix_artifact, models_dir)?;
    let recipe = read_json(&root.join(RECIPE))?;
    let hashes = receipt["provenance"].as_object().ok_or("invalid object: provenance")?;
    for name in required_provenance(root)? {
        if !hashes.contains_key(&name) {
            return Err(format!("missing required provenance: {name}"));
        }
    }
    for (name, expected) in hashes {
        if name.contains("..") || name.starts_with('/') || name.starts_with('.') {
            return Err("nonpublic provenance path".to_owned());
        }
        if Some(checksum(&root.join(name))?.as_str()) != expected.as_str() {
            return Err(format!("context provenance mismatch: {name}"));
        }
    }
    if hashes.get(PREFIX_BASELINE) != Some(&receipt["prefix_baseline_sha256"]) && receipt["prefix_baseline_sha256"].as_str() != Some(checksum(&root.join(PREFIX_BASELINE))?.as_str()) {
        return Err("prefix baseline mismatch".to_owned());
    }
    if hashes.get(RECIPE) != Some(&receipt["recipe_sha256"]) || recipe["schema_version"] != json!(1) || recipe["feature_version"] != json!(3) || recipe["profiles"] != json!(PROFILES) {
        return Err("baseline, recipe or gate policy mismatch".to_owned());
    }
    Ok(())
}

/// Everything a receipt claims except the bytes of the sources it names: the identity of the pair, every gate recomputed.
pub fn validate_evidence(receipt: &Value, artifact: &Path, root: &Path, prefix_artifact: &Path, models_dir: &Path) -> Result<(), String> {
    fields_exact(receipt, "receipt", &RECEIPT_FIELDS)?;
    let protocols = read_json(&root.join("model/context_v3/protocols.json"))?;
    let recipe = read_json(&root.join(RECIPE))?;
    let policy = &recipe["gate_policy"];
    if receipt["schema_version"] != json!(1)
        || receipt["feature_version"] != json!(3)
        || receipt["quality_gates_passed"] != json!(true)
        || receipt["scope"] != protocols["scope"]
        || receipt["protocol"] != protocols["receipt"]
        || receipt["gate_policy"] != *policy
        || receipt["baseline_path"] != json!(BASELINE)
        || receipt["recipe_path"] != json!(RECIPE)
        || receipt["prefix_baseline_path"] != json!(PREFIX_BASELINE)
    {
        return Err("unsupported receipt identity or policy".to_owned());
    }
    let actual = checksum(artifact)?;
    if actual == REJECTED_ARTIFACT || receipt["artifact_sha256"].as_str() != Some(actual.as_str()) {
        return Err("unaccepted or changed context artifact".to_owned());
    }
    let model = ContextModel::load_artifact(artifact, models_dir).map_err(|error| error.name.to_string())?;
    let payload = read_json(artifact)?;
    if model.version != format!("context-v3-{}", &sha(&receipt["weights_sha256"])?[..12]) || receipt["model_version"].as_str() != Some(model.version.as_str()) || receipt["conversion_threshold"].as_f64() != Some(model.conversion_threshold) || payload["weights_sha256"] != receipt["weights_sha256"] {
        return Err("context weights, version or threshold mismatch".to_owned());
    }
    let prefix_payload = read_json(prefix_artifact)?;
    let prefix = VersionedPrefixModel::load(prefix_artifact).map_err(|error| error.name.to_string())?;
    if receipt["prefix_artifact_sha256"].as_str() != Some(checksum(prefix_artifact)?.as_str())
        || receipt["prefix_feature_version"].as_i64() != Some(prefix.feature_version)
        || receipt["prefix_conversion_threshold"].as_f64() != Some(prefix.model.conversion_threshold)
        || prefix.version() != format!("prefix-v{}-{}", prefix.feature_version, &sha(&receipt["prefix_weights_sha256"])?[..12])
        || receipt["prefix_model_version"].as_str() != Some(prefix.version())
        || prefix_payload["weights_sha256"] != receipt["prefix_weights_sha256"]
    {
        return Err("prefix weights, version or threshold mismatch".to_owned());
    }
    let calibration = &receipt["calibration"];
    validate_calibration_block(calibration, "calibration", policy)?;
    validate_calibration_block(&calibration[OUTSIDE_LONE_WORD], "calibration outside the lone-word class", policy)?;
    validate_test(&receipt["test"])?;
    validate_devchat(&receipt[DEVCHAT], receipt)?;
    Ok(())
}

/// The inputs a case was typed from, and the counts it adds (`case_evidence`).
fn case_evidence(case: &Value) -> Result<((String, bool), Value, BTreeMap<&'static str, i64>), String> {
    let text = |field: &str| case[field].as_str().ok_or("invalid case text or identity");
    let (identifier, document, expected, actual) = (text("identifier")?, text("document")?, text("expected")?, text("actual")?);
    let group = case["group"].as_i64();
    let wrong = case["initially_wrong"].as_bool().ok_or("invalid case intervention mode")?;
    if wrong && group.is_none() {
        return Err("invalid case intervention mode".to_owned());
    }
    let (Some(last), Some(intended)) = (case["final_group"].as_i64(), case["expected_final_group"].as_i64()) else { return Err("invalid case final layout".to_owned()) };
    let error = &case["error"];
    if !error.is_null() && !error.is_string() {
        return Err("invalid case execution error".to_owned());
    }
    let exact = error.is_null() && actual == expected;
    let length = actual.chars().count() != expected.chars().count();
    let layout = last == intended;
    if case["exact"].as_bool() != Some(exact) || case["length_mismatch"].as_bool() != Some(length) || case["final_layout_matches"].as_bool() != Some(layout) {
        return Err("case outcome contradicts actual text or layout".to_owned());
    }
    let characters: Vec<char> = expected.chars().collect();
    let intervention = &case["intervention"];
    if wrong {
        let event = count(&intervention["event"], "case intervention event")?;
        let source = intervention["source_group"].as_i64();
        let target = intervention["target_group"].as_i64();
        let observed = intervention["observed"].as_str().unwrap_or("");
        if event < 1
            || event.to::<usize>() > characters.len()
            || source.is_none()
            || target != group
            || source == target
            || intervention["expected"].as_str() != Some(characters[event.to::<usize>() - 1].to_string().as_str())
            || observed.chars().count() != 1
            || intervention["observed"] == intervention["expected"]
        {
            return Err("case does not show an effective wrong intervention".to_owned());
        }
    } else if !intervention.is_null() {
        return Err("correct case unexpectedly has a wrong intervention".to_owned());
    }
    let corrections = case["corrections"].as_array().ok_or("invalid case corrections or restoration without injection")?;
    if wrong && exact && corrections.is_empty() {
        return Err("invalid case corrections or restoration without injection".to_owned());
    }
    let mut layout_errors = 0;
    for correction in corrections {
        let event = count(&correction["event"], "case correction event")?;
        if event < 1 || event.to::<usize>() > characters.len() || ["source_group", "target_group", "actual_group"].iter().any(|key| !matches!(correction[*key].as_i64(), Some(0 | 1))) {
            return Err("invalid case correction layout or event".to_owned());
        }
        for (name, caret) in [("before", "caret_before"), ("after", "caret_after")] {
            let held = correction[name].as_str().ok_or("invalid case correction text or caret")?;
            if (count(&correction[caret], "case correction caret")?).to::<usize>() > held.chars().count() {
                return Err("invalid case correction text or caret".to_owned());
            }
        }
        layout_errors += i64::from(correction["actual_group"] != correction["target_group"]);
    }
    let counts = BTreeMap::from([
        ("rows", 1), ("initially_correct", i64::from(!wrong)), ("initially_wrong", i64::from(wrong)), ("preserved_correct", i64::from(!wrong && exact)), ("exactly_restored", i64::from(wrong && exact)),
        ("correct_text_corruptions", i64::from(!wrong && !exact)), ("length_mismatches", i64::from(length)), ("injections", corrections.len().to::<i64>()), ("execution_errors", i64::from(!error.is_null())),
        ("correction_layout_mismatches", layout_errors), ("final_layout_mismatches", i64::from(!layout)),
    ]);
    let inputs = json!([document, group, expected, count(&case["trimmed_left"], "case left trim")?, count(&case["trimmed_right"], "case right trim")?, intended]);
    Ok(((identifier.to_owned(), wrong), inputs, counts))
}

/// `validate_full_report`: aggregates recomputed from the private cases, without reading corpus text.
pub fn validate_full_report(report: &Value, identity: &Value) -> Result<(), String> {
    let selection = &report["selection"];
    let identifiers: Vec<&str> = selection["source_ids"].as_array().ok_or("case selection IDs are missing")?.iter().filter_map(Value::as_str).collect();
    let selected: HashSet<&str> = identifiers.iter().copied().collect();
    if selected.len() != identifiers.len() || identifiers.len().to::<i64>() != count(&selection["selected_rows"], "selected case rows")? {
        return Err("case selection IDs are missing, duplicated or inconsistent".to_owned());
    }
    let membership = |field: &str| -> HashSet<&str> { identity["test_membership"][field].as_array().into_iter().flatten().filter_map(Value::as_str).collect() };
    let (row_hashes, document_hashes) = (membership("row_ids_sha256"), membership("document_ids_sha256"));
    if selected.iter().any(|name| !row_hashes.contains(hex_digest(name).as_str())) {
        return Err("selected case is outside immutable test membership".to_owned());
    }
    let omitted: HashSet<&str> = selection["unsupported"].as_array().ok_or("invalid unsupported case selection")?.iter().filter_map(|item| item["identifier"].as_str()).collect();
    let mut reference: Option<BTreeMap<(String, bool), Value>> = None;
    for name in PROFILES {
        let profile = &report["profiles"][name];
        for block in [profile, &profile["early_off"]] {
            for model in ["baseline", "candidate"] {
                let mut inputs = BTreeMap::new();
                let mut counts: BTreeMap<&str, i64> = COUNTS.iter().map(|name| (*name, 0)).collect();
                for case in block["cases"][model].as_array().ok_or("missing case rows")? {
                    let (key, values, outcome) = case_evidence(case)?;
                    if inputs.contains_key(&key) || !selected.contains(key.0.as_str()) || omitted.contains(key.0.as_str()) {
                        return Err("duplicate, omitted or unselected case".to_owned());
                    }
                    if !document_hashes.contains(hex_digest(values[0].as_str().unwrap_or("")).as_str()) {
                        return Err("case document is outside immutable test membership".to_owned());
                    }
                    inputs.insert(key, values);
                    for (field, number) in outcome {
                        *counts.get_mut(field).expect("a known count") += number;
                    }
                }
                let claimed = &block["counts"][model];
                if COUNTS.iter().any(|field| claimed[*field].as_i64() != Some(counts[field])) {
                    return Err("claimed counts differ from actual cases".to_owned());
                }
                if reference.as_ref().is_some_and(|known| known != &inputs) {
                    return Err("compared models, modes or profiles have different case inputs".to_owned());
                }
                reference = Some(inputs);
            }
        }
    }
    let reference = reference.ok_or("no case rows")?;
    let correct: BTreeMap<&String, &Value> = reference.iter().filter(|(key, _)| !key.1).map(|(key, values)| (&key.0, values)).collect();
    let wanted: HashSet<&str> = selected.difference(&omitted).copied().collect();
    if correct.len() != wanted.len() || correct.keys().any(|name| !wanted.contains(name.as_str())) {
        return Err("case coverage or paired correct/wrong inputs differ".to_owned());
    }
    for (key, values) in &reference {
        if key.1 && correct.get(&key.0) != Some(&values) {
            return Err("case coverage or paired correct/wrong inputs differ".to_owned());
        }
    }
    let documents = |group: Option<i64>| correct.values().filter(|values| values[1].as_i64() == group).map(|values| values[0].as_str().unwrap_or("")).collect::<HashSet<_>>().len();
    if json!({"0": documents(Some(0)), "1": documents(Some(1)), "None": documents(None)}) != selection["replayed_documents"] {
        return Err("case document counts or one-focus-per-document policy differ".to_owned());
    }
    Ok(())
}

/// `devchat_block`: the receipt's block of the candidate's developer-chat test, from the immutable result in its ledger.
fn devchat_block(artifact_sha256: &str, ledger: &Path) -> Result<Value, String> {
    let mut paths: Vec<_> = std::fs::read_dir(ledger).map_err(|error| error.to_string())?.filter_map(|entry| entry.ok().map(|entry| entry.path())).collect();
    paths.retain(|path| path.to_string_lossy().ends_with(".access.json"));
    paths.sort();
    for access in paths {
        let identity = read_json(&access)?;
        if identity["pairs"]["candidate"]["sha256"].as_str() != Some(artifact_sha256) || identity["test_membership"]["messages"].as_array().is_none_or(Vec::is_empty) {
            continue;
        }
        let path = std::path::PathBuf::from(access.to_string_lossy().replace(".access.json", ".result.json"));
        let result = read_json(&path)?;
        if result.get("error").is_some() {
            return Err("the developer-chat test of this candidate failed to run".to_owned());
        }
        let pins = &identity["messages_sha256"];
        let messages = pins.as_str().map_or_else(|| hex_digest(&ks_text::json_dumps(pins, &ks_text::JsonOptions::default())), str::to_owned);
        let pair = |name: &str| json!({"model_version": identity["pairs"][name]["model_version"], "sha256": identity["pairs"][name]["sha256"]});
        return Ok(json!({"protocol": identity["protocol"], "bench": identity["bench"], "messages_sha256": messages, "seed": identity["seed"], "messages": result["messages"], "access": result["access"], "result_sha256": checksum(&path)?,
            "pairs": {"candidate": pair("candidate"), "released": pair("released"), "baseline": pair("baseline")}, "results": result["results"], "discordance": result["discordance"], "floors": result["floors"]}));
    }
    Err("no developer-chat test of this candidate is recorded".to_owned())
}

/// `export_receipt`.
pub fn export_receipt(request: &EvaluationRequest<'_>, report_path: &Path, devchat_ledger: &Path, output: &Path) -> Result<Value, String> {
    let root = request.root;
    let models_dir = request.resource_dir.join("models");
    let seal = validate_candidate_seal(root, request.artifact, request.seal, request.corpus, &models_dir)?;
    validate_prefix_seal(root, request.prefix, request.prefix_seal, true)?;
    let identity = evaluation_identity(request)?;
    let report = read_json(report_path)?;
    let recipe = read_json(&root.join(RECIPE))?;
    let protocols = read_json(&root.join("model/context_v3/protocols.json"))?;
    if report["schema_version"] != json!(1) || report["split"] != json!("test") || report["identity"] != identity.value || report["protocol"] != protocols["sequence"] || report["gate_policy"] != recipe["gate_policy"] || report["promotion_passed"] != json!(true) || report.get("execution_error").is_some() {
        return Err("report is not an accepted current sealed test".to_owned());
    }
    if std::fs::read(request.ledger.join(format!("{}.access.json", identity.key))).map_err(|error| error.to_string())? != crate::artifact::canonical(&identity.value)
        || std::fs::read(request.ledger.join(format!("{}.outcome.json", identity.key))).map_err(|error| error.to_string())? != std::fs::read(report_path).map_err(|error| error.to_string())?
    {
        return Err("immutable access or outcome does not confirm report".to_owned());
    }
    validate_full_report(&report, &identity.value)?;
    let mut hashes = seal["provenance"].as_object().cloned().ok_or("seal provenance")?;
    for (name, fingerprint) in identity.value["runtime"].as_object().ok_or("runtime provenance")? {
        if hashes.get(name).is_some_and(|known| known != fingerprint) {
            return Err("seal and runtime provenance disagree".to_owned());
        }
        hashes.insert(name.clone(), fingerprint.clone());
    }
    let calibration = &seal["calibration"];
    let mut public_calibration = public_calibration_block(calibration);
    public_calibration[OUTSIDE_LONE_WORD] = public_calibration_block(&calibration[OUTSIDE_LONE_WORD]);
    let selection = &report["selection"];
    let mut public_profiles = Map::new();
    for profile in PROFILES {
        let private = &report["profiles"][profile];
        public_profiles.insert(profile.to_owned(), json!({"counts": private["counts"], "gates": private["gates"], "early_off": {"counts": private["early_off"]["counts"], "gates": private["early_off"]["gates"]}}));
    }
    let payload = read_json(request.artifact)?;
    let prefix_payload = read_json(request.prefix)?;
    let prefix = VersionedPrefixModel::load(request.prefix).map_err(|error| error.name.to_string())?;
    let mut receipt = json!({
        "schema_version": 1, "feature_version": 3, "model_version": seal["model_version"], "conversion_threshold": seal["conversion_threshold"], "artifact_sha256": checksum(request.artifact)?,
        "weights_sha256": payload["weights_sha256"], "baseline_path": BASELINE, "baseline_sha256": identity.value["baseline_sha256"], "recipe_path": RECIPE, "recipe_sha256": hashes[RECIPE],
        "prefix_model_version": prefix.version(), "prefix_feature_version": prefix.feature_version, "prefix_conversion_threshold": prefix.model.conversion_threshold, "prefix_artifact_sha256": identity.value["prefix_candidate_sha256"],
        "prefix_weights_sha256": prefix_payload["weights_sha256"], "prefix_candidate_seal_sha256": identity.value["prefix_candidate_seal_sha256"], "prefix_baseline_path": PREFIX_BASELINE, "prefix_baseline_sha256": identity.value["prefix_baseline_sha256"],
        "candidate_seal_sha256": checksum(request.seal)?, "full_test_report_sha256": checksum(report_path)?, "corpus_manifest_sha256": identity.value["corpus_manifest_sha256"], "provenance": hashes,
        "calibration": public_calibration, "test": {"documents": selection["replayed_documents"], "selected_rows": selection["selected_rows"], "trimmed_rows": selection["trimmed_rows"], "unsupported_rows": selection["unsupported"].as_array().map_or(0, Vec::len), "profiles": public_profiles},
        "gate_policy": recipe["gate_policy"], "protocol": protocols["receipt"], "scope": protocols["scope"], "quality_gates_passed": true,
    });
    receipt[DEVCHAT] = devchat_block(&checksum(request.artifact)?, devchat_ledger)?;
    validate_receipt(&receipt, request.artifact, root, request.prefix, &models_dir)?;
    if output.exists() {
        if std::fs::read(output).map_err(|error| error.to_string())? != readable(&receipt) {
            return Err("immutable test record differs: the receipt".to_owned());
        }
    } else {
        std::fs::write(output, readable(&receipt)).map_err(|error| error.to_string())?;
    }
    Ok(receipt)
}
