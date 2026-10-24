// SPDX-License-Identifier: MIT

use std::error::Error;
use std::fs::{self, File};
use std::io::Read;
use std::net::{SocketAddr, TcpListener};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};
use sts2_harness::provider_session::{
    CapabilityProvenance, ContinuityMode, NativeCapabilities, ProviderSessionMetadataStore,
    ProviderSessionMode, ProviderSessionPolicy, ProviderSessionPolicyOwner, SessionScope,
    TransformPolicy,
};
use sts2_harness::{ExoRestrictedProfile, sha256_hex};
use zeroize::Zeroize;

pub(crate) type Result<T = (), E = Box<dyn Error>> = std::result::Result<T, E>;

pub(crate) const INSTANCE_ID: &str = "entry-instance";
pub(crate) const CALLER_ID: &str = "harness";
pub(crate) const SESSION_ID: &str = "entry-session";
pub(crate) const LEASE_ID: &str = "entry-lease";
pub(crate) const LEASE_EPOCH: u64 = 1;
pub(crate) const MCP_SESSION_ID: &str = "entry-mcp-session";
pub(crate) const EXO_REVISION: &str = "b06869ab789dee3f80ca474b5fa89dbe47ccb859";
const NODE_SHA256: &str = "1abce2374a485bddae3c27b17a3e3143e2780232026e627c4fe74ddde3f380a1";
const MAX_CONFIG_BYTES: usize = 32 * 1024;
const MAX_CAPABILITY_BYTES: usize = 8 * 1024;
const MAX_EXO_BRIDGE_BYTES: usize = 256 * 1024 * 1024;

#[path = "served_synthetic_child_process.rs"]
mod child_process;
#[path = "exo_lifecycle_runtime_entry_peers.rs"]
mod peers;
pub(crate) use child_process::{
    OwnedService, ServiceBindings, bounded_child_output, service_command, wait_for_service,
};

pub(crate) struct TestKeys {
    policy: [u8; 32],
    context: [u8; 32],
}

impl TestKeys {
    pub(crate) fn new() -> Result<Self> {
        let mut keys = Self {
            policy: [0_u8; 32],
            context: [0_u8; 32],
        };
        keys.policy = fresh_key(&[0_u8; 32])?;
        keys.context = fresh_key(&keys.policy)?;
        Ok(keys)
    }

    pub(crate) fn policy_key(&self) -> [u8; 32] {
        self.policy
    }

    pub(crate) fn policy_hex(&self) -> String {
        hex_key(&self.policy)
    }

    pub(crate) fn context_hex(&self) -> String {
        hex_key(&self.context)
    }
}

impl Drop for TestKeys {
    fn drop(&mut self) {
        self.policy.zeroize();
        self.context.zeroize();
    }
}

fn fresh_key(exclude: &[u8; 32]) -> Result<[u8; 32]> {
    for _ in 0..16 {
        let mut key = [0_u8; 32];
        if let Err(error) = getrandom::fill(&mut key) {
            key.zeroize();
            return Err(error.into());
        }
        if key != [0_u8; 32] && key != *exclude {
            return Ok(key);
        }
        key.zeroize();
    }
    Err("fresh nonzero synthetic key generation failed".into())
}

fn hex_key(key: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(64);
    for byte in key {
        output.push(char::from(HEX[(*byte >> 4) as usize]));
        output.push(char::from(HEX[(*byte & 0x0f) as usize]));
    }
    output
}

pub(crate) struct PrivateRoot {
    pub(crate) path: PathBuf,
}

pub(crate) fn context_owner_config(root: &PrivateRoot) -> Result<String> {
    Ok(serde_json::to_string(&json!({
        "schema_version":"ascension.workflow-context-owner-config.v1",
        "store_path":root.path.join("context-owner.sqlite3"),
        "key_reference":"STS2_SERVED_CONTEXT_OWNER_KEY",
        "owner_id":"fixture-owner","owner_version":"1","context_ref":"context.live.v1",
        "limits":{"max_items":64,"max_notes":16,"max_context_bytes":131072,"max_objective_bytes":512,"max_control_events":64},
        "render_required":false,"sources":[],"membership":null
    }))?)
}

pub(crate) fn policy_configuration(
    root: &PrivateRoot,
    run_id: &str,
    caps: &NativeCapabilities,
) -> Result<String> {
    Ok(serde_json::to_string(&json!({
        "schema_version":"ascension.workflow-provider-policy-config.v1",
        "store_path":root.path.join("provider-policy.sqlite3"),
        "key_reference":"STS2_SERVED_PROVIDER_POLICY_KEY",
        "scope":{"project_id":"fixture.project","run_id":run_id,"episode_id":"fixture.episode","agent_id":"fixture.agent"},
        "capabilities":caps,
        "selected_profile":caps.profile_id
    }))?)
}

pub(crate) fn unused_loopback_address() -> Result<SocketAddr> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    drop(listener);
    Ok(address)
}

