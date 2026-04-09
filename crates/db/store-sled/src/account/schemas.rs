//! Schema definitions for the account genesis database.

use strata_db_types::types::AccountExtraData;
use strata_identifiers::{AccountId, Epoch};
use strata_primitives::nonempty_vec::NonEmptyVec;

use crate::define_table_with_default_codec;

define_table_with_default_codec!(
    /// Maps [`AccountId`] to its creation epoch (`Epoch`).
    (AccountGenesisSchema) AccountId => Epoch
);

define_table_with_default_codec!(
    /// Maps [`(AccountId, Epoch)`] tuple to extra data bytes.
    /// Stores additional account data associated with specific OL blocks.
    (AccountExtraDataSchema) (AccountId, Epoch) => NonEmptyVec<AccountExtraData>
);
