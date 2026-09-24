// SPDX-License-Identifier: MIT

use super::*;

// --- AC2/T2: bounded paging and closed schema version ---------------------

#[test]
fn paging_is_bounded_and_completeness_is_explicit() {
    let grant = default_grant();
    let fields: Vec<ResearchFieldRef> = (0..5)
        .map(|index| {
            field(
                ResearchFieldGroup::SaveStateFields,
                &format!("field_{index}"),
            )
        })
        .collect();

    let first = admit_research_read(&grant, &request(fields.clone(), 0, 2)).expect("admits");
    assert_eq!(first.len(), 2);
    assert_eq!(first.total_pages(), 3);
    assert!(
        !first.is_complete(),
        "page zero cannot be complete when pages remain"
    );

    let last = admit_research_read(&grant, &request(fields.clone(), 2, 2)).expect("admits");
    assert_eq!(last.len(), 1);
    assert_eq!(last.total_pages(), 3);
    assert!(
        !last.is_complete(),
        "a later page never claims the whole answer"
    );

    let whole = admit_research_read(&grant, &request(fields.clone(), 0, 8)).expect("admits");
    assert_eq!(whole.len(), 5);
    assert!(whole.is_complete());
}

#[test]
fn unusable_page_bounds_are_refused() {
    let grant = default_grant();
    let selected = vec![field(ResearchFieldGroup::RngStreams, "draw_cursor")];

    assert_eq!(
        admit_research_read(&grant, &request(selected.clone(), 1, 1)),
        Err(ResearchInspectionError::InvalidPage)
    );
    assert_eq!(
        admit_research_read(&grant, &request(selected.clone(), 0, 0)),
        Err(ResearchInspectionError::InvalidPage)
    );
    assert_eq!(
        admit_research_read(&grant, &request(selected.clone(), 0, 129)),
        Err(ResearchInspectionError::InvalidPage)
    );
    assert_eq!(
        admit_research_read(
            &grant,
            &request(
                selected.clone(),
                0,
                u32::try_from(MAX_RESEARCH_PAGE_ITEMS).unwrap()
            )
        ),
        Ok(admit_research_read(&grant, &request(selected, 0, 128)).expect("admits")),
    );
}

#[test]
fn an_unsupported_schema_version_is_refused() {
    let grant = default_grant();
    let mut stale = request(
        vec![field(ResearchFieldGroup::RngStreams, "draw_cursor")],
        0,
        4,
    );
    stale.schema_version = "ascension.research-inspection/v0".to_owned();
    assert_eq!(
        admit_research_read(&grant, &stale),
        Err(ResearchInspectionError::Incompatible)
    );

    let mut empty_selection = request(vec![], 0, 4);
    empty_selection.schema_version = RESEARCH_INSPECTION_SCHEMA_VERSION.to_owned();
    assert_eq!(
        admit_research_read(&grant, &empty_selection),
        Err(ResearchInspectionError::TooManyFields)
    );

    let mut malformed = request(
        vec![field(ResearchFieldGroup::RngStreams, "draw_cursor")],
        0,
        4,
    );
    malformed.consumer_lane = "lane/with/slashes".to_owned();
    assert_eq!(
        admit_research_read(&grant, &malformed),
        Err(ResearchInspectionError::InvalidScope)
    );
}
