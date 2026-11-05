//! The Python's `test_wayland_helper.py` on a fake kernel and a fake socket.

use ks_cast::To as _;
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
        Ok(vec![(LED_CAPSL, 1)])
    }
    fn list_event_devices(&mut self) -> Vec<InputDeviceInfo> {
        self.st().devices.clone()
    }
    fn open_device(&mut self, path: &str, writable: bool) -> io::Result<i32> {
        let mut state = self.st();
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
        Ok(self.st().keys_down.get(&fd).cloned().unwrap_or_default())
    }
    fn query_caps_lock(&mut self, _fd: i32) -> bool {
        false
    }
    fn read_events(&mut self, fd: i32) -> io::Result<Vec<RawInputEvent>> {
        Ok(self.st().events.entry(fd).or_default().drain(..).collect())
    }
    fn write_led(&mut self, _fd: i32, _code: u32, _value: i32) -> io::Result<()> {
        Ok(())
    }
    fn open_inotify(&mut self) -> io::Result<i32> {
        Ok(4)
    }
    fn read_inotify(&mut self, _fd: i32) -> Vec<String> {
        Vec::new()
    }
    fn poll(&mut self, _fds: &[i32], _timeout_ms: i32) -> Vec<(i32, i16)> {
        Vec::new()
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
}

impl FakeChannel {
    fn new() -> FakeChannel {
        FakeChannel { inbound: Mutex::new(VecDeque::new()), outbound: Mutex::new(Vec::new()), fd: AtomicI32::new(2) }
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
