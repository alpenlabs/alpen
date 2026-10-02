//! Fixed scalar expressions used by console pipelines.

use std::cmp::Ordering;

use crate::{ConsoleError, ConsoleResult, ConsoleRow, ConsoleScalar, ScalarType, ValueMetadata};

/// A fixed binary operation supported by the console evaluator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOperator {
    /// Equality.
    Equal,
    /// Inequality.
    NotEqual,
    /// Strictly less than.
    Less,
    /// Less than or equal.
    LessOrEqual,
    /// Strictly greater than.
    Greater,
    /// Greater than or equal.
    GreaterOrEqual,
    /// Short-circuiting boolean conjunction.
    And,
    /// Short-circuiting boolean disjunction.
    Or,
    /// Checked addition.
    Add,
    /// Checked subtraction.
    Subtract,
    /// Checked multiplication.
    Multiply,
    /// Checked division.
    Divide,
}

/// A scalar expression assembled by trusted Rust code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScalarExpression {
    /// A scalar literal.
    Literal(ConsoleScalar),
    /// The current row's key.
    Key,
    /// An approved getter on the current row.
    Field(String),
    /// Boolean negation.
    Not(Box<Self>),
    /// A fixed binary operation.
    Binary {
        /// Operation to evaluate.
        operator: BinaryOperator,
        /// Left operand.
        left: Box<Self>,
        /// Right operand.
        right: Box<Self>,
    },
}

impl ScalarExpression {
    /// Creates a literal expression.
    pub fn literal(value: ConsoleScalar) -> Self {
        Self::Literal(value)
    }

    /// Creates a field expression.
    pub fn field(name: impl Into<String>) -> Self {
        Self::Field(name.into())
    }

    /// Creates a binary expression.
    pub fn binary(operator: BinaryOperator, left: Self, right: Self) -> Self {
        Self::Binary {
            operator,
            left: Box::new(left),
            right: Box::new(right),
        }
    }

    /// Evaluates this expression against one materialized console row.
    pub fn evaluate(&self, row: &ConsoleRow) -> ConsoleResult<ConsoleScalar> {
        match self {
            Self::Literal(value) => Ok(value.clone()),
            Self::Key => Ok(row.key.clone()),
            Self::Field(name) => row.fields.get(name.as_str()).cloned().ok_or_else(|| {
                ConsoleError::invalid_input(
                    "expression field",
                    format!("field '{name}' is not present in the row"),
                )
            }),
            Self::Not(expression) => Ok(ConsoleScalar::Bool(
                !expression.evaluate(row)?.as_bool("boolean negation")?,
            )),
            Self::Binary {
                operator,
                left,
                right,
            } => evaluate_binary(*operator, left, right, row),
        }
    }

