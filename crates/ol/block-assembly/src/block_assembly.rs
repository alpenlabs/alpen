//! Block assembly logic.

use std::sync::Arc;

use strata_db_types::errors::DbError;
use strata_ledger_types::IStateAccessor;
use strata_ol_chain_types_new::{
    OLTransaction, OLTxSegment, SnarkAccountUpdateTxPayload, TransactionAttachment,
    TransactionPayload,
};
use strata_ol_mempool::{OLMempoolTransaction, OLMempoolTxPayload};
use strata_ol_state_support_types::WriteTrackingState;
use strata_ol_state_types::{NativeAccountState, OLState, WriteBatch};
use strata_ol_stf::{BlockComponents, BlockContext, BlockInfo, construct_block};
use strata_snark_acct_types::{
    AccumulatorClaim, LedgerRefProofs, MessageEntry, MessageEntryProof, MmrEntryProof,
    SnarkAccountUpdateContainer, UpdateAccumulatorProofs,
};
use strata_storage::NodeStorage;

use crate::{
    context::{BlockAssemblyContext, BlockAssemblyContextImpl},
    error::BlockAssemblyError,
    types::{BlockGenerationConfig, FullBlockTemplate},
};

/// Generate a block template from the given configuration.
pub(crate) fn generate_block_template_inner(
    config: BlockGenerationConfig,
    ctx: &BlockAssemblyContextImpl,
) -> Result<FullBlockTemplate, BlockAssemblyError> {
    // 1. Fetch parent state using the commitment
    let parent_commitment = config.parent_block_commitment();
    let parent_state = ctx
        .storage()
        .ol_state()
        .get_toplevel_ol_state_blocking(parent_commitment)
        .map_err(BlockAssemblyError::Database)?
        .ok_or_else(|| {
            BlockAssemblyError::Database(DbError::Other(format!(
                "Parent state not found for commitment: {parent_commitment}"
            )))
        })?;

    // 2. Get slot and epoch from parent state
    // Note: We use the commitment's slot, but state also tracks cur_slot/cur_epoch
    let parent_slot = parent_commitment.slot();
    let parent_epoch = parent_state.cur_epoch();
    let next_slot = parent_slot + 1;
    let next_epoch = parent_epoch; // Epoch only changes in terminal blocks

    // 3. Determine timestamp
    let timestamp = config.ts().unwrap_or_else(|| {
        // Use current time if not provided
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            * 1000 // Convert to milliseconds
    });

    // 4. Get transactions from mempool and process them
    let mut best_txs = ctx.get_mempool_transactions()?;
    let mut transaction_payloads: Vec<TransactionPayload> = Vec::new();

    while let Some((txid, mempool_tx)) = best_txs.next() {
        match convert_mempool_tx_to_payload(mempool_tx, &parent_state, ctx.storage()) {
            Ok(payload) => {
                transaction_payloads.push(payload);
            }
            Err(e) => {
                // Mark transaction as invalid if conversion fails
                tracing::debug!(
                    ?txid,
                    ?e,
                    "Failed to convert mempool transaction, marking invalid"
                );
                best_txs.mark_invalid(txid);
            }
        }
    }

    // Remove marked invalid transactions from mempool
    let invalid_txids = best_txs.marked_invalid();
    if !invalid_txids.is_empty() {
        tracing::debug!(
            count = invalid_txids.len(),
            "Removing invalid transactions from mempool"
        );
        ctx.remove_mempool_transactions(&invalid_txids)?;
    }

    // 5. Create block components
    let txs: Vec<OLTransaction> = transaction_payloads
        .into_iter()
        .map(|payload| OLTransaction::new(payload, TransactionAttachment::default()))
        .collect();

    let tx_segment = OLTxSegment::new(txs).map_err(|e| {
        BlockAssemblyError::Database(DbError::Other(format!(
            "Failed to create tx segment: {e:?}"
        )))
    })?;

    // TODO: Check if this should be a terminal block (epoch boundary)
    // For now, we'll create a non-terminal block
    let block_components = BlockComponents::new(tx_segment, None);

    // 6. Create block info
    let block_info = if next_slot == 0 && next_epoch == 0 {
        BlockInfo::new_genesis(timestamp)
    } else {
        BlockInfo::new(timestamp, next_slot, next_epoch)
    };

    // 7. Create block context
    // TODO: We need the parent block header to create BlockContext for non-genesis blocks.
    // This is a limitation - OL blocks aren't stored like L2 blocks.
    // Options:
    // 1. Store OL blocks (add OLBlockManager similar to L2BlockManager)
    // 2. Reconstruct parent header from state (if state contains header info)
    // 3. Add a public constructor for BlockContext that doesn't require parent header for block
    //    assembly
    //
    // For now, pass None which will cause BlockContext::new to panic for non-genesis.
    // This needs to be fixed by storing OL blocks or reconstructing the header.
    let block_context = BlockContext::new(&block_info, None);

    // 8. Create write-tracking state for execution
    let write_batch = WriteBatch::new_from_state(parent_state.as_ref());
    let mut exec_state = WriteTrackingState::new(parent_state.as_ref(), write_batch);

    // 9. Execute block and construct header/body
    let construct_output = construct_block(&mut exec_state, block_context, block_components)
        .map_err(|e| {
            BlockAssemblyError::Database(DbError::Other(format!("Block execution failed: {e:?}")))
        })?;

    let completed_block = construct_output.completed_block();
    let header = completed_block.header().clone();
    let body = completed_block.body().clone();

    // 10. Return full block template
    Ok(FullBlockTemplate::new(header, body))
}

