//! The model files as a loader may find them: damaged, cut, or written for another schema. Every such file is refused with its reason.
#![expect(clippy::disallowed_types, clippy::disallowed_macros, clippy::disallowed_methods, reason = "temporary: these tests still build and read their JSON as untyped trees; step 7 of the strict-json plan types them and removes this")]

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use ks_context::constants::{
    MAX_CONTEXT_ACTION_MODEL_FEATURES, MAX_CONTEXT_MODEL_BYTES, MAX_CONTEXT_MODEL_FEATURES, MAX_PREFIX_WEIGHTS, TYPO_ALPHABETS, TYPO_COUNT_LOG_SCALE, TYPO_FREQUENCY_SCHEMA_VERSION,
};
use ks_context::context_model::{BeforeOrigin, CapitalCounts, ContextModel, TermTables, CAPITALS_FREQUENCY_FILE};
use ks_context::constants::{COMMON_FEATURE_PREFIX, MAX_CONTEXT_CAPITALS_FREQUENCY_BYTES, UPPER_FEATURE_PREFIX};
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
fn digest(weights: &impl serde::Serialize) -> String {
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
fn a_long_list_of_word_counts_answers_for_every_word_in_every_block() {
    // 42 two-letter forms in order, more than two blocks of offsets, each with a weight of its own.
    let letters: Vec<char> = TYPO_ALPHABETS.iter().find(|entry| entry.0 == "ru_RU").expect("Russian").1.chars().collect();
    let pairs: Vec<(u8, u8)> = (1..=7_u8).flat_map(|first| (1..=6_u8).map(move |second| (first, second))).collect();
    let forms: Vec<u8> = pairs.iter().flat_map(|&(first, second)| [first, second, 0]).collect();
    let weights: Vec<u8> = (0..42_u8).collect();
    let lexicon = TypoLexicon::load(&written(&typo_file(&|_| {}, [&forms, &weights, &[0]])), "ru_RU").expect("it loads");
    let text = |codes: &[u8]| codes.iter().map(|&code| letters[usize::from(code) - 1]).collect::<String>();
    for (index, &(first, second)) in pairs.iter().enumerate() {
        assert_eq!(lexicon.weight(&text(&[first, second])), Some(i64::from(weights[index])), "form {index}");
    }
    // Before the first form, between forms, after the last one.
    assert_eq!(lexicon.weight(&text(&[1])), None);
    assert_eq!(lexicon.weight(&text(&[3, 3, 3])), None);
    assert_eq!(lexicon.weight(&text(&[7, 7])), None);
    assert_eq!(lexicon.weight(&text(&[8])), None);
    // A list of no words answers for none.
    let empty = TypoLexicon::load(&written(&typo_file(&|_| {}, [&[], &[], &[0]])), "ru_RU").expect("it loads");
    assert_eq!((empty.size(), empty.weight(&text(&[1]))), (0, None));
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

#[test]
fn a_corrector_knows_the_keys_beside_each_key_of_its_languages() {
    use ks_context::typo_correction::{TypoCorrector, TypoPolicy};
    let resources = models().parent().expect("the resources").to_path_buf();
    let lexicon = packaged_lexicon(&resources, "ru_RU").expect("the shipped list");
    let corrector = TypoCorrector::new(vec![("ru_RU".to_owned(), lexicon)], TypoPolicy::default()).expect("a corrector");
    assert!(!corrector.neighbours_of("ru_RU").is_empty());
}

/// A capitals table of the shape `ks-train capitals` writes; `extra` stands for a field the loader does not know.
#[derive(serde::Serialize)]
struct CapitalsTable {
    scope: String,
    sources: BTreeMap<String, String>,
    minimum_count: u64,
    web_cyrillic: BTreeMap<String, i64>,
    web_latin: BTreeMap<String, i64>,
    technical_cyrillic: BTreeMap<String, i64>,
    technical_latin: BTreeMap<String, i64>,
    iso4217: Vec<String>,
    lexicon_capitals: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extra: Option<i64>,
}

/// Counts of tokens, as a table section holds them.
fn counted(pairs: &[(&str, i64)]) -> BTreeMap<String, i64> {
    pairs.iter().map(|&(token, count)| (token.to_owned(), count)).collect()
}

/// Tokens, as a list section holds them.
fn listed(tokens: &[&str]) -> Vec<String> {
    tokens.iter().map(|&token| token.to_owned()).collect()
}

/// A capitals table of the shape `ks-train capitals` writes, with one change.
fn capitals_table(change: impl FnOnce(&mut CapitalsTable)) -> PathBuf {
    let mut table = CapitalsTable {
        scope: "a test".to_owned(),
        sources: BTreeMap::from([("web".to_owned(), "a test".to_owned())]),
        minimum_count: 5,
        web_cyrillic: counted(&[("США", 94523)]),
        web_latin: counted(&[("USD", 467)]),
        technical_cyrillic: counted(&[("НЕ", 2762)]),
        technical_latin: counted(&[("API", 900)]),
        iso4217: listed(&["USD", "KZT"]),
        lexicon_capitals: listed(&["СССР"]),
        extra: None,
    };
    change(&mut table);
    written(ks_text::json::dumps(&table, &ks_text::JsonOptions::default()).as_bytes())
}

#[test]
fn a_table_of_capitals_answers_by_source_and_refuses_another_shape() {
    let counts = CapitalCounts::load(&capitals_table(|_| ())).expect("it loads");
    assert_eq!((counts.web("США"), counts.web("USD"), counts.web("НЕ"), counts.technical("НЕ"), counts.technical("API"), counts.technical("USD")), (94523, 467, 0, 2762, 900, 0));
    assert_eq!(counts.currencies(), ["KZT", "USD"]);
    assert!(counts.currency("KZT") && !counts.currency("ЛЯЕ") && counts.lexicon("СССР") && !counts.lexicon("CCCH"));
    let refused = |change: fn(&mut CapitalsTable)| CapitalCounts::load(&capitals_table(change)).err().expect("refused").message;
    assert_eq!(refused(|table| table.web_latin = counted(&[("usd", 467)])), "invalid capitals frequency table");
    assert_eq!(refused(|table| table.web_latin = counted(&[("U2", 9)])), "invalid capitals frequency table");
    assert_eq!(refused(|table| table.web_cyrillic = counted(&[("USD", 467)])), "invalid capitals frequency table");
    assert_eq!(refused(|table| table.technical_latin = counted(&[("США", 9)])), "invalid capitals frequency table");
    assert_eq!(refused(|table| table.technical_cyrillic = counted(&[("НЕ", 4)])), "invalid capitals frequency table");
    assert_eq!(refused(|table| table.iso4217 = listed(&["Usd"])), "invalid capitals frequency table");
    assert_eq!(refused(|table| table.lexicon_capitals = listed(&["CCCP"])), "invalid capitals frequency table");
    assert!(refused(|table| table.extra = Some(1)).contains("unknown field"));
    assert!(refused(|table| table.web_latin = counted(&[("USD", -1)])).contains("invalid value"));
    let large = capitals_table(|table| table.scope = " ".repeat(MAX_CONTEXT_CAPITALS_FREQUENCY_BYTES));
    assert_eq!(CapitalCounts::load(&large).err().expect("refused").message, "capitals frequency table is too large");
    assert_eq!(CapitalCounts::load(&PathBuf::from("/nonexistent/table.json")).err().expect("refused").name, "FileNotFoundError");
}

/// A folder of models whose capitals table is `table`: the packaged term tables beside it.
fn models_with_capitals(table: Option<&Path>) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let folder = std::env::temp_dir().join(format!("ks-loaders-models-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&folder).expect("the folder");
    for name in ["context-term-frequency.json", "context-alnum-frequency.json", "context-short-frequency.json"] {
        std::fs::copy(models().join(name), folder.join(name)).expect("a packaged table");
    }
    if let Some(table) = table {
        std::fs::copy(table, folder.join(CAPITALS_FREQUENCY_FILE)).expect("the capitals table");
    }
    folder
}

/// A context artifact of the trainer's shape with the weights it is given.
#[derive(serde::Serialize)]
struct Artifact {
    feature_version: i64,
    actions: [&'static str; 4],
    weights: BTreeMap<String, [f64; 4]>,
    weights_sha256: String,
    version: &'static str,
    conversion_threshold: f64,
}

#[test]
fn a_model_with_the_upper_case_head_reads_its_table_before_the_first_word() {
    let weights = BTreeMap::from([(format!("{UPPER_FEATURE_PREFIX}bias:direction:0"), [0.5, -0.5, 0.0, 0.0]), ("f0".to_owned(), [0.5, -0.5, 0.0, 0.0])]);
    let artifact = Artifact {
        feature_version: 3,
        actions: ["keep", "convert", "wait", "suggest"],
        weights_sha256: digest(&weights),
        weights,
        version: "context-v3-test",
        conversion_threshold: 0.99,
    };
    let path = written(ks_text::json::dumps(&artifact, &ks_text::JsonOptions::default()).as_bytes());
    let good = models_with_capitals(Some(&capitals_table(|_| ())));
    assert!(ContextModel::load_artifact(&path, &good).is_ok());
    let missing = models_with_capitals(None);
    assert_eq!(ContextModel::load_artifact(&path, &missing).err().expect("refused").name, "FileNotFoundError");
    // A model without the head reads the table on the first token in capitals, and a table that does not load is no answer.
    assert!(ContextModel::load_artifact(&written(context_artifact().to_string().as_bytes()), &missing).is_ok());
    let tables = TermTables::load(&missing).expect("the term tables");
    assert!(tables.capitals().is_err());
}

#[test]
fn a_model_with_the_common_reading_head_reads_the_short_token_table_before_the_first_word() {
    let weights = BTreeMap::from([(format!("{COMMON_FEATURE_PREFIX}place:line"), [0.5, -0.5, 0.0, 0.0]), ("f0".to_owned(), [0.5, -0.5, 0.0, 0.0])]);
    let artifact = Artifact {
        feature_version: 3,
        actions: ["keep", "convert", "wait", "suggest"],
        weights_sha256: digest(&weights),
        weights,
        version: "context-v3-test",
        conversion_threshold: 0.99,
    };
    let path = written(ks_text::json::dumps(&artifact, &ks_text::JsonOptions::default()).as_bytes());
    assert!(ContextModel::load_artifact(&path, &models_with_capitals(None)).is_ok());
    let without = models_with_capitals(None);
    std::fs::remove_file(without.join("context-short-frequency.json")).expect("the short-token table removed");
    assert_eq!(ContextModel::load_artifact(&path, &without).err().expect("refused").name, "FileNotFoundError");
}

/// A schema-two artifact whose only weight is the bias, as text, with `conversion_thresholds` written as given (none
/// when empty).
fn direction_artifact(bias: [f64; 4], thresholds: &str, scalar: f64) -> PathBuf {
    #[derive(serde::Serialize)]
    struct Artifact {
        kind: &'static str,
        actions: [&'static str; 4],
        feature_version: i64,
        prefix_feature_version: i64,
        weights: BTreeMap<&'static str, [f64; 4]>,
        weights_sha256: String,
        version: String,
        conversion_threshold: f64,
    }
    let weights: BTreeMap<&'static str, [f64; 4]> = [("bias", bias)].into_iter().collect();
    let sum = digest(&weights);
    let version = format!("prefix-v2-{}", sum.get(..12).expect("a hash"));
    let artifact = Artifact { kind: "keyswitch.prefix-policy", actions: ["keep", "convert", "wait", "suggest"], feature_version: 2, prefix_feature_version: 2, weights, weights_sha256: sum, version, conversion_threshold: scalar };
    let text = ks_text::json::dumps(&artifact, &ks_text::JsonOptions::default());
    let text = if thresholds.is_empty() { text } else { text.replacen('{', &format!("{{\"conversion_thresholds\": {thresholds}, "), 1) };
    written(text.as_bytes())
}

#[test]
fn a_prefix_model_with_a_threshold_for_each_direction_converts_at_the_threshold_of_the_keys_typed() {
    use ks_context::context_model::ContextAction;
    use ks_context::prefix_model::{PrefixInput, PrefixSide};
    use ks_lang::Resources;
    // e^5.43 / (e^5.43 + 3): a conversion at 0.987, between the two thresholds.
    let between = [0.0, 5.43, 0.0, 0.0];
    let model = VersionedPrefixModel::load(&direction_artifact(between, "[0.985, 0.99]", 0.99)).expect("a model");
    assert_eq!((model.thresholds(), model.threshold_for(0), model.threshold_for(1)), ([0.985, 0.99], 0.985, 0.99));
    let resources = Resources::new(models().parent().expect("the resources"));
    let (ru, en) = (resources.engine_language_model("ru_RU").expect("ru"), resources.engine_language_model("en_US").expect("en"));
    let (en_index, ru_index) = (resources.prefix_index(&en), resources.prefix_index(&ru));
    let sides = [&PrefixSide { index: &en_index, model: &en }, &PrefixSide { index: &ru_index, model: &ru }];
    let item = |source_group: i64| {
        let (original, alternative) = if source_group == 0 { ("ghbd", "прив") } else { ("руды", "hello") };
        PrefixInput { original: original.to_owned(), alternative: alternative.to_owned(), source_group, field: ks_backend::FieldContext::new("firefox", "1") }
    };
    let latin = model.predict_with(&item(0), sides, None).expect("a prediction");
    assert!(latin.action == ContextAction::Convert && (latin.probability - 0.987).abs() < 0.001, "{latin:?}");
    assert_eq!(model.predict_with(&item(1), sides, None).expect("a prediction").action, ContextAction::Wait);
    // A model that would keep keeps in either direction; one without thresholds has its one threshold in both.
    let keeping = VersionedPrefixModel::load(&direction_artifact([5.43, 0.0, 0.0, 0.0], "[0.985, 0.99]", 0.99)).expect("a model");
    assert_eq!(keeping.predict_with(&item(0), sides, None).expect("a prediction").action, ContextAction::Keep);
    let single = VersionedPrefixModel::load(&direction_artifact(between, "", 0.99)).expect("a model");
    assert_eq!((single.thresholds, single.thresholds()), (None, [0.99, 0.99]));
    assert_eq!(single.predict_with(&item(0), sides, None).expect("a prediction").action, ContextAction::Wait);
    let refusal = |thresholds: &str, scalar: f64| VersionedPrefixModel::load(&direction_artifact(between, thresholds, scalar)).err().map(|error| error.message);
    for unsafe_thresholds in ["[0.5, 0.99]", "[0.99, 1.5]", "[0.99]", "[0.99, 0.99, 0.99]", "\"x\"", "[0.99, \"x\"]", "null"] {
        assert_eq!(refusal(unsafe_thresholds, 0.99).as_deref(), Some("unsafe prefix threshold"), "{unsafe_thresholds}");
    }
    assert_eq!(refusal("[0.985, 0.985]", 0.99).as_deref(), Some("inconsistent prefix thresholds"));
}
