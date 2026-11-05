//! The transport over `curl`, against a script in its place: the test needs no network.
#![cfg(unix)]
#![expect(unsafe_code, reason = "a test program sets PATH for itself once, before any thread reads it")]

use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use ks_updates::http_curl::CurlOpener;
use ks_updates::{ResponseOpener, UpdateRequest};

fn request(url: &str, headers: Vec<(String, String)>) -> UpdateRequest {
    UpdateRequest { url: url.to_owned(), headers }
}

#[test]
fn curl_in_every_answer_it_can_give() {
    let folder = std::env::temp_dir().join(format!("ks-fake-curl-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let script = folder.join("curl");
    std::fs::write(
        &script,
        "#!/bin/sh\nfor last; do :; done\ncase \"$last\" in\n  *ok) printf 'body' ;;\n  *empty) ;;\n  *) echo 'it failed' >&2; exit 22 ;;\nesac\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    // SAFETY: no other thread of this test program runs yet.
    unsafe { std::env::set_var("PATH", &folder) };
    let opener = CurlOpener;
    let timeout = Duration::from_secs(5);

    let refused = opener.open(&request("https://x/ok", vec![("A".to_owned(), "b\nc".to_owned())]), timeout).err().unwrap();
    assert_eq!(refused.kind(), std::io::ErrorKind::InvalidInput);
    let refused = opener.open(&request("https://x/ok", vec![("A\r".to_owned(), "b".to_owned())]), timeout).err().unwrap();
    assert_eq!(refused.kind(), std::io::ErrorKind::InvalidInput);

    let mut response = opener.open(&request("https://x/ok", vec![("A".to_owned(), "b".to_owned())]), timeout).unwrap();
    assert_eq!(response.url(), "https://x/ok");
    assert_eq!(response.read(&mut []).unwrap(), 0);
    let mut text = Vec::new();
    let mut buffer = [0_u8; 16];
    loop {
        let count = response.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        text.extend_from_slice(&buffer[..count]);
    }
    assert_eq!(text, b"body");
    drop(response);

    let empty = opener.open(&request("https://x/empty", Vec::new()), timeout).err().unwrap();
    assert_eq!(empty.kind(), std::io::ErrorKind::UnexpectedEof);
    let failed = opener.open(&request("https://x/fail", Vec::new()), timeout).err().unwrap();
    assert_eq!(failed.to_string(), "it failed");

    // SAFETY: the calls above have ended, and this thread is the only one.
    unsafe { std::env::set_var("PATH", folder.join("nothing")) };
    let missing = opener.open(&request("https://x/ok", Vec::new()), timeout).err().unwrap();
    assert!(missing.to_string().starts_with("curl is not available"));
}
