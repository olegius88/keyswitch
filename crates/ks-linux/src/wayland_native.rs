//! Linux side of the Wayland backend (the Python's `wayland_native.py`): evdev reading and uinput writing through `libc`
//! ioctls, the Wayland client that learns the keymap, the compositor adapters and the production `WaylandApi`.
//!
//! libxkbcommon is loaded at run time with `libloading`, as the Python loads it with ctypes: the compositor sends a
//! flattened keymap in XKB's text format, and reading it exactly (key types, level selection, compose-free keysym to
//! character conversion) is the library's job; `xkb_keymap.rs` parses the X server's binary map, not that text, and the
//! fallback keymap is built from rule files that only xkbcommon reads. The Wayland protocol itself is spoken here by hand
//! (a handful of messages), so there is no libwayland either; the program runs without both libraries, and says why when it
//! needs them.

use ks_cast::To as _;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::FileExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use ks_backend::ScreenAnchor;
use ks_backend::constants::{LOCK_MASK, SHIFT_MASK};
use ks_config::logsetup;
use ks_text::pystr;
use serde_json::Value;
use zbus::blocking::Connection;
use zbus::blocking::connection::Builder;
use zbus::message::Type as MessageType;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};
use zbus::{MatchRule, Message};

use crate::wayland_access::access_hint;
use crate::wayland_backend::{NativeKeyEvent, NativeListener, ReadyCallback, WaylandApi, join_within};
use crate::wayland_constants::*;
use crate::wayland_desktop::*;
use crate::wayland_helper::*;

const LOGGER: &str = "keyswitch.wayland_native";

pub const DEV_INPUT_DIR: &str = "/dev/input";
pub const SYS_INPUT_DIR: &str = "/sys/class/input";
pub const UINPUT_PATH: &str = "/dev/uinput";
pub const VIRTUAL_KEYBOARD_NAME: &[u8] = b"KeySwitch Virtual Keyboard";

pub const KWIN_CALLBACK_PATH: &str = "/io/github/olegius88/KeySwitch/KWin";
pub const KWIN_CALLBACK_IFACE: &str = "io.github.olegius88.KeySwitch.KWin";
pub const KWIN_WATCHER_PLUGIN: &str = "keyswitch_wayland_watcher";
pub const KWIN_ACTION_PLUGIN: &str = "keyswitch_wayland_action";

pub const LOGIND_BUS_NAME: &str = "org.freedesktop.login1";
pub const LOGIND_MANAGER_PATH: &str = "/org/freedesktop/login1";
pub const LOGIND_USER_SELF_PATH: &str = "/org/freedesktop/login1/user/self";
pub const LOGIND_MANAGER_IFACE: &str = "org.freedesktop.login1.Manager";
pub const LOGIND_USER_IFACE: &str = "org.freedesktop.login1.User";
pub const LOGIND_SESSION_IFACE: &str = "org.freedesktop.login1.Session";
pub const DBUS_PROPERTIES_IFACE: &str = "org.freedesktop.DBus.Properties";

const BITS_PER_BYTE: usize = 8;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `CLOCK_MONOTONIC` in seconds: the clock evdev stamps its events with (`EVIOCSCLOCKID`) and Python's `time.monotonic()`.
pub fn clock_monotonic() -> f64 {
    let mut now = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `now` is a valid, writable `timespec` for the call to fill in.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut now) };
    now.tv_sec.to::<f64>() + now.tv_nsec.to::<f64>() / 1e9
}

/// The failure of an operation on a path, worded as Python words an `OSError`: "[Errno 13] Permission denied: '/dev/uinput'".
fn os_error(path: &str, error: &io::Error) -> io::Error {
    let text = error.to_string();
    let description = text.split(" (os error").next().unwrap_or(&text);
    io::Error::new(error.kind(), format!("[Errno {}] {description}: '{path}'", error.raw_os_error().unwrap_or(0)))
}

fn check(result: c_int) -> io::Result<c_int> {
    if result < 0 { Err(io::Error::last_os_error()) } else { Ok(result) }
}

fn ioctl_buffer(fd: RawFd, request: libc::c_ulong, buffer: &mut [u8]) -> io::Result<()> {
    // SAFETY: `request` encodes a buffer of at most `buffer.len()` bytes, and `buffer` stays valid for the whole call.
    check(unsafe { libc::ioctl(fd, request.to::<libc::c_ulong>(), buffer.as_mut_ptr().cast::<c_void>()) }).map(|_| ())
}

fn ioctl_value(fd: RawFd, request: libc::c_ulong, value: libc::c_ulong) -> io::Result<()> {
    // SAFETY: the requests used with this function take their argument by value, not through a pointer.
    check(unsafe { libc::ioctl(fd, request.to::<libc::c_ulong>(), value) }).map(|_| ())
}

/// The numbers of the set bits of a bitmap, least significant bit of the first byte first.
pub fn bits_from_bytes(buffer: &[u8]) -> HashSet<u32> {
    let mut bits = HashSet::new();
    for (index, value) in buffer.iter().enumerate() {
        for bit in 0..BITS_PER_BYTE {
            if (value >> bit) & 1 == 1 {
                bits.insert((index * BITS_PER_BYTE + bit).low::<u32>());
            }
        }
    }
    bits
}

/// The same for sysfs's capability files: hexadecimal words of the machine's `unsigned long`, most significant first.
pub fn bits_from_sysfs(raw: &str) -> HashSet<u32> {
    let tokens = pystr::split(raw);
    let word_bits = std::mem::size_of::<libc::c_ulong>() * BITS_PER_BYTE;
    let mut bits = HashSet::new();
    for (word_index, token) in tokens.iter().rev().enumerate() {
        let Ok(word) = u64::from_str_radix(token, 16) else { continue };
        for bit in 0..word_bits.min(64) {
            if (word >> bit) & 1 == 1 {
                bits.insert((word_index * word_bits + bit).low::<u32>());
            }
        }
    }
    bits
}

fn read_sysfs_text(path: &Path) -> String {
    std::fs::read(path).map(|bytes| pystr::strip(&String::from_utf8_lossy(&bytes)).to_owned()).unwrap_or_default()
}

fn read_sysfs_hex(path: &Path) -> u32 {
    u32::from_str_radix(&read_sysfs_text(path), 16).unwrap_or(0)
}

/// `struct input_event` as the kernel lays it out here: the timestamp, then type, code and value.
fn input_event_bytes(ev_type: u32, code: u32, value: i32) -> Vec<u8> {
    let mut bytes = vec![0u8; std::mem::size_of::<libc::input_event>()];
    let header = std::mem::size_of::<libc::timeval>();
    bytes[header..header + 2].copy_from_slice(&ev_type.low::<u16>().to_ne_bytes());
    bytes[header + 2..header + 4].copy_from_slice(&code.low::<u16>().to_ne_bytes());
    bytes[header + 4..header + 8].copy_from_slice(&value.to_ne_bytes());
    bytes
}

fn decode_input_event(bytes: &[u8]) -> RawInputEvent {
    let word = std::mem::size_of::<libc::time_t>();
    let header = std::mem::size_of::<libc::timeval>();
    let read_word = |from: usize| -> i64 {
        let mut raw = [0u8; 8];
        raw[..word.min(8)].copy_from_slice(&bytes[from..from + word.min(8)]);
        i64::from_ne_bytes(raw)
    };
    RawInputEvent {
        sec: read_word(0),
        usec: read_word(word),
        ev_type: u32::from(u16::from_ne_bytes([bytes[header], bytes[header + 1]])),
        code: u32::from(u16::from_ne_bytes([bytes[header + 2], bytes[header + 3]])),
        value: i32::from_ne_bytes([bytes[header + 4], bytes[header + 5], bytes[header + 6], bytes[header + 7]]),
    }
}

fn read_raw(fd: RawFd, buffer: &mut [u8]) -> io::Result<usize> {
    // SAFETY: `buffer` is valid and writable for `buffer.len()` bytes.
    let count = unsafe { libc::read(fd, buffer.as_mut_ptr().cast::<c_void>(), buffer.len()) };
    if count < 0 { Err(io::Error::last_os_error()) } else { Ok(count.bits::<usize>()) }
}

fn write_raw(fd: RawFd, data: &[u8]) -> io::Result<usize> {
    // SAFETY: `data` is valid for `data.len()` bytes.
    let count = unsafe { libc::write(fd, data.as_ptr().cast::<c_void>(), data.len()) };
    if count < 0 { Err(io::Error::last_os_error()) } else { Ok(count.bits::<usize>()) }
}

fn close_raw(fd: RawFd) -> io::Result<()> {
    // SAFETY: the descriptor belongs to the caller, who does not use it after this call.
    check(unsafe { libc::close(fd) }).map(|_| ())
}

fn open_raw(path: &str, flags: c_int) -> io::Result<RawFd> {
    let c_path = CString::new(path).map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    // SAFETY: `c_path` is a valid NUL-terminated string.
    check(unsafe { libc::open(c_path.as_ptr(), flags) }).map_err(|_| os_error(path, &io::Error::last_os_error()))
}

/// Everything that reads until the descriptor has nothing more (`EAGAIN`) is this loop.
fn read_all_events(fd: RawFd) -> io::Result<Vec<RawInputEvent>> {
    let size = std::mem::size_of::<libc::input_event>();
    let mut events = Vec::new();
    let mut buffer = vec![0u8; size * EVDEV_READ_BATCH_EVENTS.to::<usize>()];
    loop {
        let count = match read_raw(fd, &mut buffer) {
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
            Err(error) => return Err(error),
        };
        if count == 0 {
            break;
        }
        events.extend(buffer[..count].chunks_exact(size).map(decode_input_event));
    }
    Ok(events)
}

/// Real Linux `evdev` / `uinput` / `inotify` / `poll` implementation.
#[derive(Default)]
pub struct LinuxHelperOs;

impl LinuxHelperOs {
    pub fn new() -> LinuxHelperOs {
        LinuxHelperOs
    }

    fn inspect_sysfs(node: &Path) -> Option<InputDeviceInfo> {
        let name = node.file_name()?.to_string_lossy().into_owned();
        let device = Path::new(SYS_INPUT_DIR).join(&name).join("device");
        if !device.is_dir() {
            return None;
        }
        let ev_raw = read_sysfs_text(&device.join("capabilities/ev"));
        if ev_raw.is_empty() {
            return None;
        }
        Some(InputDeviceInfo {
            path: node.to_string_lossy().into_owned(),
            name: read_sysfs_text(&device.join("name")),
            phys: read_sysfs_text(&device.join("phys")),
            bustype: read_sysfs_hex(&device.join("id/bustype")),
            vendor: read_sysfs_hex(&device.join("id/vendor")),
            product: read_sysfs_hex(&device.join("id/product")),
            ev_bits: bits_from_sysfs(&ev_raw),
            key_bits: bits_from_sysfs(&read_sysfs_text(&device.join("capabilities/key"))),
            rel_bits: bits_from_sysfs(&read_sysfs_text(&device.join("capabilities/rel"))),
            abs_bits: bits_from_sysfs(&read_sysfs_text(&device.join("capabilities/abs"))),
        })
    }

    fn inspect_ioctl(node: &Path) -> Option<InputDeviceInfo> {
        let path = node.to_string_lossy().into_owned();
        let fd = open_raw(&path, libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC).ok()?;
        let info = (|| -> io::Result<InputDeviceInfo> {
            let mut id = [0u8; 8];
            ioctl_buffer(fd, EVIOCGID, &mut id)?;
            let text = |request: libc::c_ulong| {
                let mut buffer = vec![0u8; EVDEV_NAME_BUFFER_BYTES.to::<usize>()];
                // A device without a name or a location is not an error.
                let _ = ioctl_buffer(fd, request, &mut buffer);
                let end = buffer.iter().position(|byte| *byte == 0).unwrap_or(buffer.len());
                String::from_utf8_lossy(&buffer[..end]).into_owned()
            };
            let bits = |request: libc::c_ulong, size: usize| {
                let mut buffer = vec![0u8; size];
                let _ = ioctl_buffer(fd, request, &mut buffer);
                bits_from_bytes(&buffer)
            };
            Ok(InputDeviceInfo {
                path: path.clone(),
                name: text(EVIOCGNAME_256),
                phys: text(EVIOCGPHYS_256),
                bustype: u32::from(u16::from_ne_bytes([id[0], id[1]])),
                vendor: u32::from(u16::from_ne_bytes([id[2], id[3]])),
                product: u32::from(u16::from_ne_bytes([id[4], id[5]])),
                ev_bits: bits(EVIOCGBIT_EV_8, EVDEV_SMALL_BITMAP_BYTES.to::<usize>()),
                key_bits: bits(EVIOCGBIT_KEY_96, EVDEV_KEY_BITMAP_BYTES.to::<usize>()),
                rel_bits: bits(EVIOCGBIT_REL_8, EVDEV_SMALL_BITMAP_BYTES.to::<usize>()),
                abs_bits: bits(EVIOCGBIT_ABS_8, EVDEV_SMALL_BITMAP_BYTES.to::<usize>()),
            })
        })();
        let _ = close_raw(fd);
        info.ok()
    }
}

