//! The Python's `test_wayland_helper.py` on a fake kernel and a fake socket.

use crate::wayland_constants::{EVDEV_KEY_A, EVDEV_KEY_PAUSE, EVDEV_KEY_Q, EVDEV_KEY_Z, WAYLAND_REPEAT_DEFAULT_DELAY_MS};
use super::*;
use std::sync::atomic::AtomicI32;
use std::time::Instant;

const SEQ_BEFORE_WRAP: u32 = 0xFFFF_FFFE;
const SEQ_AFTER_WRAP: u32 = 3;

fn keyboard_info(path: &str) -> InputDeviceInfo {
    InputDeviceInfo {
        path: path.to_owned(),
        name: "AT Translated Set 2 keyboard".to_owned(),
        phys: "isa0060/serio0/input0".to_owned(),
        ev_bits: HashSet::from([EV_KEY]),
        key_bits: HashSet::from(EVDEV_REQUIRED_KEYBOARD_KEYS),
        ..InputDeviceInfo::default()
    }
}

fn pointer_info(path: &str) -> InputDeviceInfo {
    InputDeviceInfo {
        path: path.to_owned(),
        name: "Mouse".to_owned(),
        ev_bits: HashSet::from([EV_KEY, EV_REL]),
        key_bits: HashSet::from([EVDEV_BTN_MOUSE]),
        rel_bits: HashSet::from([EVDEV_REL_X, EVDEV_REL_Y]),
        ..InputDeviceInfo::default()
    }
}

#[derive(Default)]
struct OsState {
    devices: Vec<InputDeviceInfo>,
    next_fd: i32,
    open: HashMap<i32, String>,
    deny_writable: HashSet<String>,
    deny_all: bool,
    events: HashMap<i32, VecDeque<RawInputEvent>>,
    grabbed: HashSet<i32>,
    keys_down: HashMap<i32, HashSet<u32>>,
    now: f64,
    writes: Vec<(u32, u32, i32)>,
    write_calls: Vec<usize>,
    sleeps: Vec<f64>,
    destroyed: bool,
    fail_create: bool,
    polls: VecDeque<Vec<(i32, i16)>>,
    inotify: Vec<String>,
    failing_reads: HashSet<i32>,
    failing_queries: HashSet<i32>,
    no_leds: bool,
    /// A device that cannot be opened at all, or only read.
    broken: HashSet<String>,
    broken_read_only: HashSet<String>,
    /// Keys that come up after so many looks.
    keys_up_after: usize,
}

#[derive(Clone)]
struct FakeOs(Arc<Mutex<OsState>>);

impl FakeOs {
    fn new(devices: Vec<InputDeviceInfo>) -> FakeOs {
        FakeOs(Arc::new(Mutex::new(OsState { devices, next_fd: 10, ..OsState::default() })))
    }
    fn st(&self) -> MutexGuard<'_, OsState> {
        lock(&self.0)
    }
    fn fd_of(&self, path: &str) -> i32 {
        *self.st().open.iter().find(|(_, name)| name.as_str() == path).map(|(fd, _)| fd).expect("the device is open")
    }
    fn press(&self, fd: i32, code: u32, value: i32) {
        self.st().events.entry(fd).or_default().push_back(RawInputEvent { sec: 5, usec: 2500, ev_type: EV_KEY, code, value });
    }
    fn typed(&self) -> Vec<(u32, i32)> {
        self.st().writes.iter().filter(|(ev_type, _, _)| *ev_type == EV_KEY).map(|(_, code, value)| (*code, *value)).collect()
    }
}

impl HelperOs for FakeOs {
    fn create_uinput(&mut self) -> io::Result<i32> {
        if self.st().fail_create {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "[Errno 13] Permission denied: '/dev/uinput'"));
        }
        Ok(3)
    }
    fn destroy_uinput(&mut self, _fd: i32) -> io::Result<()> {
        self.st().destroyed = true;
        Ok(())
    }
    fn write_uinput(&mut self, _fd: i32, events: &[(u32, u32, i32)]) -> io::Result<()> {
        let mut state = self.st();
        state.writes.extend_from_slice(events);
        state.write_calls.push(events.len());
        Ok(())
    }
    fn read_uinput_leds(&mut self, _fd: i32) -> io::Result<Vec<(u32, i32)>> {
        if self.st().no_leds {
            return Err(io::Error::from(io::ErrorKind::WouldBlock));
        }
        Ok(vec![(LED_CAPSL, 1), (LED_CAPSL, 0), (LED_CAPSL, 0), (0, 1)])
    }
    fn list_event_devices(&mut self) -> Vec<InputDeviceInfo> {
        self.st().devices.clone()
    }
    fn open_device(&mut self, path: &str, writable: bool) -> io::Result<i32> {
        let mut state = self.st();
        if state.broken.contains(path) || (!writable && state.broken_read_only.contains(path)) {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        if state.deny_all || (writable && state.deny_writable.contains(path)) {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        state.next_fd += 1;
        let fd = state.next_fd;
        state.open.insert(fd, path.to_owned());
        Ok(fd)
    }
    fn close_device(&mut self, fd: i32) -> io::Result<()> {
        self.st().open.remove(&fd);
        Ok(())
    }
    fn grab_device(&mut self, fd: i32) -> io::Result<()> {
        self.st().grabbed.insert(fd);
        Ok(())
    }
    fn ungrab_device(&mut self, fd: i32) -> io::Result<()> {
        self.st().grabbed.remove(&fd);
        Ok(())
    }
    fn query_keys_down(&mut self, fd: i32) -> io::Result<HashSet<u32>> {
        if self.st().failing_queries.contains(&fd) {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        let mut state = self.st();
        if state.keys_up_after > 0 {
            state.keys_up_after -= 1;
            if state.keys_up_after == 0 {
                state.keys_down.clear();
            }
        }
        Ok(state.keys_down.get(&fd).cloned().unwrap_or_default())
    }
    fn query_caps_lock(&mut self, _fd: i32) -> bool {
        false
    }
    fn read_events(&mut self, fd: i32) -> io::Result<Vec<RawInputEvent>> {
        if self.st().failing_reads.contains(&fd) {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        Ok(self.st().events.entry(fd).or_default().drain(..).collect())
    }
    fn write_led(&mut self, _fd: i32, _code: u32, _value: i32) -> io::Result<()> {
        Ok(())
    }
    fn open_inotify(&mut self) -> io::Result<i32> {
        Ok(4)
    }
    fn read_inotify(&mut self, _fd: i32) -> Vec<String> {
        std::mem::take(&mut self.st().inotify)
    }
    fn poll(&mut self, _fds: &[i32], _timeout_ms: i32) -> Vec<(i32, i16)> {
        self.st().polls.pop_front().unwrap_or_default()
    }
    fn monotonic(&mut self) -> f64 {
        self.st().now
    }
    fn sleep(&mut self, seconds_to_wait: f64) {
        let mut state = self.st();
        state.sleeps.push(seconds_to_wait);
        state.now += seconds_to_wait;
    }
}

/// A socket the test fills with the parent's frames and reads the helper's frames from.
struct FakeChannel {
    inbound: Mutex<VecDeque<u8>>,
    outbound: Mutex<Vec<u8>>,
    fd: AtomicI32,
    broken: AtomicBool,
}

impl FakeChannel {
    fn new() -> FakeChannel {
        FakeChannel { inbound: Mutex::new(VecDeque::new()), outbound: Mutex::new(Vec::new()), broken: AtomicBool::new(false), fd: AtomicI32::new(2) }
    }
    fn command(&self, msg_type: u32, payload: &[u8]) {
        lock(&self.inbound).extend(pack_frame(msg_type, payload).unwrap());
    }
    fn sent(&self) -> Vec<(u32, Vec<u8>)> {
        let bytes = std::mem::take(&mut *lock(&self.outbound));
        FrameReader::new().feed(&bytes).unwrap()
    }
}

impl ByteChannel for FakeChannel {
    fn recv(&self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.broken.load(Ordering::SeqCst) {
            return Err(io::Error::from(io::ErrorKind::ConnectionReset));
        }
        let mut inbound = lock(&self.inbound);
        let count = buffer.len().min(inbound.len());
        for slot in buffer.iter_mut().take(count) {
            *slot = inbound.pop_front().expect("counted");
        }
        Ok(count)
    }
    fn send_all(&self, data: &[u8]) -> io::Result<()> {
        lock(&self.outbound).extend_from_slice(data);
        Ok(())
    }
    fn close(&self) -> io::Result<()> {
        Ok(())
    }
    fn fileno(&self) -> i32 {
        self.fd.load(Ordering::SeqCst)
    }
}

static NEVER: AtomicBool = AtomicBool::new(false);

struct Rig {
    channel: Arc<FakeChannel>,
    os: FakeOs,
}

impl Rig {
    fn new(devices: Vec<InputDeviceInfo>) -> Rig {
        Rig { channel: Arc::new(FakeChannel::new()), os: FakeOs::new(devices) }
    }
    fn helper(&self) -> WaylandInputHelper<'_> {
        WaylandInputHelper::new(&*self.channel, Box::new(self.os.clone()), &NEVER, WAYLAND_REPEAT_DEFAULT_DELAY_MS)
    }
}

fn key_frame(frames: &[(u32, Vec<u8>)], index: usize) -> (u32, bool, u32, u32, u32) {
    let (msg_type, payload) = &frames[index];
    assert_eq!(*msg_type, HELPER_MSG_KEY);
    (
        u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]),
        payload[4] != 0,
        u32::from(u16::from_be_bytes([payload[5], payload[6]])),
        u32::from_be_bytes([payload[7], payload[8], payload[9], payload[10]]),
        u32::from(payload[11]),
    )
}

