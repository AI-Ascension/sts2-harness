// SPDX-License-Identifier: MIT

fn fake_gateway(listener: TcpListener) -> Result<(), String> {
    let mut allocation = super::accept(&listener)?;
    let headers = super::request(&mut allocation).map_err(|error| error.to_string())?;
    if !headers.starts_with("POST /v1/sessions/allocate ") {
        return Err(String::from(
            "fake gateway received an unexpected allocation",
        ));
    }
    let body = json!({
        "status":"allocated", "instance_id":"instance-1", "caller_id":"harness",
        "session_id":"session-1", "lease_id":"lease-1", "lease_epoch":1
    })
    .to_string();
    write!(
        allocation,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .map_err(|error| error.to_string())?;
    drop(allocation);
    let mut release =
        super::accept_until(&listener, std::time::Duration::from_secs(10))?;
    let headers = super::request(&mut release).map_err(|error| error.to_string())?;
    if !headers.starts_with("POST /v1/instances/instance-1/release ") {
        return Err(String::from("fake gateway received an unexpected release"));
    }
    let body = r#"{"status":"released"}"#;
    write!(
        release,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}
