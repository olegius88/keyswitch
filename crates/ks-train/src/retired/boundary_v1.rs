//! Boundary-v1, rejected by its sealed test. The candidate, the seal and the report are anchored by digests, and a replay
//! repeats the fit and the test on the phrase reserve of context-v2 and compares all three byte for byte. It is a
//! regression replay of an observed test, not a new evaluation.
use ks_cast::To as _;
use super::sources::BOUNDARY_V1;
use super::{read_object, reference_models, unchanged};
use crate::artifact::canonical;
use crate::corpus::checksum;
use crate::ortho_inputs::{assign, load_phrases};
use crate::physical::{hex_digest, pair};
use indexmap::IndexMap;
use ks_lang::boundary_model::{BoundaryModel, FEATURE_VERSION, MAX_SUFFIX, features};
use ks_lang::constants::BOUNDARY_AMBIGUOUS_PUNCTUATION;
use ks_lang::py_sum;
use ks_text::pynum::py_round_ndigits;
use ks_text::pystr;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;

const DIRECTORY: &str = "model/boundary_v1";
const NAMESPACE: &str = "keyswitch:boundary-v1:20260906";
const SOURCE_SHA: &str = "1f7df948b946f1d1aed5efd2a83a9a5a09d2b8abcb9224045b40c9ec6720e793";
const EPOCHS: usize = 35;
const FAMILY_KEY_CHARACTERS: usize = 4;
const LEARNING_RATE: f64 = 0.1;
const SPLIT_BUCKETS: u64 = 10;
const TRAIN_BUCKETS: u64 = 7;
const THRESHOLDS: [f64; 7] = [0.9, 0.95, 0.98, 0.99, 0.995, 0.999, 0.9995];
const WORD_MIN: usize = 3;
const WORD_MAX: usize = 24;
const WEIGHT_DECIMALS: i64 = 12;
const CHOICE_DIGITS: usize = 8;
const VERSION_HASH_CHARACTERS: usize = 12;
const MIN_DECIDED_FRACTION: f64 = 0.8;
const PROBABILITY_FLOOR: f64 = 1e-15;
const SUFFIXES: [&str; 7] = [",", ".", ";", "]", "...", ",;", "'"];
const ANCHORS: [(&str, &str); 3] = [
    ("seal.json", "8ef2284b990f7b8ba8a08405341db168811459b41f59ef5f5b2d32bbfae29f31"),
    ("candidate.json", "5462c7ffa9b43ebf584f782dd1aeffc6ffe0d2323eb3ffc733277b978d968f75"),
    ("report.json", "ee0c0c807d19aee57a0de6328f1206deb750b0202b26c6279066e24071bf7c89"),
];
const SCOPE: &str = "Historical rejected boundary-v1 evidence: anchored seal, candidate and report, source pins checked against archived copies; a replay repeats the sealed fit and test and is no new evaluation, new test or current-engine acceptance.";

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Row {
    split: &'static str,
    family: String,
    original: String,
    label: usize,
    alternative: String,
    suffix: usize,
}

fn tail_of(text: &str) -> usize {
    pystr::len(text) - pystr::len(text.trim_end_matches(|character| BOUNDARY_AMBIGUOUS_PUNCTUATION.contains(character)))
}

fn split_of(family: &str) -> &'static str {
    let digest = hex_digest(&format!("{NAMESPACE}{family}"));
    let bucket = u64::from_str_radix(digest.get(..CHOICE_DIGITS).unwrap_or_default(), 16).expect("a digest") % SPLIT_BUCKETS;
    if bucket < TRAIN_BUCKETS {
        "train"
    } else if bucket == TRAIN_BUCKETS {
        "development"
    } else if bucket == TRAIN_BUCKETS + 1 {
        "calibration"
    } else {
        "test"
    }
}

