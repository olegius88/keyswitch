//! The receipt of the installed pair passes the checks, and a receipt whose numbers were changed, whose fields are gone or
//! that names other files does not.
#![expect(clippy::unwrap_used, reason = "a test or a probe stops on what it did not expect")]
#![expect(clippy::disallowed_types, clippy::disallowed_macros, clippy::disallowed_methods, reason = "temporary: these tests still build and read their JSON as untyped trees; step 7 of the strict-json plan types them and removes this")]
mod support;

use ks_train::corpus::read_json;
use ks_train::artifact::{tree_of, typed};
use ks_train::evaluate::{Earliness, PROTOCOLS, SEQUENCE_PROTOCOL_VERSION, protocol_set};
use ks_train::receipt::{validate_evidence, validate_receipt};
use ks_train::sequence_report::{ByPair, FirstGates, Gated, Gates, InstalledReference, PairCounts, PublicFirst, PublicProfile, PublicTest};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn check(receipt: &Value) -> Result<(), String> {
    let root = root();
    let models = root.join("resources/models");
    validate_evidence(receipt, &models.join("context_policy_v1.json"), &root, &models.join("prefix_policy_v1.json"), &models)
}

fn receipt() -> Value {
    read_json(&root().join("model/context_v3/release-receipt.json")).unwrap()
}

#[test]
fn the_receipt_of_the_installed_pair_holds() {
    check(&receipt()).unwrap();
}

#[test]
fn a_changed_number_is_refused() {
    let edits: [(&str, fn(&mut Value)); 9] = [
        ("another artifact", |value| value["artifact_sha256"] = json!("0".repeat(64))),
        ("gates not passed", |value| value["quality_gates_passed"] = json!(false)),
        ("another policy", |value| value["gate_policy"]["sequence_length_mismatches"] = json!(1)),
        ("another threshold", |value| value["conversion_threshold"] = json!(0.5)),
        ("restorations below the baseline with the gate still claimed", |value| value["test"]["profiles"]["portable"]["counts"]["candidate"]["exactly_restored"] = json!(0)),
        ("a corruption removed from one side", |value| {
            let cell = &mut value["test"]["profiles"]["portable"]["early_off"]["counts"]["candidate"]["correct_text_corruptions"];
            *cell = json!(cell.as_i64().unwrap() + 3);
        }),
        ("a calibration count changed", |value| {
            let cell = &mut value["calibration"]["by_profile"]["portable"]["converted_correctly"];
            *cell = json!(cell.as_i64().unwrap() + 1);
        }),
        ("the chat test compared another pair", |value| value["devchat_test"]["pairs"]["candidate"]["sha256"] = json!("0".repeat(64))),
        ("a chat floor claimed without its numbers", |value| {
            let cell = &mut value["devchat_test"]["results"]["candidate"]["correct"]["ok"];
            *cell = json!(cell.as_i64().unwrap() - 10_000);
        }),
    ];
    for (name, edit) in edits {
        let mut changed = receipt();
        edit(&mut changed);
        assert!(check(&changed).is_err(), "{name}");
    }
}

#[test]
fn every_top_level_field_is_required_and_no_other_is_taken() {
    let whole = receipt();
    for name in whole.as_object().unwrap().keys() {
        let mut changed = whole.clone();
        changed.as_object_mut().unwrap().remove(name);
        assert!(check(&changed).is_err(), "{name} may be missing");
    }
    let mut changed = whole;
    changed["private_path"] = json!(".t/private/report.json");
    assert!(check(&changed).is_err(), "an unexpected field is taken");
}

