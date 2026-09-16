// SPDX-License-Identifier: MIT

impl NativeCapabilities {
    #[must_use]
    pub fn fixture() -> Self {
        let methods = [
            "initialize",
            "thread/start",
            "thread/read",
            "turn/start",
            "turn/interrupt",
            "thread/fork",
            "thread/compact/start",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        let mut capabilities = Self {
            schema: SESSION_CAPABILITIES_SCHEMA.to_owned(),
            profile_id: "codex-app-server-fixture-v1".to_owned(),
            profile_sha256: digest(b"codex-app-server-fixture-v1"),
            native_version: "fixture-peer-1".to_owned(),
            native_binary_sha256: digest(b"compiled-fake-native-peer"),
            native_schema_sha256: digest(NATIVE_FRAME_SCHEMA.as_bytes()),
            evidence: CapabilityEvidence::CompiledPeer,
            transport: "owned_stdio".to_owned(),
            enabled_methods: methods,
            hardening: CapabilityHardening {
                tools_enabled: false,
                ambient_history: false,
                encrypted_state: false,
                configuration_verified: true,
                transform_handling: TransformHandling::DetectAndFence,
            },
            effective_limits: EffectiveSessionLimits {
                policy_schema: SESSION_POLICY_SCHEMA.to_owned(),
                max_session_items: MAX_SESSION_ITEMS,
                max_dependencies: MAX_DEPENDENCIES,
                max_events: MAX_EVENTS,
                max_operations: MAX_OPERATIONS,
                max_prepared: MAX_PREPARED,
                max_candidates: MAX_CANDIDATES,
                max_maintenance_jobs: MAX_MAINTENANCE_JOBS,
                max_completed_turns: MAX_COMPLETED_TURNS,
                max_history_ttl_seconds: MAX_HISTORY_TTL_SECONDS,
                max_frame_bytes: MAX_FRAME_BYTES,
                max_history_bytes: MAX_HISTORY_BYTES,
                max_prepared_bytes: MAX_PREPARED_BYTES,
                max_suffix_bytes: MAX_SUFFIX_BYTES,
                max_output_schema_bytes: MAX_OUTPUT_SCHEMA_BYTES,
                max_method_bytes: MAX_METHOD_BYTES,
                max_json_depth: MAX_JSON_DEPTH,
            },
            binding: SessionCapabilityBinding {
                owner: SESSION_CAPABILITIES_OWNER.to_owned(),
                owner_revision: SESSION_CAPABILITIES_REVISION.to_owned(),
                policy_schema_sha256: session_policy_schema_sha256(),
                model_revision: "fixture-peer-1".to_owned(),
                adapter_revision: "codex-app-server-fixture-v1".to_owned(),
                adapter_revision_sha256: digest(b"codex-app-server-fixture-v1"),
                descriptor_sha256: String::new(),
            },
            strict_executable: false,
            experimental_api: false,
            unknown_methods: "deny".to_owned(),
            raw_rpc: false,
        };
        capabilities.binding.descriptor_sha256 = capabilities.descriptor_digest();
        capabilities
    }

    /// Builds the narrow capability descriptor for the harness-owned Exo lifecycle v2 adapter.
    ///
    /// This is deliberately a source-derived profile rather than a claim about an upstream
    /// Codex peer: the adapter launches one owned process per turn and maps only its verified
    /// operations.  It therefore cannot advertise read, fork, compaction, reuse, or raw RPC.
    /// `executor_sha256`, `wire_schema_sha256`, and `profile_sha256` must come from the inspected
    /// launch configuration; callers cannot use a policy declaration as their source.
    pub fn reviewed_exo_lifecycle(
        profile_id: &str,
        profile_sha256: String,
        executor_sha256: String,
        wire_schema_sha256: String,
    ) -> Result<Self, SessionError> {
        if !valid_id(profile_id)
            || !valid_digest(&profile_sha256)
            || !valid_digest(&executor_sha256)
            || !valid_digest(&wire_schema_sha256)
        {
            return Err(SessionError::InvalidCapabilities);
        }
        let mut capabilities = Self::fixture();
        capabilities.profile_id = profile_id.to_owned();
        capabilities.profile_sha256 = profile_sha256.clone();
        capabilities.native_version = profile_id.to_owned();
        capabilities.native_binary_sha256 = executor_sha256;
        capabilities.native_schema_sha256 = wire_schema_sha256;
        capabilities.evidence = CapabilityEvidence::SchemaOnly;
        capabilities.enabled_methods = vec![
            String::from("initialize"),
            String::from("turn/start"),
            String::from("turn/interrupt"),
        ];
        capabilities.binding.model_revision = profile_id.to_owned();
        capabilities.binding.adapter_revision = profile_id.to_owned();
        capabilities.binding.adapter_revision_sha256 = profile_sha256;
        capabilities.binding.descriptor_sha256 = capabilities.descriptor_digest();
        capabilities.validate()?;
        Ok(capabilities)
    }

}
