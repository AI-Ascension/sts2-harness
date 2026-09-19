// SPDX-License-Identifier: MIT

//! The two bounds that stop an episode going in circles.
//!
//! One counts abstentions on a state that has not changed. The other counts visits to a
//! situation whatever was decided there, which is what catches a cycle of confident decisions
//! that the first cannot see.

use std::collections::BTreeMap;

use crate::sha256_hex;

use super::EpisodeObservation;

/// Records a visit to this situation and reports whether the bound has been passed.
///
/// A cycle of confident decisions is invisible to the abstention bound: each decision clears the
/// gate, and each state is new because the generation advances. Counting the situation itself
/// catches it, whatever was decided there.
///
/// `bound` of zero disables the count entirely, which is the previous behaviour.
pub(super) fn visit_exceeds_bound(
    seen: &mut BTreeMap<String, u16>,
    observation: &EpisodeObservation,
    bound: u16,
) -> bool {
    if bound == 0 {
        return false;
    }
    let visits = seen.entry(fingerprint(observation)).or_insert(0);
    *visits = visits.saturating_add(1);
    *visits > bound
}

/// Fingerprints one situation, ignoring the identity that advances on every step.
///
/// `state_id` and `generation` change whenever anything happens, so a situation reached twice looks
/// like two situations if they are included. Everything else is what the player can see, which is
/// what makes two visits the same visit.
fn fingerprint(observation: &EpisodeObservation) -> String {
    let mut value = observation.fair_play().as_value().clone();
    if let Some(object) = value.as_object_mut() {
        object.remove("state_id");
        object.remove("generation");
    }
    sha256_hex(value.to_string().as_bytes())
}

/// Records an abstention and reports whether the bound has been reached.
///
/// Counted against the state it was made on, so the run resets whenever anything changes. `bound`
/// of zero disables it, which is the previous behaviour.
pub(super) fn abstention_reaches_bound(
    on: &mut Option<(String, u64)>,
    count: &mut u8,
    observation: &EpisodeObservation,
    bound: u8,
) -> bool {
    let here = (observation.state_id().to_owned(), observation.generation());
    if on.as_ref() == Some(&here) {
        *count = count.saturating_add(1);
    } else {
        *on = Some(here);
        *count = 1;
    }
    bound > 0 && *count >= bound
}
