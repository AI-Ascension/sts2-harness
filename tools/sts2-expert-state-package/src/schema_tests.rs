// SPDX-License-Identifier: MIT

use std::collections::HashSet;
use std::error::Error;
use std::fs;
use std::path::Path;

use serde_json::Value;

#[test]
fn representative_instances_and_negative_mutations_match_all_five_schemas()
-> Result<(), Box<dyn Error>> {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/research/sts2-expert-state-package");
    let fixtures = root.join("fixtures/schema");
    let manifest = read_json(&fixtures.join("cases.json"))?;
    let cases = manifest["cases"].as_array().ok_or("missing case array")?;
    assert_eq!(
        cases.len(),
        30,
        "review changes to the explicit coverage set"
    );
    let mut case_ids = HashSet::new();
    let mut positives = HashSet::new();
    for case in cases {
        let case_id = required_string(case, "case_id")?;
        assert!(case_ids.insert(case_id), "duplicate case {case_id}");
        let schema_name = required_string(case, "schema")?;
        let schema = read_json(&root.join("schemas").join(schema_name))?;
        let validator = jsonschema::draft202012::options()
            .should_validate_formats(true)
            .build(&schema)?;
        let mut record = read_json(&fixtures.join(required_string(case, "fixture")?))?;
        apply_changes(&mut record, &case["changes"])?;
        let expected = case["expected_valid"]
            .as_bool()
            .ok_or("missing expectation")?;
        let errors: Vec<_> = validator.iter_errors(&record).collect();
        assert_eq!(errors.is_empty(), expected, "{case_id}: {errors:?}");
        if expected {
            positives.insert(schema_name);
        }
    }
    assert_eq!(
        positives.len(),
        5,
        "each schema needs a valid full instance"
    );
    Ok(())
}

fn required_string<'a>(object: &'a Value, key: &str) -> Result<&'a str, Box<dyn Error>> {
    object[key]
        .as_str()
        .ok_or_else(|| format!("missing string {key}").into())
}

fn read_json(path: &Path) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn apply_changes(record: &mut Value, changes: &Value) -> Result<(), Box<dyn Error>> {
    for change in changes.as_array().ok_or("missing changes array")? {
        let pointer = required_string(change, "path")?;
        let (parent, key) = pointer.rsplit_once('/').ok_or("invalid pointer")?;
        let target = record.pointer_mut(parent).ok_or("missing parent")?;
        let value = change.get("value").ok_or("missing replacement")?.clone();
        if let Some(object) = target.as_object_mut() {
            object.insert(key.to_owned(), value);
        } else if let Some(array) = target.as_array_mut() {
            *array
                .get_mut(key.parse::<usize>()?)
                .ok_or("missing array member")? = value;
        } else {
            return Err("change parent is not a container".into());
        }
    }
    Ok(())
}
