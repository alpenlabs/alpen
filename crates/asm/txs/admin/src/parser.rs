use ssz::Decode;
use strata_asm_common::TxInputRef;
use strata_crypto::threshold_signature::{
    IndexedSignature as NativeIndexedSignature, SignatureSet as NativeSignatureSet,
    ThresholdSignatureError,
};
use strata_l1_envelope_fmt::parser::parse_envelope_payload;

use crate::{
    IndexedSignature, MultisigAction, SignatureSet, SignedPayload,
    errors::AdministrationTxParseError,
};

impl SignedPayload {
    /// Creates a new signed payload combining an action with its signatures.
    pub fn new(seqno: u64, action: MultisigAction, signatures: SignatureSet) -> Self {
        Self {
            seqno,
            action,
            signatures,
        }
    }
}

impl IndexedSignature {
    pub fn new(index: u8, signature: [u8; 65]) -> Self {
        Self {
            index,
            signature: signature.into(),
        }
    }

    pub fn from_native(signature: NativeIndexedSignature) -> Self {
        let mut bytes = [0u8; 65];
        bytes[0] = signature.recovery_id();
        bytes[1..33].copy_from_slice(signature.r());
        bytes[33..65].copy_from_slice(signature.s());
        Self::new(signature.index(), bytes)
    }

    pub fn index(&self) -> u8 {
        self.index
    }

    pub fn recovery_id(&self) -> u8 {
        self.signature.0[0]
    }

    pub fn r(&self) -> &[u8; 32] {
        self.signature.0[1..33]
            .try_into()
            .expect("signature r bytes are always 32 bytes")
    }

    pub fn s(&self) -> &[u8; 32] {
        self.signature.0[33..65]
            .try_into()
            .expect("signature s bytes are always 32 bytes")
    }

    pub fn compact(&self) -> [u8; 64] {
        let mut compact = [0u8; 64];
        compact.copy_from_slice(&self.signature.0[1..65]);
        compact
    }

    pub fn to_native(&self) -> NativeIndexedSignature {
        let mut bytes = [0u8; 65];
        bytes.copy_from_slice(&self.signature.0);
        NativeIndexedSignature::new(self.index, bytes)
    }

    pub fn into_native(self) -> NativeIndexedSignature {
        self.to_native()
    }
}

impl SignatureSet {
    pub fn new(signatures: Vec<IndexedSignature>) -> Result<Self, ThresholdSignatureError> {
        NativeSignatureSet::new(signatures.iter().map(IndexedSignature::to_native).collect())?;
        Ok(Self {
            signatures: signatures.into(),
        })
    }

    pub fn from_native(signatures: NativeSignatureSet) -> Self {
        Self {
            signatures: signatures
                .into_inner()
                .into_iter()
                .map(IndexedSignature::from_native)
                .collect::<Vec<_>>()
                .into(),
        }
    }

    pub fn empty() -> Self {
        Self {
            signatures: vec![].into(),
        }
    }

    pub fn signatures(&self) -> &[IndexedSignature] {
        &self.signatures
    }

    pub fn len(&self) -> usize {
        self.signatures.len()
    }

    pub fn is_empty(&self) -> bool {
        self.signatures.is_empty()
    }

    pub fn indices(&self) -> impl Iterator<Item = u8> + '_ {
        self.signatures.iter().map(|signature| signature.index())
    }

    pub fn into_inner(self) -> Vec<IndexedSignature> {
        self.signatures.into_iter().collect()
    }

    pub fn to_native(&self) -> Result<NativeSignatureSet, ThresholdSignatureError> {
        NativeSignatureSet::new(
            self.signatures
                .iter()
                .map(IndexedSignature::to_native)
                .collect(),
        )
    }

    pub fn into_native(self) -> Result<NativeSignatureSet, ThresholdSignatureError> {
        NativeSignatureSet::new(
            self.signatures
                .into_iter()
                .map(IndexedSignature::into_native)
                .collect(),
        )
    }
}

/// Parses a transaction to extract both the multisig action and the signature set.
///
/// This function extracts the signed payload from the taproot leaf script embedded
/// in the transaction's witness data. The payload contains both the administrative
/// action and its authorizing signatures.
///
/// # Arguments
/// * `tx` - A reference to the transaction input to parse
///
/// # Errors
/// Returns `AdministrationTxParseError` if:
/// - The transaction lacks a taproot leaf script in its witness
/// - The envelope payload cannot be parsed
/// - The signed payload cannot be deserialized
// TODO: https://alpenlabs.atlassian.net/browse/STR-2366
pub fn parse_tx(tx: &TxInputRef<'_>) -> Result<SignedPayload, AdministrationTxParseError> {
    let tx_type = tx.tag().tx_type();

    // Extract the taproot leaf script from the first input's witness
    let payload_script = tx.tx().input[0]
        .witness
        .taproot_leaf_script()
        .ok_or(AdministrationTxParseError::MalformedTransaction(tx_type))?
        .script;

    // Parse the envelope payload from the script
    let envelope_payload = parse_envelope_payload(&payload_script.into())?;

    // Deserialize the signed payload (action + signatures) from the envelope.
    let signed_payload = SignedPayload::from_ssz_bytes(&envelope_payload)
        .map_err(|_| AdministrationTxParseError::MalformedTransaction(tx_type))?;

    Ok(signed_payload)
}
