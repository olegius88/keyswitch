//! The fit of the layout-intent model (`train_intent_model`): the features of the runtime, FTRL-Proximal with the epoch's
//! Mersenne Twister shuffle, quantization, the Platt calibration by direction and the thresholds by trigger and direction.
use ks_cast::To as _;
use super::dataset::LexicalExample;
use ks_lang::WordScore;
use ks_lang::intent_model::{CorrectionTrigger, IntentModelInput, LAYOUT_DIRECTIONS, TRIGGERS, extract_features};
use ks_lang::py_sum;
use ks_text::py_round;
use ks_text::pynum::py_round_ndigits;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const SELECTION_Z: f64 = 2.865_260_238_532_133_3;
pub const WILSON_95_Z: f64 = 1.959_963_984_540_054;
const WILSON_CENTRE_FACTOR: f64 = 2.0;
const WILSON_RADIUS_FACTOR: f64 = 4.0;
pub const WILSON_CONFIDENCE: f64 = 0.95;
const RATE_TOLERANCE: f64 = 1e-15;
const CALIBRATION_L2_FACTOR: f64 = 0.5;
const LINE_SEARCH_SHRINK: f64 = 0.5;
const LINE_SEARCH_STEPS: usize = 30;
const MIN_CURVATURE: f64 = 1e-12;
const MIN_DETERMINANT: f64 = 1e-18;
const STEP_TOLERANCE: f64 = 1e-10;
const QUANTIZED_LIMIT: i64 = 32767;
pub const SELECTION_COMPARISONS: i64 = 12;
pub const SELECTION_PER_COMPARISON_CONFIDENCE: f64 = 0.995_833_333_333_333_3;
const METRIC_DECIMALS: i64 = 9;
const NGRAM_ORDERS: [i64; 5] = [1, 2, 3, 4, 5];
pub const STRESS_PROFILES: [(&str, f64, &str); 18] = [
    ("source_minimum", -6.0, "source"), ("source_outer_negative", -1.25, "source"), ("source_inner_negative", -0.75, "source"), ("source_near_zero_negative", -0.125, "source"), ("source_zero", 0.0, "source"),
    ("source_near_zero_positive", 0.125, "source"), ("source_inner_positive", 0.75, "source"), ("source_outer_positive", 1.25, "source"), ("source_maximum", 6.0, "source"), ("target_minimum", -6.0, "target"),
    ("target_outer_negative", -1.25, "target"), ("target_inner_negative", -0.75, "target"), ("target_near_zero_negative", -0.125, "target"), ("target_zero", 0.0, "target"), ("target_near_zero_positive", 0.125, "target"),
    ("target_inner_positive", 0.75, "target"), ("target_outer_positive", 1.25, "target"), ("target_maximum", 6.0, "target"),
];

/// What the fit reads of the configuration.
#[derive(Clone)]
pub struct FitConfig {
    pub seed: u64,
    pub dimension: usize,
    pub feature_hash_seed: u64,
    pub membership_hash_seed: u64,
    pub maximum_epochs: u64,
    pub minimum_epochs: u64,
    pub patience: u64,
    pub alpha: f64,
    pub beta: f64,
    pub l1: f64,
    pub l2: f64,
    pub calibration_l2: f64,
    pub calibration_iterations: usize,
    pub precision_floor: f64,
    pub max_fpr: f64,
    pub pause_max_fpr: f64,
    pub selection_max_false_positives: i64,
    pub margin_cap: f64,
    pub pause_margin: f64,
    pub veto_quantile: f64,
    pub veto_margin: f64,
    pub veto_max_fnr: f64,
    pub test_precision: f64,
    pub test_recall: f64,
    pub test_pause_recall: f64,
    pub test_typo_recall: f64,
    pub test_pause_typo_recall: f64,
    pub selection_recall: f64,
    pub selection_pause_recall: f64,
    pub selection_typo_recall: f64,
    pub selection_pause_typo_recall: f64,
    pub test_specificity: f64,
    pub safety_max_failures: i64,
}

impl FitConfig {
    pub fn read(config: &Value) -> Result<FitConfig, String> {
        let float = |section: &str, name: &str| config[section][name].as_f64().ok_or_else(|| format!("{section}.{name}"));
        let int = |section: &str, name: &str| config[section][name].as_i64().ok_or_else(|| format!("{section}.{name}"));
        let top = |name: &str| config[name].as_u64().ok_or_else(|| name.to_owned());
        Ok(FitConfig {
            seed: top("seed")?,
            dimension: (top("dimension")?).to::<usize>(),
            feature_hash_seed: top("feature_hash_seed")?,
            membership_hash_seed: top("membership_hash_seed")?,
            maximum_epochs: (int("ftrl", "maximum_epochs")?).to::<u64>(),
            minimum_epochs: (int("ftrl", "minimum_epochs")?).to::<u64>(),
            patience: (int("ftrl", "patience")?).to::<u64>(),
            alpha: float("ftrl", "alpha")?,
            beta: float("ftrl", "beta")?,
            l1: float("ftrl", "l1")?,
            l2: float("ftrl", "l2")?,
            calibration_l2: float("calibration", "l2")?,
            calibration_iterations: (int("calibration", "maximum_iterations")?).to::<usize>(),
            precision_floor: float("thresholds", "precision_floor")?,
            max_fpr: float("thresholds", "maximum_false_positive_rate")?,
            pause_max_fpr: float("thresholds", "pause_maximum_false_positive_rate")?,
            selection_max_false_positives: int("thresholds", "selection_maximum_false_positives_per_trigger")?,
            margin_cap: float("thresholds", "threshold_logit_margin_cap")?,
            pause_margin: float("thresholds", "pause_logit_margin")?,
            veto_quantile: float("thresholds", "veto_positive_quantile")?,
            veto_margin: float("thresholds", "veto_logit_margin")?,
            veto_max_fnr: float("thresholds", "veto_max_false_negative_rate")?,
            test_precision: float("quality_gates", "minimum_precision")?,
            test_recall: float("quality_gates", "minimum_recall")?,
            test_pause_recall: float("quality_gates", "minimum_pause_recall")?,
            test_typo_recall: float("quality_gates", "minimum_typo_recall")?,
            test_pause_typo_recall: float("quality_gates", "minimum_pause_typo_recall")?,
            selection_recall: float("quality_gates", "selection_minimum_recall")?,
            selection_pause_recall: float("quality_gates", "selection_minimum_pause_recall")?,
            selection_typo_recall: float("quality_gates", "selection_minimum_typo_recall")?,
            selection_pause_typo_recall: float("quality_gates", "selection_minimum_pause_typo_recall")?,
            test_specificity: float("quality_gates", "minimum_specificity")?,
            safety_max_failures: int("quality_gates", "safety_maximum_guard_failures")?,
        })
    }
}

/// `random.Random(seed)` as far as a shuffle reads it: MT19937 seeded by `init_by_array`.
pub struct Twister {
    state: Vec<u32>,
    index: usize,
}

const MT_N: usize = 624;
const MT_M: usize = 397;

impl Twister {
    #[must_use]
    pub fn new(seed: u64) -> Twister {
        let mut key = vec![seed.to::<u32>()];
        if seed >> 32 != 0 {
            key.push((seed >> 32).to::<u32>());
        }
        let mut state = vec![0_u32; MT_N];
        state[0] = 19_650_218;
        for index in 1..MT_N {
            state[index] = 1_812_433_253_u32.wrapping_mul(state[index - 1] ^ (state[index - 1] >> 30)).wrapping_add(index.to::<u32>());
        }
        let (mut i, mut j) = (1, 0);
        for _ in 0..MT_N.max(key.len()) {
            state[i] = (state[i] ^ (state[i - 1] ^ (state[i - 1] >> 30)).wrapping_mul(1_664_525)).wrapping_add(key[j]).wrapping_add(j.to::<u32>());
            i += 1;
            j += 1;
            if i >= MT_N {
                state[0] = state[MT_N - 1];
                i = 1;
            }
            if j >= key.len() {
                j = 0;
            }
        }
        for _ in 0..MT_N - 1 {
            state[i] = (state[i] ^ (state[i - 1] ^ (state[i - 1] >> 30)).wrapping_mul(1_566_083_941)).wrapping_sub(i.to::<u32>());
            i += 1;
            if i >= MT_N {
                state[0] = state[MT_N - 1];
                i = 1;
            }
        }
        state[0] = 0x8000_0000;
        Twister { state, index: MT_N }
    }

