use alpen_vertex_primitives::buf::Buf32;
use alpen_vertex_primitives::l1::TxnWithStatus;

use crate::define_table_with_default_codec;
use crate::define_table_without_codec;
use crate::impl_borsh_value_codec;

define_table_with_default_codec!(
    /// A table to store L1 txns
    (SeqL1TxnSchema) Buf32 => TxnWithStatus
);

define_table_with_default_codec!(
    /// A table to store mapping of idx to L1 tx
    (SequencerL1TxIdSchema) u64 => Buf32
);

define_table_with_default_codec!(
    /// A table to store idx-> blobid mapping
    (SequencerBlobIdSchema) u64 => Buf32
);

define_table_with_default_codec!(
    /// A table to store blobid -> blob mapping
    (SequencerBlobSchema) Buf32 => Vec<u8>
);
