// SPDX-License-Identifier: MIT

use super::GatewayClient;
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};
use sts2_harness::ExecutionCancellation;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn listener() -> Result<TcpListener, std::io::Error> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

fn accept(listener: &TcpListener) -> Result<std::net::TcpStream, std::io::Error> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                stream.set_write_timeout(Some(Duration::from_secs(2)))?;
                return Ok(stream);
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => return Err(error),
        }
    }
}

#[test]
fn cancelled_gateway_work_never_connects_but_explicit_release_still_runs() -> TestResult {
    let listener = listener()?;
    let cancellation = ExecutionCancellation::default();
    cancellation.cancel();
    let client = GatewayClient {
        address: listener.local_addr()?,
        token: "synthetic-token".into(),
        cancellation,
    };
    assert!(
        client
            .request(
                "POST",
                "/v1/instances/allocate",
                &Value::Null,
                BTreeMap::new()
            )
            .is_err()
    );
    assert!(
        matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
    std::thread::scope(|scope| {
        let server = scope.spawn(|| -> std::io::Result<()> {
            let mut stream = accept(&listener)?;
            let mut bytes = [0; 4096];
            let count = stream.read(&mut bytes)?;
            assert!(bytes[..count].starts_with(b"POST /v1/instances/instance-1/release "));
            stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 21\r\n\r\n{\"status\":\"released\"}",
            )
        });
        let result = client.release("instance-1", &Value::Null, BTreeMap::new());
        server.join().map_err(|_| "release server failed")??;
        assert_eq!(result?["status"], "released");
        Ok::<_, Box<dyn std::error::Error>>(())
    })?;
    assert!(client.cancellation.is_cancelled());
    Ok(())
}

#[test]
fn cancellation_after_send_closes_socket_without_resending_or_claiming_rejection() -> TestResult {
    let listener = listener()?;
    let cancellation = ExecutionCancellation::default();
    let client = GatewayClient {
        address: listener.local_addr()?,
        token: "synthetic-token".into(),
        cancellation: cancellation.clone(),
    };
    std::thread::scope(|scope| {
        let server = scope.spawn(|| -> std::io::Result<()> {
            let mut stream = accept(&listener)?;
            let mut bytes = [0; 4096];
            assert!(stream.read(&mut bytes)? > 0);
            cancellation.cancel();
            // A dropped exchange must close the socket, not leave a detached reader.
            while stream.read(&mut bytes)? != 0 {}
            Ok(())
        });
        let started = Instant::now();
        let result = client.request(
            "POST",
            "/v1/instances/allocate",
            &Value::Null,
            BTreeMap::new(),
        );
        assert_eq!(
            result,
            Err(String::from(
                "gateway exchange cancelled; outcome remains uncertain"
            ))
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        server.join().map_err(|_| "cancellation server failed")??;
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })
}
