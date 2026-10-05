use strata_asm_common::AsmManifest;
use strata_primitives::L1Height;
use strata_primitives::l1::L1BlockId;

use crate::asm::codec::{decode_framed, encode_framed, impl_asm_value_codec};
use crate::{define_table_with_integer_key, define_table_without_codec, impl_borsh_key_codec};

define_table_without_codec!(
    /// A table to store L1 Block data (as ASM Manifest). Maps block id to manifest
    (L1BlockSchema) L1BlockId => AsmManifest
);

define_table_with_integer_key!(
    /// A table to store canonical view of L1 chain
    (L1CanonicalBlockSchema) L1Height => L1BlockId
);

define_table_with_integer_key!(
    /// A table to keep track of all added blocks
    (L1BlocksByHeightSchema) L1Height => Vec<L1BlockId>
);

impl_borsh_key_codec!(L1BlockSchema, L1BlockId);
impl_asm_value_codec!(L1BlockSchema, AsmManifest, encode_framed, decode_framed);
