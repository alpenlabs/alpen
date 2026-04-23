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
    pub(crate) prover_fee_per_gas_wei: U256,

    /// Basis-points multiplier applied to estimated DA cost.
    pub(crate) da_overhead_multiplier_bps: u32,

    /// Small additive fee charged for OL and infrastructure overhead.
    pub(crate) ol_overhead_wei: U256,

    /// Source used to resolve the L1 fee rate.
    pub(crate) l1_fee_rate_source: L1FeeRateSource,
}

/// Inputs required to quote fees for a transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeeQuoteInputs {
    /// Estimated raw EVM gas for the transaction.
    pub(crate) raw_evm_gas: u64,

    /// Current base fee per gas in wei.
    pub(crate) base_fee_per_gas: U256,

    /// Current priority fee per gas in wei.
    pub(crate) priority_fee_per_gas: U256,

    /// Resolved L1 fee rate in wei per byte of DA payload.
    pub(crate) l1_fee_rate_wei_per_byte: U256,

    /// Estimated diff size in bytes for the transaction.
    pub(crate) diff_size_bytes: u64,
}

impl FeeQuoteInputs {
    /// Creates fee-quote inputs for a single transaction.
    pub fn new(
        raw_evm_gas: u64,
        base_fee_per_gas: U256,
        priority_fee_per_gas: U256,
        l1_fee_rate_wei_per_byte: U256,
        diff_size_bytes: u64,
    ) -> Self {
        Self {
            raw_evm_gas,
            base_fee_per_gas,
            priority_fee_per_gas,
            l1_fee_rate_wei_per_byte,
            diff_size_bytes,
        }
    }

    /// Returns the estimated raw EVM gas for the transaction.
    pub fn raw_evm_gas(&self) -> u64 {
        self.raw_evm_gas
    }

    /// Returns the current base fee per gas in wei.
    pub fn base_fee_per_gas(&self) -> U256 {
        self.base_fee_per_gas
    }

    /// Returns the current priority fee per gas in wei.
    pub fn priority_fee_per_gas(&self) -> U256 {
        self.priority_fee_per_gas
    }

    /// Returns the resolved L1 fee rate in wei per byte of DA payload.
    pub fn l1_fee_rate_wei_per_byte(&self) -> U256 {
        self.l1_fee_rate_wei_per_byte
    }

    /// Returns the estimated diff size in bytes for the transaction.
    pub fn diff_size_bytes(&self) -> u64 {
        self.diff_size_bytes
    }

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
    pub(crate) raw_evm_gas: u64,

    /// Total execution gas price in wei.
    pub(crate) execution_gas_price: U256,

    /// EVM execution fee in wei.
    pub(crate) execution_fee: U256,

    /// Proving fee in wei.
    pub(crate) prover_fee: U256,

    /// DA fee in wei.
    pub(crate) da_fee: U256,

    /// OL overhead fee in wei.
    pub(crate) ol_overhead_fee: U256,

    /// Sum of all non-execution fees in wei.
    pub(crate) non_execution_fee: U256,

    /// Total quoted fee in wei.
    pub(crate) total_fee: U256,
}

impl FeeBreakdown {
    /// Returns the raw EVM gas used for execution and block accounting.
    pub fn raw_evm_gas(&self) -> u64 {
        self.raw_evm_gas
    }

    /// Returns the total execution gas price in wei.
    pub fn execution_gas_price(&self) -> U256 {
        self.execution_gas_price
    }

    /// Returns the EVM execution fee in wei.
    pub fn execution_fee(&self) -> U256 {
        self.execution_fee
    }

    /// Returns the proving fee in wei.
    pub fn prover_fee(&self) -> U256 {
        self.prover_fee
    }

    /// Returns the DA fee in wei.
    pub fn da_fee(&self) -> U256 {
        self.da_fee
    }

    /// Returns the OL overhead fee in wei.
    pub fn ol_overhead_fee(&self) -> U256 {
        self.ol_overhead_fee
    }

    /// Returns the sum of all non-execution fees in wei.
    pub fn non_execution_fee(&self) -> U256 {
        self.non_execution_fee
    }

    /// Returns the total quoted fee in wei.
    pub fn total_fee(&self) -> U256 {
        self.total_fee
    }

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

        let gas_quote = GasEquivalentQuote::new(self.raw_evm_gas, non_execution_gas);
        debug_assert_eq!(gas_quote.total_gas(), total_gas);

        Ok(gas_quote)
    }
}

/// Gas-equivalent budgeting view of a quote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GasEquivalentQuote {
    /// Raw EVM execution gas.
    pub(crate) execution_gas: u64,

    /// Additional budgeting gas equivalent for non-execution fees.
    pub(crate) non_execution_gas: u64,
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
    /// Creates a runtime configuration for the v1 fee model.
    pub fn new(
        prover_fee_per_gas_wei: U256,
        da_overhead_multiplier_bps: u32,
        ol_overhead_wei: U256,
        l1_fee_rate_source: L1FeeRateSource,
    ) -> Self {
        Self {
            prover_fee_per_gas_wei,
            da_overhead_multiplier_bps,
            ol_overhead_wei,
            l1_fee_rate_source,
        }
    }

    /// Returns the proving fee charged per unit of raw EVM gas.
    pub fn prover_fee_per_gas_wei(&self) -> U256 {
        self.prover_fee_per_gas_wei
    }

    /// Returns the basis-points multiplier applied to estimated DA cost.
    pub fn da_overhead_multiplier_bps(&self) -> u32 {
        self.da_overhead_multiplier_bps
    }

    /// Returns the additive OL and infrastructure overhead fee.
    pub fn ol_overhead_wei(&self) -> U256 {
        self.ol_overhead_wei
    }

    /// Returns the source used to resolve the L1 fee rate.
    pub fn l1_fee_rate_source(&self) -> L1FeeRateSource {
        self.l1_fee_rate_source
    }

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

