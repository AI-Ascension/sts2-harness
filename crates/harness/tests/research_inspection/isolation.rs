// SPDX-License-Identifier: MIT

use super::*;

// --- AC2: wrong target, revoked scope and protected fields reject ----------

#[test]
fn wrong_checkpoint_run_or_branch_is_refused() {
    let grant = default_grant();
    let selected = vec![field(ResearchFieldGroup::RngStreams, "draw_cursor")];
    let foreign = [
        binding("checkpoint-2", RUN, BRANCH),
        binding(CHECKPOINT, "run-2", BRANCH),
        binding(CHECKPOINT, RUN, "branch-2"),
    ];

    for foreign_target in foreign {
        let mut foreign_request = request(selected.clone(), 0, 4);
        foreign_request.target = foreign_target.clone();
        assert_eq!(
            admit_research_read(&grant, &foreign_request),
            Err(ResearchInspectionError::WrongTarget),
            "target {foreign_target:?} must not read another checkpoint"
        );
    }
}

#[test]
fn a_foreign_consumer_lane_is_refused() {
    let grant = default_grant();
    let mut foreign = request(
        vec![field(ResearchFieldGroup::RngStreams, "draw_cursor")],
        0,
        4,
    );
    foreign.consumer_lane = "gameplay-lane-1".to_owned();

    assert_eq!(
        admit_research_read(&grant, &foreign),
        Err(ResearchInspectionError::RevokedScope)
    );
}

#[test]
fn revoked_scope_is_refused_and_never_revived() {
    let mut grant = default_grant();
    let read = request(
        vec![field(ResearchFieldGroup::RngStreams, "draw_cursor")],
        0,
        4,
    );
    assert!(admit_research_read(&grant, &read).is_ok());

    grant.revoke();
    assert!(grant.is_revoked());
    assert!(!grant.admits(ResearchFieldGroup::RngStreams));
    assert_eq!(
        admit_research_read(&grant, &read),
        Err(ResearchInspectionError::RevokedScope),
        "a replayed request must not outlive the approval that authorized it"
    );
    assert_eq!(
        grant.authorizes(&target(), CONSUMER),
        Err(ResearchInspectionError::RevokedScope)
    );
}

#[test]
fn a_group_outside_the_grant_is_a_protected_field() {
    let grant = grant([ResearchFieldGroup::SaveStateFields]);
    let wider = request(
        vec![field(ResearchFieldGroup::RngStreams, "draw_cursor")],
        0,
        4,
    );

    assert_eq!(
        admit_research_read(&grant, &wider),
        Err(ResearchInspectionError::ProtectedField),
        "asking for a wider visibility parameter is a refusal, not a scope change"
    );
}

#[test]
fn an_undeclared_research_identity_is_refused() {
    let grant = default_grant();
    let mut other = request(
        vec![field(ResearchFieldGroup::RngStreams, "draw_cursor")],
        0,
        4,
    );
    other.research_id = "research-2".to_owned();

    assert_eq!(
        admit_research_read(&grant, &other),
        Err(ResearchInspectionError::WrongTarget)
    );
}

// --- AC2/T1: grants only admit a bounded, explicit, usable scope -----------

#[test]
fn grant_admission_refuses_unusable_targets_consumers_and_group_sets() {
    assert_eq!(
        ResearchInspectionGrant::admit(RESEARCH_ID, target(), CONSUMER, []),
        Err(ResearchScopeRefusal::InvalidGroups)
    );
    assert_eq!(
        ResearchInspectionGrant::admit(
            RESEARCH_ID,
            binding("../escape", RUN, BRANCH),
            CONSUMER,
            [ResearchFieldGroup::RngStreams]
        ),
        Err(ResearchScopeRefusal::InvalidTarget)
    );
    assert_eq!(
        ResearchInspectionGrant::admit(
            RESEARCH_ID,
            target(),
            "lane/with/slashes",
            [ResearchFieldGroup::RngStreams]
        ),
        Err(ResearchScopeRefusal::InvalidConsumer)
    );
    assert_eq!(
        ResearchInspectionGrant::admit(
            "https://example.invalid/research",
            target(),
            CONSUMER,
            [ResearchFieldGroup::RngStreams]
        ),
        Err(ResearchScopeRefusal::InvalidTarget)
    );
}

#[test]
fn research_identities_refuse_paths_urls_and_oversized_values() {
    for usable in ["checkpoint-1", "run_1", "branch.1", "ns:value"] {
        assert!(
            is_research_identity(usable),
            "{usable} is a portable identity"
        );
    }
    for unusable in [
        "",
        ".hidden",
        "..",
        "a/../b",
        "a\\b",
        "https://example.invalid",
        "with space",
    ] {
        assert!(
            !is_research_identity(unusable),
            "{unusable} is not a portable identity"
        );
    }

    let oversized = "a".repeat(MAX_RESEARCH_IDENTITY_BYTES + 1);
    assert!(!is_research_identity(&oversized));
    assert!(is_research_identity(
        &"a".repeat(MAX_RESEARCH_IDENTITY_BYTES)
    ));
}

#[test]
fn field_references_refuse_paths_queries_and_unbounded_names() {
    assert!(ResearchFieldRef::new(ResearchFieldGroup::RngStreams, "draw_cursor").is_ok());
    assert!(ResearchFieldRef::new(ResearchFieldGroup::RngStreams, "shuffle.seed").is_ok());

    for unusable in [
        "",
        "../etc/passwd",
        "/etc/passwd",
        "..",
        ".leading",
        "trailing.",
        "select * from state",
        "UPPER",
        "with space",
        "with-dash",
    ] {
        assert_eq!(
            ResearchFieldRef::new(ResearchFieldGroup::RngStreams, unusable),
            Err(ResearchInspectionError::InvalidField),
            "{unusable} must not be expressible as a field reference"
        );
    }

    let oversized = "a".repeat(MAX_FIELD_REF_BYTES + 1);
    assert_eq!(
        ResearchFieldRef::new(ResearchFieldGroup::RngStreams, oversized),
        Err(ResearchInspectionError::InvalidField)
    );
}