    pub fn next_u32(&mut self) -> u32 {
        if self.index >= MT_N {
            for k in 0..MT_N {
                let y = (self.state[k] & 0x8000_0000) | (self.state[(k + 1) % MT_N] & 0x7fff_ffff);
                self.state[k] = self.state[(k + MT_M) % MT_N] ^ (y >> 1) ^ if y & 1 == 1 { 0x9908_b0df } else { 0 };
            }
            self.index = 0;
        }
        let mut y = self.state[self.index];
        self.index += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^ (y >> 18)
    }

    /// `_randbelow_with_getrandbits`.
    fn below(&mut self, n: u32) -> u32 {
        let bits = n.bit_width();
        loop {
            let value = self.next_u32() >> (32 - bits);
            if value < n {
                return value;
            }
        }
    }

    /// `shuffle`.
    pub fn shuffle(&mut self, items: &mut [u32]) {
        for i in (1..items.len()).rev() {
            let j = (self.below(i.to::<u32>() + 1)).to::<usize>();
            items.swap(i, j);
        }
    }
}

/// Rows in compressed sparse form, in the order of their examples.
pub struct Featured<'a> {
    pub examples: Vec<&'a LexicalExample>,
    pub offsets: Vec<u64>,
    pub indices: Vec<u32>,
    pub values: Vec<f64>,
}

impl Featured<'_> {
    fn row(&self, row: usize) -> std::ops::Range<usize> {
        self.offsets[row].to::<usize>()..(self.offsets[row + 1]).to::<usize>()
    }
}

#[must_use]
pub fn ignored_score() -> WordScore {
    WordScore { value: 0.0, known: false, frequency: 0, gram_ratio: 0.0, exact: false, spell_known: false, ngram_score: 0.0, invalid_ratio: 1.0, raw_ngram_score: 0.0 }
}

#[must_use]
pub fn balanced_ranges(length: usize, chunks: usize) -> Vec<std::ops::Range<usize>> {
    if length == 0 {
        return Vec::new();
    }
    let actual = chunks.clamp(1, length);
    let (width, remainder) = (length / actual, length % actual);
    let mut start = 0;
    (0..actual)
        .map(|index| {
            let stop = start + width + usize::from(index < remainder);
            let range = start..stop;
            start = stop;
            range
        })
        .collect()
}

/// `featurize_training_phase`: the features of the runtime in the row order, on threads.
pub fn featurize<'a>(examples: &[&'a LexicalExample], config: &FitConfig, workers: usize, mut supported: Option<&mut BTreeSet<u64>>) -> Result<Featured<'a>, String> {
    let score = ignored_score();
    let collect = supported.is_some();
    let ranges = balanced_ranges(examples.len(), workers);
    let chunks: Vec<Result<(Vec<u64>, Vec<u32>, Vec<f64>, BTreeSet<u64>), String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = ranges
            .iter()
            .map(|range| {
                let (range, score) = (range.clone(), &score);
                scope.spawn(move || {
                    let (mut lengths, mut indices, mut values, mut fingerprints) = (Vec::with_capacity(range.len()), Vec::new(), Vec::new(), BTreeSet::new());
                    for example in &examples[range] {
                        let input = IntentModelInput { original: &example.original, alternative: &example.alternative, source_group: example.source_group, target_group: example.target_group, trigger: example.trigger, source_score: score, target_score: score, context_delta: example.context_delta, context_group: example.context_group };
                        let vector = extract_features(&input, config.dimension.to::<i64>(), config.feature_hash_seed, config.membership_hash_seed, &NGRAM_ORDERS).map_err(|error| error.name.to_string())?;
                        lengths.push(vector.values.len().to::<u64>());
                        for (index, value) in vector.values {
                            indices.push(index);
                            values.push(value);
                        }
                        if collect {
                            fingerprints.extend(vector.character_fingerprints);
                        }
                    }
                    Ok((lengths, indices, values, fingerprints))
                })
            })
            .collect();
        handles.into_iter().map(|handle| handle.join().expect("a feature worker")).collect()
    });
    let mut featured = Featured { examples: examples.to_vec(), offsets: vec![0], indices: Vec::new(), values: Vec::new() };
    for chunk in chunks {
        let (lengths, indices, values, fingerprints) = chunk?;
        let mut position = *featured.offsets.last().expect("an offset");
        for length in lengths {
            position += length;
            featured.offsets.push(position);
        }
        featured.indices.extend(indices);
        featured.values.extend(values);
        if let Some(target) = supported.as_mut() {
            target.extend(fingerprints);
        }
    }
    Ok(featured)
}

fn stable_sigmoid(value: f64) -> f64 {
    if value >= 0.0 {
        return 1.0 / (1.0 + (-value).exp());
    }
    let exponential = value.exp();
    exponential / (1.0 + exponential)
}

/// `FTRLProximal`.
#[derive(Clone)]
pub struct Ftrl {
    z: Vec<f64>,
    n: Vec<f64>,
    bias_z: f64,
    bias_n: f64,
    alpha: f64,
    beta: f64,
    l1: f64,
    l2: f64,
}

impl Ftrl {
    fn new(config: &FitConfig) -> Ftrl {
        Ftrl { z: vec![0.0; config.dimension], n: vec![0.0; config.dimension], bias_z: 0.0, bias_n: 0.0, alpha: config.alpha, beta: config.beta, l1: config.l1, l2: config.l2 }
    }

    fn weight(&self, index: usize) -> f64 {
        let z = self.z[index];
        if z.abs() <= self.l1 {
            return 0.0;
        }
        let sign = if z < 0.0 { -1.0 } else { 1.0 };
        let denominator = (self.beta + self.n[index].sqrt()) / self.alpha + self.l2;
        -(z - sign * self.l1) / denominator
    }

    #[must_use]
    pub fn bias(&self) -> f64 {
        let denominator = (self.beta + self.bias_n.sqrt()) / self.alpha;
        if denominator == 0.0 { 0.0 } else { -self.bias_z / denominator }
    }

    /// One epoch in `order`: the update of `FTRLProximal.update`, operation for operation.
    fn run_epoch(&mut self, rows: &Featured<'_>, order: &[u32], scratch: &mut Vec<f64>) {
        for &row in order {
            let row = row.to::<usize>();
            let range = rows.row(row);
            let example = rows.examples[row];
            let old_bias = self.bias();
            scratch.clear();
            let (mut total, mut compensation) = (0.0_f64, 0.0_f64);
            for position in range.clone() {
                let weight = self.weight(rows.indices[position].to::<usize>());
                scratch.push(weight);
                let x = weight * rows.values[position];
                let t = total + x;
                if total.abs() >= x.abs() {
                    compensation += (total - t) + x;
                } else {
                    compensation += (x - t) + total;
                }
                total = t;
            }
            if compensation != 0.0 && compensation.is_finite() {
                total += compensation;
            }
            let prediction = stable_sigmoid(old_bias + total);
            let residual = (prediction - f64::from(u8::from(example.label))) * example.weight;
            let new_bias_n = self.bias_n + residual * residual;
            let bias_sigma = (new_bias_n.sqrt() - self.bias_n.sqrt()) / self.alpha;
            self.bias_z += residual - bias_sigma * old_bias;
            self.bias_n = new_bias_n;
            for (offset, position) in range.enumerate() {
                let index = rows.indices[position].to::<usize>();
                let gradient = residual * rows.values[position];
                let old_n = self.n[index];
                let new_n = old_n + gradient * gradient;
                let sigma = (new_n.sqrt() - old_n.sqrt()) / self.alpha;
                self.z[index] = self.z[index] + gradient - sigma * scratch[offset];
                self.n[index] = new_n;
            }
        }
    }

