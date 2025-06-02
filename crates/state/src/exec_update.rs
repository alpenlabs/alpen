//! Chain data types relating to the CL's updating view of an execution
//! environment's state.  For now the EVM EL is the only execution environment.

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_primitives::{
    buf::Buf32, evm_exec::create_evm_extra_payload, prelude::payload::BlobSpec,
};

use crate::{
    bridge_ops::{self, DepositIntent},
    prelude::StateQueue,
};

/// Full update payload containing inputs and outputs to an EE state update.
#[derive(
    Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize, Serialize, Deserialize,
)]
pub struct ExecUpdate {
    /// Inputs used to construct the call to the EL executor, or provided to the
    /// proof.
    input: UpdateInput,

    /// Conceptual "outputs" from the state transition that we verify either in
    /// the proof or by asking the EL.
    output: UpdateOutput,
}

impl ExecUpdate {
    pub fn new(input: UpdateInput, output: UpdateOutput) -> Self {
        Self { input, output }
    }

    pub fn input(&self) -> &UpdateInput {
        &self.input
    }

    pub fn output(&self) -> &UpdateOutput {
        &self.output
    }
}

/// Contains the explicit inputs to the STF.  Implicit inputs are determined
/// from the CL's exec env state.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct UpdateInput {
    /// Update index.  This is incremented exactly 1.  This is to handle the
    /// future possible cases where we skip CL blocks and provide a monotonic
    /// ordering of EL states.
    update_idx: u64,

    /// Merkle tree root of the contents of the EL payload, in the order it was
    /// strataed in the block.
    entries_root: Buf32,

    /// Buffer of any other payload data.  This is used with the other fields
    /// here to construct the full EVM header payload.
    extra_payload: Vec<u8>,

    /// last applied Deposit Ops index
    applied_ops: Vec<Op>,
}

impl<'a> Arbitrary<'a> for UpdateInput {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let update_idx = u64::arbitrary(u)?;
        let entries_root = Buf32::arbitrary(u)?;
        let block_hash = Buf32::arbitrary(u)?;
        let applied_ops = Vec::arbitrary(u)?;
        let extra_payload = create_evm_extra_payload(block_hash);

        Ok(Self {
            update_idx,
            entries_root,
            applied_ops,
            extra_payload,
        })
    }
}

impl UpdateInput {
    pub fn new(
        update_idx: u64,
        applied_ops: Vec<Op>,
        entries_root: Buf32,
        extra_payload: Vec<u8>,
    ) -> Self {
        Self {
            update_idx,
            applied_ops,
            entries_root,
            extra_payload,
        }
    }

    pub fn update_idx(&self) -> u64 {
        self.update_idx
    }

    pub fn entries_root(&self) -> &Buf32 {
        &self.entries_root
    }

    pub fn applied_ops(&self) -> &[Op] {
        &self.applied_ops
    }

    pub fn extra_payload(&self) -> &[u8] {
        &self.extra_payload
    }
}

/// Conceptual "outputs" from the state transition.  This isn't stored in the
/// state, but it's stored in the block.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
pub struct UpdateOutput {
    /// New state root for the update.  This is not just the inner EL payload,
    /// but also any extra bookkeeping we need across multiple.
    new_state: Buf32,

    /// Bridge withdrawal intents.
    withdrawals: Vec<bridge_ops::WithdrawalIntent>,

    /// DA blobs that we expect to see on L1.  This may be empty, probably is
    /// only set near the end of the range of blocks in a batch since we only
    /// assert these in a per-batch frequency.
    da_blobs: Vec<BlobSpec>,
}

impl UpdateOutput {
    pub fn new_from_state(state: Buf32) -> Self {
        Self {
            new_state: state,
            da_blobs: Vec::new(),
            withdrawals: Vec::new(),
        }
    }

    pub fn with_withdrawals(mut self, withdrawals: Vec<bridge_ops::WithdrawalIntent>) -> Self {
        self.withdrawals = withdrawals;
        self
    }

    pub fn new_state(&self) -> &Buf32 {
        &self.new_state
    }

    pub fn withdrawals(&self) -> &[bridge_ops::WithdrawalIntent] {
        &self.withdrawals
    }

    pub fn da_blobs(&self) -> &[BlobSpec] {
        &self.da_blobs
    }
}

/// Operation the CL pushes into the EL to perform as part of the block it's
/// producing.

#[derive(
    Clone, Debug, Eq, PartialEq, Arbitrary, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub enum Op {
    /// Deposit some amount.
    Deposit(ELDepositData),
}

pub fn construct_ops_from_deposit_intents(
    pending_deposits: &StateQueue<DepositIntent>,
    max_deposits_in_block: u8,
) -> Vec<Op> {
    let mut pending_deposits = pending_deposits.clone();
    let mut el_ops = Vec::new();
    while let Some(idx) = pending_deposits.front_idx() {
        //  first 16 withdrawals (full or partial) into the withdrawal queue.
        if el_ops.len() == max_deposits_in_block as usize {
            break;
        }
        let pending_deposit = pending_deposits.pop_front().unwrap();

        el_ops.push(Op::Deposit(ELDepositData::new(
            idx,
            pending_deposit.amt(),
            pending_deposit.dest_ident().to_vec(),
        )));
    }
    el_ops
}

#[derive(
    Clone, Debug, Eq, PartialEq, Arbitrary, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct ELDepositData {
    /// base index of applied deposit intent.
    intent_idx: u64,

    /// Amount in L1 native asset.  For Bitcoin this is sats.
    amt: u64,

    /// Dest addr encoded in a portable format, assumed to be valid but must be
    /// checked by EL before committing to building block.
    dest_addr: Vec<u8>,
}

impl ELDepositData {
    pub fn new(intent_idx: u64, amt: u64, dest_addr: Vec<u8>) -> Self {
        Self {
            intent_idx,
            amt,
            dest_addr,
        }
    }

    pub fn amt(&self) -> u64 {
        self.amt
    }

    pub fn dest_addr(&self) -> &[u8] {
        &self.dest_addr
    }

    pub fn intent_idx(&self) -> u64 {
        self.intent_idx
    }
}

impl<'a> Arbitrary<'a> for UpdateOutput {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Self::new_from_state(Buf32::arbitrary(u)?))
    }
}
