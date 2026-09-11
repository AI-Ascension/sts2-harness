// SPDX-License-Identifier: MIT

use crate::handle::{NativeHandle, last_error};
use crate::image::{validate_digest, validate_windows_path};
use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{FILETIME, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows_sys::Win32::Security::{
    GetLengthSid, GetTokenInformation, IsValidSid, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetProcessTimes, OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, QueryFullProcessImageNameW,
};

pub(crate) const MAX_SID_BYTES: usize = 184;

#[derive(Clone, Eq, PartialEq)]
pub struct PeerExpectation {
    pub(crate) pid: u32,
    pub(crate) creation_time_100ns: u64,
    pub(crate) executable: PathBuf,
    pub(crate) executable_sha256: String,
    pub(crate) session_id: u32,
    pub(crate) sid: String,
}

impl std::fmt::Debug for PeerExpectation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PeerExpectation")
            .field("pid", &self.pid)
            .field("creation_time_100ns", &self.creation_time_100ns)
            .field("executable", &"<protected-reference>")
            .field("executable_sha256", &self.executable_sha256)
            .field("session_id", &self.session_id)
            .field("sid", &"<protected-policy>")
            .finish()
    }
}

impl PeerExpectation {
    pub fn new(
        pid: u32,
        creation_time_100ns: u64,
        executable: impl Into<PathBuf>,
        executable_sha256: impl Into<String>,
        session_id: u32,
        sid: impl Into<String>,
    ) -> Result<Self, String> {
        let expectation = Self {
            pid,
            creation_time_100ns,
            executable: executable.into(),
            executable_sha256: executable_sha256.into(),
            session_id,
            sid: sid.into(),
        };
        validate_expectation(&expectation)?;
        Ok(expectation)
    }
}

pub(crate) struct ProcessIdentity {
    pub(crate) handle: NativeHandle,
    pub(crate) creation_time_100ns: u64,
    pub(crate) executable: PathBuf,
    pub(crate) sid: String,
}

pub(crate) fn open_process_identity(pid: u32, _session_id: u32) -> Result<ProcessIdentity, String> {
    if pid == 0 {
        return Err(String::from("worker peer PID is invalid"));
    }
    let raw = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            pid,
        )
    };
    let handle = NativeHandle::new(raw, "OpenProcess(worker peer)")?;
    let creation_time_100ns = process_creation_time(handle.raw())?;
    let executable = query_image_path(handle.raw())?;
    let sid = process_user_sid(handle.raw())?;
    Ok(ProcessIdentity {
        handle,
        creation_time_100ns,
        executable,
        sid,
    })
}

pub(crate) fn current_user_sid() -> Result<String, String> {
    let mut raw = null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut raw) } == 0 {
        return Err(last_error("OpenProcessToken(current user)"));
    }
    let token = NativeHandle::new(raw, "OpenProcessToken(current user)")?;
    process_user_sid(token.raw())
}

pub(crate) fn process_user_sid(process: HANDLE) -> Result<String, String> {
    let mut token = null_mut();
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &raw mut token) } == 0 {
        return Err(last_error("OpenProcessToken(worker peer)"));
    }
    let token = NativeHandle::new(token, "OpenProcessToken(worker peer)")?;
    token_user_sid(token.raw())
}

fn token_user_sid(token: HANDLE) -> Result<String, String> {
    let mut required = 0_u32;
    let _ = unsafe { GetTokenInformation(token, TokenUser, null_mut(), 0, &raw mut required) };
    let required =
        usize::try_from(required).map_err(|_| String::from("worker SID size overflow"))?;
    if required == 0 || required > MAX_SID_BYTES.saturating_mul(4) {
        return Err(last_error("GetTokenInformation(size)"));
    }
    let mut bytes = vec![0_u8; required];
    let mut returned =
        u32::try_from(bytes.len()).map_err(|_| String::from("worker SID buffer size overflow"))?;
    if unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            bytes.as_mut_ptr().cast(),
            returned,
            &raw mut returned,
        )
    } == 0
    {
        return Err(last_error("GetTokenInformation(TokenUser)"));
    }
    let user = unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast::<TOKEN_USER>()) };
    let sid = user.User.Sid;
    if sid.is_null() || unsafe { IsValidSid(sid) } == 0 {
        return Err(String::from("worker peer SID is invalid"));
    }
    sid_string(sid)
}

