//! Update checks: network timeouts, release-notes limits, progress.

use std::time::Duration;

/// The first automatic update check runs this long after start.
pub const UPDATE_CHECK_INITIAL_DELAY_SECONDS: u64 = 30;
/// Automatic update checks repeat at this interval.
pub const UPDATE_CHECK_INTERVAL_SECONDS: u64 = 6 * 3600;
pub const UPDATE_CHECK_INITIAL_DELAY: Duration = Duration::from_secs(UPDATE_CHECK_INITIAL_DELAY_SECONDS);
pub const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(UPDATE_CHECK_INTERVAL_SECONDS);
pub const MAX_RELEASE_JSON_BYTES: usize = 1_000_000;
pub const MAX_ASSET_BYTES: u64 = 300 * 1024 * 1024;
pub const DOWNLOAD_CHUNK_BYTES: usize = 128 * 1024;
/// Default network timeout of one update request.
pub const UPDATE_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
pub const MAX_RELEASE_NOTES_CHARACTERS: usize = 4000;
pub const PROGRESS_COMPLETE_PERCENT: u64 = 100;
/// The longest header line of a response `curl` reports, and the most header bytes of all the responses of one request
/// (each redirect adds its own).
pub const MAX_RESPONSE_HEADER_LINE_BYTES: u64 = 16 * 1024;
pub const MAX_RESPONSE_HEADER_BYTES: usize = 256 * 1024;