impl PrivateRoot {
    pub(crate) fn new() -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "h391-served-synthetic-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        Ok(Self { path })
    }

    pub(crate) fn write_private(&self, name: &str, bytes: &[u8]) -> Result<PathBuf> {
        let path = self.path.join(name);
        fs::write(&path, bytes)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        Ok(path)
    }

    pub(crate) fn cleanup(&self) -> Result<()> {
        fs::remove_dir_all(&self.path)?;
        Ok(())
    }
}

impl Drop for PrivateRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub(crate) struct PreparedExo {
    pub(crate) capabilities: NativeCapabilities,
    pub(crate) config_path: PathBuf,
    pub(crate) config_digest: String,
    pub(crate) bridge: PathBuf,
    pub(crate) executor: PathBuf,
    pub(crate) extension_digest: String,
    pub(crate) endpoint: String,
    pub(crate) private_state_root: PathBuf,
}

fn bounded_file_digest(path: &Path, maximum: usize, executable: bool) -> Result<String> {
    if !path.is_absolute() {
        return Err("pinned artifact path is not absolute".into());
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err("pinned artifact is not a regular non-symlink file".into());
    }
    if executable && metadata.permissions().mode() & 0o111 == 0 {
        return Err("pinned artifact is not executable".into());
    }
    if fs::canonicalize(path)? != path || metadata.len() > maximum as u64 {
        return Err("pinned artifact path or size is outside the fixture bound".into());
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(path)?
        .take(maximum as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > maximum {
        return Err("pinned artifact exceeds the fixture bound".into());
    }
    Ok(sha256_hex(bytes))
}

pub(crate) fn prepare_exo(
    root: &PrivateRoot,
    source: &Path,
    node: &Path,
    executor: &Path,
    endpoint: &str,
) -> Result<PreparedExo> {
    let source = fs::canonicalize(source)?;
    let node = node.to_path_buf();
    let executor = executor.to_path_buf();
    if bounded_file_digest(&node, 256 * 1024 * 1024, true)? != NODE_SHA256 {
        return Err("Node bytes do not match the qualified 22.14.0 pin".into());
    }
    let mut node_version = Command::new(&node);
    node_version
        .arg("--version")
        .env_clear()
        .env("PATH", "/usr/bin:/bin");
    let (node_status, node_output) = bounded_child_output(node_version, 128)?;
    if !node_status.success() || node_output != b"v22.14.0\n" {
        return Err("Node version does not match the qualified runtime".into());
    }
    let executor_digest = bounded_file_digest(&executor, 128 * 1024 * 1024, true)?;
    let bridge = PathBuf::from(env!("CARGO_BIN_EXE_sts2-exo-bridge"));
    // Validate the prepared executable here; service_command rechecks its bytes before launch.
    drop(bounded_file_digest(&bridge, MAX_EXO_BRIDGE_BYTES, true)?);
    let extension = fs::canonicalize(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../experiments/exo-agent/extension/src/index.ts"),
    )?;
    let extension_digest = bounded_file_digest(&extension, 1024 * 1024, false)?;
    let state_base = root.path.join("exo-private");
    fs::create_dir(&state_base)?;
    fs::set_permissions(&state_base, fs::Permissions::from_mode(0o700))?;
    let restricted = ExoRestrictedProfile::reviewed_private(&state_base);
    restricted.validate()?;
    for path in [
        &restricted.state.state_root,
        &restricted.state.cache_root,
        &restricted.state.temp_root,
    ] {
        fs::create_dir(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    let endpoint = endpoint.to_owned();
    let config = json!({
        "schema":"sts2.exo-one-shot-config-v2",
        "executor":executor,
        "executor_sha256":executor_digest,
        "source_root":source,
        "extension":extension,
        "extension_sha256":extension_digest,
        "node":node,
        "node_sha256":NODE_SHA256,
        "model":"o3-pro",
        "endpoint":endpoint,
        "private_state":restricted.state
    });
    let config_bytes = serde_json::to_vec(&config)?;
    if config_bytes.len() > MAX_CONFIG_BYTES {
        return Err("guarded Exo configuration exceeds 32 KiB".into());
    }
    let config_digest = sha256_hex(&config_bytes);
    let config_path = root.write_private("guarded-exo-v2.json", &config_bytes)?;
    let mut inspect = Command::new(&bridge);
    inspect
        .args([
            "--synthetic-provider-capabilities",
            config_path
                .to_str()
                .ok_or("configuration path is not UTF-8")?,
            &config_digest,
            INSTANCE_ID,
        ])
        .env_clear()
        .env("PATH", "/usr/bin:/bin");
    let (status, bytes) = bounded_child_output(inspect, MAX_CAPABILITY_BYTES)?;
    if !status.success() {
        return Err("guarded synthetic capability inspection refused".into());
    }
    let capabilities: NativeCapabilities = serde_json::from_slice(&bytes)?;
    capabilities.validate()?;
    if capabilities.profile_id != "sts2-exo-synthetic-one-shot-v1"
        || capabilities.provenance != CapabilityProvenance::SchemaOnly
        || capabilities.enabled_methods != ["turn/start"]
        || capabilities.native_binary_sha256 != executor_digest
    {
        return Err("synthetic capability descriptor does not match the inspected profile".into());
    }
    Ok(PreparedExo {
        capabilities,
        config_path,
        config_digest,
        bridge,
        executor,
        extension_digest,
        endpoint,
        private_state_root: state_base,
    })
}

pub(crate) fn reject_config_bytes(
    root: &PrivateRoot,
    bridge: &Path,
    bytes: &[u8],
    digest: &str,
) -> Result<bool> {
    let path = root.write_private("rejected-exo-config.json", bytes)?;
    let mut command = Command::new(bridge);
    command
        .args([
            "--synthetic-provider-capabilities",
            path.to_str().ok_or("configuration path is not UTF-8")?,
            digest,
            INSTANCE_ID,
        ])
        .env_clear()
        .env("PATH", "/usr/bin:/bin");
    let (status, _) = bounded_child_output(command, MAX_CAPABILITY_BYTES)?;
    Ok(!status.success())
}

pub(crate) fn reject_invalid_config_examples(root: &PrivateRoot, exo: &PreparedExo) -> Result<()> {
    let config_bytes = fs::read(&exo.config_path)?;
    let duplicate = format!(
        "{{\"schema\":\"sts2.exo-one-shot-config-v2\",{}",
        std::str::from_utf8(&config_bytes)?
            .strip_prefix('{')
            .ok_or("guarded config was not an object")?
    );
    if !reject_config_bytes(
        root,
        &exo.bridge,
        duplicate.as_bytes(),
        &sts2_harness::sha256_hex(duplicate.as_bytes()),
    )? {
        return Err("duplicate guarded-config key was accepted".into());
    }
    let mut unknown: Value = serde_json::from_slice(&config_bytes)?;
    unknown["unknown_fixture_field"] = json!(true);
    let unknown_bytes = serde_json::to_vec(&unknown)?;
    if !reject_config_bytes(
        root,
        &exo.bridge,
        &unknown_bytes,
        &sts2_harness::sha256_hex(&unknown_bytes),
    )? {
        return Err("unknown guarded-config field was accepted".into());
    }
    let oversized = vec![b' '; 32 * 1024 + 1];
    if !reject_config_bytes(
        root,
        &exo.bridge,
        &oversized,
        &sts2_harness::sha256_hex(&oversized),
    )? {
        return Err("oversized guarded config was accepted".into());
    }
    if !reject_config_bytes(root, &exo.bridge, &config_bytes, &"0".repeat(64))? {
        return Err("guarded config accepted a mismatched raw-byte digest".into());
    }
    Ok(())
}

pub(crate) fn seed_policy(
    root: &PrivateRoot,
    run_id: &str,
    caps: &NativeCapabilities,
    keys: &TestKeys,
) -> Result<()> {
    let scope = SessionScope::new(
        "fixture.project",
        run_id,
        "fixture.episode",
        "fixture.agent",
    )?;
    let store = ProviderSessionMetadataStore::encrypted(
        root.path.join("provider-policy.sqlite3"),
        keys.policy_key(),
        scope.clone(),
    )?;
    let owner = ProviderSessionPolicyOwner::open(store, scope.clone(), caps.clone())?;
    let mut policy = ProviderSessionPolicy::disabled(scope);
    policy.policy_id = "fixture.policy.one-turn".to_owned();
    policy.mode = ProviderSessionMode::FixtureOnly;
    policy.continuity = ContinuityMode::StrictReviewed;
    policy.credential_realm_ref = "fixture.realm.local".to_owned();
    policy.profile_sha256 = caps.profile_sha256.clone();
    policy.max_completed_turns = 1;
    policy.history_ttl_seconds = 1;
    policy.automatic_transform_policy = TransformPolicy::DenyAndFence;
    policy
        .admit_for_profile(caps)
        .map_err(|_| "synthetic one-turn policy did not fit the inspected profile")?;
    let digest = owner.import(serde_json::to_vec(&policy)?)?;
    owner.adopt_imported(&digest, 2)?;
    drop(owner);
    Ok(())
}

pub(crate) fn mcp_script(root: &PrivateRoot) -> Result<(PathBuf, PathBuf)> {
    // Type-check the frozen peer's old handler without invoking its unbounded socket path.
    let _legacy_gateway_shape: fn(TcpListener) -> std::result::Result<(), String> =
        peers::serve_gateway;
    let action_log = root.path.join("mcp-action.json");
    let script =
        peers::mcp_script(&action_log).map_err(|_| "cannot create controlled MCP fixture")?;
    let binary = root.write_private("controlled-mcp.py", script.as_bytes())?;
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700))?;
    let peer_log = root.path.join("mcp-action.peer.log");
    Ok((binary, peer_log))
}
