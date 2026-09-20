// SPDX-License-Identifier: MIT

//! Batch builders and one saved-capture port double for the semantic history import suites.
//!
//! The double hands back the bytes an owner-side importer would have written and nothing else, so a
//! suite can only ever exercise what the harness re-derives from those bytes.

#![allow(dead_code)]

use sts2_harness::semantic_history::{
    SemanticHistoryBinding, SemanticHistoryCausalParent, SemanticHistoryError,
    SemanticHistoryImportBatch, SemanticHistoryImportEvent, SemanticHistoryImportOutcome,
    SemanticHistoryImportPort, SemanticHistoryKind, SemanticHistoryOrigin, SemanticHistoryStore,
    SemanticHistoryValue, import_saved_history,
};

use crate::fixture;

/// One saved capture, held as the bytes a port would hand back.
pub struct Saved {
    pub binding: SemanticHistoryBinding,
    pub batches: Vec<(String, Vec<u8>)>,
    pub fail: Option<SemanticHistoryError>,
}

impl Saved {
    pub fn new(binding: SemanticHistoryBinding) -> Self {
        Self {
            binding,
            batches: Vec::new(),
            fail: None,
        }
    }

    pub fn holding(mut self, batch_id: &str, batch: &SemanticHistoryImportBatch) -> Self {
        self.batches
            .push((batch_id.to_owned(), batch.encode().expect("encode")));
        self
    }

    pub fn holding_bytes(mut self, batch_id: &str, bytes: Vec<u8>) -> Self {
        self.batches.push((batch_id.to_owned(), bytes));
        self
    }
}

impl SemanticHistoryImportPort for Saved {
    fn binding(&self) -> &SemanticHistoryBinding {
        &self.binding
    }

    fn batches(&self) -> Vec<String> {
        self.batches.iter().map(|(id, _)| id.clone()).collect()
    }

    fn batch(&self, batch_id: &str) -> Result<Vec<u8>, SemanticHistoryError> {
        if let Some(error) = &self.fail {
            return Err(error.clone());
        }
        self.batches
            .iter()
            .find(|(id, _)| id == batch_id)
            .map(|(_, bytes)| bytes.clone())
            .ok_or(SemanticHistoryError::Port)
    }
}

/// One imported event, carrying the coverage it was captured under.
pub fn imported(
    event_id: &str,
    kind: SemanticHistoryKind,
    sequence: u64,
    value: Option<SemanticHistoryValue>,
) -> SemanticHistoryImportEvent {
    let mut input = fixture::event(event_id, kind, sequence, value);
    input.origin = SemanticHistoryOrigin::Imported;
    SemanticHistoryImportEvent {
        input,
        causal_parent: SemanticHistoryCausalParent::NotStated,
    }
}

/// One batch of plain captured events, oldest first.
pub fn batch(capture: &str, branch: &str, count: u64) -> SemanticHistoryImportBatch {
    SemanticHistoryImportBatch::new(
        capture,
        branch,
        (1..=count)
            .map(|sequence| {
                imported(
                    &format!("event_{sequence}"),
                    SemanticHistoryKind::CardPlayed,
                    sequence,
                    None,
                )
            })
            .collect(),
    )
}

/// The document one batch encodes to, as a value so one field can be tampered with.
pub fn document(batch: &SemanticHistoryImportBatch) -> serde_json::Value {
    serde_json::from_slice(&batch.encode().expect("encode")).expect("document")
}

/// Imports one batch into one store, reporting the refusal instead of panicking.
pub fn imported_into(
    store: &mut SemanticHistoryStore,
    batch: &SemanticHistoryImportBatch,
) -> Result<Vec<SemanticHistoryImportOutcome>, SemanticHistoryError> {
    let port = Saved::new(fixture::binding()).holding("batch_0001", batch);
    import_saved_history(store, &port)
}

/// Imports one batch the boundary must refuse, and returns the reason.
pub fn refused(
    store: &mut SemanticHistoryStore,
    batch: &SemanticHistoryImportBatch,
) -> SemanticHistoryError {
    imported_into(store, batch).expect_err("the batch is refused")
}
