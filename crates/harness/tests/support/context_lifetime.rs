// SPDX-License-Identifier: MIT

//! Synthetic fixtures shared by the logical-invocation lifetime suites (issue #111).
//!
//! Every fixture is synthetic: no provider, host, game, or clock is contacted. All instants are
//! explicit logical clock values supplied by the caller, so expiry boundaries are reproducible.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
#![allow(dead_code)]

use sts2_harness::context_control::{
    CONTEXT_LIFETIME_SCHEMA, ContextLifetimeLedger, ContextLifetimeScope, InvocationOwnerScope,
    LifetimeApplicability, LogicalInvocationIdentity,
};

/// The logical instant every fixture scope is issued at.
pub const ISSUED_AT: u64 = 1_000;

/// The logical instant every fixture scope expires at.
pub const CEILING: u64 = 2_000;

/// An instant safely inside the fixture window.
pub const INSIDE: u64 = 1_500;

pub fn owner() -> InvocationOwnerScope {
    InvocationOwnerScope {
        run_id: "run-1".to_owned(),
        episode_id: "episode-1".to_owned(),
        agent_id: "agent-1".to_owned(),
        branch_id: Some("branch-1".to_owned()),
    }
}

/// The same agent in a sibling branch of the same episode.
pub fn sibling_branch_owner() -> InvocationOwnerScope {
    InvocationOwnerScope {
        branch_id: Some("branch-2".to_owned()),
        ..owner()
    }
}

/// A different agent in the same run and episode.
pub fn sibling_agent_owner() -> InvocationOwnerScope {
    InvocationOwnerScope {
        agent_id: "agent-2".to_owned(),
        ..owner()
    }
}

/// A different episode of the same run and agent.
pub fn sibling_episode_owner() -> InvocationOwnerScope {
    InvocationOwnerScope {
        episode_id: "episode-2".to_owned(),
        ..owner()
    }
}

/// A different run.
pub fn sibling_run_owner() -> InvocationOwnerScope {
    InvocationOwnerScope {
        run_id: "run-2".to_owned(),
        ..owner()
    }
}

pub fn scope_id() -> &'static str {
    "scope-1"
}

pub fn items() -> Vec<String> {
    vec!["objective-1".to_owned(), "strategy-1".to_owned()]
}

/// A scope covering exactly the one admitted invocation.
pub fn current_invocation_scope() -> ContextLifetimeScope {
    ContextLifetimeScope {
        schema: CONTEXT_LIFETIME_SCHEMA.to_owned(),
        scope_id: scope_id().to_owned(),
        owner: owner(),
        applicability: LifetimeApplicability::CurrentInvocation,
        items: items(),
        issued_at: ISSUED_AT,
        ceiling: CEILING,
    }
}

/// A scope covering the admitted invocation plus the next `bound - 1`.
pub fn next_n_scope(bound: u32) -> ContextLifetimeScope {
    ContextLifetimeScope {
        applicability: LifetimeApplicability::NextN { bound },
        ..current_invocation_scope()
    }
}

/// A scope issued without a branch identity, which authorizes the agent in any branch.
pub fn branch_free_scope() -> ContextLifetimeScope {
    ContextLifetimeScope {
        owner: InvocationOwnerScope {
            branch_id: None,
            ..owner()
        },
        ..current_invocation_scope()
    }
}

pub fn invocation(invocation_id: &str) -> LogicalInvocationIdentity {
    LogicalInvocationIdentity {
        owner: owner(),
        invocation_id: invocation_id.to_owned(),
        attempt: 0,
    }
}

/// The same logical invocation re-sent by the transport.
pub fn retry_of(invocation_id: &str, attempt: u32) -> LogicalInvocationIdentity {
    LogicalInvocationIdentity {
        attempt,
        ..invocation(invocation_id)
    }
}

pub fn invocation_by(
    owner: InvocationOwnerScope,
    invocation_id: &str,
) -> LogicalInvocationIdentity {
    LogicalInvocationIdentity {
        owner,
        invocation_id: invocation_id.to_owned(),
        attempt: 0,
    }
}

/// A ledger holding one issued scope.
pub fn ledger_with(scope: ContextLifetimeScope) -> ContextLifetimeLedger {
    let mut ledger = ContextLifetimeLedger::new();
    ledger.issue(scope).expect("scope issues");
    ledger
}
