use ssz::Encode as _;
use strata_acct_types::{
    AccountId, AcctError, AcctResult, BitcoinAmount, Mmr64, StrataHasher, tree_hash::TreeHash,
};
use strata_ledger_types::{ISnarkAccountState, IStateAccessor};
use strata_merkle::MerkleProof;
use strata_snark_acct_types::{
    LedgerRefProofs, MessageEntry, MessageEntryProof, ProofState, SnarkAccountUpdate,
    SnarkAccountUpdateContainer, UpdateOperationData, UpdateOutputs, UpdateProofPubParams,
};

/// Verifies an account update is correct with respect to the current state of
/// snark account, including checking account balances.
pub fn verify_update_correctness<S: IStateAccessor>(
    state_accessor: &S,
    target: AccountId,
    snark_state: &impl ISnarkAccountState,
    update: &SnarkAccountUpdateContainer,
    cur_balance: BitcoinAmount,
) -> AcctResult<()> {
    let operation = update.base_update().operation();

    // 1. Check seq_no matches
    validate_seq_no(target, snark_state, operation)?;

    // 2. Check message counts / proof indices line up
    validate_message_index(target, snark_state, operation)?;

    let accum_proofs = update.accumulator_proofs();

    // 3. Verify ledger references using the provided state accessor
    verify_ledger_refs(
        target,
        state_accessor.asm_manifests_mmr(),
        accum_proofs.ledger_ref_proofs(),
    )?;

    // 4. Verify input mmr proofs
    verify_input_mmr_proofs(target, snark_state, accum_proofs.inbox_proofs())?;

    // 5. Verify outputs can be applied safely
    let outputs = operation.outputs();
    verify_update_outputs_safe(outputs, state_accessor, cur_balance)?;

    // 6. Verify the proof
    verify_update_proof(target, snark_state, update.base_update())?;

    Ok(())
}

/// Validates the update sequence number against the snark state.
pub fn validate_seq_no(
    target: AccountId,
    snark_state: &impl ISnarkAccountState,
    operation: &UpdateOperationData,
) -> AcctResult<()> {
    let expected_seq = snark_state.seqno();
    if operation.seq_no() != *expected_seq.inner() {
        return Err(AcctError::InvalidUpdateSequence {
            account_id: target,
            expected: *expected_seq.inner(),
            got: operation.seq_no(),
        });
    }
    Ok(())
}

/// Validates the update message index against the snark state.
pub fn validate_message_index(
    target: AccountId,
    snark_state: &impl ISnarkAccountState,
    operation: &UpdateOperationData,
) -> AcctResult<()> {
    let expected_idx =
        snark_state.next_inbox_msg_idx() + operation.processed_messages().len() as u64;
    let claimed_idx = operation.new_proof_state().next_inbox_msg_idx();

    if expected_idx != claimed_idx {
        return Err(AcctError::InvalidMsgIndex {
            account_id: target,
            expected: expected_idx,
            got: claimed_idx,
        });
    }
    Ok(())
}

/// Verifies the ledger ref proofs against the provided asm mmr for an account.
fn verify_ledger_refs(
    account: AccountId,
    mmr: &Mmr64,
    ledger_ref_proofs: &LedgerRefProofs,
) -> AcctResult<()> {
    let generic_mmr = mmr.to_generic();
    for proof in ledger_ref_proofs.l1_headers_proofs() {
        let hash = proof.entry_hash();
        let cohashes = proof.proof().cohashes();
        let generic_proof = MerkleProof::from_cohashes(cohashes, proof.entry_idx());
        if !generic_mmr.verify::<StrataHasher>(&generic_proof, hash.as_ref()) {
            return Err(AcctError::InvalidLedgerReference {
                account_id: account,
                ref_idx: proof.entry_idx(),
            });
        }
    }
    Ok(())
}

/// Verifies the processed messages proofs against the provided account state's inbox
/// mmr.
pub(crate) fn verify_input_mmr_proofs(
    account_id: AccountId,
    state: &impl ISnarkAccountState,
    msg_proofs: &[MessageEntryProof],
) -> AcctResult<()> {
    let generic_mmr = state.inbox_mmr().to_generic();
    let mut cur_index = state.next_inbox_msg_idx();
    for msg_proof in msg_proofs {
        let hash = <MessageEntry as TreeHash>::tree_hash_root(msg_proof.entry());

        let cohashes: Vec<[u8; 32]> = msg_proof.raw_proof().cohashes();
        let proof = MerkleProof::from_cohashes(cohashes, cur_index);

        if !generic_mmr.verify::<StrataHasher>(&proof, &hash.into_inner()) {
            return Err(AcctError::InvalidMessageProof {
                account_id,
                msg_idx: cur_index,
            });
        }

        cur_index += 1;
    }
    Ok(())
}

/// Verifies that the outputs in the update are valid i.e. checks balances and that the receipents
/// exist.
fn verify_update_outputs_safe<S: IStateAccessor>(
    outputs: &UpdateOutputs,
    state_accessor: &S,
    cur_balance: BitcoinAmount,
) -> AcctResult<()> {
    let transfers = outputs.transfers();
    let messages = outputs.messages();

    // Check if receivers exist (skip special/system accounts)
    for t in transfers {
        if !t.dest().is_special() && !state_accessor.check_account_exists(t.dest())? {
            return Err(AcctError::MissingExpectedAccount(t.dest()));
        }
    }

    for m in messages {
        if !m.dest().is_special() && !state_accessor.check_account_exists(m.dest())? {
            return Err(AcctError::MissingExpectedAccount(m.dest()));
        }
    }

    let total_sent = outputs
        .compute_total_value()
        .ok_or(AcctError::BitcoinAmountOverflow)?;

    // Check if there is sufficient balance.
    if total_sent > cur_balance {
        return Err(AcctError::InsufficientBalance {
            requested: total_sent,
            available: cur_balance,
        });
    }
    Ok(())
}

/// Verifies the update witness(proof and pub params) against the VK of the snark account.
pub(crate) fn verify_update_proof(
    target: AccountId,
    snark_state: &impl ISnarkAccountState,
    update: &SnarkAccountUpdate,
) -> AcctResult<()> {
    let vk = snark_state.verification_key();
    let claim: Vec<u8> = compute_update_claim(snark_state, update.operation());
    let is_valid = vk
        .verify_claim_witness(&claim, update.update_proof())
        .is_ok();

    if !is_valid {
        return Err(AcctError::InvalidUpdateProof { account_id: target });
    }

    Ok(())
}

/// Computes the verifiable claim to be verified against a VK.
fn compute_update_claim(
    snark_state: &impl ISnarkAccountState,
    operation: &UpdateOperationData,
) -> Vec<u8> {
    // Use new state, processed messages, old state, refs and outputs to compute claim
    let cur_state = ProofState::new(
        snark_state.inner_state_root(),
        snark_state.next_inbox_msg_idx(),
    );
    let pub_params = UpdateProofPubParams::new(
        cur_state,
        operation.new_proof_state(),
        operation.processed_messages().to_vec(),
        operation.ledger_refs().clone(),
        operation.outputs().clone(),
        operation.extra_data().to_vec(),
    );
    pub_params.as_ssz_bytes()
}
