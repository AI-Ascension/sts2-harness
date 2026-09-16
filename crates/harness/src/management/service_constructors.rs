// SPDX-License-Identifier: MIT

use super::*;

impl ManagementService {
    pub fn in_memory() -> Self {
        Self::new(Arc::new(MemoryWorkflowStore::new()))
            .with_authoring_store(Arc::new(MemoryAuthoringStore::new()))
    }

    pub fn file_store(store: FileWorkflowStore) -> Self {
        Self::new(Arc::new(store))
    }

    pub fn with_definition_port(mut self, port: Arc<dyn DefinitionPort>) -> Self {
        self.definitions = port;
        self
    }

    pub fn with_authoring_store(mut self, store: Arc<dyn AuthoringStore>) -> Self {
        self.authoring = store;
        self
    }
}
