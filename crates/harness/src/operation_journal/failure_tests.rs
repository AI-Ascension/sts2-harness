// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn failed_append_poison_prevents_further_authorization() -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::OpenOptions::new().write(true).open("/dev/full")?;
    let mut journal = OperationJournal {
        entries: BTreeMap::new(),
        next_sequence: 1,
        file,
        poisoned: false,
    };
    let key = JournalKey {
        principal: "p".into(),
        instance: "i".into(),
        incarnation: "c".into(),
        operation: "capture".into(),
        idempotency_key: "k".into(),
    };
    let digest = format!("sha256:{}", "a".repeat(64));
    assert!(matches!(
        journal.begin(key.clone(), &digest),
        Err(JournalError::Persistence(_))
    ));
    assert!(journal.is_empty());
    assert_eq!(
        journal.begin(key.clone(), &digest),
        Err(JournalError::Poisoned)
    );
    assert_eq!(
        journal.complete(&key, JournalOutcome::Accepted),
        Err(JournalError::Poisoned)
    );
    Ok(())
}
