//! Batch diff aggregation with a host-side correction for EIP-7702 code clears.
//!
//! The pinned `alpen-reth-statediff` drops any `code_hash` change whose new
//! value is `KECCAK256_EMPTY`, on the stale assumption that code only changes on
//! contract creation. EIP-7702 breaks that: clearing a delegation moves an
//! existing account's `code_hash` from a designator back to empty. Without the
//! write, DA reconstruction keeps the stale designator and the acct proof fails
//! with `PostApplyStateRootMismatch`.
//!
//! The correction lives here rather than in `alpen-reth-statediff` because that
//! crate is compiled into the acct proof guest, so touching it changes the guest
//! ELF and its VK. The guest apply path already honours a `code_hash` write of
//! `KECCAK256_EMPTY`; only blob production needs fixing.

use std::collections::BTreeMap;

use alloy_primitives::{Address, B256, KECCAK256_EMPTY};
use alpen_reth_statediff::{
    AccountChange, AccountDiff, BatchBuilder, BatchStateDiff, BlockStateChanges,
};
use strata_da_framework::DaRegister;

/// Code hash of an account before the batch and after the latest applied block.
///
/// `None` means the account did not exist at that point.
#[derive(Clone, Debug)]
struct TrackedCodeHash {
    original: Option<B256>,
    current: Option<B256>,
}

impl TrackedCodeHash {
    /// Returns `true` if an existing account's code was cleared to empty across
    /// the batch, the one case the underlying builder fails to record.
    fn is_cleared_to_empty(&self) -> bool {
        matches!(
            (self.original, self.current),
            (Some(orig), Some(KECCAK256_EMPTY)) if orig != KECCAK256_EMPTY
        )
    }
}

/// [`BatchBuilder`] wrapper that re-records code clears the builder drops.
///
/// Tracks each account's batch-original and latest `code_hash` with the same
/// first-seen/last-seen semantics as [`BatchBuilder`], so a delegation set and
/// cleared within one batch still yields no write.
#[derive(Debug, Default)]
pub(crate) struct CodeClearAwareBatchBuilder {
    inner: BatchBuilder,
    code_hashes: BTreeMap<Address, TrackedCodeHash>,
}

impl CodeClearAwareBatchBuilder {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn apply_block(&mut self, block_diff: &BlockStateChanges) {
        for (addr, change) in &block_diff.accounts {
            let entry = self
                .code_hashes
                .entry(*addr)
                .or_insert_with(|| TrackedCodeHash {
                    original: change.original.as_ref().map(|s| s.code_hash),
                    current: None,
                });
            entry.current = change.current.as_ref().map(|s| s.code_hash);
        }

        self.inner.apply_block(block_diff);
    }

