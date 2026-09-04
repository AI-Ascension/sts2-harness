// SPDX-License-Identifier: MIT

use super::*;
use std::net::TcpListener;
use std::thread;

#[test]
fn numeric_loopback_only_and_request_injection_rejected() {
    for address in [
        "localhost:80",
        "example.invalid:80",
        "192.0.2.1:80",
        "[::]:80",
        "127.0.0.1:0",
    ] {
        assert!(parse_address(address).is_err(), "{address}");
    }
    for address in ["127.0.0.1:80", "[::1]:80"] {
        assert!(parse_address(address).is_ok());
    }
    for (name, value) in [
        ("Authorization", "other"),
        ("HOST", "other"),
        ("Transfer-Encoding", "chunked"),
        ("X-Test", "a\r\nInjected: yes"),
        ("X\r\nBad", "yes"),
    ] {
        assert!(
            validate_request(
                "POST",
                "/v1",
                &BTreeMap::from([(name.into(), value.into())])
            )
            .is_err()
        );
    }
    assert!(validate_request("GET", "/bad\r\n", &BTreeMap::new()).is_err());
    assert!(validate_request("GET\r\n", "/", &BTreeMap::new()).is_err());
    assert!(
        validate_request(
            "GET",
            "/",
            &BTreeMap::from([
                ("X-Test".into(), "one".into()),
                ("x-test".into(), "two".into())
            ])
        )
        .is_err()
    );
}

#[test]
fn response_framing_requires_one_length_and_no_transfer_encoding() {
    for headers in [
        "Content-Length: 2\r\ncontent-length: 2",
        "Content-Length: 2\r\nTransfer-Encoding: chunked",
        "Content-Length: +2",
        "Content-Length: 2, 2",
        "Content-Length: 2\r\nmalformed",
        "Content-Length: 2\r\n X-Fold: value",
    ] {
        assert!(response_length(headers.split("\r\n")).is_err(), "{headers}");
    }
    assert_eq!(
        response_length("Content-Length: 2\r\nX-Test: okay".split("\r\n")),
        Ok(2)
    );
}

fn exchange_response(response: Vec<u8>) -> Result<Value, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    let server = thread::spawn(move || -> std::io::Result<()> {
        let (mut stream, _) = listener.accept()?;
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        let mut request = [0; 4096];
        let _ = stream.read(&mut request)?;
        let _ = stream.write_all(&response);
        Ok(())
    });
    let result = GatewayClient {
        address,
        token: "synthetic-token".into(),
    }
    .request("GET", "/", &Value::Null, BTreeMap::new());
    server
        .join()
        .map_err(|_| "server panicked")?
        .map_err(|error| error.to_string())?;
    result
}

#[test]
fn response_payload_is_never_exposed_in_errors() {
    let secret = "{\"error\":\"synthetic-private-response\"}";
    let response = format!(
        "HTTP/1.1 403 Forbidden\r\nContent-Length: {}\r\n\r\n{secret}",
        secret.len()
    );
    assert_eq!(
        exchange_response(response.into_bytes()),
        Err("gateway returned HTTP 403".into())
    );
    assert_eq!(
        exchange_response(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}".to_vec()),
        Ok(serde_json::json!({}))
    );
}

#[test]
fn terminator_cannot_cross_header_budget() {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nX-Pad: {}\r\n\r\n{{}}",
        "a".repeat(MAX_HEADER_BYTES)
    );
    assert_eq!(
        exchange_response(response.into_bytes()),
        Err("gateway response headers exceed the bound".into())
    );
}

#[test]
fn trickling_response_cannot_extend_exchange_deadline() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    let server = thread::spawn(move || -> std::io::Result<()> {
        let (mut stream, _) = listener.accept()?;
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        let mut request = [0; 4096];
        let _ = stream.read(&mut request)?;
        for _ in 0..100 {
            if stream.write_all(b"H").is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    });
    let start = Instant::now();
    let result = GatewayClient {
        address,
        token: "synthetic-token".into(),
    }
    .exchange(
        "GET",
        "/",
        &Value::Null,
        BTreeMap::new(),
        Duration::from_millis(60),
    );
    let elapsed = start.elapsed();
    server
        .join()
        .map_err(|_| "server panicked")?
        .map_err(|error| error.to_string())?;
    assert!(result.is_err());
    assert!(elapsed < Duration::from_millis(500), "{elapsed:?}");
    Ok(())
}
