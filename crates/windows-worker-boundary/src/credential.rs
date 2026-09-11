// SPDX-License-Identifier: MIT

use crate::handle::{NativeHandle, last_error, wide_path};
use crate::identity::current_user_sid;
use crate::image::MAX_PATH_BYTES;
use std::ffi::c_void;
use std::path::{Component, Path};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{GENERIC_READ, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION, AclSizeInformation, DACL_SECURITY_INFORMATION,
    GetAce, GetAclInformation, GetLengthSid, GetSecurityDescriptorControl, IsValidSid,
    OWNER_SECURITY_INFORMATION, SE_DACL_PROTECTED,
};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_SHARE_READ, GetFileInformationByHandle, OPEN_EXISTING, ReadFile,
};
use zeroize::Zeroizing;

const MAX_CREDENTIAL_BYTES: usize = 4 * 1024;

/// Read one owner-only credential through a held Windows file handle.
///
/// The handle denies later write/delete opens while it is active. The DACL is
/// required to be protected and to contain exactly one non-inherited allow
/// ACE for the current user; no inherited or additional principal is accepted.
pub fn read_protected_credential(
    path: &Path,
    maximum: usize,
) -> Result<Zeroizing<Vec<u8>>, String> {
    if maximum == 0 || maximum > MAX_CREDENTIAL_BYTES {
        return Err(String::from("worker credential bound is invalid"));
    }
    validate_reference(path)?;
    let wide = wide_path(path, "worker credential")?;
    let raw = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ | windows_sys::Win32::Storage::FileSystem::READ_CONTROL,
            FILE_SHARE_READ,
            null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
            null_mut(),
        )
    };
    let file = NativeHandle::new(raw, "CreateFileW(worker credential)")?;
    validate_regular_file(file.raw(), "worker credential")?;
    validate_acl(file.raw())?;
    read_file(file.raw(), maximum)
}

fn validate_reference(path: &Path) -> Result<(), String> {
    let value = path
        .to_str()
        .ok_or_else(|| String::from("worker credential path is not Unicode"))?;
    let bytes = value.as_bytes();
    if value.len() < 3
        || value.len() > MAX_PATH_BYTES
        || !path.is_absolute()
        || bytes[1] != b':'
        || bytes[2] != b'\\'
        || bytes[2..].contains(&b':')
        || value.contains('\0')
        || path.file_name().is_none()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(String::from("worker credential path is invalid"));
    }
    Ok(())
}

fn validate_regular_file(handle: HANDLE, label: &str) -> Result<(), String> {
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    if unsafe { GetFileInformationByHandle(handle, &raw mut information) } == 0 {
        return Err(last_error("GetFileInformationByHandle(worker credential)"));
    }
    if information.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) != 0
    {
        return Err(format!("{label} is not a regular non-reparse file"));
    }
    Ok(())
}

fn validate_acl(file: HANDLE) -> Result<(), String> {
    let mut owner = null_mut();
    let mut dacl = null_mut();
    let mut descriptor = null_mut();
    let status = unsafe {
        GetSecurityInfo(
            file,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &raw mut owner,
            null_mut(),
            &raw mut dacl,
            null_mut(),
            &raw mut descriptor,
        )
    };
    if status != 0 {
        return Err(crate::handle::win32_error(
            "GetSecurityInfo(worker credential)",
            status,
        ));
    }
    let result = validate_acl_pointers(owner, dacl, descriptor);
    if !descriptor.is_null() {
        let _ = unsafe { LocalFree(descriptor) };
    }
    result
}

fn validate_acl_pointers(
    owner: *mut c_void,
    dacl: *mut ACL,
    descriptor: *mut c_void,
) -> Result<(), String> {
    let owner_sid = validate_owner(owner)?;
    let (dacl_start, dacl_end) = dacl_bounds(dacl, descriptor)?;
    validate_single_allow_ace(dacl, dacl_start, dacl_end, &owner_sid)
}

fn validate_owner(owner: *mut c_void) -> Result<String, String> {
    if owner.is_null() || unsafe { IsValidSid(owner) } == 0 {
        return Err(String::from("worker credential owner SID is invalid"));
    }
    let owner_sid = sid_string(owner)?;
    if owner_sid != current_user_sid()? {
        return Err(String::from(
            "worker credential owner SID is not current user",
        ));
    }
    Ok(owner_sid)
}

