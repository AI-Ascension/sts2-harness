// SPDX-License-Identifier: MIT

fn post_otlp(body: &[u8]) -> bool {
    post_otlp_to(&ENDPOINT, body)
}
fn post_otlp_to(endpoint: &SocketAddr, body: &[u8]) -> bool {
    let mut stream = match TcpStream::connect_timeout(endpoint, SOCKET_TIMEOUT) {
        Ok(stream) => stream,
        Err(_) => return false,
    };
    let deadline = Instant::now() + SOCKET_TIMEOUT;
    let request = format!(
        "POST {} HTTP/1.1\r\nHost: {endpoint}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        OTLP_PATH,
        body.len()
    );
    if !write_with_deadline(&mut stream, request.as_bytes(), deadline)
        || !write_with_deadline(&mut stream, body, deadline)
    {
        let _ = stream.shutdown(Shutdown::Both);
        return false;
    }
    let accepted = read_otlp_response(&mut stream, deadline)
        .is_some_and(|response| response.status / 100 == 2 && valid_otlp_success_body(&response.body));
    let _ = stream.shutdown(Shutdown::Both);
    accepted
}

struct OtlpHttpResponse {
    status: u16,
    body: Vec<u8>,
}

fn write_with_deadline(stream: &mut TcpStream, mut bytes: &[u8], deadline: Instant) -> bool {
    while !bytes.is_empty() {
        let Some(timeout) = deadline.checked_duration_since(Instant::now()) else {
            return false;
        };
        if stream.set_write_timeout(Some(timeout)).is_err() {
            return false;
        }
        let Ok(written) = stream.write(bytes) else {
            return false;
        };
        if written == 0 {
            return false;
        }
        bytes = &bytes[written..];
    }
    deadline.checked_duration_since(Instant::now()).is_some()
}

fn read_with_deadline(stream: &mut TcpStream, bytes: &mut [u8], deadline: Instant) -> Option<usize> {
    let timeout = deadline.checked_duration_since(Instant::now())?;
    stream.set_read_timeout(Some(timeout)).ok()?;
    stream.read(bytes).ok()
}

fn read_otlp_response(stream: &mut TcpStream, deadline: Instant) -> Option<OtlpHttpResponse> {
    const MAX_HEADER_BYTES: usize = 8 * 1024;
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 2048];
    let header_end = loop {
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            if end + 4 > MAX_HEADER_BYTES {
                return None;
            }
            break end;
        }
        if bytes.len() >= MAX_HEADER_BYTES {
            return None;
        }
        let read = read_with_deadline(stream, &mut buffer, deadline)?;
        if read == 0 {
            return None;
        }
        bytes.extend_from_slice(&buffer[..read]);
    };
    let header = std::str::from_utf8(&bytes[..header_end]).ok()?;
    let mut lines = header.split("\r\n");
    let status_line = lines.next()?;
    let mut status_parts = status_line.split_ascii_whitespace();
    if !matches!(status_parts.next(), Some("HTTP/1.1" | "HTTP/1.0")) {
        return None;
    }
    let code = status_parts.next()?;
    if code.len() != 3 || !code.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let status = code.parse::<u16>().ok()?;
    if !(100..600).contains(&status) || !valid_http_value(status_line) {
        return None;
    }

    let mut content_length = None;
    let mut content_type = None;
    let mut header_names = std::collections::BTreeSet::new();
    for line in lines {
        let (name, value) = line.split_once(':')?;
        if !valid_http_name(name)
            || !valid_http_value(value)
            || !header_names.insert(name.to_ascii_lowercase())
        {
            return None;
        }
        if name.eq_ignore_ascii_case("transfer-encoding") {
            return None;
        }
        if name.eq_ignore_ascii_case("content-length") {
            let value = value.trim_matches(' ');
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            let length = value.parse::<usize>().ok()?;
            if length > MAX_RESPONSE_BYTES || content_length.replace(length).is_some() {
                return None;
            }
        }
        if name.eq_ignore_ascii_case("content-type") {
            let value = value.trim_matches(' ');
            if value.is_empty() || content_type.replace(value.to_owned()).is_some() {
                return None;
            }
        }
    }
    let content_length = content_length?;
    let content_type = content_type?;
    if !valid_json_content_type(&content_type) {
        return None;
    }
    let body_start = header_end + 4;
    let available = bytes.len().saturating_sub(body_start);
    if available > content_length {
        return None;
    }
    let mut body = bytes[body_start..].to_vec();
    while body.len() < content_length {
        let remaining = content_length - body.len();
        let read_capacity = remaining.min(buffer.len());
        let read = read_with_deadline(stream, &mut buffer[..read_capacity], deadline)?;
        if read == 0 {
            return None;
        }
        body.extend_from_slice(&buffer[..read]);
    }
    Some(OtlpHttpResponse { status, body })
}

fn valid_http_name(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
        })
}

fn valid_http_value(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte == b' ' || byte.is_ascii_graphic())
}

fn valid_json_content_type(value: &str) -> bool {
    let mut parts = value.split(';');
    let Some(media_type) = parts.next() else {
        return false;
    };
    if !media_type.trim().eq_ignore_ascii_case("application/json") {
        return false;
    }
    parts.all(|parameter| {
        let parameter = parameter.trim();
        let Some((name, value)) = parameter.split_once('=') else {
            return false;
        };
        valid_http_name(name.trim())
            && !value.trim().is_empty()
            && valid_http_value(value.trim())
    })
}

fn valid_otlp_success_body(body: &[u8]) -> bool {
    let Ok(Value::Object(response)) = serde_json::from_slice::<Value>(body) else {
        return false;
    };
    response.iter().all(|(key, value)| match key.as_str() {
        "partialSuccess" => valid_partial_success(value),
        _ => false,
    })
}

fn valid_partial_success(value: &Value) -> bool {
    let Value::Object(partial) = value else {
        return false;
    };
    partial.iter().all(|(key, value)| match key.as_str() {
        "rejectedSpans" => rejected_spans_count(value).is_some_and(|count| count == 0),
        "errorMessage" => value.as_str().is_some_and(str::is_empty),
        _ => false,
    })
}

fn rejected_spans_count(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64(),
        Value::String(value) => value.parse::<u64>().ok(),
        _ => None,
    }
}
