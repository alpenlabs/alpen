use std::io;

use bitcoin::secp256k1::{
    schnorr::Signature, Keypair, Message, SecretKey, XOnlyPublicKey, SECP256K1,
};
use borsh::BorshSerialize;
use rand::rngs::OsRng;
use thiserror::Error;

use super::types::{BridgeMessage, Scope};
use crate::{
    buf::{Buf32, Buf64},
    operator::OperatorKeyProvider,
};

/// Contains data needed to construct [`BridgeMessage`]s.
#[derive(Debug, Clone)]
pub struct MessageSigner {
    operator_idx: u32,
    msg_signing_sk: Buf32,
}

impl MessageSigner {
    /// Creates a new [`MessageSigner`].
    ///
    /// # Notes
    ///
    /// In order to get a [`BridgeMessage`], call [`sign_raw`](Self::sign_raw)
    /// or [`sign_scope`](Self::sign_scope) on this [`MessageSigner`]
    /// depending on the use case.
    #[expect(clippy::missing_const_for_fn)]
    pub fn new(operator_idx: u32, msg_signing_sk: Buf32) -> Self {
        Self {
            operator_idx,
            msg_signing_sk,
        }
    }

    /// Gets the idx of the operator that we are using for signing messages.
    pub const fn operator_idx(&self) -> u32 {
        self.operator_idx
    }

    /// Gets the pubkey corresponding to the internal msg signing sk.
    pub fn get_pubkey(&self) -> Buf32 {
        compute_pubkey_for_privkey(&self.msg_signing_sk)
    }

    /// Signs a message using a raw scope and payload.
    pub fn sign_raw(&self, scope: Vec<u8>, payload: Vec<u8>) -> Result<BridgeMessage, io::Error> {
        let mut tmp_m = BridgeMessage {
            source_id: self.operator_idx,
            sig: Buf64::zero(),
            scope,
            payload,
        };

        let id: Buf32 = tmp_m.compute_id().into();
        // WARN: I don't know if a global context is safe here, maybe.
        let sig = sign_msg_hash(&self.msg_signing_sk, &id);
        tmp_m.sig = sig;

        Ok(tmp_m)
    }

    /// Signs a message with some particular typed scope.
    pub fn sign_scope<T: BorshSerialize>(
        &self,
        scope: &Scope,
        payload: &T,
    ) -> Result<BridgeMessage, io::Error> {
        let raw_scope = borsh::to_vec(scope)?;
        let payload: Vec<u8> = borsh::to_vec(&payload)?;
        self.sign_raw(raw_scope, payload)
    }
}

/// Computes the corresponding x-only pubkey as a buf32 for an sk.
#[cfg(feature = "std")]
pub fn compute_pubkey_for_privkey(sk: &Buf32) -> Buf32 {
    let kp = Keypair::from_seckey_slice(SECP256K1, sk.as_ref()).unwrap();
    let (xonly_pk, _) = kp.public_key().x_only_public_key();
    Buf32::from(xonly_pk.serialize())
}

/// Generates a signature for the message.
#[cfg(all(feature = "std", feature = "rand"))]
pub fn sign_msg_hash(sk: &Buf32, msg_hash: &Buf32) -> Buf64 {
    let keypair = Keypair::from_secret_key(SECP256K1, &SecretKey::from_slice(sk.as_ref()).unwrap());
    let msg = Message::from_digest(*msg_hash.as_ref());
    let sig = SECP256K1.sign_schnorr_with_rng(&msg, &keypair, &mut OsRng);

    Buf64::from(*sig.as_ref())
}

/// Returns if the signature is correct for the message.
#[cfg(feature = "std")]
pub fn verify_sig(pk: &Buf32, msg_hash: &Buf32, sig: &Buf64) -> bool {
    let pk = XOnlyPublicKey::from_slice(pk.as_ref()).unwrap();
    let msg = Message::from_digest(*msg_hash.as_ref());
    let sig = Signature::from_slice(sig.as_ref()).unwrap();

    sig.verify(&msg, &pk).is_ok()
}

#[derive(Debug, Error)]
pub enum VerifyError {
    #[error("invalid signature")]
    InvalidSig,

    #[error("unknown operator idx")]
    UnknownOperator,
}

/// Verifies a bridge message using the operator table working with and checks
/// if the operator exists and verifies the signature using their pubkeys.
pub fn verify_bridge_msg_sig(
    msg: &BridgeMessage,
    optbl: &impl OperatorKeyProvider,
) -> Result<(), VerifyError> {
    let op_signing_pk = optbl
        .get_operator_signing_pk(msg.source_id())
        .ok_or(VerifyError::UnknownOperator)?;

    let msg_hash = msg.compute_id().into_inner();
    if !verify_sig(&op_signing_pk, &msg_hash, msg.signature()) {
        return Err(VerifyError::InvalidSig);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use rand::rngs::OsRng;
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::{
        bridge::Musig2PubNonce, buf::Buf32, l1::BitcoinTxid, operator::StubOpKeyProv,
        relay::types::*,
    };

    #[test]
    fn test_sign_verify_raw() {
        let msg_hash = [
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
            25, 26, 27, 28, 29, 30, 31, 32,
        ];
        let msg_hash = Buf32::from(msg_hash);
        let sk = Buf32::from([3; 32]);
        let pk = compute_pubkey_for_privkey(&sk);

        let sig = sign_msg_hash(&sk, &msg_hash);
        assert!(verify_sig(&pk, &msg_hash, &sig));
    }

    #[test]
    fn test_sign_verify_msg_ok() {
        let sk = Buf32::from([1; 32]);

        let idx = 4;
        let signer = MessageSigner::new(idx, sk);
        let pk = signer.get_pubkey();

        let payload = vec![1, 2, 3, 4, 5];
        let m = signer.sign_scope(&Scope::Misc, &payload).unwrap();

        let stub_prov = StubOpKeyProv::new(idx, pk);
        assert!(verify_bridge_msg_sig(&m, &stub_prov).is_ok());
    }

    #[test]
    fn test_sign_verify_msg_fail() {
        let sk = Buf32::from([1; 32]);

        let idx = 4;
        let signer = MessageSigner::new(idx, sk);
        let pk = signer.get_pubkey();

        let payload = vec![1, 2, 3, 4, 5];
        let mut m = signer.sign_scope(&Scope::Misc, &payload).unwrap();
        m.sig = Buf64::zero();

        let stub_prov = StubOpKeyProv::new(idx, pk);
        assert!(verify_bridge_msg_sig(&m, &stub_prov).is_err());
    }

    #[test]
    fn test_message_signer_serde() {
        let mut generator = ArbitraryGenerator::new();
        let txid: BitcoinTxid = generator.generate();
        let scope = Scope::V0PubNonce(txid);
        let payload: Musig2PubNonce = generator.generate();
        let keypair = Keypair::new(SECP256K1, &mut OsRng);
        let msg_signer = MessageSigner::new(0, keypair.secret_key().into());

        let signed_message = msg_signer
            .sign_scope(&scope, &payload)
            .expect("scope signing should work");

        let serialized_msg = borsh::to_vec::<BridgeMessage>(&signed_message)
            .expect("BridgeMessage serialization should work");
        let deserialized_msg = borsh::from_slice::<BridgeMessage>(&serialized_msg)
            .expect("BridgeMessage deserialization should work");

        let deserialized_scope = borsh::from_slice::<Scope>(&deserialized_msg.scope)
            .expect("scope deserialization should work");

        assert_eq!(
            deserialized_scope, scope,
            "original and scope from signed message must match"
        );
    }

    // TODO add verify fail check
}
