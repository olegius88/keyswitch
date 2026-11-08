//! The choices made on development and calibration, over a few rows written out by hand: the features kept, the epoch
//! that qualifies and the serving threshold, and what each of them refuses.
#![expect(clippy::disallowed_macros, clippy::disallowed_methods, reason = "temporary: these tests still build and read their JSON as untyped trees; step 7 of the strict-json plan types them and removes this")]
use ks_train::selection::{EpochSelection, Qualifying, ThresholdOptions, assess_epoch, choose_threshold, metrics, select_features, settled_epoch};
use serde_json::json;
use std::collections::HashMap;

/// The probabilities of a row: keep, convert and the two others.
const CONVERT: [f64; 4] = [0.1, 0.8, 0.05, 0.05];
const SURE: [f64; 4] = [0.02, 0.95, 0.02, 0.01];
const LOW: [f64; 4] = [0.3, 0.6, 0.05, 0.05];
const KEEP: [f64; 4] = [0.7, 0.2, 0.05, 0.05];
/// Keep and convert alike: the first of equals, keep, is the one chosen.
const TIED: [f64; 4] = [0.4, 0.4, 0.1, 0.1];

fn flat(rows: &[[f64; 4]]) -> Vec<f64> {
    rows.iter().flatten().copied().collect()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-12
}

#[test]
fn features_are_kept_by_their_mass_and_returned_in_their_order() {
    let masses: HashMap<String, f64> = [("b", 2.0), ("a", 2.0), ("c", 5.0), ("d", 0.5), ("e", 1.0)].into_iter().map(|(name, mass)| (name.to_owned(), mass)).collect();
    assert_eq!(select_features(&masses, 1.0, 3).expect("kept"), ["a", "b", "c"]);
    assert_eq!(select_features(&masses, 1.0, 10).expect("kept"), ["a", "b", "c", "e"]);
    for minimum in [f64::NAN, f64::INFINITY, 0.0] {
        assert!(select_features(&masses, minimum, 3).unwrap_err().contains("minimum feature mass"));
    }
    assert!(select_features(&masses, 1.0, 0).unwrap_err().contains("maximum features"));
    for bad in [f64::NAN, -1.0] {
        let mut masses = masses.clone();
        masses.insert("z".to_owned(), bad);
        assert!(select_features(&masses, 1.0, 3).unwrap_err().contains("invalid accumulated"));
    }
}

fn epoch_refused(predictions: &[(String, &[f64], &[u8])], thresholds: &[f64], loss: f64, floor: i64, words: &str) {
    let said = assess_epoch(predictions, thresholds, loss, floor).expect_err(words);
    assert!(said.contains(words), "{words}: {said}");
}

