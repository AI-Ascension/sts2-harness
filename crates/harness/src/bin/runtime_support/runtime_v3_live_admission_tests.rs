// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic)]

use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use sts2_harness::exo_admission::ExoAdmissionMode;

use super::super::provider::ProviderKind;
use super::{LiveMode, admitted_live_episode, declared, install, resolve, resolve_declared};

const CHILD_TEST: &str =
    "runtime_support::runtime_v3_settings::live_admission::tests::live_admission_child";
const CHILD_MODE: &str = "STS2_LIVE_ADMISSION_TEST_CHILD";

/// Runs one case in a fresh process, because the admitted mode is installed once per process.
///
/// `declared_live_episode` is the *ambient* variable the child is given, not the mode: the two
/// child cases exist to show those are different things.
fn run_child(mode: &str, declared_live_episode: Option<&str>) -> Output {
    let mut command = Command::new(std::env::current_exe().expect("current test binary"));
    command
        .arg("--exact")
        .arg(CHILD_TEST)
        .arg("--nocapture")
        .env(CHILD_MODE, mode);
    match declared_live_episode {
        Some(value) => command.env("STS2_LIVE_EPISODE", value),
        None => command.env_remove("STS2_LIVE_EPISODE"),
    };
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn the isolated admission child");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if child
            .try_wait()
            .expect("poll the admission child")
            .is_some()
        {
            return child
                .wait_with_output()
                .expect("collect the admission child");
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("the admission child exceeded its bounded test deadline");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn assert_child_completed(output: &Output) {
    assert!(
        output.status.success(),
        "the isolated admission child failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Both child cases, in one place, so each fact is observed in a process of its own.
#[test]
fn live_admission_child() {
    match std::env::var(CHILD_MODE).as_deref() {
        Ok("declaration_is_not_admission") => {
            // The ambient declaration is present, and the lane is still not live: nothing admitted
            // it, so the process's replay stream and live diagnostics stay off.
            assert_eq!(declared(), Ok(true));
            assert!(!admitted_live_episode());
        }
        Ok("admitted_mode_is_installed_once") => {
            // The ambient declaration is absent, and the run is live anyway, because the admitted
            // record — not the variable — decides. Re-resolving the same mode is idempotent; a
            // contradicting resolution is refused rather than allowed to re-decide the process.
            assert_eq!(std::env::var("STS2_LIVE_EPISODE").ok(), None);
            assert_eq!(declared(), Ok(false));
            install(LiveMode::LiveEpisode).expect("a fresh process installs the admitted mode");
            install(LiveMode::LiveEpisode).expect("the same mode is already installed");
            assert!(install(LiveMode::Standard).is_err());
            assert!(admitted_live_episode());
        }
        _ => {}
    }
}

/// A live-episode declaration on its own admits nothing, however the process was launched.
#[test]
fn a_declaration_alone_does_not_admit_live_behavior() {
    assert_child_completed(&run_child("declaration_is_not_admission", Some("true")));
}

/// Live behavior follows the admitted mode even when the ambient variable says otherwise.
///
/// This is the check that fails if the production connection is removed: the recording stream and
/// the live diagnostics read the record that settings assembly installs, so a build that went back
/// to reading `STS2_LIVE_EPISODE` at the point of use would report standard here.
#[test]
fn the_admitted_mode_decides_live_behavior_without_the_ambient_variable() {
    assert_child_completed(&run_child("admitted_mode_is_installed_once", None));
}

#[test]
fn an_absent_declaration_is_standard_for_every_kind() {
    for kind in [
        None,
        Some(ProviderKind::OpenAstra),
        Some(ProviderKind::Ollama),
        Some(ProviderKind::TypesafeJev),
        Some(ProviderKind::Exo),
        Some(ProviderKind::Synthetic),
    ] {
        // Both admission modes: not declaring a live episode is not a capability question.
        for admission in [ExoAdmissionMode::Enveloped, ExoAdmissionMode::Legacy] {
            assert_eq!(resolve(kind, false, admission), Ok(LiveMode::Standard));
        }
    }
}

/// Capability, not the name, decides: a kind that does not declare live episodes is refused.
#[test]
fn a_kind_without_the_live_capability_cannot_run_a_live_episode() {
    for kind in [
        ProviderKind::Ollama,
        ProviderKind::TypesafeJev,
        ProviderKind::Synthetic,
    ] {
        let error = resolve(Some(kind), true, ExoAdmissionMode::Legacy)
            .expect_err("a kind without live capability must be refused");
        assert!(
            error.contains(kind.name()) && error.contains("live-episode capability"),
            "{kind:?} must be refused by capability: {error}"
        );
    }
    // Naming no kind at all cannot borrow one either.
    let error = resolve(None, true, ExoAdmissionMode::Enveloped)
        .expect_err("a live episode must name a kind that declares it");
    assert!(error.contains("STS2_PROVIDER_KIND"), "{error}");
}

/// The Astra lane's live admission is unchanged, which is what its live runs already rely on.
#[test]
fn the_astra_lane_keeps_its_live_admission_on_the_raw_wire_lane() {
    for admission in [ExoAdmissionMode::Legacy, ExoAdmissionMode::Enveloped] {
        assert_eq!(
            resolve(Some(ProviderKind::OpenAstra), true, admission),
            Ok(LiveMode::LiveEpisode)
        );
    }
}

/// The Exo lane is live-capable, but only once the reviewed envelope inspected its capability.
#[test]
fn the_exo_lane_is_live_only_through_the_reviewed_envelope() {
    assert_eq!(
        resolve(Some(ProviderKind::Exo), true, ExoAdmissionMode::Enveloped),
        Ok(LiveMode::LiveEpisode)
    );
    let error = resolve(Some(ProviderKind::Exo), true, ExoAdmissionMode::Legacy)
        .expect_err("the raw-wire acknowledgement does not inspect the Exo descriptor");
    assert!(
        error.contains("exo") && error.contains("reviewed envelope"),
        "{error}"
    );
}

/// A lane whose live claim rests on its own pinned identity resolves from the declaration alone.
#[test]
fn a_lane_without_a_decision_provider_resolves_from_its_declaration() {
    assert_eq!(resolve_declared(false), LiveMode::Standard);
    assert_eq!(resolve_declared(true), LiveMode::LiveEpisode);
}
