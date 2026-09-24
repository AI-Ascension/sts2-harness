// SPDX-License-Identifier: MIT

//! Shared synthetic fixtures for the per-invocation context membership suites (issue #106).
//!
//! Every fixture is synthetic: no provider, host, or game is contacted. The helpers are shared by
//! the policy-resolution and pin-revalidation suites so both exercise the same identity model.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;

use sts2_harness::context_control::{
    CONTEXT_MEMBERSHIP_POLICY_SCHEMA, ContextItem, ContextItemRef, ContextMembershipBroaderScope,
    ContextMembershipPolicy, ContextMembershipScope, ContextModelView, MembershipCheckContext,
    MembershipContinuity, MembershipDisposition,
};

pub const NOW: u64 = 500;
pub const FUTURE: u64 = 10_000;

pub fn sha256(bytes: &[u8]) -> String {
    sts2_harness::sha256_hex(bytes)
}

pub fn item(
    item_id: &str,
    version: u64,
    kind: &str,
    content: &str,
    protected: bool,
) -> ContextItem {
    let bytes = content.as_bytes().to_vec();
    ContextItem {
        reference: ContextItemRef {
            item_id: item_id.to_owned(),
            version,
            sha256: sha256(&bytes),
        },
        kind: kind.to_owned(),
        bytes,
        protected,
        expires_at: FUTURE,
    }
}

pub fn scope() -> ContextMembershipScope {
    ContextMembershipScope {
        run_id: "run-1".to_owned(),
        episode_id: "episode-1".to_owned(),
        agent_id: "agent-1".to_owned(),
        branch_id: Some("branch-1".to_owned()),
    }
}

/// One shared, unprotected registry item per distinct content string.
pub fn registry_history(contents: &[&str]) -> (BTreeMap<String, ContextItem>, Vec<ContextItemRef>) {
    let mut registry = BTreeMap::new();
    let mut references = Vec::new();
    for (index, content) in contents.iter().enumerate() {
        let entry = item(&format!("history-{index}"), 1, "history", content, false);
        registry.insert(
            format!("{}:{}", entry.reference.item_id, entry.reference.version),
            entry.clone(),
        );
        references.push(entry.reference);
    }
    (registry, references)
}

pub fn policy(
    invocation_id: &str,
    disposition: MembershipDisposition,
    overrides: Vec<ContextItemRef>,
) -> ContextMembershipPolicy {
    ContextMembershipPolicy {
        schema: CONTEXT_MEMBERSHIP_POLICY_SCHEMA.to_owned(),
        invocation_id: invocation_id.to_owned(),
        base_revision_id: "revision-1".to_owned(),
        disposition,
        overrides,
        inherit_pins: false,
        broader_scope: ContextMembershipBroaderScope::default(),
        model_view: ContextModelView::visible(),
        ancestor_history: sts2_harness::context_control::AncestorHistoryMode::ThroughFork,
    }
}

pub fn check() -> MembershipCheckContext {
    MembershipCheckContext {
        caller_scope: scope(),
        continuity: MembershipContinuity::Stateless,
        generation: 1,
        controller_epoch: 1,
        gate_epoch: 0,
        revoked_item_ids: Vec::new(),
        revoked_invocation_ids: Vec::new(),
    }
}

/// The effective, model-visible item ids of a resolved set, in order.
pub fn visible_ids(effective: &sts2_harness::EffectiveMembership) -> Vec<String> {
    effective
        .model_visible
        .iter()
        .map(|reference| reference.item_id.clone())
        .collect()
}
