//! For handling deterministic genesis blocks used in EE.

use std::sync::Arc;

use alpen_ee_common::{ExecBlockRecord, ExecBlockStorage};
use alpen_ee_config::{AlpenEeConfig, AlpenEeParams};
use eyre::Context;
use strata_acct_types::BitcoinAmount;
use strata_ee_acct_types::EeAccountState;
use strata_ee_chain_types::{BlockInputs, BlockOutputs, ExecBlockCommitment, ExecBlockPackage};
use strata_identifiers::OLBlockCommitment;

pub fn build_genesis_ee_account_state(params: &AlpenEeParams) -> EeAccountState {
    EeAccountState::new(
        params.genesis_blockhash().into(),
        BitcoinAmount::zero(),
        Vec::new(),
        Vec::new(),
    )
}

pub fn build_genesis_exec_block_package(params: &AlpenEeParams) -> ExecBlockPackage {
    // genesis_raw_block_encoded_hash: We dont really care about this for genesis block.
    // Sufficient for it to be deterministic.
    // Can be added to [`AlpenEeParams`] if correct value is required.
    let genesis_raw_block_encoded_hash = [0; 32];

    ExecBlockPackage::new(
        ExecBlockCommitment::new(
            params.genesis_blockhash().into(),
            genesis_raw_block_encoded_hash,
        ),
        BlockInputs::new_empty(),
        BlockOutputs::new_empty(),
    )
}

pub fn build_genesis_exec_block(params: &AlpenEeParams) -> (ExecBlockRecord, Vec<u8>) {
    let genesis_package = build_genesis_exec_block_package(params);
    let genesis_account_state = build_genesis_ee_account_state(params);
    let genesis_ol_block =
        OLBlockCommitment::new(params.genesis_ol_slot(), params.genesis_ol_blockid());
    let block = ExecBlockRecord::new(
        genesis_package,
        genesis_account_state,
        0,
        genesis_ol_block,
        0,
        [0; 32],
    );
    let payload = Vec::new();

    (block, payload)
}

pub async fn handle_finalized_exec_genesis<TStorage: ExecBlockStorage>(
    config: Arc<AlpenEeConfig>,
    storage: Arc<TStorage>,
) -> eyre::Result<()> {
    let genesis_ee_blockhash = config.params().genesis_blockhash().into();
    let (genesis_block, genesis_block_payload) = build_genesis_exec_block(config.params());

    // If exists, does not overwrite
    storage
        .save_exec_block(genesis_block, genesis_block_payload)
        .await
        .map_err(eyre::Error::from)
        .context("failed to create genesis exec block")?;
    // Inserts if empty, checks genesis blockhash is correct if exists.
    storage
        .init_finalized_chain(genesis_ee_blockhash)
        .await
        .map_err(eyre::Error::from)
        .context("failed to set genesis exec block")?;

    Ok(())
}
