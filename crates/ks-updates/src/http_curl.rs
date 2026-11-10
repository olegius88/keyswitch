//! The HTTP transport of the updater on Linux and macOS: the system's `curl`, run for each request.
//!
//! The Linux package is installed by APT, so there the updater only looks for a new release and opens its page; the
//! macOS build downloads the archive of the application too. Spawning `curl` leaves a TLS stack out of the program.
//! Redirects are followed by curl, limited to https, and curl writes the headers of every response it receives ahead of
//! the body (`--dump-header -`): each redirect moves the address to its `Location`, so `url()` is the address the body
//! comes from, known before a byte of it is read, and the updater checks that address as it does on Windows.

use ks_cast::To as _;
use std::io::{self, BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use crate::constants::{MAX_RESPONSE_HEADER_BYTES, MAX_RESPONSE_HEADER_LINE_BYTES};
use crate::{ResponseOpener, UpdateRequest, UpdateResponse};

const MAX_REDIRECTS: &str = "10";

pub struct CurlOpener;

struct CurlResponse<R> {
    child: Child,
    stdout: BufReader<R>,
    /// The byte read to learn that the body is not empty, handed out first.
    first: Option<u8>,
    url: String,
}

/// The arguments of one request.
fn arguments(request: &UpdateRequest, timeout: Duration) -> Vec<String> {
    let mut arguments: Vec<String> = [
        "--silent",
        "--show-error",
        "--location",
        "--fail",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--max-redirs",
        MAX_REDIRECTS,
        "--dump-header",
        "-",
        // The answer of a proxy to CONNECT is not a response of the server: it would read as the final one.
        "--suppress-connect-headers",
    ]
    .iter()
    .map(|argument| (*argument).to_owned())
    .collect();
    arguments.push("--max-time".to_owned());
    arguments.push(timeout.as_secs().max(1).to_string());
    for (name, value) in &request.headers {
        arguments.push("--header".to_owned());
        arguments.push(format!("{name}: {value}"));
    }
    arguments.push("--".to_owned());
    arguments.push(request.url.clone());
    arguments
}

impl ResponseOpener for CurlOpener {
    fn open(&self, request: &UpdateRequest, timeout: Duration) -> io::Result<Box<dyn UpdateResponse>> {
        if request.headers.iter().any(|(name, value)| name.contains(['\r', '\n']) || value.contains(['\r', '\n'])) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "a header carries a line break"));
        }
        let mut child = Command::new("curl")
            .args(arguments(request, timeout))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| io::Error::new(error.kind(), format!("curl is not available: {error}")))?;
        let stdout = child.stdout.take().expect("stdout is piped above");
        first_answer(child, stdout, &request.url)
    }
}

/// What the headers say of the body that follows them.
#[derive(Debug, PartialEq, Eq)]
enum Head {
    /// The body comes from this address.
    Body(String),
    /// The server answered with this error status.
    Failed(u16),
    /// The output ended before a final response.
    Ended,
}

/// The status of a response's first line: `HTTP/2 302`, `HTTP/1.1 200 OK`.
fn status_code(line: &str) -> Option<u16> {
    let mut words = line.split_whitespace();
    if !words.next()?.starts_with("HTTP/") {
        return None;
    }
    words.next()?.parse().ok()
}

/// The value of a `Location` header line, in any case.
fn location(line: &str) -> Option<&str> {
    let (name, value) = line.split_once(':')?;
    name.trim().eq_ignore_ascii_case("location").then(|| value.trim())
}

fn malformed(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("curl: {message}"))
}

