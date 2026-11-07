//! The orthotactic model made from a thinned copy of its corpus, under every schema its configuration can name.
mod support;

use flate2::Compression;
use flate2::read::MultiGzDecoder;
use flate2::write::GzEncoder;
use serde_json::{Value, json};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use support::{digest, recipe, scratch, small_tree, trainer};

const KEEP_EVERY: usize = 40;

fn read(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).expect("a file")).expect("json")
}

fn write_json(path: &Path, value: &Value) {
    std::fs::write(path, serde_json::to_vec(value).expect("json")).expect("a file");
}

/// A tree whose orthotactic corpus is every so manyth row of the real one.
fn thinned(name: &str) -> PathBuf {
    let tree = small_tree(name, &recipe());
    let directory = tree.join("model/ortho_v1");
    let tokens = directory.join("tokens.jsonl.gz");
    let mut text = String::new();
    MultiGzDecoder::new(std::fs::File::open(&tokens).expect("the corpus")).read_to_string(&mut text).expect("text");
    let kept: Vec<&str> = text.lines().enumerate().filter(|(index, _)| index % KEEP_EVERY == 0).map(|(_, line)| line).collect();
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all((kept.join("\n") + "\n").as_bytes()).expect("in memory");
    let packed = encoder.finish().expect("in memory");
    std::fs::write(&tokens, &packed).expect("a file");
    let receipt = directory.join("corpus-receipt.json");
    let mut value = read(&receipt);
    value["tokens_sha256"] = json!(digest(&packed));
    write_json(&receipt, &value);
    for (forms, receipt) in [("web-lexicon-ru.json.gz", "web-lexicon-receipt.json"), ("prose-lexicon-ru.json.gz", "prose-lexicon-receipt.json")] {
        let mut text = String::new();
        MultiGzDecoder::new(std::fs::File::open(directory.join(forms)).expect("the forms")).read_to_string(&mut text).expect("text");
        let mut payload: Value = serde_json::from_str(&text).expect("json");
        if let Some(items) = payload["forms"].as_array_mut() {
            let mut position = 0;
            items.retain(|_| { position += 1; position % KEEP_EVERY == 0 });
        }
        if let Some(items) = payload["forms"].as_object_mut() {
            let mut position = 0;
            items.retain(|_, _| { position += 1; position % KEEP_EVERY == 0 });
        }
        write_forms(&directory, forms, receipt, &payload);
    }
    tree
}

fn write_forms(directory: &Path, forms: &str, receipt: &str, payload: &Value) {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(payload.to_string().as_bytes()).expect("in memory");
    let packed = encoder.finish().expect("in memory");
    std::fs::write(directory.join(forms), &packed).expect("a file");
    let path = directory.join(receipt);
    let mut value = read(&path);
    value["forms_sha256"] = json!(digest(&packed));
    write_json(&path, &value);
}

fn ortho(tree: &Path) -> Command {
    let mut command = trainer();
    command.arg("ortho").arg("--tree").arg(tree);
    command
}

