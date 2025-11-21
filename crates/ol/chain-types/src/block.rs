use strata_asm_common::AsmManifest;
use strata_codec::{Codec, CodecError, Decoder, Encoder, encode_to_vec};
use strata_codec_derive::Codec;
use strata_identifiers::{Buf32, Buf64, L1BlockId, hash::raw};

use crate::{Epoch, OLBlockId, OLTransaction, Slot};

/// Signed full orchestration layer block.
#[derive(Clone, Debug)]
pub struct OLBlock {
    signed_header: SignedOLBlockHeader,
    body: OLBlockBody,
}

impl OLBlock {
    pub fn new(signed_header: SignedOLBlockHeader, body: OLBlockBody) -> Self {
        Self {
            signed_header,
            body,
        }
    }

    pub fn signed_header(&self) -> &SignedOLBlockHeader {
        &self.signed_header
    }

    /// Returns the actual block header inside the signed header structure.
    pub fn header(&self) -> &OLBlockHeader {
        self.signed_header.header()
    }

    pub fn body(&self) -> &OLBlockBody {
        &self.body
    }
}

/// OL header with signature.
#[derive(Clone, Debug)]
pub struct SignedOLBlockHeader {
    header: OLBlockHeader,
    signature: Buf64,
}

impl SignedOLBlockHeader {
    pub fn new(header: OLBlockHeader, signature: Buf64) -> Self {
        Self { header, signature }
    }

    pub fn header(&self) -> &OLBlockHeader {
        &self.header
    }

    /// This MUST be a schnorr signature for now.
    pub fn signature(&self) -> &Buf64 {
        &self.signature
    }
}

/// OL header.
///
/// This should not be directly used itself during execution.
#[derive(Clone, Debug, Codec)]
pub struct OLBlockHeader {
    /// The timestamp the block was created at.
    timestamp: u64,

    /// Slot the block was created for.
    slot: Slot,

    /// Epoch the block was created in.
    epoch: Epoch,

    /// Parent block id.
    parent_blkid: OLBlockId,

    /// Root of the block body.
    body_root: Buf32,

    /// The state root resulting after the block execution.
    state_root: Buf32,

    /// Root of the block logs.
    logs_root: Buf32,
}

impl OLBlockHeader {
    pub fn new(
        timestamp: u64,
        slot: Slot,
        epoch: Epoch,
        parent_blkid: OLBlockId,
        body_root: Buf32,
        state_root: Buf32,
        logs_root: Buf32,
    ) -> Self {
        Self {
            timestamp,
            slot,
            epoch,
            parent_blkid,
            body_root,
            state_root,
            logs_root,
        }
    }

    pub fn timestamp(&self) -> u64 {
        self.timestamp
    }

    pub fn slot(&self) -> u64 {
        self.slot
    }

    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    pub fn parent_blkid(&self) -> &OLBlockId {
        &self.parent_blkid
    }

    pub fn body_root(&self) -> &Buf32 {
        &self.body_root
    }

    pub fn state_root(&self) -> &Buf32 {
        &self.state_root
    }

    pub fn logs_root(&self) -> &Buf32 {
        &self.logs_root
    }

    /// Computes the block ID by hashing the header's Codec encoding.
    pub fn compute_blkid(&self) -> OLBlockId {
        let encoded = encode_to_vec(self).expect("header encoding should succeed");
        let hash = raw(&encoded);
        OLBlockId::from(hash)
    }
}

/// OL block body containing transactions and l1 updates
#[derive(Clone, Debug)]
pub struct OLBlockBody {
    /// The transactions contained in an OL block.
    tx_segment: OLTxSegment,

    /// Updates from L1.
    l1_update: Option<OLL1Update>,
}

impl OLBlockBody {
    pub(crate) fn new(tx_segment: OLTxSegment, l1_update: Option<OLL1Update>) -> Self {
        Self {
            tx_segment,
            l1_update,
        }
    }

    pub fn new_regular(tx_segment: OLTxSegment) -> Self {
        Self::new(tx_segment, None)
    }

    // TODO convert to builder?
    pub fn set_l1_update(&mut self, l1_update: OLL1Update) {
        self.l1_update = Some(l1_update);
    }

    pub fn tx_segment(&self) -> &OLTxSegment {
        &self.tx_segment
    }

    pub fn l1_update(&self) -> Option<&OLL1Update> {
        self.l1_update.as_ref()
    }

