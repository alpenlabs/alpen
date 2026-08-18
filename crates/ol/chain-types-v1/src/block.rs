//! Block-related types for OL chain.

use ssz::Encode;
use ssz_types::VariableList;
use strata_asm_common::AsmManifest;
use strata_crypto::hash;
use strata_identifiers::{Buf32, Buf64, Epoch, OLBlockCommitment, OLBlockId, Slot};
use strata_ol_tx_types_v1::OLTransactionV1;

use crate::block_flags::BlockFlagsV1;
use crate::error::ChainTypesError;
use crate::ssz_generated::ssz::block::{
    MAX_SEALING_MANIFEST_COUNT, MAX_TXS_PER_BLOCK, OLAsmManifestContainerV1, OLBlockBodyV1,
    OLBlockCredentialV1, OLBlockHeaderV1, OLBlockV1, OLTxSegmentV1, SignedOLBlockHeaderV1,
};

impl OLBlockV1 {
    pub fn new(signed_header: SignedOLBlockHeaderV1, body: OLBlockBodyV1) -> Self {
        Self {
            signed_header,
            body,
        }
    }

    pub fn signed_header(&self) -> &SignedOLBlockHeaderV1 {
        &self.signed_header
    }

    /// Returns the executionally-relevant block header inside the signed header
    /// structure.
    pub fn header(&self) -> &OLBlockHeaderV1 {
        &self.signed_header.header
    }

    pub fn body(&self) -> &OLBlockBodyV1 {
        &self.body
    }
}

impl SignedOLBlockHeaderV1 {
    pub fn new(header: OLBlockHeaderV1, signature: Buf64) -> Self {
        Self {
            header,
            credential: OLBlockCredentialV1 {
                schnorr_sig: Some(signature).into(),
            },
        }
    }

    pub fn header(&self) -> &OLBlockHeaderV1 {
        &self.header
    }

    /// This MUST be a schnorr signature over the `Codec`-encoded `header`.
    ///
    /// This is not currently checked anywhere.
    pub fn signature(&self) -> Option<&Buf64> {
        match &self.credential.schnorr_sig {
            ssz_types::Optional::Some(s) => Some(s),
            ssz_types::Optional::None => None,
        }
    }
}

