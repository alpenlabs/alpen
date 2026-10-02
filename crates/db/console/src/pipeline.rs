//! Lazy functional pipelines over registered table scans.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::num::NonZeroUsize;

use serde::Serialize;

use crate::expression::compare_scalars;
use crate::{
    ConsoleError, ConsoleRegistry, ConsoleResult, ConsoleRow, ConsoleScalar, RecordStream,
    ScalarExpression, ScalarType, ScanDirection,
};

/// One named expression in a row projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    /// Stable output column name.
    pub name: String,
    /// Expression evaluated for the column.
    pub expression: ScalarExpression,
}

impl Selection {
    /// Creates a named projected column.
    pub fn new(name: impl Into<String>, expression: ScalarExpression) -> Self {
        Self {
            name: name.into(),
            expression,
        }
    }
}

/// A projected row with runtime-selected column names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProjectedRow {
    /// Primary table name.
    pub source: &'static str,
    /// Typed key rendered as a scalar.
    pub key: ConsoleScalar,
    /// Selected scalar columns.
    pub fields: BTreeMap<String, ConsoleScalar>,
}

/// Fixed terminal operation for a pipeline.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PipelineTerminal {
    /// Return rows lazily.
    #[default]
    Rows,
    /// Return the first matching row.
    First,
    /// Return the last matching row within the scan bound.
    Last,
    /// Count matching rows.
    Count,
    /// Sum a numeric expression.
    Sum(ScalarExpression),
    /// Return the minimum expression value.
    Min(ScalarExpression),
    /// Return the maximum expression value.
    Max(ScalarExpression),
    /// Return whether any row satisfies a boolean expression.
    Any(ScalarExpression),
    /// Return whether all rows satisfy a boolean expression.
    All(ScalarExpression),
}

/// A bounded functional scan assembled without textual syntax.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipelinePlan {
    table: String,
    direction: ScanDirection,
    scan_limit: NonZeroUsize,
    filter: Option<ScalarExpression>,
    selections: Vec<Selection>,
    take: Option<NonZeroUsize>,
    terminal: PipelineTerminal,
}

impl PipelinePlan {
    /// Creates a bounded forward pipeline.
    pub fn scan(table: impl Into<String>, scan_limit: usize) -> ConsoleResult<Self> {
        Self::scan_direction(table, ScanDirection::Forward, scan_limit)
    }

    /// Creates a bounded reverse pipeline.
    pub fn scan_rev(table: impl Into<String>, scan_limit: usize) -> ConsoleResult<Self> {
        Self::scan_direction(table, ScanDirection::Reverse, scan_limit)
    }

    fn scan_direction(
        table: impl Into<String>,
        direction: ScanDirection,
        scan_limit: usize,
    ) -> ConsoleResult<Self> {
        let scan_limit = NonZeroUsize::new(scan_limit).ok_or_else(|| {
            ConsoleError::invalid_input("pipeline scan limit", "must be greater than zero")
        })?;
        Ok(Self {
            table: table.into(),
            direction,
            scan_limit,
            filter: None,
            selections: Vec::new(),
            take: None,
            terminal: PipelineTerminal::Rows,
        })
    }

    /// Filters rows with a boolean expression.
    pub fn filter(mut self, expression: ScalarExpression) -> Self {
        self.filter = Some(expression);
        self
    }

    /// Projects rows into the named scalar expressions.
    pub fn select(mut self, selections: Vec<Selection>) -> Self {
        self.selections = selections;
        self
    }

    /// Limits matching output rows after filtering.
    pub fn take(mut self, limit: usize) -> ConsoleResult<Self> {
        self.take = Some(NonZeroUsize::new(limit).ok_or_else(|| {
            ConsoleError::invalid_input("pipeline take limit", "must be greater than zero")
        })?);
        Ok(self)
    }

    /// Selects a fixed terminal operation.
    pub fn terminal(mut self, terminal: PipelineTerminal) -> Self {
        self.terminal = terminal;
        self
    }

    /// Returns the requested table name or alias.
    pub fn table(&self) -> &str {
        &self.table
    }

    pub(crate) fn returns_rows(&self) -> bool {
        matches!(self.terminal, PipelineTerminal::Rows)
    }
}

/// Lazy rows returned from a functional pipeline.
pub type PipelineRowStream = Box<dyn Iterator<Item = ConsoleResult<ProjectedRow>>>;

