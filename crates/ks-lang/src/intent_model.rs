//! Deterministic, dependency-free linear intent model runtime.
//!
//! KSLM is a deliberately small binary container. It combines a canonical JSON manifest with quantized
//! little-endian int16 weights and sorted uint64 fingerprints of the exact character features observed during
//! training. The fingerprints use an independent hash namespace: coverage therefore measures feature membership
//! rather than occupancy of collision-prone weight buckets. Loading is strict and bounded so a broken optional model
//! can never prevent `KeySwitch` from falling back to its conservative rules.
//!
//! The container is read once and kept as it is: the weights and the fingerprints are read in place from the bytes of
//! the file (little endian), without a second copy. The training side (`encode_model`, `write_model`) is not here.

use ks_cast::To as _;
use crate::constants::{VERSION_HASH_CHARACTERS, INTENT_RAW_TOKEN_MAX_CHARACTERS, FNV1A64_PRIME, FEATURE_HASH_SIGN_BIT, INTENT_LENGTH_EXACT_MAX_CHARACTERS, INTENT_LENGTH_SHORT_MAX_CHARACTERS, INTENT_LENGTH_MEDIUM_MAX_CHARACTERS, INTENT_LENGTH_LONG_MAX_CHARACTERS, INTENT_MAX_FEATURE_GROUP, KSLM_WEIGHT_ENTRY_BYTES, KSLM_FINGERPRINT_ENTRY_BYTES, KSLM_MAX_CONTAINER_BYTES, KSLM_SCHEMA_VERSION, KSLM_MIN_MANIFEST_BYTES, KSLM_MAX_MANIFEST_BYTES, KSLM_MAX_PAYLOAD_BYTES, INTENT_FEATURE_VERSION, KSLM_MAX_FINGERPRINTS, SHA256_HEX_CHARACTERS, INTENT_METADATA_MAX_DEPTH, KSLM_MAX_DIMENSION, KSLM_MAX_DIMENSION_LOG2, INTENT_NGRAM_ORDERS, INTENT_MODEL_VERSION_MAX_CHARACTERS, INTENT_MAX_MODEL_FLOAT_MAGNITUDE, INT64_MIN, UINT64_MAX};
use crate::language_model::WordScore;
use crate::load_error::LoadError;
use ks_text::{exp, normalize, pystr};
use serde_json::Value;
use sha2::{Digest, Sha256};
use serde::de::{DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// When a correction is asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CorrectionTrigger {
    BoundaryProbe,
    Pause,
    Space,
    Enter,
    Tab,
    Punctuation,
}

/// `TRIGGERS`, in the order of the Python.
pub const TRIGGERS: [CorrectionTrigger; 6] = [
    CorrectionTrigger::BoundaryProbe,
    CorrectionTrigger::Pause,
    CorrectionTrigger::Space,
    CorrectionTrigger::Enter,
    CorrectionTrigger::Tab,
    CorrectionTrigger::Punctuation,
];

impl CorrectionTrigger {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            CorrectionTrigger::BoundaryProbe => "boundary_probe",
            CorrectionTrigger::Pause => "pause",
            CorrectionTrigger::Space => "space",
            CorrectionTrigger::Enter => "enter",
            CorrectionTrigger::Tab => "tab",
            CorrectionTrigger::Punctuation => "punctuation",
        }
    }

    #[must_use]
    pub fn parse(name: &str) -> Option<CorrectionTrigger> {
        TRIGGERS.into_iter().find(|trigger| trigger.as_str() == name)
    }

    fn index(self) -> usize {
        TRIGGERS.iter().position(|trigger| *trigger == self).expect("every trigger is listed")
    }
}

/// `LayoutDirection`: `"0>1"` or `"1>0"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutDirection {
    ZeroToOne,
    OneToZero,
}

pub const LAYOUT_DIRECTIONS: [LayoutDirection; 2] = [LayoutDirection::ZeroToOne, LayoutDirection::OneToZero];

impl LayoutDirection {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            LayoutDirection::ZeroToOne => "0>1",
            LayoutDirection::OneToZero => "1>0",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::ZeroToOne => 0,
            Self::OneToZero => 1,
        }
    }
}

const MAGIC: &[u8; 4] = b"KSLM";
const HASH_ALGORITHM: &str = "fnv1a64-signed-v1";
const MEMBERSHIP_ALGORITHM: &str = "fnv1a64-unsigned-v1";
/// `HEADER.size` of `struct.Struct("<4sHHIII32s")`.
const HEADER_SIZE: usize = 52;

/// All bounded evidence needed for one layout-correction decision. The scores and the context are carried for the
/// callers; the linear model reads the texts, the groups and the trigger.
#[derive(Clone, Copy, Debug)]
pub struct IntentModelInput<'a> {
    pub original: &'a str,
    pub alternative: &'a str,
    pub source_group: i64,
    pub target_group: i64,
    pub trigger: CorrectionTrigger,
    pub source_score: &'a WordScore,
    pub target_score: &'a WordScore,
    pub context_delta: f64,
    pub context_group: Option<i64>,
}

/// Calibrated model output plus its conservative operating threshold.
#[derive(Clone, Debug, PartialEq)]
pub struct LinearPrediction {
    pub logit: f64,
    pub probability: f64,
    pub threshold: f64,
    pub coverage: f64,
    pub should_switch: bool,
    pub model_version: String,
}

/// Diagnostics-safe state returned by optional-model discovery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntentModelStatus {
    pub available: bool,
    pub path: Option<PathBuf>,
    pub version: Option<String>,
    pub checksum: Option<String>,
    pub error: Option<String>,
}

impl IntentModelStatus {
    #[must_use]
    pub fn summary(&self) -> String {
        if self.available {
            let version = self.version.as_deref().filter(|version| !version.is_empty()).unwrap_or("unknown");
            return match self.checksum.as_deref().filter(|checksum| !checksum.is_empty()) {
                Some(checksum) => format!("{version} · sha256:{}", pystr::slice(checksum, None, Some(VERSION_HASH_CHARACTERS.to::<i64>()))),
                None => version.to_string(),
            };
        }
        let error = self.error.as_deref().filter(|error| !error.is_empty()).unwrap_or("файл модели не найден");
        format!("недоступна: {error}")
    }

