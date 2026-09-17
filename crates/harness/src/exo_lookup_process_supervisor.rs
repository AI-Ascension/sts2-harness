// SPDX-License-Identifier: MIT
//! Owned duplex subprocess supervision for the additive lookup agent protocol.
use std::process::Stdio;
use std::sync::mpsc::{Receiver, SyncSender};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::ExoProcessConfig;
use crate::exo_lookup_wire::EXO_LOOKUP_FRAME_BYTES;
use crate::game_information::LookupError;

pub(super) async fn supervise(
    config: ExoProcessConfig,
    commands: Receiver<Vec<u8>>,
    responses: &SyncSender<Result<Vec<u8>, LookupError>>,
    mut cancelled: tokio::sync::watch::Receiver<bool>,
    deadline: Instant,
) -> Result<(), LookupError> {
    let mut command = tokio::process::Command::new(config.executable());
    command
        .args(config.arguments())
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(target_os = "linux")]
    command.process_group(0);
    if let Some(cwd) = config.working_directory() {
        command.current_dir(cwd);
    }
    for name in config.inherited_environment() {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let mut child = command.spawn().map_err(|_| LookupError::Transport)?;
    let mut stdin = child.stdin.take().ok_or(LookupError::Transport)?;
    let mut stdout = child.stdout.take().ok_or(LookupError::Transport)?;
    let result = async {
        while let Ok(bytes) =
            commands.recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            let io = tokio::time::timeout_at(deadline.into(), async {
                stdin
                    .write_all(&bytes)
                    .await
                    .map_err(|_| LookupError::Transport)?;
                stdin.flush().await.map_err(|_| LookupError::Transport)?;
                let mut line = Vec::new();
                loop {
                    let byte = stdout.read_u8().await.map_err(|_| LookupError::Transport)?;
                    if byte == b'\n' {
                        return Ok(line);
                    }
                    if line.len() >= EXO_LOOKUP_FRAME_BYTES {
                        return Err(LookupError::Bounds);
                    }
                    line.push(byte);
                }
            });
            let response = tokio::select! {
                result = io => result.map_err(|_|LookupError::Transport)??,
                _ = cancelled.changed() => return Err(LookupError::Transport),
            };
            responses
                .send(Ok(response))
                .map_err(|_| LookupError::Transport)?;
        }
        Ok(())
    }
    .await;
    drop(stdin);
    // EOF lets the bridge cancel and reap its separately owned executor group.
    tokio::time::sleep(Duration::from_millis(250)).await;
    #[cfg(target_os = "linux")]
    if let Some(pid) = child
        .id()
        .and_then(|id| i32::try_from(id).ok())
        .and_then(rustix::process::Pid::from_raw)
    {
        let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
    }
    let _ = child.start_kill();
    let _ = tokio::time::timeout(Duration::from_secs(1), child.wait()).await;
    result
}
