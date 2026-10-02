//! Native console views that preserve cross-table read semantics in Rust.

use std::sync::Arc;

use strata_db_console::{
    ArgumentDescriptor, ConsoleError, ConsoleResult, ConsoleScalar, ConsoleView, RecordHandle,
    RegisteredConsoleValue, ScalarType, ValueMetadata,
};
use strata_db_types::checkpoint_status::{
    read_canonical_epoch_commitment, read_checkpoint_status, read_latest_finalized_checkpoint_epoch,
};
use strata_db_types::l1::L1Database;
use strata_db_types::ol_block::{BlockStatus, OLBlockDatabase};
use strata_db_types::ol_state::OLStateDatabase;
use strata_identifiers::{EpochCommitment, OLBlockCommitment};

use super::parse_ol_block_id;
use crate::SledBackend;
use crate::l1::db::L1DBSled;
use crate::ol::db::OLBlockDBSled;
use crate::ol_checkpoint::db::OLCheckpointDBSled;
use crate::ol_state::db::OLStateDBSled;

const OL_BLOCK_VIEW_ARGUMENTS: &[ArgumentDescriptor] = &[ArgumentDescriptor {
    name: "block_id",
    scalar_type: ScalarType::Bytes,
}];

const SYNC_INFO_ARGUMENTS: &[ArgumentDescriptor] = &[ArgumentDescriptor {
    name: "l1_reorg_safe_depth",
    scalar_type: ScalarType::U64,
}];

#[derive(strata_db_console::ConsoleValue)]
struct OlBlockViewValue {
    #[console(get)]
    status: String,
    #[console(get)]
    slot: u64,
    #[console(get)]
    epoch: u64,
    #[console(get)]
    timestamp: u64,
    #[console(get)]
    parent_block_id: Vec<u8>,
    #[console(get)]
    body_root: Vec<u8>,
    #[console(get)]
    logs_root: Vec<u8>,
    #[console(get)]
    state_root: Vec<u8>,
}

#[derive(Debug)]
struct OlBlockView {
    database: Arc<OLBlockDBSled>,
}

impl ConsoleView for OlBlockView {
    fn name(&self) -> &'static str {
        "OlBlock"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["ol_block"]
    }

    fn arguments(&self) -> &'static [ArgumentDescriptor] {
        OL_BLOCK_VIEW_ARGUMENTS
    }

    fn value_metadata(&self) -> &'static ValueMetadata {
        OlBlockViewValue::value_metadata()
    }

    fn get(&self, arguments: &[ConsoleScalar]) -> ConsoleResult<Option<RecordHandle>> {
        expect_argument_count(self.name(), arguments, 1)?;
        let key = arguments[0].clone();
        let block_id = parse_ol_block_id(&key)?;
        let Some(block) = self
            .database
            .get_block_data(block_id)
            .map_err(|error| ConsoleError::storage(self.name(), error))?
        else {
            return Ok(None);
        };
        let status = self
            .database
            .get_block_status(block_id)
            .map_err(|error| ConsoleError::storage(self.name(), error))?
            .unwrap_or(BlockStatus::Unchecked);
        let header = block.header();
        let value = OlBlockViewValue {
            status: block_status_name(status).to_owned(),
            slot: header.slot(),
            epoch: u64::from(header.epoch()),
            timestamp: header.timestamp(),
            parent_block_id: header.parent_blkid().as_ref().to_vec(),
            body_root: header.body_root().as_ref().to_vec(),
            logs_root: header.logs_root().as_ref().to_vec(),
            state_root: header.state_root().as_ref().to_vec(),
        };
        Ok(Some(RecordHandle::new(self.name(), key, value)))
    }
}

#[derive(strata_db_console::ConsoleValue)]
struct SyncInfoValue {
    #[console(get)]
    l1_tip_height: u64,
    #[console(get)]
    l1_tip_block_id: Vec<u8>,
    #[console(get)]
    ol_tip_slot: u64,
    #[console(get)]
    ol_tip_block_id: Vec<u8>,
    #[console(get)]
    ol_tip_status: String,
    #[console(get)]
    current_epoch: u64,
    #[console(get)]
    current_slot: u64,
    #[console(get)]
    previous_block_slot: u64,
    #[console(get)]
    previous_block_id: Vec<u8>,
    #[console(get)]
    previous_epoch: u64,
    #[console(get)]
    previous_epoch_last_slot: u64,
    #[console(get)]
    previous_epoch_last_block_id: Vec<u8>,
    #[console(get)]
    previous_epoch_status: Option<String>,
    #[console(get)]
    finalized_epoch: u64,
    #[console(get)]
    finalized_epoch_last_slot: u64,
    #[console(get)]
    finalized_epoch_last_block_id: Vec<u8>,
    #[console(get)]
    safe_l1_height: u64,
    #[console(get)]
    safe_l1_block_id: Vec<u8>,
}

#[derive(Debug)]
struct SyncInfoView {
    l1_database: Arc<L1DBSled>,
    ol_block_database: Arc<OLBlockDBSled>,
    ol_state_database: Arc<OLStateDBSled>,
    checkpoint_database: Arc<OLCheckpointDBSled>,
}

