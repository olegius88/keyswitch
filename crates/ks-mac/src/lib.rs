//! The macOS platform of `KeySwitch`: the Quartz event tap, typing, input sources, the pasteboard, the accessibility field
//! reader, the menu bar item and the settings window.
//!
//! One file per module of the Python it comes from, under the same name. `macos_backend`, `macos_context`,
//! `macos_system` and `macos_main_thread` compile everywhere and are tested through fakes of their small native
//! interfaces; what calls macOS itself compiles on macOS only. `unsafe` is allowed in this crate alone, and each block
//! says why it holds.

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![expect(clippy::disallowed_types, clippy::disallowed_macros, clippy::disallowed_methods, reason = "temporary: this crate still handles its JSON as untyped trees; step 2c of the strict-json plan types them and removes this")]
pub mod constants;
pub mod macos_app;
pub mod macos_backend;
pub mod macos_context;
pub mod macos_main_thread;
pub mod macos_services;
pub mod macos_system;
pub mod macos_tray_native;

#[cfg(target_os = "macos")]
pub mod desktop_app;
#[cfg(target_os = "macos")]
pub mod macos_native;
#[cfg(target_os = "macos")]
pub mod macos_objc;
#[cfg(target_os = "macos")]
pub mod settings_window;
