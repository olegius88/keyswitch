//! The Windows platform of KeySwitch: the keyboard hook, typing, layouts, the clipboard, UI Automation, the registry.
//!
//! One file per module of the Python it comes from, under the same name. `windows_backend`, `windows_system` and the
//! logic of `windows_instance` compile everywhere and are tested through fakes of their small native interfaces; what
//! calls Windows itself compiles on Windows only. `unsafe` is allowed in this crate alone, and each block says why it
//! holds.

pub mod constants;
#[cfg(windows)]
pub mod desktop_app;
pub mod log;
pub mod tray_icon;
pub use ks_ui::tray_model;
pub mod windows_app;
pub mod windows_backend;
#[cfg(windows)]
pub mod windows_context;
#[cfg(windows)]
pub mod tray;
pub mod windows_instance;
pub mod windows_native;
#[cfg(windows)]
pub mod windows_registry;
pub mod windows_system;
pub use ks_ui::windows_ui_model;
#[cfg(windows)]
pub mod learning_prompt;
#[cfg(windows)]
pub mod rule_editor_dialog;
#[cfg(windows)]
pub mod settings_window;
#[cfg(windows)]
pub mod win32_controls;
pub mod winhttp;