#[test]
fn a_claimed_acceptance_cannot_override_the_counts() {
    let edits: [(&str, fn(&mut Value)); 6] = [
        ("damage in the portable profile with the gates still claimed", |value| {
            let cell = &mut value["test"]["profiles"]["portable"]["counts"]["candidate"]["correct_text_corruptions"];
            *cell = json!(cell.as_i64().unwrap() + 1);
        }),
        ("damage in the reference profile with the gates still claimed", |value| {
            let cell = &mut value["test"]["profiles"]["reference_hunspell"]["counts"]["candidate"]["correct_text_corruptions"];
            *cell = json!(cell.as_i64().unwrap() + 1);
        }),
        ("a profile gone", |value| {
            value["test"]["profiles"].as_object_mut().unwrap().remove("reference_hunspell");
        }),
        ("the control without early switching gone", |value| {
            value["test"]["profiles"]["portable"].as_object_mut().unwrap().remove("early_off");
        }),
        ("a calibration profile gone", |value| {
            value["calibration"]["by_profile"].as_object_mut().unwrap().remove("portable");
        }),
        ("the chat test gone", |value| {
            value.as_object_mut().unwrap().remove("devchat_test");
        }),
    ];
    for (name, edit) in edits {
        let mut changed = receipt();
        edit(&mut changed);
        assert!(check(&changed).is_err(), "{name}");
    }
}

#[test]
fn the_rejected_artifact_is_blocked_by_its_digest() {
    let root = root();
    let models = root.join("resources/models");
    let rejected = root.join("model/context_v2/candidate.json");
    assert!(validate_evidence(&receipt(), &rejected, &root, &models.join("prefix_policy_v1.json"), &models).is_err());
}

#[test]
fn the_installed_prefix_model_is_bound_to_the_receipt() {
    let root = root();
    let models = root.join("resources/models");
    let other = root.join("model/prefix_v1/candidate.json");
    assert!(validate_evidence(&receipt(), &models.join("context_policy_v1.json"), &root, &other, &models).is_err());
}

/// Each check of a receipt on its own: every edit leaves the checks before it holding.
fn refuses(words: &str, edit: impl FnOnce(&mut Value)) {
    let mut changed = receipt();
    edit(&mut changed);
    let said = check(&changed).expect_err(words);
    assert!(said.contains(words), "{words}: {said}");
}

fn plus(cell: &mut Value, by: i64) {
    *cell = json!(cell.as_i64().unwrap() + by);
}

#[test]
fn what_a_receipt_says_of_itself_is_checked() {
    assert!(check(&json!(1)).unwrap_err().contains("invalid object"));
    const IDENTITY: &str = "unsupported receipt identity";
    refuses(IDENTITY, |value| value["schema_version"] = json!(2));
    refuses(IDENTITY, |value| value["feature_version"] = json!(2));
    refuses(IDENTITY, |value| value["scope"] = json!("other"));
    refuses(IDENTITY, |value| value["protocol"] = json!("other"));
    refuses(IDENTITY, |value| value["baseline_path"] = json!("other"));
    refuses(IDENTITY, |value| value["recipe_path"] = json!("other"));
    refuses(IDENTITY, |value| value["prefix_baseline_path"] = json!("other"));
    const WEIGHTS: &str = "context weights, version or threshold";
    refuses(WEIGHTS, |value| value["weights_sha256"] = json!("0".repeat(64)));
    refuses("invalid SHA256", |value| value["weights_sha256"] = json!("0"));
    refuses("invalid SHA256", |value| value["weights_sha256"] = json!(1));
    refuses("invalid SHA256", |value| value["weights_sha256"] = json!("z".repeat(64)));
    refuses(WEIGHTS, |value| value["model_version"] = json!("other"));
    const PREFIX: &str = "prefix weights, version or threshold";
    refuses(PREFIX, |value| value["prefix_artifact_sha256"] = json!("0".repeat(64)));
    refuses(PREFIX, |value| value["prefix_feature_version"] = json!(99));
    refuses(PREFIX, |value| value["prefix_conversion_threshold"] = json!(0.5));
    refuses(PREFIX, |value| value["prefix_weights_sha256"] = json!("0".repeat(64)));
    refuses(PREFIX, |value| value["prefix_model_version"] = json!("other"));
}

