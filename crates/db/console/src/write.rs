//! Staging and committing one typed point write.

use std::fmt;

use serde::Serialize;

use crate::{ConsoleRegistry, ConsoleResult, ConsoleRow, ConsoleScalar};

/// Human- and machine-readable view of one staged point write.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WritePreview {
    /// Short description of the setter or modifier being applied.
    pub operation: String,
    /// Record before the operation.
    pub before: ConsoleRow,
    /// Record after the operation.
    pub after: ConsoleRow,
}

/// Type-erased point write prepared by a concrete table registration.
pub trait StagedWrite: Send + Sync {
    /// Returns the stable preview generated when the write was staged.
    fn preview(&self) -> &WritePreview;

    /// Atomically applies the replacement if the stored value is unchanged.
    fn commit(&self) -> ConsoleResult<()>;
}

/// Holds at most one staged point write against an explicit registry.
pub struct WriteSession<'a> {
    registry: &'a ConsoleRegistry,
    staged: Option<Box<dyn StagedWrite>>,
}

impl<'a> WriteSession<'a> {
    /// Creates an empty point-write session.
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
        self.staged = Some(self.registry.table(table)?.stage_set(key, field, value)?);
        Ok(self.staged().expect("staged write was just inserted"))
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
        self.staged = Some(
            self.registry
                .table(table)?
                .stage_modify(key, modifier, arguments)?,
        );
        Ok(self.staged().expect("staged write was just inserted"))
    }

    /// Returns the current staged preview.
    pub fn staged(&self) -> Option<&WritePreview> {
        self.staged.as_ref().map(|write| write.preview())
    }

    /// Discards the staged write and returns its preview.
    pub fn abort(&mut self) -> Option<WritePreview> {
        self.staged.take().map(|write| write.preview().clone())
    }

    /// Commits the staged write and clears it after a successful compare-and-swap.
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
}

impl fmt::Debug for WriteSession<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WriteSession")
            .field("staged", &self.staged())
            .finish_non_exhaustive()
    }
}
