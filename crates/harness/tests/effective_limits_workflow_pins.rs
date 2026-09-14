// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use sts2_harness::effective_limits_pins::*;

#[test]
fn aligned_consumers_require_their_exact_revision_and_repository_owned_workflow() {
    let matrix = PinMatrix::repository().expect("matrix");
    matrix.validate().expect("valid matrix");
    for index in 0..matrix.consumers.len() {
        let other = 1 - index;
        for change in 0..6 {
            let mut changed = matrix.clone();
            let other_pin = changed.consumers[other].harness_ci_pin.clone();
            let consumer = &mut changed.consumers[index];
            match change {
                0 => consumer.harness_ci_pin = None,
                1 => consumer.revision = "0".repeat(40),
                2 => consumer.harness_ci_pin = other_pin,
                3 => consumer.repository = "AI-Ascension/unknown-consumer".to_owned(),
                4 => consumer.revision_source = RevisionSource::ObservedHead,
                5 => {
                    consumer.harness_ci_pin.as_mut().expect("pin").workflow =
                        ".github/workflows/invented-success.yml".to_owned();
                }
                _ => unreachable!(),
            }
            assert_eq!(changed.validate(), Err(PinMatrixError::ConsumerPinDrift));
        }
    }
}

#[test]
fn malformed_and_stale_full_revisions_are_rejected() {
    let matrix = PinMatrix::repository().expect("matrix");
    for revision in ["main".to_owned(), "0".repeat(39), "0".repeat(40)] {
        let mut changed = matrix.clone();
        let pin = changed.consumers[0].harness_ci_pin.as_mut().expect("pin");
        pin.revision = revision.clone();
        assert_eq!(
            changed.validate(),
            Err(if revision.len() == 40 {
                PinMatrixError::ConsumerPinDrift
            } else {
                PinMatrixError::ConsumerRevisionMalformed
            })
        );
    }
}
