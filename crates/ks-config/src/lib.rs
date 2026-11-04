//! Settings, history, personal data and the small text commands: everything the engine needs that knows no language model.
//!
//! One file per Python module of the same name; `pycompat` holds the few behaviours of Python's standard library
//! (text files, equality of JSON values, `Path.with_suffix`) that the stores of this crate rely on.
#![forbid(unsafe_code)]

pub mod app_quirks;
pub mod config;
pub mod constants;
pub mod history;
pub mod indicator;
pub mod learning;
pub mod log_privacy;
pub mod logsetup;
pub mod memory_release;
pub mod personal_words;
pub mod pycompat;
pub mod selection_commands;
pub mod selection_text;
pub mod settings_diagnostics;
