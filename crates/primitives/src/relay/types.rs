#![allow(dead_code)]

use core::fmt;

use arbitrary::Arbitrary;
use borsh::{io, BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    bridge::OperatorIdx,
    buf::{Buf32, Buf64},
    prelude::BitcoinTxid,
};

/// Message container used to direct payloads depending on the context between parties.
///
/// # Caution
///
/// Users should not construct a [`BridgeMessage`] directly,
/// instead construct a [`MessageSigner`](super::util::MessageSigner) by
/// calling [`MessageSigner::new`](super::util::MessageSigner::new),
/// followed by [`sign_raw`](super::util::MessageSigner::sign_raw)
/// or [`sign_scope`](super::util::MessageSigner::sign_scope)
/// depending on the use case.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Deserialize, Serialize)]
pub struct BridgeMessage {
    /// Operator ID
    pub(crate) source_id: OperatorIdx,

    /// Schnorr signature of the message
    pub(crate) sig: Buf64,

    /// Purpose of the message.
    pub(crate) scope: Vec<u8>,

    /// Serialized message
    pub(crate) payload: Vec<u8>,
}

impl<'a> Arbitrary<'a> for BridgeMessage {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let source_id = u32::arbitrary(u)?;
        let sig = Buf64::arbitrary(u)?;
        let scope = borsh::to_vec(&Scope::Misc).unwrap();
        let mut payload = vec![0; 20];
        u.fill_buffer(&mut payload)?;

        Ok(Self {
            source_id,
            sig,
            scope,
            payload,
        })
    }
}

impl BridgeMessage {
    /// Source ID.
    pub fn source_id(&self) -> u32 {
        self.source_id
    }

    /// Signature.
    pub fn signature(&self) -> &Buf64 {
        &self.sig
    }

    /// Raw scope.
    #[allow(clippy::missing_const_for_fn)]
    pub fn scope(&self) -> &[u8] {
        &self.scope
    }

    /// Raw payload
    #[allow(clippy::missing_const_for_fn)]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// Tries to parse the scope buf as a typed scope.
    pub fn try_parse_scope(&self) -> Option<Scope> {
        Scope::try_from_slice(self.scope()).ok()
    }

    /// Computes a msg ID based on the .
    pub fn compute_id(&self) -> BridgeMsgId {
        // No signature because it might be malleable and it doesn't have any
        // useful data in it we'd want to inspect.
        let mut digest = Sha256::default();
        digest.update(self.source_id.to_be_bytes());
        digest.update((self.scope.len() as u64).to_be_bytes());
        digest.update(&self.scope);
        digest.update((self.payload.len() as u64).to_be_bytes());
        digest.update(&self.payload);

        let hash: [u8; 32] = digest.finalize().into();
        BridgeMsgId::from(Buf32::from(hash))
    }
}

impl TryFrom<Vec<u8>> for BridgeMessage {
    type Error = io::Error;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        let result = borsh::from_slice(value.as_ref())?;
        Ok(result)
    }
}

impl TryInto<Vec<u8>> for BridgeMessage {
    type Error = io::Error;

    fn try_into(self) -> Result<Vec<u8>, Self::Error> {
        borsh::to_vec(&self).map_err(|_| io::Error::other("Serialization error"))
    }
}

impl TryFrom<&[u8]> for BridgeMessage {
    type Error = io::Error;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        let result = borsh::from_slice(value)?;
        Ok(result)
    }
}

impl TryFrom<Box<[u8]>> for BridgeMessage {
    type Error = io::Error;

    fn try_from(value: Box<[u8]>) -> Result<Self, Self::Error> {
        let result = borsh::from_slice(value.as_ref())?;
        Ok(result)
    }
}

impl TryInto<Box<[u8]>> for BridgeMessage {
    type Error = io::Error;

    fn try_into(self) -> Result<Box<[u8]>, Self::Error> {
        let serialized_vec =
            borsh::to_vec(&self).map_err(|_| io::Error::other("Serialization error"))?;
        Ok(serialized_vec.into_boxed_slice()) // Convert Vec<u8> to Box<[u8]>
    }
}

/// Scope of the [`BridgeMessage`].
///
/// As V0 deposits and withdrawals can be uniquely identified via the [`BitcoinTxid`], there is no
/// need to have separate variants for `Deposits` and `Withdrawals`. From the POV of the
/// bridge-relay, a deposit signature is indistinguishable from a withdrawal signature i.e.,
/// nothing special is being done here with that kind of distinction.
///
/// Tracking the status of duties is done on the client side and the identification of such duties
/// can be identified using the outpoint of the transaction that is being spent i.e, the deposit
/// request outpoint can be used to track the status of deposit duties and the deposit outpoint can
/// be used to track the withdrawal duty. Furthermore, since the underlying bridge transactions all
/// spend the first outpoint (vout = 0), it suffices to just use the [`Txid`](bitcoin::Txid) for the
/// tracking instead of the full [`OutPoint`](bitcoin::OutPoint).
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize, Deserialize, Serialize)]
pub enum Scope {
    /// Used for debugging purposes.
    Misc,

    /// Signature with the corresponding [`BitcoinTxid`]
    V0Sig(BitcoinTxid),

    /// MuSig public nonce with the corresponding [`BitcoinTxid`]
    V0PubNonce(BitcoinTxid),
}

impl TryFrom<Vec<u8>> for Scope {
    type Error = io::Error;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        let result = borsh::from_slice(value.as_ref())?;
        Ok(result)
    }
}

impl TryInto<Vec<u8>> for Scope {
    type Error = io::Error;

    fn try_into(self) -> Result<Vec<u8>, Self::Error> {
        borsh::to_vec(&self).map_err(|_| io::Error::other("Serialization error"))
    }
}

