//! Isolated Wayland evdev/uinput helper protocol, device classifier and state machine (the Python's `wayland_helper.py`).
//!
//! The helper runs in its own child process so an unresponsive GTK main loop or a crash in the parent can never freeze
//! physical keyboards: if the parent does not answer a key within `WAYLAND_HELPER_VERDICT_TIMEOUT_MS`, the helper passes the
//! key through to `/dev/uinput` and switches into zero-latency pass-through until the parent catches up; if the socket
//! closes, `EVIOCGRAB` is released immediately.

use ks_cast::To as _;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::constants::{KEYBOARD_LISTENER_START_TIMEOUT_SECONDS, KEYBOARD_LISTENER_STOP_TIMEOUT_SECONDS};
use crate::wayland_access::access_hint;
use crate::wayland_constants::*;

/// seq, pressed, keycode, time, flags (`HELPER_KEY_*`): `!IBHIB`.
const MSG_KEY_SIZE: usize = 12;
/// request, count, check (`HELPER_EMIT_*`), the seq the check is against: `!IHBI`.
const CMD_BATCH_HEADER_SIZE: usize = 11;

pub const EXCLUDED_DEVICE_NAME_MARKERS: [&str; 9] =
    ["keyswitch", "ydotool", "uinput", "xremap", "kanata", "input-remapper", "power_switcher", "keyd virtual", "yubikey"];

const POLL_ERROR_MASK: i16 = libc::POLLHUP | libc::POLLERR | libc::POLLNVAL;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// A flag a thread waits for.
#[derive(Default)]
pub struct Flag {
    set: Mutex<bool>,
    changed: Condvar,
}

impl Flag {
    pub fn set(&self) {
        *lock(&self.set) = true;
        self.changed.notify_all();
    }
    pub fn clear(&self) {
        *lock(&self.set) = false;
    }
    pub fn is_set(&self) -> bool {
        *lock(&self.set)
    }
    pub fn wait(&self, timeout: Duration) -> bool {
        let guard = lock(&self.set);
        let (guard, _) = self.changed.wait_timeout_while(guard, timeout, |set| !*set).unwrap_or_else(PoisonError::into_inner);
        *guard
    }
}

pub fn seconds(value: f64) -> Duration {
    Duration::from_secs_f64(value.max(0.0))
}

/// Whether `seq` came after `than` among the helper's wrapping 32-bit key numbers.
pub fn seq_newer(seq: u32, than: u32) -> bool {
    seq != than && seq.wrapping_sub(than) <= u32::MAX >> 1
}

/// Encode one binary control/event frame between parent and helper.
pub fn pack_frame(msg_type: u32, payload: &[u8]) -> io::Result<Vec<u8>> {
    if payload.len() > HELPER_MAX_FRAME_BYTES.to::<usize>() {
        return Err(invalid("Helper frame payload exceeds limit"));
    }
    let mut frame = Vec::with_capacity(HELPER_FRAME_HEADER_BYTES.to::<usize>() + payload.len());
    frame.push(msg_type.low::<u8>());
    frame.extend_from_slice(&payload.len().low::<u32>().to_be_bytes());
    frame.extend_from_slice(payload);
    Ok(frame)
}

/// Incremental parser for length-prefixed helper frames.
#[derive(Default)]
pub struct FrameReader {
    buffer: Vec<u8>,
}

impl FrameReader {
    pub fn new() -> FrameReader {
        FrameReader::default()
    }

    pub fn feed(&mut self, chunk: &[u8]) -> io::Result<Vec<(u32, Vec<u8>)>> {
        self.buffer.extend_from_slice(chunk);
        let header = HELPER_FRAME_HEADER_BYTES.to::<usize>();
        let mut frames = Vec::new();
        while self.buffer.len() >= header {
            let msg_type = u32::from(self.buffer[0]);
            let length = u32::from_be_bytes([self.buffer[1], self.buffer[2], self.buffer[3], self.buffer[4]]).to::<usize>();
            if length > HELPER_MAX_FRAME_BYTES.to::<usize>() {
                return Err(invalid("Helper frame length exceeds limit"));
            }
            let end = header + length;
            if self.buffer.len() < end {
                break;
            }
            frames.push((msg_type, self.buffer[header..end].to_vec()));
            self.buffer.drain(..end);
        }
        Ok(frames)
    }
}

/// Capabilities of a `/dev/input/event*` node inspected before opening.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InputDeviceInfo {
    pub path: String,
    pub name: String,
    pub phys: String,
    pub bustype: u32,
    pub vendor: u32,
    pub product: u32,
    pub ev_bits: HashSet<u32>,
    pub key_bits: HashSet<u32>,
    pub rel_bits: HashSet<u32>,
    pub abs_bits: HashSet<u32>,
}

/// One kernel `struct input_event` read from an `evdev` descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawInputEvent {
    pub sec: i64,
    pub usec: i64,
    pub ev_type: u32,
    pub code: u32,
    pub value: i32,
}