    #[must_use]
    pub fn as_dict(&self) -> Value {
        let text = |value: &Option<String>| value.as_ref().map_or(Value::Null, |value| Value::String(value.clone()));
        let mut map = serde_json::Map::new();
        map.insert("available".into(), Value::Bool(self.available));
        map.insert("path".into(), self.path.as_ref().map_or(Value::Null, |path| Value::String(path.display().to_string())));
        map.insert("version".into(), text(&self.version));
        map.insert("checksum".into(), text(&self.checksum));
        map.insert("error".into(), text(&self.error));
        map.insert("summary".into(), Value::String(self.summary()));
        Value::Object(map)
    }
}

/// Stable sparse vector and exact-name fingerprints used for coverage.
#[derive(Clone, Debug, PartialEq)]
pub struct FeatureVector {
    /// `(bucket, value)` sorted by bucket, no zero value.
    pub values: Vec<(u32, f64)>,
    /// The set of fingerprints, sorted.
    pub character_fingerprints: Vec<u64>,
}

/// Immutable sigmoid calibration for one physical layout direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlattParameters {
    pub scale: f64,
    pub bias: f64,
}

/// Bound CPU/memory use, then apply the model's Unicode normalization.
#[must_use]
pub fn normalize_token(token: &str) -> String {
    let bounded = pystr::slice(token, None, Some(INTENT_RAW_TOKEN_MAX_CHARACTERS.to::<i64>()));
    normalize::nfc(&pystr::casefold(&normalize::nfc(bounded)))
}

/// A platform-independent FNV-1a hash of UTF-8 feature text. (A seed is a `u64` here: the Python's range check is the type.)
#[must_use]
pub fn fnv1a64(value: &str, seed: u64) -> u64 {
    value.bytes().fold(seed, |result, byte| (result ^ u64::from(byte)).wrapping_mul(FNV1A64_PRIME))
}

/// Map one feature to a bucket and a deterministic collision sign.
pub fn signed_feature_hash(feature: &str, dimension: i64, seed: u64) -> Result<(u64, i64), LoadError> {
    validate_dimension(dimension)?;
    Ok(hashed_feature(feature, dimension.to::<u64>(), seed))
}

fn hashed_feature(feature: &str, dimension: u64, seed: u64) -> (u64, i64) {
    let hashed = fnv1a64(feature, seed);
    (hashed & (dimension - 1), if hashed & FEATURE_HASH_SIGN_BIT != 0 { -1 } else { 1 })
}

/// Numerically stable sigmoid, including saturated finite inputs.
pub fn stable_sigmoid(value: f64) -> Result<f64, LoadError> {
    if value.is_nan() {
        return Err(LoadError::value("sigmoid input must not be NaN"));
    }
    // `math.exp` of a non-positive number never overflows.
    Ok(if value >= 0.0 {
        1.0 / (1.0 + exp(-value).expect("exp of a non-positive number"))
    } else {
        let exponential = exp(value).expect("exp of a negative number");
        exponential / (1.0 + exponential)
    })
}

/// Build pairwise character and bounded dense hashed features.
pub fn extract_features(
    evidence: &IntentModelInput<'_>,
    dimension: i64,
    hash_seed: u64,
    membership_seed: u64,
    ngram_orders: &[i64],
) -> Result<FeatureVector, LoadError> {
    validate_dimension(dimension)?;
    validate_membership_seed(membership_seed, hash_seed)?;
    let orders = validate_orders(ngram_orders)?;
    let dimension = dimension.to::<u64>();

    let mut additions: Vec<(u64, f64)> = Vec::new();
    let mut fingerprints: Vec<u64> = Vec::new();
    let mut add = |name: &str, value: f64, character: bool| {
        if value == 0.0 {
            return;
        }
        let (bucket, sign) = hashed_feature(name, dimension, hash_seed);
        additions.push((bucket, value * sign.to::<f64>()));
        if character {
            fingerprints.push(fnv1a64(name, membership_seed));
        }
    };

    let original = normalize_token(evidence.original);
    let alternative = normalize_token(evidence.alternative);
    let (original_length, alternative_length) = (pystr::len(&original), pystr::len(&alternative));
    let active_orders = orders.iter().filter(|&&order| (original_length + 2).to::<i64>() >= order || (alternative_length + 2).to::<i64>() >= order).count();
    let order_scale = if active_orders > 0 { 1.0 / active_orders.to::<f64>().sqrt() } else { 1.0 };
    add_character_features(&original, evidence.source_group, -order_scale, &orders, &mut add);
    add_character_features(&alternative, evidence.target_group, order_scale, &orders, &mut add);

    let direction = format!("{}>{}", group(evidence.source_group), group(evidence.target_group));
    let length_bucket = length_bucket(original_length.max(alternative_length));
    let trigger = evidence.trigger.as_str();

    add("dense:length_delta", (alternative_length.to::<f64>() - original_length.to::<f64>()) / INTENT_RAW_TOKEN_MAX_CHARACTERS.to::<f64>(), false);
    add(&format!("interaction:direction:{direction}:length:{length_bucket}"), 1.0, false);
    add(&format!("interaction:trigger:{trigger}:length:{length_bucket}"), 1.0, false);
    add(&format!("length:{length_bucket}"), 1.0, false);
    add(&format!("direction:{direction}"), 1.0, false);
    add(&format!("trigger:{trigger}"), 1.0, false);

    // A bucket adds up in the order the features were added (a stable sort keeps it).
    additions.sort_by_key(|item| item.0);
    let mut values: Vec<(u32, f64)> = Vec::new();
    let mut last: Option<u64> = None;
    for (bucket, delta) in additions {
        match values.last_mut() {
            Some(entry) if last == Some(bucket) => entry.1 += delta,
            _ => {
                values.push((bucket.to::<u32>(), 0.0 + delta));
                last = Some(bucket);
            }
        }
    }
    values.retain(|item| item.1 != 0.0);
    fingerprints.sort_unstable();
    fingerprints.dedup();
    Ok(FeatureVector { values, character_fingerprints: fingerprints })
}

