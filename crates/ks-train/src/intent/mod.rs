//! The layout-intent model: a linear n-gram classifier over physical keys (`train_intent_model`, `evaluate_intent_model`).
pub mod dataset;
pub mod evaluate;
pub mod external;
pub mod fit;
pub mod pipeline;
pub mod probe;
pub mod seal;
pub mod train;

use crate::corpus::read_json;
use dataset::{DatasetConfig, Split};
use ks_lang::detector::ProtectedTokens;
use std::path::Path;

const SAFETY_COLLISION_MIN_WORD_CHARACTERS: usize = 3;

/// The stages up to the sealed dataset, each printed for comparison with another trainer's.
pub fn dataset_stages(root: &Path) -> Result<(), String> {
    let config = read_json(&root.join("model/intent_v1/config.json"))?;
    let shape = &config["dataset"];
    let number = |name: &str| shape[name].as_u64().map(ks_cast::To::to::<usize>).ok_or_else(|| format!("dataset.{name}"));
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
    // A dictionary path in a report is the absolute one the handle was opened with.
    let root = &root.canonicalize().map_err(|error| format!("{}: {error}", root.display()))?;
    if arguments.iter().any(|argument| argument == "--dataset") {
        return dataset_stages(root);
    }
    if arguments.iter().any(|argument| argument == "--verify") {
        let workers = arguments.iter().position(|argument| argument == "--jobs").and_then(|place| arguments.get(place + 1)).and_then(|value| value.parse().ok()).unwrap_or(4);
        return verify_replay(root, workers);
    }
    if arguments.iter().any(|argument| argument == "--preseal" || argument == "--freeze") {
        return receipts(root, arguments);
    }
    if arguments.iter().any(|argument| argument == "--evaluate") {
        return strict_evaluation(root, arguments);
    }
    if arguments.iter().any(|argument| argument == "--fit") {
        return fit_and_publish(root, arguments);
    }
    if arguments.iter().any(|argument| argument == "--reject") {
        return write_rejection(root, arguments);
    }
    if arguments.iter().any(|argument| argument == "--probe") {
        println!("{}", pretty(&probe::measure()));
        return Ok(());
    }
    Err("--dataset, --verify, --preseal, --freeze, --fit, --evaluate, --reject or --probe".to_owned())
}

/// The frozen dictionaries: a dictionary is an input of the evaluation, and an `apt upgrade` must not move what is pinned
/// in the config (`_use_frozen_hunspell_dictionaries`).
pub fn frozen_dictionaries(root: &Path) -> Result<std::path::PathBuf, String> {
    let frozen = root.join("model/intent_v1/sources/hunspell");
    for name in ["en_US.aff", "en_US.dic", "ru_RU.aff", "ru_RU.dic"] {
        if !frozen.join(name).is_file() {
            return Err(format!("the frozen Hunspell dictionaries are missing: {name}"));
        }
    }
    Ok(frozen)
}

/// The runtime language models over the frozen word lists and dictionaries, whatever the system has installed.
pub fn runtime_models(root: &Path) -> Result<(std::sync::Arc<ks_lang::Resources>, [std::sync::Arc<ks_lang::LanguageModel>; 2]), String> {
    let frozen = frozen_dictionaries(root)?;
    let roots = [root.join("model/intent_v1/sources")];
    let resources = ks_lang::Resources::new(root.join("resources"));
    let load = |locale: &str| std::sync::Arc::new(ks_lang::LanguageModel::load(locale, std::iter::empty::<&str>(), &roots, Box::new(ks_hunspell::HunspellDictionary::in_roots(locale, std::slice::from_ref(&frozen)))));
    let models = [load("en_US"), load("ru_RU")];
    Ok((resources, models))
}

fn pretty(value: &serde_json::Value) -> String {
    ks_text::json_dumps(value, &ks_text::JsonOptions { ensure_ascii: false, indent: Some(ks_text::Indent::Spaces(2)), ..ks_text::JsonOptions::default() })
}

fn option<'a>(arguments: &'a [String], name: &str) -> Option<&'a String> {
    arguments.iter().position(|argument| argument == name).and_then(|place| arguments.get(place + 1))
}

