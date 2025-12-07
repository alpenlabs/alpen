use std::env;

use bitcoin::{
    absolute,
    absolute::LockTime,
    block::Header,
    secp256k1::{schnorr::Signature, Keypair, Message, PublicKey, Secp256k1},
    sighash::{Prevouts, SighashCache},
    taproot::TapTweakHash,
    transaction::Version,
    Address, Amount, BlockHash, Network, OutPoint, ScriptBuf, Sequence, TapSighashType,
    Transaction, TxIn, TxOut, Txid, Witness, XOnlyPublicKey,
};
use bitcoind_async_client::{
    traits::{Reader, Wallet},
    Client,
};
use corepc_node::Node;
use musig2::KeyAggContext;
use strata_btc_types::GenesisL1View;
use strata_crypto::{
    test_utils::schnorr::{create_musig2_signature, Musig2Tweak},
    EvenSecretKey,
};
use strata_identifiers::{L1BlockCommitment, L1BlockId};

/// Get the authentication credentials for a given `bitcoind` instance.
fn get_auth(bitcoind: &Node) -> (String, String) {
    let params = &bitcoind.params;
    let cookie_values = params.get_cookie_values().unwrap().unwrap();
    (cookie_values.user, cookie_values.password)
}

pub fn get_bitcoind_and_client() -> (Node, Client) {
    // setting the ENV variable `BITCOIN_XPRIV_RETRIEVABLE` to retrieve the xpriv
    // SAFETY: This is a test environment and we control the execution flow.
    unsafe {
        env::set_var("BITCOIN_XPRIV_RETRIEVABLE", "true");
    }
    let bitcoind = Node::new("bitcoind").unwrap();
    let url = bitcoind.rpc_url();
    let (user, password) = get_auth(&bitcoind);
    let client = Client::new(url, user, password, None, None).unwrap();
    (bitcoind, client)
}

/// Mine a number of blocks of a given size `count`, which may be specified to a given coinbase
/// `address`.
pub async fn mine_blocks(
    bitcoind: &Node,
    client: &Client,
    count: usize,
    address: Option<Address>,
) -> anyhow::Result<Vec<BlockHash>> {
    let coinbase_address = match address {
        Some(address) => address,
        None => client.get_new_address().await?,
    };
    // Use sync client from corepc-node for mining as it is reliable
    let block_hashes = bitcoind
        .client
        .generate_to_address(count as _, &coinbase_address)?
        .0
        .iter()
        .map(|hash: &String| hash.parse::<BlockHash>())
        .collect::<Result<Vec<_>, _>>()?;
    Ok(block_hashes)
}

/// Helper to construct `GenesisL1View` from a block hash using the client.
pub async fn get_genesis_l1_view(
    client: &Client,
    hash: &BlockHash,
) -> anyhow::Result<GenesisL1View> {
    let header: Header = client.get_block_header(hash).await?;
    let height = client.get_block_height(hash).await?;

    // Construct L1BlockCommitment
    let blkid: L1BlockId = header.block_hash().into();
    let blk_commitment = L1BlockCommitment::new(
        absolute::Height::from_consensus(height as u32).expect("Height u32 overflow"),
        blkid,
    );

    // Create dummy/default values for other fields
    let next_target = header.bits.to_consensus();
    let epoch_start_timestamp = header.time;
    let last_11_timestamps = [header.time - 1; 11]; // simplified: ensure median < tip time by making history older

    Ok(GenesisL1View {
        blk: blk_commitment,
        next_target,
        epoch_start_timestamp,
        last_11_timestamps, // simplified: ensure median < tip time by making history older
    })
}

/// Derive a P2TR address from a secret key.
fn derive_p2tr_address(secret_key: &EvenSecretKey) -> (Address, Keypair, XOnlyPublicKey) {
    let secp = Secp256k1::new();
    let keypair = Keypair::from_secret_key(&secp, secret_key.as_ref());
    let (internal_key, _parity) = XOnlyPublicKey::from_keypair(&keypair);
    let p2tr_address = Address::p2tr(&secp, internal_key, None, Network::Regtest);
    (p2tr_address, keypair, internal_key)
}

