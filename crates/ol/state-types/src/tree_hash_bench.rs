//! Timing harness for state root computation over a populated ledger.
//!
//! Run with:
//! `BENCH_ACCTS=100000 cargo test --release -p strata-ol-state-types \
//!   tree_hash_bench -- --ignored --nocapture`

use std::{hint::black_box, time::Instant};

use strata_acct_types::{
    AccountId, AccountSerial, BitcoinAmount, SYSTEM_RESERVED_ACCTS,
    tree_hash::{Sha256Hasher, TreeHash},
};
use strata_predicate::PredicateKey;

use crate::{
    OLAccountState, OLAccountTypeState, OLSnarkAccountState,
    test_utils::create_test_genesis_state,
};

fn account_id(i: u64) -> AccountId {
    let mut bytes = [0u8; 32];
    bytes[..8].copy_from_slice(&i.to_be_bytes());
    AccountId::from(bytes)
}

#[test]
#[ignore]
fn bench_tree_hash_root() {
    let n: u64 = std::env::var("BENCH_ACCTS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000);

    let mut state = create_test_genesis_state();
    let build_start = Instant::now();
    for i in 0..n {
        let serial = AccountSerial::new(SYSTEM_RESERVED_ACCTS + i as u32);
        let snark = OLSnarkAccountState::new_fresh(PredicateKey::always_accept(), [0u8; 32].into());
        let acct = OLAccountState::new(
            serial,
            BitcoinAmount::from_sat(1_000 + i),
            OLAccountTypeState::Snark(snark),
        );
        state
            .ledger
            .create_account(account_id(SYSTEM_RESERVED_ACCTS as u64 + i), acct)
            .expect("bench: create account");
    }
    println!("built {n} accounts in {:?}", build_start.elapsed());

    // Warmup.
    black_box(TreeHash::tree_hash_root::<Sha256Hasher>(black_box(&state)));

    const ITERS: u32 = 5;
    let start = Instant::now();
    for _ in 0..ITERS {
        black_box(TreeHash::tree_hash_root::<Sha256Hasher>(black_box(&state)));
    }
    let total = start.elapsed();
    let per_iter = total / ITERS;
    println!(
        "tree_hash_root over {n} accounts: {per_iter:?}/iter ({:.1} ns/account)",
        per_iter.as_nanos() as f64 / n as f64
    );
}
