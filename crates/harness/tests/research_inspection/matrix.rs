// SPDX-License-Identifier: MIT

use super::*;

// --- AC4 matrix: the documented field set is the supported set ------------

#[test]
fn the_supported_matrix_is_exactly_the_declared_groups() {
    let matrix: Vec<(&str, ResearchFieldGroup)> = ResearchFieldGroup::supported()
        .into_iter()
        .map(|group| (group.label(), group))
        .collect();

    assert_eq!(matrix.len(), 5);
    for (label, group) in matrix {
        assert_eq!(group.label(), label);
        let declared = ResearchInspectionGrant::admit(RESEARCH_ID, target(), CONSUMER, [group])
            .expect("every declared group is admissible");
        assert!(declared.admits(group));
    }

    let all: Vec<ResearchFieldGroup> = ResearchFieldGroup::supported().to_vec();
    let over = ResearchInspectionGrant::admit(RESEARCH_ID, target(), CONSUMER, all);
    assert!(
        over.is_ok(),
        "the full matrix fits the declared grant bound"
    );
}

#[test]
fn read_requests_round_trip_through_serialization() {
    let original = request(
        vec![field(ResearchFieldGroup::HiddenPiles, "draw_pile_order")],
        0,
        4,
    );
    let encoded = serde_json::to_string(&original).expect("request serializes");
    let decoded: ResearchReadRequest = serde_json::from_str(&encoded).expect("request parses");
    assert_eq!(decoded, original);

    let unknown = encoded.replace("\"page_index\"", "\"page_index_shadow\"");
    assert!(
        serde_json::from_str::<ResearchReadRequest>(&unknown).is_err(),
        "an unknown field is refused rather than ignored"
    );

    for escalation in [
        encoded.replace(
            "{\"schema_version\"",
            "{\"grant\":{\"groups\":[\"rng_streams\"]},\"schema_version\"",
        ),
        encoded.replace(
            "{\"schema_version\"",
            "{\"groups\":[\"rng_streams\"],\"schema_version\"",
        ),
        encoded.replace(
            "{\"schema_version\"",
            "{\"visibility\":\"research\",\"schema_version\"",
        ),
    ] {
        assert!(
            serde_json::from_str::<ResearchReadRequest>(&escalation).is_err(),
            "a request cannot widen its own visibility by adding a parameter"
        );
    }
}
