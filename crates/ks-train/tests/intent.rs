//! The layout-intent model fitted on a small lexicon: the corpora frozen and presealed, the fit, the one-shot ledgers of its test, the strict evaluation.
mod support;

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::LazyLock;
use support::{digest, recipe, scratch, small_tree, trainer};

struct Small {
    tree: PathBuf,
    folder: PathBuf,
}

fn read(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).expect("a file")).expect("json")
}

fn write_json(path: &Path, value: &Value) {
    std::fs::write(path, serde_json::to_vec(value).expect("json")).expect("a file");
}

/// Every so many words of a language model, with what stands before them.
fn thinned(text: &str, keep_every: usize, always: &[&str]) -> String {
    let mut output = String::new();
    let mut section = false;
    let mut index = 0;
    for line in text.lines() {
        if line.starts_with(r"\2-grams:") {
            break;
        }
        if section && !line.starts_with('\\') {
            index += 1;
            if index % keep_every != 0 && !line.contains('<') {
                continue;
            }
        }
        section = section || line == r"\1-grams:";
        output.push_str(line);
        output.push('\n');
    }
    output.pop();
    for word in always {
        output.push_str(&format!("\n1000 {word}"));
    }
    output.push_str("\n\\end\\\n");
    output
}

fn intent(tree: &Path) -> Command {
    let mut command = trainer();
    command.arg("intent").arg("--tree").arg(tree);
    command
}

fn run(command: &mut Command) -> Output {
    command.output().expect("the trainer starts")
}

