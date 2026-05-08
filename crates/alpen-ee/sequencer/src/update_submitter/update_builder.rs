use alpen_ee_common::{
    build_ledger_refs_from_da, Batch, BatchProver, ExecBlockRecord, ExecBlockStorage, L1DaBlockRef,
    ProofId, SequencerOLClient,
};
use eyre::{eyre, OptionExt, Result};
use futures::{future::try_join_all, FutureExt};
use strata_acct_types::{
    tree_hash::{Sha256Hasher, TreeHash},
    Hash,
};
use strata_codec::encode_to_vec;
use strata_ee_acct_types::UpdateExtraData;
use strata_snark_acct_types::{
    LedgerRefs, OutputMessage, OutputTransfer, ProofState, SnarkAccountUpdate, UpdateOperationData,
    UpdateOutputs,
};

/// Build a [`SnarkAccountUpdate`] from a batch in ProofReady state.
pub(super) async fn build_update_from_batch(
    batch: &Batch,
    da_refs: &[L1DaBlockRef],
    proof_id: &ProofId,
    ol_client: &(impl SequencerOLClient + Send + Sync),
    exec_storage: &impl ExecBlockStorage,
    prover: &impl BatchProver,
) -> Result<SnarkAccountUpdate> {
    // Get all blocks in the batch
    let blocks = try_join_all(batch.blocks_iter().map(|hash| {
        exec_storage.get_exec_block(hash).map(|block_res| {
            block_res
                .map_err(eyre::Error::from)
                .and_then(|maybe_block| maybe_block.ok_or_else(|| eyre!("missing block")))
        })
    }))
    .await?;

    // Get update proof
    let update_proof = prover
        .get_proof(*proof_id)
        .await?
        .ok_or_else(|| eyre!("missing proof: {}", proof_id))?;

    let seq_no = batch
        .update_seq_no()
        .ok_or_else(|| eyre!("cannot build update for genesis batch"))?;

    // Ledger refs MUST be byte-identical to what the prover commits — see
    // `build_ledger_refs_from_da` in alpen-ee-common.
    let ledger_refs = build_ledger_refs_from_da(da_refs, ol_client).await?;
    let update_operation = build_update_operation(seq_no, ledger_refs, blocks)?;

    // Should we re-check that proof is valid ?

    Ok(SnarkAccountUpdate::new(
        update_operation,
        update_proof.to_vec(),
    ))
}

/// Build an [`UpdateOperationData`] from data in a batch.
fn build_update_operation(
    seq_no: u64,
    ledger_refs: LedgerRefs,
    blocks: Vec<ExecBlockRecord>,
) -> Result<UpdateOperationData> {
    // 1. Get info from final block
    let (inner_state, new_tip_blkid, new_tip_state_root, next_inbox_msg_idx) = {
        let last_block = blocks.last().ok_or_eyre("blocks cannot be empty")?;
        let inner_state: Hash =
            TreeHash::<Sha256Hasher>::tree_hash_root(last_block.account_state())
                .0
                .into();
        let new_tip_blkid = last_block.package().exec_blkid();
        let new_tip_state_root = last_block.account_state().last_exec_state_root();
        let next_inbox_msg_idx = last_block.next_inbox_msg_idx();

        (
            inner_state,
            new_tip_blkid,
            new_tip_state_root,
            next_inbox_msg_idx,
        )
    };

    // 2. Process all blocks to accumulate messages and outputs
    let mut processed_inputs = 0;
    let mut messages = vec![];
    let mut outputs = UpdateOutputs::new_empty();
    for block in blocks {
        let (package, _, mut block_messages) = block.into_parts();

        processed_inputs += package.inputs().total_inputs();
        messages.append(&mut block_messages);
        outputs.try_extend_messages(
            package
                .outputs
                .output_messages
                .into_iter()
                .map(|m| OutputMessage::new(m.dest(), m.payload)),
        )?;
        outputs.try_extend_transfers(
            package
                .outputs
                .output_transfers
                .into_iter()
                .map(|t| OutputTransfer::new(t.dest, t.value)),
        )?;
    }

    // 3. Build extra data
    let extra_data = UpdateExtraData::new(
        new_tip_blkid,
        new_tip_state_root,
        processed_inputs as u32,
        0,
    );
    let extra_data_buf = encode_to_vec(&extra_data)?;

    // 4. Build update operation
    let update = UpdateOperationData::new(
        seq_no,
        ProofState::new(inner_state, next_inbox_msg_idx),
        messages,
        ledger_refs,
        outputs,
        extra_data_buf,
    );

    Ok(update)
}
