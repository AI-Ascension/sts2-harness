// SPDX-License-Identifier: MIT

//! Safe owner-supplied endpoint and launch-record policy types.

#![forbid(unsafe_code)]

use std::path::{Component, Path, PathBuf};

use crate::TransportError;

const PIPE_PREFIX: &str = r"\\.\pipe\ascension-worker-";

/// A canonical Windows account SID supplied by the owner launch record.
///
/// The textual SID is retained only as configuration.  Native code converts
/// it to a private binary SID before creating the DACL or checking a peer
/// token; no raw SID pointer crosses this API.
#[derive(Clone, Eq, PartialEq)]
pub struct Sid(String);

impl Sid {
    /// Validates and stores a canonical textual SID.
    pub fn new(value: impl Into<String>) -> Result<Self, TransportError> {
        let value = value.into();
        validate_sid(&value)?;
        Ok(Self(value))
    }

    /// Alias for [`Sid::new`] useful when loading a launch record.
    pub fn from_string(value: &str) -> Result<Self, TransportError> {
        Self::new(value)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Sid {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_tuple("Sid").field(&self.0).finish()
    }
}

/// Immutable identity of the owner-approved watchdog process for one launch.
///
/// `creation_filetime` is the exact unsigned 64-bit value formed from
/// `GetProcessTimes`' creation [`FILETIME`], not a wall-clock estimate.  The
/// image digest is checked against a held file before the listener is ready;
/// the native peer check then opens the kernel-reported image and compares its
/// held file identity.
pub struct ExpectedPeer {
    sid: Sid,
    pid: u32,
    creation_filetime: u64,
    image_path: PathBuf,
    image_sha256: [u8; 32],
    launch_nonce: [u8; 16],
}

impl ExpectedPeer {
    /// Creates one immutable peer expectation from an owner-approved launch
    /// record.  No value is read from a client connection.
    pub fn new(
        sid: Sid,
        pid: u32,
        creation_filetime: u64,
        image_path: PathBuf,
        image_sha256: [u8; 32],
        launch_nonce: [u8; 16],
    ) -> Result<Self, TransportError> {
        if pid == 0 || creation_filetime == 0 || !is_uuid_v4(&launch_nonce) {
            return Err(TransportError::Configuration);
        }
        validate_local_path(&image_path)?;
        Ok(Self {
            sid,
            pid,
            creation_filetime,
            image_path,
            image_sha256,
            launch_nonce,
        })
    }

    pub(crate) fn sid(&self) -> &Sid {
        &self.sid
    }

    pub(crate) fn pid(&self) -> u32 {
        self.pid
    }

    pub(crate) fn creation_filetime(&self) -> u64 {
        self.creation_filetime
    }

    pub(crate) fn image_path(&self) -> &Path {
        &self.image_path
    }

    pub(crate) fn image_sha256(&self) -> &[u8; 32] {
        &self.image_sha256
    }

    pub(crate) fn launch_nonce(&self) -> &[u8; 16] {
        &self.launch_nonce
    }
}

impl std::fmt::Debug for ExpectedPeer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExpectedPeer")
            .field("sid", &self.sid)
            .field("pid", &self.pid)
            .field("creation_filetime", &self.creation_filetime)
            .field("image_path", &"<redacted path>")
            .field("image_sha256", &"<redacted digest>")
            .field("launch_nonce", &self.launch_nonce)
            .finish()
    }
}

/// Owner-local policy consumed exactly once by [`crate::WorkerListener::bind`].
pub struct EndpointPolicy {
    worker_sid: Sid,
    expected_peer: ExpectedPeer,
    credential_path: PathBuf,
    pipe_name: String,
}

impl EndpointPolicy {
    /// Creates the endpoint policy.  The nonce is taken from the immutable
    /// expected peer and is rendered into the fixed local named-pipe prefix.
    pub fn new(
        worker_sid: Sid,
        expected_peer: ExpectedPeer,
        credential_path: PathBuf,
    ) -> Result<Self, TransportError> {
        validate_local_path(&credential_path)?;
        let pipe_name = pipe_name(expected_peer.launch_nonce());
        Ok(Self {
            worker_sid,
            expected_peer,
            credential_path,
            pipe_name,
        })
    }

    /// Returns the owner-generated local endpoint name.  It never contains a
    /// credential or client-provided path.
    pub fn pipe_name(&self) -> &str {
        &self.pipe_name
    }

    pub(crate) fn worker_sid(&self) -> &Sid {
        &self.worker_sid
    }

    pub(crate) fn expected_peer(&self) -> &ExpectedPeer {
        &self.expected_peer
    }

    pub(crate) fn credential_path(&self) -> &Path {
        &self.credential_path
    }
}

impl std::fmt::Debug for EndpointPolicy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EndpointPolicy")
            .field("worker_sid", &self.worker_sid)
            .field("expected_peer", &self.expected_peer)
            .field("credential_path", &"<redacted path>")
            .field("pipe_name", &self.pipe_name)
            .finish()
    }
}