impl RawInputEvent {
    pub fn timestamp_ms(&self) -> u32 {
        (self.sec.wrapping_mul(1000).wrapping_add(self.usec.div_euclid(MICROSECONDS_PER_MILLISECOND)) & i64::from(UINT32_MODULO_MASK)).low::<u32>()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceRole {
    Keyboard,
    Pointer,
    Ignore,
}

/// Classify an evdev node as a physical keyboard to grab, passive pointer, or ignored.
pub fn classify_input_device(info: &InputDeviceInfo) -> DeviceRole {
    if !info.ev_bits.contains(&EV_KEY) {
        return DeviceRole::Ignore;
    }
    if (info.vendor, info.product) == (WAYLAND_UINPUT_VENDOR_ID, WAYLAND_UINPUT_PRODUCT_ID) {
        return DeviceRole::Ignore;
    }
    let lowered_name = ks_text::pystr::casefold(&info.name);
    let lowered_phys = ks_text::pystr::casefold(&info.phys);
    if lowered_phys.starts_with("keyswitch") || EXCLUDED_DEVICE_NAME_MARKERS.iter().any(|marker| lowered_name.contains(marker)) {
        return DeviceRole::Ignore;
    }
    if info.key_bits.contains(&EVDEV_BTN_STYLUS) || info.key_bits.contains(&EVDEV_BTN_GAMEPAD) {
        return DeviceRole::Ignore;
    }
    let has_multitouch = info.ev_bits.contains(&EV_ABS) && info.abs_bits.contains(&EVDEV_ABS_MT_POSITION_X);
    let has_rel_mouse = info.ev_bits.contains(&EV_REL)
        && info.rel_bits.contains(&EVDEV_REL_X)
        && info.rel_bits.contains(&EVDEV_REL_Y)
        && info.key_bits.contains(&EVDEV_BTN_MOUSE);
    if has_rel_mouse {
        return DeviceRole::Pointer;
    }
    if !has_multitouch && EVDEV_REQUIRED_KEYBOARD_KEYS.iter().all(|key| info.key_bits.contains(key)) {
        return DeviceRole::Keyboard;
    }
    if info.key_bits.contains(&EVDEV_BTN_MOUSE) {
        return DeviceRole::Pointer;
    }
    DeviceRole::Ignore
}

/// Kernel boundary of the input helper, substituted in unit tests. Failures are `io::Error`s: a refused permission is
/// `ErrorKind::PermissionDenied`.
pub trait HelperOs {
    fn create_uinput(&mut self) -> io::Result<i32>;
    fn destroy_uinput(&mut self, fd: i32) -> io::Result<()>;
    fn write_uinput(&mut self, fd: i32, events: &[(u32, u32, i32)]) -> io::Result<()>;
    fn read_uinput_leds(&mut self, fd: i32) -> io::Result<Vec<(u32, i32)>>;
    fn list_event_devices(&mut self) -> Vec<InputDeviceInfo>;
    fn open_device(&mut self, path: &str, writable: bool) -> io::Result<i32>;
    fn close_device(&mut self, fd: i32) -> io::Result<()>;
    fn grab_device(&mut self, fd: i32) -> io::Result<()>;
    fn ungrab_device(&mut self, fd: i32) -> io::Result<()>;
    fn query_keys_down(&mut self, fd: i32) -> io::Result<HashSet<u32>>;
    fn query_caps_lock(&mut self, fd: i32) -> bool;
    fn read_events(&mut self, fd: i32) -> io::Result<Vec<RawInputEvent>>;
    fn write_led(&mut self, fd: i32, code: u32, value: i32) -> io::Result<()>;
    fn open_inotify(&mut self) -> io::Result<i32>;
    fn read_inotify(&mut self, fd: i32) -> Vec<String>;
    fn poll(&mut self, fds: &[i32], timeout_ms: i32) -> Vec<(i32, i16)>;
    fn monotonic(&mut self) -> f64;
    fn sleep(&mut self, seconds: f64);
}

/// Bidirectional byte socket shared between `HelperClient` and `WaylandInputHelper`.
pub trait ByteChannel: Send + Sync {
    /// Reads what is there; 0 bytes mean the other side closed.
    fn recv(&self, buffer: &mut [u8]) -> io::Result<usize>;
    fn send_all(&self, data: &[u8]) -> io::Result<()>;
    fn close(&self) -> io::Result<()>;
    fn fileno(&self) -> i32;
}

impl ByteChannel for UnixStream {
    fn recv(&self, buffer: &mut [u8]) -> io::Result<usize> {
        let mut stream = self;
        stream.read(buffer)
    }
    fn send_all(&self, data: &[u8]) -> io::Result<()> {
        let mut stream = self;
        stream.write_all(data)
    }
    fn close(&self) -> io::Result<()> {
        self.shutdown(std::net::Shutdown::Both)
    }
    fn fileno(&self) -> i32 {
        self.as_raw_fd()
    }
}

struct QueuedKey {
    pressed: bool,
    code: u32,
    timestamp_ms: u32,
}

struct PendingKey {
    seq: u32,
    pressed: bool,
    code: u32,
    timestamp_ms: u32,
    deadline: f64,
}

/// Child-side evdev grabber and single `/dev/uinput` writer.
pub struct WaylandInputHelper<'a> {
    channel: &'a dyn ByteChannel,
    os: Box<dyn HelperOs + 'a>,
    stop: &'a AtomicBool,
    repeat_delay_ms: u32,
    reader: FrameReader,
    uinput_fd: Option<i32>,
    inotify_fd: Option<i32>,
    keyboards: Vec<(i32, String)>,
    grabbed_fds: HashSet<i32>,
    pointers: Vec<(i32, String)>,
    device_keys_down: HashMap<i32, HashSet<u32>>,
    passed_keys: HashSet<u32>,
    press_started: HashMap<u32, f64>,
    repeat_notified: HashSet<u32>,
    key_queue: VecDeque<QueuedKey>,
    pending: Option<PendingKey>,
    /// After a late verdict: the seq the helper typed itself; until the parent acknowledges it, every key is typed at once
    /// and reported as typed.
    forced_seq: Option<u32>,
    /// The last key typed without the parent's verdict: a correction decided before the parent saw it would edit text it does
    /// not know.
    unverdicted_seq: u32,
    next_seq: u32,
    grab_enabled: bool,
    caps_lock: bool,
    running: bool,
}

impl<'a> WaylandInputHelper<'a> {
    /// `stop` is raised by whoever asks the helper to leave (a signal handler): the loop ends after the current step.
    pub fn new(channel: &'a dyn ByteChannel, os: Box<dyn HelperOs + 'a>, stop: &'a AtomicBool, repeat_delay_ms: u32) -> WaylandInputHelper<'a> {
        WaylandInputHelper {
            channel,
            os,
            stop,
            repeat_delay_ms,
            reader: FrameReader::new(),
            uinput_fd: None,
            inotify_fd: None,
            keyboards: Vec::new(),
            grabbed_fds: HashSet::new(),
            pointers: Vec::new(),
            device_keys_down: HashMap::new(),
            passed_keys: HashSet::new(),
            press_started: HashMap::new(),
            repeat_notified: HashSet::new(),
            key_queue: VecDeque::new(),
            pending: None,
            forced_seq: None,
            unverdicted_seq: 0,
            next_seq: 1,
            grab_enabled: true,
            caps_lock: false,
            running: false,
        }
    }

    pub fn run(&mut self) -> i32 {
        let outcome = self.serve();
        let status = match outcome {
            Ok(()) => 0,
            Err(error) => {
                // The parent may be gone already: the report is best effort.
                let _ = self.send(HELPER_MSG_ERROR, error.to_string().as_bytes());
                1
            }
        };
        self.cleanup();
        status
    }

    fn start_up(&mut self) -> io::Result<()> {
        self.uinput_fd = Some(self.os.create_uinput()?);
        self.os.sleep(WAYLAND_UINPUT_SETTLE_SECONDS);
        self.inotify_fd = Some(self.os.open_inotify()?);
        self.rescan_devices(true)?;
        let keyboards: Vec<i32> = self.keyboards.iter().map(|(fd, _)| *fd).collect();
        self.caps_lock = keyboards.into_iter().any(|fd| self.os.query_caps_lock(fd));
        self.send(HELPER_MSG_READY, &[u8::from(self.caps_lock)])?;
        self.running = true;
        Ok(())
    }

    fn serve(&mut self) -> io::Result<()> {
        self.start_up()?;
        while self.running && !self.stop.load(Ordering::SeqCst) {
            self.step()?;
        }
        Ok(())
    }

    fn step(&mut self) -> io::Result<()> {
        let chan_fd = self.channel.fileno();
        let (Some(uinput_fd), Some(inotify_fd)) = (self.uinput_fd, self.inotify_fd) else { return Ok(()) };
        let mut fds = vec![chan_fd, uinput_fd, inotify_fd];
        fds.extend(self.keyboards.iter().map(|(fd, _)| *fd));
        fds.extend(self.pointers.iter().map(|(fd, _)| *fd));
        let mut timeout_ms = WAYLAND_HELPER_POLL_TIMEOUT_MS;
        if let Some(pending) = &self.pending {
            let remaining = (((pending.deadline - self.os.monotonic()) * 1000.0).round_ties_even().max(0.0)).to::<i32>();
            timeout_ms = timeout_ms.min(remaining);
        }
        let events: HashMap<i32, i16> = self.os.poll(&fds, timeout_ms).into_iter().collect();

        if let Some(&mask) = events.get(&chan_fd)
            && !self.read_channel(mask)?
        {
            self.running = false;
            return Ok(());
        }

        self.expire_pending_if_due()?;

        if events.get(&uinput_fd).is_some_and(|mask| mask & libc::POLLIN != 0) {
            self.forward_uinput_leds(uinput_fd)?;
        }

        if events.get(&inotify_fd).is_some_and(|mask| mask & libc::POLLIN != 0) {
            let changed = self.os.read_inotify(inotify_fd);
            if changed.iter().any(|name| name.starts_with("event")) {
                self.rescan_devices(false)?;
            }
        }

        let keyboards: Vec<i32> = self.keyboards.iter().map(|(fd, _)| *fd).collect();
        for fd in keyboards {
            let mask = events.get(&fd).copied().unwrap_or(0);
            if mask & POLL_ERROR_MASK != 0 {
                self.close_keyboard(fd)?;
            } else if mask & libc::POLLIN != 0 {
                self.read_keyboard(fd)?;
            }
        }

        let pointers: Vec<i32> = self.pointers.iter().map(|(fd, _)| *fd).collect();
        for fd in pointers {
            let mask = events.get(&fd).copied().unwrap_or(0);
            if mask & POLL_ERROR_MASK != 0 {
                self.close_pointer(fd);
            } else if mask & libc::POLLIN != 0 {
                self.read_pointer(fd)?;
            }
        }
        Ok(())
    }

    fn read_channel(&mut self, mask: i16) -> io::Result<bool> {
        if mask & libc::POLLIN != 0 {
            let mut chunk = vec![0u8; HELPER_SOCKET_CHUNK_BYTES.to::<usize>()];
            let count = match self.channel.recv(&mut chunk) {
                Ok(count) => count,
                Err(_) => return Ok(false),
            };
            if count == 0 {
                return Ok(false);
            }
            for (msg_type, payload) in self.reader.feed(&chunk[..count])? {
                if !self.handle_command(msg_type, &payload)? {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        Ok(mask & POLL_ERROR_MASK == 0)
    }

    fn handle_command(&mut self, msg_type: u32, payload: &[u8]) -> io::Result<bool> {
        let short = |what: &str| invalid(format!("{what}: the payload is too short"));
        match msg_type {
            HELPER_CMD_STOP => return Ok(false),
            HELPER_CMD_VERDICT => {
                let (seq, swallow) = match payload {
                    [a, b, c, d, swallow, ..] => (u32::from_be_bytes([*a, *b, *c, *d]), *swallow != 0),
                    _ => return Err(short("verdict")),
                };
                self.apply_verdict(seq, swallow)?;
            }
            HELPER_CMD_EMIT_BATCH => {
                if payload.len() < CMD_BATCH_HEADER_SIZE {
                    return Err(short("emit batch"));
                }
                let req_id = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let count = usize::from(u16::from_be_bytes([payload[4], payload[5]]));
                let check = u32::from(payload[6]);
                let known_seq = u32::from_be_bytes([payload[7], payload[8], payload[9], payload[10]]);
                if self.refuses(check, known_seq) {
                    // A key reached the application after the parent decided on this text, or the withheld key is there
                    // already: refused whole.
                    self.send(HELPER_MSG_EMIT_ACK, &ack(req_id, 0))?;
                    return Ok(true);
                }
                let settled = check == HELPER_EMIT_ACTION && self.pending.as_ref().is_some_and(|pending| pending.seq == known_seq);
                if settled {
                    // The parent types the key it is still to answer: it has handled it, swallowed.
                    self.pending = None;
                }
                let items = &payload[CMD_BATCH_HEADER_SIZE..];
                if items.len() < count * HELPER_EMIT_ITEM_BYTES.to::<usize>() {
                    return Err(short("emit batch"));
                }
                let mut raw_events: Vec<(u32, u32, i32)> = Vec::new();
                let mut emitted = 0u32;
                for item in items.chunks(HELPER_EMIT_ITEM_BYTES.to::<usize>()).take(count) {
                    let pressed = item[0] != 0;
                    let keycode = u32::from(u16::from_be_bytes([item[1], item[2]]));
                    if pressed && self.passed_keys.contains(&keycode) {
                        // The key is still down in the virtual keyboard (the user holds it): the kernel would drop a second
                        // press, and the character typed with it would be missing.
                        raw_events.push((EV_KEY, keycode, KEY_VALUE_RELEASE));
                        raw_events.push((EV_SYN, SYN_REPORT, 0));
                    }
                    if pressed {
                        self.passed_keys.insert(keycode);
                    } else {
                        self.passed_keys.remove(&keycode);
                    }
                    raw_events.push((EV_KEY, keycode, if pressed { KEY_VALUE_PRESS } else { KEY_VALUE_RELEASE }));
                    raw_events.push((EV_SYN, SYN_REPORT, 0));
                    emitted += 1;
                }
                if let (false, Some(fd)) = (raw_events.is_empty(), self.uinput_fd) {
                    self.write_paced(fd, &raw_events)?;
                }
                self.send(HELPER_MSG_EMIT_ACK, &ack(req_id, emitted))?;
                if settled {
                    self.dispatch_next_queued_key()?;
                }
            }
            HELPER_CMD_SEQ_ACK => {
                let seq = match payload {
                    [a, b, c, d, ..] => u32::from_be_bytes([*a, *b, *c, *d]),
                    _ => return Err(short("sequence acknowledgement")),
                };
                if self.forced_seq == Some(seq) {
                    self.forced_seq = None;
                }
            }
            HELPER_CMD_QUERY_KEY_DOWN => {
                let (req_id, keycode) = match payload {
                    [a, b, c, d, e, f, ..] => (u32::from_be_bytes([*a, *b, *c, *d]), u32::from(u16::from_be_bytes([*e, *f]))),
                    _ => return Err(short("key query")),
                };
                let down = self.is_key_physically_down(keycode)?;
                let mut reply = req_id.to_be_bytes().to_vec();
                reply.push(u8::from(down));
                self.send(HELPER_MSG_KEY_DOWN, &reply)?;
            }
            HELPER_CMD_SET_GRAB => {
                let (req_id, enabled, repeat_delay_ms) = match payload {
                    [a, b, c, d, enabled, e, f, ..] => {
                        (u32::from_be_bytes([*a, *b, *c, *d]), *enabled != 0, u32::from(u16::from_be_bytes([*e, *f])))
                    }
                    _ => return Err(short("grab request")),
                };
                if repeat_delay_ms > 0 {
                    self.repeat_delay_ms = repeat_delay_ms;
                }
                self.set_grab_enabled(enabled)?;
                self.send(HELPER_MSG_EMIT_ACK, &ack(req_id, u32::from(self.grab_enabled)))?;
            }
            _ => {}
        }
        Ok(true)
    }

    /// Whether an injection would land after keys typed without the parent's verdict.
    fn refuses(&self, check: u32, known_seq: u32) -> bool {
        match check {
            HELPER_EMIT_TEXT => self.forced_seq.is_some() || seq_newer(self.unverdicted_seq, known_seq),
            HELPER_EMIT_ACTION => {
                self.forced_seq.is_some() || (self.unverdicted_seq != 0 && !seq_newer(known_seq, self.unverdicted_seq))
            }
            _ => false,
        }
    }

    /// Write key transitions in bursts the compositor's event queue holds (see the burst constant).
    fn write_paced(&mut self, fd: i32, raw_events: &[(u32, u32, i32)]) -> io::Result<()> {
        let burst = (WAYLAND_UINPUT_BURST_TRANSITIONS * TRANSITION_EVENTS).to::<usize>();
        for (index, part) in raw_events.chunks(burst).enumerate() {
            if index > 0 {
                self.os.sleep(WAYLAND_UINPUT_BURST_PAUSE_SECONDS);
            }
            self.os.write_uinput(fd, part)?;
        }
        Ok(())
    }

    fn apply_verdict(&mut self, seq: u32, swallow: bool) -> io::Result<()> {
        let Some(pending) = self.pending.take_if(|pending| pending.seq == seq) else {
            // Late: the key was typed when its time ran out (`expire_pending_if_due`).
            return Ok(());
        };
        if !swallow {
            self.emit_physical_key(pending.pressed, pending.code)?;
        }
        self.dispatch_next_queued_key()
    }

    fn expire_pending_if_due(&mut self) -> io::Result<()> {
        let now = self.os.monotonic();
        let Some(pending) = self.pending.take_if(|pending| now >= pending.deadline) else { return Ok(()) };
        self.forced_seq = Some(pending.seq);
        self.unverdicted_seq = pending.seq;
        self.emit_physical_key(pending.pressed, pending.code)?;
        // Reported before the queued keys, in the order the application received them.
        self.send_key(pending.seq, pending.pressed, pending.code, pending.timestamp_ms, HELPER_KEY_FORCED)?;
        while let Some(item) = self.key_queue.pop_front() {
            self.type_unasked(&item)?;
        }
        Ok(())
    }

    /// Type a key without the parent's verdict, and tell the parent so that it misses none.
    fn type_unasked(&mut self, item: &QueuedKey) -> io::Result<()> {
        let seq = self.allocate_seq();
        self.unverdicted_seq = seq;
        self.emit_physical_key(item.pressed, item.code)?;
        self.send_key(seq, item.pressed, item.code, item.timestamp_ms, 0)
    }

    fn enqueue_physical_key(&mut self, item: QueuedKey) -> io::Result<()> {
        if self.forced_seq.is_some() {
            return self.type_unasked(&item);
        }
        self.key_queue.push_back(item);
        if self.pending.is_none() {
            self.dispatch_next_queued_key()?;
        }
        Ok(())
    }

    fn allocate_seq(&mut self) -> u32 {
        let seq = self.next_seq.wrapping_add(1);
        let seq = if seq == 0 { 1 } else { seq };
        self.next_seq = seq;
        seq
    }

    fn dispatch_next_queued_key(&mut self) -> io::Result<()> {
        let Some(item) = self.key_queue.pop_front() else { return Ok(()) };
        let seq = self.allocate_seq();
        let deadline = self.os.monotonic() + f64::from(WAYLAND_HELPER_VERDICT_TIMEOUT_MS) / 1000.0;
        self.pending = Some(PendingKey { seq, pressed: item.pressed, code: item.code, timestamp_ms: item.timestamp_ms, deadline });
        self.send_key(seq, item.pressed, item.code, item.timestamp_ms, HELPER_KEY_NEEDS_VERDICT)
    }

    fn send_key(&mut self, seq: u32, pressed: bool, code: u32, timestamp_ms: u32, flags: u32) -> io::Result<()> {
        let mut payload = Vec::with_capacity(MSG_KEY_SIZE);
        payload.extend_from_slice(&seq.to_be_bytes());
        payload.push(u8::from(pressed));
        payload.extend_from_slice(&code.low::<u16>().to_be_bytes());
        payload.extend_from_slice(&timestamp_ms.to_be_bytes());
        payload.push(flags.low::<u8>());
        self.send(HELPER_MSG_KEY, &payload)
    }

    fn emit_physical_key(&mut self, pressed: bool, code: u32) -> io::Result<()> {
        let Some(fd) = self.uinput_fd else { return Ok(()) };
        if pressed {
            self.passed_keys.insert(code);
        } else {
            self.passed_keys.remove(&code);
        }
        self.os.write_uinput(fd, &[(EV_KEY, code, if pressed { KEY_VALUE_PRESS } else { KEY_VALUE_RELEASE }), (EV_SYN, SYN_REPORT, 0)])
    }

    fn read_keyboard(&mut self, fd: i32) -> io::Result<()> {
        let events = match self.os.read_events(fd) {
            Ok(events) => events,
            Err(_) => return self.close_keyboard(fd),
        };
        if !self.grab_enabled || !self.grabbed_fds.contains(&fd) {
            return Ok(());
        }
        let now = self.os.monotonic();
        self.device_keys_down.entry(fd).or_default();
        for event in events {
            if event.ev_type != EV_KEY {
                continue;
            }
            let code = event.code;
            if event.value == KEY_VALUE_REPEAT {
                if self.passed_keys.contains(&code)
                    && !EVDEV_MODIFIER_KEYS.contains(&code)
                    && code != EVDEV_KEY_CAPSLOCK
                    && !self.repeat_notified.contains(&code)
                {
                    let started = self.press_started.get(&code).copied().unwrap_or(now);
                    let elapsed_ms = (now - started) * 1000.0;
                    if elapsed_ms >= f64::from(self.repeat_delay_ms) {
                        self.repeat_notified.insert(code);
                        self.send(HELPER_MSG_REPEAT_RESET, &event.timestamp_ms().to_be_bytes())?;
                    }
                }
                continue;
            }
            if event.value == KEY_VALUE_PRESS {
                self.device_keys_down.entry(fd).or_default().insert(code);
                self.press_started.insert(code, now);
                self.repeat_notified.remove(&code);
                self.enqueue_physical_key(QueuedKey { pressed: true, code, timestamp_ms: event.timestamp_ms() })?;
            } else if event.value == KEY_VALUE_RELEASE {
                self.device_keys_down.entry(fd).or_default().remove(&code);
                self.press_started.remove(&code);
                self.repeat_notified.remove(&code);
                self.enqueue_physical_key(QueuedKey { pressed: false, code, timestamp_ms: event.timestamp_ms() })?;
            }
        }
        Ok(())
    }

    fn read_pointer(&mut self, fd: i32) -> io::Result<()> {
        let events = match self.os.read_events(fd) {
            Ok(events) => events,
            Err(_) => {
                self.close_pointer(fd);
                return Ok(());
            }
        };
        if !self.grab_enabled {
            return Ok(());
        }
        for event in events {
            if event.ev_type == EV_KEY && (EVDEV_BTN_MOUSE..=EVDEV_BTN_TASK).contains(&event.code) && event.value == KEY_VALUE_PRESS {
                self.send(HELPER_MSG_POINTER, &event.timestamp_ms().to_be_bytes())?;
            }
        }
        Ok(())
    }

    fn forward_uinput_leds(&mut self, uinput_fd: i32) -> io::Result<()> {
        let Ok(leds) = self.os.read_uinput_leds(uinput_fd) else { return Ok(()) };
        for (code, value) in leds {
            let keyboards: Vec<i32> = self.keyboards.iter().map(|(fd, _)| *fd).collect();
            for keyboard in keyboards {
                // A keyboard that refuses the light is left as it is.
                let _ = self.os.write_led(keyboard, code, value);
            }
            if code == LED_CAPSL {
                let enabled = value != 0;
                if enabled != self.caps_lock {
                    self.caps_lock = enabled;
                    self.send(HELPER_MSG_CAPS_LOCK, &[u8::from(enabled)])?;
                }
            }
        }
        Ok(())
    }

    fn is_key_physically_down(&mut self, keycode: u32) -> io::Result<bool> {
        let keyboards: Vec<i32> = self.keyboards.iter().map(|(fd, _)| *fd).collect();
        for fd in keyboards {
            match self.os.query_keys_down(fd) {
                Ok(down) => {
                    if down.contains(&keycode) {
                        return Ok(true);
                    }
                }
                Err(_) => self.close_keyboard(fd)?,
            }
        }
        Ok(false)
    }

    fn set_grab_enabled(&mut self, enabled: bool) -> io::Result<()> {
        if enabled == self.grab_enabled {
            return Ok(());
        }
        self.grab_enabled = enabled;
        // The keys down so far went through the grab: the parent forgets them either way.
        self.send(HELPER_MSG_INPUT_RESET, &[])?;
        if !enabled {
            self.pending = None;
            self.forced_seq = None;
            self.key_queue.clear();
            self.press_started.clear();
            self.repeat_notified.clear();
            self.release_all_passed_keys();
            let grabbed: Vec<i32> = self.grabbed_fds.iter().copied().collect();
            for fd in grabbed {
                let _ = self.os.ungrab_device(fd);
                if let Some(down) = self.device_keys_down.get_mut(&fd) {
                    down.clear();
                }
            }
            self.grabbed_fds.clear();
            return Ok(());
        }
        self.rescan_devices(false)?;
        let keyboards: Vec<i32> = self.keyboards.iter().map(|(fd, _)| *fd).collect();
        for fd in keyboards {
            if !self.grabbed_fds.contains(&fd) {
                self.grab_keyboard_safely(fd);
            }
        }
        Ok(())
    }

    fn rescan_devices(&mut self, initial: bool) -> io::Result<()> {
        let known_keyboards: Vec<(String, i32)> = self.keyboards.iter().map(|(fd, path)| (path.clone(), *fd)).collect();
        let known_pointers: Vec<(String, i32)> = self.pointers.iter().map(|(fd, path)| (path.clone(), *fd)).collect();
        let mut seen_paths: HashSet<String> = HashSet::new();
        let mut denied_keyboard = false;

        for info in self.os.list_event_devices() {
            seen_paths.insert(info.path.clone());
            let role = classify_input_device(&info);
            if role == DeviceRole::Keyboard && !known_keyboards.iter().any(|(path, _)| *path == info.path) {
                let fd = match self.os.open_device(&info.path, true) {
                    Ok(fd) => fd,
                    Err(error) if error.kind() == io::ErrorKind::PermissionDenied => match self.os.open_device(&info.path, false) {
                        Ok(fd) => fd,
                        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                            denied_keyboard = true;
                            continue;
                        }
                        Err(_) => continue,
                    },
                    Err(_) => continue,
                };
                self.keyboards.push((fd, info.path.clone()));
                self.device_keys_down.insert(fd, HashSet::new());
                if self.grab_enabled && !self.grab_keyboard_safely(fd) {
                    self.close_keyboard(fd)?;
                }
            } else if role == DeviceRole::Pointer
                && !known_pointers.iter().any(|(path, _)| *path == info.path)
                && let Ok(fd) = self.os.open_device(&info.path, false)
            {
                self.pointers.push((fd, info.path.clone()));
            }
        }

        for (path, fd) in known_keyboards {
            if !seen_paths.contains(&path) {
                self.close_keyboard(fd)?;
            }
        }
        for (path, fd) in known_pointers {
            if !seen_paths.contains(&path) {
                self.close_pointer(fd);
            }
        }

        if initial && denied_keyboard && self.keyboards.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("Нет доступа к клавиатурам /dev/input/event*: {}", access_hint()),
            ));
        }
        Ok(())
    }

