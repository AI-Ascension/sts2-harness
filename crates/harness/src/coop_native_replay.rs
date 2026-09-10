// SPDX-License-Identifier: MIT

/// A deterministic comparison of two validated native-co-op record streams.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct CoopNativeReplayReport {
    compared_records: usize,
    identical: bool,
    first_divergence: Option<u16>,
    expected_digest: Option<String>,
    actual_digest: Option<String>,
}

impl CoopNativeReplayReport {
    #[must_use]
    pub const fn compared_records(&self) -> usize {
        self.compared_records
    }

    #[must_use]
    pub const fn is_identical(&self) -> bool {
        self.identical
    }

    #[must_use]
    pub const fn first_divergence(&self) -> Option<u16> {
        self.first_divergence
    }

    #[must_use]
    pub fn expected_digest(&self) -> Option<&str> {
        self.expected_digest.as_deref()
    }

    #[must_use]
    pub fn actual_digest(&self) -> Option<&str> {
        self.actual_digest.as_deref()
    }
}

#[must_use]
pub fn replay_coop_native_records(
    expected: &[CoopNativeRecord],
    actual: &[CoopNativeRecord],
) -> CoopNativeReplayReport {
    let compared_records = expected.len().max(actual.len());
    let first = (0..compared_records).find(|index| expected.get(*index) != actual.get(*index));
    let (expected_digest, actual_digest) = first.map_or((None, None), |index| {
        (
            expected.get(index).map(|record| record.envelope_digest().to_owned()),
            actual.get(index).map(|record| record.envelope_digest().to_owned()),
        )
    });
    CoopNativeReplayReport {
        compared_records,
        identical: first.is_none(),
        first_divergence: first.and_then(|index| u16::try_from(index).ok()),
        expected_digest,
        actual_digest,
    }
}

impl<P: CoopNativePort> CoopNativeCoordinator<P> {
    #[must_use]
    pub fn replay_report(&self, actual: &[CoopNativeRecord]) -> CoopNativeReplayReport {
        replay_coop_native_records(&self.records, actual)
    }
}