fn add_character_features(token: &str, group_number: i64, polarity: f64, orders: &[i64], add: &mut impl FnMut(&str, f64, bool)) {
    let bounded = format!("^{token}$");
    // Byte offsets of every code point, and the end: a slice of `order` characters is `offsets[i]..offsets[i + order]`.
    let mut offsets: Vec<usize> = bounded.char_indices().map(|(offset, _)| offset).collect();
    offsets.push(bounded.len());
    let length = offsets.len() - 1;
    for &order in orders {
        let order = order.to::<usize>();
        if length < order {
            continue;
        }
        // Counter(...): first occurrence first.
        let mut counts: Vec<(&str, u64)> = Vec::new();
        let mut index: HashMap<&str, usize> = HashMap::new();
        for start in 0..=(length - order) {
            let gram = &bounded[offsets[start]..offsets[start + order]];
            if let Some(&slot) = index.get(gram) { counts[slot].1 += 1 } else {
                index.insert(gram, counts.len());
                counts.push((gram, 1));
            }
        }
        // math.sqrt of an exact integer.
        let norm = (counts.iter().map(|item| item.1 * item.1).sum::<u64>()).to::<f64>().sqrt();
        let prefix = format!("char:g{}:n{}:", group(group_number), order);
        let mut name = String::new();
        for (gram, count) in counts {
            name.clear();
            name.push_str(&prefix);
            name.push_str(gram);
            add(&name, polarity * count.to::<f64>() / norm, true);
        }
    }
}

fn length_bucket(length: usize) -> String {
    if length <= INTENT_LENGTH_EXACT_MAX_CHARACTERS {
        return length.to_string();
    }
    if length <= INTENT_LENGTH_SHORT_MAX_CHARACTERS {
        return format!("{}-{}", INTENT_LENGTH_EXACT_MAX_CHARACTERS + 1, INTENT_LENGTH_SHORT_MAX_CHARACTERS);
    }
    if length <= INTENT_LENGTH_MEDIUM_MAX_CHARACTERS {
        return format!("{}-{}", INTENT_LENGTH_SHORT_MAX_CHARACTERS + 1, INTENT_LENGTH_MEDIUM_MAX_CHARACTERS);
    }
    if length <= INTENT_LENGTH_LONG_MAX_CHARACTERS {
        return format!("{}-{}", INTENT_LENGTH_MEDIUM_MAX_CHARACTERS + 1, INTENT_LENGTH_LONG_MAX_CHARACTERS);
    }
    format!("{}+", INTENT_LENGTH_LONG_MAX_CHARACTERS + 1)
}

fn group(group: i64) -> i64 {
    group.clamp(-1, INTENT_MAX_FEATURE_GROUP)
}

pub fn layout_direction(source_group: i64, target_group: i64) -> Result<LayoutDirection, LoadError> {
    match (source_group, target_group) {
        (0, 1) => Ok(LayoutDirection::ZeroToOne),
        (1, 0) => Ok(LayoutDirection::OneToZero),
        _ => Err(LoadError::value("intent model requires an EN/RU layout direction")),
    }
}

/// Immutable KSLM v4 model with direction-calibrated inference.
pub struct LinearNgramModel {
    pub dimension: i64,
    /// The whole file: the weights and the fingerprints are read from `payload_start` on, in place.
    data: Vec<u8>,
    manifest_start: usize,
    payload_start: usize,
    fingerprint_count: usize,
    pub weight_scale: f64,
    pub bias: f64,
    /// By `LayoutDirection::index`.
    pub platt_calibration: [PlattParameters; 2],
    /// `threshold_logits[trigger][direction]`, in `TRIGGERS` order.
    pub threshold_logits: [[f64; 2]; 6],
    /// `stable_sigmoid` of `threshold_logits`.
    pub thresholds: [[f64; 2]; 6],
    pub veto_threshold: f64,
    pub model_version: String,
    pub fnv_seed: u64,
    pub membership_seed: u64,
    pub ngram_orders: [i64; 5],
    pub payload_sha256: String,
    pub checksum: String,
    pub source_path: Option<PathBuf>,
}

impl LinearNgramModel {
    /// `LinearNgramModel.load(path)`.
    pub fn load(path: &Path) -> Result<LinearNgramModel, LoadError> {
        decode_container(read_bounded(path)?, Some(path.to_path_buf()))
    }

    /// `_decode_container(data, source_path)`.
    pub fn from_bytes(data: Vec<u8>, source_path: Option<PathBuf>) -> Result<LinearNgramModel, LoadError> {
        decode_container(data, source_path)
    }

    /// `metadata`: training diagnostics, parsed from the manifest on demand.
    #[must_use]
    pub fn metadata(&self) -> Value {
        let manifest: Value = serde_json::from_slice(&self.data[self.manifest_start..self.payload_start]).expect("the manifest was validated");
        manifest.get("metadata").cloned().unwrap_or_else(|| Value::Object(serde_json::Map::new()))
    }

    /// Bytes of the file held in memory.
    #[must_use]
    pub fn resident_bytes(&self) -> usize {
        self.data.len()
    }

    /// `weights[bucket]`.
    #[must_use]
    pub fn weight(&self, bucket: usize) -> i16 {
        let at = self.payload_start + bucket * KSLM_WEIGHT_ENTRY_BYTES;
        i16::from_le_bytes([self.data[at], self.data[at + 1]])
    }

    fn fingerprint(&self, index: usize) -> u64 {
        let at = self.payload_start + self.dimension.to::<usize>() * KSLM_WEIGHT_ENTRY_BYTES + index * KSLM_FINGERPRINT_ENTRY_BYTES;
        u64::from_le_bytes(self.data[at..at + KSLM_FINGERPRINT_ENTRY_BYTES].try_into().expect("eight bytes"))
    }

