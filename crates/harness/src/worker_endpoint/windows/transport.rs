// SPDX-License-Identifier: MIT

fn accept_endpoint(listener: &mut EndpointListener) -> Result<Option<EndpointStream>, String> {
    listener.accept()
}
