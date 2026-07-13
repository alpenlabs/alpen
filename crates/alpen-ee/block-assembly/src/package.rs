use alpen_ee_common::EnginePayload;
use bitcoin_bosd::Descriptor;
use strata_acct_types::{AccountId, BitcoinAmount, Hash, MsgPayload};
use strata_codec::encode_to_vec;
use strata_ee_acct_types::PendingInputEntry;
use strata_ee_chain_types::{
    ExecBlockCommitment, ExecBlockPackage, ExecInputs, ExecOutputs, OutputMessage,
};
use strata_msg_fmt::{Msg as MsgTrait, OwnedMsg};
use strata_ol_bridge_types::OperatorSelection;
use strata_ol_msg_types::{WithdrawalMsgData, DEFAULT_OPERATOR_FEE, WITHDRAWAL_MSG_TYPE_ID};
use tracing::{info, warn};

/// Builds [`ExecInputs`] from pending input entries that were executed in the current block.
pub(crate) fn build_block_inputs(pending_inputs: Vec<PendingInputEntry>) -> ExecInputs {
    let mut inputs = ExecInputs::new_empty();
    for pending_input in pending_inputs {
        match pending_input {
            PendingInputEntry::Deposit(subj_deposit_data) => {
                info!(
                    dest_subject = ?subj_deposit_data.dest,
                    amount_sat = subj_deposit_data.value.to_sat(),
                    "accepted deposit message as EE input",
                );
                inputs.add_subject_deposit(subj_deposit_data);
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
        let dest_desc_len = withdrawal_intent.destination.to_bytes().len();
        let Some(msg_payload) = create_withdrawal_init_message_payload(
            withdrawal_intent.destination.clone(),
            BitcoinAmount::from_sat(withdrawal_intent.amt),
            withdrawal_intent.selected_operator,
        ) else {
            warn!(
                amount_sat = withdrawal_intent.amt,
                selected_operator = withdrawal_intent.selected_operator.raw(),
                dest_desc_len,
                destination = ?withdrawal_intent.destination,
                "skipping withdrawal: failed to create withdrawal message",
            );
            continue;
        };
        info!(
            amount_sat = withdrawal_intent.amt,
            selected_operator = withdrawal_intent.selected_operator.raw(),
            dest_desc_len,
            destination = ?withdrawal_intent.destination,
            "created withdrawal output message for bridge gateway",
        );
        outputs.add_message(OutputMessage::new(bridge_gateway_account_id, msg_payload));
    }
    outputs
}

/// Builds the block package based on execution inputs and results.
pub(crate) fn build_block_package<TPayload: EnginePayload>(
    bridge_gateway_account_id: AccountId,
    pending_inputs: Vec<PendingInputEntry>,
    payload: &TPayload,
) -> ExecBlockPackage {
    // 1. build block commitment
    let exec_blkid = payload.blockhash();
    // TODO(STR-3682): get using `EvmExecutionEnvironment`
    let raw_block_encoded_hash = Hash::new([0u8; 32]);
    let commitment = ExecBlockCommitment::new(exec_blkid, raw_block_encoded_hash);

    // 2. build block inputs
    let inputs = build_block_inputs(pending_inputs);

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

    MsgPayload::from_bytes(value, payload_data).ok()
}

#[cfg(test)]
mod tests {
    use strata_acct_types::SubjectId;
    use strata_ee_chain_types::SubjectDepositData;

    use super::*;

    fn make_deposit(dest_bytes: [u8; 32], sats: u64) -> PendingInputEntry {
        PendingInputEntry::Deposit(SubjectDepositData::new(
            SubjectId::new(dest_bytes),
            BitcoinAmount::from_sat(sats),
        ))
    }

    #[test]
    fn build_block_inputs_converts_pending_deposits() {
        let inputs = vec![
            make_deposit([0x01; 32], 1000),
            make_deposit([0x02; 32], 2000),
            make_deposit([0x03; 32], 3000),
        ];

        let block_inputs = build_block_inputs(inputs);

        assert_eq!(block_inputs.total_inputs(), 3);
        let deposits = block_inputs.subject_deposits();
        assert_eq!(deposits[0].dest(), SubjectId::new([0x01; 32]));
        assert_eq!(deposits[0].value(), BitcoinAmount::from_sat(1000));
        assert_eq!(deposits[1].dest(), SubjectId::new([0x02; 32]));
        assert_eq!(deposits[1].value(), BitcoinAmount::from_sat(2000));
        assert_eq!(deposits[2].dest(), SubjectId::new([0x03; 32]));
        assert_eq!(deposits[2].value(), BitcoinAmount::from_sat(3000));
    }

    #[test]
    fn build_block_inputs_empty() {
        let block_inputs = build_block_inputs(Vec::new());
        assert_eq!(block_inputs.total_inputs(), 0);
    }

    #[test]
    fn build_block_inputs_preserves_deposit_value() {
        let inputs = vec![make_deposit([0xaa; 32], 12345)];

        let block_inputs = build_block_inputs(inputs);

        assert_eq!(block_inputs.total_inputs(), 1);
        assert_eq!(
            block_inputs.subject_deposits()[0].value(),
            BitcoinAmount::from_sat(12345)
        );
    }
}
