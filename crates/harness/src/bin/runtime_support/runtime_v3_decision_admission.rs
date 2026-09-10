// SPDX-License-Identifier: MIT

use sts2_harness::Decision;

use super::durable::ProviderReservationToken;

pub(super) enum DecisionAdmission {
    Reused(Decision),
    Fresh(ProviderReservationToken),
}
