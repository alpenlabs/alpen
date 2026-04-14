//! Shared fee-model types and helpers for Alpen v1 fee quoting.
//!
//! This module defines the internal representation of fee-model inputs
//! ([`FeeQuoteInputs`]), the configurable parameters that drive the formula
//! ([`FeeModelConfig`]), the computed fee breakdown ([`FeeBreakdown`]), and
//! a budgeting-only gas-equivalent view ([`GasEquivalentQuote`]). It
//! intentionally does not perform any RPC handling or execution-time
//! charging; those layers consume these types so the v1 formula lives in a
//! single place.

use alloy_primitives::U256;
use serde::{Deserialize, Serialize};

/// Basis-points scaling factor for DA fee multipliers.
pub const DA_OVERHEAD_MULTIPLIER_SCALE_BPS: u32 = 10_000;

/// Source for the L1 fee rate used by the fee model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum L1FeeRateSource {
    /// Reuse the btcio writer policy used for publication.
    BtcioWriter,
}

/// Runtime configuration for the v1 fee model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeeModelConfig {
    /// Static proving fee charged per unit of raw EVM gas.
    pub prover_fee_per_gas_wei: U256,

    /// Basis-points multiplier applied to estimated DA cost.
    pub da_overhead_multiplier_bps: u32,

    /// Small additive fee charged for OL and infrastructure overhead.
    pub ol_overhead_wei: U256,

    /// Source used to resolve the L1 fee rate.
    pub l1_fee_rate_source: L1FeeRateSource,
}

/// Inputs required to quote fees for a transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeeQuoteInputs {
    /// Estimated raw EVM gas for the transaction.
    pub raw_evm_gas: u64,

    /// Current base fee per gas in wei.
    pub base_fee_per_gas: U256,

    /// Current priority fee per gas in wei.
    pub priority_fee_per_gas: U256,

    /// Resolved L1 fee rate in wei per byte of DA payload.
    pub l1_fee_rate_wei_per_byte: U256,

    /// Estimated diff size in bytes for the transaction.
    pub diff_size_bytes: u64,
}

impl FeeQuoteInputs {
    /// Returns the total execution gas price in wei.
    pub fn execution_gas_price(&self) -> Result<U256, FeeModelError> {
        fee_add(
            self.base_fee_per_gas,
            self.priority_fee_per_gas,
            "execution_gas_price",
        )
    }
}

/// Quoted fee breakdown for a single transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeeBreakdown {
    /// Raw EVM gas used for execution and block accounting.
    pub raw_evm_gas: u64,

    /// Total execution gas price in wei.
    pub execution_gas_price: U256,

    /// EVM execution fee in wei.
    pub execution_fee: U256,

    /// Proving fee in wei.
    pub prover_fee: U256,

    /// DA fee in wei.
    pub da_fee: U256,

    /// OL overhead fee in wei.
    pub ol_overhead_fee: U256,

    /// Sum of all non-execution fees in wei.
    pub non_execution_fee: U256,

    /// Total quoted fee in wei.
    pub total_fee: U256,
}

impl FeeBreakdown {
    /// Converts fee amounts into a [`GasEquivalentQuote`] for budgeting.
    ///
    /// Uses ceiling division when converting non-execution wei into gas
    /// units so the budgeting view never undercharges relative to the
    /// quoted [`FeeBreakdown::total_fee`].
    pub fn gas_equivalent_quote(&self) -> Result<GasEquivalentQuote, FeeModelError> {
        let non_execution_gas = if self.non_execution_fee.is_zero() {
            0
        } else if self.execution_gas_price.is_zero() {
            return Err(FeeModelError::ZeroExecutionGasPrice);
        } else {
            let gas = self.non_execution_fee.div_ceil(self.execution_gas_price);
            u64::try_from(gas)
                .map_err(|_| FeeModelError::GasEquivalentOverflow("non_execution_gas"))?
        };

        let total_gas = self
            .raw_evm_gas
            .checked_add(non_execution_gas)
            .ok_or(FeeModelError::GasEquivalentOverflow("total_gas"))?;

        Ok(GasEquivalentQuote {
            execution_gas: self.raw_evm_gas,
            non_execution_gas,
            total_gas,
        })
    }
}

/// Gas-equivalent budgeting view of a quote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GasEquivalentQuote {
    /// Raw EVM execution gas.
    pub execution_gas: u64,

    /// Additional budgeting gas equivalent for non-execution fees.
    pub non_execution_gas: u64,

    /// Total budgeting gas.
    pub total_gas: u64,
}

