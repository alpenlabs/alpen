use arbitrary::Arbitrary;
use bitcoin::{
    XOnlyPublicKey,
    secp256k1::{Keypair, SECP256K1, SecretKey},
};

use crate::unstake::UnstakeTxHeaderAux;

/// Information extracted from an unstake transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnstakeInfo {
    /// SPS-50 auxiliary data from the transaction tag.
    header_aux: UnstakeTxHeaderAux,
    /// Pubkey extracted from the stake-connector script (witness element 2).
    ///
    /// During validation, this pubkey is converted to a key-path-only P2TR script using
    /// this key as internal key, then checked against historical operator set scripts in state.
    witness_pushed_pubkey: XOnlyPublicKey,
}

impl UnstakeInfo {
    pub fn new(header_aux: UnstakeTxHeaderAux, witness_pushed_pubkey: XOnlyPublicKey) -> Self {
        Self {
            header_aux,
            witness_pushed_pubkey,
        }
    }

    pub fn header_aux(&self) -> &UnstakeTxHeaderAux {
        &self.header_aux
    }

    pub fn witness_pushed_pubkey(&self) -> &XOnlyPublicKey {
        &self.witness_pushed_pubkey
    }
}

impl<'a> Arbitrary<'a> for UnstakeInfo {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let header_aux = UnstakeTxHeaderAux::arbitrary(u)?;

        let mut secret_key_bytes = [0u8; 32];
        u.fill_buffer(&mut secret_key_bytes)?;

        let secret_key = SecretKey::from_slice(&secret_key_bytes)
            .map_err(|_| arbitrary::Error::IncorrectFormat)?;
        let keypair = Keypair::from_secret_key(SECP256K1, &secret_key);
        let (witness_pushed_pubkey, _parity) = keypair.x_only_public_key();

        Ok(Self {
            header_aux,
            witness_pushed_pubkey,
        })
    }
}
