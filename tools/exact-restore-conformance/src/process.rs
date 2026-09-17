// SPDX-License-Identifier: MIT

use std::fs::File;
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub(crate) struct PeerProcess {
    child: Child,
}

impl PeerProcess {
    pub(crate) fn spawn(
        executable: &Path,
        env: &[(String, String)],
        log: &Path,
    ) -> Result<Self, String> {
        let output =
            File::create(log).map_err(|error| format!("create {}: {error}", log.display()))?;
        let error = output
            .try_clone()
            .map_err(|error| format!("clone {}: {error}", log.display()))?;
        let mut command = Command::new(executable);
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .envs(env.iter().map(|(name, value)| (name, value)))
            .stdin(Stdio::null())
            .stdout(Stdio::from(output))
            .stderr(Stdio::from(error));
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let child = command
            .spawn()
            .map_err(|error| format!("spawn {}: {error}", executable.display()))?;
        Ok(Self { child })
    }

    pub(crate) fn wait_tcp(&mut self, address: &str) -> Result<(), String> {
        let (host, port) = address
            .rsplit_once(':')
            .ok_or_else(|| format!("invalid readiness address {address}"))?;
        let port = port.parse::<u16>().map_err(|_| "invalid readiness port")?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if TcpStream::connect((host, port)).is_ok() {
                return Ok(());
            }
            if let Some(status) = self.child.try_wait().map_err(|error| error.to_string())? {
                return Err(format!(
                    "peer {} exited before readiness: {status}",
                    self.child.id()
                ));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Err(format!("timed out waiting for {address}"))
    }
}

impl Drop for PeerProcess {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            #[cfg(unix)]
            if let Ok(pid) = i32::try_from(self.child.id())
                .map_err(|_| ())
                .and_then(|id| rustix::process::Pid::from_raw(id).ok_or(()))
            {
                let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
            }
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}
