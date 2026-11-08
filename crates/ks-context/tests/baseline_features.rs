//! The frozen baseline model of the sealed tests answers as the trainer that fitted it wrote its features.
#![expect(clippy::disallowed_types, clippy::disallowed_methods, reason = "temporary: these tests still build and read their JSON as untyped trees; step 7 of the strict-json plan types them and removes this")]
use ks_cast::To as _;
use ks_backend::{FieldContext, FieldRole};
use ks_context::context_action_features::extract_baseline_features;
use ks_context::context_model::{ContextEvidence, ContextModel};
use serde_json::Value;
use std::path::Path;

fn role(name: &str) -> FieldRole {
    match name {
        "text" => FieldRole::Text,
        "code" => FieldRole::Code,
        "search" => FieldRole::Search,
        "terminal" => FieldRole::Terminal,
        _ => FieldRole::Unknown,
    }
}

fn float_of_hex(text: &str) -> f64 {
    let (sign, text) = text.strip_prefix('-').map_or((1.0, text), |rest| (-1.0, rest));
    let body = text.strip_prefix("0x").expect("a hex float");
    let (mantissa, exponent) = body.split_once('p').expect("a hex float");
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = u64::from_str_radix(&format!("{whole}{fraction}"), 16).expect("hex digits");
    let exponent: i32 = exponent.parse::<i32>().expect("an exponent") - 4 * fraction.len().to::<i32>();
    sign * digits.to::<f64>() * 2_f64.powi(exponent)
}

#[test]
fn features_and_predictions_match_the_python_ones() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let golden: Value = serde_json::from_str(include_str!("data/baseline-features.json")).expect("the golden file");
    let model = ContextModel::load_artifact(&root.join("model/context_v3/baseline-context-v1.json"), &root.join("resources/models")).expect("the baseline model");
    for case in golden.as_array().expect("cases") {
        let values = case["case"].as_array().expect("a case");
        let text = |index: usize| values[index].as_str().expect("a text");
        let mut field = FieldContext::new(text(11), "f");
        field.before = text(3).to_owned();
        field.after = text(4).to_owned();
        field.role = role(text(5));
        let mut item = ContextEvidence::new(text(0), text(1), values[2].as_i64().expect("a group"), field);
        item.trigger = text(6).to_owned();
        item.baseline_convert = values[7].as_bool().expect("a flag");
        item.source_known = values[8].as_bool().expect("a flag");
        item.target_known = values[9].as_bool().expect("a flag");
        item.score_delta = values[10].as_f64().expect("a number");
        let item = item.finished();
        let features = extract_baseline_features(&item);
        let expected = case["features"].as_array().expect("features");
        let wanted: Vec<&str> = expected.iter().filter_map(|pair| pair[0].as_str()).collect();
        let extra: Vec<&String> = features.keys().filter(|name| !wanted.contains(&name.as_str())).collect();
        let missing: Vec<&&str> = wanted.iter().filter(|name| !features.contains_key(**name)).collect();
        assert!(extra.is_empty() && missing.is_empty(), "{}: extra {extra:?}, missing {missing:?}", text(0));
        for ((name, value), pair) in features.iter().zip(expected) {
            assert_eq!(Some(name.as_str()), pair[0].as_str(), "the order of the features of {}", text(0));
            assert_eq!(value.to_bits(), float_of_hex(pair[1].as_str().expect("a hex float")).to_bits(), "{name} of {}", text(0));
        }
        let prediction = model.predict(&item);
        assert_eq!(Some(prediction.action.as_str()), case["action"].as_str(), "{}", text(0));
        assert_eq!(Some(prediction.supported), case["supported"].as_bool(), "{}", text(0));
        for (value, wanted) in prediction.probabilities.iter().zip(case["probabilities"].as_array().expect("probabilities")) {
            assert_eq!(value.to_bits(), float_of_hex(wanted.as_str().expect("a hex float")).to_bits(), "{}", text(0));
        }
    }
}
