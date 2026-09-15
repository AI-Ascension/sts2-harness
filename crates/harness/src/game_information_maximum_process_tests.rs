// SPDX-License-Identifier: MIT
//! Maximum raw-source reconstruction through the actual bounded subprocess loop.
use super::*;

struct MaximumSourcePort(SyntheticMcpPort);
impl LookupMcpPort for MaximumSourcePort {
    fn information_correlation(&self) -> Result<String, LookupError> {
        self.0.information_correlation()
    }
    fn call_information(&mut self, tool: &str, query: &Value) -> Result<Vec<u8>, LookupError> {
        let mut bytes = self.0.call_information(tool, query)?;
        // Legal trailing JSON whitespace exercises the exact raw-byte boundary without
        // misrepresenting the independently checked field/page accounting.
        bytes.resize(65_536, b' ');
        Ok(bytes)
    }
}

fn query_arguments() -> Result<String, LookupError> {
    let mut query = request(false, "1")["query"].clone();
    for key in ["binding", "parent_observation"] {
        query
            .as_object_mut()
            .ok_or(LookupError::Invalid)?
            .remove(key);
    }
    query["target"]
        .as_object_mut()
        .ok_or(LookupError::Invalid)?
        .remove("instance_ref");
    serde_json::to_string(&json!({"operation_id":"maximum","mode":"static","query":query}))
        .map_err(|_| LookupError::Invalid)
}

const RECONSTRUCT: &str = r#"import json,sys,hashlib
f=json.loads(input())
budget=f['payload']['optional_byte_budget']
f['sequence']=1
f['payload']={'kind':'query','arguments':json.loads(sys.argv[1])}
print(json.dumps(f),flush=True)
ordinal=json.loads(input())['payload']['value']['record_ordinal']
data=bytearray()
while len(data)<65536:
 f['sequence']+=1
 f['payload']={'kind':'read_retained','record_ordinal':ordinal,'offset':len(data)}
 print(json.dumps(f),flush=True)
 r=json.loads(input())['payload']['value']
 assert len(json.dumps(r,separators=(',',':')).encode())<=budget
 part=bytes.fromhex(r['bytes'])
 assert 0<len(part)<=3000 and r['offset']==len(data) and r['next_offset']==len(data)+len(part)
 assert r['total_bytes']==65536 and r['authority']=='untrusted_game_information_data'
 data.extend(part)
assert len(data)==65536 and hashlib.sha256(data).hexdigest()==sys.argv[2]
assert f['sequence']==23
f['sequence']+=1
f['payload']={'kind':'decision','action_id':'play:card-17'}
print(json.dumps(f),flush=True)
"#;

#[test]
fn maximum_source_is_complete_in_twenty_four_production_turns() -> TestResult {
    let (mut session, mut corpus) = setup(7000)?;
    negotiate(&mut session)?;
    let mut expected = dispatch("sts2.game_information_list", &request(false, "20"), false)?;
    expected.resize(65_536, b' ');
    let expected_hash = crate::sha256_hex(&expected);
    let mut agent = agent(
        RECONSTRUCT,
        vec![query_arguments()?, expected_hash.clone()],
        Duration::from_secs(15),
    )?;
    let mut port = MaximumSourcePort(SyntheticMcpPort { id: 20 });
    assert_eq!(
        run_lookup_tool_loop(
            &mut session,
            &mut corpus,
            &mut port,
            &mut agent,
            &legal()?,
            32
        )?,
        "play:card-17"
    );
    assert_eq!(session.records.len(), 1);
    assert_eq!(session.records[0].source_bytes, 65_536);
    assert_eq!(session.records[0].source_sha256, Some(expected_hash));
    assert_eq!(port.0.id, 21);
    Ok(())
}

#[test]
fn smaller_or_denied_budget_never_reports_complete_reconstruction() -> TestResult {
    assert!(setup(0).is_err());
    let (mut session, mut corpus) = setup(2000)?;
    negotiate(&mut session)?;
    let mut agent = agent(
        RECONSTRUCT,
        vec![query_arguments()?, "unused-on-exhaustion".into()],
        Duration::from_secs(15),
    )?;
    let mut port = MaximumSourcePort(SyntheticMcpPort { id: 20 });
    assert_eq!(
        run_lookup_tool_loop(
            &mut session,
            &mut corpus,
            &mut port,
            &mut agent,
            &legal()?,
            32
        ),
        Err(LookupError::Bounds)
    );
    assert_eq!(session.records.len(), 1);
    assert_eq!(session.records[0].source_bytes, 65_536);
    Ok(())
}

#[test]
fn maximum_encoded_chunk_fits_feedback_and_rejects_invalid_coordinates() -> TestResult {
    let bytes = (0_u8..=255).cycle().take(3000).collect::<Vec<_>>();
    let feedback = LookupFeedback::Bytes {
        record_ordinal: 255,
        offset: 62_536,
        total_bytes: 65_536,
        bytes: bytes.clone(),
    };
    let value = crate::exo_lookup_wire::feedback_value(&feedback, 7000)?;
    assert_eq!(value["bytes"], crate::hex_bytes(bytes));
    assert_eq!(value["next_offset"], 65_536);
    assert!(serde_json::to_vec(&value)?.len() <= crate::exo_lookup_wire::EXO_LOOKUP_FEEDBACK_BYTES);
    let small = crate::exo_lookup_wire::feedback_value(&feedback, 2000)?;
    assert!(serde_json::to_vec(&small)?.len() <= 2000);
    assert!(small["bytes"].as_str().ok_or("hex missing")?.len() < 6000);
    assert_eq!(
        crate::exo_lookup_wire::feedback_value(&feedback, 1),
        Err(LookupError::Bounds)
    );
    for (ordinal, offset, total) in [(256, 0, 65_536), (0, usize::MAX, 65_536), (0, 0, 65_537)] {
        let bad = LookupFeedback::Bytes {
            record_ordinal: ordinal,
            offset,
            total_bytes: total,
            bytes: vec![0; 3000],
        };
        assert_eq!(
            crate::exo_lookup_wire::feedback_value(&bad, 7000),
            Err(LookupError::Bounds)
        );
    }
    Ok(())
}

#[test]
fn data_wrapper_overhead_uses_the_same_prepared_budget() -> TestResult {
    let (mut session, mut corpus) = setup(8192)?;
    negotiate(&mut session)?;
    let mut port = SyntheticMcpPort { id: 20 };
    let delivery = session.query_port(
        &session.binding.clone(),
        "view",
        &serde_json::to_vec(&request(false, "1"))?,
        &mut corpus,
        &mut port,
    )?;
    let view_bytes = serde_json::to_vec(&delivery.data)?.len();
    let feedback = LookupFeedback::Data {
        record_ordinal: 0,
        delivery: Box::new(delivery),
    };
    let projected = crate::exo_lookup_wire::feedback_value(&feedback, view_bytes)?;
    assert!(serde_json::to_vec(&projected)?.len() <= view_bytes);
    assert_eq!(projected["data"]["delivery"], "retained");
    assert_eq!(
        crate::exo_lookup_wire::feedback_value(&feedback, 1),
        Err(LookupError::Bounds)
    );
    Ok(())
}