impl OLBlockHeaderV1 {
    #[expect(clippy::too_many_arguments, reason = "headers are complicated")]
    pub fn new(
        timestamp: u64,
        flags: BlockFlagsV1,
        slot: Slot,
        epoch: Epoch,
        parent_blkid: OLBlockId,
        body_root: Buf32,
        state_root: Buf32,
        logs_root: Buf32,
    ) -> Self {
        Self {
            timestamp,
            flags,
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

    pub fn flags(&self) -> BlockFlagsV1 {
        self.flags
    }

    pub fn is_terminal(&self) -> bool {
        self.flags().is_terminal()
    }

    pub fn slot(&self) -> Slot {
        self.slot
    }

    /// Checks if this is header is the genesis slot, meaning that it's slot 0.
    pub fn is_genesis_slot(&self) -> bool {
        self.slot() == 0
    }

    pub fn epoch(&self) -> Epoch {
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

    /// Computes the block ID by hashing the header's SSZ encoding.
    pub fn compute_blkid(&self) -> OLBlockId {
        let encoded = self.as_ssz_bytes();
        let hash = hash::raw(&encoded);
        OLBlockId::from(hash)
    }

    /// Computes the block commitment.
    pub fn compute_block_commitment(&self) -> OLBlockCommitment {
        OLBlockCommitment::new(self.slot(), self.compute_blkid())
    }
}

impl OLBlockBodyV1 {
    pub fn new(tx_segment: OLTxSegmentV1, manifests: Option<OLAsmManifestContainerV1>) -> Self {
        Self {
            tx_segment: Some(tx_segment).into(),
            manifests: manifests.into(),
        }
    }

    /// Constructs a new instance for a common block with just a tx segment.
    pub fn new_common(tx_segment: OLTxSegmentV1) -> Self {
        Self::new(tx_segment, None)
    }

    // TODO(STR-3677): convert to builder?
    pub fn set_manifests(&mut self, manifests: OLAsmManifestContainerV1) {
        self.manifests = Some(manifests).into();
    }

    pub fn tx_segment(&self) -> Option<&OLTxSegmentV1> {
        match &self.tx_segment {
            ssz_types::Optional::Some(tx) => Some(tx),
            ssz_types::Optional::None => None,
        }
    }

    /// Returns the ASM manifest container included in this block, if any.
    ///
    /// Manifests may appear in any block within an epoch; their presence does
    /// not imply the block is an epoch terminal.
    pub fn manifests(&self) -> Option<&OLAsmManifestContainerV1> {
        match &self.manifests {
            ssz_types::Optional::Some(manifests) => Some(manifests),
            ssz_types::Optional::None => None,
        }
    }

    /// Computes the hash commitment of this block body.
    pub fn compute_hash_commitment(&self) -> Buf32 {
        let encoded = self.as_ssz_bytes();
        hash::raw(&encoded)
    }
}

impl OLTxSegmentV1 {
    pub fn new(txs: Vec<OLTransactionV1>) -> Result<Self, ChainTypesError> {
        let provided = txs.len();
        Ok(Self {
            txs: VariableList::new(txs).map_err(|_| ChainTypesError::TooManyTransactions {
                provided,
                max: MAX_TXS_PER_BLOCK as usize,
            })?,
        })
    }

    pub fn txs(&self) -> &[OLTransactionV1] {
        &self.txs
    }
}

impl OLAsmManifestContainerV1 {
    pub fn new(manifests: Vec<AsmManifest>) -> Result<Self, ChainTypesError> {
        let provided = manifests.len();
        Ok(Self {
            manifests: VariableList::new(manifests).map_err(|_| {
                ChainTypesError::TooManyManifests {
                    provided,
                    max: MAX_SEALING_MANIFEST_COUNT as usize,
                }
            })?,
        })
    }

    pub fn manifests(&self) -> &[AsmManifest] {
        &self.manifests
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use ssz::{Decode, Encode};
    use strata_identifiers::{Buf32, Buf64, OLBlockId};
    use strata_test_utils_ssz::ssz_proptest;

    use crate::block_flags::BlockFlagsV1;
    use crate::ssz_generated::ssz::block::{
        OLAsmManifestContainerV1, OLBlockBodyV1, OLBlockCredentialV1, OLBlockHeaderV1, OLBlockV1,
        OLTxSegmentV1, SignedOLBlockHeaderV1,
    };
    use crate::test_utils::{
        ol_block_body_strategy, ol_block_header_strategy, ol_block_strategy,
        ol_tx_segment_strategy, signed_ol_block_header_strategy,
    };

    mod ol_tx_segment {
        use super::*;

        ssz_proptest!(OLTxSegmentV1, ol_tx_segment_strategy());

        #[test]
        fn test_empty_segment() {
            let segment = OLTxSegmentV1 {
                txs: Vec::new()
                    .try_into()
                    .expect("transactions must fit within SSZ max length"),
            };
            let encoded = segment.as_ssz_bytes();
            let decoded = OLTxSegmentV1::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(segment, decoded);
        }
    }

    mod ol_manifest_container {
        use super::*;

        fn manifest_container_strategy() -> impl Strategy<Value = OLAsmManifestContainerV1> {
            Just(OLAsmManifestContainerV1::new(vec![]).expect("empty manifest should succeed"))
        }

        ssz_proptest!(OLAsmManifestContainerV1, manifest_container_strategy());

        #[test]
        fn test_empty_container() {
            let container =
                OLAsmManifestContainerV1::new(vec![]).expect("empty manifest should succeed");
            let encoded = container.as_ssz_bytes();
            let decoded = OLAsmManifestContainerV1::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(container, decoded);
        }
    }

    mod ol_block_header {
        use super::*;

        ssz_proptest!(OLBlockHeaderV1, ol_block_header_strategy());

        #[test]
        fn test_genesis_header() {
            let header = OLBlockHeaderV1 {
                timestamp: 0,
                flags: BlockFlagsV1::from(0),
                slot: 0,
                epoch: 0,
                parent_blkid: OLBlockId::from(Buf32::zero()),
                body_root: Buf32::zero(),
                state_root: Buf32::zero(),
                logs_root: Buf32::zero(),
            };
            let encoded = header.as_ssz_bytes();
            let decoded = OLBlockHeaderV1::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(header, decoded);
        }
    }

    mod signed_ol_block_header {
        use super::*;

        ssz_proptest!(SignedOLBlockHeaderV1, signed_ol_block_header_strategy());
    }

    mod ol_block_body {
        use super::*;

        ssz_proptest!(OLBlockBodyV1, ol_block_body_strategy());

        #[test]
        fn test_empty_body() {
            let body = OLBlockBodyV1 {
                tx_segment: Some(OLTxSegmentV1 {
                    txs: Vec::new()
                        .try_into()
                        .expect("transactions must fit within SSZ max length"),
                })
                .into(),
                manifests: Some(
                    OLAsmManifestContainerV1::new(vec![]).expect("empty manifest should succeed"),
                )
                .into(),
            };
            let encoded = body.as_ssz_bytes();
            let decoded = OLBlockBodyV1::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(body, decoded);
        }
    }

    mod ol_block {
        use super::*;

        ssz_proptest!(OLBlockV1, ol_block_strategy());

        #[test]
        fn test_minimal_block() {
            let block = OLBlockV1 {
                signed_header: SignedOLBlockHeaderV1 {
                    header: OLBlockHeaderV1 {
                        timestamp: 0,
                        flags: BlockFlagsV1::from(0),
                        slot: 0,
                        epoch: 0,
                        parent_blkid: OLBlockId::from(Buf32::zero()),
                        body_root: Buf32::zero(),
                        state_root: Buf32::zero(),
                        logs_root: Buf32::zero(),
                    },
                    credential: OLBlockCredentialV1 {
                        schnorr_sig: Some(Buf64::zero()).into(),
                    },
                },
                body: OLBlockBodyV1 {
                    tx_segment: Some(OLTxSegmentV1 {
                        txs: Vec::new()
                            .try_into()
                            .expect("transactions must fit within SSZ max length"),
                    })
                    .into(),
                    manifests: Some(
                        OLAsmManifestContainerV1::new(vec![])
                            .expect("empty manifest should succeed"),
                    )
                    .into(),
                },
            };
            let encoded = block.as_ssz_bytes();
            let decoded = OLBlockV1::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(block, decoded);
        }
    }
}