fn rows(root: &Path, test: bool) -> Result<Vec<Row>, String> {
    let phrases = load_phrases(root)?;
    let mut words = BTreeSet::new();
    for item in assign(&phrases).iter().filter(|item| item.split == "reserve") {
        for word in crate::boundary::canonical_tokens(&item.phrase.text) {
            if pystr::isalpha(&word) && (WORD_MIN..=WORD_MAX).contains(&pystr::len(&word)) {
                words.insert(word);
            }
        }
    }
    rows_of(words, test)
}

/// The rows of these words of the reserve: a Russian word typed in the English layout, and each word with a literal
/// suffix of ambiguous punctuation after it.
fn rows_of(words: BTreeSet<String>, test: bool) -> Result<Vec<Row>, String> {
    let layouts = pair();
    let maximum = MAX_SUFFIX.to::<usize>();
    let mut result = BTreeSet::new();
    for word in words {
        let russian = word.chars().all(|character| ('а'..='я').contains(&character) || character == 'ё');
        let english = word.chars().all(|character| character.is_ascii_lowercase());
        if !russian && !english {
            continue;
        }
        let physical = if russian { layouts.translate(&word, "ru", "us").map_err(|_| "a word off the layout")? } else { word.clone() };
        let family: String = physical.replace('`', "t").chars().take(FAMILY_KEY_CHARACTERS).collect();
        let split = split_of(&family);
        if (split == "test") != test {
            continue;
        }
        let tail = tail_of(&physical);
        if russian && 0 < tail && tail <= maximum && tail < pystr::len(&physical) {
            result.insert(Row { split, family: family.clone(), original: physical.clone(), label: 0, alternative: word.clone(), suffix: tail });
        }
        for suffix in SUFFIXES {
            let observed = format!("{physical}{suffix}");
            // Every suffix is ambiguous punctuation, so the tail is never empty.
            let tail = tail_of(&observed);
            if tail <= maximum && tail < pystr::len(&observed) {
                let alternative = layouts.translate(&observed, "us", "ru").map_err(|_| "a word off the layout")?;
                result.insert(Row { split, family: family.clone(), alternative, original: observed, label: pystr::len(suffix), suffix: tail });
            }
        }
    }
    Ok(result.into_iter().collect())
}

type Encoded = Vec<(Row, Vec<IndexMap<String, f64>>)>;

fn encoded(root: &Path, data: Vec<Row>) -> Result<Encoded, String> {
    let models = reference_models(root)?;
    Ok(data
        .into_iter()
        .map(|row| {
            let candidates = (0..=row.suffix).map(|suffix| features(&row.original, &row.alternative, suffix.to::<i64>(), &models[0], &models[1])).collect();
            (row, candidates)
        })
        .collect())
}

fn metrics(model: &BoundaryModel, data: &[&(Row, Vec<IndexMap<String, f64>>)]) -> Value {
    let mut counts: BTreeMap<&str, i64> = BTreeMap::new();
    for (row, values) in data {
        let prediction = model.predict(values).expect("a row has candidates");
        *counts.entry("rows").or_default() += 1;
        *counts.entry(if row.label > 0 { "literal" } else { "word" }).or_default() += 1;
        match prediction.suffix_length {
            None => *counts.entry("abstained").or_default() += 1,
            Some(suffix) if suffix != row.label => {
                *counts.entry("errors").or_default() += 1;
                *counts.entry(if suffix < row.label { "lost_punctuation" } else { "split_word" }).or_default() += 1;
            }
            Some(_) => *counts.entry("correct").or_default() += 1,
        }
    }
    json!(counts)
}

/// One row's candidates as columns in the order the runtime adds them up.
struct Dense {
    label: usize,
    weight_of_class: f64,
    candidates: Vec<Vec<f64>>,
}

