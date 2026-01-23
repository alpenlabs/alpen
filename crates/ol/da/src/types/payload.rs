//! Top-level DA payload types.

use std::marker::PhantomData;

use strata_codec::Codec;
use strata_da_framework::{DaError, DaWrite};
use strata_ledger_types::IStateAccessor;

use super::{global::GlobalStateDiff, ledger::LedgerDiff};

/// Versioned OL DA payload containing the state diff.
#[derive(Debug, Codec)]
pub struct OLDaPayloadV1 {
    /// State diff for the epoch.
    pub state_diff: StateDiff,
}

impl OLDaPayloadV1 {
    /// Creates a new [`OLDaPayloadV1`] from a state diff.
    pub fn new(state_diff: StateDiff) -> Self {
        Self { state_diff }
    }
}

/// Preseal OL state diff (global + ledger).
#[derive(Debug, Default, Codec)]
pub struct StateDiff {
    /// Global state diff.
    pub global: GlobalStateDiff,

    /// Ledger state diff.
    pub ledger: LedgerDiff,
}

impl StateDiff {
    /// Creates a new [`StateDiff`] from a global state diff and ledger diff.
    pub fn new(global: GlobalStateDiff, ledger: LedgerDiff) -> Self {
        Self { global, ledger }
    }
}

/// Adapter for applying a state diff to a concrete state accessor.
#[derive(Debug)]
pub struct OLStateDiff<S: IStateAccessor> {
    diff: StateDiff,
    _target: PhantomData<S>,
}

impl<S: IStateAccessor> OLStateDiff<S> {
    pub fn new(diff: StateDiff) -> Self {
        Self {
            diff,
            _target: PhantomData,
        }
    }

    pub fn as_inner(&self) -> &StateDiff {
        &self.diff
    }

    pub fn into_inner(self) -> StateDiff {
        self.diff
    }
}

impl<S: IStateAccessor> Default for OLStateDiff<S> {
    fn default() -> Self {
        Self::new(StateDiff::default())
    }
}

impl<S: IStateAccessor> From<StateDiff> for OLStateDiff<S> {
    fn from(diff: StateDiff) -> Self {
        Self::new(diff)
    }
}

impl<S: IStateAccessor> From<OLStateDiff<S>> for StateDiff {
    fn from(diff: OLStateDiff<S>) -> Self {
        diff.diff
    }
}

impl<S: IStateAccessor> DaWrite for OLStateDiff<S> {
    type Target = S;
    type Context = ();

    fn is_default(&self) -> bool {
        DaWrite::is_default(&self.diff.global) && self.diff.ledger.is_empty()
    }

    fn poll_context(
        &self,
        _target: &Self::Target,
        _context: &Self::Context,
    ) -> Result<(), DaError> {
        if !self.diff.ledger.is_empty() {
            return Err(DaError::InsufficientContext);
        }
        Ok(())
    }

    fn apply(&self, target: &mut Self::Target, _context: &Self::Context) -> Result<(), DaError> {
        if !self.diff.ledger.is_empty() {
            return Err(DaError::InsufficientContext);
        }

        let mut cur_slot = target.cur_slot();
        self.diff.global.cur_slot.apply(&mut cur_slot, &())?;
        target.set_cur_slot(cur_slot);
        Ok(())
    }
}