impl HelperOs for LinuxHelperOs {
    fn create_uinput(&mut self) -> io::Result<i32> {
        let fd = open_raw(UINPUT_PATH, libc::O_RDWR | libc::O_NONBLOCK | libc::O_CLOEXEC)?;
        let setup = (|| -> io::Result<()> {
            ioctl_value(fd, UI_SET_EVBIT, libc::c_ulong::from(EV_SYN))?;
            ioctl_value(fd, UI_SET_EVBIT, libc::c_ulong::from(EV_KEY))?;
            for code in 1..=WAYLAND_UINPUT_MAX_KEYCODE {
                ioctl_value(fd, UI_SET_KEYBIT, libc::c_ulong::from(code))?;
            }
            ioctl_value(fd, UI_SET_EVBIT, libc::c_ulong::from(EV_LED))?;
            for led in WAYLAND_FORWARDED_LEDS {
                ioctl_value(fd, UI_SET_LEDBIT, libc::c_ulong::from(led))?;
            }
            // `struct uinput_setup`: the device's id, its name and the force-feedback effect count.
            let mut setup = Vec::with_capacity(92);
            for number in [BUS_I8042, WAYLAND_UINPUT_VENDOR_ID, WAYLAND_UINPUT_PRODUCT_ID, WAYLAND_UINPUT_VERSION] {
                setup.extend_from_slice(&number.low::<u16>().to_ne_bytes());
            }
            let mut name = VIRTUAL_KEYBOARD_NAME.to_vec();
            name.resize(UINPUT_MAX_NAME_BYTES, 0);
            setup.extend_from_slice(&name);
            setup.extend_from_slice(&0u32.to_ne_bytes());
            ioctl_buffer(fd, UI_DEV_SETUP, &mut setup)?;
            ioctl_value(fd, UI_DEV_CREATE, 0)
        })();
        match setup {
            Ok(()) => Ok(fd),
            Err(error) => {
                let _ = close_raw(fd);
                Err(error)
            }
        }
    }

    fn destroy_uinput(&mut self, fd: i32) -> io::Result<()> {
        let destroyed = ioctl_value(fd, UI_DEV_DESTROY, 0);
        let closed = close_raw(fd);
        destroyed.and(closed)
    }

    fn write_uinput(&mut self, fd: i32, events: &[(u32, u32, i32)]) -> io::Result<()> {
        let payload: Vec<u8> = events.iter().flat_map(|&(ev_type, code, value)| input_event_bytes(ev_type, code, value)).collect();
        write_raw(fd, &payload).map(|_| ())
    }

    fn read_uinput_leds(&mut self, fd: i32) -> io::Result<Vec<(u32, i32)>> {
        Ok(read_all_events(fd)?.into_iter().filter(|event| event.ev_type == EV_LED).map(|event| (event.code, event.value)).collect())
    }

    fn list_event_devices(&mut self) -> Vec<InputDeviceInfo> {
        let Ok(entries) = std::fs::read_dir(DEV_INPUT_DIR) else { return Vec::new() };
        let mut nodes: Vec<PathBuf> =
            entries.flatten().map(|entry| entry.path()).filter(|path| path.file_name().is_some_and(|name| name.to_string_lossy().starts_with("event"))).collect();
        nodes.sort();
        nodes.iter().filter_map(|node| LinuxHelperOs::inspect_sysfs(node).or_else(|| LinuxHelperOs::inspect_ioctl(node))).collect()
    }

    fn open_device(&mut self, path: &str, writable: bool) -> io::Result<i32> {
        let flags = (if writable { libc::O_RDWR } else { libc::O_RDONLY }) | libc::O_NONBLOCK | libc::O_CLOEXEC;
        let fd = open_raw(path, flags)?;
        // The kernel stamps events with the clock `monotonic_ms` reads; a device that cannot be asked keeps its own.
        let _ = ioctl_buffer(fd, EVIOCSCLOCKID, &mut libc::CLOCK_MONOTONIC.to::<c_int>().to_ne_bytes());
        Ok(fd)
    }

    fn close_device(&mut self, fd: i32) -> io::Result<()> {
        close_raw(fd)
    }

    fn grab_device(&mut self, fd: i32) -> io::Result<()> {
        ioctl_value(fd, EVIOCGRAB, 1)
    }

    fn ungrab_device(&mut self, fd: i32) -> io::Result<()> {
        ioctl_value(fd, EVIOCGRAB, 0)
    }

    fn query_keys_down(&mut self, fd: i32) -> io::Result<HashSet<u32>> {
        let mut buffer = vec![0u8; EVDEV_KEY_BITMAP_BYTES.to::<usize>()];
        ioctl_buffer(fd, EVIOCGKEY_96, &mut buffer)?;
        Ok(bits_from_bytes(&buffer))
    }

    fn query_caps_lock(&mut self, fd: i32) -> bool {
        let mut buffer = vec![0u8; EVDEV_SMALL_BITMAP_BYTES.to::<usize>()];
        ioctl_buffer(fd, EVIOCGLED_8, &mut buffer).is_ok() && bits_from_bytes(&buffer).contains(&LED_CAPSL)
    }

    fn read_events(&mut self, fd: i32) -> io::Result<Vec<RawInputEvent>> {
        read_all_events(fd)
    }

    fn write_led(&mut self, fd: i32, code: u32, value: i32) -> io::Result<()> {
        write_raw(fd, &input_event_bytes(EV_LED, code, value)).map(|_| ())
    }

    fn open_inotify(&mut self) -> io::Result<i32> {
        // SAFETY: `inotify_init1` takes only flags.
        let fd = check(unsafe { libc::inotify_init1((IN_NONBLOCK | IN_CLOEXEC).bits::<c_int>()) })?;
        let watched = CString::new(DEV_INPUT_DIR).map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error)).and_then(|path| {
            // SAFETY: `path` is a valid NUL-terminated string and `fd` an inotify descriptor.
            check(unsafe { libc::inotify_add_watch(fd, path.as_ptr(), IN_CREATE | IN_ATTRIB | IN_DELETE) })
        });
        if let Err(error) = watched {
            let _ = close_raw(fd);
            return Err(error);
        }
        Ok(fd)
    }

    fn read_inotify(&mut self, fd: i32) -> Vec<String> {
        let mut names = Vec::new();
        let mut buffer = vec![0u8; INOTIFY_READ_BUFFER_BYTES.to::<usize>()];
        loop {
            let count = match read_raw(fd, &mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => count,
            };
            let header = INOTIFY_EVENT_HEADER_BYTES.to::<usize>();
            let mut offset = 0;
            while offset + header <= count {
                // `struct inotify_event`: wd, mask, cookie, then the length of the name that follows.
                let length = u32::from_ne_bytes([buffer[offset + 12], buffer[offset + 13], buffer[offset + 14], buffer[offset + 15]]).to::<usize>();
                offset += header;
                let end = (offset + length).min(count);
                let raw_name = buffer[offset..end].split(|byte| *byte == 0).next().unwrap_or(&[]);
                if !raw_name.is_empty() {
                    names.push(String::from_utf8_lossy(raw_name).into_owned());
                }
                offset += length;
            }
        }
        names
    }

    fn poll(&mut self, fds: &[i32], timeout_ms: i32) -> Vec<(i32, i16)> {
        let mut polled: Vec<libc::pollfd> = fds.iter().map(|fd| libc::pollfd { fd: *fd, events: libc::POLLIN | libc::POLLHUP | libc::POLLERR, revents: 0 }).collect();
        // SAFETY: `polled` is a valid array of `polled.len()` entries.
        let ready = unsafe { libc::poll(polled.as_mut_ptr(), polled.len().to::<libc::nfds_t>(), timeout_ms) };
        if ready <= 0 {
            return Vec::new();
        }
        polled.iter().filter(|entry| entry.revents != 0).map(|entry| (entry.fd, entry.revents)).collect()
    }

    fn monotonic(&mut self) -> f64 {
        clock_monotonic()
    }

    fn sleep(&mut self, seconds_to_wait: f64) {
        thread::sleep(seconds(seconds_to_wait));
    }
}

/// Set by the signal handlers of the helper process; the helper's loop ends after its current step.
static HELPER_STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn request_stop(_signal: c_int) {
    HELPER_STOP.store(true, Ordering::SeqCst);
}

/// Entry point for `keyswitch --wayland-input-helper <fd>`.
pub fn run_wayland_input_helper(socket_fd: i32) -> i32 {
    // SAFETY: the parent passed this descriptor to the process and nothing else owns it.
    let channel = unsafe { UnixStream::from_raw_fd(socket_fd) };
    // The handler only stores into an atomic, which is async-signal-safe. No SA_RESTART: a signal ends the wait in poll.
    for signal in [libc::SIGINT, libc::SIGTERM] {
        // SAFETY: `action` is fully initialised, and the handler is an `extern "C" fn(c_int)`.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            #[expect(clippy::as_conversions, clippy::fn_to_numeric_cast_any, reason = "libc declares the handler as an address, and a function has no safe way to become one")]
            {
                action.sa_sigaction = request_stop as extern "C" fn(c_int) as usize;
            }
            libc::sigemptyset(&mut action.sa_mask);
            libc::sigaction(signal, &action, std::ptr::null_mut());
        }
    }
    // SAFETY: ignoring a signal is a plain disposition change.
    unsafe { libc::signal(libc::SIGTSTP, libc::SIG_IGN) };
    let status = WaylandInputHelper::new(&channel, Box::new(LinuxHelperOs::new()), &HELPER_STOP, WAYLAND_REPEAT_DEFAULT_DELAY_MS).run();
    let _ = channel.shutdown(std::net::Shutdown::Both);
    status
}

// --- Wayland wl_keyboard + libxkbcommon monitor ---

#[repr(C)]
struct XkbRuleNames {
    rules: *const c_char,
    model: *const c_char,
    layout: *const c_char,
    variant: *const c_char,
    options: *const c_char,
}

/// The functions of libxkbcommon the monitor calls, resolved once. The library stays loaded as long as this value lives.
struct XkbLibrary {
    context_new: unsafe extern "C" fn(c_int) -> *mut c_void,
    context_unref: unsafe extern "C" fn(*mut c_void),
    keymap_new_from_string: unsafe extern "C" fn(*mut c_void, *const c_char, c_int, c_int) -> *mut c_void,
    keymap_new_from_names: unsafe extern "C" fn(*mut c_void, *const XkbRuleNames, c_int) -> *mut c_void,
    keymap_unref: unsafe extern "C" fn(*mut c_void),
    keymap_num_layouts: unsafe extern "C" fn(*mut c_void) -> u32,
    keymap_layout_get_name: unsafe extern "C" fn(*mut c_void, u32) -> *const c_char,
    keymap_mod_get_index: unsafe extern "C" fn(*mut c_void, *const c_char) -> u32,
    state_new: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    state_unref: unsafe extern "C" fn(*mut c_void),
    state_update_mask: unsafe extern "C" fn(*mut c_void, u32, u32, u32, u32, u32, u32) -> c_int,
    state_key_get_utf32: unsafe extern "C" fn(*mut c_void, u32) -> u32,
    state_key_get_one_sym: unsafe extern "C" fn(*mut c_void, u32) -> u32,
    keysym_get_name: unsafe extern "C" fn(u32, *mut c_char, usize) -> c_int,
    _library: libloading::Library,
}

impl XkbLibrary {
    fn load() -> Result<XkbLibrary, String> {
        // SAFETY: loading libxkbcommon runs its initialisers, which have no preconditions.
        let library = unsafe { libloading::Library::new("libxkbcommon.so.0") }.map_err(|error| error.to_string())?;
        macro_rules! symbol {
            ($name:literal) => {
                // SAFETY: the symbol has the signature of the field it is stored in (xkbcommon.h), and the library outlives it
                // because it is stored beside the pointer.
                *unsafe { library.get($name) }.map_err(|error| error.to_string())?
            };
        }
        Ok(XkbLibrary {
            context_new: symbol!(b"xkb_context_new\0"),
            context_unref: symbol!(b"xkb_context_unref\0"),
            keymap_new_from_string: symbol!(b"xkb_keymap_new_from_string\0"),
            keymap_new_from_names: symbol!(b"xkb_keymap_new_from_names\0"),
            keymap_unref: symbol!(b"xkb_keymap_unref\0"),
            keymap_num_layouts: symbol!(b"xkb_keymap_num_layouts\0"),
            keymap_layout_get_name: symbol!(b"xkb_keymap_layout_get_name\0"),
            keymap_mod_get_index: symbol!(b"xkb_keymap_mod_get_index\0"),
            state_new: symbol!(b"xkb_state_new\0"),
            state_unref: symbol!(b"xkb_state_unref\0"),
            state_update_mask: symbol!(b"xkb_state_update_mask\0"),
            state_key_get_utf32: symbol!(b"xkb_state_key_get_utf32\0"),
            state_key_get_one_sym: symbol!(b"xkb_state_key_get_one_sym\0"),
            keysym_get_name: symbol!(b"xkb_keysym_get_name\0"),
            _library: library,
        })
    }
}

