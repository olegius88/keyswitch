//! Parity of the context layer with the Python, against golden files dumped by `gen_golden_context.py`.
//!
//! The files live in `$KS_GOLDEN_DIR/context` (default: the `golden` folder of the private work tree), the resources in
//! `$KS_RESOURCES`; a test skips with a message when they are absent. Floats are compared by their bits. In a debug build
//! only every 10th row of the big files is compared.
#![expect(clippy::disallowed_types, clippy::disallowed_macros, clippy::disallowed_methods, reason = "temporary: these tests still build and read their JSON as untyped trees; step 7 of the strict-json plan types them and removes this")]

use ks_cast::To as _;
use flate2::read::GzDecoder;
use ks_backend::{FieldContext, FieldReader, FieldRole, KeyEvent};
use ks_context::context_action_features::*;
use ks_context::context_model::{AfterOrigin, BeforeOrigin, ContextEvidence, ContextModel, ContextPrediction, TermTables};
use ks_context::context_policy::{caret_lag, ends_with_typed, quoted_or_scoped, russian_slang, shown_ahead, typed_short_of, word_shaped};
use ks_context::context_policy::{ContextPolicy, ContextResult, DecideOptions};
use ks_context::input_context::{Clock, InputContext};
use ks_context::prefix_model::{PrefixInput, PrefixSide};
use ks_context::prefix_schema::{features_for_version, VersionedPrefixModel};
use ks_context::typo_correction::{
    corrected_keys, keyboard_neighbours, known_elsewhere, locale_keys, Edit, KeyTemplates, Operation, TypoChoice, TypoCorrector, TypoPolicy,
};
use ks_lang::detector::DetectionDecision;
use ks_lang::early_switch::PrefixIndex;
use ks_lang::{LanguageModel, Resources, WordScore};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const STEP: usize = if cfg!(debug_assertions) && !cfg!(coverage) { 10 } else { 1 };

fn golden_root() -> PathBuf {
    PathBuf::from(std::env::var("KS_GOLDEN_DIR").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../.t/golden").to_string()))
}

fn resource_dir() -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var("KS_RESOURCES").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources").to_string()));
    if path.join("models/context_policy_v1.json").exists() {
        Some(path)
    } else {
        eprintln!("SKIPPED: no resources at {}", path.display());
        None
    }
}

/// The full answers where the private folder is there, else those on public words kept in the repository.
fn golden_file(name: &str) -> Option<PathBuf> {
    let path = golden_root().join("context").join(name);
    let kept = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/context")).join(name);
    let found = [&path, &kept].into_iter().find(|candidate| candidate.exists()).cloned();
    if found.is_none() {
        eprintln!("SKIPPED: golden file {} is absent", path.display());
    }
    found
}

fn rows(name: &str) -> Option<Vec<Value>> {
    let path = golden_file(name)?;
    let reader = BufReader::new(GzDecoder::new(std::fs::File::open(path).expect("open")));
    Some(reader.lines().map(|line| serde_json::from_str(&line.expect("line")).expect("json")).collect())
}

#[derive(Default)]
struct Failures {
    count: usize,
    total: usize,
    shown: Vec<String>,
}

impl Failures {
    fn check(&mut self, what: &str, row: usize, found: &Value, wanted: &Value) {
        self.total += 1;
        if found != wanted {
            self.count += 1;
            if self.shown.len() < 6 {
                let cut = |value: &Value| value.to_string().chars().take(300).collect::<String>();
                self.shown.push(format!("{what} row {row}:\n  found  {}\n  wanted {}", cut(found), cut(wanted)));
            }
        }
    }

    fn finish(self, name: &str) {
        println!("{name}: {} checks, {} failures", self.total, self.count);
        assert!(self.count == 0, "{name}: {} of {} differ:\n{}", self.count, self.total, self.shown.join("\n"));
    }
}

fn hex(x: f64) -> Value {
    json!(format!("{:016x}", x.to_bits()))
}

fn bits(x: Option<f64>) -> Value {
    x.map_or(Value::Null, hex)
}

fn float(value: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(value.as_str().expect("hex"), 16).expect("hex digits"))
}

fn maybe_float(value: &Value) -> Option<f64> {
    (!value.is_null()).then(|| float(value))
}

fn word_score(value: &Value) -> Option<WordScore> {
    let a = value.as_array()?;
    Some(WordScore {
        value: float(&a[0]),
        known: a[1].as_bool().expect("flag"),
        frequency: a[2].as_i64().expect("int"),
        gram_ratio: float(&a[3]),
        exact: a[4].as_bool().expect("flag"),
        spell_known: a[5].as_bool().expect("flag"),
        ngram_score: float(&a[6]),
        invalid_ratio: float(&a[7]),
        raw_ngram_score: float(&a[8]),
    })
}

