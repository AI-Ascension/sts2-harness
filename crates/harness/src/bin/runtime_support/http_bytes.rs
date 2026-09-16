// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::net::TcpStream;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{
    GatewayClient, MAX_BODY_BYTES, MAX_HEADER_BYTES, read_response, remaining, validate_request,
    write_deadline,
};

pub(super) fn exchange_bytes(
    client: &GatewayClient,
    method: &str,
    path: &str,
    body: &Value,
    extra_headers: BTreeMap<String, String>,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let deadline = Instant::now() + timeout;
    validate_request(method, path, &extra_headers)?;
    let bytes = if body.is_null() {
        Vec::new()
    } else {
        serde_json::to_vec(body)
            .map_err(|error| format!("request serialization failed: {error}"))?
    };
    if bytes.len() > MAX_BODY_BYTES {
        return Err(String::from("gateway request body exceeds the bound"));
    }
    let mut headers = extra_headers;
    headers.insert(
        String::from("Authorization"),
        format!("Bearer {}", client.token),
    );
    headers.insert(String::from("Host"), client.address.to_string());
    headers.insert(String::from("Content-Length"), bytes.len().to_string());
    if !bytes.is_empty() {
        headers.insert(
            String::from("Content-Type"),
            String::from("application/json"),
        );
    }
    let mut request = format!("{method} {path} HTTP/1.1\r\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("Connection: close\r\n\r\n");
    if request.len() > MAX_HEADER_BYTES {
        return Err(String::from("gateway request headers exceed the bound"));
    }
    let mut stream = TcpStream::connect_timeout(&client.address, remaining(deadline)?)
        .map_err(|_| String::from("gateway connection failed"))?;
    write_deadline(&mut stream, request.as_bytes(), deadline)?;
    write_deadline(&mut stream, &bytes, deadline)?;
    let response = read_response(&mut stream, deadline)?;
    if !(200..300).contains(&response.status) {
        return Err(format!("gateway returned HTTP {}", response.status));
    }
    Ok(response.body)
}