fn batch(req_id: u32, check: u32, known_seq: u32, items: &[(bool, u32)]) -> Vec<u8> {
    let mut body = req_id.to_be_bytes().to_vec();
    body.extend(items.len().low::<u16>().to_be_bytes());
    body.push(check.low::<u8>());
    body.extend(known_seq.to_be_bytes());
    for (pressed, code) in items {
        body.push(u8::from(*pressed));
        body.extend((*code).low::<u16>().to_be_bytes());
    }
    body
}

fn ack_of(frames: &[(u32, Vec<u8>)]) -> (u32, u32) {
    let (msg_type, payload) = frames.iter().find(|(msg_type, _)| *msg_type == HELPER_MSG_EMIT_ACK).expect("an acknowledgement");
    assert_eq!(*msg_type, HELPER_MSG_EMIT_ACK);
    (u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]), u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]))
}

fn verdict(helper: &mut WaylandInputHelper<'_>, seq: u32, swallow: bool) {
    let mut payload = seq.to_be_bytes().to_vec();
    payload.push(u8::from(swallow));
    assert!(helper.handle_command(HELPER_CMD_VERDICT, &payload).unwrap());
}

#[test]
fn frames_are_packed_and_read_incrementally() {
    let mut reader = FrameReader::new();
    let first = pack_frame(HELPER_MSG_KEY, &[1, 2, 3]).unwrap();
    let second = pack_frame(HELPER_MSG_POINTER, &[]).unwrap();
    let bytes = [first.clone(), second].concat();
    assert!(reader.feed(&bytes[..4]).unwrap().is_empty());
    assert_eq!(reader.feed(&bytes[4..first.len() + 2]).unwrap(), vec![(HELPER_MSG_KEY, vec![1, 2, 3])]);
    assert_eq!(reader.feed(&bytes[first.len() + 2..]).unwrap(), vec![(HELPER_MSG_POINTER, Vec::new())]);
    assert!(pack_frame(1, &vec![0; HELPER_MAX_FRAME_BYTES.to::<usize>() + 1]).is_err());
    let mut oversized = vec![1u8];
    oversized.extend((HELPER_MAX_FRAME_BYTES + 1).to_be_bytes());
    assert!(FrameReader::new().feed(&oversized).is_err());
    assert_eq!(&first[..5], &[HELPER_MSG_KEY.low::<u8>(), 0, 0, 0, 3]);
}

#[test]
fn devices_are_told_apart_by_what_they_can_do() {
    assert_eq!(classify_input_device(&keyboard_info("/dev/input/event0")), DeviceRole::Keyboard);
    assert_eq!(classify_input_device(&pointer_info("/dev/input/event1")), DeviceRole::Pointer);
    let mut ours = keyboard_info("/dev/input/event2");
    (ours.vendor, ours.product) = (WAYLAND_UINPUT_VENDOR_ID, WAYLAND_UINPUT_PRODUCT_ID);
    assert_eq!(classify_input_device(&ours), DeviceRole::Ignore);
    for name in ["ydotoold virtual device", "Yubico YubiKey OTP", "keyd virtual keyboard", "Input-Remapper keyboard"] {
        assert_eq!(classify_input_device(&InputDeviceInfo { name: name.to_owned(), ..keyboard_info("/dev/input/event3") }), DeviceRole::Ignore, "{name}");
    }
    let own_phys = InputDeviceInfo { phys: "KeySwitch/virtual".to_owned(), ..keyboard_info("/dev/input/event4") };
    assert_eq!(classify_input_device(&own_phys), DeviceRole::Ignore);
    let mut tablet = keyboard_info("/dev/input/event5");
    tablet.key_bits.insert(EVDEV_BTN_STYLUS);
    assert_eq!(classify_input_device(&tablet), DeviceRole::Ignore);
    // A touch device with the letters is no keyboard; a device with only a button is a pointer; no keys is nothing.
    let mut touch = keyboard_info("/dev/input/event6");
    touch.ev_bits.insert(EV_ABS);
    touch.abs_bits.insert(EVDEV_ABS_MT_POSITION_X);
    assert_eq!(classify_input_device(&touch), DeviceRole::Ignore);
    touch.key_bits.insert(EVDEV_BTN_MOUSE);
    assert_eq!(classify_input_device(&touch), DeviceRole::Pointer);
    assert_eq!(classify_input_device(&InputDeviceInfo::default()), DeviceRole::Ignore);
}

#[test]
fn key_numbers_compare_across_the_wrap() {
    assert!(seq_newer(6, 5) && !seq_newer(5, 6) && !seq_newer(5, 5));
    assert!(seq_newer(SEQ_AFTER_WRAP, SEQ_BEFORE_WRAP) && !seq_newer(SEQ_BEFORE_WRAP, SEQ_AFTER_WRAP));
    let helper_rig = Rig::new(Vec::new());
    let mut helper = helper_rig.helper();
    helper.next_seq = u32::MAX;
    assert_eq!(helper.allocate_seq(), 1);
}

#[test]
fn startup_failure_sends_the_error_and_a_key_waits_for_the_verdict() {
    let rig = Rig::new(vec![keyboard_info("/dev/input/event0")]);
    rig.os.st().fail_create = true;
    assert_eq!(rig.helper().run(), 1);
    let frames = rig.channel.sent();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].0, HELPER_MSG_ERROR);
    assert!(String::from_utf8_lossy(&frames[0].1).contains("/dev/uinput"));

    let rig = Rig::new(vec![keyboard_info("/dev/input/event0"), pointer_info("/dev/input/event1")]);
    let mut helper = rig.helper();
    helper.start_up().unwrap();
    assert_eq!(rig.channel.sent()[0].0, HELPER_MSG_READY);
    let keyboard = rig.os.fd_of("/dev/input/event0");
    assert!(rig.os.st().grabbed.contains(&keyboard));
    // A press is reported with a number and held back; the verdict decides whether it is typed.
    rig.os.press(keyboard, EVDEV_KEY_A, KEY_VALUE_PRESS);
    helper.read_keyboard(keyboard).unwrap();
    let frames = rig.channel.sent();
    let (seq, pressed, code, timestamp, flags) = key_frame(&frames, 0);
    assert_eq!((pressed, code, timestamp, flags), (true, EVDEV_KEY_A, 5002, HELPER_KEY_NEEDS_VERDICT));
    assert!(rig.os.typed().is_empty());
    // The next key waits behind it.
    rig.os.press(keyboard, EVDEV_KEY_A, KEY_VALUE_RELEASE);
    helper.read_keyboard(keyboard).unwrap();
    assert!(rig.channel.sent().is_empty());
    verdict(&mut helper, seq, false);
    assert_eq!(rig.os.typed(), vec![(EVDEV_KEY_A, KEY_VALUE_PRESS)]);
    let frames = rig.channel.sent();
    let (next, pressed, _, _, _) = key_frame(&frames, 0);
    assert!(!pressed && next != seq);
    verdict(&mut helper, next, true);
    assert_eq!(rig.os.typed(), vec![(EVDEV_KEY_A, KEY_VALUE_PRESS)]);
    // A verdict for a key that is no longer waiting changes nothing.
    verdict(&mut helper, 999, false);
    assert_eq!(rig.os.typed().len(), 1);
    helper.cleanup();
    assert!(rig.os.st().destroyed && rig.os.st().grabbed.is_empty());
}