/// The keymap in force and what was already asked of it.
struct XkbCore {
    keymap: *mut c_void,
    state: *mut c_void,
    shift_mod: i64,
    lock_mod: i64,
    layout_names: Vec<String>,
    /// Answers of the keymap, kept until it changes: every key the helper asks about is translated for each layout.
    translations: HashMap<(u32, u32, i64), String>,
    symbols: HashMap<(u32, i64), String>,
}

// SAFETY: the pointers are libxkbcommon objects used only while the surrounding mutex is held, and the library has no thread
// affinity for them.
unsafe impl Send for XkbCore {}

type RepeatCallback = Box<dyn Fn(u32) + Send + Sync>;

struct XkbShared {
    xkb: XkbLibrary,
    context: *mut c_void,
    core: Mutex<XkbCore>,
    active_group: AtomicU32,
    repeat_delay_ms: AtomicU32,
    keymap_ready: Flag,
    on_repeat_delay: Option<RepeatCallback>,
}

// SAFETY: `context` is only handed to libxkbcommon calls that create keymaps, which the library allows from any thread
// when it is not used concurrently; those calls are made with the `core` mutex held or before the threads exist.
unsafe impl Send for XkbShared {}
// SAFETY: see above; every other field is `Sync` itself.
unsafe impl Sync for XkbShared {}

impl XkbShared {
    fn install_keymap(&self, keymap: *mut c_void) {
        // SAFETY: `keymap` is a keymap just created by this library.
        let state = unsafe { (self.xkb.state_new)(keymap) };
        if state.is_null() {
            // SAFETY: the keymap is ours and unused.
            unsafe { (self.xkb.keymap_unref)(keymap) };
            return;
        }
        // SAFETY: `keymap` is valid; the layout count and names are read from it.
        let count = unsafe { (self.xkb.keymap_num_layouts)(keymap) };
        let names: Vec<String> = (0..count)
            .map(|index| {
                // SAFETY: `index` is below the layout count; the name is NUL-terminated and owned by the keymap.
                let raw = unsafe { (self.xkb.keymap_layout_get_name)(keymap, index) };
                if raw.is_null() {
                    format!("layout-{index}")
                } else {
                    // SAFETY: `raw` is a valid C string for as long as the keymap lives, and it is copied here.
                    unsafe { CStr::from_ptr(raw) }.to_string_lossy().into_owned()
                }
            })
            .collect();
        let modifier = |name: &CStr| {
            // SAFETY: `keymap` is valid and `name` NUL-terminated.
            let index = unsafe { (self.xkb.keymap_mod_get_index)(keymap, name.as_ptr()) };
            if index == UINT32_MODULO_MASK { -1 } else { i64::from(index) }
        };
        let (shift_mod, lock_mod) = (modifier(c"Shift"), modifier(c"Lock"));
        let (old_state, old_keymap) = {
            let mut core = lock(&self.core);
            let old = (core.state, core.keymap);
            *core = XkbCore { keymap, state, shift_mod, lock_mod, layout_names: names, translations: HashMap::new(), symbols: HashMap::new() };
            old
        };
        if !old_state.is_null() {
            // SAFETY: the old state was created by `state_new` and nothing uses it any more.
            unsafe { (self.xkb.state_unref)(old_state) };
        }
        if !old_keymap.is_null() {
            // SAFETY: the same for the old keymap.
            unsafe { (self.xkb.keymap_unref)(old_keymap) };
        }
        self.keymap_ready.set();
    }

    fn install_from_text(&self, text: &[u8]) {
        let Ok(c_text) = CString::new(text) else { return };
        // SAFETY: `context` is valid and `c_text` NUL-terminated.
        let keymap = unsafe { (self.xkb.keymap_new_from_string)(self.context, c_text.as_ptr(), WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1.bits::<c_int>(), 0) };
        if !keymap.is_null() {
            self.install_keymap(keymap);
        }
    }
}

/// Connects to `WAYLAND_DISPLAY` to track the live `wl_keyboard` XKB keymap.
pub struct WaylandXkbMonitor {
    shared: Arc<XkbShared>,
    wake: Mutex<Option<UnixStream>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl WaylandXkbMonitor {
    pub fn new(on_repeat_delay: Option<RepeatCallback>) -> Result<WaylandXkbMonitor, String> {
        let xkb = XkbLibrary::load()?;
        // SAFETY: the function takes only flags.
        let context = unsafe { (xkb.context_new)(0) };
        if context.is_null() {
            return Err("Не удалось инициализировать libxkbcommon".to_owned());
        }
        Ok(WaylandXkbMonitor {
            shared: Arc::new(XkbShared {
                xkb,
                context,
                core: Mutex::new(XkbCore {
                    keymap: std::ptr::null_mut(),
                    state: std::ptr::null_mut(),
                    shift_mod: -1,
                    lock_mod: -1,
                    layout_names: Vec::new(),
                    translations: HashMap::new(),
                    symbols: HashMap::new(),
                }),
                active_group: AtomicU32::new(0),
                repeat_delay_ms: AtomicU32::new(WAYLAND_REPEAT_DEFAULT_DELAY_MS),
                keymap_ready: Flag::default(),
                on_repeat_delay,
            }),
            wake: Mutex::new(None),
            thread: Mutex::new(None),
        })
    }

    pub fn layout_names(&self) -> Vec<String> {
        lock(&self.shared.core).layout_names.clone()
    }

    pub fn active_group(&self) -> i64 {
        i64::from(self.shared.active_group.load(Ordering::SeqCst))
    }

    pub fn repeat_delay_ms(&self) -> u32 {
        self.shared.repeat_delay_ms.load(Ordering::SeqCst)
    }

    pub fn has_keymap(&self) -> bool {
        !lock(&self.shared.core).keymap.is_null()
    }

    pub fn start(&self) {
        if lock(&self.thread).is_some() {
            return;
        }
        let Ok((wake_reader, wake_writer)) = UnixStream::pair() else {
            self.shared.keymap_ready.set();
            return;
        };
        *lock(&self.wake) = Some(wake_writer);
        let shared = self.shared.clone();
        let spawned = thread::Builder::new().name("keyswitch-wayland-xkb".to_owned()).spawn(move || wayland_loop(&shared, &wake_reader));
        match spawned {
            Ok(handle) => *lock(&self.thread) = Some(handle),
            Err(_) => self.shared.keymap_ready.set(),
        }
        self.shared.keymap_ready.wait(seconds(WAYLAND_KEYMAP_WAIT_SECONDS));
    }

    /// Build a keymap from the layouts the desktop names when the compositor sent none (or fewer).
    pub fn ensure_fallback_keymap(&self, layouts: &[String]) {
        {
            let core = lock(&self.shared.core);
            if !core.keymap.is_null() && core.layout_names.len() >= layouts.len() {
                return;
            }
        }
        let mut codes: Vec<String> = Vec::new();
        let mut variants: Vec<String> = Vec::new();
        for entry in layouts {
            if let Some((code, variant)) = entry.split_once('+') {
                codes.push(code.to_owned());
                variants.push(variant.to_owned());
            } else if pystr::isascii(entry) && pystr::islower(entry) && entry.chars().count() <= BITS_PER_BYTE && !entry.contains(' ') {
                // An XKB layout code (`us`, `ru`); sway names layouts by description (`Russian`).
                codes.push(entry.clone());
                variants.push(String::new());
            }
        }
        if codes.is_empty() {
            codes = vec!["us".to_owned(), "ru".to_owned()];
            variants = vec![String::new(), String::new()];
        }
        let (Ok(layout), Ok(variant)) = (CString::new(codes.join(",")), CString::new(variants.join(","))) else { return };
        let names = XkbRuleNames { rules: c"evdev".as_ptr(), model: c"pc105".as_ptr(), layout: layout.as_ptr(), variant: variant.as_ptr(), options: c"".as_ptr() };
        // SAFETY: `context` is valid, and `names` points to strings that live through the call.
        let keymap = unsafe { (self.shared.xkb.keymap_new_from_names)(self.shared.context, &names, 0) };
        if !keymap.is_null() {
            self.shared.install_keymap(keymap);
        }
    }

    pub fn translate_key(&self, keycode: u32, state: u32, layout_index: i64) -> String {
        let key = (keycode, state & (SHIFT_MASK | LOCK_MASK), layout_index);
        let mut core = lock(&self.shared.core);
        if let Some(cached) = core.translations.get(&key) {
            return cached.clone();
        }
        if core.state.is_null() {
            return String::new();
        }
        let depressed = if state & SHIFT_MASK != 0 && core.shift_mod >= 0 { 1u32 << core.shift_mod } else { 0 };
        let locked = if state & LOCK_MASK != 0 && core.lock_mod >= 0 { 1u32 << core.lock_mod } else { 0 };
        // SAFETY: the state belongs to the keymap in `core`, which the mutex guards; the arguments are plain numbers.
        let codepoint = unsafe {
            (self.shared.xkb.state_update_mask)(core.state, depressed, 0, locked, 0, 0, layout_index.max(0).low::<u32>());
            (self.shared.xkb.state_key_get_utf32)(core.state, keycode + WAYLAND_XKB_KEYCODE_OFFSET)
        };
        let character = if codepoint > 0 { char::from_u32(codepoint).map(String::from).unwrap_or_default() } else { String::new() };
        let result = if pystr::isprintable(&character) { character } else { String::new() };
        core.translations.insert(key, result.clone());
        result
    }

    pub fn keysym_name(&self, keycode: u32, layout_index: i64) -> String {
        let key = (keycode, layout_index);
        let mut core = lock(&self.shared.core);
        if let Some(cached) = core.symbols.get(&key) {
            return cached.clone();
        }
        if core.state.is_null() {
            return String::new();
        }
        // SAFETY: as in `translate_key`.
        let symbol = unsafe {
            (self.shared.xkb.state_update_mask)(core.state, 0, 0, 0, 0, 0, layout_index.max(0).low::<u32>());
            (self.shared.xkb.state_key_get_one_sym)(core.state, keycode + WAYLAND_XKB_KEYCODE_OFFSET)
        };
        let mut name = String::new();
        if symbol > 0 {
            let mut buffer: [c_char; XKB_KEYSYM_NAME_BUFFER_BYTES] = [0; XKB_KEYSYM_NAME_BUFFER_BYTES];
            // SAFETY: `buffer` has the size passed, and the function NUL-terminates what it writes.
            let written = unsafe { (self.shared.xkb.keysym_get_name)(symbol, buffer.as_mut_ptr(), buffer.len()) };
            if written > 0 {
                // SAFETY: the buffer holds a NUL-terminated string written just now.
                name = unsafe { CStr::from_ptr(buffer.as_ptr()) }.to_string_lossy().into_owned();
            }
        }
        core.symbols.insert(key, name.clone());
        name
    }

    pub fn close(&self) {
        if let Some(mut wake) = lock(&self.wake).take() {
            let _ = wake.write_all(&[1]);
        }
        if let Some(handle) = lock(&self.thread).take()
            && handle.thread().id() != thread::current().id()
        {
            join_within(handle, WAYLAND_IPC_TIMEOUT_SECONDS);
        }
        let mut core = lock(&self.shared.core);
        if !core.state.is_null() {
            // SAFETY: created by `state_new`, released once.
            unsafe { (self.shared.xkb.state_unref)(core.state) };
            core.state = std::ptr::null_mut();
        }
        if !core.keymap.is_null() {
            // SAFETY: created by this library, released once.
            unsafe { (self.shared.xkb.keymap_unref)(core.keymap) };
            core.keymap = std::ptr::null_mut();
        }
    }
}

impl Drop for XkbShared {
    fn drop(&mut self) {
        if !self.context.is_null() {
            // SAFETY: the context was created in `new` and is released once, after every keymap made from it.
            unsafe { (self.xkb.context_unref)(self.context) };
        }
    }
}

/// The part of the Wayland wire protocol the monitor speaks: objects are numbers, a message is a header of object and
/// size/opcode followed by arguments, and file descriptors travel beside the bytes.
pub struct WlSocket {
    stream: UnixStream,
    input: Vec<u8>,
    descriptors: VecDeque<OwnedFd>,
    next_id: u32,
}

/// One event: the object it is for, its opcode and its arguments.
#[derive(Debug, PartialEq, Eq)]
pub struct WlMessage {
    pub object: u32,
    pub opcode: u16,
    pub payload: Vec<u8>,
}

pub const WL_DISPLAY: u32 = 1;
const WL_DISPLAY_SYNC: u16 = 0;
const WL_DISPLAY_GET_REGISTRY: u16 = 1;
const WL_REGISTRY_BIND: u16 = 0;
const WL_SEAT_GET_KEYBOARD: u16 = 1;

/// The path of the compositor's socket: `WAYLAND_DISPLAY` in `XDG_RUNTIME_DIR` (or a path of its own), `wayland-0` if unset.
pub fn wayland_socket_path(display: Option<&str>, runtime_dir: Option<&str>) -> Option<PathBuf> {
    let display = display.filter(|name| !name.is_empty()).unwrap_or("wayland-0");
    if display.starts_with('/') {
        return Some(PathBuf::from(display));
    }
    Some(Path::new(runtime_dir.filter(|dir| !dir.is_empty())?).join(display))
}

fn wl_string(text: &str) -> Vec<u8> {
    let length = text.len() + 1;
    let mut bytes = length.low::<u32>().to_ne_bytes().to_vec();
    bytes.extend_from_slice(text.as_bytes());
    bytes.push(0);
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
    bytes
}

fn wl_message(object: u32, opcode: u16, arguments: &[u8]) -> Vec<u8> {
    let size = (8 + arguments.len()).low::<u32>();
    let mut bytes = object.to_ne_bytes().to_vec();
    bytes.extend_from_slice(&((size << 16) | u32::from(opcode)).to_ne_bytes());
    bytes.extend_from_slice(arguments);
    bytes
}

/// A number at `offset` of an event's arguments.
fn wl_u32(payload: &[u8], offset: usize) -> Option<u32> {
    payload.get(offset..offset + 4).map(|bytes| u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// A string at `offset` of an event's arguments, and where it ends.
fn wl_read_string(payload: &[u8], offset: usize) -> Option<(String, usize)> {
    let length = (wl_u32(payload, offset)?).to::<usize>();
    let start = offset + 4;
    let text = payload.get(start..start + length)?;
    let content = text.split(|byte| *byte == 0).next().unwrap_or(&[]);
    Some((String::from_utf8_lossy(content).into_owned(), start + length.div_ceil(4) * 4))
}

impl WlSocket {
    pub fn connect(path: &Path) -> io::Result<WlSocket> {
        Ok(WlSocket { stream: UnixStream::connect(path)?, input: Vec::new(), descriptors: VecDeque::new(), next_id: 1 })
    }

    pub fn from_stream(stream: UnixStream) -> WlSocket {
        WlSocket { stream, input: Vec::new(), descriptors: VecDeque::new(), next_id: 1 }
    }

    pub fn fileno(&self) -> RawFd {
        self.stream.as_raw_fd()
    }

    /// A new object id of the client's range.
    pub fn allocate(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }

    pub fn send(&mut self, object: u32, opcode: u16, arguments: &[u8]) -> io::Result<()> {
        self.stream.write_all(&wl_message(object, opcode, arguments))
    }

    /// Reads what the compositor sent, with the descriptors that came with it; 0 bytes mean it closed the connection.
    pub fn receive(&mut self) -> io::Result<usize> {
        let mut data = [0u8; 4096];
        let mut control = [0u8; 256];
        let mut io_vector = libc::iovec { iov_base: data.as_mut_ptr().cast::<c_void>(), iov_len: data.len() };
        // SAFETY: an all-zero `msghdr` is a valid empty message header.
        let mut header: libc::msghdr = unsafe { std::mem::zeroed() };
        header.msg_iov = &mut io_vector;
        header.msg_iovlen = 1;
        header.msg_control = control.as_mut_ptr().cast::<c_void>();
        header.msg_controllen = control.len();
        // SAFETY: `header` points to buffers that stay alive and are as large as it says.
        let count = (check((unsafe { libc::recvmsg(self.stream.as_raw_fd(), &mut header, libc::MSG_CMSG_CLOEXEC) }).low::<c_int>())?).to::<usize>();
        // SAFETY: `header` was filled in by the kernel; the control-message macros walk only what it reports.
        unsafe {
            let mut message = libc::CMSG_FIRSTHDR(&header);
            while !message.is_null() {
                if (*message).cmsg_level == libc::SOL_SOCKET && (*message).cmsg_type == libc::SCM_RIGHTS {
                    let payload = ((*message).cmsg_len).to::<usize>() - libc::CMSG_LEN(0).to::<usize>();
                    let data_pointer = libc::CMSG_DATA(message).cast::<RawFd>();
                    for index in 0..payload / std::mem::size_of::<RawFd>() {
                        // SAFETY (of the descriptor): the kernel installed it in this process for this message.
                        self.descriptors.push_back(OwnedFd::from_raw_fd(data_pointer.add(index).read_unaligned()));
                    }
                }
                message = libc::CMSG_NXTHDR(&header, message);
            }
        }
        self.input.extend_from_slice(&data[..count]);
        Ok(count)
    }

    /// The next complete event, if the bytes read so far hold one.
    pub fn next_message(&mut self) -> Option<WlMessage> {
        let header = self.input.get(..8)?;
        let object = u32::from_ne_bytes([header[0], header[1], header[2], header[3]]);
        let word = u32::from_ne_bytes([header[4], header[5], header[6], header[7]]);
        let size = (word >> 16).to::<usize>();
        if size < 8 || self.input.len() < size {
            return None;
        }
        let payload = self.input[8..size].to_vec();
        self.input.drain(..size);
        Some(WlMessage { object, opcode: (word & 0xFFFF).to::<u16>(), payload })
    }

    pub fn take_descriptor(&mut self) -> Option<OwnedFd> {
        self.descriptors.pop_front()
    }

    /// Waits until the socket can be read, up to `timeout_ms`.
    fn wait_readable(&self, wake: Option<RawFd>, timeout_ms: i32) -> (bool, bool) {
        let mut fds = vec![libc::pollfd { fd: self.fileno(), events: libc::POLLIN | libc::POLLERR | libc::POLLHUP, revents: 0 }];
        if let Some(wake) = wake {
            fds.push(libc::pollfd { fd: wake, events: libc::POLLIN, revents: 0 });
        }
        // SAFETY: `fds` is a valid array of `fds.len()` entries.
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), fds.len().to::<libc::nfds_t>(), timeout_ms) };
        if ready <= 0 {
            return (false, false);
        }
        (fds[0].revents != 0, fds.get(1).is_some_and(|entry| entry.revents != 0))
    }
}