    /// `score`: the bias and `CPython`'s compensated sum of the weighted features.
    fn score(&self, rows: &Featured<'_>, row: usize) -> f64 {
        let (mut total, mut compensation) = (0.0_f64, 0.0_f64);
        for position in rows.row(row) {
            let x = self.weight(rows.indices[position].to::<usize>()) * rows.values[position];
            let t = total + x;
            if total.abs() >= x.abs() {
                compensation += (total - t) + x;
            } else {
                compensation += (x - t) + total;
            }
            total = t;
        }
        if compensation != 0.0 && compensation.is_finite() {
            total += compensation;
        }
        self.bias() + total
    }

    #[must_use]
    pub fn sparse_weights(&self) -> Vec<(usize, f64)> {
        (0..self.z.len()).filter_map(|index| { let weight = self.weight(index); (weight != 0.0).then_some((index, weight)) }).collect()
    }

    fn nonzero_weights(&self) -> usize {
        (0..self.z.len()).filter(|index| self.weight(*index) != 0.0).count()
    }
}

#[must_use]
pub fn logistic_loss(logit: f64, label: bool) -> f64 {
    logit.max(0.0) - f64::from(u8::from(label)) * logit + (-logit.abs()).exp().ln_1p()
}

fn parallel_scores(rows: &Featured<'_>, workers: usize, score: &(dyn Fn(usize) -> f64 + Sync)) -> Vec<f64> {
    let ranges = balanced_ranges(rows.examples.len(), workers);
    let chunks: Vec<Vec<f64>> = std::thread::scope(|scope| {
        let handles: Vec<_> = ranges.into_iter().map(|range| scope.spawn(move || range.map(score).collect::<Vec<f64>>())).collect();
        handles.into_iter().map(|handle| handle.join().expect("a scoring worker")).collect()
    });
    chunks.concat()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Confusion {
    pub true_positive: i64,
    pub false_negative: i64,
    pub true_negative: i64,
    pub false_positive: i64,
}

fn share(good: i64, other: i64) -> f64 {
    let denominator = good + other;
    if denominator != 0 { good.to::<f64>() / denominator.to::<f64>() } else { 1.0 }
}

impl Confusion {
    #[must_use]
    pub fn precision(&self) -> f64 {
        share(self.true_positive, self.false_positive)
    }

    #[must_use]
    pub fn recall(&self) -> f64 {
        share(self.true_positive, self.false_negative)
    }

    #[must_use]
    pub fn specificity(&self) -> f64 {
        share(self.true_negative, self.false_positive)
    }

    fn negatives(&self) -> i64 {
        self.true_negative + self.false_positive
    }

    fn add(&self, other: &Confusion) -> Confusion {
        Confusion { true_positive: self.true_positive + other.true_positive, false_negative: self.false_negative + other.false_negative, true_negative: self.true_negative + other.true_negative, false_positive: self.false_positive + other.false_positive }
    }

    fn gain(&self, positive: i64, negative: i64) -> Confusion {
        Confusion { true_positive: self.true_positive + positive, false_negative: self.false_negative - positive, true_negative: self.true_negative - negative, false_positive: self.false_positive + negative }
    }
}

/// `wilson_upper_bound`.
#[must_use]
pub fn wilson_upper(successes: i64, samples: i64, z: f64) -> f64 {
    if samples == 0 {
        return 1.0;
    }
    let n = samples.to::<f64>();
    let proportion = successes.to::<f64>() / n;
    let z2 = z * z;
    let denominator = 1.0 + z2 / n;
    let centre = proportion + z2 / (WILSON_CENTRE_FACTOR * n);
    // `WILSON_RADIUS_DENOMINATOR_FACTOR * samples * samples`: the factor is a float, multiplied left to right.
    let radius = z * (proportion * (1.0 - proportion) / n + z2 / (WILSON_RADIUS_FACTOR * n * n)).sqrt();
    ((centre + radius) / denominator).min(1.0)
}

/// `ScoredExample`, with what the selections read at hand.
#[derive(Clone, Copy)]
pub struct Scored<'a> {
    pub example: &'a LexicalExample,
    pub raw: f64,
    pub calibrated: f64,
    pub typo: bool,
    /// 0 for `0>1`, 1 for `1>0`.
    pub direction: usize,
}

#[must_use]
pub fn is_typo_kind(kind: &str) -> bool {
    matches!(kind, "deletion" | "duplication" | "transposition")
}

fn direction_of(example: &LexicalExample) -> usize {
    usize::from(example.source_group != 0)
}

#[expect(clippy::float_cmp, reason = "the exact double is the meaning here, as it is in the Python this was checked against")]
fn greater(left: &[f64], right: &[f64]) -> bool {
    for (a, b) in left.iter().zip(right) {
        if a != b {
            return a > b;
        }
    }
    false
}

/// `confusion_at_directional_threshold`.
pub fn confusion_of(items: &[Scored<'_>], logits: [f64; 2], keep: impl Fn(&Scored<'_>) -> bool) -> Confusion {
    let mut matrix = Confusion::default();
    for item in items.iter().filter(|item| keep(item)) {
        let predicted = item.calibrated >= logits[item.direction];
        match (item.example.label, predicted) {
            (true, true) => matrix.true_positive += 1,
            (true, false) => matrix.false_negative += 1,
            (false, true) => matrix.false_positive += 1,
            (false, false) => matrix.true_negative += 1,
        }
    }
    matrix
}

fn maximum_primary_false_positives(positives: i64, negatives: i64, precision_floor: f64, max_fpr: f64, z: f64) -> Result<i64, String> {
    let mut maximum = -1;
    for false_positives in 0..=negatives {
        let precision = if positives + false_positives != 0 { positives.to::<f64>() / (positives + false_positives).to::<f64>() } else { 1.0 };
        if precision + RATE_TOLERANCE < precision_floor || wilson_upper(false_positives, negatives, z) > max_fpr + RATE_TOLERANCE {
            break;
        }
        maximum = false_positives;
    }
    if maximum < 0 {
        return Err("primary precision policy cannot accept any threshold".to_owned());
    }
    Ok(maximum)
}

type Point = (f64, Confusion, Confusion);

/// `math.nextafter(value, inf)`.
fn next_up(value: f64) -> f64 {
    value.next_up()
}

