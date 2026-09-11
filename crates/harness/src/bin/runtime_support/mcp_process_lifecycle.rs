// SPDX-License-Identifier: MIT

impl McpProcess {
    fn terminate(&mut self) -> Result<(), String> {
        self.closed = true;
        self.input.take();
        self.output.take();
        let runtime = self.runtime.as_ref().ok_or("MCP supervisor is closed")?;
        supervised(|| {
            runtime.block_on(async {
                self.child
                    .start_kill()
                    .map_err(|_| "MCP termination failed")?;
                tokio::time::timeout(FORCE_REAP_TIMEOUT, self.child.wait())
                    .await
                    .map_err(|_| "MCP reap timed out")?
                    .map_err(|_| "MCP reap failed")?;
                Ok(())
            })
        })
    }

    pub(super) fn close(&mut self) -> Result<(), String> {
        if self.closed {
            return if self.child.id().is_some() {
                self.terminate()
            } else {
                Ok(())
            };
        }
        self.input.take();
        self.output.take();
        let runtime = self.runtime.as_ref().ok_or("MCP supervisor is closed")?;
        let result = supervised(|| {
            runtime.block_on(async {
                let status = tokio::time::timeout(GRACEFUL_CLOSE_TIMEOUT, self.child.wait())
                    .await
                    .map_err(|_| "MCP shutdown timed out")?
                    .map_err(|_| "MCP process wait failed")?;
                if status.success() {
                    Ok(())
                } else {
                    Err("MCP process exited unsuccessfully")
                }
            })
        });
        if let Err(error) = result {
            let cleanup = self.terminate();
            return match cleanup {
                Ok(()) => Err(error),
                Err(cleanup) => Err(format!("{error}; {cleanup}")),
            };
        }
        self.closed = true;
        result
    }
}

impl Drop for McpProcess {
    fn drop(&mut self) {
        if self.child.id().is_some() {
            let _cleanup = self.terminate();
        }
        if let Some(runtime) = self.runtime.take() {
            let _cleanup = supervised(|| {
                drop(runtime);
                Ok(())
            });
        }
    }
}
