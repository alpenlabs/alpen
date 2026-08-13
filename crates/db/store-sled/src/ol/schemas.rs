use strata_db_types::ol_block::BlockStatus;
use strata_identifiers::{EpochCommitment, OLBlockCommitment, OLBlockId};
use strata_ol_chain_types_v1::{OLBlockHeaderV1, OLBlockV1};

use crate::{
    define_table_without_codec, impl_cbor_value_codec, impl_codec_key_codec,
    impl_codec_value_codec, impl_ssz_value_codec,
};

define_table_without_codec!(
    /// A table to store OL Block data. Maps block ID to Block
    (OLBlockSchema) OLBlockId => OLBlockV1
);
impl_codec_key_codec!(OLBlockSchema, OLBlockId);
impl_ssz_value_codec!(OLBlockSchema, OLBlockV1);

define_table_without_codec!(
    /// Stores reconstructed unsigned headers for checkpoint terminal blocks.
    (OLTerminalHeaderSchema) OLBlockId => OLBlockHeaderV1
);
// Shares the full-block schema's key codec: `get_ol_header` looks the same id up
// in both trees, so the two encodings must agree.
impl_codec_key_codec!(OLTerminalHeaderSchema, OLBlockId);
impl_ssz_value_codec!(OLTerminalHeaderSchema, OLBlockHeaderV1);

define_table_without_codec!(
    /// A table to store OL Block status. Maps block ID to BlockStatus
    (OLBlockStatusSchema) OLBlockId => BlockStatus
);
impl_codec_key_codec!(OLBlockStatusSchema, OLBlockId);
impl_cbor_value_codec!(OLBlockStatusSchema, BlockStatus);

define_table_without_codec!(
    /// A table to store OL Block IDs by slot. Maps slot to Vec<OLBlockId>
    (OLBlockHeightSchema) u64 => Vec<OLBlockId>
);
impl_cbor_value_codec!(OLBlockHeightSchema, Vec<OLBlockId>);

define_table_without_codec!(
    /// A table mapping each slot to its canonical OL block id, as selected by
    /// fork choice. Maps slot to OLBlockId.
    (OLCanonicalBlockSchema) u64 => OLBlockId
);
impl_cbor_value_codec!(OLCanonicalBlockSchema, OLBlockId);

define_table_without_codec!(
    /// Stores the latest OL block committed through the high-watermark path.
    (OLBlockHighWatermarkSchema) u8 => OLBlockCommitment
);
impl_codec_value_codec!(OLBlockHighWatermarkSchema, OLBlockCommitment);

define_table_without_codec!(
    /// Stores the immutable base of locally available OL block history.
    (OLHistoryBaseSchema) u8 => EpochCommitment
);
impl_codec_value_codec!(OLHistoryBaseSchema, EpochCommitment);
