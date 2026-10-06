// SPDX-License-Identifier: MIT
//! Bounded newline framing shared by the legacy and guarded lookup relays.

use sts2_harness::exo_lookup_wire::EXO_LOOKUP_FRAME_BYTES;
use tokio::io::{AsyncRead, AsyncReadExt};

pub(crate) async fn read_line(
    reader: &mut (impl AsyncRead + Unpin),
) -> Result<Vec<u8>, &'static str> {
    let mut bytes = Vec::new();
    loop {
        let byte = reader
            .read_u8()
            .await
            .map_err(|_| "exo_bridge_lookup_input")?;
        if byte == b'\n' {
            return Ok(bytes);
        }
        if bytes.len() >= EXO_LOOKUP_FRAME_BYTES {
            return Err("exo_bridge_lookup_bound");
        }
        bytes.push(byte);
    }
}