#[test]
fn a_verdict_that_comes_too_late_makes_the_helper_type_and_refuse_corrections() {
    let rig = Rig::new(vec![keyboard_info("/dev/input/event0")]);
    let mut helper = rig.helper();
    helper.start_up().unwrap();
    rig.channel.sent();
    let keyboard = rig.os.fd_of("/dev/input/event0");
    rig.os.press(keyboard, EVDEV_KEY_A, KEY_VALUE_PRESS);
    helper.read_keyboard(keyboard).unwrap();
    rig.os.press(keyboard, EVDEV_KEY_Q, KEY_VALUE_PRESS);
    helper.read_keyboard(keyboard).unwrap();
    let asked = key_frame(&rig.channel.sent(), 0).0;
    // The time runs out: the key is typed, reported as forced, and the queue after it is typed and reported unasked.
    rig.os.st().now += f64::from(WAYLAND_HELPER_VERDICT_TIMEOUT_MS) / 1000.0 + 0.01;
    helper.expire_pending_if_due().unwrap();
    assert_eq!(rig.os.typed(), vec![(EVDEV_KEY_A, KEY_VALUE_PRESS), (EVDEV_KEY_Q, KEY_VALUE_PRESS)]);
    let frames = rig.channel.sent();
    assert_eq!(key_frame(&frames, 0), (asked, true, EVDEV_KEY_A, 5002, HELPER_KEY_FORCED));
    assert_eq!(key_frame(&frames, 1).4, 0);
    let unasked = key_frame(&frames, 1).0;
    // A late verdict only lets the release through; keys now are typed at once until the parent acknowledges the forced one.
    verdict(&mut helper, asked, true);
    rig.os.press(keyboard, EVDEV_KEY_Z, KEY_VALUE_PRESS);
    helper.read_keyboard(keyboard).unwrap();
    assert_eq!(rig.os.typed().len(), 3);
    assert_eq!(key_frame(&rig.channel.sent(), 0).4, 0);
    // Text decided before those keys is refused whole; so is any injection while the forced key is not acknowledged.
    rig.channel.command(HELPER_CMD_EMIT_BATCH, &batch(1, HELPER_EMIT_TEXT, asked, &[(true, EVDEV_KEY_A)]));
    helper.read_channel(libc::POLLIN).unwrap();
    assert_eq!(ack_of(&rig.channel.sent()), (1, 0));
    assert_eq!(rig.os.typed().len(), 3);
    assert!(helper.handle_command(HELPER_CMD_SEQ_ACK, &asked.to_be_bytes()).unwrap());
    // Still refused: the unasked keys came after the number the parent knows.
    assert!(helper.refuses(HELPER_EMIT_TEXT, unasked.wrapping_sub(1)));
    assert!(!helper.refuses(HELPER_EMIT_TEXT, helper.unverdicted_seq));
    assert!(!helper.refuses(HELPER_EMIT_UNCHECKED, 0));
    // The withheld key itself: typed once, checked against its own number.
    assert!(helper.refuses(HELPER_EMIT_ACTION, helper.unverdicted_seq));
    assert!(!helper.refuses(HELPER_EMIT_ACTION, helper.unverdicted_seq + 1));
}

#[test]
fn an_injection_releases_a_key_still_down_and_is_written_in_bursts() {
    let rig = Rig::new(vec![keyboard_info("/dev/input/event0")]);
    let mut helper = rig.helper();
    helper.start_up().unwrap();
    rig.channel.sent();
    // The user holds A: the virtual keyboard's A is released before the correction presses it.
    helper.passed_keys.insert(EVDEV_KEY_A);
    rig.channel.command(HELPER_CMD_EMIT_BATCH, &batch(7, HELPER_EMIT_UNCHECKED, 0, &[(true, EVDEV_KEY_A), (false, EVDEV_KEY_A)]));
    helper.read_channel(libc::POLLIN).unwrap();
    assert_eq!(ack_of(&rig.channel.sent()), (7, 2));
    assert_eq!(rig.os.typed(), vec![(EVDEV_KEY_A, 0), (EVDEV_KEY_A, 1), (EVDEV_KEY_A, 0)]);
    // 72 events (a whole word) are written in bursts of 8 transitions, with a pause between them.
    let items: Vec<(bool, u32)> = (0..36).map(|index| (index % 2 == 0, EVDEV_KEY_Q)).collect();
    rig.os.st().write_calls.clear();
    rig.os.st().sleeps.clear();
    rig.channel.command(HELPER_CMD_EMIT_BATCH, &batch(8, HELPER_EMIT_UNCHECKED, 0, &items));
    helper.read_channel(libc::POLLIN).unwrap();
    assert_eq!(ack_of(&rig.channel.sent()), (8, 36));
    assert_eq!(rig.os.st().write_calls, vec![16, 16, 16, 16, 8]);
    assert_eq!(rig.os.st().sleeps, vec![WAYLAND_UINPUT_BURST_PAUSE_SECONDS; 4]);
}

#[test]
fn a_grab_change_tells_the_parent_to_forget_held_keys_and_ends_a_pass_through() {
    let rig = Rig::new(vec![keyboard_info("/dev/input/event0"), pointer_info("/dev/input/event1")]);
    let mut helper = rig.helper();
    helper.start_up().unwrap();
    rig.channel.sent();
    let keyboard = rig.os.fd_of("/dev/input/event0");
    rig.os.press(keyboard, EVDEV_KEY_A, KEY_VALUE_PRESS);
    helper.read_keyboard(keyboard).unwrap();
    rig.channel.sent();
    let mut request = 9u32.to_be_bytes().to_vec();
    request.push(0);
    request.extend(0u16.to_be_bytes());
    helper.handle_command(HELPER_CMD_SET_GRAB, &request).unwrap();
    let frames = rig.channel.sent();
    assert_eq!(frames[0].0, HELPER_MSG_INPUT_RESET);
    assert_eq!(ack_of(&frames), (9, 0));
    assert!(rig.os.st().grabbed.is_empty() && helper.pending.is_none() && helper.key_queue.is_empty());
    // Not grabbed, a key is not reported; asking for the grab again takes the keyboards back.
    rig.os.press(keyboard, EVDEV_KEY_Q, KEY_VALUE_PRESS);
    helper.read_keyboard(keyboard).unwrap();
    assert!(rig.channel.sent().is_empty());
    request[4] = 1;
    helper.handle_command(HELPER_CMD_SET_GRAB, &request).unwrap();
    assert_eq!(ack_of(&rig.channel.sent()), (9, 1));
    assert!(rig.os.st().grabbed.contains(&keyboard));
    // The pointer's button is reported for the word to restart.
    let pointer = rig.os.fd_of("/dev/input/event1");
    rig.os.press(pointer, EVDEV_BTN_MOUSE, KEY_VALUE_PRESS);
    helper.read_pointer(pointer).unwrap();
    assert_eq!(rig.channel.sent()[0], (HELPER_MSG_POINTER, 5002u32.to_be_bytes().to_vec()));
}

