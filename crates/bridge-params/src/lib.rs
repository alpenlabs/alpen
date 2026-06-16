//! Bridge denomination and cap parameters.

#[cfg(feature = "arbitrary")]
use arbitrary::Arbitrary;
use serde::{Deserialize, Serialize, de::Error as DeError};
use ssz_derive::{Decode, Encode};
use thiserror::Error;

/// Bridge denomination and withdrawal policy parameters.
///
/// Constructed via [`BridgeParams::new`] which validates the invariants:
/// - `denomination` must be non-zero
/// - If `max_withdrawal_amount` is set, it must be `>= denomination` and a multiple of it
/// - `max_withdrawal_descriptor_len` must fit within the withdrawal message descriptor cap
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Encode)]
#[cfg_attr(feature = "arbitrary", derive(Arbitrary))]
pub struct BridgeParams {
    denomination: u64,
    max_withdrawal_amount: Option<u64>,
    max_withdrawal_descriptor_len: u32,
}

/// Raw mirror for deserialization. Serde and SSZ decode into this first,
/// then validate via [`BridgeParams::new_with_descriptor_limit`].
#[derive(Deserialize, Decode)]
struct BridgeParamsRaw {
    denomination: u64,
    max_withdrawal_amount: Option<u64>,
    #[serde(default = "default_max_withdrawal_descriptor_len")]
    max_withdrawal_descriptor_len: u32,
}

impl<'de> Deserialize<'de> for BridgeParams {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = BridgeParamsRaw::deserialize(deserializer)?;
        Self::new_with_descriptor_limit(
            raw.denomination,
            raw.max_withdrawal_amount,
            raw.max_withdrawal_descriptor_len,
        )
        .map_err(DeError::custom)
    }
}

impl ssz::Decode for BridgeParams {
    fn is_ssz_fixed_len() -> bool {
        BridgeParamsRaw::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        BridgeParamsRaw::ssz_fixed_len()
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, ssz::DecodeError> {
        let raw = BridgeParamsRaw::from_ssz_bytes(bytes)?;
        Self::new_with_descriptor_limit(
            raw.denomination,
            raw.max_withdrawal_amount,
            raw.max_withdrawal_descriptor_len,
        )
        .map_err(|e| ssz::DecodeError::BytesInvalid(e.to_string()))
    }
}

impl BridgeParams {
    /// Creates a new [`BridgeParams`] using the default descriptor length limit.
    pub fn new(
        denomination: u64,
        max_withdrawal_amount: Option<u64>,
    ) -> Result<Self, BridgeParamsError> {
        Self::new_with_descriptor_limit(
            denomination,
            max_withdrawal_amount,
            DEFAULT_MAX_WITHDRAWAL_DESCRIPTOR_LEN,
        )
    }

    /// Creates a new [`BridgeParams`] after validating invariants.
    pub fn new_with_descriptor_limit(
        denomination: u64,
        max_withdrawal_amount: Option<u64>,
        max_withdrawal_descriptor_len: u32,
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
        if max_withdrawal_descriptor_len == 0 {
            return Err(BridgeParamsError::ZeroMaxWithdrawalDescriptorLen);
        }
        if max_withdrawal_descriptor_len > MAX_WITHDRAWAL_DESCRIPTOR_LEN {
            return Err(BridgeParamsError::MaxWithdrawalDescriptorLenTooLarge {
                max: max_withdrawal_descriptor_len,
                cap: MAX_WITHDRAWAL_DESCRIPTOR_LEN,
            });
        }
        Ok(Self {
            denomination,
            max_withdrawal_amount,
            max_withdrawal_descriptor_len,
        })
    }

    pub fn denomination(&self) -> u64 {
        self.denomination
    }

    pub fn max_withdrawal_amount(&self) -> Option<u64> {
        self.max_withdrawal_amount
    }

    pub fn max_withdrawal_descriptor_len(&self) -> u32 {
        self.max_withdrawal_descriptor_len
    }

    /// Returns whether a withdrawal amount (in sats) is valid.
    pub fn validate_withdrawal_amount(&self, amount_sats: u64) -> bool {
        amount_sats > 0
            && amount_sats.is_multiple_of(self.denomination)
            && self
                .max_withdrawal_amount
                .is_none_or(|cap| amount_sats <= cap)
    }

