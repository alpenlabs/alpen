use std::fmt;

use int_enum::IntEnum;
use strata_acct_types::AccountId;
use strata_codec::{Codec, CodecError, Decoder, Encoder, Varint};
use strata_snark_acct_types::SnarkAccountUpdateContainer;

use crate::ssz_generated::ssz::{
    block::Slot,
    transaction::{
        GamTxPayload, MAX_TX_PAYLOAD_LEN, OLTransaction, SnarkAccountUpdateTxPayload,
        TransactionAttachment, TransactionPayload,
    },
};

impl OLTransaction {
    pub fn new(payload: TransactionPayload, attachment: TransactionAttachment) -> Self {
        Self {
            payload,
            attachment,
        }
    }

    pub fn attachment(&self) -> &TransactionAttachment {
        &self.attachment
    }

    pub fn payload(&self) -> &TransactionPayload {
        &self.payload
    }

    pub fn target(&self) -> Option<AccountId> {
        self.payload().target()
    }

    pub fn type_id(&self) -> TxTypeId {
        self.payload().type_id()
    }
}

impl Codec for OLTransaction {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.payload.encode(enc)?;
        self.attachment.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let payload = TransactionPayload::decode(dec)?;
        let attachment = TransactionAttachment::decode(dec)?;
        Ok(Self {
            payload,
            attachment,
        })
    }
}

impl TransactionPayload {
    pub fn target(&self) -> Option<AccountId> {
        match self {
            TransactionPayload::GenericAccountMessage(msg) => Some(*msg.target()),
            TransactionPayload::SnarkAccountUpdate(update) => Some(*update.target()),
        }
    }

    pub fn type_id(&self) -> TxTypeId {
        match self {
            TransactionPayload::GenericAccountMessage(_) => TxTypeId::GenericAccountMessage,
            TransactionPayload::SnarkAccountUpdate(_) => TxTypeId::SnarkAccountUpdate,
        }
    }
}

impl Codec for TransactionPayload {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        match self {
            TransactionPayload::GenericAccountMessage(payload) => {
                1u8.encode(enc)?;
                payload.encode(enc)?;
            }
            TransactionPayload::SnarkAccountUpdate(payload) => {
                2u8.encode(enc)?;
                payload.encode(enc)?;
            }
        }
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let variant = u8::decode(dec)?;
        match variant {
            1 => {
                let payload = GamTxPayload::decode(dec)?;
                Ok(TransactionPayload::GenericAccountMessage(payload))
            }
            2 => {
                let payload = SnarkAccountUpdateTxPayload::decode(dec)?;
                Ok(TransactionPayload::SnarkAccountUpdate(payload))
            }
            _ => Err(CodecError::InvalidVariant("TransactionPayload")),
        }
    }
}

impl TransactionAttachment {
    pub fn new_empty() -> Self {
        Self::default()
    }

    pub fn min_slot(&self) -> Option<Slot> {
        self.min_slot
    }

    pub fn set_min_slot(&mut self, min_slot: Option<Slot>) {
        self.min_slot = min_slot;
    }

    pub fn max_slot(&self) -> Option<Slot> {
        self.max_slot
    }

    pub fn set_max_slot(&mut self, max_slot: Option<Slot>) {
        self.max_slot = max_slot;
    }
}

impl Codec for TransactionAttachment {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        // Encode Option fields as bool (is_some) followed by value if present
        match self.min_slot {
            Some(slot) => {
                true.encode(enc)?;
                slot.encode(enc)?;
            }
            None => {
                false.encode(enc)?;
            }
        }

        match self.max_slot {
            Some(slot) => {
                true.encode(enc)?;
                slot.encode(enc)?;
            }
            None => {
                false.encode(enc)?;
            }
        }

        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let min_slot = if bool::decode(dec)? {
            Some(Slot::decode(dec)?)
        } else {
            None
        };
        let max_slot = if bool::decode(dec)? {
            Some(Slot::decode(dec)?)
        } else {
            None
        };
        Ok(Self { min_slot, max_slot })
    }
}

