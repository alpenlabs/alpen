use borsh::{BorshDeserialize, BorshSerialize};

/// Roles with authority in the upgrade subprotocol.
#[derive(
    Debug, Clone, Copy, Eq, PartialEq, PartialOrd, Ord, Hash, BorshSerialize, BorshDeserialize,
)]
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

#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub enum StrataProof {
    ASM,
    OlStf,
}
