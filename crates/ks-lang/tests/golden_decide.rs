//! Parity of the decision layer (`intent_model`, `detector`, `short_words`, `word_decision`) with the Python, against
//! golden files dumped by `gen_golden_decide.py`.
//!
//! The files live in `$KS_GOLDEN_DIR/decide` (default: the `golden` folder of the private work tree; the word list is
//! `lang/words.txt.gz` beside it), the resources in `$KS_RESOURCES`; a test skips with a message when they are absent.
//! Floats are compared by their bits. In a debug build only every 20th row of the big files is compared.

#![expect(clippy::string_slice, clippy::unwrap_used, reason = "a test or a probe stops on what it did not expect")]
use ks_cast::To as _;
use flate2::read::GzDecoder;
use ks_lang::detector::{DecideOptions, DetectionDecision, LanguageDetector};
use ks_lang::intent_model::{self, CorrectionTrigger, IntentModelInput, LinearNgramModel, TRIGGERS};
use ks_lang::language_model::WordScore;
use ks_lang::short_words;
use ks_lang::word_decision::{self, AutomaticOptions};
use ks_lang::Resources;
use ks_text::layouts::LayoutPair;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

const THREADS: usize = 4;
const BATCH: usize = 8192;
const SIGNS: [&str; 9] = ["", " ", ",", ".", "?", "!", ":", "#", "x"];
const CONFIDENCES: [f64; 4] = [2.0, 0.5, 1.0, 10.0];

fn golden_root() -> PathBuf {
    PathBuf::from(std::env::var("KS_GOLDEN_DIR").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../.t/golden").to_string()))
}

fn golden(folder: &str, name: &str) -> Option<PathBuf> {
    let path = golden_root().join(folder).join(name);
    // The answers on made-up files are kept in the repository; those built on real typing are not.
    let kept = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures")).join(folder).join(name);
    if path.exists() {
        Some(path)
    } else if kept.exists() {
        Some(kept)
    } else {
        eprintln!("SKIPPED: golden file {} is absent", path.display());
        None
    }
}

fn resource_dir() -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var("KS_RESOURCES").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources").to_string()));
    if path.join("models/layout_intent_v1.ksm").exists() {
        Some(path)
    } else {
        eprintln!("SKIPPED: no resources at {}", path.display());
        None
    }
}

fn reader(name: &str) -> Option<BufReader<GzDecoder<std::fs::File>>> {
    golden("decide", name).map(|path| BufReader::new(GzDecoder::new(std::fs::File::open(path).expect("open"))))
}

fn json_file(name: &str) -> Option<Value> {
    let path = golden("decide", name)?;
    Some(serde_json::from_slice(&std::fs::read(path).expect("read")).expect("json"))
}

fn hex(x: f64) -> String {
    format!("{:016x}", x.to_bits())
}

#[derive(Default)]
struct Failures {
    count: AtomicUsize,
    total: AtomicUsize,
    shown: Mutex<Vec<String>>,
}

impl Failures {
    fn check<T: PartialEq + std::fmt::Debug>(&self, what: &str, input: &str, got: T, want: T) {
        self.total.fetch_add(1, Ordering::Relaxed);
        if got != want {
            self.count.fetch_add(1, Ordering::Relaxed);
            let mut shown = self.shown.lock().unwrap();
            if shown.len() < 25 {
                shown.push(format!("{what}({input:?}): got {got:?}, want {want:?}"));
            }
        }
    }

    fn finish(self, name: &str) {
        let (count, total) = (self.count.load(Ordering::Relaxed), self.total.load(Ordering::Relaxed));
        eprintln!("{name}: {total} checks, {count} failures");
        assert!(total > 0, "{name}: nothing was compared");
        assert_eq!(count, 0, "{name}:\n{}", self.shown.lock().unwrap().join("\n"));
    }
}

