//! The Windows platform of KeySwitch: the keyboard hook, typing, layouts, the clipboard, UI Automation, the registry.
//!
//! One file per module of the Python it comes from, under the same name. `windows_backend`, `windows_system` and the
//! logic of `windows_instance` compile everywhere and are tested through fakes of their small native interfaces; what
//! calls Windows itself compiles on Windows only. `unsafe` is allowed in this crate alone, and each block says why it
//! holds.

pub mod constants;
pub mod log;
pub mod windows_backend;
#[cfg(windows)]
pub mod windows_context;
pub mod windows_instance;
pub mod windows_native;
#[cfg(windows)]
pub mod windows_registry;
pub mod windows_system;
