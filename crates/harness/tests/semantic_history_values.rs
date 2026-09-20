// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, dead_code)]

use sts2_harness::semantic_history::{
    MAX_HISTORY_IDENTITY_BYTES, SemanticHistoryBinding, SemanticHistoryError, SemanticHistoryKind,
    SemanticHistoryNamespace, SemanticHistoryValue, is_opaque_history_identity,
    validate_history_identity,
};

#[path = "support/semantic_history_fixture.rs"]
mod fixture;

#[test]
fn a_quantity_value_must_carry_a_valid_unit() {
    for unit in [String::new(), "a\nb".to_owned(), "u".repeat(129)] {
        assert_eq!(
            SemanticHistoryValue::Quantity { amount: 0, unit }.validate(),
            Err(SemanticHistoryError::InvalidLabel("value.unit"))
        );
    }
    // A stated zero is a real observation, distinct from an unavailable value.
    let zero = SemanticHistoryValue::Quantity {
        amount: 0,
        unit: "hp".to_owned(),
    };
    assert_eq!(zero.validate(), Ok(()));
    assert!(zero.is_stated());
}
#[test]
fn a_reference_value_may_not_name_a_live_instance() {
    assert_eq!(
        SemanticHistoryValue::Reference {
            namespace: SemanticHistoryNamespace::LiveInstance,
            identity: "instance_hero".to_owned()
        }
        .validate(),
        Err(SemanticHistoryError::InvalidField("value.namespace"))
    );
    assert_eq!(
        SemanticHistoryValue::Reference {
            namespace: SemanticHistoryNamespace::Definition,
            identity: "card_strike".to_owned()
        }
        .validate(),
        Ok(())
    );
    assert_eq!(
        SemanticHistoryValue::Reference {
            namespace: SemanticHistoryNamespace::Definition,
            identity: "/etc/passwd".to_owned()
        }
        .validate(),
        Err(SemanticHistoryError::NonOpaqueIdentity("value.identity"))
    );
}
#[test]
fn an_unavailable_or_label_value_must_state_a_bounded_reason() {
    for value in [
        SemanticHistoryValue::Label {
            text: String::new(),
        },
        SemanticHistoryValue::Unavailable {
            reason: String::new(),
        },
    ] {
        assert!(matches!(
            value.validate(),
            Err(SemanticHistoryError::InvalidLabel(_))
        ));
    }
    assert_eq!(
        SemanticHistoryValue::Unavailable {
            reason: "not_observed".to_owned()
        }
        .validate(),
        Ok(())
    );
    assert_eq!(
        SemanticHistoryValue::Label {
            text: "changed".to_owned()
        }
        .validate(),
        Ok(())
    );
}
#[test]
fn an_identity_with_too_many_segments_or_bytes_is_refused() {
    assert_eq!(
        validate_history_identity("", "event_id"),
        Err(SemanticHistoryError::InvalidIdentity("event_id"))
    );
    assert_eq!(
        validate_history_identity(&"a".repeat(MAX_HISTORY_IDENTITY_BYTES + 1), "event_id"),
        Err(SemanticHistoryError::InvalidIdentity("event_id"))
    );
    assert_eq!(
        validate_history_identity(&"a".repeat(MAX_HISTORY_IDENTITY_BYTES), "event_id"),
        Ok(())
    );
    // An identity that could be read as a path is refused so a record cannot become a host read.
    for candidate in [
        "a/b",
        "a\\b",
        "a:b",
        "a.b.c.d.e.f.g.h.i",
        "a\tb",
        "file_handle",
    ] {
        assert_eq!(
            validate_history_identity(candidate, "event_id"),
            Err(SemanticHistoryError::NonOpaqueIdentity("event_id")),
            "{candidate} is not opaque"
        );
    }
    assert_eq!(
        validate_history_identity("a.b.c.d.e.f.g.h", "event_id"),
        Ok(())
    );
    assert!(!is_opaque_history_identity(""));
}
#[test]
fn a_binding_field_that_is_not_opaque_is_refused() {
    assert_eq!(fixture::binding().validate(), Ok(()));
    let mut bad: SemanticHistoryBinding = fixture::binding();
    bad.run_id = String::new();
    assert_eq!(
        bad.validate(),
        Err(SemanticHistoryError::InvalidIdentity("binding.run_id"))
    );
    bad = fixture::binding_non_opaque();
    assert_eq!(
        bad.validate(),
        Err(SemanticHistoryError::NonOpaqueIdentity("binding.run_id"))
    );
    // `same_owner` answers a scope question, so the epoch is deliberately not part of it.
    assert!(fixture::binding().same_owner(&fixture::binding_other_epoch()));
    assert!(!fixture::binding().same_owner(&fixture::binding_other_run()));
}
#[test]
fn only_a_stated_value_is_a_real_observation() {
    assert!(fixture::quantity(1, "hp").is_stated());
    assert!(
        SemanticHistoryValue::Label {
            text: "changed".to_owned()
        }
        .is_stated()
    );
    assert!(
        SemanticHistoryValue::Reference {
            namespace: SemanticHistoryNamespace::Definition,
            identity: "card_strike".to_owned()
        }
        .is_stated()
    );
    assert!(
        !SemanticHistoryValue::Unavailable {
            reason: "not_observed".to_owned()
        }
        .is_stated()
    );
}
#[test]
fn every_quantity_reporting_kind_is_named_and_the_rest_are_not() {
    let changing = SemanticHistoryKind::ALL
        .iter()
        .filter(|kind| kind.requires_quantity())
        .count();
    assert_eq!(changing, 8);
    assert_eq!(SemanticHistoryKind::ALL.len(), 14);
    for kind in SemanticHistoryKind::ALL {
        let expected = matches!(
            kind,
            SemanticHistoryKind::Damage
                | SemanticHistoryKind::Block
                | SemanticHistoryKind::Heal
                | SemanticHistoryKind::ResourceChanged
                | SemanticHistoryKind::StatusApplied
                | SemanticHistoryKind::StatusRemoved
                | SemanticHistoryKind::ModifierApplied
                | SemanticHistoryKind::ModifierRemoved
        );
        assert_eq!(kind.requires_quantity(), expected, "{}", kind.name());
        assert!(!kind.name().is_empty());
    }
    let mut names: Vec<&str> = SemanticHistoryKind::ALL
        .iter()
        .map(|kind| kind.name())
        .collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), SemanticHistoryKind::ALL.len());
}
