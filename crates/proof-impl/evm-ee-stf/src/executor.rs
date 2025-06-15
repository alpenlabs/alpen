use std::sync::Arc;

use alloy_consensus::{BlockHeader, EthBlock, Header, TxReceipt};
use alpen_reth_evm::{collect_withdrawal_intents, evm::AlpenEvmFactory};
use reth_chainspec::ChainSpec;
use reth_evm::execute::{BasicBlockExecutor, ExecutionOutcome, Executor};
use reth_evm_ethereum::EthEvmConfig;
use reth_primitives::EthPrimitives;
use reth_primitives_traits::block::Block;
use reth_trie::KeccakKeyHasher;
use revm::database::WrapDatabaseRef;
use revm_primitives::alloy_primitives::Bloom;
use rsp_client_executor::{
    error::ClientError, io::EthClientExecutorInput, profile_report, BlockValidator, FromInput,
};

use crate::EvmBlockStfOutput;

pub const INIT_WITNESS_DB: &str = "init witness db";
pub const RECOVER_SENDERS: &str = "recover senders";
pub const BLOCK_EXECUTION: &str = "block execution";
pub const VALIDATE_HEADER: &str = "validate header";
pub const VALIDATE_EXECUTION: &str = "validate block post-execution";
pub const ACCRUE_LOG_BLOOM: &str = "accrue logs bloom";
pub const COMPUTE_STATE_ROOT: &str = "compute state root";
pub const COLLECT_WITHDRAWAL_INTENTS: &str = "collect withdrawal intents";

pub fn process_block(mut input: EthClientExecutorInput) -> Result<EvmBlockStfOutput, ClientError> {
    let chain_spec: Arc<ChainSpec> = Arc::new((&input.genesis).try_into().unwrap());
    let evm_config =
        EthEvmConfig::new_with_evm_factory(chain_spec.clone(), AlpenEvmFactory::default());

    // Initialize the witnessed database with verified storage proofs.
    let db = profile_report!(INIT_WITNESS_DB, {
        let trie_db = input.witness_db().unwrap();
        WrapDatabaseRef(trie_db)
    });

    let block_executor = BasicBlockExecutor::new(evm_config, db);
    let block = profile_report!(RECOVER_SENDERS, {
        EthPrimitives::from_input_block(input.current_block.clone())
            .try_into_recovered()
            .map_err(|_| ClientError::SignatureRecoveryFailed)
    })?;

    // Validate the block header
    profile_report!(VALIDATE_HEADER, {
        EthPrimitives::validate_header(block.sealed_block().sealed_header(), chain_spec.clone())
    })?;

    let execution_output = profile_report!(BLOCK_EXECUTION, { block_executor.execute(&block) })?;

    // Validate the block post execution.
    profile_report!(VALIDATE_EXECUTION, {
        EthPrimitives::validate_block_post_execution(&block, chain_spec.clone(), &execution_output)
    })?;

    // Accumulate the logs bloom.
    let logs_bloom = profile_report!(ACCRUE_LOG_BLOOM, {
        let mut logs_bloom = Bloom::default();
        execution_output.result.receipts.iter().for_each(|r| {
            logs_bloom.accrue_bloom(&r.bloom());
        });
        logs_bloom
    });

    // Accumulate withdrawal intents from the executed transactions.
    let withdrawal_intents = profile_report!(COLLECT_WITHDRAWAL_INTENTS, {
        let transactions = block.into_transactions();
        let executed_txns = transactions.iter();
        let receipts_vec = execution_output.receipts.clone();
        let receipts = receipts_vec.iter();
        let tx_receipt_pairs = executed_txns.zip(receipts);
        collect_withdrawal_intents(tx_receipt_pairs).collect::<Vec<_>>()
    });

    // Convert the output to an execution outcome.
    let executor_outcome = ExecutionOutcome::new(
        execution_output.state,
        vec![execution_output.result.receipts],
        input.current_block.number,
        vec![execution_output.result.requests],
    );

    // Verify the state root.
    let state_root = profile_report!(COMPUTE_STATE_ROOT, {
        input
            .parent_state
            .update(&executor_outcome.hash_state_slow::<KeccakKeyHasher>());
        input.parent_state.state_root()
    });

    if state_root != input.current_block.header().state_root() {
        return Err(ClientError::MismatchedStateRoot);
    }

    // Derive the block header.
    // Note: the receipts root and gas used are verified by `validate_block_post_execution`.
    let header = Header {
        parent_hash: input.current_block.header().parent_hash(),
        ommers_hash: input.current_block.header().ommers_hash(),
        beneficiary: input.current_block.header().beneficiary(),
        state_root,
        transactions_root: input.current_block.header().transactions_root(),
        receipts_root: input.current_block.header().receipts_root(),
        logs_bloom,
        difficulty: input.current_block.header().difficulty(),
        number: input.current_block.header().number(),
        gas_limit: input.current_block.header().gas_limit(),
        gas_used: input.current_block.header().gas_used(),
        timestamp: input.current_block.header().timestamp(),
        extra_data: input.current_block.header().extra_data().clone(),
        mix_hash: input.current_block.header().mix_hash().unwrap(),
        nonce: input.current_block.header().nonce().unwrap(),
        base_fee_per_gas: input.current_block.header().base_fee_per_gas(),
        withdrawals_root: input.current_block.header().withdrawals_root(),
        blob_gas_used: input.current_block.header().blob_gas_used(),
        excess_blob_gas: input.current_block.header().excess_blob_gas(),
        parent_beacon_block_root: input.current_block.header().parent_beacon_block_root(),
        requests_hash: input.current_block.header().requests_hash(),
    };

    let deposit_requests = input
        .current_block
        .withdrawals()
        .map(|w| w.to_vec())
        .unwrap_or_default();

    Ok(EvmBlockStfOutput {
        block_idx: header.number,
        new_blockhash: header.hash_slow(),
        new_state_root: header.state_root,
        prev_blockhash: header.parent_hash,
        txn_root: header.transactions_root,
        deposit_requests,
        withdrawal_intents,
    })
}
