//! Module for database local types

use arbitrary::Arbitrary;
use bitcoin::{
    consensus::{self, deserialize, serialize},
    Transaction,
};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_primitives::{
    buf::Buf32,
    l1::payload::{L1Payload, PayloadIntent},
};
use strata_state::{
    batch::{BatchInfo, BatchTransition, Checkpoint, CheckpointSidecar},
    chain_state::Chainstate,
    client_state::CheckpointL1Ref,
};
use zkaleido::Proof;

/// Represents an intent to publish to some DA, which will be bundled for efficiency.
#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub struct IntentEntry {
    pub intent: PayloadIntent,
    pub status: IntentStatus,
}

impl IntentEntry {
    pub const fn new_unbundled(intent: PayloadIntent) -> Self {
        Self {
            intent,
            status: IntentStatus::Unbundled,
        }
    }

    pub const fn new_bundled(intent: PayloadIntent, bundle_idx: u64) -> Self {
        Self {
            intent,
            status: IntentStatus::Bundled(bundle_idx),
        }
    }

    pub const fn payload(&self) -> &L1Payload {
        self.intent.payload()
    }
}

/// Status of Intent indicating various stages of being bundled to L1 transaction.
/// Unbundled Intents are collected and bundled to create [`BundledPayloadEntry`].
#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub enum IntentStatus {
    // It is not bundled yet, and thus will be collected and processed by bundler.
    Unbundled,
    // It has been bundled to [`BundledPayloadEntry`] with given bundle idx.
    Bundled(u64),
}

/// Represents data for a payload we're still planning to post to L1.
#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub struct BundledPayloadEntry {
    pub payloads: Vec<L1Payload>,
    pub commit_txid: Buf32,
    pub reveal_txid: Buf32,
    pub status: L1BundleStatus,
}

impl BundledPayloadEntry {
    #[expect(clippy::missing_const_for_fn)]
    pub fn new(
        payloads: Vec<L1Payload>,
        commit_txid: Buf32,
        reveal_txid: Buf32,
        status: L1BundleStatus,
    ) -> Self {
        Self {
            payloads,
            commit_txid,
            reveal_txid,
            status,
        }
    }

    /// Create new unsigned [`BundledPayloadEntry`].
    ///
    /// NOTE: This won't have commit - reveal pairs associated with it.
    ///   Because it is better to defer gathering utxos as late as possible to prevent being spent
    ///   by others. Those will be created and signed in a single step.
    pub fn new_unsigned(payloads: Vec<L1Payload>) -> Self {
        let cid = Buf32::zero();
        let rid = Buf32::zero();
        Self::new(payloads, cid, rid, L1BundleStatus::Unsigned)
    }
}

/// Various status that transactions corresponding to a payload can be in L1
#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub enum L1BundleStatus {
    /// The payload has not been signed yet, i.e commit-reveal transactions have not been created
    /// yet.
    Unsigned,

    /// The commit-reveal transactions for payload are signed and waiting to be published
    Unpublished,

    /// The transactions are published
    Published,

    /// The transactions are confirmed
    Confirmed,

    /// The transactions are finalized
    Finalized,

    /// The transactions need to be resigned.
    /// This could be due to transactions input UTXOs already being spent.
    NeedsResign,
}

/// This is the entry that gets saved to the database corresponding to a bitcoin transaction that
/// the broadcaster will publish and watches for until finalization
#[derive(
    Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary, Serialize, Deserialize,
)]
pub struct L1TxEntry {
    /// Raw serialized transaction. This is basically `consensus::serialize()` of [`Transaction`]
    tx_raw: Vec<u8>,

    /// The status of the transaction in bitcoin
    pub status: L1TxStatus,
}

impl L1TxEntry {
    /// Create a new [`L1TxEntry`] from a [`Transaction`].
    pub fn from_tx(tx: &Transaction) -> Self {
        Self {
            tx_raw: serialize(tx),
            status: L1TxStatus::Unpublished,
        }
    }

    /// Returns the raw serialized transaction.
    ///
    /// # Note
    ///
    /// Whenever possible use [`try_to_tx()`](L1TxEntry::try_to_tx) to deserialize the transaction.
    /// This imposes more strict type checks.
    #[expect(clippy::missing_const_for_fn)]
    pub fn tx_raw(&self) -> &[u8] {
        &self.tx_raw
    }

