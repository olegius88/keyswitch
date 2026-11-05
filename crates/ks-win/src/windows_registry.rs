//! Native registry adapter isolated from platform-independent logic.
//!
//! The Python uses `winreg`; `RegistryValue` is what its `QueryValueEx` turns a value into, so that `isinstance` checks read
//! the same here: a number for `REG_DWORD` and `REG_QWORD`, text up to the first zero for `REG_SZ` and `REG_EXPAND_SZ`, and bytes
//! for the rest, with an empty value of those being None.

#![cfg_attr(windows, expect(unsafe_code, reason = "calls into Win32"))]
use ks_cast::To as _;
use std::io;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE, KEY_WOW64_32KEY, KEY_WOW64_64KEY, REG_DWORD,
    REG_EXPAND_SZ, REG_MULTI_SZ, REG_OPTION_NON_VOLATILE, REG_QWORD, REG_SZ, REG_VALUE_TYPE, RegCloseKey, RegCreateKeyExW,
    RegDeleteValueW, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};

use crate::constants::{
    APP_PATHS_KEY, APPS_USE_LIGHT_THEME_VALUE, PERSONALIZE_KEY, REGISTRY_KEY_NAME_BUFFER_CHARACTERS,
    REGISTRY_VALUE_FIRST_BUFFER_BYTES, RUN_KEY, STARTUP_APPROVED_KEY, UNINSTALL_KEY,
};
use crate::windows_native::wide;
use crate::windows_system::WindowsRegistry;

#[derive(Debug, PartialEq, Eq)]
enum RegistryValue {
    Number(i64),
    Text(String),
    /// A list of strings: neither text nor bytes to the callers.
    TextList,
    Bytes(Vec<u8>),
    Empty,
}

/// An open registry key, closed when dropped.
struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: the handle came from RegOpenKeyExW or RegCreateKeyExW and is closed once.
        unsafe { RegCloseKey(self.0) };
    }
}

fn check(status: u32) -> io::Result<()> {
    if status == ERROR_SUCCESS { Ok(()) } else { Err(io::Error::from_raw_os_error(status.bits::<i32>())) }
}

/// `winreg.OpenKey`, with its default of `KEY_READ` where the caller asks for no more.
fn open(root: HKEY, path: &str, access: u32) -> io::Result<Key> {
    let name = wide(path);
    let mut key: HKEY = null_mut();
    // SAFETY: `name` is NUL-terminated and `key` is a valid place for the handle.
    check(unsafe { RegOpenKeyExW(root, name.as_ptr(), 0, access, &raw mut key) })?;
    Ok(Key(key))
}

/// `winreg.QueryValueEx`.
fn query(key: &Key, name: &str) -> io::Result<RegistryValue> {
    let name = wide(name);
    let mut data = vec![0u8; REGISTRY_VALUE_FIRST_BUFFER_BYTES];
    loop {
        let mut value_type: REG_VALUE_TYPE = 0;
        let mut size = data.len().low::<u32>();
        // SAFETY: `name` is NUL-terminated; `size` says how many bytes `data` holds.
        let status = unsafe {
            RegQueryValueExW(key.0, name.as_ptr(), null(), &raw mut value_type, data.as_mut_ptr(), &raw mut size)
        };
        if status == ERROR_MORE_DATA {
            let wanted = size.to::<usize>().max(data.len() * 2);
            data.resize(wanted, 0);
            continue;
        }
        check(status)?;
        data.truncate(size.to::<usize>());
        return Ok(decode(value_type, data));
    }
}

fn decode(value_type: REG_VALUE_TYPE, data: Vec<u8>) -> RegistryValue {
    match value_type {
        REG_DWORD => RegistryValue::Number(
            data.get(..4).and_then(|bytes| bytes.try_into().ok()).map_or(0, |bytes| i64::from(u32::from_le_bytes(bytes))),
        ),
        REG_QWORD => {
            RegistryValue::Number(data.get(..8).and_then(|bytes| bytes.try_into().ok()).map_or(0, i64::from_le_bytes))
        }
        REG_SZ | REG_EXPAND_SZ => {
            let units: Vec<u16> =
                data.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair)).take_while(|&unit| unit != 0).collect();
            RegistryValue::Text(String::from_utf16_lossy(&units))
        }
        REG_MULTI_SZ => RegistryValue::TextList,
        _ if data.is_empty() => RegistryValue::Empty,
        _ => RegistryValue::Bytes(data),
    }
}