/// `evaluate_intent_model`.
fn strict_evaluation(root: &Path, arguments: &[String]) -> Result<(), String> {
    let path = |name: &str, default: &str| option(arguments, name).map_or_else(|| root.join(default), std::path::PathBuf::from);
    let size = |name: &str, default: usize| option(arguments, name).map_or(Ok(default), |value| value.parse().map_err(|_| format!("{name} takes a number")));
    let (artifact, manifest) = (path("--artifact", "resources/models/layout_intent_v1.ksm"), path("--manifest", "model/intent_v1/manifest.json"));
    let options = evaluate::Options { root, artifact: &artifact, manifest: &manifest, comparison_sample: size("--comparison-sample", evaluate::COMPARISON_SAMPLE)?, latency_sample: size("--latency-sample", evaluate::LATENCY_SAMPLE)?, strict: arguments.iter().any(|argument| argument == "--strict"), provenance_only: arguments.iter().any(|argument| argument == "--provenance-only"), workers: size("--jobs", 4)? };
    let (report, status) = evaluate::evaluate(&options)?;
    match option(arguments, "--output") {
        Some(output) => std::fs::write(output, report + "\n").map_err(|error| format!("{output}: {error}"))?,
        None => println!("{report}"),
    }
    if status != 0 {
        return Err("the strict gates did not pass".to_owned());
    }
    Ok(())
}

/// `train_intent_model`.
fn fit_and_publish(root: &Path, arguments: &[String]) -> Result<(), String> {
    let workers = option(arguments, "--jobs").map_or(Ok(4), |value| value.parse().map_err(|_| "--jobs takes a number".to_owned()))?;
    let mut outputs = train::Outputs::defaults(root);
    for (name, target) in [("--artifact", &mut outputs.artifact), ("--manifest", &mut outputs.manifest), ("--report", &mut outputs.report), ("--environment", &mut outputs.environment)] {
        if let Some(path) = option(arguments, name) {
            *target = path.into();
        }
    }
    outputs.diagnostic = option(arguments, "--diagnostic").map(Into::into);
    outputs.dry_run = arguments.iter().any(|argument| argument == "--dry-run");
    if train::train(root, workers, &outputs)? != 0 {
        return Err("the quality gates did not pass".to_owned());
    }
    Ok(())
}

/// `write_intent_rejection`.
fn write_rejection(root: &Path, arguments: &[String]) -> Result<(), String> {
    let version: u32 = option(arguments, "--version").ok_or("--reject needs --version")?.parse().map_err(|_| "--version takes a number")?;
    let report = option(arguments, "--strict-report").ok_or("--reject needs --strict-report")?;
    let remediation = "Treat the disclosed sealed and external results as development evidence only. Rotate the sealed split, registry, hard-negative source and independent holdout namespaces before the next candidate and evaluate that candidate exactly once.";
    let output = option(arguments, "--output").map_or_else(|| root.join(format!("model/intent_v1/rejection-v{version}.json")), std::path::PathBuf::from);
    if output.exists() {
        return Err(format!("refusing to overwrite the existing receipt {}", output.display()));
    }
    let receipt = train::rejection_receipt(root, version, Path::new(report), option(arguments, "--remediation").map_or(remediation, String::as_str))?;
    std::fs::write(&output, pretty(&receipt) + "\n").map_err(|error| format!("{}: {error}", output.display()))?;
    println!("{}", output.display());
    Ok(())
}

/// `preseal_intent_holdout` and `freeze_intent_development_corpus`: what they print or write.
fn receipts(root: &Path, arguments: &[String]) -> Result<(), String> {
    let (resources, models) = runtime_models(root)?;
    let references = [&*models[0], &*models[1]];
    let protected = resources.protected_tokens();
    let output = arguments.iter().position(|argument| argument == "--output").and_then(|place| arguments.get(place + 1));
    if arguments.iter().any(|argument| argument == "--preseal") {
        let (config, corpora) = external::model_blind_corpora(root, &references, &protected, true)?;
        let text = pretty(&external::preseal_receipt(&config, &corpora)?) + "\n";
        return if let Some(path) = output { std::fs::write(path, text).map_err(|error| format!("{path}: {error}")) } else {
            print!("{text}");
            Ok(())
        };
    }
    let (_, corpora) = external::model_blind_corpora(root, &references, &protected, false)?;
    let text = pretty(&external::frozen_development_corpus(&corpora)?) + "\n";
    let path = output.ok_or("--freeze needs --output")?;
    std::fs::write(path, text).map_err(|error| format!("{path}: {error}"))
}

