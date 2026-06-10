//! Bridge denomination and cap parameters.

#[cfg(feature = "arbitrary")]
use arbitrary::Arbitrary;
use serde::{Deserialize, Serialize, de::Error as DeError};
use ssz_derive::{Decode, Encode};
use thiserror::Error;

/// Bridge denomination and optional maximum withdrawal amount, in satoshis.
///
/// Constructed via [`BridgeParams::new`] which validates the invariants:
/// - `denomination` must be non-zero
/// - If `max_withdrawal_amount` is set, it must be `>= denomination` and a multiple of it
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Encode)]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
pub struct BridgeParams {
    denomination: u64,
    max_withdrawal_amount: Option<u64>,
}

/// Raw mirror for deserialization. Serde and SSZ decode into this first,
/// then validate via [`BridgeParams::new`].
#[derive(Deserialize, Decode)]
struct BridgeParamsRaw {
    denomination: u64,
    max_withdrawal_amount: Option<u64>,
}

impl<'de> Deserialize<'de> for BridgeParams {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = BridgeParamsRaw::deserialize(deserializer)?;
        Self::new(raw.denomination, raw.max_withdrawal_amount).map_err(DeError::custom)
    }
}

// ssz-manual-ok: delegates byte parsing to derived BridgeParamsRaw; adds validation only.
impl ssz::Decode for BridgeParams {
    fn is_ssz_fixed_len() -> bool {
        BridgeParamsRaw::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        BridgeParamsRaw::ssz_fixed_len()
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, ssz::DecodeError> {
        let raw = BridgeParamsRaw::from_ssz_bytes(bytes)?;
        Self::new(raw.denomination, raw.max_withdrawal_amount)
            .map_err(|e| ssz::DecodeError::BytesInvalid(e.to_string()))
    }
}

impl BridgeParams {
    /// Creates a new [`BridgeParams`] after validating invariants.
    pub fn new(
        denomination: u64,
        max_withdrawal_amount: Option<u64>,
    ) -> Result<Self, BridgeParamsError> {
        if denomination == 0 {
            return Err(BridgeParamsError::ZeroDenomination);
        }
        if let Some(max) = max_withdrawal_amount {
            if max < denomination {
                return Err(BridgeParamsError::MaxBelowDenomination { denomination, max });
            }
            if max % denomination != 0 {
                return Err(BridgeParamsError::MaxNotMultiple { denomination, max });
            }
        }
        Ok(Self {
            denomination,
            max_withdrawal_amount,
        })
    }

    pub fn denomination(&self) -> u64 {
        self.denomination
    }

    pub fn max_withdrawal_amount(&self) -> Option<u64> {
        self.max_withdrawal_amount
    }

    /// Returns whether a withdrawal amount (in sats) is valid.
    pub fn validate_withdrawal_amount(&self, amount_sats: u64) -> bool {
        amount_sats > 0
            && amount_sats.is_multiple_of(self.denomination)
            && self
                .max_withdrawal_amount
                .is_none_or(|cap| amount_sats <= cap)
    }
}

/// Default bridge denomination: 1 BTC in satoshis.
pub const DEFAULT_DENOMINATION_SATS: u64 = 100_000_000;

/// Default maximum withdrawal amount: 10 BTC in satoshis.
pub const DEFAULT_MAX_WITHDRAWAL_SATS: u64 = 1_000_000_000;

impl Default for BridgeParams {
    fn default() -> Self {
        Self {
            denomination: DEFAULT_DENOMINATION_SATS,
            max_withdrawal_amount: Some(DEFAULT_MAX_WITHDRAWAL_SATS),
        }
    }
}

#[derive(Debug, Error)]
pub enum BridgeParamsError {
    #[error("denomination must not be zero")]
    ZeroDenomination,

    #[error("max_withdrawal_amount ({max}) is below denomination ({denomination})")]
    MaxBelowDenomination { denomination: u64, max: u64 },