/// `value if isinstance(value, str) else None`, with a value that is not there being None too.
fn text_or_none(value: io::Result<RegistryValue>) -> io::Result<Option<String>> {
    match value {
        Ok(RegistryValue::Text(text)) => Ok(Some(text)),
        Ok(_) => Ok(None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn delete_value(path: &str, name: &str) -> io::Result<()> {
    let result = (|| {
        let key = open(HKEY_CURRENT_USER, path, KEY_SET_VALUE)?;
        let name = wide(name);
        // SAFETY: the key is open with KEY_SET_VALUE and `name` is NUL-terminated.
        check(unsafe { RegDeleteValueW(key.0, name.as_ptr()) })
    })();
    match result {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// `winreg.EnumKey` for one index, None when there is no such key or it cannot be read: the callers stop there.
fn enumerate_key(key: &Key, index: u32) -> Option<String> {
    let mut buffer = [0u16; REGISTRY_KEY_NAME_BUFFER_CHARACTERS];
    let mut size = buffer.len().low::<u32>();
    // SAFETY: `size` says how many characters `buffer` holds; the optional out parameters are null.
    let status = unsafe {
        RegEnumKeyExW(key.0, index, buffer.as_mut_ptr(), &raw mut size, null(), null_mut(), null_mut(), null_mut())
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    buffer.get(..size.to::<usize>()).map(String::from_utf16_lossy)
}

/// `winreg.OpenKey(root, path, 0, KEY_READ | view)` where a missing key or a refusal means an empty list.
fn open_listing(root: HKEY, path: &str, view: u32) -> io::Result<Option<Key>> {
    match open(root, path, KEY_READ | view) {
        Ok(key) => Ok(Some(key)),
        Err(error) if matches!(error.kind(), io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied) => Ok(None),
        Err(error) => Err(error),
    }
}

fn application_paths_for(root: HKEY, view: u32, result: &mut Vec<(String, String)>) -> io::Result<()> {
    let Some(key) = open_listing(root, APP_PATHS_KEY, view)? else {
        return Ok(());
    };
    let mut index = 0;
    while let Some(subkey_name) = enumerate_key(&key, index) {
        index += 1;
        let Ok(subkey) = open(key.0, &subkey_name, KEY_READ) else {
            continue;
        };
        if let Ok(RegistryValue::Text(value)) = query(&subkey, "") {
            result.push((subkey_name, value));
        }
    }
    Ok(())
}

fn uninstall_paths_for(root: HKEY, view: u32, result: &mut Vec<(String, String)>) -> io::Result<()> {
    let Some(key) = open_listing(root, UNINSTALL_KEY, view)? else {
        return Ok(());
    };
    let mut index = 0;
    while let Some(subkey_name) = enumerate_key(&key, index) {
        index += 1;
        let Ok(subkey) = open(key.0, &subkey_name, KEY_READ) else {
            continue;
        };
        let Ok(icon) = query(&subkey, "DisplayIcon") else {
            continue;
        };
        let name = match query(&subkey, "DisplayName") {
            Ok(name) => name,
            Err(error) if error.kind() == io::ErrorKind::NotFound => RegistryValue::Text(subkey_name.clone()),
            Err(_) => continue,
        };
        if let (RegistryValue::Text(icon), RegistryValue::Text(name)) = (icon, name) {
            result.push((name, icon));
        }
    }
    Ok(())
}

pub struct NativeWindowsRegistry;

impl NativeWindowsRegistry {
    /// The app mode of the user, or None when Windows keeps none or it cannot be read.
    pub fn read_apps_use_light_theme(&self) -> Option<i64> {
        let key = open(HKEY_CURRENT_USER, PERSONALIZE_KEY, KEY_READ).ok()?;
        match query(&key, APPS_USE_LIGHT_THEME_VALUE).ok()? {
            RegistryValue::Number(value) => Some(value),
            RegistryValue::Text(_) | RegistryValue::TextList | RegistryValue::Bytes(_) | RegistryValue::Empty => None,
        }
    }
}

impl WindowsRegistry for NativeWindowsRegistry {
    fn read_autostart(&self, name: &str) -> io::Result<Option<String>> {
        let key = match open(HKEY_CURRENT_USER, RUN_KEY, KEY_READ) {
            Ok(key) => key,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        text_or_none(query(&key, name))
    }

    fn write_autostart(&self, name: &str, command: &str) -> io::Result<()> {
        let path = wide(RUN_KEY);
        let mut raw: HKEY = null_mut();
        // SAFETY: `path` is NUL-terminated, `raw` is a valid place for the handle, and the optional pointers are null.
        check(unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                path.as_ptr(),
                0,
                null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                null(),
                &raw mut raw,
                null_mut(),
            )
        })?;
        let key = Key(raw);
        let name = wide(name);
        let data = wide(command);
        // SAFETY: `name` is NUL-terminated, and the size is that of `data` with its terminating zero, in bytes.
        check(unsafe {
            RegSetValueExW(key.0, name.as_ptr(), 0, REG_SZ, data.as_ptr().cast::<u8>(), (data.len() * 2).low::<u32>())
        })
    }

    fn delete_autostart(&self, name: &str) -> io::Result<()> {
        delete_value(RUN_KEY, name)
    }

    fn read_startup_approval(&self, name: &str) -> io::Result<Option<Vec<u8>>> {
        let key = match open(HKEY_CURRENT_USER, STARTUP_APPROVED_KEY, KEY_READ) {
            Ok(key) => key,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        match query(&key, name) {
            Ok(RegistryValue::Bytes(value)) => Ok(Some(value)),
            Ok(_) => Ok(None),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Remove the approval record so Windows treats the value as enabled again.
    fn clear_startup_approval(&self, name: &str) -> io::Result<()> {
        delete_value(STARTUP_APPROVED_KEY, name)
    }

    fn application_paths(&self) -> io::Result<Vec<(String, String)>> {
        let mut result = Vec::new();
        for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
            for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
                uninstall_paths_for(root, view, &mut result)?;
                application_paths_for(root, view, &mut result)?;
            }
        }
        Ok(result)
    }
}
