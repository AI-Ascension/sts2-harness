// SPDX-License-Identifier: MIT

//! Gateway transport for the typed process-lifecycle port.
//!
//! This is the only place that knows the lifecycle wire shape. It builds one
//! fixed path per route from the configured instance, sends the gateway
//! credential and identity headers, and maps the closed response schema onto
//! the typed port. It never reads a command, path, or URL from its caller, and
//! it never resolves an executable.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};
use sts2_harness::management::{
    AuthContext, LifecycleAction, LifecycleCommand, LifecycleOperationView, LifecycleTarget,
    ManagementError, ProcessLifecycleCapability, ProcessLifecyclePort,
};

#[path = "gateway_lifecycle_parse.rs"]
mod parse;

use super::RuntimeConfig;
use super::http::GatewayClient;

use parse::{gateway_error, parse_capability, parse_operation};

/// The gateway transport for the lifecycle surface.
pub(super) struct GatewayLifecyclePort {
    client: GatewayClient,
    /// Configured instance identity, used only to build the fixed route path.
    instance_id: String,
}

impl GatewayLifecyclePort {
    pub(super) fn new(config: &RuntimeConfig) -> Result<Self, ManagementError> {
        Ok(Self {
            client: GatewayClient::new(config)
                .map_err(|error| ManagementError::unavailable("gateway_client", error))?,
            instance_id: config.instance_id.clone(),
        })
    }

    /// Identity headers for one lifecycle request.
    ///
    /// Caller, session, lease, and lease epoch are the gateway's own
    /// authenticated inputs; the body carries none of them, so a caller cannot
    /// assert an identity the request did not prove.
    fn headers(&self, correlation: &str) -> BTreeMap<String, String> {
        BTreeMap::from([
            (String::from("x-sts2-instance-id"), self.instance_id.clone()),
            (
                String::from("x-sts2-correlation-id"),
                correlation.to_owned(),
            ),
        ])
    }

    fn operations_path(&self, instance: &str) -> String {
        format!("/v1/instances/{instance}/process-lifecycle/operations")
    }

    fn capability_path(&self, instance: &str) -> String {
        format!("/v1/instances/{instance}/process-lifecycle")
    }

    fn lookup_path(&self, instance: &str, operation_id: u64) -> String {
        format!("/v1/instances/{instance}/process-lifecycle/operations/{operation_id}")
    }

    /// Refuses a target that is not the configured instance.
    ///
    /// The harness serves one gateway deployment, so a command for a different
    /// instance must not be translated into a request against this one. There is
    /// no fallback instance: the mismatch is a refusal.
    fn require_configured(&self, target: &LifecycleTarget) -> Result<(), ManagementError> {
        if target.instance_id != self.instance_id {
            return Err(ManagementError::conflict(
                "lifecycle_instance_mismatch",
                "lifecycle target is not the instance this harness serves",
            ));
        }
        Ok(())
    }
}

impl ProcessLifecyclePort for GatewayLifecyclePort {
    fn capability(
        &self,
        _actor: &AuthContext,
        target: &LifecycleTarget,
    ) -> Result<ProcessLifecycleCapability, ManagementError> {
        self.require_configured(target)?;
        let value = self
            .client
            .request(
                "GET",
                &self.capability_path(&target.instance_id),
                &Value::Null,
                self.headers("process-lifecycle-capability"),
            )
            .map_err(gateway_error)?;
        parse_capability(&value, &target.instance_id)
    }

    fn submit(
        &self,
        _actor: &AuthContext,
        target: &LifecycleTarget,
        command: &LifecycleCommand,
        action: &LifecycleAction,
    ) -> Result<LifecycleOperationView, ManagementError> {
        self.require_configured(target)?;
        let body = submission_body(command, action)?;
        let value = self
            .client
            .request(
                "POST",
                &self.operations_path(&target.instance_id),
                &body,
                self.headers(&command.command_id),
            )
            .map_err(gateway_error)?;
        parse_operation(&value, &target.instance_id)
    }

    fn lookup(
        &self,
        _actor: &AuthContext,
        target: &LifecycleTarget,
        operation_id: u64,
    ) -> Result<LifecycleOperationView, ManagementError> {
        self.require_configured(target)?;
        let value = self
            .client
            .request(
                "GET",
                &self.lookup_path(&target.instance_id, operation_id),
                &Value::Null,
                self.headers(&format!("process-lifecycle-lookup-{operation_id}")),
            )
            .map_err(gateway_error)?;
        parse_operation(&value, &target.instance_id)
    }
}

/// Builds the closed submission body.
///
/// Exactly the fields the gateway's schema names are emitted, and no others.
/// A field for another action is never included, so one body can never be two
/// actions at once.
fn submission_body(
    command: &LifecycleCommand,
    action: &LifecycleAction,
) -> Result<Value, ManagementError> {
    let mut action_object = Map::new();
    match action {
        LifecycleAction::LaunchNew { profile_id } => {
            action_object.insert(String::from("kind"), json!("launch_new"));
            action_object.insert(String::from("profile_id"), json!(profile_id.value()));
        }
        LifecycleAction::Restart { profile_id } => {
            action_object.insert(String::from("kind"), json!("restart"));
            action_object.insert(String::from("profile_id"), json!(profile_id.value()));
        }
        LifecycleAction::Stop { mode } => {
            action_object.insert(String::from("kind"), json!("stop"));
            action_object.insert(String::from("mode"), json!(mode.as_str()));
        }
        LifecycleAction::AttachExisting { identity } => {
            action_object.insert(String::from("kind"), json!("attach_existing"));
            action_object.insert(String::from("process"), json!(identity.process));
            action_object.insert(String::from("pid"), json!(identity.pid));
            action_object.insert(String::from("birth_id"), json!(identity.birth_id));
            action_object.insert(
                String::from("executable"),
                json!({
                    "install_id": identity.install_id,
                    "executable_id": identity.executable_id,
                    "image_id": identity.image_id,
                }),
            );
            action_object.insert(
                String::from("user_data"),
                json!({"namespace_id": identity.namespace_id}),
            );
        }
    }
    if command.operation_id == 0 || command.authority_epoch == 0 {
        return Err(ManagementError::invalid(
            "lifecycle_operation_identity_invalid",
            "operation id and authority epoch must both be non-zero",
        ));
    }
    Ok(json!({
        "operation_id": command.operation_id,
        "authority_epoch": command.authority_epoch,
        "action": Value::Object(action_object),
    }))
}
