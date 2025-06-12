use rockbound::{Schema, TransactionCtx, TransactionDBMarker};

use crate::{define_table_with_default_codec, define_table_without_codec, impl_borsh_value_codec};

define_table_with_default_codec!(
    /// A table to hold sequence numbers
    (SequenceSchema) Vec<u8> => u64
);

/// Get next incremental id for given `Schema`.
/// This should only be used to get atomically auto-incrementing keys for tables inside
/// transactions.
/// Should NEVER be updated directly outside of this method.
/// Id starts from 0, increments by 1
pub(crate) fn get_next_id<S: Schema, DB: TransactionDBMarker>(
    txn: &TransactionCtx<'_, DB>,
) -> anyhow::Result<u64> {
    get_next_id_opts::<S, DB>(txn, |last_idx| last_idx + 1, 0)
}

/// Get next incremental id for given `Schema`.
/// This should only be used to get atomically auto-incrementing keys for tables inside
/// transactions.
/// Should NEVER be updated directly outside of this method.
/// index starts from `starting_index` and updated by `update(last_idx)`
pub(crate) fn get_next_id_opts<S: Schema, DB: TransactionDBMarker>(
    txn: &TransactionCtx<'_, DB>,
    update: impl Fn(u64) -> u64,
    starting_index: u64,
) -> anyhow::Result<u64> {
    let index_key = S::COLUMN_FAMILY_NAME.as_bytes().to_vec();
    let next_idx = txn
        .get_for_update::<SequenceSchema>(&index_key)?
        .map(update)
        .unwrap_or(starting_index);
    txn.put::<SequenceSchema>(&index_key, &next_idx)?;
    Ok(next_idx)
}
