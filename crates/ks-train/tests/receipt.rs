//! The receipt of the installed pair passes the checks, and a receipt whose numbers were changed, whose fields are gone or
//! that names other files does not.
#![expect(clippy::unwrap_used, reason = "a test or a probe stops on what it did not expect")]
#![expect(clippy::disallowed_types, clippy::disallowed_macros, clippy::disallowed_methods, reason = "temporary: these tests still build and read their JSON as untyped trees; step 7 of the strict-json plan types them and removes this")]
mod support;

use ks_train::corpus::read_json;
use ks_train::receipt::{validate_evidence, validate_receipt};
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

#[test]
fn the_selection_of_a_test_must_add_up() {
    const SELECTION: &str = "inconsistent sequence selection";
    refuses("missing or extra fields", |value| value["test"]["extra"] = json!(1));
    refuses("invalid count", |value| value["test"]["documents"]["0"] = json!(-1));
    refuses(SELECTION, |value| value["test"]["documents"]["0"] = json!(513));
    refuses(SELECTION, |value| value["test"]["selected_rows"] = json!(1537));
    refuses(SELECTION, |value| plus(&mut value["test"]["selected_rows"], 1));
    refuses("invalid count", |value| value["test"]["selected_rows"] = json!(-1));
    refuses("invalid count", |value| value["test"]["unsupported_rows"] = json!(-1));
    refuses("invalid count", |value| value["test"]["trimmed_rows"] = json!(-1));
    refuses(SELECTION, |value| {
        let selected = value["test"]["selected_rows"].clone();
        value["test"]["trimmed_rows"] = json!(selected.as_i64().unwrap() + 1);
    });
    refuses("missing or extra fields", |value| value["test"]["profiles"] = json!({}));
}

#[test]
fn the_counts_of_a_replay_must_add_up() {
    const COUNTS: &str = "inconsistent sequence counts";
    for (profile, early) in [("portable", false), ("reference_hunspell", false), ("portable", true), ("reference_hunspell", true)] {
        for model in ["candidate", "baseline"] {
            let with = |edit: &dyn Fn(&mut Value), words: &str| {
                refuses(words, |value| {
                    let block = &mut value["test"]["profiles"][profile];
                    let block = if early { &mut block["early_off"] } else { block };
                    edit(&mut block["counts"][model]);
                });
            };
            with(&|counts| counts["extra"] = json!(1), "missing or extra fields");
            with(&|counts| counts["rows"] = json!(-1), "invalid count");
            with(&|counts| plus(&mut counts["rows"], 1), COUNTS);
            with(&|counts| plus(&mut counts["initially_correct"], 1), COUNTS);
            with(&|counts| plus(&mut counts["initially_wrong"], 1), COUNTS);
            with(&|counts| counts["exactly_restored"] = json!(100_000), COUNTS);
            with(&|counts| {
                let injections = counts["injections"].as_i64().unwrap();
                counts["exactly_restored"] = json!(injections + 1);
            }, COUNTS);
            with(&|counts| plus(&mut counts["preserved_correct"], 1), COUNTS);
            for name in ["length_mismatches", "execution_errors", "final_layout_mismatches"] {
                with(&|counts| counts[name] = json!(100_000), COUNTS);
            }
            with(&|counts| {
                let injections = counts["injections"].as_i64().unwrap();
                counts["correction_layout_mismatches"] = json!(injections + 1);
            }, COUNTS);
        }
        // Counts that hold together, and that no gate lets through.
        let gated = |edit: &dyn Fn(&mut Value)| {
            refuses("sequence gates failed", |value| {
                let block = &mut value["test"]["profiles"][profile];
                let block = if early { &mut block["early_off"] } else { block };
                edit(block);
            });
        };
        gated(&|block| {
            let theirs = block["counts"]["baseline"]["correct_text_corruptions"].as_i64().unwrap();
            let counts = &mut block["counts"]["candidate"];
            let more = theirs + 1 - counts["correct_text_corruptions"].as_i64().unwrap();
            plus(&mut counts["correct_text_corruptions"], more);
            plus(&mut counts["preserved_correct"], -more);
        });
        gated(&|block| block["counts"]["candidate"]["length_mismatches"] = json!(1));
        gated(&|block| {
            let (restored, spoiled) = (block["counts"]["baseline"]["exactly_restored"].as_i64().unwrap(), block["counts"]["baseline"]["correct_text_corruptions"].as_i64().unwrap());
            let counts = &mut block["counts"]["candidate"];
            let own = counts["correct_text_corruptions"].as_i64().unwrap();
            counts["exactly_restored"] = json!(restored - spoiled + own - 1);
        });
        gated(&|block| block["counts"]["candidate"]["execution_errors"] = json!(1));
        gated(&|block| block["counts"]["baseline"]["execution_errors"] = json!(1));
        gated(&|block| block["counts"]["candidate"]["correction_layout_mismatches"] = json!(1));
        gated(&|block| block["gates"]["length_preserved"] = json!(false));
    }
}

/// A gate about how much was read: fewer documents than a test needs, with everything else in step.
#[test]
fn a_test_of_too_few_documents_passes_no_gate() {
    for group in ["0", "1"] {
        refuses("sequence gates failed", |value| {
            let test = &mut value["test"];
            let held = test["documents"][group].as_i64().unwrap();
            let gone = held - 31;
            test["documents"][group] = json!(31);
            plus(&mut test["selected_rows"], -gone);
            for profile in ["portable", "reference_hunspell"] {
                for early in [false, true] {
                    let block = &mut test["profiles"][profile];
                    let block = if early { &mut block["early_off"] } else { block };
                    for model in ["candidate", "baseline"] {
                        let counts = &mut block["counts"][model];
                        plus(&mut counts["rows"], -2 * gone);
                        plus(&mut counts["initially_correct"], -gone);
                        plus(&mut counts["initially_wrong"], -gone);
                        plus(&mut counts["preserved_correct"], -gone);
                        counts["exactly_restored"] = json!(0);
                        counts["final_layout_mismatches"] = json!(0);
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