/// Calls `visit(index, row)` for rows of a file, on several threads, a batch at a time (every 20th row in a debug build).
fn for_each_row<F: Fn(usize, &Value) + Sync>(name: &str, visit: F) -> bool {
    let Some(file) = reader(name) else {
        return false;
    };
    let step = if cfg!(debug_assertions) { 20 } else { 1 };
    let mut lines = file.lines();
    let mut base = 0;
    loop {
        let batch: Vec<String> = lines.by_ref().take(BATCH).map(|line| line.expect("line")).collect();
        if batch.is_empty() {
            return true;
        }
        let visit = &visit;
        std::thread::scope(|scope| {
            for (part, chunk) in batch.chunks(BATCH.div_ceil(THREADS)).enumerate() {
                let first = base + part * BATCH.div_ceil(THREADS);
                scope.spawn(move || {
                    for (offset, line) in chunk.iter().enumerate() {
                        if (first + offset).is_multiple_of(step) && !line.is_empty() {
                            visit(first + offset, &serde_json::from_str(line).expect("json row"));
                        }
                    }
                });
            }
        });
        base += batch.len();
    }
}

struct Fixture {
    detector: LanguageDetector,
    intent: Arc<LinearNgramModel>,
    resource_dir: PathBuf,
    words: Vec<String>,
    pair: LayoutPair,
}

fn fixture() -> Option<&'static Fixture> {
    static FIXTURE: OnceLock<Option<Fixture>> = OnceLock::new();
    FIXTURE
        .get_or_init(|| {
            let dir = resource_dir()?;
            // The word list comes from real typing and is not in the repository: without it the tests over words have nothing to go through.
            let mut text = String::new();
            if let Some(mut reader) = reader_of_lang("words.txt.gz") {
                reader.read_to_string(&mut text).expect("words");
            }
            let mut words: Vec<String> = text.split('\n').map(str::to_string).collect();
            words.pop();
            let resources = Resources::new(dir.clone());
            let detector = resources.language_detector(&["en_US", "ru_RU"]).expect("detector");
            let intent = resources.intent_model().0.expect("the packaged intent model");
            Some(Fixture { detector, intent, resource_dir: dir, words, pair: LayoutPair::default() })
        })
        .as_ref()
}

fn reader_of_lang(name: &str) -> Option<GzDecoder<std::fs::File>> {
    golden("lang", name).map(|path| GzDecoder::new(std::fs::File::open(path).expect("open")))
}

const LAYOUT: [&str; 2] = ["us", "ru"];

impl Fixture {
    /// `alternative(word, source)`.
    fn alternative(&self, word: &str, source: i64) -> String {
        self.pair.translate(word, LAYOUT[source.to::<usize>()], LAYOUT[1 - source.to::<usize>()]).unwrap()
    }

    fn score(&self, group: i64, word: &str) -> WordScore {
        self.detector.model(group).unwrap().score(word)
    }

    fn evidence<'a>(&self, original: &'a str, alternative: &'a str, source: i64, target: i64, trigger: CorrectionTrigger, score: &'a WordScore) -> IntentModelInput<'a> {
        IntentModelInput { original, alternative, source_group: source, target_group: target, trigger, source_score: score, target_score: score, context_delta: 0.0, context_group: None }
    }
}

type Texts = Vec<(i64, String)>;

fn texts(value: &Value) -> Texts {
    value.as_array().unwrap().iter().map(|item| (item[0].as_i64().unwrap(), item[1].as_str().unwrap().to_string())).collect()
}

fn feature_digest(features: &intent_model::FeatureVector) -> String {
    let values: Vec<String> = features.values.iter().map(|(bucket, value)| format!("{bucket}:{}", hex(*value))).collect();
    let prints: Vec<String> = features.character_fingerprints.iter().map(|print| format!("{print:x}")).collect();
    let text = format!("{}|{}", values.join(","), prints.join(","));
    Sha256::digest(text.as_bytes()).iter().take(4).map(|byte| format!("{byte:02x}")).collect()
}