impl GasEquivalentQuote {
    fn new(execution_gas: u64, non_execution_gas: u64) -> Self {
        execution_gas
            .checked_add(non_execution_gas)
            .expect("gas-equivalent quote must fit in u64");
        Self {
            execution_gas,
            non_execution_gas,
        }
    }

    /// Returns the raw EVM execution gas.
    pub fn execution_gas(&self) -> u64 {
        self.execution_gas
    }

    /// Returns the budgeting gas equivalent for non-execution fees.
    pub fn non_execution_gas(&self) -> u64 {
        self.non_execution_gas
    }

    /// Returns the total budgeting gas.
    pub fn total_gas(&self) -> u64 {
        self.execution_gas
            .checked_add(self.non_execution_gas)
            .expect("gas-equivalent quote must fit in u64")
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
        let config = FeeModelConfig::new(
            U256::from(3u8),
            12_500,
            U256::from(7u8),
            L1FeeRateSource::BtcioWriter,
        );
        let inputs =
            FeeQuoteInputs::new(100, U256::from(10u8), U256::from(2u8), U256::from(5u8), 4);

        let quote = config.quote(&inputs).expect("quote should compute");

        assert_eq!(quote.execution_gas_price(), U256::from(12u8));
        assert_eq!(quote.execution_fee(), U256::from(1_200u64));
        assert_eq!(quote.prover_fee(), U256::from(300u64));
        assert_eq!(quote.da_fee(), U256::from(25u8));
        assert_eq!(quote.ol_overhead_fee(), U256::from(7u8));
        assert_eq!(quote.non_execution_fee(), U256::from(332u64));
        assert_eq!(quote.total_fee(), U256::from(1_532u64));
    }

    #[test]
    fn test_quote_supports_undercharge_and_overcharge_multipliers() {
        let inputs = FeeQuoteInputs::new(1, U256::from(1u8), U256::ZERO, U256::from(4u8), 5);

        let undercharge =
            FeeModelConfig::new(U256::ZERO, 5_000, U256::ZERO, L1FeeRateSource::BtcioWriter);
        let overcharge = FeeModelConfig::new(
            undercharge.prover_fee_per_gas_wei(),
            15_000,
            undercharge.ol_overhead_wei(),
            undercharge.l1_fee_rate_source(),
        );

        assert_eq!(
            undercharge
                .quote(&inputs)
                .expect("undercharge quote")
                .da_fee(),
            U256::from(10u8)
        );
        assert_eq!(
            overcharge
                .quote(&inputs)
                .expect("overcharge quote")
                .da_fee(),
            U256::from(30u8)
        );
    }

    #[test]
    fn test_quote_rounds_da_fee_up_on_fractional_multiplier() {
        // raw_da_fee = 3 * 7 = 21
        // numerator  = 21 * 12_345 = 259_245
        // 259_245 / 10_000 = 25.9245 -> ceil 26.
        let config =
            FeeModelConfig::new(U256::ZERO, 12_345, U256::ZERO, L1FeeRateSource::BtcioWriter);
        let inputs = FeeQuoteInputs::new(1, U256::from(1u8), U256::ZERO, U256::from(3u8), 7);

        let quote = config.quote(&inputs).expect("quote should compute");

        assert_eq!(quote.da_fee(), U256::from(26u8));
    }

    #[test]
    fn test_gas_equivalent_quote_uses_ceil_division() {
        let config = FeeModelConfig::new(
            U256::from(3u8),
            12_500,
            U256::from(7u8),
            L1FeeRateSource::BtcioWriter,
        );
        let inputs =
            FeeQuoteInputs::new(100, U256::from(10u8), U256::from(2u8), U256::from(5u8), 4);

        let gas_quote = config
            .quote(&inputs)
            .and_then(|quote| quote.gas_equivalent_quote())
            .expect("gas-equivalent quote should compute");

        assert_eq!(gas_quote, GasEquivalentQuote::new(100, 28));
    }

    #[test]
    fn test_gas_equivalent_quote_rejects_non_zero_fee_with_zero_execution_price() {
        let config = FeeModelConfig::new(
            U256::ZERO,
            10_000,
            U256::from(1u8),
            L1FeeRateSource::BtcioWriter,
        );
        let inputs = FeeQuoteInputs::new(1, U256::ZERO, U256::ZERO, U256::ZERO, 0);

        let error = config
            .quote(&inputs)
            .expect("quote should compute")
            .gas_equivalent_quote()
            .expect_err("non-zero non-execution fee with zero price must fail");

        assert_eq!(error, FeeModelError::ZeroExecutionGasPrice);
    }

    #[test]
    fn test_quote_reports_overflow() {
        let config =
            FeeModelConfig::new(U256::MAX, 10_000, U256::ZERO, L1FeeRateSource::BtcioWriter);
        let inputs = FeeQuoteInputs::new(2, U256::ZERO, U256::ZERO, U256::ZERO, 0);

        let error = config
            .quote(&inputs)
            .expect_err("overflowing prover fee must fail");

        assert_eq!(error, FeeModelError::Overflow("prover_fee"));
    }
}
