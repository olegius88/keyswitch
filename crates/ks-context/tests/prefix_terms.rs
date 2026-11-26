//! The term lists a prefix model of schema four reads, and the features of that schema: what the lists say of each
//! reading of the keys, each reading's own orthotactic score, and a whole word or term at the start of the typed keys.
use ks_backend::FieldContext;
use ks_context::TermTables;
use ks_context::prefix_model::{PrefixInput, PrefixSide};
use ks_context::prefix_schema::{PrefixReaders, VersionedPrefixModel, features_with_readers};
use ks_context::prefix_terms::{PrefixTerms, TermEvidence, TermMatch};
use ks_lang::Resources;
use ks_lang::early_switch::PrefixLexicon;
use ks_lang::ortho_model::{OrthoModel, artifact_path};
use std::path::PathBuf;

fn resources() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources"))
}

/// Term tables of a few words, in a folder of their own.
fn tables() -> TermTables {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let folder = std::env::temp_dir().join(format!("ks-prefix-terms-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    std::fs::create_dir_all(&folder).expect("a folder");
    let table = r#"{"cyrillic": {"бд": 3, "лямбду": 7, "руддщ": 2}, "english": {}, "latin": {"cntk": 5, "ghbdtn": 12, "github": 100, "gitlab": 30, "Git": 9}, "russian": {}}"#;
    std::fs::write(folder.join("context-term-frequency.json"), table).expect("a table");
    TermTables::load(&folder).expect("the tables")
}

/// `привет` and `hello`: what the keys of `ghbdtn` and `руддщ` read as in the other layout.
fn known(word: &str, group: i64) -> bool {
    matches!((group, word), (1, "привет") | (0, "hello"))
}

#[test]
fn the_lists_say_what_each_term_and_each_longer_one_a_reading_begins_is() {
    let tables = tables();
    let terms = PrefixTerms::build(["kubectl", "gthd"].into_iter(), ["git", "Kubectl"].into_iter(), tables.term("latin"), tables.term("cyrillic"), known);
    // `ghbdtn` and `руддщ` are words of the other layout typed in the wrong one: left out.
    assert_eq!(terms.len(), [6, 2]);
    assert_eq!(terms.lookup("gi", 0), TermMatch::default());
    let git = terms.lookup("GIT", 0);
    assert_eq!(git.exact, Some(TermEvidence { count: 9, identifier: false, protected: true }));
    assert_eq!(git.begins, Some(TermEvidence { count: 100, identifier: false, protected: false }));
    assert_eq!(terms.lookup("kube", 0).begins, Some(TermEvidence { count: 0, identifier: true, protected: true }));
    assert_eq!(terms.lookup("ghbdtn", 0), TermMatch::default());
    assert_eq!(terms.lookup("лям", 1).begins.map(|evidence| evidence.count), Some(7));
    assert_eq!(terms.lookup("руддщ", 1), TermMatch::default());
    assert_eq!(terms.lookup("zzz", 0), TermMatch::default());
    assert_eq!(PrefixTerms::default().len(), [0, 0]);
    // Closed to some tokens, the lists read as if they never held them, in either script.
    let closed = terms.without(&["git".to_owned(), "лямбду".to_owned()].into_iter().collect());
    assert_eq!(closed.len(), [5, 1]);
    assert_eq!(closed.lookup("git", 0).exact, None);
    assert_eq!(closed.lookup("git", 0).begins.map(|evidence| evidence.count), Some(100));
    assert_eq!(closed.lookup("лям", 1), TermMatch::default());
}

#[test]
fn schema_four_reads_the_lists_both_readings_scores_and_a_word_at_the_start() {
    let resources = Resources::new(resources());
    let (en, ru) = (resources.engine_language_model("en_US").expect("en"), resources.engine_language_model("ru_RU").expect("ru"));
    let (en_index, ru_index) = (resources.prefix_index(&en, PrefixLexicon::Plain), resources.prefix_index(&ru, PrefixLexicon::Plain));
    let sides = [&PrefixSide { index: &en_index, model: &en }, &PrefixSide { index: &ru_index, model: &ru }];
    let ortho = OrthoModel::load(&artifact_path(resources.resource_dir()), None, None, Some(resources.clone())).expect("the orthotactic model");
    let tables = tables();
    let terms = PrefixTerms::build(["kubectl"].into_iter(), ["git"].into_iter(), tables.term("latin"), tables.term("cyrillic"), known);
    let item = |original: &str, alternative: &str, source_group: i64| PrefixInput { original: original.to_owned(), alternative: alternative.to_owned(), source_group, field: FieldContext::new("Editor", "1") };
    let both = PrefixReaders { ortho: Some(&ortho), terms: Some(&terms), typed_terms: None };
    let features = |input: &PrefixInput| features_with_readers(input, sides, 4, both).expect("schema four");
    let refused = |readers: PrefixReaders<'_>| features_with_readers(&item("cntk", "стел", 0), sides, 4, readers).expect_err("a refusal").message;
    assert!(refused(PrefixReaders { ortho: None, terms: Some(&terms), typed_terms: None }).contains("orthotactic"));
    assert!(refused(PrefixReaders { ortho: Some(&ortho), terms: None, typed_terms: None }).contains("term lists"));
    // Lists closed to the reading as typed alone: the typed term is not found, the other reading is still looked up.
    let none = PrefixTerms::default();
    let typed_closed = features_with_readers(&item("cntk", "стел", 0), sides, 4, PrefixReaders { typed_terms: Some(&none), ..both }).expect("schema four");
    assert!(!typed_closed.keys().any(|name| name.starts_with("term:typed:")) && features(&item("cntk", "стел", 0)).keys().any(|name| name.starts_with("term:typed:")));
    // A term of the table, with its count; the other reading is no term.
    let cntk = features(&item("cntk", "стел", 0));
    assert!(cntk.contains_key("term:typed:exact") && cntk.get("term:typed:exact:count").is_some_and(|count| *count > 0.0) && !cntk.keys().any(|name| name.starts_with("term:other")));
    assert!(cntk.contains_key("ortho:typed_mean") && cntk.contains_key("ortho:other_mean") && cntk.keys().any(|name| name.starts_with("ortho:both:")));
    assert!(cntk.keys().any(|name| name.starts_with("ortho:typed_step:4:")) && cntk.keys().any(|name| name.starts_with("ortho:other_step:4:")));
    // Typed in Cyrillic, the other reading is the term, which begins a longer one too.
    let git = features(&item("пше", "git", 1));
    assert!(git.contains_key("term:other:exact:protected") && git.contains_key("term:other:begins:count") && git.contains_key("term:other:begins"));
    // A term at the start of the typed keys and a tail that begins a term.
    let compound = features(&item("gitcntk", "пшесте", 0));
    assert!(compound.keys().any(|name| ["head:term:3", "head:word:3"].contains(&name.as_str())), "{compound:?}");
    assert!(compound.contains_key("head:tail:4") && compound.contains_key("head:tail_term"));
    // A tail that only begins a term, and a term of a list without counts.
    let begun = features(&item("gitgith", "пшепшер", 0));
    assert!(begun.contains_key("head:tail_term"), "{begun:?}");
    let listed = features(&item("kubectl", "лгиусед", 0));
    assert!(listed.contains_key("term:typed:exact:identifier") && !listed.contains_key("term:typed:exact:count"), "{listed:?}");
    // An English word at the start, whose tail goes on as English words do.
    let word = features(&item("homework", "рщьуцщкл", 0));
    assert!(word.contains_key("head:word:4") && word.contains_key("head:tail_continues"), "{word:?}");
    // No word at the start of keys that are none, and no reading without keys.
    let nothing = features(&item("xqzvj", "чйяма", 0));
    assert!(!nothing.keys().any(|name| name.starts_with("head:") || name.starts_with("term:")));
    let empty = features(&item("", "", 0));
    assert!(!empty.keys().any(|name| name.starts_with("ortho:typed_mean")));
}

#[test]
fn a_model_of_schema_four_answers_only_with_the_term_lists() {
    let model = ks_context::prefix_model::PrefixModel { weights: ks_context::flat_table::WeightTable::from_entries(vec![("bias", [0.0, 5.43, 0.0, 0.0])]), version: "prefix-v4-000000000000".to_owned(), conversion_threshold: 0.99 };
    let model = VersionedPrefixModel { model, feature_version: 4, thresholds: None };
    assert!(model.needs_ortho() && model.needs_terms());
    // Its index holds every word of the language model, as it was trained.
    assert_eq!(model.prefix_lexicon(), PrefixLexicon::Held);
    let resources = Resources::new(resources());
    let (en, ru) = (resources.engine_language_model("en_US").expect("en"), resources.engine_language_model("ru_RU").expect("ru"));
    let (en_index, ru_index) = (resources.prefix_index(&en, model.prefix_lexicon()), resources.prefix_index(&ru, model.prefix_lexicon()));
    let sides = [&PrefixSide { index: &en_index, model: &en }, &PrefixSide { index: &ru_index, model: &ru }];
    let item = PrefixInput { original: "ghbd".to_owned(), alternative: "прив".to_owned(), source_group: 0, field: FieldContext::new("Editor", "1") };
    let ortho = OrthoModel::load(&artifact_path(resources.resource_dir()), None, None, Some(resources.clone())).expect("the orthotactic model");
    assert!(model.predict_with(&item, sides, Some(&ortho)).is_err());
    let terms = PrefixTerms::default();
    let answer = model.predict_with_readers(&item, sides, PrefixReaders { ortho: Some(&ortho), terms: Some(&terms), typed_terms: None }).expect("an answer");
    assert!((answer.probability - 0.987).abs() < 0.001);
}
