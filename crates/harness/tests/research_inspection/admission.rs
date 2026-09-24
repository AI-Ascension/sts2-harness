// SPDX-License-Identifier: MIT

use super::*;

// --- AC2: an admitted lane reads an exact bound target ---------------------

#[test]
fn admitted_read_admits_exactly_the_requested_fields_in_order() {
    let grant = default_grant();
    let selected = vec![
        field(ResearchFieldGroup::RngStreams, "draw_cursor"),
        field(ResearchFieldGroup::HiddenPiles, "draw_pile_order"),
    ];
    let admitted = admit_research_read(&grant, &request(selected.clone(), 0, 8)).expect("admits");

    assert_eq!(admitted.fields(), selected.as_slice());
    assert_eq!(admitted.page_index(), 0);
    assert_eq!(admitted.total_pages(), 1);
    assert!(admitted.is_complete());
    assert_eq!(admitted.len(), 2);
    assert!(!admitted.is_empty());
}

#[test]
fn every_supported_group_is_admissible_and_stably_labelled() {
    let groups = ResearchFieldGroup::supported();
    assert_eq!(groups.len(), 5);
    assert!(groups.len() <= MAX_RESEARCH_GRANTS);

    let labels: Vec<&str> = groups.iter().map(|group| group.label()).collect();
    assert_eq!(
        labels,
        vec![
            "rng_streams",
            "hidden_piles",
            "unrevealed_assignments",
            "pending_hidden_state",
            "save_state_fields",
        ]
    );

    for group in groups {
        let admitted = admit_research_read(
            &grant([group]),
            &request(vec![field(group, "sample_field")], 0, 4),
        )
        .expect("each supported group admits");
        assert_eq!(admitted.len(), 1);
        assert!(admitted.is_complete());
    }
}

#[test]
fn grant_reads_back_its_own_scope_without_widening() {
    let grant = grant([ResearchFieldGroup::RngStreams]);
    assert_eq!(grant.research_id(), RESEARCH_ID);
    assert_eq!(grant.consumer_lane(), CONSUMER);
    assert_eq!(grant.target(), &target());
    assert_eq!(grant.groups(), vec![ResearchFieldGroup::RngStreams]);
    assert!(!grant.is_revoked());
    assert!(grant.admits(ResearchFieldGroup::RngStreams));
    assert!(!grant.admits(ResearchFieldGroup::HiddenPiles));
}
