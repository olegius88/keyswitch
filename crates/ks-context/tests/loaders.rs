//! The model files as a loader may find them: damaged, cut, or written for another schema. Every such file is refused with its reason.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use ks_context::constants::{
    MAX_CONTEXT_ACTION_MODEL_FEATURES, MAX_CONTEXT_MODEL_BYTES, MAX_CONTEXT_MODEL_FEATURES, MAX_PREFIX_WEIGHTS, TYPO_ALPHABETS, TYPO_COUNT_LOG_SCALE, TYPO_FREQUENCY_SCHEMA_VERSION,
};
use ks_context::context_model::{BeforeOrigin, ContextModel};
use ks_context::flat_table::{WeightSpec, load_artifact};
use ks_context::prefix_schema::{VersionedPrefixModel, prefix_diagnostics};
use ks_context::typo_correction::{TypoLexicon, keyboard_neighbours, packaged_lexicon};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

fn models() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources/models"))
}

fn written(text: &[u8]) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let folder = std::env::temp_dir().join(format!("ks-loaders-{}", std::process::id()));
    std::fs::create_dir_all(&folder).expect("the folder");
    let path = folder.join(format!("{}.json", NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&path, text).expect("the file");
    path
}

/// The checksum a table of weights has, as the loader computes it.
fn digest(weights: &Value) -> String {
    let spec = WeightSpec {
        max_entries: usize::MAX,
        max_name_characters: usize::MAX,
        name_required: false,
        max_magnitude: None,
        entry_error: "entry",
        weight_error: "weight",
        table_error: "table",
    };
    let path = written(json!({"weights": weights}).to_string().as_bytes());
    let (artifact, _) = load_artifact(&path, usize::MAX, "too large", &spec).expect("it reads");
    artifact.and_then(|artifact| artifact.weights).expect("weights").digest
}

fn weights_of(count: usize) -> Value {
    Value::Object((0..count).map(|index| (format!("f{index}"), json!([0.5, -0.5, 0.0, 0.0]))).collect())
}

fn context_artifact() -> Value {
    let weights = weights_of(2);
    json!({
        "feature_version": 3,
        "actions": ["keep", "convert", "wait", "suggest"],
        "weights": weights,
        "weights_sha256": digest(&weights),
        "version": "context-v3-test",
        "conversion_threshold": 0.99,
    })
}

fn load_context(artifact: &Value) -> Result<ContextModel, String> {
    let path = written(artifact.to_string().as_bytes());
    ContextModel::load_artifact(&path, &models()).map_err(|error| error.message)
}

fn context_refusal(change: impl FnOnce(&mut Value)) -> String {
    let mut artifact = context_artifact();
    change(&mut artifact);
    load_context(&artifact).err().expect("refused")
}

#[test]
fn a_context_model_of_the_right_shape_loads() {
    let model = load_context(&context_artifact()).expect("it loads");
    assert_eq!(model.version, "context-v3-test");
    assert_eq!(BeforeOrigin::Field.as_str(), "field");
    assert_eq!(BeforeOrigin::PlannedPreviousReading.as_str(), "planned_previous_reading");
}

