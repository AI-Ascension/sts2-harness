// SPDX-License-Identifier: MIT

// Hand-authored MIT synthetic cold-launch fixtures. No game files, real profiles, native seeds,
// provider payloads or private identifiers.
use sts2_harness::benchmark_manifest::cold_launch::{
    ColdLaunchOrchestrator, LaunchProfile, PristineBaseline, ProcessBirth, ReadinessProof,
    TelemetryExclusions, TrialLifecycle,
};

/// The immutable pristine baseline every synthetic trial is provisioned from.
pub fn pristine() -> PristineBaseline {
    PristineBaseline {
        baseline_digest: "sha256:baseline-synthetic-0001".to_owned(),
        profile: LaunchProfile {
            profile_id: "profile-synthetic".to_owned(),
            build_digest: "sha256:build-synthetic-2".to_owned(),
            game_version: "2.0.0".to_owned(),
        },
        exclusions: TelemetryExclusions {
            paths: vec!["telemetry/run.log".to_owned()],
        },
    }
}

/// A pristine baseline whose declared build digest was deliberately mutated.
pub fn mutated_build() -> PristineBaseline {
    let mut baseline = pristine();
    baseline.profile.build_digest = "sha256:build-synthetic-9".to_owned();
    baseline
}

/// One gateway-attested process birth for a trial.
pub fn birth(token: &str, generation: u64) -> ProcessBirth {
    ProcessBirth {
        birth_token: token.to_owned(),
        instance_generation: generation,
    }
}

/// Admits, leases, provisions, launches, proves readiness and settles setup for one trial.
pub fn launched_trial(
    orchestrator: &mut ColdLaunchOrchestrator,
    key: &str,
    destination: &str,
    token: &str,
    generation: u64,
) -> TrialLifecycle {
    let reference = pristine();
    let mut trial = TrialLifecycle::admit_against(key, reference.clone(), &reference).unwrap();
    let lease = orchestrator.lease_destination(destination, key).unwrap();
    trial.reserve_destination(lease).unwrap();
    trial.record_provisioned().unwrap();
    let birth = birth(token, generation);
    trial.record_launched(birth.clone()).unwrap();
    orchestrator.attest_birth(key, &birth).unwrap();
    let proof = ReadinessProof::for_birth(birth, "token-readiness", generation);
    trial.record_ready(proof).unwrap();
    trial.record_setup_settled().unwrap();
    trial.record_running().unwrap();
    trial
}

/// Runs a whole serial trial to a stopped-and-cleaned terminal state.
pub fn completed_trial(
    orchestrator: &mut ColdLaunchOrchestrator,
    key: &str,
    destination: &str,
    token: &str,
    generation: u64,
) -> TrialLifecycle {
    let mut trial = launched_trial(orchestrator, key, destination, token, generation);
    trial.record_stopped().unwrap();
    trial.record_cleaned().unwrap();
    orchestrator
        .release_destination(destination)
        .expect("a completed trial releases its destination");
    orchestrator
        .retire_birth(token)
        .expect("a completed trial retires its birth");
    trial
}
