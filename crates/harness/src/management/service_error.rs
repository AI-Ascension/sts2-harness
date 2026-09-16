// SPDX-License-Identifier: MIT

use super::*;

impl ManagementError {
    pub fn new(class: ErrorClass, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            class,
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn invalid(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::InvalidInput, code, message)
    }

    pub fn capability(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::Capability, code, message)
    }

    pub fn conflict(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::Conflict, code, message)
    }

    pub fn forbidden(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::Forbidden, code, message)
    }

    pub fn unresolved(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::Unresolved, code, message)
    }

    pub fn unavailable(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::Unavailable, code, message)
    }

    pub fn budget(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::Budget, code, message)
    }

    pub fn store(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::Store, code, message)
    }

    pub fn replay(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::Replay, code, message)
    }

    pub fn authentication(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorClass::Authentication, code, message)
    }

    pub fn error_response(&self) -> ErrorResponse {
        ErrorResponse {
            schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
            error: ErrorBody {
                class: self.class.clone(),
                code: self.code.clone(),
                message: self.message.clone(),
            },
        }
    }
}
