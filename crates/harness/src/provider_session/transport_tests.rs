// SPDX-License-Identifier: MIT

use super::{OwnedNativeTransport, allowlisted_method, fixture_peer_executable};
use crate::provider_session::transport::config::NativeProcessConfig;
use std::fs;

#[test]
fn method_allowlist_is_closed() {
    assert!(allowlisted_method("initialize"));
    assert!(allowlisted_method("thread/start"));
    assert!(allowlisted_method("thread/read"));
    assert!(allowlisted_method("turn/start"));
    assert!(allowlisted_method("turn/interrupt"));
    assert!(allowlisted_method("thread/fork"));
    assert!(allowlisted_method("thread/compact/start"));
    assert!(!allowlisted_method("thread/retire"));
    assert!(!allowlisted_method("shell/execute"));
    assert!(!allowlisted_method("thread/start/../shell"));
}

#[test]
fn forbidden_native_method_is_rejected_before_write() {
    let Ok(mut transport) = OwnedNativeTransport::fixture_peer() else {
        return;
    };
    let state_root = transport.state_root().to_owned();
    assert!(transport.start().is_ok());
    assert!(transport.initialize().is_ok());
    assert_eq!(
        transport.request("shell/execute", serde_json::json!({})),
        Err(super::NativeTransportError::Unsupported)
    );
    assert!(!transport.fenced());
    assert!(transport.close().is_ok());
    let _ = fs::remove_dir_all(state_root);
}

#[cfg(unix)]
#[test]
fn runtime_state_growth_fences_and_stops_owned_peer() {
    let Some(executable) = fixture_peer_executable() else {
        return;
    };
    let state_root = std::env::temp_dir().join(format!(
        "ascension-provider-runtime-quota-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&state_root);
    assert!(fs::create_dir(&state_root).is_ok());
    use std::os::unix::fs::PermissionsExt;
    assert!(fs::set_permissions(&state_root, fs::Permissions::from_mode(0o700)).is_ok());
    // The working directory must also be a private directory; the CI checkout is world-readable.
    let config = NativeProcessConfig::new(
        executable.to_string_lossy().into_owned(),
        Vec::new(),
        state_root.clone(),
        Vec::new(),
        state_root.clone(),
    );
    assert!(config.is_ok());
    let Some(mut config) = config.ok() else {
        return;
    };
    config.state_quota_bytes = 1;
    let mut transport = OwnedNativeTransport::new(config);
    assert!(transport.start().is_ok());
    assert!(transport.initialize().is_ok());
    let marker = state_root.join("growth-marker");
    assert!(fs::write(&marker, [1_u8, 2_u8]).is_ok());
    assert!(fs::set_permissions(&marker, fs::Permissions::from_mode(0o600)).is_ok());
    assert!(matches!(
        transport.start_thread(),
        Err(super::NativeTransportError::Capacity)
    ));
    assert!(transport.fenced());
    assert!(transport.close().is_ok());
    assert!(fs::remove_dir_all(state_root).is_ok());
}