/// Read the header blocks: an informational response is passed over, a redirect moves the address to its `Location`
/// (as it is written: an address that is not absolute fails the caller's check of the address, as it should), and the
/// first other response ends them.
fn read_head<R: Read>(stdout: &mut BufReader<R>, url: &str) -> io::Result<Head> {
    let mut url = url.to_owned();
    let mut total = 0_usize;
    loop {
        let mut status: Option<u16> = None;
        let mut next: Option<String> = None;
        loop {
            let mut line = Vec::new();
            let count = stdout.by_ref().take(MAX_RESPONSE_HEADER_LINE_BYTES).read_until(b'\n', &mut line)?;
            total += count;
            if total > MAX_RESPONSE_HEADER_BYTES {
                return Err(malformed("the headers are too long"));
            }
            if !line.ends_with(b"\n") {
                if line.len().to::<u64>() < MAX_RESPONSE_HEADER_LINE_BYTES {
                    return Ok(Head::Ended);
                }
                return Err(malformed("a header line is too long"));
            }
            let text = String::from_utf8_lossy(&line);
            let text = text.trim_end_matches(['\r', '\n']);
            if text.is_empty() {
                break;
            }
            match status {
                None => status = Some(status_code(text).ok_or_else(|| malformed("a response without a status line"))?),
                Some(_) => {
                    if let Some(value) = location(text) {
                        next = Some(value.to_owned());
                    }
                }
            }
        }
        let code = status.ok_or_else(|| malformed("a response without a status line"))?;
        match (code, next) {
            (100..=199, _) => {}
            (300..=399, Some(next)) => url = next,
            (400.., _) => return Ok(Head::Failed(code)),
            _ => return Ok(Head::Body(url)),
        }
    }
}

/// The error of a curl that ended without a body: its own message, or the status the server gave.
fn ended(child: Child, status: Option<u16>) -> io::Result<io::Error> {
    let output = child.wait_with_output()?;
    let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if !message.is_empty() {
        return Ok(io::Error::other(message));
    }
    Ok(match status {
        Some(code) => io::Error::other(format!("HTTP {code}")),
        None if output.status.success() => io::Error::new(io::ErrorKind::UnexpectedEof, "the response is empty"),
        None => io::Error::other("curl failed"),
    })
}

fn killed(mut child: Child, error: io::Error) -> io::Error {
    let _ = child.kill();
    let _ = child.wait();
    error
}

/// What curl answered: the headers name the address of the body, its first byte shows that the transfer started, and
/// an answer without a body says why there is none.
fn first_answer<R: Read + Send + 'static>(child: Child, stdout: R, url: &str) -> io::Result<Box<dyn UpdateResponse>> {
    let mut stdout = BufReader::new(stdout);
    let url = match read_head(&mut stdout, url) {
        Ok(Head::Body(url)) => url,
        Ok(Head::Failed(code)) => return Err(ended(child, Some(code))?),
        Ok(Head::Ended) => return Err(ended(child, None)?),
        Err(error) => return Err(killed(child, error)),
    };
    let mut first = [0_u8; 1];
    match stdout.read(&mut first) {
        Ok(1) => Ok(Box::new(CurlResponse { child, stdout, first: Some(first[0]), url })),
        Ok(_) => Err(ended(child, None)?),
        Err(error) => Err(killed(child, error)),
    }
}

impl<R: Read + Send> UpdateResponse for CurlResponse<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if let Some(byte) = self.first.take() {
            buffer[0] = byte;
            return Ok(1);
        }
        self.stdout.read(buffer)
    }

    fn url(&self) -> String {
        self.url.clone()
    }
}

impl<R> Drop for CurlResponse<R> {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn the_request_is_https_only_bounded_and_shows_its_headers() {
        let request = UpdateRequest { url: "https://api.github.com/x".to_owned(), headers: vec![("Accept".to_owned(), "a/b".to_owned())] };
        let arguments = arguments(&request, Duration::from_secs(15));
        let joined = arguments.join(" ");
        assert!(
            joined.contains("--proto =https --proto-redir =https --max-redirs 10 --dump-header - --suppress-connect-headers --max-time 15 --header Accept: a/b -- https://api.github.com/x"),
            "{joined}"
        );
    }