/// What the monitor does with the client's state while it reads events.
#[derive(Default)]
struct WlClientState {
    seat: Option<u32>,
    keyboard: Option<u32>,
    seat_version: u32,
}

fn dispatch_event(shared: &XkbShared, socket: &mut WlSocket, state: &mut WlClientState, message: &WlMessage) -> io::Result<()> {
    const REGISTRY: u32 = 2;
    if message.object == WL_DISPLAY && message.opcode == 0 {
        // wl_display.error: the compositor is about to close the connection.
        return Err(io::Error::other("wl_display error"));
    }
    if message.object == REGISTRY && message.opcode == 0 {
        // wl_registry.global(name, interface, version)
        let (Some(name), Some((interface, after))) = (wl_u32(&message.payload, 0), wl_read_string(&message.payload, 4)) else { return Ok(()) };
        let version = wl_u32(&message.payload, after).unwrap_or(1);
        if interface == "wl_seat" && state.seat.is_none() {
            let bound_version = version.min(WAYLAND_SEAT_INTERFACE_MAX_VERSION);
            let seat = socket.allocate();
            let mut arguments = name.to_ne_bytes().to_vec();
            arguments.extend(wl_string("wl_seat"));
            arguments.extend(bound_version.to_ne_bytes());
            arguments.extend(seat.to_ne_bytes());
            socket.send(REGISTRY, WL_REGISTRY_BIND, &arguments)?;
            state.seat = Some(seat);
            state.seat_version = bound_version;
        }
    } else if Some(message.object) == state.seat && message.opcode == 0 {
        // wl_seat.capabilities(bits)
        let capabilities = wl_u32(&message.payload, 0).unwrap_or(0);
        if capabilities & WL_SEAT_CAPABILITY_KEYBOARD != 0 && state.keyboard.is_none() {
            let keyboard = socket.allocate();
            socket.send(message.object, WL_SEAT_GET_KEYBOARD, &keyboard.to_ne_bytes())?;
            state.keyboard = Some(keyboard);
        }
    } else if Some(message.object) == state.keyboard {
        match message.opcode {
            // wl_keyboard.keymap(format, fd, size)
            0 => {
                let (format, size) = (wl_u32(&message.payload, 0).unwrap_or(0), wl_u32(&message.payload, 4).unwrap_or(0));
                let descriptor = socket.take_descriptor();
                let mut text = Vec::new();
                if let (WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1, true, Some(descriptor)) = (format, size > 0, descriptor) {
                    let file = File::from(descriptor);
                    text = vec![0u8; size.to::<usize>()];
                    if file.read_exact_at(&mut text, 0).is_err() {
                        text.clear();
                    }
                }
                while text.last() == Some(&0) {
                    text.pop();
                }
                if !text.is_empty() {
                    shared.install_from_text(&text);
                }
            }
            // wl_keyboard.modifiers(serial, depressed, latched, locked, group)
            4 => {
                if let Some(group) = wl_u32(&message.payload, 16) {
                    shared.active_group.store(group, Ordering::SeqCst);
                }
            }
            // wl_keyboard.repeat_info(rate, delay)
            5 => {
                let delay = wl_u32(&message.payload, 4).unwrap_or(0).bits::<i32>();
                if delay > 0 {
                    shared.repeat_delay_ms.store(delay.bits::<u32>(), Ordering::SeqCst);
                    if let Some(callback) = &shared.on_repeat_delay {
                        callback(delay.bits::<u32>());
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// One round trip: asks the compositor to answer, and handles everything it sends until it does.
fn roundtrip(shared: &XkbShared, socket: &mut WlSocket, state: &mut WlClientState) -> io::Result<()> {
    let callback = socket.allocate();
    socket.send(WL_DISPLAY, WL_DISPLAY_SYNC, &callback.to_ne_bytes())?;
    let deadline = Instant::now() + seconds(WAYLAND_KEYMAP_WAIT_SECONDS);
    loop {
        while let Some(message) = socket.next_message() {
            if message.object == callback {
                return Ok(());
            }
            dispatch_event(shared, socket, state, &message)?;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "no answer from the compositor"));
        }
        if socket.wait_readable(None, remaining.as_millis().low::<i32>()).0 && socket.receive()? == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
    }
}

fn wayland_loop(shared: &XkbShared, wake: &UnixStream) {
    let environment = |name: &str| std::env::var(name).ok();
    let Some(path) = wayland_socket_path(environment("WAYLAND_DISPLAY").as_deref(), environment("XDG_RUNTIME_DIR").as_deref()) else {
        shared.keymap_ready.set();
        return;
    };
    let Ok(mut socket) = WlSocket::connect(&path) else {
        shared.keymap_ready.set();
        return;
    };
    let mut state = WlClientState::default();
    let registry = socket.allocate();
    if socket.send(WL_DISPLAY, WL_DISPLAY_GET_REGISTRY, &registry.to_ne_bytes()).is_ok() {
        // The registry names the seat, the seat its keyboard, the keyboard its keymap: one round trip each, so a fallback
        // keymap is never built while the real one is coming.
        for _ in 0..WAYLAND_KEYMAP_ROUNDTRIPS {
            if roundtrip(shared, &mut socket, &mut state).is_err() || !lock(&shared.core).keymap.is_null() {
                break;
            }
        }
    }
    shared.keymap_ready.set();
    let outcome = (|| -> io::Result<()> {
        loop {
            while let Some(message) = socket.next_message() {
                dispatch_event(shared, &mut socket, &mut state, &message)?;
            }
            let (readable, woken) = socket.wait_readable(Some(wake.as_raw_fd()), -1);
            if woken {
                return Ok(());
            }
            if readable && socket.receive()? == 0 {
                return Ok(());
            }
        }
    })();
    drop(outcome);
    shared.keymap_ready.set();
}

// --- Desktop compositor adapters ---

/// What the backend asks of the desktop the session runs on.
pub trait DesktopAdapter: Send + Sync {
    fn label(&self) -> &'static str;
    fn setup_error(&self) -> String;
    fn layout_names(&self) -> Vec<String>;
    fn current_layout_index(&self) -> i64;
    fn select_layout_index(&self, index: i64) -> bool;
    fn active_window(&self) -> Option<DesktopWindowInfo>;
    fn window_by_id(&self, window_id: i64) -> Option<DesktopWindowInfo>;
    fn input_anchor(&self) -> Option<ScreenAnchor>;
    fn position_window(&self, window: i64, x: i64, y: i64) -> bool;
    fn activate_window(&self, window: i64) -> bool;
    fn isolated_layout(&self) -> bool;
    fn locked(&self) -> bool;
    fn close(&self);
}

/// A session-bus connection whose calls time out as the Python's did.
fn session_connection() -> Result<Connection, zbus::Error> {
    Builder::session()?.method_timeout(Duration::from_millis(WAYLAND_DBUS_CALL_TIMEOUT_MS)).build()
}

fn call<B>(connection: &Connection, destination: &str, path: &str, interface: &str, method: &str, body: &B) -> Result<Message, zbus::Error>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    connection.call_method(Some(destination), path, Some(interface), method, body)
}

fn call_bool<B>(connection: &Connection, destination: &str, path: &str, interface: &str, method: &str, body: &B) -> Option<bool>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    call(connection, destination, path, interface, method, body).ok()?.body().deserialize::<bool>().ok()
}

fn dbus_name_has_owner(connection: &Connection, name: &str) -> bool {
    call_bool(connection, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "NameHasOwner", &(name,)).unwrap_or(false)
}

/// Runs `handler` for every signal that matches `rule` until the connection is closed or `stop` is set.
fn watch_signals(connection: &Connection, rule: MatchRule<'static>, stop: Arc<AtomicBool>, handler: impl Fn(&Message) + Send + 'static) -> Option<JoinHandle<()>> {
    let iterator = zbus::blocking::MessageIterator::for_match_rule(rule, connection, None).ok()?;
    thread::Builder::new()
        .name("keyswitch-wayland-dbus".to_owned())
        .spawn(move || {
            for message in iterator {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(message) = message {
                    handler(&message);
                }
            }
        })
        .ok()
}

fn signal_rule(sender: &str, interface: &str, member: &str, path: &str) -> Option<MatchRule<'static>> {
    Some(
        MatchRule::builder()
            .msg_type(MessageType::Signal)
            .sender(sender.to_owned())
            .ok()?
            .interface(interface.to_owned())
            .ok()?
            .member(member.to_owned())
            .ok()?
            .path(path.to_owned())
            .ok()?
            .build(),
    )
}

fn anchor_of(window: &Option<DesktopWindowInfo>, pointer: Option<(i64, i64)>) -> Option<ScreenAnchor> {
    let window_id = window.as_ref().map(|window| window.window_id);
    if let Some((x, y)) = pointer {
        return Some(ScreenAnchor { x: x.low::<i32>(), y: y.low::<i32>(), window: window_id });
    }
    let (x, y) = window.as_ref()?.center()?;
    Some(ScreenAnchor { x: x.low::<i32>(), y: y.low::<i32>(), window: window_id })
}

#[derive(Default)]
struct GnomeData {
    snapshot: DesktopSnapshot,
    windows: HashMap<i64, DesktopWindowInfo>,
}

type LockCallback = Box<dyn Fn(bool) + Send + Sync>;

struct GnomeShared {
    connection: Connection,
    data: Mutex<GnomeData>,
    on_lock_changed: LockCallback,
}

impl GnomeShared {
    fn apply_raw_state(&self, raw_json: &str) {
        let Ok(snapshot) = parse_gnome_state(raw_json) else { return };
        let (previous_locked, locked) = {
            let mut data = lock(&self.data);
            let previous = data.snapshot.locked;
            let locked = snapshot.locked;
            if let Some(window) = &snapshot.window {
                data.windows.insert(window.window_id, window.clone());
            }
            data.snapshot = snapshot;
            (previous, locked)
        };
        if locked != previous_locked {
            (self.on_lock_changed)(locked);
        }
    }

    fn refresh_state(&self) -> bool {
        let Ok(reply) = call(&self.connection, GNOME_DBUS_NAME, GNOME_DBUS_PATH, GNOME_DBUS_INTERFACE, "GetState", &()) else { return false };
        let Ok(raw) = reply.body().deserialize::<String>() else { return false };
        self.apply_raw_state(&raw);
        true
    }
}

/// GNOME Shell 45-51 Wayland integration via `keyswitch@io.github.olegius88.KeySwitch`.
pub struct GnomeShellAdapter {
    shared: Arc<GnomeShared>,
    error: Mutex<String>,
    stop: Arc<AtomicBool>,
}

impl GnomeShellAdapter {
    pub fn new(on_lock_changed: LockCallback) -> Result<GnomeShellAdapter, zbus::Error> {
        let adapter = GnomeShellAdapter {
            shared: Arc::new(GnomeShared { connection: session_connection()?, data: Mutex::new(GnomeData::default()), on_lock_changed }),
            error: Mutex::new(String::new()),
            stop: Arc::new(AtomicBool::new(false)),
        };
        adapter.connect_extension();
        Ok(adapter)
    }

    fn connect_extension(&self) {
        if let Some(rule) = signal_rule(GNOME_DBUS_NAME, GNOME_DBUS_INTERFACE, "StateChanged", GNOME_DBUS_PATH) {
            let shared = self.shared.clone();
            watch_signals(&self.shared.connection, rule, self.stop.clone(), move |message| {
                if let Ok(raw) = message.body().deserialize::<String>() {
                    shared.apply_raw_state(&raw);
                }
            });
        }
        if self.shared.refresh_state() {
            return;
        }
        let connection = &self.shared.connection;
        let set_error = |text: String| *lock(&self.error) = text;
        let info = call(connection, GNOME_EXTENSIONS_BUS_NAME, GNOME_EXTENSIONS_PATH, GNOME_EXTENSIONS_INTERFACE, "GetExtensionInfo", &(GNOME_EXTENSION_UUID,))
            .and_then(|reply| reply.body().deserialize::<HashMap<String, OwnedValue>>());
        let info = match info {
            Ok(info) => info,
            Err(error) => return set_error(format!("Не удалось обратиться к GNOME Shell: {error}")),
        };
        if info.is_empty() {
            return set_error(
                "Расширение GNOME Shell для KeySwitch ещё не загружено текущим сеансом: выйдите из учётной записи и войдите снова".to_owned(),
            );
        }
        if let Err(error) = call(connection, GNOME_EXTENSIONS_BUS_NAME, GNOME_EXTENSIONS_PATH, GNOME_EXTENSIONS_INTERFACE, "EnableExtension", &(GNOME_EXTENSION_UUID,)) {
            return set_error(format!("GNOME Shell не включил расширение KeySwitch: {error}"));
        }
        let deadline = Instant::now() + seconds(WAYLAND_IPC_TIMEOUT_SECONDS);
        while Instant::now() < deadline {
            if self.shared.refresh_state() {
                set_error(String::new());
                return;
            }
            thread::sleep(seconds(LAYOUT_SWITCH_POLL_SECONDS));
        }
        set_error(
            "Расширение GNOME Shell для KeySwitch не ответило после включения: проверьте расширения GNOME или перезайдите в сеанс".to_owned(),
        );
    }

    fn bool_call<B: serde::Serialize + zbus::zvariant::DynamicType>(&self, method: &str, body: &B) -> bool {
        call_bool(&self.shared.connection, GNOME_DBUS_NAME, GNOME_DBUS_PATH, GNOME_DBUS_INTERFACE, method, body).unwrap_or(false)
    }
}

impl DesktopAdapter for GnomeShellAdapter {
    fn label(&self) -> &'static str {
        "GNOME Wayland"
    }

    fn setup_error(&self) -> String {
        lock(&self.error).clone()
    }

    fn layout_names(&self) -> Vec<String> {
        let layouts = lock(&self.shared.data).snapshot.layouts.clone();
        if !layouts.is_empty() {
            return layouts;
        }
        self.shared.refresh_state();
        lock(&self.shared.data).snapshot.layouts.clone()
    }

    fn current_layout_index(&self) -> i64 {
        lock(&self.shared.data).snapshot.current_layout
    }

    fn select_layout_index(&self, index: i64) -> bool {
        let activated = self.bool_call("ActivateLayout", &(index.low::<u32>(),));
        if activated {
            self.shared.refresh_state();
        }
        activated
    }

    fn active_window(&self) -> Option<DesktopWindowInfo> {
        lock(&self.shared.data).snapshot.window.clone()
    }

    fn window_by_id(&self, window_id: i64) -> Option<DesktopWindowInfo> {
        lock(&self.shared.data).windows.get(&window_id).cloned()
    }

    fn input_anchor(&self) -> Option<ScreenAnchor> {
        self.shared.refresh_state();
        let (window, pointer) = {
            let data = lock(&self.shared.data);
            (data.snapshot.window.clone(), data.snapshot.pointer)
        };
        anchor_of(&window, pointer)
    }

    fn position_window(&self, _window: i64, x: i64, y: i64) -> bool {
        self.bool_call("MoveWindow", &(std::process::id(), LEARNING_PROMPT_WINDOW_TITLE, x.low::<i32>(), y.low::<i32>()))
    }

    fn activate_window(&self, window: i64) -> bool {
        self.bool_call("ActivateWindow", &(window.low::<u32>(),))
    }

    fn isolated_layout(&self) -> bool {
        lock(&self.shared.data).snapshot.isolated_layout
    }

    fn locked(&self) -> bool {
        lock(&self.shared.data).snapshot.locked
    }

    fn close(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// The Python's `Report` method of the KWin callback object, on a D-Bus path of its own.
struct KwinReport {
    shared: Arc<KdeShared>,
}

#[zbus::interface(name = "io.github.olegius88.KeySwitch.KWin")]
impl KwinReport {
    fn report(&self, payload: String) {
        let parsed = {
            let mut data = lock(&self.shared.data);
            parse_kwin_report(&payload, &mut data.id_map)
        };
        if let Ok((window, pointer)) = parsed {
            let mut data = lock(&self.shared.data);
            data.window = window.clone();
            if pointer.is_some() {
                data.pointer = pointer;
            }
            if let Some(window) = window {
                data.windows.insert(window.window_id, window);
            }
        }
    }
}

#[derive(Default)]
struct KdeData {
    id_map: KdeWindowIdMap,
    layouts: Vec<String>,
    current_layout: i64,
    window: Option<DesktopWindowInfo>,
    windows: HashMap<i64, DesktopWindowInfo>,
    pointer: Option<(i64, i64)>,
}

struct KdeShared {
    connection: Connection,
    data: Mutex<KdeData>,
    error: Mutex<String>,
}

impl KdeShared {
    fn refresh_layouts(&self) {
        let reply = call(&self.connection, KDE_KEYBOARD_BUS_NAME, KDE_KEYBOARD_PATH, KDE_KEYBOARD_INTERFACE, "getLayoutsList", &())
            .and_then(|reply| reply.body().deserialize::<Vec<(String, String, String)>>());
        match reply {
            Ok(items) => lock(&self.data).layouts = parse_kde_layouts(&items),
            Err(error) => *lock(&self.error) = format!("Служба раскладок KDE недоступна: {error}"),
        }
    }

    fn refresh_current_layout(&self) -> i64 {
        let reply = call(&self.connection, KDE_KEYBOARD_BUS_NAME, KDE_KEYBOARD_PATH, KDE_KEYBOARD_INTERFACE, "getLayout", &())
            .ok()
            .and_then(|reply| reply.body().deserialize::<u32>().ok());
        let mut data = lock(&self.data);
        if let Some(index) = reply {
            data.current_layout = i64::from(index);
        }
        data.current_layout
    }

    fn unload_plugin(&self, plugin: &str) {
        // A plugin that was not loaded needs no unloading.
        let _ = call(&self.connection, KWIN_BUS_NAME, KWIN_SCRIPTING_PATH, KWIN_SCRIPTING_INTERFACE, "unloadScript", &(plugin,));
    }

    /// Loads a script file under `plugin`, runs it, and gives back the id of the script; `None` if KWin refused.
    fn load_and_run(&self, path: &Path, plugin: &str) -> Option<i32> {
        let reply = call(&self.connection, KWIN_BUS_NAME, KWIN_SCRIPTING_PATH, KWIN_SCRIPTING_INTERFACE, "loadScript", &(path.to_string_lossy().as_ref(), plugin)).ok()?;
        let script_id = reply.body().deserialize::<i32>().ok()?;
        if script_id < 0 {
            return Some(script_id);
        }
        call(&self.connection, KWIN_BUS_NAME, &format!("{KWIN_SCRIPTING_PATH}/Script{script_id}"), KWIN_SCRIPT_INTERFACE, "run", &()).ok()?;
        Some(script_id)
    }
}

static SCRIPT_COUNTER: AtomicU32 = AtomicU32::new(0);

/// A script file only the user can read, which KWin (the same user) loads by its path. Created exclusively, so a file or link
/// somebody else put there is never followed.
fn write_script_file(prefix: &str, code: &str) -> io::Result<PathBuf> {
    use std::os::unix::fs::OpenOptionsExt;
    let directory = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|dir| dir.is_dir()).unwrap_or_else(std::env::temp_dir);
    for _ in 0..16 {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |time| time.subsec_nanos());
        let path = directory.join(format!("{prefix}{}-{}-{nanos}.js", std::process::id(), SCRIPT_COUNTER.fetch_add(1, Ordering::SeqCst)));
        match std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path) {
            Ok(mut file) => {
                file.write_all(code.as_bytes())?;
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "no free name for a script file"))
}

