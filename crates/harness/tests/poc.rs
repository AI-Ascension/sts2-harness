// SPDX-License-Identifier: MIT

use std::error::Error;

use sha2::{Digest, Sha256};
use sts2_harness::{
    POC_CLOCK_TICK, POC_SCHEMA_DIGEST, POC_SEED, PocAction, PocRunner, PocStatus, run_poc,
    verify_poc_artifact,
};

const EXPECTED_TRACE_SHA256: &str =
    "d8420f4432ba0eabe736c73df435f736aac40a262aadbe5659cda5daf5da43fc";

#[test]
fn deterministic_trace_covers_the_requested_path_and_outcomes() -> Result<(), Box<dyn Error>> {
    verify_poc_artifact()?;
    let first = PocRunner::new().run()?;
    let second = run_poc()?;

    assert_eq!(first, second);
    assert_eq!(first.seed(), POC_SEED);
    assert_eq!(first.clock_tick(), POC_CLOCK_TICK);
    assert_eq!(first.session_id(), "session-1");
    assert_eq!(first.trace().len(), 15);
    assert!(first.accepted_changed_once());
    assert!(first.rejected_unchanged());
    assert!(first.trace_bytes().ends_with('\n'));
    assert!(first.trace_bytes().contains("sts2.game-core/zero_units"));
    assert_eq!(
        format!("{:x}", Sha256::digest(first.trace_bytes().as_bytes())),
        EXPECTED_TRACE_SHA256
    );
    assert!(
        first
            .artifact_lineage()
            .contains("artifact=sts2-protocol/poc-v1")
    );

    assert_eq!(first.trace()[0].boundary(), "harness");
    assert_eq!(first.trace()[4].boundary(), "game-core");
    assert_eq!(first.trace()[5].tool(), "submit_action");
    assert_eq!(
        first.trace()[5].action().map(PocAction::action_id),
        Some("use_budget")
    );
    assert_eq!(first.trace()[5].action().map(PocAction::units), Some(1));
    assert_eq!(first.trace()[5].status(), Some(PocStatus::Accepted));
    assert_eq!(first.trace()[10].status(), Some(PocStatus::Rejected));
    assert_eq!(
        first.trace()[10].error_code(),
        Some("sts2.game-core/zero_units")
    );

    for (sequence, event) in first.trace().iter().enumerate() {
        assert_eq!(event.protocol_version(), "poc-v1");
        assert_eq!(event.schema_digest(), POC_SCHEMA_DIGEST);
        assert_eq!(event.instance_id(), "instance-1");
        assert_eq!(event.session_id(), "session-1");
        assert_eq!(event.lease_id(), "lease-1");
        assert_eq!(event.sequence(), sequence);
    }
    for operation in first.trace().chunks_exact(5) {
        assert_eq!(
            operation
                .iter()
                .map(|event| event.boundary())
                .collect::<Vec<_>>(),
            ["harness", "mcp", "gateway", "game-mod", "game-core"]
        );
    }
    assert!(
        first
            .trace()
            .iter()
            .take(5)
            .all(|event| event.kind() == "state_response")
    );
    assert!(
        first
            .trace()
            .iter()
            .skip(5)
            .all(|event| event.kind() == "action_response")
    );
    Ok(())
}