fn score_json(score: &WordScore) -> Value {
    json!([hex(score.value), score.known, score.frequency, hex(score.gram_ratio), score.exact, score.spell_known, hex(score.ngram_score), hex(score.invalid_ratio), hex(score.raw_ngram_score)])
}

fn field(value: &Value) -> Option<FieldContext> {
    let a = value.as_array()?;
    let text = |index: usize| a[index].as_str().expect("text").to_string();
    Some(FieldContext {
        application: text(0),
        field_id: text(1),
        before: text(2),
        after: text(3),
        role: match a[4].as_str().expect("role") {
            "text" => FieldRole::Text,
            "code" => FieldRole::Code,
            "search" => FieldRole::Search,
            "terminal" => FieldRole::Terminal,
            "password" => FieldRole::Password,
            _ => FieldRole::Unknown,
        },
        selection: a[5].as_bool().expect("flag"),
        sensitive: a[6].as_bool().expect("flag"),
        source: text(7),
    })
}

fn field_json(field: Option<&FieldContext>) -> Value {
    field.map_or(Value::Null, |f| json!([f.application, f.field_id, f.before, f.after, f.role.as_str(), f.selection, f.sensitive, f.source]))
}

fn after_origin(value: &Value) -> AfterOrigin {
    match value.as_str().expect("origin") {
        "field" => AfterOrigin::Field,
        "planned_next_conversion" => AfterOrigin::PlannedNextConversion,
        "kept_next_word" => AfterOrigin::KeptNextWord,
        _ => AfterOrigin::None,
    }
}

fn evidence(e: &Value) -> ContextEvidence {
    let text = |name: &str| e[name].as_str().expect("text").to_string();
    let flag = |name: &str| e[name].as_bool().expect("flag");
    ContextEvidence {
        original: text("original"),
        alternative: text("alternative"),
        source_group: e["source_group"].as_i64().expect("group"),
        field: field(&e["field"]).expect("field"),
        trigger: text("trigger"),
        baseline_convert: flag("baseline_convert"),
        source_known: flag("source_known"),
        target_known: flag("target_known"),
        score_delta: float(&e["score_delta"]),
        source_score: word_score(&e["source_score"]),
        target_score: word_score(&e["target_score"]),
        model_probability: maybe_float(&e["model_probability"]),
        model_threshold: maybe_float(&e["model_threshold"]),
        literal_tail: text("literal_tail"),
        ortho_score: maybe_float(&e["ortho_score"]),
        ortho_threshold: maybe_float(&e["ortho_threshold"]),
        boundary_text: text("boundary_text"),
        after_origin: after_origin(&e["after_origin"]),
        before_origin: if e["before_origin"] == "planned_previous_reading" { BeforeOrigin::PlannedPreviousReading } else { BeforeOrigin::Field },
        source_identifier: flag("source_identifier"),
        target_identifier: flag("target_identifier"),
        source_typo: flag("source_typo"),
        target_typo: flag("target_typo"),
        source_opening: flag("source_opening"),
        target_opening: flag("target_opening"),
        inside: flag("inside"),
        term_counts_dropped: flag("term_counts_dropped"),
    }
}

fn prediction_json(p: &ContextPrediction) -> Value {
    json!([p.action.as_str(), hex(p.probability), p.probabilities.iter().map(|&x| hex(x)).collect::<Vec<_>>(), p.model_version, p.supported])
}

fn decision_json(d: &DetectionDecision) -> Value {
    json!([
        d.should_convert, d.original, d.replacement, d.source_group, d.target_group, hex(d.confidence), d.reason, score_json(&d.source_score), score_json(&d.target_score),
        bits(d.model_probability), bits(d.model_threshold), d.model_version
    ])
}

fn decision(value: &Value) -> DetectionDecision {
    let a = value.as_array().expect("decision");
    DetectionDecision {
        should_convert: a[0].as_bool().expect("flag"),
        original: a[1].as_str().expect("text").to_string(),
        replacement: a[2].as_str().expect("text").to_string(),
        source_group: a[3].as_i64().expect("group"),
        target_group: a[4].as_i64().expect("group"),
        confidence: float(&a[5]),
        reason: a[6].as_str().expect("text").to_string(),
        source_score: word_score(&a[7]).expect("score"),
        target_score: word_score(&a[8]).expect("score"),
        model_probability: maybe_float(&a[9]),
        model_threshold: maybe_float(&a[10]),
        model_version: a[11].as_str().expect("text").to_string(),
    }
}

fn paths() -> Option<(PathBuf, PathBuf)> {
    let dir = resource_dir()?;
    let models = dir.join("models");
    Some((dir, models))
}