#[test]
fn files_that_cannot_be_read_stop_the_check() {
    let root = root();
    let models = root.join("resources/models");
    let folder = std::env::temp_dir().join(format!("ks-receipt-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let broken = folder.join("broken.json");
    std::fs::write(&broken, b"{}").unwrap();
    let mut changed = receipt();
    changed["artifact_sha256"] = json!(ks_train::corpus::checksum(&broken).unwrap());
    assert!(validate_evidence(&changed, &broken, &root, &models.join("prefix_policy_v1.json"), &models).is_err());
    let good = receipt();
    assert!(validate_evidence(&good, &models.join("context_policy_v1.json"), &root, &broken, &models).is_err());
    let garbage = folder.join("garbage.json");
    std::fs::write(&garbage, b"not json").unwrap();
    assert!(validate_evidence(&good, &models.join("context_policy_v1.json"), &root, &garbage, &models).is_err());
    let _ = std::fs::remove_dir_all(folder);
}

#[test]
fn the_counts_of_a_calibration_must_add_up() {
    const GATE: &str = "calibration gate failed";
    refuses("invalid count", |value| value["calibration"]["rows"] = json!(-1));
    refuses("invalid count", |value| value["calibration"]["convert_rows"] = json!("many"));
    refuses(GATE, |value| value["calibration"]["convert_rows"] = json!(0));
    refuses(GATE, |value| value["calibration"]["rows"] = json!(1));
    refuses(GATE, |value| value["calibration"]["conversion_recall"] = json!("high"));
    refuses(GATE, |value| value["calibration"]["conversion_recall"] = json!(0.5));
    refuses(GATE, |value| {
        let possible = value["calibration"]["convert_rows"].clone();
        plus(&mut value["calibration"]["converted_correctly"], possible.as_i64().unwrap());
    });
    refuses(GATE, |value| {
        let right = value["calibration"]["converted_correctly"].clone();
        value["calibration"]["false_conversions"] = json!(right.as_i64().unwrap() + 1);
    });
    refuses(GATE, |value| {
        let right = value["calibration"]["converted_correctly"].clone();
        value["calibration"]["false_conversions"] = right;
    });
    refuses("missing or extra fields", |value| value["calibration"]["by_profile"] = json!({}));
    refuses("aggregate mismatch", |value| plus(&mut value["calibration"]["rows"], 1));
    refuses("invalid count", |value| value["calibration"]["outside_lone_word"]["rows"] = json!(-1));
}

/// The test block of a receipt, as its type.
fn test_of(receipt: &Value) -> PublicTest {
    typed(&receipt["test"]).unwrap()
}

/// A receipt of the installed pair whose test block is changed as its type.
fn refuses_test(words: &str, edit: impl FnOnce(&mut PublicTest)) {
    refuses(words, |value| {
        let mut test = test_of(value);
        edit(&mut test);
        value["test"] = tree_of(&test);
    });
}

/// A profile of a test by its name.
fn profile_in<'a>(test: &'a mut PublicTest, profile: &str) -> &'a mut PublicProfile {
    match profile {
        "portable" => &mut test.profiles.portable,
        _ => &mut test.profiles.reference_hunspell,
    }
}

/// The counts and the gates of a block of a profile of a test: the default mode's, or its control's.
fn block_in<'a>(test: &'a mut PublicTest, profile: &str, early: bool) -> (&'a mut Gated<PairCounts>, &'a mut Gates) {
    let profile = profile_in(test, profile);
    if early { (&mut profile.early_off.counts, &mut profile.early_off.gates) } else { (&mut profile.counts, &mut profile.gates) }
}

/// The counts of a pair in a block of a profile of a test.
fn counts_in<'a>(test: &'a mut PublicTest, profile: &str, early: bool, model: &str) -> &'a mut PairCounts {
    let (counts, _) = block_in(test, profile, early);
    match model {
        "candidate" => &mut counts.candidate,
        _ => &mut counts.baseline,
    }
}

/// The same counts as a tree, for an edit that is no number.
fn counts_tree<'a>(value: &'a mut Value, profile: &str, early: bool, model: &str) -> &'a mut Value {
    let block = &mut value["test"]["profiles"][profile];
    let block = if early { &mut block["early_off"] } else { block };
    &mut block["counts"][model]
}