/// Create and confirm a funding UTXO locked to the given address.
///
/// # Returns
/// A tuple of (funding_txid, prev_vout, prev_output)
async fn create_funding_utxo(
    bitcoind: &Node,
    client: &Client,
    address: &Address,
    amount: Amount,
) -> anyhow::Result<(Txid, u32, TxOut)> {
    // Fund the address using the wallet
    let funding_txid_str = bitcoind
        .client
        .send_to_address(address, amount)?
        .0
        .to_string();
    let funding_txid: Txid = funding_txid_str.parse()?;

    // Get the funding transaction BEFORE mining (while it's still in mempool)
    let funding_tx_result = client
        .get_raw_transaction_verbosity_zero(&funding_txid)
        .await?;
    let funding_tx = funding_tx_result
        .transaction()
        .map_err(|e| anyhow::anyhow!("Failed to decode funding transaction: {}", e))?;

    // Find the output that pays to our address
    let (prev_vout, prev_output) = funding_tx
        .output
        .iter()
        .enumerate()
        .find(|(_, output)| output.script_pubkey == address.script_pubkey())
        .map(|(idx, output)| (idx as u32, output.clone()))
        .ok_or_else(|| anyhow::anyhow!("Could not find output in funding transaction"))?;

    Ok((funding_txid, prev_vout, prev_output))
}

/// Sign a transaction with a taproot key-spend signature.
fn sign_taproot_transaction(
    tx: &Transaction,
    keypair: &Keypair,
    internal_key: &XOnlyPublicKey,
    prev_output: &TxOut,
) -> anyhow::Result<Signature> {
    let secp = Secp256k1::new();

    // Apply BIP341 taproot tweak
    let tweak = TapTweakHash::from_key_and_tweak(*internal_key, None);
    let tweaked_keypair = keypair.add_xonly_tweak(&secp, &tweak.to_scalar())?;

    let prevouts = vec![prev_output.clone()];
    let prevouts_ref = Prevouts::All(&prevouts);
    let mut sighash_cache = SighashCache::new(tx);
    let sighash = sighash_cache.taproot_key_spend_signature_hash(
        0,
        &prevouts_ref,
        TapSighashType::Default,
    )?;

    let msg = Message::from_digest_slice(sighash.as_ref())?;
    let signature = secp.sign_schnorr(&msg, &tweaked_keypair);

    Ok(signature)
}

/// Helper to sign and broadcast a transaction using a specific private key.
///
/// This function creates funding UTXOs locked to the P2TR address derived from the private key,
/// then creates a transaction spending from those UTXOs to the specified outputs.
///
/// The function automatically calculates the required funding amount based on the transaction
/// outputs and estimated fees, then adjusts the transaction to pay the appropriate fee.
///
/// # Arguments
/// * `bitcoind` - The bitcoind node
/// * `client` - The RPC client
/// * `secret_key` - The private key to sign with
/// * `mut tx` - The transaction to sign and broadcast (inputs will be added, fee will be
///   calculated)
///
/// # Returns
/// The txid of the signed and broadcast transaction
pub async fn submit_transaction_with_key(
    bitcoind: &Node,
    client: &Client,
    secret_key: &EvenSecretKey,
    mut tx: Transaction,
) -> anyhow::Result<Txid> {
    if tx.output.is_empty() {
        return Err(anyhow::anyhow!("Transaction must have at least one output"));
    }

    // Derive P2TR address from secret key
    let (p2tr_address, keypair, internal_key) = derive_p2tr_address(secret_key);

    // Calculate funding amount
    let total_output_value: u64 = tx.output.iter().map(|out| out.value.to_sat()).sum();
    let estimated_fee = 1000u64; // Conservative estimate in sats
    let funding_amount = Amount::from_sat(total_output_value + estimated_fee);

    // Create and confirm funding UTXO
    let (funding_txid, prev_vout, prev_output) =
        create_funding_utxo(bitcoind, client, &p2tr_address, funding_amount).await?;

    // Add input to the transaction
    tx.input = vec![TxIn {
        previous_output: OutPoint::new(funding_txid, prev_vout),
        script_sig: ScriptBuf::default(),
        sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
        witness: Witness::new(),
    }];

    // Ensure transaction has proper version and locktime
    tx.version = Version::TWO;
    tx.lock_time = LockTime::ZERO;

    // Sign the transaction
    let signature = sign_taproot_transaction(&tx, &keypair, &internal_key, &prev_output)?;

    // Add the signature to the witness (Taproot key-spend signatures are 64 bytes, no sighash type
    // appended for Default)
    tx.input[0].witness.push(signature.as_ref());

    // Broadcast the transaction
    let txid_wrapper = bitcoind.client.send_raw_transaction(&tx)?;
    let core_txid = txid_wrapper.0;
    let txid_str = core_txid.to_string();
    let txid: Txid = txid_str.parse()?;

    Ok(txid)
}

