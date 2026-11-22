//! What the process asks of the operating system directly: the local time of the log, the system description of its first
//! line, and the signals that end a run without a window.

#![expect(unsafe_code, reason = "calls into the Linux kernel and system libraries")]
use ks_cast::To as _;
use std::ffi::CStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ks_config::logsetup::{self, LocalTime};

/// The local time, broken down (Python's `time.localtime`).
fn local_time() -> LocalTime {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let seconds = now.as_secs().to::<libc::time_t>();
    // SAFETY: `tm` is a C struct of integers and one pointer (`tm_zone`); all-zero is a valid value of it.
    let mut broken: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers refer to live values owned by this frame; `localtime_r` writes only into `broken`.
    let converted = unsafe { libc::localtime_r(&raw const seconds, &raw mut broken) };
    if converted.is_null() {
        return logsetup::utc_time(now.as_millis().low::<i64>());
    }
    LocalTime {
        year: broken.tm_year + 1900,
        month: (broken.tm_mon + 1).bits::<u32>(),
        day: broken.tm_mday.bits::<u32>(),
        hour: broken.tm_hour.bits::<u32>(),
        minute: broken.tm_min.bits::<u32>(),
        second: broken.tm_sec.bits::<u32>(),
        millisecond: now.subsec_millis(),
    }
}

fn field(bytes: &[libc::c_char]) -> String {
    // SAFETY: `uname` fills each field with a NUL-terminated string that fits its array.
    unsafe { CStr::from_ptr(bytes.as_ptr()) }.to_string_lossy().into_owned()
}

/// `platform.platform()`: `Linux-<release>-<machine>-with-glibc<version>`.
#[must_use]
pub fn platform_description() -> String {
    // SAFETY: `utsname` is a C struct of byte arrays; all-zero is a valid value of it.
    let mut name: libc::utsname = unsafe { std::mem::zeroed() };
    // SAFETY: the pointer refers to a live `utsname` owned by this frame.
    if unsafe { libc::uname(&raw mut name) } != 0 {
        return "Linux".to_owned();
    }
    // SAFETY: returns a pointer to a static NUL-terminated string.
    let glibc = unsafe { CStr::from_ptr(libc::gnu_get_libc_version()) }.to_string_lossy().into_owned();
    format!("{}-{}-{}-with-glibc{glibc}", field(&name.sysname), field(&name.release), field(&name.machine))
}

/// Asks glibc to return the free pages of its heap to the system.
fn trim_heap() -> bool {
    // SAFETY: `malloc_trim` takes a pad size and touches only the allocator's own bookkeeping.
    unsafe { libc::malloc_trim(0) != 0 }
}

/// Fixes the size from which glibc maps a block of the heap on its own (`HEAP_MMAP_THRESHOLD_BYTES`): left to itself it
/// moves the threshold up while the models are read, and the heap of a running program stays larger.
fn fix_mmap_threshold() {
    // SAFETY: `mallopt` takes two integers and only sets a parameter of the allocator, under the allocator's own lock.
    unsafe { libc::mallopt(libc::M_MMAP_THRESHOLD, crate::constants::HEAP_MMAP_THRESHOLD_BYTES) };
}

/// GTK opens OpenGL together with the display, to choose the visual of its windows, though the program spends most of
/// its life with no window at all: on a desktop with the nvidia driver `gtk::init` alone took 15 MiB of private memory
/// and mapped 85 MiB of GL libraries. The windows are drawn by GTK's cairo renderer instead, unless the user switched
/// OpenGL themselves or chose a renderer that needs it.
fn keep_gtk_off_opengl() {
    use crate::constants::{GTK_GL_OFF, GTK_GL_OFF_BEFORE_SWITCH, GTK_GL_SWITCH_MINOR_VERSION, GTK_RENDERER_VARIABLE, GTK_RENDERER_WITHOUT_GL};
    let chosen = [GTK_GL_OFF.0, GTK_GL_OFF_BEFORE_SWITCH.0].iter().any(|name| std::env::var_os(name).is_some());
    let renderer_needs_gl = std::env::var(GTK_RENDERER_VARIABLE).is_ok_and(|renderer| renderer != GTK_RENDERER_WITHOUT_GL);
    if chosen || renderer_needs_gl {
        return;
    }
    let (name, value) = if gtk::minor_version() >= GTK_GL_SWITCH_MINOR_VERSION { GTK_GL_OFF } else { GTK_GL_OFF_BEFORE_SWITCH };
    // SAFETY: called from `install`, at the start of `main`, before the program starts a thread that could read the
    // environment.
    unsafe { std::env::set_var(name, value) };
}

/// The clock and system description of the log, the heap's mapping threshold and GTK's OpenGL.
pub fn install() {
    fix_mmap_threshold();
    keep_gtk_off_opengl();
    ks_config::memory_release::set_trim_hook(trim_heap);
    logsetup::set_local_clock(local_time);
    logsetup::set_platform_description(&platform_description());
}

/// Blocks SIGTERM and SIGINT in this thread, and so in every thread started after it, and returns the waiter for them:
/// it says whether one arrived within the time given. Must run before any other thread starts.
pub fn signal_waiter() -> impl Fn(Duration) -> bool {
    // SAFETY: `sigset_t` is plain data; the set is initialised by `sigemptyset` before use.
    let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
    // SAFETY: `set` is a live `sigset_t`; the signal numbers are valid; `pthread_sigmask` only changes this thread's mask.
    unsafe {
        libc::sigemptyset(&raw mut set);
        libc::sigaddset(&raw mut set, libc::SIGTERM);
        libc::sigaddset(&raw mut set, libc::SIGINT);
        libc::pthread_sigmask(libc::SIG_BLOCK, &raw const set, std::ptr::null_mut());
    }
    move |timeout| {
        let limit = libc::timespec { tv_sec: timeout.as_secs().to::<libc::time_t>(), tv_nsec: timeout.subsec_nanos().to::<libc::c_long>() };
        // SAFETY: `set` and `limit` are live for the call; a null `siginfo` is allowed.
        let signal = unsafe { libc::sigtimedwait(&raw const set, std::ptr::null_mut(), &raw const limit) };
        signal > 0
    }
}

/// The engine without a window, until SIGTERM or SIGINT.
#[must_use]
pub fn run_headless(no_engine: bool) -> i32 {
    let waiter = signal_waiter();
    crate::linux_app::run_headless_with(&waiter, no_engine)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn the_system_is_described_like_python() {
        let description = platform_description();
        assert!(description.starts_with("Linux-") && description.contains("-with-glibc"), "{description}");
    }

    #[test]
    fn the_local_time_is_a_real_date() {
        let time = local_time();
        assert!(time.year >= 2026 && (1..=12).contains(&time.month) && (1..=31).contains(&time.day) && time.hour < 24 && time.millisecond < 1000);
    }
}
