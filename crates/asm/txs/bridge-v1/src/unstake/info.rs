use arbitrary::Arbitrary;
use bitcoin::XOnlyPublicKey;

use crate::unstake::UnstakeTxHeaderAux;

/// Information extracted from an unstake transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnstakeInfo {
    /// SPS-50 auxiliary data from the transaction tag.
    header_aux: UnstakeTxHeaderAux,
    /// Pubkey extracted from the stake-connector script (witness element 2).
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

        let mut candidate = [0u8; 32];
        u.fill_buffer(&mut candidate)?;
        let witness_pushed_pubkey = XOnlyPublicKey::from_slice(&candidate)
            .map_err(|_| arbitrary::Error::IncorrectFormat)?;

        Ok(Self {
            header_aux,
            witness_pushed_pubkey,
        })
    }
}
