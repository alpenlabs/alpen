use strata_db_types::types::MempoolTxMetadata;
use strata_identifiers::OLTxId;
use strata_ol_chain_types_new::OLTransaction;

use crate::{define_table_with_default_codec, define_table_without_codec, impl_borsh_value_codec};

define_table_with_default_codec!(
    /// A table to store mempool transactions
    (MempoolTxSchema) OLTxId => (OLTransaction, MempoolTxMetadata)
);