    /// `bisect_left` and an equality test.
    fn is_supported(&self, fingerprint: u64) -> bool {
        let (mut low, mut high) = (0, self.fingerprint_count);
        while low < high {
            let middle = usize::midpoint(low, high);
            if self.fingerprint(middle) < fingerprint {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        low < self.fingerprint_count && self.fingerprint(low) == fingerprint
    }

    pub fn predict(&self, evidence: &IntentModelInput<'_>) -> Result<LinearPrediction, LoadError> {
        let features = extract_features(evidence, self.dimension, self.fnv_seed, self.membership_seed, &self.ngram_orders)?;
        let mut logit = self.bias;
        for &(bucket, value) in &features.values {
            logit += f64::from(self.weight(bucket.to::<usize>())) * self.weight_scale * value;
        }
        let direction = layout_direction(evidence.source_group, evidence.target_group)?;
        let calibration = self.platt_calibration[direction.index()];
        let calibrated = (calibration.scale * logit) + calibration.bias;
        let probability = stable_sigmoid(calibrated)?;
        let trigger = evidence.trigger.index();
        let threshold_logit = self.threshold_logits[trigger][direction.index()];
        let threshold = self.thresholds[trigger][direction.index()];
        let coverage = if features.character_fingerprints.is_empty() {
            0.0
        } else {
            let supported = features.character_fingerprints.iter().filter(|&&fingerprint| self.is_supported(fingerprint)).count();
            supported.to::<f64>() / features.character_fingerprints.len().to::<f64>()
        };
        Ok(LinearPrediction { logit, probability, threshold, coverage, should_switch: calibrated >= threshold_logit, model_version: self.model_version.clone() })
    }

    /// `LinearNgramModel.try_load_default()`: the override of `KEYSWITCH_INTENT_MODEL_PATH` (given as its value), then
    /// the packaged model of the `resources` folder. The Python's per-process cache lives in `Resources::intent_model`.
    #[must_use]
    pub fn try_load_default(resource_dir: &Path, override_path: Option<&str>) -> (Option<LinearNgramModel>, IntentModelStatus) {
        let packaged = resource_dir.join("models").join("layout_intent_v1.ksm");
        let override_path = pystr::strip(override_path.unwrap_or(""));
        let mut unique: Vec<PathBuf> = Vec::new();
        let candidates = (!override_path.is_empty()).then(|| expanduser(override_path)).into_iter().chain(std::iter::once(packaged));
        for candidate in candidates {
            if !unique.contains(&candidate) {
                unique.push(candidate);
            }
        }
        let mut errors: Vec<String> = Vec::new();
        for candidate in &unique {
            match load_resolved(candidate) {
                Ok(model) => {
                    let status = IntentModelStatus {
                        available: true,
                        path: Some(candidate.clone()),
                        version: Some(model.model_version.clone()),
                        checksum: Some(model.checksum.clone()),
                        error: None,
                    };
                    return (Some(model), status);
                }
                Err(error) => errors.push(format!("{}: {}", candidate.display(), error.message)),
            }
        }
        (None, IntentModelStatus { available: false, path: Some(unique[0].clone()), version: None, checksum: None, error: Some(errors.join("; ")) })
    }
}

/// `Path.expanduser()` for `~` and `~/...`.
fn expanduser(text: &str) -> PathBuf {
    if (text == "~" || text.starts_with("~/"))
        && let Some(home) = std::env::var_os("HOME").filter(|home| !home.is_empty())
    {
        return PathBuf::from(home).join(text[1..].trim_start_matches('/'));
    }
    PathBuf::from(text)
}

/// `_load_cached` without the cache: `resolve(strict=False)`, a `stat`, then the load.
fn load_resolved(path: &Path) -> Result<LinearNgramModel, LoadError> {
    let expanded = expanduser(&path.to_string_lossy());
    let resolved = std::fs::canonicalize(&expanded).unwrap_or_else(|_| {
        if expanded.is_absolute() { expanded.clone() } else { std::env::current_dir().map(|dir| dir.join(&expanded)).unwrap_or(expanded.clone()) }
    });
    std::fs::metadata(&resolved).map_err(|error| LoadError::os(&error, &resolved))?;
    LinearNgramModel::load(&resolved)
}

fn format_error(message: impl Into<String>) -> LoadError {
    LoadError { name: "IntentModelFormatError", message: message.into() }
}

/// `_format_validation`: a `ValueError` of a check reads as a format error.
fn as_format(error: LoadError) -> LoadError {
    format_error(error.message)
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, LoadError> {
    use std::io::Read;
    let maximum = KSLM_MAX_CONTAINER_BYTES;
    let file = std::fs::File::open(path).map_err(|error| LoadError::os(&error, path))?;
    let hint = file.metadata().map_or(0, |meta| meta.len().to::<usize>()).min(maximum) + 1;
    let mut data = Vec::with_capacity(hint);
    file.take(maximum.to::<u64>() + 1).read_to_end(&mut data).map_err(|error| LoadError::os(&error, path))?;
    if data.len() > maximum {
        return Err(format_error("KSLM file exceeds size limit"));
    }
    Ok(data)
}

/// `zlib.crc32`.
pub fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut table = [0u32; 256];
        for (index, slot) in table.iter_mut().enumerate() {
            let mut value = index.to::<u32>();
            for _ in 0..8 {
                value = if value & 1 != 0 { 0xEDB8_8320 ^ (value >> 1) } else { value >> 1 };
            }
            *slot = value;
        }
        table
    });
    !data.iter().fold(!0u32, |crc, &byte| table[((crc ^ u32::from(byte)) & 0xFF).to::<usize>()] ^ (crc >> 8))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn decode_container(data: Vec<u8>, source_path: Option<PathBuf>) -> Result<LinearNgramModel, LoadError> {
    if data.len() < HEADER_SIZE {
        return Err(format_error("truncated KSLM header"));
    }
    let word = |at: usize| u32::from_le_bytes(data[at..at + 4].try_into().expect("four bytes"));
    let half = |at: usize| u16::from_le_bytes(data[at..at + 2].try_into().expect("two bytes"));
    let (magic, schema, flags) = (&data[..4], half(4), half(6));
    let (manifest_length, payload_length, payload_crc) = (word(8).to::<usize>(), word(12).to::<usize>(), word(16));
    let manifest_digest = &data[20..52];
    if magic != MAGIC {
        return Err(format_error("invalid KSLM magic"));
    }
    if i64::from(schema) != KSLM_SCHEMA_VERSION {
        return Err(format_error(format!("unsupported KSLM schema: {schema}")));
    }
    if flags != 0 {
        return Err(format_error("unsupported KSLM header flags"));
    }
    if !(KSLM_MIN_MANIFEST_BYTES..=KSLM_MAX_MANIFEST_BYTES).contains(&manifest_length) {
        return Err(format_error("invalid KSLM manifest length"));
    }
    if payload_length == 0 || payload_length > KSLM_MAX_PAYLOAD_BYTES {
        return Err(format_error("invalid KSLM payload length"));
    }
    if data.len() != HEADER_SIZE + manifest_length + payload_length {
        return Err(format_error("KSLM file length does not match header"));
    }
    let (manifest_start, payload_start) = (HEADER_SIZE, HEADER_SIZE + manifest_length);
    let manifest_bytes = &data[manifest_start..payload_start];
    let payload = &data[payload_start..];
    if Sha256::digest(manifest_bytes).as_slice() != manifest_digest {
        return Err(format_error("KSLM manifest checksum mismatch"));
    }
    if crc32(payload) != payload_crc {
        return Err(format_error("KSLM payload CRC32 mismatch"));
    }
    let Manifest {
        dimension,
        fingerprint_count,
        fnv_seed,
        membership_seed,
        orders,
        version,
        weight_scale,
        bias,
        platt,
        veto_threshold,
        threshold_logits,
        payload_sha,
    } = read_manifest(manifest_bytes)?;
    if sha256_hex(payload) != payload_sha {
        return Err(format_error("KSLM payload SHA256 mismatch"));
    }

    if payload_length != dimension * KSLM_WEIGHT_ENTRY_BYTES + fingerprint_count * KSLM_FINGERPRINT_ENTRY_BYTES {
        return Err(format_error("KSLM payload shape does not match dimension"));
    }
    let fingerprints = &payload[dimension * KSLM_WEIGHT_ENTRY_BYTES..];
    let entry = |index: usize| u64::from_le_bytes(fingerprints[index * 8..index * 8 + 8].try_into().expect("eight bytes"));
    if (1..fingerprint_count).any(|index| entry(index - 1) >= entry(index)) {
        return Err(format_error("KSLM supported fingerprints must be sorted and unique"));
    }

    let checksum = sha256_hex(&data);
    let mut thresholds = [[0.0; 2]; 6];
    for (trigger, row) in threshold_logits.iter().enumerate() {
        for (direction, logit) in row.iter().enumerate() {
            thresholds[trigger][direction] = stable_sigmoid(*logit)?;
        }
    }
    Ok(LinearNgramModel {
        dimension: dimension.to::<i64>(),
        data,
        manifest_start,
        payload_start,
        fingerprint_count,
        weight_scale,
        bias,
        platt_calibration: platt,
        threshold_logits,
        thresholds,
        veto_threshold,
        model_version: version,
        fnv_seed,
        membership_seed,
        ngram_orders: orders,
        payload_sha256: payload_sha,
        checksum,
        source_path,
    })
}

/// What the manifest says, after every check of it.
struct Manifest {
    dimension: usize,
    fingerprint_count: usize,
    fnv_seed: u64,
    membership_seed: u64,
    orders: [i64; 5],
    version: String,
    weight_scale: f64,
    bias: f64,
    platt: [PlattParameters; 2],
    veto_threshold: f64,
    threshold_logits: [[f64; 2]; 6],
    payload_sha: String,
}

/// Parses and checks the manifest (`_decode_container` from the JSON on).
fn read_manifest(manifest_bytes: &[u8]) -> Result<Manifest, LoadError> {
    let not_json = || format_error("KSLM manifest is not valid UTF-8 JSON");
    let text = std::str::from_utf8(manifest_bytes).map_err(|_| not_json())?;
    if !text.trim_start_matches([' ', '\t', '\n', '\r']).starts_with('{') {
        serde_json::from_str::<IgnoredAny>(text).map_err(|_| not_json())?;
        return Err(format_error("manifest must be an object"));
    }
    let mut deserializer = serde_json::Deserializer::from_str(text);
    let parts = deserializer.deserialize_map(PartsVisitor).map_err(|_| not_json())?;
    deserializer.end().map_err(|_| not_json())?;
    let Parts { map: manifest, metadata_position, metadata_error, metadata_object } = parts;
    // `_validated_json_object(manifest)`: the first fault in the order of the keys, the metadata where its key stands.
    for (position, (key, item)) in manifest.iter().enumerate() {
        if metadata_position == Some(position)
            && let Some(error) = &metadata_error
        {
            return Err(error.clone());
        }
        validated_json_value(item, &format!("manifest.{key}"), 0)?;
    }
    if metadata_position == Some(manifest.len())
        && let Some(error) = &metadata_error
    {
        return Err(error.clone());
    }
    if !is_canonical(manifest_bytes) {
        return Err(format_error("KSLM manifest is not canonical JSON"));
    }

    const REQUIRED: [&str; 17] = [
        "bias", "dimension", "feature_version", "fnv_seed", "format", "hash_algorithm", "membership_algorithm", "membership_seed", "model_version",
        "ngram_orders", "payload_sha256", "platt_calibration", "schema", "supported_fingerprint_count", "threshold_logits", "veto_threshold",
        "weight_scale",
    ];
    // The metadata is not in the map: the map holds the other keys, which must be exactly the required ones.
    if manifest.len() != REQUIRED.len() || !REQUIRED.iter().all(|key| manifest.contains_key(*key)) {
        return Err(format_error("KSLM manifest fields do not match schema"));
    }
    let manifest_schema = manifest_int(&manifest["schema"], "schema")?;
    if manifest["format"] != "KSLM" || manifest_schema != i128::from(KSLM_SCHEMA_VERSION) {
        return Err(format_error("KSLM manifest format/schema mismatch"));
    }
    if manifest_int(&manifest["feature_version"], "feature_version")? != i128::from(INTENT_FEATURE_VERSION) {
        return Err(format_error("unsupported KSLM feature version"));
    }
    if manifest["hash_algorithm"] != HASH_ALGORITHM {
        return Err(format_error("unsupported KSLM hash algorithm"));
    }
    if manifest["membership_algorithm"] != MEMBERSHIP_ALGORITHM {
        return Err(format_error("unsupported KSLM membership algorithm"));
    }

    let dimension = manifest_int(&manifest["dimension"], "dimension")?;
    validate_dimension_int(dimension).map_err(as_format)?;
    let fnv_seed = validate_seed_int(manifest_int(&manifest["fnv_seed"], "fnv_seed")?).map_err(as_format)?;
    let membership_seed = validate_seed_int(manifest_int(&manifest["membership_seed"], "membership_seed")?).map_err(as_format)?;
    validate_membership_seed(membership_seed, fnv_seed).map_err(as_format)?;
    let fingerprint_count = manifest_int(&manifest["supported_fingerprint_count"], "supported_fingerprint_count")?;
    if !(0..=KSLM_MAX_FINGERPRINTS.to::<i128>()).contains(&fingerprint_count) {
        return Err(format_error("supported_fingerprint_count exceeds KSLM size limit"));
    }
    let orders_value = manifest["ngram_orders"].as_array().ok_or_else(|| format_error("ngram_orders must be an array"))?;
    let orders = validate_orders_values(orders_value).map_err(as_format)?;
    let version = validate_version(&manifest["model_version"]).map_err(as_format)?;
    let weight_scale = manifest_float(&manifest["weight_scale"], "weight_scale")?;
    if weight_scale <= 0.0 {
        return Err(format_error("weight_scale must be positive"));
    }
    let bias = manifest_float(&manifest["bias"], "bias")?;
    let platt = validate_platt_calibration(as_mapping(&manifest["platt_calibration"], "platt_calibration")?).map_err(as_format)?;
    let veto_threshold = manifest_float(&manifest["veto_threshold"], "veto_threshold")?;
    let threshold_logits = validate_threshold_logits(as_mapping(&manifest["threshold_logits"], "threshold_logits")?).map_err(as_format)?;
    if metadata_object == Some(false) {
        return Err(format_error("metadata must be an object"));
    }

    let payload_sha = match &manifest["payload_sha256"] {
        Value::String(text) if text.len() == SHA256_HEX_CHARACTERS && text.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) => text.clone(),
        _ => return Err(format_error("payload_sha256 must be lowercase hexadecimal")),
    };

