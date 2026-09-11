// SPDX-License-Identifier: MIT

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE};

pub(crate) struct NativeHandle(HANDLE);

impl NativeHandle {
    pub(crate) fn new(raw: HANDLE, operation: &str) -> Result<Self, String> {
        if raw.is_null() || raw == INVALID_HANDLE_VALUE {
            return Err(win32_error(operation, unsafe { GetLastError() }));
        }
        Ok(Self(raw))
    }

    pub(crate) const fn raw(&self) -> HANDLE {
        self.0
    }
}

// A NativeHandle owns exactly one kernel handle and never exposes it outside
// this boundary. Moving the owner between the bounded worker threads is the
// only cross-thread operation; Windows kernel handles remain valid until the
// owner is dropped. It is intentionally not Sync: mutable pipe/process
// operations must stay on their owning worker thread.
unsafe impl Send for NativeHandle {}

impl Drop for NativeHandle {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

pub(crate) fn win32_error(operation: &str, code: u32) -> String {
    format!("{operation} failed with Win32 error {code}")
}

pub(crate) fn last_error(operation: &str) -> String {
    win32_error(operation, unsafe { GetLastError() })
}

pub(crate) fn wide(value: &str, operation: &str) -> Result<Vec<u16>, String> {
    if value.is_empty() || value.contains('\0') {
        return Err(format!("{operation} contains an invalid string"));
    }
    Ok(value.encode_utf16().chain(std::iter::once(0)).collect())
}

pub(crate) fn wide_path(path: &std::path::Path, operation: &str) -> Result<Vec<u16>, String> {
    use std::os::windows::ffi::OsStrExt;
    let value = path.as_os_str().encode_wide().collect::<Vec<_>>();
    if value.is_empty() || value.contains(&0) {
        return Err(format!("{operation} contains an invalid path"));
    }
    Ok(value.into_iter().chain(std::iter::once(0)).collect())
}
