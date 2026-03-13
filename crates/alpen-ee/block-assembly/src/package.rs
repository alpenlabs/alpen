use alpen_ee_common::EnginePayload;
use bitcoin_bosd::Descriptor;
use strata_acct_types::{AccountId, BitcoinAmount, Hash, MsgPayload};
use strata_bridge_types::OperatorSelection;
use strata_codec::encode_to_vec;
use strata_ee_acct_types::DecodedEeMessageData;
use strata_ee_chain_types::{
    ExecBlockCommitment, ExecBlockPackage, ExecInputs, ExecOutputs, OutputMessage,
    SubjectDepositData,
};
use strata_msg_fmt::{Msg as MsgTrait, OwnedMsg};
use strata_ol_msg_types::{WithdrawalMsgData, DEFAULT_OPERATOR_FEE, WITHDRAWAL_MSG_TYPE_ID};
use strata_snark_acct_runtime::InputMessage;
use tracing::warn;

/// Builds [`ExecInputs`] from parsed input messages.
///
/// Only `Deposit` messages are processed; other message types are logged and ignored.
// TODO(STR-2583) convert this to do it based on extracting pending inputs from the
// current EE account inner state
pub(crate) fn build_block_inputs(
    parsed_inputs: Vec<InputMessage<DecodedEeMessageData>>,
) -> ExecInputs {
    let mut inputs = ExecInputs::new_empty();
    for msg in parsed_inputs {
        let value = msg.meta().value();
        match msg.message() {
            Some(DecodedEeMessageData::Deposit(deposit_msg_data)) => {
                inputs.add_subject_deposit(SubjectDepositData::new(
                    *deposit_msg_data.dest_subject(),
                    value,
                ));
            }
            Some(DecodedEeMessageData::SubjTransfer(_)) => {
                // no need to warn on this
            }
            Some(DecodedEeMessageData::Commit(_)) => {
                // no need to warn on this
            }
            None => {
                // Unknown message, skip
            }
        }
    }
    inputs
}

/// Builds [`ExecOutputs`] from withdrawal intents in the payload.
pub(crate) fn build_block_outputs<TPayload: EnginePayload>(
    bridge_gateway_account_id: AccountId,
    payload: &TPayload,
) -> ExecOutputs {
    let mut outputs = ExecOutputs::new_empty();
    for withdrawal_intent in payload.withdrawal_intents() {
        let Some(msg_payload) = create_withdrawal_init_message_payload(
            withdrawal_intent.destination.clone(),
            BitcoinAmount::from_sat(withdrawal_intent.amt),
            withdrawal_intent.selected_operator,
        ) else {
            warn!(
                withdrawal_txid = %withdrawal_intent.withdrawal_txid,
                "skipping withdrawal: failed to create withdrawal message"
            );
            continue;
        };
        outputs.add_message(OutputMessage::new(bridge_gateway_account_id, msg_payload));
    }
    outputs
}

/// Builds the block package based on execution inputs and results.
pub(crate) fn build_block_package<TPayload: EnginePayload>(
    bridge_gateway_account_id: AccountId,
    parsed_inputs: Vec<InputMessage<DecodedEeMessageData>>,
    payload: &TPayload,
) -> ExecBlockPackage {
    // 1. build block commitment
    let exec_blkid = payload.blockhash();
    // TODO: get using `EvmExecutionEnvironment`
    let raw_block_encoded_hash = Hash::new([0u8; 32]);
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
    selected_operator: OperatorSelection,
) -> Option<MsgPayload> {
    let withdrawal_data = WithdrawalMsgData::new(
        DEFAULT_OPERATOR_FEE,
        dest_desc.to_bytes(),
        selected_operator.raw(),
    )?;
    let body = encode_to_vec(&withdrawal_data).expect("encode withdrawal data");

    let msg = OwnedMsg::new(WITHDRAWAL_MSG_TYPE_ID, body).expect("create message");
    let payload_data = msg.to_vec();

    Some(MsgPayload::new(value, payload_data))
}

#[cfg(test)]
mod tests {
    use strata_acct_types::SubjectId;
    use strata_codec::VarVec;
    use strata_ee_acct_types::{CommitMsgData, DepositMsgData, SubjTransferMsgData};
    use strata_snark_acct_runtime::MsgMeta;

    use super::*;

    fn make_deposit_msg(dest_bytes: [u8; 32], sats: u64) -> InputMessage<DecodedEeMessageData> {
        InputMessage::from_msg(
            MsgMeta::new(AccountId::zero(), 0, BitcoinAmount::from_sat(sats)),
            DecodedEeMessageData::Deposit(DepositMsgData::new(SubjectId::new(dest_bytes))),
        )
    }

    fn make_subj_transfer_msg(sats: u64) -> InputMessage<DecodedEeMessageData> {
        InputMessage::from_msg(
            MsgMeta::new(AccountId::zero(), 0, BitcoinAmount::from_sat(sats)),
            DecodedEeMessageData::SubjTransfer(SubjTransferMsgData::new(
                SubjectId::new([0xaa; 32]),
                SubjectId::new([0xbb; 32]),
                VarVec::new(),
            )),
        )
    }

    fn make_commit_msg() -> InputMessage<DecodedEeMessageData> {
        InputMessage::from_msg(
            MsgMeta::new(AccountId::zero(), 0, BitcoinAmount::from_sat(0)),
            DecodedEeMessageData::Commit(CommitMsgData::new([0xcc; 32])),
        )
    }

    #[test]
    fn build_block_inputs_filters_non_deposit_messages() {
        // Mix of deposit, transfer, and commit messages
        let inputs = vec![
            make_deposit_msg([0x01; 32], 1000),
            make_subj_transfer_msg(500), // Should be ignored
            make_deposit_msg([0x02; 32], 2000),
            make_commit_msg(), // Should be ignored
            make_deposit_msg([0x03; 32], 3000),
        ];

        let block_inputs = build_block_inputs(inputs);

        // Only deposits should be included
        assert_eq!(block_inputs.total_inputs(), 3);
        let deposits = block_inputs.subject_deposits();
        assert_eq!(deposits[0].dest(), SubjectId::new([0x01; 32]));
        assert_eq!(deposits[0].value(), BitcoinAmount::from_sat(1000));
        assert_eq!(deposits[1].dest(), SubjectId::new([0x02; 32]));
        assert_eq!(deposits[2].dest(), SubjectId::new([0x03; 32]));
    }

    #[test]
    fn build_block_inputs_preserves_deposit_value_from_msg_meta() {
        // The value comes from msg.value() (the meta), not from the deposit message itself
        let inputs = vec![make_deposit_msg([0xaa; 32], 12345)];

        let block_inputs = build_block_inputs(inputs);

        assert_eq!(block_inputs.total_inputs(), 1);
        assert_eq!(
            block_inputs.subject_deposits()[0].value(),
            BitcoinAmount::from_sat(12345)
        );
    }
}
