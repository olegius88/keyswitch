//! `KeySwitch` on Linux: the X11 backend, the field reader, the speller binding and the system helpers.
//!
//! `unsafe` lives in this crate only, each block with the reason it is sound.

pub mod constants;
pub mod keysym_names;
pub mod keysym_table;
pub mod x11_backend;
pub mod x11_native;
pub mod xkb_keymap;
pub mod atspi_bus;
pub mod atspi_context;
pub mod atspi_caret;
pub use ks_hunspell as spellcheck;
pub mod system;
pub mod wayland_desktop;
pub mod wayland_constants;
pub mod wayland_access;
pub mod wayland_helper;
pub mod wayland_backend;
pub mod wayland_native;
pub mod linux_app;
pub mod platform;
pub mod gtk_model;
pub mod tray;
pub mod desktop_backend;
pub use ks_updates::http_curl;
pub mod gtk_prompt;
pub mod gtk_window;
pub mod gtk_app;