    fn grab_keyboard_safely(&mut self, fd: i32) -> bool {
        let attempt = (|| -> io::Result<()> {
            let mut down = self.os.query_keys_down(fd)?;
            if !down.is_empty() {
                let deadline = self.os.monotonic() + WAYLAND_GRAB_WAIT_TIMEOUT_SECONDS;
                while !down.is_empty() && self.os.monotonic() < deadline {
                    self.os.sleep(WAYLAND_GRAB_POLL_SECONDS);
                    down = self.os.query_keys_down(fd)?;
                }
                if down.is_empty() {
                    self.os.sleep(WAYLAND_GRAB_RELEASE_SETTLE_SECONDS);
                }
            }
            self.os.grab_device(fd)?;
            self.os.read_events(fd)?;
            Ok(())
        })();
        if attempt.is_err() {
            return false;
        }
        self.grabbed_fds.insert(fd);
        true
    }

    fn close_keyboard(&mut self, fd: i32) -> io::Result<()> {
        self.keyboards.retain(|(known, _)| *known != fd);
        let held = self.device_keys_down.remove(&fd).unwrap_or_default();
        if self.grabbed_fds.remove(&fd) {
            let _ = self.os.ungrab_device(fd);
        }
        let _ = self.os.close_device(fd);
        let still_held: HashSet<u32> = self.device_keys_down.values().flatten().copied().collect();
        let mut orphaned: Vec<u32> = held.intersection(&self.passed_keys).filter(|code| !still_held.contains(code)).copied().collect();
        orphaned.sort_unstable();
        for code in orphaned {
            self.emit_physical_key(false, code)?;
        }
        Ok(())
    }

