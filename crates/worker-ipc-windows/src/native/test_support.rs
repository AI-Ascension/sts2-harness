// SPDX-License-Identifier: MIT
#![allow(unsafe_code)]

//! Finite Windows peer and protected-fixture helpers for native tests.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::ptr::{addr_of_mut, null, null_mut};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};
use windows_sys::Win32::Foundation::{FALSE, GENERIC_READ, GENERIC_WRITE};
use windows_sys::Win32::Security::Authorization::{SE_FILE_OBJECT, SetSecurityInfo};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACL, DACL_SECURITY_INFORMATION, GetAce, PROTECTED_DACL_SECURITY_INFORMATION,
};
use windows_sys::Win32::Storage::FileSystem::{
    CREATE_ALWAYS, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_SHARE_READ,
    FILE_SHARE_WRITE, WRITE_DAC, WriteFile,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

use super::process::{process_creation, process_sid, query_process_image};
use super::resources::{Handle, ProtectedCredential};
use super::security::PipeSecurity;
use crate::{EndpointPolicy, ExpectedPeer, Sid, TransportError};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

pub(super) struct Fixture {
    pub(super) directory: PathBuf,
    pub(super) credential: PathBuf,
    pub(super) worker_sid: String,
    pub(super) image: PathBuf,
    pub(super) image_sha256: [u8; 32],
}

impl Fixture {
    pub(super) fn new() -> Result<Self, TransportError> {
        let worker_sid = sid_text_for_test(&process_sid(unsafe { GetCurrentProcess() })?)?;
        let image = query_process_image(unsafe { GetCurrentProcess() })?;
        let bytes = fs::read(&image).map_err(|_| TransportError::Os)?;
        let image_sha256: [u8; 32] = Sha256::digest(bytes).into();
        let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "ascension-worker-ipc-{pid}-{id}",
            pid = std::process::id()
        ));
        fs::create_dir_all(&directory).map_err(|_| TransportError::Os)?;
        let credential = directory.join("credential.txt");
        create_protected_credential(&credential, &worker_sid, b"native-test-secret")?;
        Ok(Self {
            directory,
            credential,
            worker_sid,
            image,
            image_sha256,
        })
    }

    pub(super) fn policy_for_current(
        &self,
        endpoint_nonce: [u8; 16],
    ) -> Result<EndpointPolicy, TransportError> {
        let process = unsafe { GetCurrentProcess() };
        let creation = process_creation(process)?;
        let peer = ExpectedPeer::new(
            Sid::new(self.worker_sid.clone())?,
            std::process::id(),
            creation,
            self.image.clone(),
            self.image_sha256,
            endpoint_nonce,
        )?;
        EndpointPolicy::new(
            Sid::new(self.worker_sid.clone())?,
            peer,
            self.credential.clone(),
        )
    }

    pub(super) fn policy_for_child(
        &self,
        child: &Child,
        endpoint_nonce: [u8; 16],
    ) -> Result<EndpointPolicy, TransportError> {
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, child.id()) };
        let process = Handle::new(process).map_err(|_| TransportError::Os)?;
        let creation = process_creation(process.raw())?;
        let sid = sid_text_for_test(&process_sid(process.raw())?)?;
        let image = query_process_image(process.raw())?;
        self.policy_for_child_parts(
            child.id(),
            endpoint_nonce,
            sid,
            creation,
            image,
            self.image_sha256,
        )
    }

    pub(super) fn policy_for_child_parts(
        &self,
        pid: u32,
        endpoint_nonce: [u8; 16],
        sid: String,
        creation: u64,
        image: PathBuf,
        image_sha256: [u8; 32],
    ) -> Result<EndpointPolicy, TransportError> {
        let peer = ExpectedPeer::new(
            Sid::new(sid)?,
            pid,
            creation,
            image,
            image_sha256,
            endpoint_nonce,
        )?;
        EndpointPolicy::new(
            Sid::new(self.worker_sid.clone())?,
            peer,
            self.credential.clone(),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

pub(super) fn sid_text_for_test(bytes: &[u8]) -> Result<String, TransportError> {
    if bytes.len() < 8 {
        return Err(TransportError::Identity);
    }
    let count = usize::from(bytes[1]);
    let total = 8_usize
        .checked_add(count.checked_mul(4).ok_or(TransportError::Identity)?)
        .ok_or(TransportError::Identity)?;
    if total > bytes.len() {
        return Err(TransportError::Identity);
    }
    let authority = bytes[2..8]
        .iter()
        .fold(0_u64, |value, byte| (value << 8) | u64::from(*byte));
    let mut text = format!("S-{}-{authority}", bytes[0]);
    for index in 0..count {
        let offset = 8 + index * 4;
        let part = u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]);
        text.push_str(&format!("-{part}"));
    }
    Ok(text)
}

