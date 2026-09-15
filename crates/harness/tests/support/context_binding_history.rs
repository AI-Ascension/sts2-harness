// SPDX-License-Identifier: MIT

//! Recording synthetic owner and isolated SQLite fixture lifecycle.
#![allow(clippy::expect_used)]

use std::path::PathBuf;
use std::sync::Mutex;

use sts2_harness::management::{
    AuthContext, ContextBindingCatalog, ContextBindingRequest, ContextOwnerBinding,
    ContextOwnerPort, ManagementError,
};

use super::support::FakeContextOwner;

#[derive(Default)]
pub(super) struct RecordingOwner(pub(super) Mutex<Vec<ContextOwnerBinding>>);

impl ContextOwnerPort for RecordingOwner {
    fn catalog(&self, actor: &AuthContext) -> Result<ContextBindingCatalog, ManagementError> {
        FakeContextOwner.catalog(actor)
    }

    fn bind(
        &self,
        actor: &AuthContext,
        request: &ContextBindingRequest,
    ) -> Result<ContextOwnerBinding, ManagementError> {
        let binding = FakeContextOwner.bind(actor, request)?;
        self.0
            .lock()
            .expect("recording owner")
            .push(binding.clone());
        Ok(binding)
    }
}

pub(super) struct Database(pub(super) PathBuf);

impl Database {
    pub(super) fn new(name: &str) -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/context-binding-persistence")
            .join(format!("{}-{name}", std::process::id()));
        std::fs::create_dir_all(&root).expect("fixture directory");
        Self(root.join("workflow.sqlite"))
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        if let Some(root) = self.0.parent() {
            let _ = std::fs::remove_dir_all(root);
        }
    }
}