    fn close_pointer(&mut self, fd: i32) {
        self.pointers.retain(|(known, _)| *known != fd);
        let _ = self.os.close_device(fd);
    }

    fn release_all_passed_keys(&mut self) {
        let Some(fd) = self.uinput_fd.filter(|_| !self.passed_keys.is_empty()) else {
            self.passed_keys.clear();
            return;
        };
        let mut codes: Vec<u32> = self.passed_keys.iter().copied().collect();
        codes.sort_unstable();
        let events: Vec<(u32, u32, i32)> = codes.into_iter().flat_map(|code| [(EV_KEY, code, KEY_VALUE_RELEASE), (EV_SYN, SYN_REPORT, 0)]).collect();
        self.passed_keys.clear();
        let _ = self.os.write_uinput(fd, &events);
    }

    fn cleanup(&mut self) {
        self.release_all_passed_keys();
        let keyboards: Vec<i32> = self.keyboards.iter().map(|(fd, _)| *fd).collect();
        for fd in keyboards {
            let _ = self.close_keyboard(fd);
        }
        let pointers: Vec<i32> = self.pointers.iter().map(|(fd, _)| *fd).collect();
        for fd in pointers {
            self.close_pointer(fd);
        }
        if let Some(fd) = self.inotify_fd.take() {
            let _ = self.os.close_device(fd);
        }
        if let Some(fd) = self.uinput_fd.take() {
            let _ = self.os.destroy_uinput(fd);
        }
    }

