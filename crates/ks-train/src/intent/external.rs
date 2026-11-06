//! The external corpora of the layout-intent model (`evaluate_intent_model`, `preseal_intent_holdout`,
//! `freeze_intent_development_corpus`): words of the frozen Hunspell dictionaries that the Onboard lexicon does not have,
//! their physical typos neither language knows, and the receipts that fix them before any model is looked at.
use ks_cast::To as _;
use super::dataset::{self, DatasetBundle, DatasetConfig, HardNegativeCorpus, LexicalExample, LexiconWord, Split, physical_signature, row_physical_signature, signature_set_fingerprint};
use crate::corpus::read_json;
use crate::physical::{hex, pair};
use ks_lang::detector::{ProtectedTokens, is_protected_token};
use ks_lang::intent_model::{CorrectionTrigger, TRIGGERS};
use ks_lang::language_model::LanguageModel;
use ks_lang::spellcheck::find_dictionary_in;
use ks_text::{py_round, pystr};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;

pub const DEVELOPMENT_RANK_NAMESPACE: &str = "keyswitch:intent-v1:unknown-typo-rank";
pub const DEVELOPMENT_CHOICE_NAMESPACE: &str = "keyswitch:intent-v1:unknown-typo-choice";
pub const HOLDOUT_RANK_NAMESPACE: &str = "keyswitch:intent-v28:unknown-typo-holdout-rank";
pub const HOLDOUT_CHOICE_NAMESPACE: &str = "keyswitch:intent-v28:unknown-typo-holdout-choice";
pub const PAIRED_ROWS_PER_TRIGGER: usize = 2;
const SIGNATURE_PREFIX: &str = "hunspell-unknown:";
const MIN_RUNTIME_TOKEN_CHARACTERS: usize = 5;
const WORD_MAX_CHARACTERS: usize = 24;
const TYPO_AUGMENTATIONS: usize = 3;
const AFFIX_MAX_BYTES: usize = 1 << 20;
const DICTIONARY_MAX_BYTES: usize = 64 << 20;
const AFFIX_HEADER_SCAN_LINES: usize = 80;
const NAMESPACE_MAX_CHARACTERS: usize = 128;
const LOCALES: [&str; 2] = ["en_US", "ru_RU"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrozenFile {
    pub path: String,
    pub sha256: String,
    pub bytes: usize,
}

impl FrozenFile {
    fn missing() -> FrozenFile {
        FrozenFile { path: String::new(), sha256: hex(&Sha256::digest(b"")), bytes: 0 }
    }

    fn of(path: &Path, data: &[u8]) -> FrozenFile {
        FrozenFile { path: path.display().to_string(), sha256: hex(&Sha256::digest(data)), bytes: data.len() }
    }

    fn payload(&self) -> Value {
        json!({"path": self.path, "sha256": self.sha256, "bytes": self.bytes})
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DictionaryProvenance {
    pub locale: String,
    pub dictionary: FrozenFile,
    pub affix: FrozenFile,
}

impl DictionaryProvenance {
    #[must_use]
    pub fn payload(&self) -> Value {
        json!({"locale": self.locale, "dictionary": self.dictionary.payload(), "affix": self.affix.payload()})
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub words: Vec<String>,
    pub provenance: DictionaryProvenance,
}

fn missing_snapshot(locale: &str) -> Snapshot {
    Snapshot { words: Vec::new(), provenance: DictionaryProvenance { locale: locale.to_owned(), dictionary: FrozenFile::missing(), affix: FrozenFile::missing() } }
}

fn read_bounded(path: &Path, limit: usize) -> Option<Vec<u8>> {
    std::fs::read(path).ok().filter(|data| data.len() <= limit)
}

/// `_hunspell_snapshot_from_paths`: parsed words and provenance from one pair of reads.
fn snapshot_from_paths(locale: &str, dictionary_path: &Path, affix_path: &Path) -> Result<Snapshot, String> {
    let Some(affix) = read_bounded(affix_path, AFFIX_MAX_BYTES) else { return Ok(missing_snapshot(locale)) };
    let encoding = affix.split(|byte| matches!(byte, b'\n' | b'\r')).take(AFFIX_HEADER_SCAN_LINES).find_map(|line| line.strip_prefix(b"SET ")).map_or_else(|| "UTF-8".to_owned(), |name| String::from_utf8_lossy(name).into_owned());
    if !encoding.eq_ignore_ascii_case("utf-8") {
        return Err(format!("{}: the dictionary is in {encoding}; the frozen ones are UTF-8", affix_path.display()));
    }
    let Some(dictionary) = read_bounded(dictionary_path, DICTIONARY_MAX_BYTES) else { return Ok(missing_snapshot(locale)) };
    let text = String::from_utf8_lossy(&dictionary);
    let candidates: BTreeSet<String> = pystr::splitlines(&text, false).into_iter().skip(1).map(|line| LanguageModel::normalize(&line.split('/').next().unwrap_or("").replace("\\/", "/"))).collect();
    let words = candidates.into_iter().filter(|word| (MIN_RUNTIME_TOKEN_CHARACTERS..=WORD_MAX_CHARACTERS).contains(&word.chars().count()) && pystr::isalpha(word)).collect();
    Ok(Snapshot { words, provenance: DictionaryProvenance { locale: locale.to_owned(), dictionary: FrozenFile::of(dictionary_path, &dictionary), affix: FrozenFile::of(affix_path, &affix) } })
}

/// `_hunspell_dictionary_snapshot`: the exact paths behind a handle already open.
pub fn snapshot_of_model(model: &LanguageModel, locale: &str) -> Result<Snapshot, String> {
    if !model.speller.available() || model.speller.source().is_empty() {
        return Ok(missing_snapshot(locale));
    }
    let dictionary = Path::new(model.speller.source());
    snapshot_from_paths(locale, dictionary, &dictionary.with_extension("aff"))
}

/// `_discover_hunspell_snapshot`: the files as they are before Hunspell opens them.
pub fn discover_snapshot(locale: &str, frozen: &Path) -> Result<Snapshot, String> {
    match find_dictionary_in(locale, &[frozen.to_path_buf()]) {
        Some((affix, dictionary)) => snapshot_from_paths(locale, &dictionary, &affix),
        None => Ok(missing_snapshot(locale)),
    }
}

/// `_deterministic_sample`.
pub fn deterministic_sample<T: Clone>(examples: &[T], requested: usize) -> Vec<T> {
    if requested == 0 || requested >= examples.len() {
        return examples.to_vec();
    }
    if requested == 1 {
        return vec![examples[0].clone()];
    }
    (0..requested).map(|index| examples[(py_round(index.to::<f64>() * (examples.len() - 1).to::<f64>() / (requested - 1).to::<f64>())).to::<usize>()].clone()).collect()
}

pub struct SealedSignatureIndex {
    pub signatures: BTreeSet<String>,
    pub sha256: String,
    pub split_lexical_signature_count: usize,
    pub safety_lexical_signature_count: usize,
    pub protected_exact_signature_count: usize,
}

fn rendered_signature(example: &LexicalExample) -> Result<String, String> {
    let candidates: BTreeSet<String> = [physical_signature(&example.original, example.source_group), physical_signature(&example.alternative, example.target_group)].into_iter().filter(|signature| !signature.is_empty()).collect();
    if candidates.len() != 1 {
        return Err(format!("sealed lexical row has missing or contradictory physical signatures: base={:?}", example.base_signature));
    }
    Ok(candidates.into_iter().next().expect("one"))
}

fn rendered_hard_negative_signature(example: &LexicalExample) -> Result<String, String> {
    let Some(expected) = example.base_signature.strip_prefix(SIGNATURE_PREFIX) else {
        return Err(format!("sealed hard-negative row has a foreign base signature: {:?}", example.base_signature));
    };
    let signature = row_physical_signature(example)?;
    if expected.is_empty() || signature != expected {
        return Err(format!("sealed hard-negative row does not reproduce its physical signature: {:?}", example.base_signature));
    }
    Ok(signature)
}

/// `build_sealed_signature_index`: every emitted lexical sequence plus protected exact key strings.
pub fn build_sealed_signature_index(dataset: &DatasetBundle) -> Result<SealedSignatureIndex, String> {
    let (mut split_lexical, mut safety_lexical, mut protected_exact): (BTreeSet<String>, BTreeSet<String>, BTreeSet<String>) = Default::default();
    let mut take = |example: &LexicalExample, lexical: &mut BTreeSet<String>, label: &str| -> Result<(), String> {
        match example.variant_kind.as_str() {
            "identity" | "deletion" | "duplication" | "transposition" | "lexical_collision" => {
                lexical.insert(rendered_signature(example)?);
            }
            "hunspell-unknown-deletion" | "hunspell-unknown-duplication" | "hunspell-unknown-transposition" => {
                lexical.insert(rendered_hard_negative_signature(example)?);
            }
            _ => match example.base_signature.strip_prefix("hard:").filter(|_| example.protected) {
                Some(exact) => {
                    protected_exact.insert(exact.to_owned());
                }
                None => return Err(format!("cannot prove sealed-signature exclusion for unsupported {label} row: variant={:?}, base={:?}", example.variant_kind, example.base_signature)),
            },
        }
        Ok(())
    };
    for split in Split::ALL {
        for example in dataset.split(split) {
            take(example, &mut split_lexical, "dataset")?;
        }
    }
    for example in &dataset.safety {
        take(example, &mut safety_lexical, "safety")?;
    }
    let (split_count, safety_count, protected_count) = (split_lexical.len(), safety_lexical.len(), protected_exact.len());
    let mut signatures = split_lexical;
    signatures.append(&mut safety_lexical);
    signatures.append(&mut protected_exact);
    let sha256 = signature_set_fingerprint(signatures.iter().map(String::as_str));
    Ok(SealedSignatureIndex { signatures, sha256, split_lexical_signature_count: split_count, safety_lexical_signature_count: safety_count, protected_exact_signature_count: protected_count })
}

/// `LexicalDisjointCorpus`.
#[derive(Clone)]
pub struct ExternalCorpus {
    pub examples: Vec<LexicalExample>,
    pub words_by_group: [usize; 2],
    pub dictionary_sources: [String; 2],
    pub minimum_words_per_group: usize,
    pub rejected_sealed_overlaps_by_group: [usize; 2],
    pub exclusion_signature_count: usize,
    pub exclusion_signature_sha256: String,
    pub selected_sealed_overlap_count: usize,
    pub corpus_sha256: String,
    pub rank_namespace: String,
    pub choice_namespace: String,
    pub dictionary_provenance: [DictionaryProvenance; 2],
}

fn plain_row(original: &str, alternative: &str, source_group: i64, trigger: CorrectionTrigger, label: bool, base_signature: &str, variant_kind: &str) -> LexicalExample {
    LexicalExample {
        original: original.to_owned(),
        alternative: alternative.to_owned(),
        source_group,
        target_group: 1 - source_group,
        trigger,
        label,
        weight: 1.0,
        base_signature: base_signature.to_owned(),
        variant_kind: variant_kind.to_owned(),
        source_known: false,
        target_known: false,
        context_delta: 0.0,
        context_group: None,
        frequency: 0,
        protected: false,
        safety: false,
    }
}

/// `build_lexical_disjoint_corpus`.
pub fn build_lexical_disjoint_corpus<S: ::std::hash::BuildHasher>(onboard: &[HashSet<String, S>; 2], minimum_words_per_group: usize, snapshots: &[Snapshot; 2]) -> Result<ExternalCorpus, String> {
    if minimum_words_per_group < 1 {
        return Err("minimum_words_per_group must be positive".to_owned());
    }
    let mut examples = Vec::new();
    let mut counts = [0; 2];
    for group in [0_usize, 1] {
        let snapshot = &snapshots[group];
        if snapshot.provenance.locale != LOCALES[group] {
            return Err(format!("Hunspell snapshot group {group} must be {}", LOCALES[group]));
        }
        let disjoint: Vec<&String> = snapshot.words.iter().filter(|word| !onboard[group].contains(*word) && !physical_signature(word, group.to::<i64>()).is_empty()).collect();
        let selected = deterministic_sample(&disjoint, minimum_words_per_group);
        counts[group] = selected.len();
        let (from, to) = if group == 0 { ("us", "ru") } else { ("ru", "us") };
        for word in selected {
            let wrong = pair().translate(word, from, to).expect("known layouts");
            let signature = format!("hunspell:{}", physical_signature(word, group.to::<i64>()));
            examples.push(plain_row(word, &wrong, group.to::<i64>(), CorrectionTrigger::Space, false, &signature, "hunspell-disjoint"));
            examples.push(plain_row(&wrong, word, 1 - group.to::<i64>(), CorrectionTrigger::Space, true, &signature, "hunspell-disjoint"));
        }
    }
    let corpus_sha256 = dataset::external_corpus_fingerprint(&examples);
    let exclusion_signature_sha256 = signature_set_fingerprint(std::iter::empty());
    Ok(ExternalCorpus { examples, words_by_group: counts, dictionary_sources: [snapshots[0].provenance.dictionary.path.clone(), snapshots[1].provenance.dictionary.path.clone()], minimum_words_per_group, rejected_sealed_overlaps_by_group: [0, 0], exclusion_signature_count: 0, exclusion_signature_sha256, selected_sealed_overlap_count: 0, corpus_sha256, rank_namespace: String::new(), choice_namespace: String::new(), dictionary_provenance: [snapshots[0].provenance.clone(), snapshots[1].provenance.clone()] })
}

fn render(signature: &str, group: usize) -> String {
    if group == 0 { signature.to_owned() } else { pair().translate(signature, "us", "ru").expect("known layouts") }
}

/// `build_unknown_typo_disjoint_corpus`: serving-domain rows where neither decoding is known to the runtime scorers.
pub fn build_unknown_typo_disjoint_corpus<S: ::std::hash::BuildHasher>(onboard: &[HashSet<String, S>; 2], excluded: &BTreeSet<String>, minimum_words_per_group: usize, snapshots: &[Snapshot; 2], models: &[&LanguageModel; 2], protected: &ProtectedTokens, rank_namespace: &str, choice_namespace: &str) -> Result<ExternalCorpus, String> {
    if minimum_words_per_group < 1 {
        return Err("minimum_words_per_group must be positive".to_owned());
    }
    for (label, namespace) in [("rank_namespace", rank_namespace), ("choice_namespace", choice_namespace)] {
        if namespace.is_empty() || namespace.len() > NAMESPACE_MAX_CHARACTERS || namespace.contains('\0') || !namespace.is_ascii() {
            return Err(format!("{label} must be non-empty bounded ASCII"));
        }
    }
    if rank_namespace == choice_namespace {
        return Err("rank and choice namespaces must be distinct".to_owned());
    }
    let mut examples = Vec::new();
    let (mut counts, mut rejected) = ([0; 2], [0; 2]);
    let mut used: BTreeSet<String> = BTreeSet::new();
    let namespaced = |namespace: &str, signature: &str| -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(namespace.as_bytes());
        hasher.update(b"\0");
        hasher.update(signature.as_bytes());
        hasher.finalize().into()
    };
    for group in [0_usize, 1] {
        let snapshot = &snapshots[group];
        if snapshot.provenance.locale != LOCALES[group] {
            return Err(format!("Hunspell snapshot group {group} must be {}", LOCALES[group]));
        }
        let mut ordered: Vec<([u8; 32], String)> = snapshot.words.iter().filter(|word| !onboard[group].contains(*word)).filter_map(|word| { let signature = physical_signature(word, group.to::<i64>()); (!signature.is_empty()).then(|| (namespaced(rank_namespace, &signature), signature)) }).collect();
        ordered.sort_by_key(|a| a.0);
        let mut selected: Vec<(String, String, String, &'static str)> = Vec::new();
        for (_, signature) in &ordered {
            let digest = namespaced(choice_namespace, signature);
            let mut variants: Vec<(String, &'static str)> = dataset::typo_variants(signature, TYPO_AUGMENTATIONS).into_iter().skip(1).collect();
            if variants.is_empty() {
                continue;
            }
            let rotation = usize::from(digest[0]) % variants.len();
            variants.rotate_left(rotation);
            for (variant, kind) in variants {
                if excluded.contains(&variant) {
                    rejected[group] += 1;
                    continue;
                }
                if used.contains(&variant) {
                    continue;
                }
                let correct = render(&variant, group);
                let wrong = render(&variant, 1 - group);
                if LanguageModel::normalize(&correct).chars().count().max(LanguageModel::normalize(&wrong).chars().count()) < MIN_RUNTIME_TOKEN_CHARACTERS || is_protected_token(&correct, protected) || is_protected_token(&wrong, protected) || models[group].score(&correct).known || models[1 - group].score(&wrong).known {
                    continue;
                }
                used.insert(variant.clone());
                selected.push((variant, correct, wrong, kind));
                break;
            }
            if selected.len() >= minimum_words_per_group {
                break;
            }
        }
        counts[group] = selected.len();
        for (variant, correct, wrong, kind) in selected {
            let base = format!("{SIGNATURE_PREFIX}{variant}");
            let kind = format!("hunspell-unknown-{kind}");
            for trigger in TRIGGERS {
                examples.push(plain_row(&correct, &wrong, group.to::<i64>(), trigger, false, &base, &kind));
                examples.push(plain_row(&wrong, &correct, 1 - group.to::<i64>(), trigger, true, &base, &kind));
            }
        }
    }
    let selected_sealed_overlap_count = used.intersection(excluded).count();
    if selected_sealed_overlap_count != 0 {
        return Err("unknown-typo corpus contains a sealed physical signature after filtering".to_owned());
    }
    let corpus_sha256 = dataset::external_corpus_fingerprint(&examples);
    Ok(ExternalCorpus { examples, words_by_group: counts, dictionary_sources: [snapshots[0].provenance.dictionary.path.clone(), snapshots[1].provenance.dictionary.path.clone()], minimum_words_per_group, rejected_sealed_overlaps_by_group: rejected, exclusion_signature_count: excluded.len(), exclusion_signature_sha256: signature_set_fingerprint(excluded.iter().map(String::as_str)), selected_sealed_overlap_count, corpus_sha256, rank_namespace: rank_namespace.to_owned(), choice_namespace: choice_namespace.to_owned(), dictionary_provenance: [snapshots[0].provenance.clone(), snapshots[1].provenance.clone()] })
}

/// `unknown_typo_physical_signatures`: the selected physical typo domain after shape validation.
pub fn unknown_typo_physical_signatures(corpus: &ExternalCorpus) -> Result<BTreeSet<String>, String> {
    let mut rows: BTreeMap<&str, usize> = BTreeMap::new();
    for example in &corpus.examples {
        let physical = example.base_signature.strip_prefix(SIGNATURE_PREFIX).ok_or("unknown-typo corpus contains a foreign signature")?;
        if physical.is_empty() {
            return Err("unknown-typo corpus contains an empty signature".to_owned());
        }
        *rows.entry(physical).or_default() += 1;
    }
    if rows.is_empty() || rows.values().any(|count| *count != TRIGGERS.len() * PAIRED_ROWS_PER_TRIGGER) {
        return Err("unknown-typo corpus is not trigger-symmetric".to_owned());
    }
    Ok(rows.into_keys().map(str::to_owned).collect())
}

/// The Onboard words of each group, as the evaluator holds them: long enough for a training signature.
#[must_use]
pub fn onboard_words(words: &[Vec<LexiconWord>; 2], minimum_word_length: usize) -> [HashSet<String>; 2] {
    let of = |group: usize| words[group].iter().filter(|word| word.physical_signature.chars().count() >= minimum_word_length).map(|word| word.word.clone()).collect();
    [of(0), of(1)]
}

/// `ModelBlindExternalCorpora`.
pub struct Corpora {
    pub development: ExternalCorpus,
    pub holdout: ExternalCorpus,
    pub sealed_index: SealedSignatureIndex,
    pub development_signatures: BTreeSet<String>,
    pub holdout_signatures: BTreeSet<String>,
    pub combined_exclusions: BTreeSet<String>,
    pub frozen_development: Option<HardNegativeCorpus>,
}

/// `build_model_blind_external_corpora`: both corpora, without loading, receiving or scoring a model.
pub fn model_blind_corpora(root: &Path, models: &[&LanguageModel; 2], protected: &ProtectedTokens, verify_frozen_source: bool) -> Result<(Value, Corpora), String> {
    let config = read_json(&root.join("model/intent_v1/config.json"))?;
    let shape = &config["dataset"];
    let number = |name: &str| shape[name].as_u64().map(ks_cast::To::to::<usize>).ok_or_else(|| format!("dataset.{name}"));
    let dataset_config = DatasetConfig { minimum_word_length: number("minimum_word_length")?, maximum_word_length: number("maximum_word_length")?, typo_augmentations: number("typo_augmentations")? };
    let words = super::pipeline::onboard_lexicon(root, &config, &dataset_config)?;
    let onboard = onboard_words(&words, dataset_config.minimum_word_length);
    let prepared = dataset::prepare_lexicon(words.iter().flatten().cloned().collect(), dataset_config.minimum_word_length);
    let candidate = dataset::build_dataset(&prepared, &dataset_config, &Split::PRESEALED);
    dataset::assert_no_split_leakage(&candidate, &Split::PRESEALED)?;
    let sealed = dataset::build_dataset(&prepared, &dataset_config, &[Split::Test]);
    dataset::assert_no_split_leakage(&sealed, &[Split::Test])?;
    let whole = dataset::merge_sealed_test_dataset(candidate, sealed)?;
    dataset::assert_no_split_leakage(&whole, &Split::PRESEALED)?;
    let sealed_index = build_sealed_signature_index(&whole)?;
    let snapshots = [snapshot_of_model(models[0], LOCALES[0])?, snapshot_of_model(models[1], LOCALES[1])?];
    let minimum = (config["external_evaluation"]["minimum_words_per_group"].as_u64().ok_or("minimum_words_per_group")?).to::<usize>();
    let development = build_unknown_typo_disjoint_corpus(&onboard, &sealed_index.signatures, minimum, &snapshots, models, protected, DEVELOPMENT_RANK_NAMESPACE, DEVELOPMENT_CHOICE_NAMESPACE)?;
    let development_signatures = unknown_typo_physical_signatures(&development)?;
    let combined_exclusions: BTreeSet<String> = sealed_index.signatures.union(&development_signatures).cloned().collect();
    let holdout = build_unknown_typo_disjoint_corpus(&onboard, &combined_exclusions, minimum, &snapshots, models, protected, HOLDOUT_RANK_NAMESPACE, HOLDOUT_CHOICE_NAMESPACE)?;
    let holdout_signatures = unknown_typo_physical_signatures(&holdout)?;
    if development_signatures.intersection(&holdout_signatures).next().is_some() || sealed_index.signatures.intersection(&holdout_signatures).next().is_some() {
        return Err("preseal holdout overlaps a previously exposed domain".to_owned());
    }
    let frozen_development = if verify_frozen_source {
        let raw = dataset::read_frozen(root, &config["hard_negative_development"]["source"])?;
        let frozen = dataset::decode_hard_negative_corpus(&raw, &config, &dataset_config, protected)?;
        if frozen.expanded_corpus_sha256 != development.corpus_sha256 || frozen.physical_signatures_sha256 != signature_set_fingerprint(development_signatures.iter().map(String::as_str)) || frozen.signature_count.to::<usize>() != development_signatures.len() || frozen.words_by_group[&0].to::<usize>() != development.words_by_group[0] || frozen.words_by_group[&1].to::<usize>() != development.words_by_group[1] {
            return Err("frozen hard-negative source differs from model-blind development".to_owned());
        }
        Some(frozen)
    } else {
        None
    };
    Ok((config, Corpora { development, holdout, sealed_index, development_signatures, holdout_signatures, combined_exclusions, frozen_development }))
}

fn release_name() -> Result<&'static str, String> {
    dataset::SPLIT_NAMESPACE.strip_prefix("keyswitch:intent-").and_then(|rest| rest.strip_suffix(":physical-signature")).filter(|name| name.starts_with('v')).ok_or_else(|| "split namespace does not contain a release version".to_owned())
}

fn by_group(counts: &[usize; 2]) -> Value {
    json!({"0": counts[0], "1": counts[1]})
}

/// `build_preseal_receipt`: an auditable receipt, without a model or a metric.
pub fn preseal_receipt(config: &Value, corpora: &Corpora) -> Result<Value, String> {
    let frozen = corpora.frozen_development.as_ref().ok_or("preseal receipt requires the frozen development source")?;
    let policy = &config["hard_negative_development"];
    let roles: Map<String, Value> = Split::PRESEALED.iter().map(|split| (split.as_str().to_owned(), Value::Object(frozen.role_words_by_group[split.as_str()].iter().map(|(group, count)| (group.to_string(), json!(count))).collect()))).collect();
    let examples: Map<String, Value> = Split::PRESEALED.iter().map(|split| (split.as_str().to_owned(), json!(frozen.by_split[split.index()].len()))).collect();
    let (development, holdout) = (&corpora.development, &corpora.holdout);
    Ok(json!({
        "schema_version": 1,
        "policy": format!("keyswitch-intent-{}-preseal-holdout", release_name()?),
        "model_loaded": false,
        "metrics_evaluated": false,
        "development": {
            "role": "hard-negative-presealed-roles",
            "rank_namespace": development.rank_namespace,
            "choice_namespace": development.choice_namespace,
            "corpus_sha256": development.corpus_sha256,
            "signature_count": corpora.development_signatures.len(),
            "words_by_group": by_group(&development.words_by_group),
            "frozen_source": {"path": policy["source"]["path"], "sha256": policy["source"]["sha256"], "bytes": policy["source"]["bytes"]},
            "role_namespace": policy["role_namespace"],
            "role_words_by_group": roles,
            "examples_by_role": examples,
            "training_example_weight": policy["training_example_weight"],
        },
        "holdout": {
            "role": "independent-release-holdout",
            "rank_namespace": holdout.rank_namespace,
            "choice_namespace": holdout.choice_namespace,
            "corpus_sha256": holdout.corpus_sha256,
            "signature_count": corpora.holdout_signatures.len(),
            "words_by_group": by_group(&holdout.words_by_group),
        },
        "sealed_dataset_exclusions": {"signature_count": corpora.sealed_index.signatures.len(), "sha256": corpora.sealed_index.sha256},
        "combined_holdout_exclusions": {"signature_count": corpora.combined_exclusions.len(), "sha256": signature_set_fingerprint(corpora.combined_exclusions.iter().map(String::as_str))},
        "overlap_counts": {"development_holdout": corpora.development_signatures.intersection(&corpora.holdout_signatures).count(), "sealed_holdout": corpora.sealed_index.signatures.intersection(&corpora.holdout_signatures).count()},
    }))
}

/// `_compact_rows`: one record per signature, after proving the rows are its symmetric expansion.
fn compact_rows(examples: &[LexicalExample]) -> Result<Vec<Value>, String> {
    let mut grouped: BTreeMap<&str, Vec<&LexicalExample>> = BTreeMap::new();
    for example in examples {
        grouped.entry(&example.base_signature).or_default().push(example);
    }
    let mut records: Vec<(i64, String, Value)> = Vec::new();
    for (base, rows) in grouped {
        let physical = base.strip_prefix(SIGNATURE_PREFIX).filter(|_| rows.len() == TRIGGERS.len() * PAIRED_ROWS_PER_TRIGGER).ok_or("development corpus has an invalid signature group")?;
        let anchor = rows.iter().find(|row| row.trigger == CorrectionTrigger::Space && !row.label).ok_or("development corpus has duplicate trigger rows")?;
        if !matches!(anchor.source_group, 0 | 1) {
            return Err("development corpus has an invalid language group".to_owned());
        }
        for trigger in TRIGGERS {
            for (original, alternative, source_group, label) in [(&anchor.original, &anchor.alternative, anchor.source_group, false), (&anchor.alternative, &anchor.original, 1 - anchor.source_group, true)] {
                let expected = plain_row(original, alternative, source_group, trigger, label, base, &anchor.variant_kind);
                if !rows.iter().any(|row| **row == expected) {
                    return Err("development corpus rows are not trigger-symmetric".to_owned());
                }
            }
        }
        records.push((anchor.source_group, physical.to_owned(), json!({"physical_signature": physical, "correct_group": anchor.source_group, "correct_typo": anchor.original, "wrong_typo": anchor.alternative, "variant_kind": anchor.variant_kind})));
    }
    records.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    Ok(records.into_iter().map(|record| record.2).collect())
}

/// `build_frozen_development_corpus`.
pub fn frozen_development_corpus(corpora: &Corpora) -> Result<Value, String> {
    let development = &corpora.development;
    let rows = compact_rows(&development.examples)?;
    let signatures: BTreeSet<&str> = rows.iter().map(|row| row["physical_signature"].as_str().expect("a signature")).collect();
    if signatures.len() != corpora.development_signatures.len() || !signatures.iter().all(|signature| corpora.development_signatures.contains(*signature)) {
        return Err("compact development signatures changed during freezing".to_owned());
    }
    let counts = [0, 1].map(|group| rows.iter().filter(|row| row["correct_group"] == json!(group)).count());
    if counts != development.words_by_group {
        return Err("compact development group counts are inconsistent".to_owned());
    }
    let source = |group: usize| { let item = &development.dictionary_provenance[group]; json!({"locale": item.locale, "dictionary_sha256": item.dictionary.sha256, "dictionary_bytes": item.dictionary.bytes, "affix_sha256": item.affix.sha256, "affix_bytes": item.affix.bytes}) };
    Ok(json!({
        "schema_version": 1,
        "policy": dataset::HARD_NEGATIVE_POLICY,
        "role_namespace": dataset::ROLE_NAMESPACE,
        "rank_namespace": development.rank_namespace,
        "choice_namespace": development.choice_namespace,
        "expanded_corpus_sha256": development.corpus_sha256,
        "physical_signatures_sha256": signature_set_fingerprint(signatures.iter().copied()),
        "signature_count": signatures.len(),
        "words_by_group": by_group(&counts),
        "source_provenance": {"0": source(0), "1": source(1)},
        "rows": rows,
    }))
}
