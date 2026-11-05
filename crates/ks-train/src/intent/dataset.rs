//! The frozen lexical dataset of the layout-intent model (`train_intent_model`): the Onboard words, their physical key
//! signatures and immutable splits, the symmetric typos, the quarantine of generated signatures that cross splits or
//! languages, the hard-negative development rows and the fingerprints every later step is bound to.
use ks_cast::To as _;
use crate::physical::{hex, pair};
use ks_lang::detector::{ProtectedTokens, is_protected_token};
use ks_lang::intent_model::{CorrectionTrigger, TRIGGERS};
use ks_text::pyjson::{json_float, json_string};
use ks_text::pystr;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

pub const SPLIT_NAMESPACE: &str = "keyswitch:intent-v28:physical-signature";
pub const HARD_NEGATIVE_POLICY: &str = "keyswitch-intent-v28-frozen-unknown-typo-development";
const RANK_NAMESPACE: &str = "keyswitch:intent-v1:unknown-typo-rank";
const CHOICE_NAMESPACE: &str = "keyswitch:intent-v1:unknown-typo-choice";
pub const ROLE_NAMESPACE: &str = "keyswitch:intent-v28:unknown-typo-development-role";
const DRAW_DIGEST_BYTES: usize = 8;
const SPLIT_BUCKET_COUNT: u64 = 40;
const TYPO_MIN_SIGNATURE_CHARACTERS: usize = 3;
const TYPO_EDGE_CHARACTERS: usize = 2;
const EXAMPLE_FREQUENCY_WEIGHT_CAP: f64 = 2.0;
const EXAMPLE_FREQUENCY_LOG_DIVISOR: f64 = 8.0;
const PROTECTED_TOKEN_EXAMPLE_WEIGHT: f64 = 2.0;
const LEXICAL_COLLISION_EXAMPLE_WEIGHT: f64 = 3.0;
const TRAIN_ONLY_SCORER_ALGORITHM_VERSION: i64 = 2;
const LANGUAGE_MODEL_NGRAM_ORDERS: [i64; 3] = [2, 3, 4];
const LEAKAGE_ERROR_EXAMPLE_COUNT: usize = 3;
const CORPUS_STRING_MAX_CHARACTERS: usize = 256;
const ASCII_CONTROL_CHARACTER_LIMIT: u32 = 0x20;
const TRAINING_HARD_NEGATIVES: [&str; 10] = ["abc123", "api_v2", "camelCase", "docker-compose", "foo@example.com", "https://example.com", "KEYSWITCH_DEBUG", "localhost:8080", "python3.12", "user_name"];
const SAFETY_HARD_NEGATIVES: [&str; 6] = ["--force-with-lease", "/usr/bin/python3", "127.0.0.1", "C:\\Windows\\System32", "feature/intent-model-v1", "sha256:deadbeef"];
const AUGMENTED_KINDS: [&str; 4] = ["identity", "deletion", "duplication", "transposition"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Split {
    Train,
    Development,
    Calibration,
    Threshold,
    Test,
}

impl Split {
    pub const ALL: [Split; 5] = [Split::Train, Split::Development, Split::Calibration, Split::Threshold, Split::Test];
    pub const PRESEALED: [Split; 4] = [Split::Train, Split::Development, Split::Calibration, Split::Threshold];

    pub fn as_str(self) -> &'static str {
        match self {
            Split::Train => "train",
            Split::Development => "development",
            Split::Calibration => "calibration",
            Split::Threshold => "threshold",
            Split::Test => "test",
        }
    }

    pub(crate) fn index(self) -> usize {
        match self {
            Self::Train => 0,
            Self::Development => 1,
            Self::Calibration => 2,
            Self::Threshold => 3,
            Self::Test => 4,
        }
    }
}

const SPLIT_WIDTHS: [(Split, u64); 5] = [(Split::Train, 26), (Split::Development, 4), (Split::Calibration, 4), (Split::Threshold, 3), (Split::Test, 3)];

fn sha(parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().into()
}

fn draw(digest: &[u8; 32]) -> u64 {
    u64::from_be_bytes(digest[..DRAW_DIGEST_BYTES].try_into().expect("eight bytes"))
}

/// `stable_split`: a physical signature's immutable bucket.
pub fn stable_split(signature: &str) -> Split {
    let bucket = draw(&sha(&[SPLIT_NAMESPACE.as_bytes(), b"\0", signature.as_bytes()])) % SPLIT_BUCKET_COUNT;
    let mut cursor = 0;
    for (name, width) in SPLIT_WIDTHS {
        cursor += width;
        if bucket < cursor {
            return name;
        }
    }
    unreachable!("the split table covers all buckets")
}

fn training_trigger(signature: &str) -> CorrectionTrigger {
    let digest = sha(&[b"keyswitch:intent-v1:training-trigger\0", signature.as_bytes()]);
    TRIGGERS[(draw(&digest) % TRIGGERS.len().to::<u64>()).to::<usize>()]
}

fn translate(text: &str, source: &str, target: &str) -> String {
    pair().translate(text, source, target).expect("known layouts")
}

/// `_normalise_lexical_word`.
pub fn normalise_lexical_word(word: &str, group: i64) -> String {
    let folded = pystr::casefold(word);
    let normalized = pystr::strip(&folded);
    let fits = |character: char| match group {
        0 => character.is_ascii_lowercase() || character == '\'' || character == '-',
        _ => ('а'..='я').contains(&character) || character == 'ё' || character == '\'' || character == '-',
    };
    if !normalized.is_empty() && normalized.chars().all(fits) { normalized.to_owned() } else { String::new() }
}

