use std::env;

use bitcoin::{
    absolute,
    block::Header,
    consensus::encode,
    secp256k1::{Keypair, Message, Secp256k1},
    sighash::{Prevouts, SighashCache},
    Address, Amount, TapSighashType, Transaction, Txid,
};
use bitcoind_async_client::{
    traits::{Reader, Wallet},
    Client,
};
use corepc_node::Node;
use strata_btc_types::GenesisL1View;
use strata_crypto::EvenSecretKey;
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
) -> anyhow::Result<Vec<bitcoin::BlockHash>> {
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
        .map(|hash: &String| hash.parse::<bitcoin::BlockHash>())
        .collect::<Result<Vec<_>, _>>()?;
    Ok(block_hashes)
}

/// Helper to construct `GenesisL1View` from a block hash using the client.
pub async fn get_genesis_l1_view(
    client: &Client,
    hash: &bitcoin::BlockHash,
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

/// Helper to fund, sign, and broadcast a transaction.
pub async fn submit_transaction(
    bitcoind: &Node,
    client: &Client,
    mut tx: Transaction,
) -> anyhow::Result<Txid> {
    // 1. Fund
    let funded_result = bitcoind.client.fund_raw_transaction(&tx)?;
    let funded_tx_bytes = hex::decode(&funded_result.hex)?;
    tx = encode::deserialize(&funded_tx_bytes)?;

    // 2. Sign
    let signed_result = bitcoind.client.sign_raw_transaction_with_wallet(&tx)?;
    if !signed_result.complete {
        return Err(anyhow::anyhow!("Failed to sign transaction completely"));
    }
    let signed_tx_bytes = hex::decode(&signed_result.hex)?;
    tx = encode::deserialize(&signed_tx_bytes)?;

    // 3. Broadcast
    // returns SendRawTransaction newtype wrapper around Txid
    let txid_wrapper = bitcoind.client.send_raw_transaction(&tx)?;
    let core_txid = txid_wrapper.0; // Extract inner Txid
    let txid_str = core_txid.to_string();
    let txid: Txid = txid_str.parse()?;

    // 4. Mine a block to confirm
    let _ = mine_blocks(bitcoind, client, 1, None).await?;

    Ok(txid)
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
) -> anyhow::Result<bitcoin::Txid> {
    use bitcoin::{
        absolute::LockTime, transaction::Version, OutPoint, ScriptBuf, Sequence, TxIn, Witness,
        XOnlyPublicKey,
    };

    if tx.output.is_empty() {
        return Err(anyhow::anyhow!("Transaction must have at least one output"));
    }

    let secp = Secp256k1::new();

    // Get the keypair from the secret key
    let keypair = Keypair::from_secret_key(&secp, secret_key.as_ref());
    let (internal_key, _parity) = XOnlyPublicKey::from_keypair(&keypair);

    // Create a P2TR address from the internal key (no script tree for key-path spending)
    let p2tr_address = bitcoin::Address::p2tr(&secp, internal_key, None, bitcoin::Network::Regtest);

    // Calculate total output value
    let total_output_value: u64 = tx.output.iter().map(|out| out.value.to_sat()).sum();

    // Estimate fee: P2TR input (1 input, ~57 vbytes) + outputs + overhead
    // Using conservative estimate: ~150 vbytes * 1 sat/vbyte = 150 sats
    let estimated_fee = 1000u64; // Conservative estimate in sats
    let funding_amount = Amount::from_sat(total_output_value + estimated_fee);

    // Fund the P2TR address using the wallet
    let funding_txid_str = bitcoind
        .client
        .send_to_address(&p2tr_address, funding_amount)?
        .0
        .to_string();
    let funding_txid: bitcoin::Txid = funding_txid_str.parse()?;

    // Get the funding transaction BEFORE mining (while it's still in mempool)
    let funding_tx_result = client
        .get_raw_transaction_verbosity_zero(&funding_txid)
        .await?;
    let funding_tx = funding_tx_result
        .transaction()
        .map_err(|e| anyhow::anyhow!("Failed to decode funding transaction: {}", e))?;

    // Mine a block to confirm the funding transaction
    let _ = mine_blocks(bitcoind, client, 1, None).await?;

    // Find the output that pays to our P2TR address
    let (prev_vout, prev_output) = funding_tx
        .output
        .iter()
        .enumerate()
        .find(|(_, output)| output.script_pubkey == p2tr_address.script_pubkey())
        .map(|(idx, output)| (idx as u32, output.clone()))
        .ok_or_else(|| anyhow::anyhow!("Could not find P2TR output in funding transaction"))?;

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

    // Sign the transaction with the tweaked key
    // When using Address::p2tr() with None for merkle_root, it applies BIP341 taproot tweak
    use bitcoin::taproot::TapTweakHash;
    let tweak = TapTweakHash::from_key_and_tweak(internal_key, None);
    let tweaked_keypair = keypair.add_xonly_tweak(&secp, &tweak.to_scalar())?;

    let prevouts = vec![prev_output];
    let prevouts_ref = Prevouts::All(&prevouts);
    let mut sighash_cache = SighashCache::new(&tx);
    let sighash = sighash_cache.taproot_key_spend_signature_hash(
        0,
        &prevouts_ref,
        TapSighashType::Default,
    )?;

    let msg = Message::from_digest_slice(sighash.as_ref())?;
    let signature = secp.sign_schnorr(&msg, &tweaked_keypair);

    // Add the signature to the witness (Taproot key-spend signatures are 64 bytes, no sighash type
    // appended for Default)
    tx.input[0].witness.push(signature.as_ref());

    // Broadcast the transaction
    let txid_wrapper = bitcoind.client.send_raw_transaction(&tx)?;
    let core_txid = txid_wrapper.0;
    let txid_str = core_txid.to_string();
    let txid: Txid = txid_str.parse()?;

    // Mine a block to confirm
    let _ = mine_blocks(bitcoind, client, 1, None).await?;

    Ok(txid)
}
