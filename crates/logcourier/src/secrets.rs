//! The bot's token: its shape, its removal from any text, and the system's secret store. There is no plaintext fallback.
#![cfg_attr(windows, expect(unsafe_code, reason = "keeps the token in the Windows credential store"))]
use crate::error::{Error, Result};

pub const SERVICE: &str = "LogCourier.Telegram";

/// Where a token starting at `start` ends, as `[1-9][0-9]{4,19}:[A-Za-z0-9_-]{20,200}` matches greedily.
fn token_end(bytes: &[u8], start: usize) -> Option<usize> {
    if !matches!(bytes.get(start), Some(b'1'..=b'9')) {
        return None;
    }
    let digits = bytes[start..].iter().take_while(|byte| byte.is_ascii_digit()).count();
    if !(5..=20).contains(&digits) || bytes.get(start + digits) != Some(&b':') {
        return None;
    }
    let tail = start + digits + 1;
    let length = bytes[tail..].iter().take(200).take_while(|byte| byte.is_ascii_alphanumeric() || **byte == b'_' || **byte == b'-').count();
    (length >= 20).then_some(tail + length)
}

pub fn token_bot_id(token: &str) -> Result<String> {
    if token_end(token.as_bytes(), 0) != Some(token.len()) {
        return Err(Error::value("Некорректный формат токена Telegram."));
    }
    Ok(token.split(':').next().unwrap_or_default().to_owned())
}

#[must_use]
pub fn redact(text: &str) -> String {
    let bytes = text.as_bytes();
    let (mut out, mut copied, mut index) = (String::with_capacity(text.len()), 0, 0);
    while index < bytes.len() {
        match token_end(bytes, index) {
            Some(end) => {
                out.push_str(text.get(copied..index).unwrap_or_default());
                out.push_str("<TOKEN>");
                copied = end;
                index = end;
            }
            None => index += 1,
        }
    }
    out.push_str(text.get(copied..).unwrap_or_default());
    out
}

const UNAVAILABLE: &str = "Системное хранилище секретов недоступно. Введите токен на эту сессию.";
const NOT_STORED: &str = "Токен не сохранён: системное хранилище секретов недоступно. Можно снять флажок сохранения и использовать токен только до выхода.";

#[cfg(windows)]
mod platform {
    use ks_cast::To as _;
    use std::ptr::null_mut;

    use windows_sys::Win32::Foundation::{ERROR_NOT_FOUND, GetLastError};
    use windows_sys::Win32::Security::Credentials::{CRED_PERSIST_ENTERPRISE, CRED_TYPE_GENERIC, CREDENTIALW, CredFree, CredReadW, CredWriteW};

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }

    struct Credential {
        user: String,
        value: String,
    }

    fn read_one(target: &str) -> Result<Option<Credential>, ()> {
        let name = wide(target);
        let mut found: *mut CREDENTIALW = null_mut();
        // SAFETY: the name is NUL-terminated and `found` receives a block CredFree releases below.
        if unsafe { CredReadW(name.as_ptr(), CRED_TYPE_GENERIC, 0, &raw mut found) } == 0 {
            // SAFETY: reads this thread's last error.
            return if unsafe { GetLastError() } == ERROR_NOT_FOUND { Ok(None) } else { Err(()) };
        }
        // SAFETY: on success `found` points to a credential whose user name and blob stay valid until CredFree.
        let credential = unsafe {
            let credential = &*found;
            let user = if credential.UserName.is_null() {
                String::new()
            } else {
                let mut length = 0;
                while *credential.UserName.add(length) != 0 {
                    length += 1;
                }
                String::from_utf16_lossy(std::slice::from_raw_parts(credential.UserName, length))
            };
            let blob = std::slice::from_raw_parts(credential.CredentialBlob, credential.CredentialBlobSize.to::<usize>());
            let units: Vec<u16> = blob.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect();
            let value = String::from_utf16_lossy(&units);
            CredFree(found.cast());
            Credential { user, value }
        };
        Ok(Some(credential))
    }

    fn write_one(target: &str, user: &str, value: &str) -> Result<(), ()> {
        let (mut name, mut owner, mut comment) = (wide(target), wide(user), wide("Stored using python-keyring"));
        let mut blob: Vec<u8> = value.encode_utf16().flat_map(u16::to_le_bytes).collect();
        // SAFETY: every field is zero or points into a buffer that outlives the call.
        let result = unsafe {
            let mut credential: CREDENTIALW = std::mem::zeroed();
            credential.Type = CRED_TYPE_GENERIC;
            credential.TargetName = name.as_mut_ptr();
            credential.UserName = owner.as_mut_ptr();
            credential.Comment = comment.as_mut_ptr();
            credential.CredentialBlob = blob.as_mut_ptr();
            credential.CredentialBlobSize = blob.len().low::<u32>();
            credential.Persist = CRED_PERSIST_ENTERPRISE;
            CredWriteW(&raw const credential, 0)
        };
        if result == 0 { Err(()) } else { Ok(()) }
    }

    /// The token stored by the Python `keyring` library or by this program: under the service for its last
    /// user, under `user@service` for the earlier ones.
    pub(super) fn read(service: &str, user: &str) -> Result<Option<String>, ()> {
        match read_one(service)? {
            Some(found) if found.user == user => Ok(Some(found.value)),
            _ => Ok(read_one(&format!("{user}@{service}"))?.map(|found| found.value)),
        }
    }

    pub(super) fn write(service: &str, user: &str, value: &str) -> Result<(), ()> {
        if let Some(existing) = read_one(service)? {
            write_one(&format!("{}@{service}", existing.user), &existing.user, &existing.value)?;
        }
        write_one(service, user, value)
    }
}

