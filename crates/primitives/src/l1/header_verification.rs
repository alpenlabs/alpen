use arbitrary::Arbitrary;
use bitcoin::{block::Header, hashes::Hash, params::Params, BlockHash, CompactTarget, Network};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use super::{error::L1VerificationError, timestamp_store::TimestampStore, L1BlockId};
use crate::{
    buf::Buf32,
    hash::compute_borsh_hash,
    l1::{utils::compute_block_hash, BtcParams, L1BlockCommitment},
    params::GenesisL1View,
};

/// A struct containing all necessary information for validating a Bitcoin block header.
///
/// The validation process includes:
///
/// 1. Ensuring that the block's hash is below the current target, which is a threshold representing
///    a hash with a specified number of leading zeros. This target is directly related to the
///    block's difficulty.
///
/// 2. Verifying that the encoded previous block hash in the current block matches the actual hash
///    of the previous block.
///
/// 3. Checking that the block's timestamp is not lower than the median of the last eleven blocks'
///    timestamps and does not exceed the network time by more than two hours.
///
/// 4. Ensuring that the correct target is encoded in the block. If a retarget event occurred,
///    validating that the new target was accurately derived from the epoch timestamps.
///
/// Ref: [A light introduction to ZeroSync](https://geometry.xyz/notebook/A-light-introduction-to-ZeroSync)
#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    Default,
    Arbitrary,
    BorshSerialize,
    BorshDeserialize,
    Deserialize,
    Serialize,
)]
pub struct HeaderVerificationState {
    /// Bitcoin network parameters used for header verification.
    ///
    /// Contains network-specific configuration including difficulty adjustment intervals,
    /// target block spacing, and other consensus parameters required for validating block headers
    /// according to the Bitcoin protocol rules.
    params: BtcParams,

    /// Commitment to the last verified block, containing both its height and block hash.
    pub last_verified_block: L1BlockCommitment,

    /// [Target](bitcoin::pow::CompactTarget) for the next block to verify
    next_block_target: u32,

    /// Timestamp of the block at the start of a [difficulty adjustment
    /// interval](bitcoin::consensus::params::Params::difficulty_adjustment_interval).
    ///
    /// On [MAINNET](bitcoin::consensus::params::MAINNET), a difficulty adjustment interval lasts
    /// for 2016 blocks. The interval starts at blocks with heights 0, 2016, 4032, 6048, 8064,
    /// etc.
    ///
    /// This field represents the timestamp of the starting block of the interval
    /// (e.g., block 0, 2016, 4032, etc.).
    epoch_start_timestamp: u32,

    /// A ring buffer that maintains a history of block timestamps.
    ///
    /// This buffer is used to compute the median block time for consensus rules by considering the
    /// most recent 11 timestamps. However, it retains additional timestamps to support chain reorg
    /// scenarios.
    block_timestamp_history: TimestampStore,

    /// Total accumulated proof of work
    total_accumulated_pow: u128,
}

impl HeaderVerificationState {
    pub fn new(network: Network, genesis_view: GenesisL1View) -> Self {
        let params = Params::new(network).into();

        Self {
            params,
            last_verified_block: genesis_view.blk,
            next_block_target: genesis_view.next_target,
            epoch_start_timestamp: genesis_view.epoch_start_timestamp,
            block_timestamp_history: TimestampStore::new(genesis_view.last_11_timestamps),
            total_accumulated_pow: 0,
        }
    }

    /// Calculates the next difficulty target based on the current header.
    ///
    /// If this is a difficulty adjustment block (height + 1 is multiple of adjustment interval),
    /// calculates a new target using the timespan between epoch start and current block.
    /// Otherwise, returns the current target unchanged.
    fn next_target(&mut self, header: &Header) -> u32 {
        if !(self.last_verified_block.height() + 1)
            .is_multiple_of(self.params.difficulty_adjustment_interval())
        {
            return self.next_block_target;
        }

        let timespan = header.time - self.epoch_start_timestamp;

        CompactTarget::from_next_work_required(header.bits, timespan as u64, &self.params)
            .to_consensus()
    }

    /// Updates the timestamp history and epoch start timestamp if necessary.
    ///
    /// Adds the new timestamp to the ring buffer history. If the current block height
    /// is at a difficulty adjustment boundary, updates the epoch start timestamp to
    /// track the beginning of the new difficulty adjustment period.
    fn update_timestamps(&mut self, timestamp: u32) {
        self.block_timestamp_history.insert(timestamp);

        let new_block_num = self.last_verified_block.height();
        if new_block_num.is_multiple_of(self.params.difficulty_adjustment_interval()) {
            self.epoch_start_timestamp = timestamp;
        }
    }