#[test]
fn a_context_model_of_another_shape_is_refused() {
    assert_eq!(context_refusal(|a| a["feature_version"] = json!(9)), "incompatible context model");
    assert_eq!(context_refusal(|a| a["feature_version"] = json!("3")), "incompatible context model");
    assert_eq!(context_refusal(|a| a["actions"] = json!(["keep"])), "incompatible context model");
    assert_eq!(context_refusal(|a| a["actions"] = json!(["keep", "convert", "suggest", "wait"])), "incompatible context model");
    assert_eq!(context_refusal(|a| a["weights"] = json!({})), "invalid context weights");
    assert_eq!(context_refusal(|a| a["weights"] = json!(1)), "invalid context weights");
    assert_eq!(context_refusal(|a| a["weights"] = json!({"f": [1.0]})), "invalid context feature");
    assert_eq!(context_refusal(|a| a["weights"] = json!({"f": [1e9, 0.0, 0.0, 0.0]})), "invalid context weight");
    assert_eq!(context_refusal(|a| a["weights_sha256"] = json!("0")), "context model checksum mismatch");
    assert_eq!(context_refusal(|a| a["version"] = json!("other")), "invalid context version");
    assert_eq!(context_refusal(|a| a["version"] = json!(format!("context-v3-{}", "x".repeat(100)))), "invalid context version");
    assert_eq!(context_refusal(|a| a["conversion_threshold"] = json!(0.1)), "unsafe context threshold");
    assert_eq!(context_refusal(|a| a["conversion_threshold"] = json!(1.5)), "unsafe context threshold");
    assert_eq!(context_refusal(|a| a["conversion_threshold"] = json!("x")), "unsafe context threshold");
}

#[test]
fn the_baseline_schema_has_its_own_namespace_and_budget() {
    let baseline = |count: usize| {
        let weights = weights_of(count);
        let mut artifact = context_artifact();
        artifact["feature_version"] = json!(2);
        artifact["version"] = json!("context-v1-test");
        artifact["weights_sha256"] = json!(digest(&weights));
        artifact["weights"] = weights;
        artifact
    };
    assert!(load_context(&baseline(1)).is_ok());
    assert_eq!(load_context(&baseline(MAX_CONTEXT_MODEL_FEATURES + 1)).err().as_deref(), Some("invalid context weights"));
    let mut wrong = baseline(1);
    wrong["version"] = json!("context-v3-test");
    assert_eq!(load_context(&wrong).err().as_deref(), Some("invalid context version"));
}

#[test]
fn a_context_model_over_its_limits_is_refused() {
    let many = weights_of(MAX_CONTEXT_ACTION_MODEL_FEATURES + 1);
    let mut artifact = context_artifact();
    artifact["weights"] = many;
    assert_eq!(load_context(&artifact).err().as_deref(), Some("invalid context weights"));

    let mut text = context_artifact();
    text["feature_version"] = json!(2);
    text["padding"] = json!(" ".repeat(MAX_CONTEXT_MODEL_BYTES));
    assert_eq!(load_context(&text).err().as_deref(), Some("context model is too large"));
}

fn prefix_artifact() -> Value {
    let weights = weights_of(1);
    let sum = digest(&weights);
    json!({
        "kind": "keyswitch.prefix-policy",
        "actions": ["keep", "convert", "wait", "suggest"],
        "feature_version": 2,
        "prefix_feature_version": 2,
        "weights": weights,
        "weights_sha256": sum,
        "version": format!("prefix-v2-{}", sum.get(..12).expect("a hash")),
        "conversion_threshold": 0.99,
    })
}

fn prefix_refusal(change: impl FnOnce(&mut Value)) -> Option<String> {
    let mut artifact = prefix_artifact();
    change(&mut artifact);
    VersionedPrefixModel::load(&written(artifact.to_string().as_bytes())).err().map(|error| error.message)
}

