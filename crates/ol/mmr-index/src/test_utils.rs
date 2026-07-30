use strata_acct_types::{BitcoinAmount, MessageEntry, Mmr64, MsgPayload, StrataHasher};
use strata_db_types::MmrId;
use strata_identifiers::{AccountId, Buf32, Hash, L1BlockCommitment, L1BlockId};
use strata_ledger_types::{IAccountStateMut, ISnarkAccountStateMut};
use strata_merkle::{Mmr, MmrState};
use strata_ol_params::{BridgeParams, GenesisSnarkAccountData, OLParams};
use strata_ol_state_support_types::MemoryStateBaseLayer;
use strata_ol_state_types::{OLAccountState, OLState, WriteBatch};
use strata_predicate::PredicateKey;

use crate::{MmrIndexEntry, resolve_ol_mmr_target};

pub(crate) fn build_genesis_target_state() -> OLState {
    OLState::from_genesis_params(&OLParams::new_empty(
        L1BlockCommitment::default(),
        BridgeParams::default(),
    ))
    .expect("valid genesis params")
}

pub(crate) fn build_target_state_with_empty_l1_block_refs_mmr() -> OLState {
    let mut state = build_genesis_target_state();

    let mut batch = WriteBatch::<OLAccountState>::default();
    batch.epochal_writes_mut().l1_block_refs_mmr = Some(Mmr64::new_empty());
    state
        .apply_write_batch(batch)
        .expect("apply empty target L1 refs MMR");
    state
}

pub(crate) fn build_snark_inbox_message(seed: u8) -> MessageEntry {
    let payload = MsgPayload::from_bytes(BitcoinAmount::from_sat(1), vec![seed]).expect("payload");
    MessageEntry::new(AccountId::new([seed; 32]), 0, payload)
}

pub(crate) fn build_target_state_with_snark_account(account_id: AccountId) -> OLState {
    let mut params = OLParams::new_empty(
        L1BlockCommitment::new(0, L1BlockId::from(Buf32::zero())),
        BridgeParams::default(),
    );
    params.accounts.insert(
        account_id,
        GenesisSnarkAccountData {
            predicate: PredicateKey::always_accept(),
            inner_state: Hash::zero(),
            balance: BitcoinAmount::ZERO,
        },
    );

    OLState::from_genesis_params(&params).expect("valid genesis params")
}

pub(crate) fn build_target_state_with_snark_inbox(
    account_id: AccountId,
    messages: Vec<MessageEntry>,
) -> OLState {
    let mut state = build_target_state_with_snark_account(account_id);
    let mut account = state
        .get_account_state(&account_id)
        .expect("genesis snark account")
        .clone();
    let snark_account = account
        .as_snark_account_mut()
        .expect("genesis account should be snark");
    for message in messages {
        snark_account
            .insert_inbox_message(message)
            .expect("insert inbox message");
    }

    let mut batch = WriteBatch::<OLAccountState>::default();
    batch.ledger_mut().update_account(account_id, account);
    state
        .apply_write_batch(batch)
        .expect("apply target snark inbox MMR");
    state
}

/// Builds a real `count`-leaf MMR whose leaves are all `leaf`.
///
/// Uses [`Mmr::new_repeated`] so the peaks are genuine hashes rather than a
/// hand-rolled state; two calls with different `leaf` values yield states that
/// share a leaf count but differ in peaks.
pub(crate) fn build_repeated_leaf_mmr(leaf: Hash, count: u64) -> Mmr64 {
    <Mmr64 as Mmr<StrataHasher>>::new_repeated(leaf.0, count)
}

pub(crate) fn build_index_entry(mmr_id: MmrId, leaf_count: u64) -> MmrIndexEntry {
    MmrIndexEntry::new(
        mmr_id,
        build_repeated_leaf_mmr(Hash::from([0x11; 32]), leaf_count),
    )
}

pub(crate) fn build_target_index_entry(target_state: &OLState, mmr_id: MmrId) -> MmrIndexEntry {
    let target_accessor = build_target_state_accessor(target_state);
    let target = resolve_ol_mmr_target(&target_accessor, &mmr_id)
        .expect("target MMR read should succeed")
        .expect("target MMR should be OL-owned");
    MmrIndexEntry::new(mmr_id, target)
}

pub(crate) fn build_target_state_accessor(target_state: &OLState) -> MemoryStateBaseLayer {
    MemoryStateBaseLayer::new(target_state.clone())
}
