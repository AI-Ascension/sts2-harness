// SPDX-License-Identifier: MIT

use std::io::Read;
use std::net::TcpStream;
use std::time::Duration;

pub(super) fn consume_request(stream: &mut TcpStream) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| "timeout failed".to_owned())?;
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let count = stream
            .read(&mut buffer)
            .map_err(|_| "request read failed".to_owned())?;
        if count == 0 {
            return Err("request ended before body".to_owned());
        }
        request.extend_from_slice(&buffer[..count]);
        let Some(split) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
            continue;
        };
        let body_start = split + 4;
        let header_text =
            std::str::from_utf8(&request[..split]).map_err(|_| "headers utf8".to_owned())?;
        let content_length = header_text
            .lines()
            .find_map(|line| line.strip_prefix("Content-Length: "))
            .ok_or_else(|| "missing content length".to_owned())?
            .parse::<usize>()
            .map_err(|_| "invalid content length".to_owned())?;
        if request.len() >= body_start + content_length {
            return Ok(());
        }
    }
}
