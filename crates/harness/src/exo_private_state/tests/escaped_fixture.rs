// SPDX-License-Identifier: MIT
#![allow(clippy::expect_used)]

use super::{BridgeChildScope, Scratch, io_error};
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::{Child, Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};

use crate::exo_private_state::GuardedRun;

const DIRECT_CHILD_REAP_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FixturePhase {
    Preparing,
    SpawnUncertain,
    SpawnFailed,
    SpawnedUnregistered,
    Recorded,
    Quiescent,
}

/// Test-only owner for a direct executor child and its one-shot subreaper scope.
///
/// The guard cleans only after the direct child and adopted descendants are proven gone. If the
/// spawn/identity boundary is ambiguous or cleanup fails, it leaves the private attempt intact.
pub(super) struct EscapedChildFixture {
    scratch: Option<Scratch>,
    run: Option<GuardedRun>,
    scope: Option<BridgeChildScope>,
    child: Option<Child>,
    phase: FixturePhase,
    finish_child_attempted: bool,
    finish_run_attempted: bool,
    finished: bool,
}

impl EscapedChildFixture {
    pub(super) fn create(label: &str, config_digest: &str) -> io::Result<Self> {
        let scratch = Scratch::new(label)?;
        let run = match GuardedRun::create(&scratch.policy(), config_digest) {
            Ok(run) => run,
            Err(error) => {
                if !matches!(
                    error,
                    "exo_private_partial_cleanup" | "exo_private_initialization_cleanup"
                ) {
                    fs::remove_dir_all(&scratch.0)?;
                }
                return Err(io_error(error));
            }
        };
        Ok(Self {
            scratch: Some(scratch),
            run: Some(run),
            scope: None,
            child: None,
            phase: FixturePhase::Preparing,
            finish_child_attempted: false,
            finish_run_attempted: false,
            finished: false,
        })
    }

    pub(super) fn paths(&self) -> &super::super::RunPaths {
        self.run
            .as_ref()
            .expect("fixture run remains owned until explicit cleanup")
            .paths()
    }

    pub(super) fn scratch_path(&self) -> &Path {
        &self
            .scratch
            .as_ref()
            .expect("fixture scratch remains owned until explicit cleanup")
            .0
    }

    pub(super) fn enable_scope(&mut self) -> io::Result<()> {
        self.scope = Some(BridgeChildScope::enable().map_err(io_error)?);
        Ok(())
    }

    pub(super) fn spawn(&mut self, mut command: Command) -> io::Result<()> {
        let scope = self
            .scope
            .as_ref()
            .ok_or_else(|| io::Error::other("fixture subreaper scope is not enabled"))?;
        self.run
            .as_mut()
            .ok_or_else(|| io::Error::other("fixture run is unavailable"))?
            .begin_spawn()
            .map_err(io_error)?;
        self.phase = FixturePhase::SpawnUncertain;
        match command.spawn() {
            Ok(child) => {
                self.child = Some(child);
                self.phase = FixturePhase::SpawnedUnregistered;
                Ok(())
            }
            Err(error) => {
                match self
                    .run
                    .as_mut()
                    .expect("fixture run remains owned")
                    .spawn_failed(scope)
                {
                    Ok(()) => self.phase = FixturePhase::SpawnFailed,
                    Err(cleanup) => report_cleanup_failure(
                        "private-state test spawn-error cleanup refused",
                        cleanup,
                    ),
                }
                Err(error)
            }
        }
    }

    pub(super) fn record_child(&mut self) -> io::Result<()> {
        let pid = self
            .child
            .as_ref()
            .ok_or_else(|| io::Error::other("fixture child was not spawned"))?
            .id();
        self.run
            .as_mut()
            .ok_or_else(|| io::Error::other("fixture run is unavailable"))?
            .record_child(pid)
            .map_err(io_error)?;
        self.phase = FixturePhase::Recorded;
        Ok(())
    }

    pub(super) fn send_trigger(&mut self) -> io::Result<()> {
        let mut input = self
            .child
            .as_mut()
            .and_then(|child| child.stdin.take())
            .ok_or_else(|| io::Error::other("fixture child stdin was not piped"))?;
        input.write_all(b"start\n")
    }

