// SPDX-License-Identifier: MIT
#![allow(unsafe_code)]

use std::mem::{size_of, zeroed};
use std::path::PathBuf;
use std::ptr::{addr_of_mut, null_mut};

use windows_sys::Win32::Foundation::{FALSE, FILETIME, HANDLE};
use windows_sys::Win32::Security::GetTokenInformation;
use windows_sys::Win32::Security::{GetLengthSid, IsValidSid, TOKEN_QUERY, TOKEN_USER, TokenUser};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_READ_ATTRIBUTES, FILE_READ_DATA, FILE_SHARE_READ,
};
use windows_sys::Win32::System::Pipes::GetNamedPipeClientProcessId;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, GetProcessTimes, OpenProcess, OpenProcessToken,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};

use super::MAX_PATH_UTF16;
use super::resources::{Handle, HeldImage, ProtectedFile};
use super::security::sid_from_text;
use crate::transport::{Deadline, TransportError};
pub(super) struct VerifiedPeer {
    pub(super) process: Handle,
    pub(super) image: ProtectedFile,
}
pub(super) fn verify_peer(
    pipe: HANDLE,
    expected_sid: &str,
    expected_pid: u32,
    expected_creation_filetime: u64,
    held_image: &HeldImage,
    deadline: Deadline,
) -> Result<VerifiedPeer, TransportError> {
    deadline.check()?;
    let mut pid = 0_u32;
    if unsafe { GetNamedPipeClientProcessId(pipe, addr_of_mut!(pid)) } == FALSE
        || pid == 0
        || pid != expected_pid
    {
        return Err(TransportError::Identity);
    }
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) };
    let process = Handle::new(process).map_err(|_| TransportError::Identity)?;
    deadline.check()?;
    if process_creation(process.raw())? != expected_creation_filetime {
        return Err(TransportError::Identity);
    }
    let expected_sid = sid_from_text(expected_sid).map_err(|_| TransportError::Identity)?;
    let actual_sid = process_sid(process.raw())?;
    if actual_sid != expected_sid {
        return Err(TransportError::Identity);
    }
    process_is_live(process.raw())?;
    deadline.check()?;
    let image_path = query_process_image(process.raw())?;
    let image = ProtectedFile::open(
        &image_path,
        FILE_READ_DATA | FILE_READ_ATTRIBUTES,
        FILE_SHARE_READ,
    )
    .map_err(|_| TransportError::Identity)?;
    if image.identity() != held_image.file.identity() || held_image.file.verify_identity().is_err()
    {
        return Err(TransportError::Identity);
    }
    process_is_live(process.raw())?;
    deadline.check()?;
    Ok(VerifiedPeer { process, image })
}
fn process_sid(process: HANDLE) -> Result<Vec<u8>, TransportError> {
    let mut token_raw = null_mut();
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, addr_of_mut!(token_raw)) } == FALSE {
        return Err(TransportError::Identity);
    }
    let token = Handle::new(token_raw).map_err(|_| TransportError::Identity)?;
    let mut needed = 0_u32;
    let _ =
        unsafe { GetTokenInformation(token.raw(), TokenUser, null_mut(), 0, addr_of_mut!(needed)) };
    if needed == 0 || needed > 4096 {
        return Err(TransportError::Identity);
    }
    let word_count = (needed as usize).div_ceil(size_of::<u64>());
    let mut words = vec![0_u64; word_count];
    if unsafe {
        GetTokenInformation(
            token.raw(),
            TokenUser,
            words.as_mut_ptr().cast(),
            needed,
            addr_of_mut!(needed),
        )
    } == FALSE
    {
        return Err(TransportError::Identity);
    }
    if needed as usize > words.len() * size_of::<u64>() {
        return Err(TransportError::Identity);
    }
    let user = unsafe { &*words.as_ptr().cast::<TOKEN_USER>() };
    if user.User.Sid.is_null() || unsafe { IsValidSid(user.User.Sid) } == FALSE {
        return Err(TransportError::Identity);
    }
    let length = unsafe { GetLengthSid(user.User.Sid) } as usize;
    if length == 0 || length > needed as usize {
        return Err(TransportError::Identity);
    }
    let mut sid = vec![0_u8; length];
    unsafe {
        std::ptr::copy_nonoverlapping(user.User.Sid.cast::<u8>(), sid.as_mut_ptr(), length);
    }
    Ok(sid)
}

fn process_creation(process: HANDLE) -> Result<u64, TransportError> {
    let mut creation: FILETIME = unsafe { zeroed() };
    let mut exit: FILETIME = unsafe { zeroed() };
    let mut kernel: FILETIME = unsafe { zeroed() };
    let mut user: FILETIME = unsafe { zeroed() };
    if unsafe {
        GetProcessTimes(
            process,
            addr_of_mut!(creation),
            addr_of_mut!(exit),
            addr_of_mut!(kernel),
            addr_of_mut!(user),
        )
    } == FALSE
    {
        return Err(TransportError::Identity);
    }
    Ok((u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime))
}

fn query_process_image(process: HANDLE) -> Result<PathBuf, TransportError> {
    let mut buffer = vec![0_u16; MAX_PATH_UTF16];
    let mut length = u32::try_from(buffer.len()).map_err(|_| TransportError::Identity)?;
    if unsafe { QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), addr_of_mut!(length)) }
        == FALSE
        || length == 0
        || usize::try_from(length).map_err(|_| TransportError::Identity)? >= buffer.len()
    {
        return Err(TransportError::Identity);
    }
    buffer.truncate(length as usize);
    String::from_utf16(&buffer)
        .map(PathBuf::from)
        .map_err(|_| TransportError::Identity)
}

fn process_is_live(process: HANDLE) -> Result<(), TransportError> {
    let mut exit = 0_u32;
    if unsafe { GetExitCodeProcess(process, addr_of_mut!(exit)) } == FALSE || exit != 259 {
        Err(TransportError::Identity)
    } else {
        Ok(())
    }
}
pub(super) fn check_worker_sid(worker_sid: &str) -> Result<(), TransportError> {
    let expected = sid_from_text(worker_sid).map_err(|_| TransportError::Identity)?;
    let actual = process_sid(unsafe { GetCurrentProcess() })?;
    if actual == expected {
        Ok(())
    } else {
        Err(TransportError::Identity)
    }
}