fn dacl_bounds(dacl: *mut ACL, descriptor: *mut c_void) -> Result<(usize, usize), String> {
    if descriptor.is_null() || dacl.is_null() {
        return Err(String::from("worker credential DACL is missing"));
    }
    let mut control = 0_u16;
    let mut revision = 0_u32;
    if unsafe { GetSecurityDescriptorControl(descriptor, &raw mut control, &raw mut revision) } == 0
        || control & SE_DACL_PROTECTED == 0
    {
        return Err(String::from("worker credential DACL is not protected"));
    }
    let mut information = ACL_SIZE_INFORMATION::default();
    if unsafe {
        GetAclInformation(
            dacl,
            (&raw mut information).cast(),
            u32::try_from(std::mem::size_of::<ACL_SIZE_INFORMATION>())
                .map_err(|_| String::from("worker credential ACL size overflow"))?,
            AclSizeInformation,
        )
    } == 0
    {
        return Err(last_error("GetAclInformation(worker credential)"));
    }
    if information.AceCount != 1 {
        return Err(String::from(
            "worker credential DACL must contain one allow ACE",
        ));
    }
    let dacl_start = dacl.cast::<u8>() as usize;
    let dacl_size = usize::from(unsafe { (*dacl).AclSize });
    let dacl_used = usize::try_from(information.AclBytesInUse)
        .map_err(|_| String::from("worker credential ACL byte count overflow"))?;
    if dacl_size < std::mem::size_of::<ACL>() || dacl_used > dacl_size {
        return Err(String::from("worker credential ACL bounds are invalid"));
    }
    let dacl_end = dacl_start
        .checked_add(dacl_used)
        .ok_or_else(|| String::from("worker credential ACL address overflow"))?;
    Ok((dacl_start, dacl_end))
}

fn validate_single_allow_ace(
    dacl: *mut ACL,
    dacl_start: usize,
    dacl_end: usize,
    owner_sid: &str,
) -> Result<(), String> {
    let mut raw_ace = null_mut();
    if unsafe { GetAce(dacl, 0, &raw mut raw_ace) } == 0 || raw_ace.is_null() {
        return Err(last_error("GetAce(worker credential)"));
    }
    let ace_start = raw_ace as usize;
    let header_size = std::mem::size_of::<windows_sys::Win32::Security::ACE_HEADER>();
    let header_end = ace_start
        .checked_add(header_size)
        .ok_or_else(|| String::from("worker credential ACE address overflow"))?;
    if ace_start < dacl_start || header_end > dacl_end {
        return Err(String::from("worker credential ACE is outside its ACL"));
    }
    let header = unsafe {
        std::ptr::read_unaligned(raw_ace.cast::<windows_sys::Win32::Security::ACE_HEADER>())
    };
    let ace_size = usize::from(header.AceSize);
    let ace_end = ace_start
        .checked_add(ace_size)
        .ok_or_else(|| String::from("worker credential ACE size overflow"))?;
    let sid_offset = std::mem::offset_of!(ACCESS_ALLOWED_ACE, SidStart);
    if header.AceType != 0
        || u32::from(header.AceFlags) & windows_sys::Win32::Security::INHERITED_ACE != 0
        || ace_size < sid_offset.saturating_add(std::mem::size_of::<u32>())
        || ace_end > dacl_end
    {
        return Err(String::from(
            "worker credential DACL contains an invalid ACE",
        ));
    }
    let sid = unsafe { raw_ace.cast::<u8>().add(sid_offset).cast::<c_void>() };
    let sid_length = usize::try_from(unsafe { GetLengthSid(sid) })
        .map_err(|_| String::from("worker credential ACE SID length overflow"))?;
    if sid_length == 0
        || sid_length > ace_size.saturating_sub(sid_offset)
        || unsafe { IsValidSid(sid) } == 0
        || sid_string(sid)? != owner_sid
    {
        return Err(String::from(
            "worker credential DACL grants a different SID",
        ));
    }
    Ok(())
}

fn sid_string(sid: *mut c_void) -> Result<String, String> {
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    let mut string_sid = null_mut();
    if unsafe { ConvertSidToStringSidW(sid, &raw mut string_sid) } == 0 || string_sid.is_null() {
        return Err(last_error("ConvertSidToStringSidW(worker credential)"));
    }
    let result = unsafe {
        let mut length = 0_usize;
        while *string_sid.add(length) != 0 {
            length = length.saturating_add(1);
            if length > 184 {
                let _ = LocalFree(string_sid.cast());
                return Err(String::from("worker credential SID exceeds its bound"));
            }
        }
        let result = String::from_utf16(std::slice::from_raw_parts(string_sid, length))
            .map_err(|_| String::from("worker credential SID is not UTF-16"));
        let _ = LocalFree(string_sid.cast());
        result
    }?;
    Ok(result)
}

fn read_file(file: HANDLE, maximum: usize) -> Result<Zeroizing<Vec<u8>>, String> {
    let mut output = Zeroizing::new(Vec::with_capacity(maximum.min(1024)));
    let mut buffer = [0_u8; 256];
    loop {
        let mut read = 0_u32;
        if unsafe {
            ReadFile(
                file,
                buffer.as_mut_ptr().cast(),
                u32::try_from(buffer.len())
                    .map_err(|_| String::from("credential buffer overflow"))?,
                &raw mut read,
                null_mut(),
            )
        } == 0
        {
            return Err(last_error("ReadFile(worker credential)"));
        }
        if read == 0 {
            break;
        }
        let read = usize::try_from(read)
            .map_err(|_| String::from("worker credential read count overflow"))?;
        if read > buffer.len() || output.len().saturating_add(read) > maximum {
            return Err(String::from("worker credential exceeds its size bound"));
        }
        output.extend_from_slice(&buffer[..read]);
    }
    if output.is_empty() {
        return Err(String::from("worker credential is empty"));
    }
    Ok(output)
}
