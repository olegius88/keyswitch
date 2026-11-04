//! What the engine and the platform layers share: keys, the backend contract, field and selection readings.
//!
//! It depends on nothing, so a platform crate and the engine can be built apart.
#![forbid(unsafe_code)]

pub mod backend;
pub mod constants;
pub mod field_context;
pub mod input_delay;
pub mod selected_text;

pub use backend::*;
pub use field_context::{FieldContext, FieldReader, FieldRole};
pub use input_delay::{HookGap, InputDelay, InputDelayReport};
pub use selected_text::{RefusalReason, SelectedText, SelectionRefused};
