//! Reads the new bytes of the log files into the queue, with the read position in the same transaction.
use ks_cast::To as _;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::json;
use sqlite::{OptionalExtension, params};

use crate::config::{Config, Source};
use crate::constants::*;
use crate::error::{Error, Result, not_found};
use crate::store::Store;
use crate::timeutil::{iso_at, new_id, now_seconds, sha256_hex, stamp_at};
use crate::versions::{latest_version, observe_version, take_fragment};
use crate::zipfile::ZipWriter;

pub fn fingerprint(stream: &mut File, offset: u64) -> Result<String> {
    stream.seek(SeekFrom::Start(offset.saturating_sub(FINGERPRINT_WINDOW_BYTES)))?;
    let mut data = Vec::new();
    stream.by_ref().take(offset.min(FINGERPRINT_WINDOW_BYTES)).read_to_end(&mut data)?;
    Ok(sha256_hex(&data))
}

pub fn open_regular(path: &Path) -> Result<File> {
    if std::fs::symlink_metadata(path).is_ok_and(|info| info.file_type().is_symlink()) {
        return Err(Error::value("Символьные ссылки не собираются."));
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(Error::value("Источник должен быть обычным файлом."));
    }
    Ok(file)
}

/// The file's identity on this computer, stable across renames: device and inode.
fn identity_of(file: &File) -> Result<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let info = file.metadata()?;
        Ok(format!("{}:{}", info.dev(), info.ino()))
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle};
        // SAFETY: the handle is the open file's and the structure is written by the call.
        let info = unsafe {
            let mut info: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
            if GetFileInformationByHandle(file.as_raw_handle(), &mut info) == 0 {
                return Err(Error::Os("OSError"));
            }
            info
        };
        let index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
        Ok(format!("{}:{index}", info.dwVolumeSerialNumber))
    }
}

pub fn make_archive(data: &[u8], metadata: &serde_json::Value) -> Vec<u8> {
    let options = ks_text::JsonOptions { ensure_ascii: false, indent: Some(ks_text::Indent::Spaces(JSON_INDENT_SPACES)), ..ks_text::JsonOptions::default() };
    let mut archive = ZipWriter::new();
    archive.write("metadata.json", ks_text::json_dumps(metadata, &options).as_bytes(), true);
    archive.write("fragment.log", data, true);
    archive.finish()
}

pub struct Collector<'a> {
    pub store: &'a Store,
    pub chunk_size: usize,
}

