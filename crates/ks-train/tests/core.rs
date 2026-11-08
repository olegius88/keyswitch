//! The numeric core against dumps of the previous trainer: every float compared as bits.
#![expect(clippy::disallowed_types, clippy::disallowed_macros, clippy::disallowed_methods, reason = "temporary: these tests still build and read their JSON as untyped trees; step 7 of the strict-json plan types them and removes this")]
use ks_cast::To as _;
use std::collections::HashMap;
use std::io::Read;

use ks_train::artifact::{artifact_bytes, canonical};
use ks_train::kernel::{FeatureMass, Packed, epoch, log_loss, predict};
use ks_train::selection::{ThresholdOptions, assess_epoch, choose_threshold, select_features};
use serde_json::{Value, json};

fn bits(text: &Value) -> u64 {
    u64::from_str_radix(text.as_str().expect("hex"), 16).expect("hex")
}

fn same(found: &[f64], wanted: &Value, what: &str, seed: &Value) {
    let wanted = wanted.as_array().expect("array");
    assert_eq!(found.len(), wanted.len(), "{what} of case {seed}");
    for (index, value) in found.iter().enumerate() {
        assert_eq!(value.to_bits(), bits(&wanted[index]), "{what}[{index}] of case {seed}");
    }
}

fn data() -> Value {
    let mut text = String::new();
    flate2::read::GzDecoder::new(&include_bytes!("data/core.json.gz")[..]).read_to_string(&mut text).expect("gzip");
    serde_json::from_str(&text).expect("json")
}

#[test]
fn the_core_matches_bit_for_bit() {
    let data = data();
    for case in data["cases"].as_array().expect("cases") {
        let seed = &case["seed"];
        let names: Vec<String> = case["names"].as_array().unwrap().iter().map(|name| name.as_str().unwrap().to_owned()).collect();
        let rows: Vec<(Vec<(String, f64)>, u8, f64)> = case["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| (row[0].as_object().unwrap().iter().map(|(name, value)| (name.clone(), value.as_f64().unwrap())).collect(), row[1].as_u64().unwrap().to::<u8>(), row[2].as_f64().unwrap()))
            .collect();
        let packed = Packed::build(rows.iter().map(|row| (row.0.as_slice(), row.1, row.2)), &names).unwrap();
        let mut weights = vec![0.0; names.len() * 4];
        let mut accumulators = vec![1.0; names.len() * 4];
        if seed.as_i64().unwrap() % 3 == 0 {
            accumulators[..4].fill(f64::INFINITY);
        }
        let mut predictions = Vec::new();
        for step in case["trace"].as_array().unwrap() {
            epoch(&packed, &mut weights, &mut accumulators, 0.1);
            predictions = predict(&packed, &weights);
            same(&weights, &step["weights"], "weights", seed);
            same(&predictions, &step["predictions"], "predictions", seed);
            assert_eq!(log_loss(&predictions, &packed).to_bits(), bits(&step["loss"]), "loss of case {seed}");
        }
        let two = [("a".to_owned(), predictions.as_slice(), packed.labels.as_slice()), ("b".to_owned(), predictions.as_slice(), packed.labels.as_slice())];
        if packed.labels.contains(&1) {
            let chosen = assess_epoch(&two, &[0.2, 0.5, 0.7, 0.9], 0.5, 0).unwrap();
            match (&chosen, &case["selection"]) {
                (None, Value::Null) => {}
                (Some(found), wanted) => {
                    assert_eq!(found.threshold.to_bits(), bits(&wanted["threshold"]), "case {seed}");
                    assert_eq!(found.minimum_recall.to_bits(), bits(&wanted["minimum_recall"]), "case {seed}");
                    assert_eq!(found.pooled_recall.to_bits(), bits(&wanted["pooled_recall"]), "case {seed}");
                    assert_eq!(found.net_benefit, wanted["net_benefit"].as_i64().unwrap(), "case {seed}");
                    assert_eq!(found.minimum_net_benefit, wanted["minimum_net_benefit"].as_i64().unwrap(), "case {seed}");
                }
                other => panic!("case {seed}: {other:?}"),
            }
        }
        let options = ThresholdOptions { minimum_net_benefit: 0, minimum_recall: 0.0, minimum_threshold: 0.0, net_benefit_tolerance: 0.1, maximum_threshold: 0.99, authored_floor: 0.2 };
        let (threshold, report, passed) = choose_threshold(&two, &[0.1, 0.3, 0.5, 0.8, 0.95], &options).unwrap();
        let wanted = &case["threshold"];
        assert_eq!(threshold.to_bits(), bits(&wanted[0]), "threshold of case {seed}");
        assert_eq!(passed, wanted[1].as_bool().unwrap(), "case {seed}");
        assert_eq!(report["net_benefit"], wanted[2], "case {seed}");
        assert_eq!(report["false_conversions"], wanted[3], "case {seed}");
        let mut mass = FeatureMass::default();
        for row in &rows {
            mass.add(row.0.iter().map(|(name, _)| name.as_str()), row.2).unwrap();
        }
        let wanted_mass = case["mass"].as_object().unwrap();
        assert_eq!(mass.values.len(), wanted_mass.len());
        for (name, value) in wanted_mass {
            assert_eq!(mass.values[name].to_bits(), bits(value), "mass of {name} in case {seed}");
        }
        let masses: HashMap<String, f64> = mass.values.clone();
        assert_eq!(json!(select_features(&masses, 1.0, 10).unwrap()), case["selected"], "selection of case {seed}");
    }
}

#[test]
fn the_files_read_as_written() {
    let data = data();
    let payload = json!({"actions": ["keep", "convert"], "conversion_threshold": 0.9, "feature_version": "x", "version": "context-v3-abc", "weights": {"б": [0.1, -0.0], "a": [1e-9, 2.5]}, "weights_sha256": "00"});
    assert_eq!(String::from_utf8(artifact_bytes(&payload)).unwrap(), data["artifact"].as_str().unwrap());
    assert_eq!(String::from_utf8(canonical(&json!({"b": 1, "а": [1.5, "я"]}))).unwrap(), data["canonical"].as_str().unwrap());
}
