// SPDX-License-Identifier: MIT

use std::error::Error;

use sts2_harness::{
    EpisodeObservation, ReceiptQueryActionKind, ReceiptQueryCoordinate, ReceiptQueryIdentity,
    ReceiptQueryLocation, ReceiptQueryResult, ReceiptQueryStatus, RecoveryController,
    RecoveryError, RecoveryPort, TransitionReceipt, verify_coop_receipt_query_artifact,
};

const SETTLED_RESPONSE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/coop-receipt-query-v1/golden/receipt-query-response-settled.json"
));
const FRESH_SCOPE_RESPONSE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/coop-receipt-query-v1/fixtures/invalid-response-fresh-scope.json"
));

fn identity() -> Result<ReceiptQueryIdentity, Box<dyn Error>> {
    Ok(ReceiptQueryIdentity::new(
        "op:run-17:0001",
        ReceiptQueryActionKind::PlayCard,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "session-native-17",
        "run-17",
        ReceiptQueryLocation::new(1, Some(42), Some(ReceiptQueryCoordinate::new(3, 5))),
        "peer-1",
        "native-host-a",
        "epoch-9",
        17,
        17,
        vec!["peer-1".into(), "peer-2".into()],
    )?)
}

#[derive(Default)]
struct ReceiptQueryPort {
    query_calls: usize,
    reobserve_calls: usize,
    reconcile_calls: usize,
}

impl RecoveryPort for ReceiptQueryPort {
    fn reobserve(&mut self) -> Result<EpisodeObservation, RecoveryError> {
        self.reobserve_calls += 1;
        Err(RecoveryError::Unsupported)
    }

    fn reconcile(&mut self, _operation_id: &str) -> Result<TransitionReceipt, RecoveryError> {
        self.reconcile_calls += 1;
        Err(RecoveryError::Unsupported)
    }

    fn query_receipt(
        &mut self,
        identity: &ReceiptQueryIdentity,
    ) -> Result<ReceiptQueryResult, RecoveryError> {
        self.query_calls += 1;
        ReceiptQueryResult::from_json(
            SETTLED_RESPONSE,
            identity,
            "corr:17:1",
            "instance-1",
            "session-native-17",
            "lease-1",
            9,
        )
        .map_err(|_| RecoveryError::Terminal)
    }

    fn release_lease(&mut self) -> Result<(), RecoveryError> {
        Err(RecoveryError::Unsupported)
    }

    fn stop_episode(&mut self) -> Result<(), RecoveryError> {
        Err(RecoveryError::Unsupported)
    }
}

#[test]
fn public_recovery_query_returns_retained_receipt_without_new_observation()
-> Result<(), Box<dyn Error>> {
    verify_coop_receipt_query_artifact()?;
    let identity = identity()?;
    let controller = RecoveryController::new(1)?;
    let mut port = ReceiptQueryPort::default();

    let result = controller.query_receipt(&mut port, identity.clone())?;

    assert_eq!(result.status(), ReceiptQueryStatus::Settled);
    assert_eq!(result.identity(), &identity);
    let receipt = result.receipt().ok_or("settled response omitted receipt")?;
    assert_eq!(receipt.after_host_generation(), Some(18));
    assert_eq!(receipt.effect_id(), Some("effect:op:run-17:0001"));
    assert_eq!(port.query_calls, 1);
    assert_eq!(port.reobserve_calls, 0);
    assert_eq!(port.reconcile_calls, 0);
    Ok(())
}

#[test]
fn public_parser_rejects_fresh_reconciliation_evidence() -> Result<(), Box<dyn Error>> {
    let identity = identity()?;
    assert!(
        ReceiptQueryResult::from_json(
            FRESH_SCOPE_RESPONSE,
            &identity,
            "corr:17:1",
            "instance-1",
            "session-native-17",
            "lease-1",
            9,
        )
        .is_err()
    );
    Ok(())
}
