//! Dummy execution environment for testing.
//!
//! This provides a simple account-based execution model where "accounts" are
//! identified by SubjectId and have a balance. Transactions can move value
//! between accounts and emit outputs.

use std::collections::BTreeMap;

use digest::Digest;
use sha2::Sha256;
use strata_acct_types::{SubjectId, VarVec};
use strata_codec::{decode_buf_exact, Codec};
use strata_ee_acct_types::{
    EnvError, EnvResult, ExecBlock, ExecBlockOutput, ExecutionEnvironment,
};
use strata_ee_chain_types::{BlockInputs, BlockOutputs};

use crate::test_utils::dummy_ee::types::{DummyBlock, DummyBlockBody, DummyHeader, DummyPartialState};

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
        body: &DummyBlockBody,
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
        for tx in body.transactions() {
            tx.apply(&mut accounts, &mut outputs)?;
        }

        // 3. Create write batch with the changes
        let write_batch = DummyWriteBatch::new(accounts.clone());

        Ok(ExecBlockOutput::new(write_batch, outputs))
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
