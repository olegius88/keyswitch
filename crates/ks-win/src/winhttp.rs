//! The HTTP transport of the updater on Windows: WinHTTP, with TLS and the proxy settings of the system.
//!
//! Redirects are followed by WinHTTP (at most ten, never from https to http); the address the response ended at is
//! handed to the updater, which accepts only the addresses it knows. `launch_windows_installer` starts the
//! downloaded installer and compiles everywhere.

use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

use ks_updates::{UpdateError, WINDOWS_INSTALLER_ARGUMENTS};

/// Run the installer of an update, detached, after the updater has checked its name and digest.
pub fn launch_windows_installer(path: &Path) -> Result<(), UpdateError> {
    ks_updates::launch_windows_installer(path, &|path: &Path, arguments: &[&str]| -> io::Result<()> {
        debug_assert_eq!(arguments, WINDOWS_INSTALLER_ARGUMENTS);
        // The installer must not share the standard handles of a program that is about to be closed.
        Command::new(path).args(arguments).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(|_| ())
    })
}

/// The host, port and path with query of an `https://` URL.
pub fn split_https_url(url: &str) -> Option<(&str, u16, &str)> {
    let rest = url.strip_prefix("https://")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    if authority.is_empty() || authority.contains('@') || authority.starts_with('[') {
        return None;
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, port.parse::<u16>().ok()?),
        None => (authority, 443),
    };
    let tail = tail.split('#').next().unwrap_or("");
    let path = if tail.is_empty() { "/" } else if tail.starts_with('?') { return None } else { tail };
    (!host.is_empty()).then_some((host, port, path))
}

#[cfg(windows)]
pub use native::WinHttpOpener;

#[cfg(windows)]
mod native {
    use ks_cast::To as _;

    use std::ffi::c_void;
    use std::io;
    use std::ptr::{null, null_mut};
    use std::time::Duration;

    use ks_updates::{ResponseOpener, UpdateRequest, UpdateResponse};
    use windows_sys::Win32::Networking::WinHttp::{
        WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_OPTION_URL, WINHTTP_QUERY_FLAG_NUMBER,
        WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders,
        WinHttpQueryOption, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts,
    };

    use super::split_https_url;

    /// The WinHTTP calls of one response, closed in reverse order when it is dropped.
    struct Handles {
        session: *mut c_void,
        connection: *mut c_void,
        request: *mut c_void,
    }

    // SAFETY: WinHTTP handles are not tied to the thread that made them, and `Handles` is used from one thread at a time.
    unsafe impl Send for Handles {}

    impl Drop for Handles {
        fn drop(&mut self) {
            for handle in [self.request, self.connection, self.session] {
                if !handle.is_null() {
                    // SAFETY: the handle came from WinHTTP, is owned by this value, and is closed once.
                    unsafe { WinHttpCloseHandle(handle) };
                }
            }
        }
    }

    struct WinHttpResponse {
        handles: Handles,
        url: String,
    }

    impl UpdateResponse for WinHttpResponse {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let wanted = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
            let mut count: u32 = 0;
            // SAFETY: the buffer is valid for `wanted` bytes, `count` for one u32, the request handle is open.
            let done = unsafe { WinHttpReadData(self.handles.request, buffer.as_mut_ptr().cast(), wanted, &mut count) };
            if done == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(count.to::<usize>())
        }

        fn url(&self) -> String {
            self.url.clone()
        }
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn check(result: i32) -> io::Result<()> {
        if result == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }

    fn handle(raw: *mut c_void) -> io::Result<*mut c_void> {
        if raw.is_null() { Err(io::Error::last_os_error()) } else { Ok(raw) }
    }