/// The public questions and helpers of the features, for an evidence, in the shape the generator wrote them.
fn questions(e: &ContextEvidence, tables: &TermTables) -> serde_json::Map<String, Value> {
    let (o, a, b, af) = (e.original.as_str(), e.alternative.as_str(), e.field.before.as_str(), e.field.after.as_str());
    let guard = |result: Result<bool, FeatureError>| result.map_or(json!("ValueError"), |flag| json!(flag));
    let mut q = serde_json::Map::new();
    let mut put = |name: &str, value: Value| {
        q.insert(name.to_string(), value);
    };
    put("letter", json!(letter_question(o, a, b)));
    put("variable", json!(variable_question(o, a, b, &e.trigger)));
    put("cyrletter", json!(cyrletter_question(o, a, b, &e.trigger)));
    put("digit_dot", json!(digit_dot_question(o, a, &e.trigger)));
    put("quote", json!(quote_question(o, a, &e.trigger)));
    put("dash", json!(dash_question(o, a, &e.trigger)));
    put("resume", json!(resume_question(o, a, b, af, e.after_origin)));
    put("alone", json!(alone_question(o, a, b, af, &e.trigger)));
    put("start", json!(start_question(o, a, b, af)));
    put("capitals", json!(capitals_question(o, b)));
    put("abbreviation", json!(abbreviation_question(o, a, b, tables)));
    put("latin_abbreviation", json!(latin_abbreviation_question(o, a, b, tables)));
    put("unknown", json!(unknown_question(o, b, tables)));
    put("joined", json!(joined_question(o)));
    put("brief", json!(brief_question(o, b, af)));
    put("known_reply", json!(known_reply_question(o, a, b, af, tables)));
    put("dotted", json!(dotted_question(o, a)));
    put("alnum", guard(alnum_question(o, a, b, tables)));
    put("hypothesis", json!(hypothesis_question(e)));
    put("slip", json!(slip_question(o, b, tables)));
    put("short", guard(short_question(o, b, tables)));
    put("first", json!(first_question(o, b, af, tables)));
    put("term", json!(term_question(o, b, tables)));
    put("sign", json!(sign_question(o, a, b, af, &e.trigger, e.after_origin)));
    put("attested", json!(attested_abbreviation(o, tables)));
    put("unknown_shape", json!(unknown_shape(o)));
    put("kept_reachable", json!(kept_context_reachable(o, &e.trigger, &e.boundary_text, af)));
    put("settled", json!(brief_settled(o, a, tables)));
    put("dominant", json!(brief_dominant(o, a, tables)));
    put("ending", json!([russian_ending_bucket(o, tables), russian_ending_bucket(a, tables)]));
    put("start_bucket", json!([russian_start_bucket(o, tables), russian_start_bucket(a, tables)]));
    let near: Vec<u64> = [o, a].iter().flat_map(|x| ["russian", "english"].map(|language| nearest_count(x, language, tables))).collect();
    put("near", json!(near));
    let buckets: Vec<&str> = [o, a].iter().flat_map(|x| ["cyrillic", "english", "latin", "russian"].map(|alphabet| ks_context::context_model::term_bucket(x, alphabet, tables))).collect();
    put("buckets", json!(buckets));
    put("slang", json!(russian_slang(o, a, e.source_group, e.source_known, tables)));
    put("word_shaped", json!([word_shaped(o), word_shaped(a)]));
    put("scoped", if a.is_empty() { Value::Null } else { json!(quoted_or_scoped(a, o)) });
    put("ends", json!(ends_with_typed(b, o)));
    put("lag", json!(caret_lag(b, af, o, false)));
    put("lag_exact", json!(caret_lag(b, af, o, true)));
    put("ahead", json!(shown_ahead(b, o, &e.boundary_text)));
    put("short_of", json!(typed_short_of(b, af, o)));
    put("joined_parts", json!([joined_parts(o), joined_parts(a)]));
    put("leading", json!([letters_and_digits(o), letters_and_digits(a)]));
    q
}

#[test]
fn features_predictions_and_questions() {
    let Some((_, models)) = paths() else { return };
    let model = ContextModel::load(&models).expect("the context model");
    let mut failures = Failures::default();
    // The words of the batteries as the engine asked about them, and evidence made for the rare heads.
    let all: Vec<Value> = ["predict.jsonl.gz", "predict_synthetic.jsonl.gz", "predict_decisions.jsonl.gz"].iter().filter_map(|name| rows(name)).flatten().collect();
    for (index, row) in all.iter().enumerate().step_by(STEP) {
        let item = evidence(&row["e"]);
        let found = match extract_action_features(&item, &model.tables) {
            Ok(features) => features.iter().map(|(name, &value)| json!([name, hex(value)])).collect::<Vec<_>>().into(),
            Err(_) => json!("ValueError"),
        };
        failures.check("features", index, &found, &row["f"]);
        failures.check("prediction", index, &prediction_json(&model.predict(&item)), &row["p"]);
        let q = questions(&item, &model.tables);
        for (name, wanted) in row["q"].as_object().expect("questions") {
            failures.check(&format!("question {name}"), index, q.get(name).unwrap_or(&Value::Null), wanted);
        }
    }
    failures.finish("features");
}

