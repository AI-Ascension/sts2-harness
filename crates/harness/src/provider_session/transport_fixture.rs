// SPDX-License-Identifier: MIT

use super::config::NativeProcessConfig;
use super::{NativeTransportError, OwnedNativeTransport, fixture_peer_executable};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static FIXTURE_ROOT_COUNTER: AtomicU64 = AtomicU64::new(1);

impl OwnedNativeTransport {
    /// Construct the compiled offline peer without accepting a caller-selected executable,
    /// arguments, environment or credential.  The state root is a private, owner-restricted
    /// temporary directory so the fixture does not depend on the caller's checkout permissions.
    /// A real native profile must supply its own reviewed broker-owned factory rather than widening
    /// this fixture constructor.
    pub fn fixture_peer() -> Result<Self, NativeTransportError> {
        Ok(Self::new(Self::fixture_peer_config(
            private_fixture_state_root()?,
        )?))
    }

    /// Construct the compiled offline peer bound to an explicit state root, creating it with
    /// owner-only permissions when absent.  Tests use this to own and clean up their own root.
    pub fn fixture_peer_at(state_root: PathBuf) -> Result<Self, NativeTransportError> {
        Ok(Self::new(Self::fixture_peer_config(state_root)?))
    }

    fn fixture_peer_config(
        state_root: PathBuf,
    ) -> Result<NativeProcessConfig, NativeTransportError> {
        let executable = fixture_peer_executable().ok_or(NativeTransportError::Unavailable)?;
        prepare_private_fixture_root(&state_root)?;
        NativeProcessConfig::new(
            executable.to_string_lossy().into_owned(),
            Vec::new(),
            state_root.clone(),
            Vec::new(),
            state_root,
        )
        .map_err(|_| NativeTransportError::Unavailable)
    }

    /// The private state root the owned child is bound to.
    #[must_use]
    pub fn state_root(&self) -> &Path {
        self.config.state_root()
    }
}

fn private_fixture_state_root() -> Result<PathBuf, NativeTransportError> {
    let counter = FIXTURE_ROOT_COUNTER.fetch_add(1, Ordering::Relaxed);
    Ok(std::env::temp_dir().join(format!(
        "ascension-provider-fixture-peer-{}-{counter}",
        std::process::id()
    )))
}

fn prepare_private_fixture_root(path: &Path) -> Result<(), NativeTransportError> {
    std::fs::create_dir_all(path).map_err(|_| NativeTransportError::Unavailable)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| NativeTransportError::Unavailable)?;
    }
    Ok(())
}
