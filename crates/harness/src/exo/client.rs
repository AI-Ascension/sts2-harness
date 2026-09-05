// SPDX-License-Identifier: MIT

use super::protocol::{ExoConfig, ExoError, ExoProvider, ExoTransport};

/// Named client wrapper retained for callers that want an adapter identity without Exo SDK types.
#[derive(Debug)]
pub struct ExoClient<T> {
    provider: ExoProvider<T>,
}

impl<T> ExoClient<T> {
    pub fn new(transport: T, config: ExoConfig) -> Self {
        Self {
            provider: ExoProvider::new(transport, config),
        }
    }

    #[must_use]
    pub fn provider(&self) -> &ExoProvider<T> {
        &self.provider
    }

    pub fn close(&mut self) -> Result<(), ExoError>
    where
        T: ExoTransport,
    {
        self.provider.close_for_client().map_err(ExoError::from)
    }
}

impl<T: ExoTransport> ExoProvider<T> {
    pub(super) fn close_for_client(&mut self) -> Result<(), super::protocol::ExoTransportError> {
        self.transport_close()
    }
}
