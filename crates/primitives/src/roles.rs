use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};

/// Roles with authority in the administration subprotocol.
#[repr(u8)]
#[derive(
    Debug,
    Clone,
    Copy,
    Eq,
    PartialEq,
    PartialOrd,
    Ord,
    Hash,
    BorshSerialize,
    BorshDeserialize,
    Arbitrary,
)]
#[borsh(use_discriminant = false)]
pub enum Role {
    /// The multisig authority that has the exclusive ability to update
    /// (add/remove) bridge operators.
    BridgeAdmin,
    /// The multisig authority that has the exclusive abiltity to change the
    /// VerifyingKey of the Bridge. Since Bridge is implemented as a subprotcol
    /// in ASM, this entails that the new VK is for the entire ASM STF.
    BridgeConsensusManager,
    /// The multisig authority that has the exclusive abiltity to change the
    /// public key of the OL batch producer.
    StrataAdmin,
    /// The multisig authority that has the exclusive ability to change the
    /// VerifyingKey of the OL STF.
    StrataConsensusManager,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub enum ProofType {
    Asm,
    OlStf,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_role_variants_contigous() {
        // There are 4 variants.
        let count = 4;
        // let count = std::mem::variant_count::<Role>() as u8; // This is not available in stable
        // Rust, so we use a constant.

        for i in 0..count {
            let role: Role = unsafe { std::mem::transmute(i) };
            assert_eq!(role as u8, i);
        }
    }
}
