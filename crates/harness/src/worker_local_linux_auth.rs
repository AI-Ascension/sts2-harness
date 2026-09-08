// SPDX-License-Identifier: MIT

//! Protected credential prelude authentication.

#![cfg(target_os = "linux")]

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::net::UnixStream;
use tokio::time::{Instant, timeout_at};
use zeroize::Zeroizing;

use super::worker_local_linux_process::ProtectedCredential;
use super::{AUTH_MAGIC, LinuxTransportError, MAX_AUTH_BODY_BYTES};

pub(super) async fn authenticate_credential(
    stream: &mut UnixStream,
    credential: &ProtectedCredential,
    deadline: Instant,
) -> Result<(), LinuxTransportError> {
    let mut length = [0_u8; 4];
    read_exact_until(stream, &mut length, deadline).await?;
    let body_length =
        usize::try_from(u32::from_be_bytes(length)).map_err(|_| LinuxTransportError::Credential)?;
    if !(AUTH_MAGIC.len() + 1..=MAX_AUTH_BODY_BYTES).contains(&body_length) {
        return Err(LinuxTransportError::Credential);
    }
    let mut body = Zeroizing::new(vec![0_u8; body_length]);
    read_exact_until(stream, body.as_mut_slice(), deadline).await?;
    if body.get(..AUTH_MAGIC.len()) != Some(AUTH_MAGIC.as_slice()) {
        return Err(LinuxTransportError::Credential);
    }
    if !credential.matches(&body[AUTH_MAGIC.len()..]) {
        return Err(LinuxTransportError::Credential);
    }
    Ok(())
}

async fn read_exact_until<R: AsyncRead + Unpin>(
    reader: &mut R,
    bytes: &mut [u8],
    deadline: Instant,
) -> Result<(), LinuxTransportError> {
    timeout_at(deadline, reader.read_exact(bytes))
        .await
        .map_err(|_| LinuxTransportError::Deadline)?
        .map(|_| ())
        .map_err(|_| LinuxTransportError::Closed)
}
