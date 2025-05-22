// use std::ops::Deref;

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_primitives::{
    buf::Buf32, epoch::EpochCommitment, hash::compute_borsh_hash, l2::L2BlockCommitment,
};

use crate::{
    bridge_ops,
    bridge_state::{self, DepositsTable, OperatorTable},
    exec_env::{self, ExecEnvState},
    genesis::GenesisStateData,
    l1::{self, L1ViewState},
    prelude::*,
    traits::ChainstateUpdate,
};

/// L2 blockchain state.  This is the state computed as a function of a
/// pre-state and a block.
///
/// This corresponds to the beacon chain state.
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct Chainstate {
    /// The slot that contained the block that produced this chainstate.
    pub(crate) cur_slot: u64,

    /// The parent of the block that produced this chainstate.
    pub(crate) prev_block: L2BlockCommitment,

    /// The checkpoint epoch period we're currently in, and so the index we
    /// expect the next checkpoint to be for.
    ///
    /// Immediately after genesis, this is 0, so the first checkpoint batch is
    /// checkpoint 0, moving us into checkpoint period 1.
    pub(crate) cur_epoch: u64,

    /// The immediately preceding epoch.
    ///
    /// This *should* be updated in the first block of the new epoch.
    pub(crate) prev_epoch: EpochCommitment,

    /// Flag set while processing the last block of an epoch to ensure that the
    /// next block is the first block of the next epoch.
    ///
    /// This is a temporary flag.
    pub(crate) is_epoch_finishing: bool,

    /// The epoch that we have observed in a checkpoint in L1.
    pub(crate) finalized_epoch: EpochCommitment,

    /// Rollup's view of L1 state.
    pub(crate) l1_state: l1::L1ViewState,

    /// Pending withdrawals that have been initiated but haven't been sent out.
    pub(crate) pending_withdraws: StateQueue<bridge_ops::WithdrawalIntent>,

    /// Execution environment state.  This is just for the single EE we support
    /// right now.
    pub(crate) exec_env_state: exec_env::ExecEnvState,

    /// Operator table we store registered operators for.
    pub(crate) operator_table: bridge_state::OperatorTable,

    /// Deposits table tracking each deposit's state.
    pub(crate) deposits_table: bridge_state::DepositsTable,
}

impl Chainstate {
    // TODO remove genesis blkid since apparently we don't need it anymore
    pub fn from_genesis(gdata: &GenesisStateData) -> Self {
        Self {
            cur_slot: 0,
            prev_block: L2BlockCommitment::new(u64::MAX, L2BlockId::null()),
            cur_epoch: 0,
            prev_epoch: EpochCommitment::null(),
            finalized_epoch: EpochCommitment::null(),
            is_epoch_finishing: false,
            l1_state: gdata.l1_state().clone(),
            pending_withdraws: StateQueue::new_empty(),
            exec_env_state: gdata.exec_state().clone(),
            operator_table: gdata.operator_table().clone(),
            deposits_table: bridge_state::DepositsTable::new_empty(),
        }
    }

    /// Returns the slot last processed on the chainstate.
    pub fn chain_tip_slot(&self) -> u64 {
        self.cur_slot
    }

    /// Returns the commitment to the previous block.
    pub fn prev_block(&self) -> &L2BlockCommitment {
        &self.prev_block
    }

    pub fn l1_view(&self) -> &L1ViewState {
        &self.l1_state
    }

    pub fn cur_epoch(&self) -> u64 {
        self.cur_epoch
    }

    /// Gets the commitment to the immediately preceding epoch.
    pub fn prev_epoch(&self) -> &EpochCommitment {
        &self.prev_epoch
    }

    /// Gets the commitment to the finalized epoch, which we don't expect to
    /// roll back.
    pub fn finalized_epoch(&self) -> &EpochCommitment {
        &self.finalized_epoch
    }

