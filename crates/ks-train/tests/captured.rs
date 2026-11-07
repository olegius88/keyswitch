//! Questions captured from a typing run as a curriculum: the manifest and its pins, each column, each kind of row a file holds.
mod support;

use ks_context::context_model::{AfterOrigin, BeforeOrigin, ContextAction};
use ks_lang::Resources;
use ks_train::captured::{after_origin_of, captured_head_curriculum, captured_rows, captured_source_curriculum, captured_sources, group_of_word, role_of};
use ks_train::heads::own_curriculum;
use ks_train::tools::{Tools, reference_models};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use support::{digest, recipe, scratch, tree};

static TOOLS: LazyLock<Tools> = LazyLock::new(|| {
    let root = tree();
    let resources: Arc<Resources> = Resources::new(root.join("resources"));
    let portable = reference_models(&root, &resources, false).expect("the lexicons");
    Tools::new(&root, resources, portable, BTreeSet::new()).expect("the tools")
});

const COLUMNS: [&str; 21] = ["count", "original", "alternative", "source_group", "trigger", "baseline_convert", "source_known", "target_known", "score_delta", "literal_tail", "boundary_text", "after_origin", "source_typo", "target_typo", "source_opening", "target_opening", "inside", "before", "after", "role", "label"];

fn columns(schema: i64) -> Vec<&'static str> {
    let mut names = COLUMNS.to_vec();
    if schema == 2 {
        let place = names.iter().position(|name| *name == "after_origin").expect("the column") + 1;
        names.insert(place, "before_origin");
    }
    names
}

/// A question with these values, the rest of it plain, as a captured row of a schema.
fn question(schema: i64, original: &str, alternative: &str, group: i64, changes: &[(&str, Value)]) -> Value {
    let mut values: Map<String, Value> = Map::new();
    for (name, value) in [("count", json!(1)), ("original", json!(original)), ("alternative", json!(alternative)), ("source_group", json!(group)), ("trigger", json!("space")), ("baseline_convert", json!(false)), ("source_known", json!(false)), ("target_known", json!(false)), ("score_delta", json!(0.5)), ("literal_tail", json!("")), ("boundary_text", json!(" ")), ("after_origin", json!("none")), ("before_origin", json!("field")), ("source_typo", json!(false)), ("target_typo", json!(false)), ("source_opening", json!(false)), ("target_opening", json!(false)), ("inside", json!(false)), ("before", json!("это дом ")), ("after", json!("")), ("role", json!("text")), ("label", json!("keep"))] {
        values.insert(name.to_owned(), value);
    }
    for (name, value) in changes {
        values.insert((*name).to_owned(), value.clone());
    }
    Value::Array(columns(schema).iter().map(|name| values[*name].clone()).collect())
}

fn packed(rows: &[Value]) -> Vec<u8> {
    let mut encoder = xz2::write::XzEncoder::new(Vec::new(), 1);
    for row in rows {
        encoder.write_all(row.to_string().as_bytes()).expect("in memory");
        encoder.write_all(b"\n").expect("in memory");
    }
    encoder.finish().expect("in memory")
}

/// A folder with a manifest of one question file, `name`.
fn manifest_of(name: &str, schema: i64, rows: &[Value]) -> PathBuf {
    let folder = scratch("captured");
    let file = packed(rows);
    std::fs::write(folder.join(name), &file).expect("a file");
    let manifest = folder.join("manifest.json");
    std::fs::write(&manifest, json!({"schema_version": schema, "columns": columns(schema), "sources": [{"file": name, "sha256": digest(&file), "weight": 2.0}]}).to_string()).expect("a manifest");
    manifest
}

fn edited(manifest: &Path, change: &dyn Fn(&mut Value)) -> Result<(), String> {
    let good = std::fs::read(manifest).expect("a manifest");
    let mut value: Value = serde_json::from_slice(&good).expect("json");
    change(&mut value);
    std::fs::write(manifest, value.to_string()).expect("a manifest");
    let answer = captured_sources(manifest).map(|_| ());
    std::fs::write(manifest, good).expect("a manifest");
    answer
}

