use std::collections::HashMap;

use strata_identifiers::{Epoch, OLBlockId};
use strata_ol_chain_types_new::OLLog;
use strata_ol_state_support_types::EpochDaAccumulator;

#[derive(Clone, Debug)]
pub(crate) struct EpochDaTracker {
    block_da_map: HashMap<OLBlockId, AccumulatedDaData>,
}

impl EpochDaTracker {
    pub(crate) fn new_empty() -> Self {
        Self::new(HashMap::default())
    }

    pub(crate) fn new(block_da_map: HashMap<OLBlockId, AccumulatedDaData>) -> Self {
        Self { block_da_map }
    }

    pub(crate) fn block_da_map(&self) -> &HashMap<OLBlockId, AccumulatedDaData> {
        &self.block_da_map
    }

    pub(crate) fn block_da_map_mut(&mut self) -> &mut HashMap<OLBlockId, AccumulatedDaData> {
        &mut self.block_da_map
    }

    pub(crate) fn get_accumulated_da(&self, blkid: OLBlockId) -> Option<&AccumulatedDaData> {
        self.block_da_map.get(&blkid)
    }

    pub(crate) fn set_accumulated_da(&mut self, blkid: OLBlockId, da: AccumulatedDaData) {
        self.block_da_map.insert(blkid, da);
    }

    /// Inserts the entry for given block id and also removes the entry for parent if exists. This
    /// method is used to optimize memory usage because in the next assembly we would require
    /// accumulation upto the current block and not the parent block.
    pub(crate) fn set_accumulated_da_and_remove_parent_entry(
        &mut self,
        blkid: OLBlockId,
        parent: OLBlockId,
        da: AccumulatedDaData,
    ) {
        self.set_accumulated_da(blkid, da);
        self.block_da_map.remove(&parent);
    }
}

#[derive(Clone, Debug)]
pub(crate) struct AccumulatedDaData {
    epoch: Epoch,
    accumulator: EpochDaAccumulator,
    logs: Vec<OLLog>,
}

impl AccumulatedDaData {
    pub(crate) fn new_empty(epoch: Epoch) -> Self {
        Self::new(epoch, EpochDaAccumulator::default(), Vec::default())
    }

    pub(crate) fn new(epoch: Epoch, accumulator: EpochDaAccumulator, logs: Vec<OLLog>) -> Self {
        Self {
            epoch,
            accumulator,
            logs,
        }
    }

    pub(crate) fn epoch(&self) -> u32 {
        self.epoch
    }

    pub(crate) fn accumulator(&self) -> &EpochDaAccumulator {
        &self.accumulator
    }

    pub(crate) fn logs(&self) -> &[OLLog] {
        &self.logs
    }

    pub(crate) fn into_parts(self) -> (EpochDaAccumulator, Vec<OLLog>) {
        (self.accumulator, self.logs)
    }
}