fn said(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

fn run(command: &mut Command) -> Output {
    command.output().expect("the trainer starts")
}

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn fit_with(tree: &Path, change: &dyn Fn(&mut Value)) -> (Output, PathBuf) {
    let path = tree.join("model/ortho_v1/config.json");
    let good = read(&path);
    let mut config = good.clone();
    change(&mut config);
    write_json(&path, &config);
    let output = scratch(&format!("ortho-out-{}", NEXT.fetch_add(1, Ordering::Relaxed))).join("model");
    let answer = run(ortho(tree).arg("--fit").arg("--output").arg(&output));
    write_json(&path, &good);
    (answer, output)
}

#[test]
fn every_schema_a_configuration_can_name_fits() {
    let tree = thinned("ortho-schemas");
    let schemas: [(&str, &dyn Fn(&mut Value)); 9] = [
        ("as kept", &|_| ()),
        ("no collapse", &|config| config["collapse_runs"] = json!(null)),
        ("lower case unscored", &|config| config["unscored_shapes"] = json!(["lower"])),
        ("no stretch", &|config| {
            config["stretch_runs"] = json!(null);
            config["compound_min_letters"] = json!({});
        }),
        ("a stretch only", &|config| {
            config["hyphen_stretch"] = json!(false);
            config["compound_min_letters"] = json!({});
        }),
        ("plausibility", &|config| config["target_plausibility"] = json!(true)),
        ("gated", &|config| {
            config["target_plausibility"] = json!(true);
            config["feature_gated"] = json!(["extra_key"]);
        }),
        ("no web, no prose", &|config| {
            config["web_source_scripts"] = json!([]);
            config["prose_source_scripts"] = json!([]);
        }),
        ("all tied", &|config| {
            config["feature_tied"] = json!([]);
            config["feature_omitted"] = json!([]);
            config["feature_shared"] = json!([]);
            config["feature_separated"] = json!({"en": [], "ru": []});
        }),
    ];
    for (name, change) in schemas {
        let (answer, _) = fit_with(&tree, change);
        assert!(answer.status.success(), "{name}: {}", said(&answer));
    }
}

#[test]
fn a_configuration_that_cannot_be_is_refused() {
    let tree = thinned("ortho-refused");
    let refused: [(&str, &dyn Fn(&mut Value)); 7] = [
        ("needs the schema 5", &|config| {
            config["hyphen_stretch"] = json!(false);
            config["target_plausibility"] = json!(true);
        }),
        ("gates features", &|config| config["feature_gated"] = json!(["ident"])),
        ("length of a compound", &|config| config["hyphen_stretch"] = json!(false)),
        ("Russian forms only", &|config| config["web_source_scripts"] = json!(["en"])),
        ("separated column", &|config| config["feature_separated"] = json!({"en": ["nothing"], "ru": []})),
        ("in both directions or in none", &|config| config["feature_separated"] = json!({"en": ["extra_key"], "ru": []})),
        ("", &|config| config["order"] = json!("many")),
    ];
    for (words, change) in refused {
        let (answer, _) = fit_with(&tree, change);
        assert!(!answer.status.success() && said(&answer).contains(words), "{words}: {}", said(&answer));
    }
    // A shared feature fitted apart in both directions.
    let (answer, _) = fit_with(&tree, &|config| config["feature_separated"] = json!({"en": ["extra_key"], "ru": ["extra_key"]}));
    println!("{}", said(&answer).lines().last().unwrap_or(""));
    // Receipts that do not hold.
    let directory = tree.join("model/ortho_v1");
    for (receipt, field, words) in [("corpus-receipt.json", "tokens_sha256", "key-space corpus"), ("known-receipt.json", "evidence_sha256", "dictionary evidence"), ("web-lexicon-receipt.json", "forms_sha256", "web lexicon"), ("prose-lexicon-receipt.json", "forms_sha256", "prose lexicon")] {
        let path = directory.join(receipt);
        let good = read(&path);
        let mut changed = good.clone();
        changed[field] = json!("0");
        write_json(&path, &changed);
        let answer = run(ortho(&tree).arg("--fit").arg("--output").arg(scratch("ortho-receipt")));
        assert!(!answer.status.success() && said(&answer).contains(words), "{words}: {}", said(&answer));
        write_json(&path, &good);
    }
    // Forms in no shape a lexicon has.
    write_forms(&directory, "prose-lexicon-ru.json.gz", "prose-lexicon-receipt.json", &json!({"forms": 3}));
    let answer = run(ortho(&tree).arg("--fit").arg("--output").arg(scratch("ortho-forms")));
    assert!(said(&answer).contains("a lexicon without forms"), "{}", said(&answer));
    let answer = ortho_action(&tree, &[]);
    assert!(said(&answer).contains("--fit"));
}

fn ortho_action(tree: &Path, arguments: &[&str]) -> Output {
    run(ortho(tree).args(arguments))
}

#[test]
fn a_thinned_model_goes_through_its_test_and_its_promotion() {
    let tree = thinned("ortho-steps");
    let output = scratch("ortho-steps-out").join("model");
    let step = |action: &str| run(ortho(&tree).arg(format!("--{action}")).arg("--output").arg(&output));
    let fitted = step("fit");
    assert!(fitted.status.success(), "{}", said(&fitted));
    let seal_path = output.join("seal.json");
    let seal = read(&seal_path);
    // Changed provenance stops the test.
    let mut moved = seal.clone();
    moved["provenance"] = json!({});
    write_json(&seal_path, &moved);
    assert!(said(&step("test")).contains("changed after the seal"));
    write_json(&seal_path, &seal);
    let tested = step("test");
    assert!(tested.status.success(), "{}", said(&tested));
    // What the report says, and whether it names this candidate.
    let report_path = output.join("report.json");
    let report = read(&report_path);
    let mut passed = report.clone();
    passed["promotion_passed"] = json!(true);
    write_json(&report_path, &passed);
    let promoted = step("promote");
    assert!(promoted.status.success(), "{}", said(&promoted));
    let mut other = passed;
    other["candidate_sha256"] = json!("0");
    write_json(&report_path, &other);
    assert!(said(&step("promote")).contains("without a passing report"));
    // A seal with no candidate, and a test whose floors a small corpus cannot reach or can.
    let directory = tree.join("model/ortho_v1");
    let config_path = directory.join("config.json");
    let good = read(&config_path);
    for (negatives, conversions) in [(0, 1_000_000), (0, 0), (1_000_000, 1_000_000)] {
        let mut config = good.clone();
        config["promotion"]["minimum_test_negatives"] = json!(negatives);
        config["promotion"]["maximum_test_false_conversions"] = json!(conversions);
        config["promotion"]["minimum_recall"] = json!(0.0);
        write_json(&config_path, &config);
        std::fs::remove_file(&report_path).expect("the report goes");
        let again = step("test");
        println!("{}", said(&again).lines().last().unwrap_or(""));
        write_json(&report_path, &json!({}));
    }
    write_json(&config_path, &good);
    let verified = step("verify");
    println!("{}", said(&verified).lines().last().unwrap_or(""));
    // Refitting over what was sealed is refused, whichever of the two is left.
    std::fs::remove_file(output.join("candidate.json")).ok();
    assert!(said(&step("fit")).contains("do not refit over sealed evidence"));
}

#[test]
fn thresholds_that_let_everything_through_count_as_false_conversions() {
    let tree = thinned("ortho-low");
    let output = scratch("ortho-low-out").join("model");
    let path = tree.join("model/ortho_v1/config.json");
    let mut config = read(&path);
    for direction in ["en", "ru"] {
        config["thresholds_nats"][direction] = json!(-100_000.0);
    }
    write_json(&path, &config);
    let fit = run(ortho(&tree).arg("--fit").arg("--output").arg(&output));
    assert!(fit.status.success(), "{}", said(&fit));
    let tested = run(ortho(&tree).arg("--test").arg("--output").arg(&output));
    assert!(said(&tested).contains("promotion passed: false"), "{}", said(&tested));
    let report = read(&output.join("report.json"));
    assert!(report["corpus_test"]["counts"].as_object().expect("counts").keys().any(|name| name.ends_with("false_types")), "{report}");
}

/// Lower case left unscored and every threshold low enough to let the rest through: a token of the unscored shape never
/// converts, and false conversions over the maximum fail a promotion whose other floors hold.
#[test]
fn an_unscored_shape_never_converts_and_false_conversions_over_the_maximum_fail() {
    let tree = thinned("ortho-unscored");
    let output = scratch("ortho-unscored-out").join("model");
    let path = tree.join("model/ortho_v1/config.json");
    let mut config = read(&path);
    config["unscored_shapes"] = json!(["lower"]);
    for direction in ["en", "ru"] {
        config["thresholds_nats"][direction] = json!(-100_000.0);
    }
    config["promotion"]["minimum_test_negatives"] = json!(0);
    config["promotion"]["maximum_test_false_conversions"] = json!(0);
    config["promotion"]["minimum_recall"] = json!(0.0);
    write_json(&path, &config);
    let fit = run(ortho(&tree).arg("--fit").arg("--output").arg(&output));
    assert!(fit.status.success(), "{}", said(&fit));
    let tested = run(ortho(&tree).arg("--test").arg("--output").arg(&output));
    assert!(said(&tested).contains("promotion passed: false"), "{}", said(&tested));
    let report = read(&output.join("report.json"));
    let counts = report["corpus_test"]["counts"].as_object().expect("counts");
    assert!(counts.iter().any(|(name, count)| name.ends_with("false_types") && count.as_i64() > Some(0)), "{report}");
}
