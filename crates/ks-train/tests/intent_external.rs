//! The external corpora of the layout-intent model on rows made by hand: the dictionary snapshots, the index of sealed
//! signatures, and the compact form of a frozen development corpus with each way its rows can fail to be one.
mod support;

use ks_lang::LanguageModel;
use ks_lang::intent_model::{CorrectionTrigger, TRIGGERS};
use ks_lang::spellcheck::FnSpellChecker;
use ks_train::intent::dataset::{DatasetBundle, LexicalExample};
use ks_train::intent::external::{self, Corpora, DictionaryProvenance, ExternalCorpus, FrozenFile, SealedSignatureIndex};
use std::collections::BTreeSet;
use support::scratch;

fn example(original: &str, alternative: &str, source_group: i64, trigger: CorrectionTrigger, label: bool, base: &str, kind: &str) -> LexicalExample {
    LexicalExample {
        original: original.to_owned(),
        alternative: alternative.to_owned(),
        source_group,
        target_group: 1 - source_group,
        trigger,
        label,
        weight: 1.0,
        base_signature: base.to_owned(),
        variant_kind: kind.to_owned(),
        source_known: false,
        target_known: false,
        context_delta: 0.0,
        context_group: None,
        frequency: 0,
        protected: false,
        safety: false,
    }
}

/// The rows of one signature, each trigger with both labels, as an unknown-typo corpus holds them.
fn expanded(correct: &str, wrong: &str, group: i64, signature: &str) -> Vec<LexicalExample> {
    let base = format!("hunspell-unknown:{signature}");
    TRIGGERS.iter().flat_map(|trigger| [example(correct, wrong, group, *trigger, false, &base, "hunspell-unknown-deletion"), example(wrong, correct, 1 - group, *trigger, true, &base, "hunspell-unknown-deletion")]).collect()
}

fn provenance(locale: &str) -> DictionaryProvenance {
    let file = FrozenFile { path: String::new(), sha256: String::new(), bytes: 0 };
    DictionaryProvenance { locale: locale.to_owned(), dictionary: file.clone(), affix: file }
}

fn corpus(examples: Vec<LexicalExample>, words_by_group: [usize; 2]) -> ExternalCorpus {
    ExternalCorpus {
        examples,
        words_by_group,
        dictionary_sources: [String::new(), String::new()],
        minimum_words_per_group: 1,
        rejected_sealed_overlaps_by_group: [0, 0],
        exclusion_signature_count: 0,
        exclusion_signature_sha256: String::new(),
        selected_sealed_overlap_count: 0,
        corpus_sha256: String::new(),
        rank_namespace: String::new(),
        choice_namespace: String::new(),
        dictionary_provenance: [provenance("en_US"), provenance("ru_RU")],
    }
}

fn corpora(development: ExternalCorpus, signatures: &[&str]) -> Corpora {
    let index = SealedSignatureIndex { signatures: BTreeSet::new(), sha256: String::new(), split_lexical_signature_count: 0, safety_lexical_signature_count: 0, protected_exact_signature_count: 0 };
    let development_signatures: BTreeSet<String> = signatures.iter().map(|signature| (*signature).to_owned()).collect();
    Corpora { holdout: development.clone(), development, sealed_index: index, development_signatures, holdout_signatures: BTreeSet::new(), combined_exclusions: BTreeSet::new(), frozen_development: None }
}

#[test]
fn a_dictionary_snapshot_is_of_the_files_a_handle_names() {
    let unnamed = LanguageModel::load("en_US", std::iter::empty::<&str>(), &[], Box::new(FnSpellChecker::new("", |_: &str| false)));
    assert!(external::snapshot_of_model(&unnamed, "en_US").expect("a snapshot").words.is_empty());
    let folder = scratch("intent-snapshots");
    let dictionary = folder.join("en_US.dic");
    let named = LanguageModel::load("en_US", std::iter::empty::<&str>(), &[], Box::new(FnSpellChecker::new(dictionary.display().to_string(), |_: &str| false)));
    // Neither file, then the affix file alone, then both.
    let gone = external::snapshot_of_model(&named, "en_US").expect("a snapshot");
    assert!(gone.words.is_empty() && gone.provenance.affix.bytes == 0);
    std::fs::write(folder.join("en_US.aff"), "# no encoding named\nTRY abc\n").expect("a file");
    assert_eq!(external::snapshot_of_model(&named, "en_US").expect("a snapshot").provenance.dictionary.bytes, 0);
    std::fs::write(&dictionary, "4\nhello/S\nworld\nab\no'clock\n").expect("a file");
    let snapshot = external::snapshot_of_model(&named, "en_US").expect("a snapshot");
    assert_eq!(snapshot.words, ["hello", "world"]);
    assert!(snapshot.provenance.payload()["dictionary"]["bytes"].as_u64().is_some_and(|bytes| bytes > 0));
}

