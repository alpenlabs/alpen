//! Schnorr signature signing and verification.

#[cfg(not(target_os = "zkvm"))]
use std::ops::Deref;

#[cfg(not(target_os = "zkvm"))]
use borsh::{BorshDeserialize, BorshSerialize};
#[cfg(all(not(target_os = "zkvm"), feature = "serde"))]
use hex;
#[cfg(not(target_os = "zkvm"))]
use secp256k1::{Keypair, Message, Parity, PublicKey, SecretKey, XOnlyPublicKey, SECP256K1};
#[cfg(all(not(target_os = "zkvm"), feature = "serde"))]
use serde::de::Error as DeError;
#[cfg(all(not(target_os = "zkvm"), feature = "serde"))]
use serde::{Deserialize, Serialize};
use strata_identifiers::{Buf32, Buf64};

/// Sign a message with a Schnorr signature.
#[cfg(not(target_os = "zkvm"))]
pub fn sign_schnorr_sig(msg: &Buf32, sk: &Buf32) -> Buf64 {
    let sk = SecretKey::from_slice(sk.as_ref()).expect("Invalid private key");
    let kp = Keypair::from_secret_key(SECP256K1, &sk);
    let msg = Message::from_digest_slice(msg.as_ref()).expect("Invalid message hash");
    let sig = SECP256K1.sign_schnorr_no_aux_rand(&msg, &kp);
    Buf64::from(sig.serialize())
}

/// Sign a message with a Schnorr signature (zkvm version).
#[cfg(target_os = "zkvm")]
pub fn sign_schnorr_sig(_msg: &Buf32, _sk: &Buf32) -> Buf64 {
    // Signing is not typically done in zkvm
    unimplemented!("Schnorr signing not available in zkvm")
}

/// Verify a Schnorr signature (non-zkvm version using secp256k1).
#[cfg(not(target_os = "zkvm"))]
pub fn verify_schnorr_sig(sig: &Buf64, msg: &Buf32, pk: &Buf32) -> bool {
    use secp256k1::schnorr::Signature;

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

/// Verify a Schnorr signature (zkvm version using k256).
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
#[cfg(not(target_os = "zkvm"))]
#[derive(Debug, Clone, Copy)]
pub struct EvenSecretKey(SecretKey);

#[cfg(not(target_os = "zkvm"))]
impl Deref for EvenSecretKey {
    type Target = SecretKey;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(not(target_os = "zkvm"))]
impl AsRef<SecretKey> for EvenSecretKey {
    fn as_ref(&self) -> &SecretKey {
        &self.0
    }
}

#[cfg(not(target_os = "zkvm"))]
impl From<SecretKey> for EvenSecretKey {
    fn from(value: SecretKey) -> Self {
        match value.x_only_public_key(SECP256K1).1 == Parity::Odd {
            true => Self(value.negate()),
            false => Self(value),
        }
    }
}

#[cfg(not(target_os = "zkvm"))]
impl From<EvenSecretKey> for SecretKey {
    fn from(value: EvenSecretKey) -> Self {
        value.0
    }
}

/// A public key with guaranteed even parity
#[cfg(not(target_os = "zkvm"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EvenPublicKey(PublicKey);

#[cfg(not(target_os = "zkvm"))]
impl Deref for EvenPublicKey {
    type Target = PublicKey;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(not(target_os = "zkvm"))]
impl AsRef<PublicKey> for EvenPublicKey {
    fn as_ref(&self) -> &PublicKey {
        &self.0
    }
}

#[cfg(not(target_os = "zkvm"))]
impl From<PublicKey> for EvenPublicKey {
    fn from(value: PublicKey) -> Self {
        match value.x_only_public_key().1 == Parity::Odd {
            true => Self(value.negate(SECP256K1)),
            false => Self(value),
        }
    }
}

#[cfg(not(target_os = "zkvm"))]
impl From<EvenPublicKey> for PublicKey {
    fn from(value: EvenPublicKey) -> Self {
        value.0
    }
}

#[cfg(not(target_os = "zkvm"))]
impl From<EvenPublicKey> for XOnlyPublicKey {
    fn from(value: EvenPublicKey) -> Self {
        value.0.x_only_public_key().0
    }
}

#[cfg(not(target_os = "zkvm"))]
impl From<XOnlyPublicKey> for EvenPublicKey {
    fn from(value: XOnlyPublicKey) -> Self {
        // Convert x-only to full public key with even parity
        PublicKey::from_x_only_public_key(value, Parity::Even).into()
    }
}

#[cfg(not(target_os = "zkvm"))]
impl From<EvenPublicKey> for Buf32 {
    fn from(value: EvenPublicKey) -> Self {
        Buf32::from(value.0.x_only_public_key().0.serialize())
    }
}

#[cfg(not(target_os = "zkvm"))]
impl TryFrom<Buf32> for EvenPublicKey {
    type Error = secp256k1::Error;

    fn try_from(value: Buf32) -> Result<Self, Self::Error> {
        let x_only = XOnlyPublicKey::from_slice(value.as_ref())?;
        Ok(PublicKey::from_x_only_public_key(x_only, Parity::Even).into())
    }
}

#[cfg(not(target_os = "zkvm"))]
impl BorshSerialize for EvenPublicKey {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        let x_only = self.0.x_only_public_key().0;
        BorshSerialize::serialize(&Buf32::from(x_only.serialize()), writer)
    }
}

