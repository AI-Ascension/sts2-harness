// SPDX-License-Identifier: MIT

//! Backfilling saved history through an owned port, without letting the owner author an event.
//!
//! Saved history is not native history. It was captured by an earlier process under an authority
//! this one did not hold, so a backfill may only re-import what that capture actually recorded. The
//! port is therefore the narrowest one that can still carry the data: it hands back **opaque encoded
//! bytes**, so an implementation outside this module can supply a document but cannot construct a
//! [`SemanticHistoryEventInput`] in process, and every rule the append path applies is re-derived
//! from the bytes rather than assumed.
//!
//! Three properties keep an import honest, and each is structural rather than conventional:
//!
//! - The bytes are decoded by this module's own confined decoder under the same schema and the same
//!   byte bound the store applies to its own documents, with unknown fields refused. An importer
//!   cannot widen the record shape, and cannot state a field the store would not have written.
//! - Every imported event must already say its origin is `imported`, and the capture window is the
//!   store's, never the port's. A batch that offers a native or derived origin is refused rather than
//!   relabelled, so saved history cannot be laundered into history this boundary observed, and a
//!   backfill cannot declare its own gaps and then close them.
//! - A batch is applied whole or not at all. Every event is appended to a copy of the store first,
//!   and the store is replaced only when the whole batch was admitted, because a backfill that
//!   stopped halfway would leave behind a history no capture ever produced.

use super::{
    Error, MAX_HISTORY_IMPORT_BATCHES, MAX_HISTORY_IMPORT_BYTES, MAX_HISTORY_IMPORT_EVENTS,
    SEMANTIC_HISTORY_SCHEMA, SemanticHistoryAppend, SemanticHistoryBinding,
    SemanticHistoryCausalParent, SemanticHistoryEventInput, SemanticHistoryOrigin,
    SemanticHistoryStore, validate_history_identity,
};
use serde::{Deserialize, Serialize};

/// The port an owner-side importer supplies saved history through.
///
/// The only thing this port can return is bytes. That is deliberate: a port returning events would
/// let an implementation mint one, and a history whose imported records were authored at import time
/// is indistinguishable from a captured one. An implementation names the batches it holds and hands
/// back the encoded document for each.
pub trait SemanticHistoryImportPort {
    /// The owner scope the saved history was captured under.
    fn binding(&self) -> &SemanticHistoryBinding;

    /// The batch identities this port holds, in the order they should be imported.
    fn batches(&self) -> Vec<String>;

    /// The encoded document for one batch identity.
    ///
    /// The bytes are refused unless they decode as a batch under this module's decoder, so an
    /// implementation cannot answer with a shape this boundary does not admit.
    fn batch(&self, batch_id: &str) -> Result<Vec<u8>, Error>;
}

/// One imported event, as the earlier capture recorded it.
///
/// The event travels with the coverage and the source label it was captured under. Both are checked
/// rather than rewritten: coverage is honoured against the store's own capture window, and the
/// origin must already say `imported`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticHistoryImportEvent {
    /// The event as the earlier capture recorded it.
    pub input: SemanticHistoryEventInput,
    /// The causal parent recorded with it, stated or absent.
    pub causal_parent: SemanticHistoryCausalParent,
}

/// One batch of saved history, as a port encodes it.
///
/// A batch is the unit a backfill arrives in: which saved capture produced it, the branch that
/// capture recorded on, and the events it recorded. It states no owner scope of its own — the store
/// supplies that — so a batch cannot carry a run, profile or epoch across a scope it was not saved
/// under. This is also the document a port hands back, so the shape an encoder writes and the shape
/// the decoder admits are one shape rather than two that could drift apart.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticHistoryImportBatch {
    /// The schema this batch is written under.
    pub schema: String,
    /// The saved capture these events came from, as an opaque identity.
    pub capture_id: String,
    /// The branch the saved capture recorded these events on.
    pub branch_id: String,
    /// The events, oldest first.
    pub events: Vec<SemanticHistoryImportEvent>,
}

impl SemanticHistoryImportBatch {
    /// Builds a batch under the current schema.
    #[must_use]
    pub fn new(capture_id: &str, branch_id: &str, events: Vec<SemanticHistoryImportEvent>) -> Self {
        Self {
            schema: SEMANTIC_HISTORY_SCHEMA.to_owned(),
            capture_id: capture_id.to_owned(),
            branch_id: branch_id.to_owned(),
            events,
        }
    }