#[test]
fn an_epoch_qualifies_by_what_every_profile_nets() {
    let values = flat(&[CONVERT, SURE, SURE, LOW, LOW, KEEP, TIED]);
    let labels: [u8; 7] = [1, 1, 1, 0, 0, 0, 1];
    let other_values = flat(&[SURE, KEEP]);
    let other_labels: [u8; 2] = [1, 0];
    let predictions = [("one".to_owned(), values.as_slice(), labels.as_slice()), ("two".to_owned(), other_values.as_slice(), other_labels.as_slice())];
    // At 0.5 the first profile nets one; at 0.7 its two false conversions are gone and it nets three, the best; at 0.9 it
    // nets two; at 0.99 nothing converts.
    let best = assess_epoch(&predictions, &[0.5, 0.7, 0.9, 0.9, 0.99], 0.25, 1).expect("assessed").expect("an epoch that qualifies");
    assert!(close(best.threshold, 0.7), "{best:?}");
    assert_eq!((best.net_benefit, best.minimum_net_benefit), (4, 1));
    assert!(best.rank() > EpochSelection { threshold: 0.0, minimum_recall: 0.0, pooled_recall: 0.0, loss: 1.0, by_profile: Vec::new(), net_benefit: 0, minimum_net_benefit: 0 }.rank());
    assert_eq!(assess_epoch(&predictions, &[0.99], 0.25, 1).expect("assessed"), None);
    assert!(assess_epoch(&predictions, &[0.5], 0.25, 5).expect("assessed").is_none());
    // What it refuses.
    let (none, no_labels): (&[f64], &[u8]) = (&[], &[]);
    let one = flat(&[CONVERT]);
    epoch_refused(&[], &[0.5], 0.1, 1, "requires development profiles");
    epoch_refused(&predictions, &[], 0.1, 1, "requires development profiles");
    epoch_refused(&predictions, &[0.5], f64::NAN, 1, "invalid development loss");
    epoch_refused(&predictions, &[0.5], -1.0, 1, "invalid development loss");
    epoch_refused(&predictions, &[0.5], 0.1, -1, "invalid net-benefit floor");
    epoch_refused(&predictions, &[f64::NAN], 0.1, 1, "invalid development threshold");
    epoch_refused(&predictions, &[1.5], 0.1, 1, "invalid development threshold");
    epoch_refused(&[("x".to_owned(), none, no_labels)], &[0.5], 0.1, 1, "dimensions");
    epoch_refused(&[("x".to_owned(), one.as_slice(), [1, 1].as_slice())], &[0.5], 0.1, 1, "dimensions");
    epoch_refused(&[("x".to_owned(), one.as_slice(), [4].as_slice())], &[0.5], 0.1, 1, "invalid development labels");
    epoch_refused(&[("x".to_owned(), [0.1, f64::NAN, 0.0, 0.0].as_slice(), [1].as_slice())], &[0.5], 0.1, 1, "invalid development labels");
    epoch_refused(&[("x".to_owned(), [0.0, 1.5, 0.0, 0.0].as_slice(), [1].as_slice())], &[0.5], 0.1, 1, "invalid development labels");
    epoch_refused(&[("x".to_owned(), [0.1, 0.1, 0.1, 0.1].as_slice(), [1].as_slice())], &[0.5], 0.1, 1, "not normalized");
    epoch_refused(&[("x".to_owned(), one.as_slice(), [0].as_slice())], &[0.5], 0.1, 1, "no conversion targets");
}

fn qualifying(epoch: u8, net: i64, minimum: i64) -> Qualifying {
    let selection = EpochSelection { threshold: 0.5, minimum_recall: 0.5, pooled_recall: 0.5, loss: 0.1, by_profile: Vec::new(), net_benefit: net, minimum_net_benefit: minimum };
    Qualifying { epoch: usize::from(epoch), selection, weights: vec![f64::from(epoch)], loss: 0.1 }
}

#[test]
fn the_epoch_kept_is_the_latest_within_the_tolerance_of_the_best() {
    assert!(settled_epoch(&[]).is_none());
    // The best is epoch 2, the earliest of three equals. Within two of it in both parts: epochs 2, 3, 4 and 7; epoch 1
    // and 5 fall short in the worst profile, epoch 6 in the balance.
    let entries = [qualifying(1, 4, 2), qualifying(3, 10, 5), qualifying(2, 10, 5), qualifying(7, 10, 5), qualifying(4, 9, 4), qualifying(6, 7, 5), qualifying(5, 10, 2)];
    assert_eq!(settled_epoch(&entries).expect("an epoch").epoch, 7);
    assert_eq!(settled_epoch(&entries[..1]).expect("an epoch").epoch, 1);
}

#[test]
fn the_metrics_of_a_split_count_what_converts() {
    let values = flat(&[CONVERT, KEEP, CONVERT]);
    let row = metrics(&values, &[1, 0, 0], 0.5);
    assert_eq!((row["converted_correctly"].clone(), row["false_conversions"].clone(), row["correct_classes"].clone()), (json!(1), json!(1), json!(2)));
    assert_eq!(metrics(&values, &[1, 0, 0], 0.9)["converted_correctly"], json!(0));
    assert_eq!(metrics(&values, &[0, 0, 0], 0.5)["conversion_recall"], json!(0.0));
}

