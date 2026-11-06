//! ZIP archives as `zipfile` writes them: stored or deflated entries, nothing more.
use ks_cast::To as _;
use std::io::Write;

use flate2::Compression;
use flate2::write::DeflateEncoder;

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// One archive being written in memory.
#[derive(Default)]
pub struct ZipWriter {
    output: Vec<u8>,
    central: Vec<u8>,
    entries: u16,
}

impl ZipWriter {
    #[must_use]
    pub fn new() -> ZipWriter {
        ZipWriter::default()
    }

    pub fn write(&mut self, name: &str, data: &[u8], deflate: bool) {
        let crc = crc32(data);
        let stored;
        let body: &[u8] = if deflate {
            let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
            let _ = encoder.write_all(data);
            stored = encoder.finish().unwrap_or_default();
            &stored
        } else {
            data
        };
        let method: u16 = if deflate { 8 } else { 0 };
        let flags: u16 = if name.is_ascii() { 0 } else { 0x800 };
        let offset = self.output.len().low::<u32>();
        let common = |out: &mut Vec<u8>| {
            out.extend_from_slice(&20u16.to_le_bytes());
            out.extend_from_slice(&flags.to_le_bytes());
            out.extend_from_slice(&method.to_le_bytes());
            // The DOS time of 1980-01-01 00:00: the bytes carry their own dates in the metadata.
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0x0021u16.to_le_bytes());
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&body.len().low::<u32>().to_le_bytes());
            out.extend_from_slice(&data.len().low::<u32>().to_le_bytes());
            out.extend_from_slice(&name.len().low::<u16>().to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
        };
        self.output.extend_from_slice(b"PK\x03\x04");
        common(&mut self.output);
        self.output.extend_from_slice(name.as_bytes());
        self.output.extend_from_slice(body);
        self.central.extend_from_slice(b"PK\x01\x02");
        self.central.extend_from_slice(&20u16.to_le_bytes());
        common(&mut self.central);
        self.central.extend_from_slice(&[0u8; 10]);
        self.central.extend_from_slice(&offset.to_le_bytes());
        self.central.extend_from_slice(name.as_bytes());
        self.entries += 1;
    }

    #[must_use]
    pub fn finish(mut self) -> Vec<u8> {
        let start = self.output.len().low::<u32>();
        self.output.extend_from_slice(&self.central);
        self.output.extend_from_slice(b"PK\x05\x06");
        self.output.extend_from_slice(&[0u8; 4]);
        self.output.extend_from_slice(&self.entries.to_le_bytes());
        self.output.extend_from_slice(&self.entries.to_le_bytes());
        self.output.extend_from_slice(&self.central.len().low::<u32>().to_le_bytes());
        self.output.extend_from_slice(&start.to_le_bytes());
        self.output.extend_from_slice(&0u16.to_le_bytes());
        self.output
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn a_name_beyond_ascii_says_it_is_utf8() {
        let mut zip = ZipWriter::new();
        zip.write("логи.txt", b"x", false);
        let bytes = zip.finish();
        assert_eq!(u16::from_le_bytes([bytes[6], bytes[7]]) & 0x800, 0x800);
    }

    #[test]
    fn the_checksum_is_the_standard_one() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn an_archive_ends_with_its_directory() {
        let mut zip = ZipWriter::new();
        zip.write("a.txt", b"hello hello hello", true);
        zip.write("b.txt", b"x", false);
        let bytes = zip.finish();
        assert_eq!(&bytes[..4], b"PK\x03\x04");
        assert_eq!(&bytes[bytes.len() - 22..bytes.len() - 18], b"PK\x05\x06");
        assert_eq!(u16::from_le_bytes([bytes[bytes.len() - 12], bytes[bytes.len() - 11]]), 2);
    }
}
