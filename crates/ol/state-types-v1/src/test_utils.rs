//! Test utilities and proptest strategies for OL state types.

use proptest::prelude::*;
use ssz_types::VariableList;
use strata_acct_types::BitcoinAmount;
use strata_identifiers::test_utils::{
    account_id_strategy, account_serial_strategy, buf32_strategy,
};
use strata_identifiers::{EpochCommitment, L1BlockCommitment, L1BlockId, OLBlockId};
use strata_merkle::Mmr64B32;
use strata_ol_params::OLParams;
use strata_predicate::PredicateKey;

use crate::ssz_generated::ssz::state::*;

const MAX_BITCOIN_MONEY_SATS: u64 = 21_000_000 * 100_000_000;

/// Creates a genesis OLStateV1 using minimal empty parameters.
pub fn create_test_genesis_state() -> OLStateV1 {
    let params = OLParams::default();
    OLStateV1::from_genesis_params(&params).expect("valid params")
}

pub fn bitcoin_amount_strategy() -> impl Strategy<Value = BitcoinAmount> {
    (0..=MAX_BITCOIN_MONEY_SATS).prop_map(|sats| {
        BitcoinAmount::try_from(sats).expect("strategy only generates valid Bitcoin amounts")
    })
}

pub fn global_state_strategy() -> impl Strategy<Value = GlobalStateV1> {
    any::<(u64, u64, u64)>().prop_map(|(cur_slot, next_avail_serial, limbo_funds_sats)| {
        GlobalStateV1 {
            cur_slot,
            next_avail_serial,
            limbo_funds_sats,
        }
    })
}

pub fn epochal_state_strategy() -> impl Strategy<Value = EpochalStateV1> {
    (
        bitcoin_amount_strategy(),
        any::<u32>(),
        buf32_strategy(),
        (any::<u32>(), any::<u64>(), buf32_strategy()),
    )
        .prop_map(
            |(funds, epoch, l1_blkid, (cp_epoch, cp_slot, cp_blkid))| EpochalStateV1 {
                total_ledger_funds: funds,
                cur_epoch: epoch,
                last_l1_block: L1BlockCommitment::new(0, L1BlockId::from(l1_blkid)),
                checkpointed_epoch: EpochCommitment::new(
                    cp_epoch,
                    cp_slot,
                    OLBlockId::from(cp_blkid),
                ),
                l1_block_refs_mmr: Mmr64B32 {
                    entries: 0,
                    roots: Default::default(),
                },
            },
        )
}

pub fn proof_state_strategy() -> impl Strategy<Value = ProofStateV1> {
    (buf32_strategy(), any::<u64>()).prop_map(|(inner_state, next_idx)| {
        let hash_bytes: [u8; 32] = inner_state.into();
        ProofStateV1 {
            inner_state_root: hash_bytes.into(),
            next_msg_read_idx: next_idx,
        }
    })
}

pub fn ol_snark_account_state_strategy() -> impl Strategy<Value = OLSnarkAccountStateV1> {
    buf32_strategy().prop_map(|inner_state| {
        // Use new_fresh to create a valid snark account state
        OLSnarkAccountStateV1::new_fresh(PredicateKey::always_accept(), inner_state)
    })
}

pub fn ol_account_type_state_strategy() -> impl Strategy<Value = OLAccountTypeStateV1> {
    prop::bool::ANY.prop_flat_map(|is_snark| {
        if is_snark {
            ol_snark_account_state_strategy()
                .prop_map(OLAccountTypeStateV1::Snark)
                .boxed()
        } else {
            Just(OLAccountTypeStateV1::Empty).boxed()
        }
    })
}

pub fn ol_account_state_strategy() -> impl Strategy<Value = OLAccountStateV1> {
    (
        account_serial_strategy(),
        bitcoin_amount_strategy(),
        ol_account_type_state_strategy(),
    )
        .prop_map(|(serial, balance, state)| OLAccountStateV1 {
            serial,
            balance,
            state,
        })
}

pub fn tsnl_account_entry_strategy() -> impl Strategy<Value = TsnlAccountEntryV1> {
    (account_id_strategy(), ol_account_state_strategy())
        .prop_map(|(id, state)| TsnlAccountEntryV1 { id, state })
}

pub fn tsnl_ledger_accounts_table_strategy() -> impl Strategy<Value = TsnlLedgerAccountsTableV1> {
    // Small number of accounts for testing (0-10)
    prop::collection::vec(tsnl_account_entry_strategy(), 0..10).prop_map(|mut entries| {
        // Sort entries by account ID (requirement for TsnlLedgerAccountsTableV1)
        entries.sort_by_key(|e| e.id);

        let mut accounts = VariableList::default();

        // Add entries
        for entry in entries {
            accounts
                .push(entry)
                .expect("within MAX_LEDGER_ACCOUNTS capacity");
        }

        TsnlLedgerAccountsTableV1 { accounts }
    })
}

pub fn ol_state_strategy() -> impl Strategy<Value = OLStateV1> {
    (
        epochal_state_strategy(),
        global_state_strategy(),
        tsnl_ledger_accounts_table_strategy(),
    )
        .prop_map(|(epoch, global, ledger)| OLStateV1 {
            epoch,
            global,
            intraepoch: IntraepochStateV1::default(),
            ledger,
        })
}
