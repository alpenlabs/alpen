use std::fmt;

use int_enum::IntEnum;
use strata_acct_types::AccountId;
use strata_identifiers::Slot;

use crate::ssz_generated::ssz::transaction::{
    GamTxPayload, OLTransaction, SauTxPayload, TransactionAttachment, TransactionPayload, TxData,
    TxProofs,
};

impl OLTransaction {
    pub fn new(data: TxData, proofs: TxProofs) -> Self {
        Self { data, proofs }
    }

    pub fn data(&self) -> &TxData {
        &self.data
    }

    pub fn proofs(&self) -> &TxProofs {
        &self.proofs
    }

    pub fn attachment(&self) -> &TransactionAttachment {
        &self.data.attachment
    }

    pub fn payload(&self) -> &TransactionPayload {
        &self.data.payload
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
            TransactionPayload::GenericAccountMessage(msg) => Some(msg.target),
            TransactionPayload::SnarkAccountUpdate(update) => Some(update.target),
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

impl SauTxPayload {
    pub fn target(&self) -> &AccountId {
        &self.target
    }
}

#[cfg(test)]
mod tests {
    use ssz::{Decode, Encode};
    use strata_acct_types::AccountId;
    use strata_test_utils_ssz::ssz_proptest;

    use crate::{
        GamTxPayload, OLTransaction, SauTxLedgerRefs, SauTxOperationData, SauTxPayload,
        SauTxProofState, SauTxUpdateData, TransactionAttachment, TransactionPayload, TxData,
        TxProofs,
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
            let payload = TransactionPayload::SnarkAccountUpdate(SauTxPayload {
                target: AccountId::from([0u8; 32]),
                update_operation: SauTxOperationData {
                    update_data: SauTxUpdateData {
                        seq_no: 1,
                        proof_state: SauTxProofState {
                            new_next_msg_idx: 0,
                            inner_state_root: [0u8; 32].into(),
                        },
                        extra_data: vec![].into(),
                    },
                    messages: vec![].into(),
                    ledger_refs: SauTxLedgerRefs {
                        asm_history_proofs: ssz_types::Optional::None,
                    },
                },
                update_proof: vec![].into(),
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
                data: TxData {
                    payload: TransactionPayload::GenericAccountMessage(GamTxPayload {
                        target: AccountId::from([0u8; 32]),
                        payload: vec![].into(),
                    }),
                    attachment: TransactionAttachment {
                        min_slot: ssz_types::Optional::None,
                        max_slot: ssz_types::Optional::None,
                    },
                },
                proofs: TxProofs {
                    inbox_proofs: ssz_types::Optional::None,
                    asm_history_proofs: ssz_types::Optional::None,
                },
            };
            let encoded = tx.as_ssz_bytes();
            let decoded = OLTransaction::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(tx, decoded);
        }

        #[test]
        fn test_snark_account_update() {
            let tx = OLTransaction {
                data: TxData {
                    payload: TransactionPayload::SnarkAccountUpdate(SauTxPayload {
                        target: AccountId::from([1u8; 32]),
                        update_operation: SauTxOperationData {
                            update_data: SauTxUpdateData {
                                seq_no: 42,
                                proof_state: SauTxProofState {
                                    new_next_msg_idx: 10,
                                    inner_state_root: [5u8; 32].into(),
                                },
                                extra_data: vec![].into(),
                            },
                            messages: vec![].into(),
                            ledger_refs: SauTxLedgerRefs {
                                asm_history_proofs: ssz_types::Optional::None,
                            },
                        },
                        update_proof: vec![].into(),
                    }),
                    attachment: TransactionAttachment {
                        min_slot: ssz_types::Optional::Some(100),
                        max_slot: ssz_types::Optional::Some(200),
                    },
                },
                proofs: TxProofs {
                    inbox_proofs: ssz_types::Optional::None,
                    asm_history_proofs: ssz_types::Optional::None,
                },
            };
            let encoded = tx.as_ssz_bytes();
            let decoded = OLTransaction::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(tx, decoded);
        }
    }
}
