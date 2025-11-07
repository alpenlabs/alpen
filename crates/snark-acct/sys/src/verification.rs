use strata_acct_types::{
    AccountId, AcctError, AcctResult, BitcoinAmount, MerkleProof, Mmr64, StrataHasher,
};
use strata_ledger_types::{IL1ViewState, ISnarkAccountState, StateAccessor};
use strata_merkle::hasher::MerkleHasher;
use strata_snark_acct_types::{
    LedgerRefProofs, MessageEntryProof, SnarkAccountUpdate, SnarkAccountUpdateWithMmrProofs,
    UpdateOperationData, UpdateOutputs,
};

/// Verifies an account update is correct with respect to the current state of
/// snark account, including checking account balances.
pub fn verify_update_correctness<'a, S: StateAccessor>(
    state_accessor: &S,
    sender: AccountId,
    snark_state: &impl ISnarkAccountState,
    update: &'a SnarkAccountUpdateWithMmrProofs,
    cur_balance: BitcoinAmount,
) -> AcctResult<VerifiedUpdate<'a>> {
    let operation = update.update().operation();
    let outputs = operation.outputs();

    // 1. Check seq_no matches
    if snark_state.seqno() != operation.seq_no() {
        return Err(AcctError::InvalidUpdateSequence {
            account_id: sender,
            expected: snark_state.seqno(),
            got: operation.seq_no(),
        });
    }

    // 2. Check message counts / proof indices line up
    let cur_idx = snark_state.next_inbox_idx();
    let new_idx = operation.new_state().next_inbox_msg_idx();
    let processed_len = operation.processed_messages().len() as u64;

    let expected_idx = cur_idx
        .checked_add(processed_len)
        .ok_or(AcctError::MsgIndexOverflow { account_id: sender })?;
    if expected_idx != new_idx {
        return Err(AcctError::InvalidMsgIndex {
            account_id: sender,
            expected: expected_idx,
            got: new_idx,
        });
    }

    // 3. Verify ledger references using the provided state accessor
    verify_ledger_refs(
        sender,
        state_accessor.l1_view().asm_manifests_mmr(),
        update.ledger_ref_proofs(),
    )?;

    // 4. Verify input mmr proofs
    verify_input_mmr_proofs(sender, snark_state, update.message_proofs())?;

    // 4. Verify outputs can be applied safely
    verify_update_outputs_safe(outputs, state_accessor, cur_balance)?;

    // 5. Verify the witness check
    verify_update_witness(sender, snark_state, update.update(), state_accessor)?;

    Ok(VerifiedUpdate { operation })
}

fn verify_ledger_refs(
    account: AccountId,
    mmr: &Mmr64,
    ledger_ref_proofs: &LedgerRefProofs,
) -> AcctResult<()> {
    for proof in ledger_ref_proofs.l1_headers_proofs() {
        let hash = proof.entry_hash();
        if !mmr.verify(proof.proof(), hash) {
            return Err(AcctError::InvalidLedgerReference {
                account_id: account,
                ref_idx: proof.entry_idx(),
            });
        }
    }
    Ok(())
}

pub fn verify_input_mmr_proofs(
    account_id: AccountId,
    state: &impl ISnarkAccountState,
    msg_proofs: &[MessageEntryProof],
) -> AcctResult<()> {
    let mut cur_index = state.next_inbox_idx();
    for msg_proof in msg_proofs {
        let msg_bytes: Vec<u8> = msg_proof.entry().to_ssz_bytes();
        let hash = StrataHasher::hash_leaf(&msg_bytes);

        let cohashes = msg_proof.raw_proof().cohashes();
        let proof = MerkleProof::from_cohashes(cohashes.to_vec(), cur_index);

        if !state.inbox_mmr().verify(&proof, &hash) {
            return Err(AcctError::InvalidMessageProof {
                account_id,
                msg_idx: cur_index,
            });
        }

        cur_index += 1;
    }
    Ok(())
}

pub fn verify_update_witness<S: StateAccessor>(
    sender: AccountId,
    snark_state: &impl ISnarkAccountState,
    update: &SnarkAccountUpdate,
    _state_accessor: &S,
) -> AcctResult<()> {
    let vk = snark_state.verifier_key();
    let claim: Vec<u8> = compute_update_claim(snark_state, update.operation());
    let is_valid = vk
        .verify_claim_witness(&claim, update.update_proof())
        .is_ok();

    if !is_valid {
        return Err(AcctError::InvalidUpdateProof { account_id: sender });
    }

    Ok(())
}

fn compute_update_claim(
    _snark_state: &impl ISnarkAccountState,
    _operation: &UpdateOperationData,
) -> Vec<u8> {
    // Use new state, processed messages, old state, refs and outputs to compute claim
    todo!()
}

pub fn verify_update_outputs_safe<S: StateAccessor>(
    outputs: &UpdateOutputs,
    state_accessor: &S,
    cur_balance: BitcoinAmount,
) -> AcctResult<()> {
    let transfers = outputs.transfers();
    let messages = outputs.messages();

    // Check if receivers exist
    for t in transfers {
        if !state_accessor.check_account_exists(t.dest())? {
            return Err(AcctError::NonExistentAccount(t.dest()));
        }
    }

    for m in messages {
        if !state_accessor.check_account_exists(m.dest())? {
            return Err(AcctError::NonExistentAccount(m.dest()));
        }
    }

    let total_sent = outputs
        .compute_total_value()
        .ok_or(AcctError::BitcoinAmountOverflow)?;

    // Check if there is sufficient balance.
    if total_sent > cur_balance {
        return Err(AcctError::InsufficientBalance);
    }
    Ok(())
}

/// Type safe update that indicates it has been verified.
#[derive(Clone, Debug)]
pub struct VerifiedUpdate<'a> {
    operation: &'a UpdateOperationData,
}

impl<'a> VerifiedUpdate<'a> {
    pub fn operation(&self) -> &'a UpdateOperationData {
        self.operation
    }
}