fn said(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

/// Pin a file in the config by what it is.
fn pin(config: &mut Value, at: &str, path: &str, bytes: &[u8]) {
    let entry = json!({"path": path, "sha256": digest(bytes), "bytes": bytes.len()});
    let pointer = config.pointer_mut(at).expect("a place in the config");
    if let Some(table) = pointer.as_object_mut() {
        for (name, value) in entry.as_object().expect("an object") {
            table.insert(name.clone(), value.clone());
        }
    }
}

/// Gates a model of two thousand words can pass: what is tested here is what happens around them.
fn relaxed(config: &mut Value) {
    let thresholds = &mut config["thresholds"];
    thresholds["precision_floor"] = json!(0.5);
    thresholds["maximum_false_positive_rate"] = json!(0.5);
    thresholds["pause_maximum_false_positive_rate"] = json!(0.5);
    thresholds["selection_maximum_false_positives_per_trigger"] = json!(100_000);
    thresholds["veto_max_false_negative_rate"] = json!(0.5);
    let gates = config["quality_gates"].as_object_mut().expect("the gates");
    for (name, value) in gates.iter_mut() {
        if name != "safety_maximum_guard_failures" {
            *value = json!(0.0);
        }
    }
}

fn lexical_corpus(tree: &Path, config: &Value) -> String {
    use ks_train::intent::{external, pipeline, runtime_models};
    let shape = pipeline::dataset_config(config).expect("a dataset");
    let words = pipeline::onboard_lexicon(tree, config, &shape).expect("a lexicon");
    let onboard = external::onboard_words(&words, shape.minimum_word_length);
    let (_, models) = runtime_models(tree).expect("the dictionaries");
    let snapshots = [external::snapshot_of_model(&models[0], "en_US").expect("a snapshot"), external::snapshot_of_model(&models[1], "ru_RU").expect("a snapshot")];
    let minimum = usize::try_from(config["external_evaluation"]["minimum_words_per_group"].as_u64().expect("a size")).expect("small");
    external::build_lexical_disjoint_corpus(&onboard, minimum, &snapshots).expect("a corpus").corpus_sha256
}

/// A tree with a small lexicon, its corpora frozen and presealed, and the config's pins following them.
fn prepared(name: &str, change: &dyn Fn(&mut Value)) -> Small {
    let folder = scratch(name);
    let tree = small_tree(&format!("{name}-tree"), &recipe());
    let config_path = tree.join("model/intent_v1/config.json");
    let mut config = read(&config_path);
    // With the words that read as words in both layouts, which are few and short.
    for (locale, every, both) in [("en_US", 25, ["herb", "here", "keys", "verb", "eat", "her"]), ("ru_RU", 250, ["руки", "руку", "луны", "муки", "уфе", "рук"])] {
        let text = std::fs::read_to_string(tree.join(format!("model/intent_v1/sources/{locale}.lm"))).expect("a language model");
        let small = thinned(&text, every, &both);
        let relative = format!("model/intent_v1/{locale}-small.lm");
        std::fs::write(tree.join(&relative), &small).expect("a file");
        pin(&mut config, &format!("/sources/languages/{locale}"), &relative, small.as_bytes());
    }
    config["external_evaluation"]["minimum_words_per_group"] = json!(40);
    config["hard_negative_development"]["words_per_group"] = json!({"train": 22, "development": 6, "calibration": 6, "threshold": 6});
    config["ftrl"]["maximum_epochs"] = json!(3);
    config["ftrl"]["minimum_epochs"] = json!(1);
    config["ftrl"]["patience"] = json!(1);
    config["sealed_evaluation"]["registry_path"] = json!("model/intent_v1/seal-registry-v99.json");
    relaxed(&mut config);
    change(&mut config);
    write_json(&config_path, &config);
    // The corpora, frozen, and the receipt of what was frozen.
    let relative = "model/intent_v1/unknown-typo-development-small.json";
    let frozen = run(intent(&tree).arg("--freeze").arg("--output").arg(tree.join(relative)));
    assert!(frozen.status.success(), "{}", said(&frozen));
    let bytes = std::fs::read(tree.join(relative)).expect("the corpus");
    pin(&mut config, "/hard_negative_development/source", relative, &bytes);
    config["external_evaluation"]["unknown_typo_development_corpus_sha256"] = read(&tree.join(relative))["expanded_corpus_sha256"].clone();
    write_json(&config_path, &config);
    let receipt_path = tree.join("model/intent_v1/holdout-v99-preseal.json");
    let preseal = run(intent(&tree).arg("--preseal").arg("--output").arg(&receipt_path));
    assert!(preseal.status.success(), "{}", said(&preseal));
    let receipt = read(&receipt_path);
    config["external_evaluation"]["unknown_typo_holdout_corpus_sha256"] = receipt["holdout"]["corpus_sha256"].clone();
    config["external_evaluation"]["lexical_disjoint_corpus_sha256"] = json!(lexical_corpus(&tree, &config));
    write_json(&config_path, &config);
    Small { tree, folder }
}

/// The model of the small lexicon, fitted and published in its tree.
static PUBLISHED: LazyLock<Small> = LazyLock::new(|| {
    let small = prepared("intent", &|_| ());
    let fitted = run(intent(&small.tree).arg("--fit").args(["--jobs", "2"]));
    assert!(fitted.status.success(), "{}", said(&fitted));
    small
});

fn config_of(small: &Small) -> PathBuf {
    small.tree.join("model/intent_v1/config.json")
}

/// A model of the shipped tree's format that its checks take: a fit that passed writes each file of a publication.
#[test]
fn a_model_that_passed_is_published_with_its_manifest_and_report() {
    let small = &*PUBLISHED;
    let manifest = read(&small.tree.join("model/intent_v1/manifest.json"));
    assert_eq!(manifest["quality_gates_passed"], json!(true));
    assert!(manifest["artifact_model_version"].as_str().expect("a version").starts_with("intent-v1-"));
    assert!(read(&small.tree.join("model/intent_v1/build-environment.json"))["environment_probe"].is_object());
    assert!(read(&small.tree.join("model/intent_v1/test-report.json")).get("sealed_test").is_some());
    let model = small.tree.join("resources/models/layout_intent_v1.ksm");
    assert!(ks_lang::intent_model::LinearNgramModel::load(&model).is_ok());
    // The same run again is the same claim on the ledgers.
    let again = run(intent(&small.tree).arg("--fit").args(["--jobs", "2"]).arg("--dry-run"));
    assert!(again.status.success(), "{}", said(&again));
    // And the replay finds what was recorded, to the byte.
    let replay = run(intent(&small.tree).arg("--verify").args(["--jobs", "2"]));
    assert!(replay.status.success() && said(&replay).contains("reproduces"), "{}", said(&replay));
    // Other places to write to.
    let elsewhere = small.folder.join("elsewhere");
    let moved = run(intent(&small.tree).arg("--fit").arg("--artifact").arg(elsewhere.join("a")).arg("--manifest").arg(elsewhere.join("m")).arg("--report").arg(elsewhere.join("r")).arg("--environment").arg(elsewhere.join("e")));
    assert!(moved.status.success(), "{}", said(&moved));
    assert!(elsewhere.join("a").is_file());
}

#[test]
fn the_strict_evaluation_reads_what_was_published() {
    let small = &*PUBLISHED;
    let report_path = small.folder.join("strict.json");
    let strict = |extra: &[&str]| run(intent(&small.tree).arg("--evaluate").args(["--comparison-sample", "60", "--latency-sample", "10", "--jobs", "2"]).args(extra).arg("--output").arg(&report_path));
    let provenance = strict(&["--provenance-only"]);
    println!("{}", said(&provenance));
    let report = read(&report_path);
    assert!(report.get("provenance").is_some(), "{report}");
    let whole = strict(&["--strict"]);
    println!("{}", said(&whole));
    let report = read(&report_path);
    assert!(report.get("strict_gates").is_some(), "{report}");
    // A receipt for what it rejects, and none for what it does not.
    let receipt = small.folder.join("rejection.json");
    let reject = |report: &Path, output: &Path| run(intent(&small.tree).arg("--reject").args(["--version", "28"]).arg("--strict-report").arg(report).arg("--output").arg(output));
    let answer = reject(&report_path, &receipt);
    println!("{}", said(&answer));
    let passed = small.folder.join("passed.json");
    write_json(&passed, &json!({"strict_gates": {"a": true}, "strict_passed": true}));
    assert!(said(&reject(&passed, &small.folder.join("r1.json"))).contains("nothing to reject"));
    write_json(&passed, &json!({"strict_gates": {"a": false}, "strict_passed": true}));
    assert!(said(&reject(&passed, &small.folder.join("r2.json"))).contains("does not record"));
    write_json(&passed, &json!({"strict_gates": 1}));
    assert!(said(&reject(&passed, &small.folder.join("r3.json"))).contains("lacks strict_gates"));
    std::fs::write(small.folder.join("there.json"), b"").expect("a file");
    assert!(said(&reject(&passed, &small.folder.join("there.json"))).contains("refusing to overwrite"));
    assert!(said(&run(intent(&small.tree).arg("--reject"))).contains("needs --version"));
    assert!(said(&run(intent(&small.tree).arg("--reject").args(["--version", "many"]))).contains("takes a number"));
    assert!(said(&run(intent(&small.tree).arg("--reject").args(["--version", "28"]))).contains("needs --strict-report"));
    // The size of a sample is a number.
    assert!(said(&run(intent(&small.tree).arg("--evaluate").args(["--comparison-sample", "many"]))).contains("takes a number"));
    // Printed, rather than written.
    let printed = run(intent(&small.tree).arg("--evaluate").args(["--provenance-only", "--comparison-sample", "60", "--latency-sample", "10"]));
    assert!(said(&printed).contains("provenance"));
}

#[test]
fn a_gate_that_fails_publishes_nothing() {
    // Before the test: the safety gate cannot pass, and what is known goes to a diagnostic.
    let small = prepared("intent-presealed", &|config| config["quality_gates"]["safety_maximum_guard_failures"] = json!(-1));
    let diagnostic = small.folder.join("diagnostic.json");
    let answer = run(intent(&small.tree).arg("--fit").args(["--jobs", "2"]).arg("--diagnostic").arg(&diagnostic));
    assert!(!answer.status.success() && said(&answer).contains("quality gates did not pass"), "{}", said(&answer));
    let written = read(&diagnostic);
    assert_eq!(written["phase"], json!("presealed_candidate"));
    assert!(!small.tree.join("model/intent_v1/seal-registry-v99.json").exists(), "the test stays unread");
    let quiet = run(intent(&small.tree).arg("--fit").args(["--jobs", "2"]));
    assert!(!quiet.status.success());
    let replay = run(intent(&small.tree).arg("--verify").args(["--jobs", "2"]));
    assert!(said(&replay).contains("pre-seal gate"), "{}", said(&replay));
    // After the test: it was read, its answer is claimed, and nothing is written.
    let small = prepared("intent-sealed", &|config| {
        for gate in ["minimum_precision", "minimum_recall", "minimum_pause_recall", "minimum_typo_recall", "minimum_pause_typo_recall", "minimum_specificity"] {
            config["quality_gates"][gate] = json!(1.0);
        }
    });
    let before = std::fs::read(small.tree.join("model/intent_v1/manifest.json")).expect("the manifest");
    let answer = run(intent(&small.tree).arg("--fit").args(["--jobs", "2"]));
    assert!(said(&answer).contains("quality gates"), "{}", said(&answer));
    assert!(small.tree.join("model/intent_v1/seal-registry-v99.json").exists());
    assert_eq!(std::fs::read(small.tree.join("model/intent_v1/manifest.json")).expect("the manifest"), before);
}

#[test]
fn what_a_fit_reads_is_checked_before_it_starts() {
    let small = prepared("intent-inputs", &|_| ());
    let path = config_of(&small);
    let good = read(&path);
    let refused = |edit: &dyn Fn(&mut Value), words: &str| {
        let mut changed = good.clone();
        edit(&mut changed);
        write_json(&path, &changed);
        let answer = run(intent(&small.tree).arg("--fit").args(["--jobs", "1"]));
        assert!(!answer.status.success() && said(&answer).contains(words), "{words}: {}", said(&answer));
    };
    refused(&|config| config["sealed_evaluation"]["split_namespace"] = json!("other"), "different split namespaces");
    refused(&|config| config["sealed_evaluation"]["registry_path"] = json!(1), "a registry path");
    refused(&|config| config["sealed_evaluation"]["split_namespace"] = json!(1), "a namespace");
    refused(&|config| config["sealed_evaluation"]["registry_path"] = json!("model/intent_v1/other.json"), "unexpected registry path");
    refused(&|config| config["sources"]["languages"]["en_US"]["sha256"] = json!("0"), "");
    refused(&|config| config["sources"]["languages"]["ru_RU"]["sha256"] = json!("0"), "");
    refused(&|config| config["sources"]["license_evidence"]["sha256"] = json!("0"), "");
    refused(&|config| config["sources"]["license_declaration"] = json!("Another"), "missing required lines");
    write_json(&path, &good);
    // Where it writes.
    let answer = run(intent(&small.tree).arg("--fit").arg("--artifact").arg(small.folder.join("x")).arg("--manifest").arg(small.folder.join("x")));
    assert!(said(&answer).contains("must be distinct"), "{}", said(&answer));
    let answer = run(intent(&small.tree).arg("--fit").arg("--artifact").arg(&path));
    assert!(said(&answer).contains("aliases an immutable"), "{}", said(&answer));
    assert!(said(&run(intent(&small.tree).arg("--fit").args(["--jobs", "many"]))).contains("takes a number"));
    // The commands.
    assert!(said(&run(&mut intent(&small.tree))).contains("--dataset, --verify"));
    assert!(!run(intent(Path::new("/nonexistent")).arg("--probe")).status.success());
    let probe = run(intent(&small.tree).arg("--probe"));
    assert!(said(&probe).contains("probe_sha256"), "{}", said(&probe));
    let stages = run(intent(&small.tree).arg("--dataset"));
    assert!(said(&stages).contains("sealed"), "{}", said(&stages));
    assert!(said(&run(intent(&small.tree).arg("--freeze"))).contains("needs --output"));
    let printed = run(intent(&small.tree).arg("--preseal"));
    assert!(said(&printed).contains("keyswitch-intent-"), "{}", said(&printed));
    // A frozen corpus that is not what the sources give.
    let mut other = good.clone();
    other["external_evaluation"]["minimum_words_per_group"] = json!(41);
    write_json(&path, &other);
    assert!(!run(intent(&small.tree).arg("--preseal")).status.success());
    write_json(&path, &good);
    assert!(ks_train::intent::frozen_dictionaries(Path::new("/nonexistent")).is_err());
}

#[test]
fn a_sealed_namespace_answers_one_candidate_once() {
    use ks_train::intent::seal::{claim_evaluation, claim_outcome, outcome_path, preseal_receipt_path, publish_bundle};
    let root = scratch("intent-ledgers");
    let registry = "seal-registry-v1.json";
    let receipt = claim_evaluation(&root, registry, "space", "candidate", "config", "dataset").expect("a claim");
    receipt.verify().expect("it holds");
    assert_eq!(receipt.payload()["candidate_sha256"], json!("candidate"));
    // The same again is allowed; each other way of being another candidate says which.
    claim_evaluation(&root, registry, "space", "candidate", "config", "dataset").expect("the same claim");
    assert!(claim_evaluation(&root, registry, "another", "candidate", "config", "dataset").map(|_| ()).unwrap_err().contains("wrong registry file"));
    assert!(claim_evaluation(&root, registry, "space", "candidate", "other", "dataset").map(|_| ()).unwrap_err().contains("another candidate"));
    assert!(claim_evaluation(&root, registry, "space", "other", "config", "dataset").map(|_| ()).unwrap_err().contains("different candidate hash"));
    // Moved after the claim.
    std::fs::write(root.join(registry), b"other").expect("a file");
    assert!(receipt.verify().unwrap_err().contains("changed after claim"));
    assert!(claim_evaluation(&root, registry, "space", "candidate", "config", "dataset").map(|_| ()).unwrap_err().contains("unreadable"));
    std::fs::remove_file(root.join(registry)).expect("the file goes");
    assert!(receipt.verify().unwrap_err().contains("unavailable"));
    std::os::unix::fs::symlink(root.join("nowhere"), root.join(registry)).expect("a link");
    assert!(receipt.verify().unwrap_err().contains("symlink"));
    std::fs::remove_file(root.join(registry)).expect("the link goes");
    std::fs::write(root.join(registry), vec![b'x'; 20_000]).expect("a file");
    assert!(receipt.verify().unwrap_err().contains("size limit"));
    // The outcome.
    claim_outcome(&root, registry, "space", "answer", "candidate").expect("a claim");
    claim_outcome(&root, registry, "space", "answer", "candidate").expect("the same answer");
    assert!(claim_outcome(&root, registry, "space", "answer", "other").unwrap_err().contains("different candidate"));
    assert!(claim_outcome(&root, registry, "space", "another", "candidate").unwrap_err().contains("different answer"));
    std::fs::write(root.join("seal-outcome-v1.json"), b"other").expect("a file");
    assert!(claim_outcome(&root, registry, "space", "answer", "candidate").unwrap_err().contains("unreadable"));
    assert!(claim_outcome(&root, "other.json", "space", "answer", "candidate").unwrap_err().contains("unexpected registry"));
    assert_eq!(outcome_path("a/seal-registry-v9.json").expect("a path"), "a/seal-outcome-v9.json");
    assert!(preseal_receipt_path("other.json").is_err());
    // A bundle goes in whole, or the old bytes come back.
    let (first, second) = (root.join("deep/first"), root.join("second"));
    publish_bundle(&[(first.clone(), b"1".to_vec()), (second.clone(), b"2".to_vec())]).expect("published");
    assert_eq!(std::fs::read(&first).expect("a file"), b"1");
    publish_bundle(&[(first.clone(), b"3".to_vec())]).expect("published again");
    let folder_in_the_way = root.join("a-folder");
    std::fs::create_dir_all(folder_in_the_way.join("inside")).expect("a folder");
    let new = root.join("new");
    let said = publish_bundle(&[(first.clone(), b"4".to_vec()), (new.clone(), b"5".to_vec()), (folder_in_the_way, b"6".to_vec())]).unwrap_err();
    assert!(!said.is_empty());
    assert_eq!(std::fs::read(&first).expect("a file"), b"3", "the old bytes are back");
    assert!(!new.exists(), "what was new is gone");
    let in_a_file = second.join("x");
    assert!(publish_bundle(&[(first.clone(), b"7".to_vec()), (in_a_file, b"8".to_vec())]).is_err());
    assert_eq!(std::fs::read(&first).expect("a file"), b"3");
    let huge = root.join("huge");
    let file = std::fs::File::create(&huge).expect("a file");
    file.set_len(70 << 20).expect("a sparse file");
    assert!(publish_bundle(&[(first, b"9".to_vec()), (huge, b"10".to_vec())]).unwrap_err().contains("rollback limit"));
}