/// KDE Plasma 6 / KWin Wayland integration via `org.kde.keyboard` and KWin Scripting.
pub struct KdePlasmaAdapter {
    shared: Arc<KdeShared>,
    stop: Arc<AtomicBool>,
    watcher_path: Mutex<Option<PathBuf>>,
}

impl KdePlasmaAdapter {
    pub fn new() -> Result<KdePlasmaAdapter, zbus::Error> {
        let connection = session_connection()?;
        let shared = Arc::new(KdeShared { connection, data: Mutex::new(KdeData::default()), error: Mutex::new(String::new()) });
        let adapter = KdePlasmaAdapter { shared, stop: Arc::new(AtomicBool::new(false)), watcher_path: Mutex::new(None) };
        adapter.setup();
        Ok(adapter)
    }

    fn setup(&self) {
        let connection = &self.shared.connection;
        // The callback object is registered before the layouts are read, so a report that arrives early finds it.
        let _ = connection.object_server().at(KWIN_CALLBACK_PATH, KwinReport { shared: self.shared.clone() });
        if let Some(rule) = signal_rule(KDE_KEYBOARD_BUS_NAME, KDE_KEYBOARD_INTERFACE, "layoutChanged", KDE_KEYBOARD_PATH) {
            let shared = self.shared.clone();
            watch_signals(connection, rule, self.stop.clone(), move |message| {
                if let Ok(index) = message.body().deserialize::<u32>() {
                    lock(&shared.data).current_layout = i64::from(index);
                }
            });
        }
        if let Some(rule) = signal_rule(KDE_KEYBOARD_BUS_NAME, KDE_KEYBOARD_INTERFACE, "layoutListChanged", KDE_KEYBOARD_PATH) {
            let shared = self.shared.clone();
            watch_signals(connection, rule, self.stop.clone(), move |_| {
                shared.refresh_layouts();
                shared.refresh_current_layout();
            });
        }
        self.shared.refresh_layouts();
        self.shared.refresh_current_layout();
        self.install_kwin_watcher();
    }

