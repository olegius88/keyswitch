//! `ks-train fit --tree <root> --corpus <folder> --output <folder> [--recipe <file>] [--ledger <folder>] [--jobs N] [--stages <folder>]`
//! `ks-train prefix --tree <root> --output <folder> [--recipe <file>] [--reuse-installed] [--jobs N]`
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
