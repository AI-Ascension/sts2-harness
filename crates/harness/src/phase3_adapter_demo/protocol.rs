// SPDX-License-Identifier: MIT

use serde_json::{Value, json};

pub(super) const ADAPTER_SCHEMA: &str = "ascension.context-memory.adapter-request.v1";
pub(super) const NOW: &str = "2026-09-10T12:00:00Z";
pub(super) const LATER: &str = "2026-09-11T12:00:00Z";
pub(super) const OUTPUT: &[u8] = b"fake-summary-requires-review";

#[allow(clippy::too_many_arguments)]
pub(super) fn adapter_request(
    binding_id: &str,
    selection_id: &str,
    phase2_revision_id: &str,
    selection_sha256: &str,
    audit_sha256: &str,
    policy_id: &str,
    review_id: &str,
    summary_output_sha256: &str,
    first_input_sha256: &str,
    preview_only: bool,
) -> Value {
    json!({
        "schema": ADAPTER_SCHEMA,
        "binding_id": binding_id,
        "selection_id": selection_id,
        "phase2_revision_id": phase2_revision_id,
        "selection_sha256": selection_sha256,
        "audit_sha256": audit_sha256,
        "policy_id": policy_id,
        "policy_version": 1,
        "review_id": review_id,
        "summary_output_sha256": summary_output_sha256,
        "first_input_sha256": first_input_sha256,
        "preview_only": preview_only,
    })
}
