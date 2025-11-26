//! Block-related types for OL chain.

use ssz_types::VariableList;
use strata_acct_types::VarVec;
use strata_asm_common::AsmManifest;
use strata_codec::{Codec, CodecError, Decoder, Encoder, encode_to_vec};
use strata_identifiers::{Buf32, Buf64, L1BlockId, OLBlockId, hash::raw};

use crate::{
    block_flags::BlockFlags,
    ssz_generated::ssz::{
        block::{
            Epoch, OLBlock, OLBlockBody, OLBlockHeader, OLL1ManifestContainer, OLL1Update,
            OLTxSegment, SignedOLBlockHeader, Slot,
        },
        transaction::OLTransaction,
    },
};

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

    /// Returns the executionally-relevant block header inside the signed header
    /// structure.
    pub fn header(&self) -> &OLBlockHeader {
        &self.signed_header.header
    }

    pub fn body(&self) -> &OLBlockBody {
        &self.body
    }
}

impl SignedOLBlockHeader {
    pub fn new(header: OLBlockHeader, signature: Buf64) -> Self {
        Self { header, signature }
    }

    pub fn header(&self) -> &OLBlockHeader {
        &self.header
    }

    /// This MUST be a schnorr signature over the `Codec`-encoded `header`.
    ///
    /// This is not currently checked anywhere.
    pub fn signature(&self) -> &Buf64 {
        &self.signature
    }
}

impl OLBlockHeader {
    #[expect(clippy::too_many_arguments, reason = "headers are complicated")]
    pub fn new(
        timestamp: u64,
        flags: BlockFlags,
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

    pub fn flags(&self) -> BlockFlags {
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

    /// Computes the block ID by hashing the header's Codec encoding.
    pub fn compute_blkid(&self) -> OLBlockId {
        let encoded = encode_to_vec(self).expect("block: header encoding should succeed");
        let hash = raw(&encoded);
        OLBlockId::from(hash)
    }
}

impl Codec for OLBlockHeader {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.timestamp.encode(enc)?;
        // flags is stored as u16 internally in the generated struct
        self.flags.encode(enc)?;
        self.slot.encode(enc)?;
        self.epoch.encode(enc)?;
        self.parent_blkid.encode(enc)?;
        self.body_root.encode(enc)?;
        self.state_root.encode(enc)?;
        self.logs_root.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let timestamp = u64::decode(dec)?;
        let flags = BlockFlags::decode(dec)?;
        let slot = Slot::decode(dec)?;
        let epoch = Epoch::decode(dec)?;
        let parent_blkid = OLBlockId::decode(dec)?;
        let body_root = Buf32::decode(dec)?;
        let state_root = Buf32::decode(dec)?;
        let logs_root = Buf32::decode(dec)?;
        Ok(Self {
            timestamp,
            flags, // Convert BlockFlags to u16 for storage
            slot,
            epoch,
            parent_blkid,
            body_root,
            state_root,
            logs_root,
        })
    }
}

impl OLBlockBody {
    pub fn new(tx_segment: OLTxSegment, l1_update: Option<OLL1Update>) -> Self {
        Self {
            tx_segment,
            l1_update,
        }
    }

    /// Constructs a new instance for a common block with just a tx segment.
    pub fn new_common(tx_segment: OLTxSegment) -> Self {
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
        let encoded = encode_to_vec(self).expect("block: block body encoding should succeed");
        raw(&encoded)
    }

    /// Checks if the body looks like an epoch terminal.  Ie. if the L1 update
    /// is present.  This has to match the `IS_TERMINAL` flag in the header.
    pub fn is_body_terminal(&self) -> bool {
        self.l1_update().is_some()
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

impl OLTxSegment {
    pub fn new(txs: Vec<OLTransaction>) -> Self {
        Self {
            txs: VariableList::new(txs).expect("block: too many txs"),
        }
    }

    pub fn txs(&self) -> &[OLTransaction] {
        &self.txs
    }
}

impl Codec for OLTxSegment {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        // Convert SSZ VariableList to VarVec for Codec encoding
        let txs_vec: Vec<OLTransaction> = self.txs.iter().cloned().collect();
        let varvec = VarVec::from_vec(txs_vec).ok_or(CodecError::OverflowContainer)?;
        varvec.encode(enc)
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        // Decode VarVec and convert to SSZ VariableList
        let varvec: VarVec<OLTransaction> = VarVec::decode(dec)?;
        let txs_vec: Vec<OLTransaction> = varvec.into_inner();
        let txs = VariableList::new(txs_vec).map_err(|_| CodecError::OverflowContainer)?;
        Ok(Self { txs })
    }
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

impl Codec for OLL1Update {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        self.preseal_state_root.encode(enc)?;
        self.manifest_cont.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let preseal_state_root = Buf32::decode(dec)?;
        let manifest_cont = OLL1ManifestContainer::decode(dec)?;
        Ok(Self {
            preseal_state_root,
            manifest_cont,
        })
    }
}

impl OLL1ManifestContainer {
    pub fn new(manifests: Vec<AsmManifest>) -> Self {
        Self {
            manifests: manifests.into(),
        }
    }

