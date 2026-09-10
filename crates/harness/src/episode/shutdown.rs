// SPDX-License-Identifier: MIT

/// Owner of gateway/MCP/episode cleanup work.
pub trait ShutdownPort {
    fn release_lease(&mut self) -> Result<(), ShutdownError>;
    fn close_mcp(&mut self) -> Result<(), ShutdownError>;
    fn close_gateway(&mut self) -> Result<(), ShutdownError>;
}

/// All cleanup failures observed after an episode runtime has been launched.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpisodeCleanupReport {
    failures: Vec<ShutdownError>,
}

impl EpisodeCleanupReport {
    #[must_use]
    pub fn failures(&self) -> &[ShutdownError] {
        &self.failures
    }

    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.failures.is_empty()
    }

    #[must_use]
    pub fn first_failure(&self) -> Option<ShutdownError> {
        self.failures.first().copied()
    }
}

impl std::fmt::Display for EpisodeCleanupReport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} cleanup operation(s) failed",
            self.failures.len()
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EpisodeShutdown;

impl EpisodeShutdown {
    /// Runs every owned cleanup operation and retains each failure in execution order.
    pub fn close_report<P: ShutdownPort>(&self, port: &mut P) -> EpisodeCleanupReport {
        let mut failures = Vec::new();
        if let Err(error) = port.release_lease() {
            failures.push(error);
        }
        if let Err(error) = port.close_mcp() {
            failures.push(error);
        }
        if let Err(error) = port.close_gateway() {
            failures.push(error);
        }
        EpisodeCleanupReport { failures }
    }

    /// Compatibility wrapper retaining the historical first-cleanup-error result.
    pub fn close<P: ShutdownPort>(&self, port: &mut P) -> Result<(), ShutdownError> {
        self.close_report(port).first_failure().map_or(Ok(()), Err)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShutdownError {
    ReleaseFailed,
    McpCloseFailed,
    GatewayCloseFailed,
}

impl std::fmt::Display for ShutdownError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::ReleaseFailed => "lease release failed",
            Self::McpCloseFailed => "MCP close failed",
            Self::GatewayCloseFailed => "gateway close failed",
        })
    }
}

impl std::error::Error for ShutdownError {}
