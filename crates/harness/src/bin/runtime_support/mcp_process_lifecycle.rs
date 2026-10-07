// SPDX-License-Identifier: MIT

impl McpProcess {
    fn cleanup_deadline_at(
        maximum: std::time::Duration,
        deadline: Option<std::time::Instant>,
        now: std::time::Instant,
    ) -> Option<std::time::Instant> {
        let maximum_deadline = now.checked_add(maximum)?;
        Some(deadline.map_or(maximum_deadline, |deadline| {
            maximum_deadline.min(deadline)
        }))
    }

    fn terminate(&mut self) -> Result<(), String> {
        self.closed = true;
        self.input.take();
        self.output.take();
        let runtime = self.runtime.as_ref().ok_or("MCP supervisor is closed")?;
        supervised(|| {
            runtime.block_on(async {
                // Signal the owned child even when the absolute profile deadline has expired;
                // only the bounded wait below is skipped in that case.
                self.child
                    .start_kill()
                    .map_err(|_| "MCP termination failed")?;
                let wait_deadline = Self::cleanup_deadline_at(
                    FORCE_REAP_TIMEOUT,
                    self.profile_deadline,
                    std::time::Instant::now(),
                )
                .ok_or("MCP reap deadline is unavailable")?;
                if wait_deadline <= std::time::Instant::now() {
                    return Err("MCP reap deadline expired after termination signal");
                }
                tokio::time::timeout_at(
                    tokio::time::Instant::from_std(wait_deadline),
                    self.child.wait(),
                )
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
            let wait_deadline = Self::cleanup_deadline_at(
                GRACEFUL_CLOSE_TIMEOUT,
                self.profile_deadline,
                std::time::Instant::now(),
            )
            .ok_or("MCP close deadline is unavailable")?;
            if wait_deadline <= std::time::Instant::now() {
                return Err("MCP close deadline expired before graceful shutdown");
            }
            runtime.block_on(async {
                let status = tokio::time::timeout_at(
                    tokio::time::Instant::from_std(wait_deadline),
                    self.child.wait(),
                )
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