    /// Checks all verification criteria for a header and updates the state if all conditions pass.
    ///
    /// The checks include:
    /// 1. Continuity: Ensuring the header's previous block hash matches the last verified hash.
    /// 2. Proof-of-Work: Validating that the header’s target matches the expected target and that
    ///    the computed block hash meets the target.
    /// 3. Timestamp: Ensuring the header's timestamp is greater than the median of the last 11
    ///    blocks.
    /// # Errors
    ///
    /// Returns a [`L1VerificationError`] if any of the checks fail.
    pub fn check_and_update(&mut self, header: &Header) -> Result<(), L1VerificationError> {
        // Check continuity
        let prev_blockhash: L1BlockId =
            Buf32::from(header.prev_blockhash.as_raw_hash().to_byte_array()).into();
        if prev_blockhash != *self.last_verified_block.blkid() {
            return Err(L1VerificationError::ContinuityError {
                expected: *self.last_verified_block.blkid(),
                found: prev_blockhash,
            });
        }

        let block_hash_raw = compute_block_hash(header);
        let block_hash = BlockHash::from_byte_array(*block_hash_raw.as_ref());

        // Check Proof-of-Work target encoding
        if header.bits.to_consensus() != self.next_block_target {
            return Err(L1VerificationError::PowMismatch {
                expected: self.next_block_target,
                found: header.bits.to_consensus(),
            });
        }

        // Check that the block hash meets the target difficulty.
        if !header.target().is_met_by(block_hash) {
            return Err(L1VerificationError::PowNotMet {
                block_hash,
                target: header.bits.to_consensus(),
            });
        }

        // Check timestamp against the median of the last 11 timestamps.
        let median = self.block_timestamp_history.median();
        if header.time <= median {
            return Err(L1VerificationError::TimestampError {
                time: header.time,
                median,
            });
        }

        // Increase the last verified block number by 1 and set the new block hash
        self.last_verified_block =
            L1BlockCommitment::new(self.last_verified_block.height() + 1, block_hash_raw.into());

        // Update the timestamps
        self.update_timestamps(header.time);

        // Set the target for the next block
        self.next_block_target = self.next_target(header);

        // Update total accumulated PoW
        self.total_accumulated_pow += header.difficulty(&self.params);

        Ok(())
    }

    /// Calculate the hash of the verification state
    pub fn compute_hash(&self) -> Result<Buf32, L1VerificationError> {
        Ok(compute_borsh_hash(&self))
    }
}

/// Calculates the height at which a specific difficulty adjustment occurs relative to a
/// starting height.
///
/// # Arguments
///
/// * `idx` - The index of the difficulty adjustment (1-based). 1 for the first adjustment, 2 for
///   the second, and so on.
/// * `start` - The starting height from which to calculate.
/// * `params` - [`Params`] of the bitcoin network in use
pub fn get_relative_difficulty_adjustment_height(idx: u64, start: u64, params: &Params) -> u64 {
    let difficulty_adjustment_interval = params.difficulty_adjustment_interval();
    ((start / difficulty_adjustment_interval) + idx) * difficulty_adjustment_interval
}

#[cfg(test)]
mod tests {

    use bitcoin::params::MAINNET;
    use rand::{rngs::OsRng, Rng};
    use strata_test_utils_btc::segment::BtcChainSegment;

    use super::*;

    #[test]
    fn test_blocks() {
        let chain = BtcChainSegment::load();
        let h2 = get_relative_difficulty_adjustment_height(2, chain.start, &MAINNET);
        let r1 = OsRng.gen_range(h2..chain.end);
        let mut verification_state = chain.get_verification_state(r1).unwrap();

        for header_idx in r1 + 1..chain.end {
            verification_state
                .check_and_update(&chain.get_block_header_at(header_idx).unwrap())
                .unwrap()
        }
    }

    #[test]
    fn test_get_difficulty_adjustment_height() {
        let start = 0;
        let idx = OsRng.gen_range(1..1000);
        let h = get_relative_difficulty_adjustment_height(idx, start, &MAINNET);
        assert_eq!(h, MAINNET.difficulty_adjustment_interval() * idx);
    }

    #[test]
    fn test_hash() {
        let chain = BtcChainSegment::load();
        let r1 = 45000;
        let verification_state = chain.get_verification_state(r1).unwrap();
        let hash = verification_state.compute_hash();
        assert!(hash.is_ok());
    }
}
