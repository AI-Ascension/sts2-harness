// SPDX-License-Identifier: MIT

impl RecoveryContext {
    pub(super) fn child_environment(&self) -> Result<Vec<(String, String)>, String> {
        let context = self
            .original_context
            .as_object()
            .ok_or_else(|| String::from("recovery original context is not an object"))?;
        let string_field = |field: &str| {
            context
                .get(field)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| format!("recovery original context {field} is invalid"))
        };
        let number_field = |field: &str| {
            context
                .get(field)
                .and_then(Value::as_u64)
                .map(|value| value.to_string())
                .ok_or_else(|| format!("recovery original context {field} is invalid"))
        };
        let current_fence = serde_json::to_string(&self.current_fence)
            .map_err(|_| String::from("recovery current fence is not encodable"))?;
        Ok(vec![
        (
            String::from("STS2_RECOVERY_DEPLOYMENT_ID"),
            string_field("deployment_id")?,
        ),
        (
            String::from("STS2_RECOVERY_INSTANCE_ID"),
            string_field("instance_id")?,
        ),
        (
            String::from("STS2_RECOVERY_INSTANCE_INCAR"),
            string_field("instance_incarnation")?,
        ),
        (
            String::from("STS2_RECOVERY_BOOT_ID"),
            string_field("boot_id")?,
        ),
        (
            String::from("STS2_RECOVERY_AUTHORITY_GENERATION"),
            number_field("authority_generation")?,
        ),
        (
            String::from("STS2_RECOVERY_LEASE_ID"),
            string_field("lease_id")?,
        ),
        (
            String::from("STS2_RECOVERY_LEASE_EPOCH"),
            number_field("lease_epoch")?,
        ),
        (
            String::from("STS2_RECOVERY_CURRENT_FENCE_JSON"),
            current_fence,
        ),
        ])
    }
}