#[test]
fn a_manifest_names_its_columns_and_pins_its_files() {
    let manifest = manifest_of("one.jsonl.xz", 1, &[question(1, "дом", "ljv", 1, &[])]);
    assert!(edited(&manifest, &|_| ()).is_ok());
    let refused: [(&dyn Fn(&mut Value), &str); 5] = [
        (&|value: &mut Value| value["schema_version"] = json!(3), "invalid captured question manifest"),
        (&|value: &mut Value| value["columns"] = json!(["count"]), "invalid captured question manifest"),
        (&|value: &mut Value| value["sources"][0]["sha256"] = json!("0"), "captured questions changed"),
        (&|value: &mut Value| value["sources"][0]["weight"] = json!(0.0), "invalid weight or balance"),
        (&|value: &mut Value| value["sources"][0]["weight"] = json!(-1.0), "invalid weight or balance"),
    ];
    for (change, words) in refused {
        let error = edited(&manifest, change).expect_err(words);
        assert!(error.contains(words), "{error}");
    }
    let sources = captured_sources(&manifest).expect("the sources");
    assert!(sources[0].balanced);
    let wrong = manifest_of("wrong.jsonl.xz", 1, &[json!([1, "дом"])]);
    let error = captured_rows(&captured_sources(&wrong).expect("the sources")[0]).expect_err("other columns");
    assert!(error.contains("other columns"), "{error}");
}

#[test]
fn each_value_a_column_can_hold_is_read() {
    let mut rows = Vec::new();
    for role in ["text", "code", "search", "terminal", "password", "unknown"] {
        rows.push(question(2, "дом", "ljv", 1, &[("role", json!(role))]));
    }
    for (after, before) in [("none", "field"), ("field", "planned_previous_reading"), ("planned_next_conversion", "field"), ("kept_next_word", "field")] {
        rows.push(question(2, "дом", "ljv", 1, &[("after_origin", json!(after)), ("before_origin", json!(before))]));
    }
    let manifest = manifest_of("all.jsonl.xz", 2, &rows);
    let read = captured_rows(&captured_sources(&manifest).expect("the sources")[0]).expect("the rows");
    assert_eq!(read.len(), rows.len());
    assert!(read.iter().any(|(item, _)| item.before_origin == BeforeOrigin::PlannedPreviousReading));
    assert!(read.iter().any(|(item, _)| item.after_origin == AfterOrigin::KeptNextWord));
    assert_eq!(group_of_word("дом"), 1);
    assert_eq!(group_of_word("word"), 0);
}

#[test]
#[should_panic(expected = "no field role")]
fn a_role_no_field_has_stops_the_fit() {
    let _ = role_of("nothing");
}

#[test]
#[should_panic(expected = "no origin")]
fn an_origin_no_word_has_stops_the_fit() {
    let _ = after_origin_of("nothing");
}

#[test]
#[should_panic(expected = "no origin")]
fn an_origin_no_left_text_has_stops_the_fit() {
    let manifest = manifest_of("bad.jsonl.xz", 2, &[question(2, "дом", "ljv", 1, &[("before_origin", json!("nothing"))])]);
    let _ = captured_rows(&captured_sources(&manifest).expect("the sources")[0]);
}

fn options() -> Value {
    json!({"maximum_rows_per_source": 100, "convert_share": null, "keep_unknown_share": 0.5, "unknown_keep_groups": [1], "weight_scale": 1.0, "minimum_word_characters": 2, "excluded_file_markers": ["left-out"]})
}

