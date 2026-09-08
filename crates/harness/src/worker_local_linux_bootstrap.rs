// SPDX-License-Identifier: MIT

//! Convert validated launch policy into native expected-peer configuration.
//! This performs no I/O and creates no authentication witness.

use std::path::PathBuf;

use crate::worker_bootstrap::{ExpectedBootstrapPeer, WorkerBootstrap};

use super::{LinuxPeerIdentity, LinuxTransportError, LinuxWorkerConfig};

impl LinuxWorkerConfig {
    /// Bind the owner-configured component and paths to the startup frame's
    /// exact expected process. Live identity and credential checks occur only
    /// when the listener is bound and accepts a connection.
    pub fn from_bootstrap(
        endpoint: PathBuf,
        credential: PathBuf,
        expected_component: &str,
        bootstrap: &WorkerBootstrap,
    ) -> Result<Self, LinuxTransportError> {
        if bootstrap.component_id() != expected_component {
            return Err(LinuxTransportError::Configuration);
        }
        let ExpectedBootstrapPeer::Linux {
            pid,
            creation_token,
            executable,
            executable_sha256,
            uid,
            gid,
        } = bootstrap.expected_peer()
        else {
            return Err(LinuxTransportError::Configuration);
        };
        let mut digest = [0_u8; 32];
        for (output, pair) in digest
            .iter_mut()
            .zip(executable_sha256.as_bytes().chunks_exact(2))
        {
            let pair = std::str::from_utf8(pair).map_err(|_| LinuxTransportError::Configuration)?;
            *output =
                u8::from_str_radix(pair, 16).map_err(|_| LinuxTransportError::Configuration)?;
        }
        let peer = LinuxPeerIdentity::new(
            *uid,
            *gid,
            *pid,
            creation_token
                .parse()
                .map_err(|_| LinuxTransportError::Configuration)?,
            PathBuf::from(executable),
            digest,
        )?;
        Self::new(endpoint, credential, peer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bootstrap(platform: &str) -> Result<WorkerBootstrap, Box<dyn std::error::Error>> {
        let bytes: &[u8] = match platform {
            "linux" => {
                include_bytes!("../../../protocol-artifact/worker-bootstrap-v1/valid/linux.json")
            }
            _ => {
                include_bytes!("../../../protocol-artifact/worker-bootstrap-v1/valid/windows.json")
            }
        };
        let mut frame = b"ASC-WB01".to_vec();
        frame.extend_from_slice(&u32::try_from(bytes.len())?.to_be_bytes());
        frame.extend_from_slice(bytes);
        Ok(WorkerBootstrap::decode(&frame)?)
    }

    #[test]
    fn bootstrap_configuration_preserves_every_native_policy_field()
    -> Result<(), Box<dyn std::error::Error>> {
        let bootstrap = bootstrap("linux")?;
        let config = LinuxWorkerConfig::from_bootstrap(
            "/run/synthetic/worker.sock".into(),
            "/run/synthetic/worker.secret".into(),
            bootstrap.component_id(),
            &bootstrap,
        )?;
        let ExpectedBootstrapPeer::Linux {
            pid,
            uid,
            gid,
            creation_token,
            executable,
            executable_sha256,
        } = bootstrap.expected_peer()
        else {
            return Err("fixture platform mismatch".into());
        };
        assert_eq!(config.peer.pid, *pid);
        assert_eq!(config.peer.uid, *uid);
        assert_eq!(config.peer.gid, *gid);
        assert_eq!(config.peer.start_token.to_string(), *creation_token);
        assert_eq!(config.peer.executable, PathBuf::from(executable));
        let rendered: String = config
            .peer
            .executable_sha256
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(rendered, *executable_sha256);
        Ok(())
    }

    #[test]
    fn bootstrap_configuration_rejects_component_platform_and_path_mismatch()
    -> Result<(), Box<dyn std::error::Error>> {
        let linux = bootstrap("linux")?;
        let windows = bootstrap("windows")?;
        for (frame, component, endpoint, credential) in [
            (
                &linux,
                "wrong-component",
                "/run/synthetic/worker.sock",
                "/run/synthetic/worker.secret",
            ),
            (
                &windows,
                windows.component_id(),
                "/run/synthetic/worker.sock",
                "/run/synthetic/worker.secret",
            ),
            (
                &linux,
                linux.component_id(),
                "relative.sock",
                "/run/synthetic/worker.secret",
            ),
            (
                &linux,
                linux.component_id(),
                "/run/synthetic/same",
                "/run/synthetic/same",
            ),
        ] {
            assert!(matches!(
                LinuxWorkerConfig::from_bootstrap(
                    endpoint.into(),
                    credential.into(),
                    component,
                    frame
                ),
                Err(LinuxTransportError::Configuration)
            ));
        }
        Ok(())
    }
}
