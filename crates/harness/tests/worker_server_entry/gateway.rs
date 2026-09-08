// SPDX-License-Identifier: MIT

//! Bounded synthetic gateway cleanup acknowledgment; never answers allocation.

use super::TestResult;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(super) async fn acknowledge_release(gateway: &tokio::net::TcpListener) -> TestResult {
    tokio::time::timeout(Duration::from_secs(3), async {
        let (mut stream, _) = gateway.accept().await?;
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 1024];
        while !bytes.windows(4).any(|value| value == b"\r\n\r\n") {
            let count = stream.read(&mut buffer).await?;
            if count == 0 || bytes.len() + count > 4096 {
                return Err("invalid synthetic cleanup request".into());
            }
            bytes.extend_from_slice(&buffer[..count]);
        }
        assert!(bytes.starts_with(b"POST /v1/instances/instance-1/release "));
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 21\r\n\r\n{\"status\":\"released\"}")
            .await?;
        Ok(())
    })
    .await?
}
