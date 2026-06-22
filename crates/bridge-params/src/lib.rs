//! Bridge denomination and cap parameters.

#[cfg(feature = "arbitrary")]
use arbitrary::{Arbitrary, Unstructured};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as DeError};
use ssz::{
    Decode as SszDecode, DecodeError,
    view::{DecodeView, SszTypeInfo},
};
use ssz_derive::Encode;
use ssz_types::view::ToOwnedSsz;
use thiserror::Error;
use tree_hash::{PackedEncoding, TreeHash, TreeHashDigest, TreeHashType};
use tree_hash_derive::TreeHash;

/// Sentinel used in the SSZ representation to encode an uncapped withdrawal amount.
///
/// The public API exposes this as [`None`] via [`BridgeParams::max_withdrawal_amount`].
const NO_MAX_WITHDRAWAL_AMOUNT: u64 = u64::MAX;
const SSZ_U64_LEN: usize = 8;
const BRIDGE_PARAMS_SSZ_LEN: usize = SSZ_U64_LEN * 2;

/// Bridge denomination and optional maximum withdrawal amount, in satoshis.
///
/// Constructed via [`BridgeParams::new`] which validates the invariants:
/// - `denomination` must be non-zero
/// - If `max_withdrawal_amount` is set, it must be `>= denomination` and a multiple of it
#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, TreeHash)]
pub struct BridgeParams {
    denomination: u64,
    /// Maximum withdrawal amount in satoshis, or [`NO_MAX_WITHDRAWAL_AMOUNT`] for no cap.
    max_withdrawal_amount: u64,
}

/// Raw mirror for deserialization.
#[derive(Deserialize, Serialize)]
struct BridgeParamsRaw {
    denomination: u64,
    max_withdrawal_amount: Option<u64>,
}

impl<'de> Deserialize<'de> for BridgeParams {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = BridgeParamsRaw::deserialize(deserializer)?;
        Self::new(raw.denomination, raw.max_withdrawal_amount).map_err(DeError::custom)
    }
}

impl Serialize for BridgeParams {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        BridgeParamsRaw {
            denomination: self.denomination,
            max_withdrawal_amount: self.max_withdrawal_amount(),
        }
        .serialize(serializer)
    }
}

impl SszDecode for BridgeParams {
    fn is_ssz_fixed_len() -> bool {
        true
    }

    fn ssz_fixed_len() -> usize {
        BRIDGE_PARAMS_SSZ_LEN
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        if bytes.len() != Self::ssz_fixed_len() {
            return Err(DecodeError::BytesInvalid(format!(
                "expected {} bytes, got {}",
                Self::ssz_fixed_len(),
                bytes.len()
            )));
        }

        let denomination = <u64 as SszDecode>::from_ssz_bytes(&bytes[..SSZ_U64_LEN])?;
        let max_withdrawal_amount =
            <u64 as SszDecode>::from_ssz_bytes(&bytes[SSZ_U64_LEN..BRIDGE_PARAMS_SSZ_LEN])?;

        Self::new(
            denomination,
            decode_max_withdrawal_amount(max_withdrawal_amount),
        )
        .map_err(|e| DecodeError::BytesInvalid(e.to_string()))
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
            if max == NO_MAX_WITHDRAWAL_AMOUNT {
                return Err(BridgeParamsError::ReservedMaxWithdrawalAmount);
            }
            if max < denomination {
                return Err(BridgeParamsError::MaxBelowDenomination { denomination, max });
            }
            if max % denomination != 0 {
                return Err(BridgeParamsError::MaxNotMultiple { denomination, max });
            }
        }
        Ok(Self {
            denomination,
            max_withdrawal_amount: encode_max_withdrawal_amount(max_withdrawal_amount),
        })
    }

    pub fn denomination(&self) -> u64 {
        self.denomination
    }

    pub fn max_withdrawal_amount(&self) -> Option<u64> {
        decode_max_withdrawal_amount(self.max_withdrawal_amount)
    }

    /// Returns whether a withdrawal amount (in sats) is valid.
    pub fn validate_withdrawal_amount(&self, amount_sats: u64) -> bool {
        amount_sats > 0
            && amount_sats.is_multiple_of(self.denomination)
            && self
                .max_withdrawal_amount()
                .is_none_or(|cap| amount_sats <= cap)
    }
}

