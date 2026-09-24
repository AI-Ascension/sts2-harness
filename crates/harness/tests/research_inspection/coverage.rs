// SPDX-License-Identifier: MIT

use super::*;

// --- AC1: no refusal leaks a value, a digest or a field name ---------------

#[test]
fn refusals_are_value_free_and_distinguishable() {
    let refusals = [
        ResearchInspectionError::Incompatible,
        ResearchInspectionError::InvalidField,
        ResearchInspectionError::InvalidScope,
        ResearchInspectionError::WrongTarget,
        ResearchInspectionError::RevokedScope,
        ResearchInspectionError::ProtectedField,
        ResearchInspectionError::InvalidPage,
        ResearchInspectionError::TooManyFields,
    ];

    for refusal in refusals {
        let text = refusal.to_string();
        assert!(!text.is_empty());
        assert!(!text.contains(CHECKPOINT));
        assert!(!text.contains(RESEARCH_ID));
        assert!(!text.contains(CONSUMER));
        assert!(!text.to_lowercase().contains("sha"));
        assert!(
            !text.contains('.') || text.ends_with('.'),
            "no field path in {text}"
        );
    }

    let texts: Vec<String> = refusals.iter().map(ToString::to_string).collect();
    let distinct: std::collections::BTreeSet<&String> = texts.iter().collect();
    assert_eq!(
        distinct.len(),
        refusals.len(),
        "each refusal reads distinctly"
    );
}

#[test]
fn admission_never_invents_a_field_value() {
    let grant = default_grant();
    let selected = vec![
        field(ResearchFieldGroup::UnrevealedAssignments, "next_card"),
        field(ResearchFieldGroup::PendingHiddenState, "pending_relic"),
    ];
    let admitted = admit_research_read(&grant, &request(selected.clone(), 0, 4)).expect("admits");

    assert_eq!(admitted.fields(), selected.as_slice());
    assert!(
        admitted
            .fields()
            .iter()
            .all(|entry| !entry.field().is_empty())
    );
    assert_eq!(
        admitted.len(),
        2,
        "admission names fields, not availability"
    );
}

// --- AC3: coverage is reported, never collapsed into a value ---------------

#[test]
fn coverage_vocabulary_stays_distinct_and_only_available_yields_a_value() {
    let materialized = FieldAvailability::Available {
        value: "7".to_owned(),
    };
    let pending = FieldAvailability::SimulationRequired {
        dependency: "player_choice".to_owned(),
    };

    assert_eq!(materialized.label(), "available");
    assert_eq!(materialized.value(), Some("7"));
    assert!(materialized.is_available());

    assert_eq!(
        FieldAvailability::NotMaterialized.label(),
        "not_materialized"
    );
    assert_eq!(FieldAvailability::NotMaterialized.value(), None);
    assert!(!FieldAvailability::NotMaterialized.is_available());

    assert_eq!(pending.label(), "simulation_required");
    assert_eq!(pending.value(), None);
    assert!(!pending.is_available());

    assert_eq!(FieldAvailability::Unsupported.label(), "unsupported");
    assert_eq!(FieldAvailability::Unsupported.value(), None);
    assert!(!FieldAvailability::Unsupported.is_available());
}

#[test]
fn an_owner_report_matching_the_admission_verifies() {
    let grant = default_grant();
    let admitted = admit_research_read(
        &grant,
        &request(
            vec![
                field(ResearchFieldGroup::RngStreams, "draw_cursor"),
                field(ResearchFieldGroup::HiddenPiles, "draw_pile_order"),
            ],
            0,
            4,
        ),
    )
    .expect("admits");

    let page = report(
        &admitted,
        vec![
            available(ResearchFieldGroup::RngStreams, "draw_cursor", "12"),
            ResearchFieldReport {
                field: field(ResearchFieldGroup::HiddenPiles, "draw_pile_order"),
                availability: FieldAvailability::NotMaterialized,
            },
        ],
    );

    assert_eq!(page.verify(&admitted), Ok(()));
    assert_eq!(page.entries.len(), admitted.len());
}

#[test]
fn an_owner_report_that_adds_omits_reorders_or_mislabels_is_refused() {
    let grant = default_grant();
    let admitted = admit_research_read(
        &grant,
        &request(
            vec![
                field(ResearchFieldGroup::RngStreams, "draw_cursor"),
                field(ResearchFieldGroup::HiddenPiles, "draw_pile_order"),
            ],
            0,
            4,
        ),
    )
    .expect("admits");

    let added = report(
        &admitted,
        vec![
            available(ResearchFieldGroup::RngStreams, "draw_cursor", "12"),
            available(ResearchFieldGroup::HiddenPiles, "draw_pile_order", "a,b,c"),
            available(ResearchFieldGroup::UnrevealedAssignments, "extra", "leak"),
        ],
    );
    assert_eq!(
        added.verify(&admitted),
        Err(ResearchInspectionError::ProtectedField)
    );

    let omitted = report(
        &admitted,
        vec![available(
            ResearchFieldGroup::RngStreams,
            "draw_cursor",
            "12",
        )],
    );
    assert_eq!(
        omitted.verify(&admitted),
        Err(ResearchInspectionError::ProtectedField)
    );

    let reordered = report(
        &admitted,
        vec![
            available(ResearchFieldGroup::HiddenPiles, "draw_pile_order", "a,b,c"),
            available(ResearchFieldGroup::RngStreams, "draw_cursor", "12"),
        ],
    );
    assert_eq!(
        reordered.verify(&admitted),
        Err(ResearchInspectionError::ProtectedField)
    );

    let mislabelled = report(
        &admitted,
        vec![
            available(ResearchFieldGroup::RngStreams, "other_field", "12"),
            available(ResearchFieldGroup::HiddenPiles, "draw_pile_order", "a,b,c"),
        ],
    );
    assert_eq!(
        mislabelled.verify(&admitted),
        Err(ResearchInspectionError::ProtectedField)
    );
}

#[test]
fn an_owner_report_for_another_page_is_refused() {
    let grant = default_grant();
    let admitted = admit_research_read(
        &grant,
        &request(
            vec![
                field(ResearchFieldGroup::RngStreams, "a"),
                field(ResearchFieldGroup::RngStreams, "b"),
                field(ResearchFieldGroup::RngStreams, "c"),
            ],
            1,
            2,
        ),
    )
    .expect("admits");
    assert_eq!(admitted.page_index(), 1);
    assert_eq!(admitted.total_pages(), 2);
    assert!(
        !admitted.is_complete(),
        "a partial page is never labelled complete"
    );

    let wrong_page = ResearchPageReport {
        page_index: 0,
        entries: vec![available(ResearchFieldGroup::RngStreams, "c", "3")],
    };
    assert_eq!(
        wrong_page.verify(&admitted),
        Err(ResearchInspectionError::InvalidPage)
    );

    let right_page = report(
        &admitted,
        vec![available(ResearchFieldGroup::RngStreams, "c", "3")],
    );
    assert_eq!(right_page.verify(&admitted), Ok(()));
}