#[cfg(not(target_os = "zkvm"))]
impl BorshDeserialize for EvenPublicKey {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let buf = Buf32::deserialize_reader(reader)?;
        let x_only = XOnlyPublicKey::from_slice(buf.as_ref())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(PublicKey::from_x_only_public_key(x_only, Parity::Even).into())
    }
}

#[cfg(all(not(target_os = "zkvm"), feature = "serde"))]
impl Serialize for EvenPublicKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Serialize as full compressed public key (33 bytes with 0x02 prefix for even parity)
        let compressed = self.0.serialize();
        let hex_string = hex::encode(compressed);
        serializer.serialize_str(&hex_string)
    }
}

#[cfg(all(not(target_os = "zkvm"), feature = "serde"))]
impl<'de> Deserialize<'de> for EvenPublicKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let hex_string: String = Deserialize::deserialize(deserializer)?;
        let bytes = hex::decode(&hex_string).map_err(DeError::custom)?;
        let pk = PublicKey::from_slice(&bytes).map_err(DeError::custom)?;
        // Verify it's even parity
        if pk.x_only_public_key().1 != Parity::Even {
            return Err(DeError::custom(
                "Expected even parity public key, got odd parity",
            ));
        }
        Ok(EvenPublicKey(pk))
    }
}

/// Ensures a keypair is even by checking the public key's parity and negating if odd.
#[cfg(not(target_os = "zkvm"))]
pub fn even_kp((sk, pk): (SecretKey, PublicKey)) -> (EvenSecretKey, EvenPublicKey) {
    match (sk, pk) {
        (sk, pk) if pk.x_only_public_key().1 == Parity::Odd => (
            EvenSecretKey(sk.negate()),
            EvenPublicKey(pk.negate(SECP256K1)),
        ),
        (sk, pk) => (EvenSecretKey(sk), EvenPublicKey(pk)),
    }
}

#[cfg(test)]
mod tests {
    use rand::{rngs::OsRng, Rng};
    use strata_identifiers::Buf32;

    use super::{sign_schnorr_sig, verify_schnorr_sig};

    #[test]
    #[cfg(not(target_os = "zkvm"))]
    fn test_schnorr_signature_pass() {
        use secp256k1::{SecretKey, SECP256K1};

        let msg: [u8; 32] = [(); 32].map(|_| OsRng.gen());

        let mut mod_msg = msg;
        mod_msg.swap(1, 2);
        let msg = Buf32::from(msg);
        let mod_msg = Buf32::from(mod_msg);

        let mut sk_bytes = [0u8; 32];
        OsRng.fill(&mut sk_bytes);
        let sk = SecretKey::from_slice(&sk_bytes).expect("valid key");
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