    /// Returns whether a withdrawal destination BOSD descriptor byte length is valid.
    pub fn validate_withdrawal_descriptor_len(&self, len: usize) -> bool {
        len > 0 && len <= self.max_withdrawal_descriptor_len as usize
    }
}

/// Default bridge denomination: 1 BTC in satoshis.
pub const DEFAULT_DENOMINATION_SATS: u64 = 100_000_000;

/// Default maximum withdrawal amount: 10 BTC in satoshis.
pub const DEFAULT_MAX_WITHDRAWAL_SATS: u64 = 1_000_000_000;

/// Default maximum BOSD descriptor length for withdrawals, including the type tag.
pub const DEFAULT_MAX_WITHDRAWAL_DESCRIPTOR_LEN: u32 = 81;

/// Maximum BOSD descriptor length accepted by withdrawal message data.
pub const MAX_WITHDRAWAL_DESCRIPTOR_LEN: u32 = 255;

const fn default_max_withdrawal_descriptor_len() -> u32 {
    DEFAULT_MAX_WITHDRAWAL_DESCRIPTOR_LEN
}

impl Default for BridgeParams {
    fn default() -> Self {
        Self {
            denomination: DEFAULT_DENOMINATION_SATS,
            max_withdrawal_amount: Some(DEFAULT_MAX_WITHDRAWAL_SATS),
            max_withdrawal_descriptor_len: DEFAULT_MAX_WITHDRAWAL_DESCRIPTOR_LEN,
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

    #[error("max_withdrawal_descriptor_len must not be zero")]
    ZeroMaxWithdrawalDescriptorLen,

    #[error("max_withdrawal_descriptor_len ({max}) exceeds descriptor cap ({cap})")]
    MaxWithdrawalDescriptorLenTooLarge { max: u32, cap: u32 },
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
    fn default_descriptor_len_is_81() {
        assert_eq!(
            BridgeParams::default().max_withdrawal_descriptor_len(),
            DEFAULT_MAX_WITHDRAWAL_DESCRIPTOR_LEN
        );
        assert_eq!(
            BridgeParams::new(100_000_000, Some(1_000_000_000))
                .unwrap()
                .max_withdrawal_descriptor_len(),
            DEFAULT_MAX_WITHDRAWAL_DESCRIPTOR_LEN
        );
    }

    #[test]
    fn valid_custom_descriptor_len() {
        let w = BridgeParams::new_with_descriptor_limit(100_000_000, None, 100).unwrap();
        assert!(w.validate_withdrawal_descriptor_len(100));
        assert!(!w.validate_withdrawal_descriptor_len(101));
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
    fn zero_descriptor_len_rejected() {
        assert!(BridgeParams::new_with_descriptor_limit(100_000_000, None, 0).is_err());
    }

    #[test]
    fn descriptor_len_over_cap_rejected() {
        assert!(
            BridgeParams::new_with_descriptor_limit(
                100_000_000,
                None,
                MAX_WITHDRAWAL_DESCRIPTOR_LEN + 1
            )
            .is_err()
        );
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
    fn serde_missing_descriptor_len_uses_default() {
        let json = r#"{"denomination":100000000,"max_withdrawal_amount":null}"#;
        let bp: BridgeParams = serde_json::from_str(json).unwrap();
        assert_eq!(
            bp.max_withdrawal_descriptor_len(),
            DEFAULT_MAX_WITHDRAWAL_DESCRIPTOR_LEN
        );
    }

    #[test]
    fn serde_rejects_invalid_descriptor_len() {
        let json = r#"{"denomination":100000000,"max_withdrawal_amount":null,"max_withdrawal_descriptor_len":256}"#;
        assert!(serde_json::from_str::<BridgeParams>(json).is_err());
    }

    #[test]
    fn ssz_roundtrip() {
        let bp =
            BridgeParams::new_with_descriptor_limit(100_000_000, Some(1_000_000_000), 100).unwrap();
        let encoded = ssz::Encode::as_ssz_bytes(&bp);
        let decoded = <BridgeParams as ssz::Decode>::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(bp, decoded);
    }
}