/// Replays the fit on the frozen sources. The parameters, every section of the manifest, the identity of the candidate,
/// the answer of the sealed test and the model file itself must be the recorded ones, to the byte. The toolchain block is
/// taken from the record: it names the sources that wrote the model.
pub fn verify_replay(root: &Path, workers: usize) -> Result<(), String> {
    let candidate = pipeline::fit_candidate(root, workers)?;
    if candidate.presealed_gate["passed"] != serde_json::json!(true) {
        return Err("the replay does not pass the pre-seal gate".to_owned());
    }
    let recorded = read_json(&root.join("model/intent_v1/manifest.json"))?;
    let config_sha256 = recorded["config_sha256"].as_str().ok_or("config_sha256")?.to_owned();
    let metadata = candidate.metadata(&config_sha256, &recorded["toolchain"]);
    let candidate_sha256 = seal::candidate_sha256(dataset::SPLIT_NAMESPACE, &config_sha256, &candidate.candidate_dataset_sha256, &metadata);
    let (built, gates_passed) = pipeline::open_sealed_test(candidate, root, workers)?;
    let mut differences = 0;
    for (name, value) in &built.sections {
        if &recorded[name] != value {
            differences += 1;
            eprintln!("differs: {name}");
        }
    }
    let manifest = recorded.as_object().ok_or("a manifest is an object")?;
    let mut rebuilt = train::manifest_of(&built, &config_sha256, &recorded["toolchain"], &recorded["sealed_evaluation"])?;
    let rebuilt_artifact = train::seal_artifact(&built, &mut rebuilt)?;
    let same_file = |path: &str, text: String| std::fs::read(root.join(path)).is_ok_and(|bytes| bytes == text.into_bytes());
    let model = ks_lang::intent_model::LinearNgramModel::load(&root.join("resources/models/layout_intent_v1.ksm")).map_err(|error| error.name.to_string())?;
    let parameters = seal::runtime_parameters(&model);
    let registry = read_json(&root.join(built.raw_config["sealed_evaluation"]["registry_path"].as_str().ok_or("a registry path")?))?;
    let outcome = read_json(&root.join(seal::outcome_path(built.raw_config["sealed_evaluation"]["registry_path"].as_str().ok_or("a registry path")?)?))?;
    let checks = [
        ("model parameters", built.parameters == parameters),
        ("candidate", registry["candidate_sha256"].as_str() == Some(candidate_sha256.as_str())),
        ("outcome", outcome["sealed_outcome_sha256"].as_str() == Some(seal::outcome_sha256(&built.sections)?.as_str())),
        ("build provenance", manifest["build_provenance_sha256"].as_str() == Some(seal::build_provenance_sha256(manifest)?.as_str())),
        ("model file", std::fs::read(root.join("resources/models/layout_intent_v1.ksm")).is_ok_and(|bytes| bytes == rebuilt_artifact)),
        ("manifest file", same_file("model/intent_v1/manifest.json", train::pretty_sorted(&serde_json::Value::Object(rebuilt.clone())))),
        ("test report file", same_file("model/intent_v1/test-report.json", train::pretty_sorted(&train::report_of(&rebuilt)))),
        ("gates", gates_passed),
    ];
    for (name, equal) in checks {
        println!("{name}: {}", if equal { "same" } else { "DIFFERS" });
        differences += usize::from(!equal);
    }
    if differences != 0 {
        return Err(format!("{differences} sections differ from the recorded model"));
    }
    println!("{} reproduces", model.model_version);
    Ok(())
}
