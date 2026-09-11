// SPDX-License-Identifier: MIT

impl RuntimeV3Port {
    pub(super) fn call_expert_tool(
        &mut self,
        name: &str,
        arguments: Value,
    ) -> Result<(u64, Value), String> {
        self.call_expert_tool_classified(name, arguments)
            .map_err(|error| error.message().to_owned())
    }
}