#[test]
fn devices_without_permission_fall_back_and_a_missing_one_releases_its_keys() {
    let rig = Rig::new(vec![keyboard_info("/dev/input/event0")]);
    rig.os.st().deny_writable.insert("/dev/input/event0".to_owned());
    let mut helper = rig.helper();
    helper.start_up().unwrap();
    // Read-only, the keyboard cannot be lit but is still grabbed.
    assert_eq!(helper.keyboards.len(), 1);

    let rig = Rig::new(vec![keyboard_info("/dev/input/event0")]);
    rig.os.st().deny_all = true;
    let mut helper = rig.helper();
    let error = helper.start_up().unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert!(error.to_string().contains("--enable-wayland-input"));

    // A keyboard that goes away while the user holds a key lets that key go on the virtual keyboard.
    let rig = Rig::new(vec![keyboard_info("/dev/input/event0")]);
    let mut helper = rig.helper();
    helper.start_up().unwrap();
    rig.channel.sent();
    let keyboard = rig.os.fd_of("/dev/input/event0");
    rig.os.press(keyboard, EVDEV_KEY_A, KEY_VALUE_PRESS);
    helper.read_keyboard(keyboard).unwrap();
    let asked = key_frame(&rig.channel.sent(), 0).0;
    verdict(&mut helper, asked, false);
    rig.os.st().devices.clear();
    helper.rescan_devices(false).unwrap();
    assert!(helper.keyboards.is_empty());
    assert_eq!(rig.os.typed(), vec![(EVDEV_KEY_A, KEY_VALUE_PRESS), (EVDEV_KEY_A, KEY_VALUE_RELEASE)]);
}

#[test]
fn the_grab_waits_for_held_keys_to_go_up() {
    let rig = Rig::new(vec![keyboard_info("/dev/input/event0")]);
    let mut helper = rig.helper();
    helper.start_up().unwrap();
    let keyboard = rig.os.fd_of("/dev/input/event0");
    helper.grabbed_fds.clear();
    rig.os.st().grabbed.clear();
    rig.os.st().sleeps.clear();
    rig.os.st().keys_down.insert(keyboard, HashSet::from([EVDEV_KEY_A]));
    // A key that never goes up: the helper gives up waiting after the timeout and grabs anyway.
    assert!(helper.grab_keyboard_safely(keyboard));
    let waited: f64 = rig.os.st().sleeps.iter().sum();
    assert!(waited >= WAYLAND_GRAB_WAIT_TIMEOUT_SECONDS);
    assert!(rig.os.st().grabbed.contains(&keyboard));
}

/// A peer that speaks the helper's side of the protocol over a real socket.
fn peer_pair() -> (HelperClient, UnixStream, Arc<Mutex<Vec<HelperKey>>>) {
    let (parent, helper_side) = UnixStream::pair().unwrap();
    let keys: Arc<Mutex<Vec<HelperKey>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = keys.clone();
    let callbacks = HelperCallbacks {
        on_key: Box::new(move |key| {
            lock(&sink).push(key);
            key.keycode == EVDEV_KEY_PAUSE
        }),
        on_pointer: Box::new(|_| {}),
        on_repeat_reset: Box::new(|_| {}),
        on_caps_lock: Box::new(|_| {}),
        on_input_reset: Box::new(|| {}),
    };
    (HelperClient::new(Arc::new(parent), callbacks), helper_side, keys)
}

fn read_frame(stream: &mut UnixStream) -> (u32, Vec<u8>) {
    let mut header = [0u8; 5];
    stream.read_exact(&mut header).unwrap();
    let length = u32::from_be_bytes([header[1], header[2], header[3], header[4]]).to::<usize>();
    let mut payload = vec![0u8; length];
    stream.read_exact(&mut payload).unwrap();
    (u32::from(header[0]), payload)
}

#[test]
fn the_client_starts_asks_and_answers_the_helpers_keys() {
    let (client, mut helper_side, keys) = peer_pair();
    helper_side.write_all(&pack_frame(HELPER_MSG_READY, &[1]).unwrap()).unwrap();
    assert!(client.start(2.0).unwrap());
    assert!(client.caps_lock());

    let peer = thread::spawn(move || {
        // The helper asks about a key and gets the verdict; then answers a batch and a key query.
        let mut key = 77u32.to_be_bytes().to_vec();
        key.push(1);
        key.extend(EVDEV_KEY_PAUSE.low::<u16>().to_be_bytes());
        key.extend(1234u32.to_be_bytes());
        key.push(HELPER_KEY_NEEDS_VERDICT.low::<u8>());
        helper_side.write_all(&pack_frame(HELPER_MSG_KEY, &key).unwrap()).unwrap();
        // The verdict and the request come from two threads of the client, in either order.
        let mut frames = vec![read_frame(&mut helper_side), read_frame(&mut helper_side)];
        frames.sort();
        assert_eq!(frames[0], (HELPER_CMD_VERDICT, vec![0, 0, 0, 77, HELPER_VERDICT_SWALLOW.low::<u8>()]));
        let (msg_type, payload) = frames.remove(1);
        assert_eq!(msg_type, HELPER_CMD_EMIT_BATCH);
        assert_eq!(payload[6], HELPER_EMIT_TEXT.low::<u8>());
        assert_eq!(u32::from_be_bytes([payload[7], payload[8], payload[9], payload[10]]), 5);
        helper_side.write_all(&pack_frame(HELPER_MSG_EMIT_ACK, &ack(u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]), 2)).unwrap()).unwrap();
        let (msg_type, payload) = read_frame(&mut helper_side);
        assert_eq!(msg_type, HELPER_CMD_QUERY_KEY_DOWN);
        let mut reply = payload[..4].to_vec();
        reply.push(1);
        helper_side.write_all(&pack_frame(HELPER_MSG_KEY_DOWN, &reply).unwrap()).unwrap();
        // The client closes: it tells the helper to stop.
        read_frame(&mut helper_side)
    });
    assert_eq!(client.emit_batch(&[(true, EVDEV_KEY_A), (false, EVDEV_KEY_A)], Some(5), false).unwrap(), 2);
    assert!(client.query_key_down(EVDEV_KEY_A).unwrap());
    assert_eq!(lock(&keys).clone(), vec![HelperKey { seq: 77, pressed: true, keycode: EVDEV_KEY_PAUSE, timestamp_ms: 1234, flags: HELPER_KEY_NEEDS_VERDICT }]);
    assert_eq!(client.emit_batch(&[], None, false).unwrap(), 0);
    client.close();
    assert!(client.closed());
    assert_eq!(peer.join().unwrap().0, HELPER_CMD_STOP);
    // A request on a closed client fails at once.
    assert!(client.emit_batch(&[(true, 1)], None, false).is_err());
}

#[test]
fn the_client_reports_a_helper_that_fails_or_stays_silent() {
    let (client, mut helper_side, _keys) = peer_pair();
    helper_side.write_all(&pack_frame(HELPER_MSG_ERROR, "нет доступа".as_bytes()).unwrap()).unwrap();
    assert_eq!(client.start(2.0).unwrap_err(), "нет доступа");

    let (client, helper_side, _keys) = peer_pair();
    assert!(client.start(0.05).unwrap_err().contains("не ответил вовремя"));
    drop(helper_side);

    let (client, helper_side, _keys) = peer_pair();
    drop(helper_side);
    assert!(client.start(2.0).unwrap_err().contains("завершился при запуске"));

    // A helper that never answers a request: the wait ends after the command timeout.
    let (client, mut helper_side, _keys) = peer_pair();
    helper_side.write_all(&pack_frame(HELPER_MSG_READY, &[0]).unwrap()).unwrap();
    client.start(2.0).unwrap();
    let started = Instant::now();
    assert!(client.set_grab(false, 0).unwrap_err().contains("Нет ответа"));
    assert!(started.elapsed() >= seconds(WAYLAND_HELPER_COMMAND_TIMEOUT_SECONDS));
    client.close();
    drop(helper_side);
}

// ---------------------------------------------------------------- the main loop

const UINPUT_FD: i32 = 3;
const INOTIFY_FD: i32 = 4;
const CHANNEL_FD: i32 = 2;

impl Rig {
    fn poll(&self, fd: i32, mask: i16) {
        self.os.st().polls.push_back(vec![(fd, mask)]);
    }
}

