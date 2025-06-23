//! Logic to check block credentials.
use std::ops::Deref;

use secp256k1::{
    schnorr::Signature, Keypair, Message, Parity, SecretKey, XOnlyPublicKey, SECP256K1,
};

use crate::buf::{Buf32, Buf64};

#[cfg(feature = "rand")]
pub fn sign_schnorr_sig(msg: &Buf32, sk: &Buf32) -> Buf64 {
    let sk = SecretKey::from_slice(sk.as_ref()).expect("Invalid private key");
    let kp = Keypair::from_secret_key(SECP256K1, &sk);
    let msg = Message::from_digest_slice(msg.as_ref()).expect("Invalid message hash");
    let sig = SECP256K1.sign_schnorr(&msg, &kp);
    Buf64::from(sig.serialize())
}

#[cfg(not(target_os = "zkvm"))]
pub fn verify_schnorr_sig(sig: &Buf64, msg: &Buf32, pk: &Buf32) -> bool {
    let msg = match Message::from_digest_slice(msg.as_ref()) {
        Ok(msg) => msg,
        Err(_) => return false,
    };

    let pk = match XOnlyPublicKey::from_slice(pk.as_ref()) {
        Ok(pk) => pk,
        Err(_) => return false,
    };

    let sig = match Signature::from_slice(sig.0.as_ref()) {
        Ok(sig) => sig,
        Err(_) => return false,
    };

    sig.verify(&msg, &pk).is_ok()
}

#[cfg(target_os = "zkvm")]
pub fn verify_schnorr_sig(sig: &Buf64, msg: &Buf32, pk: &Buf32) -> bool {
    use k256::schnorr::{signature::hazmat::PrehashVerifier, Signature, VerifyingKey};
    let sig = match Signature::try_from(sig.as_slice()) {
        Ok(sig) => sig,
        Err(_) => return false,
    };

    let vk = match VerifyingKey::from_bytes(pk.as_slice()) {
        Ok(vk) => vk,
        Err(_) => return false,
    };

    vk.verify_prehash(msg.as_slice(), &sig).is_ok()
}

/// A secret key that is guaranteed to have a even x-only public key
#[derive(Debug)]
pub struct EvenSecretKey(SecretKey);

impl Deref for EvenSecretKey {
    type Target = SecretKey;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<SecretKey> for EvenSecretKey {
    fn as_ref(&self) -> &SecretKey {
        &self.0
    }
}

impl From<SecretKey> for EvenSecretKey {
    fn from(value: SecretKey) -> Self {
        match value.x_only_public_key(SECP256K1).1 == Parity::Odd {
            true => Self(value.negate()),
            false => Self(value),
        }
    }
}

#[cfg(test)]
mod tests {
    use rand::{rngs::OsRng, Rng};
    use secp256k1::{SecretKey, SECP256K1};

    use super::{sign_schnorr_sig, verify_schnorr_sig};
    use crate::buf::Buf32;

    #[test]
    fn test_schnorr_signature_pass() {
        let msg: [u8; 32] = [(); 32].map(|_| OsRng.gen());

        let mut mod_msg = msg;
        mod_msg.swap(1, 2);
        let msg = Buf32::from(msg);
        let mod_msg = Buf32::from(mod_msg);

        let sk = SecretKey::new(&mut OsRng);
        let (pk, _) = sk.x_only_public_key(SECP256K1);

        let sk = Buf32::from(*sk.as_ref());
        let pk = Buf32::from(pk.serialize());

        let sig = sign_schnorr_sig(&msg, &sk);
        assert!(verify_schnorr_sig(&sig, &msg, &pk));

        assert!(!verify_schnorr_sig(&sig, &mod_msg, &pk));

        let sig = sign_schnorr_sig(&mod_msg, &sk);
        let res = verify_schnorr_sig(&sig, &mod_msg, &pk);
        assert!(res);
    }
}
