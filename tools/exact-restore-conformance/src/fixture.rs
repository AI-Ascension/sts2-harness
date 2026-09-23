// SPDX-License-Identifier: MIT

#[path = "fixture_support.rs"]
mod support;
#[path = "fixture_transport.rs"]
mod transport;
pub(crate) use support::Paths;
pub(crate) use support::run_case;
pub(crate) use support::write_binary_provenance;
use transport::{
    base64, bootstrap, free_port, hex, host_fence, required_string, required_u64, run_harness,
};

use crate::http;
use crate::process::PeerProcess;
use crate::proxy::AllocationProxy;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use uuid::Uuid;

const RECOVERY_SCHEMA: &str = "fb934d3157485aaf6e13e6ebbb213ec8a14c7fc6f5eeebc06b7a22c1f0009217";
const RUNTIME_V3_SCHEMA: &str = "daa216902d3211b9537924105b27e7718dd93dec82969a3c550131a27147c06b";
const ZERO_DIGEST: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn run_case_inner(paths: &Paths, outcome: &str, root: &Path) -> Result<(), String> {
    let caller = Uuid::new_v4().to_string();
    let instance = Uuid::new_v4().to_string();
    let session = Uuid::new_v4().to_string();
    let deployment = Uuid::new_v4().to_string();
    let incarnation = Uuid::new_v4().to_string();
    let boot = Uuid::new_v4().to_string();
    let lease = Uuid::new_v4().to_string();
    let initial_host_fence = Uuid::new_v4().to_string();
    let gateway_token = format!("gateway-{}", Uuid::new_v4());
    let mod_token = format!("mod-{}", Uuid::new_v4());
    let mut host_key_bytes = [0_u8; 32];
    host_key_bytes[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    host_key_bytes[16..].copy_from_slice(Uuid::new_v4().as_bytes());
    let host_key = hex(&host_key_bytes);
    let bootstrap_secret = {
        let mut bytes = [0_u8; 32];
        bytes[..16].copy_from_slice(Uuid::new_v4().as_bytes());
        bytes[16..].copy_from_slice(Uuid::new_v4().as_bytes());
        base64(&bytes)
    };
    let gateway_backend = format!("127.0.0.1:{}", free_port()?);
    let mod_address = format!("127.0.0.1:{}", free_port()?);
    let exact_store = root.join("exact-store");
    let recovery_store = root.join("recovery-store.sqlite");
    support::create_private_store(&exact_store)?;
    let owner_path = root.join("owner-initial.json");
    let owner = json!({
        "fence": {
            "deployment_id": deployment, "instance_id": instance,
            "instance_incarnation": incarnation, "boot_id": boot,
            "authority_generation": 1, "host_fence_id": initial_host_fence,
            "host_fence_generation": 1, "lease_id": lease, "lease_epoch": 1,
            "session_id": session, "lease_expires_at_millis": 4_000_000_000_000_u64
        },
        "observed_at_millis": 2_000_000_000_000_u64
    });
    fs::write(
        &owner_path,
        serde_json::to_vec(&owner).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("write owner: {error}"))?;
    let mut mod_env = vec![
        ("STS2_MOD_ADDR".into(), mod_address.clone()),
        ("STS2_MOD_TOKEN".into(), mod_token.clone()),
        ("STS2_CALLER_ID".into(), caller.clone()),
        ("STS2_INSTANCE_ID".into(), instance.clone()),
        ("STS2_SESSION_ID".into(), session.clone()),
        ("STS2_LEASE_ID".into(), lease.clone()),
        ("STS2_LEASE_EPOCH".into(), "1".into()),
        (
            "STS2_EXACT_OWNER_FILE".into(),
            owner_path.display().to_string(),
        ),
        ("STS2_EXACT_STORE".into(), exact_store.display().to_string()),
        ("STS2_RUNTIME_HOST_LEASE_KEY".into(), host_key.clone()),
    ];
    if outcome == "refused" {
        mod_env.push(("STS2_EXACT_NATIVE_UNSUPPORTED".into(), "1".into()));
    } else if outcome == "unknown" {
        mod_env.push(("STS2_EXACT_COMMIT_UNKNOWN".into(), "1".into()));
    }
    let gateway_env = vec![
        ("STS2_GATEWAY_ADDR".into(), gateway_backend.clone()),
        ("STS2_MOD_ADDR".into(), mod_address.clone()),
        ("STS2_MOD_TOKEN".into(), mod_token),
        ("STS2_GATEWAY_TOKEN".into(), gateway_token.clone()),
        (
            "STS2_GATEWAY_TOKEN_SCOPE".into(),
            "read,mutate,control".into(),
        ),
        ("STS2_RECOVERY_TOKEN".into(), gateway_token.clone()),
        (
            "STS2_RECOVERY_TOKEN_SCOPE".into(),
            "read,mutate,control".into(),
        ),
        ("STS2_RUNTIME_PROFILE".into(), "watchdog-recovery".into()),
        (
            "STS2_RECOVERY_STORE".into(),
            recovery_store.display().to_string(),
        ),
        ("STS2_DEPLOYMENT_ID".into(), deployment.clone()),
        ("STS2_INSTANCE_ID".into(), instance.clone()),
        ("STS2_CALLER_ID".into(), caller.clone()),
        ("STS2_RECOVERY_PRINCIPAL_ID".into(), caller.clone()),
        ("STS2_SESSION_ID".into(), session.clone()),
        ("STS2_MCP_SESSION_ID".into(), "exact-restore-mcp".into()),
        ("STS2_LEASE_ID".into(), lease),
        ("STS2_LEASE_EPOCH".into(), "1".into()),
        ("STS2_RUNTIME_HOST_PRINCIPAL_ID".into(), caller.clone()),
        ("STS2_RUNTIME_HOST_LEASE_KEY".into(), host_key),
        (
            "STS2_RUNTIME_BOOTSTRAP_SECRET".into(),
            bootstrap_secret.clone(),
        ),
        (
            "STS2_RUNTIME_RECOVERY_READ_SECRET".into(),
            bootstrap_secret.clone(),
        ),
        (
            "STS2_RUNTIME_RECOVERY_RECONCILE_SECRET".into(),
            bootstrap_secret,
        ),
        ("STS2_RECOVERY_LEASE_TTL_SECONDS".into(), "30".into()),
        (
            "STS2_RECOVERY_LEASE_RENEWAL_INTERVAL_SECONDS".into(),
            "10".into(),
        ),
        (
            "STS2_RECOVERY_RUNTIME_V3_SCHEMA_DIGEST".into(),
            RUNTIME_V3_SCHEMA.into(),
        ),
    ];
    let mod_log = root.join("mod.log");
    let mut mod_process = PeerProcess::spawn(&paths.mod_bin, &mod_env, &mod_log)?;
    mod_process.wait_tcp(&mod_address).map_err(|error| {
        format!(
            "{error}; mod log={}",
            fs::read_to_string(&mod_log).unwrap_or_default()
        )
    })?;
    let mut gateway_process =
        PeerProcess::spawn(&paths.gateway_bin, &gateway_env, &root.join("gateway.log"))?;
    gateway_process.wait_tcp(&gateway_backend)?;
    let boot = bootstrap(
        &gateway_backend,
        &gateway_token,
        &caller,
        &deployment,
        &instance,
        &incarnation,
    )?;
    host_fence(&gateway_backend, &gateway_token, &caller, &boot)?;
    let (allocation_status, allocation) = http::post(
        &gateway_backend,
        "/v1/sessions/allocate",
        &gateway_token,
        None,
        &json!({"instance_id": instance, "caller_id": caller, "session_id": session}),
    )?;
    if allocation_status != 200 {
        return Err(format!(
            "allocation failed: {allocation_status} {allocation}"
        ));
    }
    if allocation["recovery_authority"].is_null() {
        return Err(format!(
            "allocation omitted recovery authority: {allocation}"
        ));
    }
    let allocation_proxy = AllocationProxy::start(&gateway_backend, allocation.clone())?;
    let gateway_frontend = allocation_proxy.address().to_owned();
    let authority = &allocation["recovery_authority"];
    let context = authority["context"].clone();
    let fence = authority["current_fence"].clone();
    let owner = json!({"fence": {
        "deployment_id": required_string(&context, "deployment_id")?,
        "instance_id": required_string(&context, "instance_id")?,
        "instance_incarnation": required_string(&context, "instance_incarnation")?,
        "boot_id": required_string(&context, "boot_id")?,
        "authority_generation": required_u64(&context, "authority_generation")?,
        "host_fence_id": required_string(&fence, "host_fence_id")?,
        "host_fence_generation": required_u64(&fence, "fence_generation")?,
        "lease_id": required_string(&context, "lease_id")?,
        "lease_epoch": required_u64(&context, "lease_epoch")?,
        "session_id": session, "lease_expires_at_millis": allocation["expires_at_millis"]
    }});
    let expected_owner = serde_json::to_string(&owner).map_err(|error| error.to_string())?;
    let mut test_env = vec![
        (
            "STS2_EXACT_HARNESS_BINARY".into(),
            paths.harness_bin.display().to_string(),
        ),
        (
            "STS2_EXACT_MCP_BINARY".into(),
            paths.mcp_bin.display().to_string(),
        ),
        ("STS2_EXACT_GATEWAY_ADDR".into(), gateway_frontend),
        ("STS2_EXACT_OWNER_JSON".into(), expected_owner),
        ("STS2_EXACT_GATEWAY_TOKEN".into(), gateway_token.clone()),
        ("STS2_EXACT_RECOVERY_TOKEN".into(), gateway_token),
        ("STS2_EXACT_RECOVERY_PRINCIPAL_ID".into(), caller.clone()),
        (
            "STS2_EXACT_RECOVERY_DEPLOYMENT_ID".into(),
            required_string(&context, "deployment_id")?,
        ),
        (
            "STS2_EXACT_RECOVERY_INSTANCE_ID".into(),
            required_string(&context, "instance_id")?,
        ),
        (
            "STS2_EXACT_RECOVERY_INSTANCE_INCAR".into(),
            required_string(&context, "instance_incarnation")?,
        ),
        (
            "STS2_EXACT_RECOVERY_BOOT_ID".into(),
            required_string(&context, "boot_id")?,
        ),
        (
            "STS2_EXACT_RECOVERY_AUTHORITY_GENERATION".into(),
            required_u64(&context, "authority_generation")?.to_string(),
        ),
        (
            "STS2_EXACT_RECOVERY_LEASE_ID".into(),
            required_string(&context, "lease_id")?,
        ),
        (
            "STS2_EXACT_RECOVERY_LEASE_EPOCH".into(),
            required_u64(&context, "lease_epoch")?.to_string(),
        ),
        (
            "STS2_EXACT_RECOVERY_CURRENT_FENCE_JSON".into(),
            fence.to_string(),
        ),
        ("STS2_EXACT_INSTANCE_ID".into(), instance),
        ("STS2_EXACT_CALLER_ID".into(), caller),
        ("STS2_EXACT_SESSION_ID".into(), session),
        (
            "STS2_EXACT_LEASE_ID".into(),
            required_string(&context, "lease_id")?,
        ),
        (
            "STS2_EXACT_LEASE_EPOCH".into(),
            required_u64(&context, "lease_epoch")?.to_string(),
        ),
        ("STS2_EXACT_EXPECTED_OUTCOME".into(), outcome.into()),
        ("STS2_EXACT_STORE".into(), exact_store.display().to_string()),
    ];
    if outcome == "refused" {
        test_env.push(("STS2_EXACT_NATIVE_UNSUPPORTED".into(), "1".into()));
    } else if outcome == "unknown" {
        test_env.push(("STS2_EXACT_COMMIT_UNKNOWN".into(), "1".into()));
    }
    let log = root.join("harness.log");
    let status = run_harness(paths, &test_env, &log)?;
    let output = fs::read_to_string(&log).unwrap_or_default();
    if status != 0 || !output.contains("test result: ok. 1 passed; 0 failed;") {
        return Err(format!(
            "exact case did not execute one passing test (status {status}): {output}"
        ));
    }
    let counter_path = exact_store.join("synthetic-output/synthetic-applier-counter");
    let counter = match fs::read_to_string(&counter_path) {
        Ok(value) => value
            .trim()
            .parse::<u64>()
            .map_err(|error| format!("synthetic effect count is invalid: {error}"))?,
        Err(error) if outcome == "refused" && error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(format!("read synthetic effect count: {error}")),
    };
    let expected = u64::from(outcome != "refused");
    if counter != expected {
        return Err(format!(
            "synthetic effect count was {counter}, expected {expected}"
        ));
    }
    if outcome == "positive" {
        let ledger: Value = serde_json::from_slice(
            &fs::read(exact_store.join("runtime-v3-effects.json"))
                .map_err(|error| format!("read positive effects: {error}"))?,
        )
        .map_err(|error| format!("decode positive effects: {error}"))?;
        if ledger["action_count"] != 1 || ledger["settled_count"] != 1 {
            return Err(format!(
                "positive action/settlement evidence was invalid: {ledger}"
            ));
        }
    }
    Ok(())
}