    pub(super) fn wait_direct_child(&mut self, timeout: Duration) -> io::Result<ExitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            let child = self
                .child
                .as_mut()
                .ok_or_else(|| io::Error::other("fixture child was already reaped"))?;
            if let Some(status) = child.try_wait()? {
                self.child.take();
                return Ok(status);
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "direct fixture child exceeded its bounded wait",
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    pub(super) fn finish_child(&mut self) -> io::Result<bool> {
        if self.phase != FixturePhase::Recorded || self.finish_child_attempted {
            return Err(io::Error::other("fixture child cleanup is not available"));
        }
        self.finish_child_attempted = true;
        let clean = self
            .run
            .as_mut()
            .ok_or_else(|| io::Error::other("fixture run is unavailable"))?
            .finish_child(
                self.scope
                    .as_ref()
                    .ok_or_else(|| io::Error::other("fixture scope is unavailable"))?,
            )
            .map_err(io_error)?;
        self.phase = FixturePhase::Quiescent;
        Ok(clean)
    }

    pub(super) fn finish_run(&mut self) -> io::Result<()> {
        self.finish_run_and_remove_scratch()
    }

    fn reap_direct_child(&mut self) -> Result<(), &'static str> {
        let Some(child) = self.child.as_mut() else {
            return Ok(());
        };
        if child
            .try_wait()
            .map_err(|_| "fixture_child_wait")?
            .is_none()
        {
            let _ = child.kill();
        }
        let deadline = Instant::now() + DIRECT_CHILD_REAP_TIMEOUT;
        loop {
            if child
                .try_wait()
                .map_err(|_| "fixture_child_wait")?
                .is_some()
            {
                self.child.take();
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err("fixture_child_reap_timeout");
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn drain_unregistered_children(&mut self) -> Result<(), &'static str> {
        self.scope
            .as_ref()
            .ok_or("fixture_scope_unavailable")?
            .drain()
            .map(|_| ())
    }

    fn finish_run_and_remove_scratch(&mut self) -> io::Result<()> {
        if self.finish_run_attempted {
            return Err(io::Error::other(
                "fixture run cleanup was already attempted",
            ));
        }
        if !matches!(
            self.phase,
            FixturePhase::Preparing | FixturePhase::SpawnFailed | FixturePhase::Quiescent
        ) {
            return Err(io::Error::other("fixture run is not proven quiescent"));
        }
        self.finish_run_attempted = true;
        self.run
            .as_mut()
            .ok_or_else(|| io::Error::other("fixture run is unavailable"))?
            .finish()
            .map_err(io_error)?;
        self.run.take();
        let scratch_path = self
            .scratch
            .as_ref()
            .ok_or_else(|| io::Error::other("fixture scratch is unavailable"))?
            .0
            .clone();
        fs::remove_dir_all(scratch_path)?;
        self.scratch.take();
        self.finished = true;
        Ok(())
    }

    fn cleanup_on_drop(&mut self) {
        if self.finished {
            return;
        }
        if let Err(error) = self.reap_direct_child() {
            report_cleanup_failure("private-state test direct-child cleanup incomplete", error);
        }
        match self.phase {
            FixturePhase::Preparing | FixturePhase::SpawnFailed | FixturePhase::Quiescent => {
                if let Err(error) = self.finish_run_and_remove_scratch() {
                    report_cleanup_failure("private-state test run cleanup refused", error);
                }
            }
            FixturePhase::Recorded if !self.finish_child_attempted => {
                self.finish_child_attempted = true;
                let cleanup = self
                    .run
                    .as_mut()
                    .ok_or("fixture_run_unavailable")
                    .and_then(|run| {
                        self.scope
                            .as_ref()
                            .ok_or("fixture_scope_unavailable")
                            .and_then(|scope| run.finish_child(scope))
                    });
                match cleanup {
                    Ok(_) => {
                        self.phase = FixturePhase::Quiescent;
                        if let Err(error) = self.finish_run_and_remove_scratch() {
                            report_cleanup_failure(
                                "private-state test unwind cleanup refused",
                                error,
                            );
                        }
                    }
                    Err(error) => {
                        report_cleanup_failure("private-state test unwind cleanup refused", error);
                    }
                }
            }
            FixturePhase::Recorded => report_cleanup_failure(
                "private-state test preserves paths after an earlier cleanup error",
                "cleanup was already attempted",
            ),
            FixturePhase::SpawnUncertain | FixturePhase::SpawnedUnregistered => {
                if let Err(error) = self.drain_unregistered_children() {
                    report_cleanup_failure(
                        "private-state test ambiguous-child drain refused",
                        error,
                    );
                }
                report_cleanup_failure(
                    "private-state test preserves paths for an unregistered spawn",
                    "spawn identity was not durably recorded",
                );
            }
        }
    }
}

impl Drop for EscapedChildFixture {
    fn drop(&mut self) {
        self.cleanup_on_drop();
    }
}

pub(super) fn report_cleanup_failure(context: &str, error: impl std::fmt::Display) {
    let _ = writeln!(io::stderr().lock(), "{context}: {error}");
}
