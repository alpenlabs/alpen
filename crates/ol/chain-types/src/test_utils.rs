//! Test utilities and proptest strategies for OL chain types.
//!
//! This module contains reusable test utilities and proptest strategies that are used
//! across multiple test modules to avoid code duplication.

#![allow(unreachable_pub, reason = "test utils module")]

use proptest::prelude::*;
use strata_acct_types::{AccountId, AccountSerial, BitcoinAmount, MessageEntry, MsgPayload};
use strata_identifiers::{
    Epoch, Slot,
    test_utils::{buf32_strategy, buf64_strategy, ol_block_id_strategy},
};

use strata_acct_types::TxEffects;

use crate::{*, block_flags::BlockFlags, ssz_generated::ssz::block::*};

/// Strategy for generating random [`OLLog`] values.
pub fn ol_log_strategy() -> impl Strategy<Value = OLLog> {
    (
        any::<u32>().prop_map(AccountSerial::from),
        prop::collection::vec(any::<u8>(), 0..1024),
    )
        .prop_map(|(account_serial, payload)| OLLog::new(account_serial, payload))
}

pub fn ol_tx_segment_strategy() -> impl Strategy<Value = OLTxSegment> {
    prop::collection::vec(ol_transaction_strategy(), 0..10)
        .prop_map(|txs| OLTxSegment { txs: txs.into() })
}

pub fn l1_update_strategy() -> impl Strategy<Value = Option<OLL1Update>> {
    prop::option::of(buf32_strategy().prop_map(|preseal_state_root| OLL1Update {
        preseal_state_root,
        manifest_cont: OLL1ManifestContainer::new(vec![]).expect("empty manifest should succeed"),
    }))
}

pub fn ol_block_header_strategy() -> impl Strategy<Value = OLBlockHeader> {
    (
        any::<u64>(),
        any::<u16>().prop_map(BlockFlags::from),
        any::<Slot>(),
        any::<Epoch>(),
        ol_block_id_strategy(),
        buf32_strategy(),
        buf32_strategy(),
        buf32_strategy(),
    )
        .prop_map(
            |(timestamp, flags, slot, epoch, parent_blkid, body_root, state_root, logs_root)| {
                OLBlockHeader {
                    timestamp,
                    flags,
                    slot,
                    epoch,
                    parent_blkid,
                    body_root,
                    state_root,
                    logs_root,
                }
            },
        )
}

pub fn signed_ol_block_header_strategy() -> impl Strategy<Value = SignedOLBlockHeader> {
    (ol_block_header_strategy(), buf64_strategy()).prop_map(|(header, signature)| {
        SignedOLBlockHeader {
            header,
            credential: OLBlockCredential {
                schnorr_sig: Some(signature).into(),
            },
        }
    })
}

pub fn ol_block_body_strategy() -> impl Strategy<Value = OLBlockBody> {
    (ol_tx_segment_strategy(), l1_update_strategy()).prop_map(|(tx_segment, l1_update)| {
        OLBlockBody {
            tx_segment: Some(tx_segment).into(),
            l1_update: l1_update.into(),
        }
    })
}

pub fn ol_block_strategy() -> impl Strategy<Value = OLBlock> {
    (signed_ol_block_header_strategy(), ol_block_body_strategy()).prop_map(
        |(signed_header, body)| OLBlock {
            signed_header,
            body,
        },
    )
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

pub fn accumulator_claim_strategy() -> impl Strategy<Value = AccumulatorClaim> {
    (any::<u64>(), any::<[u8; 32]>()).prop_map(|(idx, entry_hash)| AccumulatorClaim {
        idx,
        entry_hash: entry_hash.into(),
    })
}

pub fn transaction_attachment_strategy() -> impl Strategy<Value = TxConstraints> {
    (any::<Option<u64>>(), any::<Option<u64>>()).prop_map(|(min_slot, max_slot)| {
        TxConstraints {
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

pub fn sau_tx_payload_strategy() -> impl Strategy<Value = SauTxPayload> {
    (
        any::<[u8; 32]>(),
        any::<[u8; 32]>(),
        any::<u64>(),
        prop::collection::vec(message_entry_strategy(), 0..10),
        prop::collection::vec(any::<u8>(), 0..32),
        prop::collection::vec(any::<u8>(), 0..64),
    )
        .prop_map(
            |(target_bytes, state_bytes, seq_no, messages, extra_data, update_proof)| {
                SauTxPayload {
                    target: AccountId::from(target_bytes),
                    update_operation: SauTxOperationData {
                        update_data: SauTxUpdateData {
                            seq_no,
                            proof_state: SauTxProofState {
                                new_next_msg_idx: 0,
                                inner_state_root: state_bytes.into(),
                            },
                            extra_data: extra_data.into(),
                        },
                        messages: messages.into(),
                        ledger_refs: SauTxLedgerRefs {
                            asm_history_proofs: ssz_types::Optional::None,
                        },
                    },
                    update_proof: update_proof.into(),
                }
            },
        )
}

pub fn transaction_payload_strategy() -> impl Strategy<Value = TransactionPayload> {
    prop_oneof![
        gam_tx_payload_strategy().prop_map(TransactionPayload::GenericAccountMessage),
        sau_tx_payload_strategy().prop_map(TransactionPayload::SnarkAccountUpdate),
    ]
}

pub fn ol_transaction_strategy() -> impl Strategy<Value = OLTransaction> {
    (
        transaction_payload_strategy(),
        transaction_attachment_strategy(),
    )
        .prop_map(|(payload, constraints)| OLTransaction {
            data: TxData {
                payload,
                constraints,
                effects: TxEffects::default(),
            },
            proofs: TxProofs {
                inbox_proofs: ssz_types::Optional::None,
                asm_history_proofs: ssz_types::Optional::None,
            },
        })
}
