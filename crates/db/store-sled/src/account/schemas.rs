//! Schema definitions for the account genesis database.

use strata_identifiers::AccountId;
use strata_primitives::OLBlockId;

use crate::{define_table_with_default_codec, define_table_without_codec, impl_borsh_value_codec};

define_table_with_default_codec!(
    /// Maps [`AccountId`] to its creation epoch (`u32`).
    (AccountGenesisSchema) AccountId => u32
);

define_table_with_default_codec!(
    /// Maps [`(AccountId, OLBlockId)`] tuple to extra data bytes.
    /// Stores additional account data associated with specific OL blocks.
    (AccountExtraDataSchema) (AccountId, OLBlockId) => Vec<u8>
);
