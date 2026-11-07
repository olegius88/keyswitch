//! The rejected model generations. None of them is trained again; their seals and reports stay evidence, and the code
//! that produced them is kept here so that the evidence can be replayed and compared byte for byte with the frozen files.
pub mod boundary_v1;
pub mod context_frames;
pub mod context_v2;
pub mod feature_two;
pub mod ortho_v2;
pub mod prefix_v1;
pub mod sources;

use crate::corpus::{checksum, read_json};
use ks_lang::LanguageModel;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

pub const LOCALES: [&str; 2] = ["en_US", "ru_RU"];

/// The reference lexicons of the rejected generations: the pinned Onboard word lists, without the packaged supplements
/// that came later. With `spelling`, the reference morphology too, checked against the digests the config pins.
pub fn lexicons(root: &Path, spelling: bool) -> Result<Vec<Arc<LanguageModel>>, String> {
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

/// The portable lexicons of the replays.
pub fn reference_models(root: &Path) -> Result<Vec<Arc<LanguageModel>>, String> {
    lexicons(root, false)
}

/// A file rebuilt by a replay is byte for byte the frozen one, or the replay stops with what changed.
pub fn unchanged(made: &[u8], kept: &[u8], refusal: &str) -> Result<(), String> {
    if made == kept { Ok(()) } else { Err(refusal.to_owned()) }
}

/// A JSON object read from a file, or a refusal.
pub fn read_object(path: &Path) -> Result<serde_json::Map<String, Value>, String> {
    match read_json(path)? {
        Value::Object(object) => Ok(object),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => Err(format!("{}: not an object", path.display())),
    }
}

/// Scratch trees for the tests of the replays: links to the repository, with some files of their own.
#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) mod testing {
    use serde_json::Value;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub(crate) fn repository() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("the repository")
    }

    /// A folder of links to the repository's entries. A file written into it is the tree's own: every folder on its way
    /// is made a real folder of links first, and the link of the file itself is removed, so nothing is written through
    /// a link into the repository.
    pub(crate) struct Tree {
        pub(crate) root: PathBuf,
    }

    impl Tree {
        pub(crate) fn new(name: &str) -> Tree {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let root = std::env::temp_dir().join(format!("ks-train-retired-{name}-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).expect("a folder");
            let tree = Tree { root };
            tree.link_entries(&repository(), Path::new(""));
            tree
        }

        fn link_entries(&self, source: &Path, relative: &Path) {
            for entry in std::fs::read_dir(source).expect("a folder of the repository").flatten() {
                let name = entry.file_name();
                if relative.as_os_str().is_empty() && matches!(name.to_str(), Some(".git" | ".t" | "build" | "target" | "dist")) {
                    continue;
                }
                std::os::unix::fs::symlink(entry.path(), self.root.join(relative).join(&name)).expect("a link");
            }
        }

        /// Every folder on the way to this path is the tree's own, and the path itself is no link.
        fn own(&self, relative: &str) -> PathBuf {
            let path = Path::new(relative);
            let mut walked = PathBuf::new();
            for part in path.parent().into_iter().flat_map(Path::components) {
                walked.push(part);
                let here = self.root.join(&walked);
                if here.is_symlink() {
                    let target = std::fs::read_link(&here).expect("a link");
                    std::fs::remove_file(&here).expect("the link goes");
                    std::fs::create_dir(&here).expect("a folder");
                    self.link_entries(&target, &walked);
                } else if !here.exists() {
                    std::fs::create_dir(&here).expect("a folder");
                }
            }
            let file = self.root.join(path);
            if file.is_symlink() {
                std::fs::remove_file(&file).expect("the link goes");
            }
            file
        }

        pub(crate) fn path(&self, relative: &str) -> PathBuf {
            self.root.join(relative)
        }

        pub(crate) fn write(&self, relative: &str, bytes: &[u8]) {
            let file = self.own(relative);
            std::fs::write(file, bytes).expect("a file");
        }

        pub(crate) fn remove(&self, relative: &str) {
            let file = self.own(relative);
            if file.is_dir() {
                std::fs::remove_dir_all(file).expect("the folder goes");
            } else if file.exists() {
                std::fs::remove_file(file).expect("the file goes");
            }
        }

        pub(crate) fn json(&self, relative: &str) -> Value {
            serde_json::from_slice(&std::fs::read(self.path(relative)).expect("a file")).expect("json")
        }

        /// The JSON file changed by `edit` and written back as the tree's own.
        pub(crate) fn edit(&self, relative: &str, edit: impl FnOnce(&mut Value)) {
            let mut value = self.json(relative);
            edit(&mut value);
            self.write(relative, &serde_json::to_vec(&value).expect("json"));
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// What a refusal said, and that it said these words.
    pub(crate) fn refused<T>(result: Result<T, String>, words: &str) {
        match result {
            Ok(_) => panic!("not refused: {words}"),
            Err(said) => assert!(said.contains(words), "{words}: {said}"),
        }
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::testing::{Tree, refused};
    use super::*;
    use serde_json::json;

    #[test]
    fn a_rebuilt_file_is_the_frozen_one_or_the_replay_stops() {
        assert!(unchanged(b"same", b"same", "changed").is_ok());
        refused(unchanged(b"made", b"kept", "the candidate changed"), "the candidate changed");
    }

    #[test]
    fn a_json_file_that_holds_no_object_is_refused() {
        let tree = Tree::new("object");
        for text in ["[1]", "null", "true", "1", "\"text\""] {
            tree.write("value.json", text.as_bytes());
            refused(read_object(&tree.path("value.json")), "value.json: not an object");
        }
        refused(read_object(&tree.path("gone.json")), "gone.json");
    }

    #[test]
    fn a_reference_lexicon_is_the_one_the_config_pins() {
        let tree = Tree::new("lexicons");
        let config = "model/intent_v1/config.json";
        tree.edit(config, |value| drop(value["sources"]["languages"]["en_US"].as_object_mut().expect("a lexicon").remove("path")));
        refused(lexicons(&tree.root, false), "a lexicon without a path");
        tree.edit(config, |value| value["sources"]["languages"]["en_US"] = json!({"path": "model/intent_v1/sources/en_US.lm", "sha256": "0"}));
        refused(lexicons(&tree.root, false), "reference lexicon checksum mismatch");
    }

    #[test]
    fn the_reference_morphology_is_there_and_the_one_the_config_pins() {
        let tree = Tree::new("morphology");
        let config = "model/intent_v1/config.json";
        let kept = tree.json(config);
        for field in ["dictionary_sha256", "affix_sha256"] {
            let mut changed = kept.clone();
            changed["external_evaluation"]["hunspell"]["en_US"][field] = json!("0");
            tree.write(config, &serde_json::to_vec(&changed).expect("json"));
            refused(lexicons(&tree.root, true), "reference morphology unavailable or changed");
        }
        tree.write(config, &serde_json::to_vec(&kept).expect("json"));
        tree.remove("model/intent_v1/sources/hunspell");
        refused(lexicons(&tree.root, true), "reference morphology unavailable or changed");
    }
}