    pub(crate) fn validate(
        &self,
        key_type: ScalarType,
        metadata: &ValueMetadata,
    ) -> ConsoleResult<ExpressionType> {
        match self {
            Self::Literal(value) => Ok(ExpressionType::from_literal(value)),
            Self::Key => Ok(ExpressionType::scalar(key_type, false)),
            Self::Field(name) => metadata
                .fields
                .iter()
                .find(|field| field.name == name)
                .map(|field| ExpressionType::scalar(field.scalar_type, field.nullable))
                .ok_or_else(|| ConsoleError::UnknownField {
                    value: metadata.name,
                    field: name.clone(),
                }),
            Self::Not(expression) => {
                let expression_type = expression.validate(key_type, metadata)?;
                expression_type.require(ScalarType::Bool, "boolean negation")?;
                Ok(ExpressionType::scalar(ScalarType::Bool, false))
            }
            Self::Binary {
                operator,
                left,
                right,
            } => {
                let left_type = left.validate(key_type, metadata)?;
                let right_type = right.validate(key_type, metadata)?;
                validate_binary(*operator, left_type, right_type)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ExpressionType {
    scalar_type: Option<ScalarType>,
    nullable: bool,
}

impl ExpressionType {
    fn from_literal(value: &ConsoleScalar) -> Self {
        match value {
            ConsoleScalar::Null => Self {
                scalar_type: None,
                nullable: true,
            },
            ConsoleScalar::Bool(_) => Self::scalar(ScalarType::Bool, false),
            ConsoleScalar::I64(_) => Self::scalar(ScalarType::I64, false),
            ConsoleScalar::U64(_) => Self::scalar(ScalarType::U64, false),
            ConsoleScalar::String(_) => Self::scalar(ScalarType::String, false),
            ConsoleScalar::Bytes(_) => Self::scalar(ScalarType::Bytes, false),
        }
    }

    const fn scalar(scalar_type: ScalarType, nullable: bool) -> Self {
        Self {
            scalar_type: Some(scalar_type),
            nullable,
        }
    }

    pub(crate) fn require(self, expected: ScalarType, target: &'static str) -> ConsoleResult<()> {
        if self.scalar_type == Some(expected) {
            Ok(())
        } else {
            Err(ConsoleError::invalid_input(
                target,
                format!("expected {}, got {}", expected.as_str(), self.describe()),
            ))
        }
    }

    pub(crate) const fn scalar_type(self) -> Option<ScalarType> {
        self.scalar_type
    }

    fn describe(self) -> &'static str {
        self.scalar_type.map_or("null", ScalarType::as_str)
    }
}

fn validate_binary(
    operator: BinaryOperator,
    left: ExpressionType,
    right: ExpressionType,
) -> ConsoleResult<ExpressionType> {
    use BinaryOperator::{
        Add, And, Divide, Equal, Greater, GreaterOrEqual, Less, LessOrEqual, Multiply, NotEqual,
        Or, Subtract,
    };

    match operator {
        Equal | NotEqual => {
            let compatible = match (left.scalar_type, right.scalar_type) {
                (Some(left), Some(right)) => left == right,
                (None, Some(_)) => right.nullable,
                (Some(_), None) => left.nullable,
                (None, None) => true,
            };
            if !compatible {
                return Err(type_mismatch("equality", left, right));
            }
            Ok(ExpressionType::scalar(ScalarType::Bool, false))
        }
        Less | LessOrEqual | Greater | GreaterOrEqual => {
            let scalar_type = matching_types("comparison", left, right)?;
            if matches!(scalar_type, ScalarType::Bool) {
                return Err(ConsoleError::invalid_input(
                    "comparison",
                    "bool values support only equality",
                ));
            }
            Ok(ExpressionType::scalar(ScalarType::Bool, false))
        }
        And | Or => {
            left.require(ScalarType::Bool, "boolean operator")?;
            right.require(ScalarType::Bool, "boolean operator")?;
            Ok(ExpressionType::scalar(ScalarType::Bool, false))
        }
        Add | Subtract | Multiply | Divide => {
            let scalar_type = matching_types("arithmetic", left, right)?;
            if !matches!(scalar_type, ScalarType::I64 | ScalarType::U64) {
                return Err(ConsoleError::invalid_input(
                    "arithmetic",
                    format!("{} values do not support arithmetic", scalar_type.as_str()),
                ));
            }
            Ok(ExpressionType::scalar(
                scalar_type,
                left.nullable || right.nullable,
            ))
        }
    }
}

fn matching_types(
    target: &'static str,
    left: ExpressionType,
    right: ExpressionType,
) -> ConsoleResult<ScalarType> {
    match (left.scalar_type, right.scalar_type) {
        (Some(left_type), Some(right_type)) if left_type == right_type => Ok(left_type),
        _ => Err(type_mismatch(target, left, right)),
    }
}

fn type_mismatch(
    target: &'static str,
    left: ExpressionType,
    right: ExpressionType,
) -> ConsoleError {
    ConsoleError::invalid_input(
        target,
        format!(
            "incompatible {} and {} operands",
            left.describe(),
            right.describe()
        ),
    )
}

fn evaluate_binary(
    operator: BinaryOperator,
    left: &ScalarExpression,
    right: &ScalarExpression,
    row: &ConsoleRow,
) -> ConsoleResult<ConsoleScalar> {
    use BinaryOperator::{
        Add, And, Divide, Equal, Greater, GreaterOrEqual, Less, LessOrEqual, Multiply, NotEqual,
        Or, Subtract,
    };

    let left_value = left.evaluate(row)?;
    match operator {
        And if !left_value.as_bool("boolean operator")? => {
            return Ok(ConsoleScalar::Bool(false));
        }
        Or if left_value.as_bool("boolean operator")? => {
            return Ok(ConsoleScalar::Bool(true));
        }
        _ => {}
    }
    let right_value = right.evaluate(row)?;

    match operator {
        Equal => equal_scalars(&left_value, &right_value).map(ConsoleScalar::Bool),
        NotEqual => {
            equal_scalars(&left_value, &right_value).map(|equal| ConsoleScalar::Bool(!equal))
        }
        Less => compare_scalars(&left_value, &right_value)
            .map(|order| ConsoleScalar::Bool(order.is_lt())),
        LessOrEqual => compare_scalars(&left_value, &right_value)
            .map(|order| ConsoleScalar::Bool(order.is_le())),
        Greater => compare_scalars(&left_value, &right_value)
            .map(|order| ConsoleScalar::Bool(order.is_gt())),
        GreaterOrEqual => compare_scalars(&left_value, &right_value)
            .map(|order| ConsoleScalar::Bool(order.is_ge())),
        And | Or => Ok(ConsoleScalar::Bool(
            right_value.as_bool("boolean operator")?,
        )),
        Add | Subtract | Multiply | Divide => arithmetic(operator, left_value, right_value),
    }
}

fn equal_scalars(left: &ConsoleScalar, right: &ConsoleScalar) -> ConsoleResult<bool> {
    let equal = match (left, right) {
        (ConsoleScalar::Null, _) | (_, ConsoleScalar::Null) => left == right,
        (ConsoleScalar::Bool(left), ConsoleScalar::Bool(right)) => left == right,
        (ConsoleScalar::I64(left), ConsoleScalar::I64(right)) => left == right,
        (ConsoleScalar::U64(left), ConsoleScalar::U64(right)) => left == right,
        (ConsoleScalar::String(left), ConsoleScalar::String(right)) => left == right,
        (ConsoleScalar::Bytes(left), ConsoleScalar::Bytes(right)) => left == right,
        _ => {
            return Err(ConsoleError::invalid_input(
                "equality",
                format!("incompatible {} and {} operands", left.kind(), right.kind()),
            ));
        }
    };
    Ok(equal)
}

pub(crate) fn compare_scalars(
    left: &ConsoleScalar,
    right: &ConsoleScalar,
) -> ConsoleResult<Ordering> {
    let ordering = match (left, right) {
        (ConsoleScalar::I64(left), ConsoleScalar::I64(right)) => left.cmp(right),
        (ConsoleScalar::U64(left), ConsoleScalar::U64(right)) => left.cmp(right),
        (ConsoleScalar::String(left), ConsoleScalar::String(right)) => left.cmp(right),
        (ConsoleScalar::Bytes(left), ConsoleScalar::Bytes(right)) => left.cmp(right),
        _ => {
            return Err(ConsoleError::invalid_input(
                "comparison",
                format!("incompatible {} and {} operands", left.kind(), right.kind()),
            ));
        }
    };
    Ok(ordering)
}

fn arithmetic(
    operator: BinaryOperator,
    left: ConsoleScalar,
    right: ConsoleScalar,
) -> ConsoleResult<ConsoleScalar> {
    let value = match (left, right) {
        (ConsoleScalar::I64(left), ConsoleScalar::I64(right)) => {
            let value = match operator {
                BinaryOperator::Add => left.checked_add(right),
                BinaryOperator::Subtract => left.checked_sub(right),
                BinaryOperator::Multiply => left.checked_mul(right),
                BinaryOperator::Divide => left.checked_div(right),
                _ => unreachable!("caller passes only arithmetic operators"),
            };
            value.map(ConsoleScalar::I64)
        }
        (ConsoleScalar::U64(left), ConsoleScalar::U64(right)) => {
            let value = match operator {
                BinaryOperator::Add => left.checked_add(right),
                BinaryOperator::Subtract => left.checked_sub(right),
                BinaryOperator::Multiply => left.checked_mul(right),
                BinaryOperator::Divide => left.checked_div(right),
                _ => unreachable!("caller passes only arithmetic operators"),
            };
            value.map(ConsoleScalar::U64)
        }
        (left, right) => {
            return Err(ConsoleError::invalid_input(
                "arithmetic",
                format!("incompatible {} and {} operands", left.kind(), right.kind()),
            ));
        }
    };
    value.ok_or_else(|| {
        ConsoleError::invalid_input("arithmetic", "overflow, underflow, or division by zero")
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::{FieldDescriptor, ValueMetadata};

    use super::*;

    const FIELDS: &[FieldDescriptor] = &[
        FieldDescriptor {
            name: "enabled",
            scalar_type: ScalarType::Bool,
            nullable: false,
            settable: false,
        },
        FieldDescriptor {
            name: "value",
            scalar_type: ScalarType::U64,
            nullable: false,
            settable: false,
        },
        FieldDescriptor {
            name: "optional",
            scalar_type: ScalarType::U64,
            nullable: true,
            settable: false,
        },
    ];
    const METADATA: ValueMetadata = ValueMetadata {
        name: "ExpressionValue",
        fields: FIELDS,
    };

    fn binary(
        operator: BinaryOperator,
        left: ScalarExpression,
        right: ScalarExpression,
    ) -> ScalarExpression {
        ScalarExpression::binary(operator, left, right)
    }

    #[test]
    fn fixed_scalar_primitives_are_strict_checked_and_short_circuiting() {
        let row = ConsoleRow {
            source: "Expression",
            key: ConsoleScalar::U64(2),
            fields: BTreeMap::from([
                ("enabled".to_owned(), ConsoleScalar::Bool(true)),
                ("value".to_owned(), ConsoleScalar::U64(10)),
                ("optional".to_owned(), ConsoleScalar::Null),
            ]),
        };
        let cases = [
            (
                binary(
                    BinaryOperator::Add,
                    ScalarExpression::field("value"),
                    ScalarExpression::Key,
                ),
                ConsoleScalar::U64(12),
            ),
            (
                binary(
                    BinaryOperator::Greater,
                    ScalarExpression::field("value"),
                    ScalarExpression::literal(ConsoleScalar::U64(4)),
                ),
                ConsoleScalar::Bool(true),
            ),
            (
                binary(
                    BinaryOperator::Equal,
                    ScalarExpression::field("optional"),
                    ScalarExpression::literal(ConsoleScalar::Null),
                ),
                ConsoleScalar::Bool(true),
            ),
            (
                binary(
                    BinaryOperator::And,
                    ScalarExpression::literal(ConsoleScalar::Bool(false)),
                    binary(
                        BinaryOperator::Greater,
                        binary(
                            BinaryOperator::Divide,
                            ScalarExpression::field("value"),
                            ScalarExpression::literal(ConsoleScalar::U64(0)),
                        ),
                        ScalarExpression::literal(ConsoleScalar::U64(0)),
                    ),
                ),
                ConsoleScalar::Bool(false),
            ),
        ];

        for (expression, expected) in cases {
            expression
                .validate(ScalarType::U64, &METADATA)
                .expect("test: expression validates");
            assert_eq!(
                expression
                    .evaluate(&row)
                    .expect("test: evaluate expression"),
                expected
            );
        }

        let mixed_numbers = binary(
            BinaryOperator::Equal,
            ScalarExpression::field("value"),
            ScalarExpression::literal(ConsoleScalar::I64(10)),
        );
        assert!(mixed_numbers.validate(ScalarType::U64, &METADATA).is_err());
        assert!(mixed_numbers.evaluate(&row).is_err());

        let overflow = binary(
            BinaryOperator::Add,
            ScalarExpression::literal(ConsoleScalar::U64(u64::MAX)),
            ScalarExpression::literal(ConsoleScalar::U64(1)),
        );
        assert!(overflow.evaluate(&row).is_err());
    }
}
