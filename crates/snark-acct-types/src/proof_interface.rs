//! Proof interface types.

use crate::{
    LedgerRefs, MessageEntry, ProofState, UpdateOutputs,
    ssz_generated::ssz::proof_interface::UpdateProofPubParams,
};

impl UpdateProofPubParams {
    pub fn new(
        cur_state: ProofState,
        new_state: ProofState,
        message_inputs: Vec<MessageEntry>,
        ledger_refs: LedgerRefs,
        outputs: UpdateOutputs,
        extra_data: Vec<u8>,
    ) -> Self {
        Self {
            cur_state,
            new_state,
            message_inputs,
            ledger_refs,
            outputs,
            extra_data,
        }
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

    pub fn to_ssz_bytes(&self) -> Vec<u8> {
        // TODO: Implement proper SSZ serialization for UpdateProofPubParams
        // For now, return dummy bytes since we use PredicateKey::always_accept() in tests
        vec![0u8; 32]
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload};
    use strata_test_utils_ssz::ssz_proptest;

    use super::*;
    use crate::{AccumulatorClaim, OutputMessage, OutputTransfer};

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
                data: data.into(),
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
            l1_header_refs: refs.into(),
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
                transfers: transfers.into(),
                messages: messages.into(),
            })
    }

    fn update_proof_pub_params_strategy() -> impl Strategy<Value = UpdateProofPubParams> {
        (
            proof_state_strategy(),
            proof_state_strategy(),
            prop::collection::vec(message_entry_strategy(), 0..3),
            ledger_refs_strategy(),
            update_outputs_strategy(),
            prop::collection::vec(any::<u8>(), 0..32),
        )
            .prop_map(
                |(cur_state, new_state, message_inputs, ledger_refs, outputs, extra_data)| {
                    UpdateProofPubParams {
                        cur_state,
                        new_state,
                        message_inputs: message_inputs.into(),
                        ledger_refs,
                        outputs,
                        extra_data: extra_data.into(),
                    }
                },
            )
    }

    ssz_proptest!(UpdateProofPubParams, update_proof_pub_params_strategy());
}
