// SPDX-License-Identifier: MIT

use std::cmp::Ordering;
use std::collections::BTreeMap;

use super::definition::{GuardExpression, GuardValue};
use super::ids::FieldId;

const MAX_GUARD_DEPTH: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TruthValue {
    True,
    False,
    Unknown,
}

impl TruthValue {
    #[must_use]
    pub const fn is_true(self) -> bool {
        matches!(self, Self::True)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardError {
    DepthExceeded,
}

impl std::fmt::Display for GuardError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("workflow guard exceeded its evaluation bound")
    }
}

impl std::error::Error for GuardError {}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GuardContext {
    fields: BTreeMap<FieldId, GuardValue>,
}

impl GuardContext {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, field: FieldId, value: GuardValue) {
        self.fields.insert(field, value);
    }

    #[must_use]
    pub fn get(&self, field: &FieldId) -> Option<&GuardValue> {
        self.fields.get(field)
    }

    pub fn evaluate(&self, expression: &GuardExpression) -> Result<TruthValue, GuardError> {
        evaluate(self, expression, 0)
    }
}

fn evaluate(
    context: &GuardContext,
    expression: &GuardExpression,
    depth: usize,
) -> Result<TruthValue, GuardError> {
    if depth > MAX_GUARD_DEPTH {
        return Err(GuardError::DepthExceeded);
    }
    match expression {
        GuardExpression::Literal(value) => Ok(truth_from_value(value)),
        GuardExpression::Field(field) => Ok(context
            .get(field)
            .map_or(TruthValue::Unknown, truth_from_value)),
        GuardExpression::Exists(field) => Ok(if context.get(field).is_some() {
            TruthValue::True
        } else {
            TruthValue::False
        }),
        GuardExpression::Equal { left, right } => Ok(compare_values(context.get(left), right)
            .map_or(TruthValue::Unknown, |ordering| {
                if ordering == Ordering::Equal {
                    TruthValue::True
                } else {
                    TruthValue::False
                }
            })),
        GuardExpression::NotEqual { left, right } => Ok(compare_values(context.get(left), right)
            .map_or(TruthValue::Unknown, |ordering| {
                if ordering == Ordering::Equal {
                    TruthValue::False
                } else {
                    TruthValue::True
                }
            })),
        GuardExpression::Less { left, right } => {
            ordered(context, left, right, |value| value.is_lt())
        }
        GuardExpression::LessOrEqual { left, right } => {
            ordered(context, left, right, |value| value.is_le())
        }
        GuardExpression::Greater { left, right } => {
            ordered(context, left, right, |value| value.is_gt())
        }
        GuardExpression::GreaterOrEqual { left, right } => {
            ordered(context, left, right, |value| value.is_ge())
        }
        GuardExpression::And(values) => {
            let mut result = TruthValue::True;
            for child in values {
                result = and(result, evaluate(context, child, depth + 1)?);
                if result == TruthValue::False {
                    break;
                }
            }
            Ok(result)
        }
        GuardExpression::Or(values) => {
            let mut result = TruthValue::False;
            for child in values {
                result = or(result, evaluate(context, child, depth + 1)?);
                if result == TruthValue::True {
                    break;
                }
            }
            Ok(result)
        }
        GuardExpression::Not(value) => Ok(match evaluate(context, value, depth + 1)? {
            TruthValue::True => TruthValue::False,
            TruthValue::False => TruthValue::True,
            TruthValue::Unknown => TruthValue::Unknown,
        }),
        GuardExpression::In { field, values } => {
            let Some(actual) = context.get(field) else {
                return Ok(TruthValue::Unknown);
            };
            Ok(if values.iter().any(|value| value == actual) {
                TruthValue::True
            } else {
                TruthValue::False
            })
        }
        GuardExpression::Add { left, right } => {
            let Some(GuardValue::Integer(value)) = context.get(left) else {
                return Ok(TruthValue::Unknown);
            };
            Ok(match value.checked_add(*right) {
                Some(sum) if sum != 0 => TruthValue::True,
                Some(_) => TruthValue::False,
                None => TruthValue::Unknown,
            })
        }
    }
}

fn ordered(
    context: &GuardContext,
    field: &FieldId,
    right: &GuardValue,
    predicate: impl FnOnce(Ordering) -> bool,
) -> Result<TruthValue, GuardError> {
    Ok(
        compare_values(context.get(field), right).map_or(TruthValue::Unknown, |ordering| {
            if predicate(ordering) {
                TruthValue::True
            } else {
                TruthValue::False
            }
        }),
    )
}

fn compare_values(left: Option<&GuardValue>, right: &GuardValue) -> Option<Ordering> {
    let left = left?;
    match (left, right) {
        (GuardValue::Null, GuardValue::Null) => Some(Ordering::Equal),
        (GuardValue::Boolean(left), GuardValue::Boolean(right)) => Some(left.cmp(right)),
        (GuardValue::Integer(left), GuardValue::Integer(right)) => Some(left.cmp(right)),
        (GuardValue::Text(left), GuardValue::Text(right)) => Some(left.cmp(right)),
        _ => None,
    }
}

fn truth_from_value(value: &GuardValue) -> TruthValue {
    match value {
        GuardValue::Boolean(value) => {
            if *value {
                TruthValue::True
            } else {
                TruthValue::False
            }
        }
        GuardValue::Integer(value) => {
            if *value == 0 {
                TruthValue::False
            } else {
                TruthValue::True
            }
        }
        GuardValue::Text(value) => {
            if value.is_empty() {
                TruthValue::False
            } else {
                TruthValue::True
            }
        }
        GuardValue::Null => TruthValue::False,
    }
}

fn and(left: TruthValue, right: TruthValue) -> TruthValue {
    match (left, right) {
        (TruthValue::False, _) | (_, TruthValue::False) => TruthValue::False,
        (TruthValue::Unknown, _) | (_, TruthValue::Unknown) => TruthValue::Unknown,
        _ => TruthValue::True,
    }
}

fn or(left: TruthValue, right: TruthValue) -> TruthValue {
    match (left, right) {
        (TruthValue::True, _) | (_, TruthValue::True) => TruthValue::True,
        (TruthValue::Unknown, _) | (_, TruthValue::Unknown) => TruthValue::Unknown,
        _ => TruthValue::False,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::{GuardContext, TruthValue};
    use crate::workflow::{FieldId, GuardExpression, GuardValue};

    #[test]
    fn missing_values_are_unknown_and_short_circuiting_is_kleene() {
        let field = FieldId::new("ready").expect("valid field");
        let context = GuardContext::new();
        let expression = GuardExpression::And(vec![
            GuardExpression::Field(field.clone()),
            GuardExpression::Literal(GuardValue::Boolean(false)),
        ]);
        assert_eq!(context.evaluate(&expression), Ok(TruthValue::False));
        assert_eq!(
            context.evaluate(&GuardExpression::Field(field)),
            Ok(TruthValue::Unknown)
        );
    }

    #[test]
    fn numeric_comparisons_and_overflow_fail_closed() {
        let field = FieldId::new("count").expect("valid field");
        let mut context = GuardContext::new();
        context.insert(field.clone(), GuardValue::Integer(i64::MAX));
        assert_eq!(
            context.evaluate(&GuardExpression::Greater {
                left: field.clone(),
                right: GuardValue::Integer(1),
            }),
            Ok(TruthValue::True)
        );
        assert_eq!(
            context.evaluate(&GuardExpression::Add {
                left: field,
                right: 1,
            }),
            Ok(TruthValue::Unknown)
        );
    }
}
