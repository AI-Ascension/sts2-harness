// SPDX-License-Identifier: MIT

/// True for a recovery code the legal-action read is allowed to carry.
///
/// A refused launch contract reaches this route as `503` carrying a code the game-mod composes from
/// its own refusal prefix plus an optional bounded reason token (`AI-Ascension/sts2-gateway#85`).
/// The admitted set is the producer's vocabulary rather than a second one: see
/// [`is_launch_contract_refusal`].
fn catalog_recovery_code(code: &str) -> bool {
    matches!(
        code,
        "stale_generation" | "host_not_configured" | "host_observation_unavailable"
    ) || is_launch_contract_refusal(code)
}

/// True for the recovery code a refused launch contract carries.
///
/// The mod answers the bare prefix (`launch_contract_refused`) when a reason cannot be named on the
/// wire, and otherwise the prefix, `_`, and one reason token. The token rule is mirrored from the
/// producer so a code it cannot emit is refused here too: widening this set must not admit a
/// neighbouring string that merely starts the same way.
fn is_launch_contract_refusal(code: &str) -> bool {
    const PREFIX: &str = "launch_contract_refused";
    const MAX_REASON_BYTES: usize = 64;
    let Some(reason) = code.strip_prefix(PREFIX) else {
        return false;
    };
    if reason.is_empty() {
        return true;
    }
    let Some(token) = reason.strip_prefix('_') else {
        return false;
    };
    !token.is_empty()
        && token.len() <= MAX_REASON_BYTES
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}
