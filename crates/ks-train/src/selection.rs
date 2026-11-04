//! Which features, which epoch and which threshold: chosen on development and calibration, never on a sealed test.
use std::cmp::Ordering;
use std::collections::HashMap;

use ks_text::py_round_ndigits;
use serde_json::{Value, json};

use crate::{ACTIONS, DETERMINISTIC_ROUNDING_DECIMALS, EPOCH_NET_BENEFIT_TOLERANCE, FEATURE_MASS_TOLERANCE, PROBABILITY_SUM_TOLERANCE};

/// Use sample mass, with absolute 1e-9 cutoff tolerance and stable ties: ranking rounds mass to nine decimal places, then
/// breaks ties by name. Returned columns retain the canonical alphabetical order.
pub fn select_features(masses: &HashMap<String, f64>, minimum: f64, maximum: usize) -> Result<Vec<String>, String> {
    if !minimum.is_finite() || minimum <= 0.0 {
        return Err("minimum feature mass must be positive and finite".to_owned());
    }
    if maximum == 0 {
        return Err("maximum features must be a positive integer".to_owned());
    }
    if masses.values().any(|value| !value.is_finite() || *value < 0.0) {
        return Err("invalid accumulated feature mass".to_owned());
    }
    let mut ranked: Vec<(f64, &String)> = masses
        .iter()
        .filter(|(_, value)| **value >= minimum - FEATURE_MASS_TOLERANCE)
        .map(|(name, value)| (py_round_ndigits(*value, DETERMINISTIC_ROUNDING_DECIMALS).unwrap_or(*value), name))
        .collect();
    ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal).then_with(|| ks_text::pystr::cmp(a.1, b.1)));
    ranked.truncate(maximum);
    let mut names: Vec<String> = ranked.into_iter().map(|(_, name)| name.clone()).collect();
    names.sort_by(|a, b| ks_text::pystr::cmp(a, b));
    Ok(names)
}

fn best_action(scores: &[f64]) -> usize {
    let mut best = 0;
    for (action, score) in scores.iter().enumerate().skip(1) {
        // `max(range(...), key=...)`: the first of equals stays.
        if *score > scores[best] {
            best = action;
        }
    }
    best
}

#[derive(Clone, Debug, PartialEq)]
pub struct EpochSelection {
    pub threshold: f64,
    pub minimum_recall: f64,
    pub pooled_recall: f64,
    pub loss: f64,
    pub by_profile: Vec<(String, Value)>,
    pub net_benefit: i64,
    pub minimum_net_benefit: i64,
}

impl EpochSelection {
    /// Worst profile first, then the pooled balance: a model is judged by what it nets.
    pub fn rank(&self) -> (f64, f64, f64, f64, f64) {
        (self.minimum_net_benefit as f64, self.net_benefit as f64, self.pooled_recall, -self.loss, -self.threshold)
    }
}

fn rank_order(a: &EpochSelection, b: &EpochSelection) -> Ordering {
    a.rank().partial_cmp(&b.rank()).unwrap_or(Ordering::Equal)
}

fn lower_bound(sorted: &[f64], value: f64) -> usize {
    sorted.partition_point(|item| *item < value)
}

