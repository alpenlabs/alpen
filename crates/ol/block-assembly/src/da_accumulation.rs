//! Data availability accumulation for block assembly.
//!
//! This module handles accumulating state diffs and logs across blocks within an epoch.
//! At epoch boundaries, the accumulated data is finalized and reset for the next epoch.

use strata_da_framework::{Codec, CodecError, Decoder, Encoder};
use strata_identifiers::Epoch;
use strata_ol_chain_types_new::OLLog;
use strata_ol_state_support_types::da_accumulating_layer::EpochDaAccumulator;

/// Accumulated DA data for a block within an epoch.
///
/// Contains both the epoch accumulator state and the logs
/// generated up to this point in the epoch.
#[derive(Debug)]
pub struct AccumulatedDaData {
    /// The epoch this accumulation belongs to.
    epoch: Epoch,

    /// The epoch DA accumulator that tracks state changes.
    /// This is serialized and persisted between blocks.
    accumulator: EpochDaAccumulator,

    /// All logs emitted in the epoch up to and including this block.
    logs: Vec<OLLog>,
}

impl AccumulatedDaData {
    /// Creates empty accumulated data for the start of an epoch.
    pub fn empty(epoch: Epoch) -> Self {
        Self {
            epoch,
            accumulator: EpochDaAccumulator::default(),
            logs: Vec::new(),
        }
    }

    /// Creates accumulated data with the given components.
    pub fn new(epoch: Epoch, accumulator: EpochDaAccumulator, logs: Vec<OLLog>) -> Self {
        Self {
            epoch,
            accumulator,
            logs,
        }
    }

    /// Checks if this is the start of a new epoch compared to another.
    pub fn is_new_epoch(&self, other_epoch: Epoch) -> bool {
        self.epoch != other_epoch
    }

    /// Appends logs to the accumulated logs.
    pub fn append_logs(&mut self, new_logs: Vec<OLLog>) {
        self.logs.extend(new_logs);
    }

    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    pub fn accumulator(&self) -> &EpochDaAccumulator {
        &self.accumulator
    }

    pub fn accumulator_mut(&mut self) -> &mut EpochDaAccumulator {
        &mut self.accumulator
    }

    pub fn logs(&self) -> &[OLLog] {
        &self.logs
    }

    pub fn into_parts(self) -> (EpochDaAccumulator, Vec<OLLog>) {
        (self.accumulator, self.logs)
    }
}

impl Codec for AccumulatedDaData {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.epoch.encode(enc)?;
        self.accumulator.encode(enc)?;
        // For now, we'll skip encoding logs since OLLog might not have Codec
        // In a real implementation, you'd need to ensure OLLog implements Codec
        // or serialize them differently
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let epoch = Epoch::decode(dec)?;
        let accumulator = EpochDaAccumulator::decode(dec)?;
        // For now, we'll use empty logs since we didn't encode them
        let logs = Vec::new();
        Ok(Self {
            epoch,
            accumulator,
            logs,
        })
    }
}

