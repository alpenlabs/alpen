use arbitrary::Arbitrary;
use bitcoin::{block::Header, hashes::Hash, params::Params, BlockHash, CompactTarget};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use super::{error::L1VerificationError, timestamp_store::TimestampStore, L1BlockId};
use crate::{
    buf::Buf32,
    hash::compute_borsh_hash,
    l1::{utils::compute_block_hash, L1BlockCommitment},
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
    /// Commitment to the last verified block, containing both its height and block hash.
    pub last_verified_block: L1BlockCommitment,

    /// [Target](bitcoin::pow::CompactTarget) for the next block to verify
    pub next_block_target: u32,

    /// Timestamps marking the boundaries of the difficulty adjustment epochs.
    ///
    /// These timestamps are used in the computation of the new difficulty target. They correspond
    /// to:
    /// - `current`: The timestamp at the start of the current difficulty adjustment epoch (end
    ///   boundary).
    /// - `previous`: The timestamp at the start of the previous difficulty adjustment epoch (start
    ///   boundary).
    ///
    /// For example, to successfully compute the first difficulty adjustment on the Bitcoin
    /// network, one would pass the header for Block 2015 as `current` and the header for Block
    /// 0 as `previous`.
    pub epoch_timestamps: EpochTimestamps,

    /// A ring buffer that maintains a history of block timestamps.
    ///
    /// This buffer is used to compute the median block time for consensus rules by considering the
    /// most recent 11 timestamps. However, it retains additional timestamps to support chain reorg
    /// scenarios.
    pub block_timestamp_history: TimestampStore,
}

/// `EpochTimestamps` stores the timestamps corresponding to the boundaries of difficulty adjustment
/// epochs.
///
/// This structure holds two values for handling reorg scenarios where the epoch boundary
/// information might be lost:
///
/// On [MAINNET](bitcoin::consensus::params::MAINNET), a difficulty adjustment interval lasts
/// for 2016 blocks. The interval starts at blocks with heights 0, 2016, 4032, 6048, 8064,
/// etc.
#[derive(
    Clone,
    Debug,
    Default,
    PartialEq,
    Eq,
    Arbitrary,
    BorshSerialize,
    BorshDeserialize,
    Deserialize,
    Serialize,
)]
pub struct EpochTimestamps {
    /// Timestamp of the block at the start of the current difficulty adjustment epoch.
    pub current: u32,

    /// Timestamp of the block at the start of the previous difficulty adjustment epoch.
    pub previous: u32,
}

impl HeaderVerificationState {
    fn next_target(&mut self, header: &Header, params: &Params) -> u32 {
        if !(self.last_verified_block.height() + 1)
            .is_multiple_of(params.difficulty_adjustment_interval())
        {
            return self.next_block_target;
        }

        let timespan = header.time - self.epoch_timestamps.current;

        CompactTarget::from_next_work_required(header.bits, timespan as u64, params).to_consensus()
    }

