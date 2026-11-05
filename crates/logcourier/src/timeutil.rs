//! UTC time as Python's `datetime` writes it, and the ids of `uuid4().hex`.
use ks_cast::To as _;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

#[must_use]
pub fn now_seconds() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |elapsed| elapsed.as_secs_f64())
}

/// Seconds on a clock that only goes forward, from the first call.
pub fn monotonic() -> f64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

/// (year, month, day) of a count of days since 1970-01-01.
#[must_use]
pub fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

struct Moment {
    date: (i64, i64, i64),
    time: (i64, i64, i64),
    micros: i64,
}

fn moment(seconds: f64) -> Moment {
    let whole = seconds.floor().to::<i64>();
    let day = whole.div_euclid(86400);
    let second = whole.rem_euclid(86400);
    Moment {
        date: civil_from_days(day),
        time: (second / 3600, second / 60 % 60, second % 60),
        micros: ((seconds - seconds.floor()) * 1e6).to::<i64>(),
    }
}

/// `datetime.now(timezone.utc).isoformat()`.
#[must_use]
pub fn iso_now() -> String {
    iso_at(now_seconds())
}

#[must_use]
pub fn iso_at(seconds: f64) -> String {
    let at = moment(seconds);
    let (y, m, d) = at.date;
    let (hh, mm, ss) = at.time;
    if at.micros == 0 {
        format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}+00:00")
    } else {
        format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}.{:06}+00:00", at.micros)
    }
}

/// The time part of an archive's name: `%Y%m%dT%H%M%SZ`.
#[must_use]
pub fn stamp_at(seconds: f64) -> String {
    let at = moment(seconds);
    let (y, m, d) = at.date;
    let (hh, mm, ss) = at.time;
    format!("{y:04}{m:02}{d:02}T{hh:02}{mm:02}{ss:02}Z")
}

fn digits(text: &[u8]) -> bool {
    !text.is_empty() && text.iter().all(u8::is_ascii_digit)
}

/// The date of an ISO text as `datetime.fromisoformat` would read it, or None when it would refuse the text.
#[must_use]
pub fn iso_date(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    if bytes.len() < 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    if !digits(&bytes[..4]) || !digits(&bytes[5..7]) || !digits(&bytes[8..10]) {
        return None;
    }
    let number = |range: std::ops::Range<usize>| text.get(range).unwrap_or_default().parse::<i64>().unwrap_or(0);
    let (year, month, day) = (number(0..4), number(5..7), number(8..10));
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let last = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    if year == 0 || !(1..=last).contains(&day) {
        return None;
    }
    if bytes.len() == 10 {
        return Some(text.get(..10).unwrap_or_default());
    }
    if !bytes[10].is_ascii() {
        return None;
    }
    let rest = &bytes[11..];
    if rest.len() < 2 || !digits(&rest[..2]) || rest.iter().any(|byte| !(byte.is_ascii_digit() || b":.,+-Z".contains(byte))) {
        return None;
    }
    Some(text.get(..10).unwrap_or_default())
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Thirty-two hexadecimal digits that no other call returns.
pub fn new_id() -> String {
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    let mut digest = Sha256::new();
    digest.update(hasher.finish().to_le_bytes());
    digest.update(now_seconds().to_le_bytes());
    digest.update(std::process::id().to_le_bytes());
    digest.finalize()[..16].iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The lower-case hexadecimal SHA-256 of some bytes.
#[must_use]
pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn times_read_as_in_python() {
        assert_eq!(iso_at(0.0), "1970-01-01T00:00:00+00:00");
        assert_eq!(iso_at(1_793_835_296.5), "2026-11-04T23:34:56.500000+00:00");
        assert_eq!(stamp_at(1_793_835_296.0), "20261104T233456Z");
        assert_eq!(iso_date("2026-09-05"), Some("2026-09-05"));
        assert_eq!(iso_date("2026-11-04T20:14:56.5+00:00"), Some("2026-11-04"));
        assert_eq!(iso_date("2026-13-05"), None);
        assert_eq!(iso_date("nope"), None);
    }

    #[test]
    fn ids_are_hexadecimal_and_differ() {
        let (a, b) = (new_id(), new_id());
        assert_ne!(a, b);
        assert!(a.len() == 32 && a.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()));
    }
}
