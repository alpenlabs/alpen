use alpen_ee_common::{ExecBlockPayload, ExecBlockRecord};
use alpen_ee_config::AlpenEeParams;
use strata_acct_types::{BitcoinAmount, Hash};
use strata_ee_acct_types::EeAccountState;
use strata_ee_chain_types::{BlockInputs, BlockOutputs, ExecBlockCommitment, ExecBlockPackage};
use strata_identifiers::{Buf32, EpochCommitment, OLBlockCommitment};

pub fn build_genesis_ee_account_state(params: &AlpenEeParams) -> EeAccountState {
    EeAccountState::new(
        params.genesis_blockhash().0.into(),
        BitcoinAmount::zero(),
        Vec::new(),
        Vec::new(),
    )
}

pub fn build_genesis_exec_block_package(params: &AlpenEeParams) -> ExecBlockPackage {
    // genesis_raw_block_encoded_hash: We dont really care about this for genesis block.
    // Sufficient for it to be deterministic.
    // Can be added to [`AlpenEeParams`] if correct value is required.
    let genesis_raw_block_encoded_hash = Hash::new([0; 32]);

    ExecBlockPackage::new(
        ExecBlockCommitment::new(
            params.genesis_blockhash().0.into(),
            genesis_raw_block_encoded_hash,
        ),
        BlockInputs::new_empty(),
        BlockOutputs::new_empty(),
    )
}

pub fn build_genesis_exec_block(params: &AlpenEeParams) -> (ExecBlockRecord, ExecBlockPayload) {
    let genesis_package = build_genesis_exec_block_package(params);
    let genesis_account_state = build_genesis_ee_account_state(params);
    let genesis_ol_block =
        OLBlockCommitment::new(params.genesis_ol_slot(), params.genesis_ol_blockid());

    // These fields are for evm genesis block.
    let genesis_blocknum = 0;
    // Note: This timestamp is only used during blockproduction, so its not necessary for this to be
    // accurate. Can be added to [`AlpenEeParams`] if correct value is required.
    let genesis_block_timestamp_ms = 0;
    let genesis_parent_blockhash = Buf32([0; 32]); // 0x0
    let genesis_next_inbox_msg_idx = 0;
    let genesis_messages = vec![];

    let block = ExecBlockRecord::new(
        genesis_package,
        genesis_account_state,
        genesis_blocknum,
        genesis_ol_block,
        genesis_block_timestamp_ms,
        genesis_parent_blockhash,
        genesis_next_inbox_msg_idx,
        genesis_messages,
    );
    let payload = ExecBlockPayload::from_bytes(Vec::new());

    (block, payload)
}

pub fn build_ee_genesis_ol_epoch(params: &AlpenEeParams) -> EpochCommitment {
    EpochCommitment::new(
        params.genesis_ol_epoch(),
        params.genesis_ol_slot(),
        params.genesis_ol_blockid(),
    )
}