#[test]
fn the_main_loop_answers_each_device_as_the_kernel_reports_it() {
    let rig = Rig::new(vec![keyboard_info("/dev/input/event1"), pointer_info("/dev/input/event2")]);
    let mut helper = rig.helper();
    helper.start_up().unwrap();
    let keyboard = rig.os.fd_of("/dev/input/event1");
    let pointer = rig.os.fd_of("/dev/input/event2");
    rig.channel.sent();

    // A quiet moment, then the lights the system asked for.
    helper.step().unwrap();
    rig.poll(UINPUT_FD, libc::POLLIN);
    helper.step().unwrap();
    let frames = rig.channel.sent();
    assert_eq!(frames.iter().filter(|(kind, _)| *kind == HELPER_MSG_CAPS_LOCK).count(), 2, "on, then off");
    rig.os.st().no_leds = true;
    rig.poll(UINPUT_FD, libc::POLLIN);
    helper.step().unwrap();
    rig.poll(UINPUT_FD, libc::POLLOUT);
    helper.step().unwrap();

    // A device comes and another notice says nothing of devices.
    rig.os.st().inotify = vec!["by-id".to_owned()];
    rig.poll(INOTIFY_FD, libc::POLLIN);
    helper.step().unwrap();
    rig.os.st().devices.push(keyboard_info("/dev/input/event3"));
    rig.os.st().inotify = vec!["event3".to_owned()];
    rig.poll(INOTIFY_FD, libc::POLLIN);
    helper.step().unwrap();
    assert!(rig.os.st().open.values().any(|path| path == "/dev/input/event3"));

    // A key from the keyboard and a click from the pointer.
    rig.os.press(keyboard, EVDEV_KEY_A, KEY_VALUE_PRESS);
    rig.poll(keyboard, libc::POLLIN);
    helper.step().unwrap();
    assert!(rig.channel.sent().iter().any(|(kind, _)| *kind == HELPER_MSG_KEY));
    rig.os.st().events.entry(pointer).or_default().push_back(RawInputEvent { sec: 5, usec: 0, ev_type: EV_KEY, code: EVDEV_BTN_MOUSE, value: KEY_VALUE_PRESS });
    rig.os.st().events.entry(pointer).or_default().push_back(RawInputEvent { sec: 5, usec: 0, ev_type: EV_KEY, code: EVDEV_BTN_MOUSE, value: KEY_VALUE_RELEASE });
    rig.poll(pointer, libc::POLLIN);
    helper.step().unwrap();
    assert_eq!(rig.channel.sent().iter().filter(|(kind, _)| *kind == HELPER_MSG_POINTER).count(), 1);

    // A device that stops answering, or is reported gone, is closed.
    rig.os.st().failing_reads.insert(pointer);
    rig.poll(pointer, libc::POLLIN);
    helper.step().unwrap();
    assert!(!rig.os.st().open.contains_key(&pointer));
    let second = rig.os.fd_of("/dev/input/event3");
    rig.os.st().failing_reads.insert(second);
    rig.poll(second, libc::POLLIN);
    helper.step().unwrap();
    assert!(!rig.os.st().open.contains_key(&second));
    rig.poll(keyboard, libc::POLLERR);
    helper.step().unwrap();
    assert!(!rig.os.st().open.contains_key(&keyboard));
}

#[test]
fn a_pointer_reported_gone_is_closed_and_clicks_are_ignored_without_the_grab() {
    let rig = Rig::new(vec![keyboard_info("/dev/input/event1"), pointer_info("/dev/input/event2")]);
    let mut helper = rig.helper();
    helper.start_up().unwrap();
    let pointer = rig.os.fd_of("/dev/input/event2");
    let keyboard = rig.os.fd_of("/dev/input/event1");
    helper.grab_enabled = false;
    rig.os.st().events.entry(pointer).or_default().push_back(RawInputEvent { sec: 5, usec: 0, ev_type: EV_KEY, code: EVDEV_BTN_MOUSE, value: KEY_VALUE_PRESS });
    rig.channel.sent();
    helper.read_pointer(pointer).unwrap();
    helper.read_keyboard(keyboard).unwrap();
    assert!(rig.channel.sent().is_empty());
    helper.grab_enabled = true;
    rig.poll(pointer, libc::POLLHUP);
    helper.step().unwrap();
    assert!(!rig.os.st().open.contains_key(&pointer));
    rig.os.st().devices.retain(|device| device.path != "/dev/input/event1");
    helper.rescan_devices(false).unwrap();
    assert!(!rig.os.st().open.contains_key(&keyboard));
}

#[test]
fn the_loop_ends_with_the_channel_and_on_stop() {
    let rig = Rig::new(vec![keyboard_info("/dev/input/event1")]);
    let mut helper = rig.helper();
    helper.start_up().unwrap();
    rig.poll(CHANNEL_FD, libc::POLLIN);
    helper.step().unwrap();
    assert!(!helper.running, "nothing to read: the parent is gone");
    helper.running = true;
    rig.poll(CHANNEL_FD, libc::POLLHUP);
    helper.step().unwrap();
    assert!(!helper.running);
    helper.running = true;
    rig.poll(CHANNEL_FD, libc::POLLOUT);
    helper.step().unwrap();
    assert!(helper.running, "no words and no trouble");
    rig.channel.command(HELPER_CMD_STOP, &[]);
    rig.poll(CHANNEL_FD, libc::POLLIN);
    helper.step().unwrap();
    assert!(!helper.running);

    let rig = Rig::new(vec![keyboard_info("/dev/input/event1")]);
    rig.channel.command(HELPER_CMD_STOP, &[]);
    rig.poll(CHANNEL_FD, libc::POLLIN);
    assert_eq!(rig.helper().run(), 0, "a whole run ends well");

    let rig = Rig::new(vec![keyboard_info("/dev/input/event1")]);
    let stopping = AtomicBool::new(true);
    let mut helper = WaylandInputHelper::new(&*rig.channel, Box::new(rig.os.clone()), &stopping, WAYLAND_REPEAT_DEFAULT_DELAY_MS);
    assert_eq!(helper.run(), 0, "the stop flag ends it");

    // A helper that was not started has nothing to look after.
    let rig = Rig::new(Vec::new());
    rig.helper().step().unwrap();
}

#[test]
fn a_command_cut_short_is_an_error_and_an_unknown_one_is_passed_over() {
    let rig = Rig::new(vec![keyboard_info("/dev/input/event1")]);
    let mut helper = rig.helper();
    helper.start_up().unwrap();
    for command in [HELPER_CMD_VERDICT, HELPER_CMD_EMIT_BATCH, HELPER_CMD_SEQ_ACK, HELPER_CMD_QUERY_KEY_DOWN, HELPER_CMD_SET_GRAB] {
        assert!(helper.handle_command(command, &[1]).is_err(), "{command}");
    }
    assert!(helper.handle_command(HELPER_CMD_EMIT_BATCH, &batch(1, HELPER_EMIT_UNCHECKED, 0, &[(true, EVDEV_KEY_A)])[..12]).is_err());
    assert!(helper.handle_command(9_999, &[]).unwrap());
    assert!(!helper.handle_command(HELPER_CMD_STOP, &[]).unwrap());
}

#[test]
fn the_parent_asks_whether_a_key_is_down() {
    let rig = Rig::new(vec![keyboard_info("/dev/input/event1"), keyboard_info("/dev/input/event2")]);
    let mut helper = rig.helper();
    helper.start_up().unwrap();
    let keyboard = rig.os.fd_of("/dev/input/event1");
    let other = rig.os.fd_of("/dev/input/event2");
    rig.channel.sent();
    let ask = |helper: &mut WaylandInputHelper<'_>, code: u32| {
        let mut payload = 7_u32.to_be_bytes().to_vec();
        payload.extend(code.low::<u16>().to_be_bytes());
        assert!(helper.handle_command(HELPER_CMD_QUERY_KEY_DOWN, &payload).unwrap());
        rig.channel.sent().last().unwrap().1[4]
    };
    assert_eq!(ask(&mut helper, EVDEV_KEY_A), 0);
    rig.os.st().keys_down.insert(other, HashSet::from([EVDEV_KEY_A]));
    assert_eq!(ask(&mut helper, EVDEV_KEY_A), 1);
    rig.os.st().failing_queries.insert(keyboard);
    assert_eq!(ask(&mut helper, EVDEV_KEY_Q), 0);
    assert!(!rig.os.st().open.contains_key(&keyboard), "a device that does not answer is closed");
}