#[test]
fn prefix_models_of_every_shape() {
    let base = VersionedPrefixModel::load(&models().join("prefix_policy_v1.json")).expect("the shipped model");
    assert!(prefix_diagnostics(&models()).0);
    assert_eq!(prefix_diagnostics(&models()).1, base.version());
    assert_eq!(prefix_diagnostics(Path::new("/nonexistent")), (false, String::new()));

    let with_kind = |change: &dyn Fn(&mut Value)| prefix_refusal(change);
    assert_eq!(with_kind(&|_| {}), None);
    assert_eq!(with_kind(&|a| a["feature_version"] = json!(9)).as_deref(), Some("unsupported prefix model"));
    assert_eq!(with_kind(&|a| a["kind"] = json!("other")).as_deref(), Some("unsupported prefix model"));
    assert_eq!(with_kind(&|a| a["prefix_feature_version"] = json!(1)).as_deref(), Some("inconsistent prefix features"));
    assert_eq!(with_kind(&|a| drop(a.as_object_mut().expect("an object").remove("prefix_feature_version"))).as_deref(), Some("inconsistent prefix features"));
    assert_eq!(with_kind(&|a| a["version"] = json!("prefix-v2-short")).as_deref(), Some("invalid prefix version"));
    assert_eq!(with_kind(&|a| a["conversion_threshold"] = json!(0.1)).as_deref(), Some("unsafe prefix threshold"));
    assert_eq!(with_kind(&|a| a["conversion_threshold"] = json!("x")).as_deref(), Some("unsafe prefix threshold"));
    assert_eq!(with_kind(&|a| a["weights"] = json!({})).as_deref(), Some("invalid prefix weights"));
    assert_eq!(with_kind(&|a| a["weights"] = json!(1)).as_deref(), Some("invalid prefix weights"));
    assert_eq!(with_kind(&|a| a["weights"] = weights_of(MAX_PREFIX_WEIGHTS + 1)).as_deref(), Some("invalid prefix weights"));
    assert_eq!(with_kind(&|a| a["weights"] = json!({"f": [1.0]})).as_deref(), Some("invalid prefix coefficient"));
    assert_eq!(with_kind(&|a| a["weights_sha256"] = json!("0")).as_deref(), Some("prefix checksum mismatch"));
    assert_eq!(with_kind(&|a| a["version"] = json!("prefix-v2-000000000000")).as_deref(), Some("prefix checksum mismatch"));
    assert!(VersionedPrefixModel::load(&written(b"[1]")).is_err());
    assert!(VersionedPrefixModel::load(&written(b"7")).is_err());
    assert!(VersionedPrefixModel::load(&written(b"\"x\"")).is_err());
    assert!(VersionedPrefixModel::load(&written(b"null")).is_err());
    assert!(VersionedPrefixModel::load(&written(b"true")).is_err());
    assert!(VersionedPrefixModel::load(&written(b"1.5")).is_err());
    assert!(VersionedPrefixModel::load(&written(b"{")).is_err());
}

fn zlib(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).expect("in memory");
    encoder.finish().expect("in memory")
}

/// A file of word counts: the words `абв` and `г` and a filter that holds nothing.
fn typo_file(change: &dyn Fn(&mut Value), parts: [&[u8]; 3]) -> Vec<u8> {
    let (forms, weights) = (zlib(parts[0]), zlib(parts[1]));
    let payload = [forms.as_slice(), weights.as_slice(), parts[2]].concat();
    let mut header = json!({
        "schema_version": TYPO_FREQUENCY_SCHEMA_VERSION,
        "locale": "ru_RU",
        "alphabet": TYPO_ALPHABETS.iter().find(|entry| entry.0 == "ru_RU").expect("Russian").1,
        "count_log_scale": TYPO_COUNT_LOG_SCALE,
        "payload_sha256": Sha256::digest(&payload).iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
        "sections": {"forms": forms.len(), "weights": weights.len(), "seen": parts[2].len()},
        "seen": {"bits": 8, "hashes": 1},
    });
    change(&mut header);
    let mut bytes = header.to_string().into_bytes();
    bytes.push(b'\n');
    bytes.extend_from_slice(&payload);
    bytes
}

const GOOD: [&[u8]; 3] = [&[1, 2, 3, 0, 4, 0], &[5, 6], &[0]];

fn typo_refusal(bytes: &[u8]) -> String {
    TypoLexicon::load(&written(bytes), "ru_RU").err().map_or_else(|| "loaded".to_owned(), |error| error.message)
}

#[test]
fn a_list_of_word_counts_answers_for_its_words() {
    let lexicon = TypoLexicon::load(&written(&typo_file(&|_| {}, GOOD)), "ru_RU").expect("it loads");
    assert_eq!(lexicon.size(), 2);
    assert!(lexicon.weight("абв").is_some());
    assert_eq!(lexicon.weight(""), None);
    assert_eq!(lexicon.weight("абвг"), None);
    assert_eq!(lexicon.weight("\u{e9}"), None);
    assert_eq!(lexicon.weight("\u{4e2d}"), None);
    assert_eq!(lexicon.weight("a"), None);
}

