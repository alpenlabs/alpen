use std::sync::{Arc, Mutex};
use anyhow::Result;
use serde::{Deserialize, Serialize};

use stf_runner::{
    account::{AccountId, AccountState, AccountInnerState, SnarkAccountState, SnarkAccountProofState},
    block::{OLBlock, OLBlockHeader},
    ledger::{InMemoryVectorLedger, LedgerProvider},
    state::{OLState, L1View},
    stf::{process_block, StfError},
};
use strata_chaintsn::context::StateAccessor;
use strata_primitives::{buf::Buf32, params::RollupParams};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateInfo {
    pub accounts_root: Buf32,
    pub cur_slot: u64,
    pub cur_epoch: u64,
    pub l1_block_height: u64,
    pub account_count: usize,
}

pub struct SimpleStateAccessor {
    state: OLState,
}

impl SimpleStateAccessor {
    pub fn new(state: OLState) -> Self {
        Self { state }
    }
}

impl StateAccessor<OLState> for SimpleStateAccessor {
    fn state_untracked(&self) -> &OLState {
        &self.state
    }

    fn state_mut_untracked(&mut self) -> &mut OLState {
        &mut self.state
    }

    fn slot(&self) -> u64 {
        self.state.cur_slot()
    }

    fn set_slot(&mut self, slot: u64) {
        self.state.set_cur_slot(slot);
    }

    fn cur_epoch(&self) -> u64 {
        self.state.cur_epoch()
    }

    fn set_cur_epoch(&mut self, epoch: u64) {
        self.state.set_cur_epoch(epoch);
    }

    fn get_toplevel_state(&mut self) -> &OLState {
        &self.state
    }

    fn set_accounts_root(&mut self, root: Buf32) {
        self.state.set_accounts_root(root);
    }

    // Placeholder implementations for required methods
    fn prev_block(&self) -> strata_chaintsn::context::L2BlockCommitment {
        strata_chaintsn::context::L2BlockCommitment::new(strata_primitives::buf::Buf32::zero(), 0)
    }

    fn set_prev_block(&mut self, _block: strata_chaintsn::context::L2BlockCommitment) {}

    fn prev_epoch(&self) -> strata_chaintsn::context::EpochCommitment {
        strata_chaintsn::context::EpochCommitment::new(strata_primitives::buf::Buf32::zero(), 0)
    }

    fn set_prev_epoch(&mut self, _epoch: strata_chaintsn::context::EpochCommitment) {}

    fn finalized_epoch(&self) -> strata_chaintsn::context::EpochCommitment {
        strata_chaintsn::context::EpochCommitment::new(strata_primitives::buf::Buf32::zero(), 0)
    }

    fn set_finalized_epoch(&mut self, _epoch: strata_chaintsn::context::EpochCommitment) {}

    fn last_l1_block(&self) -> strata_chaintsn::context::L1BlockCommitment {
        strata_chaintsn::context::L1BlockCommitment::new(strata_primitives::buf::Buf32::zero(), 0)
    }

    fn epoch_finishing_flag(&self) -> bool {
        false
    }

    fn set_epoch_finishing_flag(&mut self, _flag: bool) {}
}

pub struct DemoState {
    ledger: InMemoryVectorLedger,
    state_accessor: SimpleStateAccessor,
    block_history: Vec<OLBlock>,
    params: RollupParams,
}

impl DemoState {
    pub fn new() -> Self {
        let genesis_state = OLState::new(
            Buf32::zero(),
            L1View::new(Buf32::zero(), 0),
            0,
            0,
        );

        Self {
            ledger: InMemoryVectorLedger::new(),
            state_accessor: SimpleStateAccessor::new(genesis_state),
            block_history: Vec::new(),
            params: RollupParams::default(),
        }
    }

    pub fn initialize_accounts(&mut self, count: u32) {
        for i in 0..count {
            let account_id = Buf32::from([i as u8; 32]);
            let initial_balance = 1000 * (i as u64 + 1);
            
            let snark_state = SnarkAccountState {
                update_vk: Buf32::zero(),
                proof_state: SnarkAccountProofState {
                    inner_state_root: Buf32::zero(),
                    next_input_idx: 0,
                },
                seq_no: 0,
                input: Vec::new(),
            };

            let account_state = AccountState {
                serial: i,
                ty: 1, // SNARK account type
                balance: initial_balance,
                inner_state: AccountInnerState::Snark(snark_state),
            };

            self.ledger.create_account(i, account_id, account_state);
        }
    }

    pub fn process_block(&mut self, block: OLBlock) -> Result<String, StfError> {
        let prev_header = self.get_latest_header();
        
        let result = process_block(
            self.state_accessor.get_toplevel_state(),
            &prev_header,
            &block,
            &self.params,
            &mut self.state_accessor,
            &mut self.ledger,
        )?;

        self.block_history.push(block);
        Ok(format!("Block processed successfully. New state root: {}", result.state_root()))
    }

    pub fn get_state_info(&self) -> StateInfo {
        let state = self.state_accessor.get_toplevel_state();
        StateInfo {
            accounts_root: *state.accounts_root(),
            cur_slot: state.cur_slot(),
            cur_epoch: state.cur_epoch(),
            l1_block_height: state.l1_view().block_height(),
            account_count: self.ledger.account_states.len(),
        }
    }

    pub fn get_account(&self, account_id: &AccountId) -> Option<AccountState> {
        self.ledger.account_state(account_id).ok().flatten()
    }

    pub fn get_latest_header(&self) -> OLBlockHeader {
        if let Some(last_block) = self.block_history.last() {
            last_block.signed_header().header().clone()
        } else {
            // Genesis header
            OLBlockHeader::new(0, 0, 0, Buf32::zero(), Buf32::zero(), Buf32::zero())
        }
    }

    pub fn block_count(&self) -> usize {
        self.block_history.len()
    }
}

pub type SharedDemoState = Arc<Mutex<DemoState>>;