#[test]
fn a_held_key_that_repeats_tells_the_parent_once_after_the_delay() {
    let rig = Rig::new(vec![keyboard_info("/dev/input/event1")]);
    let mut helper = rig.helper();
    helper.start_up().unwrap();
    let keyboard = rig.os.fd_of("/dev/input/event1");
    rig.os.press(keyboard, EVDEV_KEY_A, KEY_VALUE_PRESS);
    helper.read_keyboard(keyboard).unwrap();
    let frames = rig.channel.sent();
    let sequence = key_frame(&frames, frames.len() - 1).0;
    verdict(&mut helper, sequence, false);
    rig.channel.sent();
    let resets = |helper: &mut WaylandInputHelper<'_>, code: u32| {
        rig.os.press(keyboard, code, KEY_VALUE_REPEAT);
        helper.read_keyboard(keyboard).unwrap();
        rig.channel.sent().iter().filter(|(kind, _)| *kind == HELPER_MSG_REPEAT_RESET).count()
    };
    assert_eq!(resets(&mut helper, EVDEV_KEY_A), 0, "too soon");
    rig.os.st().now += 10.0;
    assert_eq!(resets(&mut helper, EVDEV_KEY_A), 1);
    assert_eq!(resets(&mut helper, EVDEV_KEY_A), 0, "only once");
    assert_eq!(resets(&mut helper, EVDEV_KEY_Z), 0, "a key that did not pass");
    rig.os.st().events.entry(keyboard).or_default().push_back(RawInputEvent { sec: 5, usec: 0, ev_type: 4, code: 4, value: 1 });
    helper.read_keyboard(keyboard).unwrap();
}

#[test]
fn the_parent_changes_the_repeat_delay_with_the_grab() {
    let rig = Rig::new(vec![keyboard_info("/dev/input/event1")]);
    let mut helper = rig.helper();
    helper.start_up().unwrap();
    let mut payload = 3_u32.to_be_bytes().to_vec();
    payload.push(1);
    payload.extend(450_u16.to_be_bytes());
    assert!(helper.handle_command(HELPER_CMD_SET_GRAB, &payload).unwrap());
    assert_eq!(helper.repeat_delay_ms, 450);
    let mut zero = 3_u32.to_be_bytes().to_vec();
    zero.push(1);
    zero.extend(0_u16.to_be_bytes());
    assert!(helper.handle_command(HELPER_CMD_SET_GRAB, &zero).unwrap());
    assert_eq!(helper.repeat_delay_ms, 450, "zero keeps it");
}

#[test]
fn devices_that_cannot_be_opened_or_grabbed_are_left_alone() {
    let rig = Rig::new(vec![keyboard_info("/dev/input/event1"), pointer_info("/dev/input/event2")]);
    rig.os.st().deny_all = true;
    let mut helper = rig.helper();
    let _ = helper.start_up();
    assert!(rig.os.st().open.is_empty());
    assert!(!helper.keyboards.iter().any(|(_, path)| path == "/dev/input/event1"));
}

#[test]
fn the_event_flags_read_as_they_are_set() {
    let forced = HelperKey { seq: 1, pressed: true, keycode: 2, timestamp_ms: 3, flags: HELPER_KEY_FORCED };
    assert!(forced.forced() && !forced.needs_verdict());
    let plain = HelperKey { seq: 1, pressed: true, keycode: 2, timestamp_ms: 3, flags: HELPER_KEY_NEEDS_VERDICT };
    assert!(!plain.forced() && plain.needs_verdict());
}

#[test]
fn a_frame_in_pieces_waits_for_its_end() {
    let frame = pack_frame(HELPER_MSG_READY, &[1, 2, 3]).unwrap();
    let mut reader = FrameReader::new();
    assert!(reader.feed(&frame[..frame.len() - 1]).unwrap().is_empty());
    assert_eq!(reader.feed(&frame[frame.len() - 1..]).unwrap(), [(HELPER_MSG_READY, vec![1, 2, 3])]);
}

#[test]
fn a_device_is_a_keyboard_or_a_pointer_only_by_all_it_can_do() {
    let role = |info: &InputDeviceInfo| classify_input_device(info);
    let mut gamepad = keyboard_info("/dev/input/event1");
    gamepad.key_bits.insert(EVDEV_BTN_GAMEPAD);
    assert_ne!(role(&gamepad), DeviceRole::Keyboard);
    let mut no_y = pointer_info("/dev/input/event2");
    no_y.rel_bits.remove(&EVDEV_REL_Y);
    assert_eq!(role(&no_y), DeviceRole::Pointer, "a touchpad of buttons");
    let mut no_x = pointer_info("/dev/input/event2");
    no_x.rel_bits.remove(&EVDEV_REL_X);
    assert_eq!(role(&no_x), DeviceRole::Pointer);
    let mut few_keys = keyboard_info("/dev/input/event1");
    few_keys.key_bits.remove(&EVDEV_KEY_A);
    assert_ne!(role(&few_keys), DeviceRole::Keyboard);
}

#[test]
fn an_action_settles_the_key_it_answers_and_the_next_one_goes_on() {
    let rig = Rig::new(vec![keyboard_info("/dev/input/event1")]);
    let mut helper = rig.helper();
    helper.start_up().unwrap();
    let keyboard = rig.os.fd_of("/dev/input/event1");
    rig.os.press(keyboard, EVDEV_KEY_A, KEY_VALUE_PRESS);
    rig.os.press(keyboard, EVDEV_KEY_A, KEY_VALUE_RELEASE);
    helper.read_keyboard(keyboard).unwrap();
    let frames = rig.channel.sent();
    let sequence = key_frame(&frames, frames.len() - 1).0;
    assert!(helper.handle_command(HELPER_CMD_EMIT_BATCH, &batch(1, HELPER_EMIT_ACTION, sequence, &[(true, EVDEV_KEY_Q), (false, EVDEV_KEY_Q)])).unwrap());
    let frames = rig.channel.sent();
    assert_eq!(ack_of(&frames), (1, 2));
    assert!(frames.iter().any(|(kind, _)| *kind == HELPER_MSG_KEY), "the release waiting behind it");
    let empty = batch(2, HELPER_EMIT_UNCHECKED, 0, &[]);
    assert!(helper.handle_command(HELPER_CMD_EMIT_BATCH, &empty).unwrap());
    assert_eq!(ack_of(&rig.channel.sent()), (2, 0));
    helper.uinput_fd = None;
    assert!(helper.handle_command(HELPER_CMD_EMIT_BATCH, &batch(3, HELPER_EMIT_UNCHECKED, 0, &[(true, EVDEV_KEY_Q)])).unwrap());
    helper.emit_physical_key(true, EVDEV_KEY_Q).unwrap();
}

#[test]
fn a_forced_key_refuses_injections_until_the_parent_knows_it() {
    let rig = Rig::new(vec![keyboard_info("/dev/input/event1")]);
    let mut helper = rig.helper();
    helper.start_up().unwrap();
    helper.forced_seq = Some(9);
    assert!(helper.refuses(HELPER_EMIT_TEXT, 100));
    assert!(helper.refuses(HELPER_EMIT_ACTION, 100));
    assert!(!helper.refuses(HELPER_EMIT_UNCHECKED, 100));
    assert!(helper.handle_command(HELPER_CMD_SEQ_ACK, &8_u32.to_be_bytes()).unwrap());
    assert_eq!(helper.forced_seq, Some(9), "another key's answer");
    assert!(helper.handle_command(HELPER_CMD_SEQ_ACK, &9_u32.to_be_bytes()).unwrap());
    assert_eq!(helper.forced_seq, None);
    helper.unverdicted_seq = 5;
    assert!(helper.refuses(HELPER_EMIT_ACTION, 5));
    assert!(!helper.refuses(HELPER_EMIT_ACTION, 6));
    assert!(helper.refuses(HELPER_EMIT_TEXT, 4));
    helper.unverdicted_seq = 0;
    assert!(!helper.refuses(HELPER_EMIT_ACTION, 1));
}