/// Rank development operating points by what they net after the runtime support checks. An epoch qualifies when every
/// profile repairs more than it breaks by at least `minimum_net_benefit` conversions.
pub fn assess_epoch(predictions: &[(String, &[f64], &[u8])], thresholds: &[f64], loss: f64, minimum_net_benefit: i64) -> Result<Option<EpochSelection>, String> {
    if predictions.is_empty() || thresholds.is_empty() {
        return Err("epoch selection requires development profiles and thresholds".to_owned());
    }
    if !loss.is_finite() || loss < 0.0 {
        return Err("invalid development loss".to_owned());
    }
    if minimum_net_benefit < 0 {
        return Err("invalid net-benefit floor".to_owned());
    }
    if thresholds.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) {
        return Err("invalid development threshold".to_owned());
    }
    let mut observations = Vec::new();
    for (profile, values, labels) in predictions {
        if labels.is_empty() || values.len() != labels.len() * ACTIONS {
            return Err("invalid development prediction dimensions".to_owned());
        }
        let (mut positives, mut negatives, mut possible) = (Vec::new(), Vec::new(), 0i64);
        for (row, label) in labels.iter().enumerate() {
            let scores = &values[row * ACTIONS..(row + 1) * ACTIONS];
            if usize::from(*label) >= ACTIONS || scores.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) {
                return Err("invalid development labels or probabilities".to_owned());
            }
            if (ks_lang::py_sum(scores.iter().copied()) - 1.0).abs() > PROBABILITY_SUM_TOLERANCE {
                return Err("development probabilities are not normalized".to_owned());
            }
            possible += i64::from(*label == 1);
            if best_action(scores) == 1 {
                if *label == 1 { &mut positives } else { &mut negatives }.push(scores[1]);
            }
        }
        if possible == 0 {
            return Err("development profile has no conversion targets".to_owned());
        }
        positives.sort_by(f64::total_cmp);
        negatives.sort_by(f64::total_cmp);
        observations.push((profile.clone(), positives, negatives, possible, labels.len()));
    }
    let mut grid: Vec<f64> = thresholds.to_vec();
    grid.sort_by(f64::total_cmp);
    grid.dedup();
    let mut best: Option<EpochSelection> = None;
    for threshold in grid {
        let (mut by_profile, mut nets, mut recalls, mut pooled_true, mut pooled_possible) = (Vec::new(), Vec::new(), Vec::new(), 0i64, 0i64);
        for (profile, positives, negatives, possible, rows) in &observations {
            let correct = (positives.len() - lower_bound(positives, threshold)) as i64;
            let wrong = (negatives.len() - lower_bound(negatives, threshold)) as i64;
            let recall = correct as f64 / *possible as f64;
            by_profile.push((profile.clone(), json!({"rows": rows, "converted_correctly": correct, "false_conversions": wrong, "convert_rows": possible, "conversion_recall": recall})));
            nets.push(correct - wrong);
            recalls.push(recall);
            pooled_true += correct;
            pooled_possible += possible;
        }
        let minimum_net = nets.iter().copied().min().unwrap_or(0);
        if minimum_net < minimum_net_benefit {
            continue;
        }
        let candidate = EpochSelection {
            threshold,
            minimum_recall: recalls.iter().copied().fold(f64::INFINITY, f64::min),
            pooled_recall: pooled_true as f64 / pooled_possible as f64,
            loss,
            by_profile,
            net_benefit: nets.iter().sum(),
            minimum_net_benefit: minimum_net,
        };
        if best.as_ref().is_none_or(|known| rank_order(&candidate, known) == Ordering::Greater) {
            best = Some(candidate);
        }
    }
    Ok(best)
}

/// One epoch that qualified, with its weights.
pub struct Qualifying {
    pub epoch: usize,
    pub selection: EpochSelection,
    pub weights: Vec<f64>,
    pub loss: f64,
}

/// The epoch the heads keep: the latest whose development balance is within the tolerance of the best. The balance is
/// flat over most epochs; within the tolerance the difference is noise, and the latest epoch fitted the curricula furthest.
pub fn settled_epoch(qualifying: &[Qualifying]) -> Option<&Qualifying> {
    let mut best: Option<&Qualifying> = None;
    for entry in qualifying {
        // The best rank, the earliest epoch among equals.
        let better = best.is_none_or(|known| match rank_order(&entry.selection, &known.selection) {
            Ordering::Greater => true,
            Ordering::Equal => entry.epoch < known.epoch,
            Ordering::Less => false,
        });
        if better {
            best = Some(entry);
        }
    }
    let best = &best?.selection;
    qualifying
        .iter()
        .filter(|entry| entry.selection.minimum_net_benefit >= best.minimum_net_benefit - EPOCH_NET_BENEFIT_TOLERANCE && entry.selection.net_benefit >= best.net_benefit - EPOCH_NET_BENEFIT_TOLERANCE)
        .max_by_key(|entry| entry.epoch)
}

pub fn metrics(probabilities: &[f64], labels: &[u8], threshold: f64) -> Value {
    let (mut wrong, mut right, mut possible, mut correct) = (0i64, 0i64, 0i64, 0i64);
    for (index, label) in labels.iter().enumerate() {
        let scores = &probabilities[index * ACTIONS..(index + 1) * ACTIONS];
        let predicted = best_action(scores);
        let converted = predicted == 1 && scores[1] >= threshold;
        wrong += i64::from(converted && *label != 1);
        right += i64::from(converted && *label == 1);
        possible += i64::from(*label == 1);
        correct += i64::from(predicted == usize::from(*label));
    }
    let recall = if possible > 0 { right as f64 / possible as f64 } else { 0.0 };
    json!({"rows": labels.len(), "false_conversions": wrong, "converted_correctly": right, "convert_rows": possible, "conversion_recall": recall, "correct_classes": correct})
}