/// Type ID to indicate transaction types.
#[repr(u16)]
#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq, Ord, PartialOrd, IntEnum)]
pub enum TxTypeId {
    /// Transactions that are messages being sent to other accounts.
    GenericAccountMessage = 0,

    /// Transactions that are snark account updates.
    SnarkAccountUpdate = 1,
}

impl fmt::Display for TxTypeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            TxTypeId::GenericAccountMessage => "generic-account-message",
            TxTypeId::SnarkAccountUpdate => "snark-account-update",
        };
        f.write_str(s)
    }
}

impl GamTxPayload {
    pub fn new(target: AccountId, payload: Vec<u8>) -> Result<Self, &'static str> {
        Ok(Self {
            target,
            payload: payload.into(),
        })
    }

    pub fn target(&self) -> &AccountId {
        &self.target
    }

    pub fn payload(&self) -> &[u8] {
        self.payload.as_ref()
    }
}

impl Codec for GamTxPayload {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.target.encode(enc)?;
        // Encode VariableList as length-prefixed Vec
        let payload_vec = self.payload.as_ref();
        (payload_vec.len() as u32).encode(enc)?;
        enc.write_buf(payload_vec)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let target = AccountId::decode(dec)?;
        let len = u32::decode(dec)? as usize;
        if len > MAX_TX_PAYLOAD_LEN as usize {
            return Err(CodecError::MalformedField(
                "GamTxPayload.payload: length exceeds maximum",
            ));
        }
        let mut payload_vec = vec![0u8; len];
        dec.read_buf(&mut payload_vec)?;
        let payload = payload_vec.into();
        Ok(Self { target, payload })
    }
}

impl SnarkAccountUpdateTxPayload {
    pub fn new(target: AccountId, update_container: SnarkAccountUpdateContainer) -> Self {
        Self {
            target,
            update_container,
        }
    }

    pub fn target(&self) -> &AccountId {
        &self.target
    }

    pub fn update_container(&self) -> &SnarkAccountUpdateContainer {
        &self.update_container
    }
}

impl Codec for SnarkAccountUpdateTxPayload {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        use ssz::Encode;
        self.target.encode(enc)?;
        // Encode SnarkAccountUpdateContainer as SSZ bytes (Varint length-prefixed)
        // This matches CodecSsz format for compatibility with main branch
        let ssz_bytes = self.update_container.as_ssz_bytes();
        let len = Varint::new_usize(ssz_bytes.len()).ok_or(CodecError::OverflowContainer)?;
        len.encode(enc)?;
        enc.write_buf(&ssz_bytes)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        use ssz::Decode;
        let target = AccountId::decode(dec)?;
        // Decode Varint length (matching CodecSsz format)
        let len_vi = Varint::decode(dec)?;
        let len = len_vi.inner() as usize;
        let mut ssz_bytes = vec![0u8; len];
        dec.read_buf(&mut ssz_bytes)?;
        let update_container =
            SnarkAccountUpdateContainer::from_ssz_bytes(&ssz_bytes).map_err(|_| {
                CodecError::MalformedField("SnarkAccountUpdateTxPayload.update_container")
            })?;
        Ok(Self {
            target,
            update_container,
        })
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use ssz::{Decode, Encode};
    use strata_acct_types::AccountId;
    use strata_snark_acct_types::{
        LedgerRefProofs, LedgerRefs, ProofState, SnarkAccountUpdate, SnarkAccountUpdateContainer,
        UpdateAccumulatorProofs, UpdateInputData, UpdateOperationData, UpdateOutputs,
        UpdateStateData,
    };
    use strata_test_utils_ssz::ssz_proptest;

    use crate::{
        GamTxPayload, OLTransaction, SnarkAccountUpdateTxPayload, TransactionAttachment,
        TransactionPayload,
    };

