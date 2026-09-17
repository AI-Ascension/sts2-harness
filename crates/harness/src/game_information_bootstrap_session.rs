// SPDX-License-Identifier: MIT
/// Typed bootstrap transcript retained beside query records. A replay consumes
/// this value directly and never calls the gateway or MCP again.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookupBootstrapRecord {
    pub schema: String,
    pub binding: LookupBinding,
    pub operation_id: String,
    pub request: Value,
    pub response: Option<Value>,
    pub error: Option<LookupError>,
}

impl LookupSession {
    pub fn bootstrap_records(&self) -> &[LookupBootstrapRecord] {
        &self.bootstrap_records
    }

    /// Install an owner-validated current observation snapshot.
    pub fn observe_snapshot(&mut self, snapshot: Value) {
        self.binding.snapshot = Some(snapshot);
        self.pages.clear();
    }

    /// Clears only the native live attestation after a new LBR observation.
    pub fn invalidate_live_snapshot(&mut self) {
        self.binding.snapshot = None;
        self.pages.clear();
    }

    /// Installs an authenticated bootstrap snapshot and retains its transcript.
    pub fn install_bootstrap(
        &mut self,
        operation_id: &str,
        request: Value,
        response: Value,
        snapshot: Value,
    ) -> Result<usize, LookupError> {
        if operation_id.is_empty()
            || operation_id.len() > 128
            || !operation_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
        {
            return Err(LookupError::Invalid);
        }
        let bytes = serde_json::to_vec(&response).map_err(|_| LookupError::Invalid)?;
        if bytes.len()
            > crate::game_information_binding::game_information_bootstrap::MAX_MESSAGE_BYTES
        {
            return Err(LookupError::Bounds);
        }
        self.binding.snapshot = Some(snapshot);
        self.pages.clear();
        let ordinal = self.bootstrap_records.len();
        self.bootstrap_records.push(LookupBootstrapRecord {
            schema: "ascension.game-information-bootstrap-record.v1".to_owned(),
            binding: self.binding.clone(),
            operation_id: operation_id.to_owned(),
            request,
            response: Some(response),
            error: None,
        });
        Ok(ordinal)
    }

    pub(crate) fn record_bootstrap_error(
        &mut self,
        operation_id: &str,
        request: Value,
        error: LookupError,
    ) {
        self.bootstrap_records.push(LookupBootstrapRecord {
            schema: "ascension.game-information-bootstrap-record.v1".to_owned(),
            binding: self.binding.clone(),
            operation_id: operation_id.to_owned(),
            request,
            response: None,
            error: Some(error),
        });
    }
}
