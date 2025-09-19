use strata_primitives::l1::L1BlockCommitment;
use strata_state::asm_state::AsmState;

use crate::{define_table_with_seek_key_codec, define_table_without_codec, impl_borsh_value_codec};

// ASM state per block schema and corresponding codecs implementation.
define_table_with_seek_key_codec!(
    /// A table to store ASM state per l1 block.
    (AsmBlockSchema) L1BlockCommitment => AsmState
);
