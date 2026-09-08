// SPDX-License-Identifier: MIT

use super::{HttpResponse, read_response, remaining};
use std::net::SocketAddr;
use std::time::Instant;
use sts2_harness::ExecutionCancellation;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub(super) fn exchange(
    address: SocketAddr,
    headers: &[u8],
    body: &[u8],
    deadline: Instant,
    cancellation: &ExecutionCancellation,
) -> Result<HttpResponse, String> {
    std::thread::scope(|scope| {
        let owner = std::thread::Builder::new()
            .name(String::from("gateway-exchange"))
            .spawn_scoped(scope, || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|_| String::from("gateway exchange executor unavailable"))?;
                runtime.block_on(async {
                    tokio::select! {
                        biased;
                        _ = cancellation.cancelled() => Err(String::from("gateway exchange cancelled; outcome remains uncertain")),
                        result = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), exchange_io(address, headers, body, deadline)) => {
                            result.map_err(|_| String::from("gateway exchange deadline expired"))?
                        }
                    }
                })
            })
            .map_err(|_| String::from("gateway exchange owner unavailable"))?;
        owner
            .join()
            .map_err(|_| String::from("gateway exchange owner failed"))?
    })
}

async fn exchange_io(
    address: SocketAddr,
    headers: &[u8],
    body: &[u8],
    deadline: Instant,
) -> Result<HttpResponse, String> {
    let mut stream = TcpStream::connect(address)
        .await
        .map_err(|_| String::from("gateway connection failed"))?;
    write_deadline(&mut stream, headers, deadline).await?;
    write_deadline(&mut stream, body, deadline).await?;
    read_response(&mut stream, deadline).await
}

async fn write_deadline(
    stream: &mut TcpStream,
    mut bytes: &[u8],
    deadline: Instant,
) -> Result<(), String> {
    while !bytes.is_empty() {
        remaining(deadline)?;
        let written = stream
            .write(bytes)
            .await
            .map_err(|_| String::from("gateway request failed or timed out"))?;
        if written == 0 {
            return Err(String::from("gateway closed during request"));
        }
        bytes = &bytes[written..];
    }
    remaining(deadline)?;
    Ok(())
}

pub(super) async fn read_deadline(
    stream: &mut TcpStream,
    bytes: &mut [u8],
    deadline: Instant,
) -> Result<usize, String> {
    remaining(deadline)?;
    let read = stream
        .read(bytes)
        .await
        .map_err(|_| String::from("gateway response read failed or timed out"))?;
    remaining(deadline)?;
    Ok(read)
}