    /// Encodes this batch as the bytes a port hands back.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        serde_json::to_vec(self).map_err(|_| Error::Corrupt)
    }
}

/// What one imported batch did, so a caller can tell a first import from a re-import.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticHistoryImportOutcome {
    /// The batch identity that was imported.
    pub batch_id: String,
    /// The saved capture it came from.
    pub capture_id: String,
    /// The branch it was appended to.
    pub branch_id: String,
    /// Events newly recorded by this import.
    pub recorded: usize,
    /// Events already recorded with identical content, which wrote nothing.
    pub replayed: usize,
}

/// Imports every batch one port holds, in the order the port names them.
///
/// Each batch is imported under the store's own append path, so an imported event is ordered,
/// deduplicated, gap-checked and capacity-checked exactly like a captured one. The port's owner
/// scope and authority epoch must be the store's: saved history from another run, profile, manifest
/// or epoch is refused rather than re-stamped into this one.
pub fn import_saved_history(
    store: &mut SemanticHistoryStore,
    port: &dyn SemanticHistoryImportPort,
) -> Result<Vec<SemanticHistoryImportOutcome>, Error> {
    let batches = port.batches();
    if batches.is_empty() || batches.len() > MAX_HISTORY_IMPORT_BATCHES {
        return Err(Error::Bounds);
    }
    if !port.binding().same_owner(store.binding()) {
        return Err(Error::Scope);
    }
    if port.binding().authority_epoch != store.binding().authority_epoch {
        return Err(Error::Epoch);
    }
    let mut outcomes = Vec::with_capacity(batches.len());
    for batch_id in batches {
        validate_history_identity(&batch_id, "import.batch_id")?;
        outcomes.push(import_batch(store, &batch_id, &port.batch(&batch_id)?)?);
    }
    Ok(outcomes)
}

/// Imports one encoded batch into a copy of the store, and keeps the copy only if it all held.
fn import_batch(
    store: &mut SemanticHistoryStore,
    batch_id: &str,
    bytes: &[u8],
) -> Result<SemanticHistoryImportOutcome, Error> {
    if bytes.is_empty() || bytes.len() > MAX_HISTORY_IMPORT_BYTES {
        // A backfill is bounded before it is parsed, so an oversized document is never read as
        // history by first being decoded.
        return Err(Error::Bounds);
    }
    let document: SemanticHistoryImportBatch =
        serde_json::from_slice(bytes).map_err(|_| Error::Corrupt)?;
    if document.schema != SEMANTIC_HISTORY_SCHEMA {
        return Err(Error::Corrupt);
    }
    validate_history_identity(&document.capture_id, "import.capture_id")?;
    validate_history_identity(&document.branch_id, "import.branch_id")?;
    if document.events.len() > MAX_HISTORY_IMPORT_EVENTS {
        return Err(Error::Bounds);
    }
    // The branch must be one this store holds. Appending would refuse a batch that carries events,
    // but a batch that names a branch that does not exist and carries none would otherwise report a
    // successful backfill of a scope this store never captured.
    store.events(&document.branch_id)?;
    // The batch is applied to a copy first, so a batch that is refused partway through leaves the
    // store exactly as it was. The copy is taken directly rather than through the store's own
    // encoded document, because that document is written to be re-validated and retention
    // deliberately leaves a redacted payload's digest as recorded.
    let mut candidate = store.clone();
    let mut outcome = SemanticHistoryImportOutcome {
        batch_id: batch_id.to_owned(),
        capture_id: document.capture_id,
        branch_id: document.branch_id.clone(),
        recorded: 0,
        replayed: 0,
    };
    for event in document.events {
        if event.input.origin != SemanticHistoryOrigin::Imported {
            // Saved history may not be offered as native or derived history: a record that claims to
            // have been observed here is the one thing a backfill must not produce.
            return Err(Error::ImportedOrigin);
        }
        match candidate.append(&document.branch_id, event.input, event.causal_parent)? {
            SemanticHistoryAppend::Recorded => outcome.recorded += 1,
            SemanticHistoryAppend::Replayed => outcome.replayed += 1,
        }
    }
    *store = candidate;
    Ok(outcome)
}
