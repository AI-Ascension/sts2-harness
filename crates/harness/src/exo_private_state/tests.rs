// SPDX-License-Identifier: MIT

use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use super::BridgeChildScope;
use crate::ExoPrivateStatePolicy;

const CHILD_MODE: &str = "STS2_EXO_PRIVATE_STATE_CHILD_TEST";
const POLICY_ROOT: &str = "STS2_EXO_PRIVATE_STATE_POLICY_ROOT_TEST";

#[path = "tests/owned_child.rs"]
mod owned_child;

use self::owned_child::{OwnedChild, require_one_passing_test};

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> io::Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "sts2-private-state-{}-{}-{}",
            std::process::id(),
            label,
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::DirBuilder::new().mode(0o700).create(&path)?;
        Ok(Self(path))
    }

    fn policy(&self) -> ExoPrivateStatePolicy {
        ExoPrivateStatePolicy {
            state_root: self.0.join("state").display().to_string(),
            cache_root: self.0.join("cache").display().to_string(),
            temp_root: self.0.join("temp").display().to_string(),
            quota_bytes: 4 * 1024 * 1024,
            max_retention_days: 1,
            permissions_octal: 0o700,
        }
    }
}

fn io_error(error: &'static str) -> io::Error {
    io::Error::other(error)
}

fn launch_isolated_test(name: &str, mode: &str) -> io::Result<()> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("--exact")
        .arg(name)
        .arg("--nocapture")
        .arg("--test-threads=1")
        .env(CHILD_MODE, mode);
    let mut child = OwnedChild::spawn(command)?;
    let output = child.wait_until(Duration::from_secs(5))?;
    require_one_passing_test(&output)
}

#[test]
fn guarded_v2_subreaper_refuses_a_second_scope_in_one_bridge_process() {
    if std::env::var_os(CHILD_MODE).as_deref() != Some(std::ffi::OsStr::new("duplicate")) {
        return;
    }
    assert!(BridgeChildScope::enable().is_ok());
    assert_eq!(
        BridgeChildScope::enable().err(),
        Some("exo_private_subreaper_scope")
    );
}

#[test]
fn subreaper_helper_process_passes_the_no_descendant_and_escape_cases() -> io::Result<()> {
    launch_isolated_test(
        "exo_private_state::tests::children::isolated_subreaper_child",
        "escape",
    )?;
    launch_isolated_test(
        "exo_private_state::tests::children::isolated_subreaper_unwind_cleanup",
        "escape-unwind",
    )?;
    launch_isolated_test(
        "exo_private_state::tests::escaped_supervisor::isolated_abrupt_subreaper_supervisor",
        "abrupt-supervisor",
    )?;
    launch_isolated_test(
        "exo_private_state::tests::children::isolated_clean_child",
        "clean",
    )?;
    launch_isolated_test(
        "exo_private_state::tests::guarded_v2_subreaper_refuses_a_second_scope_in_one_bridge_process",
        "duplicate",
    )?;
    launch_isolated_test(
        "exo_private_state::tests::children::isolated_preexisting_live_child_is_refused_without_reaping",
        "live-child",
    )?;
    launch_isolated_test(
        "exo_private_state::tests::children::isolated_preexisting_zombie_child_is_refused_without_reaping",
        "zombie-child",
    )
}

#[path = "tests/overlapping_roots.rs"]
mod overlapping_roots;

#[path = "tests/escaped_fixture.rs"]
mod escaped_fixture;

#[path = "tests/escaped_process.rs"]
mod escaped_process;

#[path = "tests/escaped_scope.rs"]
mod escaped_scope;

#[path = "tests/escaped_supervisor.rs"]
mod escaped_supervisor;

#[path = "tests/children.rs"]
mod children;
