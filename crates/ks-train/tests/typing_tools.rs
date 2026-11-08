//! What every curriculum reads, at its edges: a word of no letters, a glyph no layout types, a lexicon or a morphology that is
//! not the pinned one, and the capitals the onboard Russian lexicon writes.
#![expect(clippy::disallowed_types, clippy::disallowed_macros, clippy::disallowed_methods, reason = "temporary: these tests still build and read their JSON as untyped trees; step 7 of the strict-json plan types them and removes this")]
mod support;

use ks_lang::Resources;
use ks_train::tools::{Tools, reference_models};
use serde_json::json;
use std::collections::BTreeSet;
use std::sync::{Arc, LazyLock};
use support::{scratch, tree};

static TOOLS: LazyLock<Tools> = LazyLock::new(|| {
    let root = tree();
    let resources: Arc<Resources> = Resources::new(root.join("resources"));
    let portable = reference_models(&root, &resources, false).expect("the lexicons");
    Tools::new(&root, resources, portable, BTreeSet::new()).expect("the tools")
});

#[test]
fn an_empty_word_or_one_no_layout_types_is_never_held() {
    assert!(!TOOLS.deferred_isolated("", "", 0));
    assert!(!TOOLS.lookahead_focus("«", 0));
    assert!(TOOLS.lookahead_focus("on", 0));
}

#[test]
fn the_capitals_of_the_onboard_lexicon_are_its_words_of_capitals() {
    let root = scratch("tools-capitals");
    let sources = root.join("model/intent_v1/sources");
    std::fs::create_dir_all(&sources).expect("a folder");
    let lexicon = "\\data\\\nngram 1=5\n\n\\1-grams:\n-1.0 ДОМ\n-1.0 Я\n-1.0 дом\n-1.0 ДОМ2\n-1.0\n\n\\2-grams:\n-1.0 НА ДОМ\n\n\\end\\\n";
    std::fs::write(sources.join("ru_RU.lm"), lexicon).expect("a lexicon");
    let resources: Arc<Resources> = Resources::new(tree().join("resources"));
    let tools = Tools::new(&root, resources, TOOLS.portable.clone(), BTreeSet::new()).expect("the tools");
    let capitals: BTreeSet<&str> = tools.onboard_capitals().iter().map(String::as_str).collect();
    assert_eq!(capitals, BTreeSet::from(["дом"]));
}

#[test]
fn a_lexicon_that_is_not_the_pinned_one_is_refused() {
    let root = scratch("tools-lexicon");
    std::fs::create_dir_all(root.join("model/intent_v1")).expect("a folder");
    std::fs::write(root.join("en_US.lm"), b"\\data\\\n").expect("a lexicon");
    let config = json!({"sources": {"languages": {"en_US": {"path": "en_US.lm", "sha256": "0"}}}, "external_evaluation": {"hunspell": {}}});
    std::fs::write(root.join("model/intent_v1/config.json"), serde_json::to_vec(&config).expect("json")).expect("a config");
    let resources = Resources::new(tree().join("resources"));
    let said = reference_models(&root, &resources, false).err().expect("a lexicon of another pin");
    assert!(said.contains("reference lexicon checksum mismatch"), "{said}");
}

#[test]
fn a_morphology_that_is_not_the_pinned_one_is_refused() {
    // Whichever dictionary this machine has, if any, it is not one of these pins.
    let root = scratch("tools-morphology");
    std::fs::create_dir_all(root.join("model/intent_v1")).expect("a folder");
    let lexicon = tree().join("model/intent_v1/sources/en_US.lm");
    let pins = json!({"dictionary_sha256": "0", "affix_sha256": "0"});
    let config = json!({"sources": {"languages": {"en_US": {"path": lexicon, "sha256": ks_train::corpus::checksum(&lexicon).expect("a checksum")}}}, "external_evaluation": {"hunspell": {"en_US": pins}}});
    std::fs::write(root.join("model/intent_v1/config.json"), serde_json::to_vec(&config).expect("json")).expect("a config");
    let resources = Resources::new(tree().join("resources"));
    let said = reference_models(&root, &resources, true).err().expect("another morphology");
    assert!(said.contains("reference morphology unavailable or changed"), "{said}");
}