#[test]
fn the_selection_of_a_test_must_add_up() {
    const SELECTION: &str = "inconsistent sequence selection";
    refuses("missing or extra fields", |value| value["test"]["extra"] = json!(1));
    refuses_test("invalid count", |test| test.documents.us = -1);
    refuses_test(SELECTION, |test| test.documents.us = 513);
    refuses_test(SELECTION, |test| test.selected_rows = 1537);
    refuses_test(SELECTION, |test| test.selected_rows += 1);
    refuses_test("invalid count", |test| test.selected_rows = -1);
    refuses_test("invalid count", |test| test.unsupported_rows = -1);
    refuses_test("invalid count", |test| test.trimmed_rows = -1);
    refuses_test(SELECTION, |test| test.trimmed_rows = test.selected_rows + 1);
    refuses("missing or extra fields", |value| value["test"]["profiles"] = json!({}));
}

#[test]
fn the_counts_of_a_replay_must_add_up() {
    const COUNTS: &str = "inconsistent sequence counts";
    for (profile, early) in [("portable", false), ("reference_hunspell", false), ("portable", true), ("reference_hunspell", true)] {
        for model in ["candidate", "baseline"] {
            let with = |edit: &dyn Fn(&mut PairCounts), words: &str| refuses_test(words, |test| edit(counts_in(test, profile, early, model)));
            refuses("missing or extra fields", |value| counts_tree(value, profile, early, model)["extra"] = json!(1));
            with(&|counts| counts.rows = -1, "invalid count");
            with(&|counts| counts.rows += 1, COUNTS);
            with(&|counts| counts.initially_correct += 1, COUNTS);
            with(&|counts| counts.initially_wrong += 1, COUNTS);
            with(&|counts| counts.exactly_restored = 100_000, COUNTS);
            with(&|counts| counts.exactly_restored = counts.injections + 1, COUNTS);
            with(&|counts| counts.preserved_correct += 1, COUNTS);
            with(&|counts| counts.length_mismatches = 100_000, COUNTS);
            with(&|counts| counts.execution_errors = 100_000, COUNTS);
            with(&|counts| counts.final_layout_mismatches = 100_000, COUNTS);
            with(&|counts| counts.correction_layout_mismatches = counts.injections + 1, COUNTS);
        }
        // Counts that hold together, and that no gate lets through.
        let gated = |edit: &dyn Fn(&mut Gated<PairCounts>, &mut Gates)| {
            refuses_test("sequence gates failed", |test| {
                let (counts, gates) = block_in(test, profile, early);
                edit(counts, gates);
            });
        };
        gated(&|counts, _| {
            let more = counts.baseline.correct_text_corruptions + 1 - counts.candidate.correct_text_corruptions;
            counts.candidate.correct_text_corruptions += more;
            counts.candidate.preserved_correct -= more;
        });
        gated(&|counts, _| counts.candidate.length_mismatches = 1);
        gated(&|counts, _| counts.candidate.exactly_restored = counts.baseline.exactly_restored - counts.baseline.correct_text_corruptions + counts.candidate.correct_text_corruptions - 1);
        gated(&|counts, _| counts.candidate.execution_errors = 1);
        gated(&|counts, _| counts.baseline.execution_errors = 1);
        gated(&|counts, _| counts.candidate.correction_layout_mismatches = 1);
        gated(&|_, gates| gates.length_preserved = false);
    }
}

/// A gate about how much was read: fewer documents than a test needs, with everything else in step.
#[test]
fn a_test_of_too_few_documents_passes_no_gate() {
    for russian in [false, true] {
        refuses_test("sequence gates failed", |test| {
            let documents = if russian { &mut test.documents.ru } else { &mut test.documents.us };
            let gone = *documents - 31;
            *documents = 31;
            test.selected_rows -= gone;
            for (_, profile) in test.profiles.each_mut() {
                for counts in [&mut profile.counts, &mut profile.early_off.counts] {
                    for pair in [&mut counts.candidate, &mut counts.baseline] {
                        pair.rows -= 2 * gone;
                        pair.initially_correct -= gone;
                        pair.initially_wrong -= gone;
                        pair.preserved_correct -= gone;
                        pair.exactly_restored = 0;
                        pair.final_layout_mismatches = 0;
                    }
                }
            }
        });
    }
}