fn extract(f: &Fixture, item: &IntentModelInput<'_>) -> intent_model::FeatureVector {
    let model = &f.intent;
    intent_model::extract_features(item, model.dimension, model.fnv_seed, model.membership_seed, &model.ngram_orders).unwrap()
}

fn projection(f: &Fixture, decision: &DetectionDecision, original: &str, source: i64, alternatives: &Texts) -> Value {
    let target = decision.target_group;
    let replacement = if decision.replacement == original {
        json!(0)
    } else if alternatives.iter().find(|item| item.0 == target).is_some_and(|item| item.1 == decision.replacement) {
        json!(1)
    } else {
        json!(decision.replacement)
    };
    assert_eq!(decision.original, original);
    assert_eq!(decision.source_group, source);
    let version = i64::from(decision.model_version == f.intent.model_version);
    assert!(decision.model_version.is_empty() || version == 1);
    let scores = i64::from(decision.source_score == f.score(source, original)) | i64::from(decision.target_score == f.score(target, &decision.replacement)) << 1;
    json!([
        i64::from(decision.should_convert),
        replacement,
        target,
        hex(decision.confidence),
        decision.reason,
        decision.model_probability.map(hex),
        decision.model_threshold.map(hex),
        version,
        scores,
        i64::from(short_words::is_short_word_override(decision))
    ])
}

fn projected_option(f: &Fixture, decision: Option<DetectionDecision>, original: &str, source: i64, alternatives: &Texts) -> Value {
    decision.map_or(Value::Null, |decision| projection(f, &decision, original, source, alternatives))
}

// ---------------------------------------------------------------- the model

