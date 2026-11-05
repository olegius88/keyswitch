//! What is testable of the Linux layer without a compositor: bit maps, the evdev byte layout, the Wayland wire protocol
//! against a scripted compositor, libxkbcommon where it is installed, and the sway adapter against a scripted sway.

#![expect(unsafe_code, reason = "calls into the Linux kernel and system libraries")]
use crate::wayland_constants::{EVDEV_KEY_ENTER, EVDEV_KEY_Q, SYN_REPORT};
use super::*;

fn temporary_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("keyswitch-native-{name}-{}", std::process::id()))
}

#[test]
fn capability_bitmaps_are_read_from_bytes_and_from_sysfs_words() {
    assert_eq!(bits_from_bytes(&[0b0000_0101, 0, 0b1000_0000]), HashSet::from([0, 2, 23]));
    // Words are the machine's unsigned long, the most significant first: "1 0" is bit 64.
    let word_bits = (libc::c_ulong::BITS as usize).low::<u32>();
    assert_eq!(bits_from_sysfs("1 0\n"), HashSet::from([word_bits]));
    assert_eq!(bits_from_sysfs("zz 3"), HashSet::from([0, 1]));
    assert!(bits_from_sysfs("  ").is_empty());
}

#[test]
fn input_events_keep_the_kernels_layout() {
    let bytes = input_event_bytes(EV_KEY, 30, 1);
    assert_eq!(bytes.len(), size_of::<libc::input_event>());
    let event = decode_input_event(&bytes);
    assert_eq!((event.sec, event.usec, event.ev_type, event.code, event.value), (0, 0, EV_KEY, 30, 1));
    // The same bytes through a pipe, read the way the helper reads a device.
    let mut ends = [0.to::<c_int>(); 2];
    // SAFETY: `ends` has room for the two descriptors `pipe2` writes.
    assert_eq!(unsafe { libc::pipe2(ends.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) }, 0);
    let mut os = LinuxHelperOs::new();
    os.write_uinput(ends[1], &[(EV_KEY, 30, 1), (EV_SYN, SYN_REPORT, 0), (EV_LED, LED_CAPSL, 1)]).unwrap();
    let events = os.read_events(ends[0]).unwrap();
    assert_eq!(events.iter().map(|event| (event.ev_type, event.code, event.value)).collect::<Vec<_>>(), vec![(EV_KEY, 30, 1), (EV_SYN, 0, 0), (EV_LED, 1, 1)]);
    os.write_led(ends[1], LED_CAPSL, 0).unwrap();
    assert_eq!(os.read_uinput_leds(ends[0]).unwrap(), vec![(LED_CAPSL, 0)]);
    assert!(os.read_events(ends[0]).unwrap().is_empty());
    assert_eq!(os.poll(&[ends[0]], 0), Vec::<(i32, i16)>::new());
    os.write_led(ends[1], LED_CAPSL, 1).unwrap();
    assert_eq!(os.poll(&[ends[0]], 100), vec![(ends[0], libc::POLLIN)]);
    os.close_device(ends[0]).unwrap();
    os.close_device(ends[1]).unwrap();
}

#[test]
fn the_wayland_socket_is_found_the_way_libwayland_finds_it() {
    assert_eq!(wayland_socket_path(Some("wayland-1"), Some("/run/user/1000")), Some(PathBuf::from("/run/user/1000/wayland-1")));
    assert_eq!(wayland_socket_path(None, Some("/run/user/1000")), Some(PathBuf::from("/run/user/1000/wayland-0")));
    assert_eq!(wayland_socket_path(Some("/tmp/own-socket"), None), Some(PathBuf::from("/tmp/own-socket")));
    assert_eq!(wayland_socket_path(Some("wayland-0"), None), None);
    assert_eq!(wl_read_string(&wl_string("wl_seat"), 0), Some(("wl_seat".to_owned(), 12)));
    assert_eq!(wl_message(5, 3, &[1, 2, 3, 4]), [5, 0, 0, 0, 3, 0, 12, 0, 1, 2, 3, 4].to_vec());
}