#[derive(Default)]
struct Script {
    values: Mutex<VecDeque<f64>>,
}

impl Script {
    fn clock(self: &Arc<Self>) -> Clock {
        let script = self.clone();
        Arc::new(move || script.values.lock().expect("lock").pop_front().expect("the clock was read more often than the Python read it"))
    }

    fn load(&self, readings: &Value) {
        *self.values.lock().expect("lock") = readings.as_array().expect("readings").iter().map(float).collect();
    }

    fn left(&self) -> usize {
        self.values.lock().expect("lock").len()
    }
}

struct Reader(Mutex<Option<FieldContext>>);

impl FieldReader for Reader {
    fn read(&self, _application: &str, _window: i64) -> Option<FieldContext> {
        self.0.lock().expect("lock").clone()
    }
}

fn result_json(result: &ContextResult) -> Value {
    json!({
        "decision": decision_json(&result.decision),
        "prediction": result.prediction.as_ref().map_or(Value::Null, prediction_json),
        "field": field_json(result.field.as_ref()),
        "policy_applied": result.policy_applied,
        "decision_source": result.decision_source,
        "fallback_reason": result.fallback_reason,
        "withheld": result.withheld_after_characters,
    })
}

#[test]
fn policy_decisions() {
    let Some((dir, _)) = paths() else { return };
    // Decisions the engine took on the batteries, and decisions made with the reader in every state and every mode.
    let rows: Vec<Value> = ["decide.jsonl.gz", "decide_decisions.jsonl.gz"].iter().filter_map(|name| rows(name)).flatten().collect();
    let resources = Resources::new(&dir);
    let detector = resources.language_detector(&["en_US", "ru_RU"]).expect("the detector");
    let script = Arc::new(Script::default());
    let reader = Arc::new(Reader(Mutex::new(None)));
    let mut policy = ContextPolicy::new(&resources, Some(reader.clone()), None, Some(script.clock()));
    assert!(policy.model.is_some(), "the context model loads: {}", policy.status);
    let mut failures = Failures::default();
    for (index, row) in rows.iter().enumerate().step_by(STEP) {
        let stream = row["stream"].as_array().expect("stream");
        policy.stream.application = stream[0].as_str().expect("text").to_string();
        policy.stream.window = stream[1].as_i64().expect("window");
        policy.stream.text = stream[2].as_str().expect("text").to_string();
        policy.stream.updated_at = float(&stream[3]);
        script.load(&row["clock"]);
        *reader.0.lock().expect("lock") = row["reads"].as_array().and_then(|reads| reads.first()).and_then(|read| field(&read[0]));
        let args = &row["args"];
        let ahead = args["typed_ahead"].as_str().unwrap_or("").to_string();
        let typed_ahead = move || ahead.clone();
        let text = |name: &str| args[name].as_str().expect("text").to_string();
        let (after, literal_tail, boundary_text) = (text("after"), text("literal_tail"), text("boundary_text"));
        let options = DecideOptions {
            after: &after,
            read_field: args["read_field"].as_bool().expect("flag"),
            field_override: field(&args["field_override"]),
            literal_tail: &literal_tail,
            boundary_text: &boundary_text,
            after_origin: after_origin(&args["after_origin"]),
            inside: args["inside"].as_bool().expect("flag"),
            before_origin: if args["before_origin"] == "planned_previous_reading" { BeforeOrigin::PlannedPreviousReading } else { BeforeOrigin::Field },
            typed_ahead: args["has_typed_ahead"].as_bool().expect("flag").then_some(&typed_ahead),
        };
        let result = policy
            .decide(
                &decision(&row["baseline"]),
                row["alternative"].as_str().expect("text"),
                row["target_group"].as_i64().expect("group"),
                &detector,
                row["trigger"].as_str().expect("text"),
                row["mode"].as_str().expect("text"),
                &options,
            )
            .expect("a decision");
        failures.check("decision", index, &result_json(&result), &row["result"]);
        failures.check("clock readings left", index, &json!(script.left()), &json!(0));
    }
    failures.finish("decide");
}