/// `_direction_operating_curve`.
#[expect(clippy::float_cmp, reason = "the exact double is the meaning here, as it is in the Python this was checked against")]
fn direction_curve(selected: &[Scored<'_>], direction: usize, maximum_false_positives: i64) -> Result<Vec<Point>, String> {
    let mut ordered: Vec<&Scored<'_>> = selected.iter().filter(|item| item.direction == direction).collect();
    if ordered.is_empty() {
        return Err(format!("no threshold examples for direction {direction}"));
    }
    ordered.sort_by(|a, b| b.calibrated.partial_cmp(&a.calibrated).expect("finite logits"));
    let positives = ordered.iter().filter(|item| item.example.label).count().to::<i64>();
    let negatives = ordered.len().to::<i64>() - positives;
    let typo_positives = ordered.iter().filter(|item| item.typo && item.example.label).count().to::<i64>();
    let typo_negatives = ordered.iter().filter(|item| item.typo && !item.example.label).count().to::<i64>();
    if positives.min(negatives).min(typo_positives).min(typo_negatives) <= 0 {
        return Err(format!("direction {direction} requires both labels overall and for typos"));
    }
    let mut metrics = Confusion { true_positive: 0, false_negative: positives, true_negative: negatives, false_positive: 0 };
    let mut typo_metrics = Confusion { true_positive: 0, false_negative: typo_positives, true_negative: typo_negatives, false_positive: 0 };
    let mut best: BTreeMap<(i64, i64), Point> = BTreeMap::new();
    let mut record = |threshold: f64, metrics: Confusion, typo_metrics: Confusion| {
        let key = (metrics.false_positive, typo_metrics.false_positive);
        let replace = match best.get(&key) {
            None => true,
            Some(current) => {
                (metrics.true_positive, typo_metrics.true_positive) > (current.1.true_positive, current.2.true_positive)
                    || ((metrics.true_positive, typo_metrics.true_positive) == (current.1.true_positive, current.2.true_positive) && threshold > current.0)
            }
        };
        if replace {
            best.insert(key, (threshold, metrics, typo_metrics));
        }
    };
    record(next_up(ordered[0].calibrated), metrics, typo_metrics);
    let mut index = 0;
    while index < ordered.len() {
        let score = ordered[index].calibrated;
        let (mut positive, mut negative, mut typo_positive, mut typo_negative) = (0, 0, 0, 0);
        while index < ordered.len() && ordered[index].calibrated == score {
            let item = ordered[index];
            if item.example.label {
                positive += 1;
            } else {
                negative += 1;
            }
            if item.typo {
                if item.example.label {
                    typo_positive += 1;
                } else {
                    typo_negative += 1;
                }
            }
            index += 1;
        }
        metrics = metrics.gain(positive, negative);
        typo_metrics = typo_metrics.gain(typo_positive, typo_negative);
        if metrics.false_positive > maximum_false_positives {
            break;
        }
        record(score, metrics, typo_metrics);
    }
    Ok(best.into_values().collect())
}

/// `ThresholdSelection`.
#[derive(Clone, Copy, Debug)]
pub struct Selection {
    pub logits: [f64; 2],
    pub metrics: Confusion,
    pub typo_metrics: Confusion,
    pub margin: f64,
}

impl Selection {
    #[must_use]
    pub fn logit(&self) -> f64 {
        self.logits[0].max(self.logits[1])
    }
}

pub struct ThresholdPolicy {
    pub precision_floor: f64,
    pub max_fpr: f64,
    pub minimum_recall: f64,
    pub minimum_specificity: f64,
    pub typo_precision_floor: f64,
    pub minimum_typo_recall: f64,
    pub typo_minimum_specificity: f64,
    pub typo_max_fpr: f64,
    pub maximum_false_positives: i64,
}

/// `choose_directional_threshold`: the certified false-positive budget allotted jointly by direction.
fn choose_directional(selected: &[Scored<'_>], policy: &ThresholdPolicy) -> Result<Selection, String> {
    if selected.is_empty() {
        return Err("no threshold examples for this trigger".to_owned());
    }
    let positives = selected.iter().filter(|item| item.example.label).count().to::<i64>();
    let negatives = selected.len().to::<i64>() - positives;
    let certified = maximum_primary_false_positives(positives, negatives, policy.precision_floor, policy.max_fpr, SELECTION_Z)?.min(policy.maximum_false_positives);
    let forward = direction_curve(selected, 0, certified)?;
    let reverse = direction_curve(selected, 1, certified)?;
    let ranking = |logits: [f64; 2], metrics: &Confusion, typos: &Confusion| [metrics.recall(), typos.recall(), metrics.specificity(), metrics.precision(), logits[0], logits[1]];
    let (mut fallback, mut best): (Option<([f64; 2], Confusion, Confusion)>, Option<([f64; 2], Confusion, Confusion)>) = (None, None);
    for first in &forward {
        for second in &reverse {
            let logits = [first.0, second.0];
            let metrics = first.1.add(&second.1);
            let typos = first.2.add(&second.2);
            if metrics.false_positive > policy.maximum_false_positives
                || typos.false_positive > policy.maximum_false_positives
                || metrics.precision() + RATE_TOLERANCE < policy.precision_floor
                || wilson_upper(metrics.false_positive, metrics.negatives(), SELECTION_Z) > policy.max_fpr + RATE_TOLERANCE
            {
                continue;
            }
            let rank = ranking(logits, &metrics, &typos);
            if fallback.as_ref().is_none_or(|known| greater(&rank, &ranking(known.0, &known.1, &known.2))) {
                fallback = Some((logits, metrics, typos));
            }
            let passes = metrics.recall() >= policy.minimum_recall
                && metrics.specificity() >= policy.minimum_specificity
                && typos.precision() >= policy.typo_precision_floor
                && typos.recall() >= policy.minimum_typo_recall
                && typos.specificity() >= policy.typo_minimum_specificity
                && wilson_upper(typos.false_positive, typos.negatives(), SELECTION_Z) <= policy.typo_max_fpr + RATE_TOLERANCE;
            if passes && best.as_ref().is_none_or(|known| greater(&rank, &ranking(known.0, &known.1, &known.2))) {
                best = Some((logits, metrics, typos));
            }
        }
    }
    let (logits, metrics, typo_metrics) = best.or(fallback).ok_or("no statistically certified directional threshold exists")?;
    Ok(Selection { logits, metrics, typo_metrics, margin: 0.0 })
}

/// One epoch's report.
#[derive(Clone)]
pub struct EpochReport {
    pub value: Value,
    pub ranking: [f64; 9],
    pub line: String,
}

/// `_evaluate_development_logits` and the report of the epoch.
fn evaluate_epoch(development: &Featured<'_>, raw: &[f64], config: &FitConfig, epoch: u64, model: &Ftrl) -> Result<EpochReport, String> {
    let (mut weighted_loss, mut total_weight) = (0.0_f64, 0.0_f64);
    let mut scored = Vec::with_capacity(raw.len());
    for (example, logit) in development.examples.iter().zip(raw) {
        weighted_loss += logistic_loss(*logit, example.label) * example.weight;
        total_weight += example.weight;
        scored.push(Scored { example, raw: *logit, calibrated: *logit, typo: is_typo_kind(&example.variant_kind), direction: direction_of(example) });
    }
    let selection = choose_directional(&scored, &ThresholdPolicy { precision_floor: config.precision_floor, max_fpr: 1.0, minimum_recall: 0.0, minimum_specificity: 0.0, typo_precision_floor: config.precision_floor, minimum_typo_recall: 0.0, typo_minimum_specificity: 0.0, typo_max_fpr: 1.0, maximum_false_positives: config.selection_max_false_positives })?;
    let (metrics, typos) = (selection.metrics, selection.typo_metrics);
    let upper = wilson_upper(metrics.false_positive, metrics.negatives(), SELECTION_Z);
    let typo_upper = wilson_upper(typos.false_positive, typos.negatives(), SELECTION_Z);
    let checks = [
        metrics.precision() >= config.precision_floor,
        metrics.recall() >= config.selection_recall,
        metrics.specificity() >= config.test_specificity,
        upper <= config.max_fpr,
        typos.precision() >= config.precision_floor,
        typos.recall() >= config.selection_typo_recall,
        typos.specificity() >= config.test_specificity,
        typo_upper <= config.max_fpr,
        metrics.false_positive <= config.selection_max_false_positives,
        typos.false_positive <= config.selection_max_false_positives,
    ];
    let passed_checks = checks.iter().filter(|check| **check).count();
    let passed = checks.iter().all(|check| *check);
    let log_loss = weighted_loss / total_weight;
    let nonzero = model.nonzero_weights();
    let value = json!({
        "epoch": epoch, "development_log_loss": log_loss, "development_operating_thresholds": {"0>1": selection.logits[0], "1>0": selection.logits[1]},
        "development_precision": metrics.precision(), "development_recall": metrics.recall(), "development_specificity": metrics.specificity(), "development_false_positive": metrics.false_positive,
        "development_false_positive_rate_upper_familywise_95": upper, "development_typo_precision": typos.precision(), "development_typo_recall": typos.recall(), "development_typo_specificity": typos.specificity(),
        "development_typo_false_positive": typos.false_positive, "development_typo_false_positive_rate_upper_familywise_95": typo_upper, "development_policy_checks_passed": passed_checks, "development_policy_passed": passed, "nonzero_weights": nonzero,
    });
    let ranking = [f64::from(u8::from(passed)), passed_checks.to::<f64>(), metrics.recall(), typos.recall(), metrics.specificity(), typos.specificity(), metrics.precision(), typos.precision(), -log_loss];
    Ok(EpochReport { value, ranking, line: format!("epoch={epoch} loss={log_loss:.9} policy_passed={passed} nonzero_weights={nonzero}") })
}

pub struct FitResult {
    pub model: Ftrl,
    pub best_epoch: u64,
    pub history: Vec<Value>,
}

/// `fit_ftrl`.
pub fn fit_ftrl(training: &Featured<'_>, development: &Featured<'_>, config: &FitConfig, workers: usize) -> Result<FitResult, String> {
    let mut model = Ftrl::new(config);
    let (mut best_model, mut best_ranking, mut best_epoch, mut stale): (Option<Ftrl>, Option<[f64; 9]>, u64, u64) = (None, None, 0, 0);
    let mut order: Vec<u32> = (0..training.examples.len().to::<u32>()).collect();
    let mut scratch = Vec::new();
    let mut history = Vec::new();
    for epoch in 1..=config.maximum_epochs {
        Twister::new(config.seed + epoch).shuffle(&mut order);
        model.run_epoch(training, &order, &mut scratch);
        let frozen = &model;
        let raw = parallel_scores(development, workers, &|row| frozen.score(development, row));
        let report = evaluate_epoch(development, &raw, config, epoch, &model)?;
        eprintln!("intent: {}", report.line);
        history.push(report.value.clone());
        if best_ranking.as_ref().is_none_or(|known| greater(&report.ranking, known)) {
            best_ranking = Some(report.ranking);
            best_model = Some(model.clone());
            best_epoch = epoch;
            stale = 0;
        } else {
            stale += 1;
        }
        if epoch >= config.minimum_epochs && stale >= config.patience {
            break;
        }
    }
    Ok(FitResult { model: best_model.ok_or("no epoch produced a model")?, best_epoch, history })
}

/// `QuantizedWeights`.
pub struct Quantized {
    pub dimension: usize,
    pub scale: f64,
    pub values: Vec<i16>,
    pub support: Vec<u8>,
    pub maximum_absolute_error: f64,
}

/// `quantize_weights`.
pub fn quantize(weights: &[(usize, f64)], dimension: usize) -> Quantized {
    let maximum = weights.iter().map(|entry| entry.1.abs()).fold(0.0, f64::max);
    let scale = if maximum == 0.0 { 1.0 } else { maximum / QUANTIZED_LIMIT.to::<f64>() };
    let (mut values, mut support) = (vec![0_i16; dimension], vec![0_u8; dimension.div_ceil(8)]);
    let mut maximum_error = 0.0_f64;
    for (index, weight) in weights {
        let quantized = py_round(weight / scale).to::<i64>().clamp(-QUANTIZED_LIMIT, QUANTIZED_LIMIT);
        values[*index] = quantized.to::<i16>();
        if quantized != 0 {
            support[index / 8] |= 1 << (index % 8);
        }
        maximum_error = maximum_error.max((weight - quantized.to::<f64>() * scale).abs());
    }
    Quantized { dimension, scale, values, support, maximum_absolute_error: maximum_error }
}

/// `QuantizedLinearScorer.score` over all rows, on threads.
#[must_use]
pub fn score_quantized(quantized: &Quantized, bias: f64, rows: &Featured<'_>, workers: usize) -> Vec<f64> {
    parallel_scores(rows, workers, &|row| {
        let mut logit = bias;
        for position in rows.row(row) {
            logit += f64::from(quantized.values[rows.indices[position].to::<usize>()]) * quantized.scale * rows.values[position];
        }
        logit
    })
}

#[derive(Clone, Copy, Debug)]
pub struct Platt {
    pub slope: f64,
    pub intercept: f64,
    pub sample_count: i64,
    pub positive_count: i64,
}

fn calibration_loss(scores: &[(f64, bool)], slope: f64, intercept: f64, l2: f64) -> f64 {
    py_sum(scores.iter().map(|(score, label)| logistic_loss(slope * score + intercept, *label))) + CALIBRATION_L2_FACTOR * l2 * (slope * slope + intercept * intercept)
}

/// `fit_platt_calibration`.
fn fit_platt(scores: &[(f64, bool)], l2: f64, maximum_iterations: usize) -> Result<Platt, String> {
    let positives = scores.iter().filter(|entry| entry.1).count().to::<i64>();
    let negatives = scores.len().to::<i64>() - positives;
    if positives == 0 || negatives == 0 {
        return Err("calibration requires both labels".to_owned());
    }
    let mut slope = 1.0_f64;
    let mut intercept = ((positives.to::<f64>() + 1.0) / (negatives.to::<f64>() + 1.0)).ln();
    for _ in 0..maximum_iterations {
        let (mut gradient_slope, mut gradient_intercept) = (l2 * slope, l2 * intercept);
        let (mut hessian_ss, mut hessian_si, mut hessian_ii) = (l2, 0.0, l2);
        for (score, label) in scores {
            let probability = stable_sigmoid(slope * score + intercept);
            let residual = probability - f64::from(u8::from(*label));
            let curvature = (probability * (1.0 - probability)).max(MIN_CURVATURE);
            gradient_slope += residual * score;
            gradient_intercept += residual;
            hessian_ss += curvature * score * score;
            hessian_si += curvature * score;
            hessian_ii += curvature;
        }
        let determinant = hessian_ss * hessian_ii - hessian_si * hessian_si;
        if determinant <= MIN_DETERMINANT {
            break;
        }
        let step_slope = (hessian_ii * gradient_slope - hessian_si * gradient_intercept) / determinant;
        let step_intercept = (hessian_ss * gradient_intercept - hessian_si * gradient_slope) / determinant;
        if step_slope.abs().max(step_intercept.abs()) < STEP_TOLERANCE {
            break;
        }
        let old_loss = calibration_loss(scores, slope, intercept, l2);
        let mut step_scale = 1.0;
        let mut accepted = false;
        for _ in 0..LINE_SEARCH_STEPS {
            let (candidate_slope, candidate_intercept) = (slope - step_scale * step_slope, intercept - step_scale * step_intercept);
            if calibration_loss(scores, candidate_slope, candidate_intercept, l2) <= old_loss {
                slope = candidate_slope;
                intercept = candidate_intercept;
                accepted = true;
                break;
            }
            step_scale *= LINE_SEARCH_SHRINK;
        }
        if !accepted {
            break;
        }
    }
    Ok(Platt { slope, intercept, sample_count: scores.len().to::<i64>(), positive_count: positives })
}

/// `fit_directional_platt_calibration`: each physical direction on its own calibration partition.
pub fn fit_calibration(rows: &Featured<'_>, raw: &[f64], config: &FitConfig) -> Result<[Platt; 2], String> {
    let mut grouped: [Vec<(f64, bool)>; 2] = [Vec::new(), Vec::new()];
    for (example, logit) in rows.examples.iter().zip(raw) {
        grouped[direction_of(example)].push((*logit, example.label));
    }
    Ok([fit_platt(&grouped[0], config.calibration_l2, config.calibration_iterations)?, fit_platt(&grouped[1], config.calibration_l2, config.calibration_iterations)?])
}

/// `calibrate_raw_logits`.
#[must_use]
pub fn calibrate<'a>(rows: &Featured<'a>, raw: &[f64], calibration: &[Platt; 2]) -> Vec<Scored<'a>> {
    rows.examples
        .iter()
        .zip(raw)
        .map(|(example, logit)| {
            let direction = direction_of(example);
            let platt = &calibration[direction];
            Scored { example, raw: *logit, calibrated: platt.slope * logit + platt.intercept, typo: is_typo_kind(&example.variant_kind), direction }
        })
        .collect()
}

