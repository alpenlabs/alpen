//! Per-transaction data-availability (DA) fee sizing.
//!
//! The DA fee charges a transaction for the Bitcoin data-availability cost of its
//! state diff: `da_fee = da_rate_wei_per_byte * diff_size`. This module owns the
//! single, deterministic routine that computes `diff_size` for one transaction from
//! the EVM state change-set.
//!
//! Determinism is the core requirement: the sequencer, a re-executing full node, and
//! the chunk proof must all compute the identical `diff_size` (and therefore the
//! identical post-charge balances / state root). [`calc_diff_size`] is a pure,
//! order-independent integer sum over the change-set using only compile-time constants,
//! so it reproduces exactly across all three.
//!
//! This routine is intended to be the single source of truth for DA sizing — the
//! in-EVM charge and the fee-estimation RPCs must both call it, so a quote can never
//! disagree with the charge.

use reth_evm::{eth::EthEvmContext, Database};
use revm::state::EvmState;
use revm_primitives::{Bytes, KECCAK_EMPTY, U256};

/// Basis-points denominator for the ratio/discount constants below.
const BPS_DENOM: u64 = 10_000;

/// Byte cost attributed to the key (address) of a changed account.
const ACCOUNT_KEY_BYTES: u64 = 20;

/// Byte cost attributed to the account-info change of a changed EOA.
///
/// Alpen's DA encoding stores account info as trimmed deltas (balance delta, nonce
/// varint, unset code-hash), so an EOA change is small. Conservative typical estimate.
const ACCOUNT_INFO_EOA_BYTES: u64 = 12;

/// Byte cost attributed to the account-info change of a changed contract account.
///
/// Adds the 33-byte code-hash register that a contract carries over an EOA. Deployed
/// bytecode itself is deduplicated by hash at the batch level and is not attributed
/// per transaction here (TODO: attribute deploy bytecode once per unique deployment).
const ACCOUNT_INFO_CONTRACT_BYTES: u64 = 44;

/// Byte cost attributed to the key of a changed storage slot (untrimmed 32-byte hash).
const SLOT_KEY_BYTES: u64 = 32;

/// Byte cost attributed to the value of a changed storage slot (trimmed; typical).
const SLOT_VALUE_BYTES: u64 = 8;

/// Discount applied to account-info contributions (basis points).
///
/// Per-transaction sizing over-counts relative to the merged batch diff (several txs
/// touching the same account each "create" its diff, but the batch stores it once).
/// The discount makes the sum of per-tx charges approximate the realized batch DA.
const ACCOUNT_DISCOUNT_BPS: u64 = 3_200;

/// Discount applied to storage-slot contributions (basis points). See
/// [`ACCOUNT_DISCOUNT_BPS`].
const STORAGE_DISCOUNT_BPS: u64 = 6_600;

/// Estimated compressed-to-uncompressed ratio (basis points).
///
/// Real DA is compressed over the whole batch (non-linear, not per-tx attributable), so
/// an empirically-measured average ratio is applied as a flat scalar rather than
/// compressing per transaction.
const COMPRESSION_RATIO_BPS: u64 = 4_800;

/// Flat per-transaction additive for DA bytes outside the account/storage diff
/// (block metadata and other state written to DA).
const DA_FIXED_OVERHEAD: u64 = 2;

/// Computes the DA `diff_size` (in bytes) for a single transaction's state change-set.
///
/// `state` is the post-execution [`EvmState`] for the transaction. Every touched
/// account and every changed storage slot contributes a fixed byte cost; the account
/// and storage totals are discounted (to approximate batch-level dedup), summed,
/// scaled by the compression ratio, and offset by a fixed overhead.
///
/// The result is deterministic and independent of map iteration order.
pub fn calc_diff_size(state: &EvmState) -> u64 {
    let mut account_raw: u64 = 0;
    let mut storage_raw: u64 = 0;

    for account in state.values() {
        // Only accounts that were actually touched enter the state diff.
        if !account.is_touched() {
            continue;
        }

        let account_info_bytes = if account.info.code_hash != KECCAK_EMPTY {
            ACCOUNT_INFO_CONTRACT_BYTES
        } else {
            ACCOUNT_INFO_EOA_BYTES
        };
        account_raw = account_raw.saturating_add(ACCOUNT_KEY_BYTES + account_info_bytes);

        for slot in account.storage.values() {
            if slot.is_changed() {
                storage_raw = storage_raw.saturating_add(SLOT_KEY_BYTES + SLOT_VALUE_BYTES);
            }
        }
    }

    let account_discounted = account_raw.saturating_mul(ACCOUNT_DISCOUNT_BPS) / BPS_DENOM;
    let storage_discounted = storage_raw.saturating_mul(STORAGE_DISCOUNT_BPS) / BPS_DENOM;
    let uncompressed = account_discounted.saturating_add(storage_discounted);

    uncompressed.saturating_mul(COMPRESSION_RATIO_BPS) / BPS_DENOM + DA_FIXED_OVERHEAD
}

