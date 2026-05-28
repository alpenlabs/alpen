//! Tests for the buried-manifest filter applied by
//! [`BlockAssemblyAnchorContext::fetch_asm_manifests_from`].
//!
//! The filter prevents L1 reorgs from cascading into OL by only sourcing manifests
//! at or below `asm_tip - l1_reorg_safe_depth`.

use std::sync::Arc;

use strata_ol_state_provider::OLStateManagerProviderImpl;

use crate::{
    context::{BlockAssemblyAnchorContext, BlockAssemblyContext},
    test_utils::{
        BlockAssemblyContextImpl, MockMempoolProvider, create_test_storage,
        setup_asm_state_with_l1_manifests,
    },
};

const MAX_COUNT: u32 = 100;

async fn build_ctx(asm_tip: u32, l1_reorg_safe_depth: u32) -> BlockAssemblyContextImpl {
    let storage = create_test_storage();
    setup_asm_state_with_l1_manifests(storage.as_ref(), 1, asm_tip).await;
    let mempool_provider = Arc::new(MockMempoolProvider::new());
    let state_provider = OLStateManagerProviderImpl::new(storage.ol_state().clone());
    BlockAssemblyContext::new(
        storage,
        mempool_provider,
        state_provider,
        l1_reorg_safe_depth,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn returns_only_buried_manifests() {
    // ASM tip = 10, depth = 3 -> buried tip = 7. Caller asks from height 1.
    let ctx = build_ctx(10, 3).await;
    let manifests = ctx
        .fetch_asm_manifests_from(1, MAX_COUNT)
        .await
        .expect("fetch should succeed");

    let heights: Vec<u32> = manifests.iter().map(|m| m.height()).collect();
    assert_eq!(heights, (1..=7).collect::<Vec<_>>());
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_when_tip_not_yet_buried() {
    // ASM tip = 2, depth = 3 -> buried tip saturates to 0, so nothing is buried yet.
    let ctx = build_ctx(2, 3).await;
    let manifests = ctx
        .fetch_asm_manifests_from(1, MAX_COUNT)
        .await
        .expect("fetch should succeed");
    assert!(manifests.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn max_count_is_binding_cap() {
    // ASM tip = 100, depth = 3 -> buried tip = 97. max_count = 4 caps before buried tip.
    let ctx = build_ctx(100, 3).await;
    let manifests = ctx
        .fetch_asm_manifests_from(1, 4)
        .await
        .expect("fetch should succeed");

    let heights: Vec<u32> = manifests.iter().map(|m| m.height()).collect();
    assert_eq!(heights, vec![1, 2, 3, 4]);
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_when_start_past_buried_tip() {
    // ASM tip = 5, depth = 3 -> buried tip = 2. Asking from 4 returns empty.
    let ctx = build_ctx(5, 3).await;
    let manifests = ctx
        .fetch_asm_manifests_from(4, MAX_COUNT)
        .await
        .expect("fetch should succeed");
    assert!(manifests.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn depth_zero_returns_through_tip() {
    // depth = 0 means buried tip == asm tip; the filter is a no-op.
    let ctx = build_ctx(5, 0).await;
    let manifests = ctx
        .fetch_asm_manifests_from(1, MAX_COUNT)
        .await
        .expect("fetch should succeed");

    let heights: Vec<u32> = manifests.iter().map(|m| m.height()).collect();
    assert_eq!(heights, (1..=5).collect::<Vec<_>>());
}
