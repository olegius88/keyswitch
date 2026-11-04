//! The numeric core of the context model's training: sparse AdaGrad softmax over packed rows, the mass that selects
//! features, the epoch and threshold selection, and the artifact's bytes. Corpus and curricula build on it.
#![forbid(unsafe_code)]

pub mod artifact;
pub mod kernel;
pub mod selection;

pub const ACTIONS: usize = 4;
pub const FEATURE_MASS_TOLERANCE: f64 = 1e-9;
pub const DETERMINISTIC_ROUNDING_DECIMALS: i64 = 9;
pub const LOG_LOSS_PROBABILITY_FLOOR: f64 = 1e-15;
pub const PROBABILITY_SUM_TOLERANCE: f64 = 1e-8;
pub const EPOCH_NET_BENEFIT_TOLERANCE: i64 = 2;
