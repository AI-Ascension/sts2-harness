// SPDX-License-Identifier: MIT

//! Immutable endpoint selection from owner launch configuration, never from a request.

use std::path::PathBuf;
use std::time::Duration;
use sts2_harness::worker_bootstrap::WorkerBootstrap;
use sts2_harness::worker_local_linux::LinuxWorkerConfig;

pub(super) struct ListenerSettings {
    pub(super) transport: LinuxWorkerConfig,
    pub(super) exchange_timeout: Duration,
}

impl ListenerSettings {
    pub(super) fn from_environment(bootstrap: &WorkerBootstrap) -> Result<Self, String> {
        Self::from_values(
            &required("STS2_WORKER_ENDPOINT")?,
            &required("STS2_WORKER_CREDENTIAL_PATH")?,
            &required("STS2_WORKER_TIMEOUT_MS")?,
            bootstrap,
        )
    }

    fn from_values(
        endpoint: &str,
        credential: &str,
        timeout: &str,
        bootstrap: &WorkerBootstrap,
    ) -> Result<Self, String> {
        let millis = timeout
            .parse::<u64>()
            .ok()
            .filter(|value| (1..=5_000).contains(value) && value.to_string() == timeout)
            .ok_or_else(|| {
                String::from("STS2_WORKER_TIMEOUT_MS must be a canonical integer in 1..=5000")
            })?;
        let transport = LinuxWorkerConfig::from_bootstrap(
            PathBuf::from(endpoint),
            PathBuf::from(credential),
            "harness",
            bootstrap,
        )
        .map_err(|error| error.to_string())?;
        Ok(Self {
            transport,
            exchange_timeout: Duration::from_millis(millis),
        })
    }
}

fn required(name: &str) -> Result<String, String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{name} must contain a nonempty UTF-8 launch value"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_launch_policy_rejects_noncanonical_bounds_and_paths() -> Result<(), String> {
        let json =
            include_bytes!("../../../../../protocol-artifact/worker-bootstrap-v1/valid/linux.json");
        let mut frame = sts2_harness::worker_bootstrap::BOOTSTRAP_MAGIC.to_vec();
        frame.extend_from_slice(
            &u32::try_from(json.len())
                .map_err(|_| "fixture length")?
                .to_be_bytes(),
        );
        frame.extend_from_slice(json);
        let bootstrap = WorkerBootstrap::decode(&frame).map_err(|error| error.to_string())?;
        for timeout in [
            "0",
            "5001",
            "01",
            "+1",
            " 1",
            "1.0",
            "",
            "18446744073709551616",
        ] {
            assert!(
                ListenerSettings::from_values(
                    "/run/synthetic/worker.sock",
                    "/run/synthetic/secret",
                    timeout,
                    &bootstrap
                )
                .is_err()
            );
        }
        for timeout in ["1", "5000"] {
            assert!(
                ListenerSettings::from_values(
                    "/run/synthetic/worker.sock",
                    "/run/synthetic/secret",
                    timeout,
                    &bootstrap
                )
                .is_ok()
            );
        }
        for (endpoint, credential) in [
            ("relative", "/run/synthetic/secret"),
            ("/run/synthetic/same", "/run/synthetic/same"),
            ("/run/../worker.sock", "/run/synthetic/secret"),
        ] {
            assert!(
                ListenerSettings::from_values(endpoint, credential, "5000", &bootstrap).is_err()
            );
        }
        Ok(())
    }
}