    // Note/TODO: Figure out a better way so we don't have to params each time.
    fn update_timestamps(&mut self, timestamp: u32, params: &Params) {
        self.block_timestamp_history.insert(timestamp);

        let new_block_num = self.last_verified_block.height();
        if new_block_num.is_multiple_of(params.difficulty_adjustment_interval()) {
            self.epoch_timestamps.previous = self.epoch_timestamps.current;
            self.epoch_timestamps.current = timestamp;
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
    pub fn check_and_update_full(
        &mut self,
        header: &Header,
        params: &Params,
    ) -> Result<(), L1VerificationError> {
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
        self.update_timestamps(header.time, params);

        // Set the target for the next block
        self.next_block_target = self.next_target(header, params);

        Ok(())
    }

    /// Checks header continuity and updates the state accordingly.
    ///
    /// This function verifies that the header's previous block hash matches the expected value,
    /// updates the block number, the block hash, the timestamp store, and the total accumulated
    /// PoW.
    pub fn check_and_update_continuity(
        &mut self,
        header: &Header,
        params: &Params,
    ) -> Result<(), L1VerificationError> {
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

        // Increase the last verified block number by 1 and set the new block hash
        self.last_verified_block =
            L1BlockCommitment::new(self.last_verified_block.height() + 1, block_hash_raw.into());

        // Update the timestamps
        self.update_timestamps(header.time, params);

        Ok(())
    }

    /// Checks header continuity and returns a new verification state with the updated values.
    ///
    /// This is a non-mutating version that clones the state, applies the continuity check, and
    /// returns the updated state.
    pub fn check_and_update_continuity_new(
        &self,
        header: &Header,
        params: &Params,
    ) -> Result<HeaderVerificationState, L1VerificationError> {
        let mut vs = self.clone();
        vs.check_and_update_continuity(header, params)?;
        Ok(vs)
    }

    /// Calculate the hash of the verification state
    pub fn compute_hash(&self) -> Result<Buf32, L1VerificationError> {
        Ok(compute_borsh_hash(&self))
    }

    /// Reorganizes the verification state by removing headers from the old chain and applying
    /// headers from the new chain.
    ///
    /// The function first "rolls back" the state by removing the effects of `old_headers` and then
    /// applies `new_headers`. The new headers must be at least as long as the old headers.
    ///
    /// # Arguments
    ///
    /// * `old_headers` - A slice of headers representing the chain to be removed.
    /// * `new_headers` - A slice of headers representing the new chain to be applied.
    /// * `params` - The Bitcoin network parameters.
    ///
    /// # Errors
    ///
    /// Returns a [`L1VerificationError::ReorgLengthError`] if the new headers are fewer than the
    /// old headers, or any error from header verification.
    pub fn reorg(
        &mut self,
        old_headers: &[Header],
        new_headers: &[Header],
        params: &Params,
    ) -> Result<(), L1VerificationError> {
        if new_headers.len() < old_headers.len() {
            return Err(L1VerificationError::ReorgLengthError {
                new_headers: new_headers.len(),
                old_headers: old_headers.len(),
            });
        }

        for old_header in old_headers.iter().rev() {
            if compute_block_hash(old_header) != (*self.last_verified_block.blkid()).into() {
                return Err(L1VerificationError::ContinuityError {
                    expected: *self.last_verified_block.blkid(),
                    found: old_header.prev_blockhash.into(),
                });
            }
            if self
                .last_verified_block
                .height()
                .is_multiple_of(params.difficulty_adjustment_interval())
            {
                self.epoch_timestamps.current = self.epoch_timestamps.previous;
            }
            self.last_verified_block = L1BlockCommitment::new(
                self.last_verified_block.height() - 1,
                old_header.prev_blockhash.into(),
            );
            self.block_timestamp_history.remove();
            self.next_block_target = old_header.bits.to_consensus();
        }

        for new_header in new_headers {
            self.check_and_update_full(new_header, params)?;
        }

        Ok(())
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
    use strata_test_utils::bitcoin_mainnet_segment::BtcChainSegment;

    use super::*;

    #[test]
    fn test_blocks() {
        let chain = BtcChainSegment::load();
        let h2 = get_relative_difficulty_adjustment_height(2, chain.start, &MAINNET);
        let r1 = OsRng.gen_range(h2..chain.end);
        let mut verification_state = chain.get_verification_state(r1, 0).unwrap();

        for header_idx in r1 + 1..chain.end {
            verification_state
                .check_and_update_full(&chain.get_block_header_at(header_idx).unwrap(), &MAINNET)
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
        let verification_state = chain.get_verification_state(r1, 0).unwrap();
        let hash = verification_state.compute_hash();
        assert!(hash.is_ok());
    }

    fn test_reorg(reorg: (u64, u64)) {
        let chain = BtcChainSegment::load();

        let reorg_len = (reorg.1 - reorg.0) as u32;
        let reorg = reorg.0..reorg.1;
        let headers: Vec<Header> = reorg
            .clone()
            .map(|h| chain.get_block_header_at(h).unwrap())
            .collect();
        let mut verification_state = chain
            .get_verification_state(reorg.start - 1, reorg_len)
            .unwrap();

        for header in &headers {
            verification_state
                .check_and_update_full(header, &MAINNET)
                .unwrap();
        }
        let before_vs = verification_state.clone();

        verification_state
            .reorg(&headers, &headers, &MAINNET)
            .unwrap();

        // We use the same headers for reorg to check if they are consistent
        assert_eq!(before_vs, verification_state);
    }

    #[test]
    fn test_reorgs() {
        let chain = BtcChainSegment::load();
        let h3 = get_relative_difficulty_adjustment_height(3, chain.start, &MAINNET);
        let h4 = get_relative_difficulty_adjustment_height(4, chain.start, &MAINNET);
        let h5 = get_relative_difficulty_adjustment_height(5, chain.start, &MAINNET);

        // Reorg of 10 blocks with no difficulty adjustment in between
        let reorg = (h3 + 100, h3 + 110);
        test_reorg(reorg);

        let reorg = (h4 + 100, h4 + 110);
        test_reorg(reorg);

        let reorg = (h5 + 100, h5 + 110);
        test_reorg(reorg);

        // Reorg of 10 blocks with difficulty adjustment in between
        let reorg = (h3 - 5, h3 + 5);
        test_reorg(reorg);

        let reorg = (h4 - 5, h4 + 5);
        test_reorg(reorg);

        let reorg = (h5 - 5, h5 + 5);
        test_reorg(reorg);

        // Reorg exactly at the difficulty adjusted block
        let reorg = (h3, h3 + 10);
        test_reorg(reorg);

        let reorg = (h4, h4 + 10);
        test_reorg(reorg);

        let reorg = (h5, h5 + 10);
        test_reorg(reorg);
    }
}
