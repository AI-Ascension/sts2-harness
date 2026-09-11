// SPDX-License-Identifier: MIT

use std::io;
use std::os::windows::io::AsRawHandle;
use std::thread;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{
    ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_NO_DATA, ERROR_OPERATION_ABORTED,
    ERROR_PIPE_LISTENING, ERROR_PIPE_NOT_CONNECTED, HANDLE,
};
use windows_sys::Win32::Storage::FileSystem::ReadFile;
use windows_sys::Win32::System::Pipes::{PIPE_NOWAIT, SetNamedPipeHandleState};

const POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Read one length-delimited startup frame from the inherited watchdog pipe.
/// The pipe is switched to bounded polling mode so a malformed launcher or a
/// suspended parent cannot leave the worker blocked forever.
pub fn read_bootstrap_stdin(
    magic: &[u8; 8],
    maximum_payload: usize,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    if maximum_payload == 0 || timeout.is_zero() {
        return Err(String::from("worker bootstrap bounds are invalid"));
    }
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| String::from("worker bootstrap deadline overflow"))?;
    let stdin = io::stdin();
    let handle = stdin.as_raw_handle();
    let mode = PIPE_NOWAIT;
    if unsafe {
        SetNamedPipeHandleState(handle, &raw const mode, std::ptr::null(), std::ptr::null())
    } == 0
    {
        return Err(String::from("worker bootstrap stdin is not a polling pipe"));
    }
    let mut prefix = [0_u8; 12];
    read_exact(handle, &mut prefix, deadline)?;
    if &prefix[..magic.len()] != magic {
        return Err(String::from("worker bootstrap magic is invalid"));
    }
    let payload_length = usize::try_from(u32::from_be_bytes([
        prefix[8], prefix[9], prefix[10], prefix[11],
    ]))
    .map_err(|_| String::from("worker bootstrap payload length overflow"))?;
    if payload_length == 0 || payload_length > maximum_payload {
        return Err(String::from("worker bootstrap payload length is invalid"));
    }
    let mut payload = vec![0_u8; payload_length];
    read_exact(handle, &mut payload, deadline)?;
    let mut trailing = [0_u8; 1];
    loop {
        match read_once(handle, &mut trailing)? {
            Some(0) => break,
            Some(_) => return Err(String::from("worker bootstrap has trailing bytes")),
            None => wait_for(deadline)?,
        }
    }
    let mut frame = Vec::with_capacity(prefix.len() + payload.len());
    frame.extend_from_slice(&prefix);
    frame.extend_from_slice(&payload);
    Ok(frame)
}

fn read_exact(handle: HANDLE, target: &mut [u8], deadline: Instant) -> Result<(), String> {
    let mut offset = 0;
    while offset < target.len() {
        match read_once(handle, &mut target[offset..])? {
            Some(0) => return Err(String::from("worker bootstrap stdin closed early")),
            Some(count) => offset += count,
            None => wait_for(deadline)?,
        }
    }
    Ok(())
}

fn read_once(handle: HANDLE, buffer: &mut [u8]) -> Result<Option<usize>, String> {
    let mut read = 0_u32;
    if unsafe {
        ReadFile(
            handle,
            buffer.as_mut_ptr().cast(),
            u32::try_from(buffer.len())
                .map_err(|_| String::from("worker bootstrap buffer overflow"))?,
            &raw mut read,
            std::ptr::null_mut(),
        )
    } != 0
    {
        let read = usize::try_from(read)
            .map_err(|_| String::from("worker bootstrap read count overflow"))?;
        if read > buffer.len() {
            return Err(String::from("worker bootstrap read count is invalid"));
        }
        return Ok(Some(read));
    }
    let code = unsafe { windows_sys::Win32::Foundation::GetLastError() };
    if matches!(
        code,
        ERROR_NO_DATA | ERROR_PIPE_LISTENING | ERROR_IO_PENDING
    ) {
        return Ok(None);
    }
    if matches!(
        code,
        ERROR_BROKEN_PIPE | ERROR_PIPE_NOT_CONNECTED | ERROR_OPERATION_ABORTED
    ) {
        return Ok(Some(0));
    }
    Err(crate::handle::win32_error(
        "ReadFile(worker bootstrap)",
        code,
    ))
}

fn wait_for(deadline: Instant) -> Result<(), String> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(String::from("worker bootstrap deadline expired"));
    }
    thread::sleep(remaining.min(POLL_INTERVAL));
    Ok(())
}
