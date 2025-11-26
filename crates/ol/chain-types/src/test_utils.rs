//! Test utilities and proptest strategies for OL chain types.
//!
//! This module contains reusable test utilities and proptest strategies that are used
//! across multiple test modules to avoid code duplication.

#![allow(unreachable_pub, reason = "test utils module")]

use proptest::prelude::*;
use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload, RawMerkleProof};
use strata_identifiers::Buf32;
use strata_snark_acct_types::{
    AccumulatorClaim, LedgerRefProofs, LedgerRefs, MessageEntry, MessageEntryProof, MmrEntryProof,
    OutputMessage, OutputTransfer, ProofState, SnarkAccountUpdate, SnarkAccountUpdateContainer,
    UpdateAccumulatorProofs, UpdateInputData, UpdateOperationData, UpdateOutputs, UpdateStateData,
};

use crate::{
    GamTxPayload, OLTransaction, SnarkAccountUpdateTxPayload, TransactionAttachment,
    TransactionPayload,
};

pub fn buf32_strategy() -> impl Strategy<Value = Buf32> {
    any::<[u8; 32]>().prop_map(Buf32::from)
}

pub fn message_entry_strategy() -> impl Strategy<Value = MessageEntry> {
    (
        any::<[u8; 32]>(),
        any::<u32>(),
        any::<u64>(),
        prop::collection::vec(any::<u8>(), 0..256),
    )
        .prop_map(|(source_bytes, incl_epoch, value, data)| MessageEntry {
            source: AccountId::from(source_bytes),
            incl_epoch,
            payload: MsgPayload {
                value: BitcoinAmount::from_sat(value),
                data: data.into(),
            },
        })
}

pub fn message_entry_proof_strategy() -> impl Strategy<Value = MessageEntryProof> {
    (
        message_entry_strategy(),
        prop::collection::vec(any::<[u8; 32]>(), 0..10),
    )
        .prop_map(|(entry, proof_hashes)| {
            let raw_proof = RawMerkleProof {
                cohashes: proof_hashes
                    .into_iter()
                    .map(|h| h.into())
                    .collect::<Vec<_>>()
                    .into(),
            };
            MessageEntryProof { entry, raw_proof }
        })
}

pub fn mmr_entry_proof_strategy() -> impl Strategy<Value = MmrEntryProof> {
    (
        any::<[u8; 32]>(),
        any::<u64>(),
        prop::collection::vec(any::<[u8; 32]>(), 0..10),
    )
        .prop_map(|(entry_hash, index, proof_hashes)| {
            let raw_proof = RawMerkleProof {
                cohashes: proof_hashes
                    .into_iter()
                    .map(|h| h.into())
                    .collect::<Vec<_>>()
                    .into(),
            };
            MmrEntryProof {
                entry_hash: entry_hash.into(),
                proof: strata_acct_types::MerkleProof {
                    inner: raw_proof,
                    index,
                },
            }
        })
}

pub fn accumulator_claim_strategy() -> impl Strategy<Value = AccumulatorClaim> {
    (any::<u64>(), any::<[u8; 32]>()).prop_map(|(idx, entry_hash)| AccumulatorClaim {
        idx,
        entry_hash: entry_hash.into(),
    })
}

pub fn output_transfer_strategy() -> impl Strategy<Value = OutputTransfer> {
    (any::<[u8; 32]>(), any::<u64>()).prop_map(|(dest_bytes, value)| OutputTransfer {
        dest: AccountId::from(dest_bytes),
        value: BitcoinAmount::from_sat(value),
    })
}

pub fn output_message_strategy() -> impl Strategy<Value = OutputMessage> {
    (
        any::<[u8; 32]>(),
        any::<u64>(),
        prop::collection::vec(any::<u8>(), 0..256),
    )
        .prop_map(|(dest_bytes, value, data)| OutputMessage {
            dest: AccountId::from(dest_bytes),
            payload: MsgPayload {
                value: BitcoinAmount::from_sat(value),
                data: data.into(),
            },
        })
}

pub fn transaction_attachment_strategy() -> impl Strategy<Value = TransactionAttachment> {
    (any::<Option<u64>>(), any::<Option<u64>>()).prop_map(|(min_slot, max_slot)| {
        TransactionAttachment {
            min_slot: min_slot.into(),
            max_slot: max_slot.into(),
        }
    })
}

pub fn gam_tx_payload_strategy() -> impl Strategy<Value = GamTxPayload> {
    (
        any::<[u8; 32]>(),
        prop::collection::vec(any::<u8>(), 0..256),
    )
        .prop_map(|(target_bytes, payload)| GamTxPayload {
            target: AccountId::from(target_bytes),
            payload: payload.into(),
        })
}

pub fn snark_account_update_tx_payload_strategy()
-> impl Strategy<Value = SnarkAccountUpdateTxPayload> {
    (
        any::<[u8; 32]>(),
        any::<[u8; 32]>(),
        any::<u64>(),
        prop::collection::vec(message_entry_strategy(), 0..10), // messages
        prop::collection::vec(any::<u8>(), 0..32),              // extra_data
        prop::collection::vec(accumulator_claim_strategy(), 0..5), // l1_header_refs
        prop::collection::vec(any::<u8>(), 0..64),              // update_proof
        prop::collection::vec(message_entry_proof_strategy(), 0..5), // inbox_proofs
        prop::collection::vec(mmr_entry_proof_strategy(), 0..5), // l1_headers_proofs
        prop::collection::vec(output_transfer_strategy(), 0..5), // output_transfers
        prop::collection::vec(output_message_strategy(), 0..5), // output_messages
    )
        .prop_map(
            |(
                target_bytes,
                state_bytes,
                seq_no,
                messages,
                extra_data,
                l1_header_refs,
                update_proof,
                inbox_proofs,
                l1_headers_proofs,
                output_transfers,
                output_messages,
            )| {
                SnarkAccountUpdateTxPayload {
                    target: AccountId::from(target_bytes),
                    update_container: SnarkAccountUpdateContainer {
                        base_update: SnarkAccountUpdate {
                            operation: UpdateOperationData {
                                input: UpdateInputData {
                                    seq_no,
                                    messages: messages.into(),
                                    update_state: UpdateStateData {
                                        proof_state: ProofState {
                                            inner_state: state_bytes.into(),
                                            next_inbox_msg_idx: 0,
                                        },
                                        extra_data: extra_data.into(),
                                    },
                                },
                                ledger_refs: LedgerRefs {
                                    l1_header_refs: l1_header_refs.into(),
                                },
                                outputs: UpdateOutputs {
                                    transfers: output_transfers.into(),
                                    messages: output_messages.into(),
                                },
                            },
                            update_proof: update_proof.into(),
                        },
                        accumulator_proofs: UpdateAccumulatorProofs {
                            inbox_proofs: inbox_proofs.into(),
                            ledger_ref_proofs: LedgerRefProofs {
                                l1_headers_proofs: l1_headers_proofs.into(),
                            },
                        },
                    },
                }
            },
        )
}

pub fn transaction_payload_strategy() -> impl Strategy<Value = TransactionPayload> {
    prop_oneof![
        gam_tx_payload_strategy().prop_map(TransactionPayload::GenericAccountMessage),
        snark_account_update_tx_payload_strategy().prop_map(TransactionPayload::SnarkAccountUpdate),
    ]
}

pub fn ol_transaction_strategy() -> impl Strategy<Value = OLTransaction> {
    (
        transaction_payload_strategy(),
        transaction_attachment_strategy(),
    )
        .prop_map(|(payload, attachment)| OLTransaction {
            payload,
            attachment,
        })
}
