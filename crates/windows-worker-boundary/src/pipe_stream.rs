// SPDX-License-Identifier: MIT

use crate::handle::{NativeHandle, last_error};
use crate::identity::{
    PeerExpectation, ProcessIdentity, open_process_identity, process_creation_time,
    process_user_sid, query_image_path,
};
use crate::image::{HeldFile, file_identity, open_hashed_file, same_path};
use std::thread;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{
    ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_MORE_DATA, ERROR_NO_DATA, ERROR_OPERATION_ABORTED,
    ERROR_PIPE_LISTENING, ERROR_PIPE_NOT_CONNECTED, HANDLE,
};
use windows_sys::Win32::System::Pipes::{GetNamedPipeClientProcessId, GetNamedPipeClientSessionId};

const POLL_INTERVAL: Duration = Duration::from_millis(5);

pub struct WorkerPipeStream {
    handle: NativeHandle,
    peer: VerifiedPeer,
}

impl std::fmt::Debug for WorkerPipeStream {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WorkerPipeStream")
            .field("peer", &self.peer)
            .finish_non_exhaustive()
    }
}

impl WorkerPipeStream {
    pub(crate) fn new(handle: NativeHandle, peer: VerifiedPeer) -> Self {
        Self { handle, peer }
    }

    pub fn read_frame(&mut self, timeout: Duration, maximum: usize) -> Result<Vec<u8>, String> {
        let deadline = deadline(timeout)?;
        let mut prefix = [0_u8; 4];
        self.read_exact(&mut prefix, deadline)?;
        let length = usize::try_from(u32::from_be_bytes(prefix))
            .map_err(|_| String::from("worker frame length overflow"))?;
        if length == 0 || length > maximum {
            return Err(String::from("worker frame exceeds its bound"));
        }
        let mut body = vec![0_u8; length];
        self.read_exact(&mut body, deadline)?;
        Ok(body)
    }

    pub fn write_frame(
        &mut self,
        body: &[u8],
        timeout: Duration,
        maximum: usize,
    ) -> Result<(), String> {
        if body.is_empty() || body.len() > maximum {
            return Err(String::from("worker frame exceeds its bound"));
        }
        let deadline = deadline(timeout)?;
        self.write_all(
            &u32::try_from(body.len())
                .map_err(|_| String::from("worker frame length overflow"))?
                .to_be_bytes(),
            deadline,
        )?;
        self.write_all(body, deadline)
    }

    fn read_exact(&mut self, target: &mut [u8], deadline: Instant) -> Result<(), String> {
        let mut offset = 0;
        while offset < target.len() {
            self.peer.verify(self.handle.raw())?;
            match read_once(self.handle.raw(), &mut target[offset..])? {
                Some(0) => return Err(String::from("worker pipe closed during read")),
                Some(count) => offset += count,
                None => wait_for(deadline)?,
            }
        }
        self.peer.verify(self.handle.raw())
    }

    fn write_all(&mut self, bytes: &[u8], deadline: Instant) -> Result<(), String> {
        let mut offset = 0;
        while offset < bytes.len() {
            self.peer.verify(self.handle.raw())?;
            match write_once(self.handle.raw(), &bytes[offset..])? {
                Some(0) => return Err(String::from("worker pipe closed during write")),
                Some(count) => offset += count,
                None => wait_for(deadline)?,
            }
        }
        self.peer.verify(self.handle.raw())
    }
}

pub(crate) struct VerifiedPeer {
    process: ProcessIdentity,
    image: HeldFile,
    expected: PeerExpectation,
}

impl std::fmt::Debug for VerifiedPeer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VerifiedPeer")
            .field("pid", &self.expected.pid)
            .field("creation_time_100ns", &self.expected.creation_time_100ns)
            .field("executable", &"<protected-reference>")
            .field("sid", &"<protected-policy>")
            .finish_non_exhaustive()
    }
}

