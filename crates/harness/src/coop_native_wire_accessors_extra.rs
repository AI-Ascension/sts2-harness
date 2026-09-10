// SPDX-License-Identifier: MIT

impl CoopNativeRecovery {
    #[must_use]
    pub const fn kind(&self) -> CoopNativeRecoveryKind {
        self.kind
    }

    #[must_use]
    pub const fn rejoin_epoch(&self) -> u64 {
        self.rejoin_epoch
    }
}
impl CoopNativePeerSnapshot {
    #[must_use]
    pub fn peer_token(&self) -> &CoopNativePeerId {
        &self.peer_token
    }

    #[must_use]
    pub fn authority_id(&self) -> &CoopNativeAuthorityId {
        &self.authority_id
    }

    #[must_use]
    pub const fn role(&self) -> CoopNativePeerRole {
        self.role
    }

    #[must_use]
    pub const fn connected(&self) -> bool {
        self.connected
    }

    #[must_use]
    pub const fn peer_generation(&self) -> u64 {
        self.peer_generation
    }

    #[must_use]
    pub fn state_digest(&self) -> &str {
        &self.state_digest
    }

    #[must_use]
    pub const fn rejoin_epoch(&self) -> u64 {
        self.rejoin_epoch
    }

    #[must_use]
    pub fn authority_epoch(&self) -> &CoopNativeAuthorityId {
        &self.authority_epoch
    }

    #[must_use]
    pub fn checkpoint_id(&self) -> Option<&CoopNativeCheckpointId> {
        self.checkpoint_id.as_ref()
    }

    #[must_use]
    pub const fn digest_known(&self) -> bool {
        self.digest_known
    }

    #[must_use]
    pub const fn is_loading(&self) -> bool {
        self.is_loading
    }

    #[must_use]
    pub const fn is_divergent(&self) -> bool {
        self.is_divergent
    }

    #[must_use]
    pub const fn checksum_status(&self) -> CoopNativeChecksumStatus {
        self.checksum_status
    }
}

impl CoopNativeEffectResponse {
    #[must_use]
    pub fn operation_id(&self) -> &CoopNativeOperationId {
        &self.operation_id
    }

    #[must_use]
    pub const fn status(&self) -> CoopNativeStatus {
        self.status
    }

    #[must_use]
    pub fn observation(&self) -> &CoopNativeObservation {
        &self.observation
    }

    #[must_use]
    pub fn effect(&self) -> Option<&CoopNativeEffect> {
        self.effect.as_ref()
    }

    #[must_use]
    pub fn receipt(&self) -> &CoopNativeReceipt {
        &self.receipt
    }
}

impl CoopNativeRecoveryResponse {
    #[must_use]
    pub fn operation_id(&self) -> &CoopNativeOperationId {
        &self.operation_id
    }

    #[must_use]
    pub const fn status(&self) -> Option<CoopNativeStatus> {
        self.status
    }

    #[must_use]
    pub fn observation(&self) -> Option<&CoopNativeObservation> {
        self.observation.as_ref()
    }

    #[must_use]
    pub fn recovery(&self) -> &CoopNativeRecovery {
        &self.recovery
    }

    #[must_use]
    pub fn receipt(&self) -> Option<&CoopNativeReceipt> {
        self.receipt.as_ref()
    }
}

impl CoopNativeEffect {
    #[must_use]
    pub fn effect_id(&self) -> &CoopNativeEffectId {
        &self.effect_id
    }

    #[must_use]
    pub fn operation_id(&self) -> &CoopNativeOperationId {
        &self.operation_id
    }

    #[must_use]
    pub const fn kind(&self) -> CoopNativeEffectKind {
        self.kind
    }

    #[must_use]
    pub const fn from_generation(&self) -> u64 {
        self.from_generation
    }

    #[must_use]
    pub const fn to_generation(&self) -> u64 {
        self.to_generation
    }

    #[must_use]
    pub fn state_digest(&self) -> &str {
        &self.state_digest
    }

