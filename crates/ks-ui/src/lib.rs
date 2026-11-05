//! What the settings window and the tray show, apart from any toolkit: shared by the Windows and the macOS
//! platforms (the Python's `windows_ui_model.py`, `tray_model.py`, `desktop_theme.py`). Safe code only.
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![forbid(unsafe_code)]

pub mod constants;
pub mod tray_model;
pub mod windows_ui_model;