/// `sum(weight * value for ...)` in the features' own order, then the softmax.
fn probabilities(weights: &[f64], candidates: &[Vec<f64>]) -> Vec<f64> {
    let scores: Vec<f64> = candidates.iter().map(|values| py_sum(values.iter().zip(weights).map(|(value, weight)| weight * value))).collect();
    let maximum = ks_text::max_first(scores.iter().copied()).expect("a row has candidates");
    let exp: Vec<f64> = scores.iter().map(|value| (value - maximum).exp()).collect();
    let total = py_sum(exp.iter().copied());
    exp.iter().map(|value| value / total).collect()
}

fn train(root: &Path, pins: Map<String, Value>) -> Result<(Vec<u8>, Vec<u8>), String> {
    Ok(fitted(&encoded(root, rows(root, false)?)?, pins))
}

/// The fit, the threshold and the seal on rows already encoded.
fn fitted(data: &Encoded, pins: Map<String, Value>) -> (Vec<u8>, Vec<u8>) {
    let of = |split: &str| data.iter().filter(|(row, _)| row.split == split).collect::<Vec<_>>();
    let (training, development, calibration) = (of("train"), of("development"), of("calibration"));
    // The runtime adds features up in the order they were inserted; the optimizer steps through them by sorted name.
    let names: Vec<String> = training[0].1[0].keys().cloned().collect();
    let mut order: Vec<usize> = (0..names.len()).collect();
    order.sort_by(|&a, &b| pystr::cmp(&names[a], &names[b]));
    let literal = (training.iter().filter(|(row, _)| row.label > 0).count()).to::<f64>();
    let word = training.len().to::<f64>() - literal;
    let dense = |rows: &[&(Row, Vec<IndexMap<String, f64>>)], weight: &dyn Fn(bool) -> f64| -> Vec<Dense> {
        rows.iter()
            .map(|(row, candidates)| Dense {
                label: row.label,
                weight_of_class: weight(row.label > 0),
                candidates: candidates.iter().map(|values| names.iter().map(|name| values[name]).collect()).collect(),
            })
            .collect()
    };
    let total = training.len().to::<f64>();
    let trained = dense(&training, &|is_literal| total / (2.0 * if is_literal { literal } else { word }));
    let developed = dense(&development, &|is_literal| if is_literal { literal } else { word });
    let (mut weights, mut squared) = (vec![0.0_f64; names.len()], vec![1.0_f64; names.len()]);
    let (mut best, mut best_loss, mut epoch_used) = (weights.clone(), f64::INFINITY, 0_i64);
    for epoch in 0..EPOCHS {
        for row in &trained {
            let scores = probabilities(&weights, &row.candidates);
            for &name in &order {
                let gradient = py_sum(row.candidates.iter().zip(&scores).enumerate().map(|(index, (candidate, probability))| {
                    (probability - f64::from(u8::from(index == row.label))) * candidate[name]
                })) * row.weight_of_class;
                squared[name] += gradient * gradient;
                weights[name] -= LEARNING_RATE * gradient / squared[name].sqrt();
            }
        }
        let loss = py_sum(developed.iter().map(|row| {
            let probability = probabilities(&weights, &row.candidates)[row.label];
            let floored = if PROBABILITY_FLOOR >= probability { PROBABILITY_FLOOR } else { probability };
            -floored.ln() / row.weight_of_class
        }));
        if loss < best_loss {
            best.clone_from(&weights);
            best_loss = loss;
            epoch_used = epoch.to::<i64>() + 1;
        }
    }
    let table: IndexMap<String, f64> = names.iter().cloned().zip(best.iter().map(|value| py_round_ndigits(*value, WEIGHT_DECIMALS).expect("a finite weight"))).collect();
    let best_json = Value::Object(table.iter().map(|(name, value)| (name.clone(), json!(value))).collect());
    let mut threshold = THRESHOLDS[THRESHOLDS.len() - 1];
    for candidate in THRESHOLDS {
        let result = metrics(&BoundaryModel::new(table.clone(), candidate, "candidate"), &calibration);
        if result.get("errors").and_then(Value::as_i64).unwrap_or(0) == 0 {
            threshold = candidate;
            break;
        }
    }
    let digest = crate::physical::hex(&<sha2::Sha256 as sha2::Digest>::digest(canonical(&best_json)));
    let version = format!("boundary-v1-{}", digest.get(..VERSION_HASH_CHARACTERS).unwrap_or_default());
    let raw = canonical(&json!({"feature_version": FEATURE_VERSION, "version": version, "weights": best_json, "threshold": threshold}));
    let model = BoundaryModel::new(table, threshold, version);
    let families: HashSet<(&str, &str)> = data.iter().map(|(row, _)| (row.split, row.family.as_str())).collect();
    let mut family_counts: BTreeMap<&str, i64> = BTreeMap::new();
    for (split, _) in families {
        *family_counts.entry(split).or_default() += 1;
    }
    let receipt = json!({
        "schema_version": 1, "source_sha256": SOURCE_SHA, "source_scope": "context-v2 unused reserve; public CC0 only",
        "provenance": pins, "candidate_sha256": crate::physical::hex(&<sha2::Sha256 as sha2::Digest>::digest(&raw)),
        "namespace": NAMESPACE, "epoch": epoch_used, "training_rows": training.len(),
        "development": metrics(&model, &development), "calibration": metrics(&model, &calibration),
        "family_counts": family_counts,
        "promotion_gates": {"test_errors_max": 0, "test_decided_fraction_min": MIN_DECIDED_FRACTION},
        "limitations": "Synthetic segmentation labels; lexical resources may know held-out words. No independent real-user intent or native input evidence.",
    });
    (raw, canonical(&receipt))
}

