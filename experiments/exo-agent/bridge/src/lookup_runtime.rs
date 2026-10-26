// SPDX-License-Identifier: MIT
use crate::{
    lookup_bootstrap, lookup_manifest,
    lookup_wire::{self as wire, Frame, Payload},
};
use async_trait::async_trait;
use executor::{AgentConfig, ConversationConfig, ToolRuntime};
use exoharness::{AgentHandle, ConversationHandle, ToolRequest, ToolResult, TurnHandle};
use serde::Deserialize;
use serde_json::Value;
use tokio::{
    io::{BufReader, Stdin, Stdout},
    sync::Mutex,
};

pub struct LookupRuntime {
    request_id: String,
    turn_id: String,
    profile: LookupProfile,
    channel: Mutex<Channel>,
}

/// The additive provider surface one executor invocation was explicitly selected under.
///
/// Selection is the caller's: the relay passes the matching executor argument, and a tool belonging
/// to another surface is denied rather than forwarded. History is additive over bootstrap, so
/// selecting it widens the surface instead of trading the shipped capability away.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LookupProfile {
    Terminal,
    Bootstrap,
    History,
    ContentManifest,
}

impl LookupProfile {
    fn admits_bootstrap(self) -> bool {
        matches!(
            self,
            Self::Bootstrap | Self::History | Self::ContentManifest
        )
    }

    fn admits_history(self) -> bool {
        matches!(self, Self::History | Self::ContentManifest)
    }
}

struct Channel {
    input: BufReader<Stdin>,
    output: Stdout,
    tools: u64,
    failed: bool,
}

impl LookupRuntime {
    pub fn new(
        request_id: String,
        turn_id: String,
        input: BufReader<Stdin>,
        profile: LookupProfile,
    ) -> Self {
        Self {
            request_id,
            turn_id,
            profile,
            channel: Mutex::new(Channel {
                input,
                output: tokio::io::stdout(),
                tools: 0,
                failed: false,
            }),
        }
    }
    pub async fn count(&self) -> Result<u64, &'static str> {
        let channel = self.channel.lock().await;
        if channel.failed {
            return Err("exo_lookup_runtime_failed");
        }
        Ok(channel.tools)
    }
    pub async fn decision(&self, action_id: String) -> Result<(), &'static str> {
        let mut channel = self.channel.lock().await;
        if channel.failed {
            return Err("exo_lookup_runtime_failed");
        }
        let frame = Frame {
            wire_version: wire::VERSION.into(),
            request_id: self.request_id.clone(),
            turn_id: self.turn_id.clone(),
            sequence: channel.tools + 1,
            payload: Payload::Decision { action_id },
        };
        channel.failed = true; // No second terminal or tool may follow publication.
        wire::write_frame(&mut channel.output, &frame).await
    }
    async fn relay(&self, request: &ToolRequest) -> Result<Value, &'static str> {
        let mut channel = self.channel.lock().await;
        if channel.failed || channel.tools >= 32 {
            channel.failed = true;
            return Err("exo_lookup_tool_bound");
        }
        channel.failed = true; // Remains failed on every validation, I/O or cancellation exit.
        let payload = tool_payload_with_profile(request, self.profile)?;
        let version = wire::version_for_payload(&payload);
        channel.tools += 1;
        let sequence = channel.tools;
        let frame = Frame {
            wire_version: version.into(),
            request_id: self.request_id.clone(),
            turn_id: self.turn_id.clone(),
            sequence,
            payload,
        };
        wire::write_frame(&mut channel.output, &frame).await?;
        let bytes = wire::read_line(&mut channel.input).await?;
        let value =
            wire::feedback_for_version(&bytes, &self.request_id, &self.turn_id, sequence, version)?;
        channel.failed = false;
        Ok(value)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadArguments {
    record_ordinal: usize,
    offset: usize,
}

#[cfg(test)]
fn tool_payload(request: &ToolRequest) -> Result<Payload, &'static str> {
    tool_payload_with_profile(request, LookupProfile::Terminal)
}

fn tool_payload_with_profile(
    request: &ToolRequest,
    profile: LookupProfile,
) -> Result<Payload, &'static str> {
    if request.namespace.is_some()
        || serde_json::to_vec(&request.arguments)
            .map_err(|_| "exo_lookup_arguments")?
            .len()
            > wire::TOOL_BYTES
    {
        return Err("exo_lookup_arguments");
    }
    let arguments = Value::Object(request.arguments.clone());
    match request.function_name.as_str() {
        "sts2_lookup_content_manifest" | "sts2_lookup_manifest_read" => {
            if profile != LookupProfile::ContentManifest {
                return Err("exo_lookup_tool_denied");
            }
            lookup_manifest::payload(
                arguments,
                request.function_name == "sts2_lookup_manifest_read",
            )
        }
        "sts2_lookup_query" => Ok(Payload::Query { arguments }),
        "sts2_lookup_bootstrap" => {
            if !profile.admits_bootstrap() {
                return Err("exo_lookup_tool_denied");
            }
            lookup_bootstrap::payload(arguments)
        }
        // History is answered by the harness-owned store port. The executor only carries the closed
        // question across, so the question is forwarded for that boundary to validate against its
        // own vocabulary rather than re-implemented here; nothing in it can name an owner, epoch,
        // path, bucket, artifact or record ordinal.
        "sts2_lookup_history" => {
            if !profile.admits_history() {
                return Err("exo_lookup_tool_denied");
            }
            Ok(Payload::History { arguments })
        }
        "sts2_lookup_read" => {
            let read: ReadArguments =
                serde_json::from_value(arguments).map_err(|_| "exo_lookup_arguments")?;
            if read.record_ordinal > 255 || read.offset > 65536 {
                return Err("exo_lookup_arguments");
            }
            Ok(Payload::ReadRetained {
                record_ordinal: read.record_ordinal,
                offset: read.offset,
            })
        }
        _ => Err("exo_lookup_tool_denied"),
    }
}

#[async_trait]
impl ToolRuntime for LookupRuntime {
    async fn execute(
        &self,
        _agent: &dyn AgentHandle,
        _conversation: &dyn ConversationHandle,
        turn: Option<&dyn TurnHandle>,
        _agent_config: &AgentConfig,
        _config: &ConversationConfig,
        request: &ToolRequest,
    ) -> exoharness::Result<ToolResult> {
        if turn.is_none() {
            self.channel.lock().await.failed = true;
            return Err(anyhow::anyhow!("exo_lookup_turn_missing"));
        }
        self.relay(request)
            .await
            .map_err(|code| anyhow::anyhow!(code))
    }
}

#[cfg(test)]
mod tests;
