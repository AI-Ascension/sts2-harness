// SPDX-License-Identifier: MIT

//! Bounded exact-owner cleanup between serialized synthetic verifier tests.

use super::{
    Arc, ChildStatus, Ordering, RegistryError, SESSION_ACTIVE, SESSION_AVAILABLE, SESSION_POISONED,
    SESSION_STARTING, release_reaped_session, retained_sessions,
};

#[derive(Debug)]
pub(in super::super) enum TestReapFailure {
    Registry(RegistryError),
    UnexpectedState(u8),
    Status(ChildStatus),
    Deadline(ChildStatus),
    OwnerChanged,
    OwnerRetained,
}

impl std::fmt::Display for TestReapFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Registry(error) => write!(formatter, "registry error: {error:?}"),
            Self::UnexpectedState(state) => write!(formatter, "unexpected session state {state}"),
            Self::Status(status) => write!(formatter, "unexpected child status {status:?}"),
            Self::Deadline(status) => write!(formatter, "reap deadline expired at {status:?}"),
            Self::OwnerChanged => formatter.write_str("retained owner changed during reap"),
            Self::OwnerRetained => formatter.write_str("reaped owner remained retained"),
        }
    }
}

/// Reap the exact poisoned owner left by a test controller. Production keeps
/// uncertain ownership retained for the next bounded constructor attempt.
pub(in super::super) fn reap_poisoned_session_for_test() -> Result<(), TestReapFailure> {
    let session = retained_sessions()
        .current()
        .map_err(TestReapFailure::Registry)?;
    let Some(session) = session else {
        return Ok(());
    };

    match session.state.load(Ordering::Acquire) {
        SESSION_AVAILABLE | SESSION_ACTIVE | SESSION_STARTING => return Ok(()),
        SESSION_POISONED => {}
        state => return Err(TestReapFailure::UnexpectedState(state)),
    }

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let status = session.try_cleanup();
        match status {
            ChildStatus::Exited | ChildStatus::Absent => {
                release_reaped_session(&session);
                let current = retained_sessions()
                    .current()
                    .map_err(TestReapFailure::Registry)?;
                return match current {
                    None => Ok(()),
                    Some(current) if Arc::ptr_eq(&current, &session) => {
                        Err(TestReapFailure::OwnerRetained)
                    }
                    Some(_) => Err(TestReapFailure::OwnerChanged),
                };
            }
            ChildStatus::Alive | ChildStatus::Unknown => {
                if std::time::Instant::now() >= deadline {
                    return Err(TestReapFailure::Deadline(status));
                }
                std::thread::yield_now();
            }
            ChildStatus::Starting => return Err(TestReapFailure::Status(status)),
        }
    }
}
