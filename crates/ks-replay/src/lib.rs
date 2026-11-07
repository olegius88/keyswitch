//! The model editor and the fake clock an engine is typed over, shared by the replay gate and the trainer.
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![forbid(unsafe_code)]
#![expect(clippy::disallowed_macros, clippy::disallowed_methods, reason = "temporary: this crate still handles its JSON as untyped trees; step 3 of the strict-json plan types them and removes this")]

pub mod harness;