impl TryFrom<&[u8]> for Scope {
    type Error = io::Error;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        let result = borsh::from_slice(value)?;
        Ok(result)
    }
}

impl TryFrom<Box<[u8]>> for Scope {
    type Error = io::Error;

    fn try_from(value: Box<[u8]>) -> Result<Self, Self::Error> {
        let result = borsh::from_slice(value.as_ref())?;
        Ok(result)
    }
}

impl TryInto<Box<[u8]>> for Scope {
    type Error = io::Error;

    fn try_into(self) -> Result<Box<[u8]>, Self::Error> {
        let serialized_vec =
            borsh::to_vec(&self).map_err(|_| io::Error::other("Serialization error"))?;
        Ok(serialized_vec.into_boxed_slice()) // Convert Vec<u8> to Box<[u8]>
    }
}

/// ID of a [``BridgeMessage``] computed from the sender ID, scope, and payload.
#[derive(
    Copy, Clone, Hash, Eq, PartialEq, Ord, PartialOrd, Arbitrary, BorshDeserialize, BorshSerialize,
)]
pub struct BridgeMsgId(Buf32);

impl BridgeMsgId {
    pub fn inner(&self) -> &Buf32 {
        &self.0
    }

    pub fn into_inner(self) -> Buf32 {
        self.0
    }
}

impl From<Buf32> for BridgeMsgId {
    fn from(value: Buf32) -> Self {
        Self(value)
    }
}

impl From<BridgeMsgId> for Buf32 {
    fn from(value: BridgeMsgId) -> Self {
        value.0
    }
}

impl AsRef<[u8; 32]> for BridgeMsgId {
    fn as_ref(&self) -> &[u8; 32] {
        self.0.as_ref()
    }
}

impl fmt::Debug for BridgeMsgId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl fmt::Display for BridgeMsgId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

#[cfg(test)]
mod tests {
    use strata_test_utils::ArbitraryGenerator;

    use super::{BridgeMessage, Scope};
    use crate::{
        bridge::{Musig2PartialSig, Musig2PubNonce},
        buf::Buf64,
        l1::BitcoinTxid,
    };

    fn get_arb_bridge_msg() -> BridgeMessage {
        let msg: BridgeMessage = ArbitraryGenerator::new().generate();
        msg
    }

    fn make_misc_bridge_msg() -> BridgeMessage {
        BridgeMessage {
            source_id: 1,
            sig: Buf64::from([0; 64]),
            scope: borsh::to_vec(&Scope::Misc).unwrap(),
            payload: vec![1, 2, 3, 4, 5],
        }
    }

    #[test]
    fn test_get_scope_raw() {
        let msg = make_misc_bridge_msg();
        assert_eq!(msg.scope(), vec![0])
    }

    #[test]
    fn test_scoped_nonce_bridge_msg_serde() {
        let txid: BitcoinTxid = ArbitraryGenerator::new().generate();
        let scope = Scope::V0PubNonce(txid);
        let nonce: Musig2PubNonce = ArbitraryGenerator::new().generate();

        let msg = make_nonce_bridge_msg(&scope, &nonce);
        let serialized_msg = borsh::to_vec(&msg).unwrap();

        let deserialized_msg = borsh::from_slice::<BridgeMessage>(&serialized_msg)
            .expect("should be able to deserialize BridgeMessage");

        let deserialized_scope = borsh::from_slice::<Scope>(&deserialized_msg.scope)
            .expect("should be able to deserialize Scope");

        assert_eq!(
            scope, deserialized_scope,
            "original and deserialized scopes must be the same"
        );

        let deserialized_payload = borsh::from_slice::<Musig2PubNonce>(&deserialized_msg.payload)
            .expect("should be able to deserialize Payload");

        assert_eq!(
            nonce, deserialized_payload,
            "original and deserialized payloads must be the same"
        );
    }

    fn make_nonce_bridge_msg(scope: &Scope, nonce: &Musig2PubNonce) -> BridgeMessage {
        BridgeMessage {
            source_id: 1,
            sig: Buf64::zero(),
            scope: borsh::to_vec(scope).unwrap(),
            payload: borsh::to_vec(nonce).unwrap(),
        }
    }

    #[test]
    fn test_scoped_sig_bridge_msg_serde() {
        let txid: BitcoinTxid = ArbitraryGenerator::new().generate();
        let scope = Scope::V0PubNonce(txid);
        let sig: Musig2PartialSig = ArbitraryGenerator::new().generate();

        let msg = make_sig_bridge_msg(&scope, &sig);
        let serialized_msg = borsh::to_vec(&msg).unwrap();

        let deserialized_msg = borsh::from_slice::<BridgeMessage>(&serialized_msg)
            .expect("should be able to deserialize BridgeMessage");

        let deserialized_scope = borsh::from_slice::<Scope>(&deserialized_msg.scope)
            .expect("should be able to deserialize Scope");

        assert_eq!(
            scope, deserialized_scope,
            "original and deserialized scopes must be the same"
        );

        let deserialized_payload = borsh::from_slice::<Musig2PartialSig>(&deserialized_msg.payload)
            .expect("should be able to deserialize Payload");

        assert_eq!(
            sig, deserialized_payload,
            "original and deserialized payloads must be the same"
        );
    }

    fn make_sig_bridge_msg(scope: &Scope, sig: &Musig2PartialSig) -> BridgeMessage {
        BridgeMessage {
            source_id: 1,
            sig: Buf64::zero(),
            scope: borsh::to_vec(scope).unwrap(),
            payload: borsh::to_vec(sig).unwrap(),
        }
    }
}