    /// A reader that always fails, on a program that ends by itself.
    #[test]
    fn a_failed_read_ends_curl_and_is_reported() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("the pipe broke"))
            }
        }
        let child = Command::new(std::env::current_exe().expect("this test")).arg("--list").stdout(Stdio::null()).spawn().expect("started");
        let failure = first_answer(child, Broken, "https://x").err().expect("it failed");
        assert_eq!(failure.to_string(), "the pipe broke");
    }

    /// The headers end, and reading the body fails.
    #[test]
    fn a_body_that_cannot_be_read_ends_curl_and_is_reported() {
        struct HeadOnly(Vec<u8>);
        impl Read for HeadOnly {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                if self.0.is_empty() {
                    return Err(io::Error::other("the body broke"));
                }
                let count = buffer.len().min(self.0.len());
                buffer[..count].copy_from_slice(&self.0[..count]);
                self.0.drain(..count);
                Ok(count)
            }
        }
        let child = Command::new(std::env::current_exe().expect("this test")).arg("--list").stdout(Stdio::null()).spawn().expect("started");
        let failure = first_answer(child, HeadOnly(b"HTTP/2 200 \r\n\r\n".to_vec()), "https://x").err().expect("it failed");
        assert_eq!(failure.to_string(), "the body broke");
    }

    #[test]
    fn a_header_with_a_line_break_is_refused() {
        let request = UpdateRequest { url: "https://x".to_owned(), headers: vec![("A".to_owned(), "b\nc".to_owned())] };
        assert!(CurlOpener.open(&request, Duration::from_secs(1)).is_err());
    }

    #[test]
    fn status_lines_and_locations_are_read_as_curl_writes_them() {
        assert_eq!(status_code("HTTP/2 302 "), Some(302));
        assert_eq!(status_code("HTTP/1.1 200 OK"), Some(200));
        assert_eq!(status_code("HTTP/1.1"), None);
        assert_eq!(status_code("hello world"), None);
        assert_eq!(status_code(""), None);
        assert_eq!(status_code("HTTP/2 abc"), None);
        assert_eq!(location("Location: https://a/b"), Some("https://a/b"));
        assert_eq!(location("location:https://a/b "), Some("https://a/b"));
        assert_eq!(location("content-location: x"), None);
        assert_eq!(location("no colon"), None);
    }

    #[test]
    fn the_head_names_the_address_of_the_body() {
        let head = |text: &str| read_head(&mut BufReader::new(text.as_bytes()), "https://start");
        assert_eq!(head("HTTP/2 200 \r\n\r\nbody").unwrap(), Head::Body("https://start".to_owned()));
        assert_eq!(
            head("HTTP/2 302 \r\nlocation: https://a\r\n\r\nHTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 301 Moved\r\nLocation: https://b\r\n\r\nHTTP/2 200 \r\n\r\n").unwrap(),
            Head::Body("https://b".to_owned())
        );
        // A redirect without an address is the final response.
        assert_eq!(head("HTTP/2 304 \r\n\r\n").unwrap(), Head::Body("https://start".to_owned()));
        assert_eq!(head("HTTP/2 404 \r\n\r\n").unwrap(), Head::Failed(404));
        assert_eq!(head("").unwrap(), Head::Ended);
        assert_eq!(head("HTTP/2 200 \r\nx: y").unwrap(), Head::Ended);
        assert_eq!(head("\r\n").unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert_eq!(head("hello\r\n\r\n").unwrap_err().kind(), io::ErrorKind::InvalidData);
        let long_line = format!("HTTP/2 200 \r\nx: {}\r\n\r\n", "a".repeat(MAX_RESPONSE_HEADER_LINE_BYTES.to::<usize>()));
        assert!(head(&long_line).unwrap_err().to_string().contains("line is too long"));
        let many = format!("HTTP/2 200 \r\n{}\r\n", "x: y\r\n".repeat(MAX_RESPONSE_HEADER_BYTES / 6 + 1));
        assert!(head(&many).unwrap_err().to_string().contains("headers are too long"));
    }
}
