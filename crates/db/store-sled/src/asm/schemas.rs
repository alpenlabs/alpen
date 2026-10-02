use strata_asm_common::{AnchorState, AsmLogEntry, AuxData};
use strata_primitives::l1::L1BlockCommitment;

use super::codec::{
    AUX_V1, STATE_V1, decode_logs, decode_versioned, encode_logs, encode_versioned,
    impl_asm_value_codec,
};
use crate::{define_table_without_codec, impl_bincode_key_codec};

// ASM state per block schema and corresponding codecs implementation.
define_table_without_codec!(
    /// A table to store ASM state per l1 block.
    (AsmStateSchema) L1BlockCommitment => AnchorState
);

// ASM logs per block schema and corresponding codecs implementation.
define_table_without_codec!(
    /// A table to store ASM logs per l1 block.
    (AsmLogSchema) L1BlockCommitment => Vec<AsmLogEntry>
);

// ASM auxiliary data per block schema and corresponding codecs implementation.
define_table_without_codec!(
    /// A table to store ASM auxiliary data per l1 block.
    (AsmAuxDataSchema) L1BlockCommitment => AuxData
);

impl_bincode_key_codec!(AsmStateSchema, L1BlockCommitment);
impl_bincode_key_codec!(AsmLogSchema, L1BlockCommitment);
impl_bincode_key_codec!(AsmAuxDataSchema, L1BlockCommitment);

impl_asm_value_codec!(
    AsmStateSchema,
    AnchorState,
    |value| encode_versioned(value, STATE_V1),
    |bytes| decode_versioned(bytes, STATE_V1)
);
impl_asm_value_codec!(
    AsmAuxDataSchema,
    AuxData,
    |value| encode_versioned(value, AUX_V1),
    |bytes| decode_versioned(bytes, AUX_V1)
);
impl_asm_value_codec!(
    AsmLogSchema,
    Vec<AsmLogEntry>,
    |logs: &Vec<AsmLogEntry>| encode_logs(logs),
    decode_logs
);