fn send_with_descriptor(stream: &UnixStream, bytes: &[u8], descriptor: RawFd) {
    let mut io_vector = libc::iovec { iov_base: bytes.as_ptr().cast_mut().cast::<c_void>(), iov_len: bytes.len() };
    // SAFETY: the space is the size the macro computes for one descriptor, and the pointers stay valid for the call.
    unsafe {
        let space = (libc::CMSG_SPACE((size_of::<RawFd>()).low::<u32>())).to::<usize>();
        let mut control = vec![0u8; space];
        let mut header: libc::msghdr = std::mem::zeroed();
        header.msg_iov = &raw mut io_vector;
        header.msg_iovlen = 1;
        header.msg_control = control.as_mut_ptr().cast::<c_void>();
        header.msg_controllen = space.to::<usize>();
        let message = libc::CMSG_FIRSTHDR(&raw const header);
        (*message).cmsg_level = libc::SOL_SOCKET;
        (*message).cmsg_type = libc::SCM_RIGHTS;
        (*message).cmsg_len = libc::CMSG_LEN((size_of::<RawFd>()).low::<u32>()).to::<usize>();
        libc::CMSG_DATA(message).cast::<RawFd>().write_unaligned(descriptor);
        assert!(libc::sendmsg(stream.as_raw_fd(), &raw const header, 0) >= 0);
    }
}

const TEST_KEYMAP: &str = "xkb_keymap {\n xkb_keycodes \"k\" { minimum = 8; maximum = 255; <AD01> = 24; };\n xkb_types \"t\" { include \"complete\" };\n \
    xkb_compat \"c\" { include \"complete\" };\n xkb_symbols \"s\" { name[Group1] = \"Test\"; key <AD01> { [ q, Q ] }; };\n};\n";

fn read_request(stream: &mut UnixStream) -> Option<WlMessage> {
    let mut header = [0u8; 8];
    stream.read_exact(&mut header).ok()?;
    let word = u32::from_ne_bytes([header[4], header[5], header[6], header[7]]);
    let mut payload = vec![0u8; (word >> 16).to::<usize>() - 8];
    stream.read_exact(&mut payload).ok()?;
    Some(WlMessage { object: u32::from_ne_bytes([header[0], header[1], header[2], header[3]]), opcode: (word & 0xFFFF).to::<u16>(), payload })
}

#[test]
fn the_monitor_learns_the_keymap_and_the_repeat_delay_from_a_scripted_compositor() {
    let delays: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = delays.clone();
    let monitor = match WaylandXkbMonitor::new(Some(Box::new(move |delay| lock(&sink).push(delay)))) {
        Ok(monitor) => monitor,
        Err(error) => return eprintln!("libxkbcommon is not available, the test is skipped: {error}"),
    };
    let (client, mut compositor_side) = UnixStream::pair().unwrap();
    let compositor = thread::spawn(move || {
        let mut sent_global = false;
        while let Some(request) = read_request(&mut compositor_side) {
            match (request.object, request.opcode) {
                (WL_DISPLAY, WL_DISPLAY_SYNC) => {
                    if !sent_global {
                        sent_global = true;
                        let mut global = 7u32.to_ne_bytes().to_vec();
                        global.extend(wl_string("wl_seat"));
                        global.extend(7u32.to_ne_bytes());
                        compositor_side.write_all(&wl_message(2, 0, &global)).unwrap();
                    }
                    let done = request.payload[..4].to_vec();
                    let callback = u32::from_ne_bytes([done[0], done[1], done[2], done[3]]);
                    compositor_side.write_all(&wl_message(callback, 0, &0u32.to_ne_bytes())).unwrap();
                }
                (2, WL_REGISTRY_BIND) => {
                    // name, "wl_seat", version, id: the seat announces a keyboard.
                    assert_eq!(wl_u32(&request.payload, 0), Some(7));
                    compositor_side.write_all(&wl_message(4, 0, &WL_SEAT_CAPABILITY_KEYBOARD.to_ne_bytes())).unwrap();
                }
                (4, WL_SEAT_GET_KEYBOARD) => {
                    // SAFETY: a fresh anonymous file owned by this closure's `File` below.
                    let descriptor = unsafe { libc::memfd_create(c"keymap".as_ptr(), 0) };
                    // SAFETY: the descriptor was just created and nothing else owns it.
                    let file = unsafe { File::from_raw_fd(descriptor) };
                    (&file).write_all(TEST_KEYMAP.as_bytes()).unwrap();
                    (&file).write_all(&[0]).unwrap();
                    let mut keymap = WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1.to_ne_bytes().to_vec();
                    keymap.extend((TEST_KEYMAP.len().low::<u32>() + 1).to_ne_bytes());
                    send_with_descriptor(&compositor_side, &wl_message(6, 0, &keymap), descriptor);
                    let mut repeat = 25i32.to_ne_bytes().to_vec();
                    repeat.extend(300i32.to_ne_bytes());
                    compositor_side.write_all(&wl_message(6, 5, &repeat)).unwrap();
                    let mut modifiers = vec![0u8; 16];
                    modifiers.extend(1u32.to_ne_bytes());
                    compositor_side.write_all(&wl_message(6, 4, &modifiers)).unwrap();
                }
                _ => {}
            }
        }
    });
    let mut socket = WlSocket::from_stream(client);
    let mut state = WlClientState::default();
    let registry = socket.allocate();
    assert_eq!(registry, 2);
    socket.send(WL_DISPLAY, WL_DISPLAY_GET_REGISTRY, &registry.to_ne_bytes()).unwrap();
    for _ in 0..WAYLAND_KEYMAP_ROUNDTRIPS {
        roundtrip(&monitor.shared, &mut socket, &mut state).unwrap();
        if monitor.has_keymap() {
            break;
        }
    }
    // The modifiers come after the keymap in the same stream: one more round trip reads them.
    roundtrip(&monitor.shared, &mut socket, &mut state).unwrap();
    assert!(monitor.has_keymap());
    assert_eq!(monitor.layout_names(), vec!["Test".to_owned()]);
    assert_eq!(monitor.repeat_delay_ms(), 300);
    assert_eq!(*lock(&delays), vec![300]);
    assert_eq!(monitor.active_group(), 1);
    assert_eq!(monitor.translate_key(16, 0, 0), "q");
    assert_eq!(monitor.translate_key(16, SHIFT_MASK, 0), "Q");
    assert_eq!(monitor.translate_key(16, LOCK_MASK, 0), "Q");
    assert_eq!(monitor.keysym_name(16, 0), "q");
    assert_eq!(monitor.translate_key(1, 0, 0), "");
    monitor.close();
    drop(socket);
    compositor.join().unwrap();
}