    /// Computes a commitment to a the chainstate.  This is super expensive
    /// because it does a bunch of hashing.
    pub fn compute_state_root(&self) -> Buf32 {
        // FIXME this is all broken because we're doing this badly, the real
        // solution is to use SSZ for all of this
        let hashed_state = HashedChainState {
            prev_block: compute_borsh_hash(&self.prev_block),
            cur_epoch: self.cur_epoch,
            prev_epoch: compute_borsh_hash(&self.prev_epoch),
            l1_state_hash: compute_borsh_hash(&self.l1_state),
            pending_withdraws_hash: compute_borsh_hash(&self.pending_withdraws),
            exec_env_hash: compute_borsh_hash(&self.exec_env_state),
            operators_hash: compute_borsh_hash(&self.operator_table),
            deposits_hash: compute_borsh_hash(&self.deposits_table),
        };
        compute_borsh_hash(&hashed_state)
    }

    pub fn operator_table(&self) -> &OperatorTable {
        &self.operator_table
    }

    pub fn deposits_table(&self) -> &DepositsTable {
        &self.deposits_table
    }

    pub fn deposits_table_mut(&mut self) -> &mut DepositsTable {
        &mut self.deposits_table
    }

    pub fn exec_env_state(&self) -> &ExecEnvState {
        &self.exec_env_state
    }

    pub fn is_epoch_finishing(&self) -> bool {
        self.is_epoch_finishing
    }
}

/// Hashed Chain State. This is used to compute the state root of the [`Chainstate`]
// TODO: FIXME: Note that this is used as a temporary solution for the state root calculation
// It should be replaced once we swap out Chainstate's type definitions with SSZ type definitions
// which defines all of this more rigorously
#[derive(BorshSerialize, BorshDeserialize, Clone, Copy)]
pub struct HashedChainState {
    pub prev_block: Buf32,
    pub cur_epoch: u64,
    pub prev_epoch: Buf32,
    pub l1_state_hash: Buf32,
    pub pending_withdraws_hash: Buf32,
    pub exec_env_hash: Buf32,
    pub operators_hash: Buf32,
    pub deposits_hash: Buf32,
}

// NOTE: This is a helper setter that is supposed to be used only in tests.
// This is being used in `strata_btcio::reader` to test the reader's behaviour when the epoch
// changes.
#[cfg(any(test, feature = "test_utils"))]
impl Chainstate {
    pub fn set_epoch(&mut self, ep: u64) {
        self.cur_epoch = ep;
    }
}

impl<'a> Arbitrary<'a> for Chainstate {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let gdata = GenesisStateData::arbitrary(u)?;
        Ok(Self::from_genesis(&gdata))
    }
}

pub struct ChainstateEntry {
    state: Chainstate,
    tip: L2BlockId,
}

impl ChainstateEntry {
    pub fn new(state: Chainstate, tip: L2BlockId) -> Self {
        Self { state, tip }
    }

    pub fn to_parts(self) -> (Chainstate, L2BlockId) {
        (self.state, self.tip)
    }

    pub fn to_chainstate(self) -> Chainstate {
        self.state
    }

    pub fn state(&self) -> &Chainstate {
        &self.state
    }

    pub fn tip_blockid(&self) -> &L2BlockId {
        &self.tip
    }
}

impl From<ChainstateEntry> for Chainstate {
    fn from(value: ChainstateEntry) -> Self {
        value.to_chainstate()
    }
}

pub struct ChainstateUpdateImpl {
    new_chainstate: Chainstate,
}

impl ChainstateUpdateImpl {
    pub fn new(new_chainstate: Chainstate) -> Self {
        ChainstateUpdateImpl { new_chainstate }
    }
}

impl ChainstateUpdate for ChainstateUpdateImpl {
    #[allow(unused)]
    fn apply_to_chainstate(&self, chainstate: Option<&Chainstate>) -> Chainstate {
        self.new_chainstate.clone()
    }
}