fn options(minimum_net_benefit: i64) -> ThresholdOptions {
    ThresholdOptions { minimum_net_benefit, minimum_recall: 0.0, minimum_threshold: 0.3, net_benefit_tolerance: 0.5, maximum_threshold: 0.99, authored_floor: 0.2 }
}

fn threshold_refused(predictions: &[(String, &[f64], &[u8])], candidates: &[f64], options: &ThresholdOptions, words: &str) {
    let said = choose_threshold(predictions, candidates, options).expect_err(words);
    assert!(said.contains(words), "{words}: {said}");
}

#[test]
fn the_serving_threshold_is_chosen_on_calibration_by_what_it_nets() {
    let values = flat(&[SURE, CONVERT, CONVERT, KEEP, SURE]);
    let labels: [u8; 5] = [1, 1, 0, 0, 1];
    let predictions = [("one".to_owned(), values.as_slice(), labels.as_slice())];
    // Of the candidates between the floor and the ceiling, at 0.5 three right and one false, at 0.9 two right and none
    // false: the same balance, and the fewer false conversions win.
    let (threshold, report, passed) = choose_threshold(&predictions, &[0.5, 0.9, 0.1, 0.995], &options(1)).expect("chosen");
    assert!(passed && close(threshold, 0.9), "{threshold} {report}");
    assert_eq!(report["admissible_thresholds"], json!([0.5, 0.9]));
    // Nothing nets what is asked of it: the highest candidate, not passed.
    let (threshold, _, passed) = choose_threshold(&predictions, &[0.5, 0.9], &options(5)).expect("chosen");
    assert!(!passed && close(threshold, 0.9));
    // A recall floor only the lower threshold reaches.
    let (threshold, _, passed) = choose_threshold(&predictions, &[0.5, 0.9], &ThresholdOptions { minimum_recall: 0.99, ..options(1) }).expect("chosen");
    assert!(passed && close(threshold, 0.5));
    // Equal in everything, the lower threshold stays.
    let sure = flat(&[SURE, SURE]);
    let (threshold, _, _) = choose_threshold(&[("one".to_owned(), sure.as_slice(), [1, 1].as_slice())], &[0.9, 0.5], &options(1)).expect("chosen");
    assert!(close(threshold, 0.5));
    // The best balance in its worst profile at 0.5; at 0.9 one profile nets more and the other falls below half of the
    // best worst profile, so 0.9 is not admissible.
    let (first, second) = (flat(&[CONVERT, CONVERT, CONVERT, CONVERT, SURE]), flat(&[[SURE; 11].as_slice(), [LOW; 6].as_slice()].concat()));
    let (first_labels, second_labels): ([u8; 5], Vec<u8>) = ([1; 5], [vec![1; 11], vec![0; 6]].concat());
    let profiles = [("one".to_owned(), first.as_slice(), first_labels.as_slice()), ("two".to_owned(), second.as_slice(), second_labels.as_slice())];
    let (threshold, report, _) = choose_threshold(&profiles, &[0.5, 0.9], &options(1)).expect("chosen");
    assert!(close(threshold, 0.5) && report["admissible_thresholds"] == json!([0.5]), "{threshold} {report}");
    // A calibration without a row to convert recalls nothing.
    let keep = flat(&[KEEP]);
    let (_, report, _) = choose_threshold(&[("one".to_owned(), keep.as_slice(), [0].as_slice())], &[0.5], &options(0)).expect("chosen");
    assert_eq!(report["conversion_recall"], json!(0.0));
    // What it refuses.
    threshold_refused(&[], &[0.5], &options(1), "requires profiles");
    threshold_refused(&predictions, &[], &options(1), "requires profiles");
    threshold_refused(&predictions, &[0.5], &ThresholdOptions { authored_floor: 0.999, ..options(1) }, "ceiling must not fall below");
    threshold_refused(&predictions, &[0.5], &ThresholdOptions { maximum_threshold: 1.5, ..options(1) }, "ceiling must not fall below");
    threshold_refused(&predictions, &[0.1], &options(1), "no threshold candidate");
    threshold_refused(&predictions, &[0.5], &ThresholdOptions { net_benefit_tolerance: 1.0, ..options(1) }, "tolerance");
}
