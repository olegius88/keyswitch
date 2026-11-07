//! Strict GitHub Releases update discovery and verified package download.
//!
//! The network is reached through [`ResponseOpener`], so the logic is tested on a fake and each platform brings its
//! own opener (`WinHTTP` on Windows).
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![forbid(unsafe_code)]
#![expect(clippy::disallowed_types, clippy::disallowed_methods, reason = "temporary: this crate still handles its JSON as untyped trees; step 2c of the strict-json plan types them and removes this")]

pub mod constants;
pub mod updates;

#[cfg(unix)]
pub mod http_curl;
pub use updates::*;