/// Errors returned while computing fee quotes.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FeeModelError {
    #[error("{0} overflowed during fee computation")]
    Overflow(&'static str),

    #[error("cannot convert non-execution fee into gas units when execution gas price is zero")]
    ZeroExecutionGasPrice,

    #[error("{0} does not fit in u64 gas units")]
    GasEquivalentOverflow(&'static str),
}

impl FeeModelConfig {
    /// Computes the v1 fee-model breakdown for the provided inputs.
    ///
    /// Returns a [`FeeBreakdown`] on success. The DA component rounds up
    /// when applying [`FeeModelConfig::da_overhead_multiplier_bps`] so the
    /// operator never silently undercharges due to integer truncation.
    pub fn quote(&self, inputs: &FeeQuoteInputs) -> Result<FeeBreakdown, FeeModelError> {
        let raw_evm_gas = U256::from(inputs.raw_evm_gas);
        let execution_gas_price = inputs.execution_gas_price()?;
        let execution_fee = fee_mul(execution_gas_price, raw_evm_gas, "execution_fee")?;
        let prover_fee = fee_mul(self.prover_fee_per_gas_wei, raw_evm_gas, "prover_fee")?;

        let raw_da_fee = fee_mul(
            inputs.l1_fee_rate_wei_per_byte,
            U256::from(inputs.diff_size_bytes),
            "raw_da_fee",
        )?;
        let da_fee_numerator = fee_mul(
            raw_da_fee,
            U256::from(self.da_overhead_multiplier_bps),
            "da_fee_numerator",
        )?;
        // Ceil so a fractional bps multiplier never rounds a non-zero DA
        // cost down to a lower wei value than the raw input.
        let da_fee = da_fee_numerator.div_ceil(U256::from(DA_OVERHEAD_MULTIPLIER_SCALE_BPS));

        let non_execution_fee = fee_add(
            fee_add(prover_fee, da_fee, "non_execution_fee")?,
            self.ol_overhead_wei,
            "non_execution_fee",
        )?;
        let total_fee = fee_add(execution_fee, non_execution_fee, "total_fee")?;

        Ok(FeeBreakdown {
            raw_evm_gas: inputs.raw_evm_gas,
            execution_gas_price,
            execution_fee,
            prover_fee,
            da_fee,
            ol_overhead_fee: self.ol_overhead_wei,
            non_execution_fee,
            total_fee,
        })
    }
}

/// Checked `U256` addition that returns a [`FeeModelError::Overflow`] tagged
/// with the calling field name on overflow.
fn fee_add(lhs: U256, rhs: U256, field: &'static str) -> Result<U256, FeeModelError> {
    lhs.checked_add(rhs).ok_or(FeeModelError::Overflow(field))
}

/// Checked `U256` multiplication that returns a [`FeeModelError::Overflow`]
/// tagged with the calling field name on overflow.
fn fee_mul(lhs: U256, rhs: U256, field: &'static str) -> Result<U256, FeeModelError> {
    lhs.checked_mul(rhs).ok_or(FeeModelError::Overflow(field))
}

#[cfg(test)]
mod tests {
    use alloy_primitives::U256;

    use super::{
        FeeModelConfig, FeeModelError, FeeQuoteInputs, GasEquivalentQuote, L1FeeRateSource,
    };

    #[test]
    fn test_quote_computes_v1_fee_breakdown() {
        let config = FeeModelConfig {
            prover_fee_per_gas_wei: U256::from(3u8),
            da_overhead_multiplier_bps: 12_500,
            ol_overhead_wei: U256::from(7u8),
            l1_fee_rate_source: L1FeeRateSource::BtcioWriter,
        };
        let inputs = FeeQuoteInputs {
            raw_evm_gas: 100,
            base_fee_per_gas: U256::from(10u8),
            priority_fee_per_gas: U256::from(2u8),
            l1_fee_rate_wei_per_byte: U256::from(5u8),
            diff_size_bytes: 4,
        };

        let quote = config.quote(&inputs).expect("quote should compute");

        assert_eq!(quote.execution_gas_price, U256::from(12u8));
        assert_eq!(quote.execution_fee, U256::from(1_200u64));
        assert_eq!(quote.prover_fee, U256::from(300u64));
        assert_eq!(quote.da_fee, U256::from(25u8));
        assert_eq!(quote.ol_overhead_fee, U256::from(7u8));
        assert_eq!(quote.non_execution_fee, U256::from(332u64));
        assert_eq!(quote.total_fee, U256::from(1_532u64));
    }

    #[test]
    fn test_quote_supports_undercharge_and_overcharge_multipliers() {
        let inputs = FeeQuoteInputs {
            raw_evm_gas: 1,
            base_fee_per_gas: U256::from(1u8),
            priority_fee_per_gas: U256::ZERO,
            l1_fee_rate_wei_per_byte: U256::from(4u8),
            diff_size_bytes: 5,
        };

        let undercharge = FeeModelConfig {
            prover_fee_per_gas_wei: U256::ZERO,
            da_overhead_multiplier_bps: 5_000,
            ol_overhead_wei: U256::ZERO,
            l1_fee_rate_source: L1FeeRateSource::BtcioWriter,
        };
        let overcharge = FeeModelConfig {
            da_overhead_multiplier_bps: 15_000,
            ..undercharge.clone()
        };

        assert_eq!(
            undercharge
                .quote(&inputs)
                .expect("undercharge quote")
                .da_fee,
            U256::from(10u8)
        );
        assert_eq!(
            overcharge.quote(&inputs).expect("overcharge quote").da_fee,
            U256::from(30u8)
        );
    }

    #[test]
    fn test_quote_rounds_da_fee_up_on_fractional_multiplier() {
        // raw_da_fee = 3 * 7 = 21
        // numerator  = 21 * 12_345 = 259_245
        // 259_245 / 10_000 = 25.9245 -> ceil 26.
        let config = FeeModelConfig {
            prover_fee_per_gas_wei: U256::ZERO,
            da_overhead_multiplier_bps: 12_345,
            ol_overhead_wei: U256::ZERO,
            l1_fee_rate_source: L1FeeRateSource::BtcioWriter,
        };
        let inputs = FeeQuoteInputs {
            raw_evm_gas: 1,
            base_fee_per_gas: U256::from(1u8),
            priority_fee_per_gas: U256::ZERO,
            l1_fee_rate_wei_per_byte: U256::from(3u8),
            diff_size_bytes: 7,
        };

        let quote = config.quote(&inputs).expect("quote should compute");

        assert_eq!(quote.da_fee, U256::from(26u8));
    }

    #[test]
    fn test_gas_equivalent_quote_uses_ceil_division() {
        let config = FeeModelConfig {
            prover_fee_per_gas_wei: U256::from(3u8),
            da_overhead_multiplier_bps: 12_500,
            ol_overhead_wei: U256::from(7u8),
            l1_fee_rate_source: L1FeeRateSource::BtcioWriter,
        };
        let inputs = FeeQuoteInputs {
            raw_evm_gas: 100,
            base_fee_per_gas: U256::from(10u8),
            priority_fee_per_gas: U256::from(2u8),
            l1_fee_rate_wei_per_byte: U256::from(5u8),
            diff_size_bytes: 4,
        };

        let gas_quote = config
            .quote(&inputs)
            .and_then(|quote| quote.gas_equivalent_quote())
            .expect("gas-equivalent quote should compute");

        assert_eq!(
            gas_quote,
            GasEquivalentQuote {
                execution_gas: 100,
                non_execution_gas: 28,
                total_gas: 128,
            }
        );
    }

    #[test]
    fn test_gas_equivalent_quote_rejects_non_zero_fee_with_zero_execution_price() {
        let config = FeeModelConfig {
            prover_fee_per_gas_wei: U256::ZERO,
            da_overhead_multiplier_bps: 10_000,
            ol_overhead_wei: U256::from(1u8),
            l1_fee_rate_source: L1FeeRateSource::BtcioWriter,
        };
        let inputs = FeeQuoteInputs {
            raw_evm_gas: 1,
            base_fee_per_gas: U256::ZERO,
            priority_fee_per_gas: U256::ZERO,
            l1_fee_rate_wei_per_byte: U256::ZERO,
            diff_size_bytes: 0,
        };

        let error = config
            .quote(&inputs)
            .expect("quote should compute")
            .gas_equivalent_quote()
            .expect_err("non-zero non-execution fee with zero price must fail");

        assert_eq!(error, FeeModelError::ZeroExecutionGasPrice);
    }

    #[test]
    fn test_quote_reports_overflow() {
        let config = FeeModelConfig {
            prover_fee_per_gas_wei: U256::MAX,
            da_overhead_multiplier_bps: 10_000,
            ol_overhead_wei: U256::ZERO,
            l1_fee_rate_source: L1FeeRateSource::BtcioWriter,
        };
        let inputs = FeeQuoteInputs {
            raw_evm_gas: 2,
            base_fee_per_gas: U256::ZERO,
            priority_fee_per_gas: U256::ZERO,
            l1_fee_rate_wei_per_byte: U256::ZERO,
            diff_size_bytes: 0,
        };

        let error = config
            .quote(&inputs)
            .expect_err("overflowing prover fee must fail");

        assert_eq!(error, FeeModelError::Overflow("prover_fee"));
    }
}
