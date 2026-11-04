//! LogCourier: a personal collector of log files into a closed Telegram group.
pub mod batching;
pub mod catalog;
pub mod cli;
pub mod collector;
pub mod config;
pub mod constants;
pub mod error;
pub mod gui_model;
#[cfg(windows)]
pub mod gui_win;
pub mod rate_limit;
pub mod reset;
pub mod secrets;
pub mod service;
pub mod store;
pub mod telegram;
pub mod timeutil;
pub mod versions;
pub mod zipfile;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
