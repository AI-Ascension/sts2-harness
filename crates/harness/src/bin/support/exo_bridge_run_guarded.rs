// SPDX-License-Identifier: MIT
//! Guarded-v2 executor launch and private-state lifecycle.

use super::*;

pub(super) fn execute_guarded(
    loaded: &Loaded,
    envelope: ExoBridgeRequestEnvelope,
    synthetic: bool,
    wire_v2: bool,
) -> Result<Vec<u8>, &'static str> {
    let credential = if synthetic {
        String::from("sts2-synthetic-model-key")
    } else {
        std::env::var("STS2_EXO_MODEL_KEY").map_err(|_| "exo_bridge_credentials_unavailable")?
    };
    if credential.is_empty() {
        return Err("exo_bridge_credentials_unavailable");
    }
    let policy = loaded
        .guarded_private_state()
        .ok_or("exo_bridge_private_profile")?;
    let mut guarded = GuardedRun::create(policy, &loaded.digest)?;
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => {
            guarded.finish()?;
            return Err("exo_bridge_runtime");
        }
    };
    let exchange = runtime.block_on(exchange_guarded(
        loaded,
        &mut guarded,
        &envelope,
        credential,
    ));
    let cleanup = guarded.finish();
    let bytes = exchange?;
    cleanup?;
    if wire_v2 {
        response_v2(&envelope, &bytes)
    } else {
        response(&envelope, &bytes)
    }
}

fn guarded_invocation(
    loaded: &Loaded,
    envelope: &ExoBridgeRequestEnvelope,
    private: &ExecutorPrivateState,
    credential: String,
) -> Result<Vec<u8>, &'static str> {
    let request = &envelope.request;
    serde_json::to_vec(&json!({
        "version": "sts2.exo-executor-input-v2",
        "request_id": envelope.request_id,
        "host_turn_id": envelope.turn_id,
        "model": loaded.config.model,
        "endpoint": loaded.config.endpoint,
        "module_path": loaded.config.extension,
        "source_root": loaded.config.source_root,
        "state_root": private.paths.state_root,
        "input": {
            "observation": request.observation,
            "legal_action_ids": request.legal_action_ids,
            "objective": request.objective,
            "hard_constraints": request.hard_constraints
        },
        "timeout_millis": 115_000,
        "max_output_tokens": 4096,
        "credential": credential,
        "private_state": private
    }))
    .map_err(|_| "exo_bridge_input")
}

pub(super) fn guarded_executor_command(
    loaded: &Loaded,
    paths: &RunPaths,
) -> Result<tokio::process::Command, &'static str> {
    let mut command = tokio::process::Command::new(&loaded.config.executor);
    command.as_std_mut().process_group(0);
    command
        .env_clear()
        .env(
            "PATH",
            loaded.config.node.parent().ok_or("exo_bridge_node")?,
        )
        .env("XDG_CONFIG_HOME", &paths.config_root)
        .env("XDG_CACHE_HOME", &paths.cache_root)
        .env("TMPDIR", &paths.temp_root)
        .env(
            "EXO_LITELLM_PRICES_PATH",
            paths.cache_root.join("no-prices.json"),
        )
        .env("STS2_EXO_ALLOWED_ENDPOINT", &loaded.config.endpoint)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    Ok(command)
}

async fn exchange_guarded(
    loaded: &Loaded,
    guarded: &mut GuardedRun,
    envelope: &ExoBridgeRequestEnvelope,
    credential: String,
) -> Result<Vec<u8>, &'static str> {
    guarded.verify_quota()?;
    let mut command = guarded_executor_command(loaded, guarded.paths())?;
    let child_scope = BridgeChildScope::enable()?;
    guarded.begin_spawn()?;
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => {
            guarded.spawn_failed(&child_scope)?;
            return Err("exo_bridge_executor_unavailable");
        }
    };

    let mut leader_reaped = false;
    let operation = async {
        let pid = child.id().ok_or("exo_bridge_executor_unavailable")?;
        let private = guarded.record_child(pid)?;
        guarded.verify_quota()?;
        let invocation = guarded_invocation(loaded, envelope, &private, credential)?;
        let mut input = child.stdin.take().ok_or("exo_bridge_executor_pipe")?;
        let mut output = child.stdout.take().ok_or("exo_bridge_executor_pipe")?;
        let io = async {
            let write = async move {
                input.write_all(&invocation).await?;
                input.shutdown().await
            };
            let read = async {
                let mut bytes = Vec::new();
                (&mut output).take(16_385).read_to_end(&mut bytes).await?;
                Ok::<_, std::io::Error>(bytes)
            };
            let ((), bytes) = tokio::try_join!(write, read)?;
            Ok::<_, std::io::Error>(bytes)
        };
        let io_result = tokio::time::timeout(std::time::Duration::from_secs(118), async {
            tokio::pin!(io);
            let mut quota_poll = tokio::time::interval(std::time::Duration::from_millis(250));
            loop {
                tokio::select! {
                    result = &mut io => break result.map_err(|_| "exo_bridge_executor_failed")?,
                    _ = quota_poll.tick() => { guarded.verify_quota()?; }
                }
            }
        })
        .await
        .map_err(|_| "exo_bridge_executor_timeout")??;
        if io_result.len() > 16_384 {
            return Err("exo_bridge_executor_failed");
        }
        let status = tokio::time::timeout(std::time::Duration::from_secs(1), child.wait())
            .await
            .map_err(|_| "exo_bridge_executor_timeout")?
            .map_err(|_| "exo_bridge_executor_failed")?;
        leader_reaped = true;
        if !status.success() {
            return Err("exo_bridge_executor_failed");
        }
        Ok(io_result)
    }
    .await;

    if !leader_reaped {
        terminate_and_wait(&mut child).await?;
        leader_reaped = true;
    }
    if !leader_reaped {
        return Err("exo_bridge_executor_timeout");
    }
    let no_descendants = guarded.finish_child(&child_scope)?;
    if !no_descendants {
        return Err("exo_bridge_executor_descendant");
    }
    operation
}

async fn terminate_and_wait(child: &mut tokio::process::Child) -> Result<(), &'static str> {
    if child.id().is_some() {
        let _ = child.start_kill();
        tokio::time::timeout(std::time::Duration::from_secs(1), child.wait())
            .await
            .map_err(|_| "exo_bridge_executor_timeout")?
            .map_err(|_| "exo_bridge_executor_failed")?;
    }
    Ok(())
}
