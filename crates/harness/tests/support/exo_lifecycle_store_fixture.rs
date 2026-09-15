// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use super::fixture::{Fixture, Handle};
use std::cell::Cell;
use std::rc::Rc;
use sts2_harness::exo_lifecycle::*;
use sts2_harness::*;

pub struct CountedEffect {
    pub polls: Rc<Cell<usize>>,
    pub error: bool,
    pub units: Option<u64>,
}
pub struct CountedHandle {
    polls: Rc<Cell<usize>>,
    error: bool,
    inner: Handle,
}
impl EffectPort for CountedEffect {
    type Handle = CountedHandle;
    fn try_start(&mut self, _: SendPermit, _: &[u8]) -> Result<Self::Handle, LifecycleError> {
        Ok(CountedHandle {
            polls: self.polls.clone(),
            error: self.error,
            inner: Handle {
                ready: true,
                units: self.units,
            },
        })
    }
}
impl EffectHandle for CountedHandle {
    fn poll(&mut self) -> Result<Option<EffectCompletion>, LifecycleError> {
        self.polls.set(self.polls.get() + 1);
        if self.error {
            Err(LifecycleError::Unavailable)
        } else {
            self.inner.poll()
        }
    }
}

pub fn seed_foreign(original: &Fixture, exact: bool) -> (ExecutionStore, String) {
    let mut store = ExecutionStore::open_in_memory().expect("foreign store");
    let m = &original.manifest;
    let lineage = if exact {
        ExecutionLineage::new(
            &m.scope.run_id,
            &m.scope.episode_id,
            &m.episode_attempt_id,
            &m.trajectory_id,
        )
    } else {
        ExecutionLineage::new(
            "foreign-run",
            "foreign-episode",
            "foreign-attempt",
            "foreign-trajectory",
        )
    }
    .expect("lineage");
    let execution = if exact {
        m.execution_id.as_str()
    } else {
        "foreign-execution"
    };
    store
        .start_episode(&lineage, &original.fingerprint)
        .expect("episode");
    let decision = DecisionReference::new(
        lineage.clone(),
        execution,
        if exact {
            &m.input_digest
        } else {
            "foreign-input"
        },
        if exact {
            &m.model_revision
        } else {
            "foreign-model"
        },
        if exact {
            &m.config_digest
        } else {
            "foreign-config"
        },
    )
    .expect("decision");
    store.record_decision(&decision).expect("record");
    store
        .reserve_provider(
            &ProviderReservation::new(
                lineage,
                &m.reservation_id,
                execution,
                if exact {
                    &m.provider_attempt_id
                } else {
                    "foreign-provider"
                },
                m.reserved_units,
            )
            .expect("reservation"),
        )
        .expect("reserve");
    (store, execution.to_owned())
}

pub struct MutatingEffect {
    pub path: std::path::PathBuf,
    pub error: bool,
}
pub struct MutatingHandle {
    path: std::path::PathBuf,
    error: bool,
}
impl EffectPort for MutatingEffect {
    type Handle = MutatingHandle;
    fn try_start(&mut self, _: SendPermit, _: &[u8]) -> Result<Self::Handle, LifecycleError> {
        Ok(MutatingHandle {
            path: self.path.clone(),
            error: self.error,
        })
    }
}
impl EffectHandle for MutatingHandle {
    fn poll(&mut self) -> Result<Option<EffectCompletion>, LifecycleError> {
        // Synthetic external corruption during the callback, after the initial poll fence.
        let database = rusqlite::Connection::open(&self.path).expect("synthetic database");
        database
            .execute_batch(
                "UPDATE provider_reservations SET provider_execution_id = 'changed-during-poll'",
            )
            .expect("synthetic corruption");
        if self.error {
            Err(LifecycleError::Unavailable)
        } else {
            Handle {
                ready: true,
                units: Some(3),
            }
            .poll()
        }
    }
}