    /// The address a request ended at, after the redirects.
    fn final_url(request: *mut c_void) -> io::Result<String> {
        let mut size: u32 = 0;
        // SAFETY: a null buffer with a zero size asks WinHTTP for the size it needs, in bytes.
        unsafe { WinHttpQueryOption(request, WINHTTP_OPTION_URL, null_mut(), &mut size) };
        let mut buffer = vec![0u16; (size as usize).div_ceil(2).max(1)];
        let mut size = (buffer.len() * 2).low::<u32>();
        // SAFETY: the buffer holds `size` bytes.
        check(unsafe { WinHttpQueryOption(request, WINHTTP_OPTION_URL, buffer.as_mut_ptr().cast(), &mut size) })?;
        let characters = (size.to::<usize>() / 2).min(buffer.len());
        let text = &buffer[..characters];
        let end = text.iter().position(|&unit| unit == 0).unwrap_or(text.len());
        Ok(String::from_utf16_lossy(&text[..end]))
    }

    /// HTTPS GET over WinHTTP; a status outside 200-299 is an error, as `urlopen` raises one.
    #[derive(Debug, Default, Clone, Copy)]
    pub struct WinHttpOpener;

    impl ResponseOpener for WinHttpOpener {
        fn open(&self, request: &UpdateRequest, timeout: Duration) -> io::Result<Box<dyn UpdateResponse>> {
            let invalid = || io::Error::new(io::ErrorKind::InvalidInput, format!("unsupported address: {}", request.url));
            let (host, port, path) = split_https_url(&request.url).ok_or_else(invalid)?;
            let milliseconds = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
            let mut handles = Handles { session: null_mut(), connection: null_mut(), request: null_mut() };
            let agent = wide("KeySwitch");
            // SAFETY: every pointer is to a NUL-terminated buffer that outlives the call, or null where WinHTTP allows it.
            unsafe {
                handles.session = handle(WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, null(), null(), 0))?;
                // Resolve, connect, send and receive each wait at most this long, as the socket timeout of `urlopen`.
                check(WinHttpSetTimeouts(handles.session, milliseconds, milliseconds, milliseconds, milliseconds))?;
                let host_wide = wide(host);
                handles.connection = handle(WinHttpConnect(handles.session, host_wide.as_ptr(), port, 0))?;
                let (verb, object) = (wide("GET"), wide(path));
                handles.request = handle(WinHttpOpenRequest(
                    handles.connection,
                    verb.as_ptr(),
                    object.as_ptr(),
                    null(),
                    null(),
                    null(),
                    WINHTTP_FLAG_SECURE,
                ))?;
                let headers: Vec<u16> =
                    request.headers.iter().flat_map(|(name, value)| format!("{name}: {value}\r\n").encode_utf16().collect::<Vec<_>>()).collect();
                let headers_length = u32::try_from(headers.len()).unwrap_or(0);
                check(WinHttpSendRequest(handles.request, headers.as_ptr(), headers_length, null(), 0, 0, 0))?;
                check(WinHttpReceiveResponse(handles.request, null_mut()))?;
                let mut status: u32 = 0;
                let mut size = (size_of::<u32>()).low::<u32>();
                check(WinHttpQueryHeaders(
                    handles.request,
                    WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                    null(),
                    (&mut status as *mut u32).cast(),
                    &mut size,
                    null_mut(),
                ))?;
                if !(200..300).contains(&status) {
                    return Err(io::Error::other(format!("HTTP Error {status}")));
                }
            }
            let url = final_url(handles.request)?;
            Ok(Box::new(WinHttpResponse { handles, url }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_addresses_are_split_for_winhttp() {
        assert_eq!(
            split_https_url("https://api.github.com/repos/olegius88/keyswitch/releases/latest"),
            Some(("api.github.com", 443, "/repos/olegius88/keyswitch/releases/latest"))
        );
        assert_eq!(split_https_url("https://example.test:8443/a?b=1#c"), Some(("example.test", 8443, "/a?b=1")));
        assert_eq!(split_https_url("https://example.test"), Some(("example.test", 443, "/")));
        for refused in ["http://example.test/", "https://", "https://u@example.test/", "https://example.test:x/", "ftp://a/b", "https://example.test?x=1"] {
            assert_eq!(split_https_url(refused), None, "{refused}");
        }
    }
}
