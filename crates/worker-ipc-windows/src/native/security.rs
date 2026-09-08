// SPDX-License-Identifier: MIT
#![allow(unsafe_code)]

use std::mem::size_of;
use std::ptr::{addr_of, addr_of_mut, null_mut};

use windows_sys::Win32::Foundation::{FALSE, HANDLE, LocalFree, TRUE};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSidToSidW, GetSecurityInfo, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACL, ACL_REVISION, DACL_SECURITY_INFORMATION, EqualSid, GetAce,
    GetLengthSid, GetSecurityDescriptorControl, GetSecurityDescriptorDacl, InitializeAcl,
    InitializeSecurityDescriptor, IsValidSid, PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED,
    SECURITY_ATTRIBUTES, SECURITY_DESCRIPTOR, SetSecurityDescriptorControl,
    SetSecurityDescriptorDacl,
};

use super::wide_string;
use super::{
    ACE_ALLOW, ACE_DENY, FILE_APPEND_DATA, FILE_DELETE, FILE_WRITE_ATTRIBUTES, FILE_WRITE_DAC,
    FILE_WRITE_DATA, FILE_WRITE_EA, FILE_WRITE_OWNER, GENERIC_ALL, GENERIC_WRITE, INHERITED_ACE,
};
use crate::transport::TransportError;
/// Explicit, non-inherited DACL that grants only the two configured SIDs.
pub(super) struct PipeSecurity {
    pub(super) descriptor: Box<SECURITY_DESCRIPTOR>,
    pub(super) acl: Vec<u32>,
    pub(super) sids: Vec<Vec<u8>>,
}

// The descriptor's raw pointers borrow the separately allocated ACL/SID
// buffers owned by this same value.  Moving the value does not move those
// allocations, and the listener mutates neither the descriptor nor its
// buffers after construction.  This permits a listener to be moved to the
// thread that owns accept/shutdown while keeping the raw FFI pointers private.
unsafe impl Send for PipeSecurity {}

impl PipeSecurity {
    pub(super) fn new(worker_sid: &str, peer_sid: &str) -> Result<Self, TransportError> {
        let worker = sid_from_text(worker_sid)?;
        let peer = sid_from_text(peer_sid)?;
        let mut sids = vec![worker];
        if sids[0] != peer {
            sids.push(peer);
        }
        let acl_size = size_of::<ACL>()
            + sids
                .iter()
                .map(|sid| size_of::<ACCESS_ALLOWED_ACE>() - size_of::<u32>() + sid.len())
                .sum::<usize>();
        let acl_words = acl_size.div_ceil(size_of::<u32>());
        let mut acl = vec![0_u32; acl_words];
        let acl_ptr = acl.as_mut_ptr().cast::<ACL>();
        let acl_len = u32::try_from(
            acl.len()
                .checked_mul(size_of::<u32>())
                .ok_or(TransportError::Configuration)?,
        )
        .map_err(|_| TransportError::Configuration)?;
        if unsafe { InitializeAcl(acl_ptr, acl_len, ACL_REVISION) } == FALSE {
            return Err(TransportError::Os);
        }
        for sid in &sids {
            let sid_ptr = sid.as_ptr().cast_mut().cast();
            if unsafe {
                windows_sys::Win32::Security::AddAccessAllowedAceEx(
                    acl_ptr,
                    ACL_REVISION,
                    0,
                    windows_sys::Win32::Foundation::GENERIC_READ
                        | windows_sys::Win32::Foundation::GENERIC_WRITE,
                    sid_ptr,
                )
            } == FALSE
            {
                return Err(TransportError::Os);
            }
        }
        let descriptor = Box::new(SECURITY_DESCRIPTOR::default());
        if unsafe { InitializeSecurityDescriptor(addr_of!(*descriptor).cast_mut().cast(), 1) }
            == FALSE
        {
            return Err(TransportError::Os);
        }
        if unsafe {
            SetSecurityDescriptorDacl(
                addr_of!(*descriptor).cast_mut().cast(),
                TRUE,
                acl_ptr,
                FALSE,
            )
        } == FALSE
        {
            return Err(TransportError::Os);
        }
        if unsafe {
            SetSecurityDescriptorControl(
                addr_of!(*descriptor).cast_mut().cast(),
                SE_DACL_PROTECTED,
                SE_DACL_PROTECTED,
            )
        } == FALSE
        {
            return Err(TransportError::Os);
        }
        Ok(Self {
            descriptor,
            acl,
            sids,
        })
    }

    pub(super) fn attributes(&mut self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: addr_of_mut!(*self.descriptor).cast(),
            bInheritHandle: FALSE,
        }
    }
}
pub(super) fn validate_private_acl(handle: HANDLE, worker_sid: &str) -> Result<(), TransportError> {
    let expected = sid_from_text(worker_sid)?;
    let mut owner: PSID = null_mut();
    let mut group: PSID = null_mut();
    let mut dacl: *mut ACL = null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
    let result = unsafe {
        GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            windows_sys::Win32::Security::OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            addr_of_mut!(owner),
            addr_of_mut!(group),
            addr_of_mut!(dacl),
            null_mut(),
            addr_of_mut!(descriptor),
        )
    };
    if result != 0 || descriptor.is_null() || owner.is_null() {
        free_security_descriptor(descriptor);
        return Err(TransportError::Credential);
    }
    let valid = validate_acl_parts(descriptor, dacl, owner, &expected);
    free_security_descriptor(descriptor);
    valid
}