    fn send(&self, msg_type: u32, payload: &[u8]) -> io::Result<()> {
        self.channel.send_all(&pack_frame(msg_type, payload)?)
    }
}

fn ack(req_id: u32, value: u32) -> [u8; 8] {
    let mut bytes = [0u8; 8];
    bytes[..4].copy_from_slice(&req_id.to_be_bytes());
    bytes[4..].copy_from_slice(&value.to_be_bytes());
    bytes
}

/// A physical key the helper reports: asked about, typed by the helper after a late verdict (`forced`, the same seq as when
/// asked), or typed without asking (neither flag).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HelperKey {
    pub seq: u32,
    pub pressed: bool,
    pub keycode: u32,
    pub timestamp_ms: u32,
    pub flags: u32,
}

impl HelperKey {
    pub fn needs_verdict(&self) -> bool {
        self.flags & HELPER_KEY_NEEDS_VERDICT != 0
    }
    pub fn forced(&self) -> bool {
        self.flags & HELPER_KEY_FORCED != 0
    }
}

struct PendingReply {
    event: Flag,
    value: Mutex<u32>,
    failed: AtomicBool,
}

type KeyCallback = Box<dyn Fn(HelperKey) -> bool + Send + Sync>;
type TimestampCallback = Box<dyn Fn(u32) + Send + Sync>;
type CapsCallback = Box<dyn Fn(bool) + Send + Sync>;
type ResetCallback = Box<dyn Fn() + Send + Sync>;

