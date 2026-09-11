// SPDX-License-Identifier: MIT

// Executable fake summary-peer boundary.  It accepts only a closed line-delimited document and
// returns digests/metadata, never control commands or raw source text.

pub const FAKE_PEER_SCHEMA: &str = "ascension.context-memory.fake-peer.v1";
const MAX_PEER_LINE_BYTES: usize = MAX_JOB_INPUT_BYTES + 4096;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FakePeerSource {
    pub entry_id: String,
    pub version: u64,
    pub sha256: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FakePeerRequest {
    pub schema: String,
    pub job_id: String,
    pub generator_profile: String,
    pub prompt_sha256: String,
    pub output_schema_sha256: String,
    pub sources: Vec<FakePeerSource>,
    pub max_output_bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FakePeerResponse {
    pub schema: String,
    pub job_id: String,
    pub source_manifest_sha256: String,
    pub source_count: usize,
    pub input_bytes: usize,
    pub output_sha256: String,
    pub output_bytes: usize,
    pub effect_class: String,
}

pub fn run_fake_peer_line(
    line: &[u8],
    peer: &mut FakeSummaryPeer,
) -> Result<FakePeerResponse, MemoryError> {
    if line.len() > MAX_PEER_LINE_BYTES {
        return Err(MemoryError::QueryTooLarge);
    }
    let request: FakePeerRequest = parse_strict_json(line).map_err(|_| MemoryError::InvalidQuery)?;
    if request.schema != FAKE_PEER_SCHEMA
        || !valid_id(&request.job_id)
        || !valid_id(&request.generator_profile)
        || !valid_digest(&request.prompt_sha256)
        || !valid_digest(&request.output_schema_sha256)
        || request.sources.is_empty()
        || request.sources.len() > MAX_SOURCES_PER_JOB
        || request.max_output_bytes == 0
        || request.max_output_bytes > MAX_SUMMARY_OUTPUT_BYTES
    {
        return Err(MemoryError::InvalidQuery);
    }
    let mut source_bytes = Vec::with_capacity(request.sources.len());
    let mut references = Vec::with_capacity(request.sources.len());
    for source in request.sources {
        let reference = MemoryRef::new(source.entry_id, source.version, source.sha256);
        if !reference.valid() || sha256_hex(&source.bytes) != reference.sha256 {
            return Err(MemoryError::InvalidProposal);
        }
        references.push(reference.clone());
        source_bytes.push((reference, source.bytes));
    }
    let input_bytes = source_bytes
        .iter()
        .map(|(_, bytes)| bytes.len())
        .sum::<usize>();
    if input_bytes > MAX_JOB_INPUT_BYTES {
        return Err(MemoryError::Capacity);
    }
    let generated = peer.generate(SummaryGenerationRequest {
        job_id: request.job_id.clone(),
        generator_profile: request.generator_profile,
        prompt_sha256: request.prompt_sha256,
        output_schema_sha256: request.output_schema_sha256,
        source_bytes,
        max_output_bytes: request.max_output_bytes,
    })?;
    Ok(FakePeerResponse {
        schema: FAKE_PEER_SCHEMA.to_owned(),
        job_id: request.job_id,
        source_manifest_sha256: source_manifest_digest(&references),
        source_count: references.len(),
        input_bytes,
        output_sha256: generated.output_sha256,
        output_bytes: generated.output.len(),
        effect_class: "authorized_summary_generation_only".to_owned(),
    })
}
