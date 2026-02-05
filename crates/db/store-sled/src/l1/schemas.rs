use strata_asm_common::AsmManifest;
use strata_primitives::l1::L1BlockId;

use crate::{
    define_table_with_integer_key, define_table_without_codec, impl_ssz_key_codec,
    impl_ssz_value_codec,
};

define_table_without_codec!(
    /// A table to store L1 Block data (as ASM Manifest). Maps block id to manifest
    (L1BlockSchema) L1BlockId => AsmManifest
);

impl_ssz_key_codec!(L1BlockSchema, L1BlockId);
impl_ssz_value_codec!(L1BlockSchema, AsmManifest);

define_table_with_integer_key!(
    /// A table to store canonical view of L1 chain
    (L1CanonicalBlockSchema) u64 => L1BlockId
);

define_table_with_integer_key!(
    /// A table to keep track of all added blocks
    (L1BlocksByHeightSchema) u64 => Vec<L1BlockId>
);
