//! Native console views that preserve cross-table read semantics in Rust.

use std::sync::Arc;

use strata_db_console::{
    ArgumentDescriptor, ConsoleError, ConsoleResult, ConsoleScalar, ConsoleView, RecordHandle,
    RegisteredConsoleValue, ScalarType, ValueMetadata,
};
use strata_db_types::ol_block::{BlockStatus, OLBlockDatabase};

use super::parse_ol_block_id;
use crate::SledBackend;
use crate::ol::db::OLBlockDBSled;

const OL_BLOCK_VIEW_ARGUMENTS: &[ArgumentDescriptor] = &[ArgumentDescriptor {
    name: "block_id",
    scalar_type: ScalarType::Bytes,
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

pub(super) fn console_view(backend: &SledBackend) -> Arc<dyn ConsoleView> {
    Arc::new(OlBlockView {
        database: backend.ol_block_db.clone(),
    })
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
