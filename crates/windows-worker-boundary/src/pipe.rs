// SPDX-License-Identifier: MIT

use crate::handle::{NativeHandle, last_error, wide};
use crate::identity::{PeerExpectation, current_user_sid, validate_sid};
use crate::pipe_stream::WorkerPipeStream;
use std::ffi::c_void;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{
    ERROR_NO_DATA, ERROR_PIPE_CONNECTED, ERROR_PIPE_LISTENING, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_NOWAIT, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
};

const MAX_PIPE_INSTANCES: u32 = 8;
const PIPE_BUFFER_BYTES: u32 = 65_536;

/// One owner-ACL, local-only Windows worker endpoint.
pub struct WorkerPipeListener {
    handle: NativeHandle,
    name: Vec<u16>,
    security: SecurityDescriptor,
    expected: PeerExpectation,
}

impl std::fmt::Debug for WorkerPipeListener {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WorkerPipeListener")
            .field("name", &"<protected-pipe>")
            .field("expected", &self.expected)
            .finish_non_exhaustive()
    }
}

impl WorkerPipeListener {
    pub fn create(name: &str, expected: PeerExpectation) -> Result<Self, String> {
        validate_pipe_name(name)?;
        let current_sid = current_user_sid()?;
        if current_sid != expected.sid {
            return Err(String::from(
                "worker peer SID does not match the endpoint owner",
            ));
        }
        let security = SecurityDescriptor::for_sid(&expected.sid)?;
        let name = wide(name, "worker endpoint name")?;
        let handle = create_instance(&name, &security, true)?;
        Ok(Self {
            handle,
            name,
            security,
            expected,
        })
    }

    /// Poll one connection. A peer that fails identity checks is discarded
    /// and reported as no connection; a native endpoint failure is returned.
    pub fn accept(&mut self) -> Result<Option<WorkerPipeStream>, String> {
        let connected = unsafe { ConnectNamedPipe(self.handle.raw(), null_mut()) } != 0;
        if !connected {
            let code = unsafe { windows_sys::Win32::Foundation::GetLastError() };
            if code == ERROR_PIPE_LISTENING || code == ERROR_NO_DATA {
                return Ok(None);
            }
            if code != ERROR_PIPE_CONNECTED {
                return Err(crate::handle::win32_error("ConnectNamedPipe(worker)", code));
            }
        }

        let current = std::mem::replace(
            &mut self.handle,
            create_instance(&self.name, &self.security, false)?,
        );
        let stream = match crate::pipe_stream::authenticate_pipe(current, &self.expected) {
            Ok((handle, peer)) => Some(WorkerPipeStream::new(handle, peer)),
            Err(_) => None,
        };
        Ok(stream)
    }
}

struct SecurityDescriptor(*mut c_void);

impl SecurityDescriptor {
    fn for_sid(sid: &str) -> Result<Self, String> {
        validate_sid(sid)?;
        let sddl = format!("D:P(A;;GA;;;{sid})");
        let wide = wide(&sddl, "worker endpoint security descriptor")?;
        let mut raw = null_mut();
        let mut size = 0_u32;
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide.as_ptr(),
                SDDL_REVISION_1,
                &raw mut raw,
                &raw mut size,
            )
        } == 0
        {
            return Err(last_error(
                "ConvertStringSecurityDescriptorToSecurityDescriptorW",
            ));
        }
        if raw.is_null() || size == 0 {
            return Err(String::from("worker endpoint security descriptor is empty"));
        }
        Ok(Self(raw.cast()))
    }

    fn raw(&self) -> *mut c_void {
        self.0
    }
}

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        let _ = unsafe { LocalFree(self.0) };
    }
}

fn create_instance(
    name: &[u16],
    security: &SecurityDescriptor,
    first: bool,
) -> Result<NativeHandle, String> {
    let mut open_mode = PIPE_ACCESS_DUPLEX;
    if first {
        open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    let pipe_mode = PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(std::mem::size_of::<SECURITY_ATTRIBUTES>())
            .map_err(|_| String::from("worker pipe security attributes overflow"))?,
        lpSecurityDescriptor: security.raw(),
        bInheritHandle: 0,
    };
    let raw = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            open_mode,
            pipe_mode,
            MAX_PIPE_INSTANCES,
            PIPE_BUFFER_BYTES,
            PIPE_BUFFER_BYTES,
            2_000,
            &raw const attributes,
        )
    };
    NativeHandle::new(raw, "CreateNamedPipeW(worker)")
}

fn validate_pipe_name(name: &str) -> Result<(), String> {
    const PREFIX: &str = r"\\.\pipe\ascension-worker-";
    if !name.starts_with(PREFIX) {
        return Err(String::from(
            "worker endpoint name is outside its namespace",
        ));
    }
    let nonce = &name[PREFIX.len()..];
    if nonce.len() != 36 || nonce.as_bytes()[14] != b'4' {
        return Err(String::from("worker endpoint name has an invalid nonce"));
    }
    for (index, byte) in nonce.bytes().enumerate() {
        let hyphen = matches!(index, 8 | 13 | 18 | 23);
        if hyphen {
            if byte != b'-' {
                return Err(String::from("worker endpoint name has an invalid nonce"));
            }
        } else if !(byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
            return Err(String::from("worker endpoint name has an invalid nonce"));
        }
    }
    if !matches!(nonce.as_bytes()[19], b'8'..=b'b') {
        return Err(String::from("worker endpoint name has an invalid nonce"));
    }
    Ok(())
}
