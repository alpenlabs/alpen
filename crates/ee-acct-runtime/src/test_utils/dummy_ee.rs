//! Dummy execution environment for testing.
//!
//! This provides a simple account-based execution model where "accounts" are
//! identified by SubjectId and have a balance. Transactions can move value
//! between accounts and emit outputs.

use std::collections::BTreeMap;

use digest::Digest;
use sha2::Sha256;
use strata_acct_types::SubjectId;
use strata_codec::Codec;
use strata_ee_acct_types::{
    EnvError, EnvResult, ExecBlockOutput, ExecPartialState, ExecPayload, ExecutionEnvironment,
};
use strata_ee_chain_types::{BlockInputs, BlockOutputs};

use crate::test_utils::dummy_ee::types::{DummyBlock, DummyBlockBody, DummyPartialState};

pub mod types;

/// Dummy execution environment for testing.
pub struct DummyExecutionEnvironment;

impl ExecutionEnvironment for DummyExecutionEnvironment {
    type PartialState = DummyPartialState;
    type Block = DummyBlock;
    type WriteBatch = DummyWriteBatch;

    fn execute_block_body(
        &self,
        pre_state: &Self::PartialState,
        exec_payload: &ExecPayload<'_, Self::Block>,
        inputs: &BlockInputs,
    ) -> EnvResult<ExecBlockOutput<Self>> {
        // Start with a copy of the pre-state
        let mut accounts = pre_state.accounts().clone();
        let mut outputs = BlockOutputs::new_empty();

        // 1. Apply deposits from inputs
        for deposit in inputs.subject_deposits() {
            let balance = accounts.entry(deposit.dest()).or_insert(0);
            *balance = balance
                .checked_add(*deposit.value())
                .ok_or(EnvError::ConflictingPublicState)?;
        }

        // 2. Apply transactions from the block body
        for tx in exec_payload.body().transactions() {
            tx.apply(&mut accounts, &mut outputs)?;
        }

        // 3. Create write batch with the changes
        let write_batch = DummyWriteBatch::new(accounts.clone());

        Ok(ExecBlockOutput::new(write_batch, outputs))
    }

    fn complete_header(
        &self,
        exec_payload: &ExecPayload<'_, Self::Block>,
        output: &ExecBlockOutput<Self>,
    ) -> EnvResult<<Self::Block as strata_ee_acct_types::ExecBlock>::Header> {
        use crate::test_utils::dummy_ee::types::DummyHeader;

        // Apply write batch to get state root
        let mut post_state = DummyPartialState::new(output.write_batch().accounts().clone());
        let state_root = post_state.compute_state_root()?;

        let intrinsics = exec_payload.header_intrinsics();
        Ok(DummyHeader::new(
            intrinsics.parent_blkid,
            state_root,
            intrinsics.index,
        ))
    }

    fn verify_outputs_against_header(
        &self,
        _header: &<Self::Block as strata_ee_acct_types::ExecBlock>::Header,
        _outputs: &ExecBlockOutput<Self>,
    ) -> EnvResult<()> {
        // For dummy environment, no additional verification needed
        Ok(())
    }

    fn merge_write_into_state(
        &self,
        state: &mut Self::PartialState,
        wb: &Self::WriteBatch,
    ) -> EnvResult<()> {
        *state = DummyPartialState::new(wb.accounts().clone());
        Ok(())
    }
}

/// Write batch containing the updated account state.
#[derive(Clone, Debug)]
pub struct DummyWriteBatch {
    accounts: BTreeMap<SubjectId, u64>,
}

impl DummyWriteBatch {
    pub fn new(accounts: BTreeMap<SubjectId, u64>) -> Self {
        Self { accounts }
    }

    pub fn accounts(&self) -> &BTreeMap<SubjectId, u64> {
        &self.accounts
    }
}