#[cfg(not(windows))]
mod platform {
    use std::io::Write;
    use std::process::{Command, Stdio};

    /// The Secret Service, through `secret-tool` of libsecret, with the attributes the Python `keyring` library uses.
    pub(super) fn read(service: &str, user: &str) -> Result<Option<String>, ()> {
        let output = Command::new("secret-tool").args(["lookup", "service", service, "username", user]).stderr(Stdio::null()).output().map_err(|_| ())?;
        let value = String::from_utf8_lossy(&output.stdout).trim_end_matches('\n').to_owned();
        Ok((output.status.success() && !value.is_empty()).then_some(value))
    }

    pub(super) fn write(service: &str, user: &str, value: &str) -> Result<(), ()> {
        let label = format!("Password for '{user}' on '{service}'");
        let mut child = Command::new("secret-tool")
            .args(["store", "--label", &label, "application", "Python keyring library", "service", service, "username", user])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| ())?;
        child.stdin.take().ok_or(())?.write_all(value.as_bytes()).map_err(|_| ())?;
        if child.wait().map_err(|_| ())?.success() { Ok(()) } else { Err(()) }
    }
}

pub fn read_token(bot_id: &str) -> Result<String> {
    if bot_id.is_empty() {
        return Ok(String::new());
    }
    platform::read(SERVICE, bot_id).map(Option::unwrap_or_default).map_err(|()| Error::Runtime(UNAVAILABLE.to_owned()))
}

pub fn store_token(token: &str) -> Result<()> {
    let bot_id = token_bot_id(token)?;
    platform::write(SERVICE, &bot_id, token).map_err(|()| Error::Runtime(NOT_STORED.to_owned()))
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    const TOKEN: &str = "123456789:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    /// The real credential store of Windows, under a name of this test, removed after.
    #[cfg(windows)]
    #[test]
    fn the_credential_store_keeps_a_token_for_each_bot() {
        let service = format!("LogCourier.Test.{}", std::process::id());
        let forget = |target: &str| {
            let _ = std::process::Command::new("cmdkey").arg(format!("/delete:{target}")).output();
        };
        assert_eq!(platform::read(&service, "one"), Ok(None));
        platform::write(&service, "one", "first").expect("stored");
        assert_eq!(platform::read(&service, "one"), Ok(Some("first".to_owned())));
        platform::write(&service, "two", "second").expect("stored");
        assert_eq!(platform::read(&service, "two"), Ok(Some("second".to_owned())));
        assert_eq!(platform::read(&service, "one"), Ok(Some("first".to_owned())), "the earlier bot moved aside");
        assert_eq!(platform::read(&service, "three"), Ok(None));
        forget(&service);
        forget(&format!("one@{service}"));
    }

    /// A program of its own first on the path stands for the secret store; the environment is set by starting this test again.
    #[cfg(unix)]
    #[test]
    fn the_secret_store_is_asked_through_its_tool() {
        use std::os::unix::fs::PermissionsExt as _;
        const CHILD: &str = "KS_TEST_SECRET_TOOL";
        match std::env::var(CHILD).as_deref() {
            Ok("answers") => {
                assert_eq!(read_token("123456789").as_deref(), Ok("the token"));
                assert!(store_token(TOKEN).is_ok());
                assert!(store_token("no token").is_err());
                assert_eq!(read_token("").as_deref(), Ok(""));
                return;
            }
            Ok("fails") => {
                assert_eq!(read_token("123456789").as_deref(), Ok(""));
                assert!(matches!(store_token(TOKEN), Err(Error::Runtime(_))));
                return;
            }
            Ok(_) => {
                assert!(matches!(read_token("123456789"), Err(Error::Runtime(_))));
                assert!(matches!(store_token(TOKEN), Err(Error::Runtime(_))));
                return;
            }
            Err(_) => {}
        }
        let folder = std::env::temp_dir().join(format!("lc-secrets-{}", std::process::id()));
        let tool = folder.join("secret-tool");
        std::fs::create_dir_all(&folder).expect("the folder");
        let run = |kind: &str, path: &std::ffi::OsStr| {
            let status = std::process::Command::new(std::env::current_exe().expect("this test"))
                .args(["--exact", "secrets::tests::the_secret_store_is_asked_through_its_tool"])
                .env(CHILD, kind)
                .env("PATH", path)
                .status()
                .expect("started");
            assert!(status.success(), "{kind}");
        };
        std::fs::write(&tool, "#!/bin/sh\ncat > /dev/null\necho 'the token'\n").expect("the file");
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).expect("the mode");
        let path = std::env::join_paths(std::iter::once(folder.clone()).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()))).expect("a path");
        run("answers", &path);
        std::fs::remove_file(&tool).expect("removed");
        std::fs::write(&tool, "#!/bin/sh\ncat > /dev/null\nexit 1\n").expect("the file");
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).expect("the mode");
        run("fails", &path);
        run("missing", std::ffi::OsStr::new("/nonexistent"));
        std::fs::remove_dir_all(&folder).expect("cleaned");
    }

    #[test]
    fn a_token_is_recognised_and_cut_out() {
        assert_eq!(token_bot_id(TOKEN).as_deref(), Ok("123456789"));
        assert!(token_bot_id("12:short").is_err());
        assert!(token_bot_id(&format!("{TOKEN} ")).is_err());
        assert_eq!(redact(&format!("ошибка https://api/bot{TOKEN}/x конец")), "ошибка https://api/bot<TOKEN>/x конец");
        assert_eq!(redact("нет 12345 токена"), "нет 12345 токена");
        assert_eq!(redact("a 123456789:AAAAAAAAAA_AAAAAAAAAA-AAAAAAAAAAAAAAA b"), "a <TOKEN> b");
    }
}