impl<'a> Collector<'a> {
    pub fn new(store: &'a Store) -> Collector<'a> {
        Collector { store, chunk_size: CHUNK_BYTES }
    }

    pub fn scan(&self, config: &Config) -> Result<(i64, Vec<String>)> {
        self.scan_with(config, DEFAULT_MAX_CHUNKS_PER_SCAN)
    }

    pub fn scan_with(&self, config: &Config, max_chunks: i64) -> Result<(i64, Vec<String>)> {
        if !config.consent {
            return Ok((0, Vec::new()));
        }
        if config.bot_id.is_empty() || config.chat_id.is_empty() {
            return Err(Error::value("Сначала настройте бота и группу."));
        }
        let (mut total, mut errors) = (0, Vec::new());
        for source in &config.sources {
            match self.scan_source(source, config, (max_chunks - total).max(0)) {
                Ok(count) => total += count,
                // Do not expose paths or OS exception strings containing private paths.
                Err(error @ (Error::Os(_) | Error::Value(_))) => errors.push(format!("{}: источник недоступен ({}).", source.label, error.type_name())),
                Err(error) => return Err(error),
            }
        }
        Ok((total, errors))
    }

    pub fn scan_source(&self, source: &Source, config: &Config, budget: i64) -> Result<i64> {
        let mut paths: Vec<PathBuf> = (1..=source.rotations).rev().map(|n| PathBuf::from(format!("{}.{n}", source.path))).collect();
        paths.push(PathBuf::from(&source.path));
        // Probe the live file first, independent of the historical rotation backlog.
        for path in paths.iter().rev() {
            let mut stream = match open_regular(path) {
                Ok(stream) => stream,
                Err(error) if not_found(&error) => continue,
                Err(error) => return Err(error),
            };
            if stream.metadata()?.len() == 0 {
                continue;
            }
            if let Some(version) = latest_version(&mut stream)? {
                observe_version(self.store, config, source, &version)?;
            }
            break;
        }
        if budget <= 0 {
            return Ok(0); // Version probes must not wait behind another source's backlog.
        }
        let db = &self.store.db;
        let initial: Option<i64> = db.query_row("SELECT 1 FROM initialized WHERE source=?", [&source.id], |row| row.get(0)).optional()?;
        if initial.is_none() && !source.include_existing {
            // Initial baseline includes rotated files, so old logs cannot leak later.
            let transaction = db.unchecked_transaction()?;
            for path in &paths {
                let mut stream = match open_regular(path) {
                    Ok(stream) => stream,
                    Err(error) if not_found(&error) => continue,
                    Err(error) => return Err(error),
                };
                let identity = identity_of(&stream)?;
                let size = stream.metadata()?.len();
                let anchor = fingerprint(&mut stream, size)?;
                db.execute("INSERT OR IGNORE INTO cursors VALUES (?,?,?,?)", params![source.id, identity, size.bits::<i64>(), anchor])?;
                let version = latest_version(&mut stream)?;
                db.execute("INSERT OR REPLACE INTO cursor_context VALUES (?,?,?)", params![source.id, identity, version])?;
            }
            db.execute("INSERT INTO initialized VALUES (?)", [&source.id])?;
            transaction.commit()?;
            return Ok(0);
        }
        db.execute("INSERT OR IGNORE INTO initialized VALUES (?)", [&source.id])?;
        let (mut count, mut found) = (0, false);
        for path in &paths {
            if count >= budget {
                break;
            }
            let mut stream = match open_regular(path) {
                Ok(stream) => stream,
                Err(error) if not_found(&error) => continue,
                Err(error) => return Err(error),
            };
            found = true;
            let identity = identity_of(&stream)?;
            let size = stream.metadata()?.len();
            let mut cursor: Option<(i64, String)> = db
                .query_row("SELECT offset,anchor FROM cursors WHERE source=? AND identity=?", params![source.id, identity], |row| Ok((row.get(0)?, row.get(1)?)))
                .optional()?;
            if cursor.is_none() {
                cursor = self.adopt_copy(&source.id, &identity, &mut stream, size)?;
            }
            let mut offset = cursor.as_ref().map_or(0, |(offset, _)| (*offset).bits::<u64>());
            let mut reset = match &cursor {
                Some((_, anchor)) => offset > size || fingerprint(&mut stream, offset)? != *anchor,
                None => false,
            };
            if reset {
                offset = 0;
            }
            let context: Option<Option<String>> = db
                .query_row("SELECT version FROM cursor_context WHERE source=? AND identity=?", params![source.id, identity], |row| row.get(0))
                .optional()?;
            let mut version = if cursor.is_some() && !reset { context.flatten() } else { None };
            while offset < size && count < budget {
                stream.seek(SeekFrom::Start(offset))?;
                let mut data = Vec::new();
                stream.by_ref().take(self.chunk_size.to::<u64>().min(size - offset)).read_to_end(&mut data)?;
                if data.is_empty() {
                    break;
                }
                // Prefer line boundaries; huge individual lines remain byte-exact fragments.
                if let Some(newline) = data.iter().rposition(|byte| *byte == b'\n') {
                    if newline < data.len() - 1 {
                        data.truncate(newline + 1);
                    }
                }
                stream.seek(SeekFrom::Start(offset + data.len().to::<u64>()))?;
                let mut lookahead = Vec::new();
                stream.by_ref().take(HEADER_BYTES.to::<u64>()).read_to_end(&mut lookahead)?;
                let line_start = offset == 0 || {
                    stream.seek(SeekFrom::Start(offset - 1))?;
                    let mut byte = [0u8; 1];
                    stream.read_exact(&mut byte)?;
                    byte[0] == b'\n'
                };
                let (length, taken) = take_fragment(&data, version, line_start, &lookahead);
                version = taken;
                data.truncate(length);
                if data.is_empty() {
                    break; // The writer has not finished a version header yet.
                }
                let bundle_id = new_id();
                let now = now_seconds();
                let mut metadata = json!({
                    "schema": 1, "bundle_id": bundle_id, "collector_version": crate::VERSION, "device_id": config.device_id,
                    "device_name": config.device_name, "source_id": source.id, "source_label": source.label, "keyswitch_version": version,
                    "created_at": iso_at(now), "start_byte": offset, "end_byte": offset + data.len().to::<u64>(), "source_reset": reset,
                    "ends_with_newline": data.last() == Some(&b'\n'), "raw_sha256": sha256_hex(&data),
                });
                let payload = make_archive(&data, &metadata);
                if payload.len() > ARCHIVE_LIMIT {
                    return Err(Error::value("Архив превышает ограничение размера."));
                }
                metadata["sha256"] = json!(sha256_hex(&payload));
                metadata["size"] = json!(payload.len());
                let name = format!("lc-{}-{bundle_id}.zip", stamp_at(now));
                let new_offset = offset + data.len().to::<u64>();
                let anchor = fingerprint(&mut stream, new_offset)?;
                self.store.enqueue(&source.id, &identity, new_offset.bits::<i64>(), &anchor, &config.destination(), &metadata, &payload, &name, version.as_deref())?;
                offset = new_offset;
                reset = false;
                count += 1;
            }
        }
        if !found {
            return Err(Error::Os("FileNotFoundError"));
        }
        Ok(count)
    }

    /// Continue a file never seen before where a copy of it was already read.
    ///
    /// Rotation by copying (logrotate copytruncate) leaves the rotated slot as a new file whose bytes up to a cursor of
    /// this source are what that cursor read, the lines left out by the initial baseline included. Reading it from the
    /// start sent them again; the cursor of the furthest matching copy is taken over, and stored at once, so the match
    /// does not depend on that cursor staying put.
    fn adopt_copy(&self, source: &str, identity: &str, stream: &mut File, size: u64) -> Result<Option<(i64, String)>> {
        let db = &self.store.db;
        let known: Vec<(String, i64, String)> = {
            let mut statement = db.prepare("SELECT identity,offset,anchor FROM cursors WHERE source=? AND offset>0 AND offset<=? ORDER BY offset DESC")?;
            let rows = statement.query_map(params![source, size.bits::<i64>()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
            rows.collect::<sqlite::Result<_>>()?
        };
        for (other, offset, anchor) in known {
            if fingerprint(stream, offset.bits::<u64>())? != anchor {
                continue;
            }
            let context: Option<Option<String>> =
                db.query_row("SELECT version FROM cursor_context WHERE source=? AND identity=?", params![source, other], |row| row.get(0)).optional()?;
            let transaction = db.unchecked_transaction()?;
            db.execute("INSERT INTO cursors VALUES (?,?,?,?)", params![source, identity, offset, anchor])?;
            db.execute("INSERT OR REPLACE INTO cursor_context VALUES (?,?,?)", params![source, identity, context.flatten()])?;
            transaction.commit()?;
            return Ok(Some((offset, anchor)));
        }
        Ok(None)
    }
}
