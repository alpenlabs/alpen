//! Schema definitions for the account genesis database.

use strata_db_types::types::AccountExtraDataEntry;
use strata_identifiers::{AccountId, Epoch};
use strata_primitives::nonempty_vec::NonEmptyVec;

use crate::{define_table_with_default_codec, define_table_without_codec, impl_borsh_value_codec};

define_table_with_default_codec!(
    /// Maps [`AccountId`] to its creation epoch (`u32`).
    (AccountGenesisSchema) AccountId => u32
);

define_table_with_default_codec!(
    /// Maps [`(AccountId, Epoch)`] tuple to extra data bytes.
    /// Stores additional account data associated with specific OL blocks.
    (AccountExtraDataSchema) (AccountId, Epoch) => NonEmptyVec<AccountExtraDataEntry>
);
