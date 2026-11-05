//! Strict GitHub Releases update discovery and verified package download.
//!
//! The network is reached through [`ResponseOpener`], so the logic is tested on a fake and each platform brings its
//! own opener (`WinHTTP` on Windows).
#![forbid(unsafe_code)]

pub mod constants;
pub mod updates;

#[cfg(unix)]
pub mod http_curl;
pub use updates::*;