#[test]
fn prefix_models() {
    let Some((dir, models)) = paths() else { return };
    let Some(rows) = rows("prefix.jsonl.gz") else { return };
    let resources = Resources::new(&dir);
    let en = resources.engine_language_model("en_US").expect("en");
    let ru = resources.engine_language_model("ru_RU").expect("ru");
    // The replay patches `LanguageModel.load`, so its index is built from the model it was given (lexicon and supplement).
    let (en_index, ru_index) = (PrefixIndex::from_lexicon(en.frequencies.clone(), &en.source), PrefixIndex::from_lexicon(ru.frequencies.clone(), &ru.source));
    let sides = [PrefixSide { index: &en_index, model: &en }, PrefixSide { index: &ru_index, model: &ru }];
    let model = VersionedPrefixModel::default_model(&models).expect("the prefix model");
    let mut failures = Failures::default();
    let pairs = |features: &indexmap::IndexMap<String, f64>| Value::Array(features.iter().map(|(name, &value)| json!([name, hex(value)])).collect());
    for (index, row) in rows.iter().enumerate().step_by(STEP) {
        let item = PrefixInput {
            original: row["original"].as_str().expect("text").to_string(),
            alternative: row["alternative"].as_str().expect("text").to_string(),
            source_group: row["source_group"].as_i64().expect("group"),
            field: field(&row["field"]).expect("field"),
        };
        failures.check("version", index, &json!(model.feature_version), &row["version"]);
        failures.check("features", index, &pairs(&features_for_version(&item, [&sides[0], &sides[1]], model.feature_version).expect("schema")), &row["features"]);
        failures.check("features v1", index, &pairs(&features_for_version(&item, [&sides[0], &sides[1]], 1).expect("schema")), &row["features1"]);
        failures.check("prediction", index, &prediction_json(&model.predict(&item, [&sides[0], &sides[1]]).expect("schema")), &row["prediction"]);
    }
    failures.finish("prefix");
}

fn key_event(value: &Value) -> KeyEvent {
    let a = value.as_array().expect("event");
    KeyEvent {
        pressed: a[0].as_bool().expect("flag"),
        keycode: a[1].as_u64().expect("code").to::<u32>(),
        key_name: a[2].as_str().expect("text").to_string(),
        character: a[3].as_str().expect("text").to_string(),
        characters: a[4].as_array().expect("characters").iter().map(|c| c.as_str().expect("text").to_string()).collect(),
        group: a[5].as_i64().expect("group").to::<i32>(),
        state: a[6].as_u64().expect("state").to::<u32>(),
        timestamp: a[7].as_i64().expect("time"),
        synthetic: a[8].as_bool().expect("flag"),
        deferred: a[9].as_bool().expect("flag"),
        foreign: a[10].as_bool().expect("flag"),
        window: 0,
    }
}

fn key_json(e: &KeyEvent) -> Value {
    json!([e.pressed, e.keycode, e.key_name, e.character, e.characters, e.group, e.state, e.timestamp, e.synthetic, e.deferred, e.foreign])
}

#[test]
fn input_context_sequences() {
    let Some(rows) = rows("input_context.jsonl.gz") else { return };
    let script = Arc::new(Script::default());
    let mut contexts: HashMap<u64, InputContext> = HashMap::new();
    let mut failures = Failures::default();
    for (index, row) in rows.iter().enumerate() {
        let id = row["id"].as_u64().expect("id");
        let context = contexts.entry(id).or_insert_with(|| InputContext::with_clock(script.clock()));
        script.load(&row["clock"]);
        let args = row["args"].as_array().expect("args");
        let text = |position: usize| args[position].as_str().expect("text").to_string();
        let returned = match row["op"].as_str().expect("op") {
            "clear" => {
                context.clear();
                Value::Null
            }
            "focus" => {
                context.focus(&text(0), args[1].as_i64().expect("window"));
                Value::Null
            }
            "observe" => {
                context.observe(&key_event(&args[0]));
                Value::Null
            }
            "before_word" => json!(context.before_word(&text(0))),
            "withdraw" => json!(context.withdraw(&text(0))),
            "replace_suffix" => {
                context.replace_suffix(&text(0), &text(1), &text(2));
                Value::Null
            }
            "snapshot" => field_json(Some(&context.snapshot(&text(0)))),
            other => panic!("unknown operation {other}"),
        };
        failures.check(row["op"].as_str().expect("op"), index, &returned, &row["ret"]);
        let state = json!([context.application, context.window, context.text, hex(context.updated_at), context.revision]);
        failures.check("state", index, &state, &row["state"]);
        failures.check("clock readings left", index, &json!(script.left()), &json!(0));
    }
    failures.finish("input_context");
}

fn operation(name: &str) -> Operation {
    match name {
        "transposition" => Operation::Transposition,
        "deletion" => Operation::Deletion,
        "insertion" => Operation::Insertion,
        _ => Operation::Substitution,
    }
}

fn edit_of(operation_name: &Value, position: &Value, letter: &Value) -> Edit {
    Edit {
        operation: operation(operation_name.as_str().expect("operation")),
        position: position.as_u64().expect("position").to::<usize>(),
        letter: letter.as_str().and_then(|letter| letter.chars().next()),
    }
}

fn edit_json(edit: &Edit) -> Value {
    json!([edit.operation.as_str(), edit.position, edit.letter.map(String::from).unwrap_or_default()])
}

