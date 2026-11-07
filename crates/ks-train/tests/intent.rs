//! The layout-intent model fitted on a small lexicon: the corpora frozen and presealed, the fit, the one-shot ledgers of its test, the strict evaluation.
mod support;

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::LazyLock;
use support::{digest, recipe, scratch, small_tree, trainer, tree};

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

fn paths(value: &Value, here: &str, found: &mut Vec<String>) {
    found.push(here.to_owned());
    match value {
        Value::Object(table) => {
            for (name, item) in table {
                paths(item, &format!("{here}/{name}"), found);
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                paths(item, &format!("{here}/{index}"), found);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

/// Every place of a value changed, one at a time.
fn each_change(value: &Value, mut seen: impl FnMut(&Value)) {
    let mut found = Vec::new();
    paths(value, "", &mut found);
    for path in found {
        let original = value.pointer(&path).expect("a path").clone();
        let changes = match &original {
            Value::Bool(flag) => vec![json!(!flag)],
            Value::Number(number) => vec![json!(number.as_f64().unwrap_or(0.0) + 1.0), json!(-1), json!("x")],
            Value::String(text) if !text.is_empty() => vec![json!("x"), json!(""), json!(1), json!(format!("{text}x"))],
            Value::String(_) | Value::Null | Value::Array(_) | Value::Object(_) => vec![json!(null), json!({"x": 1})],
        };
        for change in changes {
            let mut copy = value.clone();
            *copy.pointer_mut(&path).expect("a path") = change;
            seen(&copy);
        }
    }
}

#[test]
fn a_frozen_corpus_is_decoded_only_as_it_was_frozen() {
    use ks_lang::detector::ProtectedTokens;
    use ks_train::intent::{dataset, pipeline};
    let small = prepared("intent-frozen", &|_| ());
    let config = read(&config_of(&small));
    let shape = pipeline::dataset_config(&config).expect("a dataset");
    let protected = ProtectedTokens::load(&small.tree.join("resources"));
    let raw = dataset::read_frozen(&small.tree, &config["hard_negative_development"]["source"]).expect("the corpus");
    let good = dataset::decode_hard_negative_corpus(&raw, &config, &shape, &protected).expect("decoded");
    assert!(good.provenance().is_object());
    // Each thing the file says, changed.
    let frozen: Value = serde_json::from_slice(&raw).expect("json");
    let mut whole = frozen.clone();
    let rows = whole["rows"].as_array_mut().expect("rows");
    rows.truncate(3);
    let row_count = frozen["rows"].as_array().expect("rows").len();
    each_change(&whole, |changed| {
        let mut copy = changed.clone();
        if let Some(mine) = copy["rows"].as_array() {
            let mut rows = mine.clone();
            rows.extend(frozen["rows"].as_array().expect("rows")[3..row_count].iter().cloned());
            copy["rows"] = json!(rows);
        }
        let bytes = serde_json::to_vec(&copy).expect("json");
        assert!(dataset::decode_hard_negative_corpus(&bytes, &config, &shape, &protected).is_err());
    });
    assert!(dataset::decode_hard_negative_corpus(b"not json", &config, &shape, &protected).is_err());
    // The same corpus against a config that expects something else of it.
    let mut other = config.clone();
    other["hard_negative_development"]["words_per_group"] = json!({"train": 1000, "development": 6, "calibration": 6, "threshold": 6});
    assert!(dataset::decode_hard_negative_corpus(&raw, &other, &shape, &protected).is_err());
    let mut tight = pipeline::dataset_config(&config).expect("a dataset");
    tight.maximum_word_length = 2;
    assert!(dataset::decode_hard_negative_corpus(&raw, &config, &tight, &protected).is_err());
    // A row that is a protected token.
    let mut token = frozen.clone();
    let protected_word = "iphone";
    let _ = protected_word;
    token["rows"][0]["correct_typo"] = json!("github");
    assert!(dataset::decode_hard_negative_corpus(&serde_json::to_vec(&token).expect("json"), &config, &shape, &protected).is_err());
    let mut twice = frozen;
    let first = twice["rows"][0].clone();
    twice["rows"][1] = first;
    assert!(dataset::decode_hard_negative_corpus(&serde_json::to_vec(&twice).expect("json"), &config, &shape, &protected).is_err());
    // A frozen file that is not the one pinned.
    let mut spec = config["hard_negative_development"]["source"].clone();
    spec["bytes"] = json!(1);
    assert!(dataset::read_frozen(&small.tree, &spec).is_err());
    spec["sha256"] = json!("0");
    assert!(dataset::read_frozen(&small.tree, &spec).is_err());
    spec["path"] = json!("gone");
    assert!(dataset::read_frozen(&small.tree, &spec).is_err());
}

#[test]
fn a_dataset_that_leaks_across_its_splits_is_refused() {
    use ks_train::intent::dataset::{self, Split, assert_no_split_leakage};
    use ks_train::intent::pipeline;
    let small = prepared("intent-leaks", &|_| ());
    let presealed = pipeline::presealed_dataset(&small.tree).expect("a dataset");
    let good = presealed.dataset.clone();
    assert_no_split_leakage(&good, &Split::PRESEALED).expect("it holds");
    let (train, development) = (Split::Train.index(), Split::Development.index());
    let refuses = |edit: &dyn Fn(&mut dataset::DatasetBundle), words: &str| {
        let mut changed = good.clone();
        edit(&mut changed);
        let said = assert_no_split_leakage(&changed, &Split::PRESEALED).expect_err(words);
        assert!(said.contains(words), "{words}: {said}");
    };
    refuses(&|bundle| { let row = bundle.by_split[train][0].clone(); bundle.by_split[development].push(row); }, "leaked from");
    refuses(&|bundle| { let mut row = bundle.by_split[train][0].clone(); row.base_signature = "another".to_owned(); bundle.by_split[development].push(row); }, "leaked between splits");
    refuses(&|bundle| { let mut row = bundle.by_split[train][0].clone(); row.base_signature = "another".to_owned(); row.label = !row.label; bundle.by_split[train].push(row); }, "contradictory intended languages");
    refuses(&|bundle| { bundle.by_split[train][0].original = "смесьmix".to_owned(); bundle.by_split[train][0].alternative = "other".to_owned(); }, "inconsistent physical signatures");
    refuses(&|bundle| { bundle.safety[0].original = "смесьmix".to_owned(); bundle.safety[0].alternative = "other".to_owned(); }, "inconsistent physical signatures");
    refuses(&|bundle| { let mut row = bundle.safety[0].clone(); row.safety = false; bundle.by_split[train].push(row); }, "safety corpus");
    refuses(&|bundle| { let mut row = bundle.by_split[train][0].clone(); row.base_signature.clone_from(&bundle.safety[0].base_signature); bundle.by_split[train][0] = row; }, "safety corpus");
    refuses(&|bundle| bundle.variant_quarantine.sha256 = "0".to_owned(), "does not match its records");
    let with_occurrence = !good.variant_quarantine.occurrences.is_empty();
    if with_occurrence {
        refuses(&|bundle| { let first = bundle.variant_quarantine.occurrences[0].clone(); bundle.variant_quarantine.occurrences.push(first); bundle.variant_quarantine.sha256 = dataset::quarantine_fingerprint(&bundle.variant_quarantine.occurrences); }, "duplicate occurrence");
        refuses(&|bundle| { bundle.variant_quarantine.occurrences[0].split = "test"; bundle.variant_quarantine.sha256 = dataset::quarantine_fingerprint(&bundle.variant_quarantine.occurrences); }, "outside its phase");
        refuses(&|bundle| { bundle.variant_quarantine.occurrences[0].reason = "other"; bundle.variant_quarantine.sha256 = dataset::quarantine_fingerprint(&bundle.variant_quarantine.occurrences); }, "evidence is invalid");
    }
    // The test split in, with what was excluded from it.
    let sealed = dataset::build_dataset(&presealed.prepared, &presealed.dataset_config, &[Split::Test]);
    let whole = dataset::merge_sealed_test_dataset(good.clone(), sealed.clone()).expect("merged");
    assert_no_split_leakage(&whole, &Split::PRESEALED).expect("it holds");
    let mut emitted = whole.clone();
    let row = &emitted.by_split[Split::Test.index()][0];
    let signature = dataset::row_physical_signature(row).expect("a signature");
    emitted.sealed_test_exclusion_signatures.push(signature);
    assert!(assert_no_split_leakage(&emitted, &Split::PRESEALED).unwrap_err().contains("excluded sealed-test signature"));
    // A sealed row that is in the candidate already goes; a quarantine record twice stops the merge.
    let mut repeated = sealed.clone();
    let row = good.by_split[train][0].clone();
    repeated.by_split[Split::Test.index()].push(row);
    let safety_row = good.safety[0].clone();
    repeated.by_split[Split::Test.index()].push(safety_row);
    let merged = dataset::merge_sealed_test_dataset(good.clone(), repeated).expect("merged");
    assert!(merged.sealed_test_exclusion_signatures.len() >= whole.sealed_test_exclusion_signatures.len());
    if with_occurrence {
        let mut doubled = sealed;
        doubled.variant_quarantine.occurrences = good.variant_quarantine.occurrences.clone();
        doubled.sealed_variant_quarantine.occurrences = good.variant_quarantine.occurrences.clone();
        assert!(dataset::merge_sealed_test_dataset(good, doubled).is_err());
    }
    // A word list, each line of its kinds, and the typos of a word.
    let raw = "\\data\\\nngram 1=2\n\n\\1-grams:\n\n1 <s>\nnonumber\nmany слово\n5 hello\n7 HeLLo\n3 it's\n3 a\n\\2-grams:\n1 a b\n";
    let words = dataset::load_onboard_unigrams(raw.as_bytes(), 0, 3, 16);
    assert!(words.iter().any(|word| word.word == "hello" && word.frequency == 12), "{}", words.len());
    assert_eq!(dataset::typo_variants("abcdefgh", 0).len(), 1);
    assert_eq!(dataset::typo_variants("ab", 3).len(), 1);
    assert_eq!(dataset::typo_variants("abcdefgh", 2).len(), 3);
    assert_eq!(dataset::normalise_lexical_word("Ёж'ик", 1), "ёж'ик");
    assert!(dataset::physical_signature("", 0).is_empty());
    assert!(dataset::physical_signature("слово", 0).is_empty());
    let prepared_words = dataset::prepare_lexicon(vec![], 5);
    assert!(prepared_words.collisions.is_empty());
}

#[test]
fn settings_of_a_fit_each_change_what_it_can_accept() {
    use ks_train::intent::pipeline::fit_candidate;
    let small = prepared("intent-settings", &|_| ());
    let path = config_of(&small);
    let good = read(&path);
    let variants: [&dyn Fn(&mut Value); 21] = [
        &|config| config["thresholds"]["threshold_logit_margin_cap"] = json!(0.0),
        &|config| config["thresholds"]["threshold_logit_margin_cap"] = json!(0.001),
        &|config| config["thresholds"]["threshold_logit_margin_cap"] = json!(60.0),
        &|config| config["thresholds"]["selection_maximum_false_positives_per_trigger"] = json!(-1),
        &|config| config["thresholds"]["selection_maximum_false_positives_per_trigger"] = json!(1),
        &|config| config["thresholds"]["precision_floor"] = json!(1.0),
        &|config| config["thresholds"]["maximum_false_positive_rate"] = json!(0.0),
        &|config| config["quality_gates"]["selection_minimum_recall"] = json!(0.5),
        &|config| config["quality_gates"]["selection_minimum_recall"] = json!(1.0),
        &|config| config["quality_gates"]["selection_minimum_pause_recall"] = json!(1.0),
        &|config| config["quality_gates"]["selection_minimum_typo_recall"] = json!(1.0),
        &|config| config["quality_gates"]["selection_minimum_pause_typo_recall"] = json!(1.0),
        &|config| config["quality_gates"]["minimum_specificity"] = json!(1.0),
        &|config| config["thresholds"]["veto_positive_quantile"] = json!(0.5),
        &|config| config["thresholds"]["veto_max_false_negative_rate"] = json!(0.0),
        &|config| {
            config["ftrl"]["maximum_epochs"] = json!(6);
            config["ftrl"]["minimum_epochs"] = json!(1);
            config["ftrl"]["patience"] = json!(1);
            config["ftrl"]["alpha"] = json!(4.0);
        },
        &|config| {
            config["ftrl"]["maximum_epochs"] = json!(3);
            config["ftrl"]["l1"] = json!(100.0);
        },
        &|config| {
            config["ftrl"]["maximum_epochs"] = json!(4);
            config["ftrl"]["minimum_epochs"] = json!(4);
            config["ftrl"]["patience"] = json!(1);
        },
        &|config| config["calibration"]["maximum_iterations"] = json!(1),
        &|config| config["calibration"]["l2"] = json!(1_000_000.0),
        &|config| config["seed"] = json!(1_u64 << 33),
    ];
    for variant in variants {
        let mut changed = good.clone();
        variant(&mut changed);
        std::fs::write(&path, serde_json::to_vec(&changed).expect("json")).expect("a config");
        let _ = fit_candidate(&small.tree, 2).map(|candidate| candidate.presealed_gate["passed"].clone());
    }
    std::fs::write(&path, serde_json::to_vec(&good).expect("json")).expect("a config");
}

#[test]
fn the_audit_of_the_safety_rows_says_what_is_wrong_with_each() {
    use ks_lang::detector::ProtectedTokens;
    use ks_train::intent::pipeline::{self, SafetyAudit};
    let small = &*PUBLISHED;
    let presealed = pipeline::presealed_dataset(&small.tree).expect("a dataset");
    let protected = ProtectedTokens::load(&small.tree.join("resources"));
    let rows = &presealed.dataset.safety;
    let audit = SafetyAudit::of(rows, &protected);
    assert!(audit.passes(0));
    assert!(!audit.passes(-1));
    assert!(audit.payload().is_object());
    assert!(!SafetyAudit::of(&[], &protected).passes(0));
    let protected_row = rows.iter().find(|row| row.protected).expect("a protected row");
    let collision = rows.iter().find(|row| row.variant_kind == "lexical_collision").expect("a collision");
    let edits: [(&dyn Fn(&mut ks_train::intent::dataset::LexicalExample), bool); 9] = [
        (&|row| row.label = true, true),
        (&|row| row.safety = false, true),
        (&|row| row.variant_kind = "other".to_owned(), true),
        (&|row| row.source_known = true, true),
        (&|row| row.target_known = true, true),
        (&|row| row.original = "ordinary".to_owned(), true),
        (&|row| row.source_known = false, false),
        (&|row| row.target_known = false, false),
        (&|row| row.variant_kind = "other".to_owned(), false),
    ];
    for (edit, of_protected) in edits {
        let mut row = if of_protected { protected_row.clone() } else { collision.clone() };
        edit(&mut row);
        let audited = SafetyAudit::of(&[row], &protected);
        assert!(!audited.failures.is_empty());
        assert!(!audited.passes(0));
    }
    // Rows of one kind or of too few triggers do not make a whole audit.
    let only_protected: Vec<_> = rows.iter().filter(|row| row.protected).cloned().collect();
    assert!(!SafetyAudit::of(&only_protected, &protected).passes(0));
    let only_collisions: Vec<_> = rows.iter().filter(|row| !row.protected).cloned().collect();
    assert!(!SafetyAudit::of(&only_collisions, &protected).passes(0));
    let fewer_collisions: Vec<_> = rows.iter().filter(|row| row.protected || row.trigger.as_str() == "space").cloned().collect();
    assert!(!SafetyAudit::of(&fewer_collisions, &protected).passes(0));
    let fewer_protected: Vec<_> = rows.iter().filter(|row| !row.protected || row.trigger.as_str() == "space").cloned().collect();
    assert!(!SafetyAudit::of(&fewer_protected, &protected).passes(0));
}

#[test]
fn the_receipt_of_a_seal_is_held_to_its_ledger() {
    use ks_train::intent::seal::{claim_evaluation, receipt_is_valid, sha256_file, toolchain_snapshot, verify_toolchain};
    let small = prepared("intent-receipt", &|_| ());
    let config_path = config_of(&small);
    let config = read(&config_path);
    let registry = config["sealed_evaluation"]["registry_path"].as_str().expect("a registry").to_owned();
    let namespace = config["sealed_evaluation"]["split_namespace"].as_str().expect("a namespace").to_owned();
    let config_sha = sha256_file(&config_path).expect("a digest");
    let receipt = claim_evaluation(&small.tree, &registry, &namespace, "candidate", &config_sha, "dataset").expect("a claim");
    let evidence = receipt.payload();
    assert!(receipt_is_valid(&small.tree, &evidence, &config, &config_sha, "dataset", "candidate"));
    let check = |value: &Value| receipt_is_valid(&small.tree, value, &config, &config_sha, "dataset", "candidate");
    assert!(!check(&json!(1)));
    each_change(&evidence, |changed| assert!(!check(changed), "{changed}"));
    let mut extra = evidence.clone();
    extra["other"] = json!(1);
    assert!(!check(&extra));
    let mut less = evidence.clone();
    less.as_object_mut().expect("a table").remove("registry_sha256");
    assert!(!check(&less));
    let mut other_name = evidence.clone();
    let moved = other_name.as_object_mut().expect("a table").remove("registry_sha256").expect("a field");
    other_name["other"] = moved;
    assert!(!check(&other_name));
    let mut no_section = config.clone();
    no_section["sealed_evaluation"] = json!({});
    assert!(!receipt_is_valid(&small.tree, &evidence, &no_section, &config_sha, "dataset", "candidate"));
    assert!(!receipt_is_valid(&small.tree, &evidence, &config, "other", "dataset", "candidate"));
    let registry_path = small.tree.join(&registry);
    let bytes = std::fs::read(&registry_path).expect("the ledger");
    std::fs::write(&registry_path, b"{}").expect("a file");
    assert!(!check(&evidence));
    std::fs::remove_file(&registry_path).expect("the file goes");
    assert!(!check(&evidence));
    std::fs::write(&registry_path, bytes).expect("a file");
    // The toolchain, the same or moved.
    let snapshot = toolchain_snapshot(&small.tree, &config_sha, &registry).expect("a snapshot");
    verify_toolchain(&small.tree, &snapshot, &config_path, &registry).expect("the same");
    let mut moved = snapshot;
    moved["trainer_sha256"] = json!("0");
    assert!(verify_toolchain(&small.tree, &moved, &config_path, &registry).unwrap_err().contains("trainer_sha256"));
}

#[test]
fn the_frozen_dictionaries_and_the_samples_of_a_corpus() {
    use ks_lang::LanguageModel;
    use ks_train::intent::{external, frozen_dictionaries};
    let root = tree();
    let frozen = frozen_dictionaries(&root).expect("the dictionaries");
    assert!(!external::discover_snapshot("en_US", &frozen).expect("a snapshot").words.is_empty());
    assert!(external::discover_snapshot("en_US", Path::new("/nonexistent")).expect("a snapshot").words.is_empty());
    // A dictionary in another encoding, one with no words, one half there.
    let folder = scratch("intent-dictionaries");
    std::fs::write(folder.join("en_US.aff"), "SET KOI8-R\n").expect("a file");
    std::fs::write(folder.join("en_US.dic"), "1\nword\n").expect("a file");
    assert!(external::discover_snapshot("en_US", &folder).map(|_| ()).unwrap_err().contains("KOI8-R"));
    std::fs::write(folder.join("en_US.aff"), "SET UTF-8\n").expect("a file");
    assert!(!external::discover_snapshot("en_US", &folder).expect("a snapshot").provenance.locale.is_empty());
    let no_speller = LanguageModel::load("en_US", std::iter::empty::<&str>(), &[root.join("model/intent_v1/sources")], LanguageModel::disabled_speller());
    assert!(external::snapshot_of_model(&no_speller, "en_US").expect("a snapshot").words.is_empty());
    // A sample of a list.
    let list = [1, 2, 3, 4, 5];
    assert_eq!(external::deterministic_sample(&list, 10), list);
    assert_eq!(external::deterministic_sample(&list, 5), list);
    assert_eq!(external::deterministic_sample(&list, 1).len(), 1);
    assert_eq!(external::deterministic_sample(&list, 2).len(), 2);
    assert_eq!(external::deterministic_sample(&list, 0), list);
}

#[test]
fn a_manifest_that_does_not_fit_stops_the_strict_evaluation_where_it_differs() {
    use ks_train::intent::evaluate::{Options, evaluate};
    let small = &*PUBLISHED;
    let manifest_path = small.tree.join("model/intent_v1/manifest.json");
    let artifact = small.tree.join("resources/models/layout_intent_v1.ksm");
    let good = read(&manifest_path);
    let folder = scratch("intent-phases");
    let with = |edit: &dyn Fn(&mut Value)| -> (String, i32) {
        let mut changed = good.clone();
        edit(&mut changed);
        let path = folder.join("manifest.json");
        write_json(&path, &changed);
        evaluate(&Options { root: &small.tree, artifact: &artifact, manifest: &path, comparison_sample: 20, latency_sample: 5, strict: true, provenance_only: true, workers: 2 }).expect("a report")
    };
    let phase = |edit: &dyn Fn(&mut Value)| {
        let (report, status) = with(edit);
        assert_eq!(status, 1);
        let value: Value = serde_json::from_str(&report).expect("json");
        assert!(value["phase"].is_string(), "{report}");
    };
    let edits: [&dyn Fn(&mut Value); 12] = [
        &|value| value["artifact_sha256"] = json!("0".repeat(64)),
        &|value| value["artifact_model_version"] = json!("other"),
        &|value| value["quality_gate_breakdown"]["passed"] = json!(false),
        &|value| value["dataset_sha256"] = json!("0".repeat(64)),
        &|value| value["counts"] = json!({}),
        &|value| value["variant_quarantine_sha256"] = json!("0".repeat(64)),
        &|value| value["sealed_variant_quarantine_sha256"] = json!("0".repeat(64)),
        &|value| value["veto"] = json!({"selection": {}}),
        &|value| value["veto"]["extra"] = json!(1),
        &|value| value["safety"] = json!({"guard_audit": {}}),
        &|value| value["safety"]["extra"] = json!(1),
        &|value| value["config_sha256"] = json!("0".repeat(64)),
    ];
    for edit in edits {
        phase(edit);
    }
    // A model that cannot be read, a manifest that is not there.
    let missing = Path::new("/nonexistent");
    assert!(evaluate(&Options { root: &small.tree, artifact: missing, manifest: &manifest_path, comparison_sample: 1, latency_sample: 1, strict: false, provenance_only: false, workers: 1 }).is_err());
    assert!(evaluate(&Options { root: &small.tree, artifact: &artifact, manifest: missing, comparison_sample: 1, latency_sample: 1, strict: false, provenance_only: false, workers: 1 }).is_err());
    let answer = run(intent(&small.tree).arg("--evaluate").arg("--artifact").arg(missing));
    assert!(!answer.status.success());
    // Against the tree a config differs from.
    let other = prepared("intent-other-config", &|config| config["ftrl"]["maximum_epochs"] = json!(1));
    std::fs::copy(&artifact, other.tree.join("resources/models/layout_intent_v1.ksm")).expect("a copy");
    std::fs::copy(&manifest_path, other.tree.join("model/intent_v1/manifest.json")).expect("a copy");
    let answer = run(intent(&other.tree).arg("--evaluate").args(["--comparison-sample", "5", "--latency-sample", "2"]));
    assert!(said(&answer).contains("presealed_provenance"), "{}", said(&answer));
    // The replay of a tree whose recorded model is another one.
    let replay = run(intent(&other.tree).arg("--verify").args(["--jobs", "1"]));
    assert!(!replay.status.success(), "{}", said(&replay));
}

#[test]
fn a_frozen_corpus_whose_rows_do_not_add_up_is_refused() {
    use ks_lang::detector::ProtectedTokens;
    use ks_train::intent::{dataset, pipeline};
    let small = &*PUBLISHED;
    let config = read(&config_of(small));
    let shape = pipeline::dataset_config(&config).expect("a dataset");
    let protected = ProtectedTokens::load(&small.tree.join("resources"));
    let raw = dataset::read_frozen(&small.tree, &config["hard_negative_development"]["source"]).expect("the corpus");
    let frozen: Value = serde_json::from_slice(&raw).expect("json");
    let refused = |corpus: &Value, config: &Value, words: &str| {
        let said = dataset::decode_hard_negative_corpus(&serde_json::to_vec(corpus).expect("json"), config, &shape, &protected).map(|_| ()).unwrap_err();
        assert!(said.contains(words), "{words}: {said}");
    };
    let mut fewer = frozen.clone();
    fewer["rows"].as_array_mut().expect("rows").pop();
    refused(&fewer, &config, "compact row count is inconsistent");
    // A protected word, typed either way round.
    for (group, correct, wrong) in [(0, "github", "пшерги"), (1, "пшерги", "github")] {
        let mut guarded = frozen.clone();
        let row = &mut guarded["rows"][0];
        row["physical_signature"] = json!("github");
        row["correct_group"] = json!(group);
        row["correct_typo"] = json!(correct);
        row["wrong_typo"] = json!(wrong);
        refused(&guarded, &config, "contains a protected token");
    }
    // A row of the other group than the counts say.
    let mut moved = frozen.clone();
    let row = moved["rows"][0].clone();
    moved["rows"][0]["correct_group"] = json!(1 - row["correct_group"].as_i64().expect("a group"));
    moved["rows"][0]["correct_typo"] = row["wrong_typo"].clone();
    moved["rows"][0]["wrong_typo"] = row["correct_typo"].clone();
    refused(&moved, &config, "disagree with group counts");
    // Roles that leave rows out, a field longer than a field can be, an expansion that is not the pinned one.
    let mut fewer_roles = config.clone();
    fewer_roles["hard_negative_development"]["words_per_group"]["train"] = json!(21);
    refused(&frozen, &fewer_roles, "left unused rows");
    let mut long = frozen.clone();
    long["rows"][0]["variant_kind"] = json!("x".repeat(300));
    refused(&long, &config, "non-empty bounded text");
    let zero = "0".repeat(64);
    let mut pinned = frozen;
    pinned["expanded_corpus_sha256"] = json!(zero);
    let mut repinned = config.clone();
    repinned["external_evaluation"]["unknown_typo_development_corpus_sha256"] = json!(zero);
    refused(&pinned, &repinned, "expanded corpus SHA-256 mismatch");
}

#[test]
fn an_evaluation_that_is_not_strict_reports_and_passes() {
    let small = &*PUBLISHED;
    let report = small.folder.join("lenient.json");
    let answer = run(intent(&small.tree).arg("--evaluate").args(["--comparison-sample", "20", "--latency-sample", "5", "--jobs", "2"]).arg("--output").arg(&report));
    assert!(answer.status.success(), "{}", said(&answer));
    assert!(read(&report).get("strict_gates").is_some());
}

#[test]
fn a_rejection_names_its_version_and_carries_what_its_failed_gates_say() {
    let small = &*PUBLISHED;
    let report = small.folder.join("failed.json");
    write_json(&report, &json!({"strict_gates": {"a": false, "safety": false, "veto": false}, "strict_passed": false, "veto": {"items": (0..20).collect::<Vec<i32>>()}}));
    let reject = |version: &str, output: &Path| run(intent(&small.tree).arg("--reject").args(["--version", version]).arg("--strict-report").arg(&report).arg("--output").arg(output));
    assert!(said(&reject("99", &small.folder.join("rejected-v99.json"))).contains("is not v99"));
    let receipt = small.folder.join("rejected-v28.json");
    let answer = reject("28", &receipt);
    assert!(answer.status.success(), "{}", said(&answer));
    let written = read(&receipt);
    assert_eq!(written["failure"]["veto"], json!({"items": {"omitted_items": 20}}));
    assert!(written["failure"].get("safety").is_none());
}

#[test]
fn a_replay_finds_each_file_that_is_not_the_one_it_makes_again() {
    let small = prepared("intent-replay", &|_| ());
    let path = config_of(&small);
    let good = read(&path);
    // A lexicon that holds the typos the development corpus chose: they are sealed rows now, and it has to choose others.
    let relative = good["sources"]["languages"]["en_US"]["path"].as_str().expect("a path").to_owned();
    let lexicon = small.tree.join(&relative);
    let words = std::fs::read_to_string(&lexicon).expect("a lexicon");
    let frozen = read(&small.tree.join(good["hard_negative_development"]["source"]["path"].as_str().expect("a path")));
    let chosen: String = frozen["rows"].as_array().expect("rows").iter().filter(|row| row["correct_group"] == json!(0)).map(|row| format!("1000 {}\n", row["correct_typo"].as_str().expect("a typo"))).collect();
    let more = words.replacen("\\end\\", &format!("{chosen}\\end\\"), 1);
    std::fs::write(&lexicon, &more).expect("a lexicon");
    let mut other = good.clone();
    pin(&mut other, "/sources/languages/en_US", &relative, more.as_bytes());
    write_json(&path, &other);
    let preseal = run(intent(&small.tree).arg("--preseal"));
    assert!(said(&preseal).contains("frozen hard-negative source differs"), "{}", said(&preseal));
    std::fs::write(&lexicon, &words).expect("a lexicon");
    write_json(&path, &good);
    let fitted = run(intent(&small.tree).arg("--fit").args(["--jobs", "2"]));
    assert!(fitted.status.success(), "{}", said(&fitted));
    let report = small.tree.join("model/intent_v1/test-report.json");
    let bytes = std::fs::read(&report).expect("the report");
    std::fs::write(&report, [bytes.as_slice(), b"\n"].concat()).expect("a report");
    let replay = run(intent(&small.tree).arg("--verify").args(["--jobs", "2"]));
    assert!(said(&replay).contains("1 sections differ"), "{}", said(&replay));
}
