//! Signature types for threshold signing.

use std::collections::HashSet;

use borsh::{BorshDeserialize, BorshSerialize};
use ssz::{Decode, DecodeError, Encode};
use ssz_derive::{Decode as DeriveDecode, Encode as DeriveEncode};

use super::ThresholdSignatureError;

/// An ECDSA signature with its signer index.
///
/// The signature is in recoverable format (65 bytes): `header || r || s`.
///
/// # Hardware Wallet Compatibility
///
/// The first byte (header) can be in two formats:
///
/// 1. **Raw recovery ID** (0-3): Used by some signing libraries
/// 2. **BIP-137 format** (27-42): Used by Bitcoin message signing in hardware wallets
///    - 27-30: Uncompressed P2PKH
///    - 31-34: Compressed P2PKH (most common for Ledger/Trezor)
///    - 35-38: SegWit P2SH-P2WPKH
///    - 39-42: Native SegWit P2WPKH
///
/// The verification code normalizes both formats to extract the raw recovery ID (0-3).
///
/// The signer includes their own index (position in `ThresholdConfig::keys`) when creating
/// an `IndexedSignature`. Verification uses that index to fetch the expected public key and
/// compare it against the recovered key from the signature.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct IndexedSignature {
    /// Index of the signer in the ThresholdConfig keys array (0-255).
    index: u8,
    /// 65-byte recoverable ECDSA signature (header || r || s).
    ///
    /// The header byte contains the recovery ID, possibly with BIP-137 address type encoding.
    /// See struct-level documentation for format details.
    signature: [u8; 65],
}

/// SSZ-friendly representation of [`IndexedSignature`].
#[derive(DeriveEncode, DeriveDecode)]
struct IndexedSignatureSsz {
    /// The index of the signer in the ThresholdConfig keys array (0-255).
    index: u8,

    /// The 65-byte recoverable ECDSA signature (header || r || s).
    signature: Vec<u8>,
}

impl IndexedSignature {
    /// Create a new indexed signature.
    pub fn new(index: u8, signature: [u8; 65]) -> Self {
        Self { index, signature }
    }

    /// Get the signer index.
    pub fn index(&self) -> u8 {
        self.index
    }

    /// Get the header byte (first byte of the signature).
    ///
    /// This byte contains the recovery ID, possibly encoded in BIP-137 format.
    /// The verification code handles normalization automatically.
    ///
    /// # Format
    /// - Raw: 0-3 (recovery ID directly)
    /// - BIP-137: 27-42 (encodes address type + recovery ID)
    pub fn recovery_id(&self) -> u8 {
        self.signature[0]
    }

    /// Get the r component (bytes 1-32).
    pub fn r(&self) -> &[u8; 32] {
        self.signature[1..33]
            .try_into()
            .expect("signature[1..33] is always 32 bytes")
    }

    /// Get the s component (bytes 33-64).
    pub fn s(&self) -> &[u8; 32] {
        self.signature[33..65]
            .try_into()
            .expect("signature[33..65] is always 32 bytes")
    }

    /// Get the compact signature (r || s) without recovery ID.
    pub fn compact(&self) -> [u8; 64] {
        let mut compact = [0u8; 64];
        compact.copy_from_slice(&self.signature[1..65]);
        compact
    }
}

impl Encode for IndexedSignature {
    fn is_ssz_fixed_len() -> bool {
        <IndexedSignatureSsz as Encode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <IndexedSignatureSsz as Encode>::ssz_fixed_len()
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        IndexedSignatureSsz {
            index: self.index,
            signature: self.signature.to_vec(),
        }
        .ssz_append(buf);
    }

    fn ssz_bytes_len(&self) -> usize {
        IndexedSignatureSsz {
            index: self.index,
            signature: self.signature.to_vec(),
        }
        .ssz_bytes_len()
    }
}

impl Decode for IndexedSignature {
    fn is_ssz_fixed_len() -> bool {
        <IndexedSignatureSsz as Decode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <IndexedSignatureSsz as Decode>::ssz_fixed_len()
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let value = IndexedSignatureSsz::from_ssz_bytes(bytes)?;
        let signature: [u8; 65] = value.signature.try_into().map_err(|actual: Vec<u8>| {
            DecodeError::BytesInvalid(format!(
                "indexed signature must be 65 bytes, got {}",
                actual.len()
            ))
        })?;

        Ok(Self {
            index: value.index,
            signature,
        })
    }
}

/// A set of indexed ECDSA signatures for threshold verification.
///
/// Signatures are guaranteed duplicate-free.
#[derive(Debug, Clone, PartialEq, Eq, Default, BorshSerialize, BorshDeserialize)]
pub struct SignatureSet {
    /// Sorted signatures by index, no duplicates.
    signatures: Vec<IndexedSignature>,
}

