use strata_asm_common::{AnchorState, AsmLogEntry};
use strata_primitives::{buf::Buf32, l1::L1BlockCommitment};

use crate::{
    define_table_with_integer_key, define_table_with_seek_key_codec, define_table_without_codec,
    impl_borsh_value_codec,
};

// ASM state per block schema and corresponding codecs implementation.
define_table_with_seek_key_codec!(
    /// A table to store ASM state per l1 block.
    (AsmStateSchema) L1BlockCommitment => AnchorState
);

// ASM logs per block schema and corresponding codecs implementation.
define_table_with_seek_key_codec!(
    /// A table to store ASM logs per l1 block.
    (AsmLogSchema) L1BlockCommitment => Vec<AsmLogEntry>
);

define_table_with_integer_key!(
    /// Manifest hash storage: manifest_index -> hash. Maps leaf indices to manifest hashes.
    (AsmManifestHashSchema) u64 => Buf32
);