#[test]
fn intent_model_facts() {
    let (Some(f), Some(meta)) = (fixture(), json_file("intent_meta.json")) else {
        return;
    };
    let fail = Failures::default();
    let (model, real) = (&f.intent, &meta["real"]);
    let s = |value: &Value| value.as_str().unwrap().to_string();
    fail.check("dimension", "", model.dimension, real["dimension"].as_i64().unwrap());
    fail.check("weight_scale", "", hex(model.weight_scale), s(&real["weight_scale"]));
    fail.check("bias", "", hex(model.bias), s(&real["bias"]));
    fail.check("veto", "", hex(model.veto_threshold), s(&real["veto"]));
    fail.check("version", "", model.model_version.clone(), s(&real["version"]));
    fail.check("fnv_seed", "", model.fnv_seed, real["fnv_seed"].as_u64().unwrap());
    fail.check("membership_seed", "", model.membership_seed, real["membership_seed"].as_u64().unwrap());
    fail.check("payload_sha256", "", model.payload_sha256.clone(), s(&real["payload_sha256"]));
    fail.check("checksum", "", model.checksum.clone(), s(&real["checksum"]));
    let weights: Vec<u8> = (0..model.dimension.to::<usize>()).flat_map(|bucket| model.weight(bucket).to_le_bytes()).collect();
    fail.check("weights", "", format!("{:x}", Sha256::digest(&weights)), s(&real["weights_sha256"]));
    let canonical = ks_text::json_dumps(
        &model.metadata(),
        &ks_text::JsonOptions { ensure_ascii: false, sort_keys: true, separators: Some((",".into(), ":".into())), ..Default::default() },
    );
    fail.check("metadata", "", format!("{:x}", Sha256::digest(canonical.as_bytes())), s(&real["metadata_sha256"]));
    for (direction, name) in intent_model::LAYOUT_DIRECTIONS.iter().zip(["0>1", "1>0"]) {
        let want = &real["platt"][name];
        let got = model.platt_calibration[direction_index(name)];
        fail.check("platt", name, json!([hex(got.scale), hex(got.bias)]), want.clone());
        assert_eq!(direction.as_str(), name);
    }
    for (index, trigger) in TRIGGERS.iter().enumerate() {
        for (slot, direction) in ["0>1", "1>0"].iter().enumerate() {
            fail.check("threshold_logit", trigger.as_str(), hex(model.threshold_logits[index][slot]), s(&real["threshold_logits"][trigger.as_str()][*direction]));
            fail.check("threshold", trigger.as_str(), hex(model.thresholds[index][slot]), s(&real["thresholds"][trigger.as_str()][*direction]));
        }
    }
    let (loaded, status) = LinearNgramModel::try_load_default(&f.resource_dir, None);
    fail.check("status", "", status.summary(), s(&meta["status_summary"]));
    fail.check("status version", "", status.version.clone().unwrap(), s(&meta["status_version"]));
    fail.check("status checksum", "", status.checksum.clone().unwrap(), s(&meta["status_checksum"]));
    fail.check("override that is missing", "", LinearNgramModel::try_load_default(&f.resource_dir, Some("/nonexistent/x.ksm")).1.available, true);
    drop(loaded);
    let triggers: Vec<&str> = TRIGGERS.iter().map(|trigger| trigger.as_str()).collect();
    fail.check("trigger order", "", json!(triggers), meta["trigger_order"].clone());
    for row in meta["scalars"].as_array().unwrap() {
        if row[0] == "dimension" {
            let got = intent_model::signed_feature_hash("a", row[1].as_i64().unwrap(), 0xCBF2_9CE4_8422_2325);
            let message = got.err().map_or("ok".to_string(), |error| error.message);
            fail.check("signed_feature_hash dimension", &row[1].to_string(), message, s(&row[2]));
            continue;
        }
        let text = row[0].as_str().unwrap();
        fail.check("fnv1a64", text, format!("{:x}", intent_model::fnv1a64(text, 0xCBF2_9CE4_8422_2325)), s(&row[1]));
        fail.check("fnv1a64 seed", text, format!("{:x}", intent_model::fnv1a64(text, 5)), s(&row[2]));
        let (bucket, sign) = intent_model::signed_feature_hash(text, 1 << 10, 7).unwrap();
        fail.check("signed_feature_hash", text, json!([bucket, sign]), row[3].clone());
        fail.check("normalize_token", text, intent_model::normalize_token(text), s(&row[4]));
    }
    for row in meta["sigmoid"].as_array().unwrap() {
        let x = f64::from_bits(u64::from_str_radix(row[0].as_str().unwrap(), 16).unwrap());
        fail.check("stable_sigmoid", row[0].as_str().unwrap(), hex(intent_model::stable_sigmoid(x).unwrap()), s(&row[1]));
    }
    fail.check("sigmoid nan", "", intent_model::stable_sigmoid(f64::NAN).unwrap_err().message, s(&meta["sigmoid_nan"]));
    for row in meta["directions"].as_array().unwrap() {
        let got = match intent_model::layout_direction(row[0].as_i64().unwrap(), row[1].as_i64().unwrap()) {
            Ok(direction) => direction.as_str().to_string(),
            Err(error) => format!("ValueError: {}", error.message),
        };
        fail.check("layout_direction", &row[0].to_string(), got, s(&row[2]));
    }
    let protected = f.detector.protected_tokens.sorted();
    let want: Vec<&str> = meta["protected_tokens"].as_array().unwrap().iter().map(|item| item.as_str().unwrap()).collect();
    fail.check("protected tokens", "", protected, want);
    for (slot, group) in [0usize, 1].iter().enumerate() {
        let model = f.detector.model(slot.to::<i64>()).unwrap().as_language_model().unwrap();
        let starts = short_words::word_starts(model);
        let got: Vec<Value> = starts.items().iter().map(|(start, count)| json!([start, count])).collect();
        fail.check("word_starts", &group.to_string(), json!(got), meta["word_starts"][group.to_string()].clone());
    }
    for row in meta["errors"].as_array().unwrap() {
        let (name, want) = (row[0].as_str().unwrap(), (row[1].as_str().unwrap(), s(&row[2])));
        let alternatives: Texts = vec![(1, "привет".to_string())];
        let got = match name {
            "one-model" => {
                let one: Vec<(i64, Arc<dyn ks_lang::detector::LanguageScorer>)> = vec![(0, f.detector.models[0].1.clone())];
                let error = LanguageDetector::new(one, None, f.detector.protected_tokens.clone()).err().unwrap();
                (error.name, error.message)
            }
            "missing-source" => {
                let error = f.detector.decide("ghbdtn", &alternatives, 5, &DecideOptions::default()).unwrap_err();
                (error.name, error.message)
            }
            "missing-source-automatic" => {
                let error = word_decision::automatic_word_decision(&f.detector, "ghbdtn", &alternatives, 5, &AutomaticOptions::default()).unwrap_err();
                (error.name, error.message)
            }
            _ => {
                let score = f.score(0, "");
                let item = f.evidence("a", "б", 0, 1, CorrectionTrigger::Space, &score);
                let error = intent_model::extract_features(&item, 3, 1, 2, &[1, 2, 3, 4, 5]).unwrap_err();
                (error.name, error.message)
            }
        };
        fail.check("error", name, (got.0.to_string(), got.1), (want.0.to_string(), want.1));
    }
    fail.finish("intent_model_facts");
}