    Ok(Manifest {
        dimension: dimension.to::<usize>(),
        fingerprint_count: fingerprint_count.to::<usize>(),
        fnv_seed,
        membership_seed,
        orders,
        version,
        weight_scale,
        bias,
        platt,
        veto_threshold,
        threshold_logits,
        payload_sha,
    })
}


/// What the streaming read of the manifest keeps: every key but `metadata` as a value; of the metadata only whether it
/// is an object and the first fault `_validated_json_value` finds in it. Its tree (half a megabyte of text, four of
/// values) is never built: the loaded model would keep that memory for as long as it lives.
struct Parts {
    map: serde_json::Map<String, Value>,
    metadata_position: Option<usize>,
    metadata_error: Option<LoadError>,
    metadata_object: Option<bool>,
}

struct PartsVisitor;

impl<'de> Visitor<'de> for PartsVisitor {
    type Value = Parts;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a JSON object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Parts, A::Error> {
        let mut parts = Parts { map: serde_json::Map::new(), metadata_position: None, metadata_error: None, metadata_object: None };
        while let Some(key) = access.next_key::<String>()? {
            if key == "metadata" {
                let error = RefCell::new(None);
                let object = access.next_value_seed(MetadataSeed { error: &error })?;
                parts.metadata_position = Some(parts.map.len());
                parts.metadata_error = error.into_inner();
                parts.metadata_object = Some(object);
            } else {
                let value: Value = access.next_value()?;
                parts.map.insert(key, value);
            }
        }
        Ok(parts)
    }
}

