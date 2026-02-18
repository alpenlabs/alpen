//! Checkpoint predicate resolution based on enabled features.

use strata_predicate::PredicateKey;

/// Returns the appropriate [`PredicateKey`] based on the enabled features.
///
/// If the **sp1** feature is enabled, returns an Sp1Groth16 PredicateKey.
/// Otherwise, returns an AlwaysAccept PredicateKey.
pub(crate) fn resolve_checkpoint_predicate() -> PredicateKey {
    // Use SP1 if `sp1` feature is enabled
    #[cfg(feature = "sp1-builder")]
    {
        use strata_predicate::PredicateTypeId;
        use strata_primitives::buf::Buf32;
        use strata_sp1_guest_builder::GUEST_CHECKPOINT_VK_HASH_STR;
        use zkaleido_sp1_groth16_verifier::SP1Groth16Verifier;
        let vk_buf32: Buf32 = GUEST_CHECKPOINT_VK_HASH_STR
            .parse()
            .expect("invalid sp1 checkpoint verifier key hash");
        let sp1_verifier = SP1Groth16Verifier::load(&sp1_verifier::GROTH16_VK_BYTES, vk_buf32.0)
            .expect("Failed to load SP1 Groth16 verifier");
        let condition_bytes = sp1_verifier.vk.to_uncompressed_bytes();
        PredicateKey::new(PredicateTypeId::Sp1Groth16, condition_bytes)
    }

    // If `sp1` is not enabled, use the AlwaysAccept predicate
    #[cfg(not(feature = "sp1-builder"))]
    {
        PredicateKey::always_accept()
    }
}