/// Result of one functional pipeline.
pub enum PipelineOutput {
    /// A lazy stream of projected rows.
    Rows(PipelineRowStream),
    /// The first or last matching row.
    Row(Option<ProjectedRow>),
    /// A fixed aggregate.
    Scalar(ConsoleScalar),
}

impl fmt::Debug for PipelineOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rows(_) => formatter.write_str("Rows(..)"),
            Self::Row(row) => formatter.debug_tuple("Row").field(row).finish(),
            Self::Scalar(value) => formatter.debug_tuple("Scalar").field(value).finish(),
        }
    }
}

/// Executes validated functional plans against an explicit registry.
#[derive(Debug)]
pub struct PipelineExecutor<'a> {
    registry: &'a ConsoleRegistry,
}

impl<'a> PipelineExecutor<'a> {
    /// Creates an executor over a completed registry.
    pub const fn new(registry: &'a ConsoleRegistry) -> Self {
        Self { registry }
    }

    /// Validates and executes one bounded pipeline.
    pub fn execute(&self, plan: PipelinePlan) -> ConsoleResult<PipelineOutput> {
        let table = self.registry.table(plan.table())?;
        validate_plan(&plan, table.key_type(), table.value_metadata())?;

        let records = table.scan(plan.direction)?.take(plan.scan_limit.get());
        let rows = FilteredRows {
            records: Box::new(records),
            filter: plan.filter,
            remaining: plan.take.map(NonZeroUsize::get),
        };

        match plan.terminal {
            PipelineTerminal::Rows => Ok(PipelineOutput::Rows(project_rows(rows, plan.selections))),
            PipelineTerminal::First => {
                let row = rows.into_iter().next().transpose()?;
                Ok(PipelineOutput::Row(
                    row.map(|row| project_row(row, &plan.selections))
                        .transpose()?,
                ))
            }
            PipelineTerminal::Last => {
                let mut last = None;
                for row in rows {
                    last = Some(row?);
                }
                Ok(PipelineOutput::Row(
                    last.map(|row| project_row(row, &plan.selections))
                        .transpose()?,
                ))
            }
            PipelineTerminal::Count => count_rows(rows).map(PipelineOutput::Scalar),
            PipelineTerminal::Sum(expression) => {
                sum_rows(rows, &expression).map(PipelineOutput::Scalar)
            }
            PipelineTerminal::Min(expression) => {
                extreme_rows(rows, &expression, true).map(PipelineOutput::Scalar)
            }
            PipelineTerminal::Max(expression) => {
                extreme_rows(rows, &expression, false).map(PipelineOutput::Scalar)
            }
            PipelineTerminal::Any(expression) => {
                boolean_rows(rows, &expression, true).map(PipelineOutput::Scalar)
            }
            PipelineTerminal::All(expression) => {
                boolean_rows(rows, &expression, false).map(PipelineOutput::Scalar)
            }
        }
    }
}

fn validate_plan(
    plan: &PipelinePlan,
    key_type: ScalarType,
    metadata: &crate::ValueMetadata,
) -> ConsoleResult<()> {
    if let Some(filter) = &plan.filter {
        filter
            .validate(key_type, metadata)?
            .require(ScalarType::Bool, "pipeline filter")?;
    }

    let mut names = BTreeSet::new();
    for selection in &plan.selections {
        if selection.name.is_empty() {
            return Err(ConsoleError::invalid_input(
                "pipeline selection",
                "column names must not be empty",
            ));
        }
        if !names.insert(selection.name.as_str()) {
            return Err(ConsoleError::invalid_input(
                "pipeline selection",
                format!("duplicate column name '{}'", selection.name),
            ));
        }
        selection.expression.validate(key_type, metadata)?;
    }

    match &plan.terminal {
        PipelineTerminal::Sum(expression) => {
            let expression_type = expression.validate(key_type, metadata)?;
            if !matches!(
                expression_type.scalar_type(),
                Some(ScalarType::I64 | ScalarType::U64)
            ) {
                return Err(ConsoleError::invalid_input(
                    "sum",
                    "expected an i64 or u64 expression",
                ));
            }
        }
        PipelineTerminal::Min(expression) | PipelineTerminal::Max(expression) => {
            let expression_type = expression.validate(key_type, metadata)?;
            if matches!(expression_type.scalar_type(), None | Some(ScalarType::Bool)) {
                return Err(ConsoleError::invalid_input(
                    "minimum or maximum",
                    "expected an ordered scalar expression",
                ));
            }
        }
        PipelineTerminal::Any(expression) | PipelineTerminal::All(expression) => {
            expression
                .validate(key_type, metadata)?
                .require(ScalarType::Bool, "boolean aggregate")?;
        }
        PipelineTerminal::Rows
        | PipelineTerminal::First
        | PipelineTerminal::Last
        | PipelineTerminal::Count => {}
    }
    Ok(())
}

