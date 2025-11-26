use std::fmt;

use int_enum::IntEnum;
use strata_acct_types::AccountId;
use strata_identifiers::Slot;
use strata_snark_acct_types::SnarkAccountUpdateContainer;

use crate::ssz_generated::ssz::transaction::{
    GamTxPayload, OLTransaction, SnarkAccountUpdateTxPayload, TransactionAttachment,
    TransactionPayload,
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

impl TransactionAttachment {
    pub fn new(min_slot: Option<Slot>, max_slot: Option<Slot>) -> Self {
        Self {
            min_slot: min_slot.into(),
            max_slot: max_slot.into(),
        }
    }

    pub fn min_slot(&self) -> Option<Slot> {
        match &self.min_slot {
            ssz_types::Optional::Some(slot) => Some(*slot),
            ssz_types::Optional::None => None,
        }
    }

    pub fn set_min_slot(&mut self, min_slot: Option<Slot>) {
        self.min_slot = min_slot.into();
    }

    pub fn max_slot(&self) -> Option<Slot> {
        match &self.max_slot {
            ssz_types::Optional::Some(slot) => Some(*slot),
            ssz_types::Optional::None => None,
        }
    }

    pub fn set_max_slot(&mut self, max_slot: Option<Slot>) {
        self.max_slot = max_slot.into();
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

#[cfg(test)]
mod tests {
    use ssz::{Decode, Encode};
    use strata_acct_types::AccountId;
    use strata_snark_acct_types::{
        LedgerRefProofs, LedgerRefs, ProofState, UpdateAccumulatorProofs, UpdateInputData,
        UpdateOperationData, UpdateOutputs, UpdateStateData,
    };
    use strata_test_utils_ssz::ssz_proptest;

    use crate::{
        GamTxPayload, OLTransaction, SnarkAccountUpdateTxPayload, TransactionAttachment,
        TransactionPayload,
        test_utils::{
            gam_tx_payload_strategy, ol_transaction_strategy, transaction_attachment_strategy,
            transaction_payload_strategy,
        },
    };

    mod transaction_attachment {
        use super::*;

        ssz_proptest!(TransactionAttachment, transaction_attachment_strategy());

        #[test]
        fn test_none_values() {
            let attachment = TransactionAttachment {
                min_slot: ssz_types::Optional::None,
                max_slot: ssz_types::Optional::None,
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
                    min_slot: ssz_types::Optional::None,
                    max_slot: ssz_types::Optional::None,
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
                    min_slot: ssz_types::Optional::Some(100),
                    max_slot: ssz_types::Optional::Some(200),
                },
            };
            let encoded = tx.as_ssz_bytes();
            let decoded = OLTransaction::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(tx, decoded);
        }
    }
}
