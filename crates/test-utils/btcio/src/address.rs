use bitcoin::{
    Address, Network, XOnlyPublicKey,
    secp256k1::{Keypair, PublicKey, Secp256k1},
};
use musig2::KeyAggContext;
use strata_crypto::EvenSecretKey;

/// Derive a P2TR address from a secret key.
///
/// # Returns
/// A tuple of (address, keypair, internal_key)
pub fn derive_p2tr_address(secret_key: &EvenSecretKey) -> (Address, Keypair, XOnlyPublicKey) {
    let secp = Secp256k1::new();
    let keypair = Keypair::from_secret_key(&secp, secret_key.as_ref());
    let (internal_key, _parity) = XOnlyPublicKey::from_keypair(&keypair);
    let p2tr_address = Address::p2tr(&secp, internal_key, None, Network::Regtest);
    (p2tr_address, keypair, internal_key)
}

/// Derive a MuSig2 aggregated P2TR address from multiple secret keys.
///
/// # Returns
/// A tuple of (address, aggregated_internal_key)
pub fn derive_musig2_p2tr_address(
    secret_keys: &[EvenSecretKey],
) -> anyhow::Result<(Address, XOnlyPublicKey)> {
    if secret_keys.is_empty() {
        return Err(anyhow::anyhow!("At least one secret key is required"));
    }

    let secp = Secp256k1::new();

    // Extract public keys for MuSig2 aggregation
    // We convert secret keys directly to PublicKey to preserve parity
    let pubkeys: Vec<PublicKey> = secret_keys
        .iter()
        .map(|sk| PublicKey::from_secret_key(&secp, sk))
        .collect();

    // Create MuSig2 key aggregation context (untweaked)
    let key_agg_ctx = KeyAggContext::new(pubkeys)?;
    let aggregated_pubkey_untweaked: PublicKey = key_agg_ctx.aggregated_pubkey_untweaked();
    let aggregated_internal_key = aggregated_pubkey_untweaked.x_only_public_key().0;

    // Create P2TR address from the aggregated key
    let p2tr_address = Address::p2tr(&secp, aggregated_internal_key, None, Network::Regtest);

    Ok((p2tr_address, aggregated_internal_key))
}
