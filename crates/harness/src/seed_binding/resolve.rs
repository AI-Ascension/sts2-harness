// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};

use super::{SEED_BINDING_VERSION, SeedBindingError, SeedMode, SetupRequest, canonicalize_seed};

/// A source of generated seed material. Injecting it makes the draw count
/// observable, so a caller can prove exactly one draw per logical run.
pub trait SeedSource {
    /// Draw one canonical seed value. Called at most once per logical run.
    ///
    /// # Errors
    ///
    /// Returns [`SeedBindingError::RandomnessUnavailable`] when no entropy is
    /// available.
    fn draw(&mut self) -> Result<String, SeedBindingError>;
}

/// The host's operating-system randomness, encoded as lowercase hex.
#[derive(Clone, Copy, Debug, Default)]
pub struct OsSeedSource;

impl SeedSource for OsSeedSource {
    fn draw(&mut self) -> Result<String, SeedBindingError> {
        let mut bytes = [0_u8; 16];
        getrandom::fill(&mut bytes).map_err(|_| SeedBindingError::RandomnessUnavailable)?;
        Ok(crate::hex_bytes(bytes))
    }
}

/// Durable storage for one resolved seed per operation.
pub trait SeedStore {
    /// Load the persisted record for `operation_id`, if any.
    ///
    /// # Errors
    ///
    /// Returns [`SeedBindingError::StoreUnavailable`] when the store cannot be
    /// read.
    fn load(&self, operation_id: &str) -> Result<Option<SeedRecord>, SeedBindingError>;

    /// Persist the resolved record before any setup mutation.
    ///
    /// # Errors
    ///
    /// Returns [`SeedBindingError::PersistenceFailed`] when the record cannot be
    /// durably written.
    fn persist(&mut self, record: &SeedRecord) -> Result<(), SeedBindingError>;
}

/// The persisted binding of an operation's effective seed.
///
/// `requested_seed` is `None` for a generate-once run; `effective_seed` is the
/// canonical seed actually bound. The two stay distinct so a repeated explicit
/// seed is never itself treated as a reproducibility claim.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct SeedRecord {
    /// Persisted record version.
    pub version: String,
    /// The logical operation identity.
    pub operation_id: String,
    /// The bound game instance identity.
    pub instance_id: String,
    /// The bound profile baseline digest.
    pub baseline_digest: String,
    /// The bound lease identity.
    pub lease_id: String,
    /// The bound setup context identity.
    pub setup: String,
    /// The caller-supplied seed, if the mode was explicit.
    pub requested_seed: Option<String>,
    /// The canonical effective seed.
    pub effective_seed: String,
    /// The length-prefixed SHA-256 binding digest over every other field.
    pub binding_digest: String,
}

impl SeedRecord {
    /// Return the binding digest over the record's identity, mode and seeds.
    ///
    /// The input is length-prefixed so no two field partitions can collide, and
    /// the mode is tagged so an explicit empty seed cannot masquerade as a
    /// generated one.
    #[must_use]
    pub fn compute_binding_digest(&self) -> String {
        let mode = if self.requested_seed.is_some() {
            "explicit"
        } else {
            "generated"
        };
        let requested = self.requested_seed.as_deref().unwrap_or("");
        let mut framed = String::new();
        for part in [
            self.version.as_str(),
            self.operation_id.as_str(),
            self.instance_id.as_str(),
            self.baseline_digest.as_str(),
            self.lease_id.as_str(),
            self.setup.as_str(),
            mode,
            requested,
            self.effective_seed.as_str(),
        ] {
            framed.push_str(&part.len().to_string());
            framed.push(':');
            framed.push_str(part);
            framed.push(';');
        }
        crate::sha256_hex(framed.as_bytes())
    }
}

/// The outcome of resolving one operation's seed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedSeed {
    /// The persisted binding, whether newly written or restored.
    pub record: SeedRecord,
    /// True when an existing record was reused and no draw happened.
    pub reused: bool,
}

