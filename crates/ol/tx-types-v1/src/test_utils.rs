//! Test utilities and proptest strategies for OL transaction types.
//!
//! This module contains reusable test utilities and proptest strategies that are used
//! across multiple test modules to avoid code duplication.

#![allow(unreachable_pub, reason = "test utils module")]

use proptest::prelude::*;
use strata_acct_types::{
    AccountId, AccumulatorClaim, BitcoinAmount, MessageEntry, MsgPayload, TxEffects,
};
use strata_identifiers::Buf32;
use strata_identifiers::test_utils::buf32_strategy;
use strata_predicate::{PredicateKey, PredicateTypeId};

use crate::*;

const MAX_BITCOIN_MONEY_SATS: u64 = 21_000_000 * 100_000_000;

/// Creates a [`PredicateKey`] for a BIP-340 Schnorr sequencer pubkey.
pub fn schnorr_predicate(pubkey: &Buf32) -> PredicateKey {
    PredicateKey::try_new(PredicateTypeId::Bip340Schnorr, pubkey.as_slice().to_vec())
        .expect("BIP-340 public key must fit in a predicate condition")
}

pub fn message_entry_strategy() -> impl Strategy<Value = MessageEntry> {
    (
        any::<[u8; 32]>(),
        any::<u32>(),
        0..=MAX_BITCOIN_MONEY_SATS,
        prop::collection::vec(any::<u8>(), 0..256),
    )
        .prop_map(|(source_bytes, incl_epoch, value, data)| MessageEntry {
            source: AccountId::from(source_bytes),
            incl_epoch,
            payload: MsgPayload {
                value: BitcoinAmount::try_from(value)
                    .expect("amount must not exceed the Bitcoin money supply"),
                data: data
                    .try_into()
                    .expect("message payload bytes must fit within SSZ max length"),
            },
        })
}

pub fn accumulator_claim_strategy() -> impl Strategy<Value = AccumulatorClaim> {
    (any::<u64>(), any::<[u8; 32]>()).prop_map(|(idx, entry_hash)| AccumulatorClaim {
        idx,
        entry_hash: entry_hash.into(),
    })
}

pub fn tx_constraints_strategy() -> impl Strategy<Value = TxConstraintsV1> {
    (any::<Option<u64>>(), any::<Option<u64>>()).prop_map(|(min_slot, max_slot)| TxConstraintsV1 {
        min_slot: min_slot.into(),
        max_slot: max_slot.into(),
    })
}

pub fn gam_tx_payload_strategy() -> impl Strategy<Value = GamTxPayloadV1> {
    any::<[u8; 32]>().prop_map(|target_bytes| GamTxPayloadV1 {
        target: AccountId::from(target_bytes),
    })
}

/// Strategy for generating an optional [`PredicateKey`] rotation declaration.
pub fn new_predicate_strategy() -> impl Strategy<Value = Option<PredicateKey>> {
    prop::option::of(prop_oneof![
        Just(PredicateKey::always_accept()),
        buf32_strategy().prop_map(|pubkey| schnorr_predicate(&pubkey)),
    ])
}

pub fn sau_tx_payload_strategy() -> impl Strategy<Value = SauTxPayloadV1> {
    (
        any::<[u8; 32]>(),
        any::<[u8; 32]>(),
        any::<u64>(),
        prop::collection::vec(message_entry_strategy(), 0..10),
        prop::collection::vec(any::<u8>(), 0..32),
        new_predicate_strategy(),
    )
        .prop_map(
            |(target_bytes, state_bytes, seq_no, messages, extra_data, new_predicate)| {
                SauTxPayloadV1 {
                    target: AccountId::from(target_bytes),
                    operation_data: SauTxOperationDataV1 {
                        update_data: SauTxUpdateDataV1 {
                            seq_no,
                            proof_state: SauTxProofStateV1 {
                                new_next_msg_idx: 0,
                                inner_state_root: state_bytes.into(),
                            },
                            extra_data: extra_data
                                .try_into()
                                .expect("extra data must fit within SSZ max length"),
                            new_predicate: new_predicate.into(),
                        },
                        messages: messages
                            .try_into()
                            .expect("messages must fit within SSZ max length"),
                        ledger_refs: SauTxLedgerRefsV1 {
                            l1_block_ref_claims: ssz_types::Optional::None,
                        },
                    },
                }
            },
        )
}

pub fn transaction_payload_strategy() -> impl Strategy<Value = TransactionPayloadV1> {
    prop_oneof![
        gam_tx_payload_strategy().prop_map(TransactionPayloadV1::GenericAccountMessage),
        sau_tx_payload_strategy().prop_map(TransactionPayloadV1::SnarkAccountUpdate),
    ]
}

pub fn ol_transaction_strategy() -> impl Strategy<Value = OLTransactionV1> {
    (transaction_payload_strategy(), tx_constraints_strategy()).prop_map(
        |(payload, constraints)| OLTransactionV1 {
            data: OLTransactionDataV1 {
                payload,
                constraints,
                effects: TxEffects::default(),
            },
            proofs: TxProofsV1 {
                predicate_satisfiers: ssz_types::Optional::None,
                accumulator_proofs: ssz_types::Optional::None,
            },
        },
    )
}