#[test]
fn every_row_of_a_sealed_dataset_has_a_signature_the_index_can_prove() {
    let bundle = |rows: Vec<LexicalExample>, safety: Vec<LexicalExample>| DatasetBundle { by_split: vec![rows, Vec::new(), Vec::new(), Vec::new(), Vec::new()], safety, ..DatasetBundle::default() };
    let space = CorrectionTrigger::Space;
    let good_typo = example("hello", "руддщ", 0, space, false, "hunspell-unknown:hello", "hunspell-unknown-deletion");
    let protected = LexicalExample { protected: true, ..example("git", "пше", 0, space, false, "hard:git", "protected") };
    let collision = example("here", "руку", 0, space, false, "here", "lexical_collision");
    let index = external::build_sealed_signature_index(&bundle(vec![good_typo.clone(), protected.clone()], vec![collision])).expect("an index");
    assert_eq!((index.split_lexical_signature_count, index.safety_lexical_signature_count, index.protected_exact_signature_count), (1, 1, 1));
    let refused = |rows: Vec<LexicalExample>, safety: Vec<LexicalExample>, words: &str| {
        let said = external::build_sealed_signature_index(&bundle(rows, safety)).map(|_| ()).unwrap_err();
        assert!(said.contains(words), "{words}: {said}");
    };
    refused(vec![example("hello", "привет", 0, space, false, "hello", "identity")], Vec::new(), "contradictory physical signatures");
    refused(vec![example("", "", 0, space, false, "x", "deletion")], Vec::new(), "contradictory physical signatures");
    refused(vec![LexicalExample { base_signature: "other:hello".to_owned(), ..good_typo.clone() }], Vec::new(), "foreign base signature");
    refused(vec![LexicalExample { base_signature: "hunspell-unknown:".to_owned(), ..good_typo.clone() }], Vec::new(), "does not reproduce");
    refused(vec![LexicalExample { base_signature: "hunspell-unknown:world".to_owned(), ..good_typo.clone() }], Vec::new(), "does not reproduce");
    refused(vec![LexicalExample { alternative: "привет".to_owned(), ..good_typo }], Vec::new(), "rendered sides disagree");
    refused(vec![LexicalExample { protected: false, ..protected.clone() }], Vec::new(), "unsupported dataset row");
    refused(Vec::new(), vec![LexicalExample { base_signature: "git".to_owned(), ..protected }], "unsupported safety row");
}

