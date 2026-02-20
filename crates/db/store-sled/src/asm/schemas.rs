use strata_asm_common::{AnchorState, AsmLogEntry, AuxData};
use strata_primitives::l1::L1BlockCommitment;

use crate::{define_table_with_seek_key_codec, define_table_without_codec, impl_borsh_value_codec};

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

// ASM auxiliary data per block schema and corresponding codecs implementation.
define_table_with_seek_key_codec!(
    /// A table to store ASM auxiliary data per l1 block.
    (AsmAuxDataSchema) L1BlockCommitment => AuxData
);
