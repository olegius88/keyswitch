//! The HTTP transport of the updater on Linux: the system's `curl`, run for each request.
//!
//! The Linux package is installed by APT, so the updater only looks for a new release and opens its page: one small request
//! to GitHub. Spawning `curl` leaves a TLS stack out of the program. Redirects are followed by curl, limited to https; the
//! address a response ended at is not reported, so `url()` is the address asked for.

use std::io::{self, Read};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::Duration;

use ks_updates::{ResponseOpener, UpdateRequest, UpdateResponse};

const MAX_REDIRECTS: &str = "10";

pub struct CurlOpener;

struct CurlResponse {
    child: Child,
    stdout: ChildStdout,
    /// The byte read to learn that the request did not fail, handed out first.
    first: Option<u8>,
    url: String,
}

/// The arguments of one request.
fn arguments(request: &UpdateRequest, timeout: Duration) -> Vec<String> {
    let mut arguments: Vec<String> = ["--silent", "--show-error", "--location", "--fail", "--proto", "=https", "--proto-redir", "=https", "--max-redirs", MAX_REDIRECTS]
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
        let Some(mut stdout) = child.stdout.take() else {
            let _ = child.kill();
            return Err(io::Error::other("curl gave no output"));
        };
        let mut first = [0_u8; 1];
        match stdout.read(&mut first) {
            Ok(1) => Ok(Box::new(CurlResponse { child, stdout, first: Some(first[0]), url: request.url.clone() })),
            Ok(_) => {
                // An empty body: fine when curl succeeded, else its message says what failed (status 400 and above included).
                let output = child.wait_with_output()?;
                if output.status.success() {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the response is empty"));
                }
                Err(io::Error::other(String::from_utf8_lossy(&output.stderr).trim().to_owned()))
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(error)
            }
        }
    }
}

impl UpdateResponse for CurlResponse {
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

impl Drop for CurlResponse {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_is_https_only_and_bounded() {
        let request = UpdateRequest { url: "https://api.github.com/x".to_owned(), headers: vec![("Accept".to_owned(), "a/b".to_owned())] };
        let arguments = arguments(&request, Duration::from_secs(15));
        let joined = arguments.join(" ");
        assert!(joined.contains("--proto =https --proto-redir =https --max-redirs 10 --max-time 15 --header Accept: a/b -- https://api.github.com/x"), "{joined}");
    }

    #[test]
    fn a_header_with_a_line_break_is_refused() {
        let request = UpdateRequest { url: "https://x".to_owned(), headers: vec![("A".to_owned(), "b\nc".to_owned())] };
        assert!(CurlOpener.open(&request, Duration::from_secs(1)).is_err());
    }
}