/// Convert a mempool transaction to a transaction payload with accumulator proofs.
///
/// For SnarkAccountUpdate transactions, this generates:
/// - MessageEntryProof for each message using the account's inbox MMR from OL state
/// - MmrEntryProof for each L1 header reference using the MMR database
fn convert_mempool_tx_to_payload(
    mempool_tx: OLMempoolTransaction,
    parent_state: &Arc<OLState>,
    storage: &NodeStorage,
) -> Result<TransactionPayload, BlockAssemblyError> {
    match mempool_tx.payload() {
        OLMempoolTxPayload::GenericAccountMessage(gam) => {
            // Generic account messages don't need proofs
            Ok(TransactionPayload::GenericAccountMessage(gam.clone()))
        }

        OLMempoolTxPayload::SnarkAccountUpdate(mempool_payload) => {
            let target = *mempool_payload.target();
            let base_update = mempool_payload.base_update().clone();
            let operation = base_update.operation();

            // Get account state to access inbox MMR
            let account_state = parent_state
                .get_account_state(target)
                .map_err(|e| {
                    BlockAssemblyError::Database(DbError::Other(format!(
                        "Failed to get account state: {e:?}"
                    )))
                })?
                .ok_or_else(|| {
                    BlockAssemblyError::Database(DbError::Other(format!(
                        "Account not found: {target}"
                    )))
                })?;

            // Generate inbox message proofs
            let inbox_proofs = generate_inbox_proofs(
                operation.processed_messages(),
                account_state,
                operation.new_state().next_inbox_msg_idx(),
            )?;

            // Generate L1 header proofs using MMR database
            let l1_header_proofs =
                generate_l1_header_proofs(operation.ledger_refs().l1_header_refs(), storage)?;

            // Create accumulator proofs
            let accumulator_proofs = UpdateAccumulatorProofs::new(inbox_proofs, l1_header_proofs);

            // Convert to full container
            let update_container =
                SnarkAccountUpdateContainer::new(base_update, accumulator_proofs);

            // Create transaction payload
            let tx_payload = SnarkAccountUpdateTxPayload::new(target, update_container);
            Ok(TransactionPayload::SnarkAccountUpdate(tx_payload))
        }
    }
}

/// Generate MessageEntryProof for each message using the account's inbox MMR.
///
/// # TODO
///
/// Inbox MMR proof generation is not yet implemented. The issue is that `MerkleMr64B32`
/// (the in-memory MMR type) doesn't expose `get_node()` or other methods needed to
/// implement `MmrDatabase`, and it doesn't have a direct proof generation method.
///
/// To implement this, we need one of:
/// 1. A method on `MerkleMr64B32` to generate proofs directly (e.g., `proof_at(index)`)
/// 2. A way to access individual nodes from `MerkleMr64B32` (e.g., `get_node(pos)`)
/// 3. An adapter that can reconstruct nodes on-demand from the compact representation
fn generate_inbox_proofs(
    _messages: &[MessageEntry],
    _account_state: &NativeAccountState,
    _start_idx: u64,
) -> Result<Vec<MessageEntryProof>, BlockAssemblyError> {
    // TODO: Implement inbox MMR proof generation
    // This requires either:
    // - A proof generation method on MerkleMr64B32 (e.g., proof_at)
    // - A get_node method on MerkleMr64B32 to implement MmrDatabase adapter
    // - A way to reconstruct nodes from the compact representation
    Err(BlockAssemblyError::Database(DbError::Other(
        "Inbox MMR proof generation not yet implemented".to_string(),
    )))
}

/// Generate MmrEntryProof for each L1 header reference using the MMR database.
fn generate_l1_header_proofs(
    l1_header_refs: &[AccumulatorClaim],
    storage: &NodeStorage,
) -> Result<LedgerRefProofs, BlockAssemblyError> {
    let mmr_manager = storage.mmr();
    let mut l1_header_proofs = Vec::new();

    for claim in l1_header_refs {
        // Generate proof for the claim's index
        let merkle_proof = mmr_manager.generate_proof(claim.idx()).map_err(|e| {
            BlockAssemblyError::Database(DbError::Other(format!(
                "Failed to generate MMR proof for L1 header at index {}: {e:?}",
                claim.idx()
            )))
        })?;

        // Verify the entry hash matches
        let entry_hash: [u8; 32] = claim.entry_hash().into();
        // Note: We should verify the entry hash matches what's in the MMR at this index
        // For now, we'll trust the claim and generate the proof

        // Create MmrEntryProof
        let mmr_proof = MmrEntryProof::new(entry_hash, merkle_proof);
        l1_header_proofs.push(mmr_proof);
    }

    Ok(LedgerRefProofs::new(l1_header_proofs))
}