/// Decodes the per-block DA rate (wei per byte) from the EVM header `extra_data`.
///
/// The rate is stored as a big-endian `u64` in the first 8 bytes of `extra_data`.
/// Anything shorter (e.g. the genesis label, or an empty field) decodes to `0`, which
/// disables the DA charge — so the charge is dormant until a rate is committed.
pub fn da_rate_from_extra_data(extra_data: &Bytes) -> u64 {
    if extra_data.len() < 8 {
        return 0;
    }
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&extra_data[..8]);
    u64::from_be_bytes(buf)
}

/// Encodes a per-block DA rate (wei per byte) into big-endian `extra_data` bytes.
pub fn da_rate_to_extra_data(da_rate: u64) -> Bytes {
    Bytes::copy_from_slice(&da_rate.to_be_bytes())
}

/// Wei per satoshi: the L2 gas token is BTC with 18 decimals, and 1 BTC = 10^8 sat,
/// so 1 sat = 10^10 wei.
const WEI_PER_SAT: u64 = 10_000_000_000;

/// SegWit witness discount: DA payload rides in witness data, weighted at 1/4 of a vByte.
const SEGWIT_WITNESS_DIVISOR: u64 = 4;

/// Converts a Bitcoin fee rate (satoshis per virtual byte) to the DA rate (wei per byte).
///
/// `da_rate = btc_fee_rate[sat/vB] * 10^10[wei/sat] / 4` (the SegWit witness discount).
///
/// NOTE: for now this reuses the sequencer's Bitcoin publication fee rate
/// (`btcio::writer::fees::resolve_fee_rate`). The DA fee-model rate is expected to be
/// decoupled from the publication rate — and smoothed/cached — in a later revision.
pub fn btc_fee_rate_to_da_rate(sat_per_vbyte: u64) -> u64 {
    sat_per_vbyte
        .saturating_mul(WEI_PER_SAT)
        .saturating_div(SEGWIT_WITNESS_DIVISOR)
}

/// Computes the DA fee to charge, bounded by the caller's unused authorized gas value.
///
/// The raw fee is `da_rate * diff_size`, but it is capped at `remaining_value` — the
/// value of the gas the caller authorized (prepaid) but did not consume. Capping there
/// guarantees the charge never exceeds what the signature authorized and never fails
/// (the caller was just refunded `remaining_value`). If the raw fee exceeds the budget
/// (rate/diff drift versus the quote), the protocol undercharges rather than overcharge.
pub fn bounded_da_fee(da_rate: U256, diff_size: u64, remaining_value: U256) -> U256 {
    da_rate
        .saturating_mul(U256::from(diff_size))
        .min(remaining_value)
}

/// Read access to the raw EVM state change-set on the concrete EVM context.
///
/// The generic revm `Handler` cannot reach the whole `EvmState` through `JournalTr`, but
/// the concrete [`EthEvmContext`] exposes it via its journal. The DA charge binds to this
/// trait so it can size the diff in the handler. The fee itself is applied through the
/// journal (`load_account_mut`), not by mutating this map, so accounts stay loaded.
pub trait DaStateAccess {
    /// Returns the transaction's state change-set.
    fn evm_state(&self) -> &EvmState;
}

impl<DB: Database> DaStateAccess for EthEvmContext<DB> {
    fn evm_state(&self) -> &EvmState {
        &self.journaled_state.state
    }
}

#[cfg(test)]
mod tests {
    use revm::state::{Account, AccountInfo, EvmStorageSlot};
    use revm_primitives::{Address, B256, U256};

    use super::*;

    fn touched_eoa() -> Account {
        let mut account = Account::from(AccountInfo::default());
        account.mark_touch();
        account
    }

    fn touched_contract() -> Account {
        let info = AccountInfo {
            code_hash: B256::repeat_byte(0x11),
            ..AccountInfo::default()
        };
        let mut account = Account::from(info);
        account.mark_touch();
        account
    }

