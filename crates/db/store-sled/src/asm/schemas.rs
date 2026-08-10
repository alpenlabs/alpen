use strata_asm_common::{AnchorState, AsmLogEntry, AuxData};
use strata_primitives::l1::L1BlockCommitment;

use crate::{define_table_without_codec, impl_bincode_key_codec, impl_ssz_value_codec};

define_table_without_codec!(
    /// A table to store ASM state per l1 block.
    (AsmStateSchema) L1BlockCommitment => AnchorState
);
impl_bincode_key_codec!(AsmStateSchema, L1BlockCommitment);
impl_ssz_value_codec!(AsmStateSchema, AnchorState);

define_table_without_codec!(
    /// A table to store ASM logs per l1 block.
    (AsmLogSchema) L1BlockCommitment => Vec<AsmLogEntry>
);
impl_bincode_key_codec!(AsmLogSchema, L1BlockCommitment);
impl_ssz_value_codec!(AsmLogSchema, Vec<AsmLogEntry>);

define_table_without_codec!(
    /// A table to store ASM auxiliary data per l1 block.
    (AsmAuxDataSchema) L1BlockCommitment => AuxData
);
impl_bincode_key_codec!(AsmAuxDataSchema, L1BlockCommitment);
impl_ssz_value_codec!(AsmAuxDataSchema, AuxData);
