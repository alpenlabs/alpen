//! Bootstraps EE Sled metadata from verified reconstructed state.

use std::{fs::File, path::PathBuf};

use alloy_primitives::B256;
use alpen_ee_common::{
    Batch, BatchStatus, BatchStorage, ExecBlockPayload, ExecBlockRecord, ExecBlockStorage, Storage,
};
use alpen_ee_database::init_db_storage;
use anyhow::{anyhow, bail, Context, Error, Result};
use serde::Deserialize;
use strata_acct_types::Hash;
use strata_ee_acct_types::EeAccountState;
use strata_ee_chain_types::{ExecBlockCommitment, ExecBlockPackage, ExecInputs, ExecOutputs};
use strata_identifiers::{Buf32, EpochCommitment, OLBlockCommitment, OLBlockId};
use tokio::runtime::Builder;

use super::reconstruct::ReconstructedStateMetadata;

/// Bootstrap minimum EE metadata in a fresh Sled database.
#[derive(Debug, PartialEq)]
pub(super) struct SledBootstrapConfig {
    /// fresh EE datadir to initialize
    pub datadir: PathBuf,

    /// metadata emitted only after the reconstructed account state matches OL
    pub verified_metadata: PathBuf,

    /// earlier proof-backed state used by OL epochs before the reconstructed epoch
    pub base_verified_metadata: Option<PathBuf>,

    /// ordered OL epoch commitments and their proof-backed inner-state roots
    pub ol_epoch_history: Option<PathBuf>,

    /// ol epoch at the recovered tracker frontier
    pub ol_epoch: u32,

    /// ol slot at the recovered tracker frontier
    pub ol_slot: u64,

    /// ol block ID at the recovered tracker frontier
    pub ol_block_id: B256,

    /// ol block visible when the older finalized execution anchor was built
    pub finalized_anchor_ol_slot: Option<u64>,

    /// ol block ID visible when the older finalized execution anchor was built
    pub finalized_anchor_ol_block_id: Option<B256>,

    /// end block of the preceding accepted batch
    pub previous_batch_block_hash: B256,

    /// next OL inbox message index
    pub next_inbox_msg_idx: u64,

    /// next bridge deposit index
    pub next_deposit_idx: u64,
}

#[derive(Debug, Deserialize)]
struct EpochHistoryEntry {
    epoch: u32,
    last_slot: u64,
    last_blkid: B256,
    inner_state_root: B256,
}

fn hash(value: B256) -> Hash {
    value.0.into()
}

fn ol_block_id(value: B256) -> OLBlockId {
    OLBlockId::from(Buf32::new(value.0))
}

fn build_account_state(metadata: &ReconstructedStateMetadata) -> EeAccountState {
    EeAccountState::new(
        hash(metadata.last_exec_blkid),
        hash(metadata.last_exec_state_root),
        metadata
            .pending_inputs
            .iter()
            .cloned()
            .map(Into::into)
            .collect(),
        metadata
            .pending_fincls
            .iter()
            .cloned()
            .map(Into::into)
            .collect(),
    )
}

