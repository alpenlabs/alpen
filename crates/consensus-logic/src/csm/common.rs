use std::{
    collections::{HashMap, HashSet},
    vec,
};

use bitcoin::block::Header;
use strata_primitives::l1::L1Block;
use strata_state::l1::L1BlockId;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) struct U256(pub u128, pub u128); // (high, low)

impl U256 {
    /// Construct from a big-endian [u8; 32]
    pub(crate) fn from_be_bytes(bytes: [u8; 32]) -> Self {
        let high = u128::from_be_bytes(bytes[0..16].try_into().unwrap());
        let low = u128::from_be_bytes(bytes[16..32].try_into().unwrap());
        U256(high, low)
    }

    /// Convert back to [u8; 32] big-endian
    pub(crate) fn to_be_bytes(&self) -> [u8; 32] {
        let mut out = [0u8; 32];
        out[0..16].copy_from_slice(&self.0.to_be_bytes());
        out[16..32].copy_from_slice(&self.1.to_be_bytes());
        out
    }

    /// Saturating addition
    pub(crate) fn saturating_add(self, other: U256) -> U256 {
        let (low, carry) = self.1.overflowing_add(other.1);
        let (high, overflow) = self.0.overflowing_add(other.0 + (carry as u128));
        if overflow {
            U256(u128::MAX, u128::MAX) // saturate to max
        } else {
            U256(high, low)
        }
    }

    pub(crate) fn zero() -> Self {
        U256(0, 0)
    }
}

// Implement Ord and PartialOrd for comparison
impl Ord for U256 {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0).then(self.1.cmp(&other.1))
    }
}

impl PartialOrd for U256 {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct L1Header {
    height: u64,
    block_id: L1BlockId,
    accumulated_pow: U256,
    inner: Header,
}

impl L1Header {
    pub(crate) fn new(height: u64, accumulated_pow: U256, header: Header) -> Self {
        Self {
            height,
            block_id: header.block_hash().into(),
            accumulated_pow,
            inner: header,
        }
    }

    #[cfg(test)]
    pub(crate) fn from_parts(
        height: u64,
        block_id: L1BlockId,
        accumulated_pow: U256,
        header: Header,
    ) -> Self {
        Self {
            height,
            block_id,
            accumulated_pow,
            inner: header,
        }
    }

    pub(crate) fn block_id(&self) -> L1BlockId {
        self.block_id
    }

    pub(crate) fn parent_id(&self) -> L1BlockId {
        self.inner.prev_blockhash.into()
    }

    pub(crate) fn height(&self) -> u64 {
        self.height
    }

    pub(crate) fn accumulated_pow(&self) -> U256 {
        self.accumulated_pow
    }

    pub(crate) fn inner(&self) -> &Header {
        &self.inner
    }

    pub(crate) fn from_block(block: &L1Block, accumulated_pow: U256) -> Self {
        Self {
            height: block.height(),
            block_id: block.block_id(),
            accumulated_pow,
            inner: block.inner().header,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct IndexedBlockTable {
    pub by_block_id: HashMap<L1BlockId, L1Header>,
    pub by_parent_id: HashMap<L1BlockId, Vec<L1BlockId>>,
    pub by_height: HashMap<u64, Vec<L1BlockId>>,
}

impl IndexedBlockTable {
    pub(crate) fn insert(&mut self, block: L1Header) {
        let height = block.height();
        let block_id = block.block_id();
        let parent_id = block.parent_id();

        self.by_block_id.insert(block_id, block);
        self.by_parent_id
            .entry(parent_id)
            .and_modify(|entry| entry.push(block_id))
            .or_insert(vec![block_id]);
        self.by_height
            .entry(height)
            .and_modify(|entry| entry.push(block_id))
            .or_insert(vec![block_id]);
    }

    pub(crate) fn remove(&mut self, block_id: &L1BlockId) -> Option<L1Header> {
        let block = self.by_block_id.remove(block_id)?;

        self.by_parent_id
            .entry(block.parent_id())
            .and_modify(|entries| entries.retain(|id| id != block_id));
        self.by_height
            .entry(block.height())
            .and_modify(|entries| entries.retain(|id| id != block_id));

        Some(block)
    }

    pub(crate) fn prune_to_height(&mut self, retain_min_height: u64) -> HashSet<L1BlockId> {
        let to_prune_blocks = self
            .by_height
            .iter()
            .filter_map(|(height, block_ids)| {
                if height < &retain_min_height {
                    Some(block_ids)
                } else {
                    None
                }
            })
            .flatten()
            .copied()
            .collect::<HashSet<_>>();

        for block_id in &to_prune_blocks {
            self.remove(block_id);
        }

        to_prune_blocks
    }
}
