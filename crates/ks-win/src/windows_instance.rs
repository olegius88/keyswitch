//! Single-instance coordination for the Windows frontend: the logic, and below it the native named mutex and the search
//! for the running program's window (the Python's `windows_instance.py` and `windows_instance_native.py`).

#![cfg_attr(windows, expect(unsafe_code, reason = "calls into Win32"))]
use crate::windows_system::WindowsSystemError;

pub trait WindowsInstanceAPI {
    fn acquire(&mut self) -> Result<bool, WindowsSystemError>;
    fn activate_existing(&mut self) -> bool;
    fn close(&mut self);
}

#[cfg(windows)]
fn native_api() -> Result<Box<dyn WindowsInstanceAPI>, WindowsSystemError> {
    Ok(Box::new(NativeWindowsInstanceAPI::new()))
}

#[cfg(not(windows))]
fn native_api() -> Result<Box<dyn WindowsInstanceAPI>, WindowsSystemError> {
    Err(WindowsSystemError::Failed("Single-instance Win32 доступен только в Windows".to_owned()))
}

pub struct WindowsSingleInstance {
    api: Box<dyn WindowsInstanceAPI>,
    closed: bool,
}

impl WindowsSingleInstance {
    pub fn new(api: Option<Box<dyn WindowsInstanceAPI>>) -> Result<Self, WindowsSystemError> {
        Ok(WindowsSingleInstance {
            api: match api {
                Some(api) => api,
                None => native_api()?,
            },
            closed: false,
        })
    }

    pub fn acquire(&mut self) -> Result<bool, WindowsSystemError> {
        if self.closed { Ok(false) } else { self.api.acquire() }
    }

    pub fn activate_existing(&mut self) -> bool {
        !self.closed && self.api.activate_existing()
    }

    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.api.close();
    }
}

#[cfg(windows)]
pub use native::NativeWindowsInstanceAPI;

/// Native named-mutex and existing-window adapter for Windows.
#[cfg(windows)]
mod native {
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE};
    use windows_sys::Win32::System::Threading::CreateMutexW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow, ShowWindow};

    use super::WindowsInstanceAPI;
    use crate::constants::{ERROR_ALREADY_EXISTS, MUTEX_NAME, SETTINGS_WINDOW_CLASS, SW_RESTORE};
    use crate::windows_native::wide;
    use crate::windows_system::WindowsSystemError;

    pub struct NativeWindowsInstanceAPI {
        handle: HANDLE,
    }

    impl NativeWindowsInstanceAPI {
        #[must_use]
        pub fn new() -> Self {
            NativeWindowsInstanceAPI { handle: null_mut() }
        }
    }

    impl Default for NativeWindowsInstanceAPI {
        fn default() -> Self {
            Self::new()
        }
    }

    impl WindowsInstanceAPI for NativeWindowsInstanceAPI {
        fn acquire(&mut self) -> Result<bool, WindowsSystemError> {
            if !self.handle.is_null() {
                return Ok(true);
            }
            let name = wide(MUTEX_NAME);
            // SAFETY: `name` is NUL-terminated and null security attributes are allowed. GetLastError right after the call
            // tells a new mutex from an existing one.
            let (handle, code) = unsafe {
                let handle = CreateMutexW(null(), 0, name.as_ptr());
                (handle, GetLastError())
            };
            if handle.is_null() {
                return Err(WindowsSystemError::Failed(format!(
                    "Не удалось создать single-instance mutex (Win32 error {code})"
                )));
            }
            self.handle = handle;
            Ok(code != ERROR_ALREADY_EXISTS)
        }

        /// The running program, found by the private class of its settings window: the title `KeySwitch` also belongs to
        /// the learning prompt, which is not the window to bring forward.
        fn activate_existing(&mut self) -> bool {
            let class = wide(SETTINGS_WINDOW_CLASS);
            // SAFETY: `class` is NUL-terminated and a null title matches any window of the class. The other two calls take
            // the handle by value and fail on a window that is gone.
            unsafe {
                let window = FindWindowW(class.as_ptr(), null());
                if window.is_null() {
                    return false;
                }
                ShowWindow(window, SW_RESTORE);
                SetForegroundWindow(window) != 0
            }
        }

        fn close(&mut self) {
            let handle = std::mem::replace(&mut self.handle, null_mut());
            if !handle.is_null() {
                // SAFETY: the handle came from CreateMutexW and is closed once.
                unsafe { CloseHandle(handle) };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;

    #[derive(Default)]
    struct Calls {
        acquire: Cell<u32>,
        activate: Cell<u32>,
        close: Cell<u32>,
    }

    struct FakeInstanceAPI {
        acquired: bool,
        activated: bool,
        calls: Rc<Calls>,
    }

    impl WindowsInstanceAPI for FakeInstanceAPI {
        fn acquire(&mut self) -> Result<bool, WindowsSystemError> {
            self.calls.acquire.set(self.calls.acquire.get() + 1);
            Ok(self.acquired)
        }

        fn activate_existing(&mut self) -> bool {
            self.calls.activate.set(self.calls.activate.get() + 1);
            self.activated
        }

        fn close(&mut self) {
            self.calls.close.set(self.calls.close.get() + 1);
        }
    }

    #[test]
    fn lifecycle_duplicate_activation_and_idempotent_close() {
        let calls = Rc::new(Calls::default());
        let api = FakeInstanceAPI { acquired: false, activated: true, calls: Rc::clone(&calls) };
        let mut instance = WindowsSingleInstance::new(Some(Box::new(api))).unwrap();
        assert!(!instance.acquire().unwrap());
        assert!(instance.activate_existing());
        instance.close();
        instance.close();
        assert!(!instance.acquire().unwrap());
        assert!(!instance.activate_existing());
        assert_eq!((calls.acquire.get(), calls.activate.get(), calls.close.get()), (1, 1, 1));
    }

    #[cfg(not(windows))]
    #[test]
    fn the_native_factory_is_guarded() {
        let error = WindowsSingleInstance::new(None).err().unwrap();
        assert!(error.to_string().contains("только в Windows"));
    }
}