fn choice_json(choice: &TypoChoice) -> Value {
    json!([choice.word, choice.edit.operation.as_str(), choice.edit.position, choice.edit.letter.map(String::from).unwrap_or_default(), choice.weight, choice.candidates, hex(choice.margin)])
}

#[test]
fn typo_correction() {
    let Some((dir, models)) = paths() else { return };
    let Some(meta_path) = golden_file("typo_meta.json") else {
        eprintln!("SKIPPED: typo_meta.json is absent");
        return;
    };
    let meta: Value = serde_json::from_slice(&std::fs::read(meta_path).expect("read")).expect("json");
    let corrector = TypoCorrector::packaged(&dir, &["en_US", "ru_RU"]).expect("the corrector");
    let mut failures = Failures::default();
    for locale in ["en_US", "ru_RU"] {
        let found = keyboard_neighbours(locale_keys(locale).expect("keys")).expect("rows");
        let wanted: HashMap<char, String> = meta["neighbours"][locale].as_object().expect("map").iter().map(|(key, value)| (key.chars().next().expect("key"), value.as_str().expect("text").to_string())).collect();
        failures.check("neighbours", 0, &json!(found.len()), &json!(wanted.len()));
        for (key, near) in &wanted {
            failures.check("neighbours", 0, &json!(found.get(key)), &json!(near));
        }
        let lexicon = &corrector.lexicons.iter().find(|entry| entry.0 == locale).expect("list").1;
        failures.check("version", 0, &json!(lexicon.version), &meta["versions"][locale]);
        failures.check("size", 0, &json!(lexicon.size()), &meta["sizes"][locale]);
    }
    failures.check("minimum weight", 0, &json!(TypoPolicy::default().minimum_weight()), &meta["minimum_weight"]);
    failures.check("description", 0, &json!(ks_context::typo_correction::correct_typos_description()), &meta["description"]);

    let resources = Resources::new(&dir);
    let en = resources.engine_language_model("en_US").expect("en");
    let ru = resources.engine_language_model("ru_RU").expect("ru");
    let tables = TermTables::load(&models).expect("tables");
    let identifiers = resources.identifier_lexicon().0;
    let protected = resources.protected_tokens();
    let known = |text: &str| known_elsewhere(text, &[&en.frequencies, &ru.frequencies], &tables, identifiers.as_deref(), &protected, &|lower| lower == "zzzz");
    let _: Option<&LanguageModel> = None;
    let locales = ["en_US", "ru_RU"];
    if let Some(rows) = rows("typo_judge.jsonl.gz") {
        for (index, row) in rows.iter().enumerate().step_by(STEP) {
            let group = (row["group"].as_u64().expect("group")).to::<usize>();
            let word = row["word"].as_str().expect("text");
            let reading = row["reading"].as_str().expect("text");
            let judgement = corrector.judge(word, locales[group], &[(locales[1 - group], reading)], row["capital"].as_bool().expect("flag"), &known);
            failures.check("reason", index, &json!(judgement.reason.as_str()), &row["reason"]);
            failures.check("candidates", index, &json!(judgement.candidates), &row["candidates"]);
            failures.check("order", index, &json!(judgement.count_order()), &row["order"]);
            failures.check("choice", index, &judgement.choice.as_ref().map_or(Value::Null, choice_json), &row["choice"]);
            failures.check("known", index, &json!([known(word), known(reading)]), &row["known"]);
        }
    }
    if let Some(rows) = rows("typo_lexicon.jsonl.gz") {
        for (index, row) in rows.iter().enumerate().step_by(STEP) {
            let lexicon = |locale: &str| &corrector.lexicons.iter().find(|entry| entry.0 == locale).expect("list").1;
            let form = row["form"].as_str().expect("text");
            let list = lexicon(row["locale"].as_str().expect("locale"));
            failures.check("weight", index, &json!(list.weight(form)), &row["weight"]);
            failures.check("occurs", index, &json!(list.occurs(form)), &row["occurs"]);
            let other = lexicon(if row["locale"] == "en_US" { "ru_RU" } else { "en_US" });
            let reading = row["reading"][0].as_str().expect("text");
            failures.check("reading", index, &json!([reading, other.weight(reading), other.occurs(reading)]), &row["reading"]);
        }
    }
    if let Some(rows) = rows("typo_candidates.jsonl.gz") {
        for (index, row) in rows.iter().enumerate() {
            let found = corrector.candidates(row["word"].as_str().expect("text"), row["locale"].as_str().expect("locale"));
            let found: Vec<Value> = found.iter().map(|c| json!([c.word, c.weight, c.edits.iter().map(edit_json).collect::<Vec<_>>()])).collect();
            failures.check("candidates", index, &Value::Array(found), &row["candidates"]);
        }
    }
    if let Some(rows) = rows("typo_keys.jsonl.gz") {
        for (index, row) in rows.iter().enumerate() {
            let group = (row["group"].as_i64().expect("group")).to::<i32>();
            let mut templates = KeyTemplates::new();
            for event in row["learned"].as_array().expect("events").iter().chain(row["typed"].as_array().expect("events")) {
                templates.observe(&key_event(event));
            }
            let typed: Vec<KeyEvent> = row["typed"].as_array().expect("events").iter().map(key_event).collect();
            let c = &row["choice"];
            let choice = TypoChoice {
                word: c[0].as_str().expect("text").to_string(),
                edit: edit_of(&c[1], &c[2], &c[3]),
                weight: c[4].as_i64().expect("weight"),
                candidates: c[5].as_u64().expect("count").to::<usize>(),
                margin: float(&c[6]),
            };
            let found = corrected_keys(&typed, group, &choice, &templates).map_or(Value::Null, |keys| Value::Array(keys.iter().map(key_json).collect()));
            failures.check("keys", index, &found, &row["keys"]);
        }
    }
    failures.finish("typo");
}