    fn install_kwin_watcher(&self) {
        let Some(unique_name) = self.shared.connection.unique_name().map(|name| name.to_string()) else { return };
        let plugin = format!("{KWIN_WATCHER_PLUGIN}_{}", std::process::id());
        let script = build_kwin_watcher_script(&unique_name, KWIN_CALLBACK_PATH, KWIN_CALLBACK_IFACE);
        let Ok(path) = write_script_file("keyswitch-kwin-", &script) else { return };
        *lock(&self.watcher_path) = Some(path.clone());
        self.shared.unload_plugin(&plugin);
        let _ = self.shared.load_and_run(&path, &plugin);
    }

    fn run_kwin_oneshot(&self, code: &str) -> bool {
        let plugin = format!("{KWIN_ACTION_PLUGIN}_{}", std::process::id());
        let Ok(path) = write_script_file("keyswitch-kwin-act-", code) else { return false };
        self.shared.unload_plugin(&plugin);
        let loaded = self.shared.load_and_run(&path, &plugin);
        let ran = matches!(loaded, Some(id) if id >= 0);
        if ran {
            self.shared.unload_plugin(&plugin);
        }
        // The file is only needed while KWin loads it.
        let _ = std::fs::remove_file(&path);
        ran
    }
}

impl DesktopAdapter for KdePlasmaAdapter {
    fn label(&self) -> &'static str {
        "KDE Plasma Wayland"
    }

    fn setup_error(&self) -> String {
        lock(&self.shared.error).clone()
    }

    fn layout_names(&self) -> Vec<String> {
        let layouts = lock(&self.shared.data).layouts.clone();
        if !layouts.is_empty() {
            return layouts;
        }
        self.shared.refresh_layouts();
        lock(&self.shared.data).layouts.clone()
    }

    fn current_layout_index(&self) -> i64 {
        // Asked at every key: answered from the layoutChanged signal, not by a call to KWin.
        lock(&self.shared.data).current_layout
    }

    fn select_layout_index(&self, index: i64) -> bool {
        let accepted = call_bool(&self.shared.connection, KDE_KEYBOARD_BUS_NAME, KDE_KEYBOARD_PATH, KDE_KEYBOARD_INTERFACE, "setLayout", &(index.low::<u32>(),)).unwrap_or(false);
        if accepted {
            lock(&self.shared.data).current_layout = index;
        }
        accepted
    }

    fn active_window(&self) -> Option<DesktopWindowInfo> {
        lock(&self.shared.data).window.clone()
    }

    fn window_by_id(&self, window_id: i64) -> Option<DesktopWindowInfo> {
        lock(&self.shared.data).windows.get(&window_id).cloned()
    }

    fn input_anchor(&self) -> Option<ScreenAnchor> {
        if let Some(unique_name) = self.shared.connection.unique_name().map(|name| name.to_string()) {
            let action = KwinAction { report_target: Some((&unique_name, KWIN_CALLBACK_PATH, KWIN_CALLBACK_IFACE)), ..KwinAction::default() };
            self.run_kwin_oneshot(&build_kwin_action_script(&action));
        }
        let (window, pointer) = {
            let data = lock(&self.shared.data);
            (data.window.clone(), data.pointer)
        };
        anchor_of(&window, pointer)
    }

    fn position_window(&self, _window: i64, x: i64, y: i64) -> bool {
        let action = KwinAction { move_pid: i64::from(std::process::id()), move_title: LEARNING_PROMPT_WINDOW_TITLE, x, y, ..KwinAction::default() };
        self.run_kwin_oneshot(&build_kwin_action_script(&action))
    }

    fn activate_window(&self, window: i64) -> bool {
        let uuid = lock(&self.shared.data).id_map.resolve(window);
        if uuid.is_empty() {
            return false;
        }
        self.run_kwin_oneshot(&build_kwin_action_script(&KwinAction { activate_uuid: &uuid, ..KwinAction::default() }))
    }

    fn isolated_layout(&self) -> bool {
        false
    }

    fn locked(&self) -> bool {
        false
    }

    fn close(&self) {
        self.shared.unload_plugin(&format!("{KWIN_WATCHER_PLUGIN}_{}", std::process::id()));
        if let Some(path) = lock(&self.watcher_path).take() {
            let _ = std::fs::remove_file(path);
        }
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.shared.connection.object_server().remove::<KwinReport, _>(KWIN_CALLBACK_PATH);
    }
}

#[derive(Default)]
struct SwayData {
    layouts: Vec<String>,
    current_layout: i64,
    window: Option<DesktopWindowInfo>,
    windows: HashMap<i64, DesktopWindowInfo>,
}

struct SwayShared {
    socket_path: PathBuf,
    data: Mutex<SwayData>,
    command_socket: Mutex<Option<UnixStream>>,
    stop: AtomicBool,
}

impl SwayShared {
    fn open_socket(&self) -> io::Result<UnixStream> {
        let socket = UnixStream::connect(&self.socket_path)?;
        socket.set_read_timeout(Some(seconds(WAYLAND_IPC_TIMEOUT_SECONDS)))?;
        socket.set_write_timeout(Some(seconds(WAYLAND_IPC_TIMEOUT_SECONDS)))?;
        Ok(socket)
    }

    fn recv_exact(socket: &mut UnixStream, count: usize) -> io::Result<Vec<u8>> {
        let mut buffer = vec![0u8; count];
        socket.read_exact(&mut buffer).map_err(|error| {
            if error.kind() == io::ErrorKind::UnexpectedEof { io::Error::new(io::ErrorKind::UnexpectedEof, "sway IPC socket closed") } else { error }
        })?;
        Ok(buffer)
    }

    fn roundtrip(&self, message_type: u32, payload: &str) -> io::Result<Value> {
        let mut guard = lock(&self.command_socket);
        if guard.is_none() {
            *guard = Some(self.open_socket()?);
        }
        let exchange = (|| -> io::Result<Vec<u8>> {
            let socket = guard.as_mut().ok_or_else(|| io::Error::other("no socket"))?;
            socket.write_all(&encode_sway_ipc(message_type, payload))?;
            let header = Self::recv_exact(socket, SWAY_IPC_HEADER_BYTES.to::<usize>())?;
            let (length, _reply_type) = decode_sway_ipc_header(&header).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.0))?;
            Self::recv_exact(socket, length.to::<usize>())
        })();
        match exchange {
            Ok(body) => serde_json::from_slice(&body).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
            Err(error) => {
                *guard = None;
                Err(error)
            }
        }
    }

    fn refresh_inputs(&self) -> io::Result<()> {
        let data = self.roundtrip(SWAY_IPC_GET_INPUTS, "")?;
        let (layouts, active) = parse_sway_inputs(&data);
        let mut state = lock(&self.data);
        state.layouts = layouts;
        state.current_layout = active;
        Ok(())
    }

    fn refresh_tree(&self) -> io::Result<Value> {
        let tree = self.roundtrip(SWAY_IPC_GET_TREE, "")?;
        let window = parse_sway_focused_window(&tree);
        let mut state = lock(&self.data);
        state.window = window.clone();
        if let Some(window) = window {
            state.windows.insert(window.window_id, window);
        }
        Ok(tree)
    }

    fn command_succeeded(&self, command: &str, all: bool) -> bool {
        let Ok(Value::Array(reply)) = self.roundtrip(SWAY_IPC_RUN_COMMAND, command) else { return false };
        let success = |item: &Value| item.is_object() && item.get("success") == Some(&Value::Bool(true));
        if all { reply.iter().all(success) } else { reply.iter().any(success) }
    }
}

/// sway / i3-IPC Wayland integration over `SWAYSOCK`.
pub struct SwayIpcAdapter {
    shared: Arc<SwayShared>,
    error: Mutex<String>,
    subscription: Mutex<Option<UnixStream>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl SwayIpcAdapter {
    pub fn new(socket_path: &str) -> SwayIpcAdapter {
        let adapter = SwayIpcAdapter {
            shared: Arc::new(SwayShared {
                socket_path: PathBuf::from(socket_path),
                data: Mutex::new(SwayData::default()),
                command_socket: Mutex::new(None),
                stop: AtomicBool::new(false),
            }),
            error: Mutex::new(String::new()),
            subscription: Mutex::new(None),
            thread: Mutex::new(None),
        };
        if let Err(error) = adapter.setup() {
            *lock(&adapter.error) = format!("Не удалось подключиться к sway IPC: {error}");
        }
        adapter
    }

    fn setup(&self) -> io::Result<()> {
        self.shared.refresh_inputs()?;
        self.shared.refresh_tree()?;
        let mut socket = self.shared.open_socket()?;
        let subscribe = ks_text::pyjson::json_dumps(&serde_json::json!(["window", "input"]), &ks_text::pyjson::JsonOptions::default());
        socket.write_all(&encode_sway_ipc(SWAY_IPC_SUBSCRIBE, &subscribe))?;
        let header = SwayShared::recv_exact(&mut socket, SWAY_IPC_HEADER_BYTES.to::<usize>())?;
        let (length, _) = decode_sway_ipc_header(&header).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.0))?;
        SwayShared::recv_exact(&mut socket, length.to::<usize>())?;
        socket.set_read_timeout(None)?;
        let mut events = socket.try_clone()?;
        *lock(&self.subscription) = Some(socket);
        let shared = self.shared.clone();
        let handle = thread::Builder::new().name("keyswitch-sway-ipc".to_owned()).spawn(move || {
            while !shared.stop.load(Ordering::SeqCst) {
                let Ok(header) = SwayShared::recv_exact(&mut events, SWAY_IPC_HEADER_BYTES.to::<usize>()) else { return };
                let Ok((length, event_type)) = decode_sway_ipc_header(&header) else { return };
                if SwayShared::recv_exact(&mut events, length.to::<usize>()).is_err() {
                    return;
                }
                // A refresh that fails is tried again at the next event.
                if event_type == SWAY_IPC_EVENT_INPUT {
                    let _ = shared.refresh_inputs();
                } else if event_type == SWAY_IPC_EVENT_WINDOW {
                    let _ = shared.refresh_tree();
                }
            }
        })?;
        *lock(&self.thread) = Some(handle);
        Ok(())
    }
}

