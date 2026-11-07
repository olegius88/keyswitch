//! What the settings window and the tray show, apart from any toolkit: shared by the Windows and the macOS
//! platforms (the Python's `windows_ui_model.py`, `tray_model.py`, `desktop_theme.py`). Safe code only.
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![forbid(unsafe_code)]
#![expect(clippy::disallowed_types, clippy::disallowed_macros, clippy::disallowed_methods, reason = "temporary: this crate still handles its JSON as untyped trees; step 2c of the strict-json plan types them and removes this")]

pub mod constants;
pub mod tray_model;
pub mod windows_ui_model;