fn validate_acl_parts(
    descriptor: PSECURITY_DESCRIPTOR,
    dacl: *mut ACL,
    owner: PSID,
    expected: &[u8],
) -> Result<(), TransportError> {
    let mut control = 0_u16;
    let mut revision = 0_u32;
    if unsafe {
        GetSecurityDescriptorControl(descriptor, addr_of_mut!(control), addr_of_mut!(revision))
    } == FALSE
        || control & SE_DACL_PROTECTED == 0
    {
        return Err(TransportError::Credential);
    }
    let mut present = FALSE;
    let mut defaulted = FALSE;
    let mut queried_dacl = null_mut();
    if unsafe {
        GetSecurityDescriptorDacl(
            descriptor,
            addr_of_mut!(present),
            addr_of_mut!(queried_dacl),
            addr_of_mut!(defaulted),
        )
    } == FALSE
        || present == FALSE
        || defaulted != FALSE
        || dacl.is_null()
        || queried_dacl != dacl
    {
        return Err(TransportError::Credential);
    }
    if unsafe { EqualSid(owner, expected.as_ptr().cast_mut().cast()) } == FALSE {
        return Err(TransportError::Credential);
    }
    let count = unsafe { (*dacl).AceCount };
    if count == 0 {
        return Err(TransportError::Credential);
    }
    let mut found_owner = false;
    for index in 0..u32::from(count) {
        let mut ace = null_mut();
        if unsafe { GetAce(dacl, index, addr_of_mut!(ace)) } == FALSE || ace.is_null() {
            return Err(TransportError::Credential);
        }
        let header = unsafe { &*ace.cast::<windows_sys::Win32::Security::ACE_HEADER>() };
        if header.AceFlags & INHERITED_ACE != 0 || header.AceFlags != 0 {
            return Err(TransportError::Credential);
        }
        if header.AceType == ACE_ALLOW {
            let allowed = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
            if allowed.Mask
                & (GENERIC_WRITE
                    | GENERIC_ALL
                    | FILE_WRITE_DATA
                    | FILE_APPEND_DATA
                    | FILE_WRITE_EA
                    | FILE_WRITE_ATTRIBUTES
                    | FILE_DELETE
                    | FILE_WRITE_DAC
                    | FILE_WRITE_OWNER)
                != 0
            {
                return Err(TransportError::Credential);
            }
            let sid = addr_of!(allowed.SidStart).cast_mut().cast();
            if unsafe { EqualSid(sid, expected.as_ptr().cast_mut().cast()) } != FALSE {
                found_owner = true;
            } else {
                // A credential may be readable by the owner only.  Broad or
                // unrelated SIDs are rejected even when their mask is read.
                return Err(TransportError::Credential);
            }
        } else if header.AceType == ACE_DENY {
            let denied = unsafe { &*ace.cast::<windows_sys::Win32::Security::ACCESS_DENIED_ACE>() };
            let sid = addr_of!(denied.SidStart).cast_mut().cast();
            if unsafe { EqualSid(sid, expected.as_ptr().cast_mut().cast()) } == FALSE {
                return Err(TransportError::Credential);
            }
        } else {
            return Err(TransportError::Credential);
        }
    }
    if found_owner {
        Ok(())
    } else {
        Err(TransportError::Credential)
    }
}

fn free_security_descriptor(descriptor: PSECURITY_DESCRIPTOR) {
    if !descriptor.is_null() {
        unsafe {
            let _ = LocalFree(descriptor.cast());
        }
    }
}

pub(super) fn sid_from_text(value: &str) -> Result<Vec<u8>, TransportError> {
    let wide = wide_string(value)?;
    let mut sid: PSID = null_mut();
    if unsafe { ConvertStringSidToSidW(wide.as_ptr(), addr_of_mut!(sid)) } == FALSE
        || sid.is_null()
        || unsafe { IsValidSid(sid) } == FALSE
    {
        if !sid.is_null() {
            unsafe {
                let _ = LocalFree(sid.cast());
            }
        }
        return Err(TransportError::Configuration);
    }
    let length = unsafe { GetLengthSid(sid) } as usize;
    if length == 0 || length > 68 {
        unsafe {
            let _ = LocalFree(sid.cast());
        }
        return Err(TransportError::Configuration);
    }
    let mut bytes = vec![0_u8; length];
    unsafe {
        std::ptr::copy_nonoverlapping(sid.cast::<u8>(), bytes.as_mut_ptr(), length);
        let _ = LocalFree(sid.cast());
    }
    Ok(bytes)
}
