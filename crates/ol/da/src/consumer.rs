//! Consumer-side helpers for checkpoint DA decoding and state application.

use bitcoin::{ScriptBuf, Transaction};
use ssz::Decode;
use strata_asm_proto_checkpoint_txs::{CHECKPOINT_V0_SUBPROTOCOL_ID, OL_STF_CHECKPOINT_TX_TYPE};
use strata_btc_types::RawBitcoinTx;
use strata_checkpoint_types_ssz::SignedCheckpointPayload;
use strata_codec::decode_buf_exact;
use strata_da_framework::DaWrite;
use strata_l1_envelope_fmt::parser::parse_envelope_payload;
use strata_l1_txfmt::{MagicBytes, ParseConfig};
use strata_ledger_types::{
    IAccountState, IAccountStateConstructible, ISnarkAccountStateConstructible, IStateAccessor,
};

use crate::{DaConsumerError, DaConsumerResult, DaResult, OLDaPayloadV1, OLStateDiff, StateDiff};

/// Decoded checkpoint DA data extracted from an L1 checkpoint transaction.
#[derive(Debug)]
pub struct DecodedCheckpointDa {
    /// Signed checkpoint payload included in the transaction envelope.
    pub signed_checkpoint: SignedCheckpointPayload,

    /// OL DA payload decoded from the checkpoint sidecar.
    pub da_payload: OLDaPayloadV1,
}

/// Decodes checkpoint sidecar DA payload from a raw Bitcoin checkpoint transaction.
pub fn decode_checkpoint_da_blob(
    raw_tx: &RawBitcoinTx,
    magic_bytes: MagicBytes,
) -> DaConsumerResult<DecodedCheckpointDa> {
    let tx = decode_bitcoin_tx(raw_tx)?;
    verify_checkpoint_tag(&tx, magic_bytes)?;
    let signed_checkpoint = decode_signed_checkpoint_payload(&tx)?;
    let da_payload = decode_da_payload_from_signed_checkpoint(&signed_checkpoint)?;

    Ok(DecodedCheckpointDa {
        signed_checkpoint,
        da_payload,
    })
}

/// Decodes the OL DA payload from a signed checkpoint payload's sidecar.
pub fn decode_da_payload_from_signed_checkpoint(
    signed_checkpoint: &SignedCheckpointPayload,
) -> DaConsumerResult<OLDaPayloadV1> {
    decode_buf_exact(signed_checkpoint.inner().sidecar().ol_state_diff())
        .map_err(|e| DaConsumerError::DaPayloadDecode(e.to_string()))
}

/// Applies a decoded DA payload to state (preseal transition only).
pub fn apply_da_payload<S>(state: &mut S, payload: OLDaPayloadV1) -> DaResult<()>
where
    S: IStateAccessor,
    S::AccountState: IAccountStateConstructible,
    <S::AccountState as IAccountState>::SnarkAccountState: ISnarkAccountStateConstructible,
{
    apply_state_diff(state, payload.state_diff)
}

/// Applies a state diff to OL state using DA primitive semantics.
pub fn apply_state_diff<S>(state: &mut S, state_diff: StateDiff) -> DaResult<()>
where
    S: IStateAccessor,
    S::AccountState: IAccountStateConstructible,
    <S::AccountState as IAccountState>::SnarkAccountState: ISnarkAccountStateConstructible,
{
    let da_diff = OLStateDiff::<S>::from(state_diff);
    da_diff.poll_context(state, &())?;
    da_diff.apply(state, &())
}

/// Converts a raw Bitcoin transaction wrapper into a decoded [`Transaction`].
fn decode_bitcoin_tx(raw_tx: &RawBitcoinTx) -> DaConsumerResult<Transaction> {
    Transaction::try_from(raw_tx).map_err(|e| DaConsumerError::TxDecode(e.to_string()))
}

/// Validates that the transaction carries the expected checkpoint subprotocol and tx type tags.
fn verify_checkpoint_tag(tx: &Transaction, magic_bytes: MagicBytes) -> DaConsumerResult<()> {
    let parser = ParseConfig::new(magic_bytes);
    let tag = parser
        .try_parse_tx(tx)
        .map_err(|e| DaConsumerError::TagParse(e.to_string()))?;

    if tag.subproto_id() != CHECKPOINT_V0_SUBPROTOCOL_ID
        || tag.tx_type() != OL_STF_CHECKPOINT_TX_TYPE
    {
        return Err(DaConsumerError::UnsupportedCheckpointTag {
            expected_subprotocol: CHECKPOINT_V0_SUBPROTOCOL_ID,
            actual_subprotocol: tag.subproto_id(),
            expected_tx_type: OL_STF_CHECKPOINT_TX_TYPE,
            actual_tx_type: tag.tx_type(),
        });
    }

    Ok(())
}