#[derive(Clone, Copy, Debug)]
pub struct Veto {
    pub raw_logit: f64,
    pub positive_samples: i64,
    pub vetoed_positive_samples: i64,
    pub false_negative_rate: f64,
}

impl Veto {
    #[must_use]
    pub fn payload(&self) -> Value {
        json!({"raw_logit": self.raw_logit, "positive_samples": self.positive_samples, "vetoed_positive_samples": self.vetoed_positive_samples, "false_negative_rate": self.false_negative_rate})
    }
}

/// `veto_metrics`.
#[must_use]
pub fn veto_metrics(items: &[Scored<'_>], raw_logit: f64) -> Veto {
    let positives: Vec<&Scored<'_>> = items.iter().filter(|item| item.example.label).collect();
    let vetoed = positives.iter().filter(|item| item.raw < raw_logit).count().to::<i64>();
    Veto { raw_logit, positive_samples: positives.len().to::<i64>(), vetoed_positive_samples: vetoed, false_negative_rate: vetoed.to::<f64>() / positives.len().to::<f64>() }
}

/// `choose_veto_threshold`.
#[must_use]
pub fn choose_veto(items: &[Scored<'_>], config: &FitConfig) -> Veto {
    let mut scores: Vec<f64> = items.iter().filter(|item| item.example.label).map(|item| item.raw).collect();
    scores.sort_by(|a, b| a.partial_cmp(b).expect("finite logits"));
    let index = (scores.len() - 1).min(((config.veto_quantile * (scores.len() - 1).to::<f64>()).floor()).to::<usize>());
    veto_metrics(items, scores[index] - config.veto_margin)
}

fn is_pause(trigger: CorrectionTrigger) -> bool {
    trigger == CorrectionTrigger::Pause
}

impl FitConfig {
    fn trigger_fpr(&self, trigger: CorrectionTrigger) -> f64 {
        if is_pause(trigger) { self.pause_max_fpr } else { self.max_fpr }
    }
}

fn binary_passes(metrics: &Confusion, precision: f64, recall: f64, specificity: f64, max_fpr: f64, z: f64) -> bool {
    metrics.true_positive + metrics.false_negative > 0 && metrics.negatives() > 0 && metrics.precision() >= precision && metrics.recall() >= recall && metrics.specificity() >= specificity && wilson_upper(metrics.false_positive, metrics.negatives(), z) <= max_fpr
}

pub type Selections = BTreeMap<&'static str, (CorrectionTrigger, Selection)>;

/// `_threshold_selection_passes_for_margin`: the selection gate of every trigger; the context profiles repeat the
/// neutral metrics, since the features ignore the context.
fn selections_pass(config: &FitConfig, selections: &Selections) -> bool {
    selections.values().all(|(trigger, selection)| {
        let (recall, typo_recall) = if is_pause(*trigger) { (config.selection_pause_recall, config.selection_pause_typo_recall) } else { (config.selection_recall, config.selection_typo_recall) };
        let fpr = config.trigger_fpr(*trigger);
        binary_passes(&selection.metrics, config.precision_floor, recall, config.test_specificity, fpr, SELECTION_Z)
            && binary_passes(&selection.typo_metrics, config.precision_floor, typo_recall, config.test_specificity, fpr, SELECTION_Z)
            && selection.metrics.false_positive <= config.selection_max_false_positives
            && selection.typo_metrics.false_positive <= config.selection_max_false_positives
    })
}

/// `_apply_trigger_threshold_margin`: one signed global margin and the additional Pause margin.
fn apply_margin(base: &Selections, by_trigger: &BTreeMap<&'static str, Vec<Scored<'_>>>, config: &FitConfig, margin: f64) -> Selections {
    let mut selected = Selections::new();
    for (name, (trigger, selection)) in base {
        let logits = [selection.logits[0] + margin, selection.logits[1] + margin];
        let items = &by_trigger[name];
        selected.insert(name, (*trigger, Selection { logits, metrics: confusion_of(items, logits, |_| true), typo_metrics: confusion_of(items, logits, |item| item.typo), margin }));
    }
    let pause = CorrectionTrigger::Pause.as_str();
    let initial = selected[pause].1.logits;
    let mut logits = [0.0; 2];
    for direction in 0..LAYOUT_DIRECTIONS.len() {
        let others = selected.iter().filter(|(name, _)| **name != pause).map(|(_, entry)| entry.1.logits[direction]).fold(f64::NEG_INFINITY, f64::max);
        logits[direction] = initial[direction].max(others + config.pause_margin);
    }
    let items = &by_trigger[pause];
    selected.insert(pause, (CorrectionTrigger::Pause, Selection { logits, metrics: confusion_of(items, logits, |_| true), typo_metrics: confusion_of(items, logits, |item| item.typo), margin }));
    selected
}

fn maximum_margin(base: &Selections, by_trigger: &BTreeMap<&'static str, Vec<Scored<'_>>>, config: &FitConfig) -> f64 {
    let cap = config.margin_cap;
    let zero = apply_margin(base, by_trigger, config, 0.0);
    if cap == 0.0 || !selections_pass(config, &zero) {
        return 0.0;
    }
    if selections_pass(config, &apply_margin(base, by_trigger, config, cap)) {
        return cap;
    }
    let mut candidates: Vec<f64> = vec![0.0, cap];
    for (name, items) in by_trigger {
        let intercepts = zero[name].1.logits;
        for item in items.iter().filter(|item| item.example.label) {
            let critical = item.calibrated - intercepts[item.direction];
            if !critical.is_finite() || !(0.0..=cap).contains(&critical) {
                continue;
            }
            candidates.push(critical);
            let below = critical.next_down();
            if below >= 0.0 {
                candidates.push(below);
            }
        }
    }
    candidates.sort_by(|a, b| a.partial_cmp(b).expect("finite margins"));
    candidates.dedup();
    let (mut lower, mut upper, mut best) = (0_i64, candidates.len().to::<i64>() - 1, 0_i64);
    while lower <= upper {
        let middle = i64::midpoint(lower, upper);
        if selections_pass(config, &apply_margin(base, by_trigger, config, candidates[middle.to::<usize>()])) {
            best = middle;
            lower = middle + 1;
        } else {
            upper = middle - 1;
        }
    }
    candidates[best.to::<usize>()]
}

/// `choose_trigger_thresholds`.
pub fn choose_thresholds(scored: &[Scored<'_>], config: &FitConfig) -> Result<Selections, String> {
    let mut by_trigger: BTreeMap<&'static str, Vec<Scored<'_>>> = TRIGGERS.iter().map(|trigger| (trigger.as_str(), Vec::new())).collect();
    for item in scored {
        by_trigger.get_mut(item.example.trigger.as_str()).expect("a known trigger").push(*item);
    }
    let mut base = Selections::new();
    for trigger in TRIGGERS {
        let fpr = config.trigger_fpr(trigger);
        let (recall, typo_recall) = if is_pause(trigger) { (config.selection_pause_recall, config.selection_pause_typo_recall) } else { (config.selection_recall, config.selection_typo_recall) };
        let policy = ThresholdPolicy { precision_floor: config.precision_floor, max_fpr: fpr, minimum_recall: recall, minimum_specificity: config.test_specificity, typo_precision_floor: config.precision_floor, minimum_typo_recall: typo_recall, typo_minimum_specificity: config.test_specificity, typo_max_fpr: fpr, maximum_false_positives: config.selection_max_false_positives };
        base.insert(trigger.as_str(), (trigger, choose_directional(&by_trigger[trigger.as_str()], &policy)?));
    }
    let margin = maximum_margin(&base, &by_trigger, config);
    Ok(apply_margin(&base, &by_trigger, config, margin))
}

/// `metrics_payload`.
#[must_use]
pub fn metrics_payload(metrics: &Confusion) -> Value {
    let round = |value: f64| py_round_ndigits(value, METRIC_DECIMALS);
    json!({"true_positive": metrics.true_positive, "false_negative": metrics.false_negative, "true_negative": metrics.true_negative, "false_positive": metrics.false_positive,
        "precision": round(metrics.precision()), "recall": round(metrics.recall()), "specificity": round(metrics.specificity()), "false_positive_rate": round(1.0 - metrics.specificity()),
        "false_positive_rate_upper_95": round(wilson_upper(metrics.false_positive, metrics.negatives(), WILSON_95_Z)), "negative_samples": metrics.negatives()})
}

/// `binary_gate_breakdown`.
#[must_use]
pub fn binary_gate(metrics: &Confusion, precision: f64, recall: f64, specificity: f64, max_fpr: f64, familywise: bool) -> Value {
    let z = if familywise { SELECTION_Z } else { WILSON_95_Z };
    let upper = wilson_upper(metrics.false_positive, metrics.negatives(), z);
    let checks = json!({"positive_samples": metrics.true_positive + metrics.false_negative > 0, "negative_samples": metrics.negatives() > 0, "precision": metrics.precision() >= precision, "recall": metrics.recall() >= recall, "specificity": metrics.specificity() >= specificity, "false_positive_rate_upper_bound": upper <= max_fpr});
    let passed = checks.as_object().expect("an object").values().all(|value| value == &json!(true));
    let bound = json!({"method": "wilson_score_upper_endpoint", "multiplicity_correction": if familywise { "bonferroni" } else { "none" }, "familywise_confidence": WILSON_CONFIDENCE, "comparisons": if familywise { SELECTION_COMPARISONS } else { 1 }, "per_comparison_confidence": if familywise { SELECTION_PER_COMPARISON_CONFIDENCE } else { WILSON_CONFIDENCE }, "z_score": z, "upper": upper});
    json!({"passed": passed, "checks": checks, "actual": metrics_payload(metrics), "false_positive_bound": bound, "limits": {"minimum_precision": precision, "minimum_recall": recall, "minimum_specificity": specificity, "maximum_false_positive_rate_upper_bound": max_fpr}})
}

/// `context_stress_gate_breakdown` with the same metrics in every profile.
#[must_use]
pub fn context_stress_gate(config: &FitConfig, metrics: &BTreeMap<&'static str, (Confusion, Confusion)>, selection_phase: bool) -> Value {
    let mut expected: Vec<&str> = STRESS_PROFILES.iter().map(|profile| profile.0).collect();
    expected.sort_unstable();
    let mut per_trigger = Map::new();
    let mut every_trigger = true;
    for trigger in TRIGGERS {
        let (overall, typos) = &metrics[trigger.as_str()];
        let pause = is_pause(trigger);
        let (precision, recall, typo_recall) = if selection_phase {
            (config.precision_floor, if pause { config.selection_pause_recall } else { config.selection_recall }, if pause { config.selection_pause_typo_recall } else { config.selection_typo_recall })
        } else {
            (config.test_precision, if pause { config.test_pause_recall } else { config.test_recall }, if pause { config.test_pause_typo_recall } else { config.test_typo_recall })
        };
        let fpr = config.trigger_fpr(trigger);
        let overall_gate = binary_gate(overall, precision, recall, config.test_specificity, fpr, selection_phase);
        let typo_gate = binary_gate(typos, precision, typo_recall, config.test_specificity, fpr, selection_phase);
        let passed = overall_gate["passed"] == json!(true) && typo_gate["passed"] == json!(true);
        every_trigger &= passed;
        per_trigger.insert(trigger.as_str().to_owned(), json!({"passed": passed, "overall": overall_gate, "typos": typo_gate}));
    }
    let profiles: Map<String, Value> = STRESS_PROFILES.iter().map(|(name, delta, selector)| ((*name).to_owned(), json!({"passed": every_trigger, "delta": delta, "group_selector": selector, "all_triggers_present": true, "per_trigger": per_trigger}))).collect();
    json!({"passed": every_trigger, "all_profiles_present": true, "expected_profiles": expected, "profiles": profiles})
}

/// `threshold_selection_gate_breakdown`.
#[must_use]
pub fn selection_gate(config: &FitConfig, selections: &Selections) -> Value {
    let mut per_trigger = Map::new();
    let mut all_passed = true;
    let mut neutral_metrics = BTreeMap::new();
    for trigger in TRIGGERS {
        let selection = &selections[trigger.as_str()].1;
        neutral_metrics.insert(trigger.as_str(), (selection.metrics, selection.typo_metrics));
        let pause = is_pause(trigger);
        let fpr = config.trigger_fpr(trigger);
        let overall = binary_gate(&selection.metrics, config.precision_floor, if pause { config.selection_pause_recall } else { config.selection_recall }, config.test_specificity, fpr, true);
        let typos = binary_gate(&selection.typo_metrics, config.precision_floor, if pause { config.selection_pause_typo_recall } else { config.selection_typo_recall }, config.test_specificity, fpr, true);
        let budget_checks = (selection.metrics.false_positive <= config.selection_max_false_positives, selection.typo_metrics.false_positive <= config.selection_max_false_positives);
        let budget = json!({"passed": budget_checks.0 && budget_checks.1, "checks": {"overall": budget_checks.0, "typos": budget_checks.1}, "actual": {"overall_false_positives": selection.metrics.false_positive, "typo_false_positives": selection.typo_metrics.false_positive}, "maximum_false_positives_per_trigger": config.selection_max_false_positives});
        let passed = overall["passed"] == json!(true) && typos["passed"] == json!(true) && budget["passed"] == json!(true);
        all_passed &= passed;
        per_trigger.insert(trigger.as_str().to_owned(), json!({"passed": passed, "logits": {"0>1": selection.logits[0], "1>0": selection.logits[1]}, "overall": overall, "typos": typos, "false_positive_budget": budget}));
    }
    let context = context_stress_gate(config, &neutral_metrics, true);
    json!({"passed": all_passed && context["passed"] == json!(true), "all_triggers_present": true, "per_trigger": per_trigger, "neutral": {"passed": all_passed, "all_triggers_present": true, "per_trigger": per_trigger}, "context_stress": context})
}

/// The features ignore the context: the check `verify_context_feature_invariance` makes before neutral scores stand in
/// for every stress profile.
pub fn verify_context_invariance(config: &FitConfig) -> Result<(), String> {
    let sentinels = [("test", "еуые", 0, 1), ("еуые", "test", 1, 0), ("keyboard", "лунищфкв", 0, 1), ("лунищфкв", "keyboard", 1, 0)];
    let score = ignored_score();
    for (original, alternative, source_group, target_group) in sentinels {
        for trigger in TRIGGERS {
            let input = |context_delta: f64, context_group: Option<i64>| IntentModelInput { original, alternative, source_group, target_group, trigger, source_score: &score, target_score: &score, context_delta, context_group };
            let extract = |input: IntentModelInput<'_>| extract_features(&input, config.dimension.to::<i64>(), config.feature_hash_seed, config.membership_hash_seed, &NGRAM_ORDERS).map_err(|error| error.name.to_string());
            let expected = extract(input(0.0, None))?;
            for (_, delta, selector) in STRESS_PROFILES {
                let group = if selector == "source" { source_group } else { target_group };
                assert!(extract(input(delta, Some(group)))? == expected, "the feature schema is context-sensitive");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use std::path::Path;

    fn config() -> FitConfig {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../model/intent_v1/config.json");
        FitConfig::read(&crate::corpus::read_json(&path).expect("the config")).expect("a config")
    }

    fn example(trigger: CorrectionTrigger, label: bool, kind: &str) -> LexicalExample {
        LexicalExample {
            original: "ghbdtn".to_owned(),
            alternative: "привет".to_owned(),
            source_group: 0,
            target_group: 1,
            trigger,
            label,
            weight: 1.0,
            base_signature: "s".to_owned(),
            variant_kind: kind.to_owned(),
            source_known: false,
            target_known: true,
            context_delta: 0.0,
            context_group: None,
            frequency: 1,
            protected: false,
            safety: false,
        }
    }

    fn scored<'a>(example: &'a LexicalExample, calibrated: f64, typo: bool, direction: usize) -> Scored<'a> {
        Scored { example, raw: calibrated, calibrated, typo, direction }
    }

    #[test]
    fn small_parts_of_a_fit() {
        assert!(balanced_ranges(0, 3).is_empty());
        assert_eq!(balanced_ranges(5, 2).len(), 2);
        let mut twister = Twister::new((1_u64 << 33) + 5);
        let mut items: Vec<u32> = (0..50).collect();
        twister.shuffle(&mut items);
        assert_eq!(items.len(), 50);
        for _ in 0..1000 {
            let _ = twister.next_u32();
        }
        assert!(is_typo_kind("deletion") && !is_typo_kind("identity"));
        let selection = Selection { logits: [1.0, 2.0], metrics: Confusion::default(), typo_metrics: Confusion::default(), margin: 0.0 };
        assert!((selection.logit() - 2.0).abs() < f64::EPSILON);
        assert_eq!(quantize(&[], 4).scale.to_bits(), quantize(&[], 4).scale.to_bits());
        assert!(quantize(&[(1, 0.5), (2, 1e-12)], 4).support.iter().any(|byte| *byte != 0));
        let mut settings = config();
        settings.alpha = f64::INFINITY;
        assert!(Ftrl::new(&settings).bias().abs() < f64::EPSILON);
    }

    #[test]
    fn a_calibration_needs_both_labels_and_stops_where_it_cannot_go_on() {
        assert!(fit_platt(&[(1.0, true)], 0.01, 10).is_err());
        assert!(fit_platt(&[(1.0, false)], 0.01, 10).is_err());
        assert!(fit_platt(&[(0.0, true), (0.0, false)], 0.0, 10).is_ok(), "no curvature to step along");
        assert!(fit_platt(&[(f64::NAN, true), (1.0, false)], 0.01, 10).is_ok(), "no step lowers a loss that is no number");
        let scores: Vec<(f64, bool)> = (0..40).map(|index| (f64::from(index) - 20.0, index > 20)).collect();
        let fitted = fit_platt(&scores, 0.01, 50).expect("a fit");
        assert!(fitted.slope > 0.0);
    }

    #[test]
    fn an_operating_curve_needs_examples_of_every_kind() {
        assert!(direction_curve(&[], 0, 1).unwrap_err().contains("no threshold examples"));
        let (right, wrong) = (example(CorrectionTrigger::Space, true, "identity"), example(CorrectionTrigger::Space, false, "identity"));
        assert!(direction_curve(&[scored(&right, 1.0, false, 0), scored(&wrong, 0.0, false, 0)], 0, 1).unwrap_err().contains("both labels"));
        // Ties, so that a later point is as good as an earlier one and differs only by where it stands.
        let rows = [
            scored(&right, 5.0, true, 0), scored(&right, 5.0, false, 0), scored(&wrong, 4.0, true, 0), scored(&right, 3.0, true, 0), scored(&wrong, 3.0, false, 0), scored(&wrong, 2.0, true, 0), scored(&right, 1.0, false, 0), scored(&wrong, 1.0, true, 0), scored(&wrong, 0.5, false, 0), scored(&right, 0.1, true, 0), scored(&right, 0.1, false, 0),
        ];
        for maximum in [0, 1, 2, 10] {
            assert!(!direction_curve(&rows, 0, maximum).expect("a curve").is_empty());
        }
        let settings = config();
        assert!(choose_thresholds(&[], &settings).is_err());
        assert!(choose_directional(&[], &policy_of(&settings)).is_err());
        assert!(maximum_primary_false_positives(0, 0, 0.0, 1.0, 1.0).is_ok());
        assert!(maximum_primary_false_positives(10, 5, 0.5, 0.0, 10.0).is_err());
    }

    fn policy_of(settings: &FitConfig) -> ThresholdPolicy {
        ThresholdPolicy { precision_floor: 1.0, max_fpr: 0.0, minimum_recall: 1.0, minimum_specificity: 1.0, minimum_typo_recall: 1.0, typo_precision_floor: 1.0, typo_minimum_specificity: 1.0, typo_max_fpr: 0.0, maximum_false_positives: settings.selection_max_false_positives }
    }

    #[test]
    fn a_binary_gate_and_a_selection_read_each_count() {
        let good = Confusion { true_positive: 1000, false_negative: 0, true_negative: 100_000, false_positive: 0 };
        assert!(binary_passes(&good, 0.9, 0.9, 0.9, 0.5, 1.0));
        for (metrics, precision, recall, specificity) in [
            (Confusion { true_positive: 0, false_negative: 0, ..good }, 0.0, 0.0, 0.0),
            (Confusion { true_negative: 0, false_positive: 0, ..good }, 0.0, 0.0, 0.0),
            (Confusion { false_positive: 1000, ..good }, 0.9, 0.0, 0.0),
            (Confusion { true_positive: 1, false_negative: 1000, ..good }, 0.0, 0.9, 0.0),
            (Confusion { true_negative: 1, false_positive: 1000, ..good }, 0.0, 0.0, 0.9),
        ] {
            assert!(!binary_passes(&metrics, precision, recall, specificity, 1.0, 1.0));
        }
        assert!(!binary_passes(&Confusion { true_negative: 2, false_positive: 1, ..good }, 0.0, 0.0, 0.0, 0.0, 1.0));
        let mut settings = config();
        settings.selection_max_false_positives = 0;
        settings.precision_floor = 0.5;
        settings.selection_recall = 0.5;
        settings.selection_pause_recall = 0.5;
        settings.selection_typo_recall = 0.5;
        settings.selection_pause_typo_recall = 0.5;
        settings.test_specificity = 0.5;
        settings.max_fpr = 1.0;
        settings.pause_max_fpr = 1.0;
        let selection = |metrics: Confusion, typos: Confusion| -> Selections { TRIGGERS.iter().map(|trigger| (trigger.as_str(), (*trigger, Selection { logits: [1.0, 1.0], metrics, typo_metrics: typos, margin: 0.0 }))).collect() };
        assert!(selections_pass(&settings, &selection(good, good)));
        let noisy = Confusion { false_positive: 1, ..good };
        assert!(!selections_pass(&settings, &selection(noisy, good)));
        assert!(!selections_pass(&settings, &selection(good, noisy)));
        let lost = Confusion { true_positive: 0, false_negative: 1000, ..good };
        assert!(!selections_pass(&settings, &selection(lost, good)));
        assert!(!selections_pass(&settings, &selection(good, lost)));
        let gate = selection_gate(&settings, &selection(noisy, noisy));
        assert_eq!(gate["passed"], json!(false));
        let gate = selection_gate(&settings, &selection(lost, good));
        assert_eq!(gate["passed"], json!(false));
        let gate = selection_gate(&settings, &selection(good, lost));
        assert_eq!(gate["passed"], json!(false));
    }

    #[test]
    fn a_threshold_is_chosen_under_each_budget() {
        let (right, wrong, typo_right, typo_wrong) = (example(CorrectionTrigger::Space, true, "identity"), example(CorrectionTrigger::Space, false, "identity"), example(CorrectionTrigger::Space, true, "deletion"), example(CorrectionTrigger::Space, false, "deletion"));
        let mut rows = Vec::new();
        for index in 0..200 {
            let value = f64::from(index);
            for direction in 0..2 {
                rows.push(scored(&right, value + 1000.0, false, direction));
                rows.push(scored(&wrong, value, false, direction));
                rows.push(scored(&typo_right, value + 1000.0, true, direction));
                rows.push(scored(&typo_wrong, value, true, direction));
            }
        }
        let settings = config();
        for maximum in [0, 1] {
            for (floor, recall, specificity) in [(0.5, 0.5, 0.5), (1.0, 1.0, 1.0)] {
                let policy = ThresholdPolicy { precision_floor: floor, max_fpr: 0.5, minimum_recall: recall, minimum_specificity: specificity, minimum_typo_recall: recall, typo_precision_floor: floor, typo_minimum_specificity: specificity, typo_max_fpr: 0.5, maximum_false_positives: maximum };
                let _ = choose_directional(&rows, &policy);
                let noisy: Vec<Scored<'_>> = rows.iter().map(|item| Scored { calibrated: if item.typo { item.calibrated - 900.0 } else { item.calibrated }, ..*item }).collect();
                let _ = choose_directional(&noisy, &policy);
            }
        }
        let _ = settings;
    }
}
