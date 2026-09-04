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
        let mut first_error = None;
        if let Err(error) = port.release_lease() {
            first_error = Some(error);
        }
        if let Err(error) = port.close_mcp()
            && first_error.is_none()
        {
            first_error = Some(error);
        }
        if let Err(error) = port.close_gateway()
            && first_error.is_none()
        {
            first_error = Some(error);
        }
        first_error.map_or(Ok(()), Err)
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