pub(super) fn bootstrap(args: SledBootstrapConfig) -> Result<()> {
    let metadata: ReconstructedStateMetadata =
        serde_json::from_reader(File::open(&args.verified_metadata).with_context(|| {
            format!(
                "opening verified state metadata {}",
                args.verified_metadata.display()
            )
        })?)?;
    if !metadata.account_state_verified {
        bail!("state metadata has not passed the inner_state_root check");
    }
    let base_metadata: Option<ReconstructedStateMetadata> =
        if let Some(path) = &args.base_verified_metadata {
            Some(serde_json::from_reader(File::open(path)?)?)
        } else {
            None
        };
    if let Some(base) = &base_metadata {
        if !base.account_state_verified {
            bail!("base state metadata has not passed the inner_state_root check");
        }
    }
    let timestamp_ms = metadata
        .evm_header
        .timestamp
        .checked_mul(1000)
        .context("recovered timestamp overflow")?;
    let runtime = Builder::new_multi_thread().enable_all().build()?;
    let databases = init_db_storage(&args.datadir, 5).map_err(|err| anyhow!("{err:#}"))?;
    let storage = databases.node_storage(runtime.handle().clone());

    runtime.block_on(async move {
        let block_hash = hash(metadata.last_exec_blkid);
        let account_state = build_account_state(&metadata);
        let epoch =
            EpochCommitment::new(args.ol_epoch, args.ol_slot, ol_block_id(args.ol_block_id));
        let ol_block = OLBlockCommitment::new(args.ol_slot, ol_block_id(args.ol_block_id));
        let package = ExecBlockPackage::new(
            ExecBlockCommitment::new(block_hash, Hash::zero()),
            ExecInputs::new_empty(),
            ExecOutputs::new_empty(),
        );
        let block = ExecBlockRecord::new(
            package,
            account_state.clone(),
            metadata.evm_header.block_num,
            ol_block,
            timestamp_ms,
            // The reconstructed finalized block is the beginning of the locally
            // available sparse history, so it intentionally has no local parent.
            Hash::zero(),
            args.next_inbox_msg_idx,
            args.next_deposit_idx,
            Vec::new(),
        );

        if let Some(history_path) = &args.ol_epoch_history {
            let base = base_metadata
                .as_ref()
                .context("--base-verified-metadata is required with --ol-epoch-history")?;
            let base_account_state = build_account_state(base);
            let history: Vec<EpochHistoryEntry> =
                serde_json::from_reader(File::open(history_path)?)?;
            for entry in history {
                let state = if entry.inner_state_root == base.inner_state_root {
                    base_account_state.clone()
                } else if entry.inner_state_root == metadata.inner_state_root {
                    account_state.clone()
                } else {
                    bail!(
                        "epoch {} has unverified inner_state_root {}",
                        entry.epoch,
                        entry.inner_state_root
                    );
                };
                let commitment = EpochCommitment::new(
                    entry.epoch,
                    entry.last_slot,
                    ol_block_id(entry.last_blkid),
                );
                storage
                    .store_ee_account_state(&commitment, &state)
                    .await
                    .with_context(|| {
                        format!("storing recovered EE state at epoch {}", entry.epoch)
                    })?;
            }
            let existing = storage
                .best_ee_account_state()
                .await?
                .context("epoch history did not create an EE account-state frontier")?;
            if existing.epoch_commitment() != &epoch
                || (existing.ee_state() != &base_account_state
                    && existing.ee_state() != &account_state)
            {
                bail!("epoch history does not end at the requested tracker frontier");
            }
        } else if let Some(existing) = storage.best_ee_account_state().await? {
            if existing.epoch_commitment() != &epoch || existing.ee_state() != &account_state {
                bail!("Sled already contains a different EE account-state frontier");
            }
        } else {
            storage
                .store_ee_account_state(&epoch, &account_state)
                .await
                .context("storing recovered EE account state")?;
        }

        if let Some(existing) = storage.get_exec_block(block_hash).await? {
            if existing.blocknum() != metadata.evm_header.block_num
                || existing.account_state() != &account_state
            {
                bail!("Sled already contains different data for the reconstructed block");
            }
        } else {
            storage
                .save_exec_block(block, ExecBlockPayload::from_bytes(Vec::new()))
                .await
                .context("storing recovered execution block")?;
        }
        let finalized_anchor = if let Some(base) = &base_metadata {
            let anchor_ol_slot = args
                .finalized_anchor_ol_slot
                .context("--finalized-anchor-ol-slot is required with base metadata")?;
            let anchor_ol_block_id = args
                .finalized_anchor_ol_block_id
                .context("--finalized-anchor-ol-block-id is required with base metadata")?;
            let anchor_hash = hash(base.last_exec_blkid);
            let anchor_record = ExecBlockRecord::new(
                ExecBlockPackage::new(
                    ExecBlockCommitment::new(anchor_hash, Hash::zero()),
                    ExecInputs::new_empty(),
                    ExecOutputs::new_empty(),
                ),
                build_account_state(base),
                base.evm_header.block_num,
                OLBlockCommitment::new(anchor_ol_slot, ol_block_id(anchor_ol_block_id)),
                base.evm_header
                    .timestamp
                    .checked_mul(1000)
                    .context("base timestamp overflow")?,
                // This is also a sparse finalized-history boundary.
                Hash::zero(),
                0,
                0,
                Vec::new(),
            );
            if storage.get_exec_block(anchor_hash).await?.is_none() {
                storage
                    .save_exec_block(anchor_record, ExecBlockPayload::from_bytes(Vec::new()))
                    .await
                    .context("storing older finalized execution anchor")?;
            }
            anchor_hash
        } else {
            block_hash
        };
        storage
            .init_finalized_chain(finalized_anchor)
            .await
            .context("initializing sparse finalized execution chain")?;

        let batch_idx = metadata
            .update_seq_no
            .checked_add(1)
            .context("reconstructed batch index overflow")?;
        let batch = Batch::new(
            batch_idx,
            hash(args.previous_batch_block_hash),
            block_hash,
            metadata.evm_header.block_num,
            Vec::new(),
        )
        .map_err(Error::msg)?;
        if let Some((existing, status)) = storage.get_latest_batch().await? {
            if existing != batch || !matches!(status, BatchStatus::Genesis) {
                bail!("Sled already contains a different batch frontier");
            }
        } else {
            // The storage API calls the first retained batch "genesis". In a
            // reconstructed database this entry is the trusted sparse anchor.
            storage
                .save_genesis_batch(batch)
                .await
                .context("storing recovered batch frontier")?;
        }

        println!(
            "bootstrapped recovered EE block {} ({}) at OL epoch {}",
            metadata.evm_header.block_num, metadata.last_exec_blkid, args.ol_epoch
        );
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use strata_acct_types::{BitcoinAmount, SubjectId};
    use strata_ee_acct_types::PendingInputEntry;

    use super::*;
    use crate::cmd::ee_state_recovery::reconstruct::{HeaderMetadata, PendingInputMetadata};

    #[test]
    fn builds_account_state_with_reconstructed_pending_inputs() {
        let metadata = ReconstructedStateMetadata {
            update_seq_no: 3,
            last_exec_blkid: B256::from([1; 32]),
            last_exec_state_root: B256::from([2; 32]),
            inner_state_root: B256::from([3; 32]),
            pending_inputs: vec![PendingInputMetadata::Deposit {
                destination: B256::from([4; 32]),
                value_sats: 25,
            }],
            pending_fincls: Vec::new(),
            account_state_verified: true,
            evm_header: HeaderMetadata {
                block_num: 10,
                timestamp: 20,
                base_fee: 30,
                gas_used: 40,
                gas_limit: 50,
            },
        };

        let account_state = build_account_state(&metadata);

        assert_eq!(account_state.pending_inputs().len(), 1);
        let PendingInputEntry::Deposit(deposit) = &account_state.pending_inputs()[0];
        assert_eq!(deposit.dest(), SubjectId::new([4; 32]));
        assert_eq!(deposit.value(), BitcoinAmount::from_sat(25));
    }
}
