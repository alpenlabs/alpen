//! Lazy functional pipelines over registered table scans.

use std::collections::BTreeSet;
use std::num::NonZeroUsize;

use crate::expression::compare_scalars;
use crate::read::{ConsoleOutput, ConsoleRow, RowStream, ScanPlan};
use crate::{
    ConsoleError, ConsoleRegistry, ConsoleResult, ConsoleScalar, RecordStream, ScalarExpression,
    ScalarType, ScanDirection,
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
pub struct RowSetPlan {
    scan: ScanPlan,
    filter: Option<ScalarExpression>,
    take: Option<NonZeroUsize>,
}

impl RowSetPlan {
    /// Creates a bounded forward row selection.
    pub fn scan(table: impl Into<String>, scan_limit: usize) -> ConsoleResult<Self> {
        Self::scan_direction(table, ScanDirection::Forward, scan_limit)
    }

    /// Creates a bounded reverse row selection.
    pub fn scan_rev(table: impl Into<String>, scan_limit: usize) -> ConsoleResult<Self> {
        Self::scan_direction(table, ScanDirection::Reverse, scan_limit)
    }

    fn scan_direction(
        table: impl Into<String>,
        direction: ScanDirection,
        scan_limit: usize,
    ) -> ConsoleResult<Self> {
        Ok(Self {
            scan: ScanPlan::new(table, direction, scan_limit)?,
            filter: None,
            take: None,
        })
    }

    /// Filters rows with a boolean expression.
    pub fn filter(mut self, expression: ScalarExpression) -> Self {
        self.filter = Some(expression);
        self
    }

    /// Limits matching output rows after filtering.
    pub fn take(mut self, limit: usize) -> ConsoleResult<Self> {
        self.take = Some(NonZeroUsize::new(limit).ok_or_else(|| {
            ConsoleError::invalid_input("pipeline take limit", "must be greater than zero")
        })?);
        Ok(self)
    }

    /// Returns the requested table name or alias.
    pub fn table(&self) -> &str {
        self.scan.table()
    }
}

/// A bounded functional scan assembled without textual syntax.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipelinePlan {
    rows: RowSetPlan,
    selections: Vec<Selection>,
    terminal: PipelineTerminal,
}

impl PipelinePlan {
    /// Creates a bounded forward pipeline.
    pub fn scan(table: impl Into<String>, scan_limit: usize) -> ConsoleResult<Self> {
        RowSetPlan::scan(table, scan_limit).map(Self::from)
    }

    /// Creates a bounded reverse pipeline.
    pub fn scan_rev(table: impl Into<String>, scan_limit: usize) -> ConsoleResult<Self> {
        RowSetPlan::scan_rev(table, scan_limit).map(Self::from)
    }

    /// Filters rows with a boolean expression.
    pub fn filter(mut self, expression: ScalarExpression) -> Self {
        self.rows = self.rows.filter(expression);
        self
    }

    /// Projects rows into the named scalar expressions.
    pub fn select(mut self, selections: Vec<Selection>) -> Self {
        self.selections = selections;
        self
    }

    /// Limits matching output rows after filtering.
    pub fn take(mut self, limit: usize) -> ConsoleResult<Self> {
        self.rows = self.rows.take(limit)?;
        Ok(self)
    }

    /// Selects a fixed terminal operation.
    pub fn terminal(mut self, terminal: PipelineTerminal) -> Self {
        self.terminal = terminal;
        self
    }

    /// Returns the requested table name or alias.
    pub fn table(&self) -> &str {
        self.rows.table()
    }
}

impl From<RowSetPlan> for PipelinePlan {
    fn from(rows: RowSetPlan) -> Self {
        Self {
            rows,
            selections: Vec::new(),
            terminal: PipelineTerminal::Rows,
        }
    }
}

/// Validates and executes one bounded pipeline against an explicit registry.
pub(crate) fn execute_pipeline(
    registry: &ConsoleRegistry,
    plan: PipelinePlan,
) -> ConsoleResult<ConsoleOutput> {
    let table = registry.table(plan.table())?;
    validate_plan(&plan, table.key_type(), table.value_metadata())?;

    let rows = filtered_rows(table.as_ref(), plan.rows)?;

    match plan.terminal {
        PipelineTerminal::Rows => Ok(ConsoleOutput::Rows(project_rows(rows, plan.selections))),
        PipelineTerminal::First => {
            let row = rows.into_iter().next().transpose()?;
            Ok(ConsoleOutput::Row(
                row.map(|row| project_row(row, &plan.selections))
                    .transpose()?,
            ))
        }
        PipelineTerminal::Last => {
            let mut last = None;
            for row in rows {
                last = Some(row?);
            }
            Ok(ConsoleOutput::Row(
                last.map(|row| project_row(row, &plan.selections))
                    .transpose()?,
            ))
        }
        PipelineTerminal::Count => count_rows(rows).map(ConsoleOutput::Scalar),
        PipelineTerminal::Sum(expression) => sum_rows(rows, &expression).map(ConsoleOutput::Scalar),
        PipelineTerminal::Min(expression) => {
            extreme_rows(rows, &expression, true).map(ConsoleOutput::Scalar)
        }
        PipelineTerminal::Max(expression) => {
            extreme_rows(rows, &expression, false).map(ConsoleOutput::Scalar)
        }
        PipelineTerminal::Any(expression) => {
            boolean_rows(rows, &expression, true).map(ConsoleOutput::Scalar)
        }
        PipelineTerminal::All(expression) => {
            boolean_rows(rows, &expression, false).map(ConsoleOutput::Scalar)
        }
    }
}

fn validate_plan(
    plan: &PipelinePlan,
    key_type: ScalarType,
    metadata: &crate::ValueMetadata,
) -> ConsoleResult<()> {
    validate_row_set(&plan.rows, key_type, metadata)?;

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

fn validate_row_set(
    plan: &RowSetPlan,
    key_type: ScalarType,
    metadata: &crate::ValueMetadata,
) -> ConsoleResult<()> {
    if let Some(filter) = &plan.filter {
        filter
            .validate(key_type, metadata)?
            .require(ScalarType::Bool, "pipeline filter")?;
    }
    Ok(())
}

pub(crate) fn matching_rows(
    registry: &ConsoleRegistry,
    plan: RowSetPlan,
) -> ConsoleResult<RowStream> {
    let table = registry.table(plan.table())?;
    validate_row_set(&plan, table.key_type(), table.value_metadata())?;
    Ok(Box::new(filtered_rows(table.as_ref(), plan)?))
}

fn filtered_rows(table: &dyn crate::ConsoleTable, plan: RowSetPlan) -> ConsoleResult<FilteredRows> {
    let records = table
        .scan(plan.scan.direction())?
        .take(plan.scan.limit().get());
    Ok(FilteredRows {
        records: Box::new(records),
        filter: plan.filter,
        remaining: plan.take.map(NonZeroUsize::get),
    })
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

fn project_rows(rows: FilteredRows, selections: Vec<Selection>) -> RowStream {
    Box::new(rows.map(move |row| row.and_then(|row| project_row(row, &selections))))
}

fn project_row(row: ConsoleRow, selections: &[Selection]) -> ConsoleResult<ConsoleRow> {
    let fields = if selections.is_empty() {
        row.fields
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
    Ok(ConsoleRow {
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
