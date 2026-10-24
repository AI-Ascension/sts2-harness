// SPDX-License-Identifier: MIT

use sts2_harness::PortError;

#[derive(Debug, Eq, PartialEq)]
pub(super) enum MapCollectionError {
    Failed(String),
    OwnerSnapshotInvalid(String),
}

impl MapCollectionError {
    pub(super) fn failed(message: impl Into<String>) -> Self {
        Self::Failed(message.into())
    }

    pub(super) fn owner_snapshot_invalid(message: impl Into<String>) -> Self {
        Self::OwnerSnapshotInvalid(message.into())
    }

    #[cfg(test)]
    pub(super) fn into_message(self) -> String {
        match self {
            Self::Failed(message) | Self::OwnerSnapshotInvalid(message) => message,
        }
    }

    pub(super) fn into_port_error(self) -> PortError {
        match self {
            Self::Failed(message) => PortError::new("map_snapshot_failed", message, false),
            Self::OwnerSnapshotInvalid(message) => {
                PortError::new("map_snapshot_invalid", message, false)
            }
        }
    }
}