fn validate_sid(value: &str) -> Result<(), TransportError> {
    let mut parts = value.split('-');
    if parts.next() != Some("S") || parts.next() != Some("1") {
        return Err(TransportError::Configuration);
    }
    let mut count = 0_u8;
    for part in parts {
        if part.is_empty()
            || part.len() > 10
            || (part.len() > 1 && part.starts_with('0'))
            || !part.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(TransportError::Configuration);
        }
        count = count.saturating_add(1);
    }
    if count == 0 || count > 15 {
        return Err(TransportError::Configuration);
    }
    Ok(())
}

pub(crate) fn validate_local_path(path: &Path) -> Result<(), TransportError> {
    let value = path.to_str().ok_or(TransportError::Configuration)?;
    if value.is_empty() || value.contains('\0') || value.starts_with("\\\\") {
        return Err(TransportError::Configuration);
    }
    let bytes = value.as_bytes();
    if bytes.len() < 3
        || !bytes[0].is_ascii_alphabetic()
        || bytes[1] != b':'
        || !matches!(bytes[2], b'\\' | b'/')
    {
        return Err(TransportError::Configuration);
    }
    // Path::components normalizes interior `.` segments on Windows. Inspect
    // the original spelling first so normalization cannot bypass this policy.
    if value[2..].contains(':')
        || value[3..]
            .split(['\\', '/'])
            .any(|component| matches!(component, "." | ".."))
    {
        return Err(TransportError::Configuration);
    }
    for component in path.components() {
        match component {
            Component::ParentDir | Component::CurDir => {
                return Err(TransportError::Configuration);
            }
            Component::Prefix(_) | Component::RootDir => {}
            Component::Normal(value) if value.to_str().is_none() => {
                return Err(TransportError::Configuration);
            }
            Component::Normal(_) => {}
        }
    }
    Ok(())
}

fn is_uuid_v4(value: &[u8; 16]) -> bool {
    value[6] & 0xf0 == 0x40 && value[8] & 0xc0 == 0x80
}

fn pipe_name(nonce: &[u8; 16]) -> String {
    format!(
        "{PIPE_PREFIX}{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        u32::from_be_bytes([nonce[0], nonce[1], nonce[2], nonce[3]]),
        u16::from_be_bytes([nonce[4], nonce[5]]),
        u16::from_be_bytes([nonce[6], nonce[7]]),
        nonce[8],
        nonce[9],
        nonce[10],
        nonce[11],
        nonce[12],
        nonce[13],
        nonce[14],
        nonce[15]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sid() -> Sid {
        Sid::new("S-1-5-21-42").unwrap_or_else(|_| unreachable!())
    }

    fn nonce() -> [u8; 16] {
        [
            0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x47, 0x80, 0x90, 0xa0, 0xb0, 0xc0, 0xd0, 0xe0,
            0xf0, 0x01,
        ]
    }

    #[test]
    fn sid_and_nonce_are_closed() {
        assert!(Sid::new("S-1-5-21-42").is_ok());
        assert!(Sid::new("S-1-5-21-x").is_err());
        assert!(Sid::new("S-1").is_err());
        assert!(is_uuid_v4(&nonce()));
        let mut wrong_version = nonce();
        wrong_version[6] = 0x37;
        assert!(!is_uuid_v4(&wrong_version));
    }

    #[test]
    fn path_policy_rejects_reparse_primitives() {
        assert!(validate_local_path(Path::new(r"C:\worker\credential.txt")).is_ok());
        assert!(validate_local_path(Path::new(r"C:\worker\..\credential.txt")).is_err());
        assert!(validate_local_path(Path::new(r"C:\worker\.\credential.txt")).is_err());
        assert!(validate_local_path(Path::new("C:/worker/./credential.txt")).is_err());
        assert!(validate_local_path(Path::new(r"C:\worker/../credential.txt")).is_err());
        assert!(validate_local_path(Path::new(r"C:\worker\.")).is_err());
        assert!(validate_local_path(Path::new(r"C:\worker\credential.txt:stream")).is_err());
        assert!(validate_local_path(Path::new(r"C:\worker\credential.txt")).is_ok());
        assert!(validate_local_path(Path::new(r"\\server\share\credential.txt")).is_err());
        assert!(validate_local_path(Path::new(r"C:worker\credential.txt")).is_err());
    }

    #[test]
    fn endpoint_name_is_nonce_bound_and_path_debug_is_redacted() {
        let peer = ExpectedPeer::new(
            sid(),
            42,
            7,
            PathBuf::from(r"C:\worker\watchdog.exe"),
            [0x11; 32],
            nonce(),
        )
        .unwrap_or_else(|_| unreachable!());
        let policy = EndpointPolicy::new(sid(), peer, PathBuf::from(r"C:\worker\credential.txt"))
            .unwrap_or_else(|_| unreachable!());
        assert!(policy.pipe_name().starts_with(PIPE_PREFIX));
        let debug = format!("{policy:?}");
        assert!(!debug.contains("credential.txt"));
        assert!(!debug.contains("watchdog.exe"));
    }
}
