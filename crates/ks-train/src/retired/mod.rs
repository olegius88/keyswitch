//! The rejected model generations. None of them is trained again; their seals and reports stay evidence, and the code
//! that produced them is kept here so that the evidence can be replayed and compared byte for byte with the frozen files.
pub mod boundary_v1;
pub mod context_frames;
pub mod context_v2;
pub mod sources;

use crate::corpus::{checksum, read_json};
use ks_lang::LanguageModel;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

pub const LOCALES: [&str; 2] = ["en_US", "ru_RU"];

/// The reference lexicons of the rejected generations: the pinned Onboard word lists, without the packaged supplements
/// that came later. With `spelling`, the reference morphology too, checked against the digests the config pins.
pub fn reference_models(root: &Path, spelling: bool) -> Result<Vec<Arc<LanguageModel>>, String> {
    let config = read_json(&root.join("model/intent_v1/config.json"))?;
    let languages = &config["sources"]["languages"];
    let dictionaries = &config["external_evaluation"]["hunspell"];
    let frozen = root.join("model/intent_v1/sources/hunspell");
    let mut models = Vec::new();
    for locale in LOCALES {
        let spec = &languages[locale];
        let path = root.join(spec["path"].as_str().ok_or("a lexicon without a path")?);
        if Value::String(checksum(&path)?) != spec["sha256"] {
            return Err("reference lexicon checksum mismatch".to_owned());
        }
        let directory = path.parent().ok_or("a lexicon outside a folder")?.to_path_buf();
        let speller: Box<dyn ks_lang::SpellChecker> =
            if spelling { Box::new(ks_hunspell::HunspellDictionary::in_roots(locale, std::slice::from_ref(&frozen))) } else { LanguageModel::disabled_speller() };
        let model = LanguageModel::load(locale, std::iter::empty::<&str>(), &[directory], speller);
        if spelling {
            let dictionary = std::path::PathBuf::from(model.speller.source());
            if !model.speller.available()
                || Value::String(checksum(&dictionary)?) != dictionaries[locale]["dictionary_sha256"]
                || Value::String(checksum(&dictionary.with_extension("aff"))?) != dictionaries[locale]["affix_sha256"]
            {
                return Err("reference morphology unavailable or changed".to_owned());
            }
        }
        models.push(Arc::new(model));
    }
    Ok(models)
}

/// A JSON object read from a file, or a refusal.
pub fn read_object(path: &Path) -> Result<serde_json::Map<String, Value>, String> {
    match read_json(path)? {
        Value::Object(object) => Ok(object),
        _ => Err(format!("{}: not an object", path.display())),
    }
}