/// What the client tells its owner.
pub struct HelperCallbacks {
    pub on_key: KeyCallback,
    pub on_pointer: TimestampCallback,
    pub on_repeat_reset: TimestampCallback,
    pub on_caps_lock: CapsCallback,
    pub on_input_reset: ResetCallback,
}

/// What the reader thread hands the dispatching one: a message, or the end.
type Event = Option<(u32, Vec<u8>)>;

struct ClientShared {
    channel: Arc<dyn ByteChannel>,
    callbacks: HelperCallbacks,
    send_lock: Mutex<()>,
    replies: Mutex<HashMap<u32, Arc<PendingReply>>>,
    next_req_id: Mutex<u32>,
    ready: Flag,
    closed: AtomicBool,
    error: Mutex<String>,
    caps_lock: AtomicBool,
    events: Mutex<Option<Sender<Event>>>,
}

/// Parent-side client speaking the binary helper protocol without callback deadlocks.
pub struct HelperClient {
    shared: Arc<ClientShared>,
    receiver: Mutex<Option<Receiver<Event>>>,
    threads: Mutex<Vec<JoinHandle<()>>>,
}

impl HelperClient {
    pub fn new(channel: Arc<dyn ByteChannel>, callbacks: HelperCallbacks) -> HelperClient {
        let (sender, receiver) = std::sync::mpsc::channel();
        HelperClient {
            shared: Arc::new(ClientShared {
                channel,
                callbacks,
                send_lock: Mutex::new(()),
                replies: Mutex::new(HashMap::new()),
                next_req_id: Mutex::new(1),
                ready: Flag::default(),
                closed: AtomicBool::new(false),
                error: Mutex::new(String::new()),
                caps_lock: AtomicBool::new(false),
                events: Mutex::new(Some(sender)),
            }),
            receiver: Mutex::new(Some(receiver)),
            threads: Mutex::new(Vec::new()),
        }
    }

