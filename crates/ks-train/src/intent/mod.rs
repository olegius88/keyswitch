//! The layout-intent model: a linear n-gram classifier over physical keys (`train_intent_model`, `evaluate_intent_model`).
pub mod dataset;
pub mod fit;
pub mod pipeline;

use crate::corpus::read_json;
use dataset::{DatasetConfig, Split};
use ks_lang::detector::ProtectedTokens;
use std::path::Path;

const SAFETY_COLLISION_MIN_WORD_CHARACTERS: usize = 3;

/// The stages up to the sealed dataset, each printed for comparison with another trainer's.
pub fn dataset_stages(root: &Path) -> Result<(), String> {
    let config = read_json(&root.join("model/intent_v1/config.json"))?;
    let shape = &config["dataset"];
    let number = |name: &str| shape[name].as_u64().map(|value| value as usize).ok_or_else(|| format!("dataset.{name}"));
    let dataset_config = DatasetConfig { minimum_word_length: number("minimum_word_length")?, maximum_word_length: number("maximum_word_length")?, typo_augmentations: number("typo_augmentations")? };
    let languages = &config["sources"]["languages"];
    let english = dataset::load_onboard_unigrams(&dataset::read_frozen(root, &languages["en_US"])?, 0, SAFETY_COLLISION_MIN_WORD_CHARACTERS, dataset_config.maximum_word_length);
    let russian = dataset::load_onboard_unigrams(&dataset::read_frozen(root, &languages["ru_RU"])?, 1, SAFETY_COLLISION_MIN_WORD_CHARACTERS, dataset_config.maximum_word_length);
    println!("words {} {}", english.len(), russian.len());
    let mut all = english;
    all.extend(russian);
    let prepared = dataset::prepare_lexicon(all, dataset_config.minimum_word_length);
    let sizes = |rows: &[Vec<dataset::LexiconWord>]| rows.iter().map(Vec::len).collect::<Vec<_>>();
    println!("prepared {:?} {}", sizes(&prepared.words_by_split), prepared.collisions.len());
    let protected = ProtectedTokens::load(&root.join("resources"));
    let raw = dataset::read_frozen(root, &config["hard_negative_development"]["source"])?;
    let hard = dataset::decode_hard_negative_corpus(&raw, &config, &dataset_config, &protected)?;
    let built = dataset::build_dataset(&prepared, &dataset_config, &Split::PRESEALED);
    let lengths = |bundle: &dataset::DatasetBundle| Split::ALL.iter().map(|split| bundle.split(*split).len()).collect::<Vec<_>>();
    println!("built {:?} {} {} {}", lengths(&built), built.safety.len(), built.variant_quarantine.occurrences.len(), built.variant_quarantine.sha256);
    println!("hard {}", hard.provenance());
    let merged = dataset::merge_hard_negative_development(built, &hard);
    dataset::assert_no_split_leakage(&merged, &Split::PRESEALED)?;
    println!("candidate {}", dataset::dataset_fingerprint(&merged));
    println!("scorer {}", dataset::train_only_scorer_provenance(&prepared, &merged.variant_quarantine));
    let sealed = dataset::build_dataset(&prepared, &dataset_config, &[Split::Test]);
    dataset::assert_no_split_leakage(&sealed, &[Split::Test])?;
    let whole = dataset::merge_sealed_test_dataset(merged, sealed)?;
    dataset::assert_no_split_leakage(&whole, &Split::PRESEALED)?;
    println!("sealed {:?} {} {} {}", lengths(&whole), whole.safety.len(), whole.sealed_test_exclusion_signatures.len(), dataset::dataset_fingerprint(&whole));
    Ok(())
}

/// The commands of the model.
pub fn command(root: &Path, arguments: &[String]) -> Result<(), String> {
    if arguments.iter().any(|argument| argument == "--dataset") {
        return dataset_stages(root);
    }
    if arguments.iter().any(|argument| argument == "--verify") {
        let workers = arguments.iter().position(|argument| argument == "--jobs").and_then(|place| arguments.get(place + 1)).and_then(|value| value.parse().ok()).unwrap_or(4);
        return verify_replay(root, workers);
    }
    Err("--dataset or --verify".to_owned())
}

/// Replays the fit on the frozen sources: the parameters of the model file and every section of the manifest must be
/// the recorded ones.
pub fn verify_replay(root: &Path, workers: usize) -> Result<(), String> {
    let built = pipeline::fit_candidate(root, workers, true)?;
    let recorded = read_json(&root.join("model/intent_v1/manifest.json"))?;
    let mut differences = 0;
    for (name, value) in &built.sections {
        if name == "presealed_gates_passed" {
            continue;
        }
        if &recorded[name] != value {
            differences += 1;
            eprintln!("differs: {name}");
        }
    }
    let model = ks_lang::intent_model::LinearNgramModel::load(&root.join("resources/models/layout_intent_v1.ksm")).map_err(|error| error.name.to_string())?;
    let parameters = &built.candidate_parameters;
    let checks = [
        ("payload_sha256", parameters["payload_sha256"].as_str() == Some(model.payload_sha256.as_str())),
        ("weight_scale", parameters["weight_scale_hex"].as_str() == Some(ks_text::pynum::float_hex(model.weight_scale).as_str())),
        ("bias", parameters["bias_hex"].as_str() == Some(ks_text::pynum::float_hex(model.bias).as_str())),
        ("veto", parameters["veto_threshold_hex"].as_str() == Some(ks_text::pynum::float_hex(model.veto_threshold).as_str())),
    ];
    for (name, equal) in checks {
        println!("{name}: {}", if equal { "same" } else { "DIFFERS" });
        differences += usize::from(!equal);
    }
    println!("dataset {} candidate {}", built.dataset_sha256, built.candidate_dataset_sha256);
    println!("gates passed: {}", built.gates_passed);
    if differences != 0 {
        return Err(format!("{differences} sections differ from the recorded model"));
    }
    println!("{} reproduces", model.model_version);
    Ok(())
}