#[test]
fn the_fallback_keymap_comes_from_the_layout_names_the_desktop_gives() {
    let Ok(monitor) = WaylandXkbMonitor::new(None) else { return eprintln!("libxkbcommon is not available, the test is skipped") };
    monitor.ensure_fallback_keymap(&["us".to_owned(), "ru".to_owned()]);
    if !monitor.has_keymap() {
        return eprintln!("the XKB rule files are not installed, the test is skipped");
    }
    assert_eq!(monitor.layout_names().len(), 2);
    assert_eq!(monitor.translate_key(EVDEV_KEY_Q, 0, 0), "q");
    assert_eq!(monitor.translate_key(EVDEV_KEY_Q, 0, 1), "й");
    assert_eq!(monitor.translate_key(EVDEV_KEY_Q, SHIFT_MASK, 1), "Й");
    assert_eq!(monitor.keysym_name(EVDEV_KEY_Q, 1), "Cyrillic_shorti");
    assert_eq!(monitor.keysym_name(EVDEV_KEY_ENTER, 0), "Return");
    // A keymap that already has the layouts is kept.
    let before = monitor.layout_names();
    monitor.ensure_fallback_keymap(&["us".to_owned()]);
    assert_eq!(monitor.layout_names(), before);
    // `sway` names layouts by description; `layout+variant` is split.
    let Ok(other) = WaylandXkbMonitor::new(None) else { return };
    other.ensure_fallback_keymap(&["English (US)".to_owned(), "Russian".to_owned()]);
    assert_eq!(other.layout_names().len(), 2);
    other.close();
    monitor.close();
}