fn direction_index(name: &str) -> usize {
    usize::from(name == "1>0")
}

#[test]
fn intent_loader_cases() {
    let (Some(f), Some(cases)) = (fixture(), json_file("intent_loader.json")) else {
        return;
    };
    let fail = Failures::default();
    let decode = |text: &str| -> Vec<u8> { (0..text.len() / 2).map(|at| u8::from_str_radix(&text[at * 2..at * 2 + 2], 16).unwrap()).collect() };
    for case in cases.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let result = LinearNgramModel::from_bytes(decode(case["data"].as_str().unwrap()), None);
        match (result, case.get("error")) {
            (Err(error), Some(want)) => fail.check("error", name, json!([error.name, error.message]), want.clone()),
            (Err(error), None) => fail.check("loaded", name, format!("{}: {}", error.name, error.message), "a model".to_string()),
            (Ok(_), Some(want)) => fail.check("error", name, "a model".to_string(), want.to_string()),
            (Ok(model), None) => {
                let facts = &case["facts"];
                let s = |value: &Value| value.as_str().unwrap().to_string();
                fail.check("dimension", name, model.dimension, facts["dimension"].as_i64().unwrap());
                fail.check("scale", name, hex(model.weight_scale), s(&facts["weight_scale"]));
                fail.check("bias", name, hex(model.bias), s(&facts["bias"]));
                fail.check("veto", name, hex(model.veto_threshold), s(&facts["veto"]));
                fail.check("version", name, model.model_version.clone(), s(&facts["version"]));
                fail.check("checksum", name, model.checksum.clone(), s(&facts["checksum"]));
                fail.check("payload sha", name, model.payload_sha256.clone(), s(&facts["payload_sha256"]));
                let canonical = ks_text::json_dumps(
                    &model.metadata(),
                    &ks_text::JsonOptions { ensure_ascii: false, sort_keys: true, separators: Some((",".into(), ":".into())), ..Default::default() },
                );
                fail.check("metadata", name, format!("{:x}", Sha256::digest(canonical.as_bytes())), s(&facts["metadata_sha256"]));
                for (index, trigger) in TRIGGERS.iter().enumerate() {
                    for (slot, direction) in ["0>1", "1>0"].iter().enumerate() {
                        fail.check("threshold", name, hex(model.thresholds[index][slot]), s(&facts["thresholds"][trigger.as_str()][*direction]));
                    }
                }
                for row in case["predictions"].as_array().unwrap() {
                    let (original, alt) = (row[0].as_str().unwrap(), row[1].as_str().unwrap());
                    let trigger = CorrectionTrigger::parse(row[4].as_str().unwrap()).unwrap();
                    let score = f.score(0, "");
                    let item = f.evidence(original, alt, row[2].as_i64().unwrap(), row[3].as_i64().unwrap(), trigger, &score);
                    match model.predict(&item) {
                        Ok(p) => fail.check(
                            "predict",
                            name,
                            json!([hex(p.logit), hex(p.probability), hex(p.threshold), hex(p.coverage), i64::from(p.should_switch)]),
                            json!([row[5], row[6], row[7], row[8], row[9]]),
                        ),
                        Err(error) => fail.check("predict error", name, json!(["ValueError", error.message]), json!([row[5], row[6]])),
                    }
                }
            }
        }
    }
    fail.finish("intent_loader_cases");
}

