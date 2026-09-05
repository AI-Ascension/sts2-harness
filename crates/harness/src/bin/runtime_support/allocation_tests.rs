// SPDX-License-Identifier: MIT

// Original synthetic allocation fixtures; no external service or private data.
use super::*;

fn config() -> RuntimeConfig {
    RuntimeConfig {
        gateway_address: "127.0.0.1:1".into(),
        gateway_token: "synthetic-token".into(),
        mcp_binary: "unused".into(),
        instance_id: "instance-1".into(),
        caller_id: "harness".into(),
        session_id: "session-1".into(),
        lease_id: "lease-1".into(),
        lease_epoch: 1,
        mcp_session_id: "mcp-session-1".into(),
    }
}

fn allocation() -> Value {
    json!({"instance_id":"instance-1","caller_id":"harness","session_id":"session-1",
        "lease_id":"lease-2","lease_epoch":2})
}

#[test]
fn changed_allocation_fence_is_used_only_for_cleanup() {
    let config = config();
    let mut calls = 0;
    let result = validate_or_release_allocation(Ok(allocation()), &config, |headers| {
        calls += 1;
        assert_eq!(headers["x-sts2-lease-id"], "lease-2");
        assert_eq!(headers["x-sts2-lease-epoch"], "2");
        assert_eq!(headers["x-sts2-instance-id"], "instance-1");
        assert_eq!(headers["x-sts2-session-id"], "session-1");
        Ok(json!({"status":"released"}))
    });
    assert_eq!(
        result,
        Err("gateway allocation returned unexpected lease_id".into())
    );
    assert_eq!(calls, 1);
    assert_eq!(config.lease_id, "lease-1");
    assert_eq!(config.lease_epoch, 1);
}

#[test]
fn untrusted_allocation_fields_never_replace_the_configured_fence() {
    for (key, invalid) in [
        ("instance_id", json!("foreign")),
        ("caller_id", json!("foreign")),
        ("session_id", json!("foreign")),
        ("lease_id", json!("injected\r\nheader")),
        ("lease_id", json!("")),
        ("lease_id", json!("a".repeat(129))),
        ("lease_id", json!("../lease")),
        ("lease_epoch", json!(0)),
        ("lease_epoch", json!(9_007_199_254_740_992_u64)),
        ("lease_epoch", Value::Null),
    ] {
        let mut allocation = allocation();
        allocation[key] = invalid;
        let mut calls = 0;
        let result = validate_or_release_allocation(Ok(allocation), &config(), |headers| {
            calls += 1;
            assert_eq!(headers["x-sts2-lease-id"], "lease-1");
            assert_eq!(headers["x-sts2-lease-epoch"], "1");
            assert_eq!(headers["x-sts2-instance-id"], "instance-1");
            assert_eq!(headers["x-sts2-session-id"], "session-1");
            Err("gateway returned HTTP 409".into())
        });
        assert!(result.is_err_and(|error| error.contains("allocation cleanup failed")));
        assert_eq!(calls, 1);
    }
}

#[test]
fn unknown_allocation_reports_failed_or_unconfirmed_fenced_cleanup() {
    for cleanup in [
        Err("gateway unavailable".into()),
        Ok(json!({"status":"ready"})),
    ] {
        let result = validate_or_release_allocation(
            Err("gateway response was not JSON".into()),
            &config(),
            |headers| {
                assert_eq!(headers["x-sts2-lease-id"], "lease-1");
                assert_eq!(headers["x-sts2-lease-epoch"], "1");
                cleanup
            },
        );
        assert!(result.is_err_and(|error| {
            error.starts_with("gateway response was not JSON; allocation cleanup")
        }));
    }
}

#[test]
fn matching_allocation_proceeds_without_releasing() {
    let mut allocation = allocation();
    allocation["lease_id"] = json!("lease-1");
    allocation["lease_epoch"] = json!(1);
    let mut released = false;
    let result = validate_or_release_allocation(Ok(allocation), &config(), |_| {
        released = true;
        Ok(json!({"status":"released"}))
    });
    assert_eq!(result, Ok(()));
    assert!(!released);
}
