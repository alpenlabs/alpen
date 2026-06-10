//! Depth-0 per-block witness construction.
//!
//! Builds a single block's transition witness ([`EvmPartialState`]) at the
//! instant the block is produced/imported, while the block is still at tip.
//! Both multiproofs are taken against the **parent** state provider (`state(B−1)`,
//! which is the committed tip at that moment), so neither call ever opens a
//! historical state provider:
//!
//! - **pre** proofs use an empty overlay → proofs at `root(B−1)`,
//! - **post** proofs overlay the block's write set → proofs at `root(B)`.
//!
//! The result is anchored at `root(B−1)` and carries enough structure to be
//! updated to `root(B)` — the same shape [`EthereumState::from_transition_proofs`]
//! produces for a chunk, scoped to one block. The chunk witness is the ordered
//! list of these across the chunk's blocks.

use std::collections::BTreeMap;

use alloy_consensus::Header;
use alloy_primitives::{
    keccak256,
    map::{B256Set, DefaultHashBuilder, HashMap},
    Address, B256,
};
use eyre::{eyre, Result};
use reth_provider::StateProvider;
use reth_revm::state::Bytecode;
use reth_trie::{HashedPostState, MultiProofTargets, TrieInput};
use reth_trie_common::KeccakKeyHasher;
use rsp_mpt::EthereumState;
use strata_evm_ee::EvmPartialState;

/// Builds the depth-0 transition witness for a single block.
///
/// `provider` must be the state provider for the block's **parent**
/// (`state(B−1)`), opened while that state is at (or near) tip so the
/// multiproofs stay shallow. `touched` maps each accessed account to the
/// storage slots the block touched; `write_set` is the block's hashed write
/// set (used as the post-state overlay); `start_state_root` is `root(B−1)`;
/// `bytecodes` and `ancestor_headers` are the block's touched bytecodes and the
/// `BLOCKHASH` ancestor headers it used.
pub fn build_block_witness<P: StateProvider>(
    provider: &P,
    touched: &HashMap<Address, Vec<B256>>,
    write_set: HashedPostState,
    start_state_root: B256,
    bytecodes: BTreeMap<B256, Bytecode>,
    ancestor_headers: Vec<Header>,
) -> Result<EvmPartialState> {
    // All accessed accounts/slots go into the multiproof targets.
    let targets = MultiProofTargets::from_iter(touched.iter().map(|(addr, keys)| {
        (
            keccak256(addr),
            B256Set::from_iter(keys.iter().map(keccak256)),
        )
    }));

    // pre: proofs against the parent state (`root(B−1)`), empty overlay.
    let proof_pre = provider.multiproof(
        TrieInput::from_state(HashedPostState::from_bundle_state::<KeccakKeyHasher>([])),
        targets.clone(),
    )?;
    // post: proofs against the parent state with the block's writes overlaid,
    // yielding proofs valid at `root(B)` without committing the block.
    let proof_post = provider.multiproof(TrieInput::from_state(write_set), targets)?;

    let mut pre_proofs =
        HashMap::with_capacity_and_hasher(touched.len(), DefaultHashBuilder::default());
    let mut post_proofs =
        HashMap::with_capacity_and_hasher(touched.len(), DefaultHashBuilder::default());
    for (addr, keys) in touched {
        pre_proofs.insert(*addr, proof_pre.account_proof(*addr, keys)?);
        post_proofs.insert(*addr, proof_post.account_proof(*addr, keys)?);
    }

    let state = EthereumState::from_transition_proofs(start_state_root, &pre_proofs, &post_proofs)
        .map_err(|e| eyre!("failed to build per-block EthereumState: {e}"))?;

    Ok(EvmPartialState::new(state, bytecodes, ancestor_headers))
}
