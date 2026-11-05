//! `ks-train fit --tree <root> --corpus <folder> --output <folder> [--recipe <file>] [--ledger <folder>] [--jobs N] [--stages <folder>]`
//! `ks-train prefix --tree <root> --output <folder> [--recipe <file>] [--reuse-installed] [--jobs N]`
//! `ks-train evaluate --tree <root> --candidate <file> --seal <file> --prefix-candidate <file> --prefix-seal <file> --corpus <folder> --output <file> [--split development|test] [--jobs N]`
//! `ks-train holdout --tree <root> --source-root <folder> --treebank <name>... --sid-directory|--arch-directory|--rpm-directory|--alpine-directory <folder> --base <folder> ... --output <folder>`
//! `ks-train capture --tree <root> --manifest <file> --messages <folder> --model <file> [--only <file>] [--verify] [--jobs N]`
//! `ks-train mixed --tree <root> --messages <file> --count N --output <file> [--model <file>] [--kinds k,k] [--seed S] [--russian-share R] [--typo-rate R] [--prefix-rate R --code <file>] [--edit] [--jobs N]`
//! `ks-train devchat --tree <root> --split dev|test --directory <folder> --candidate <file> [--released <file>] [--baseline <file>] --output <file> [--ledger <folder>] [--jobs N]`
//! `ks-train compare --tree <root> --corpus <folder> --artifact NAME=PATH... [--output <file>] [--jobs N]`
//! `ks-train verify --tree <root> [--receipt <file> --artifact <file> --prefix <file> --evidence-only] [--report <file>]`
//! `ks-train promote --tree <root> --candidate <folder> --prefix-candidate <folder> --corpus <folder> --report <file> --released <folder> --devchat <folder> [--check-only] [--jobs N]`
//! `ks-train boundary --tree <root> --freeze|--train|--test|--verify|--promote [--directory <folder>] [--output <folder>]`
//! `ks-train engine boundary|prefix --tree <root> [--verify|--refresh-runtime|--output <file>] [--jobs N]`
//! `ks-train fitting --tree <root> --base <folder> --output <folder> --namespace <text> --tatoeba-directory <folder> --tatoeba-language <code>... ...`
//!
//! Fits a candidate of the context action model or of the prefix model. Never reads a test split. With `--stages` every stage is written down, one
//! line per frame with floats as C99 hex, to be compared with another trainer's.
use ks_train::fit::{FitRequest, NoStages, fit};
use std::path::{Path, PathBuf};

fn option<'a>(arguments: &'a [String], name: &str) -> Option<&'a str> {
    arguments.iter().position(|argument| argument == name).and_then(|place| arguments.get(place + 1)).map(String::as_str)
}