#[test]
fn a_chat_test_is_held_to_its_own_numbers() {
    for name in ["messages_sha256", "result_sha256", "access"] {
        refuses("invalid SHA256", |value| value["devchat_test"][name] = json!("0"));
    }
    const PAIRS: &str = "compared other pairs";
    refuses(PAIRS, |value| value["devchat_test"]["pairs"]["candidate"]["model_version"] = json!("other"));
    refuses(PAIRS, |value| value["devchat_test"]["pairs"]["baseline"]["sha256"] = json!("0".repeat(64)));
    const FLOORS: &str = "developer-chat floors";
    refuses(FLOORS, |value| value["devchat_test"]["results"]["candidate"]["correct"]["errors"] = json!(1));
    refuses(FLOORS, |value| value["devchat_test"]["floors"]["every_message_typed"] = json!(false));
    // More corruptions than the baseline, with the net still ahead of it.
    refuses(FLOORS, |value| {
        let results = &mut value["devchat_test"]["results"];
        plus(&mut results["candidate"]["correct"]["ok"], -5);
        plus(&mut results["candidate"]["forget_ru"]["ok"], 1000);
    });
    // Worse than the released model by chance alone, in one mode.
    refuses(FLOORS, |value| value["devchat_test"]["discordance"]["forget_ru"]["losses"] = json!(50));
    refuses(FLOORS, |value| {
        let results = &mut value["devchat_test"]["results"];
        plus(&mut results["released"]["forget_ru"]["ok"], 100);
    });
    refuses(FLOORS, |value| {
        let results = &mut value["devchat_test"]["results"];
        plus(&mut results["released"]["correct"]["ok"], 100);
    });
}

/// `validate_receipt`: what a receipt names of the tree, over a tree whose provenance a receipt could name.
#[test]
fn the_sources_a_receipt_names_are_those_of_the_tree() {
    let root = root();
    let models = root.join("resources/models");
    let (artifact, prefix) = (models.join("context_policy_v1.json"), models.join("prefix_policy_v1.json"));
    let recipe = root.join("model/context_v3/recipe.json");
    let mut good = receipt();
    good["provenance"] = json!(ks_train::fit::provenance(&root, &recipe).unwrap());
    good["recipe_sha256"] = json!(ks_train::corpus::checksum(&recipe).unwrap());
    good["prefix_baseline_sha256"] = json!(ks_train::corpus::checksum(&root.join("model/prefix_v2/baseline-prefix-v1.json")).unwrap());
    validate_receipt(&good, &artifact, &root, &prefix, &models).unwrap();
    let refused = |words: &str, edit: &dyn Fn(&mut Value)| {
        let mut changed = good.clone();
        edit(&mut changed);
        let said = validate_receipt(&changed, &artifact, &root, &prefix, &models).unwrap_err();
        assert!(said.contains(words), "{words}: {said}");
    };
    refused("invalid object: provenance", &|value| value["provenance"] = json!(1));
    refused("missing required provenance", &|value| {
        value["provenance"].as_object_mut().unwrap().remove("Cargo.lock");
    });
    for path in ["../x", "/x", ".x"] {
        refused("nonpublic provenance", &|value| value["provenance"][path] = json!("0"));
    }
    refused("provenance mismatch", &|value| value["provenance"]["Cargo.lock"] = json!("0"));
    refused("No such file", &|value| value["provenance"]["gone"] = json!("0"));
    refused("prefix baseline mismatch", &|value| value["prefix_baseline_sha256"] = json!("0".repeat(64)));
    refused("recipe or gate policy", &|value| value["recipe_sha256"] = json!("0".repeat(64)));
    // The baseline named in the provenance answers for itself.
    let mut named = good.clone();
    named["provenance"]["model/prefix_v2/baseline-prefix-v1.json"] = named["prefix_baseline_sha256"].clone();
    validate_receipt(&named, &artifact, &root, &prefix, &models).unwrap();
}