fn evaluate(root: &Path) -> Result<Vec<u8>, String> {
    let directory = root.join(DIRECTORY);
    let (candidate, seal) = (directory.join("candidate.json"), directory.join("seal.json"));
    let data = encoded(root, rows(root, true)?)?;
    let model = BoundaryModel::load(&candidate).map_err(|error| error.name.to_string())?;
    let (result, accepted) = judged(&model, &data)?;
    let families: HashSet<&str> = data.iter().map(|(row, _)| row.family.as_str()).collect();
    Ok(canonical(&json!({
        "schema_version": 1, "seal_sha256": checksum(&seal)?, "candidate_sha256": checksum(&candidate)?,
        "test": result, "test_families": families.len(), "accepted": accepted,
        "scope": "first sealed synthetic boundary test; replays are regression checks",
    })))
}

/// The test's counts and whether they pass: no error, and most rows decided.
fn judged(model: &BoundaryModel, data: &Encoded) -> Result<(Value, bool), String> {
    let result = metrics(model, &data.iter().collect::<Vec<_>>());
    let count = (result["rows"].as_i64().ok_or("no test rows")?).to::<f64>();
    let accepted = result.get("errors").and_then(Value::as_i64).unwrap_or(0) == 0 && result.get("correct").and_then(Value::as_i64).unwrap_or(0).to::<f64>() / count >= MIN_DECIDED_FRACTION;
    Ok((result, accepted))
}

/// The evidence is whole and says what its numbers say. Reads no phrase.
pub fn verify(root: &Path) -> Result<Value, String> {
    let directory = root.join(DIRECTORY);
    for (name, expected) in ANCHORS {
        if checksum(&directory.join(name))? != expected {
            return Err(format!("historical boundary-v1 anchor changed: {name}"));
        }
    }
    historical_evidence(root)
}

