//! L1 writer database interface and its payload/intent record types.

use std::fmt;

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_csm_types::{L1Payload, PayloadIntent};
#[cfg(feature = "proxies")]
use strata_db_macros::gen_proxy;
use strata_identifiers::{Buf32, Buf64};

use crate::common::L1TxId;
#[cfg(feature = "proxies")]
use crate::DbError;
use crate::DbResult;

/// Taproot script-spend sighash for the reveal transaction.
pub type Sighash = Buf32;

/// Represents an intent to publish to some DA, which will be bundled for efficiency.
#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub struct IntentEntry {
    pub intent: PayloadIntent,
    pub status: IntentStatus,
}

impl IntentEntry {
    pub fn new_unbundled(intent: PayloadIntent) -> Self {
        Self {
            intent,
            status: IntentStatus::Unbundled,
        }
    }

    pub fn new_bundled(intent: PayloadIntent, bundle_idx: u64) -> Self {
        Self {
            intent,
            status: IntentStatus::Bundled(bundle_idx),
        }
    }

    pub fn payload(&self) -> &L1Payload {
        self.intent.payload()
    }
}

/// Status of Intent indicating various stages of being bundled to L1 transaction.
/// Unbundled Intents are collected and bundled to create [`BundledPayloadEntry`].
#[derive(Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub enum IntentStatus {
    /// The intent has not been bundled yet.
    Unbundled,
    /// The intent has been bundled into the [`BundledPayloadEntry`] at the given index.
    Bundled(u64),
    /// Reconciliation abandoned the intent before it could publish.
    ///
    /// Intent entries are keyed by commitment, while intent indices reference that shared entry.
    /// A later submission with the same commitment allocates a fresh index and refreshes the
    /// shared entry to [`IntentStatus::Unbundled`], so older indices then resolve to the refreshed
    /// state as well. The bundler still creates the payload exactly once because it skips the
    /// remaining aliases after the shared entry becomes [`IntentStatus::Bundled`].
    Abandoned,
}

/// Represents data for a payload we're still planning to post to L1.
#[derive(Clone, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub struct BundledPayloadEntry {
    pub payload: L1Payload,
    pub commit_txid: L1TxId,
    pub reveal_txid: L1TxId,
    pub status: L1BundleStatus,
    /// Schnorr signature provided by the external signer for envelope reveal tx.
    ///
    /// Populated when the signer calls `complete_payload_signature` RPC.
    pub payload_signature: Option<Buf64>,
}

impl BundledPayloadEntry {
    pub fn new(
        payload: L1Payload,
        commit_txid: L1TxId,
        reveal_txid: L1TxId,
        status: L1BundleStatus,
    ) -> Self {
        Self {
            payload,
            commit_txid,
            reveal_txid,
            status,
            payload_signature: None,
        }
    }

    /// Create new unsigned [`BundledPayloadEntry`].
    ///
    /// NOTE: This won't have commit - reveal pairs associated with it.
    ///   Because it is better to defer gathering utxos as late as possible to prevent being spent
    ///   by others. Those will be created and signed in a single step.
    pub fn new_unsigned(payload: L1Payload) -> Self {
        let cid = L1TxId::zero();
        let rid = L1TxId::zero();
        Self::new(payload, cid, rid, L1BundleStatus::Unsigned)
    }
}

// Custom debug implementation to print commit_txid and reveal_txid in Bitcoin order.
impl fmt::Debug for BundledPayloadEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let commit_txid = format!("{:?}", self.commit_txid);
        let reveal_txid = format!("{:?}", self.reveal_txid);

        f.debug_struct("BundledPayloadEntry")
            .field("payload", &self.payload)
            .field("commit_txid", &commit_txid)
            .field("reveal_txid", &reveal_txid)
            .field("status", &self.status)
            .finish()
    }
}

// Custom display implementation to print commit_txid and reveal_txid in Bitcoin order.
impl fmt::Display for BundledPayloadEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "BundledPayloadEntry {{ payload: {:?}, commit_txid: {:?}, reveal_txid: {:?}, status: {:?} }}",
            self.payload, self.commit_txid, self.reveal_txid, self.status
        )
    }
}

/// Various status that transactions corresponding to a payload can be in L1
#[derive(
    Debug, Clone, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary, Serialize, Deserialize,
)]
pub enum L1BundleStatus {
    /// The payload has not been signed yet, i.e commit-reveal transactions have not been created
    /// yet.
    Unsigned,

    /// The envelope has been built and the commit tx signed; waiting for the external signer to
    /// provide a Schnorr signature for the reveal tx sighash.
    PendingRevealTxSign(Sighash),

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