/// Resolve, persist and bind one operation's effective seed.
///
/// A generate-once request draws exactly once: duplicate requests, lost
/// responses and restarts that find an existing record reuse it without drawing
/// again. The record is persisted before this returns, so a caller mutates setup
/// only after a successful resolution, and a persistence failure fails closed.
///
/// # Errors
///
/// Returns the setup mismatch, canonicalization failure, or persistence failure.
pub fn resolve_seed<S, R>(
    request: &SetupRequest,
    store: &mut S,
    source: &mut R,
) -> Result<ResolvedSeed, SeedBindingError>
where
    S: SeedStore,
    R: SeedSource,
{
    request.validate()?;
    if let Some(record) = store.load(&request.operation_id)? {
        return Ok(ResolvedSeed {
            record: bind_existing(request, record)?,
            reused: true,
        });
    }
    let (requested_seed, effective_seed) = match &request.mode {
        SeedMode::Explicit(seed) => {
            let canonical = canonicalize_seed(seed)?;
            (Some(canonical.clone()), canonical)
        }
        SeedMode::GenerateOnce => (None, canonicalize_seed(&source.draw()?)?),
    };
    let record = SeedRecord {
        version: String::from(SEED_BINDING_VERSION),
        operation_id: request.operation_id.clone(),
        instance_id: request.instance_id.clone(),
        baseline_digest: request.baseline_digest.clone(),
        lease_id: request.lease_id.clone(),
        setup: request.setup.clone(),
        requested_seed,
        effective_seed,
        binding_digest: String::new(),
    };
    let binding_digest = record.compute_binding_digest();
    let record = SeedRecord {
        binding_digest,
        ..record
    };
    store
        .persist(&record)
        .map_err(|_| SeedBindingError::PersistenceFailed)?;
    Ok(ResolvedSeed {
        record,
        reused: false,
    })
}

fn bind_existing(
    request: &SetupRequest,
    record: SeedRecord,
) -> Result<SeedRecord, SeedBindingError> {
    if record.version != SEED_BINDING_VERSION
        || record.operation_id != request.operation_id
        || record.instance_id != request.instance_id
        || record.baseline_digest != request.baseline_digest
        || record.lease_id != request.lease_id
        || record.setup != request.setup
        || record.binding_digest != record.compute_binding_digest()
    {
        return Err(SeedBindingError::ConfigurationConflict);
    }
    match &request.mode {
        SeedMode::Explicit(seed) => {
            let canonical = canonicalize_seed(seed)?;
            if record.requested_seed.as_deref() != Some(canonical.as_str()) {
                return Err(SeedBindingError::ConfigurationConflict);
            }
        }
        SeedMode::GenerateOnce => {
            if record.requested_seed.is_some() {
                return Err(SeedBindingError::ConfigurationConflict);
            }
        }
    }
    Ok(record)
}

/// The bounded start request sent for a resolved seed. No other field is added.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct SentStart {
    /// The operation identity carried unchanged from the record.
    pub operation_id: String,
    /// The persisted effective seed carried unchanged from the record.
    pub effective_seed: String,
    /// The binding digest carried unchanged from the record.
    pub binding_digest: String,
}

/// Sends the resolved seed into the existing seeded-run start path.
pub trait StartTransport {
    /// Send one start request.
    ///
    /// # Errors
    ///
    /// Returns an error when the transport cannot accept the request.
    fn send_start(&mut self, start: &SentStart) -> Result<(), SeedBindingError>;
}

/// A transport that records what it was asked to send.
#[derive(Clone, Debug, Default)]
pub struct RecordingTransport {
    starts: Vec<SentStart>,
}

impl RecordingTransport {
    /// Return the recorded start requests in send order.
    #[must_use]
    pub fn starts(&self) -> &[SentStart] {
        &self.starts
    }
}

impl StartTransport for RecordingTransport {
    fn send_start(&mut self, start: &SentStart) -> Result<(), SeedBindingError> {
        self.starts.push(start.clone());
        Ok(())
    }
}

/// Send the persisted effective seed and existing operation identity unchanged.
///
/// # Errors
///
/// Returns an error when the transport cannot accept the request.
pub fn dispatch_start<T: StartTransport>(
    record: &SeedRecord,
    transport: &mut T,
) -> Result<SentStart, SeedBindingError> {
    let start = SentStart {
        operation_id: record.operation_id.clone(),
        effective_seed: record.effective_seed.clone(),
        binding_digest: record.binding_digest.clone(),
    };
    transport.send_start(&start)?;
    Ok(start)
}
