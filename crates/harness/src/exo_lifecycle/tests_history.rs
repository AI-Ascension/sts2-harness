// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn completed_entry_survives_a_later_broker_turn_becoming_unknown() {
    let mut fixture = Fixture::new();
    let mut owner = fixture.owner();
    let mut effect = Effect::default();
    let StartOutcome::Started(mut handle) = owner
        .start(
            fixture.manifest.clone(),
            &fixture.input,
            &mut fixture.store,
            &fixture.fingerprint,
            &mut effect,
        )
        .expect("start")
    else {
        panic!("handle")
    };
    owner
        .poll(&mut handle, &mut fixture.store)
        .expect("complete");
    let later = owner
        .broker
        .admit_turn(
            "owner-fixture",
            &fixture.manifest.binding_id,
            &fixture.manifest.prepared_id,
            "later-turn",
        )
        .expect("later admitted");
    owner
        .broker
        .mark_sent("owner-fixture", &later.operation_id)
        .expect("later sent");
    owner
        .broker
        .mark_unknown("owner-fixture", &later.operation_id)
        .expect("later unknown");
    owner.persist().expect("retain completed historical entry");
    drop(owner);
    let reopened = fixture.reopen().expect("restart");
    assert_eq!(reopened.entries()[0].phase, LifecyclePhase::Completed);
    assert_eq!(effect.calls, 1);
}