/// Derive a MuSig2 aggregated P2TR address from multiple secret keys.
///
/// # Returns
/// A tuple of (address, aggregated_internal_key)
fn derive_musig2_p2tr_address(
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

/// Sign a transaction with MuSig2 aggregated signature.
///
/// # Returns
/// The aggregated Schnorr signature
fn sign_musig2_transaction(
    tx: &Transaction,
    secret_keys: &[EvenSecretKey],
    _internal_key: &XOnlyPublicKey,
    prev_output: &TxOut,
) -> anyhow::Result<Signature> {
    // Calculate sighash
    let prevouts = vec![prev_output.clone()];
    let prevouts_ref = Prevouts::All(&prevouts);
    let mut sighash_cache = SighashCache::new(tx);
    let sighash = sighash_cache.taproot_key_spend_signature_hash(
        0,
        &prevouts_ref,
        TapSighashType::Default,
    )?;

    // Taproot key-path spend without a script tree uses the standard tweak with an empty merkle
    // root. Musig2 helper applies that tweak when using the TaprootKeySpend variant.
    let sighash_bytes: &[u8; 32] = sighash.as_ref();
    let compact_sig =
        create_musig2_signature(secret_keys, sighash_bytes, Musig2Tweak::TaprootKeySpend);

    // Convert CompactSignature to bitcoin::secp256k1::schnorr::Signature
    let sig = Signature::from_slice(&compact_sig.serialize())?;

    Ok(sig)
}

/// Helper to sign and broadcast a transaction using multiple secret keys with MuSig2 aggregation.
///
/// This function creates funding UTXOs locked to a P2TR address derived from MuSig2 aggregated
/// public keys, then creates a transaction spending from those UTXOs to the specified outputs
/// using MuSig2 signature aggregation.
///
/// # Arguments
/// * `bitcoind` - The bitcoind node
/// * `client` - The RPC client
/// * `secret_keys` - Slice of secret keys to aggregate for signing
/// * `mut tx` - The transaction to sign and broadcast (inputs will be added, fee will be
///   calculated)
///
/// # Returns
/// The txid of the signed and broadcast transaction
pub async fn submit_transaction_with_keys(
    bitcoind: &Node,
    client: &Client,
    secret_keys: &[EvenSecretKey],
    mut tx: Transaction,
) -> anyhow::Result<Txid> {
    if tx.output.is_empty() {
        return Err(anyhow::anyhow!("Transaction must have at least one output"));
    }

    if secret_keys.is_empty() {
        return Err(anyhow::anyhow!("At least one secret key is required"));
    }

    // Derive MuSig2 aggregated P2TR address
    let (p2tr_address, aggregated_internal_key) = derive_musig2_p2tr_address(secret_keys)?;

    // Calculate funding amount
    let total_output_value: u64 = tx.output.iter().map(|out| out.value.to_sat()).sum();
    let estimated_fee = 1000u64; // Conservative estimate in sats
    let funding_amount = Amount::from_sat(total_output_value + estimated_fee);

    // Create and confirm funding UTXO
    let (funding_txid, prev_vout, prev_output) =
        create_funding_utxo(bitcoind, client, &p2tr_address, funding_amount).await?;

    // Add input to the transaction
    tx.input = vec![TxIn {
        previous_output: OutPoint::new(funding_txid, prev_vout),
        script_sig: ScriptBuf::default(),
        sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
        witness: Witness::new(),
    }];

    // Ensure transaction has proper version and locktime
    tx.version = Version::TWO;
    tx.lock_time = LockTime::ZERO;

    // Sign the transaction with MuSig2
    let signature =
        sign_musig2_transaction(&tx, secret_keys, &aggregated_internal_key, &prev_output)?;

    // Add the aggregated signature to the witness
    tx.input[0].witness.push(signature.as_ref());

    // Broadcast the transaction
    let txid_wrapper = bitcoind.client.send_raw_transaction(&tx)?;
    let core_txid = txid_wrapper.0;
    let txid_str = core_txid.to_string();
    let txid: Txid = txid_str.parse()?;

    Ok(txid)
}