/// Decodes the signed checkpoint payload from the transaction's envelope-bearing leaf script.
fn decode_signed_checkpoint_payload(tx: &Transaction) -> DaConsumerResult<SignedCheckpointPayload> {
    let payload_script = checkpoint_payload_script(tx)?;
    let envelope_payload = parse_envelope_payload(&payload_script)
        .map_err(|e| DaConsumerError::EnvelopeParse(e.to_string()))?;

    SignedCheckpointPayload::from_ssz_bytes(&envelope_payload)
        .map_err(|e| DaConsumerError::SignedCheckpointDecode(format!("{e:?}")))
}

/// Extracts the taproot leaf script that contains the checkpoint envelope payload.
fn checkpoint_payload_script(tx: &Transaction) -> DaConsumerResult<ScriptBuf> {
    if tx.input.is_empty() {
        return Err(DaConsumerError::MissingInputs);
    }

    tx.input[0]
        .witness
        .taproot_leaf_script()
        .map(|leaf| leaf.script.into())
        .ok_or(DaConsumerError::MissingLeafScript)
}

#[cfg(test)]
mod tests {
    use ssz::Encode;
    use strata_da_framework::{DaCounter, counter_schemes::CtrU64ByU16};
    use strata_identifiers::{Buf32, OLBlockCommitment, OLBlockId};
    use strata_ledger_types::IStateAccessor;
    use strata_ol_state_types::OLState;

    use super::*;
    use crate::{
        GlobalStateDiff, LedgerDiff, OLDaPayloadV1, StateDiff,
        test_utils::{TEST_MAGIC_BYTES, make_checkpoint_tx, make_signed_checkpoint_payload},
    };

    #[test]
    fn test_decode_checkpoint_da_blob_works() {
        let secret_key = Buf32::from([1u8; 32]);
        let state_diff = StateDiff::default();
        let da_payload = OLDaPayloadV1::new(state_diff);
        let da_payload_bytes = strata_codec::encode_to_vec(&da_payload).expect("encode da payload");
        let signed_checkpoint = make_signed_checkpoint_payload(
            1,
            101,
            OLBlockCommitment::new(5, OLBlockId::from(Buf32::from([7u8; 32]))),
            da_payload_bytes,
            secret_key,
        );

        let tx = make_checkpoint_tx(
            &signed_checkpoint.as_ssz_bytes(),
            CHECKPOINT_V0_SUBPROTOCOL_ID,
            OL_STF_CHECKPOINT_TX_TYPE,
            secret_key,
        );

        let decoded = decode_checkpoint_da_blob(&RawBitcoinTx::from(tx), TEST_MAGIC_BYTES)
            .expect("decode checkpoint da blob");

        assert_eq!(
            decoded.signed_checkpoint.inner().new_tip().epoch,
            signed_checkpoint.inner().new_tip().epoch
        );
        assert_eq!(
            decoded
                .da_payload
                .state_diff
                .ledger
                .new_accounts
                .entries()
                .len(),
            0
        );
    }

    #[test]
    fn test_decode_checkpoint_da_blob_rejects_wrong_tag() {
        let secret_key = Buf32::from([1u8; 32]);
        let signed_checkpoint = make_signed_checkpoint_payload(
            1,
            101,
            OLBlockCommitment::new(5, OLBlockId::from(Buf32::from([7u8; 32]))),
            vec![],
            secret_key,
        );
        let tx = make_checkpoint_tx(
            &signed_checkpoint.as_ssz_bytes(),
            CHECKPOINT_V0_SUBPROTOCOL_ID,
            OL_STF_CHECKPOINT_TX_TYPE + 1,
            secret_key,
        );

        let err = decode_checkpoint_da_blob(&RawBitcoinTx::from(tx), TEST_MAGIC_BYTES)
            .expect_err("wrong checkpoint tag should fail");
        assert!(matches!(
            err,
            DaConsumerError::UnsupportedCheckpointTag { .. }
        ));
    }

    #[test]
    fn test_apply_state_diff_updates_slot() {
        let mut state = OLState::new_genesis();
        assert_eq!(state.cur_slot(), 0);

        let state_diff = StateDiff::new(
            GlobalStateDiff::new(DaCounter::<CtrU64ByU16>::new_changed(2)),
            LedgerDiff::default(),
        );

        apply_state_diff(&mut state, state_diff).expect("apply state diff");
        assert_eq!(state.cur_slot(), 2);
    }
}
