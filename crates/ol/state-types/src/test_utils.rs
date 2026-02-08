//! Test utilities and proptest strategies for OL state types.

use proptest::prelude::*;
use ssz_types::VariableList;
use strata_acct_types::{AccountId, BitcoinAmount};
use strata_identifiers::{
    EpochCommitment, L1BlockCommitment, L1BlockId, OLBlockId,
    test_utils::{account_id_strategy, account_serial_strategy, buf32_strategy},
};
use strata_merkle::Mmr64B32;
use strata_predicate::PredicateKey;

use crate::ssz_generated::ssz::state::{
    EpochalState, GlobalState, OLAccountState, OLAccountTypeState, OLSnarkAccountState, OLState,
    ProofState, TsnlAccountEntry, TsnlLedgerAccountsTable,
};

pub(crate) fn bitcoin_amount_strategy() -> impl Strategy<Value = BitcoinAmount> {
    any::<u64>().prop_map(BitcoinAmount::from_sat)
}

pub fn global_state_strategy() -> impl Strategy<Value = GlobalState> {
    any::<u64>().prop_map(|cur_slot| GlobalState { cur_slot })
}

pub fn epochal_state_strategy() -> impl Strategy<Value = EpochalState> {
    (
        bitcoin_amount_strategy(),
        any::<u32>(),
        buf32_strategy(),
        (any::<u32>(), any::<u64>(), buf32_strategy()),
    )
        .prop_map(
            |(funds, epoch, l1_blkid, (cp_epoch, cp_slot, cp_blkid))| EpochalState {
                total_ledger_funds: funds,
                cur_epoch: epoch,
                last_l1_block: L1BlockCommitment::from_height_u64(0, L1BlockId::from(l1_blkid))
                    .unwrap(),
                checkpointed_epoch: EpochCommitment::new(
                    cp_epoch,
                    cp_slot,
                    OLBlockId::from(cp_blkid),
                ),
                manifests_mmr: Mmr64B32 {
                    entries: 0,
                    roots: Default::default(),
                },
            },
        )
}

pub fn proof_state_strategy() -> impl Strategy<Value = ProofState> {
    (buf32_strategy(), any::<u64>()).prop_map(|(inner_state, next_idx)| {
        let hash_bytes: [u8; 32] = inner_state.into();
        ProofState {
            inner_state_root: hash_bytes.into(),
            next_msg_read_idx: next_idx,
        }
    })
}

pub fn ol_snark_account_state_strategy() -> impl Strategy<Value = OLSnarkAccountState> {
    buf32_strategy().prop_map(|inner_state| {
        // Use new_fresh to create a valid snark account state
        OLSnarkAccountState::new_fresh(PredicateKey::always_accept(), inner_state)
    })
}

pub fn ol_account_type_state_strategy() -> impl Strategy<Value = OLAccountTypeState> {
    prop::bool::ANY.prop_flat_map(|is_snark| {
        if is_snark {
            ol_snark_account_state_strategy()
                .prop_map(OLAccountTypeState::Snark)
                .boxed()
        } else {
            Just(OLAccountTypeState::Empty).boxed()
        }
    })
}

pub fn ol_account_state_strategy() -> impl Strategy<Value = OLAccountState> {
    (
        account_serial_strategy(),
        bitcoin_amount_strategy(),
        ol_account_type_state_strategy(),
    )
        .prop_map(|(serial, balance, state)| OLAccountState {
            serial,
            balance,
            state,
        })
}

pub fn tsnl_account_entry_strategy() -> impl Strategy<Value = TsnlAccountEntry> {
    (account_id_strategy(), ol_account_state_strategy())
        .prop_map(|(id, state)| TsnlAccountEntry { id, state })
}

pub fn tsnl_ledger_accounts_table_strategy() -> impl Strategy<Value = TsnlLedgerAccountsTable> {
    // Small number of accounts for testing (0-10)
    prop::collection::vec(tsnl_account_entry_strategy(), 0..10).prop_map(|mut entries| {
        // Sort entries by account ID (requirement for TsnlLedgerAccountsTable)
        entries.sort_by_key(|e| e.id);

        let mut accounts = VariableList::default();
        let mut serials = VariableList::default();

        // Add system reserved serials (zeros)
        for _ in 0..strata_acct_types::SYSTEM_RESERVED_ACCTS {
            serials
                .push(AccountId::zero())
                .expect("within MAX_ACCOUNT_SERIALS");
        }

        // Add entries
        for entry in entries {
            accounts
                .push(entry)
                .expect("within MAX_LEDGER_ACCOUNTS capacity");
            serials
                .push(AccountId::zero()) // Placeholder, would need proper ID mapping
                .expect("within MAX_ACCOUNT_SERIALS");
        }

        TsnlLedgerAccountsTable { accounts, serials }
    })
}

pub fn ol_state_strategy() -> impl Strategy<Value = OLState> {
    (
        epochal_state_strategy(),
        global_state_strategy(),
        tsnl_ledger_accounts_table_strategy(),
    )
        .prop_map(|(epoch, global, ledger)| OLState {
            epoch,
            global,
            ledger,
        })
}
