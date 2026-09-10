// SPDX-License-Identifier: MIT

use super::*;

fn identity(participants: Vec<String>) -> Result<ReceiptQueryIdentity, ReceiptQueryIdentityError> {
    ReceiptQueryIdentity::new(
        "op:1",
        ReceiptQueryActionKind::PlayCard,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "session-1",
        "run-1",
        ReceiptQueryLocation::new(1, Some(2), Some(ReceiptQueryCoordinate::new(3, 4))),
        "peer-1",
        "host-1",
        "epoch-1",
        7,
        7,
        participants,
    )
}

#[test]
fn identity_requires_sorted_actor_member_set() {
    assert_eq!(
        identity(vec!["peer-2".into(), "peer-1".into()]).map(|_| ()),
        Err(ReceiptQueryIdentityError::InvalidParticipants)
    );
    assert_eq!(
        identity(vec!["peer-2".into(), "peer-3".into()]).map(|_| ()),
        Err(ReceiptQueryIdentityError::InvalidParticipants)
    );
}