struct MetadataSeed<'a> {
    error: &'a RefCell<Option<LoadError>>,
}

impl<'de> DeserializeSeed<'de> for MetadataSeed<'_> {
    type Value = bool;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<bool, D::Error> {
        let walker = MetadataWalker { error: self.error, field: "manifest.metadata".to_string(), depth: 0 };
        deserializer.deserialize_any(walker)
    }
}

/// `_validated_json_value`, as a visitor: the value is the answer to "is it an object".
struct MetadataWalker<'a> {
    error: &'a RefCell<Option<LoadError>>,
    field: String,
    depth: usize,
}

impl MetadataWalker<'_> {
    /// Records the first fault; true when the value may be walked on.
    fn enter(&self) -> bool {
        let mut error = self.error.borrow_mut();
        if error.is_none() && self.depth > INTENT_METADATA_MAX_DEPTH {
            *error = Some(format_error(format!("{} exceeds JSON nesting limit", self.field)));
        }
        error.is_none()
    }
}

impl<'de> Visitor<'de> for MetadataWalker<'_> {
    type Value = bool;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_bool<E>(self, _: bool) -> Result<bool, E> {
        self.enter();
        Ok(false)
    }

    fn visit_i64<E>(self, _: i64) -> Result<bool, E> {
        self.enter();
        Ok(false)
    }

    fn visit_u64<E>(self, _: u64) -> Result<bool, E> {
        self.enter();
        Ok(false)
    }

    fn visit_f64<E>(self, value: f64) -> Result<bool, E> {
        if self.enter() && !value.is_finite() {
            *self.error.borrow_mut() = Some(format_error(format!("{} must be finite", self.field)));
        }
        Ok(false)
    }

    fn visit_str<E>(self, _: &str) -> Result<bool, E> {
        self.enter();
        Ok(false)
    }

    fn visit_unit<E>(self) -> Result<bool, E> {
        self.enter();
        Ok(false)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut access: A) -> Result<bool, A::Error> {
        if self.enter() {
            let field = format!("{}[]", self.field);
            while access.next_element_seed(MetadataItem { error: self.error, field: &field, depth: self.depth + 1 })?.is_some() {}
        } else {
            while access.next_element::<IgnoredAny>()?.is_some() {}
        }
        Ok(false)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<bool, A::Error> {
        let walk = self.enter();
        while let Some(key) = access.next_key::<String>()? {
            if walk {
                let field = format!("{}.{key}", self.field);
                access.next_value_seed(MetadataItem { error: self.error, field: &field, depth: self.depth + 1 })?;
            } else {
                access.next_value::<IgnoredAny>()?;
            }
        }
        Ok(true)
    }
}

struct MetadataItem<'a> {
    error: &'a RefCell<Option<LoadError>>,
    field: &'a str,
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for MetadataItem<'_> {
    type Value = bool;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<bool, D::Error> {
        deserializer.deserialize_any(MetadataWalker { error: self.error, field: self.field.to_string(), depth: self.depth })
    }
}

