//! Loads every model of the engine together, as the engine does (the language layer of `ks-lang`, then this crate's
//! context model, prefix model and typo lists), and prints what each step cost:
//! `/usr/bin/time -v memory_probe <resources>`; the steady RSS is the value printed last, the peak is `VmHWM`.

#![expect(clippy::mem_forget, clippy::string_slice, reason = "a test or a probe stops on what it did not expect")]
use ks_context::context_model::{ContextEvidence, ContextModel};
use ks_context::input_context::FieldContext;
use ks_context::prefix_schema::VersionedPrefixModel;
use ks_context::typo_correction::TypoCorrector;
use ks_lang::ortho_model::OrthoEvidence;
use ks_lang::Resources;

fn status(name: &str) -> String {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|text| text.lines().find(|line| line.starts_with(name)).map(|line| line[name.len() + 1..].trim().to_string()))
        .unwrap_or_default()
}

fn report(step: &str) {
    println!("{step:<32} VmRSS {:>12}  VmHWM {:>12}", status("VmRSS:"), status("VmHWM:"));
}

fn main() {
    let dir = std::env::args().nth(1).expect("the resources folder");
    let only_context = std::env::args().nth(2).as_deref() == Some("context");
    report("start");
    let resources = Resources::new(&dir);
    let mut language = Vec::new();
    if !only_context {
        let en = resources.engine_language_model("en_US").expect("en");
        let ru = resources.engine_language_model("ru_RU").expect("ru");
        let _indexes = (resources.prefix_index(&en), resources.prefix_index(&ru));
        let (ortho, _) = resources.ortho_model();
        let ortho = ortho.expect("ortho");
        let _lexicon = resources.identifier_lexicon().0.expect("identifiers");
        let _policy = resources.boundary_policy().expect("policy");
        ortho.score(&OrthoEvidence { keys: "ghbdtn", shape: "lower", source_script: "en" }, None);
        report("ks-lang models");
        std::mem::forget(ortho);
        language.push((en, ru));
    }
    let models = std::path::Path::new(&dir).join("models");
    let model = ContextModel::load(&models).expect("context model");
    report("context model");
    let prefix = VersionedPrefixModel::default_model(&models).expect("prefix model");
    report("prefix model");
    let typo = TypoCorrector::packaged(std::path::Path::new(&dir), &["en_US", "ru_RU"]).expect("typo lists");
    report("typo lists");
    // The tables built on first use: the endings of the head that asks for them, the letters-and-digits and short tables.
    let mut evidence = ContextEvidence::new("зк", "pr", 1, FieldContext::new("firefox", "1"));
    evidence.field.before = "принимай все ".to_string();
    let prediction = model.predict(&evidence);
    let _ = ContextEvidence::new("м67", "v67", 1, FieldContext::new("firefox", "1"));
    let alnum = model.predict(&ContextEvidence::new("м67", "v67", 1, FieldContext::new("firefox", "1")));
    report("first predictions");
    println!(
        "{} features, {} weights bytes, term tables {} bytes, prefix {}, typo lists {}, {:?} {:?}",
        model.weights.len(),
        model.weights.heap_bytes(),
        model.tables.heap_bytes(),
        prefix.version(),
        typo.lexicons.len(),
        prediction.action,
        alnum.action
    );
    std::mem::forget((model, prefix, typo, language));
}