#[test]
fn keys_and_clicks_are_read_only_as_far_as_they_mean_something() {
    let rig = Rig::new(vec![keyboard_info("/dev/input/event1"), pointer_info("/dev/input/event2")]);
    let mut helper = rig.helper();
    helper.start_up().unwrap();
    let keyboard = rig.os.fd_of("/dev/input/event1");
    let pointer = rig.os.fd_of("/dev/input/event2");
    rig.channel.sent();
    helper.grabbed_fds.clear();
    rig.os.press(keyboard, EVDEV_KEY_A, KEY_VALUE_PRESS);
    helper.read_keyboard(keyboard).unwrap();
    assert!(rig.channel.sent().is_empty(), "a keyboard not held by the helper");
    helper.grabbed_fds.insert(keyboard);

    helper.passed_keys.extend([EVDEV_KEY_CAPSLOCK, EVDEV_MODIFIER_KEYS[0]]);
    rig.os.st().now += 10.0;
    for code in [EVDEV_KEY_CAPSLOCK, EVDEV_MODIFIER_KEYS[0]] {
        rig.os.press(keyboard, code, KEY_VALUE_REPEAT);
    }
    rig.os.press(keyboard, EVDEV_KEY_A, 9);
    helper.read_keyboard(keyboard).unwrap();
    assert!(rig.channel.sent().is_empty(), "modifiers do not repeat, nor do unknown values count");

    let event = |ev_type: u32, code: u32, value: i32| RawInputEvent { sec: 5, usec: 0, ev_type, code, value };
    rig.os.st().events.entry(pointer).or_default().extend([event(EV_REL, EVDEV_REL_X, 1), event(EV_KEY, EVDEV_KEY_A, KEY_VALUE_PRESS), event(EV_KEY, EVDEV_BTN_TASK + 1, KEY_VALUE_PRESS)]);
    helper.read_pointer(pointer).unwrap();
    assert!(rig.channel.sent().is_empty());
}

#[test]
fn devices_that_fail_in_other_ways_are_passed_over() {
    let mut info = keyboard_info("/dev/input/event1");
    info.path = "/dev/input/event1".to_owned();
    let rig = Rig::new(vec![info]);
    let mut helper = rig.helper();
    helper.start_up().unwrap();
    let keyboard = rig.os.fd_of("/dev/input/event1");
    helper.close_keyboard(keyboard).unwrap();
    assert!(!rig.os.st().open.contains_key(&keyboard));

    // Not held any more: no ungrab needed, and the keys it held are forgotten.
    let rig = Rig::new(vec![keyboard_info("/dev/input/event1")]);
    let mut helper = rig.helper();
    helper.start_up().unwrap();
    let keyboard = rig.os.fd_of("/dev/input/event1");
    helper.grabbed_fds.clear();
    helper.close_keyboard(keyboard).unwrap();

    // A new device while the grab is off stays free.
    let rig = Rig::new(vec![keyboard_info("/dev/input/event1")]);
    let mut helper = rig.helper();
    helper.start_up().unwrap();
    helper.grab_enabled = false;
    rig.os.st().devices.push(keyboard_info("/dev/input/event2"));
    helper.rescan_devices(false).unwrap();
    assert_eq!(rig.os.st().grabbed.len(), 1);
    helper.set_grab_enabled(true).unwrap();
    assert_eq!(rig.os.st().grabbed.len(), 2);
    assert!(helper.set_grab_enabled(true).is_ok());
    helper.set_grab_enabled(false).unwrap();
    assert!(rig.os.st().grabbed.is_empty());
    helper.set_grab_enabled(false).unwrap();
}

#[test]
fn a_pointer_that_goes_away_is_closed_on_the_rescan() {
    let rig = Rig::new(vec![keyboard_info("/dev/input/event1"), pointer_info("/dev/input/event2")]);
    let mut helper = rig.helper();
    helper.start_up().unwrap();
    let pointer = rig.os.fd_of("/dev/input/event2");
    rig.os.st().devices.retain(|device| device.path == "/dev/input/event1");
    helper.rescan_devices(false).unwrap();
    assert!(!rig.os.st().open.contains_key(&pointer));
}

#[test]
fn a_grab_waits_for_keys_to_rise_and_a_failed_grab_closes_the_device() {
    let rig = Rig::new(vec![keyboard_info("/dev/input/event1")]);
    let mut helper = rig.helper();
    helper.start_up().unwrap();
    let keyboard = rig.os.fd_of("/dev/input/event1");
    helper.grabbed_fds.clear();
    rig.os.st().failing_queries.insert(keyboard);
    assert!(!helper.grab_keyboard_safely(keyboard), "a keyboard that does not answer");
    rig.os.st().failing_queries.clear();
    rig.os.st().keys_down.insert(keyboard, HashSet::new());
    assert!(helper.grab_keyboard_safely(keyboard));
    rig.os.st().devices.push(keyboard_info("/dev/input/event2"));
    let already = rig.os.st().next_fd + 1;
    rig.os.st().failing_queries.insert(already);
    helper.rescan_devices(false).unwrap();
    assert!(!rig.os.st().open.contains_key(&already), "a new keyboard that cannot be held is let go");
}

#[test]
fn a_keyboard_denied_at_the_start_says_how_to_allow_it() {
    let rig = Rig::new(vec![keyboard_info("/dev/input/event1")]);
    rig.os.st().deny_all = true;
    let mut helper = rig.helper();
    assert_eq!(helper.start_up().unwrap_err().kind(), io::ErrorKind::PermissionDenied);
}

fn callbacks_client() -> (HelperClient, UnixStream, Arc<Mutex<Vec<String>>>) {
    let (parent, helper_side) = UnixStream::pair().unwrap();
    let said: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let (key, pointer, repeat, caps, reset) = (said.clone(), said.clone(), said.clone(), said.clone(), said.clone());
    let callbacks = HelperCallbacks {
        on_key: Box::new(move |event| {
            lock(&key).push(format!("key {}", event.seq));
            event.seq % 2 == 0
        }),
        on_pointer: Box::new(move |time| lock(&pointer).push(format!("pointer {time}"))),
        on_repeat_reset: Box::new(move |time| lock(&repeat).push(format!("repeat {time}"))),
        on_caps_lock: Box::new(move |on| lock(&caps).push(format!("caps {on}"))),
        on_input_reset: Box::new(move || lock(&reset).push("reset".to_owned())),
    };
    (HelperClient::new(Arc::new(parent), callbacks), helper_side, said)
}

fn key_message(seq: u32, flags: u32) -> Vec<u8> {
    let mut body = seq.to_be_bytes().to_vec();
    body.push(1);
    body.extend(EVDEV_KEY_A.low::<u16>().to_be_bytes());
    body.extend(1_u32.to_be_bytes());
    body.push(flags.low::<u8>());
    body
}

#[test]
fn the_client_hands_each_message_to_its_place() {
    let (client, mut helper_side, said) = callbacks_client();
    let send = |side: &mut UnixStream, kind: u32, body: &[u8]| side.write_all(&pack_frame(kind, body).unwrap()).unwrap();
    send(&mut helper_side, HELPER_MSG_READY, &[0]);
    assert!(!client.start_default().unwrap(), "the lock light is off");
    send(&mut helper_side, HELPER_MSG_KEY, &key_message(1, HELPER_KEY_NEEDS_VERDICT));
    send(&mut helper_side, HELPER_MSG_KEY, &key_message(2, HELPER_KEY_NEEDS_VERDICT));
    send(&mut helper_side, HELPER_MSG_KEY, &key_message(3, HELPER_KEY_FORCED));
    send(&mut helper_side, HELPER_MSG_KEY, &key_message(4, 0));
    send(&mut helper_side, HELPER_MSG_KEY, &[1]);
    send(&mut helper_side, HELPER_MSG_POINTER, &7_u32.to_be_bytes());
    send(&mut helper_side, HELPER_MSG_POINTER, &[1]);
    send(&mut helper_side, HELPER_MSG_REPEAT_RESET, &8_u32.to_be_bytes());
    send(&mut helper_side, HELPER_MSG_REPEAT_RESET, &[1]);
    send(&mut helper_side, HELPER_MSG_CAPS_LOCK, &[1]);
    send(&mut helper_side, HELPER_MSG_CAPS_LOCK, &[]);
    send(&mut helper_side, HELPER_MSG_INPUT_RESET, &[]);
    send(&mut helper_side, 999, &[]);
    send(&mut helper_side, HELPER_MSG_EMIT_ACK, &[0; 8]);
    send(&mut helper_side, HELPER_MSG_KEY_DOWN, &[0; 5]);
    let deadline = Instant::now() + Duration::from_secs(5);
    while lock(&said).len() < 8 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(*lock(&said), ["key 1", "key 2", "key 3", "key 4", "pointer 7", "repeat 8", "caps true", "reset"]);
    assert_eq!(read_frame(&mut helper_side), (HELPER_CMD_VERDICT, {
        let mut body = 1_u32.to_be_bytes().to_vec();
        body.push(HELPER_VERDICT_PASS.low::<u8>());
        body
    }));
    assert_eq!(read_frame(&mut helper_side).0, HELPER_CMD_VERDICT);
    assert_eq!(read_frame(&mut helper_side), (HELPER_CMD_SEQ_ACK, 3_u32.to_be_bytes().to_vec()));
    assert!(client.caps_lock());
    client.close();
    client.close();
}

