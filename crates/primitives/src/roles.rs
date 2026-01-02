use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};

/// Roles with authority in the administration subprotocol.
#[repr(u8)]
#[derive(
    Copy,
    Clone,
    Debug,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Arbitrary,
    BorshDeserialize,
    BorshSerialize,
)]
#[borsh(use_discriminant = false)]
pub enum Role {
    /// The multisig authority that has exclusive ability to:
    /// 1. update (add/remove) bridge signers
    /// 2. update (add/remove) bridge operators
    /// 3. update the definition of what is considered a valid bridge deposit address for:
    ///    - registering deposit UTXOs
    ///    - accepting and minting bridge deposits
    ///    - assigning registered UTXOs to withdrawal requests
    /// 4. update the verifying key for the OL STF
    StrataAdministrator,

    /// The multisig authority that has exclusive ability to change the canonical
    /// public key of the default orchestration layer sequencer.
    StrataSequencerManager,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize)]
pub enum ProofType {
    Asm,
    OLStf,
}

#[cfg(test)]
mod tests {
    use std::mem;

    use super::*;
    #[test]
    fn test_role_variants_contigous() {
        // There are 2 variants.
        let count = 2;
        // let count = std::mem::variant_count::<Role>() as u8; // This is not available in stable
        // Rust, so we use a constant.

        for i in 0..count {
            let role: Role = unsafe { mem::transmute(i) };
            assert_eq!(role as u8, i);
        }
    }
}
