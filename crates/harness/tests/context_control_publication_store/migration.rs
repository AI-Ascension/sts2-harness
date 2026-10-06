// SPDX-License-Identifier: MIT

#[test]
fn v2_marker_migration_preserves_v2_aad_ciphertext_and_adds_empty_publication_tables() {
    let (directory, mut store) = publication_store_fixture("migration-v2");
    let owner_state = publication_owner_state(&mut store);
    add_static_source(&mut store);
    let (_authority, command, record) = persist_pause_control_receipt(&mut store);

    let path = directory.join("control.sqlite3");
    drop(store);
    // This is an additive-migration fixture, not an archived historical database image: current
    // writers create the rows, and these independent decryptions pin the retained v2 AAD bytes
    // before the fixture switches to the v2 marker and removes only the two v3 tables.
    assert_v2_ciphertext_aad_compatibility(&path, &owner_state, &record);
    let connection = rusqlite::Connection::open(&path).expect("open v3 fixture to mark v2");
    connection
        .execute_batch(
            "UPDATE context_control_meta SET value = 'ascension.context-control.sqlite.v2' WHERE key = 'schema';
             UPDATE context_control_meta SET value = '2' WHERE key = 'schema_version';
             DROP TABLE context_control_owner_publications;
             DROP TABLE context_control_active_publication_links;",
        )
        .expect("construct v2 schema marker and additive table set");
    drop(connection);
    let before = legacy_cipher_rows(&path);

    let migrated = ContextControlStore::open(&path, PUBLICATION_KEY, PUBLICATION_RUN)
        .expect("add publication tables to v2 store");
    let after_open = legacy_cipher_rows(&path);
    assert_eq!(
        after_open, before,
        "migration leaves every v2 ciphertext and index unchanged"
    );
    let marker = rusqlite::Connection::open(&path).expect("inspect migrated schema");
    assert_eq!(
        marker
            .query_row(
                "SELECT value FROM context_control_meta WHERE key = 'schema_version'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("schema version"),
        "3"
    );
    for table in [
        "context_control_owner_publications",
        "context_control_active_publication_links",
    ] {
        assert_eq!(
            marker
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [table],
                    |row| row.get::<_, i64>(0),
                )
                .expect("new table exists"),
            1
        );
        assert_eq!(
            marker
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row
                    .get::<_, i64>(0),)
                .expect("new table starts empty"),
            0
        );
    }
    drop(marker);

    let recovered_authority = migrated.load().expect("migrated journal decrypts");
    assert_eq!(recovered_authority.state().boundary.run_id, PUBLICATION_RUN);
    assert_eq!(
        migrated
            .lookup_owner_control_receipt(PUBLICATION_OWNER, PUBLICATION_ACTOR, &command)
            .expect("migrated control receipt authenticates"),
        Some(record)
    );
    assert_eq!(
        migrated
            .load_owner_context_state(PUBLICATION_OWNER)
            .expect("migrated owner state decrypts")
            .expect("owner state")
            .bytes,
        owner_state
    );
    let static_document = publication_source("configured.source");
    assert!(
        migrated
            .load_context_source("configured.source.1", 1, &static_document.digest,)
            .expect("migrated source decrypts")
            .is_some()
    );
    drop(migrated);
    assert_eq!(legacy_cipher_rows(&path), before);
    cleanup_publication_store(directory);
}