impl DesktopAdapter for SwayIpcAdapter {
    fn label(&self) -> &'static str {
        "sway"
    }

    fn setup_error(&self) -> String {
        lock(&self.error).clone()
    }

    fn layout_names(&self) -> Vec<String> {
        let layouts = lock(&self.shared.data).layouts.clone();
        if !layouts.is_empty() {
            return layouts;
        }
        if self.shared.refresh_inputs().is_err() {
            return Vec::new();
        }
        lock(&self.shared.data).layouts.clone()
    }

    fn current_layout_index(&self) -> i64 {
        // Asked at every key: answered from the input events sway sends, not by a round trip.
        lock(&self.shared.data).current_layout
    }

    fn select_layout_index(&self, index: i64) -> bool {
        if self.shared.command_succeeded(&sway_switch_layout_command(index), true) {
            let _ = self.shared.refresh_inputs();
            return true;
        }
        false
    }

    fn active_window(&self) -> Option<DesktopWindowInfo> {
        // Kept by the window events sway sends; a tree read at every key costs a round trip.
        lock(&self.shared.data).window.clone()
    }

    fn window_by_id(&self, window_id: i64) -> Option<DesktopWindowInfo> {
        // Geometry is asked for rarely (the prompt, a caret) and goes stale: a window gets its title bar after the event
        // announcing it, and no event follows. It is read afresh here.
        let _ = self.shared.refresh_tree();
        lock(&self.shared.data).windows.get(&window_id).cloned()
    }

    fn input_anchor(&self) -> Option<ScreenAnchor> {
        let _ = self.shared.refresh_tree();
        let window = self.active_window()?;
        let (x, y) = window.center().unwrap_or((window.x, window.y));
        Some(ScreenAnchor { x: x.low::<i32>(), y: y.low::<i32>(), window: Some(window.window_id) })
    }

    fn position_window(&self, _window: i64, x: i64, y: i64) -> bool {
        let Ok(tree) = self.shared.refresh_tree() else { return false };
        let Some(con_id) = find_sway_window_by_pid(&tree, i64::from(std::process::id()), LEARNING_PROMPT_WINDOW_TITLE) else { return false };
        self.shared.command_succeeded(&sway_popup_move_command(con_id, x, y), false)
    }

    fn activate_window(&self, window: i64) -> bool {
        self.shared.command_succeeded(&sway_focus_command(window), false)
    }

    fn isolated_layout(&self) -> bool {
        false
    }

    fn locked(&self) -> bool {
        false
    }

    fn close(&self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(socket) = lock(&self.subscription).take() {
            let _ = socket.shutdown(std::net::Shutdown::Both);
        }
        if let Some(socket) = lock(&self.shared.command_socket).take() {
            let _ = socket.shutdown(std::net::Shutdown::Both);
        }
        if let Some(handle) = lock(&self.thread).take()
            && handle.thread().id() != thread::current().id()
        {
            join_within(handle, WAYLAND_IPC_TIMEOUT_SECONDS);
        }
    }
}

/// Fallback adapter reading layouts and active group from `wl_keyboard`.
pub struct GenericWaylandAdapter {
    xkb: Arc<WaylandXkbMonitor>,
}

impl GenericWaylandAdapter {
    pub fn new(xkb: Arc<WaylandXkbMonitor>) -> GenericWaylandAdapter {
        GenericWaylandAdapter { xkb }
    }
}

impl DesktopAdapter for GenericWaylandAdapter {
    fn label(&self) -> &'static str {
        "Wayland"
    }
    fn setup_error(&self) -> String {
        "Текущий композитор Wayland не предоставляет интерфейс управления раскладкой (поддерживаются GNOME Shell, KDE Plasma 6 и sway)".to_owned()
    }
    fn layout_names(&self) -> Vec<String> {
        self.xkb.layout_names()
    }
    fn current_layout_index(&self) -> i64 {
        self.xkb.active_group()
    }
    fn select_layout_index(&self, _index: i64) -> bool {
        false
    }
    fn active_window(&self) -> Option<DesktopWindowInfo> {
        None
    }
    fn window_by_id(&self, _window_id: i64) -> Option<DesktopWindowInfo> {
        None
    }
    fn input_anchor(&self) -> Option<ScreenAnchor> {
        None
    }
    fn position_window(&self, _window: i64, x: i64, y: i64) -> bool {
        false
    }
    fn activate_window(&self, _window: i64) -> bool {
        false
    }
    fn isolated_layout(&self) -> bool {
        false
    }
    fn locked(&self) -> bool {
        false
    }
    fn close(&self) {}
}

// --- logind LockedHint / VT Active monitor ---

struct LogindShared {
    locked_hint: AtomicBool,
    session_active: AtomicBool,
    sleeping: AtomicBool,
    on_change: Box<dyn Fn() + Send + Sync>,
}

/// Watches `org.freedesktop.login1` so `EVIOCGRAB` is released on lock screens and VT switches.
pub struct LogindLockMonitor {
    shared: Arc<LogindShared>,
    stop: Arc<AtomicBool>,
    _connection: Option<Connection>,
}

impl LogindLockMonitor {
    pub fn new(on_change: Box<dyn Fn() + Send + Sync>) -> LogindLockMonitor {
        let shared = Arc::new(LogindShared {
            locked_hint: AtomicBool::new(false),
            session_active: AtomicBool::new(true),
            sleeping: AtomicBool::new(false),
            on_change,
        });
        let stop = Arc::new(AtomicBool::new(false));
        let connection = Builder::system().and_then(|builder| builder.method_timeout(Duration::from_millis(WAYLAND_DBUS_CALL_TIMEOUT_MS)).build()).ok();
        if let Some(connection) = &connection {
            LogindLockMonitor::setup(connection, &shared, &stop);
        }
        LogindLockMonitor { shared, stop, _connection: connection }
    }

    pub fn grab_blocked(&self) -> bool {
        self.shared.locked_hint.load(Ordering::SeqCst) || !self.shared.session_active.load(Ordering::SeqCst) || self.shared.sleeping.load(Ordering::SeqCst)
    }

    fn session_path(connection: &Connection) -> String {
        let reply = call(connection, LOGIND_BUS_NAME, LOGIND_USER_SELF_PATH, DBUS_PROPERTIES_IFACE, "Get", &(LOGIND_USER_IFACE, "Display"));
        let Ok(reply) = reply else { return String::new() };
        let Ok(value) = reply.body().deserialize::<OwnedValue>() else { return String::new() };
        match <(String, OwnedObjectPath)>::try_from(value) {
            Ok((_, path)) => path.to_string(),
            Err(_) => String::new(),
        }
    }

    fn read_session_props(connection: &Connection, shared: &LogindShared, session_path: &str) {
        let reply = call(connection, LOGIND_BUS_NAME, session_path, DBUS_PROPERTIES_IFACE, "GetAll", &(LOGIND_SESSION_IFACE,));
        let Ok(properties) = reply.map_err(|_| ()).and_then(|reply| reply.body().deserialize::<HashMap<String, OwnedValue>>().map_err(|_| ())) else {
            return;
        };
        if let Some(Ok(hint)) = properties.get("LockedHint").map(|value| bool::try_from(value.clone())) {
            shared.locked_hint.store(hint, Ordering::SeqCst);
        }
        if let Some(Ok(active)) = properties.get("Active").map(|value| bool::try_from(value.clone())) {
            shared.session_active.store(active, Ordering::SeqCst);
        }
    }

    fn setup(connection: &Connection, shared: &Arc<LogindShared>, stop: &Arc<AtomicBool>) {
        let session_path = LogindLockMonitor::session_path(connection);
        if !session_path.is_empty() {
            LogindLockMonitor::read_session_props(connection, shared, &session_path);
            if let Some(rule) = signal_rule(LOGIND_BUS_NAME, DBUS_PROPERTIES_IFACE, "PropertiesChanged", &session_path) {
                let shared = shared.clone();
                watch_signals(connection, rule, stop.clone(), move |message| {
                    let Ok((_, changed, _)) = message.body().deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>() else { return };
                    let mut updated = false;
                    if let Some(Ok(hint)) = changed.get("LockedHint").map(|value| bool::try_from(value.clone())) {
                        shared.locked_hint.store(hint, Ordering::SeqCst);
                        updated = true;
                    }
                    if let Some(Ok(active)) = changed.get("Active").map(|value| bool::try_from(value.clone())) {
                        shared.session_active.store(active, Ordering::SeqCst);
                        updated = true;
                    }
                    if updated {
                        (shared.on_change)();
                    }
                });
            }
        }
        if let Some(rule) = signal_rule(LOGIND_BUS_NAME, LOGIND_MANAGER_IFACE, "PrepareForSleep", LOGIND_MANAGER_PATH) {
            let shared = shared.clone();
            watch_signals(connection, rule, stop.clone(), move |message| {
                if let Ok(starting) = message.body().deserialize::<bool>() {
                    shared.sleeping.store(starting, Ordering::SeqCst);
                    (shared.on_change)();
                }
            });
        }
    }

