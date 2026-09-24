//! Chainstate held in one of the supported layouts.

use ssz::{Decode, DecodeError, Encode};
use strata_identifiers::{Buf32, Epoch, EpochCommitment, L1BlockCommitment, Slot};
use strata_ol_state_types_v1::OLStateV1;

use crate::layout::OLStateLayout;

/// Read access that every chainstate layout provides.
///
/// The chainstate type of each layout implements this trait, and
/// [`dispatch_chainstate!`] forwards the [`OLStateSeries`] accessors to the
/// implementation for the variant held. A new layout adds an implementation,
/// which the compiler requires to provide every accessor, and one macro arm.
pub(crate) trait IChainstate: Encode {
    /// Computes the hash tree root of the chainstate in its own layout.
    fn compute_chainstate_root(&self) -> Buf32;

    /// Returns the current slot.
    fn cur_slot(&self) -> Slot;

    /// Returns the current epoch.
    fn cur_epoch(&self) -> Epoch;

    /// Returns the last L1 block the chainstate has accepted.
    fn last_l1_block(&self) -> L1BlockCommitment;

    /// Returns the epoch the ASM considers recorded, as of the last accepted
    /// ASM manifest.
    fn asm_recorded_epoch(&self) -> &EpochCommitment;
}

impl IChainstate for OLStateV1 {
    fn compute_chainstate_root(&self) -> Buf32 {
        OLStateV1::compute_chainstate_root(self)
    }

    fn cur_slot(&self) -> Slot {
        self.global_state().get_cur_slot()
    }

    fn cur_epoch(&self) -> Epoch {
        self.epoch_state().cur_epoch()
    }

    fn last_l1_block(&self) -> L1BlockCommitment {
        let epoch_state = self.epoch_state();
        L1BlockCommitment::new(epoch_state.last_l1_height(), *epoch_state.last_l1_blkid())
    }

    fn asm_recorded_epoch(&self) -> &EpochCommitment {
        self.epoch_state().asm_recorded_epoch()
    }
}

/// Evaluates `$body` with `$state` bound to the chainstate `$series` holds.
///
/// Each arm binds a different [`IChainstate`] implementation, so `$body` may
/// use that trait and its [`Encode`] supertrait.
macro_rules! dispatch_chainstate {
    ($series:expr, $state:ident => $body:expr) => {
        match $series {
            OLStateSeries::V1($state) => $body,
        }
    };
}

/// Chainstate in one of the layouts this binary supports.
///
/// This is an in-memory sum type only. It has no SSZ union encoding, and its
/// variant is never hashed: the root's current spec selects the layout.
///
/// The read accessors cover the fields every layout keeps, for callers that
/// inspect a stored state without executing it. Callers that need
/// layout-specific data, such as ledger accounts, match on the variant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OLStateSeries {
    /// Chainstate in the [`OLStateLayout::V1`] layout.
    V1(OLStateV1),
}

impl OLStateSeries {
    /// Returns the layout of this chainstate.
    pub fn layout(&self) -> OLStateLayout {
        match self {
            Self::V1(_) => OLStateLayout::V1,
        }
    }

    /// Computes the hash tree root of the chainstate in its own layout.
    pub fn compute_chainstate_root(&self) -> Buf32 {
        dispatch_chainstate!(self, state => IChainstate::compute_chainstate_root(state))
    }

    /// Returns the current slot.
    // TODO(STR-2863): remove once deblockification drops the slot from the
    // state.
    pub fn cur_slot(&self) -> Slot {
        dispatch_chainstate!(self, state => IChainstate::cur_slot(state))
    }

    /// Returns the current epoch.
    pub fn cur_epoch(&self) -> Epoch {
        dispatch_chainstate!(self, state => IChainstate::cur_epoch(state))
    }

    /// Returns the last L1 block the chainstate has accepted.
    pub fn last_l1_block(&self) -> L1BlockCommitment {
        dispatch_chainstate!(self, state => IChainstate::last_l1_block(state))
    }

    /// Returns the epoch the ASM considers recorded, as of the last accepted
    /// ASM manifest.
    pub fn asm_recorded_epoch(&self) -> &EpochCommitment {
        dispatch_chainstate!(self, state => IChainstate::asm_recorded_epoch(state))
    }

    /// Returns the length of the chainstate's SSZ encoding.
    pub(crate) fn ssz_bytes_len(&self) -> usize {
        dispatch_chainstate!(self, state => Encode::ssz_bytes_len(state))
    }

    /// Encodes the chainstate as SSZ, without any layout selector.
    pub(crate) fn to_ssz_bytes(&self) -> Vec<u8> {
        dispatch_chainstate!(self, state => Encode::as_ssz_bytes(state))
    }

    /// Decodes a chainstate in `layout` from its SSZ encoding.
    pub(crate) fn from_ssz_bytes(layout: OLStateLayout, bytes: &[u8]) -> Result<Self, DecodeError> {
        match layout {
            OLStateLayout::V1 => OLStateV1::from_ssz_bytes(bytes).map(Self::V1),
        }
    }
}

impl From<OLStateV1> for OLStateSeries {
    fn from(state: OLStateV1) -> Self {
        Self::V1(state)
    }
}