/// A sway that answers `GET_INPUTS`, `GET_TREE`, `RUN_COMMAND` and `SUBSCRIBE`, and pushes an input event when told.
fn scripted_sway(path: &Path, commands: Arc<Mutex<Vec<String>>>, push_event: Arc<Flag>) -> JoinHandle<()> {
    let listener = std::os::unix::net::UnixListener::bind(path).unwrap();
    thread::spawn(move || {
        let mut layout = 0;
        let mut subscription: Option<UnixStream> = None;
        let mut workers = Vec::new();
        for connection in listener.incoming().take(2) {
            let mut connection = connection.unwrap();
            let mut header = [0u8; 14];
            // The first request decides what the connection is: a subscription stays open and waits for an event.
            connection.read_exact(&mut header).unwrap();
            let (length, message_type) = decode_sway_ipc_header(&header).unwrap();
            let mut body = vec![0u8; length.to::<usize>()];
            connection.read_exact(&mut body).unwrap();
            if message_type == SWAY_IPC_SUBSCRIBE {
                connection.write_all(&[encode_sway_ipc(SWAY_IPC_SUBSCRIBE, "{\"success\": true}")].concat()).unwrap();
                subscription = Some(connection);
                continue;
            }
            let commands = commands.clone();
            workers.push(thread::spawn(move || {
                let mut message = (message_type, String::from_utf8(body).unwrap());
                loop {
                    let reply = match message.0 {
                        SWAY_IPC_GET_INPUTS => serde_json::json!([{"type": "keyboard", "xkb_layout_names": ["English (US)", "Russian"], "xkb_active_layout_index": 0}]),
                        SWAY_IPC_GET_TREE => serde_json::json!({"id": 1, "nodes": [{"id": 9, "pid": 4242, "focused": true, "app_id": "foot", "rect": {"x": 10, "y": 20, "width": 300, "height": 200}}]}),
                        _ => {
                            lock(&commands).push(message.1.clone());
                            serde_json::json!([{"success": true}])
                        }
                    };
                    connection.write_all(&encode_sway_ipc(message.0, &reply.to_string())).unwrap();
                    if connection.read_exact(&mut header).is_err() {
                        return;
                    }
                    let (length, message_type) = decode_sway_ipc_header(&header).unwrap();
                    let mut body = vec![0u8; length.to::<usize>()];
                    if connection.read_exact(&mut body).is_err() {
                        return;
                    }
                    message = (message_type, String::from_utf8(body).unwrap());
                }
            }));
        }
        push_event.wait(Duration::from_secs(5));
        layout += 1;
        let _ = layout;
        if let Some(mut subscription) = subscription {
            let mut frame = encode_sway_ipc(SWAY_IPC_EVENT_WINDOW, "{}");
            frame[10..14].copy_from_slice(&SWAY_IPC_EVENT_WINDOW.to_ne_bytes());
            let _ = subscription.write_all(&frame);
            thread::sleep(Duration::from_millis(100));
        }
        drop(workers);
    })
}

#[test]
fn the_sway_adapter_reads_layouts_and_the_focused_window_and_sends_commands() {
    let path = temporary_path("sway");
    let _ = std::fs::remove_file(&path);
    let commands = Arc::new(Mutex::new(Vec::new()));
    let release = Arc::new(Flag::default());
    let server = scripted_sway(&path, commands.clone(), release.clone());
    let adapter = SwayIpcAdapter::new(path.to_str().unwrap());
    assert_eq!(adapter.setup_error(), "");
    assert_eq!(adapter.label(), "sway");
    assert_eq!(adapter.layout_names(), vec!["English (US)".to_owned(), "Russian".to_owned()]);
    assert_eq!(adapter.current_layout_index(), 0);
    let window = adapter.active_window().unwrap();
    assert_eq!((window.window_id, window.application.as_str(), window.pid), (9, "foot", 4242));
    assert_eq!(adapter.window_by_id(9).unwrap().x, 10);
    assert_eq!(adapter.input_anchor(), Some(ScreenAnchor { x: 160, y: 120, window: Some(9) }));
    assert!(adapter.select_layout_index(1));
    assert!(adapter.activate_window(9));
    assert_eq!(*lock(&commands), vec!["input type:keyboard xkb_switch_layout 1".to_owned(), "[con_id=9] focus".to_owned()]);
    assert!(!adapter.isolated_layout() && !adapter.locked());
    release.set();
    adapter.close();
    server.join().unwrap();
    let _ = std::fs::remove_file(&path);

    // A socket nobody listens on is reported, not raised.
    let missing = SwayIpcAdapter::new("/nonexistent/sway.sock");
    assert!(missing.setup_error().contains("sway IPC"));
    assert!(missing.layout_names().is_empty());
    assert!(!missing.select_layout_index(0));
    missing.close();
}

#[test]
fn the_kde_script_file_is_private_and_never_reused() {
    let first = write_script_file("keyswitch-test-", "var a = 1;\n").unwrap();
    let second = write_script_file("keyswitch-test-", "var b = 2;\n").unwrap();
    assert_ne!(first, second);
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&first).unwrap().permissions().mode() & 0o777, 0o600);
    }
    assert_eq!(std::fs::read_to_string(&first).unwrap(), "var a = 1;\n");
    let _ = std::fs::remove_file(first);
    let _ = std::fs::remove_file(second);
}

#[test]
fn the_helper_command_names_this_executable_and_the_descriptor() {
    let command = helper_command(9).unwrap();
    let arguments: Vec<String> = command.get_args().map(|argument| argument.to_string_lossy().into_owned()).collect();
    assert_eq!(arguments, vec!["--wayland-input-helper".to_owned(), "9".to_owned()]);
    assert!(Path::new(command.get_program()).is_absolute());
    assert!(monotonic_ms() > 0 || clock_monotonic() >= 0.0);
}
