// SPDX-License-Identifier: MIT

fn accept_endpoint(listener: &mut EndpointListener) -> Result<Option<EndpointStream>, String> {
    match listener.accept() {
        Ok((stream, _)) => Ok(Some(stream)),
        Err(error) if error.kind() == ErrorKind::WouldBlock => Ok(None),
        Err(_) => Err(String::from("worker endpoint accept failed")),
    }
}