pub(crate) fn authenticate_pipe(
    handle: NativeHandle,
    expected: &PeerExpectation,
) -> Result<(NativeHandle, VerifiedPeer), String> {
    let (pid, session_id) = pipe_peer(handle.raw())?;
    if pid != expected.pid || session_id != expected.session_id {
        return Err(String::from(
            "worker pipe peer process identity is not approved",
        ));
    }
    let process = open_process_identity(pid, session_id)?;
    if process.creation_time_100ns != expected.creation_time_100ns
        || !same_path(&process.executable, &expected.executable)
        || process.sid != expected.sid
    {
        return Err(String::from(
            "worker pipe peer process identity is not approved",
        ));
    }
    let image = open_hashed_file(&process.executable, "worker peer executable")?;
    if image.digest != expected.executable_sha256 {
        return Err(String::from(
            "worker peer executable digest is not approved",
        ));
    }
    Ok((
        handle,
        VerifiedPeer {
            process,
            image,
            expected: expected.clone(),
        },
    ))
}

fn pipe_peer(handle: HANDLE) -> Result<(u32, u32), String> {
    let mut pid = 0_u32;
    if unsafe { GetNamedPipeClientProcessId(handle, &raw mut pid) } == 0 || pid == 0 {
        return Err(last_error("GetNamedPipeClientProcessId(worker)"));
    }
    let mut session = 0_u32;
    if unsafe { GetNamedPipeClientSessionId(handle, &raw mut session) } == 0 {
        return Err(last_error("GetNamedPipeClientSessionId(worker)"));
    }
    Ok((pid, session))
}

impl VerifiedPeer {
    fn verify(&self, pipe: HANDLE) -> Result<(), String> {
        let (pid, session_id) = pipe_peer(pipe)?;
        if pid != self.expected.pid || session_id != self.expected.session_id {
            return Err(String::from("worker pipe peer identity changed"));
        }
        if process_creation_time(self.process.handle.raw())? != self.expected.creation_time_100ns
            || !same_path(
                &query_image_path(self.process.handle.raw())?,
                &self.expected.executable,
            )
            || process_user_sid(self.process.handle.raw())? != self.expected.sid
        {
            return Err(String::from("worker pipe peer identity changed"));
        }
        if file_identity(self.image.handle.raw())? != self.image.identity {
            return Err(String::from("worker peer image identity changed"));
        }
        Ok(())
    }
}

fn deadline(timeout: Duration) -> Result<Instant, String> {
    if timeout.is_zero() {
        return Err(String::from("worker pipe timeout is zero"));
    }
    Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| String::from("worker pipe deadline overflow"))
}

fn wait_for(deadline: Instant) -> Result<(), String> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(String::from("worker pipe deadline expired"));
    }
    thread::sleep(remaining.min(POLL_INTERVAL));
    Ok(())
}

fn read_once(handle: HANDLE, buffer: &mut [u8]) -> Result<Option<usize>, String> {
    let count = u32::try_from(buffer.len())
        .map_err(|_| String::from("worker pipe read buffer exceeds Win32 bounds"))?;
    let mut read = 0_u32;
    if unsafe {
        windows_sys::Win32::Storage::FileSystem::ReadFile(
            handle,
            buffer.as_mut_ptr().cast(),
            count,
            &raw mut read,
            std::ptr::null_mut(),
        )
    } != 0
    {
        let read =
            usize::try_from(read).map_err(|_| String::from("worker pipe read count overflow"))?;
        if read > buffer.len() {
            return Err(String::from("worker pipe read count is invalid"));
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
    if code == ERROR_MORE_DATA {
        return Err(String::from(
            "worker pipe returned an unexpected message boundary",
        ));
    }
    Err(crate::handle::win32_error("ReadFile(worker)", code))
}

fn write_once(handle: HANDLE, buffer: &[u8]) -> Result<Option<usize>, String> {
    let count = u32::try_from(buffer.len())
        .map_err(|_| String::from("worker pipe write buffer exceeds Win32 bounds"))?;
    let mut written = 0_u32;
    if unsafe {
        windows_sys::Win32::Storage::FileSystem::WriteFile(
            handle,
            buffer.as_ptr().cast(),
            count,
            &raw mut written,
            std::ptr::null_mut(),
        )
    } != 0
    {
        let written = usize::try_from(written)
            .map_err(|_| String::from("worker pipe write count overflow"))?;
        if written > buffer.len() {
            return Err(String::from("worker pipe write count is invalid"));
        }
        return Ok(Some(written));
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
    Err(crate::handle::win32_error("WriteFile(worker)", code))
}
