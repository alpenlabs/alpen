//! Snark account subject-transfer transaction builder.

use anyhow::Context;
use ssz::Encode;
use strata_acct_types::{AccountId, BitcoinAmount, Hash, MsgPayload, SubjectId};
use strata_codec::VarVec;
use strata_ee_acct_types::{SubjTransferMsgData, SUBJ_TRANSFER_MSG_TYPE};
use strata_msg_fmt::{Msg, OwnedMsg};
use strata_snark_acct_types::{
    LedgerRefs, OutputMessage, ProofState, Seqno, UpdateOperationData, UpdateOutputs,
};

use crate::mock_ee::withdrawal::{bip340_test_sign, sign_claim_ssz};

/// Builds an `RpcOLTransaction` JSON value for a snark account subject transfer.
#[expect(
    clippy::too_many_arguments,
    reason = "This function is a test utility and needs to accept many parameters to construct the transaction."
)]
pub(crate) fn build_snark_subject_transfer_json(
    target: AccountId,
    seq_no: u64,
    inner_state: Hash,
    next_inbox_idx: u64,
    dest_account: AccountId,
    source_subject: SubjectId,
    dest_subject: SubjectId,
    transfer_data: Vec<u8>,
    amount: u64,
) -> anyhow::Result<serde_json::Value> {
    let transfer_msg_data = SubjTransferMsgData::new(
        source_subject,
        dest_subject,
        VarVec::from_vec(transfer_data).ok_or_else(|| anyhow::anyhow!("transfer data too long"))?,
    );
    let encoded_body = strata_codec::encode_to_vec(&transfer_msg_data)
        .context("failed to encode subject transfer msg data")?;
    let owned_msg = OwnedMsg::new(SUBJ_TRANSFER_MSG_TYPE, encoded_body)
        .context("failed to create subject transfer message")?;
    let msg_payload = MsgPayload::from_bytes(BitcoinAmount::from_sat(amount), owned_msg.to_vec())
        .expect("subject transfer message payload bytes must fit within SSZ max length");

    let output_message = OutputMessage::new(dest_account, msg_payload);
    let outputs = UpdateOutputs::new(vec![], vec![output_message]);
    let proof_state = ProofState::new(inner_state, next_inbox_idx);
    let operation_data = UpdateOperationData::new(
        seq_no,
        proof_state.clone(),
        vec![],
        LedgerRefs::new_empty(),
        outputs.clone(),
        vec![],
    );

    let ssz_hex = hex::encode(operation_data.as_ssz_bytes());
    let claim_ssz = sign_claim_ssz(Seqno::new(seq_no), &proof_state, &proof_state, &outputs);
    let update_proof_hex = hex::encode(bip340_test_sign(&claim_ssz));

    let target_bytes: [u8; 32] = target.into();
    let target_hex = hex::encode(target_bytes);

    Ok(serde_json::json!({
        "payload": {
            "type": "snark_account_update",
            "target": target_hex,
            "update_operation_encoded": ssz_hex,
            "update_proof": update_proof_hex,
        },
        "constraints": {
            "min_slot": null,
            "max_slot": null
        }
    }))
}