#[test]
fn a_question_file_gives_a_frame_of_each_kind() {
    let plain = |original: &str, alternative: &str, group: i64, label: &str, changes: &[(&str, Value)]| {
        let mut all = vec![("label", json!(label))];
        all.extend(changes.iter().cloned());
        question(1, original, alternative, group, &all)
    };
    let rows = vec![
        plain("дом", "ljv", 1, "keep", &[]),
        plain("ljv", "дом", 0, "convert", &[]),
        plain("дом", "ljv", 1, "wait", &[("before", json!("ещё "))]),
        plain("я", "z", 1, "keep", &[]),
        plain("ñn", "ñn", 0, "keep", &[]),
        plain("цуиоы", "webjs", 1, "keep", &[("before", json!("там "))]),
        plain("word", "цщкв", 0, "keep", &[("before", json!("там "))]),
        plain("щт", "on", 1, "convert", &[("after_origin", json!("planned_next_conversion")), ("after", json!(" дом"))]),
        plain("ghbdtn", "привет", 0, "convert", &[("after_origin", json!("planned_next_conversion")), ("after", json!(" дом"))]),
        plain("ещё", "tot", 1, "convert", &[("after_origin", json!("planned_next_conversion")), ("after", json!(" "))]),
        plain("on", "щт", 0, "keep", &[("before", json!("")), ("after", json!(""))]),
        plain("щт", "on", 1, "convert", &[("before", json!("")), ("after", json!("")), ("trigger", json!("enter")), ("boundary_text", json!("\n"))]),
        plain("щт", "on", 1, "convert", &[("before", json!("дом ")), ("after", json!(""))]),
        plain("щт", "on", 1, "convert", &[("before", json!("")), ("after", json!("дом"))]),
        plain("ф", "a", 1, "convert", &[("before", json!("")), ("after", json!(""))]),
        plain("window", "цштвщц", 0, "keep", &[("before", json!("")), ("after", json!(""))]),
    ];
    let manifest = manifest_of("every-kind.jsonl.xz", 1, &rows);
    let source = &captured_sources(&manifest).expect("the sources")[0];
    let (frames, report) = captured_source_curriculum(&TOOLS, source, &options()).expect("the frames");
    assert!(!frames.is_empty(), "{report}");
    assert!(report["skipped_short"].as_i64() == Some(2), "{report}");
    assert!(frames.iter().any(|frame| frame.action == ContextAction::Wait), "{report}");
    assert!(frames.iter().any(|frame| frame.action == ContextAction::Suggest), "{report}");
    assert!(report["chosen"]["skipped_planned_without_word"].as_i64() == Some(1), "{report}");
    let mut shared = options();
    shared["convert_share"] = json!(0.25);
    shared["maximum_rows_per_source"] = json!(8);
    assert!(captured_source_curriculum(&TOOLS, source, &shared).expect("the frames").0.len() <= 8);
    let left_out = manifest_of("a-left-out-file.jsonl.xz", 1, &rows);
    let (none, said) = captured_source_curriculum(&TOOLS, &captured_sources(&left_out).expect("the sources")[0], &options()).expect("the frames");
    assert!(none.is_empty() && said["excluded"] == json!(true), "{said}");
}

