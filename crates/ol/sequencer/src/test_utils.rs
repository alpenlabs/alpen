//! Storage-backed checkpoint history for sequencer duty and submission tests.

use std::sync::{Arc, Mutex};

use strata_asm_checkpoint_types::{
    CheckpointPayload, CheckpointSidecar, CheckpointTip, TerminalHeaderComplement,
};
use strata_checkpoint_types::EpochSummary;
use strata_db_store_sled::test_utils::get_test_sled_backend;
use strata_db_types::{DbError, DbResult};
use strata_identifiers::{Buf32, Epoch, EpochCommitment, L1BlockCommitment, OLBlockCommitment};
use strata_ol_canonical_chain_provider::{OLCanonicalChainProvider, OLCanonicalChainProviderImpl};
use strata_ol_chain_types_v1::{BlockFlagsV1, OLBlockHeaderV1};
use strata_storage::{create_node_storage, test_runtime_handle, NodeStorage};

use crate::extraction::get_earliest_unsigned_checkpoint;
use crate::{CheckpointDutyContext, Error};

/// Seeds genesis, then persists linked one-block epochs through the storage managers.
#[expect(
    missing_debug_implementations,
    reason = "storage managers do not implement Debug"
)]
pub struct CheckpointFixture {
    storage: Arc<NodeStorage>,
    provider: OLCanonicalChainProviderImpl,
    genesis: EpochCommitment,
    reorg_after_payload: Mutex<Option<EpochCommitment>>,
    fail_canonical_read: Mutex<Option<usize>>,
}

impl CheckpointFixture {
    /// Creates a fixed epoch-zero anchor without an unsigned payload or signing duty.
    #[expect(
        clippy::new_without_default,
        reason = "creates isolated test storage rather than a domain default"
    )]
    pub fn new() -> Self {
        let storage =
            Arc::new(create_node_storage(get_test_sled_backend(), test_runtime_handle()).unwrap());
        let header = terminal_header(0, 0, OLBlockCommitment::null(), 0);
        let genesis = store_summary(&storage, header, OLBlockCommitment::null());
        let fixture = Self {
            provider: OLCanonicalChainProviderImpl::new(&storage),
            storage,
            genesis,
            reorg_after_payload: Mutex::new(None),
            fail_canonical_read: Mutex::new(None),
        };
        fixture.publish(genesis);
        fixture
    }

    pub fn storage(&self) -> &NodeStorage {
        &self.storage
    }
    pub fn genesis(&self) -> EpochCommitment {
        self.genesis
    }

    /// Runs checkpoint selection against the fixture's stored history.
    pub fn get_earliest_unsigned_checkpoint_blocking(
        &self,
    ) -> Result<Option<CheckpointPayload>, Error> {
        test_runtime_handle().block_on(get_earliest_unsigned_checkpoint(self))
    }

    /// Stores a completed epoch on the supplied predecessor, before fork choice or payload creation.
    pub fn extend(&self, previous: EpochCommitment, branch: u8) -> EpochCommitment {
        assert!(self
            .storage
            .ol_checkpoint()
            .get_epoch_summary_blocking(previous)
            .unwrap()
            .is_some());
        let header = terminal_header(
            previous.epoch() + 1,
            previous.last_slot() + 1,
            previous.to_block_commitment(),
            branch,
        );
        store_summary(&self.storage, header, previous.to_block_commitment())
    }

    /// Applies fork choice, retaining the prefix and replacing the selected suffix.
    pub fn publish(&self, commitment: EpochCommitment) {
        self.storage
            .ol_block()
            .replace_canonical_suffix_from_blocking(
                commitment.last_slot(),
                vec![*commitment.last_blkid()],
            )
            .unwrap();
    }

    /// Stores a payload whose tip and header complement match the completed epoch.
    pub fn store_payload(&self, commitment: EpochCommitment) -> CheckpointPayload {
        let summary = self
            .storage
            .ol_checkpoint()
            .get_epoch_summary_blocking(commitment)
            .unwrap()
            .unwrap();
        let header = self
            .storage
            .ol_block()
            .get_ol_header_blocking(*commitment.last_blkid())
            .unwrap()
            .unwrap();
        // DA and proof bytes are opaque to signing; these tests do not replay or verify them.
        let sidecar = CheckpointSidecar::new(
            vec![1],
            vec![],
            TerminalHeaderComplement::new(
                header.timestamp(),
                *header.parent_blkid(),
                *header.body_root(),
                *header.logs_root(),
            ),
        )
        .unwrap();
        let payload = CheckpointPayload::new(
            CheckpointTip::new(
                commitment.epoch(),
                summary.new_l1().height(),
                *summary.terminal(),
            ),
            sidecar,
            vec![1],
        )
        .unwrap();
        self.storage
            .ol_checkpoint()
            .put_checkpoint_payload_entry_blocking(commitment, payload.clone())
            .unwrap();
        payload
    }

    pub fn record_submission(&self, commitment: EpochCommitment, intent_index: u64) {
        self.storage
            .ol_checkpoint()
            .put_checkpoint_signing_entry_blocking(commitment, intent_index)
            .unwrap();
    }

    /// Injects a fork-choice change after fetching the old payload and before the final check.
    pub fn reorg_after_payload(&self, replacement: EpochCommitment) {
        *self.reorg_after_payload.lock().unwrap() = Some(replacement);
    }

    /// Injects a storage failure after the specified number of successful canonical reads.
    pub fn fail_canonical_read_after(&self, successful_reads: usize) {
        *self.fail_canonical_read.lock().unwrap() = Some(successful_reads);
    }
}

