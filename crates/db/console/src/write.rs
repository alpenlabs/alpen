//! Staging and committing one typed point or bounded-batch write.

use std::fmt;

use serde::Serialize;

use crate::pipeline::matching_rows;
use crate::{ConsoleError, ConsoleRegistry, ConsoleResult, ConsoleRow, ConsoleScalar, RowSetPlan};

/// One record change in a staged write.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WriteChange {
    /// Record before the operation.
    pub before: ConsoleRow,
    /// Record after the operation.
    pub after: ConsoleRow,
}

/// Human- and machine-readable view of one staged write.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WritePreview {
    /// Short description of the setter or modifier being applied.
    pub operation: String,
    /// Ordered record changes prepared by the operation.
    pub changes: Vec<WriteChange>,
}

/// Type-erased write prepared by a concrete table registration.
pub trait StagedWrite: Send + Sync {
    /// Returns the stable preview generated when the write was staged.
    fn preview(&self) -> &WritePreview;

    /// Atomically applies the replacement if the stored value is unchanged.
    fn commit(&self) -> ConsoleResult<()>;
}

/// A parsed setter or modifier that can be staged against a completed registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WritePlan {
    /// Sets one approved field through its domain setter.
    Set {
        /// Registered table name or alias.
        table: String,
        /// Scalar table key.
        key: ConsoleScalar,
        /// Approved field name.
        field: String,
        /// Replacement scalar passed to the domain setter.
        value: ConsoleScalar,
    },
    /// Applies one approved modifier to a point record.
    Modify {
        /// Registered table name or alias.
        table: String,
        /// Scalar table key.
        key: ConsoleScalar,
        /// Approved modifier name.
        modifier: String,
        /// Ordered modifier arguments.
        arguments: Vec<ConsoleScalar>,
    },
    /// Applies one approved modifier to a bounded row selection.
    ModifyRows {
        /// Bounded scan, filter, and optional output limit.
        rows: RowSetPlan,
        /// Approved modifier name.
        modifier: String,
        /// Ordered modifier arguments.
        arguments: Vec<ConsoleScalar>,
    },
}

impl WritePlan {
    /// Stages this plan without applying it to storage.
    pub fn stage<'session>(
        self,
        session: &'session mut WriteSession<'_>,
    ) -> ConsoleResult<&'session WritePreview> {
        match self {
            Self::Set {
                table,
                key,
                field,
                value,
            } => session.stage_set(&table, &key, &field, &value),
            Self::Modify {
                table,
                key,
                modifier,
                arguments,
            } => session.stage_modify(&table, &key, &modifier, &arguments),
            Self::ModifyRows {
                rows,
                modifier,
                arguments,
            } => session.stage_modify_scan(rows, &modifier, &arguments),
        }
    }
}

/// Holds at most one staged point or bounded-batch write against an explicit registry.
pub struct WriteSession<'a> {
    registry: &'a ConsoleRegistry,
    staged: Option<Box<dyn StagedWrite>>,
}

impl<'a> WriteSession<'a> {
    /// Creates an empty write session.
    pub const fn new(registry: &'a ConsoleRegistry) -> Self {
        Self {
            registry,
            staged: None,
        }
    }

    /// Stages a domain setter against one existing record.
    pub fn stage_set(
        &mut self,
        table: &str,
        key: &ConsoleScalar,
        field: &str,
        value: &ConsoleScalar,
    ) -> ConsoleResult<&WritePreview> {
        self.ensure_empty()?;
        let write = self.registry.table(table)?.stage_set(key, field, value)?;
        Ok(self.store(write))
    }

    /// Stages a broader table modifier against one existing record.
    pub fn stage_modify(
        &mut self,
        table: &str,
        key: &ConsoleScalar,
        modifier: &str,
        arguments: &[ConsoleScalar],
    ) -> ConsoleResult<&WritePreview> {
        self.ensure_empty()?;
        let write = self
            .registry
            .table(table)?
            .stage_modify(key, modifier, arguments)?;
        Ok(self.store(write))
    }

    /// Stages one modifier for every row produced by a bounded row selection.
    pub fn stage_modify_scan(
        &mut self,
        plan: RowSetPlan,
        modifier: &str,
        arguments: &[ConsoleScalar],
    ) -> ConsoleResult<&WritePreview> {
        self.ensure_empty()?;
        let table = plan.table().to_owned();
        let keys = matching_rows(self.registry, plan)?
            .map(|row| row.map(|row| row.key))
            .collect::<ConsoleResult<Vec<_>>>()?;
        if keys.is_empty() {
            return Err(ConsoleError::NoMatchingRecords { table });
        }
        let write = self
            .registry
            .table(&table)?
            .stage_modify_many(&keys, modifier, arguments)?;
        Ok(self.store(write))
    }

    /// Returns the current staged preview.
    pub fn staged(&self) -> Option<&WritePreview> {
        self.staged.as_ref().map(|write| write.preview())
    }

    /// Discards the staged write and returns its preview.
    pub fn abort(&mut self) -> Option<WritePreview> {
        self.staged.take().map(|write| write.preview().clone())
    }

    /// Commits the staged write and clears it after a successful atomic storage operation.
    pub fn commit(&mut self) -> ConsoleResult<WritePreview> {
        let write = self
            .staged
            .as_ref()
            .ok_or(crate::ConsoleError::NoStagedWrite)?;
        write.commit()?;
        let write = self
            .staged
            .take()
            .expect("successful commit retains the staged write until now");
        Ok(write.preview().clone())
    }

    fn ensure_empty(&self) -> ConsoleResult<()> {
        if self.staged.is_some() {
            Err(crate::ConsoleError::WriteAlreadyStaged)
        } else {
            Ok(())
        }
    }

    fn store(&mut self, write: Box<dyn StagedWrite>) -> &WritePreview {
        self.staged.insert(write).preview()
    }
}

impl fmt::Debug for WriteSession<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WriteSession")
            .field("staged", &self.staged())
            .finish_non_exhaustive()
    }
}
