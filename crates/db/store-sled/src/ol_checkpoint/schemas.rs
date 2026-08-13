use strata_asm_checkpoint_types::CheckpointPayload;
use strata_checkpoint_types::EpochSummary;
use strata_csm_types::CheckpointL1Ref;
use strata_db_types::common::L1PayloadIntentIndex;
use strata_identifiers::{Epoch, EpochCommitment};

use crate::{
    define_table_without_codec, impl_cbor_value_codec, impl_codec_key_codec, impl_ssz_value_codec,
};

// `EpochCommitment` keys encode big-endian as `epoch(4) || last_slot(8) || blkid(32)`, so
// lexicographic tree order matches epoch order and the checkpoint queries can range-seek.

define_table_without_codec!(
    /// Table mapping epoch commitment to OL checkpoint payload.
    (OLCheckpointPayloadSchema) EpochCommitment => CheckpointPayload
);
impl_codec_key_codec!(OLCheckpointPayloadSchema, EpochCommitment);
impl_ssz_value_codec!(OLCheckpointPayloadSchema, CheckpointPayload);

define_table_without_codec!(
    /// Table mapping epoch commitment to the OL checkpoint payload extracted
    /// from L1, kept separate from the sequencer's locally-built payload table.
    (OLCheckpointL1ObservedPayloadSchema) EpochCommitment => CheckpointPayload
);
impl_codec_key_codec!(OLCheckpointL1ObservedPayloadSchema, EpochCommitment);
impl_ssz_value_codec!(OLCheckpointL1ObservedPayloadSchema, CheckpointPayload);

define_table_without_codec!(
    /// Table mapping epoch to OL checkpoint payload intent index.
    (OLCheckpointSigningSchema) EpochCommitment => L1PayloadIntentIndex
);
impl_codec_key_codec!(OLCheckpointSigningSchema, EpochCommitment);
impl_cbor_value_codec!(OLCheckpointSigningSchema, L1PayloadIntentIndex);

define_table_without_codec!(
    /// Table mapping epoch commitment to persisted [`CheckpointL1Ref`].
    (OLCheckpointL1RefSchema) EpochCommitment => CheckpointL1Ref
);
impl_codec_key_codec!(OLCheckpointL1RefSchema, EpochCommitment);
impl_cbor_value_codec!(OLCheckpointL1RefSchema, CheckpointL1Ref);

define_table_without_codec!(
    /// Presence marker: this epoch has at least one unsigned payload.
    (UnsignedCheckpointIndexSchema) Epoch => ()
);
// `Epoch` is a `u32` alias and uses typed-sled's blanket big-endian integer key codec.
impl_cbor_value_codec!(UnsignedCheckpointIndexSchema, ());

define_table_without_codec!(
    /// Table mapping epoch indexes to the list of summaries in that index.
    (OLEpochSummarySchema) u64 => Vec<EpochSummary>
);
impl_cbor_value_codec!(OLEpochSummarySchema, Vec<EpochSummary>);

define_table_without_codec!(
    /// Observed checkpoint commitments per epoch number.
    ///
    /// Observed candidate set: reorged observations remain until explicit
    /// pruning, so canonicity is resolved at read time.
    (OLCheckpointEpochIndexSchema) Epoch => Vec<EpochCommitment>
);
impl_cbor_value_codec!(OLCheckpointEpochIndexSchema, Vec<EpochCommitment>);

#[cfg(test)]
mod tests {
    use strata_identifiers::{Buf32, OLBlockId};
    use typed_sled::codec::KeyCodec;

    use super::*;

    /// Byte length of an `EpochCommitment` key: `epoch(4) || last_slot(8) || blkid(32)`.
    const KEY_LEN: usize = 44;

    fn commitment(epoch: u32, last_slot: u64, tag: u8) -> EpochCommitment {
        EpochCommitment::new(epoch, last_slot, OLBlockId::from(Buf32::from([tag; 32])))
    }

    fn encode(commitment: &EpochCommitment) -> Vec<u8> {
        <EpochCommitment as KeyCodec<OLCheckpointPayloadSchema>>::encode_key(commitment)
            .expect("encode epoch commitment key")
    }

    /// Pins the on-disk key layout; drift would orphan existing checkpoint entries and
    /// silently break the range-seek queries in `db.rs`.
    #[test]
    fn key_bytes_match_on_disk_layout() {
        #[rustfmt::skip]
        let expected: Vec<u8> = [
            vec![1, 2, 3, 4],                                     // epoch (u32 BE)
            vec![0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10, 0x11], // last_slot (u64 BE)
            vec![0x77; 32],                                       // last_blkid
        ]
        .concat();

        let key = commitment(0x0102_0304, 0x0A0B_0C0D_0E0F_1011, 0x77);
        assert_eq!(encode(&key), expected);
        assert_eq!(expected.len(), KEY_LEN);
    }

    /// Lexicographic byte order must follow epoch order, including across a byte boundary --
    /// under little-endian or varint encoding epoch 256 would sort below epoch 255.
    #[test]
    fn key_bytes_sort_by_epoch_across_byte_boundary() {
        assert!(encode(&commitment(255, u64::MAX, 0xFF)) < encode(&commitment(256, 0, 0x00)));
    }

    /// Within an epoch, order falls through to `last_slot` then `last_blkid`, which is what
    /// makes `last()` return the greatest commitment of the highest epoch.
    #[test]
    fn key_bytes_sort_by_slot_then_blkid_within_an_epoch() {
        assert!(encode(&commitment(7, 1, 0xFF)) < encode(&commitment(7, 2, 0x00)));
        assert!(encode(&commitment(7, 1, 0x01)) < encode(&commitment(7, 1, 0x02)));
    }
}
