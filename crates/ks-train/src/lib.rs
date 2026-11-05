//! Training of the context action model: the corpus frames and curricula, the features, sparse AdaGrad softmax over packed
//! rows, the epoch and threshold selection, the artifact's bytes and the seal. A fit builds the same candidate as the
//! Python trainer did, to the bit, so that a model can be retrained without Python.
#![forbid(unsafe_code)]

pub mod artifact;
pub mod base_corpus;
pub mod boundary;
pub mod captured;
pub mod compare;
pub mod corpus;
pub mod curricula;
pub mod detector;
pub mod engine_reports;
pub mod evaluate;
pub mod evidence;
pub mod fit;
pub mod freeze;
pub mod heads;
pub mod historical;
pub mod intent;
pub mod kernel;
pub mod lookahead;
pub mod messages;
pub mod mixed;
pub mod ortho;
pub mod ortho_inputs;
pub mod physical;
pub mod prefix;
pub mod prefix_corpus;
pub mod rows;
pub mod promote;
pub mod receipt;
pub mod replay;
pub mod selection;
pub mod small_tools;
pub mod spans;
pub mod tools;
pub mod typo_evaluation;
pub mod typo_frequency;

pub const ACTIONS: usize = 4;
pub const FEATURE_MASS_TOLERANCE: f64 = 1e-9;
pub const DETERMINISTIC_ROUNDING_DECIMALS: i64 = 9;
pub const LOG_LOSS_PROBABILITY_FLOOR: f64 = 1e-15;
pub const PROBABILITY_SUM_TOLERANCE: f64 = 1e-8;
pub const EPOCH_NET_BENEFIT_TOLERANCE: i64 = 2;

/// A record a person opens to read: indented, keys sorted, and a final newline.
pub fn train_json(value: &serde_json::Value) -> Vec<u8> {
    (ks_text::json_dumps(value, &ks_text::JsonOptions { ensure_ascii: false, indent: Some(ks_text::Indent::Spaces(2)), sort_keys: true, ..ks_text::JsonOptions::default() }) + "\n").into_bytes()
}

/// The same, keys in the order they were written.
pub fn train_json_unsorted(value: &serde_json::Value) -> Vec<u8> {
    (ks_text::json_dumps(value, &ks_text::JsonOptions { ensure_ascii: false, indent: Some(ks_text::Indent::Spaces(2)), ..ks_text::JsonOptions::default() }) + "\n").into_bytes()
}