mod stages;

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.first().map(String::as_str) == Some("prefix") {
        let tree = PathBuf::from(option(&arguments, "--tree").expect("--tree is required"));
        let recipe = option(&arguments, "--recipe").map_or_else(|| tree.join("model/prefix_v2/recipe.json"), PathBuf::from);
        let output = PathBuf::from(option(&arguments, "--output").expect("--output is required"));
        let resource_dir = tree.join("resources");
        let jobs = option(&arguments, "--jobs").map_or(1, |value| value.parse().expect("--jobs is a number"));
        let request = ks_train::prefix::PrefixRequest { root: &tree, resource_dir: &resource_dir, recipe: &recipe, output: &output, jobs, reuse_installed: arguments.iter().any(|argument| argument == "--reuse-installed") };
        match ks_train::prefix::fit(&request) {
            Ok(seal) => {
                println!("{} calibration passed: {}", seal["model_version"].as_str().unwrap_or(""), seal["calibration_passed"]);
                std::process::exit(i32::from(seal["calibration_passed"].as_bool() != Some(true)));
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    }
    if arguments.first().map(String::as_str) == Some("evaluate") {
        let required = |name: &str| option(&arguments, name).map(PathBuf::from).unwrap_or_else(|| panic!("{name} is required"));
        let root = required("--tree");
        let resource_dir = root.join("resources");
        let (artifact, seal, prefix, prefix_seal, corpus, output) = (required("--candidate"), option(&arguments, "--seal").map(PathBuf::from).unwrap_or_default(), required("--prefix-candidate"), option(&arguments, "--prefix-seal").map(PathBuf::from).unwrap_or_default(), required("--corpus"), required("--output"));
        let ledger = option(&arguments, "--ledger").map_or_else(|| root.join(".t/reliable-release-2026-09-12/context-action-test-ledger"), PathBuf::from);
        let request = ks_train::evaluate::EvaluationRequest {
            root: &root,
            resource_dir: &resource_dir,
            artifact: &artifact,
            seal: &seal,
            prefix: &prefix,
            prefix_seal: &prefix_seal,
            corpus: &corpus,
            split: option(&arguments, "--split").unwrap_or("development"),
            output: &output,
            ledger: &ledger,
            jobs: option(&arguments, "--jobs").map_or(1, |value| value.parse().expect("--jobs is a number")),
            unsealed: arguments.iter().any(|argument| argument == "--unsealed"),
        };
        match ks_train::evaluate::evaluate(&request) {
            Ok(report) => {
                println!("promotion_passed: {}", report["promotion_passed"]);
                let development = request.split == "development";
                std::process::exit(i32::from(!(development || report["promotion_passed"].as_bool() == Some(true))));
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    }
    if arguments.first().map(String::as_str) == Some("engine") {
        let root = PathBuf::from(option(&arguments, "--tree").expect("--tree is required"));
        let resources = ks_lang::Resources::with_speller(root.join("resources"), ks_hunspell::hunspell_speller(ks_config::history::data_dir()));
        let flag = |name: &str| arguments.iter().any(|argument| argument == name);
        let jobs: usize = option(&arguments, "--jobs").map_or(4, |value| value.parse().expect("--jobs is a number"));
        let which = arguments.get(1).map_or("", String::as_str);
        if let Err(error) = ks_train::engine_reports::command(&root, &resources, which, flag("--verify"), flag("--refresh-runtime"), option(&arguments, "--output").map(Path::new), jobs) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if arguments.first().map(String::as_str) == Some("boundary") {
        let root = PathBuf::from(option(&arguments, "--tree").expect("--tree is required"));
        let resources = ks_lang::Resources::with_speller(root.join("resources"), ks_hunspell::hunspell_speller(ks_config::history::data_dir()));
        let directory = option(&arguments, "--directory").map_or_else(|| root.join("model/boundary_v2"), PathBuf::from);
        let output = option(&arguments, "--output").map_or_else(|| directory.clone(), PathBuf::from);
        let Some(action) = ["freeze", "train", "test", "verify", "promote"].into_iter().find(|name| arguments.iter().any(|argument| *argument == format!("--{name}"))) else {
            eprintln!("--freeze, --train, --test, --verify or --promote");
            std::process::exit(2);
        };
        if let Err(error) = ks_train::boundary::command(&root, &resources, action, &directory, &output) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if matches!(arguments.first().map(String::as_str), Some("compare" | "verify" | "promote")) {
        std::process::exit(cycle_command(&arguments));
    }
    if matches!(arguments.first().map(String::as_str), Some("capture" | "mixed" | "devchat")) {
        std::process::exit(typing_command(&arguments));
    }
    if matches!(arguments.first().map(String::as_str), Some("holdout" | "fitting")) {
        std::process::exit(freeze_command(&arguments));
    }
    if arguments.first().map(String::as_str) != Some("fit") {
        eprintln!("usage: ks-train fit --tree <root> --corpus <folder> --output <folder> [--recipe <file>] [--ledger <folder>] [--jobs N] [--stages <folder>]");
        std::process::exit(2);
    }
    let required = |name: &str| option(&arguments, name).map(PathBuf::from).unwrap_or_else(|| panic!("{name} is required"));
    let root = required("--tree");
    let recipe = option(&arguments, "--recipe").map_or_else(|| root.join("model/context_v3/recipe.json"), PathBuf::from);
    let ledger = option(&arguments, "--ledger").map_or_else(|| root.join(".t/reliable-release-2026-09-12/context-action-test-ledger"), PathBuf::from);
    let resource_dir = root.join("resources");
    let jobs = option(&arguments, "--jobs").map_or(1, |value| value.parse().expect("--jobs is a number"));
    let (corpus, output) = (required("--corpus"), required("--output"));
    let request = FitRequest { root: &root, resource_dir: &resource_dir, recipe: &recipe, corpus: &corpus, output: &output, ledger: &ledger, jobs };
    let result = match option(&arguments, "--stages") {
        Some(folder) => fit(&request, &mut stages::Writer::new(Path::new(folder))),
        None => fit(&request, &mut NoStages),
    };
    match result {
        Ok(seal) => std::process::exit(i32::from(seal["stage"].as_str() != Some("sealed-before-test"))),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

fn all_of<'a>(arguments: &'a [String], name: &str) -> Vec<&'a str> {
    arguments.iter().enumerate().filter(|(_, argument)| *argument == name).filter_map(|(place, _)| arguments.get(place + 1)).map(String::as_str).collect()
}

/// The two corpus freezes share their arguments.
fn freeze_command(arguments: &[String]) -> i32 {
    use ks_train::freeze::{self, FreezeRequest, TechnicalSource};
    let required = |name: &str| option(arguments, name).map(PathBuf::from).unwrap_or_else(|| panic!("{name} is required"));
    let root = required("--tree");
    let holdout = arguments[0] == "holdout";
    let (base, ud_origin, technical_origin, prefix_inventory) = (required("--base"), required("--ud-origin"), required("--technical-origin"), required("--prefix-inventory"));
    let lexicon = option(arguments, "--lexicon").map_or_else(|| root.join("resources/identifiers.json"), PathBuf::from);
    let ledger = option(arguments, "--ledger").map_or_else(|| root.join(".t/reliable-release-2026-09-12/context-action-test-ledger"), PathBuf::from);
    let namespace = option(arguments, "--namespace").expect("--namespace is required");
    let output = option(arguments, "--output").map(PathBuf::from);
    let report = option(arguments, "--report").map(PathBuf::from);
    let tatoeba = option(arguments, "--tatoeba-directory").map(PathBuf::from);
    let default_documents = if holdout { freeze::HOLDOUT_MAX_DOCUMENTS_PER_SOURCE } else { freeze::FITTING_DEFAULT_MAX_DOCUMENTS_PER_SOURCE };
    let request = FreezeRequest {
        root: &root,
        base: &base,
        ud_origin: &ud_origin,
        technical_origin: &technical_origin,
        prefix_inventory: &prefix_inventory,
        lexicon: &lexicon,
        ledger: &ledger,
        namespace,
        output: output.as_deref(),
        report: report.as_deref(),
        extra_exposure: all_of(arguments, "--extra-exposure").into_iter().map(PathBuf::from).collect(),
        tatoeba_directory: tatoeba.as_deref(),
        tatoeba_languages: all_of(arguments, "--tatoeba-language").into_iter().map(str::to_owned).collect(),
        max_documents: option(arguments, "--max-documents").map_or(default_documents, |value| value.parse().expect("a number")),
        max_sentences_per_document: option(arguments, "--max-sentences-per-document").map_or(freeze::DEFAULT_MAX_SENTENCES_PER_DOCUMENT, |value| value.parse().expect("a number")),
    };
    let result = if holdout {
        let technical: Vec<(&str, PathBuf)> = ["--sid-directory", "--arch-directory", "--rpm-directory", "--alpine-directory"].into_iter().filter_map(|name| option(arguments, name).map(|path| (name, PathBuf::from(path)))).collect();
        let [(name, directory)] = technical.as_slice() else {
            eprintln!("exactly one technical source is required: --sid-directory, --arch-directory, --rpm-directory or --alpine-directory");
            return 2;
        };
        let source = match *name {
            "--sid-directory" => TechnicalSource::Sid(directory),
            "--arch-directory" => TechnicalSource::Arch(directory),
            "--rpm-directory" => TechnicalSource::Rpm(directory),
            _ => TechnicalSource::Alpine(directory),
        };
        let treebanks: Vec<String> = all_of(arguments, "--treebank").into_iter().map(str::to_owned).collect();
        freeze::freeze_holdout(&request, &required("--source-root"), &treebanks, &source)
    } else {
        freeze::freeze_fitting(&request)
    };
    match result {
        Ok(summary) => {
            println!("{summary}");
            0
        }
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

/// The commands of the typing bench.
fn typing_command(arguments: &[String]) -> i32 {
    let required = |name: &str| option(arguments, name).map(PathBuf::from).unwrap_or_else(|| panic!("{name} is required"));
    let root = required("--tree");
    let resource_dir = root.join("resources");
    let jobs: usize = option(arguments, "--jobs").map_or(4, |value| value.parse().expect("--jobs is a number"));
    let installed = resource_dir.join("models/context_policy_v1.json");
    let result = match arguments[0].as_str() {
        "capture" => ks_train::mixed::capture(&resource_dir, &required("--manifest"), &required("--messages"), &required("--model"), option(arguments, "--only"), arguments.iter().any(|argument| argument == "--verify"), jobs),
        "mixed" => {
            let number = |name: &str, default: f64| option(arguments, name).map_or(default, |value| value.parse().expect("a number"));
            let kinds: Vec<&str> = option(arguments, "--kinds").unwrap_or("comment").split(',').collect();
            let recipe = serde_json::json!({"seed": option(arguments, "--seed").unwrap_or("mixbench-v1"), "kinds": kinds, "count": number("--count", 0.0) as i64, "russian_share": number("--russian-share", 0.25),
                "typo_rate": number("--typo-rate", 0.0), "prefix_rate": number("--prefix-rate", 0.0), "edit": arguments.iter().any(|argument| argument == "--edit")});
            let code = option(arguments, "--code").map(PathBuf::from);
            let model = option(arguments, "--model").map_or_else(|| installed.clone(), PathBuf::from);
            ks_train::mixed::evaluate(&resource_dir, &required("--messages"), &recipe, code.as_deref(), &model, &required("--output"), jobs)
        }
        _ => {
            let ledger_root = root.join(".t/reliable-release-2026-09-12");
            let (directory, candidate, output) = (required("--directory"), required("--candidate"), required("--output"));
            let released = option(arguments, "--released").map_or_else(|| installed.clone(), PathBuf::from);
            let baseline = option(arguments, "--baseline").map_or_else(|| root.join("model/context_v3/baseline-context-v1.json"), PathBuf::from);
            let ledger = option(arguments, "--ledger").map_or_else(|| ledger_root.join("devchat-test-ledger"), PathBuf::from);
            let sequences = option(arguments, "--sequences-ledger").map_or_else(|| ledger_root.join("context-action-test-ledger"), PathBuf::from);
            ks_train::mixed::devchat(&ks_train::mixed::DevchatRequest {
                root: &root,
                resource_dir: &resource_dir,
                split: option(arguments, "--split").expect("--split is required"),
                directory: &directory,
                candidate: &candidate,
                released: &released,
                baseline: &baseline,
                output: &output,
                ledger: &ledger,
                sequences_ledger: &sequences,
                workers: jobs,
            })
            .map(|report| serde_json::json!({"floors": report["floors"], "messages": report["messages"]}))
        }
    };
    match result {
        Ok(summary) => {
            println!("{summary}");
            0
        }
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

/// The steps of a model cycle after a fit.
fn cycle_command(arguments: &[String]) -> i32 {
    let required = |name: &str| option(arguments, name).map(PathBuf::from).unwrap_or_else(|| panic!("{name} is required"));
    let root = required("--tree");
    let resource_dir = root.join("resources");
    let jobs: usize = option(arguments, "--jobs").map_or(4, |value| value.parse().expect("--jobs is a number"));
    let ledger_root = root.join(".t/reliable-release-2026-09-12");
    let ledger = option(arguments, "--ledger").map_or_else(|| ledger_root.join("context-action-test-ledger"), PathBuf::from);
    let devchat_ledger = option(arguments, "--devchat-ledger").map_or_else(|| ledger_root.join("devchat-test-ledger"), PathBuf::from);
    let result: Result<i32, String> = match arguments[0].as_str() {
        "verify" => (|| {
            let path_of = |name: &str, default: PathBuf| option(arguments, name).map_or(default, PathBuf::from);
            let receipt = ks_train::corpus::read_json(&path_of("--receipt", root.join("model/context_v3/release-receipt.json")))?;
            let (artifact, prefix) = (path_of("--artifact", resource_dir.join("models/context_policy_v1.json")), path_of("--prefix", resource_dir.join("models/prefix_policy_v1.json")));
            let models = resource_dir.join("models");
            if arguments.iter().any(|argument| argument == "--evidence-only") {
                ks_train::receipt::validate_evidence(&receipt, &artifact, &root, &prefix, &models)?;
            } else {
                ks_train::receipt::validate_receipt(&receipt, &artifact, &root, &prefix, &models)?;
            }
            if let Some(path) = option(arguments, "--report") {
                let report = ks_train::corpus::read_json(Path::new(path))?;
                ks_train::receipt::validate_full_report(&report, &report["identity"])?;
                println!("the private cases give the counts of the report");
            }
            println!("{} verified", receipt["model_version"].as_str().unwrap_or(""));
            Ok(0)
        })(),
        "compare" => (|| {
            let artifacts: Vec<(String, PathBuf)> = all_of(arguments, "--artifact").into_iter().map(|text| { let (name, path) = text.split_once('=').expect("an artifact is NAME=PATH"); (name.to_owned(), PathBuf::from(path)) }).collect();
            let options = ks_train::corpus::read_json(&root.join("model/context_v3/recipe.json"))?;
            let inputs = ks_train::fit::load_inputs(&root, &resource_dir, &required("--corpus"), options, &ledger)?;
            let report = ks_train::compare::compare(&inputs, &artifacts, jobs)?;
            if let Some(path) = option(arguments, "--output") {
                let format = ks_text::JsonOptions { ensure_ascii: false, sort_keys: true, indent: Some(ks_text::Indent::Spaces(1)), ..ks_text::JsonOptions::default() };
                std::fs::write(path, ks_text::json_dumps(&report, &format) + "\n").map_err(|error| error.to_string())?;
            }
            for (split, profiles) in report.as_object().into_iter().flatten() {
                for (profile, pairs) in profiles.as_object().into_iter().flatten() {
                    for (name, scored) in pairs.as_object().into_iter().flatten() {
                        println!("{split} {profile} {name} ({}): {}", scored["model_version"].as_str().unwrap_or(""), scored["by_class"]["all"]);
                    }
                }
            }
            Ok(0)
        })(),
        _ => {
            let (candidate, prefix_candidate, corpus, report, released, devchat) = (required("--candidate"), required("--prefix-candidate"), required("--corpus"), required("--report"), required("--released"), required("--devchat"));
            ks_train::promote::promote(&ks_train::promote::PromoteRequest {
                root: &root,
                candidate: &candidate,
                prefix_candidate: &prefix_candidate,
                corpus: &corpus,
                report: &report,
                released_rows: &released,
                devchat_directory: &devchat,
                ledger: &ledger,
                devchat_ledger: &devchat_ledger,
                check_only: arguments.iter().any(|argument| argument == "--check-only"),
                jobs,
            })
        }
    };
    match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}