impl ConsoleView for SyncInfoView {
    fn name(&self) -> &'static str {
        "SyncInfo"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["sync_info"]
    }

    fn arguments(&self) -> &'static [ArgumentDescriptor] {
        SYNC_INFO_ARGUMENTS
    }

    fn value_metadata(&self) -> &'static ValueMetadata {
        SyncInfoValue::value_metadata()
    }

    fn get(&self, arguments: &[ConsoleScalar]) -> ConsoleResult<Option<RecordHandle>> {
        expect_argument_count(self.name(), arguments, 1)?;
        let l1_reorg_safe_depth =
            u32::try_from(arguments[0].as_u64("SyncInfo argument 'l1_reorg_safe_depth'")?)
                .map_err(|_| {
                    ConsoleError::invalid_input(
                        "SyncInfo argument 'l1_reorg_safe_depth'",
                        "value exceeds u32",
                    )
                })?;

        let (l1_tip_height, l1_tip_block_id) = self
            .l1_database
            .get_canonical_chain_tip()
            .map_err(|error| ConsoleError::storage(self.name(), error))?
            .ok_or_else(|| ConsoleError::read(self.name(), "L1 canonical tip is missing"))?;
        let ol_tip_slot = self
            .ol_block_database
            .get_tip_slot()
            .map_err(|error| ConsoleError::storage(self.name(), error))?;
        let ol_tip_block_id = self
            .ol_block_database
            .get_canonical_block(ol_tip_slot)
            .map_err(|error| ConsoleError::storage(self.name(), error))?
            .ok_or_else(|| ConsoleError::read(self.name(), "canonical OL tip is missing"))?;
        let ol_tip_commitment = OLBlockCommitment::new(ol_tip_slot, ol_tip_block_id);
        let ol_tip_status = self
            .ol_block_database
            .get_block_status(ol_tip_block_id)
            .map_err(|error| ConsoleError::storage(self.name(), error))?
            .unwrap_or(BlockStatus::Unchecked);
        let ol_tip_block = self
            .ol_block_database
            .get_block_data(ol_tip_block_id)
            .map_err(|error| ConsoleError::storage(self.name(), error))?
            .ok_or_else(|| ConsoleError::read(self.name(), "canonical OL tip block is missing"))?;
        let state = self
            .ol_state_database
            .get_toplevel_ol_state(ol_tip_commitment)
            .map_err(|error| ConsoleError::storage(self.name(), error))?
            .ok_or_else(|| ConsoleError::read(self.name(), "canonical OL tip state is missing"))?;

        let chainstate = state.chainstate();
        let current_epoch = chainstate.cur_epoch();
        let previous_epoch_number = current_epoch.saturating_sub(1);
        let previous_epoch = if previous_epoch_number == 0 {
            EpochCommitment::null()
        } else {
            read_canonical_epoch_commitment(
                self.checkpoint_database.as_ref(),
                previous_epoch_number,
            )
            .map_err(|error| ConsoleError::storage(self.name(), error))?
            .ok_or_else(|| {
                ConsoleError::read(self.name(), "previous epoch commitment is missing")
            })?
        };
        let previous_epoch_status = read_checkpoint_status(
            self.checkpoint_database.as_ref(),
            previous_epoch_number,
            l1_tip_height,
            l1_reorg_safe_depth,
        )
        .map_err(|error| ConsoleError::storage(self.name(), error))?
        .map(|status| status.as_str().to_owned());
        let finalized_epoch = read_latest_finalized_checkpoint_epoch(
            self.checkpoint_database.as_ref(),
            l1_tip_height,
            l1_reorg_safe_depth,
        )
        .map_err(|error| ConsoleError::storage(self.name(), error))?
        .unwrap_or_else(EpochCommitment::null);
        let previous_block_slot = ol_tip_block.header().slot().saturating_sub(1);
        let safe_block = chainstate.last_l1_block();

        let value = SyncInfoValue {
            l1_tip_height: u64::from(l1_tip_height),
            l1_tip_block_id: l1_tip_block_id.as_ref().to_vec(),
            ol_tip_slot,
            ol_tip_block_id: ol_tip_block_id.as_ref().to_vec(),
            ol_tip_status: block_status_name(ol_tip_status).to_owned(),
            current_epoch: u64::from(current_epoch),
            current_slot: chainstate.cur_slot(),
            previous_block_slot,
            previous_block_id: ol_tip_block.header().parent_blkid().as_ref().to_vec(),
            previous_epoch: u64::from(previous_epoch.epoch()),
            previous_epoch_last_slot: previous_epoch.last_slot(),
            previous_epoch_last_block_id: previous_epoch.last_blkid().as_ref().to_vec(),
            previous_epoch_status,
            finalized_epoch: u64::from(finalized_epoch.epoch()),
            finalized_epoch_last_slot: finalized_epoch.last_slot(),
            finalized_epoch_last_block_id: finalized_epoch.last_blkid().as_ref().to_vec(),
            safe_l1_height: u64::from(safe_block.height()),
            safe_l1_block_id: safe_block.blkid().as_ref().to_vec(),
        };
        Ok(Some(RecordHandle::new(
            self.name(),
            ConsoleScalar::Null,
            value,
        )))
    }
}

pub(super) fn console_views(backend: &SledBackend) -> Vec<Arc<dyn ConsoleView>> {
    vec![
        Arc::new(OlBlockView {
            database: backend.ol_block_db.clone(),
        }),
        Arc::new(SyncInfoView {
            l1_database: backend.l1_db.clone(),
            ol_block_database: backend.ol_block_db.clone(),
            ol_state_database: backend.ol_state_db.clone(),
            checkpoint_database: backend.ol_checkpoint_db.clone(),
        }),
    ]
}

fn expect_argument_count(
    source: &'static str,
    arguments: &[ConsoleScalar],
    expected: usize,
) -> ConsoleResult<()> {
    if arguments.len() == expected {
        Ok(())
    } else {
        Err(ConsoleError::invalid_input(
            "view arguments",
            format!(
                "view '{source}' expected {expected} arguments, got {}",
                arguments.len()
            ),
        ))
    }
}

const fn block_status_name(status: BlockStatus) -> &'static str {
    match status {
        BlockStatus::Unchecked => "unchecked",
        BlockStatus::Valid => "valid",
        BlockStatus::Invalid => "invalid",
    }
}