pub struct ThresholdOptions {
    pub minimum_net_benefit: i64,
    pub minimum_recall: f64,
    pub minimum_threshold: f64,
    pub net_benefit_tolerance: f64,
    pub maximum_threshold: f64,
    pub authored_floor: f64,
}

/// Choose the serving threshold on calibration by what it nets, in both profiles. Every profile must be ahead by
/// `minimum_net_benefit`; within the tolerance of the best balance the threshold with the fewest false conversions wins,
/// the lowest such threshold on a tie. `authored_floor` and `maximum_threshold` come from the cases the product promises:
/// calibration counts rows and cannot see either end.
pub fn choose_threshold(predictions: &[(String, &[f64], &[u8])], candidates: &[f64], options: &ThresholdOptions) -> Result<(f64, Value, bool), String> {
    if predictions.is_empty() || candidates.is_empty() {
        return Err("calibration requires profiles and threshold candidates".to_owned());
    }
    let floor_of_threshold = options.minimum_threshold.max(options.authored_floor);
    if !(floor_of_threshold <= options.maximum_threshold && options.maximum_threshold <= 1.0) {
        return Err("threshold ceiling must not fall below the floor it has to clear".to_owned());
    }
    let mut grid: Vec<f64> = candidates.iter().copied().filter(|threshold| (floor_of_threshold..=options.maximum_threshold).contains(threshold)).collect();
    grid.sort_by(f64::total_cmp);
    if grid.is_empty() {
        return Err("no threshold candidate reaches the development operating threshold".to_owned());
    }
    if !(0.0..1.0).contains(&options.net_benefit_tolerance) {
        return Err("net benefit tolerance must be a fraction below one".to_owned());
    }
    let mut qualifying: Vec<(i64, i64, i64, f64, Value)> = Vec::new();
    let mut report = Value::Null;
    for threshold in &grid {
        let mut by_profile = serde_json::Map::new();
        let (mut wrong, mut right, mut possible, mut rows, mut minimum_net, mut recall_ok) = (0i64, 0i64, 0i64, 0i64, i64::MAX, true);
        for (name, values, labels) in predictions {
            let row = metrics(values, labels, *threshold);
            let (w, r) = (row["false_conversions"].as_i64().unwrap_or(0), row["converted_correctly"].as_i64().unwrap_or(0));
            wrong += w;
            right += r;
            possible += row["convert_rows"].as_i64().unwrap_or(0);
            rows += row["rows"].as_i64().unwrap_or(0);
            minimum_net = minimum_net.min(r - w);
            recall_ok &= row["conversion_recall"].as_f64().unwrap_or(0.0) >= options.minimum_recall;
            by_profile.insert(name.clone(), row);
        }
        report = json!({"false_conversions": wrong, "conversion_recall": if possible > 0 { right as f64 / possible as f64 } else { 0.0 }, "converted_correctly": right, "convert_rows": possible, "net_benefit": right - wrong, "minimum_net_benefit": minimum_net, "rows": rows, "by_profile": by_profile});
        if minimum_net >= options.minimum_net_benefit && recall_ok {
            qualifying.push((minimum_net, right - wrong, wrong, *threshold, report.clone()));
        }
    }
    if qualifying.is_empty() {
        return Ok((grid[grid.len() - 1], report, false));
    }
    let ceiling = qualifying.iter().map(|row| (row.0, row.1)).max().unwrap_or((0, 0));
    let tolerance = 1.0 - options.net_benefit_tolerance;
    let floor = ((ceiling.0 as f64 * tolerance).floor() as i64, (ceiling.1 as f64 * tolerance).floor() as i64);
    // Both parts of the balance have to hold.
    let admissible: Vec<&(i64, i64, i64, f64, Value)> = qualifying.iter().filter(|row| row.0 >= floor.0 && row.1 >= floor.1).collect();
    let mut chosen = admissible[0];
    for row in &admissible[1..] {
        if (row.2, row.3) < (chosen.2, chosen.3) {
            chosen = row;
        }
    }
    let mut result = chosen.4.clone();
    result["net_benefit_ceiling"] = json!(ceiling.1);
    result["net_benefit_floor"] = json!(floor.1);
    result["admissible_thresholds"] = json!(admissible.iter().map(|row| row.3).collect::<Vec<_>>());
    Ok((chosen.3, result, true))
}