#[test]
fn a_frozen_development_corpus_is_the_symmetric_expansion_of_its_rows() {
    let mut rows = expanded("hello", "руддщ", 0, "hello");
    rows.extend(expanded("привет", "ghbdtn", 1, "ghbdtn"));
    let good = corpora(corpus(rows.clone(), [1, 1]), &["hello", "ghbdtn"]);
    let frozen = external::frozen_development_corpus(&good).expect("a frozen corpus");
    assert_eq!(frozen["rows"].as_array().map(Vec::len), Some(2));
    assert_eq!(frozen["rows"][0]["correct_group"], serde_json::json!(0));
    let refused = |edit: &dyn Fn(&mut Corpora), words: &str| {
        let mut changed = corpora(corpus(rows.clone(), [1, 1]), &["hello", "ghbdtn"]);
        edit(&mut changed);
        let said = external::frozen_development_corpus(&changed).unwrap_err();
        assert!(said.contains(words), "{words}: {said}");
    };
    let space = TRIGGERS.iter().position(|trigger| *trigger == CorrectionTrigger::Space).expect("space") * 2;
    refused(&|value| { value.development.examples.pop(); }, "invalid signature group");
    refused(&|value| { for row in &mut value.development.examples { row.base_signature = row.base_signature.replace("hunspell-unknown:", "other:"); } }, "invalid signature group");
    refused(&|value| value.development.examples[space].label = true, "duplicate trigger rows");
    refused(&|value| { for row in value.development.examples.iter_mut().take(12) { row.source_group += 2; row.target_group -= 2; } }, "invalid language group");
    refused(&|value| value.development.examples[1].original = "other".to_owned(), "not trigger-symmetric");
    refused(&|value| value.development.examples[space].original = "other".to_owned(), "not trigger-symmetric");
    refused(&|value| value.development_signatures = BTreeSet::from(["hello".to_owned()]), "signatures changed");
    refused(&|value| value.development_signatures = BTreeSet::from(["hello".to_owned(), "other".to_owned()]), "signatures changed");
    refused(&|value| value.development.words_by_group = [2, 0], "group counts are inconsistent");
    assert!(external::preseal_receipt(&serde_json::json!({}), &good).unwrap_err().contains("frozen development source"));
    // A signature of the unknown typos is never empty.
    let empty = corpus(expanded("hello", "руддщ", 0, ""), [1, 0]);
    assert!(external::unknown_typo_physical_signatures(&empty).unwrap_err().contains("empty signature"));
}

/// A word of these letters with one more, whose typos hold the given word.
fn with_typo(target: &str) -> String {
    (1..target.len())
        .flat_map(|place| ('a'..='z').map(move |letter| { let mut word = target.to_owned(); word.insert(place, letter); word }))
        .find(|word| ks_train::intent::dataset::typo_variants(word, 3).iter().any(|(variant, _)| variant == target))
        .expect("a word of one letter more with this typo")
}

fn snapshot(locale: &str, words: &[String]) -> external::Snapshot {
    external::Snapshot { words: words.to_vec(), provenance: provenance(locale) }
}

#[test]
fn an_unknown_typo_is_none_of_the_sealed_ones_and_reads_as_no_known_word_either_way() {
    let (_, models) = ks_train::intent::runtime_models(&support::tree()).expect("the dictionaries");
    let references = [&*models[0], &*models[1]];
    let protected = ks_lang::Resources::new(support::tree().join("resources")).protected_tokens();
    let pair = ks_text::layouts::LayoutPair::default();
    let russian = |keys: &str| pair.translate(keys, "us", "ru").expect("the layouts");
    // A typo that is a protected word typed in either layout, and one that reads as an English word typed in Russian.
    let (protected_word, known_word) = (with_typo("github"), with_typo("hello"));
    let english = vec!["ab".to_owned(), protected_word.clone(), "kldmwnx".to_owned()];
    let others = vec![russian(&protected_word), russian(&known_word), russian("kldmwnx")];
    let mut excluded: BTreeSet<String> = BTreeSet::new();
    for (word, kept) in [(&protected_word, "github"), (&known_word, "hello")] {
        excluded.extend(ks_train::intent::dataset::typo_variants(word, 3).into_iter().skip(1).map(|(variant, _)| variant).filter(|variant| variant != kept));
    }
    let onboard: [std::collections::HashSet<String>; 2] = [std::collections::HashSet::new(), std::collections::HashSet::new()];
    let built = external::build_unknown_typo_disjoint_corpus(&onboard, &excluded, 10, &[snapshot("en_US", &english), snapshot("ru_RU", &others)], &references, &protected, "rank", "choice").expect("a corpus");
    assert!(built.rejected_sealed_overlaps_by_group.iter().all(|count| *count > 0), "{:?}", built.rejected_sealed_overlaps_by_group);
    let chosen = external::unknown_typo_physical_signatures(&built).expect("signatures");
    assert!(!chosen.contains("github") && !chosen.contains("hello"), "{chosen:?}");
    // The same keys in both groups: the second takes a typo the first did not.
    assert_eq!(built.words_by_group, [1, 1], "{chosen:?}");
}