    pub(crate) fn build(self) -> BatchStateDiff {
        let mut diff = self.inner.build();

        for (addr, tracked) in self.code_hashes {
            if !tracked.is_cleared_to_empty() {
                continue;
            }

            let cleared = DaRegister::new_set(KECCAK256_EMPTY.into());
            match diff.accounts.get_mut(&addr) {
                Some(AccountChange::Updated(account_diff)) => {
                    account_diff.code_hash = cleared;
                }
                // The builder drops an account whose only change was the code
                // clear; reinstate it with just the code_hash write.
                None => {
                    let account_diff = AccountDiff {
                        code_hash: cleared,
                        ..AccountDiff::new_unchanged()
                    };
                    diff.accounts
                        .insert(addr, AccountChange::Updated(account_diff));
                }
                // Created implies no batch-original; Deleted implies no current.
                // Neither can coincide with a tracked clear.
                Some(AccountChange::Created(_) | AccountChange::Deleted) => {}
            }
        }

        diff
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::U256;
    use alpen_reth_statediff::{AccountSnapshot, BlockAccountChange};
    use strata_codec::{decode_buf_exact, encode_to_vec};
    use strata_da_framework::DaWrite;

    use super::*;

    const ADDR: Address = Address::repeat_byte(0xaa);
    const DESIGNATOR: B256 = B256::repeat_byte(0x77);
    const DESIGNATOR_2: B256 = B256::repeat_byte(0x88);

    fn snapshot(nonce: u64, code_hash: B256) -> AccountSnapshot {
        AccountSnapshot {
            balance: U256::from(1000),
            nonce,
            code_hash,
        }
    }

    fn block(
        original: Option<AccountSnapshot>,
        current: Option<AccountSnapshot>,
    ) -> BlockStateChanges {
        let mut changes = BlockStateChanges::new();
        changes
            .accounts
            .insert(ADDR, BlockAccountChange { original, current });
        changes
    }

    fn build(blocks: &[BlockStateChanges]) -> BatchStateDiff {
        let mut builder = CodeClearAwareBatchBuilder::new();
        for b in blocks {
            builder.apply_block(b);
        }
        builder.build()
    }

    fn code_hash_write(diff: &BatchStateDiff) -> Option<Option<B256>> {
        match diff.accounts.get(&ADDR)? {
            AccountChange::Updated(d) | AccountChange::Created(d) => {
                Some(d.code_hash.new_value().map(|v| B256::from(*v)))
            }
            AccountChange::Deleted => None,
        }
    }

    #[test]
    fn clear_to_empty_is_recorded() {
        let diff = build(&[block(
            Some(snapshot(1, DESIGNATOR)),
            Some(snapshot(2, KECCAK256_EMPTY)),
        )]);

        assert_eq!(code_hash_write(&diff), Some(Some(KECCAK256_EMPTY)));
    }

    #[test]
    fn clear_with_no_other_change_reinstates_account() {
        // The underlying builder drops this account entirely; the write must
        // still reach the blob.
        let diff = build(&[block(
            Some(snapshot(1, DESIGNATOR)),
            Some(snapshot(1, KECCAK256_EMPTY)),
        )]);

        let AccountChange::Updated(d) = diff.accounts.get(&ADDR).expect("account present") else {
            panic!("expected Updated");
        };
        assert_eq!(
            d.code_hash.new_value().map(|v| B256::from(*v)),
            Some(KECCAK256_EMPTY)
        );
        assert!(d.balance.is_default());
        assert!(d.nonce.is_default());
    }

    #[test]
    fn set_then_clear_within_batch_yields_no_write() {
        let diff = build(&[
            block(
                Some(snapshot(1, KECCAK256_EMPTY)),
                Some(snapshot(2, DESIGNATOR)),
            ),
            block(
                Some(snapshot(2, DESIGNATOR)),
                Some(snapshot(3, KECCAK256_EMPTY)),
            ),
        ]);

        // Nonce moved so the account is present, but code is back to original.
        assert_eq!(code_hash_write(&diff), Some(None));
    }

    #[test]
    fn clear_across_blocks_is_recorded() {
        let diff = build(&[
            block(
                Some(snapshot(1, DESIGNATOR)),
                Some(snapshot(1, DESIGNATOR_2)),
            ),
            block(
                Some(snapshot(1, DESIGNATOR_2)),
                Some(snapshot(2, KECCAK256_EMPTY)),
            ),
        ]);

        assert_eq!(code_hash_write(&diff), Some(Some(KECCAK256_EMPTY)));
    }

    #[test]
    fn created_empty_account_stays_unset() {
        let diff = build(&[block(None, Some(snapshot(0, KECCAK256_EMPTY)))]);

        assert!(matches!(
            diff.accounts.get(&ADDR),
            Some(AccountChange::Created(_))
        ));
        assert_eq!(code_hash_write(&diff), Some(None));
    }

    #[test]
    fn redelegation_still_recorded() {
        let diff = build(&[block(
            Some(snapshot(1, DESIGNATOR)),
            Some(snapshot(2, DESIGNATOR_2)),
        )]);

        assert_eq!(code_hash_write(&diff), Some(Some(DESIGNATOR_2)));
    }

    #[test]
    fn clear_survives_codec_roundtrip() {
        let diff = build(&[block(
            Some(snapshot(1, DESIGNATOR)),
            Some(snapshot(2, KECCAK256_EMPTY)),
        )]);

        let encoded = encode_to_vec(&diff).unwrap();
        let decoded: BatchStateDiff = decode_buf_exact(&encoded).unwrap();

        assert_eq!(code_hash_write(&decoded), Some(Some(KECCAK256_EMPTY)));
    }
}