    #[error("max_withdrawal_amount ({max}) is not a multiple of denomination ({denomination})")]
    MaxNotMultiple { denomination: u64, max: u64 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_no_cap() {
        let w = BridgeParams::new(100_000_000, None).unwrap();
        assert!(w.validate_withdrawal_amount(100_000_000));
        assert!(w.validate_withdrawal_amount(300_000_000));
        assert!(!w.validate_withdrawal_amount(0));
        assert!(!w.validate_withdrawal_amount(150_000_000));
    }

    #[test]
    fn valid_with_cap() {
        let w = BridgeParams::new(100_000_000, Some(1_000_000_000)).unwrap();
        assert!(w.validate_withdrawal_amount(100_000_000));
        assert!(w.validate_withdrawal_amount(1_000_000_000));
        assert!(!w.validate_withdrawal_amount(1_100_000_000));
        assert!(!w.validate_withdrawal_amount(0));
        assert!(!w.validate_withdrawal_amount(150_000_000));
    }

    #[test]
    fn zero_denomination_rejected() {
        assert!(BridgeParams::new(0, None).is_err());
    }

    #[test]
    fn max_below_denomination_rejected() {
        assert!(BridgeParams::new(100_000_000, Some(50_000_000)).is_err());
    }

    #[test]
    fn max_not_multiple_rejected() {
        assert!(BridgeParams::new(100_000_000, Some(150_000_000)).is_err());
    }

    #[test]
    fn max_equals_denomination_accepted() {
        let w = BridgeParams::new(100_000_000, Some(100_000_000)).unwrap();
        assert!(w.validate_withdrawal_amount(100_000_000));
        assert!(!w.validate_withdrawal_amount(200_000_000));
    }

    #[test]
    fn serde_roundtrip() {
        let bp = BridgeParams::new(100_000_000, Some(1_000_000_000)).unwrap();
        let json = serde_json::to_string(&bp).unwrap();
        let decoded: BridgeParams = serde_json::from_str(&json).unwrap();
        assert_eq!(bp, decoded);
    }

    #[test]
    fn serde_rejects_zero_denomination() {
        let json = r#"{"denomination":0,"max_withdrawal_amount":null}"#;
        assert!(serde_json::from_str::<BridgeParams>(json).is_err());
    }

    #[test]
    fn serde_rejects_invalid_cap() {
        let json = r#"{"denomination":100000000,"max_withdrawal_amount":150000000}"#;
        assert!(serde_json::from_str::<BridgeParams>(json).is_err());
    }

    #[test]
    fn ssz_roundtrip() {
        let bp = BridgeParams::new(100_000_000, Some(1_000_000_000)).unwrap();
        let encoded = ssz::Encode::as_ssz_bytes(&bp);
        let decoded = <BridgeParams as ssz::Decode>::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(bp, decoded);
    }

    proptest::proptest! {
        /// Any valid params survive an SSZ encode/decode roundtrip unchanged.
        #[test]
        fn ssz_roundtrip_valid(denom in 1u64..=1_000_000u64, mult in 0u64..=1_000u64) {
            // `mult == 0` -> no cap; otherwise the cap is a multiple of and >= the
            // denomination, so `new` always succeeds for these bounded inputs.
            let cap = if mult == 0 { None } else { Some(denom * mult) };
            let bp = BridgeParams::new(denom, cap).unwrap();
            let encoded = ssz::Encode::as_ssz_bytes(&bp);
            let decoded = <BridgeParams as ssz::Decode>::from_ssz_bytes(&encoded).unwrap();
            proptest::prop_assert_eq!(bp, decoded);
        }
    }

    /// SSZ decode re-validates invariants: bytes encoding a zero denomination are
    /// rejected rather than decoded into an illegal value. `denomination` is the
    /// leading fixed field, so overwriting the first 8 bytes targets it directly.
    #[test]
    fn ssz_decode_rejects_zero_denomination() {
        let bp = BridgeParams::new(100_000_000, None).unwrap();
        let mut encoded = ssz::Encode::as_ssz_bytes(&bp);
        encoded[0..8].copy_from_slice(&0u64.to_le_bytes());
        let err = <BridgeParams as ssz::Decode>::from_ssz_bytes(&encoded).unwrap_err();
        assert!(matches!(err, ssz::DecodeError::BytesInvalid(_)));
    }

    /// SSZ decode rejects a denomination that exceeds the encoded cap.
    #[test]
    fn ssz_decode_rejects_max_below_denomination() {
        let bp = BridgeParams::new(100_000_000, Some(1_000_000_000)).unwrap();
        let mut encoded = ssz::Encode::as_ssz_bytes(&bp);
        // Overwrite the denomination to exceed the 1 BTC cap.
        encoded[0..8].copy_from_slice(&1_500_000_000u64.to_le_bytes());
        let err = <BridgeParams as ssz::Decode>::from_ssz_bytes(&encoded).unwrap_err();
        assert!(matches!(err, ssz::DecodeError::BytesInvalid(_)));
    }
}
