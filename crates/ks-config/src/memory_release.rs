//! Handing the C heap's free pages back to the system once the models are built.
//!
//! The Python called glibc's `malloc_trim` through `ctypes`. That is a foreign call, which takes `unsafe`, and `unsafe`
//! is forbidden here; the Python's reason for the call is also gone: it freed the pages that the parsed model files
//! (tens of megabytes of dicts and strings) left behind in the interpreter's heap, and the Rust program keeps the
//! models in packed tables and builds no such garbage. So the call is a hook: the platform crate that may use `unsafe`
//! hands in a function that trims the heap (on Linux with glibc), and without one nothing is released and the answer is
//! `false`, as the Python's was on Windows and macOS.

use std::sync::Mutex;

static TRIM: Mutex<Option<fn() -> bool>> = Mutex::new(None);

/// The platform crate sets the function that asks the C library to return the free pages of its heap; it answers
/// whether it asked.
pub fn set_trim_hook(hook: fn() -> bool) {
    *TRIM.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(hook);
}

/// Ask the C library to return the free pages of its heap; whether it was asked.
pub fn release_free_memory() -> bool {
    let hook = *TRIM.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    match hook {
        Some(trim) if cfg!(target_os = "linux") => trim(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_only_through_the_hook() {
        // No hook set yet in this process (the only test that sets one is this one).
        assert!(!release_free_memory());
        set_trim_hook(|| true);
        assert_eq!(release_free_memory(), cfg!(target_os = "linux"));
    }
}
