// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use super::RuntimeConfig;

/// Canonical gateway identity headers for one request.
///
/// Every fenced gateway route derives caller, session, instance, and lease
/// identity from these headers, so they are built in exactly one place.
pub(crate) fn identity_headers(
    config: &RuntimeConfig,
    correlation: &str,
) -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            String::from("x-sts2-instance-id"),
            config.instance_id.clone(),
        ),
        (String::from("x-sts2-caller-id"), config.caller_id.clone()),
        (String::from("x-sts2-session-id"), config.session_id.clone()),
        (
            String::from("x-mcp-session-id"),
            config.mcp_session_id.clone(),
        ),
        (String::from("x-sts2-lease-id"), config.lease_id.clone()),
        (
            String::from("x-sts2-lease-epoch"),
            config.lease_epoch.to_string(),
        ),
        (
            String::from("x-sts2-correlation-id"),
            String::from(correlation),
        ),
    ])
}
