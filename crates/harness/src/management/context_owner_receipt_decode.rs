// SPDX-License-Identifier: MIT

//! Strict wire decoding for the versioned context-control receipt.

use serde::Deserialize;
use serde::de::{Deserializer, Error as DeError};

use super::{
    CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION, ContextBoundary, ContextControlCommandKind,
    ContextControlReceipt,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextControlReceiptWire {
    schema_version: String,
    owner_id: String,
    invocation_id: String,
    binding_id: String,
    binding_digest: String,
    command: Option<ContextControlCommandKind>,
    command_id: String,
    idempotency_key: String,
    effect: String,
    control_version: u64,
    plan_epoch: u64,
    controller_epoch: u64,
    gate_epoch: u64,
    boundary: Option<ContextBoundary>,
    revision_id: Option<String>,
    preview_manifest_digest: Option<String>,
    approved_manifest_digest: Option<String>,
}

impl<'de> Deserialize<'de> for ContextControlReceipt {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ContextControlReceiptWire::deserialize(deserializer)?;
        if wire.schema_version != CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION {
            return Err(D::Error::custom(format!(
                "unsupported context control receipt schema {}; expected {}; \
                 v1 receipts cannot be upgraded safely and must be reissued",
                wire.schema_version, CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION
            )));
        }
        Ok(Self {
            schema_version: wire.schema_version,
            owner_id: wire.owner_id,
            invocation_id: wire.invocation_id,
            binding_id: wire.binding_id,
            binding_digest: wire.binding_digest,
            command: wire
                .command
                .ok_or_else(|| D::Error::missing_field("command"))?,
            command_id: wire.command_id,
            idempotency_key: wire.idempotency_key,
            effect: wire.effect,
            control_version: wire.control_version,
            plan_epoch: wire.plan_epoch,
            controller_epoch: wire.controller_epoch,
            gate_epoch: wire.gate_epoch,
            boundary: wire
                .boundary
                .ok_or_else(|| D::Error::missing_field("boundary"))?,
            revision_id: wire.revision_id,
            preview_manifest_digest: wire.preview_manifest_digest,
            approved_manifest_digest: wire.approved_manifest_digest,
        })
    }
}