// ---------------------------------------------------------------- words

#[test]
fn words_helpers() {
    let Some(f) = fixture() else {
        return;
    };
    let fail = Failures::default();
    let ran = for_each_row("words_helpers.jsonl.gz", |_, row| {
        let word = &f.words[row[0].as_u64().unwrap().to::<usize>()];
        fail.check("token_key", word, LanguageDetector::token_key(word), row[1].as_str().unwrap().to_string());
        let model = |group: i64| f.detector.model(group).unwrap().as_language_model().unwrap();
        let flags = i64::from(f.detector.is_protected_token(word))
            | i64::from(short_words::opens_sentences(word, 0)) << 1
            | i64::from(short_words::opens_sentences(word, 1)) << 2
            | i64::from(short_words::begins_words(model(0), word)) << 3
            | i64::from(short_words::begins_words(model(1), word)) << 4;
        fail.check("flags", word, flags, row[2].as_i64().unwrap());
        fail.check("normalize_token", word, intent_model::normalize_token(word), row[3].as_str().unwrap().to_string());
        fail.check("fnv1a64", word, format!("{:x}", intent_model::fnv1a64(word, 0xCBF2_9CE4_8422_2325)), row[4].as_str().unwrap().to_string());
        let (bucket, sign) = intent_model::signed_feature_hash(word, 1 << 21, 0xCBF2_9CE4_8422_2325).unwrap();
        fail.check("signed_feature_hash", word, json!([bucket, sign]), json!([row[5], row[6]]));
    });
    if ran {
        fail.finish("words_helpers");
    }
}

#[test]
fn words_intent() {
    let Some(f) = fixture() else {
        return;
    };
    let fail = Failures::default();
    let ran = for_each_row("words_intent.jsonl.gz", |_, row| {
        let word = &f.words[row[0].as_u64().unwrap().to::<usize>()];
        let (source, trigger) = (row[1].as_i64().unwrap(), TRIGGERS[row[2].as_u64().unwrap().to::<usize>()]);
        let alt = f.alternative(word, source);
        let score = f.score(0, "");
        let item = f.evidence(word, &alt, source, 1 - source, trigger, &score);
        let features = extract(f, &item);
        let p = f.intent.predict(&item).unwrap();
        let supported = ((p.coverage * features.character_fingerprints.len().to::<f64>()).round()).to::<u64>();
        let got = json!([hex(p.logit), hex(p.probability), supported, features.character_fingerprints.len(), i64::from(p.should_switch), features.values.len(), feature_digest(&features)]);
        fail.check("predict", word, got, json!([row[3], row[4], row[5], row[6], row[7], row[8], row[9]]));
    });
    if ran {
        fail.finish("words_intent");
    }
}

#[test]
fn words_features() {
    let Some(f) = fixture() else {
        return;
    };
    let fail = Failures::default();
    let ran = for_each_row("words_features.jsonl.gz", |_, row| {
        let word = &f.words[row[0].as_u64().unwrap().to::<usize>()];
        let source = row[1].as_i64().unwrap();
        let alt = f.alternative(word, source);
        let score = f.score(0, "");
        let features = extract(f, &f.evidence(word, &alt, source, 1 - source, CorrectionTrigger::Space, &score));
        let values: Vec<Value> = features.values.iter().map(|(bucket, value)| json!([bucket, hex(*value)])).collect();
        let prints: Vec<String> = features.character_fingerprints.iter().map(|print| format!("{print:x}")).collect();
        fail.check("features", word, json!([values, prints]), json!([row[2], row[3]]));
    });
    if ran {
        fail.finish("words_features");
    }
}