fn create_protected_credential(
    path: &Path,
    worker_sid: &str,
    secret: &[u8],
) -> Result<(), TransportError> {
    let path_text = path.to_str().ok_or(TransportError::Configuration)?;
    let wide = super::wide_string(path_text)?;
    let raw = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ | GENERIC_WRITE | WRITE_DAC,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null(),
            CREATE_ALWAYS,
            FILE_ATTRIBUTE_NORMAL,
            null_mut(),
        )
    };
    let file = Handle::new(raw).map_err(|_| TransportError::Os)?;
    let length = u32::try_from(secret.len()).map_err(|_| TransportError::Configuration)?;
    let mut written = 0_u32;
    if unsafe {
        WriteFile(
            file.raw(),
            secret.as_ptr(),
            length,
            addr_of_mut!(written),
            null_mut(),
        )
    } == FALSE
        || written != length
    {
        return Err(TransportError::Os);
    }

    let mut security = PipeSecurity::new(worker_sid, worker_sid)?;
    let acl = security.acl.as_mut_ptr().cast::<ACL>();
    let mut ace = null_mut();
    if unsafe { GetAce(acl, 0, addr_of_mut!(ace)) } == FALSE || ace.is_null() {
        return Err(TransportError::Os);
    }
    unsafe {
        (*ace.cast::<ACCESS_ALLOWED_ACE>()).Mask = FILE_GENERIC_READ;
    }
    security.descriptor.Dacl = acl;
    let security_result = unsafe {
        SetSecurityInfo(
            file.raw(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            acl,
            null_mut(),
        )
    };
    if security_result != 0 {
        return Err(TransportError::Os);
    }
    drop(file);
    let credential = ProtectedCredential::open(path, worker_sid)?;
    drop(credential);
    Ok(())
}

pub(super) fn nonce(seed: u64) -> [u8; 16] {
    let mut value = [0_u8; 16];
    value[..8].copy_from_slice(&seed.to_be_bytes());
    value[8..].copy_from_slice(&(!seed).to_be_bytes());
    value[6] = (value[6] & 0x0f) | 0x40;
    value[8] = (value[8] & 0x3f) | 0x80;
    value
}

pub(super) fn pipe_name(endpoint_nonce: [u8; 16]) -> String {
    format!(
        r"\\.\pipe\ascension-worker-{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        u32::from_be_bytes([
            endpoint_nonce[0],
            endpoint_nonce[1],
            endpoint_nonce[2],
            endpoint_nonce[3],
        ]),
        u16::from_be_bytes([endpoint_nonce[4], endpoint_nonce[5]]),
        u16::from_be_bytes([endpoint_nonce[6], endpoint_nonce[7]]),
        endpoint_nonce[8],
        endpoint_nonce[9],
        endpoint_nonce[10],
        endpoint_nonce[11],
        endpoint_nonce[12],
        endpoint_nonce[13],
        endpoint_nonce[14],
        endpoint_nonce[15]
    )
}
