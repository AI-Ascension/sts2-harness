// SPDX-License-Identifier: MIT
#![allow(unsafe_code)]

use std::mem::zeroed;
use std::ptr::addr_of_mut;

use windows_sys::Win32::Foundation::{
    ERROR_BROKEN_PIPE, ERROR_INVALID_HANDLE, ERROR_IO_INCOMPLETE, ERROR_IO_PENDING,
    ERROR_NOT_FOUND, ERROR_OPERATION_ABORTED, ERROR_PIPE_CONNECTED, ERROR_PIPE_NOT_CONNECTED,
    FALSE, GetLastError, HANDLE, TRUE, WAIT_TIMEOUT,
};
use windows_sys::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, ReadFile, WriteFile,
};
use windows_sys::Win32::System::IO::{
    CancelIoEx, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use zeroize::Zeroizing;

use super::resources::{Handle, ProtectedCredential};
use super::security::PipeSecurity;
use super::wide_string;
use crate::transport::{Deadline, TransportError};
use crate::{AUTH_MAGIC, MAX_AUTH_BODY_BYTES, MAX_FRAME_BYTES, MIN_AUTH_BODY_BYTES};

pub(super) fn create_pipe(
    name: &str,
    security: &PipeSecurity,
    first_instance: bool,
) -> Result<Handle, TransportError> {
    let name_wide = wide_string(name)?;
    let mut security = PipeSecurity {
        descriptor: Box::new(*security.descriptor),
        acl: security.acl.clone(),
        sids: security.sids.clone(),
    };
    security.descriptor.Dacl = security.acl.as_mut_ptr().cast();
    let mut attrs = security.attributes();
    let mut open_mode = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;
    if first_instance {
        open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    let handle = unsafe {
        CreateNamedPipeW(
            name_wide.as_ptr(),
            open_mode,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            MAX_FRAME_BYTES as u32,
            MAX_FRAME_BYTES as u32,
            0,
            addr_of_mut!(attrs),
        )
    };
    Handle::new(handle)
}

pub(super) fn connect_pipe(handle: HANDLE, deadline: Deadline) -> Result<(), TransportError> {
    deadline.check()?;
    let mut overlapped = zeroed_overlapped();
    let connected = unsafe { ConnectNamedPipe(handle, addr_of_mut!(overlapped)) };
    if connected != FALSE {
        deadline.check()?;
        return Ok(());
    }
    let error = unsafe { GetLastError() };
    if error == ERROR_PIPE_CONNECTED {
        deadline.check()?;
        return Ok(());
    }
    if error != ERROR_IO_PENDING {
        return Err(TransportError::Os);
    }
    wait_overlapped(handle, &mut overlapped, deadline).map(|_| ())
}

pub(super) fn authenticate(
    pipe: HANDLE,
    credential: &ProtectedCredential,
    deadline: Deadline,
) -> Result<(), TransportError> {
    let mut prefix = [0_u8; 4];
    read_exact(pipe, &mut prefix, deadline).map_err(map_auth_io)?;
    let length =
        usize::try_from(u32::from_be_bytes(prefix)).map_err(|_| TransportError::Framing)?;
    if !(MIN_AUTH_BODY_BYTES..=MAX_AUTH_BODY_BYTES).contains(&length) {
        return Err(TransportError::Framing);
    }
    let mut body = Zeroizing::new(vec![0_u8; length]);
    read_exact(pipe, &mut body, deadline).map_err(map_auth_io)?;
    if body[..AUTH_MAGIC.len()] != *AUTH_MAGIC {
        return Err(TransportError::Credential);
    }
    if !credential.matches(&body[AUTH_MAGIC.len()..]) {
        return Err(TransportError::Credential);
    }
    deadline.check()
}

fn map_auth_io(error: TransportError) -> TransportError {
    match error {
        TransportError::Framing => TransportError::Framing,
        TransportError::Deadline => TransportError::Deadline,
        TransportError::Closed => TransportError::Closed,
        _ => TransportError::Credential,
    }
}
pub(super) fn read_frame(
    pipe: HANDLE,
    limit: usize,
    deadline: Deadline,
) -> Result<Vec<u8>, TransportError> {
    let mut prefix = [0_u8; 4];
    read_exact(pipe, &mut prefix, deadline)?;
    let length =
        usize::try_from(u32::from_be_bytes(prefix)).map_err(|_| TransportError::Framing)?;
    if length == 0 || length > limit {
        return Err(TransportError::Framing);
    }
    let mut body = vec![0_u8; length];
    read_exact(pipe, &mut body, deadline)?;
    Ok(body)
}

pub(super) fn write_frame(
    pipe: HANDLE,
    body: &[u8],
    limit: usize,
    deadline: Deadline,
) -> Result<(), TransportError> {
    if body.is_empty() || body.len() > limit {
        return Err(TransportError::Framing);
    }
    let length = u32::try_from(body.len())
        .map_err(|_| TransportError::Framing)?
        .to_be_bytes();
    write_all(pipe, &length, deadline)?;
    write_all(pipe, body, deadline)
}

fn read_exact(pipe: HANDLE, body: &mut [u8], deadline: Deadline) -> Result<(), TransportError> {
    let mut offset = 0_usize;
    while offset < body.len() {
        let count = read_once(pipe, &mut body[offset..], deadline)?;
        if count == 0 {
            return Err(TransportError::Closed);
        }
        let next = offset.checked_add(count).ok_or(TransportError::Framing)?;
        if next > body.len() {
            return Err(TransportError::Framing);
        }
        offset = next;
    }
    Ok(())
}

fn write_all(pipe: HANDLE, body: &[u8], deadline: Deadline) -> Result<(), TransportError> {
    let mut offset = 0_usize;
    while offset < body.len() {
        let count = write_once(pipe, &body[offset..], deadline)?;
        if count == 0 {
            return Err(TransportError::Closed);
        }
        let next = offset.checked_add(count).ok_or(TransportError::Framing)?;
        if next > body.len() {
            return Err(TransportError::Framing);
        }
        offset = next;
    }
    Ok(())
}

fn read_once(pipe: HANDLE, body: &mut [u8], deadline: Deadline) -> Result<usize, TransportError> {
    deadline.check()?;
    let length = u32::try_from(body.len()).map_err(|_| TransportError::Framing)?;
    let mut transferred = 0_u32;
    let mut overlapped = zeroed_overlapped();
    let ok = unsafe {
        ReadFile(
            pipe,
            body.as_mut_ptr(),
            length,
            addr_of_mut!(transferred),
            addr_of_mut!(overlapped),
        )
    };
    if ok != FALSE {
        deadline.check()?;
        let count = usize::try_from(transferred).map_err(|_| TransportError::Framing)?;
        return (count <= body.len())
            .then_some(count)
            .ok_or(TransportError::Framing);
    }
    let error = unsafe { GetLastError() };
    if error == ERROR_BROKEN_PIPE || error == ERROR_PIPE_NOT_CONNECTED {
        return Err(TransportError::Closed);
    }
    if error != ERROR_IO_PENDING {
        return Err(TransportError::Os);
    }
    let count = wait_overlapped(pipe, &mut overlapped, deadline)?;
    if count > body.len() {
        return Err(TransportError::Framing);
    }
    Ok(count)
}

fn write_once(pipe: HANDLE, body: &[u8], deadline: Deadline) -> Result<usize, TransportError> {
    deadline.check()?;
    let length = u32::try_from(body.len()).map_err(|_| TransportError::Framing)?;
    let mut transferred = 0_u32;
    let mut overlapped = zeroed_overlapped();
    let ok = unsafe {
        WriteFile(
            pipe,
            body.as_ptr(),
            length,
            addr_of_mut!(transferred),
            addr_of_mut!(overlapped),
        )
    };
    if ok != FALSE {
        deadline.check()?;
        let count = usize::try_from(transferred).map_err(|_| TransportError::Framing)?;
        return (count <= body.len())
            .then_some(count)
            .ok_or(TransportError::Framing);
    }
    let error = unsafe { GetLastError() };
    if error == ERROR_BROKEN_PIPE || error == ERROR_PIPE_NOT_CONNECTED {
        return Err(TransportError::Closed);
    }
    if error != ERROR_IO_PENDING {
        return Err(TransportError::Os);
    }
    let count = wait_overlapped(pipe, &mut overlapped, deadline)?;
    if count > body.len() {
        return Err(TransportError::Framing);
    }
    Ok(count)
}

fn wait_overlapped(
    handle: HANDLE,
    overlapped: &mut OVERLAPPED,
    deadline: Deadline,
) -> Result<usize, TransportError> {
    let remaining = deadline.remaining_millis()?;
    let mut transferred = 0_u32;
    let ok = unsafe {
        GetOverlappedResultEx(
            handle,
            overlapped,
            addr_of_mut!(transferred),
            remaining,
            FALSE,
        )
    };
    if ok != FALSE {
        deadline.check()?;
        return usize::try_from(transferred).map_err(|_| TransportError::Framing);
    }
    let error = unsafe { GetLastError() };
    if error == WAIT_TIMEOUT {
        cancel_and_join(handle, overlapped)?;
        return Err(TransportError::Deadline);
    }
    if error == ERROR_OPERATION_ABORTED {
        return Err(TransportError::Closed);
    }
    if error == ERROR_BROKEN_PIPE || error == ERROR_PIPE_NOT_CONNECTED {
        return Err(TransportError::Closed);
    }
    Err(TransportError::Os)
}

fn cancel_and_join(handle: HANDLE, overlapped: &mut OVERLAPPED) -> Result<(), TransportError> {
    let cancelled = unsafe { CancelIoEx(handle, overlapped) };
    if cancelled == FALSE {
        let error = unsafe { GetLastError() };
        if error != ERROR_NOT_FOUND && error != ERROR_INVALID_HANDLE {
            return Err(TransportError::Os);
        }
    }
    let mut transferred = 0_u32;
    loop {
        let completed =
            unsafe { GetOverlappedResult(handle, overlapped, addr_of_mut!(transferred), TRUE) };
        if completed != FALSE {
            return Ok(());
        }
        let error = unsafe { GetLastError() };
        if error == ERROR_OPERATION_ABORTED || error == ERROR_BROKEN_PIPE {
            return Ok(());
        }
        if error != ERROR_IO_INCOMPLETE {
            return Err(TransportError::Os);
        }
    }
}

fn zeroed_overlapped() -> OVERLAPPED {
    // OVERLAPPED is a C POD; all fields must be zero before the native call.
    unsafe { zeroed() }
}

pub(super) fn disconnect(handle: HANDLE) {
    unsafe {
        let _ = DisconnectNamedPipe(handle);
    }
}
