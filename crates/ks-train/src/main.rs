//! `ks-train fit --tree <root> --corpus <folder> --output <folder> [--recipe <file>] [--ledger <folder>] [--jobs N] [--stages <folder>]`
//! `ks-train prefix --tree <root> --output <folder> [--recipe <file>] [--reuse-installed] [--jobs N]`
//! `ks-train evaluate --tree <root> --candidate <file> --seal <file> --prefix-candidate <file> --prefix-seal <file> --corpus <folder> --output <file> [--split development|test] [--jobs N]`
//! `ks-train holdout --tree <root> --source-root <folder> --treebank <name>... --sid-directory|--arch-directory|--rpm-directory|--alpine-directory <folder> --base <folder> ... --output <folder>`
//! `ks-train capture --tree <root> --manifest <file> --messages <folder> --model <file> [--only <file>] [--verify] [--jobs N]`
//! `ks-train mixed --tree <root> --messages <file> --count N --output <file> [--model <file>] [--kinds k,k] [--seed S] [--russian-share R] [--typo-rate R] [--prefix-rate R --code <file>] [--edit] [--jobs N]`
//! `ks-train devchat --tree <root> --split dev|test --directory <folder> --candidate <file> [--released <file>] [--baseline <file>] --output <file> [--ledger <folder>] [--jobs N]`
//! `ks-train compare --tree <root> --corpus <folder> --artifact NAME=PATH... [--output <file>] [--jobs N]`
//! `ks-train verify --tree <root> [--receipt <file> --artifact <file> --prefix <file> --evidence-only] [--report <file>]`
//! `ks-train promote --tree <root> --candidate <folder> --prefix-candidate <folder> --corpus <folder> --report <file> --released <folder> --devchat <folder> [--field-root <tree>] [--check-only] [--jobs N]`
//! `ks-train boundary --tree <root> --freeze|--train|--test|--verify|--promote [--directory <folder>] [--output <folder>]`
//! `ks-train engine boundary|prefix --tree <root> [--verify|--refresh-runtime|--output <file>] [--jobs N]`
//! `ks-train messages --out <dir> [--so <zst> --tatoeba <dir>] [--taiga <dir>]`, `terms --messages <dir> --out <file>`, `alnum --messages <dir> [--members <file>...] --out <file>`,
//! `short --leipzig <dir> --out <file>`, `sources --manifest <file> --messages <dir> --tatoeba <dir>`,
//! `select --messages <file> [--members <file>...] --class <name> --development-share R --out <dir> [--prefix P] [--only-ids <file>]`,
//! `devchat-messages --comments <dir> --dump <zst> --out <dir> [--earlier <file>...]` (each with `--tree <root>`)
//! `ks-train typo-frequency --tree <root> [--private <folder>] [--verify]`
//! `ks-train carry --tree <root> --pair <file> --refit <head>... --output <file>`, `doctor --tree <root> [--corpus <folder>] [--ledger <folder>] [--stage <name>...]`,
//! `supplement --tree <root> --verify | --input <file> [--output <file>] [--receipt <file>] [--removed-list <file>]`,
//! `apt-build --tree <root> --repository <folder> --package <file>... [--passphrase-file <file>]`, `apt-verify --tree <root> --repository <folder> --expect-version <version> [--public-key <file>]`
//! `ks-train ortho --tree <root> --fit|--verify|--test|--promote|--inputs|--lexicon-sources [--web-source <file>] [--prose-source <folder>]|--freeze-inputs [--output <folder>]`
//! `ks-train base freeze|reconcile|technical|merge|inventory --tree <root> ...`
//! `ks-train typo-evaluation synthetic|clean|field|all|vocabulary --tree <root> --messages <folder> --manifest <file> --out <folder> [--tune] [--utc-offset S]`
//! `ks-train prefix-corpus --tree <root> [--freeze <folder>]`, `ks-train archive --tree <root> [--write]`
//! `ks-train gates --tree <root>`, `ks-train licenses --tree <root> [--verify]`, `ks-train deb-check --tree <root> <package>`
//! `ks-train retired boundary-v1|context-v2|ortho-v2|prefix-v1 --tree <root> [--replay]`, `retired context-corpus --tree <root>`, `retired context-engine --tree <root> [--output <file>]`
//! `ks-train detector --tree <root> [--sample N] [--dictionary-sample N] [--strict]`
//! `ks-train fitting --tree <root> --base <folder> --output <folder> --namespace <text> --tatoeba-directory <folder> --tatoeba-language <code>... ...`
//!
//! Fits a candidate of the context action model or of the prefix model. Never reads a test split. With `--stages` every stage is written down, one
//! line per frame with floats as C99 hex, to be compared with another trainer's.
#![expect(clippy::panic, reason = "a tool stops on input it cannot use: nobody is typing through it")]
use ks_cast::To as _;
use ks_text::JsonOptions;
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
        let required = |name: &str| option(&arguments, name).map_or_else(|| panic!("{name} is required"), PathBuf::from);
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
    if matches!(arguments.first().map(String::as_str), Some("messages" | "terms" | "alnum" | "short" | "sources" | "select" | "devchat-messages")) {
        let root = PathBuf::from(option(&arguments, "--tree").expect("--tree is required"));
        let resources = ks_lang::Resources::with_speller(root.join("resources"), ks_hunspell::hunspell_speller(ks_config::history::data_dir()));
        match ks_train::messages::command(&arguments, &root, &resources) {
            Ok(result) => println!("{}", ks_text::json_dumps(&result, &JsonOptions { ensure_ascii: false, indent: Some(ks_text::Indent::Spaces(1)), ..JsonOptions::default() })),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    if matches!(arguments.first().map(String::as_str), Some("carry" | "doctor" | "supplement" | "apt-build" | "apt-verify")) {
        std::process::exit(small_command(&arguments));
    }
    if arguments.first().map(String::as_str) == Some("intent") {
        let root = PathBuf::from(option(&arguments, "--tree").expect("--tree is required"));
        if let Err(error) = ks_train::intent::command(&root, &arguments) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if arguments.first().map(String::as_str) == Some("typo-evaluation") {
        let root = PathBuf::from(option(&arguments, "--tree").expect("--tree is required"));
        let resources = ks_lang::Resources::new(root.join("resources"));
        let request = ks_train::typo_evaluation::Request { root: &root, measurement: arguments.get(1).expect("synthetic, clean, field or all"), messages: Path::new(option(&arguments, "--messages").expect("--messages is required")), manifest: Path::new(option(&arguments, "--manifest").unwrap_or(".t/field/manifest.json")), out: Path::new(option(&arguments, "--out").expect("--out is required")), tune: arguments.iter().any(|argument| argument == "--tune"), local_offset_seconds: option(&arguments, "--utc-offset").map_or(0, |value| value.parse().expect("seconds")), vocabulary: option(&arguments, "--clean").map(|clean| ks_train::typo_evaluation::VocabularyInputs { clean: PathBuf::from(clean), ru: PathBuf::from(option(&arguments, "--ru").expect("--ru is required")), en: PathBuf::from(option(&arguments, "--en").expect("--en is required")), train_messages: PathBuf::from(option(&arguments, "--train-messages").expect("--train-messages is required")), train_code: PathBuf::from(option(&arguments, "--train-code").expect("--train-code is required")) }) };
        if let Err(error) = ks_train::typo_evaluation::command(&request, &resources) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if arguments.first().map(String::as_str) == Some("deb-check") {
        let root = PathBuf::from(option(&arguments, "--tree").expect("--tree is required"));
        match ks_train::deb_check::verify(&root, Path::new(arguments.last().expect("the package"))) {
            Ok(line) => println!("{line}"),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    if arguments.first().map(String::as_str) == Some("licenses") {
        let root = PathBuf::from(option(&arguments, "--tree").expect("--tree is required"));
        if let Err(error) = ks_train::licenses::command(&root, arguments.iter().any(|argument| argument == "--verify")) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if arguments.first().map(String::as_str) == Some("gates") {
        let root = PathBuf::from(option(&arguments, "--tree").expect("--tree is required"));
        match ks_train::gates::run(&root) {
            Ok(report) => println!("{}", ks_text::json_dumps(&report, &JsonOptions { ensure_ascii: false, indent: Some(ks_text::Indent::Spaces(2)), ..JsonOptions::default() })),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    if arguments.first().map(String::as_str) == Some("retired") {
        std::process::exit(retired_command(&arguments));
    }
    if arguments.first().map(String::as_str) == Some("archive") {
        let root = PathBuf::from(option(&arguments, "--tree").expect("--tree is required"));
        if let Err(error) = ks_train::small_tools::archive(&root, arguments.iter().any(|argument| argument == "--write")) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if arguments.first().map(String::as_str) == Some("prefix-corpus") {
        let root = PathBuf::from(option(&arguments, "--tree").expect("--tree is required"));
        let resources = ks_lang::Resources::new(root.join("resources"));
        if let Err(error) = ks_train::prefix_corpus::command(&root, &resources, option(&arguments, "--freeze").map(Path::new)) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if arguments.first().map(String::as_str) == Some("base") {
        match base_command(&arguments) {
            Ok(summary) => println!("{summary}"),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    if arguments.first().map(String::as_str) == Some("detector") {
        let root = PathBuf::from(option(&arguments, "--tree").expect("--tree is required"));
        let resources = ks_lang::Resources::with_speller(root.join("resources"), ks_hunspell::hunspell_speller(ks_config::history::data_dir()));
        let size = |name: &str| option(&arguments, name).map_or(5000, |value| value.parse().expect("a number"));
        match ks_train::detector::command(&resources, size("--sample"), size("--dictionary-sample"), arguments.iter().any(|argument| argument == "--strict")) {
            Ok(code) => std::process::exit(code),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    }
    if arguments.first().map(String::as_str) == Some("ortho") {
        let root = PathBuf::from(option(&arguments, "--tree").expect("--tree is required"));
        if let Some(description) = option(&arguments, "--freeze-independent") {
            let directory = Path::new(option(&arguments, "--files").expect("--files is required"));
            let result = match option(&arguments, "--check-within") {
                Some(name) => ks_train::ortho_inputs::check_rows_within(&root, Path::new(description), directory, name, &root.join("model/ortho_v1/independent/tokens.jsonl.gz")).map(|(within, rows)| serde_json::json!({"within": within, "rows": rows})),
                None => ks_train::ortho_inputs::freeze_independent(&root, Path::new(description), directory),
            };
            match result {
                Ok(report) => println!("{report}"),
                Err(error) => {
                    eprintln!("{error}");
                    std::process::exit(1);
                }
            }
            return;
        }
        if arguments.iter().any(|argument| argument == "--lexicon-sources") {
            if let Err(error) = ks_train::ortho_inputs::verify_lexicon_sources(&root, option(&arguments, "--web-source").map(Path::new), option(&arguments, "--prose-source").map(Path::new)) {
                eprintln!("{error}");
                std::process::exit(1);
            }
            return;
        }
        if arguments.iter().any(|argument| argument == "--inputs" || argument == "--freeze-inputs") {
            let result = if arguments.iter().any(|argument| argument == "--inputs") { ks_train::ortho_inputs::verify(&root) } else { ks_train::ortho_inputs::freeze(&root, Path::new(option(&arguments, "--output").expect("--output is required"))) };
            if let Err(error) = result {
                eprintln!("{error}");
                std::process::exit(1);
            }
            return;
        }
        let Some(action) = ["fit", "verify", "test", "promote"].into_iter().find(|name| arguments.iter().any(|argument| *argument == format!("--{name}"))) else {
            eprintln!("--fit, --verify, --test, --promote, --inputs or --freeze-inputs");
            std::process::exit(2);
        };
        if let Err(error) = ks_train::ortho::command(&root, action, option(&arguments, "--output").map(Path::new)) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if arguments.first().map(String::as_str) == Some("typo-frequency") {
        let root = PathBuf::from(option(&arguments, "--tree").expect("--tree is required"));
        let private = option(&arguments, "--private").map_or_else(|| root.join(".t"), PathBuf::from);
        let paths = ks_train::typo_frequency::Paths::in_private(&private);
        match ks_train::typo_frequency::command(&root, &paths, arguments.iter().any(|argument| argument == "--verify")) {
            Ok(true) => return,
            Ok(false) => std::process::exit(1),
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
    let required = |name: &str| option(&arguments, name).map_or_else(|| panic!("{name} is required"), PathBuf::from);
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

/// `ks-train retired <family> --tree <root> [--replay]`.
fn retired_command(arguments: &[String]) -> i32 {
    use ks_train::retired;
    let root = PathBuf::from(option(arguments, "--tree").expect("--tree is required"));
    let replaying = arguments.iter().any(|argument| argument == "--replay");
    let result = match arguments.get(1).map(String::as_str) {
        Some("boundary-v1") if replaying => retired::boundary_v1::replay(&root),
        Some("boundary-v1") => retired::boundary_v1::verify(&root),
        Some("context-v2") if replaying => retired::context_v2::replay(&root),
        Some("context-v2") => retired::context_v2::verify(&root),
        Some("ortho-v2") if replaying => retired::ortho_v2::replay(&root),
        Some("ortho-v2") => retired::ortho_v2::verify(&root),
        Some("prefix-v1") if replaying => retired::prefix_v1::replay(&root),
        Some("prefix-v1") => retired::prefix_v1::verify(&root),
        Some("context-engine") => (|| {
            let resources = ks_lang::Resources::new(root.join("resources"));
            let scratch = std::env::temp_dir().join(format!("keyswitch-context-engine-{}", std::process::id()));
            let report = ks_train::engine_reports::context_v2_report(&root, &resources, &scratch);
            let _ = std::fs::remove_dir_all(&scratch);
            let report = report?;
            if let Some(path) = option(arguments, "--output") {
                std::fs::write(path, ks_train::train_json(&report)).map_err(|error| error.to_string())?;
            }
            Ok(serde_json::json!({"results": report["results"], "promotion_passed": report["promotion_passed"]}))
        })(),
        Some("context-corpus") => retired::context_v2::verify_corpus_receipt(&root),
        other => Err(format!("unknown retired family: {other:?}")),
    };
    match result {
        Ok(report) => {
            println!("{}", ks_text::json_dumps(&report, &JsonOptions { ensure_ascii: false, indent: Some(ks_text::Indent::Spaces(2)), ..JsonOptions::default() }));
            0
        }
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

fn all_of<'a>(arguments: &'a [String], name: &str) -> Vec<&'a str> {
    arguments.iter().enumerate().filter(|(_, argument)| *argument == name).filter_map(|(place, _)| arguments.get(place + 1)).map(String::as_str).collect()
}

/// The two corpus freezes share their arguments.
fn freeze_command(arguments: &[String]) -> i32 {
    use ks_train::freeze::{self, FreezeRequest, TechnicalSource};
    let required = |name: &str| option(arguments, name).map_or_else(|| panic!("{name} is required"), PathBuf::from);
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
    let required = |name: &str| option(arguments, name).map_or_else(|| panic!("{name} is required"), PathBuf::from);
    let root = required("--tree");
    let resource_dir = root.join("resources");
    let jobs: usize = option(arguments, "--jobs").map_or(4, |value| value.parse().expect("--jobs is a number"));
    let installed = resource_dir.join("models/context_policy_v1.json");
    let result = match arguments[0].as_str() {
        "capture" => ks_train::mixed::capture(&resource_dir, &required("--manifest"), &required("--messages"), &required("--model"), option(arguments, "--only"), arguments.iter().any(|argument| argument == "--verify"), jobs),
        "mixed" => {
            let number = |name: &str, default: f64| option(arguments, name).map_or(default, |value| value.parse().expect("a number"));
            let kinds: Vec<&str> = option(arguments, "--kinds").unwrap_or("comment").split(',').collect();
            let recipe = serde_json::json!({"seed": option(arguments, "--seed").unwrap_or("mixbench-v1"), "kinds": kinds, "count": number("--count", 0.0).to::<i64>(), "russian_share": number("--russian-share", 0.25),
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
    let required = |name: &str| option(arguments, name).map_or_else(|| panic!("{name} is required"), PathBuf::from);
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
                let format = JsonOptions { ensure_ascii: false, sort_keys: true, indent: Some(ks_text::Indent::Spaces(1)), ..JsonOptions::default() };
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
            let field_root = option(arguments, "--field-root").map_or_else(|| root.clone(), PathBuf::from);
            ks_train::promote::promote(&ks_train::promote::PromoteRequest {
                root: &root,
                candidate: &candidate,
                prefix_candidate: &prefix_candidate,
                corpus: &corpus,
                report: &report,
                released_rows: &released,
                field_root: &field_root,
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

/// The short tools of a cycle and a release.
fn small_command(arguments: &[String]) -> i32 {
    use ks_train::small_tools as tools;
    let path = |name: &str| option(arguments, name).map(PathBuf::from);
    let required = |name: &str| path(name).unwrap_or_else(|| panic!("{name} is required"));
    let root = required("--tree");
    let private = root.join(".t/reliable-release-2026-09-12");
    let printed = |text: String| {
        println!("{text}");
        Ok(0)
    };
    let pretty = JsonOptions { ensure_ascii: false, indent: Some(ks_text::Indent::Spaces(1)), ..JsonOptions::default() };
    let result: Result<i32, String> = match arguments[0].as_str() {
        "carry" => {
            let refit: Vec<String> = all_of(arguments, "--refit").into_iter().map(str::to_owned).collect();
            tools::carry(&root, &required("--pair"), &refit, &required("--output")).and_then(|value| printed(ks_text::json_dumps(&value, &pretty)))
        }
        "doctor" => {
            let (corpus, ledger) = (path("--corpus").unwrap_or_else(|| private.join("context-action-corpus")), path("--ledger").unwrap_or_else(|| private.join("context-action-test-ledger")));
            let stages: Vec<String> = all_of(arguments, "--stage").into_iter().map(str::to_owned).collect();
            Ok(tools::doctor(&root, &corpus, &ledger, &stages))
        }
        "supplement" => {
            if arguments.iter().any(|argument| argument == "--verify") {
                tools::verify_supplement(&root).and_then(|value| printed(ks_text::json_dumps(&value, &pretty)))
            } else {
                let (output, receipt) = (path("--output").unwrap_or_else(|| root.join("resources/lexicon-supplement-ru_RU.json")), path("--receipt").unwrap_or_else(|| root.join("resources/lexicon-supplement-ru_RU.receipt.json")));
                let removed = path("--removed-list");
                tools::filter_supplement(&root, &required("--input"), &output, &receipt, removed.as_deref()).and_then(|value| printed(ks_text::json_dumps(&value, &pretty)))
            }
        }
        "apt-build" => {
            let packages: Vec<PathBuf> = all_of(arguments, "--package").into_iter().map(PathBuf::from).collect();
            let passphrase = path("--passphrase-file");
            tools::apt_build(&root, &required("--repository"), &packages, passphrase.as_deref()).and_then(printed)
        }
        _ => {
            let key = path("--public-key");
            tools::apt_verify(&root, &required("--repository"), option(arguments, "--expect-version").expect("--expect-version is required"), key.as_deref()).and_then(printed)
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

/// `ks-train base freeze|reconcile|technical|merge|inventory`: the base of a context corpus, from pinned public sources.
fn base_command(arguments: &[String]) -> Result<serde_json::Value, String> {
    use ks_train::base_corpus as base;
    let root = PathBuf::from(option(arguments, "--tree").ok_or("--tree is required")?);
    let required = |name: &str| option(arguments, name).map(PathBuf::from).ok_or_else(|| format!("{name} is required"));
    let exposures: Vec<PathBuf> = arguments.windows(2).filter(|pair| pair[0] == "--extra-exposure").map(|pair| root.join(&pair[1])).collect();
    let size = |name: &str, default: usize| option(arguments, name).map_or(Ok(default), |value| value.parse().map_err(|_| format!("{name} takes a number")));
    match arguments.get(1).map(String::as_str) {
        Some("freeze") => {
            let namespace = option(arguments, "--namespace").ok_or("--namespace is required")?;
            let manifest = base::freeze_base(&root, &required("--source-root")?, &required("--output")?, namespace, &exposures, size("--max-documents", base::DEFAULT_MAX_DOCUMENTS_PER_SOURCE)?, size("--max-sentences-per-document", base::DEFAULT_MAX_SENTENCES_PER_DOCUMENT)?)?;
            Ok(serde_json::json!({"namespace": manifest["namespace"], "sampling": manifest["sampling"], "partitioning": manifest["partitioning"], "splits": manifest["splits"], "test_membership_sha256": manifest["test_membership_sha256"]}))
        }
        Some("reconcile") => base::reconcile(&root, &required("--corpus")?, &required("--source-root")?, &required("--report")?, option(arguments, "--output").map(PathBuf::from).as_deref()),
        Some("technical") => base::freeze_technical(&root, &required("--source-directory")?, &required("--ud-corpus")?, &exposures, option(arguments, "--output").map(PathBuf::from).as_deref(), &required("--preview-report")?),
        Some("merge") => {
            let manifest = base::merge(&root, &required("--base")?, &required("--technical")?, &required("--output")?)?;
            Ok(serde_json::json!({"test_membership_sha256": manifest["test_membership_sha256"], "partitioning": manifest["partitioning"], "alias_audit": manifest["alias_audit"]}))
        }
        Some("inventory") => base::prefix_inventory(&root, &option(arguments, "--corpus").map_or_else(|| root.join("model/prefix_v1"), PathBuf::from), &required("--output")?),
        _ => Err("base freeze|reconcile|technical|merge|inventory".to_owned()),
    }
}