/// The questions the model asks again, each with the words it asks about, in every position of a phrase.
#[test]
fn every_question_is_asked_in_every_position() {
    let Some((_, models)) = paths() else { return };
    let tables = TermTables::load(&models).expect("tables");
    let model = ContextModel::load(&models).expect("the context model");
    let pairs = [
        ("ю", "."), (".", "ю"), ("б", ","), (",", "б"), ("1ю", "1."), ("ю5", ".5"), ("3б14", "3,14"), ("э'", "''"), ("'f", "эа"), ("v67", "м67"), ("м67", "v67"), ("c2h5", "с2р5"), ("J.K.", "О.Л."), ("О.Л.", "J.K."), ("США", "CIF"), ("DKRCV", "ВЛКСМ"),
        ("ЗОР", "PHP"), ("кз", "rp"), ("rp", "кз"), ("tot", "еще"), ("еще", "tot"), ("Ы", "S"), ("S", "Ы"), ("а", "f"), ("f", "а"), ("d", "в"), ("в", "d"), ("b", "и"), ("и", "b"), ("x", "ч"), ("ч", "x"), ("X", "Ч"), ("Ч", "X"), ("ж", "z"), ("Ж", "Z"), ("ABC1", "ФИС1"), ("A1b2C3d4E5f6G7h8", "Ф1и2С3в4У5а6П7р8"),
    ];
    let befores = ["", " ", "для ", "текст ", "Текст. ", "x = ", "a b ", "1 ", "ABC ", "Да: ", "foo.", "\"", "вот, ", "the "];
    let afters = ["", " ", "=", "+", "12", "-", "--", "a", "==", "=1", "%", "abcdefghijklmnopqrstuvwxyz=", "да", "the", "1.5", ")"];
    let origins = [AfterOrigin::None, AfterOrigin::Field, AfterOrigin::PlannedNextConversion, AfterOrigin::KeptNextWord];
    let score = WordScore { value: 0.5, known: false, frequency: 5, gram_ratio: 0.5, exact: false, spell_known: false, ngram_score: -2.0, invalid_ratio: 0.1, raw_ngram_score: -2.0 };
    for (original, alternative) in pairs {
        for before in befores {
            for after in afters {
                for origin in origins {
                    for trigger in ks_lang::intent_model::TRIGGERS {
                        for source_group in [0, 1] {
                            let field = FieldContext { application: "app".to_string(), field_id: "1".to_string(), before: before.to_string(), after: after.to_string(), role: FieldRole::Text, selection: false, sensitive: false, source: "test".to_string() };
                            let item = ContextEvidence {
                                original: original.to_string(),
                                alternative: alternative.to_string(),
                                source_group,
                                field,
                                trigger: trigger.as_str().to_string(),
                                baseline_convert: source_group == 0,
                                source_known: false,
                                target_known: true,
                                score_delta: 1.5,
                                source_score: Some(score.clone()),
                                target_score: Some(score.clone()),
                                model_probability: None,
                                model_threshold: None,
                                literal_tail: String::new(),
                                ortho_score: Some(if source_group == 0 { -0.5 } else { 0.5 }),
                                ortho_threshold: Some(0.1),
                                boundary_text: " ".to_string(),
                                after_origin: origin,
                                before_origin: BeforeOrigin::Field,
                                source_identifier: false,
                                target_identifier: false,
                                source_typo: false,
                                target_typo: false,
                                source_opening: false,
                                target_opening: false,
                                inside: false,
                                term_counts_dropped: false,
                            };
                            let _ = questions(&item, &tables);
                            let _ = extract_action_features(&item, &tables);
                            assert!((0.0..=1.0).contains(&model.predict(&item).probability));
                            if original == "ч" && before == "для " && after == "=" && origin == AfterOrigin::KeptNextWord {
                                let refused = |change: &dyn Fn(&mut ContextEvidence)| {
                                    let mut broken = item.clone();
                                    change(&mut broken);
                                    extract_action_features(&broken, &tables).err().map(|error| error.to_string())
                                };
                                assert!(refused(&|broken| broken.source_group = 5).is_some());
                                assert!(refused(&|broken| broken.score_delta = f64::NAN).is_some());
                                assert!(refused(&|broken| broken.source_score.as_mut().unwrap().frequency = -1).is_some());
                                assert!(refused(&|broken| broken.field.after = "x".repeat(1000)).is_some());
                                assert!(refused(&|broken| { broken.after_origin = AfterOrigin::PlannedNextConversion; broken.field.after = "x".repeat(1000); }).is_some());
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Every shape of text a caller can hand over, in a grid: the answers are the recorded tests' business, this one says that
/// no combination stops the program, and that a prediction is always a number between 0 and 1.
#[test]
fn every_shape_of_text_gets_an_answer() {
    let Some((_, models)) = paths() else { return };
    let tables = TermTables::load(&models).expect("tables");
    let model = ContextModel::load(&models).expect("the context model");
    let words = [
        "", "a", "я", "A", "Я", "the", "The", "THE", "да", "Да", "ДА", "ghbdtn", "привет", "Привет", "ПРИВЕТ", "[f", "х", "б", "ю", "ж", "э", ",", ".", ";", "'", "[", "]", "{", "}", "1", "12", "1.", "1,", "z1", "а1", "A1", "a-b", "a_b",
        "да-да", "\"a", "«а", "aя", "яa", "%", "&", "#", "ё", "Ё", ".net", "c++", "123456789012", "abcdefghijklmnopqrstuvwxyz", "кто-то", "ab12cd34ef56gh78", "a1b2c3d4e5f6", "Ъ", "ъ",
    ];
    let befores = ["", " ", "текст ", "text ", "Текст. ", "a_b(", "x = ", "\"", "1 ", "ABC ", "а\n", "Да: ", "foo.", "http://", "«", "123 ", "да ", "the ", "слово, ", "ghbdtn "];
    let afters = ["", " ", "текст", "text", "1", "12", "+", "a b", "\n", ".", ", да", "x".repeat(200).leak(), "+=", "%"];
    let origins = [AfterOrigin::None, AfterOrigin::Field, AfterOrigin::PlannedNextConversion, AfterOrigin::KeptNextWord];
    let score = |known: bool, frequency: i64| WordScore { value: 0.5, known, frequency, gram_ratio: 0.5, exact: known, spell_known: !known, ngram_score: -2.0, invalid_ratio: 0.1, raw_ngram_score: -2.0 };
    let mut index = 0_usize;
    for original in words {
        for alternative in words {
            for before in befores {
                index += 1;
                let bit = |shift: usize| (index >> shift) & 1 == 1;
                let after = afters[index % afters.len()];
                let field = FieldContext {
                    application: "app".to_string(),
                    field_id: "1".to_string(),
                    before: before.to_string(),
                    after: after.to_string(),
                    role: [FieldRole::Text, FieldRole::Code, FieldRole::Search, FieldRole::Terminal, FieldRole::Unknown][index % 5],
                    selection: false,
                    sensitive: false,
                    source: "test".to_string(),
                };
                let item = ContextEvidence {
                    original: original.to_string(),
                    alternative: alternative.to_string(),
                    source_group: i64::from(bit(0)),
                    field,
                    trigger: ks_lang::intent_model::TRIGGERS[index % ks_lang::intent_model::TRIGGERS.len()].as_str().to_string(),
                    baseline_convert: bit(1),
                    source_known: bit(2),
                    target_known: bit(3),
                    score_delta: if bit(4) { 1.5 } else { -1.5 },
                    source_score: bit(5).then(|| score(bit(6), 5)),
                    target_score: bit(6).then(|| score(bit(7), 500)),
                    model_probability: bit(7).then_some(0.7),
                    model_threshold: bit(7).then_some(0.5),
                    literal_tail: if bit(8) { "'s".to_string() } else { String::new() },
                    ortho_score: bit(9).then_some(0.2),
                    ortho_threshold: bit(9).then_some(0.1),
                    boundary_text: if bit(10) { " ".to_string() } else { String::new() },
                    after_origin: origins[index % origins.len()],
                    before_origin: if bit(11) { BeforeOrigin::PlannedPreviousReading } else { BeforeOrigin::Field },
                    source_identifier: bit(12),
                    target_identifier: bit(13),
                    source_typo: bit(14),
                    target_typo: bit(15),
                    source_opening: bit(1),
                    target_opening: bit(2),
                    inside: bit(3),
                    term_counts_dropped: bit(4),
                };
                let _ = questions(&item, &tables);
                let probability = model.predict(&item).probability;
                assert!((0.0..=1.0).contains(&probability), "{item:?}: {probability}");
            }
        }
    }
}