/// What the anchored files say, checked as if nothing anchored them.
fn historical_evidence(root: &Path) -> Result<Value, String> {
    let directory = root.join(DIRECTORY);
    let seal = read_object(&directory.join("seal.json"))?;
    let report = read_object(&directory.join("report.json"))?;
    BOUNDARY_V1.recorded(seal.get("provenance").unwrap_or(&Value::Null), &BOUNDARY_V1.all_paths(), root)?;
    if seal.get("candidate_sha256").and_then(Value::as_str) != Some(checksum(&directory.join("candidate.json"))?.as_str()) {
        return Err("boundary candidate/provenance changed".to_owned());
    }
    if report.get("seal_sha256").and_then(Value::as_str) != Some(checksum(&directory.join("seal.json"))?.as_str()) || report.get("candidate_sha256") != seal.get("candidate_sha256") {
        return Err("boundary test identity changed".to_owned());
    }
    if seal.get("promotion_gates") != Some(&json!({"test_errors_max": 0, "test_decided_fraction_min": MIN_DECIDED_FRACTION})) {
        return Err("boundary promotion gates changed".to_owned());
    }
    let metrics = report.get("test").and_then(Value::as_object).ok_or("boundary test metrics missing")?;
    let count = |name: &str| -> Result<i64, String> {
        match metrics.get(name) {
            None => Ok(0),
            Some(value) => value.as_i64().filter(|number| *number >= 0).ok_or_else(|| "invalid boundary count".to_owned()),
        }
    };
    let (rows, correct, abstained, errors) = (count("rows")?, count("correct")?, count("abstained")?, count("errors")?);
    let (literal, word, lost, split) = (count("literal")?, count("word")?, count("lost_punctuation")?, count("split_word")?);
    if rows <= 0 || correct + abstained + errors != rows || literal + word != rows || lost + split != errors {
        return Err("inconsistent boundary counts".to_owned());
    }
    let accepted = errors == 0 && correct.to::<f64>() / rows.to::<f64>() >= MIN_DECIDED_FRACTION;
    if report.get("accepted").and_then(Value::as_bool) != Some(accepted) {
        return Err("boundary promotion contradicts metrics".to_owned());
    }
    let active = root.join("resources/models/boundary-v1.json");
    if active.exists() && (!accepted || seal.get("candidate_sha256").and_then(Value::as_str) != Some(checksum(&active)?.as_str())) {
        return Err("unapproved boundary model must not ship".to_owned());
    }
    Ok(json!({"schema_version": 1, "accepted": accepted, "active": active.exists(), "test": metrics, "historical_evidence_verified": true, "current_runtime_verified": false, "scope": SCOPE}))
}

