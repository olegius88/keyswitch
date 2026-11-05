//! The correction engine: the keyboard event state machine and the corrections it orders, ported from the Python with
//! the same names, so that the two can be read side by side.
//!
//! `engine` is one Python module split by concern. `EngineCore` holds the state and is owned by one thread, the worker;
//! `KeySwitchEngine` is the handle the application keeps, and `engine::Shared` is all that another thread may touch.
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![forbid(unsafe_code)]

pub mod constants;
pub mod context_access;
pub mod engine;
pub mod engine_loader;
pub mod rule_editor;
pub mod unicode_category;

pub use engine::{
    CorrectionPlan, EngineCore, EngineError, EngineParts, EngineSnapshot, Hotkey, KeySwitchEngine, LearningPrompt, Shared, Stroke,
};
