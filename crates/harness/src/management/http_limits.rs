// SPDX-License-Identifier: MIT

// `MAX_CONNECTIONS` stays private: no sibling `http_*` module reads it, and a
// `pub use` nothing consumes is the unused-import warning this repo denies.
pub use super::super::contract::{
    MAX_HEADER_BYTES, MAX_JSON_BYTES, MAX_PATH_BYTES, MAX_RESPONSE_BYTES, REQUEST_DEADLINE_MILLIS,
};
use super::super::contract::MAX_CONNECTIONS;
use super::HttpError;
use std::time::Duration;

/// The admitted bounds on one management HTTP connection.
///
/// Split out of `http.rs` so that file stays inside the preferred production
/// budget; the values themselves are unchanged.
#[derive(Clone, Debug)]
pub struct HttpLimits {
    pub max_header_bytes: usize,
    pub max_body_bytes: usize,
    pub max_response_bytes: usize,
    pub max_path_bytes: usize,
    pub max_connections: usize,
    pub deadline: Duration,
}

impl Default for HttpLimits {
    fn default() -> Self {
        Self {
            max_header_bytes: MAX_HEADER_BYTES,
            max_body_bytes: MAX_JSON_BYTES,
            max_response_bytes: MAX_RESPONSE_BYTES,
            max_path_bytes: MAX_PATH_BYTES,
            max_connections: MAX_CONNECTIONS,
            deadline: Duration::from_millis(REQUEST_DEADLINE_MILLIS),
        }
    }
}

impl HttpLimits {
    pub(super) fn validate(&self) -> Result<(), HttpError> {
        if self.max_header_bytes == 0
            || self.max_header_bytes > MAX_HEADER_BYTES
            || self.max_body_bytes == 0
            || self.max_body_bytes > MAX_JSON_BYTES
            || self.max_response_bytes == 0
            || self.max_response_bytes > MAX_RESPONSE_BYTES
            || self.max_path_bytes == 0
            || self.max_path_bytes > MAX_PATH_BYTES
            || self.max_connections == 0
            || self.max_connections > MAX_CONNECTIONS
            || self.deadline.is_zero()
        {
            return Err(HttpError::new(
                "invalid_limits",
                "management HTTP limits are outside the admitted bounds",
            ));
        }
        Ok(())
    }
}