    fn transaction_attachment_strategy() -> impl Strategy<Value = TransactionAttachment> {
        (any::<Option<u64>>(), any::<Option<u64>>())
            .prop_map(|(min_slot, max_slot)| TransactionAttachment { min_slot, max_slot })
    }

    fn gam_tx_payload_strategy() -> impl Strategy<Value = GamTxPayload> {
        (
            any::<[u8; 32]>(),
            prop::collection::vec(any::<u8>(), 0..256),
        )
            .prop_map(|(target_bytes, payload)| GamTxPayload {
                target: AccountId::from(target_bytes),
                payload: payload.into(),
            })
    }

    fn snark_account_update_tx_payload_strategy()
    -> impl Strategy<Value = SnarkAccountUpdateTxPayload> {
        (any::<[u8; 32]>(), any::<[u8; 32]>(), any::<u64>()).prop_map(
            |(target_bytes, state_bytes, seq_no)| SnarkAccountUpdateTxPayload {
                target: AccountId::from(target_bytes),
                update_container: SnarkAccountUpdateContainer {
                    base_update: SnarkAccountUpdate {
                        operation: UpdateOperationData {
                            input: UpdateInputData {
                                seq_no,
                                messages: vec![].into(),
                                update_state: UpdateStateData {
                                    proof_state: ProofState {
                                        inner_state: state_bytes.into(),
                                        next_inbox_msg_idx: 0,
                                    },
                                    extra_data: vec![].into(),
                                },
                            },
                            ledger_refs: LedgerRefs {
                                l1_header_refs: vec![].into(),
                            },
                            outputs: UpdateOutputs {
                                transfers: vec![].into(),
                                messages: vec![].into(),
                            },
                        },
                        update_proof: vec![].into(),
                    },
                    accumulator_proofs: UpdateAccumulatorProofs {
                        inbox_proofs: vec![].into(),
                        ledger_ref_proofs: LedgerRefProofs {
                            l1_headers_proofs: vec![].into(),
                        },
                    },
                },
            },
        )
    }

    fn transaction_payload_strategy() -> impl Strategy<Value = TransactionPayload> {
        prop_oneof![
            gam_tx_payload_strategy().prop_map(TransactionPayload::GenericAccountMessage),
            snark_account_update_tx_payload_strategy()
                .prop_map(TransactionPayload::SnarkAccountUpdate),
        ]
    }

    fn ol_transaction_strategy() -> impl Strategy<Value = OLTransaction> {
        (
            transaction_payload_strategy(),
            transaction_attachment_strategy(),
        )
            .prop_map(|(payload, attachment)| OLTransaction {
                payload,
                attachment,
            })
    }

    mod transaction_attachment {
        use super::*;

        ssz_proptest!(TransactionAttachment, transaction_attachment_strategy());

        #[test]
        fn test_none_values() {
            let attachment = TransactionAttachment {
                min_slot: None,
                max_slot: None,
            };
            let encoded = attachment.as_ssz_bytes();
            let decoded = TransactionAttachment::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(attachment, decoded);
        }
    }

    mod gam_tx_payload {
        use super::*;

        ssz_proptest!(GamTxPayload, gam_tx_payload_strategy());

        #[test]
        fn test_empty_payload() {
            let msg = GamTxPayload {
                target: AccountId::from([0u8; 32]),
                payload: vec![].into(),
            };
            let encoded = msg.as_ssz_bytes();
            let decoded = GamTxPayload::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(msg, decoded);
        }

        #[test]
        fn test_with_payload() {
            let msg = GamTxPayload {
                target: AccountId::from([1u8; 32]),
                payload: vec![1, 2, 3, 4, 5].into(),
            };
            let encoded = msg.as_ssz_bytes();
            let decoded = GamTxPayload::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(msg, decoded);
        }
    }

