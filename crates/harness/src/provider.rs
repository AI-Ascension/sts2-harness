// SPDX-License-Identifier: MIT

use crate::error::{PortError, ProviderError};
use crate::identity::{IdempotencyKey, ModelExecutionId};
use crate::records::Correlation;

const MAX_PROMPT_BYTES: usize = 64 * 1024;
const MAX_OUTPUT_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Prompt(String);

impl Prompt {
    pub fn new(value: impl Into<String>) -> Result<Self, PortError> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_PROMPT_BYTES {
            return Err(PortError::new(
                "invalid_prompt",
                "prompt must be nonempty and bounded",
                false,
            ));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelOutput(String);

impl ModelOutput {
    pub fn new(value: impl Into<String>) -> Result<Self, PortError> {
        let value = value.into();
        if value.len() > MAX_OUTPUT_BYTES {
            return Err(PortError::new(
                "invalid_model_output",
                "model output exceeds its bound",
                false,
            ));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelRequest {
    execution_id: ModelExecutionId,
    correlation: Correlation,
    prompt: Prompt,
    idempotency_key: IdempotencyKey,
}

impl ModelRequest {
    #[must_use]
    pub const fn new(
        execution_id: ModelExecutionId,
        correlation: Correlation,
        prompt: Prompt,
        idempotency_key: IdempotencyKey,
    ) -> Self {
        Self {
            execution_id,
            correlation,
            prompt,
            idempotency_key,
        }
    }

    #[must_use]
    pub const fn execution_id(&self) -> ModelExecutionId {
        self.execution_id
    }

    #[must_use]
    pub const fn correlation(&self) -> &Correlation {
        &self.correlation
    }

    #[must_use]
    pub fn prompt(&self) -> &Prompt {
        &self.prompt
    }

    #[must_use]
    pub fn idempotency_key(&self) -> &IdempotencyKey {
        &self.idempotency_key
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelResponse {
    execution_id: ModelExecutionId,
    correlation: Correlation,
    output: ModelOutput,
}

impl ModelResponse {
    pub fn new(
        execution_id: ModelExecutionId,
        correlation: Correlation,
        output: ModelOutput,
    ) -> Result<Self, ProviderError> {
        if correlation.model_execution_id() != Some(execution_id) {
            return Err(ProviderError::new(
                "provider_correlation_mismatch",
                "response execution identity does not match correlation",
                false,
            ));
        }
        Ok(Self {
            execution_id,
            correlation,
            output,
        })
    }

    #[must_use]
    pub const fn execution_id(&self) -> ModelExecutionId {
        self.execution_id
    }

    #[must_use]
    pub const fn correlation(&self) -> &Correlation {
        &self.correlation
    }

    #[must_use]
    pub fn output(&self) -> &ModelOutput {
        &self.output
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetryPolicy {
    max_attempts: u8,
}

impl RetryPolicy {
    pub fn new(max_attempts: u8) -> Result<Self, PortError> {
        if max_attempts == 0 {
            return Err(PortError::new(
                "invalid_retry_policy",
                "max_attempts must be nonzero",
                false,
            ));
        }
        Ok(Self { max_attempts })
    }

    #[must_use]
    pub const fn max_attempts(self) -> u8 {
        self.max_attempts
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelResult {
    request: ModelRequest,
    response: ModelResponse,
    attempts: u8,
}

impl ModelResult {
    #[must_use]
    pub const fn request(&self) -> &ModelRequest {
        &self.request
    }

    #[must_use]
    pub const fn response(&self) -> &ModelResponse {
        &self.response
    }

    #[must_use]
    pub const fn attempts(&self) -> u8 {
        self.attempts
    }
}

pub(crate) fn model_result(
    request: ModelRequest,
    response: ModelResponse,
    attempts: u8,
) -> ModelResult {
    ModelResult {
        request,
        response,
        attempts,
    }
}

pub trait ProviderPort {
    fn execute(&mut self, request: &ModelRequest) -> Result<ModelResponse, ProviderError>;

    fn close(&mut self) -> Result<(), PortError>;
}
