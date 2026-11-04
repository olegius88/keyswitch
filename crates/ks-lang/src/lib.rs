//! The language layer of the detector, ported from the Python with bit-exact parity: lexicons and character models,
//! the orthotactic model, the early-switch rule and the boundary models.
//!
//! Each module is the Python module of the same name. The models read one resources folder given at load time
//! (`Resources`); nothing here derives a path from the executable.

#![forbid(unsafe_code)]

pub mod boundary_model;
pub mod boundary_policy;
pub mod constants;
pub mod early_switch;
pub mod identifier_lexicon;
pub mod language_model;
pub mod lexicon_supplement;
pub mod load_error;
pub mod ortho_model;
pub mod packed_grams;
pub mod pysum;
pub mod resources;
pub mod spellcheck;
pub mod tables;

pub use language_model::{LanguageModel, WordScore};
pub use load_error::LoadError;
pub use pysum::py_sum;
pub use resources::Resources;
pub use spellcheck::{DisabledSpellChecker, FnSpellChecker, SpellChecker};
pub use tables::{WordList, WordTable};
