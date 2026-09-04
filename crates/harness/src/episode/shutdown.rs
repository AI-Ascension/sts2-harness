// SPDX-License-Identifier: MIT

/// Owner of gateway/MCP/episode cleanup work.
pub trait ShutdownPort {
    fn release_lease(&mut self) -> Result<(), ShutdownError>;
    fn close_mcp(&mut self) -> Result<(), ShutdownError>;
    fn close_gateway(&mut self) -> Result<(), ShutdownError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EpisodeShutdown;

impl EpisodeShutdown {
    pub fn close<P: ShutdownPort>(&self, port: &mut P) -> Result<(), ShutdownError> {
        port.release_lease()?;
        port.close_mcp()?;
        port.close_gateway()
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
