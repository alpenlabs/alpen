use strata_db_types::mmr_helpers::MmrMetadata;
use strata_identifiers::AccountId;
use strata_primitives::buf::Buf32;

use crate::{define_table_with_seek_key_codec, define_table_without_codec, impl_borsh_value_codec};

define_table_with_seek_key_codec!(
    /// MMR node storage: (account_id, position) -> hash
    (SnarkMsgMmrNodeSchema) (AccountId, u64) => Buf32
);

define_table_with_seek_key_codec!(
    /// MMR metadata schema: account_id -> metadata
    (SnarkMsgMmrMetaSchema) AccountId => MmrMetadata
);
