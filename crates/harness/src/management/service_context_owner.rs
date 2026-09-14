// SPDX-License-Identifier: MIT

use std::sync::Arc;

use super::ContextOwnerPort;
use super::ManagementService;

impl ManagementService {
    pub fn with_context_owner_port(mut self, port: Arc<dyn ContextOwnerPort>) -> Self {
        self.context_owner = port;
        self
    }

    pub fn context_owner_port(&self) -> &dyn ContextOwnerPort {
        self.context_owner.as_ref()
    }
}
