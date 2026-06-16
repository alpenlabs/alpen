//! Gas-limit based batching policy implementation.
//!
//! Seals a batch when the cumulative gas used across blocks reaches a
//! configured maximum.

use super::policy::{AccumulationPolicy, SealingPolicy};

/// Gas-limit based batching policy.
#[derive(Debug)]
pub struct GasLimitPolicy;

/// Per-block gas data.
#[derive(Debug, Clone)]
pub struct GasBlockData {
    /// Gas used by this block.
    pub gas_used: u64,
}

/// Accumulated gas across blocks in the pending batch.
#[derive(Debug, Default)]
pub struct GasValue {
    /// Total gas used so far.
    pub total_gas: u64,
}

impl AccumulationPolicy for GasLimitPolicy {
    type BlockData = GasBlockData;
    type AccumulatedValue = GasValue;

    fn accumulate(value: &mut GasValue, data: &GasBlockData) {
        value.total_gas += data.gas_used;
    }
}

/// Seals a batch when cumulative gas reaches the configured limit.
#[derive(Debug)]
pub struct MaxGasSealing {
    max_gas: u64,
}

impl MaxGasSealing {
    /// Create a new max-gas sealing policy.
    pub fn new(max_gas: u64) -> Self {
        Self { max_gas }
    }

    /// Get the gas limit.
    pub fn max_gas(&self) -> u64 {
        self.max_gas
    }
}

impl SealingPolicy<GasLimitPolicy> for MaxGasSealing {
    fn would_exceed(&self, value: &GasValue, data: &GasBlockData) -> bool {
        value.total_gas + data.gas_used > self.max_gas
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{sealing_policy::Accumulator, test_utils::*};

    #[test]
    fn test_accumulates_gas() {
        let mut acc: Accumulator<GasLimitPolicy> = Accumulator::new();

        acc.add_block(test_blocknumhash(1), &GasBlockData { gas_used: 100 });
        acc.add_block(test_blocknumhash(2), &GasBlockData { gas_used: 200 });

        assert_eq!(acc.value().total_gas, 300);
    }

    #[test]
    fn test_would_not_exceed_below_limit() {
        let sealing = MaxGasSealing::new(1000);
        let mut acc: Accumulator<GasLimitPolicy> = Accumulator::new();

        // 500 + 400 = 900 <= 1000
        acc.add_block(test_blocknumhash(1), &GasBlockData { gas_used: 500 });
        assert!(!acc.would_exceed(&sealing, &GasBlockData { gas_used: 400 }));
    }

    #[test]
    fn test_would_not_exceed_at_exact_limit() {
        let sealing = MaxGasSealing::new(1000);
        let mut acc: Accumulator<GasLimitPolicy> = Accumulator::new();

        // 500 + 500 = 1000 <= 1000, batch of exactly max_gas is allowed
        acc.add_block(test_blocknumhash(1), &GasBlockData { gas_used: 500 });
        assert!(!acc.would_exceed(&sealing, &GasBlockData { gas_used: 500 }));
    }

    #[test]
    fn test_would_exceed_past_limit() {
        let sealing = MaxGasSealing::new(1000);
        let mut acc: Accumulator<GasLimitPolicy> = Accumulator::new();

        // 500 + 501 = 1001 > 1000, seal before adding this block
        acc.add_block(test_blocknumhash(1), &GasBlockData { gas_used: 500 });
        assert!(acc.would_exceed(&sealing, &GasBlockData { gas_used: 501 }));
    }

    #[test]
    fn test_single_block_exceeding_limit_seals_empty_batch() {
        let sealing = MaxGasSealing::new(1000);
        let acc: Accumulator<GasLimitPolicy> = Accumulator::new();

        // 0 + 1500 = 1500 > 1000, but accumulator is empty so the
        // caller's `!is_empty()` guard prevents sealing an empty batch.
        // The block still gets added. This test documents that the
        // sealing check itself returns true.
        assert!(acc.would_exceed(&sealing, &GasBlockData { gas_used: 1500 }));
    }

    #[test]
    fn test_resets_after_drain() {
        let mut acc: Accumulator<GasLimitPolicy> = Accumulator::new();

        acc.add_block(test_blocknumhash(1), &GasBlockData { gas_used: 999 });
        acc.drain();

        assert_eq!(acc.value().total_gas, 0);
    }
}