    pub fn close(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

// --- NativeWaylandApi ---

/// The command that starts the input helper: this executable with the descriptor of its end of the socket.
pub fn helper_command(child_fd: RawFd) -> io::Result<Command> {
    let binary = std::fs::canonicalize("/proc/self/exe").or_else(|_| std::env::current_exe())?;
    let mut command = Command::new(binary);
    command.arg("--wayland-input-helper").arg(child_fd.to_string());
    Ok(command)
}

/// What the session is made of: the keymap monitor, the desktop adapter and the lock monitor.
struct Components {
    xkb: Arc<WaylandXkbMonitor>,
    adapter: Arc<dyn DesktopAdapter>,
    logind: Mutex<Option<LogindLockMonitor>>,
}

struct ApiShared {
    components: Mutex<Option<Arc<Components>>>,
    client: Mutex<Option<Arc<HelperClient>>>,
    process: Mutex<Option<Child>>,
    stop_listener: Flag,
    caps_lock: AtomicBool,
}

impl ApiShared {
    fn components(&self) -> Option<Arc<Components>> {
        lock(&self.components).clone()
    }

    fn should_grab(&self) -> bool {
        let Some(components) = self.components() else { return true };
        let logind_blocked = lock(&components.logind).as_ref().is_some_and(LogindLockMonitor::grab_blocked);
        !(logind_blocked || components.adapter.locked())
    }

    fn set_grab(&self, enabled: bool, repeat_delay_ms: u32) {
        let client = lock(&self.client).clone();
        if let Some(client) = client {
            // A helper that does not answer is dealt with by the loop that watches it.
            let _ = client.set_grab(enabled, repeat_delay_ms);
        }
    }

    fn sync_grab_state(&self) {
        let delay = self.components().map_or(WAYLAND_REPEAT_DEFAULT_DELAY_MS, |components| components.xkb.repeat_delay_ms());
        self.set_grab(self.should_grab(), delay);
    }
}

/// Production `WaylandApi` combining `HelperClient`, `WaylandXkbMonitor` and the desktop's IPC.
pub struct NativeWaylandApi {
    shared: Arc<ApiShared>,
}

fn open_components(shared: &Arc<ApiShared>) -> Result<Arc<Components>, String> {
    let weak: Weak<ApiShared> = Arc::downgrade(shared);
    let repeat_weak = weak.clone();
    let xkb = Arc::new(WaylandXkbMonitor::new(Some(Box::new(move |delay| {
        if let Some(shared) = repeat_weak.upgrade() {
            shared.set_grab(shared.should_grab(), delay);
        }
    })))?);
    xkb.start();
    let adapter = create_adapter(&xkb, &weak);
    xkb.ensure_fallback_keymap(&adapter.layout_names());
    let components = Arc::new(Components { xkb, adapter, logind: Mutex::new(None) });
    *lock(&shared.components) = Some(components.clone());
    let sync_weak = weak;
    *lock(&components.logind) = Some(LogindLockMonitor::new(Box::new(move || {
        if let Some(shared) = sync_weak.upgrade() {
            shared.sync_grab_state();
        }
    })));
    Ok(components)
}

fn create_adapter(xkb: &Arc<WaylandXkbMonitor>, shared: &Weak<ApiShared>) -> Arc<dyn DesktopAdapter> {
    let environment = |name: &str| std::env::var(name).ok();
    let sway_socket = pystr::strip(&environment("SWAYSOCK").unwrap_or_default()).to_owned();
    let sway_socket = if !sway_socket.is_empty() && is_socket(Path::new(&sway_socket)) { sway_socket } else { String::new() };
    let bus = session_connection().ok();
    let has_gnome = bus.as_ref().is_some_and(|bus| dbus_name_has_owner(bus, GNOME_EXTENSIONS_BUS_NAME));
    let has_kde = bus.as_ref().is_some_and(|bus| dbus_name_has_owner(bus, KDE_KEYBOARD_BUS_NAME));
    let kind = detect_desktop_kind(&environment, has_gnome, has_kde, &sway_socket);
    match kind {
        DesktopKind::Sway if !sway_socket.is_empty() => return Arc::new(SwayIpcAdapter::new(&sway_socket)),
        DesktopKind::Gnome if bus.is_some() => {
            let weak = shared.clone();
            let notify: LockCallback = Box::new(move |_locked| {
                if let Some(shared) = weak.upgrade() {
                    shared.sync_grab_state();
                }
            });
            if let Ok(adapter) = GnomeShellAdapter::new(notify) {
                return Arc::new(adapter);
            }
        }
        DesktopKind::Kde if bus.is_some() => {
            if let Ok(adapter) = KdePlasmaAdapter::new() {
                return Arc::new(adapter);
            }
        }
        _ => {}
    }
    Arc::new(GenericWaylandAdapter::new(xkb.clone()))
}

fn is_socket(path: &Path) -> bool {
    use std::os::unix::fs::FileTypeExt;
    std::fs::metadata(path).is_ok_and(|metadata| metadata.file_type().is_socket())
}

fn access(path: &str, mode: c_int) -> bool {
    let Ok(c_path) = CString::new(path) else { return false };
    // SAFETY: `c_path` is a valid NUL-terminated string.
    unsafe { libc::access(c_path.as_ptr(), mode) == 0 }
}

impl NativeWaylandApi {
    pub fn new() -> Result<NativeWaylandApi, String> {
        let shared = Arc::new(ApiShared {
            components: Mutex::new(None),
            client: Mutex::new(None),
            process: Mutex::new(None),
            stop_listener: Flag::default(),
            caps_lock: AtomicBool::new(false),
        });
        open_components(&shared)?;
        Ok(NativeWaylandApi { shared })
    }

    fn components(&self) -> Result<Arc<Components>, String> {
        match self.shared.components() {
            Some(components) => Ok(components),
            None => open_components(&self.shared),
        }
    }

    /// Run one helper process; true when it exited on its own (not stopped).
    fn run_helper_once(&self, listener: &NativeListener, ready: Option<&ReadyCallback>) -> Result<bool, String> {
        let components = self.components()?;
        let (parent_socket, child_socket) = UnixStream::pair().map_err(|error| error.to_string())?;
        let child_fd = child_socket.as_raw_fd();
        let mut command = helper_command(child_fd).map_err(|error| error.to_string())?;
        command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        // SAFETY: the closure runs in the forked child before `exec` and only calls `fcntl`, which is async-signal-safe; it
        // clears close-on-exec on the child's end of the socket so the helper inherits it.
        unsafe {
            command.pre_exec(move || {
                check(libc::fcntl(child_fd, libc::F_SETFD, 0))?;
                Ok(())
            });
        }
        let child = command.spawn().map_err(|error| error.to_string())?;
        drop(child_socket);
        *lock(&self.shared.process) = Some(child);

        let on_key = {
            let listener = listener.clone();
            move |key: HelperKey| {
                listener(NativeKeyEvent {
                    seq: key.seq,
                    delivered: !key.needs_verdict() && !key.forced(),
                    forced: key.forced(),
                    ..NativeKeyEvent::new(key.pressed, key.keycode, key.timestamp_ms)
                })
            }
        };
        let callbacks = HelperCallbacks {
            on_key: Box::new(on_key),
            on_pointer: {
                let listener = listener.clone();
                Box::new(move |timestamp| {
                    listener(NativeKeyEvent { pointer: true, ..NativeKeyEvent::new(true, 0, timestamp) });
                })
            },
            on_repeat_reset: {
                let listener = listener.clone();
                Box::new(move |timestamp| {
                    listener(NativeKeyEvent { repeat_reset: true, ..NativeKeyEvent::new(true, 0, timestamp) });
                })
            },
            on_caps_lock: {
                let shared = self.shared.clone();
                Box::new(move |enabled| shared.caps_lock.store(enabled, Ordering::SeqCst))
            },
            on_input_reset: {
                let listener = listener.clone();
                Box::new(move || {
                    listener(NativeKeyEvent { input_reset: true, ..NativeKeyEvent::new(true, 0, monotonic_ms()) });
                })
            },
        };
        let client = Arc::new(HelperClient::new(Arc::new(parent_socket), callbacks));
        let outcome = (|| -> Result<bool, String> {
            let caps = client.start_default()?;
            self.shared.caps_lock.store(caps, Ordering::SeqCst);
            *lock(&self.shared.client) = Some(client.clone());
            let delay = components.xkb.repeat_delay_ms();
            if !self.shared.should_grab() {
                client.set_grab(false, delay)?;
            } else if delay != WAYLAND_REPEAT_DEFAULT_DELAY_MS {
                client.set_grab(true, delay)?;
            }
            if let Some(ready) = ready {
                ready();
            }
            while !self.shared.stop_listener.wait(seconds(WAYLAND_IPC_TIMEOUT_SECONDS)) {
                let exited = lock(&self.shared.process).as_mut().is_none_or(|child| child.try_wait().ok().flatten().is_some());
                if exited || client.closed() {
                    return Ok(true);
                }
            }
            Ok(false)
        })();
        *lock(&self.shared.client) = None;
        client.close();
        if let Some(mut child) = lock(&self.shared.process).take()
            && child.try_wait().ok().flatten().is_none()
        {
            // SAFETY: the pid is of our own child, which has not been reaped.
            unsafe { libc::kill(child.id().to::<libc::pid_t>(), libc::SIGTERM) };
            let deadline = Instant::now() + seconds(WAYLAND_IPC_TIMEOUT_SECONDS);
            while child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        outcome
    }
}

/// `int(time.monotonic() * 1000) & 0xFFFFFFFF`: what the engine and the helper's key stamps are compared with.
pub fn monotonic_ms() -> u32 {
    ((clock_monotonic() * 1000.0).to::<u64>() & u64::from(UINT32_MODULO_MASK)).low::<u32>()
}

impl WaylandApi for NativeWaylandApi {
    fn desktop_name(&self) -> String {
        let display = std::env::var("WAYLAND_DISPLAY").ok().map(|value| pystr::strip(&value).to_owned()).filter(|value| !value.is_empty());
        let label = self.shared.components().map_or("Wayland", |components| components.adapter.label());
        format!("{label} ({})", display.unwrap_or_else(|| "wayland".to_owned()))
    }

    fn permission_error(&self) -> String {
        if let Some(components) = self.shared.components() {
            let error = components.adapter.setup_error();
            if !error.is_empty() {
                return error;
            }
        }
        if !Path::new(UINPUT_PATH).exists() {
            return "Устройство /dev/uinput отсутствует: загрузите модуль ядра uinput (sudo modprobe uinput)".to_owned();
        }
        if !access(UINPUT_PATH, libc::R_OK | libc::W_OK) {
            return format!("Нет прав доступа к /dev/uinput: {}", access_hint());
        }
        let keyboards: Vec<InputDeviceInfo> =
            LinuxHelperOs::new().list_event_devices().into_iter().filter(|info| classify_input_device(info) == DeviceRole::Keyboard).collect();
        if !keyboards.is_empty() && !keyboards.iter().any(|info| access(&info.path, libc::R_OK)) {
            return format!("Нет прав чтения клавиатур /dev/input/event*: {}", access_hint());
        }
        String::new()
    }

    fn layout_names(&self) -> Vec<String> {
        let Some(components) = self.shared.components() else { return Vec::new() };
        let names = components.adapter.layout_names();
        if !names.is_empty() {
            components.xkb.ensure_fallback_keymap(&names);
            return names;
        }
        components.xkb.layout_names()
    }

    fn current_layout_index(&self) -> i64 {
        self.shared.components().map_or(-1, |components| components.adapter.current_layout_index())
    }

    fn select_layout_index(&self, index: i64) -> bool {
        self.shared.components().is_some_and(|components| components.adapter.select_layout_index(index))
    }

    fn translate_key(&self, keycode: u32, state: u32, layout_index: i64) -> String {
        self.shared.components().map_or_else(String::new, |components| components.xkb.translate_key(keycode, state, layout_index))
    }

    fn keysym_name(&self, keycode: u32, layout_index: i64) -> String {
        self.shared.components().map_or_else(String::new, |components| components.xkb.keysym_name(keycode, layout_index))
    }

    fn post_inputs(&self, inputs: &[(bool, u32)], known_seq: Option<u32>, action: bool) -> Result<u32, String> {
        let client = lock(&self.shared.client).clone();
        // Not 0: from the helper, 0 is a correction it refused (a key reached the window first).
        let client = client.ok_or_else(|| "Процесс ввода Wayland не запущен; текст не изменён".to_owned())?;
        client.emit_batch(inputs, known_seq, action)
    }

    fn active_application(&self) -> String {
        self.shared.components().and_then(|components| components.adapter.active_window()).map(|window| window.application).unwrap_or_default()
    }

    fn focused_window(&self) -> i64 {
        self.shared.components().and_then(|components| components.adapter.active_window()).map_or(0, |window| window.window_id)
    }

    fn window_process_id(&self, window: i64) -> i64 {
        let Some(components) = self.shared.components() else { return 0 };
        let found = components.adapter.window_by_id(window).or_else(|| components.adapter.active_window().filter(|active| active.window_id == window));
        found.map_or(0, |window| window.pid)
    }

    fn current_process_id(&self) -> i64 {
        i64::from(std::process::id())
    }

    fn window_origin(&self, window: i64) -> Option<(i64, i64)> {
        let components = self.shared.components()?;
        let found = components
            .adapter
            .window_by_id(window)
            .or_else(|| components.adapter.active_window().filter(|active| window == 0 || active.window_id == window))?;
        Some((found.x, found.y))
    }

    fn window_is_password(&self, window: i64) -> bool {
        self.shared.components().and_then(|components| components.adapter.active_window()).is_some_and(|active| active.window_id == window && active.password)
    }

    fn input_anchor(&self) -> Option<ScreenAnchor> {
        self.shared.components()?.adapter.input_anchor()
    }

    fn position_window(&self, window: i64, x: i64, y: i64) -> bool {
        self.shared.components().is_some_and(|components| components.adapter.position_window(window, x, y))
    }

    fn activate_window(&self, window: i64) -> bool {
        self.shared.components().is_some_and(|components| components.adapter.activate_window(window))
    }

    fn isolated_layout(&self) -> bool {
        self.shared.components().is_some_and(|components| components.adapter.isolated_layout())
    }

    fn caps_lock_enabled(&self) -> bool {
        self.shared.caps_lock.load(Ordering::SeqCst)
    }

    fn key_down(&self, keycode: u32) -> bool {
        let client = lock(&self.shared.client).clone();
        client.is_some_and(|client| client.query_key_down(keycode).unwrap_or(false))
    }

    fn monotonic_ms(&self) -> u32 {
        monotonic_ms()
    }

    /// Run the input helper until stopped; a helper that dies is started again.
    ///
    /// Its grab ends with it, so the keyboard keeps working, but KeySwitch would stay deaf: it is restarted up to
    /// `WAYLAND_HELPER_RESTARTS` times, and the parent forgets the keys it believed held down. A helper that fails at the first
    /// start is the caller's error.
    fn run_input_listener(&self, listener: NativeListener, ready: ReadyCallback) -> Result<(), String> {
        self.components()?;
        self.shared.stop_listener.clear();
        let mut restarts = 0;
        loop {
            let exited = self.run_helper_once(&listener, if restarts == 0 { Some(&ready) } else { None })?;
            if !exited || self.shared.stop_listener.is_set() {
                return Ok(());
            }
            if restarts >= WAYLAND_HELPER_RESTARTS {
                logsetup::error(LOGGER, &format!("Процесс перехвата клавиатуры Wayland завершился {} раз подряд; перехват остановлен", restarts + 1));
                return Ok(());
            }
            restarts += 1;
            logsetup::warning(LOGGER, &format!("Процесс перехвата клавиатуры Wayland завершился; запуск заново ({restarts})"));
            listener(NativeKeyEvent { input_reset: true, ..NativeKeyEvent::new(true, 0, monotonic_ms()) });
            if self.shared.stop_listener.wait(seconds(WAYLAND_HELPER_RESTART_DELAY_SECONDS)) {
                return Ok(());
            }
        }
    }

    fn stop_input_listener(&self) {
        self.shared.stop_listener.set();
    }

    fn close(&self) {
        let Some(components) = lock(&self.shared.components).take() else { return };
        self.stop_input_listener();
        if let Some(logind) = lock(&components.logind).take() {
            logind.close();
        }
        components.adapter.close();
        components.xkb.close();
    }
}

/// The Wayland backend of this platform: the production API behind the backend's logic, and the reader of the selection hotkeys
/// built on first use. An error says why the session cannot be served (libxkbcommon is missing).
pub fn new_wayland_backend(selection_factory: Option<crate::x11_backend::SelectionFactory>) -> Result<Arc<crate::wayland_backend::WaylandBackend>, String> {
    let api = NativeWaylandApi::new()?;
    Ok(Arc::new(crate::wayland_backend::WaylandBackend::new(Arc::new(api), selection_factory)))
}

#[cfg(test)]
#[path = "wayland_native_tests.rs"]
mod tests;