    mod transaction_payload {
        use super::*;

        ssz_proptest!(TransactionPayload, transaction_payload_strategy());

        #[test]
        fn test_gam_tx_payload_variant() {
            let payload = TransactionPayload::GenericAccountMessage(GamTxPayload {
                target: AccountId::from([0u8; 32]),
                payload: vec![1, 2, 3].into(),
            });
            let encoded = payload.as_ssz_bytes();
            let decoded = TransactionPayload::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(payload, decoded);
        }

        #[test]
        fn test_snark_account_update_tx_payload_variant() {
            let payload = TransactionPayload::SnarkAccountUpdate(SnarkAccountUpdateTxPayload {
                target: AccountId::from([0u8; 32]),
                update_container: strata_snark_acct_types::SnarkAccountUpdateContainer {
                    base_update: strata_snark_acct_types::SnarkAccountUpdate {
                        operation: UpdateOperationData {
                            input: UpdateInputData {
                                seq_no: 1,
                                messages: vec![].into(),
                                update_state: UpdateStateData {
                                    proof_state: ProofState {
                                        inner_state: [0u8; 32].into(),
                                        next_inbox_msg_idx: 0,
                                    },
                                    extra_data: vec![].into(),
                                },
                            },
                            ledger_refs: LedgerRefs {
                                l1_header_refs: vec![].into(),
                            },
                            outputs: UpdateOutputs {
                                transfers: vec![].into(),
                                messages: vec![].into(),
                            },
                        },
                        update_proof: vec![].into(),
                    },
                    accumulator_proofs: UpdateAccumulatorProofs::new(
                        vec![],
                        LedgerRefProofs::new(vec![]),
                    ),
                },
            });
            let encoded = payload.as_ssz_bytes();
            let decoded = TransactionPayload::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(payload, decoded);
        }
    }

    mod ol_transaction {
        use super::*;

        ssz_proptest!(OLTransaction, ol_transaction_strategy());

        #[test]
        fn test_generic_message() {
            let tx = OLTransaction {
                payload: TransactionPayload::GenericAccountMessage(GamTxPayload {
                    target: AccountId::from([0u8; 32]),
                    payload: vec![].into(),
                }),
                attachment: TransactionAttachment {
                    min_slot: None,
                    max_slot: None,
                },
            };
            let encoded = tx.as_ssz_bytes();
            let decoded = OLTransaction::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(tx, decoded);
        }

        #[test]
        fn test_snark_account_update() {
            let tx = OLTransaction {
                payload: TransactionPayload::SnarkAccountUpdate(SnarkAccountUpdateTxPayload {
                    target: AccountId::from([1u8; 32]),
                    update_container: strata_snark_acct_types::SnarkAccountUpdateContainer {
                        base_update: strata_snark_acct_types::SnarkAccountUpdate {
                            operation: UpdateOperationData {
                                input: UpdateInputData {
                                    seq_no: 42,
                                    messages: vec![].into(),
                                    update_state: UpdateStateData {
                                        proof_state: ProofState {
                                            inner_state: [5u8; 32].into(),
                                            next_inbox_msg_idx: 10,
                                        },
                                        extra_data: vec![].into(),
                                    },
                                },
                                ledger_refs: LedgerRefs {
                                    l1_header_refs: vec![].into(),
                                },
                                outputs: UpdateOutputs {
                                    transfers: vec![].into(),
                                    messages: vec![].into(),
                                },
                            },
                            update_proof: vec![].into(),
                        },
                        accumulator_proofs: UpdateAccumulatorProofs::new(
                            vec![],
                            LedgerRefProofs::new(vec![]),
                        ),
                    },
                }),
                attachment: TransactionAttachment {
                    min_slot: Some(100),
                    max_slot: Some(200),
                },
            };
            let encoded = tx.as_ssz_bytes();
            let decoded = OLTransaction::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(tx, decoded);
        }
    }
}