    pub fn caps_lock(&self) -> bool {
        self.shared.caps_lock.load(Ordering::SeqCst)
    }

    /// The helper's socket is gone: it exited, or the client was closed.
    pub fn closed(&self) -> bool {
        self.shared.closed.load(Ordering::SeqCst)
    }

    /// Starts the threads and waits for the helper to say it is ready; the Caps Lock state it reports.
    pub fn start(&self, timeout: f64) -> Result<bool, String> {
        let reader_shared = self.shared.clone();
        let reader = thread::Builder::new()
            .name("keyswitch-wayland-helper-read".to_owned())
            .spawn(move || reader_shared.read_loop())
            .map_err(|error| error.to_string())?;
        let dispatch_shared = self.shared.clone();
        let receiver = lock(&self.receiver).take().ok_or_else(|| "The helper client was started already".to_owned())?;
        let dispatcher = thread::Builder::new()
            .name("keyswitch-wayland-helper-dispatch".to_owned())
            .spawn(move || dispatch_shared.dispatch_loop(&receiver))
            .map_err(|error| error.to_string())?;
        lock(&self.threads).extend([reader, dispatcher]);
        if !self.shared.ready.wait(seconds(timeout)) {
            self.close();
            return Err("Процесс перехвата клавиатуры Wayland не ответил вовремя".to_owned());
        }
        let error = lock(&self.shared.error).clone();
        if !error.is_empty() {
            self.close();
            return Err(error);
        }
        if self.closed() {
            self.close();
            return Err("Процесс перехвата клавиатуры Wayland завершился при запуске".to_owned());
        }
        Ok(self.caps_lock())
    }

    pub fn start_default(&self) -> Result<bool, String> {
        self.start(KEYBOARD_LISTENER_START_TIMEOUT_SECONDS)
    }

    /// Type `items`; with `known_seq` the helper types none of them (the answer is 0) if a key it typed without a verdict came
    /// after that seq - or, for an `action`, if it typed that very key itself already.
    pub fn emit_batch(&self, items: &[(bool, u32)], known_seq: Option<u32>, action: bool) -> Result<u32, String> {
        if items.is_empty() {
            return Ok(0);
        }
        let check = match (known_seq, action) {
            (None, _) => HELPER_EMIT_UNCHECKED,
            (Some(_), true) => HELPER_EMIT_ACTION,
            (Some(_), false) => HELPER_EMIT_TEXT,
        };
        let (req_id, slot) = self.shared.allocate_reply();
        let mut body = Vec::with_capacity(CMD_BATCH_HEADER_SIZE + items.len() * HELPER_EMIT_ITEM_BYTES.to::<usize>());
        body.extend_from_slice(&req_id.to_be_bytes());
        body.extend_from_slice(&items.len().low::<u16>().to_be_bytes());
        body.push(check.low::<u8>());
        body.extend_from_slice(&known_seq.unwrap_or(0).to_be_bytes());
        for &(pressed, keycode) in items {
            body.push(u8::from(pressed));
            body.extend_from_slice(&keycode.low::<u16>().to_be_bytes());
        }
        self.shared.request(req_id, &slot, HELPER_CMD_EMIT_BATCH, &body)
    }

    pub fn query_key_down(&self, keycode: u32) -> Result<bool, String> {
        let (req_id, slot) = self.shared.allocate_reply();
        let mut payload = req_id.to_be_bytes().to_vec();
        payload.extend_from_slice(&keycode.low::<u16>().to_be_bytes());
        Ok(self.shared.request(req_id, &slot, HELPER_CMD_QUERY_KEY_DOWN, &payload)? != 0)
    }

    pub fn set_grab(&self, enabled: bool, repeat_delay_ms: u32) -> Result<bool, String> {
        let (req_id, slot) = self.shared.allocate_reply();
        let mut payload = req_id.to_be_bytes().to_vec();
        payload.push(u8::from(enabled));
        payload.extend_from_slice(&repeat_delay_ms.low::<u16>().to_be_bytes());
        Ok(self.shared.request(req_id, &slot, HELPER_CMD_SET_GRAB, &payload)? != 0)
    }