    pub fn manifests(&self) -> &[AsmManifest] {
        &self.manifests
    }
}

// TODO rewrite this Codec impl by giving AsmManifest a Codec impl and converting to using VarVec
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
        Ok(Self {
            manifests: manifests.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use ssz::{Decode, Encode};
    use strata_acct_types::AccountId;
    use strata_identifiers::{Buf32, Buf64, OLBlockId};
    use strata_snark_acct_types::{
        LedgerRefProofs, LedgerRefs, ProofState, UpdateAccumulatorProofs, UpdateInputData,
        UpdateOperationData, UpdateOutputs, UpdateStateData,
    };
    use strata_test_utils_ssz::ssz_proptest;

    use crate::{
        block_flags::BlockFlags,
        ssz_generated::ssz::{
            block::{
                Epoch, OLBlock, OLBlockBody, OLBlockHeader, OLL1ManifestContainer, OLL1Update,
                OLTxSegment, SignedOLBlockHeader, Slot,
            },
            transaction::{
                GamTxPayload, OLTransaction, SnarkAccountUpdateTxPayload, TransactionAttachment,
                TransactionPayload,
            },
        },
    };

    fn buf32_strategy() -> impl Strategy<Value = Buf32> {
        any::<[u8; 32]>().prop_map(Buf32::from)
    }

    fn buf64_strategy() -> impl Strategy<Value = Buf64> {
        any::<[u8; 64]>().prop_map(Buf64::from)
    }

    fn ol_block_id_strategy() -> impl Strategy<Value = OLBlockId> {
        buf32_strategy().prop_map(OLBlockId::from)
    }

    fn transaction_attachment_strategy() -> impl Strategy<Value = TransactionAttachment> {
        (any::<Option<u64>>(), any::<Option<u64>>())
            .prop_map(|(min_slot, max_slot)| TransactionAttachment { min_slot, max_slot })
    }

    fn gam_tx_payload_strategy() -> impl Strategy<Value = GamTxPayload> {
        (
            any::<[u8; 32]>(),
            prop::collection::vec(any::<u8>(), 0..256),
        )
            .prop_map(|(target_bytes, payload)| GamTxPayload {
                target: AccountId::from(target_bytes),
                payload: payload.into(),
            })
    }

    fn snark_account_update_tx_payload_strategy()
    -> impl Strategy<Value = SnarkAccountUpdateTxPayload> {
        (any::<[u8; 32]>(), any::<[u8; 32]>(), any::<u64>()).prop_map(
            |(target_bytes, state_bytes, seq_no)| SnarkAccountUpdateTxPayload {
                target: AccountId::from(target_bytes),
                update_container: strata_snark_acct_types::SnarkAccountUpdateContainer {
                    base_update: strata_snark_acct_types::SnarkAccountUpdate {
                        operation: UpdateOperationData {
                            input: UpdateInputData {
                                seq_no,
                                messages: vec![].into(),
                                update_state: UpdateStateData {
                                    proof_state: ProofState {
                                        inner_state: state_bytes.into(),
                                        next_inbox_msg_idx: 0,
                                    },
                                    extra_data: vec![].into(),
                                },
                            },
                            ledger_refs: LedgerRefs {
                                l1_header_refs: vec![].into(),
                            },
                            outputs: UpdateOutputs {
                                transfers: vec![].into(),
                                messages: vec![].into(),
                            },
                        },
                        update_proof: vec![].into(),
                    },
                    accumulator_proofs: UpdateAccumulatorProofs::new(
                        vec![],
                        LedgerRefProofs::new(vec![]),
                    ),
                },
            },
        )
    }

    fn transaction_payload_strategy() -> impl Strategy<Value = TransactionPayload> {
        prop_oneof![
            gam_tx_payload_strategy().prop_map(TransactionPayload::GenericAccountMessage),
            snark_account_update_tx_payload_strategy()
                .prop_map(TransactionPayload::SnarkAccountUpdate),
        ]
    }

    fn ol_transaction_strategy() -> impl Strategy<Value = OLTransaction> {
        (
            transaction_payload_strategy(),
            transaction_attachment_strategy(),
        )
            .prop_map(|(payload, attachment)| OLTransaction {
                payload,
                attachment,
            })
    }

    fn ol_tx_segment_strategy() -> impl Strategy<Value = OLTxSegment> {
        prop::collection::vec(ol_transaction_strategy(), 0..10)
            .prop_map(|txs| OLTxSegment { txs: txs.into() })
    }

    fn l1_update_strategy() -> impl Strategy<Value = Option<OLL1Update>> {
        prop::option::of(buf32_strategy().prop_map(|preseal_state_root| OLL1Update {
            preseal_state_root,
            manifest_cont: OLL1ManifestContainer::new(vec![]),
        }))
    }

    fn ol_block_header_strategy() -> impl Strategy<Value = OLBlockHeader> {
        (
            any::<u64>(),
            any::<u16>().prop_map(BlockFlags::from),
            any::<Slot>(),
            any::<Epoch>(),
            ol_block_id_strategy(),
            buf32_strategy(),
            buf32_strategy(),
            buf32_strategy(),
        )
            .prop_map(
                |(
                    timestamp,
                    flags,
                    slot,
                    epoch,
                    parent_blkid,
                    body_root,
                    state_root,
                    logs_root,
                )| {
                    OLBlockHeader {
                        timestamp,
                        flags,
                        slot,
                        epoch,
                        parent_blkid,
                        body_root,
                        state_root,
                        logs_root,
                    }
                },
            )
    }

    fn signed_ol_block_header_strategy() -> impl Strategy<Value = SignedOLBlockHeader> {
        (ol_block_header_strategy(), buf64_strategy())
            .prop_map(|(header, signature)| SignedOLBlockHeader { header, signature })
    }

    fn ol_block_body_strategy() -> impl Strategy<Value = OLBlockBody> {
        (ol_tx_segment_strategy(), l1_update_strategy()).prop_map(|(tx_segment, l1_update)| {
            OLBlockBody {
                tx_segment,
                l1_update,
            }
        })
    }

    fn ol_block_strategy() -> impl Strategy<Value = OLBlock> {
        (signed_ol_block_header_strategy(), ol_block_body_strategy()).prop_map(
            |(signed_header, body)| OLBlock {
                signed_header,
                body,
            },
        )
    }

    mod ol_tx_segment {
        use super::*;

        ssz_proptest!(OLTxSegment, ol_tx_segment_strategy());

        #[test]
        fn test_empty_segment() {
            let segment = OLTxSegment { txs: vec![].into() };
            let encoded = segment.as_ssz_bytes();
            let decoded = OLTxSegment::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(segment, decoded);
        }
    }

    mod l1_update {
        use super::*;

        fn l1_update_non_option_strategy() -> impl Strategy<Value = OLL1Update> {
            buf32_strategy().prop_map(|preseal_state_root| OLL1Update {
                preseal_state_root,
                manifest_cont: OLL1ManifestContainer::new(vec![]),
            })
        }

        ssz_proptest!(OLL1Update, l1_update_non_option_strategy());

        #[test]
        fn test_zero_height() {
            let update = OLL1Update {
                preseal_state_root: Buf32::zero(),
                manifest_cont: OLL1ManifestContainer::new(vec![]),
            };
            let encoded = update.as_ssz_bytes();
            let decoded = OLL1Update::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(update, decoded);
        }
    }

    mod ol_block_header {
        use super::*;

        ssz_proptest!(OLBlockHeader, ol_block_header_strategy());

        #[test]
        fn test_genesis_header() {
            let header = OLBlockHeader {
                timestamp: 0,
                flags: BlockFlags::from(0),
                slot: 0,
                epoch: 0,
                parent_blkid: OLBlockId::from(Buf32::zero()),
                body_root: Buf32::zero(),
                state_root: Buf32::zero(),
                logs_root: Buf32::zero(),
            };
            let encoded = header.as_ssz_bytes();
            let decoded = OLBlockHeader::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(header, decoded);
        }
    }

    mod signed_ol_block_header {
        use super::*;

        ssz_proptest!(SignedOLBlockHeader, signed_ol_block_header_strategy());
    }

    mod ol_block_body {
        use super::*;

        ssz_proptest!(OLBlockBody, ol_block_body_strategy());

        #[test]
        fn test_empty_body() {
            let body = OLBlockBody {
                tx_segment: OLTxSegment { txs: vec![].into() },
                l1_update: Some(OLL1Update {
                    preseal_state_root: Buf32::zero(),
                    manifest_cont: OLL1ManifestContainer::new(vec![]),
                }),
            };
            let encoded = body.as_ssz_bytes();
            let decoded = OLBlockBody::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(body, decoded);
        }
    }

    mod ol_block {
        use super::*;

        ssz_proptest!(OLBlock, ol_block_strategy());

        #[test]
        fn test_minimal_block() {
            let block = OLBlock {
                signed_header: SignedOLBlockHeader {
                    header: OLBlockHeader {
                        timestamp: 0,
                        flags: BlockFlags::from(0),
                        slot: 0,
                        epoch: 0,
                        parent_blkid: OLBlockId::from(Buf32::zero()),
                        body_root: Buf32::zero(),
                        state_root: Buf32::zero(),
                        logs_root: Buf32::zero(),
                    },
                    signature: Buf64::zero(),
                },
                body: OLBlockBody {
                    tx_segment: OLTxSegment { txs: vec![].into() },
                    l1_update: Some(OLL1Update {
                        preseal_state_root: Buf32::zero(),
                        manifest_cont: OLL1ManifestContainer::new(vec![]),
                    }),
                },
            };
            let encoded = block.as_ssz_bytes();
            let decoded = OLBlock::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(block, decoded);
        }
    }
}