/// Whether the bytes are what `json.dumps(json.loads(text), ensure_ascii=False, sort_keys=True, separators=(",", ":"))`
/// writes. The text has been parsed already, so only its form is looked at: no blank, keys in strictly increasing
/// order of code points, the escapes of Python and no others, integers without sign or zeros in front, floats as `repr`.
fn is_canonical(text: &[u8]) -> bool {
    let mut at = 0;
    canonical_value(text, &mut at) && at == text.len()
}

fn canonical_value(text: &[u8], at: &mut usize) -> bool {
    match text.get(*at) {
        Some(b'{') => {
            *at += 1;
            if text.get(*at) == Some(&b'}') {
                *at += 1;
                return true;
            }
            let mut previous: Option<String> = None;
            loop {
                let Some(key) = canonical_string(text, at) else {
                    return false;
                };
                if previous.as_ref().is_some_and(|previous| *previous >= key) || text.get(*at) != Some(&b':') {
                    return false;
                }
                *at += 1;
                previous = Some(key);
                if !canonical_value(text, at) {
                    return false;
                }
                match text.get(*at) {
                    Some(b',') => *at += 1,
                    Some(b'}') => {
                        *at += 1;
                        return true;
                    }
                    _ => return false,
                }
            }
        }
        Some(b'[') => {
            *at += 1;
            if text.get(*at) == Some(&b']') {
                *at += 1;
                return true;
            }
            loop {
                if !canonical_value(text, at) {
                    return false;
                }
                match text.get(*at) {
                    Some(b',') => *at += 1,
                    Some(b']') => {
                        *at += 1;
                        return true;
                    }
                    _ => return false,
                }
            }
        }
        Some(b'"') => canonical_string(text, at).is_some(),
        Some(b't') => take_literal(text, at, b"true"),
        Some(b'f') => take_literal(text, at, b"false"),
        Some(b'n') => take_literal(text, at, b"null"),
        Some(b'-' | b'0'..=b'9') => {
            let start = *at;
            while text.get(*at).is_some_and(|byte| matches!(byte, b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')) {
                *at += 1;
            }
            let token = std::str::from_utf8(&text[start..*at]).expect("ASCII");
            if token.contains(['.', 'e', 'E']) {
                token.parse::<f64>().is_ok_and(|number| ks_text::float_repr(number) == token)
            } else {
                let digits = token.strip_prefix('-').unwrap_or(token);
                token == "0" || (!digits.is_empty() && !digits.starts_with('0') && digits.bytes().all(|byte| byte.is_ascii_digit()))
            }
        }
        _ => false,
    }
}

fn take_literal(text: &[u8], at: &mut usize, literal: &[u8]) -> bool {
    let found = text[*at..].starts_with(literal);
    *at += literal.len();
    found
}

/// A string as Python writes it with `ensure_ascii=False`; its decoded text.
fn canonical_string(text: &[u8], at: &mut usize) -> Option<String> {
    if text.get(*at) != Some(&b'"') {
        return None;
    }
    *at += 1;
    let mut decoded: Vec<u8> = Vec::new();
    loop {
        let byte = *text.get(*at)?;
        *at += 1;
        match byte {
            b'"' => break,
            0..=0x1f => return None,
            b'\\' => {
                let escape = *text.get(*at)?;
                *at += 1;
                match escape {
                    b'"' | b'\\' => decoded.push(escape),
                    b'n' => decoded.push(b'\n'),
                    b'r' => decoded.push(b'\r'),
                    b't' => decoded.push(b'\t'),
                    b'b' => decoded.push(0x08),
                    b'f' => decoded.push(0x0c),
                    b'u' => {
                        let digits = text.get(*at..*at + 4)?;
                        *at += 4;
                        let code = u32::from_str_radix(std::str::from_utf8(digits).ok()?, 16).ok()?;
                        // Only the control characters Python has no short escape for, in lower case.
                        if code >= 0x20 || matches!(code, 0x08 | 0x09 | 0x0a | 0x0c | 0x0d) || digits.iter().any(u8::is_ascii_uppercase) || digits[..2] != *b"00" {
                            return None;
                        }
                        decoded.push(code.to::<u8>());
                    }
                    _ => return None,
                }
            }
            _ => decoded.push(byte),
        }
    }
    String::from_utf8(decoded).ok()
}

// ---------------------------------------------------------------- validation

fn validate_dimension(dimension: i64) -> Result<(), LoadError> {
    validate_dimension_int(i128::from(dimension))
}

fn validate_dimension_int(dimension: i128) -> Result<(), LoadError> {
    if dimension < 1 || dimension > i128::from(KSLM_MAX_DIMENSION) || dimension & (dimension - 1) != 0 {
        return Err(LoadError::value(format!("model dimension must be a power of two up to 2^{KSLM_MAX_DIMENSION_LOG2}")));
    }
    Ok(())
}

fn validate_seed_int(seed: i128) -> Result<u64, LoadError> {
    u64::try_from(seed).map_err(|_| LoadError::value("FNV seed must be an unsigned 64-bit integer"))
}

fn validate_membership_seed(seed: u64, feature_hash_seed: u64) -> Result<(), LoadError> {
    if seed == feature_hash_seed {
        return Err(LoadError::value("membership_seed must differ from fnv_seed"));
    }
    Ok(())
}

fn validate_orders(orders: &[i64]) -> Result<Vec<i64>, LoadError> {
    if orders != INTENT_NGRAM_ORDERS {
        return Err(LoadError::value("KSLM v4 requires n-gram orders 1, 2, 3, 4, 5"));
    }
    Ok(orders.to_vec())
}

fn validate_orders_values(values: &[Value]) -> Result<[i64; 5], LoadError> {
    let mut parsed: Vec<i64> = Vec::new();
    for order in values {
        match order {
            Value::Number(number) if number.is_i64() || number.is_u64() => parsed.push(number.as_i64().unwrap_or(i64::MAX)),
            _ => return Err(LoadError::value("n-gram orders must be integers")),
        }
    }
    validate_orders(&parsed)?;
    Ok(INTENT_NGRAM_ORDERS)
}

fn validate_version(value: &Value) -> Result<String, LoadError> {
    match value {
        Value::String(text) if !text.is_empty() && pystr::len(text) <= INTENT_MODEL_VERSION_MAX_CHARACTERS && pystr::isprintable(text) => Ok(text.clone()),
        _ => Err(LoadError::value("model_version must be a short printable string")),
    }
}

/// `_model_float`: a number that is finite and bounded.
fn model_float(value: &Value, field: &str) -> Result<f64, LoadError> {
    let parsed = match value {
        Value::Number(number) => number.as_f64().expect("a JSON number reads as f64"),
        _ => return Err(LoadError::value(format!("{field} must be numeric"))),
    };
    if !parsed.is_finite() || parsed.abs() > INTENT_MAX_MODEL_FLOAT_MAGNITUDE {
        return Err(LoadError::value(format!("{field} must be finite and bounded")));
    }
    Ok(parsed)
}

fn manifest_float(value: &Value, field: &str) -> Result<f64, LoadError> {
    model_float(value, field).map_err(as_format)
}

fn manifest_int(value: &Value, field: &str) -> Result<i128, LoadError> {
    match value {
        Value::Number(number) if number.is_i64() => Ok(i128::from(number.as_i64().expect("checked"))),
        Value::Number(number) if number.is_u64() => Ok(i128::from(number.as_u64().expect("checked"))),
        _ => Err(format_error(format!("{field} must be an integer"))),
    }
}

fn as_mapping<'a>(value: &'a Value, field: &str) -> Result<&'a serde_json::Map<String, Value>, LoadError> {
    value.as_object().ok_or_else(|| format_error(format!("{field} must be an object")))
}