/// The floor of a policy that asks for one: the recipe of the shipped pair asks for none.
#[test]
fn a_recall_below_the_floor_of_the_policy_is_refused() {
    let mut recipe = read_json(&root().join("model/context_v3/recipe.json")).unwrap();
    recipe["gate_policy"]["minimum_calibration_conversion_recall"] = json!(1.0);
    let tree = support::small_tree("floor", &recipe);
    let mut changed = receipt();
    changed["gate_policy"] = recipe["gate_policy"].clone();
    let models = tree.join("resources/models");
    let said = validate_evidence(&changed, &models.join("context_policy_v1.json"), &tree, &models.join("prefix_policy_v1.json"), &models).unwrap_err();
    assert!(said.contains("calibration gate failed"), "{said}");
}

/// A digest that names the version a model carries and still is not the digest of its weights.
fn other_tail(digest: &Value) -> Value {
    let text = digest.as_str().unwrap();
    let (head, last) = text.split_at(text.len() - 1);
    json!(format!("{head}{}", if last == "0" { "1" } else { "0" }))
}

#[test]
fn a_field_renamed_or_a_digest_that_only_begins_the_same_is_refused() {
    refuses("missing or extra fields: receipt", |value| {
        let table = value.as_object_mut().unwrap();
        let scope = table.remove("scope").unwrap();
        table.insert("scope_of_it".to_owned(), scope);
    });
    refuses("context weights, version or threshold", |value| value["weights_sha256"] = other_tail(&value["weights_sha256"]));
    refuses("prefix weights, version or threshold", |value| value["prefix_weights_sha256"] = other_tail(&value["prefix_weights_sha256"]));
}

/// `validate_receipt` over a tree whose recipe is of another schema, feature version or set of profiles.
#[test]
fn a_recipe_of_another_kind_than_the_receipt_knows_is_refused() {
    for (field, value) in [("schema_version", json!(2)), ("feature_version", json!(4)), ("profiles", json!(["portable"]))] {
        let mut recipe = read_json(&root().join("model/context_v3/recipe.json")).unwrap();
        recipe[field] = value;
        let tree = support::small_tree("recipe-kind", &recipe);
        let models = tree.join("resources/models");
        let recipe_path = tree.join("model/context_v3/recipe.json");
        let mut changed = receipt();
        changed["provenance"] = json!(ks_train::fit::provenance(&tree, &recipe_path).unwrap());
        changed["recipe_sha256"] = json!(ks_train::corpus::checksum(&recipe_path).unwrap());
        changed["prefix_baseline_sha256"] = json!(ks_train::corpus::checksum(&tree.join("model/prefix_v2/baseline-prefix-v1.json")).unwrap());
        let said = validate_receipt(&changed, &models.join("context_policy_v1.json"), &tree, &models.join("prefix_policy_v1.json"), &models).unwrap_err();
        assert!(said.contains("recipe or gate policy"), "{field}: {said}");
    }
}

/// The installed pair's receipt as protocol 5 states one: the same numbers under the protocol and scope of version 5, a
/// word without first plans, and first plans whose counts, earliness and gates hold together. `restored` gives the rows
/// the baseline, candidate and installed pairs restore of their wrong first plans, each over ten.
fn version_five() -> Value {
    let protocols = read_json(&root().join(PROTOCOLS)).unwrap();
    let set = protocol_set(&protocols, SEQUENCE_PROTOCOL_VERSION).unwrap();
    let mut five = receipt();
    five["protocol"] = set["receipt"].clone();
    five["scope"] = set["scope"].clone();
    let mut test = test_of(&five);
    let words = test.documents.us + test.documents.ru - 1;
    test.first_ineligible_rows = Some(1);
    test.installed_reference = Some(InstalledReference { context_sha256: "a".repeat(64), prefix_sha256: "b".repeat(64) });
    let counts = |restored: i64, spoiled: i64| PairCounts {
        rows: 2 * words,
        initially_correct: words,
        initially_wrong: words,
        preserved_correct: words - spoiled,
        exactly_restored: restored,
        correct_text_corruptions: spoiled,
        injections: words,
        ..PairCounts::default()
    };
    // Every restored row right at the end: ten at its last letter or the key after it, the rest at the fourth key.
    let early = |restored: i64| {
        let mut early = Earliness {
            inside_word: restored - 10,
            last_letter: 5,
            boundary_key: 5,
            never: words - restored,
            first_word_correct: restored,
            switch_key_rows: restored,
            false_early_sequence: 1,
            switch_key_sum: 4 * (restored - 10) + 6 * 5 + 7 * 5,
            ..Earliness::default()
        };
        for (bin, rows) in [("4", restored - 10), ("6", 5), ("7", 5)] {
            early.switch_key_histogram.insert(bin.to_owned(), rows);
        }
        early.first_correction_kinds = [("early", restored - 10), ("pause", 5), ("boundary", 5)].into_iter().map(|(kind, rows)| (kind.to_owned(), rows)).collect();
        early
    };
    for (_, profile) in test.profiles.each_mut() {
        profile.first = Some(PublicFirst {
            counts: ByPair { baseline: counts(690, 1), candidate: counts(700, 0), installed: counts(695, 1) },
            earliness: ByPair { baseline: early(690), candidate: early(700), installed: early(695) },
            gates: FirstGates { net_restorations_at_least_baseline: true, correct_text_preserved: true, length_preserved: true },
        });
    }
    five["test"] = tree_of(&test);
    five
}