    pub fn close(&self) {
        if !self.shared.closed.swap(true, Ordering::SeqCst) {
            // The helper may be gone already.
            let _ = self.shared.send_frame(HELPER_CMD_STOP, &[]);
        }
        let _ = self.shared.channel.close();
        self.shared.fail_pending_replies();
        self.shared.finish_events();
        self.shared.ready.set();
        let current = thread::current().id();
        let threads: Vec<JoinHandle<()>> = lock(&self.threads).drain(..).collect();
        for handle in threads {
            if handle.thread().id() != current {
                // The threads end on their own once the channel is shut; waiting longer than the stop timeout is no use.
                let deadline = std::time::Instant::now() + seconds(KEYBOARD_LISTENER_STOP_TIMEOUT_SECONDS);
                while !handle.is_finished() && std::time::Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(1));
                }
                if handle.is_finished() {
                    let _ = handle.join();
                }
            }
        }
    }
}

impl ClientShared {
    fn allocate_reply(&self) -> (u32, Arc<PendingReply>) {
        let mut next = lock(&self.next_req_id);
        let id = next.wrapping_add(1);
        let id = if id == 0 { 1 } else { id };
        *next = id;
        let slot = Arc::new(PendingReply { event: Flag::default(), value: Mutex::new(0), failed: AtomicBool::new(false) });
        lock(&self.replies).insert(id, slot.clone());
        (id, slot)
    }

    fn request(&self, req_id: u32, slot: &PendingReply, cmd_type: u32, payload: &[u8]) -> Result<u32, String> {
        let outcome = (|| {
            self.send_frame(cmd_type, payload).map_err(|_| "Нет ответа от процесса ввода Wayland".to_owned())?;
            if !slot.event.wait(seconds(WAYLAND_HELPER_COMMAND_TIMEOUT_SECONDS)) || slot.failed.load(Ordering::SeqCst) {
                return Err("Нет ответа от процесса ввода Wayland".to_owned());
            }
            Ok(*lock(&slot.value))
        })();
        lock(&self.replies).remove(&req_id);
        outcome
    }

    fn send_frame(&self, msg_type: u32, payload: &[u8]) -> io::Result<()> {
        let frame = pack_frame(msg_type, payload)?;
        let _guard = lock(&self.send_lock);
        self.channel.send_all(&frame)
    }

    fn read_loop(&self) {
        let mut reader = FrameReader::new();
        let mut chunk = vec![0u8; HELPER_SOCKET_CHUNK_BYTES.to::<usize>()];
        'reading: while !self.closed.load(Ordering::SeqCst) {
            let count = match self.channel.recv(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(count) => count,
            };
            let Ok(frames) = reader.feed(&chunk[..count]) else { break };
            for (msg_type, payload) in frames {
                if self.route_incoming(msg_type, payload).is_err() {
                    break 'reading;
                }
            }
        }
        self.closed.store(true, Ordering::SeqCst);
        self.fail_pending_replies();
        self.finish_events();
        self.ready.set();
    }

    fn finish_events(&self) {
        if let Some(sender) = lock(&self.events).take() {
            let _ = sender.send(None);
        }
    }

    fn route_incoming(&self, msg_type: u32, payload: Vec<u8>) -> Result<(), ()> {
        match msg_type {
            HELPER_MSG_READY => {
                let caps = *payload.first().ok_or(())?;
                self.caps_lock.store(caps != 0, Ordering::SeqCst);
                self.ready.set();
            }
            HELPER_MSG_ERROR => {
                *lock(&self.error) = String::from_utf8_lossy(&payload).into_owned();
                self.ready.set();
            }
            HELPER_MSG_EMIT_ACK if payload.len() >= 8 => {
                let req_id = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let value = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
                self.resolve_reply(req_id, value);
            }
            HELPER_MSG_KEY_DOWN if payload.len() >= 5 => {
                let req_id = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                self.resolve_reply(req_id, u32::from(payload[4]));
            }
            HELPER_MSG_EMIT_ACK | HELPER_MSG_KEY_DOWN => return Err(()),
            _ => {
                if let Some(sender) = lock(&self.events).as_ref() {
                    let _ = sender.send(Some((msg_type, payload)));
                }
            }
        }
        Ok(())
    }

    fn resolve_reply(&self, req_id: u32, value: u32) {
        if let Some(slot) = lock(&self.replies).get(&req_id) {
            *lock(&slot.value) = value;
            slot.event.set();
        }
    }

    fn fail_pending_replies(&self) {
        for slot in lock(&self.replies).values() {
            slot.failed.store(true, Ordering::SeqCst);
            slot.event.set();
        }
    }

    fn dispatch_loop(&self, receiver: &Receiver<Event>) {
        while let Ok(Some((msg_type, payload))) = receiver.recv() {
            match msg_type {
                HELPER_MSG_KEY if payload.len() >= MSG_KEY_SIZE => {
                    let key = HelperKey {
                        seq: u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]),
                        pressed: payload[4] != 0,
                        keycode: u32::from(u16::from_be_bytes([payload[5], payload[6]])),
                        timestamp_ms: u32::from_be_bytes([payload[7], payload[8], payload[9], payload[10]]),
                        flags: u32::from(payload[11]),
                    };
                    let swallow = (self.callbacks.on_key)(key);
                    let sent = if key.needs_verdict() {
                        let verdict = if swallow { HELPER_VERDICT_SWALLOW } else { HELPER_VERDICT_PASS };
                        let mut body = key.seq.to_be_bytes().to_vec();
                        body.push(verdict.low::<u8>());
                        self.send_frame(HELPER_CMD_VERDICT, &body)
                    } else if key.forced() {
                        self.send_frame(HELPER_CMD_SEQ_ACK, &key.seq.to_be_bytes())
                    } else {
                        Ok(())
                    };
                    if sent.is_err() {
                        return;
                    }
                }
                HELPER_MSG_INPUT_RESET => (self.callbacks.on_input_reset)(),
                HELPER_MSG_POINTER if payload.len() >= 4 => {
                    (self.callbacks.on_pointer)(u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]));
                }
                HELPER_MSG_REPEAT_RESET if payload.len() >= 4 => {
                    (self.callbacks.on_repeat_reset)(u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]));
                }
                HELPER_MSG_CAPS_LOCK if !payload.is_empty() => {
                    let enabled = payload[0] != 0;
                    self.caps_lock.store(enabled, Ordering::SeqCst);
                    (self.callbacks.on_caps_lock)(enabled);
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
#[path = "wayland_helper_tests.rs"]
mod tests;
