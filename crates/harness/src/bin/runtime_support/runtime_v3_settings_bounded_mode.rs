// SPDX-License-Identifier: MIT

/// Whether the operator named an explicit bounded mode for a local provider bridge.
///
/// A local bridge runs only in a mode the operator states outright. Three name one:
///
///   - the combat demo, which acts only while the host is already in combat;
///   - a live episode, which the caller above restricts to the OpenAI Astra provider;
///   - a campaign episode, which plays a run from the main menu through the full catalog.
///
/// The campaign mode exists because the combat demo never starts a run: it waits for combat
/// and nothing in that path leaves a menu, so a provider pinned to it can observe a campaign
/// but never begin one. Combat and campaign are exclusive rather than layered, because the two
/// take different runners and a vector naming both says nothing about which was intended.
pub(super) fn bounded_mode_named(
    combat_demo: bool,
    live_episode: bool,
    campaign_episode: bool,
) -> Result<bool, String> {
    if combat_demo && campaign_episode {
        return Err(String::from(
            "Campaign episode mode and combat demo mode are mutually exclusive",
        ));
    }
    Ok(combat_demo || live_episode || campaign_episode)
}

#[cfg(test)]
mod tests {
    use super::bounded_mode_named;

    #[test]
    fn every_single_explicit_mode_is_a_named_bounded_mode() {
        assert_eq!(bounded_mode_named(true, false, false), Ok(true));
        assert_eq!(bounded_mode_named(false, true, false), Ok(true));
        assert_eq!(bounded_mode_named(false, false, true), Ok(true));
    }

    #[test]
    fn naming_no_mode_leaves_a_local_bridge_without_one() {
        assert_eq!(bounded_mode_named(false, false, false), Ok(false));
    }

    #[test]
    fn combat_and_campaign_together_are_refused_rather_than_ranked() {
        // Each takes a different runner, so a vector naming both states no intent at all.
        assert!(bounded_mode_named(true, false, true).is_err());
        assert!(bounded_mode_named(true, true, true).is_err());
    }

    #[test]
    fn a_campaign_episode_pairs_with_a_live_episode() {
        // Only combat conflicts with campaign; the live-episode flag is constrained by provider
        // kind in verify_revision, not here.
        assert_eq!(bounded_mode_named(false, true, true), Ok(true));
    }
}