fn refuses_five(words: &str, edit: impl FnOnce(&mut Value)) {
    let mut changed = version_five();
    edit(&mut changed);
    let said = check(&changed).expect_err(words);
    assert!(said.contains(words), "{words}: {said}");
}

/// The same, with the test block changed as its type.
fn refuses_five_typed(words: &str, edit: impl FnOnce(&mut PublicTest)) {
    refuses_five(words, |value| {
        let mut test = test_of(value);
        edit(&mut test);
        value["test"] = tree_of(&test);
    });
}

/// The first plans of the portable profile of a receipt's test, as a tree and as their type.
fn first_tree(value: &mut Value) -> &mut Value {
    &mut value["test"]["profiles"]["portable"]["first"]
}

fn early_tree(value: &mut Value) -> &mut Value {
    &mut first_tree(value)["earliness"]["candidate"]
}

fn first_of(test: &mut PublicTest) -> &mut PublicFirst {
    test.profiles.portable.first.as_mut().unwrap()
}

/// A receipt is checked by the protocol it names: the installed pair's of version 4 as before, one of version 5 with its
/// first plans; neither takes the other's fields.
#[test]
fn a_receipt_is_checked_by_the_version_of_its_protocol() {
    check(&receipt()).unwrap();
    check(&version_five()).unwrap();
    refuses("unsupported receipt identity", |value| value["protocol"]["version"] = json!(6));
    refuses_five("unsupported receipt identity", |value| value["scope"] = receipt()["scope"].clone());
    refuses_five("unsupported receipt identity", |value| value["protocol"]["first_gates"] = json!("other"));
    let mut with_rows = test_of(&receipt());
    with_rows.first_ineligible_rows = Some(0);
    refuses("missing or extra fields: test", |value| value["test"] = tree_of(&with_rows));
    refuses_five_typed("missing or extra fields: test", |test| test.installed_reference = None);
    refuses_five_typed("missing or extra fields: test profile", |test| test.profiles.portable.first = None);
    refuses_five("unknown field `cases`", |value| value["test"]["profiles"]["portable"]["cases"] = json!([]));
    refuses_five("unknown field `extra`", |value| value["test"]["installed_reference"]["extra"] = json!(1));
    refuses_five_typed("invalid SHA256", |test| test.installed_reference.as_mut().unwrap().prefix_sha256 = "0".to_owned());
    refuses_five_typed("invalid count", |test| test.first_ineligible_rows = Some(-1));
    refuses_five_typed("inconsistent sequence selection", |test| test.first_ineligible_rows = Some(100_000));
    // One word fewer than the first plans count.
    refuses_five_typed("inconsistent sequence counts", |test| test.first_ineligible_rows = Some(2));
}

