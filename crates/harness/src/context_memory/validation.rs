// SPDX-License-Identifier: MIT

fn valid_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    if !(20..=30).contains(&bytes.len())
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
    {
        return false;
    }
    let digits = |start: usize, end: usize| {
        bytes[start..end].iter().all(u8::is_ascii_digit)
    };
    if !digits(0, 4)
        || !digits(5, 7)
        || !digits(8, 10)
        || !digits(11, 13)
        || !digits(14, 16)
        || !digits(17, 19)
    {
        return false;
    }
    if bytes.len() > 20 && (bytes[20] != b'.' || bytes[21..].is_empty() || !digits(21, bytes.len())) {
        return false;
    }
    let parse = |start: usize, end: usize| {
        value[start..end].parse::<u32>().unwrap_or_default()
    };
    let year = parse(0, 4);
    let month = parse(5, 7);
    let day = parse(8, 10);
    let hour = parse(11, 13);
    let minute = parse(14, 16);
    let second = parse(17, 19);
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    };
    (1..=12).contains(&month)
        && (1..=days).contains(&day)
        && hour < 24
        && minute < 60
        && second < 60
}

fn validate_tombstone(entry: &MemoryEntry) -> Result<(), MemoryError> {
    if entry.schema != MEMORY_ENTRY_SCHEMA
        || !valid_id(&entry.entry_id)
        || entry.version == 0
        || !entry.scope.valid()
        || !valid_id(&entry.branch_id)
        || !valid_id(&entry.source_record_id)
        || !valid_id(&entry.content_ref)
        || entry.authority != "historical_data_only"
        || !valid_digest(&entry.sha256)
        || entry.lineage_depth > MAX_LINEAGE_DEPTH
        || entry.parents.len() > 16
        || entry
            .parents
            .iter()
            .map(MemoryParent::reference)
            .collect::<BTreeSet<_>>()
            .len()
            != entry.parents.len()
        || entry.status != EntryStatus::Revoked
        || !valid_timestamp(&entry.created_at)
        || !valid_timestamp(&entry.expires_at)
        || entry.expires_at.as_str() <= entry.created_at.as_str()
        || entry.game_profile.is_empty()
        || !entry.content.is_empty()
    {
        return Err(MemoryError::InvalidEntry);
    }
    if entry.parents.iter().any(|parent| {
        let reference = parent.reference();
        !reference.valid() || reference.entry_id == entry.entry_id
    }) {
        return Err(MemoryError::LineageCycle);
    }
    Ok(())
}

fn valid_lineage_depth(
    entry: &MemoryEntry,
    entries: &BTreeMap<MemoryRef, MemoryEntry>,
) -> bool {
    if entry.parents.is_empty() {
        return entry.lineage_depth == 0;
    }
    entry.lineage_depth
        == entry
            .parents
            .iter()
            .filter_map(|parent| entries.get(&parent.reference()))
            .map(|parent| parent.lineage_depth)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
}