    /// The payload was abandoned before its transactions could publish.
    ///
    /// This terminal state preserves the payload index so the sequential watcher
    /// can advance without leaving a database gap.
    Abandoned,
}

/// Encapsulates provider and store traits to create/update [`BundledPayloadEntry`] in the
/// database and to fetch [`BundledPayloadEntry`] and indices from the database
#[cfg_attr(
    feature = "proxies",
    gen_proxy(error = DbError, tracing_component = "storage:l1_writer")
)]
pub trait L1WriterDatabase: Send + Sync + 'static {
    /// Store the [`BundledPayloadEntry`].
    fn put_payload_entry(&self, idx: u64, payloadentry: BundledPayloadEntry) -> DbResult<()>;

    /// Get a [`BundledPayloadEntry`] by its index.
    fn get_payload_entry_by_idx(&self, idx: u64) -> DbResult<Option<BundledPayloadEntry>>;

    /// Get the next payload index
    fn get_next_payload_idx(&self) -> DbResult<u64>;

    /// Delete a specific payload entry by its index.
    ///
    /// Returns true if the payload existed and was deleted, false otherwise.
    fn del_payload_entry(&self, idx: u64) -> DbResult<bool>;

    /// Delete payload entries from the specified index onwards (inclusive).
    ///
    /// This method deletes all payload entries with index >= start_idx.
    /// Returns a vector of deleted payload indices.
    fn del_payload_entries_from_idx(&self, start_idx: u64) -> DbResult<Vec<u64>>;

    /// Store the [`IntentEntry`].
    fn put_intent_entry(&self, payloadid: Buf32, payloadentry: IntentEntry) -> DbResult<u64>;

    /// Updates an existing [`IntentEntry`] without allocating another index.
    fn update_intent_entry(&self, payloadid: Buf32, payloadentry: IntentEntry) -> DbResult<()>;

    /// Atomically stores a payload entry and marks an existing intent as bundled.
    ///
    /// Returns the payload index allocated for the new [`BundledPayloadEntry`].
    fn bundle_intent_payload(
        &self,
        intent_id: Buf32,
        intent_entry: IntentEntry,
        payloadentry: BundledPayloadEntry,
    ) -> DbResult<u64>;

    /// Get a [`IntentEntry`] by its hash
    fn get_intent_by_id(&self, id: Buf32) -> DbResult<Option<IntentEntry>>;

    /// Get a [`IntentEntry`] by its idx
    fn get_intent_by_idx(&self, idx: u64) -> DbResult<Option<IntentEntry>>;

    /// Get  the next intent index
    fn get_next_intent_idx(&self) -> DbResult<u64>;

    /// Delete a specific intent entry by its ID.
    ///
    /// Returns true if the intent existed and was deleted, false otherwise.
    fn del_intent_entry(&self, id: Buf32) -> DbResult<bool>;

    /// Delete intent entries from the specified index onwards (inclusive).
    ///
    /// This method deletes all intent entries with index >= start_idx.
    /// Returns a vector of deleted intent indices.
    fn del_intent_entries_from_idx(&self, start_idx: u64) -> DbResult<Vec<u64>>;
}

#[cfg(test)]
mod tests {
    use strata_l1_txfmt::TagData;

    use super::*;

    fn bytes_from_start(start: u8) -> [u8; 32] {
        let mut bytes = [0u8; 32];
        for (idx, byte) in bytes.iter_mut().enumerate() {
            *byte = start.wrapping_add(idx as u8);
        }
        bytes
    }

    fn reversed_hex(bytes: [u8; 32]) -> String {
        bytes
            .into_iter()
            .rev()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    #[test]
    fn bundled_payload_entry_formats_full_reversed_txids() {
        let commit_bytes = bytes_from_start(0x10);
        let reveal_bytes = bytes_from_start(0x40);
        let payload =
            L1Payload::new(vec![vec![1, 2, 3]], TagData::new(1, 1, vec![]).unwrap()).unwrap();
        let entry = BundledPayloadEntry::new(
            payload,
            L1TxId::from(commit_bytes),
            L1TxId::from(reveal_bytes),
            L1BundleStatus::Unpublished,
        );

        let display = entry.to_string();
        let debug = format!("{entry:?}");
        let expected_commit = reversed_hex(commit_bytes);
        let expected_reveal = reversed_hex(reveal_bytes);

        assert!(display.contains(&expected_commit));
        assert!(display.contains(&expected_reveal));
        assert!(!display.contains(".."));
        assert!(debug.contains(&expected_commit));
        assert!(debug.contains(&expected_reveal));
        assert!(!debug.contains(".."));
    }
}
