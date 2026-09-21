// SPDX-License-Identifier: MIT

#![cfg(target_os = "linux")]

//! The provider lane a run may be admitted on, observed at the executable boundary.
//!
//! `STS2_PROVIDER_KIND` used to be a bare string, compared against one spelling in one place. A
//! name no lane implemented was therefore never refused: it fell through the local-bridge branch,
//! ran under the reviewed source revision, and could carry a live episode the operator had only
//! acknowledged on the raw wire. These cases drive the real binary and read the admission it
//! granted from the outside — the refusal it printed, the boundary it never reached, and the
//! durable execution store it never opened.
//!
//! Every case is a process of its own, because the admitted live mode is installed once per
//! process and a lane is admitted while the run is still assembling settings.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

/// The source revision of the reviewed Exo executor, which a lane with no bridge of its own runs.
///
/// Taken from the contract rather than repeated here, so this probe cannot drift from the
/// revision the harness actually pins — and so it carries no revision literal of its own.
const REVIEWED_EXO_REVISION: &str = sts2_harness::EXO_SOURCE_REVISION;

/// A locally launched provider bridge is digest-pinned, so its lane carries a SHA256 instead.
const BRIDGE_DIGEST: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0";

/// Every admitted lane names its provider bridge next, so this is the boundary an admitted
/// declaration reaches and a refused one never does.
const BRIDGE_REQUIREMENT: &str = "STS2_EXO_BRIDGE_BINARY is required";

const UNIMPLEMENTED_KIND_REFUSAL: &str = "is not a provider kind this runtime implements";
const UNNAMED_KIND_REFUSAL: &str = "requires STS2_PROVIDER_KIND to name a kind that declares";
const LIVE_CAPABILITY_REFUSAL: &str = "does not declare live-episode capability";
const RAW_WIRE_REFUSAL: &str = "admitted for a live episode only through the reviewed envelope";
const DIGEST_REFUSAL: &str = "Provider bridge digest or arguments do not match";

/// The declaration a run is launched under.
struct Declaration<'a> {
    kind: Option<&'a str>,
    live_episode: bool,
    admission: Option<&'a str>,
    revision: &'a str,
    bridge: Option<&'a PathBuf>,
    arguments: Option<&'a str>,
    combat_demo: bool,
}

impl<'a> Declaration<'a> {
    fn lane(kind: &'a str) -> Self {
        Self {
            kind: Some(kind),
            live_episode: false,
            admission: None,
            revision: REVIEWED_EXO_REVISION,
            bridge: None,
            arguments: None,
            combat_demo: false,
        }
    }

    fn undeclared() -> Self {
        Self {
            kind: None,
            ..Self::lane("")
        }
    }

    fn live_episode(mut self) -> Self {
        self.live_episode = true;
        self
    }

    fn acknowledged(mut self, admission: &'a str) -> Self {
        self.admission = Some(admission);
        self
    }

    fn digest_pinned(mut self) -> Self {
        self.revision = BRIDGE_DIGEST;
        self
    }

    /// The binary the declaration names, which a local kind pins by digest.
    fn declaring(mut self, bridge: &'a PathBuf) -> Self {
        self.bridge = Some(bridge);
        self
    }

    /// The argument vector the declaration records for that binary.
    fn running(mut self, arguments: &'a str) -> Self {
        self.arguments = Some(arguments);
        self
    }

    /// The revision the declaration pins, for a control that pins the real one.
    fn at(mut self, revision: &'a str) -> Self {
        self.revision = revision;
        self
    }

    /// The bounded mode a local bridge may run in, stated outright as the lane requires.
    fn in_the_combat_demo(mut self) -> Self {
        self.combat_demo = true;
        self
    }
}

/// What one run of the real binary did.
struct Observation {
    output: Output,
    execution_store_opened: bool,
}

impl Observation {
    fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.output.stderr).into_owned()
    }
}

/// A private directory the run's durable state would live in.
struct Scratch {
    root: PathBuf,
    store: PathBuf,
    missing_mcp: PathBuf,
    bridge: PathBuf,
}