    #[must_use]
    pub fn authority_epoch(&self) -> &CoopNativeAuthorityId {
        &self.authority_epoch
    }

    #[must_use]
    pub fn checkpoint_id(&self) -> &CoopNativeCheckpointId {
        &self.checkpoint_id
    }

    #[must_use]
    pub fn authority_id(&self) -> &CoopNativeAuthorityId {
        &self.authority_id
    }

    #[must_use]
    pub fn native_checksum(&self) -> Option<&str> {
        self.native_checksum.as_deref()
    }
}

impl CoopNativeReceipt {
    #[must_use]
    pub fn operation_id(&self) -> &CoopNativeOperationId {
        &self.operation_id
    }

    #[must_use]
    pub const fn status(&self) -> CoopNativeStatus {
        self.status
    }

    #[must_use]
    pub const fn before_host_generation(&self) -> u64 {
        self.before_host_generation
    }

    #[must_use]
    pub const fn after_host_generation(&self) -> Option<u64> {
        self.after_host_generation
    }

    #[must_use]
    pub fn authority_id(&self) -> &CoopNativeAuthorityId {
        &self.authority_id
    }

    #[must_use]
    pub fn authority_epoch(&self) -> &CoopNativeAuthorityId {
        &self.authority_epoch
    }

    #[must_use]
    pub fn checkpoint_id(&self) -> &CoopNativeCheckpointId {
        &self.checkpoint_id
    }

    #[must_use]
    pub fn state_digest(&self) -> &str {
        &self.state_digest
    }

    #[must_use]
    pub fn native_checksum(&self) -> Option<&str> {
        self.native_checksum.as_deref()
    }

    #[must_use]
    pub fn error_code(&self) -> Option<&str> {
        self.error_code.as_deref()
    }
}

impl CoopNativeCatalog {
    #[must_use]
    pub const fn host_generation(&self) -> u64 {
        self.host_generation
    }

    #[must_use]
    pub fn actor_peer(&self) -> &CoopNativePeerId {
        &self.actor_peer
    }

    #[must_use]
    pub fn actions(&self) -> &[CoopNativeAction] {
        &self.actions
    }

    #[must_use]
    pub fn votes(&self) -> &[CoopNativeVote] {
        &self.votes
    }
}

impl CoopNativeObservation {
    #[must_use]
    pub fn host_authority_epoch(&self) -> &CoopNativeAuthorityId {
        &self.host_authority_epoch
    }

    #[must_use]
    pub fn authority_id(&self) -> &CoopNativeAuthorityId {
        &self.authority_id
    }

    #[must_use]
    pub fn run_id(&self) -> &CoopNativeRunId {
        &self.run_id
    }

    #[must_use]
    pub fn host_sequence_kind(&self) -> &str {
        &self.host_sequence_kind
    }

    #[must_use]
    pub const fn host_generation(&self) -> u64 {
        self.host_generation
    }

    #[must_use]
    pub fn state_digest(&self) -> &str {
        &self.state_digest
    }

    #[must_use]
    pub fn checkpoint_id(&self) -> &CoopNativeCheckpointId {
        &self.checkpoint_id
    }

    #[must_use]
    pub fn checksum_algorithm(&self) -> &str {
        &self.checksum_algorithm
    }

    #[must_use]
    pub const fn checksum_status(&self) -> CoopNativeChecksumStatus {
        self.checksum_status
    }

    #[must_use]
    pub fn native_checksum(&self) -> Option<&str> {
        self.native_checksum.as_deref()
    }

    #[must_use]
    pub const fn host_digest_known(&self) -> bool {
        self.host_digest_known
    }

    #[must_use]
    pub const fn host_loading(&self) -> bool {
        self.host_loading
    }

    #[must_use]
    pub const fn host_divergent(&self) -> bool {
        self.host_divergent
    }

    #[must_use]
    pub fn peers(&self) -> &[CoopNativePeerSnapshot] {
        &self.peers
    }
}