#[test]
fn a_damaged_list_of_word_counts_is_refused() {
    assert_eq!(typo_refusal(b"no line break"), "typo frequency file has no header");
    assert!(TypoLexicon::load(&written(b"{\n"), "ru_RU").is_err());
    let invalid_header = "invalid typo frequency header";
    assert_eq!(typo_refusal(b"[1]\n"), invalid_header);
    let refusal = |change: &dyn Fn(&mut Value)| typo_refusal(&typo_file(change, GOOD));
    assert_eq!(refusal(&|_| {}), "loaded");
    assert_eq!(refusal(&|h| h["schema_version"] = json!(true)), "loaded");
    let changes: [fn(&mut Value); 9] = [
        |h| h["schema_version"] = json!(99),
        |h| h["schema_version"] = json!(false),
        |h| h["schema_version"] = json!("x"),
        |h| h["locale"] = json!("en_US"),
        |h| h["alphabet"] = json!("abc"),
        |h| h["count_log_scale"] = json!(1),
        |h| h["payload_sha256"] = json!("0"),
        |h| h["sections"] = json!(1),
        |h| h["seen"] = json!(1),
    ];
    for change in changes {
        assert_eq!(refusal(&change), invalid_header);
    }
    let sections = "invalid typo frequency sections";
    assert_eq!(refusal(&|h| h["seen"] = json!({"bits": 8})), sections);
    assert_eq!(refusal(&|h| h["seen"] = json!({"hashes": 1})), sections);
    assert_eq!(refusal(&|h| h["seen"] = json!({"bits": 0, "hashes": 1})), sections);
    assert_eq!(refusal(&|h| h["seen"] = json!({"bits": 8, "hashes": 0})), sections);
    assert_eq!(refusal(&|h| h["sections"] = json!({"forms": 1})), sections);
    assert_eq!(refusal(&|h| h["sections"] = json!({"forms": 1, "weights": 1, "seen": 1})), sections);
    assert!(TypoLexicon::load(&written(&typo_file(&|_| {}, GOOD)), "de_DE").is_err());
    let payload = "invalid typo frequency payload";
    assert_eq!(typo_refusal(&typo_file(&|_| {}, [&[1, 0], &[5, 6], &[0]])), payload);
    assert_eq!(typo_refusal(&typo_file(&|_| {}, [&[1], &[], &[0]])), payload);
    assert_eq!(typo_refusal(&typo_file(&|h| h["seen"] = json!({"bits": 16, "hashes": 1}), GOOD)), payload);
    assert_eq!(typo_refusal(&typo_file(&|_| {}, [&[], &[], &[0]])), "loaded");
    let mut broken = typo_file(&|_| {}, GOOD);
    let length = broken.len();
    broken.truncate(length - 1);
    assert_eq!(typo_refusal(&broken), invalid_header);
    assert!(packaged_lexicon(Path::new("/nonexistent"), "ru_RU").is_none());
    assert!(packaged_lexicon(&models().join(".."), "de_DE").is_none());
    assert!(keyboard_neighbours("short").is_err());
}