/// SSZ-friendly representation of [`SignatureSet`].
#[derive(DeriveEncode, DeriveDecode)]
struct SignatureSetSsz {
    /// Sorted signatures by index, no duplicates.
    signatures: Vec<IndexedSignature>,
}

impl SignatureSet {
    /// Create a new signature set from a vector of indexed signatures.
    ///
    /// The signatures will be checked for duplicates.
    pub fn new(signatures: Vec<IndexedSignature>) -> Result<Self, ThresholdSignatureError> {
        let mut seen = HashSet::new();
        for sig in &signatures {
            if !seen.insert(sig.index) {
                return Err(ThresholdSignatureError::DuplicateSignerIndex(sig.index));
            }
        }

        Ok(Self { signatures })
    }

    /// Create an empty signature set.
    pub fn empty() -> Self {
        Self {
            signatures: Vec::new(),
        }
    }

    /// Get the signatures.
    pub fn signatures(&self) -> &[IndexedSignature] {
        &self.signatures
    }

    /// Get the number of signatures.
    pub fn len(&self) -> usize {
        self.signatures.len()
    }

    /// Check if the signature set is empty.
    pub fn is_empty(&self) -> bool {
        self.signatures.is_empty()
    }

    /// Iterate over signer indices.
    pub fn indices(&self) -> impl Iterator<Item = u8> + '_ {
        self.signatures.iter().map(|s| s.index)
    }

    /// Consume and return the inner signatures.
    pub fn into_inner(self) -> Vec<IndexedSignature> {
        self.signatures
    }
}

impl Encode for SignatureSet {
    fn is_ssz_fixed_len() -> bool {
        <SignatureSetSsz as Encode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <SignatureSetSsz as Encode>::ssz_fixed_len()
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        SignatureSetSsz {
            signatures: self.signatures.clone(),
        }
        .ssz_append(buf);
    }

    fn ssz_bytes_len(&self) -> usize {
        SignatureSetSsz {
            signatures: self.signatures.clone(),
        }
        .ssz_bytes_len()
    }
}

impl Decode for SignatureSet {
    fn is_ssz_fixed_len() -> bool {
        <SignatureSetSsz as Decode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <SignatureSetSsz as Decode>::ssz_fixed_len()
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let value = SignatureSetSsz::from_ssz_bytes(bytes)?;
        Self::new(value.signatures).map_err(|err| DecodeError::BytesInvalid(err.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sig(index: u8) -> IndexedSignature {
        let mut signature = [0u8; 65];
        signature[0] = 27; // recovery id
        signature[1] = index; // put index in r for easy identification
        IndexedSignature::new(index, signature)
    }

    #[test]
    fn test_signature_set_creation() {
        let sigs = vec![make_sig(2), make_sig(0), make_sig(1)];
        let set = SignatureSet::new(sigs).unwrap();

        assert_eq!(set.signatures()[0].index(), 2);
        assert_eq!(set.signatures()[1].index(), 0);
        assert_eq!(set.signatures()[2].index(), 1);
    }

    #[test]
    fn test_signature_set_duplicate_index() {
        let sigs = vec![make_sig(1), make_sig(1)];
        let result = SignatureSet::new(sigs);
        assert!(matches!(
            result,
            Err(ThresholdSignatureError::DuplicateSignerIndex(1))
        ));
    }

    #[test]
    fn test_signature_set_borsh_roundtrip() {
        let sigs = vec![make_sig(0), make_sig(2), make_sig(5)];
        let set = SignatureSet::new(sigs).unwrap();

        let encoded = borsh::to_vec(&set).unwrap();
        let decoded: SignatureSet = borsh::from_slice(&encoded).unwrap();

        assert_eq!(set, decoded);
    }

    #[test]
    fn test_indexed_signature_components() {
        let mut signature = [0u8; 65];
        // BIP-137 format: 27 = uncompressed P2PKH with recid 0
        signature[0] = 27;
        signature[1..33].copy_from_slice(&[0xAA; 32]); // r
        signature[33..65].copy_from_slice(&[0xBB; 32]); // s

        let sig = IndexedSignature::new(5, signature);

        assert_eq!(sig.index(), 5);
        assert_eq!(sig.recovery_id(), 27); // Raw header byte (verification normalizes this)
        assert_eq!(sig.r(), &[0xAA; 32]);
        assert_eq!(sig.s(), &[0xBB; 32]);
    }

    #[test]
    fn test_indexed_signature_raw_recid() {
        let mut signature = [0u8; 65];
        // Raw format: recid 1 directly
        signature[0] = 1;
        signature[1..33].copy_from_slice(&[0xCC; 32]); // r
        signature[33..65].copy_from_slice(&[0xDD; 32]); // s

        let sig = IndexedSignature::new(3, signature);

        assert_eq!(sig.index(), 3);
        assert_eq!(sig.recovery_id(), 1); // Raw recovery ID
        assert_eq!(sig.r(), &[0xCC; 32]);
        assert_eq!(sig.s(), &[0xDD; 32]);
    }
}
