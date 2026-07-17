use alpen_ee_common::{ExecBlockPayload, ExecBlockRecord};
use alpen_ee_params::AlpenParams;
use strata_acct_types::Hash;
use strata_ee_acct_types::EeAccountState;
use strata_ee_chain_types::{ExecBlockCommitment, ExecBlockPackage, ExecInputs, ExecOutputs};
use strata_identifiers::{Buf32, OLBlockCommitment};

pub fn build_genesis_ee_account_state(params: &AlpenParams) -> EeAccountState {
    let genesis_info = params.genesis_block_info();
    EeAccountState::new(
        genesis_info.blockhash().0.into(),
        genesis_info.stateroot().0.into(),
        Vec::new(),
        Vec::new(),
    )
}

pub fn build_genesis_exec_block_package(params: &AlpenParams) -> ExecBlockPackage {
    // genesis_raw_block_encoded_hash: We dont really care about this for genesis block.
    // Sufficient for it to be deterministic.
    // Can be added to [`AlpenParams`] if correct value is required.
    let genesis_raw_block_encoded_hash = Hash::new([0; 32]);

    ExecBlockPackage::new(
        ExecBlockCommitment::new(
            params.genesis_block_info().blockhash().0.into(),
            genesis_raw_block_encoded_hash,
        ),
        ExecInputs::new_empty(),
        ExecOutputs::new_empty(),
    )
}

pub fn build_genesis_exec_block(
    params: &AlpenParams,
    genesis_ol_block: OLBlockCommitment,
) -> (ExecBlockRecord, ExecBlockPayload) {
    let genesis_package = build_genesis_exec_block_package(params);
    let genesis_account_state = build_genesis_ee_account_state(params);

    // These fields are for evm genesis block.
    let genesis_blocknum = params.genesis_block_info().blocknum();
    // Note: This timestamp is only used during blockproduction, so its not necessary for this to be
    // accurate. Can be added to [`AlpenParams`] if correct value is required.
    let genesis_block_timestamp_ms = 0;
    let genesis_parent_blockhash = Buf32([0; 32]); // 0x0
    let genesis_next_inbox_msg_idx = 0;
    let genesis_next_deposit_idx = 0;
    let genesis_messages = vec![];

    let block = ExecBlockRecord::new(
        genesis_package,
        genesis_account_state,
        genesis_blocknum,
        genesis_ol_block,
        genesis_block_timestamp_ms,
        genesis_parent_blockhash,
        genesis_next_inbox_msg_idx,
        genesis_next_deposit_idx,
        genesis_messages,
    );
    let payload = ExecBlockPayload::from_bytes(Vec::new());

    (block, payload)
}