#[test]
fn intent_groups() {
    let Some(f) = fixture() else {
        return;
    };
    let fail = Failures::default();
    let ran = for_each_row("intent_groups.jsonl.gz", |_, row| {
        let word = row[0].as_str().unwrap();
        let (source, target) = (row[1].as_i64().unwrap(), row[2].as_i64().unwrap());
        let trigger = CorrectionTrigger::parse(row[3].as_str().unwrap()).unwrap();
        let alt = f.pair.translate(word, "us", "ru").unwrap();
        let score = f.score(0, "");
        let item = f.evidence(word, &alt, source, target, trigger, &score);
        let features = intent_model::extract_features(&item, 1 << 21, 0xCBF2_9CE4_8422_2325, 0x9E37_79B9_7F4A_7C15, &[1, 2, 3, 4, 5]).unwrap();
        let prediction = match f.intent.predict(&item) {
            Ok(p) => json!([hex(p.logit), hex(p.probability), i64::from(p.should_switch), hex(p.coverage)]),
            Err(error) => json!(["ValueError", error.message]),
        };
        fail.check("groups", word, json!([features.values.len(), feature_digest(&features), prediction]), json!([row[4], row[5], row[6]]));
    });
    if ran {
        fail.finish("intent_groups");
    }
}

// ---------------------------------------------------------------- decisions

/// `base_options` of the generator: the option vector of a sweep row, from its index.
struct Base {
    ignored: Vec<String>,
    previous: Texts,
    rejected: Vec<i64>,
    following: Texts,
    minimum_length: i64,
    confidence: f64,
    aggressive: bool,
    protect: bool,
    context_group: Option<i64>,
    forced: Option<i64>,
    trigger: CorrectionTrigger,
    intent: bool,
    tracked: bool,
}

impl Base {
    fn new(f: &Fixture, kind: u64, i: usize, source: i64) -> Base {
        let target = 1 - source;
        let mut base = Base {
            ignored: Vec::new(),
            previous: Vec::new(),
            rejected: Vec::new(),
            following: Vec::new(),
            minimum_length: 3,
            confidence: 2.0,
            aggressive: false,
            protect: true,
            context_group: None,
            forced: None,
            trigger: TRIGGERS[(i + source.to::<usize>() + usize::from(kind > 1)) % 6],
            intent: true,
            tracked: true,
        };
        if kind == 2 {
            base.minimum_length = 1 + (i % 4).to::<i64>();
            base.confidence = CONFIDENCES[(i / 3) % 4];
            base.aggressive = i % 2 == 1;
            base.protect = !i.is_multiple_of(7);
            base.context_group = [None, Some(source), Some(target)][i % 3];
            base.intent = !i.is_multiple_of(3);
            if i.is_multiple_of(11) {
                base.forced = Some(target);
            }
            if i.is_multiple_of(13) {
                base.rejected = vec![target];
            }
            if !i.is_multiple_of(5) {
                let previous = &f.words[(i * 7 + 13) % f.words.len()];
                let (own, other) = (previous.clone(), f.alternative(previous, source));
                base.previous = if source == 0 { vec![(source, own), (target, other)] } else { vec![(target, other), (source, own)] };
            }
            let text = format!("{}{}", SIGNS[i % 9], &"yz"[..i % 3]);
            base.tracked = i % 4 != 3;
            if !text.is_empty() {
                base.following = vec![(target, text)];
            }
        }
        base
    }

    fn options(&self) -> DecideOptions<'_> {
        DecideOptions {
            minimum_length: self.minimum_length,
            confidence_threshold: self.confidence,
            ignored_words: &self.ignored,
            aggressive: self.aggressive,
            protect_code: self.protect,
            previous_words: &self.previous,
            context_group: self.context_group,
            forced_target_group: self.forced,
            rejected_targets: &self.rejected,
            trigger: self.trigger,
            use_intent_model: self.intent,
        }
    }
}