#[test]
fn a_head_takes_the_questions_of_its_class_from_its_files() {
    let rows = vec![
        question(1, "дом", "ljv", 1, &[]),
        question(1, "ljv", "дом", 0, &[("label", json!("convert"))]),
        question(1, "ljv", "дом", 0, &[("label", json!("wait"))]),
        question(1, "щт", "on", 1, &[("label", json!("convert")), ("after_origin", json!("planned_next_conversion")), ("after", json!(" дом"))]),
        question(1, "ghbdtn", "привет", 0, &[("label", json!("convert")), ("after_origin", json!("planned_next_conversion")), ("after", json!(" дом"))]),
        question(1, "щт", "on", 1, &[("label", json!("convert")), ("after_origin", json!("planned_next_conversion")), ("after", json!(" "))]),
        question(1, "щт", "on", 1, &[("after_origin", json!("kept_next_word")), ("after", json!(" дом"))]),
        question(1, "щт", "on", 1, &[("after_origin", json!("kept_next_word")), ("after", json!(" дом")), ("trigger", json!("enter")), ("boundary_text", json!("\n"))]),
    ];
    let manifest = manifest_of("head.jsonl.xz", 1, &rows);
    let wanted = json!({"captured": {"train": [[manifest, "head.jsonl.xz"]]}, "sample_weight": 1.0, "convert_weight": 2.0, "token_cap": 0});
    let (frames, report) = captured_head_curriculum(&TOOLS, "train", &wanted, &|_| true, "test").expect("the frames");
    assert!(!frames.is_empty(), "{report}");
    let (none, _) = captured_head_curriculum(&TOOLS, "train", &wanted, &|_| false, "test").expect("the frames");
    assert!(none.is_empty());
    let (unused, said) = captured_head_curriculum(&TOOLS, "development", &wanted, &|_| true, "test").expect("the frames");
    assert!(unused.is_empty() && said["scope"] == json!("not used"), "{said}");
    let (capped, _) = captured_head_curriculum(&TOOLS, "train", &{ let mut value = wanted.clone(); value["token_cap"] = json!(1); value }, &|_| true, "test").expect("the frames");
    assert!(capped.len() <= frames.len());
    let missing = captured_head_curriculum(&TOOLS, "train", &json!({"captured": {"train": [[manifest, "nothing.jsonl.xz"]]}, "sample_weight": 1.0}), &|_| true, "test").expect_err("no such file");
    assert!(missing.contains("no captured file"), "{missing}");
}

#[test]
fn the_heads_on_captured_questions_leave_out_what_they_were_told_to() {
    // A hypothesis, a short Russian word, a slip: asked with the left text given, or planned.
    let rows = vec![
        question(2, "да", "lf", 1, &[("before", json!("это очень важная вещь для всех "))]),
        question(2, "да", "lf", 1, &[("before", json!("это очень важная вещь для всех ")), ("before_origin", json!("planned_previous_reading"))]),
        question(2, "вы", "ds", 1, &[("before", json!("это очень важная вещь для всех ")), ("before_origin", json!("planned_previous_reading")), ("baseline_convert", json!(true))]),
        question(2, "ljv", "дом", 0, &[("before_origin", json!("planned_previous_reading"))]),
    ];
    let manifest = manifest_of("planned.jsonl.xz", 2, &rows);
    let mut wanted = recipe();
    for head in ["hypothesis", "short", "slip"] {
        let options = json!({"captured": {"train": [[manifest, "planned.jsonl.xz"]]}, "sample_weight": 1.0, "held_out": ["да"]});
        let name = format!("{head}_curriculum");
        wanted[name.as_str()] = options.clone();
        let (frames, report) = own_curriculum(&TOOLS, head, "train", &[], &wanted).expect("the frames");
        assert!(report.is_object());
        assert!(frames.iter().all(|frame| frame.original != "да"), "{head}: {report}");
    }
}

#[test]
fn a_curated_letter_alone_in_a_field_keeps_its_answer() {
    let alone = [("before", json!("")), ("after", json!(""))];
    let rows = vec![question(1, "я", "z", 1, &alone), question(1, "z", "я", 0, &[("before", json!("")), ("after", json!("")), ("label", json!("convert"))])];
    let manifest = manifest_of("letters.jsonl.xz", 1, &rows);
    let mut wanted = options();
    wanted["minimum_word_characters"] = json!(1);
    let (frames, report) = captured_source_curriculum(&TOOLS, &captured_sources(&manifest).expect("the sources")[0], &wanted).expect("the frames");
    assert!(frames.iter().any(|frame| frame.original == "я" && frame.action == ContextAction::Keep), "{report}");
    assert!(frames.iter().any(|frame| frame.original == "z" && frame.action == ContextAction::Convert), "{report}");
}