#[test]
fn the_first_plans_of_a_receipt_hold_together() {
    refuses_five("unknown field `cases`", |value| first_tree(value)["cases"] = json!({}));
    refuses_five("missing field `installed`", |value| {
        first_tree(value)["counts"].as_object_mut().unwrap().remove("installed");
    });
    refuses_five("unknown field `ablation`", |value| first_tree(value)["earliness"]["ablation"] = json!({}));
    refuses_five_typed("inconsistent sequence counts", |test| test.profiles.reference_hunspell.first.as_mut().unwrap().counts.installed.rows += 1);
    // Gates claimed that the counts do not give: more corruptions than the baseline, fewer restored, a length changed.
    const GATES: &str = "first-plan gates failed";
    refuses_five_typed(GATES, |test| {
        let counts = &mut first_of(test).counts.candidate;
        counts.correct_text_corruptions += 2;
        counts.preserved_correct -= 2;
    });
    refuses_five_typed(GATES, |test| first_of(test).counts.candidate.length_mismatches = 1);
    refuses_five_typed(GATES, |test| first_of(test).gates.length_preserved = false);
    refuses_five_typed(GATES, |test| {
        // The baseline restores more than the candidate: its earliness moves with it.
        let first = first_of(test);
        first.counts.baseline.exactly_restored = 702;
        let early = &mut first.earliness.baseline;
        early.inside_word += 12;
        early.never -= 12;
        early.first_word_correct += 12;
        early.switch_key_rows += 12;
        early.switch_key_sum += 48;
        *early.switch_key_histogram.get_mut("4").unwrap() += 12;
        *early.first_correction_kinds.get_mut("early").unwrap() += 12;
    });
}

#[test]
fn the_earliness_of_a_receipt_holds_together() {
    const EARLY: &str = "inconsistent first-plan earliness";
    let with = |words: &str, edit: &dyn Fn(&mut Earliness)| refuses_five_typed(words, |test| edit(&mut first_of(test).earliness.candidate));
    refuses_five("unknown field `extra`", |value| early_tree(value)["extra"] = json!(1));
    refuses_five("missing field `never`", |value| {
        early_tree(value).as_object_mut().unwrap().remove("never");
    });
    refuses_five("invalid type", |value| early_tree(value)["never"] = json!("many"));
    with(EARLY, &|early| early.never += 1);
    with(EARLY, &|early| early.false_early_first = -1);
    with(EARLY, &|early| {
        early.switch_key_histogram.remove("9+");
    });
    with(EARLY, &|early| {
        early.first_correction_kinds.insert(String::new(), 0);
    });
    with(EARLY, &|early| *early.first_correction_kinds.get_mut("early").unwrap() += 1);
    with(EARLY, &|early| early.first_word_correct += 100_000);
    // The rows restored exactly have their first word right.
    with(EARLY, &|early| {
        early.first_word_correct -= 1;
        early.switch_key_rows -= 1;
        *early.switch_key_histogram.get_mut("4").unwrap() -= 1;
        early.switch_key_sum -= 4;
    });
    with(EARLY, &|early| early.started_right_ended_wrong = 100_000);
    with(EARLY, &|early| *early.switch_key_histogram.get_mut("5").unwrap() += 1);
    with(EARLY, &|early| early.switch_key_sum += 1);
    with(EARLY, &|early| early.switch_key_sum -= 1);
    with(EARLY, &|early| early.false_early_first = 100_000);
    with(EARLY, &|early| early.false_early_sequence = 100_000);
    // Rows past the eighth key let the sum be larger than the bins alone give, never smaller.
    with(EARLY, &|early| {
        *early.switch_key_histogram.get_mut("7").unwrap() -= 1;
        *early.switch_key_histogram.get_mut("9+").unwrap() += 1;
        early.switch_key_sum -= 10;
    });
    let mut later = version_five();
    let mut test = test_of(&later);
    let early = &mut first_of(&mut test).earliness.candidate;
    *early.switch_key_histogram.get_mut("7").unwrap() -= 1;
    *early.switch_key_histogram.get_mut("9+").unwrap() += 1;
    early.switch_key_sum += 20;
    later["test"] = tree_of(&test);
    check(&later).unwrap();
}
