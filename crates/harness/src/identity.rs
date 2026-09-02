// SPDX-License-Identifier: MIT

use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IdentityError {
    Empty,
    InvalidDigest,
    TooLong,
    ZeroValue,
    Exhausted,
}

impl fmt::Display for IdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Empty => "value must not be empty",
            Self::InvalidDigest => "digest must be 64 lowercase hexadecimal characters",
            Self::TooLong => "value exceeds its maximum length",
            Self::ZeroValue => "identifier value must be nonzero",
            Self::Exhausted => "identifier allocator is exhausted",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for IdentityError {}

macro_rules! numeric_id {
    ($name:ident, $label:literal) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(u64);

        impl $name {
            #[must_use]
            pub const fn new(value: u64) -> Option<Self> {
                if value == 0 { None } else { Some(Self(value)) }
            }

            #[must_use]
            pub const fn get(self) -> u64 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}-{}", $label, self.0)
            }
        }

        impl NumericId for $name {
            fn from_value(value: u64) -> Option<Self> {
                Self::new(value)
            }
        }
    };
}

numeric_id!(RunId, "run");
numeric_id!(EpisodeId, "episode");
numeric_id!(TrajectoryId, "trajectory");
numeric_id!(InstanceId, "instance");
numeric_id!(GatewaySessionId, "gateway-session");
numeric_id!(RequestId, "request");
numeric_id!(ActionId, "action");
numeric_id!(TraceId, "trace");
numeric_id!(ModelExecutionId, "model-execution");
numeric_id!(ArtifactId, "artifact");
numeric_id!(RecordId, "record");

pub(crate) trait NumericId: Sized {
    fn from_value(value: u64) -> Option<Self>;
}

#[derive(Debug)]
pub(crate) struct IdAllocator {
    next: u64,
}

impl IdAllocator {
    pub(crate) const fn new(start: u64) -> Self {
        Self {
            next: if start == 0 { 1 } else { start },
        }
    }

    pub(crate) fn allocate<T: NumericId>(&mut self) -> Result<T, IdentityError> {
        if self.next == 0 {
            return Err(IdentityError::Exhausted);
        }
        let value = self.next;
        self.next = if value == u64::MAX { 0 } else { value + 1 };
        T::from_value(value).ok_or(IdentityError::Exhausted)
    }
}

const MAX_KEY_BYTES: usize = 128;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    pub fn new(value: impl Into<String>) -> Result<Self, IdentityError> {
        let value = value.into();
        if value.is_empty() {
            return Err(IdentityError::Empty);
        }
        if value.len() > MAX_KEY_BYTES {
            return Err(IdentityError::TooLong);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for IdempotencyKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Digest(String);

impl Digest {
    pub fn new(value: impl Into<String>) -> Result<Self, IdentityError> {
        let value = value.into();
        let valid = value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte.is_ascii_lowercase() && byte <= b'f');
        if !valid {
            return Err(IdentityError::InvalidDigest);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SchemaVersion(u16);

impl SchemaVersion {
    #[must_use]
    pub const fn new(value: u16) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}