/// Replays the fit and the sealed test; writes nothing.
pub fn replay(root: &Path) -> Result<Value, String> {
    let result = verify(root)?;
    let directory = root.join(DIRECTORY);
    let seal = read_object(&directory.join("seal.json"))?;
    let pins = BOUNDARY_V1.recorded(seal.get("provenance").unwrap_or(&Value::Null), &BOUNDARY_V1.all_paths(), root)?;
    let (candidate, sealed) = train(root, pins)?;
    let read = |name: &str| std::fs::read(directory.join(name)).map_err(|error| error.to_string());
    unchanged(&candidate, &read("candidate.json")?, "boundary-v1 candidate changed")?;
    unchanged(&sealed, &read("seal.json")?, "boundary-v1 seal changed")?;
    unchanged(&evaluate(root)?, &read("report.json")?, "boundary-v1 test replay changed")?;
    let mut result = result;
    result["frozen_training_replay"] = json!(true);
    Ok(result)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::super::testing::{Tree, refused, repository};
    use super::*;

    const FILES: [&str; 3] = ["seal.json", "report.json", "candidate.json"];

    /// The tree's evidence back to the repository's, then changed by `edit`, and checked past its anchors.
    fn evidence_after(tree: &Tree, edit: impl FnOnce(&Tree)) -> Result<Value, String> {
        for name in FILES {
            tree.write(&format!("{DIRECTORY}/{name}"), &std::fs::read(repository().join(DIRECTORY).join(name)).expect("a file"));
        }
        tree.remove("resources/models/boundary-v1.json");
        edit(tree);
        historical_evidence(&tree.root)
    }

    fn edit(tree: &Tree, name: &str, change: impl FnOnce(&mut Value)) {
        tree.edit(&format!("{DIRECTORY}/{name}"), change);
    }

    /// The report changed, and still naming the seal as it is now.
    fn report(tree: &Tree, change: impl FnOnce(&mut Value)) {
        let seal = checksum(&tree.path(&format!("{DIRECTORY}/seal.json"))).expect("a digest");
        edit(tree, "report.json", |report| {
            report["seal_sha256"] = json!(seal);
            change(report);
        });
    }

    /// A test that would have passed: no error, and most rows decided.
    fn passed(report: &mut Value) {
        for (name, value) in [("correct", 10_000), ("abstained", 1992), ("errors", 0), ("lost_punctuation", 0)] {
            report["test"][name] = json!(value);
        }
        report["accepted"] = json!(true);
    }

    #[test]
    fn the_seal_and_the_report_describe_the_candidate() {
        let tree = Tree::new("boundary-evidence");
        let kept = evidence_after(&tree, |_| ()).expect("the kept evidence");
        assert_eq!((&kept["accepted"], &kept["active"]), (&json!(false), &json!(false)));
        refused(evidence_after(&tree, |tree| edit(tree, "seal.json", |seal| drop(seal["provenance"].as_object_mut().expect("pins").remove("tools/train_boundary_model.py")))), "recorded provenance names other files");
        refused(evidence_after(&tree, |tree| edit(tree, "seal.json", |seal| seal["candidate_sha256"] = json!("0"))), "boundary candidate/provenance changed");
        refused(evidence_after(&tree, |tree| edit(tree, "report.json", |report| report["seal_sha256"] = json!("0"))), "boundary test identity changed");
        refused(evidence_after(&tree, |tree| edit(tree, "report.json", |report| report["candidate_sha256"] = json!("0"))), "boundary test identity changed");
        let gates = |tree: &Tree| {
            edit(tree, "seal.json", |seal| seal["promotion_gates"]["test_errors_max"] = json!(1));
            report(tree, |_| ());
        };
        refused(evidence_after(&tree, gates), "boundary promotion gates changed");
        refused(evidence_after(&tree, |tree| report(tree, |report| report["test"] = json!([]))), "boundary test metrics missing");
        for value in [json!(-1), json!("many")] {
            refused(evidence_after(&tree, |tree| report(tree, |report| report["test"]["rows"] = value)), "invalid boundary count");
        }
        for (name, value) in [("rows", 0), ("correct", 1994), ("literal", 11_948), ("lost_punctuation", 0)] {
            refused(evidence_after(&tree, |tree| report(tree, |report| report["test"][name] = json!(value))), "inconsistent boundary counts");
        }
        refused(evidence_after(&tree, |tree| report(tree, |report| report["accepted"] = json!(true))), "boundary promotion contradicts metrics");
        let undecided = |report: &mut Value| {
            report["test"]["errors"] = json!(0);
            report["test"]["lost_punctuation"] = json!(0);
            report["test"]["correct"] = json!(1994);
        };
        assert_eq!(evidence_after(&tree, |tree| report(tree, undecided)).expect("an undecided test")["accepted"], json!(false));
        assert_eq!(evidence_after(&tree, |tree| report(tree, passed)).expect("a passing test")["accepted"], json!(true));
    }

    #[test]
    fn only_the_sealed_candidate_of_a_passing_test_may_ship() {
        let tree = Tree::new("boundary-active");
        let candidate = std::fs::read(repository().join(DIRECTORY).join("candidate.json")).expect("the candidate");
        let shipped = |tree: &Tree, bytes: &[u8]| tree.write("resources/models/boundary-v1.json", bytes);
        let refusal = "unapproved boundary model must not ship";
        refused(evidence_after(&tree, |tree| shipped(tree, &candidate)), refusal);
        refused(evidence_after(&tree, |tree| {
            report(tree, passed);
            shipped(tree, b"{}");
        }), refusal);
        let active = evidence_after(&tree, |tree| {
            report(tree, passed);
            shipped(tree, &candidate);
        });
        assert_eq!(active.expect("the sealed candidate")["active"], json!(true));
    }

    #[test]
    fn an_anchored_file_that_changed_stops_the_check() {
        let tree = Tree::new("boundary-anchor");
        tree.edit(&format!("{DIRECTORY}/report.json"), |report| report["accepted"] = json!(true));
        refused(verify(&tree.root), "historical boundary-v1 anchor changed: report.json");
    }

    #[test]
    fn a_word_that_is_all_tail_or_whose_tail_is_too_long_gives_no_row_of_it() {
        let words: BTreeSet<String> = ["ххх", "аххххххххх", "люблю", "hello"].iter().map(|word| (*word).to_owned()).collect();
        let mut made = rows_of(words.clone(), false).expect("rows");
        made.extend(rows_of(words, true).expect("rows"));
        let originals: BTreeSet<&str> = made.iter().map(|row| row.original.as_str()).collect();
        // «ххх» is typed «[[[»: no word is left before the punctuation, with a suffix or without.
        assert!(!originals.iter().any(|original| original.starts_with("[[[")), "{originals:?}");
        // «аххххххххх» ends in more ambiguous keys than a literal suffix may hold.
        assert!(!originals.iter().any(|original| original.starts_with("f[[[")), "{originals:?}");
        // «люблю» is typed «k.,k.»: its last key may be a full stop, and so it is a row of its own.
        assert!(originals.contains("k.,k.") && originals.contains("k.,k.,") && originals.contains("hello..."), "{originals:?}");
    }

    fn candidate(value: f64) -> IndexMap<String, f64> {
        [("x".to_owned(), value)].into_iter().collect()
    }

    fn row(split: &'static str, label: usize, candidates: Vec<IndexMap<String, f64>>) -> (Row, Vec<IndexMap<String, f64>>) {
        let suffix = candidates.len() - 1;
        (Row { split, family: format!("{split}{label}"), original: "word,".to_owned(), label, alternative: "щкв,".to_owned(), suffix }, candidates)
    }

    #[test]
    fn a_development_row_past_the_floor_keeps_the_first_epoch() {
        let data = vec![
            row("train", 1, vec![candidate(0.0), candidate(1.0)]),
            row("train", 1, vec![candidate(0.0), candidate(1.0)]),
            row("train", 0, vec![candidate(0.0), candidate(-1.0)]),
            row("train", 0, vec![candidate(0.0), candidate(-1.0)]),
            // The more the fit learns, the less likely the whole word is here: below the floor from the first epoch on.
            row("development", 0, vec![candidate(0.0), candidate(1000.0)]),
        ];
        let (_, receipt) = fitted(&data, Map::new());
        let receipt: Value = serde_json::from_slice(&receipt).expect("json");
        assert_eq!(receipt["epoch"], json!(1), "{receipt}");
    }

    #[test]
    fn a_test_passes_without_an_error_and_with_most_rows_decided() {
        let model = BoundaryModel::new(candidate(1.0), 0.9, "probe");
        let decided = vec![row("test", 1, vec![candidate(0.0), candidate(10.0)]), row("test", 0, vec![candidate(10.0), candidate(0.0)])];
        let (counts, accepted) = judged(&model, &decided).expect("counts");
        assert!(accepted, "{counts}");
        let undecided = vec![row("test", 1, vec![candidate(0.0), candidate(0.0)]), row("test", 0, vec![candidate(10.0), candidate(0.0)])];
        let (counts, accepted) = judged(&model, &undecided).expect("counts");
        assert!(!accepted && counts["abstained"] == json!(1), "{counts}");
        let wrong = vec![row("test", 1, vec![candidate(10.0), candidate(0.0)])];
        assert!(!judged(&model, &wrong).expect("counts").1);
    }
}