    /// Deserializes the raw transaction into a [`Transaction`].
    pub fn try_to_tx(&self) -> Result<Transaction, consensus::encode::Error> {
        deserialize(&self.tx_raw)
    }

    pub const fn is_valid(&self) -> bool {
        !matches!(self.status, L1TxStatus::InvalidInputs)
    }

    pub const fn is_finalized(&self) -> bool {
        matches!(self.status, L1TxStatus::Finalized { .. })
    }
}

/// The possible statuses of a publishable transaction
#[derive(
    Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary, Serialize, Deserialize,
)]
#[serde(tag = "status")]
pub enum L1TxStatus {
    /// The transaction is waiting to be published
    Unpublished,

    /// The transaction is published
    Published,

    /// The transaction is included in L1 and has `u64` confirmations
    // FIXME this doesn't make sense to be "confirmations"
    Confirmed { confirmations: u64 },

    /// The transaction is finalized in L1 and has `u64` confirmations
    // FIXME this doesn't make sense to be "confirmations"
    Finalized { confirmations: u64 },

    /// The transaction is not included in L1 because it's inputs were invalid
    InvalidInputs,
}

/// Entry corresponding to a BatchCommitment
#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub struct CheckpointEntry {
    /// The batch checkpoint containing metadata, state transitions, and proof data.
    pub checkpoint: Checkpoint,

    /// Proving Status
    pub proving_status: CheckpointProvingStatus,

    /// Confirmation Status
    pub confirmation_status: CheckpointConfStatus,
}

impl CheckpointEntry {
    #[expect(clippy::missing_const_for_fn)]
    pub fn new(
        checkpoint: Checkpoint,
        proving_status: CheckpointProvingStatus,
        confirmation_status: CheckpointConfStatus,
    ) -> Self {
        Self {
            checkpoint,
            proving_status,
            confirmation_status,
        }
    }

    pub fn into_batch_checkpoint(self) -> Checkpoint {
        self.checkpoint
    }

    /// Creates a new instance for a freshly defined checkpoint.
    pub fn new_pending_proof(
        info: BatchInfo,
        transition: BatchTransition,
        chainstate: &Chainstate,
    ) -> Self {
        let sidecar =
            CheckpointSidecar::new(borsh::to_vec(chainstate).expect("serialize chainstate"));
        let checkpoint = Checkpoint::new(info, transition, Proof::default(), sidecar);
        Self::new(
            checkpoint,
            CheckpointProvingStatus::PendingProof,
            CheckpointConfStatus::Pending,
        )
    }

    pub fn is_proof_ready(&self) -> bool {
        self.proving_status == CheckpointProvingStatus::ProofReady
    }
}

impl From<CheckpointEntry> for Checkpoint {
    fn from(entry: CheckpointEntry) -> Checkpoint {
        entry.into_batch_checkpoint()
    }
}

/// Status of the commmitment
#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub enum CheckpointProvingStatus {
    /// Proof has not been created for this checkpoint
    PendingProof,
    /// Proof is ready
    ProofReady,
}

#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub enum CheckpointConfStatus {
    /// Pending to be posted on L1
    Pending,
    /// Confirmed on L1, with reference.
    Confirmed(CheckpointL1Ref),
    /// Finalized on L1, with reference
    Finalized(CheckpointL1Ref),
}

#[cfg(test)]
mod tests {
    use serde_json;

    use super::*;

    #[test]
    fn check_serde_of_l1txstatus() {
        let test_cases: Vec<(L1TxStatus, &str)> = vec![
            (L1TxStatus::Unpublished, r#"{"status":"Unpublished"}"#),
            (L1TxStatus::Published, r#"{"status":"Published"}"#),
            (
                L1TxStatus::Confirmed { confirmations: 10 },
                r#"{"status":"Confirmed","confirmations":10}"#,
            ),
            (
                L1TxStatus::Finalized { confirmations: 100 },
                r#"{"status":"Finalized","confirmations":100}"#,
            ),
            (L1TxStatus::InvalidInputs, r#"{"status":"InvalidInputs"}"#),
        ];

        // check serialization and deserialization
        for (l1_tx_status, serialized) in test_cases {
            let actual = serde_json::to_string(&l1_tx_status).unwrap();
            assert_eq!(actual, serialized);

            let actual: L1TxStatus = serde_json::from_str(serialized).unwrap();
            assert_eq!(actual, l1_tx_status);
        }
    }
}
