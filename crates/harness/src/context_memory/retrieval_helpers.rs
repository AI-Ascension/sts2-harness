// SPDX-License-Identifier: MIT

fn bounded_snippet(bytes: &[u8]) -> Option<String> {
    std::str::from_utf8(bytes)
        .ok()
        .map(|text| text.chars().take(512).collect())
}

const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "has", "have", "in", "is",
    "it", "of", "on", "or", "that", "the", "this", "to", "was", "with",
];

pub fn normalize_terms(value: &str) -> Result<Vec<String>, MemoryError> {
    if value.is_empty() {
        return Ok(Vec::new());
    }
    if value.len() > MAX_QUERY_BYTES {
        return Err(MemoryError::QueryTooLarge);
    }
    let mut terms = Vec::new();
    let mut current = String::new();
    for character in value.chars() {
        if character.is_alphanumeric() {
            for lower in character.to_lowercase() {
                current.push(lower);
            }
        } else if !current.is_empty() {
            if !STOPWORDS.contains(&current.as_str()) {
                terms.push(std::mem::take(&mut current));
            } else {
                current.clear();
            }
        }
        if terms.len() > MAX_CANDIDATES {
            return Err(MemoryError::TooManyTerms);
        }
    }
    if !current.is_empty() && !STOPWORDS.contains(&current.as_str()) {
        terms.push(current);
    }
    if terms.len() > MAX_CANDIDATES {
        return Err(MemoryError::TooManyTerms);
    }
    Ok(terms)
}

fn normalize_document_terms(value: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let mut current = String::new();
    for character in value.chars() {
        if character.is_alphanumeric() {
            for lower in character.to_lowercase() {
                current.push(lower);
            }
        } else if !current.is_empty() {
            if !STOPWORDS.contains(&current.as_str()) {
                terms.push(std::mem::take(&mut current));
            } else {
                current.clear();
            }
            if terms.len() >= MAX_CANDIDATES * 16 {
                break;
            }
        }
    }
    if terms.len() < MAX_CANDIDATES * 16
        && !current.is_empty()
        && !STOPWORDS.contains(&current.as_str())
    {
        terms.push(current);
    }
    terms
}
