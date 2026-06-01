//! Proof interface types.

use strata_acct_types::MessageEntry;

use crate::{
    LedgerRefs, ProofState, Seqno, UpdateOutputs,
    ssz_generated::ssz::proof_interface::UpdateProofPubParams,
};

impl UpdateProofPubParams {
    pub fn new(
        seq_no: Seqno,
        cur_state: ProofState,
        new_state: ProofState,
        message_inputs: Vec<MessageEntry>,
        ledger_refs: LedgerRefs,
        outputs: UpdateOutputs,
        extra_data: Vec<u8>,
    ) -> Self {
        Self {
            seq_no: *seq_no.inner(),
            cur_state,
            new_state,
            message_inputs: message_inputs
                .try_into()
                .expect("message inputs must fit within SSZ max length"),
            ledger_refs,
            outputs,
            extra_data: extra_data
                .try_into()
                .expect("extra data must fit within SSZ max length"),
        }
    }

    pub fn seq_no(&self) -> Seqno {
        Seqno::new(self.seq_no)
    }

    pub fn cur_state(&self) -> ProofState {
        self.cur_state.clone()
    }

    pub fn new_state(&self) -> ProofState {
        self.new_state.clone()
    }

    pub fn message_inputs(&self) -> &[MessageEntry] {
        &self.message_inputs
    }

    pub fn ledger_refs(&self) -> &LedgerRefs {
        &self.ledger_refs
    }

    pub fn outputs(&self) -> &UpdateOutputs {
        &self.outputs
    }

    pub fn extra_data(&self) -> &[u8] {
        &self.extra_data
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use ssz::Encode as _;
    use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload};
    use strata_test_utils_ssz::ssz_proptest;

    use super::*;
    use crate::{AccumulatorClaim, OutputMessage, OutputTransfer, Seqno};

    fn proof_state_strategy() -> impl Strategy<Value = ProofState> {
        (any::<[u8; 32]>(), any::<u64>()).prop_map(|(inner_state, next_idx)| ProofState {
            inner_state: inner_state.into(),
            next_inbox_msg_idx: next_idx,
        })
    }

    fn account_id_strategy() -> impl Strategy<Value = AccountId> {
        any::<[u8; 32]>().prop_map(AccountId::from)
    }

    fn msg_payload_strategy() -> impl Strategy<Value = MsgPayload> {
        (any::<u64>(), prop::collection::vec(any::<u8>(), 0..32)).prop_map(|(value, data)| {
            MsgPayload {
                value: BitcoinAmount::from_sat(value),
                data: data
                    .try_into()
                    .expect("message payload bytes must fit within SSZ max length"),
            }
        })
    }

    fn message_entry_strategy() -> impl Strategy<Value = MessageEntry> {
        (account_id_strategy(), any::<u32>(), msg_payload_strategy()).prop_map(
            |(source, incl_epoch, payload)| MessageEntry {
                source,
                incl_epoch,
                payload,
            },
        )
    }

    fn accumulator_claim_strategy() -> impl Strategy<Value = AccumulatorClaim> {
        (any::<u64>(), any::<[u8; 32]>()).prop_map(|(idx, entry_hash)| AccumulatorClaim {
            idx,
            entry_hash: entry_hash.into(),
        })
    }

    fn ledger_refs_strategy() -> impl Strategy<Value = LedgerRefs> {
        prop::collection::vec(accumulator_claim_strategy(), 0..3).prop_map(|refs| LedgerRefs {
            l1_block_refs: refs
                .try_into()
                .expect("ledger refs must fit within SSZ max length"),
        })
    }

    fn output_message_strategy() -> impl Strategy<Value = OutputMessage> {
        (account_id_strategy(), msg_payload_strategy())
            .prop_map(|(dest, payload)| OutputMessage { dest, payload })
    }

    fn output_transfer_strategy() -> impl Strategy<Value = OutputTransfer> {
        (account_id_strategy(), any::<u64>()).prop_map(|(dest, value)| OutputTransfer {
            dest,
            value: BitcoinAmount::from_sat(value),
        })
    }

    fn update_outputs_strategy() -> impl Strategy<Value = UpdateOutputs> {
        (
            prop::collection::vec(output_transfer_strategy(), 0..3),
            prop::collection::vec(output_message_strategy(), 0..3),
        )
            .prop_map(|(transfers, messages)| UpdateOutputs {
                transfers: transfers
                    .try_into()
                    .expect("transfers must fit within SSZ max length"),
                messages: messages
                    .try_into()
                    .expect("messages must fit within SSZ max length"),
            })
    }

    fn update_proof_pub_params_strategy() -> impl Strategy<Value = UpdateProofPubParams> {
        (
            any::<u64>(),
            proof_state_strategy(),
            proof_state_strategy(),
            prop::collection::vec(message_entry_strategy(), 0..3),
            ledger_refs_strategy(),
            update_outputs_strategy(),
            prop::collection::vec(any::<u8>(), 0..32),
        )
            .prop_map(
                |(
                    seq_no,
                    cur_state,
                    new_state,
                    message_inputs,
                    ledger_refs,
                    outputs,
                    extra_data,
                )| {
                    UpdateProofPubParams {
                        seq_no,
                        cur_state,
                        new_state,
                        message_inputs: message_inputs
                            .try_into()
                            .expect("message inputs must fit within SSZ max length"),
                        ledger_refs,
                        outputs,
                        extra_data: extra_data
                            .try_into()
                            .expect("extra data must fit within SSZ max length"),
                    }
                },
            )
    }

    ssz_proptest!(UpdateProofPubParams, update_proof_pub_params_strategy());

    /// Regression test for the Zellic replay finding: two
    /// `UpdateProofPubParams` that are otherwise identical but built for
    /// different `seq_no` values must produce different SSZ encodings.
    /// `compute_update_claim` (in `strata-snark-acct-sys`) hashes this SSZ
    /// encoding into the proof claim, so this property is what prevents an
    /// older proof from being replayed at a later `seq_no`.
    #[test]
    fn pub_params_encoding_binds_seq_no() {
        let proof_state = ProofState::new([0u8; 32].into(), 0);
        let make = |seq_no: u64| {
            UpdateProofPubParams::new(
                Seqno::new(seq_no),
                proof_state.clone(),
                proof_state.clone(),
                Vec::new(),
                LedgerRefs::new_empty(),
                UpdateOutputs::new_empty(),
                Vec::new(),
            )
            .as_ssz_bytes()
        };
        assert_ne!(
            make(7),
            make(8),
            "claim must differ across seq_no to prevent proof replay"
        );
    }
}