#[cfg(feature = "arbitrary")]
impl<'a> Arbitrary<'a> for BridgeParams {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let denomination = u64::arbitrary(u)?.max(1);
        let max_multiplier = Option::<u16>::arbitrary(u)?;
        let max_withdrawal_amount = max_multiplier
            .map(|multiplier| denomination.saturating_mul(u64::from(multiplier).max(1)))
            .filter(|max| *max != NO_MAX_WITHDRAWAL_AMOUNT);
        Ok(Self::new(denomination, max_withdrawal_amount).expect("generated params are valid"))
    }
}

impl<'a> DecodeView<'a> for BridgeParams {
    fn from_ssz_bytes(bytes: &'a [u8]) -> Result<Self, DecodeError> {
        <Self as SszDecode>::from_ssz_bytes(bytes)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BridgeParamsRef<'a> {
    bytes: &'a [u8],
}

impl<'a> DecodeView<'a> for BridgeParamsRef<'a> {
    fn from_ssz_bytes(bytes: &'a [u8]) -> Result<Self, DecodeError> {
        <BridgeParams as SszDecode>::from_ssz_bytes(bytes)?;
        Ok(Self { bytes })
    }
}

impl SszTypeInfo for BridgeParamsRef<'_> {
    fn is_ssz_fixed_len() -> bool {
        <BridgeParams as SszDecode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <BridgeParams as SszDecode>::ssz_fixed_len()
    }
}

impl ToOwnedSsz<BridgeParams> for BridgeParamsRef<'_> {
    fn to_owned(&self) -> BridgeParams {
        <BridgeParams as SszDecode>::from_ssz_bytes(self.bytes)
            .expect("BridgeParamsRef validates bytes on construction")
    }
}

impl TreeHash for BridgeParamsRef<'_> {
    fn tree_hash_type() -> TreeHashType {
        <BridgeParams as TreeHash>::tree_hash_type()
    }

    fn tree_hash_packed_encoding(&self) -> PackedEncoding {
        let owned = <Self as ToOwnedSsz<BridgeParams>>::to_owned(self);
        <BridgeParams as TreeHash>::tree_hash_packed_encoding(&owned)
    }

    fn tree_hash_packing_factor() -> usize {
        <BridgeParams as TreeHash>::tree_hash_packing_factor()
    }

    fn tree_hash_root<H: TreeHashDigest>(&self) -> H::Output {
        let owned = <Self as ToOwnedSsz<BridgeParams>>::to_owned(self);
        <BridgeParams as TreeHash>::tree_hash_root::<H>(&owned)
    }
}

fn encode_max_withdrawal_amount(max_withdrawal_amount: Option<u64>) -> u64 {
    max_withdrawal_amount.unwrap_or(NO_MAX_WITHDRAWAL_AMOUNT)
}

fn decode_max_withdrawal_amount(max_withdrawal_amount: u64) -> Option<u64> {
    (max_withdrawal_amount != NO_MAX_WITHDRAWAL_AMOUNT).then_some(max_withdrawal_amount)
}

/// Default bridge denomination: 1 BTC in satoshis.
pub const DEFAULT_DENOMINATION_SATS: u64 = 100_000_000;

/// Default maximum withdrawal amount: 10 BTC in satoshis.
pub const DEFAULT_MAX_WITHDRAWAL_SATS: u64 = 1_000_000_000;

impl Default for BridgeParams {
    fn default() -> Self {
        Self {
            denomination: DEFAULT_DENOMINATION_SATS,
            max_withdrawal_amount: DEFAULT_MAX_WITHDRAWAL_SATS,
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

    #[error("max_withdrawal_amount reserves u64::MAX as the no-cap sentinel")]
    ReservedMaxWithdrawalAmount,
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
}