pub(crate) fn validate_sid(sid: &str) -> Result<(), String> {
    let mut parts = sid.split('-');
    if parts.next() != Some("S") || parts.next() != Some("1") {
        return Err(String::from("worker peer SID is invalid"));
    }
    let authority = parts
        .next()
        .filter(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        .ok_or_else(|| String::from("worker peer SID is invalid"))?;
    if authority
        .parse::<u64>()
        .ok()
        .as_ref()
        .is_none_or(|value| *value >= (1_u64 << 48))
    {
        return Err(String::from("worker peer SID is invalid"));
    }
    let subauthorities = parts.collect::<Vec<_>>();
    if subauthorities.is_empty()
        || subauthorities.len() > 15
        || subauthorities.iter().any(|part| {
            part.is_empty() || part.len() > 10 || !part.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return Err(String::from("worker peer SID is invalid"));
    }
    Ok(())
}

fn sid_string(sid: *mut c_void) -> Result<String, String> {
    let length = usize::try_from(unsafe { GetLengthSid(sid) })
        .map_err(|_| String::from("worker SID length overflow"))?;
    if length == 0 || length > MAX_SID_BYTES {
        return Err(String::from("worker SID exceeds its size bound"));
    }
    let mut string_sid = null_mut();
    if unsafe { ConvertSidToStringSidW(sid, &raw mut string_sid) } == 0 || string_sid.is_null() {
        return Err(last_error("ConvertSidToStringSidW"));
    }
    let result = unsafe {
        let mut length = 0_usize;
        while *string_sid.add(length) != 0 {
            length = length.saturating_add(1);
            if length > MAX_SID_BYTES {
                let _ = LocalFree(string_sid.cast());
                return Err(String::from("worker SID string exceeds its size bound"));
            }
        }
        let result = String::from_utf16(std::slice::from_raw_parts(string_sid, length))
            .map_err(|_| String::from("worker SID is not valid UTF-16"));
        let _ = LocalFree(string_sid.cast());
        result
    }?;
    validate_sid(&result)?;
    Ok(result)
}

pub(crate) fn process_creation_time(process: HANDLE) -> Result<u64, String> {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    if unsafe {
        GetProcessTimes(
            process,
            &raw mut creation,
            &raw mut exit,
            &raw mut kernel,
            &raw mut user,
        )
    } == 0
    {
        return Err(last_error("GetProcessTimes(worker peer)"));
    }
    Ok((u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime))
}

pub(crate) fn query_image_path(process: HANDLE) -> Result<PathBuf, String> {
    let mut buffer = vec![0_u16; 32_768];
    let mut length = u32::try_from(buffer.len())
        .map_err(|_| String::from("worker image path buffer overflow"))?;
    if unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            buffer.as_mut_ptr(),
            &raw mut length,
        )
    } == 0
    {
        return Err(last_error("QueryFullProcessImageNameW(worker peer)"));
    }
    buffer.truncate(
        usize::try_from(length).map_err(|_| String::from("worker image path length overflow"))?,
    );
    if buffer.is_empty() {
        return Err(String::from("worker image path is empty"));
    }
    String::from_utf16(&buffer)
        .map(PathBuf::from)
        .map_err(|_| String::from("worker image path is not valid UTF-16"))
}

fn validate_expectation(expectation: &PeerExpectation) -> Result<(), String> {
    if expectation.pid == 0 || expectation.creation_time_100ns == 0 {
        return Err(String::from("worker peer process identity is invalid"));
    }
    validate_windows_path(&expectation.executable, "worker peer executable")?;
    validate_digest(&expectation.executable_sha256)?;
    validate_sid(&expectation.sid)
}
