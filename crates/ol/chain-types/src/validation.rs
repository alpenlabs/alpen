//! Validation helpers for OL chain objects.

use strata_crypto::verify_schnorr_sig;
use strata_identifiers::{Buf32, Buf64};
use strata_predicate::{PredicateKey, PredicateTypeId};

/// Returns whether a sequencer predicate requires a block signature.
///
/// [`PredicateTypeId::AlwaysAccept`] keeps the legacy unchecked behavior. Every
/// other predicate requires a signature witness so verification can reject
/// invalid proposers.
pub fn sequencer_predicate_requires_signature(sequencer_predicate: &PredicateKey) -> bool {
    sequencer_predicate.id() != PredicateTypeId::AlwaysAccept.as_u8()
}

/// Verifies a sequencer signature witness against the configured predicate.
pub fn verify_sequencer_predicate_signature(
    sequencer_predicate: &PredicateKey,
    msg: &Buf32,
    sig: &Buf64,
) -> bool {
    let Ok(predicate_type) = PredicateTypeId::try_from(sequencer_predicate.id()) else {
        return false;
    };

    match predicate_type {
        PredicateTypeId::NeverAccept => false,
        PredicateTypeId::AlwaysAccept => true,
        PredicateTypeId::Bip340Schnorr => {
            let Ok(pubkey) = Buf32::try_from(sequencer_predicate.condition()) else {
                return false;
            };
            verify_schnorr_sig(sig, msg, &pubkey)
        }
        PredicateTypeId::Sp1Groth16 => false,
    }
}

#[cfg(test)]
mod tests {
    use strata_crypto::sign_schnorr_sig;
    use strata_primitives::utils::get_test_schnorr_keys;

    use super::*;
    use crate::test_utils::schnorr_predicate;

    fn test_msg() -> Buf32 {
        Buf32::from([42; 32])
    }

    fn test_schnorr_keypair() -> (Buf32, Buf32) {
        let keypair = get_test_schnorr_keys()[0].clone();
        (keypair.sk, keypair.pk)
    }

    #[test]
    fn always_accept_accepts_any_signature() {
        let predicate = PredicateKey::always_accept();
        let msg = test_msg();
        let sig = Buf64::zero();

        assert!(verify_sequencer_predicate_signature(&predicate, &msg, &sig));
    }

    #[test]
    fn bip340_schnorr_accepts_valid_signature() {
        let (sk, pk) = test_schnorr_keypair();
        let predicate = schnorr_predicate(&pk);
        let msg = test_msg();
        let sig = sign_schnorr_sig(&msg, &sk);

        assert!(verify_sequencer_predicate_signature(&predicate, &msg, &sig));
    }

    #[test]
    fn bip340_schnorr_rejects_wrong_signature() {
        let (_sk, pk) = test_schnorr_keypair();
        let predicate = schnorr_predicate(&pk);
        let msg = test_msg();
        let sig = Buf64::zero();

        assert!(!verify_sequencer_predicate_signature(
            &predicate, &msg, &sig
        ));
    }

    #[test]
    fn never_accept_rejects_signature() {
        let predicate = PredicateKey::never_accept();
        let msg = test_msg();
        let sig = Buf64::zero();

        assert!(!verify_sequencer_predicate_signature(
            &predicate, &msg, &sig
        ));
    }

    #[test]
    fn signature_required_only_for_non_always_accept_predicates() {
        let (_sk, pk) = test_schnorr_keypair();
        let schnorr = schnorr_predicate(&pk);

        assert!(!sequencer_predicate_requires_signature(
            &PredicateKey::always_accept()
        ));
        assert!(sequencer_predicate_requires_signature(&schnorr));
        assert!(sequencer_predicate_requires_signature(
            &PredicateKey::never_accept()
        ));
    }
}