/// `physical_signature`: the layout-independent physical key sequence of a word.
pub fn physical_signature(word: &str, group: i64) -> String {
    let normalized = normalise_lexical_word(word, group);
    if normalized.is_empty() || group == 0 { normalized } else { translate(&normalized, "ru", "us") }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LexiconWord {
    pub word: String,
    pub group: i64,
    pub frequency: i64,
    pub physical_signature: String,
    pub split: Split,
}

/// `load_onboard_unigrams`: the deterministic filter of the one-gram section of an Onboard model.
pub fn load_onboard_unigrams(raw: &[u8], group: i64, minimum_length: usize, maximum_length: usize) -> Vec<LexiconWord> {
    let text = String::from_utf8_lossy(raw);
    let mut frequencies: HashMap<String, i64> = HashMap::new();
    let mut section = false;
    for line in pystr::splitlines(&text, false) {
        if line == r"\1-grams:" {
            section = true;
            continue;
        }
        if section && line.starts_with('\\') {
            break;
        }
        if !section || line.is_empty() {
            continue;
        }
        let Some((count_text, token)) = line.split_once(' ') else { continue };
        if token.starts_with('<') {
            continue;
        }
        let Some(frequency) = ks_text::pynum::py_int(count_text) else { continue };
        let normalized = normalise_lexical_word(token, group);
        if !(minimum_length..=maximum_length).contains(&normalized.chars().count()) || physical_signature(&normalized, group).is_empty() {
            continue;
        }
        *frequencies.entry(normalized).or_insert(0) += (frequency as i64).max(1);
    }
    let mut ranked: Vec<(String, i64)> = frequencies.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked
        .into_iter()
        .map(|(word, frequency)| {
            let signature = physical_signature(&word, group);
            let split = stable_split(&signature);
            LexiconWord { word, group, frequency, physical_signature: signature, split }
        })
        .collect()
}

pub struct LexiconCollision {
    pub physical_signature: String,
    pub candidates: Vec<LexiconWord>,
}

pub struct PreparedLexicon {
    pub words_by_split: Vec<Vec<LexiconWord>>,
    pub collisions: Vec<LexiconCollision>,
}

/// `prepare_lexicon`: deduplicate train words while retaining all safety collisions.
pub fn prepare_lexicon(words: Vec<LexiconWord>, minimum_training_signature_length: usize) -> PreparedLexicon {
    let mut grouped: BTreeMap<String, Vec<LexiconWord>> = BTreeMap::new();
    for word in words {
        if !word.physical_signature.is_empty() {
            grouped.entry(word.physical_signature.clone()).or_default().push(word);
        }
    }
    let mut by_split: Vec<Vec<LexiconWord>> = vec![Vec::new(); Split::ALL.len()];
    let mut collisions = Vec::new();
    for (signature, mut candidates) in grouped {
        let groups: BTreeSet<i64> = candidates.iter().map(|candidate| candidate.group).collect();
        if groups.len() > 1 {
            candidates.sort_by(|a, b| a.group.cmp(&b.group).then(b.frequency.cmp(&a.frequency)).then_with(|| a.word.cmp(&b.word)));
            collisions.push(LexiconCollision { physical_signature: signature, candidates });
            continue;
        }
        if signature.chars().count() < minimum_training_signature_length {
            continue;
        }
        let selected = candidates.into_iter().min_by(|a, b| b.frequency.cmp(&a.frequency).then_with(|| a.word.cmp(&b.word))).expect("a word");
        by_split[selected.split.index()].push(selected);
    }
    for words in &mut by_split {
        words.sort_by(|a, b| a.physical_signature.cmp(&b.physical_signature).then(a.group.cmp(&b.group)));
    }
    PreparedLexicon { words_by_split: by_split, collisions }
}

/// `typo_variants`: symmetric physical-key typos made without consulting either language.
pub fn typo_variants(signature: &str, maximum_augmentations: usize) -> Vec<(String, &'static str)> {
    let mut variants = vec![(signature.to_owned(), "identity")];
    let keys: Vec<char> = signature.chars().collect();
    if maximum_augmentations == 0 || keys.len() < TYPO_MIN_SIGNATURE_CHARACTERS {
        return variants;
    }
    let digest = sha(&[b"keyswitch:intent-v1:typo\0", signature.as_bytes()]);
    let interior = 1 + usize::from(digest[0]) % (keys.len() - TYPO_EDGE_CHARACTERS).max(1);
    let piece = |range: std::ops::Range<usize>| -> String { keys[range].iter().collect() };
    let (head, at, next) = (piece(0..interior), keys[interior], keys[interior + 1]);
    let candidates = [
        (format!("{head}{}", piece(interior + 1..keys.len())), "deletion"),
        (format!("{head}{at}{}", piece(interior..keys.len())), "duplication"),
        (format!("{head}{next}{at}{}", piece(interior + 2..keys.len())), "transposition"),
    ];
    let mut seen: HashSet<String> = HashSet::from([signature.to_owned()]);
    for (candidate, kind) in candidates {
        if variants.len() > maximum_augmentations || seen.contains(&candidate) {
            continue;
        }
        seen.insert(candidate.clone());
        variants.push((candidate, kind));
    }
    variants
}

#[derive(Clone, Debug, PartialEq)]
pub struct LexicalExample {
    pub original: String,
    pub alternative: String,
    pub source_group: i64,
    pub target_group: i64,
    pub trigger: CorrectionTrigger,
    pub label: bool,
    pub weight: f64,
    pub base_signature: String,
    pub variant_kind: String,
    pub source_known: bool,
    pub target_known: bool,
    pub context_delta: f64,
    pub context_group: Option<i64>,
    pub frequency: i64,
    pub protected: bool,
    pub safety: bool,
}

impl LexicalExample {
    /// `json.dumps(asdict(example), ensure_ascii=False, sort_keys=True, separators=(",", ":"))`.
    pub fn canonical(&self) -> String {
        let group = self.context_group.map_or_else(|| "null".to_owned(), |group| group.to_string());
        format!(
            "{{\"alternative\":{},\"base_signature\":{},\"context_delta\":{},\"context_group\":{group},\"frequency\":{},\"label\":{},\"original\":{},\"protected\":{},\"safety\":{},\"source_group\":{},\"source_known\":{},\"target_group\":{},\"target_known\":{},\"trigger\":\"{}\",\"variant_kind\":{},\"weight\":{}}}",
            json_string(&self.alternative, false),
            json_string(&self.base_signature, false),
            json_float(self.context_delta),
            self.frequency,
            self.label,
            json_string(&self.original, false),
            self.protected,
            self.safety,
            self.source_group,
            self.source_known,
            self.target_group,
            self.target_known,
            self.trigger.as_str(),
            json_string(&self.variant_kind, false),
            json_float(self.weight),
        )
    }

    /// The key of the rows of a split.
    fn split_order<'a>(&'a self) -> (&'a str, &'a str, &'static str, bool, i64) {
        (&self.base_signature, &self.variant_kind, self.trigger.as_str(), self.label, self.source_group)
    }

    fn safety_order<'a>(&'a self) -> (&'a str, &'static str, i64) {
        (&self.base_signature, self.trigger.as_str(), self.source_group)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct QuarantinedOccurrence {
    pub physical_signature: String,
    pub split: &'static str,
    pub base_signature: String,
    pub group: i64,
    pub variant_kind: &'static str,
    pub reason: &'static str,
}

impl QuarantinedOccurrence {
    fn canonical(&self) -> String {
        format!(
            "{{\"base_signature\":{},\"group\":{},\"physical_signature\":{},\"reason\":\"{}\",\"split\":\"{}\",\"variant_kind\":\"{}\"}}",
            json_string(&self.base_signature, false),
            self.group,
            json_string(&self.physical_signature, false),
            self.reason,
            self.split,
            self.variant_kind
        )
    }
}

#[derive(Clone, Debug, Default)]
pub struct VariantQuarantine {
    pub occurrences: Vec<QuarantinedOccurrence>,
    pub sha256: String,
}

impl VariantQuarantine {
    pub fn signature_count(&self) -> usize {
        self.occurrences.iter().map(|item| &item.physical_signature).collect::<HashSet<_>>().len()
    }
}

/// `variant_quarantine_fingerprint`: the hash of a canonical, order-independent form.
fn quarantine_fingerprint(occurrences: &[QuarantinedOccurrence]) -> String {
    let mut sorted: Vec<&QuarantinedOccurrence> = occurrences.iter().collect();
    sorted.sort();
    let mut hasher = Sha256::new();
    hasher.update(b"keyswitch:intent-v1:variant-quarantine\0");
    for occurrence in sorted {
        hasher.update(occurrence.canonical().as_bytes());
        hasher.update(b"\n");
    }
    hex(&hasher.finalize())
}

fn quarantine_reason(splits: &BTreeSet<&'static str>, groups: &BTreeSet<i64>) -> Option<&'static str> {
    match (splits.len() > 1, groups.len() > 1) {
        (true, true) => Some("cross_split+cross_language"),
        (true, false) => Some("cross_split"),
        (false, true) => Some("cross_language"),
        (false, false) => None,
    }
}

fn canonical_physical_text(text: &str, group: i64) -> String {
    let folded = pystr::casefold(text);
    let normalized = pystr::strip(&folded);
    if group == 0 { normalized.to_owned() } else { translate(normalized, "ru", "us") }
}

#[derive(Clone, Default)]
pub struct DatasetBundle {
    pub by_split: Vec<Vec<LexicalExample>>,
    pub safety: Vec<LexicalExample>,
    pub variant_quarantine: VariantQuarantine,
    pub sealed_variant_quarantine: VariantQuarantine,
    pub sealed_test_exclusion_signatures: Vec<String>,
}

impl DatasetBundle {
    pub fn split(&self, split: Split) -> &[LexicalExample] {
        &self.by_split[split.index()]
    }
}

pub struct DatasetConfig {
    pub minimum_word_length: usize,
    pub maximum_word_length: usize,
    pub typo_augmentations: usize,
}

/// `build_variant_quarantine`: generated signatures owned by several splits or intended groups.
fn build_variant_quarantine(prepared: &PreparedLexicon, config: &DatasetConfig, included: &[Split]) -> VariantQuarantine {
    let mut split_owners: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
    let mut group_owners: HashMap<String, BTreeSet<i64>> = HashMap::new();
    let mut candidates: Vec<(String, String, &'static str, i64, &'static str)> = Vec::new();
    for collision in &prepared.collisions {
        if included.contains(&stable_split(&collision.physical_signature)) {
            group_owners.entry(collision.physical_signature.clone()).or_default().extend([0, 1]);
        }
    }
    for token in TRAINING_HARD_NEGATIVES {
        let signature = canonical_physical_text(token, 0);
        let split = stable_split(&format!("hard:{}", pystr::casefold(token)));
        if included.contains(&split) {
            split_owners.entry(signature.clone()).or_default().insert(split.as_str());
            group_owners.entry(signature).or_default().insert(0);
        }
    }
    for token in SAFETY_HARD_NEGATIVES {
        group_owners.entry(canonical_physical_text(token, 0)).or_default().extend([0, 1]);
    }
    for split in Split::ALL {
        if !included.contains(&split) {
            continue;
        }
        for record in &prepared.words_by_split[split.index()] {
            for (variant, kind) in typo_variants(&record.physical_signature, config.typo_augmentations) {
                if variant.chars().count() < config.minimum_word_length {
                    continue;
                }
                split_owners.entry(variant.clone()).or_default().insert(split.as_str());
                group_owners.entry(variant.clone()).or_default().insert(record.group);
                candidates.push((variant, record.physical_signature.clone(), split.as_str(), record.group, kind));
            }
        }
    }
    let empty_splits = BTreeSet::new();
    let empty_groups = BTreeSet::new();
    let signatures: HashSet<&String> = split_owners.keys().chain(group_owners.keys()).collect();
    let reasons: HashMap<&String, &'static str> = signatures
        .into_iter()
        .filter_map(|signature| quarantine_reason(split_owners.get(signature).unwrap_or(&empty_splits), group_owners.get(signature).unwrap_or(&empty_groups)).map(|reason| (signature, reason)))
        .collect();
    let mut occurrences: Vec<QuarantinedOccurrence> = candidates
        .iter()
        .filter_map(|(signature, base, split, group, kind)| {
            reasons.get(signature).map(|reason| QuarantinedOccurrence { physical_signature: signature.clone(), split, base_signature: base.clone(), group: *group, variant_kind: kind, reason })
        })
        .collect();
    occurrences.sort();
    let sha256 = quarantine_fingerprint(&occurrences);
    VariantQuarantine { occurrences, sha256 }
}

fn render_physical(signature: &str, group: i64) -> String {
    if group == 0 { signature.to_owned() } else { translate(signature, "us", "ru") }
}

/// `_example_weight`.
fn example_weight(frequency: i64) -> f64 {
    1.0 + EXAMPLE_FREQUENCY_WEIGHT_CAP.min((frequency.max(1) as f64).ln_1p() / EXAMPLE_FREQUENCY_LOG_DIVISOR)
}

fn hard_negative_examples(token: &str, safety: bool) -> Vec<LexicalExample> {
    let alternative = translate(token, "us", "ru");
    let signature = format!("hard:{}", pystr::casefold(token));
    TRIGGERS
        .iter()
        .map(|trigger| LexicalExample {
            original: token.to_owned(),
            alternative: alternative.clone(),
            source_group: 0,
            target_group: 1,
            trigger: *trigger,
            label: false,
            weight: PROTECTED_TOKEN_EXAMPLE_WEIGHT,
            base_signature: signature.clone(),
            variant_kind: "protected".to_owned(),
            source_known: false,
            target_known: false,
            context_delta: 0.0,
            context_group: None,
            frequency: 0,
            protected: true,
            safety,
        })
        .collect()
}

/// `build_dataset`: balanced positive and negative examples for an explicit phase.
pub fn build_dataset(prepared: &PreparedLexicon, config: &DatasetConfig, included: &[Split]) -> DatasetBundle {
    let quarantine = build_variant_quarantine(prepared, config, included);
    let quarantined: HashSet<&String> = quarantine.occurrences.iter().map(|item| &item.physical_signature).collect();
    let mut by_split: Vec<Vec<LexicalExample>> = vec![Vec::new(); Split::ALL.len()];
    for split in Split::ALL {
        if !included.contains(&split) {
            continue;
        }
        for record in &prepared.words_by_split[split.index()] {
            for (variant, kind) in typo_variants(&record.physical_signature, config.typo_augmentations) {
                if variant.chars().count() < config.minimum_word_length || quarantined.contains(&variant) {
                    continue;
                }
                let correct = render_physical(&variant, record.group);
                let wrong_group = 1 - record.group;
                let wrong = render_physical(&variant, wrong_group);
                let known = kind == "identity";
                let weight = example_weight(record.frequency);
                let single = [training_trigger(&record.physical_signature)];
                let active: &[CorrectionTrigger] = if matches!(split, Split::Train | Split::Development) { &single } else { &TRIGGERS };
                for trigger in active {
                    let base = LexicalExample {
                        original: correct.clone(),
                        alternative: wrong.clone(),
                        source_group: record.group,
                        target_group: wrong_group,
                        trigger: *trigger,
                        label: false,
                        weight,
                        base_signature: record.physical_signature.clone(),
                        variant_kind: kind.to_owned(),
                        source_known: known,
                        target_known: false,
                        context_delta: 0.0,
                        context_group: None,
                        frequency: record.frequency,
                        protected: false,
                        safety: false,
                    };
                    let flipped = LexicalExample { original: wrong.clone(), alternative: correct.clone(), source_group: wrong_group, target_group: record.group, label: true, source_known: false, target_known: known, ..base.clone() };
                    by_split[split.index()].push(base);
                    by_split[split.index()].push(flipped);
                }
            }
        }
    }
    for token in TRAINING_HARD_NEGATIVES {
        let split = stable_split(&format!("hard:{}", pystr::casefold(token)));
        if included.contains(&split) {
            by_split[split.index()].extend(hard_negative_examples(token, false));
        }
    }
    let mut safety = Vec::new();
    for collision in &prepared.collisions {
        if !included.contains(&stable_split(&collision.physical_signature)) {
            continue;
        }
        for source_group in [0, 1] {
            let target_group = 1 - source_group;
            for trigger in TRIGGERS {
                safety.push(LexicalExample {
                    original: render_physical(&collision.physical_signature, source_group),
                    alternative: render_physical(&collision.physical_signature, target_group),
                    source_group,
                    target_group,
                    trigger,
                    label: false,
                    weight: LEXICAL_COLLISION_EXAMPLE_WEIGHT,
                    base_signature: collision.physical_signature.clone(),
                    variant_kind: "lexical_collision".to_owned(),
                    source_known: true,
                    target_known: true,
                    context_delta: 0.0,
                    context_group: None,
                    frequency: 0,
                    protected: false,
                    safety: true,
                });
            }
        }
    }
    for token in SAFETY_HARD_NEGATIVES {
        safety.extend(hard_negative_examples(token, true));
    }
    for rows in &mut by_split {
        rows.sort_by(|a, b| a.split_order().cmp(&b.split_order()));
    }
    safety.sort_by(|a, b| a.safety_order().cmp(&b.safety_order()));
    DatasetBundle { by_split, safety, variant_quarantine: quarantine, sealed_variant_quarantine: empty_quarantine(), sealed_test_exclusion_signatures: Vec::new() }
}

fn empty_quarantine() -> VariantQuarantine {
    VariantQuarantine { occurrences: Vec::new(), sha256: quarantine_fingerprint(&[]) }
}

pub struct HardNegativeCorpus {
    pub by_split: Vec<Vec<LexicalExample>>,
    pub source_sha256: String,
    pub source_bytes: usize,
    pub expanded_corpus_sha256: String,
    pub physical_signatures_sha256: String,
    pub signature_count: i64,
    pub words_by_group: BTreeMap<i64, i64>,
    pub role_words_by_group: BTreeMap<&'static str, BTreeMap<i64, i64>>,
    pub training_example_weight: f64,
}

impl HardNegativeCorpus {
    pub fn provenance(&self) -> Value {
        let per_group = |table: &BTreeMap<i64, i64>| -> Value { Value::Object(table.iter().map(|(group, count)| (group.to_string(), json!(count))).collect()) };
        let roles: serde_json::Map<String, Value> = Split::PRESEALED.iter().map(|split| (split.as_str().to_owned(), per_group(&self.role_words_by_group[split.as_str()]))).collect();
        let examples: serde_json::Map<String, Value> = Split::PRESEALED.iter().map(|split| (split.as_str().to_owned(), json!(self.by_split[split.index()].len()))).collect();
        json!({"schema_version": 2, "source": {"path": "model/intent_v1/unknown-typo-development-v28.json", "sha256": self.source_sha256, "bytes": self.source_bytes}, "role_namespace": ROLE_NAMESPACE, "expanded_corpus_sha256": self.expanded_corpus_sha256, "physical_signatures_sha256": self.physical_signatures_sha256, "signature_count": self.signature_count, "words_by_group": per_group(&self.words_by_group), "role_words_by_group": roles, "examples_by_role": examples, "training_example_weight": self.training_example_weight})
    }
}

/// `external_corpus_fingerprint`: the evaluator's path-independent external-corpus hash.
pub fn external_corpus_fingerprint(examples: &[LexicalExample]) -> String {
    let mut rows: Vec<String> = examples.iter().map(LexicalExample::canonical).collect();
    rows.sort();
    let mut hasher = Sha256::new();
    hasher.update(b"keyswitch:intent-v1:external-corpus\0");
    for row in rows {
        hasher.update(row.as_bytes());
        hasher.update(b"\n");
    }
    hex(&hasher.finalize())
}

/// `physical_signature_set_fingerprint`: the evaluator's separator-safe physical-signature hash.
pub fn signature_set_fingerprint<'a>(signatures: impl IntoIterator<Item = &'a str>) -> String {
    let unique: BTreeSet<&str> = signatures.into_iter().collect();
    let mut hasher = Sha256::new();
    hasher.update(b"keyswitch:intent-v2:sealed-signatures\0");
    for signature in unique {
        hasher.update(json_string(signature, false).as_bytes());
        hasher.update(b"\n");
    }
    hex(&hasher.finalize())
}

fn bounded_string(value: &Value, label: &str) -> Result<String, String> {
    let text = value.as_str().filter(|text| !text.is_empty() && text.chars().count() <= CORPUS_STRING_MAX_CHARACTERS && text.chars().all(|character| character.to::<u32>() >= ASCII_CONTROL_CHARACTER_LIMIT));
    text.map(str::to_owned).ok_or_else(|| format!("{label} must be non-empty bounded text"))
}

/// `_decode_hard_negative_development_corpus`: strictly decode, expand, repartition and fingerprint the frozen source.
pub fn decode_hard_negative_corpus(raw: &[u8], config: &Value, dataset: &DatasetConfig, protected: &ProtectedTokens) -> Result<HardNegativeCorpus, String> {
    let root: Value = serde_json::from_slice(raw).map_err(|_| "hard-negative corpus must be strict UTF-8 JSON")?;
    let external = &config["external_evaluation"];
    let policy = &config["hard_negative_development"];
    let namespace = policy["role_namespace"].as_str().ok_or("a role namespace")?;
    if root["schema_version"] != json!(1) || root["policy"].as_str() != Some(HARD_NEGATIVE_POLICY) || root["role_namespace"].as_str() != Some(ROLE_NAMESPACE) || root["rank_namespace"].as_str() != Some(RANK_NAMESPACE) || root["choice_namespace"].as_str() != Some(CHOICE_NAMESPACE) {
        return Err("the hard-negative corpus is not the frozen one of this generation".to_owned());
    }
    let expanded_digest = root["expanded_corpus_sha256"].as_str().ok_or("expanded digest")?.to_owned();
    let physical_digest = root["physical_signatures_sha256"].as_str().ok_or("physical digest")?.to_owned();
    if Some(expanded_digest.as_str()) != external["unknown_typo_development_corpus_sha256"].as_str() {
        return Err("hard-negative corpus expanded SHA-256 is not pinned".to_owned());
    }
    let minimum_words = external["minimum_words_per_group"].as_i64().ok_or("words per group")?;
    let signature_count = root["signature_count"].as_i64().ok_or("signature count")?;
    if signature_count != minimum_words * 2 {
        return Err("hard-negative corpus signature count is incomplete".to_owned());
    }
    let words_by_group: BTreeMap<i64, i64> = [0, 1].into_iter().map(|group| (group, root["words_by_group"][group.to_string()].as_i64().unwrap_or(-1))).collect();
    if words_by_group.values().any(|count| *count != minimum_words) {
        return Err("hard-negative corpus group counts are incomplete".to_owned());
    }
    let hunspell = &external["hunspell"];
    let provenance = |locale: &str| json!({"locale": locale, "dictionary_sha256": hunspell[locale]["dictionary_sha256"], "dictionary_bytes": hunspell[locale]["dictionary_bytes"], "affix_sha256": hunspell[locale]["affix_sha256"], "affix_bytes": hunspell[locale]["affix_bytes"]});
    if root["source_provenance"] != json!({"0": provenance("en_US"), "1": provenance("ru_RU")}) {
        return Err("hard-negative dictionary provenance is not pinned".to_owned());
    }
    let rows = root["rows"].as_array().ok_or("hard-negative corpus rows must be an array")?;
    if rows.len().to::<i64>() != signature_count {
        return Err("hard-negative compact row count is inconsistent".to_owned());
    }
    let mut records: BTreeMap<String, (i64, String, String, String)> = BTreeMap::new();
    let mut records_by_group: [Vec<String>; 2] = [Vec::new(), Vec::new()];
    for item in rows {
        let signature = bounded_string(&item["physical_signature"], "physical_signature")?;
        let correct_group = item["correct_group"].as_i64().filter(|group| matches!(group, 0 | 1)).ok_or("hard-negative correct_group must be 0 or 1")?;
        let correct_typo = bounded_string(&item["correct_typo"], "correct_typo")?;
        let wrong_typo = bounded_string(&item["wrong_typo"], "wrong_typo")?;
        let variant_kind = bounded_string(&item["variant_kind"], "variant_kind")?;
        if !matches!(variant_kind.as_str(), "hunspell-unknown-deletion" | "hunspell-unknown-duplication" | "hunspell-unknown-transposition") {
            return Err("hard-negative variant kind is unsupported".to_owned());
        }
        if !(dataset.minimum_word_length..=dataset.maximum_word_length).contains(&signature.chars().count()) {
            return Err("hard-negative physical signature length is invalid".to_owned());
        }
        if canonical_physical_text(&correct_typo, correct_group) != signature || canonical_physical_text(&wrong_typo, 1 - correct_group) != signature {
            return Err("hard-negative rendered physical signature changed".to_owned());
        }
        if is_protected_token(&correct_typo, protected) || is_protected_token(&wrong_typo, protected) {
            return Err("hard-negative corpus contains a protected token".to_owned());
        }
        records_by_group[correct_group.to::<usize>()].push(signature.clone());
        if records.insert(signature, (correct_group, correct_typo, wrong_typo, variant_kind)).is_some() {
            return Err("hard-negative corpus contains duplicate signatures".to_owned());
        }
    }
    if records_by_group.iter().enumerate().any(|(group, signatures)| signatures.len().to::<i64>() != words_by_group[&(group as i64)]) {
        return Err("hard-negative compact rows disagree with group counts".to_owned());
    }
    if signature_set_fingerprint(records.keys().map(String::as_str)) != physical_digest {
        return Err("hard-negative physical-signature SHA-256 mismatch".to_owned());
    }
    let role_count = |split: Split| (policy["words_per_group"][split.as_str()].as_u64().unwrap_or(0)).to::<usize>();
    let mut role_by_signature: HashMap<String, Split> = HashMap::new();
    let mut role_words: BTreeMap<&'static str, BTreeMap<i64, i64>> = Split::ALL.iter().map(|split| (split.as_str(), BTreeMap::from([(0, 0), (1, 0)]))).collect();
    for group in [0_usize, 1] {
        let mut ordered: Vec<([u8; 32], &String)> = records_by_group[group].iter().map(|signature| (sha(&[namespace.as_bytes(), b"\0", signature.as_bytes()]), signature)).collect();
        ordered.sort();
        let mut cursor = 0;
        for split in Split::PRESEALED {
            let count = role_count(split);
            if cursor + count > ordered.len() {
                return Err("hard-negative role partition is incomplete".to_owned());
            }
            for (_, signature) in &ordered[cursor..cursor + count] {
                role_by_signature.insert((*signature).clone(), split);
            }
            role_words.get_mut(split.as_str()).expect("a split").insert(group.to::<i64>(), count.to::<i64>());
            cursor += count;
        }
        if cursor != ordered.len() {
            return Err("hard-negative role partition left unused rows".to_owned());
        }
    }
    let training_example_weight = policy["training_example_weight"].as_f64().ok_or("a training weight")?;
    let mut expanded = Vec::new();
    let mut by_split: Vec<Vec<LexicalExample>> = vec![Vec::new(); Split::ALL.len()];
    for (signature, (correct_group, correct_typo, wrong_typo, variant_kind)) in &records {
        let split = role_by_signature[signature];
        let base_signature = format!("hunspell-unknown:{signature}");
        for trigger in TRIGGERS {
            let negative = LexicalExample {
                original: correct_typo.clone(),
                alternative: wrong_typo.clone(),
                source_group: *correct_group,
                target_group: 1 - correct_group,
                trigger,
                label: false,
                weight: 1.0,
                base_signature: base_signature.clone(),
                variant_kind: variant_kind.clone(),
                source_known: false,
                target_known: false,
                context_delta: 0.0,
                context_group: None,
                frequency: 0,
                protected: false,
                safety: false,
            };
            let positive = LexicalExample { original: wrong_typo.clone(), alternative: correct_typo.clone(), source_group: 1 - correct_group, target_group: *correct_group, label: true, ..negative.clone() };
            for row in [negative, positive] {
                let kept = if split == Split::Train { LexicalExample { weight: training_example_weight, ..row.clone() } } else { row.clone() };
                by_split[split.index()].push(kept);
                expanded.push(row);
            }
        }
    }
    if external_corpus_fingerprint(&expanded) != expanded_digest {
        return Err("hard-negative expanded corpus SHA-256 mismatch".to_owned());
    }
    for rows in &mut by_split {
        rows.sort_by(|a, b| a.split_order().cmp(&b.split_order()));
    }
    Ok(HardNegativeCorpus { by_split, source_sha256: hex(&Sha256::digest(raw)), source_bytes: raw.len(), expanded_corpus_sha256: expanded_digest, physical_signatures_sha256: physical_digest, signature_count, words_by_group, role_words_by_group: role_words, training_example_weight })
}

/// `merge_hard_negative_development`: each external-development signature joins exactly one pre-seal role.
pub fn merge_hard_negative_development(mut dataset: DatasetBundle, corpus: &HardNegativeCorpus) -> DatasetBundle {
    for split in Split::ALL {
        let rows = &mut dataset.by_split[split.index()];
        rows.extend(corpus.by_split[split.index()].iter().cloned());
        rows.sort_by(|a, b| a.split_order().cmp(&b.split_order()));
    }
    dataset
}

/// `row_physical_signature`: the physical keys both rendered sides of a row agree on.
pub fn row_physical_signature(example: &LexicalExample) -> Result<String, String> {
    let (source, target) = (canonical_physical_text(&example.original, example.source_group), canonical_physical_text(&example.alternative, example.target_group));
    let unique: BTreeSet<&str> = [source.as_str(), target.as_str()].into_iter().filter(|text| !text.is_empty()).collect();
    match unique.len() {
        0 => Err("neither rendered side has a valid physical signature".to_owned()),
        1 => Ok((*unique.iter().next().expect("one")).to_owned()),
        _ => Err(format!("rendered sides disagree: {unique:?}")),
    }
}

/// `merge_sealed_test_dataset`: a post-claim test partition, without changing candidate rows.
pub fn merge_sealed_test_dataset(presealed: DatasetBundle, sealed: DatasetBundle) -> Result<DatasetBundle, String> {
    let mut signatures: HashSet<String> = presealed.variant_quarantine.occurrences.iter().map(|item| item.physical_signature.clone()).collect();
    for split in Split::PRESEALED {
        for example in presealed.split(split) {
            signatures.insert(row_physical_signature(example).map_err(|error| format!("malformed presealed row while merging test: {error}"))?);
        }
    }
    let mut safety: Vec<LexicalExample> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for example in presealed.safety.iter().chain(&sealed.safety) {
        if seen.insert(example.canonical()) {
            safety.push(example.clone());
        }
    }
    safety.sort_by(|a, b| a.safety_order().cmp(&b.safety_order()));
    let bases: HashSet<&String> = safety.iter().map(|example| &example.base_signature).collect();
    let mut safety_signatures: HashSet<String> = HashSet::new();
    for example in &safety {
        safety_signatures.insert(row_physical_signature(example).map_err(|error| format!("malformed safety row while merging test: {error}"))?);
    }
    let mut excluded: BTreeSet<String> = BTreeSet::new();
    let mut retained = Vec::new();
    for example in sealed.split(Split::Test) {
        let signature = row_physical_signature(example).map_err(|error| format!("malformed sealed test row: {error}"))?;
        if signatures.contains(&signature) || safety_signatures.contains(&signature) || bases.contains(&example.base_signature) {
            excluded.insert(signature);
        } else {
            retained.push(example.clone());
        }
    }
    let mut by_split = presealed.by_split;
    by_split[Split::Test.index()] = retained;
    let safety_kept = safety;
    Ok(DatasetBundle { by_split, safety: safety_kept, variant_quarantine: presealed.variant_quarantine, sealed_variant_quarantine: sealed.variant_quarantine, sealed_test_exclusion_signatures: excluded.into_iter().collect() })
}

fn check_quarantine(dataset: &DatasetBundle, quarantine: &VariantQuarantine, label: &str, included: &[Split]) -> Result<(), String> {
    if quarantine.sha256 != quarantine_fingerprint(&quarantine.occurrences) {
        return Err(format!("{label} variant quarantine fingerprint does not match its records"));
    }
    if quarantine.occurrences.iter().collect::<HashSet<_>>().len() != quarantine.occurrences.len() {
        return Err(format!("{label} variant quarantine contains duplicate occurrence records"));
    }
    let names: Vec<&str> = included.iter().map(|split| split.as_str()).collect();
    let (mut splits, mut groups): (HashMap<String, BTreeSet<&'static str>>, HashMap<String, BTreeSet<i64>>) = Default::default();
    let mut reasons: HashMap<&str, BTreeSet<&'static str>> = HashMap::new();
    for occurrence in &quarantine.occurrences {
        if !names.contains(&occurrence.split) {
            return Err(format!("{label} variant quarantine occurrence is outside its phase: {}", occurrence.split));
        }
        splits.entry(occurrence.physical_signature.clone()).or_default().insert(occurrence.split);
        groups.entry(occurrence.physical_signature.clone()).or_default().insert(occurrence.group);
        reasons.entry(&occurrence.physical_signature).or_default().insert(occurrence.reason);
    }
    for split in included {
        for example in dataset.split(*split) {
            let Ok(signature) = row_physical_signature(example) else { continue };
            if AUGMENTED_KINDS.contains(&example.variant_kind.as_str()) {
                splits.entry(signature.clone()).or_default().insert(split.as_str());
                groups.entry(signature).or_default().insert(if example.label { example.target_group } else { example.source_group });
            } else if example.variant_kind == "protected" {
                splits.entry(signature.clone()).or_default().insert(split.as_str());
                groups.entry(signature).or_default().insert(example.source_group);
            }
        }
    }
    for example in &dataset.safety {
        let Ok(signature) = row_physical_signature(example) else { continue };
        if example.protected || (example.variant_kind == "lexical_collision" && included.contains(&stable_split(&example.base_signature))) {
            groups.entry(signature).or_default().extend([0, 1]);
        }
    }
    let (no_splits, no_groups) = (BTreeSet::new(), BTreeSet::new());
    for (signature, seen) in &reasons {
        let expected = quarantine_reason(splits.get(*signature).unwrap_or(&no_splits), groups.get(*signature).unwrap_or(&no_groups));
        if expected.is_none_or(|reason| seen.len() != 1 || !seen.contains(reason)) {
            return Err(format!("{label} variant quarantine ownership/reason evidence is invalid: {signature}"));
        }
    }
    Ok(())
}

/// `assert_no_split_leakage`: phase-local quarantine evidence and all emitted dataset rows.
pub fn assert_no_split_leakage(dataset: &DatasetBundle, quarantine_splits: &[Split]) -> Result<(), String> {
    let label = if quarantine_splits == [Split::Test] { "sealed" } else { "candidate" };
    check_quarantine(dataset, &dataset.variant_quarantine, label, quarantine_splits)?;
    check_quarantine(dataset, &dataset.sealed_variant_quarantine, "sealed", &[Split::Test])?;
    let occurrences: Vec<&QuarantinedOccurrence> = dataset.variant_quarantine.occurrences.iter().chain(&dataset.sealed_variant_quarantine.occurrences).collect();
    if occurrences.iter().collect::<HashSet<_>>().len() != occurrences.len() {
        return Err("candidate and sealed variant quarantines contain a duplicate occurrence record".to_owned());
    }
    let quarantined: HashSet<(&str, &str, &str, i64, &str)> = occurrences.iter().map(|item| (item.physical_signature.as_str(), item.base_signature.as_str(), item.split, item.group, item.variant_kind)).collect();
    let mut base_owner: HashMap<&str, &str> = HashMap::new();
    let (mut owners, mut intended): (HashMap<String, BTreeSet<&str>>, HashMap<String, BTreeSet<i64>>) = Default::default();
    let mut malformed = Vec::new();
    let mut present = BTreeSet::new();
    for split in Split::ALL {
        for example in dataset.split(split) {
            let previous = *base_owner.entry(&example.base_signature).or_insert(split.as_str());
            if previous != split.as_str() {
                return Err(format!("physical signature leaked from {previous} into {}: {}", split.as_str(), example.base_signature));
            }
            match row_physical_signature(example) {
                Err(error) => malformed.push(format!("{} {}: {error}", split.as_str(), example.base_signature)),
                Ok(signature) => {
                    let group = if example.label { example.target_group } else { example.source_group };
                    if AUGMENTED_KINDS.contains(&example.variant_kind.as_str()) && quarantined.contains(&(signature.as_str(), example.base_signature.as_str(), split.as_str(), group, example.variant_kind.as_str())) {
                        present.insert(signature.clone());
                    }
                    owners.entry(signature.clone()).or_default().insert(split.as_str());
                    intended.entry(signature).or_default().insert(group);
                }
            }
        }
    }
    let mut safety_signatures = HashSet::new();
    let safety_bases: HashSet<&str> = dataset.safety.iter().map(|example| example.base_signature.as_str()).collect();
    for example in &dataset.safety {
        match row_physical_signature(example) {
            Err(error) => malformed.push(format!("safety {}: {error}", example.base_signature)),
            Ok(signature) => {
                safety_signatures.insert(signature);
            }
        }
    }
    let examples = |mut items: Vec<String>| -> String {
        items.sort();
        items.truncate(LEAKAGE_ERROR_EXAMPLE_COUNT);
        items.join(", ")
    };
    if !malformed.is_empty() {
        return Err(format!("generated lexical rows have inconsistent physical signatures: {}", examples(malformed)));
    }
    let test_rows: HashSet<String> = dataset.split(Split::Test).iter().filter_map(|example| row_physical_signature(example).ok()).collect();
    if dataset.sealed_test_exclusion_signatures.iter().any(|signature| test_rows.contains(signature)) {
        return Err("excluded sealed-test signature was emitted".to_owned());
    }
    let crossing: Vec<String> = owners.iter().filter(|(_, splits)| splits.len() > 1).map(|(signature, _)| signature.clone()).collect();
    if !crossing.is_empty() {
        return Err(format!("actual augmented physical signature leaked between splits: {}", examples(crossing)));
    }
    let contradicting: Vec<String> = intended.iter().filter(|(_, groups)| groups.len() > 1).map(|(signature, _)| signature.clone()).collect();
    if !contradicting.is_empty() {
        return Err(format!("actual augmented physical signature has contradictory intended languages: {}", examples(contradicting)));
    }
    let overlapping: Vec<String> = owners.keys().filter(|signature| safety_signatures.contains(*signature)).cloned().collect();
    if !overlapping.is_empty() {
        return Err(format!("actual augmented physical signature overlaps the safety corpus: {}", examples(overlapping)));
    }
    let base_overlap: Vec<String> = base_owner.keys().filter(|base| safety_bases.contains(**base)).map(|base| (*base).to_owned()).collect();
    if !base_overlap.is_empty() {
        return Err(format!("base signature overlaps the safety corpus: {}", examples(base_overlap)));
    }
    if !present.is_empty() {
        return Err(format!("quarantined physical signatures were emitted as rows: {}", examples(present.into_iter().collect())));
    }
    Ok(())
}

/// `dataset_fingerprint`: the hash of every sealed row, for exact provenance.
pub fn dataset_fingerprint(dataset: &DatasetBundle) -> String {
    let mut hasher = Sha256::new();
    for (label, quarantine) in [("candidate-variant-quarantine", &dataset.variant_quarantine), ("sealed-variant-quarantine", &dataset.sealed_variant_quarantine)] {
        hasher.update(format!("{label}:{}:{}:{}\n", quarantine.occurrences.len(), quarantine.signature_count(), quarantine.sha256).as_bytes());
        for occurrence in &quarantine.occurrences {
            hasher.update(occurrence.canonical().as_bytes());
            hasher.update(b"\n");
        }
    }
    hasher.update(b"sealed-test-exclusions\n");
    for signature in &dataset.sealed_test_exclusion_signatures {
        hasher.update(signature.as_bytes());
        hasher.update(b"\n");
    }
    for split in Split::ALL {
        hasher.update(format!("split:{}\n", split.as_str()).as_bytes());
        for example in dataset.split(split) {
            hasher.update(example.canonical().as_bytes());
            hasher.update(b"\n");
        }
    }
    hasher.update(b"safety\n");
    for example in &dataset.safety {
        hasher.update(example.canonical().as_bytes());
        hasher.update(b"\n");
    }
    hex(&hasher.finalize())
}

/// `TrainOnlyLanguageScorers.provenance_payload`: the training partition's words the classifier never reads.
pub fn train_only_scorer_provenance(prepared: &PreparedLexicon, quarantine: &VariantQuarantine) -> Value {
    let quarantined: HashSet<&String> = quarantine.occurrences.iter().map(|item| &item.physical_signature).collect();
    let mut records: Vec<&LexiconWord> = prepared.words_by_split[Split::Train.index()].iter().collect();
    records.sort_by(|a, b| (a.group, &a.physical_signature, &a.word, a.frequency).cmp(&(b.group, &b.physical_signature, &b.word, b.frequency)));
    let mut hasher = Sha256::new();
    hasher.update(b"keyswitch:intent-v1:train-only-language-scorer\0");
    hasher.update(format!("{{\"algorithm_version\":{TRAIN_ONLY_SCORER_ALGORITHM_VERSION},\"ngram_orders\":[2,3,4],\"score_mode\":\"character-ngram-only\",\"spellcheck_enabled\":false}}\n").as_bytes());
    let (mut excluded, mut counts) = (0, [0_i64; 2]);
    for record in records {
        if quarantined.contains(&record.physical_signature) {
            excluded += 1;
            continue;
        }
        counts[record.group.to::<usize>()] += 1;
        hasher.update(format!("{{\"frequency\":{},\"group\":{},\"physical_signature\":{},\"word\":{}}}\n", record.frequency, record.group, json_string(&record.physical_signature, false), json_string(&record.word, false)).as_bytes());
    }
    json!({"kind": "train-only-character-ngram", "algorithm_version": TRAIN_ONLY_SCORER_ALGORITHM_VERSION, "ngram_orders": LANGUAGE_MODEL_NGRAM_ORDERS, "score_mode": "character-ngram-only", "spellcheck_enabled": false, "word_counts_by_group": {"0": counts[0], "1": counts[1]}, "excluded_quarantined_identities": excluded, "source_sha256": hex(&hasher.finalize())})
}

/// `verify_frozen_file`-style read: the bytes of a frozen file, once its digest and size are the configured ones.
pub fn read_frozen(root: &Path, spec: &Value) -> Result<Vec<u8>, String> {
    let relative = spec["path"].as_str().ok_or("a frozen file without a path")?;
    let bytes = std::fs::read(root.join(relative)).map_err(|error| format!("{relative}: {error}"))?;
    if Some(hex(&Sha256::digest(&bytes)).as_str()) != spec["sha256"].as_str() || spec["bytes"].as_u64().is_some_and(|size| size.to::<usize>() != bytes.len()) {
        return Err(format!("{relative} differs from the frozen source"));
    }
    Ok(bytes)
}
