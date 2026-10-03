// SPDX-License-Identifier: MIT

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RpcReadKind {
    None,
    Catalog,
    Recovery,
    /// A gameplay read that is not itself a recovery operation, but whose
    /// transport failures are still transient: the gateway may be briefly
    /// unavailable (`-32003`) or time out (`-32008`) while the owner brings an
    /// episode up. This enables *only* the transient classification; unlike
    /// `Recovery` it does not widen which response envelopes are accepted, so
    /// `sts2.observe` cannot come to accept a watchdog-recovery envelope.
    Gameplay,
}

/// Classify a dispatch by whether it is a read whose transient gateway faults
/// may be retried. `sts2.legal_actions` is a catalog read and `sts2.observe` is a
/// gameplay read; every other dispatch, including writes and plain RPCs, keeps
/// the terminal behaviour.
fn read_kind_for_dispatch(method: &str, params: &Value) -> RpcReadKind {
    if method != "tools/call" {
        return RpcReadKind::None;
    }
    match params["name"].as_str() {
        Some("sts2.legal_actions") => RpcReadKind::Catalog,
        Some("sts2.observe") => RpcReadKind::Gameplay,
        // `sts2.reobserve` and `sts2.coop_receipt_query` are dispatched by the
        // call site through `rpc_call_recovery_read`, which is itself a read that
        // classifies transient gateway faults. Naming them here keeps this
        // function the single source of truth for "is this dispatch a read".
        Some("sts2.reobserve" | "sts2.coop_receipt_query") => RpcReadKind::Recovery,
        _ => RpcReadKind::None,
    }
}

/// Whether a read kind treats a transient gateway fault as recoverable rather
/// than terminal. Every kind except `None` classifies these; `None` is a
/// dispatch or write whose failure must not be silently retried as a read.
fn classifies_transient_gateway_faults(read_kind: RpcReadKind) -> bool {
    !matches!(read_kind, RpcReadKind::None)
}

/// Whether a `tools/call` dispatch classifies transient gateway faults, i.e.
/// whether it is dispatched as a read at all.
///
/// The caller must use this — not the narrower recovery-envelope predicate — to
/// decide whether a wire-level transient failure survives as retryable. Using
/// the recovery predicate alone would leave `sts2.observe` (a gameplay read that
/// widens no envelope) with terminal classification, which is exactly the
/// condition the owner's launch fence turns into `live_launch_fence_failed`.
pub(super) fn dispatch_classifies_transient_faults(name: &str) -> bool {
    classifies_transient_gateway_faults(read_kind_for_dispatch(
        "tools/call",
        &json!({ "name": name }),
    ))
}