#[test]
fn a_table_of_weights_reads_any_value() {
    let spec = WeightSpec {
        max_entries: 10,
        max_name_characters: 8,
        name_required: true,
        max_magnitude: Some(5.0),
        entry_error: "entry",
        weight_error: "weight",
        table_error: "table",
    };
    let refused = |text: &str| -> Option<&'static str> {
        let path = written(text.as_bytes());
        load_artifact(&path, 10_000, "too large", &spec).expect("it reads").0.and_then(|artifact| artifact.weights).and_then(|weights| weights.refusal)
    };
    assert_eq!(refused(r#"{"weights": {"a": [0, 0, 0, 0]}}"#), None);
    assert_eq!(refused(r#"{"weights": {"a": {"x": 1}}}"#), Some("entry"));
    assert_eq!(refused(r#"{"weights": {"a": [0, 0, 0, 9], "b": [1]}}"#), Some("weight"));
    assert_eq!(refused(r#"{"weights": {"": [0, 0, 0, 0]}}"#), Some("entry"));
    assert!(load_artifact(&written(b"{"), 10_000, "too large", &spec).is_err());
    assert!(load_artifact(&written(b"{\"weights\": 1e999}"), 10_000, "too large", &spec).is_err());
}

#[test]
fn a_prefix_model_says_which_features_it_reads() {
    use ks_context::prefix_model::{PrefixInput, PrefixSide};
    use ks_context::prefix_schema::features_for_version;
    use ks_lang::Resources;
    let resources = Resources::new(models().parent().expect("the resources"));
    let (ru, en) = (resources.engine_language_model("ru_RU").expect("ru"), resources.engine_language_model("en_US").expect("en"));
    let (en_index, ru_index) = (resources.prefix_index(&en), resources.prefix_index(&ru));
    let sides = [&PrefixSide { index: &en_index, model: &en }, &PrefixSide { index: &ru_index, model: &ru }];
    let mut field = ks_backend::FieldContext::new("firefox", "1");
    "/* a comment ".clone_into(&mut field.before);
    let item = PrefixInput { original: "ghbd".to_owned(), alternative: "прив".to_owned(), source_group: 0, field };
    assert!(features_for_version(&item, sides, 99).is_err());
    assert!(features_for_version(&item, sides, 1).expect("the first schema").len() > 1);
}

#[test]
fn a_prefix_artifact_of_the_first_schema_need_not_say_so() {
    let mut artifact = prefix_artifact();
    artifact["feature_version"] = json!(1);
    artifact.as_object_mut().expect("an object").remove("prefix_feature_version");
    let _ = VersionedPrefixModel::load(&written(artifact.to_string().as_bytes()));
}

#[test]
fn a_list_of_word_counts_is_refused_past_its_size_and_for_a_language_it_has_no_letters_of() {
    use ks_context::constants::TYPO_FREQUENCY_MAX_BYTES;
    let path = written(b"");
    std::fs::File::options().write(true).open(&path).expect("the file").set_len(u64::try_from(TYPO_FREQUENCY_MAX_BYTES).expect("small") + 1).expect("sparse");
    assert!(TypoLexicon::load(&path, "ru_RU").is_err());
    std::fs::remove_file(&path).expect("removed");
    let header = json!({"schema_version": 1, "locale": "de_DE", "alphabet": "abc"});
    let path = written(format!("{header}\n").as_bytes());
    assert!(TypoLexicon::load(&path, "de_DE").is_err());
    assert!(ks_context::typo_correction::locale_keys("de_DE").is_none());
    assert!(ks_context::typo_correction::locale_keys("en_US").is_some());
}

#[test]
fn keys_that_type_nothing_teach_the_templates_nothing() {
    use ks_backend::KeyEvent;
    use ks_backend::constants::{ALT_MASK, CONTROL_MASK, SUPER_MASK};
    use ks_context::typo_correction::KeyTemplates;
    let mut templates = KeyTemplates::default();
    let key = KeyEvent { pressed: true, keycode: 38, key_name: "a".to_owned(), character: "a".to_owned(), characters: vec!["a".to_owned(), "ф".to_owned()], ..KeyEvent::default() };
    for event in [
        KeyEvent { pressed: false, ..key.clone() },
        KeyEvent { synthetic: true, ..key.clone() },
        KeyEvent { state: CONTROL_MASK, ..key.clone() },
        KeyEvent { state: ALT_MASK, ..key.clone() },
        KeyEvent { state: SUPER_MASK, ..key.clone() },
        key,
    ] {
        templates.observe(&event);
    }
}
