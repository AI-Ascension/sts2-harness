// SPDX-License-Identifier: MIT

    struct ActiveExecution {
        handoff_id: String,
        handle: thread::JoinHandle<WorkerExecutionCompletion>,
    }

    fn serve(
        listener: EndpointListener,
        config: &EndpointConfig,
        bootstrap: &Bootstrap,
        runtime: &mut WorkerRuntime,
    ) -> Result<(), String> {
        let owner_proof = WorkerOwnerProof::new(owner_proof_label())
            .map_err(|_| String::from("worker owner proof is invalid"))?;
        let mut active = None;
        let mut auth_slots = AuthSlotBudget::default();
        let mut pending_authentication = Vec::new();
        let mut listener = listener;
        loop {
            reap_execution(&mut active, runtime)?;
            let mut progressed = false;
            if let Some(result) = reap_authentication(
                &mut pending_authentication,
                &mut auth_slots,
            ) {
                progressed = true;
                if let Ok(connection) = result {
                    handle_authenticated_connection(
                        connection.stream,
                        connection.peer,
                        config,
                        runtime,
                        &owner_proof,
                        &mut active,
                    )?;
                }
            }
            if auth_slots.try_acquire() {
                match accept_endpoint(&mut listener)? {
                    Some(stream) => {
                        progressed = true;
                        let peer = bootstrap.peer.clone();
                        let credential_path = config.credential_path.clone();
                        let handle = thread::Builder::new()
                            .name(String::from("sts2-worker-authentication"))
                            .spawn(move || {
                                authenticate_connection_owned(
                                    stream,
                                    peer,
                                    credential_path,
                                )
                            })
                            .map_err(|_| {
                                String::from("worker authentication thread could not start")
                            })?;
                        pending_authentication.push(PendingAuthentication { handle });
                    }
                    None => {
                        auth_slots.release();
                    }
                }
            }
            if !progressed {
                thread::sleep(POLL_INTERVAL);
            }
        }
    }

    struct PendingAuthentication {
        handle: thread::JoinHandle<Result<AuthenticatedConnection, String>>,
    }

    struct AuthenticatedConnection {
        stream: EndpointStream,
        peer: PeerSession,
    }

    #[derive(Default)]
    struct AuthSlotBudget {
        active: usize,
    }

    impl AuthSlotBudget {
        fn try_acquire(&mut self) -> bool {
            if self.active >= MAX_AUTH_SLOTS {
                return false;
            }
            self.active += 1;
            true
        }

        fn release(&mut self) {
            if self.active > 0 {
                self.active -= 1;
            }
        }
    }

    fn reap_authentication(
        pending: &mut Vec<PendingAuthentication>,
        slots: &mut AuthSlotBudget,
    ) -> Option<Result<AuthenticatedConnection, String>> {
        let index = pending
            .iter()
            .position(|authentication| authentication.handle.is_finished())?;
        let authentication = pending.swap_remove(index);
        slots.release();
        Some(
            authentication
                .handle
                .join()
                .unwrap_or_else(|_| Err(String::from("worker authentication thread failed"))),
        )
    }

    fn reap_execution(
        active: &mut Option<ActiveExecution>,
        runtime: &mut WorkerRuntime,
    ) -> Result<(), String> {
        let finished = active
            .as_ref()
            .is_some_and(|execution| execution.handle.is_finished());
        if !finished {
            return Ok(());
        }
        let execution = active
            .take()
            .ok_or_else(|| String::from("worker execution disappeared"))?;
        let completion = execution.handle.join().map_err(|_| {
            let _ = runtime.retain_unknown(&execution.handoff_id);
            String::from("worker execution thread failed")
        })?;
        runtime.complete_execution(completion)
    }

    fn handle_authenticated_connection(
        stream: EndpointStream,
        peer: PeerSession,
        config: &EndpointConfig,
        runtime: &mut WorkerRuntime,
        owner_proof: &WorkerOwnerProof,
        active: &mut Option<ActiveExecution>,
    ) -> Result<(), String> {
        let mut transport = stream;
        let request_bytes = read_transport_frame(&mut transport, &peer, TRANSPORT_TIMEOUT)?;
        let request = WorkerRequest::decode(&request_bytes)
            .map_err(|_| String::from("worker request is invalid"))?;
        let capability = capability_for(request.command());
        let authenticated = AuthenticatedWorkerRequest::from_transport(
            request.clone(),
            capability,
            owner_proof.clone(),
        );
        let exchange = runtime.handle_authenticated(&authenticated)?;
        let (reply, reservation) = exchange.into_parts();
        let response = match request.encode_response(runtime.worker_boot_id(), reply) {
            Ok(response) => response,
            Err(_) => {
                let _ = runtime.finish_reservation(reservation, ResponseWriteStatus::Failed);
                return Err(String::from("worker response could not be encoded"));
            }
        };
        let response_status = match write_transport_frame(&mut transport, &response, &peer) {
            Ok(()) => ResponseWriteStatus::Written,
            Err(error) => {
                let _ = runtime.finish_reservation(reservation, ResponseWriteStatus::Failed);
                return Err(error);
            }
        };
        let outcome = runtime.finish_reservation(reservation, response_status)?;
        if let WorkerStartOutcome::Started(handoff) = outcome {
            start_execution(*handoff, config, runtime, active)?;
        }
        Ok(())
    }

    fn capability_for(command: WorkerCommand) -> WorkerCapability {
        match command {
            WorkerCommand::Probe => WorkerCapability::Probe,
            WorkerCommand::Dispatch => WorkerCapability::Dispatch,
            WorkerCommand::Lookup => WorkerCapability::Lookup,
            WorkerCommand::Acknowledge => WorkerCapability::Acknowledge,
            WorkerCommand::SetControlMode => WorkerCapability::SetControlMode,
        }
    }

    fn start_execution(
        handoff: crate::StoredWorkerHandoff,
        config: &EndpointConfig,
        runtime: &mut WorkerRuntime,
        active: &mut Option<ActiveExecution>,
    ) -> Result<(), String> {
        if active.is_some() {
            return Err(String::from(
                "worker execution lane is unexpectedly occupied",
            ));
        }
        let handoff_id = handoff.tuple.handoff_id.clone();
        let task = runtime.take_execution(handoff)?;
        let child_config = ChildConfig::from_endpoint(config);
        let thread_handoff_id = handoff_id.clone();
        let handle = thread::Builder::new()
            .name(String::from("sts2-worker-execution"))
            .spawn(move || task.run(|task| run_runtime_child(task, &child_config)))
            .map_err(|_| String::from("worker execution thread could not start"))?;
        *active = Some(ActiveExecution {
            handoff_id: thread_handoff_id,
            handle,
        });
        Ok(())
    }

    struct ChildConfig {
        executable: ApprovedExecutable,
        environment: Vec<(OsString, OsString)>,
        store_path: PathBuf,
        fingerprint: ExecutionFingerprint,
    }

    impl ChildConfig {
        fn from_endpoint(config: &EndpointConfig) -> Self {
            Self {
                executable: config.runtime_executable.clone(),
                environment: config.environment.clone(),
                store_path: config.execution_store_path.clone(),
                fingerprint: config.fingerprint.clone(),
            }
        }
    }

    fn run_runtime_child(task: &WorkerExecutionTask, config: &ChildConfig) -> Result<(), String> {
        let tuple = &task.running().tuple;
        config.executable.verify_before_launch()?;
        let mut command = std::process::Command::new(config.executable.command_path());
        command
            .arg("--resume")
            .env_clear()
            .envs(config.environment.iter().map(|(key, value)| (key, value)))
            .env("STS2_RUNTIME_PROFILE", "runtime-v3-gameplay")
            .env("STS2_RESUME", "true")
            .env("STS2_APPROVED_WORKER_FINGERPRINT", "true")
            .env("STS2_EXECUTION_STORE_PATH", &config.store_path)
            .env("STS2_RUN_ID", &tuple.run_id)
            .env("STS2_EPISODE_ID", &tuple.episode_id)
            .env("STS2_ATTEMPT_ID", &tuple.attempt_id)
            .env("STS2_TRAJECTORY_ID", &tuple.trajectory_id)
            .env("STS2_WORKER_SEED", &config.fingerprint.seed)
            .env(
                "STS2_WORKER_RELEASE_DIGEST",
                &config.fingerprint.build_digest,
            )
            .env("STS2_WORKER_STATE_DIGEST", &config.fingerprint.state_digest)
            .env(
                "STS2_WORKER_CONFIG_DIGEST",
                &config.fingerprint.config_digest,
            )
            .env(
                "STS2_WORKER_PROVIDER_DIGEST",
                &config.fingerprint.provider_digest,
            )
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit());
        let mut child = command
            .spawn()
            .map_err(|_| String::from("approved worker runtime could not be started"))?;
        loop {
            if task.cancellation().is_cancelled() {
                terminate_child(&mut child)?;
                return Err(String::from("worker runtime was cancelled by control"));
            }
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(_)) => return Err(String::from("worker runtime exited unsuccessfully")),
                Ok(None) => thread::sleep(POLL_INTERVAL),
                Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(String::from("worker runtime status could not be observed"));
                }
            }
        }
    }

    fn terminate_child(child: &mut std::process::Child) -> Result<(), String> {
        let _ = child.kill();
        let deadline = Instant::now()
            .checked_add(CHILD_REAP_TIMEOUT)
            .ok_or_else(|| String::from("worker runtime reap deadline overflow"))?;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => return Ok(()),
                Ok(None) if Instant::now() < deadline => thread::sleep(POLL_INTERVAL),
                Ok(None) => return Err(String::from("worker runtime did not stop")),
                Err(_) => return Err(String::from("worker runtime could not be reaped")),
            }
        }
    }