fn terminal_header(
    epoch: Epoch,
    slot: u64,
    parent: OLBlockCommitment,
    branch: u8,
) -> OLBlockHeaderV1 {
    let mut flags = BlockFlagsV1::zero();
    flags.set_is_terminal(true);
    let root = Buf32::from([branch; 32]);
    OLBlockHeaderV1::new(
        1_000 + slot,
        flags,
        slot,
        epoch,
        *parent.blkid(),
        root,
        root,
        root,
    )
}

fn store_summary(
    storage: &NodeStorage,
    header: OLBlockHeaderV1,
    previous: OLBlockCommitment,
) -> EpochCommitment {
    let summary = EpochSummary::new(
        header.epoch(),
        header.compute_block_commitment(),
        previous,
        L1BlockCommitment::new(100 + header.epoch(), Buf32::from([0x10; 32]).into()),
        *header.state_root(),
    );
    storage
        .ol_block()
        .put_terminal_header_blocking(header.compute_blkid(), header)
        .unwrap();
    storage
        .ol_checkpoint()
        .insert_epoch_summary_blocking(summary)
        .unwrap();
    summary.get_epoch_commitment()
}

impl CheckpointDutyContext for CheckpointFixture {
    async fn get_next_unsigned_checkpoint_epoch(&self) -> DbResult<Option<Epoch>> {
        self.storage
            .ol_checkpoint()
            .get_next_unsigned_checkpoint_epoch_async()
            .await
    }
    async fn get_canonical_epoch_commitment_at(
        &self,
        epoch: Epoch,
    ) -> DbResult<Option<EpochCommitment>> {
        {
            let mut remaining = self.fail_canonical_read.lock().unwrap();
            match *remaining {
                Some(0) => {
                    *remaining = None;
                    return Err(DbError::Busy);
                }
                Some(n) => *remaining = Some(n - 1),
                None => {}
            }
        }
        self.provider
            .get_canonical_epoch_commitment_at_async(epoch)
            .await
    }
    async fn get_checkpoint_payload(
        &self,
        commitment: EpochCommitment,
    ) -> DbResult<Option<CheckpointPayload>> {
        let payload = self
            .storage
            .ol_checkpoint()
            .get_checkpoint_payload_entry_async(commitment)
            .await?;
        let replacement = self.reorg_after_payload.lock().unwrap().take();
        if let Some(replacement) = replacement {
            self.storage
                .ol_block()
                .replace_canonical_suffix_from_async(
                    replacement.last_slot(),
                    vec![*replacement.last_blkid()],
                )
                .await?;
        }
        Ok(payload)
    }
    async fn has_checkpoint_signing_entry(&self, commitment: EpochCommitment) -> DbResult<bool> {
        Ok(self
            .storage
            .ol_checkpoint()
            .get_checkpoint_signing_entry_async(commitment)
            .await?
            .is_some())
    }
}