#[test]
fn a_damaged_message_ends_the_reader_and_requests_fail_at_once() {
    for damage in [pack_frame(HELPER_MSG_EMIT_ACK, &[1]).unwrap(), pack_frame(HELPER_MSG_KEY_DOWN, &[1]).unwrap(), pack_frame(HELPER_MSG_READY, &[]).unwrap(), vec![0xFF; 16]] {
        let (client, mut helper_side, _said) = callbacks_client();
        helper_side.write_all(&pack_frame(HELPER_MSG_READY, &[0]).unwrap()).unwrap();
        assert!(client.start(2.0).is_ok());
        helper_side.write_all(&damage).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !client.closed() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(client.query_key_down(EVDEV_KEY_A).is_err());
        client.close();
    }
    let (client, mut helper_side, _said) = callbacks_client();
    helper_side.write_all(&pack_frame(HELPER_MSG_READY, &[0]).unwrap()).unwrap();
    assert!(client.start(2.0).is_ok());
    let closer = {
        let (mut side, client) = (helper_side, &client);
        move || {
            let _ = read_frame(&mut side);
            client.close();
        }
    };
    thread::scope(|scope| {
        scope.spawn(closer);
        assert!(client.query_key_down(EVDEV_KEY_A).is_err(), "the client closed while it waited");
    });
}

#[test]
fn the_flag_says_what_it_was_told() {
    let flag = Flag::default();
    assert!(!flag.is_set());
    flag.set();
    assert!(flag.is_set());
    let (socket, _other) = UnixStream::pair().unwrap();
    assert!(ByteChannel::fileno(&socket) > 0);
}

#[test]
fn request_numbers_go_round_past_zero() {
    let (client, _helper_side, _said) = callbacks_client();
    *lock(&client.shared.next_req_id) = u32::MAX;
    assert_eq!(client.shared.allocate_reply().0, 1);
    client.close();
}

#[test]
fn a_socket_that_breaks_ends_the_service_quietly() {
    let rig = Rig::new(vec![keyboard_info("/dev/input/event1")]);
    let mut helper = rig.helper();
    helper.start_up().unwrap();
    rig.channel.broken.store(true, Ordering::SeqCst);
    assert!(!helper.read_channel(libc::POLLIN).unwrap());
}

#[test]
fn a_device_that_will_not_open_is_passed_over_unless_it_is_all_there_is() {
    // Gone between the listing and the opening, for writing or for reading.
    let rig = Rig::new(vec![keyboard_info("/dev/input/event1"), keyboard_info("/dev/input/event2"), keyboard_info("/dev/input/event3")]);
    rig.os.st().broken.insert("/dev/input/event1".to_owned());
    rig.os.st().deny_writable.insert("/dev/input/event2".to_owned());
    rig.os.st().broken_read_only.insert("/dev/input/event2".to_owned());
    let mut helper = rig.helper();
    assert!(helper.start_up().is_ok());
    assert_eq!(helper.keyboards.len(), 1);
    // One keyboard denied and another open: no failure.
    let rig = Rig::new(vec![keyboard_info("/dev/input/event1"), keyboard_info("/dev/input/event2")]);
    rig.os.st().deny_writable.insert("/dev/input/event1".to_owned());
    let mut helper = rig.helper();
    let open = Rig::new(vec![keyboard_info("/dev/input/event2")]);
    drop(open);
    assert!(helper.start_up().is_ok());
}

#[test]
fn a_grab_waits_for_the_keys_to_come_up_and_only_so_long() {
    let rig = Rig::new(vec![keyboard_info("/dev/input/event1")]);
    let mut helper = rig.helper();
    helper.start_up().unwrap();
    let fd = rig.os.fd_of("/dev/input/event1");
    // Down, then up after a few looks: the keys are given a moment before the grab.
    rig.os.st().keys_down.insert(fd, HashSet::from([EVDEV_KEY_A]));
    rig.os.st().keys_up_after = 3;
    assert!(helper.grab_keyboard_safely(fd));
    assert!(!rig.os.st().sleeps.is_empty());
    // Down for good: the wait ends at its deadline and the grab goes ahead.
    rig.os.st().keys_down.insert(fd, HashSet::from([EVDEV_KEY_A]));
    rig.os.st().keys_up_after = 0;
    let _ = helper.grab_keyboard_safely(fd);
    // A grab let go for a device the helper has no keys of.
    helper.grabbed_fds.insert(999);
    helper.set_grab_enabled(false).unwrap();
    assert!(helper.grabbed_fds.is_empty());
}

#[test]
fn a_client_asks_for_what_it_needs_and_stops_answering_once_its_socket_is_gone() {
    // An action checked against its sequence is asked for in its own way.
    let (client, mut helper_side, _) = peer_pair();
    let answer = thread::spawn(move || {
        let _ready = helper_side.write_all(&pack_frame(HELPER_MSG_READY, &[0]).unwrap());
        let (_, body) = read_frame(&mut helper_side);
        let request = body[..4].to_vec();
        let mut reply = request;
        reply.push(0);
        let _ = helper_side.write_all(&pack_frame(HELPER_MSG_EMIT_ACK, &reply).unwrap());
        helper_side
    });
    client.start(2.0).unwrap();
    let _ = client.emit_batch(&[(true, EVDEV_KEY_A)], Some(5), true);
    let helper_side = answer.join().unwrap();
    // A message with nobody to take it, and a stop from the client's own thread.
    drop(helper_side);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !client.closed() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
    client.close();
    client.close();
}

#[test]
fn a_thread_that_will_not_end_is_left_behind() {
    let (client, helper_side, _) = peer_pair();
    // A thread of the client's that takes longer than a close waits.
    lock(&client.threads).push(thread::spawn(|| thread::sleep(Duration::from_secs(3))));
    let began = Instant::now();
    client.close();
    assert!(began.elapsed() < Duration::from_secs(3));
    drop(helper_side);
}

#[test]
fn a_close_from_a_thread_of_the_client_does_not_wait_for_itself() {
    let (client, _helper_side, _) = peer_pair();
    let client = Arc::new(client);
    let go = Arc::new(Flag::default());
    let (worker, signal) = (client.clone(), go.clone());
    let handle = thread::spawn(move || {
        assert!(signal.wait(Duration::from_secs(5)));
        worker.close();
    });
    lock(&client.threads).push(handle);
    go.set();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !lock(&client.threads).is_empty() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
    assert!(lock(&client.threads).is_empty());
}

#[test]
fn a_key_answered_after_the_socket_went_ends_the_loop_and_a_read_after_the_close_stops_it() {
    let (client, mut helper_side, _) = peer_pair();
    helper_side.write_all(&pack_frame(HELPER_MSG_READY, &[0]).unwrap()).unwrap();
    client.start(2.0).unwrap();
    // A key whose answer has nowhere to go.
    let mut body = 1_u32.to_be_bytes().to_vec();
    body.push(1);
    body.extend(EVDEV_KEY_A.low::<u16>().to_be_bytes());
    body.extend(1_u32.to_be_bytes());
    body.push(0);
    helper_side.write_all(&pack_frame(HELPER_MSG_KEY, &body).unwrap()).unwrap();
    drop(helper_side);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !client.closed() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
    client.close();
}