fn assert_v2_ciphertext_aad_compatibility(
    path: &Path,
    owner_state: &[u8],
    control_receipt: &DurableContextOwnerControlReceipt,
) {
    let connection = rusqlite::Connection::open(path).expect("open v2-format rows");
    let journal: Vec<u8> = connection
        .query_row(
            "SELECT envelope FROM context_control_journal WHERE run_id = ?1",
            [PUBLICATION_RUN],
            |row| row.get(0),
        )
        .expect("legacy journal envelope");
    let journal_plaintext = decrypt_with_v2_aad(&journal, b"ascension.context-control.sqlite.v1\0");
    assert!(
        serde_json::from_slice::<serde_json::Value>(&journal_plaintext).is_ok(),
        "original v2 journal AAD decrypts a valid authority journal"
    );

    let owner_envelope: Vec<u8> = connection
        .query_row(
            "SELECT envelope FROM context_control_owner_state WHERE run_id = ?1",
            [PUBLICATION_RUN],
            |row| row.get(0),
        )
        .expect("legacy owner-state envelope");
    let mut owner_aad = b"ascension.context-control.owner-state.v1\0".to_vec();
    owner_aad.extend_from_slice(&length_prefixed_v2_aad(&[
        PUBLICATION_RUN.as_bytes(),
        PUBLICATION_OWNER.as_bytes(),
    ]));
    assert_eq!(
        decrypt_with_v2_aad(&owner_envelope, &owner_aad).as_slice(),
        owner_state
    );

    let (source_id, source_envelope): (String, Vec<u8>) = connection
        .query_row(
            "SELECT source_id, envelope FROM context_control_context_sources
             WHERE run_id = ?1 AND source_id = 'configured.source.1' AND version = 1",
            [PUBLICATION_RUN],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("legacy source envelope");
    let source_aad = format!(
        "ascension.context-control.source.v1\0{PUBLICATION_RUN}\0{source_id}\0{}",
        1_u64
    );
    let expected_source = serde_json::to_vec(&publication_source("configured.source").document)
        .expect("serialize expected legacy source");
    assert_eq!(
        decrypt_with_v2_aad(&source_envelope, source_aad.as_bytes()),
        expected_source
    );

    let command_bytes =
        serde_json::to_vec(&control_receipt.command).expect("serialize legacy control command");
    let command_digest = Sha256::digest(&command_bytes);
    let command_digest = command_digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let (stored_command_digest, receipt_envelope): (String, Vec<u8>) = connection
        .query_row(
            "SELECT command_digest, envelope FROM context_control_owner_receipts
             WHERE run_id = ?1 AND owner_id = ?2",
            rusqlite::params![PUBLICATION_RUN, PUBLICATION_OWNER],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("legacy owner control receipt envelope");
    assert_eq!(stored_command_digest, command_digest);
    let receipt_aad = length_prefixed_v2_aad(&[
        b"ascension.context-control.owner-receipt.v1\0",
        PUBLICATION_RUN.as_bytes(),
        PUBLICATION_OWNER.as_bytes(),
        command_digest.as_bytes(),
    ]);
    let expected_receipt =
        serde_json::to_vec(control_receipt).expect("serialize expected legacy control receipt");
    assert_eq!(
        decrypt_with_v2_aad(&receipt_envelope, &receipt_aad),
        expected_receipt
    );
}

fn length_prefixed_v2_aad(components: &[&[u8]]) -> Vec<u8> {
    let mut aad = Vec::new();
    for component in components {
        aad.extend_from_slice(
            &u64::try_from(component.len())
                .expect("fixture field length fits u64")
                .to_be_bytes(),
        );
        aad.extend_from_slice(component);
    }
    aad
}

fn decrypt_with_v2_aad(envelope: &[u8], aad: &[u8]) -> Vec<u8> {
    assert!(
        envelope.len() >= 40,
        "legacy XChaCha envelope includes nonce and tag"
    );
    let (nonce, ciphertext) = envelope.split_at(24);
    let nonce: &XNonce = nonce.try_into().expect("24-byte legacy nonce");
    XChaCha20Poly1305::new(&Key::from(PUBLICATION_KEY))
        .decrypt(
            nonce,
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .expect("decrypt with frozen v2 AAD framing")
}

#[derive(Debug, Eq, PartialEq)]
struct LegacyCipherRows {
    journal: (Vec<u8>, String),
    owner_state: (i64, Vec<u8>, String),
    control_receipts: Vec<(String, String, Vec<u8>, String)>,
    sources: Vec<(String, i64, String, Vec<u8>, String)>,
}

fn legacy_cipher_rows(path: &Path) -> LegacyCipherRows {
    let connection = rusqlite::Connection::open(path).expect("open legacy rows");
    let journal = connection
        .query_row(
            "SELECT envelope, envelope_digest FROM context_control_journal WHERE run_id = ?1",
            [PUBLICATION_RUN],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("legacy encrypted journal");
    let owner_state = connection
        .query_row(
            "SELECT record_version, envelope, envelope_digest FROM context_control_owner_state WHERE run_id = ?1",
            [PUBLICATION_RUN],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("legacy encrypted owner state");
    let control_receipts = connection
        .prepare(
            "SELECT command_digest, idempotency_digest, envelope, envelope_digest
             FROM context_control_owner_receipts WHERE run_id = ?1 ORDER BY command_digest",
        )
        .expect("prepare legacy receipt rows")
        .query_map([PUBLICATION_RUN], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .expect("query legacy receipt rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("read legacy receipt rows");
    let sources = connection
        .prepare(
            "SELECT source_id, version, source_digest, envelope, envelope_digest
             FROM context_control_context_sources WHERE run_id = ?1 ORDER BY source_id, version",
        )
        .expect("prepare legacy source rows")
        .query_map([PUBLICATION_RUN], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .expect("query legacy source rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("read legacy source rows");
    LegacyCipherRows {
        journal,
        owner_state,
        control_receipts,
        sources,
    }
}