#[test]
fn decide_sweep() {
    let Some(f) = fixture() else {
        return;
    };
    let fail = Failures::default();
    let ran = for_each_row("decide_sweep.jsonl.gz", |_, row| {
        let (i, source, kind) = (row[0].as_u64().unwrap().to::<usize>(), row[1].as_i64().unwrap(), row[2].as_u64().unwrap());
        let word = &f.words[i];
        let alternatives: Texts = vec![(1 - source, f.alternative(word, source))];
        let base = Base::new(f, kind, i, source);
        let decision = if kind == 0 {
            f.detector.decide(word, &alternatives, source, &base.options()).unwrap()
        } else {
            let options = AutomaticOptions { decide: base.options(), context_tracked: base.tracked, following: &base.following };
            word_decision::automatic_word_decision(&f.detector, word, &alternatives, source, &options).unwrap()
        };
        fail.check("decision", &format!("{word} {source} {kind}"), projection(f, &decision, word, source, &alternatives), row[3].clone());
    });
    if ran {
        fail.finish("decide_sweep");
    }
}

fn explicit_row(f: &Fixture, fail: &Failures, row: &Value) {
    let (original, source) = (row[0].as_str().unwrap(), row[1].as_i64().unwrap());
    let alternatives = texts(&row[2]);
    let o = &row[3];
    let ignored: Vec<String> = o[9].as_array().unwrap().iter().map(|item| item.as_str().unwrap().to_string()).collect();
    let rejected: Vec<i64> = o[8].as_array().unwrap().iter().map(|item| item.as_i64().unwrap()).collect();
    let (previous, following) = (texts(&o[10]), texts(&o[12]));
    let group = |value: &Value| value.as_i64();
    let options = DecideOptions {
        minimum_length: o[0].as_i64().unwrap(),
        confidence_threshold: o[1].as_f64().unwrap(),
        ignored_words: &ignored,
        aggressive: o[2] == 1,
        protect_code: o[3] == 1,
        previous_words: &previous,
        context_group: group(&o[6]),
        forced_target_group: group(&o[7]),
        rejected_targets: &rejected,
        trigger: TRIGGERS[o[4].as_u64().unwrap().to::<usize>()],
        use_intent_model: o[5] == 1,
    };
    let key = format!("{original:?} {source} {o}");
    let decision = f.detector.decide(original, &alternatives, source, &options).unwrap();
    let automatic = word_decision::automatic_word_decision(&f.detector, original, &alternatives, source, &AutomaticOptions { decide: options, context_tracked: o[11] == 1, following: &following }).unwrap();
    let trusted = short_words::trusted_short_word_decision(&f.detector, original, &alternatives, source, &ignored, &rejected, options.protect_code, options.context_group).unwrap();
    let opening = word_decision::opening_letter_decision(&f.detector, original, &alternatives, source, options.context_group, &ignored, &rejected, options.protect_code, &following).unwrap();
    let veto = short_words::natural_short_source_veto(decision.clone(), options.context_group);
    let shape = word_decision::word_shape_veto(decision.clone());
    let got = json!([
        projection(f, &decision, original, source, &alternatives),
        projection(f, &automatic, original, source, &alternatives),
        projected_option(f, trusted, original, source, &alternatives),
        projected_option(f, opening, original, source, &alternatives),
        projection(f, &veto, original, source, &alternatives),
        projection(f, &shape, original, source, &alternatives)
    ]);
    fail.check("explicit case", &key, got, row[4].clone());
}

#[test]
fn decide_options() {
    let Some(f) = fixture() else {
        return;
    };
    let fail = Failures::default();
    if for_each_row("decide_options.jsonl.gz", |_, row| explicit_row(f, &fail, row)) {
        fail.finish("decide_options");
    }
}

#[test]
fn decide_curated() {
    let Some(f) = fixture() else {
        return;
    };
    let fail = Failures::default();
    if for_each_row("decide_curated.jsonl.gz", |_, row| explicit_row(f, &fail, row)) {
        fail.finish("decide_curated");
    }
}
