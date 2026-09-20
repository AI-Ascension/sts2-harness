// SPDX-License-Identifier: MIT

//! The owned mod port saved history is restored through.
//!
//! Saved native history exists before this harness watched anything, so it can never be admitted as
//! capture. It is restored through one port the harness owns, and the restored records keep the
//! labels their own source stated: an observed restored record carries the `Imported` origin, so it
//! can never read as an event this harness observed, and a restored gap keeps the coverage label
//! naming what its source could not see. Restoring history is a deliberate grant rather than a
//! capability that arrives with the type, and the store gains no second write path for it: the span
//! is admitted through the same vocabulary, identity and bound checks as a live batch, and lands
//! ahead of the retained capture start.

use serde::{Deserialize, Serialize};

use super::error::{
    SemanticHistoryError, SemanticHistoryRefusal as Refusal, SemanticHistoryResult,
};
use super::ingest::validate_identity;
use super::record::SemanticEventBatch;
use super::scope::{SEMANTIC_MAX_IDENTITY_BYTES, SemanticCatalogBinding, SemanticHistoryFence};
use super::store::SemanticHistoryStore;

/// The mod port a harness-owned backfill reads saved history through.
///
/// The harness declares the port and drives it; the mod implements it and owns the native read. The
/// port yields a batch and nothing else, so it cannot write to the store, name a branch the store
/// does not retain, or reach the retained bytes directly.
pub trait SemanticBackfillPort {
    /// Reads the requested span of saved history, or refuses because it cannot.
    fn restore(
        &mut self,
        request: &SemanticBackfillRequest,
    ) -> SemanticHistoryResult<SemanticEventBatch>;
}

/// Whether the harness has authorized restoring saved history through the owned mod port.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SemanticBackfillAuthority {
    /// No saved history is restored; the default until the harness grants the port deliberately.
    #[default]
    NotGranted,
    /// The harness has granted the port.
    Granted,
}

/// One bounded request to restore a span of saved history.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticBackfillRequest {
    /// Caller operation identity, so a re-delivered restore is recognised rather than repeated.
    pub operation_id: String,
    /// Run, branch, episode and epoch whose history is being extended backwards.
    pub fence: SemanticHistoryFence,
    /// Catalog binding the caller expects the retained history to be bound to.
    pub binding: SemanticCatalogBinding,
    /// First sequence number of the restored span; capture will begin here.
    pub first_sequence: u64,
    /// Last sequence number of the restored span, inclusive; it must abut the retained start.
    pub last_sequence: u64,
}

/// What one restore did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SemanticBackfillOutcome {
    /// How many records the restore put ahead of the retained capture.
    pub restored: usize,
    /// How many records the history holds afterwards.
    pub total: usize,
    /// Where capture begins after the restore.
    pub capture_start_sequence: u64,
}

/// Restores saved history through the owned mod port, granted deliberately by the harness.
///
/// A restore the harness has not granted is refused before the port is consulted, so a mod is never
/// asked for native history the harness has not authorized. Everything the port returns is then
/// admitted by the store under the same rules a live batch faces, and the restore is idempotent by
/// operation identity: a re-delivery restores nothing a second time, and reusing that identity with
/// a different span or binding is refused.
pub fn restore_saved_history<P: SemanticBackfillPort>(
    store: &mut SemanticHistoryStore,
    port: &mut P,
    authority: SemanticBackfillAuthority,
    request: &SemanticBackfillRequest,
) -> SemanticHistoryResult<SemanticBackfillOutcome> {
    if authority != SemanticBackfillAuthority::Granted {
        return Err(SemanticHistoryError::about(
            Refusal::BackfillNotGranted,
            &request.operation_id,
        ));
    }
    validate_identity(&request.operation_id)?;
    validate_identity(&request.fence.run_id)?;
    validate_identity(&request.fence.branch_id)?;
    if request.fence.branch_id.len() > SEMANTIC_MAX_IDENTITY_BYTES {
        return Err(SemanticHistoryError::new(Refusal::Identity));
    }
    let batch = port.restore(request)?;
    let restored = store.restore(request, &batch)?;
    let total = store
        .records(&request.fence.branch_id)
        .map_or(0, |records| records.len());
    let start = store
        .window(&request.fence.branch_id)
        .map_or(0, |window| window.capture_start_sequence);
    Ok(SemanticBackfillOutcome {
        restored,
        total,
        capture_start_sequence: start,
    })
}