    fn state_of(accounts: impl IntoIterator<Item = (Address, Account)>) -> EvmState {
        let mut state = EvmState::default();
        for (addr, account) in accounts {
            state.insert(addr, account);
        }
        state
    }

    /// Expected `diff_size` for the given raw account/storage byte totals, mirroring
    /// [`calc_diff_size`]'s arithmetic so the tests pin the exact formula.
    fn expected(account_raw: u64, storage_raw: u64) -> u64 {
        let account_discounted = account_raw * ACCOUNT_DISCOUNT_BPS / BPS_DENOM;
        let storage_discounted = storage_raw * STORAGE_DISCOUNT_BPS / BPS_DENOM;
        (account_discounted + storage_discounted) * COMPRESSION_RATIO_BPS / BPS_DENOM
            + DA_FIXED_OVERHEAD
    }

    #[test]
    fn empty_state_is_only_overhead() {
        assert_eq!(calc_diff_size(&EvmState::default()), DA_FIXED_OVERHEAD);
    }

    #[test]
    fn untouched_accounts_are_ignored() {
        // An account present but not touched must not contribute.
        let state = state_of([(
            Address::repeat_byte(1),
            Account::from(AccountInfo::default()),
        )]);
        assert_eq!(calc_diff_size(&state), DA_FIXED_OVERHEAD);
    }

    #[test]
    fn single_eoa() {
        let state = state_of([(Address::repeat_byte(1), touched_eoa())]);
        assert_eq!(
            calc_diff_size(&state),
            expected(ACCOUNT_KEY_BYTES + ACCOUNT_INFO_EOA_BYTES, 0)
        );
    }

    #[test]
    fn contract_costs_more_than_eoa() {
        let eoa = state_of([(Address::repeat_byte(1), touched_eoa())]);
        let contract = state_of([(Address::repeat_byte(1), touched_contract())]);
        assert!(calc_diff_size(&contract) > calc_diff_size(&eoa));
    }

    #[test]
    fn changed_storage_counts_unchanged_does_not() {
        let mut account = touched_eoa();
        // changed slot: original != present
        account.storage.insert(
            U256::from(1),
            EvmStorageSlot::new_changed(U256::ZERO, U256::from(9), 0),
        );
        // unchanged slot: original == present
        account
            .storage
            .insert(U256::from(2), EvmStorageSlot::new(U256::from(7), 0));

        let state = state_of([(Address::repeat_byte(1), account)]);
        assert_eq!(
            calc_diff_size(&state),
            expected(
                ACCOUNT_KEY_BYTES + ACCOUNT_INFO_EOA_BYTES,
                SLOT_KEY_BYTES + SLOT_VALUE_BYTES
            )
        );
    }

    #[test]
    fn deterministic_regardless_of_insertion_order() {
        let a = (Address::repeat_byte(1), touched_eoa());
        let b = (Address::repeat_byte(2), touched_contract());
        let forward = state_of([a.clone(), b.clone()]);
        let backward = state_of([b, a]);
        assert_eq!(calc_diff_size(&forward), calc_diff_size(&backward));
    }

    #[test]
    fn da_rate_extra_data_roundtrips() {
        let rate = 2_500_000_000_u64;
        assert_eq!(da_rate_from_extra_data(&da_rate_to_extra_data(rate)), rate);
    }

    #[test]
    fn short_extra_data_decodes_to_zero() {
        // Genesis label / empty field => no rate => charge is dormant.
        assert_eq!(da_rate_from_extra_data(&Bytes::from_static(b"SC")), 0);
        assert_eq!(da_rate_from_extra_data(&Bytes::new()), 0);
    }

    #[test]
    fn bounded_da_fee_caps_at_remaining_value() {
        let da_rate = U256::from(1_000u64);
        // raw = 1000 * 50 = 50_000; budget 60_000 => full fee charged.
        assert_eq!(
            bounded_da_fee(da_rate, 50, U256::from(60_000u64)),
            U256::from(50_000u64)
        );
        // raw = 50_000 but budget only 20_000 => capped (undercharge, never overcharge).
        assert_eq!(
            bounded_da_fee(da_rate, 50, U256::from(20_000u64)),
            U256::from(20_000u64)
        );
        // zero rate or zero budget => zero fee.
        assert_eq!(
            bounded_da_fee(U256::ZERO, 50, U256::from(60_000u64)),
            U256::ZERO
        );
        assert_eq!(bounded_da_fee(da_rate, 50, U256::ZERO), U256::ZERO);
    }
}