struct FilteredRows {
    records: RecordStream,
    filter: Option<ScalarExpression>,
    remaining: Option<usize>,
}

impl Iterator for FilteredRows {
    type Item = ConsoleResult<ConsoleRow>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == Some(0) {
            return None;
        }
        loop {
            let row = match self.records.next()? {
                Ok(record) => match ConsoleRow::from_handle(record) {
                    Ok(row) => row,
                    Err(error) => return Some(Err(error)),
                },
                Err(error) => return Some(Err(error)),
            };
            let included = match &self.filter {
                Some(filter) => match filter.evaluate(&row) {
                    Ok(value) => match value.as_bool("pipeline filter") {
                        Ok(value) => value,
                        Err(error) => return Some(Err(error)),
                    },
                    Err(error) => return Some(Err(error)),
                },
                None => true,
            };
            if included {
                if let Some(remaining) = &mut self.remaining {
                    *remaining -= 1;
                }
                return Some(Ok(row));
            }
        }
    }
}

fn project_rows(rows: FilteredRows, selections: Vec<Selection>) -> PipelineRowStream {
    Box::new(rows.map(move |row| row.and_then(|row| project_row(row, &selections))))
}

fn project_row(row: ConsoleRow, selections: &[Selection]) -> ConsoleResult<ProjectedRow> {
    let fields = if selections.is_empty() {
        row.fields
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect()
    } else {
        selections
            .iter()
            .map(|selection| {
                selection
                    .expression
                    .evaluate(&row)
                    .map(|value| (selection.name.clone(), value))
            })
            .collect::<ConsoleResult<_>>()?
    };
    Ok(ProjectedRow {
        source: row.source,
        key: row.key,
        fields,
    })
}

fn count_rows(rows: FilteredRows) -> ConsoleResult<ConsoleScalar> {
    let mut count = 0_u64;
    for row in rows {
        row?;
        count = count
            .checked_add(1)
            .ok_or_else(|| ConsoleError::invalid_input("count", "row count overflowed u64"))?;
    }
    Ok(ConsoleScalar::U64(count))
}

fn sum_rows(rows: FilteredRows, expression: &ScalarExpression) -> ConsoleResult<ConsoleScalar> {
    let mut sum = None;
    for row in rows {
        let value = expression.evaluate(&row?)?;
        sum = Some(match (sum, value) {
            (None, value @ (ConsoleScalar::I64(_) | ConsoleScalar::U64(_))) => value,
            (Some(ConsoleScalar::I64(sum)), ConsoleScalar::I64(value)) => {
                ConsoleScalar::I64(sum.checked_add(value).ok_or_else(|| {
                    ConsoleError::invalid_input("sum", "signed sum overflowed i64")
                })?)
            }
            (Some(ConsoleScalar::U64(sum)), ConsoleScalar::U64(value)) => {
                ConsoleScalar::U64(sum.checked_add(value).ok_or_else(|| {
                    ConsoleError::invalid_input("sum", "unsigned sum overflowed u64")
                })?)
            }
            _ => {
                return Err(ConsoleError::invalid_input(
                    "sum",
                    "encountered a non-numeric or inconsistent value",
                ));
            }
        });
    }
    Ok(sum.unwrap_or(ConsoleScalar::Null))
}

fn extreme_rows(
    rows: FilteredRows,
    expression: &ScalarExpression,
    minimum: bool,
) -> ConsoleResult<ConsoleScalar> {
    let mut extreme = None;
    for row in rows {
        let candidate = expression.evaluate(&row?)?;
        extreme = Some(match extreme {
            None => candidate,
            Some(current) => {
                let ordering = compare_scalars(&candidate, &current)?;
                if (minimum && ordering.is_lt()) || (!minimum && ordering.is_gt()) {
                    candidate
                } else {
                    current
                }
            }
        });
    }
    Ok(extreme.unwrap_or(ConsoleScalar::Null))
}

fn boolean_rows(
    rows: FilteredRows,
    expression: &ScalarExpression,
    any: bool,
) -> ConsoleResult<ConsoleScalar> {
    for row in rows {
        let value = expression.evaluate(&row?)?.as_bool("boolean aggregate")?;
        if value == any {
            return Ok(ConsoleScalar::Bool(any));
        }
    }
    Ok(ConsoleScalar::Bool(!any))
}
