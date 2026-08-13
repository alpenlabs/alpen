use strata_asm_common::AsmManifest;
use strata_primitives::L1Height;
use strata_primitives::l1::L1BlockId;

use crate::{
    define_table_without_codec, impl_cbor_value_codec, impl_codec_key_codec, impl_ssz_value_codec,
};

define_table_without_codec!(
    /// A table to store L1 Block data (as ASM Manifest). Maps block id to manifest
    (L1BlockSchema) L1BlockId => AsmManifest
);
impl_codec_key_codec!(L1BlockSchema, L1BlockId);
impl_ssz_value_codec!(L1BlockSchema, AsmManifest);

define_table_without_codec!(
    /// A table to store canonical view of L1 chain
    (L1CanonicalBlockSchema) L1Height => L1BlockId
);
// `L1Height` is a `u32` alias and uses typed-sled's blanket big-endian integer key codec.
impl_cbor_value_codec!(L1CanonicalBlockSchema, L1BlockId);

define_table_without_codec!(
    /// A table to keep track of all added blocks
    (L1BlocksByHeightSchema) L1Height => Vec<L1BlockId>
);
impl_cbor_value_codec!(L1BlocksByHeightSchema, Vec<L1BlockId>);
