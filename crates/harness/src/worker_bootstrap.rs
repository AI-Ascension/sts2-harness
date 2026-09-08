// SPDX-License-Identifier: MIT

//! Closed startup policy decoder for the watchdog-owned bootstrap v1 contract.
//! These values describe an expected peer; they are not authentication proof.

use serde::Deserialize;

pub const BOOTSTRAP_MAGIC: &[u8; 8] = b"ASC-WB01";
pub const MAX_BOOTSTRAP_BYTES: usize = 16_384;
const PREFIX_BYTES: usize = 12;

/// Fixed redacted bootstrap failure. Input bytes and paths are never displayed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootstrapError;

impl std::fmt::Display for BootstrapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid worker bootstrap")
    }
}
impl std::error::Error for BootstrapError {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BootstrapWire {
    version: u32,
    launch_nonce: String,
    watchdog_boot_id: String,
    component_id: String,
    expected_peer: ExpectedBootstrapPeer,
}

/// Platform policy data only. A transport must independently authenticate every
/// field against the connected process and retain native lifetime/image proof.
#[derive(Deserialize)]
#[serde(tag = "platform", rename_all = "lowercase", deny_unknown_fields)]
pub enum ExpectedBootstrapPeer {
    Linux {
        pid: u32,
        creation_token: String,
        executable: String,
        executable_sha256: String,
        uid: u32,
        gid: u32,
    },
    Windows {
        pid: u32,
        creation_token: String,
        executable: String,
        executable_sha256: String,
        session_id: u32,
        sid: String,
    },
}

/// Validated immutable startup policy. There is no public deserialization or
/// unchecked constructor for this wrapper.
pub struct WorkerBootstrap(BootstrapWire);

impl WorkerBootstrap {
    /// Decode exactly one complete frame; reject trailing bytes or a second frame.
    pub fn decode(frame: &[u8]) -> Result<Self, BootstrapError> {
        let prefix = frame.get(..PREFIX_BYTES).ok_or(BootstrapError)?;
        if &prefix[..8] != BOOTSTRAP_MAGIC {
            return Err(BootstrapError);
        }
        let length = u32::from_be_bytes(prefix[8..12].try_into().map_err(|_| BootstrapError)?);
        let length = usize::try_from(length).map_err(|_| BootstrapError)?;
        if length == 0 || length > MAX_BOOTSTRAP_BYTES || frame.len() != PREFIX_BYTES + length {
            return Err(BootstrapError);
        }
        let wire: BootstrapWire =
            serde_json::from_slice(&frame[PREFIX_BYTES..]).map_err(|_| BootstrapError)?;
        if wire.version != 1
            || !uuid4(&wire.launch_nonce)
            || !uuid4(&wire.watchdog_boot_id)
            || !component(&wire.component_id)
        {
            return Err(BootstrapError);
        }
        wire.expected_peer.validate()?;
        Ok(Self(wire))
    }

    pub fn launch_nonce(&self) -> &str {
        &self.0.launch_nonce
    }
    pub fn watchdog_boot_id(&self) -> &str {
        &self.0.watchdog_boot_id
    }
    pub fn component_id(&self) -> &str {
        &self.0.component_id
    }
    pub fn expected_peer(&self) -> &ExpectedBootstrapPeer {
        &self.0.expected_peer
    }
}

impl ExpectedBootstrapPeer {
    fn validate(&self) -> Result<(), BootstrapError> {
        let (pid, creation, executable, digest, platform_valid) = match self {
            Self::Linux {
                pid,
                creation_token,
                executable,
                executable_sha256,
                ..
            } => (
                *pid,
                creation_token,
                executable,
                executable_sha256,
                executable.starts_with('/'),
            ),
            Self::Windows {
                pid,
                creation_token,
                executable,
                executable_sha256,
                sid,
                ..
            } => {
                let bytes = executable.as_bytes();
                let absolute = bytes.len() >= 3
                    && bytes[0].is_ascii_alphabetic()
                    && bytes[1] == b':'
                    && bytes[2] == b'\\';
                (
                    *pid,
                    creation_token,
                    executable,
                    executable_sha256,
                    absolute && valid_sid(sid),
                )
            }
        };
        if pid == 0
            || !positive_u64(creation)
            || executable.len() > 4096
            || executable.contains('\0')
            || !platform_valid
            || digest.len() != 64
            || !digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(BootstrapError);
        }
        Ok(())
    }
}

fn positive_u64(value: &str) -> bool {
    value.len() <= 20
        && value.starts_with(|c: char| ('1'..='9').contains(&c))
        && value.bytes().all(|b| b.is_ascii_digit())
        && value.parse::<u64>().is_ok()
}

fn component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !matches!(value, "." | "..")
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

fn uuid4(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| {
        id.get_version_num() == 4
            && id.get_variant() == uuid::Variant::RFC4122
            && id.to_string() == value
    })
}

fn canonical_decimal(value: &str) -> bool {
    !value.is_empty()
        && (value == "0" || !value.starts_with('0'))
        && value.bytes().all(|b| b.is_ascii_digit())
}

fn valid_sid(value: &str) -> bool {
    if value.len() > 184 {
        return false;
    }
    let Some(tail) = value.strip_prefix("S-1-") else {
        return false;
    };
    let mut parts = tail.split('-');
    let Some(authority) = parts.next() else {
        return false;
    };
    if !canonical_decimal(authority)
        || authority
            .parse::<u64>()
            .map_or(true, |n| n > 0xffff_ffff_ffff)
    {
        return false;
    }
    let mut count = 0;
    for part in parts {
        if !canonical_decimal(part) || part.parse::<u32>().is_err() {
            return false;
        }
        count += 1;
    }
    (1..=15).contains(&count)
}

#[cfg(test)]
#[path = "worker_bootstrap_tests.rs"]
mod tests;