    /// Computes the hash commitment of this block body.
    pub fn compute_hash_commitment(&self) -> Buf32 {
        // Encode the block body and hash it
        let encoded = encode_to_vec(self).expect("block body encoding should succeed");
        let hash = raw(&encoded);
        hash
    }
}

impl Codec for OLBlockBody {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.tx_segment.encode(enc)?;
        // Encode Option as bool (is_some) followed by value if present
        match &self.l1_update {
            Some(update) => {
                true.encode(enc)?;
                update.encode(enc)?;
            }
            None => {
                false.encode(enc)?;
            }
        }
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let tx_segment = OLTxSegment::decode(dec)?;
        let l1_update = if bool::decode(dec)? {
            Some(OLL1Update::decode(dec)?)
        } else {
            None
        };
        Ok(Self {
            tx_segment,
            l1_update,
        })
    }
}

#[derive(Clone, Debug)]
pub struct OLTxSegment {
    /// Transactions in the segment.
    txs: Vec<OLTransaction>,
    // Add other attributes.
}

impl OLTxSegment {
    pub fn new(txs: Vec<OLTransaction>) -> Self {
        Self { txs }
    }

    pub fn txs(&self) -> &[OLTransaction] {
        &self.txs
    }
}

impl Codec for OLTxSegment {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        // Encode Vec as length followed by elements
        (self.txs.len() as u64).encode(enc)?;
        for tx in &self.txs {
            tx.encode(enc)?;
        }
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let len = u64::decode(dec)? as usize;
        let mut txs = Vec::with_capacity(len);
        for _ in 0..len {
            txs.push(OLTransaction::decode(dec)?);
        }
        Ok(Self { txs })
    }
}

/// Represents an update from L1.
#[derive(Clone, Debug, Codec)]
pub struct OLL1Update {
    /// The state root before applying updates from L1.
    preseal_state_root: Buf32,

    /// The manifests we extend the chain with.
    manifest_cont: OLL1ManifestContainer,
}

impl OLL1Update {
    pub fn new(preseal_state_root: Buf32, manifest_cont: OLL1ManifestContainer) -> Self {
        Self {
            preseal_state_root,
            manifest_cont,
        }
    }

    pub fn preseal_state_root(&self) -> &Buf32 {
        &self.preseal_state_root
    }

    pub fn manifest_cont(&self) -> &OLL1ManifestContainer {
        &self.manifest_cont
    }
}

#[derive(Clone, Debug)]
pub struct OLL1ManifestContainer {
    /// Manifests building on top of previous l1 height to the new l1 height.
    manifests: Vec<AsmManifest>,
}

impl OLL1ManifestContainer {
    pub fn new(manifests: Vec<AsmManifest>) -> Self {
        Self { manifests }
    }

    pub fn manifests(&self) -> &[AsmManifest] {
        &self.manifests
    }
}

impl Codec for OLL1ManifestContainer {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        // Encode Vec as length followed by elements
        (self.manifests.len() as u64).encode(enc)?;
        for manifest in &self.manifests {
            // Encode each manifest field directly
            manifest.blkid.encode(enc)?;
            manifest.wtxids_root.encode(enc)?;
            // Encode logs as length + elements
            (manifest.logs.len() as u64).encode(enc)?;
            for log in &manifest.logs {
                // We need to encode AsmLogEntry - let's encode it as bytes for now
                let bytes = borsh::to_vec(log)
                    .map_err(|_| CodecError::InvalidVariant("Failed to serialize AsmLogEntry"))?;
                (bytes.len() as u64).encode(enc)?;
                for byte in bytes {
                    byte.encode(enc)?;
                }
            }
        }
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let len = u64::decode(dec)? as usize;
        let mut manifests = Vec::with_capacity(len);
        for _ in 0..len {
            // Decode each manifest field
            let blkid = L1BlockId::decode(dec)?;
            let wtxids_root = Buf32::decode(dec)?;
            // Decode logs
            let logs_len = u64::decode(dec)? as usize;
            let mut logs = Vec::with_capacity(logs_len);
            for _ in 0..logs_len {
                // Decode AsmLogEntry from bytes
                let byte_len = u64::decode(dec)? as usize;
                let mut bytes = Vec::with_capacity(byte_len);
                for _ in 0..byte_len {
                    bytes.push(u8::decode(dec)?);
                }
                let log = borsh::from_slice(&bytes)
                    .map_err(|_| CodecError::InvalidVariant("Failed to deserialize AsmLogEntry"))?;
                logs.push(log);
            }
            manifests.push(AsmManifest::new(blkid, wtxids_root, logs));
        }
        Ok(Self { manifests })
    }
}
