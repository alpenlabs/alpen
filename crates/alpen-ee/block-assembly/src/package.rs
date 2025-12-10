use alpen_ee_common::EnginePayload;
use bitcoin_bosd::Descriptor;
use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload, SentMessage};
use strata_codec::encode_to_vec;
use strata_ee_acct_runtime::MsgData;
use strata_ee_acct_types::DecodedEeMessageData;
use strata_ee_chain_types::{
    BlockInputs, BlockOutputs, ExecBlockCommitment, ExecBlockPackage, SubjectDepositData,
};
use strata_msg_fmt::{Msg as MsgTrait, OwnedMsg};
use strata_ol_msg_types::{WithdrawalMsgData, WITHDRAWAL_MSG_TYPE_ID};
use tracing::warn;

/// Builds [`BlockInputs`] from parsed input messages.
///
/// Only `Deposit` messages are processed; other message types are logged and ignored.
pub(crate) fn build_block_inputs(parsed_inputs: Vec<MsgData>) -> BlockInputs {
    let mut inputs = BlockInputs::new_empty();
    for msg in parsed_inputs {
        match msg.decoded_message() {
            DecodedEeMessageData::Deposit(deposit_msg_data) => {
                inputs.add_subject_deposit(SubjectDepositData::new(
                    *deposit_msg_data.dest_subject(),
                    msg.value(),
                ));
            }
            DecodedEeMessageData::SubjTransfer(_) => {
                warn!("ignoring unsupported message type: SubjTransfer")
            }
            DecodedEeMessageData::Commit(_) => {
                warn!("ignoring unsupported message type: Commit")
            }
        }
    }
    inputs
}

/// Builds [`BlockOutputs`] from withdrawal intents in the payload.
pub(crate) fn build_block_outputs<TPayload: EnginePayload>(
    bridge_gateway_account_id: AccountId,
    payload: &TPayload,
) -> BlockOutputs {
    let mut outputs = BlockOutputs::new_empty();
    for withdrawal_intent in payload.withdrawal_intents() {
        let msg_payload = create_withdrawal_init_message_payload(
            withdrawal_intent.destination.clone(),
            BitcoinAmount::from_sat(withdrawal_intent.amt),
        );
        outputs.add_message(SentMessage::new(bridge_gateway_account_id, msg_payload));
    }
    outputs
}

/// Builds the block package based on execution inputs and results.
pub(crate) fn build_block_package<TPayload: EnginePayload>(
    bridge_gateway_account_id: AccountId,
    parsed_inputs: Vec<MsgData>,
    payload: &TPayload,
) -> ExecBlockPackage {
    // 1. build block commitment
    let exec_blkid = payload.blockhash();
    // TODO: get using `EvmExecutionEnvironment`
    let raw_block_encoded_hash = [0u8; 32];
    let commitment = ExecBlockCommitment::new(exec_blkid, raw_block_encoded_hash);

    // 2. build block inputs
    let inputs = build_block_inputs(parsed_inputs);

    // 3. build block outputs
    let outputs = build_block_outputs(bridge_gateway_account_id, payload);

    ExecBlockPackage::new(commitment, inputs, outputs)
}

fn create_withdrawal_init_message_payload(
    dest_desc: Descriptor,
    value: BitcoinAmount,
) -> MsgPayload {
    // Encode the deposit message data
    let withdrawal_data =
        WithdrawalMsgData::new(0, dest_desc.to_bytes()).expect("valid descriptor");
    let body = encode_to_vec(&withdrawal_data).expect("encode withdrawal data");

    // Create properly formatted message
    let msg = OwnedMsg::new(WITHDRAWAL_MSG_TYPE_ID, body).expect("create message");
    let payload_data = msg.to_vec();

    MsgPayload::new(value, payload_data)
}