impl Scratch {
    fn new(label: &str) -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "sts2-provider-kind-admission-{}-{}-{}",
            label,
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root)
            .map_err(|error| format!("cannot create the scratch directory: {error}"))?;
        Ok(Self {
            store: root.join("execution.sqlite3"),
            missing_mcp: root.join("no-such-mcp-probe"),
            bridge: root.join("declared-provider-bridge"),
            root,
        })
    }

    /// A runtime command whose peers cannot start.
    ///
    /// The MCP locator names a path that does not exist, so a lane admitted past settings assembly
    /// stops at its provider bridge requirement. A run that reaches either boundary cannot be
    /// mistaken for one that refused the declaration.
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_sts2-harness-runtime"));
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("STS2_RUNTIME_PROFILE", "runtime-v3-gameplay")
            .env("STS2_GATEWAY_ADDR", "127.0.0.1:1")
            .env("STS2_GATEWAY_TOKEN", "provider-kind-admission-token")
            .env("STS2_MCP_BINARY", &self.missing_mcp)
            .env("STS2_EXECUTION_STORE_PATH", &self.store)
            .env("STS2_OBJECTIVE", "observe the admitted provider lane");
        command
    }

    fn run(&self, declaration: Declaration<'_>) -> Result<Observation, String> {
        let mut command = self.command();
        command.env("STS2_EXO_REVISION", declaration.revision);
        if let Some(kind) = declaration.kind {
            command.env("STS2_PROVIDER_KIND", kind);
        }
        if declaration.live_episode {
            command.env("STS2_LIVE_EPISODE", "true");
        }
        if let Some(admission) = declaration.admission {
            command.env("STS2_EXO_ADMISSION", admission);
        }
        if let Some(bridge) = declaration.bridge {
            command.env("STS2_EXO_BRIDGE_BINARY", bridge);
        }
        if let Some(arguments) = declaration.arguments {
            command.env("STS2_EXO_BRIDGE_ARGS_JSON", arguments);
        }
        if declaration.combat_demo {
            command.env("STS2_COMBAT_DEMO", "true");
        }
        let output = command
            .output()
            .map_err(|error| format!("cannot run the runtime: {error}"))?;
        Ok(Observation {
            output,
            execution_store_opened: self.store.exists(),
        })
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// A refused declaration never reached the bridge, the peer session or the durable store.
fn assert_refused_before_the_lane_ran(
    observation: &Observation,
    expected: &str,
) -> Result<(), String> {
    let stderr = observation.stderr();
    if observation.output.status.success() {
        return Err(format!(
            "the runtime admitted a lane it must refuse: {stderr}"
        ));
    }
    if !stderr.contains(expected) {
        return Err(format!("the runtime error {stderr:?} omitted {expected:?}"));
    }
    if stderr.contains(BRIDGE_REQUIREMENT) {
        return Err(format!(
            "the run reached the provider bridge boundary before refusing: {stderr}"
        ));
    }
    if observation.execution_store_opened {
        return Err(format!(
            "a refused run opened the durable execution store: {stderr}"
        ));
    }
    match observation.output.status.code() {
        Some(2) => Ok(()),
        code => Err(format!(
            "a refused run must fail with the runtime exit status: {code:?}\n{stderr}"
        )),
    }
}

/// The declared lane was admitted: it went on to require its provider, rather than being refused
/// the lane or the live episode it claims.
fn assert_admitted_past_the_live_gate(observation: &Observation) -> Result<(), String> {
    let stderr = observation.stderr();
    for refusal in [
        UNIMPLEMENTED_KIND_REFUSAL,
        UNNAMED_KIND_REFUSAL,
        LIVE_CAPABILITY_REFUSAL,
        RAW_WIRE_REFUSAL,
    ] {
        if stderr.contains(refusal) {
            return Err(format!("the declared lane was refused: {stderr}"));
        }
    }
    if !stderr.contains(BRIDGE_REQUIREMENT) {
        return Err(format!(
            "the admitted lane did not reach its provider bridge requirement: {stderr}"
        ));
    }
    Ok(())
}

/// A name this runtime does not implement must be refused even when it carries the reviewed
/// revision, which is exactly what let an undeclared lane run as the reviewed executor.
#[test]
fn an_unimplemented_provider_name_is_refused_before_the_lane_runs() -> Result<(), String> {
    let scratch = Scratch::new("unimplemented")?;
    for kind in ["reviewed-exo", "exo-envelope", "OpenAI-Astra"] {
        let observation = scratch.run(Declaration::lane(kind))?;
        assert_refused_before_the_lane_ran(
            &observation,
            &format!("STS2_PROVIDER_KIND {kind} {UNIMPLEMENTED_KIND_REFUSAL}"),
        )?;
    }
    Ok(())
}

/// The Astra lane already ran live episodes, and its declaration is still the whole of its claim.
#[test]
fn the_astra_lane_keeps_the_live_episode_it_already_had() -> Result<(), String> {
    let scratch = Scratch::new("astra-live")?;
    let observation = scratch.run(
        Declaration::lane("openai-astra")
            .live_episode()
            .digest_pinned(),
    )?;
    assert_admitted_past_the_live_gate(&observation)
}

/// The reviewed envelope is what inspects the capability descriptor, so the raw-wire
/// acknowledgement cannot stand in for it.
#[test]
fn the_exo_lane_is_refused_a_live_episode_on_the_raw_wire_acknowledgement() -> Result<(), String> {
    let scratch = Scratch::new("exo-raw-wire")?;
    let observation = scratch.run(
        Declaration::lane("exo")
            .live_episode()
            .acknowledged("legacy"),
    )?;
    assert_refused_before_the_lane_ran(&observation, RAW_WIRE_REFUSAL)
}

/// The same lane, once the envelope inspected its identity, carries a live episode.
#[test]
fn the_exo_lane_is_admitted_a_live_episode_through_the_reviewed_envelope() -> Result<(), String> {
    let scratch = Scratch::new("exo-enveloped")?;
    let observation = scratch.run(
        Declaration::lane("exo")
            .live_episode()
            .acknowledged("envelope"),
    )?;
    assert_admitted_past_the_live_gate(&observation)
}

/// Capability, not the admission mode, refuses a lane that never declared a live episode.
#[test]
fn a_kind_without_the_live_capability_is_refused_on_the_raw_wire_lane() -> Result<(), String> {
    let scratch = Scratch::new("no-capability")?;
    for kind in ["ollama", "typesafe-jev", "synthetic"] {
        let observation = scratch.run(
            Declaration::lane(kind)
                .live_episode()
                .acknowledged("legacy"),
        )?;
        assert_refused_before_the_lane_ran(
            &observation,
            &format!("provider kind {kind} {LIVE_CAPABILITY_REFUSAL}"),
        )?;
    }
    Ok(())
}

/// A live episode must name the lane that claims the capability.
#[test]
fn a_live_episode_with_no_declared_kind_is_refused() -> Result<(), String> {
    let scratch = Scratch::new("unnamed")?;
    let observation = scratch.run(Declaration::undeclared().live_episode())?;
    assert_refused_before_the_lane_ran(&observation, UNNAMED_KIND_REFUSAL)
}

/// The operator's raw-wire probe lane is neither a local bridge nor live-capable: it runs the
/// reviewed source revision, and a live episode is refused on it.
#[test]
fn the_synthetic_probe_lane_stays_non_bridge_and_non_live() -> Result<(), String> {
    let scratch = Scratch::new("synthetic")?;
    assert_admitted_past_the_live_gate(&scratch.run(Declaration::lane("synthetic"))?)?;
    let observation = scratch.run(Declaration::lane("synthetic").live_episode())?;
    assert_refused_before_the_lane_ran(
        &observation,
        &format!("provider kind synthetic {LIVE_CAPABILITY_REFUSAL}"),
    )
}

/// A local kind pins the executable it launches by digest, so bytes that do not hash to the declared
/// revision are refused by that name rather than launched as the operator's bridge.
///
/// The refusal is also shown to be the digest and nothing else: the same declaration, the same file
/// and the same argument vector are not refused when the revision is the one those bytes really have.
#[test]
fn a_declared_bridge_whose_bytes_do_not_match_its_pinned_digest_is_refused() -> Result<(), String> {
    let scratch = Scratch::new("digest")?;
    let bytes = b"not the provider bridge this declaration pins\n";
    fs::write(&scratch.bridge, bytes)
        .map_err(|error| format!("cannot write the declared bridge: {error}"))?;
    let digest = sts2_harness::sha256_hex(bytes);
    if digest.len() != 64 {
        return Err(format!("the computed digest is not a SHA256: {digest}"));
    }
    if digest == BRIDGE_DIGEST {
        return Err(String::from(
            "the fixture happens to hash to the pinned digest, so it cannot show a mismatch",
        ));
    }

    let refused = scratch.run(
        Declaration::lane("typesafe-jev")
            .digest_pinned()
            .declaring(&scratch.bridge)
            .running("[]")
            .in_the_combat_demo(),
    )?;
    assert_refused_before_the_lane_ran(&refused, DIGEST_REFUSAL)?;

    let control = scratch.run(
        Declaration::lane("typesafe-jev")
            .at(&digest)
            .declaring(&scratch.bridge)
            .running("[]")
            .in_the_combat_demo(),
    )?;
    if control.stderr().contains(DIGEST_REFUSAL) {
        return Err(format!(
            "a bridge matching its pinned digest was refused as a mismatch: {}",
            control.stderr()
        ));
    }
    Ok(())
}
