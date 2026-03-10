use strata_asm_common::AsmManifest;
use strata_primitives::{L1Height, l1::L1BlockId};

use crate::{define_table_with_default_codec, define_table_with_integer_key};

define_table_with_default_codec!(
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
