//! KeySwitch on Linux: the X11 backend, the field reader, the speller binding and the system helpers.
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
pub mod spellcheck;
pub mod system;
pub mod wayland_desktop;
pub mod linux_app;
pub mod platform;