fn validate_threshold_logits(values: &serde_json::Map<String, Value>) -> Result<[[f64; 2]; 6], LoadError> {
    if values.len() != TRIGGERS.len() || !TRIGGERS.iter().all(|trigger| values.contains_key(trigger.as_str())) {
        return Err(LoadError::value("threshold_logits must contain every correction trigger"));
    }
    let mut result = [[0.0; 2]; 6];
    for (index, trigger) in TRIGGERS.iter().enumerate() {
        let name = trigger.as_str();
        let raw = values[name].as_object().ok_or_else(|| LoadError::value(format!("threshold_logits.{name} must be an object")))?;
        if raw.len() != LAYOUT_DIRECTIONS.len() || !LAYOUT_DIRECTIONS.iter().all(|direction| raw.contains_key(direction.as_str())) {
            return Err(LoadError::value(format!("threshold_logits.{name} must contain both directions")));
        }
        for direction in LAYOUT_DIRECTIONS {
            let key = direction.as_str();
            result[index][direction.index()] = model_float(&raw[key], &format!("threshold_logits.{name}.{key}"))?;
        }
    }
    Ok(result)
}

fn validate_platt_calibration(values: &serde_json::Map<String, Value>) -> Result<[PlattParameters; 2], LoadError> {
    if values.len() != LAYOUT_DIRECTIONS.len() || !LAYOUT_DIRECTIONS.iter().all(|direction| values.contains_key(direction.as_str())) {
        return Err(LoadError::value("platt_calibration must contain both EN/RU directions"));
    }
    let mut result = [PlattParameters { scale: 1.0, bias: 0.0 }; 2];
    for direction in LAYOUT_DIRECTIONS {
        let key = direction.as_str();
        let raw = values[key].as_object().ok_or_else(|| LoadError::value(format!("platt_calibration.{key} must be an object")))?;
        if raw.len() != 2 || !raw.contains_key("scale") || !raw.contains_key("bias") {
            return Err(LoadError::value(format!("platt_calibration.{key} fields do not match schema")));
        }
        let scale = model_float(&raw["scale"], &format!("platt_calibration.{key}.scale"))?;
        if scale <= 0.0 {
            return Err(LoadError::value(format!("platt_calibration.{key}.scale must be positive")));
        }
        let bias = model_float(&raw["bias"], &format!("platt_calibration.{key}.bias"))?;
        result[direction.index()] = PlattParameters { scale, bias };
    }
    Ok(result)
}

fn validated_json_value(value: &Value, field: &str, depth: usize) -> Result<(), LoadError> {
    if depth > INTENT_METADATA_MAX_DEPTH {
        return Err(format_error(format!("{field} exceeds JSON nesting limit")));
    }
    match value {
        Value::Number(number) if number.is_i64() || number.is_u64() => {
            let integer = number.as_i64().map_or_else(|| i128::from(number.as_u64().expect("an integer")), i128::from);
            if !(INT64_MIN..=UINT64_MAX).contains(&integer) {
                return Err(format_error(format!("{field} integer is out of range")));
            }
        }
        Value::Number(number) => {
            if !number.as_f64().is_some_and(f64::is_finite) {
                return Err(format_error(format!("{field} must be finite")));
            }
        }
        Value::Array(items) => {
            for item in items {
                validated_json_value(item, &format!("{field}[]"), depth + 1)?;
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                validated_json_value(item, &format!("{field}.{key}"), depth + 1)?;
            }
        }
        Value::Null | Value::Bool(_) | Value::String(_) => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::constants::{FNV1A64_OFFSET_BASIS};
    use super::*;

    #[test]
    fn crc_and_fnv_match_the_references() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(fnv1a64("", FNV1A64_OFFSET_BASIS), FNV1A64_OFFSET_BASIS);
        assert_eq!(fnv1a64("a", FNV1A64_OFFSET_BASIS), 0xAF63_DC4C_8601_EC8C);
    }

    #[test]
    fn length_buckets_and_directions() {
        assert_eq!(length_bucket(4), "4");
        assert_eq!(length_bucket(5), "5-7");
        assert_eq!(length_bucket(12), "12-19");
        assert_eq!(length_bucket(20), "20+");
        assert!(layout_direction(0, 0).is_err());
        assert_eq!(layout_direction(1, 0).unwrap().as_str(), "1>0");
        assert_eq!(stable_sigmoid(0.0).unwrap(), 0.5);
        assert!(stable_sigmoid(f64::NAN).is_err());
    }

    #[test]
    fn a_short_file_is_a_truncated_header() {
        assert_eq!(LinearNgramModel::from_bytes(vec![0; 10], None).err().unwrap().message, "truncated KSLM header");
    }
}
