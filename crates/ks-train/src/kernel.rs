//! Training-only sparse AdaGrad softmax over packed rows.
use std::collections::HashMap;

use crate::{ACTIONS, LOG_LOSS_PROBABILITY_FLOOR};

/// Rows of named features as indices into a vocabulary; features outside it are left out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Packed {
    pub offsets: Vec<u64>,
    pub indices: Vec<u32>,
    pub values: Vec<f64>,
    pub labels: Vec<u8>,
    pub importance: Vec<f64>,
}

impl Packed {
    pub fn build<'a>(rows: impl IntoIterator<Item = (&'a [(String, f64)], u8, f64)>, names: &[String]) -> Result<Packed, String> {
        let vocabulary: HashMap<&str, u32> = names.iter().enumerate().map(|(index, name)| (name.as_str(), index as u32)).collect();
        let mut result = Packed { offsets: vec![0], ..Packed::default() };
        for (features, label, importance) in rows {
            if usize::from(label) >= ACTIONS || !importance.is_finite() || importance <= 0.0 {
                return Err("invalid training target or importance".to_owned());
            }
            for (name, value) in features {
                if !value.is_finite() {
                    return Err("non-finite feature".to_owned());
                }
                if let Some(index) = vocabulary.get(name.as_str()) {
                    result.indices.push(*index);
                    result.values.push(*value);
                }
            }
            result.offsets.push(result.indices.len() as u64);
            result.labels.push(label);
            result.importance.push(importance);
        }
        Ok(result)
    }

    pub fn rows(&self) -> usize {
        self.labels.len()
    }
}

fn probabilities(data: &Packed, row: usize, weights: &[f64], p: &mut [f64; ACTIONS]) {
    *p = [0.0; ACTIONS];
    for position in data.offsets[row] as usize..data.offsets[row + 1] as usize {
        let base = data.indices[position] as usize * ACTIONS;
        for k in 0..ACTIONS {
            p[k] += weights[base + k] * data.values[position];
        }
    }
    let mut maximum = p[0];
    for value in &p[1..] {
        if *value > maximum {
            maximum = *value;
        }
    }
    let mut total = 0.0;
    for value in p.iter_mut() {
        *value = (*value - maximum).exp();
        total += *value;
    }
    for value in p.iter_mut() {
        *value /= total;
    }
}

/// One pass over the rows in order: each row's gradient moves the weights before the next row is read.
pub fn epoch(data: &Packed, weights: &mut [f64], accumulators: &mut [f64], rate: f64) {
    let mut p = [0.0; ACTIONS];
    for row in 0..data.rows() {
        probabilities(data, row, weights, &mut p);
        for position in data.offsets[row] as usize..data.offsets[row + 1] as usize {
            let base = data.indices[position] as usize * ACTIONS;
            for (k, probability) in p.iter().enumerate() {
                let target = if k == usize::from(data.labels[row]) { 1.0 } else { 0.0 };
                let gradient = data.importance[row] * (probability - target) * data.values[position];
                accumulators[base + k] += gradient * gradient;
                weights[base + k] -= rate * gradient / accumulators[base + k].sqrt();
            }
        }
    }
}

pub fn predict(data: &Packed, weights: &[f64]) -> Vec<f64> {
    let mut output = vec![0.0; data.rows() * ACTIONS];
    let mut p = [0.0; ACTIONS];
    for row in 0..data.rows() {
        probabilities(data, row, weights, &mut p);
        output[row * ACTIONS..(row + 1) * ACTIONS].copy_from_slice(&p);
    }
    output
}

/// The importance-weighted log loss of `predictions` on `data`, summed over its rows.
pub fn log_loss(predictions: &[f64], data: &Packed) -> f64 {
    let terms: Vec<f64> = data
        .labels
        .iter()
        .enumerate()
        .map(|(row, label)| -data.importance[row] * predictions[row * ACTIONS + usize::from(*label)].max(LOG_LOSS_PROBABILITY_FLOOR).ln())
        .collect();
    ks_lang::py_sum(terms.iter().copied())
}

/// Stream frame-presence mass with constant summation state per feature: each frame contributes its sample weight once,
/// independently of feature magnitude or class importance.
#[derive(Default)]
pub struct FeatureMass {
    pub values: HashMap<String, f64>,
    errors: HashMap<String, f64>,
}

impl FeatureMass {
    pub fn add<'a>(&mut self, features: impl IntoIterator<Item = &'a str>, sample_weight: f64) -> Result<(), String> {
        if !sample_weight.is_finite() || sample_weight <= 0.0 {
            return Err("feature sample weight must be positive and finite".to_owned());
        }
        for name in features {
            let previous = self.values.get(name).copied().unwrap_or(0.0);
            let increment = sample_weight - self.errors.get(name).copied().unwrap_or(0.0);
            let total = previous + increment;
            if !total.is_finite() {
                return Err("feature mass overflow".to_owned());
            }
            self.errors.insert(name.to_owned(), (total - previous) - increment);
            self.values.insert(name.to_owned(), total);
        }
        Ok(())
    }
}
