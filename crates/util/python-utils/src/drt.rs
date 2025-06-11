use std::str::FromStr;

use bdk_wallet::{
    bitcoin::{
        consensus::encode::serialize, hashes::Hash, taproot::LeafVersion, Address, FeeRate,
        TapNodeHash, Transaction, XOnlyPublicKey,
    },
    miniscript::{miniscript::Tap, Miniscript},
    template::DescriptorTemplateOut,
    KeychainKind, TxOrdering, Wallet,
};
use pyo3::prelude::*;
use revm_primitives::alloy_primitives::Address as RethAddress;
use strata_primitives::constants::{RECOVER_DELAY, UNSPENDABLE_PUBLIC_KEY};

use crate::{
    constants::{BRIDGE_IN_AMOUNT, MAGIC_BYTES, NETWORK, XPRIV},
    error::Error,
    parse::{parse_address, parse_el_address, parse_xonly_pk},
    taproot::{bridge_wallet, new_bitcoind_client, sync_wallet, taproot_wallet, ExtractP2trPubkey},
};

/// Generates a deposit request transaction (DRT).
///
/// # Arguments
///
/// - `el_address`: Execution layer address of the account that will receive the funds.
/// - `musig_bridge_pk`: MuSig bridge X-only public key.
/// - `bitcoind_url`: URL of the `bitcoind` instance.
/// - `bitcoind_user`: Username for the `bitcoind` instance.
/// - `bitcoind_password`: Password for the `bitcoind` instance.
///
/// # Returns
///
/// A signed (with the `private_key`) and serialized transaction.
#[pyfunction]
pub(crate) fn deposit_request_transaction(
    el_address: String,
    musig_bridge_pk: String,
    bitcoind_url: String,
    bitcoind_user: String,
    bitcoind_password: String,
) -> PyResult<Vec<u8>> {
    let signed_tx = deposit_request_transaction_inner(
        el_address.as_str(),
        musig_bridge_pk.as_str(),
        bitcoind_url.as_str(),
        bitcoind_user.as_str(),
        bitcoind_password.as_str(),
    )?;
    let signed_tx = serialize(&signed_tx);
    Ok(signed_tx)
}

/// Generates a deposit request transaction (DRT).
///
/// # Arguments
///
/// - `el_address`: Execution layer address of the account that will receive the funds.
/// - `musig_bridge_pk`: MuSig bridge X-only public key.
/// - `bitcoind_url`: URL of the `bitcoind` instance.
/// - `bitcoind_user`: Username for the `bitcoind` instance.
/// - `bitcoind_password`: Password for the `bitcoind` instance.
///
/// # Returns
///
/// A signed (with the `private_key`) and serialized transaction.
fn deposit_request_transaction_inner(
    el_address: &str,
    musig_bridge_pk: &str,
    bitcoind_url: &str,
    bitcoind_user: &str,
    bitcoind_password: &str,
) -> Result<Transaction, Error> {
    // Parse stuff
    let el_address = parse_el_address(el_address)?;
    let musig_bridge_pk = parse_xonly_pk(musig_bridge_pk)?;

    // Instantiate the BitcoinD client
    let client = new_bitcoind_client(
        bitcoind_url,
        None,
        Some(bitcoind_user),
        Some(bitcoind_password),
    )?;

    // Get the address and the bridge descriptor
    let mut wallet = taproot_wallet()?;
    let recovery_address = wallet.reveal_next_address(KeychainKind::External).address;
    let (_, recovery_script_hash) = bridge_in_descriptor(musig_bridge_pk, recovery_address.clone())
        .expect("drt: invalid bridge in descriptor");

    let mut bridge_wallet = bridge_wallet(musig_bridge_pk, recovery_address)?;
    let bridge_in_address = bridge_wallet
        .reveal_next_address(KeychainKind::External)
        .address;

    // Magic bytes + TapNodeHash + Recovery Address
    const MBL: usize = MAGIC_BYTES.len();
    const TNHL: usize = TapNodeHash::LEN;
    let mut op_return_data = [0u8; MBL + TNHL + RethAddress::len_bytes()];
    op_return_data[..MBL].copy_from_slice(MAGIC_BYTES);
    op_return_data[MBL..MBL + TNHL]
        .copy_from_slice(recovery_script_hash.as_raw_hash().as_byte_array());
    op_return_data[MBL + TNHL..].copy_from_slice(el_address.as_slice());

    // For regtest 2 sat/vbyte is enough
    let fee_rate = FeeRate::from_sat_per_vb_unchecked(2);

    // Before signing the transaction, we need to sync the wallet with bitcoind
    sync_wallet(&mut wallet, &client)?;

    let mut psbt = {
        let mut builder = wallet.build_tx();
        // NOTE: the deposit won't be found by the sequencer if the order isn't correct.
        builder.ordering(TxOrdering::Untouched);
        builder.add_recipient(bridge_in_address.script_pubkey(), BRIDGE_IN_AMOUNT);
        builder.add_data(&op_return_data);
        builder.fee_rate(fee_rate);
        builder.finish().expect("drt: invalid psbt")
    };
    wallet.sign(&mut psbt, Default::default()).unwrap();

    let tx = psbt.extract_tx().expect("drt: invalid tx");
    Ok(tx)
}

/// Spends the take back script path of the deposit request transaction (DRT).
///
/// # Arguments
///
/// - `address_to_send`: Address to send the funds to.
/// - `musig_bridge_pk`: MuSig bridge X-only public key.
/// - `bitcoind_url`: URL of the `bitcoind` instance.
/// - `bitcoind_user`: Username for the `bitcoind` instance.
/// - `bitcoind_password`: Password for the `bitcoind` instance.
///
/// # Note
///
/// The take back script path is relative-timelocked to [`RECOVER_DELAY`] blocks.
///
/// # Returns
///
/// A signed (with the private key) and serialized transaction.
#[pyfunction]
pub(crate) fn take_back_transaction(
    address_to_send: String,
    musig_bridge_pk: String,
    bitcoind_url: String,
    bitcoind_user: String,
    bitcoind_password: String,
) -> PyResult<Vec<u8>> {
    let signed_tx = spend_recovery_path_inner(
        address_to_send.as_str(),
        musig_bridge_pk.as_str(),
        bitcoind_url.as_str(),
        bitcoind_user.as_str(),
        bitcoind_password.as_str(),
    )?;
    let signed_tx = serialize(&signed_tx);
    Ok(signed_tx)
}

/// Spends the take back script path of the deposit request transaction (DRT).
///
/// # Arguments
///
/// - `address_to_send`: Address to send the funds to.
/// - `musig_bridge_pk`: MuSig bridge X-only public key.
/// - `bitcoind_url`: URL of the `bitcoind` instance.
/// - `bitcoind_user`: Username for the `bitcoind` instance.
/// - `bitcoind_password`: Password for the `bitcoind` instance.
///
/// # Note
///
/// The take back script path is relative-timelocked to [`RECOVER_DELAY`] blocks.
///
/// # Returns
///
/// A signed (with the private key) and serialized transaction.
fn spend_recovery_path_inner(
    address_to_send: &str,
    musig_bridge_pk: &str,
    bitcoind_url: &str,
    bitcoind_user: &str,
    bitcoind_password: &str,
) -> Result<Transaction, Error> {
    // Parse stuff
    let address_to_send = parse_address(address_to_send)?;
    let musig_bridge_pk = parse_xonly_pk(musig_bridge_pk)?;

    // Get the recovery wallet
    let mut wallet = recovery_wallet(musig_bridge_pk)?;

    // For regtest 2 sat/vbyte is enough
    let fee_rate = FeeRate::from_sat_per_vb_unchecked(2);

    // Instantiate the BitcoinD client
    let client = new_bitcoind_client(
        bitcoind_url,
        None,
        Some(bitcoind_user),
        Some(bitcoind_password),
    )?;

    let external_policy = wallet
        .policies(KeychainKind::External)
        .expect("valid policy")
        .expect("valid policy");
    let root_id = external_policy.id;
    // child #2 is and_v(v:pk(xkey),older(1008))
    let path = vec![(root_id, vec![2])].into_iter().collect();

    // Before signing the transaction, we need to sync the wallet with bitcoind
    sync_wallet(&mut wallet, &client)?;

    // Spend the recovery path
    let mut psbt = {
        let mut builder = wallet.build_tx();
        builder.policy_path(path, KeychainKind::External);
        builder.drain_wallet();
        builder.drain_to(address_to_send.script_pubkey());
        builder.fee_rate(fee_rate);
        builder.finish().expect("valid psbt")
    };
    wallet.sign(&mut psbt, Default::default()).unwrap();

    let tx = psbt.extract_tx().expect("valid tx");
    Ok(tx)
}

/// The descriptor for the bridge-in transaction.
///
/// # Note
///
/// The descriptor is a Tapscript that enforces the following conditions:
///
/// - The funds can be spent by the bridge operator.
/// - The funds can be spent by the recovery address after a delay.
///
/// # Returns
///
/// The descriptor and the script hash for the recovery path.
pub(crate) fn bridge_in_descriptor(
    bridge_pubkey: XOnlyPublicKey,
    recovery_address: Address,
) -> Result<(DescriptorTemplateOut, TapNodeHash), Error> {
    let recovery_xonly_pubkey = recovery_address.extract_p2tr_pubkey()?;

    let desc = bdk_wallet::descriptor!(
        tr(UNSPENDABLE_PUBLIC_KEY, {
            pk(bridge_pubkey),
            and_v(v:pk(recovery_xonly_pubkey),older(RECOVER_DELAY))
        })
    )
    .expect("valid descriptor");

    // we have to do this to obtain the script hash
    // i have tried to extract it directly from the desc above
    // it is a massive pita
    let recovery_script = Miniscript::<XOnlyPublicKey, Tap>::from_str(&format!(
        "and_v(v:pk({recovery_xonly_pubkey}),older(1008))",
    ))
    .expect("valid recovery script")
    .encode();

    let recovery_script_hash = TapNodeHash::from_script(&recovery_script, LeafVersion::TapScript);

    Ok((desc, recovery_script_hash))
}

/// The descriptor for the take-back script path of the
/// deposit request transaction (DRT).
///
/// # Note
///
/// The descriptor is a Tapscript that enforces the following conditions:
///
/// - The funds can be spent by the bridge operator.
/// - The funds can be spent by the recovery address after a delay.
///
/// # Returns
///
/// The descriptor and the script hash for the recovery path.
pub(crate) fn take_back_descriptor(
    bridge_pubkey: XOnlyPublicKey,
) -> Result<DescriptorTemplateOut, Error> {
    let xkey = format!("{XPRIV}/86'/1'/0'/0/*");
    let desc = bdk_wallet::descriptor!(
        tr(UNSPENDABLE_PUBLIC_KEY, {
            pk(bridge_pubkey),
            and_v(v:pk(xkey.as_str()),older(RECOVER_DELAY))
        })
    )
    .expect("valid descriptor");

    Ok(desc)
}

/// The recovery wallet used for the take-back script path of the
/// deposit request transaction (DRT).
///
/// # Note
///
/// This uses the hardcoded [`XPRIV`] key.
pub(crate) fn recovery_wallet(bridge_pubkey: XOnlyPublicKey) -> Result<Wallet, Error> {
    let desc = take_back_descriptor(bridge_pubkey)?;
    Ok(Wallet::create_single(desc)
        .network(NETWORK)
        .create_wallet_no_persist()
        .map_err(|_| Error::Wallet))?
}

/// Gets a (receiving/external) address from the [`recovery_wallet`] at the given `index`.
#[pyfunction]
pub(crate) fn get_recovery_address(index: u32, musig_bridge_pk: String) -> PyResult<String> {
    let musig_bridge_pk = parse_xonly_pk(&musig_bridge_pk)?;
    let wallet = recovery_wallet(musig_bridge_pk)?;
    let address = wallet
        .peek_address(KeychainKind::External, index)
        .address
        .to_string();
    Ok(address)
}

/// Gets the balance for a specific [`Address`] from the taproot wallet.
///
/// # Returns
///
/// The balance in satoshis where 1 BTC = 100_000_000 satoshis.
#[pyfunction]
pub(crate) fn get_balance(
    address: String,
    bitcoind_url: String,
    bitcoind_user: String,
    bitcoind_password: String,
) -> PyResult<u64> {
    let balance = get_balance_inner(&address, &bitcoind_url, &bitcoind_user, &bitcoind_password)?;
    Ok(balance)
}

/// Gets the balance for a specific [`Address`] from the taproot wallet.
///
/// # Returns
///
/// The balance in satoshis where 1 BTC = 100_000_000 satoshis.
pub(crate) fn get_balance_inner(
    address: &str,
    bitcoind_url: &str,
    bitcoind_user: &str,
    bitcoind_password: &str,
) -> Result<u64, Error> {
    // Parse stuff
    let address = address
        .parse::<Address<_>>()
        .map_err(|_| Error::BitcoinAddress)?
        .assume_checked();

    // Get the wallet
    let mut wallet = taproot_wallet()?;

    // Instantiate the BitcoinD client
    let client = new_bitcoind_client(
        bitcoind_url,
        None,
        Some(bitcoind_user),
        Some(bitcoind_password),
    )?;
    sync_wallet(&mut wallet, &client)?;

    let balance = wallet
        .list_unspent()
        .filter_map(|utxo| {
            if utxo.txout.script_pubkey == address.script_pubkey() {
                Some(utxo.txout.value.to_sat())
            } else {
                None
            }
        })
        .sum();

    Ok(balance)
}

/// Gets the balance for a specific [`Address`] from the recovery wallet.
///
/// The recovery wallet is the wallet that is used to recover
/// the funds from an unprocessed deposit request transaction (DRT)
/// after the [`RECOVERY_DELAY`] has passed.
///
/// # Returns
///
/// The balance in satoshis where 1 BTC = 100_000_000 satoshis.
#[pyfunction]
pub(crate) fn get_balance_recovery(
    address: String,
    musig_bridge_pk: String,
    bitcoind_url: String,
    bitcoind_user: String,
    bitcoind_password: String,
) -> PyResult<u64> {
    let balance = get_balance_recovery_inner(
        &address,
        &musig_bridge_pk,
        &bitcoind_url,
        &bitcoind_user,
        &bitcoind_password,
    )?;
    Ok(balance)
}

/// Gets the balance for a specific [`Address`] from the recovery wallet.
///
/// The recovery wallet is the wallet that is used to recover
/// the funds from an unprocessed deposit request transaction (DRT)
/// after the [`RECOVERY_DELAY`] has passed.
///
/// # Returns
///
/// The balance in satoshis where 1 BTC = 100_000_000 satoshis.
pub(crate) fn get_balance_recovery_inner(
    address: &str,
    musig_bridge_pk: &str,
    bitcoind_url: &str,
    bitcoind_user: &str,
    bitcoind_password: &str,
) -> Result<u64, Error> {
    // Parse stuff
    let address = address
        .parse::<Address<_>>()
        .map_err(|_| Error::BitcoinAddress)?
        .assume_checked();
    let musig_bridge_pk = parse_xonly_pk(musig_bridge_pk)?;

    // Get the wallet
    let mut wallet = recovery_wallet(musig_bridge_pk)?;

    // Instantiate the BitcoinD client
    let client = new_bitcoind_client(
        bitcoind_url,
        None,
        Some(bitcoind_user),
        Some(bitcoind_password),
    )?;
    sync_wallet(&mut wallet, &client)?;

    let balance = wallet
        .list_unspent()
        .filter_map(|utxo| {
            if utxo.txout.script_pubkey == address.script_pubkey() {
                Some(utxo.txout.value.to_sat())
            } else {
                None
            }
        })
        .sum();

    Ok(balance)
}

#[cfg(test)]
mod tests {
    use std::sync::Once;

    use bdk_wallet::{bitcoin::Amount, KeychainKind, LocalOutput};
    use bitcoind_async_client::{traits::Broadcaster, Client};
    use corepc_node::Node;
    use strata_common::logging;
    use tokio::time::{sleep, Duration};
    use tracing::{debug, info, trace};

    use super::*;
    use crate::taproot::taproot_wallet;

    static INIT: Once = Once::new();

    const EL_ADDRESS: &str = "deedf001900dca3ebeefdeadf001900dca3ebeef";
    const MUSIG_BRIDGE_PK: &str =
        "14ced579c6a92533fa68ccc16da93b41073993cfc6cc982320645d8e9a63ee65";

    /// Initializes logging for a given test.
    ///
    /// This avoids multiple threads calling `logging::init` at the same time.
    fn init_logging(name: &str) {
        INIT.call_once(|| {
            logging::init(logging::LoggerConfig::with_base_name(name));
        });
    }

    /// Get the authentication credentials for a given `bitcoind` instance.
    fn get_auth(bitcoind: &Node) -> (String, String) {
        let params = &bitcoind.params;
        let cookie_values = params.get_cookie_values().unwrap().unwrap();
        (cookie_values.user, cookie_values.password)
    }

    /// Mine a number of blocks of a given size `count`, which may be specified to a given coinbase
    /// `address`.
    fn mine_blocks(bitcoind: &Node, count: usize, address: Option<Address>) -> anyhow::Result<()> {
        let coinbase_address = match address {
            Some(address) => address,
            None => bitcoind.client.new_address()?,
        };
        let _ = bitcoind
            .client
            .generate_to_address(count as _, &coinbase_address)?;
        Ok(())
    }

    #[tokio::test]
    async fn drt_mempool_accept() {
        init_logging("drt-tests");

        let bitcoind = Node::from_downloaded().unwrap();
        let url = bitcoind.rpc_url();
        let (user, password) = get_auth(&bitcoind);
        let client = Client::new(url.clone(), user.clone(), password.clone(), None, None).unwrap();

        let mut wallet = taproot_wallet().unwrap();
        let address = wallet.reveal_next_address(KeychainKind::External).address;
        debug!(%address, "wallet receiving address");

        // Mine and get the last UTXO which should have 50 BTC.
        mine_blocks(&bitcoind, 101, Some(address)).unwrap();
        debug!("mined 101 blocks");

        let signed_tx =
            deposit_request_transaction_inner(EL_ADDRESS, MUSIG_BRIDGE_PK, &url, &user, &password)
                .unwrap();
        trace!(?signed_tx, "signed drt tx");

        let txid = client.send_raw_transaction(&signed_tx).await.unwrap();
        debug!(%txid, "sent drt tx");

        assert_eq!(txid, signed_tx.compute_txid());
    }

    #[tokio::test]
    async fn recovery_path_mempool_accept() {
        init_logging("recovery-path-tests");

        let bitcoind = Node::from_downloaded().unwrap();
        let url = bitcoind.rpc_url();
        let (user, password) = get_auth(&bitcoind);
        let client = Client::new(url.clone(), user.clone(), password.clone(), None, None).unwrap();
        let wallet_client = new_bitcoind_client(&url, None, Some(&user), Some(&password))
            .expect("valid wallet client");

        // Get the taproot wallet.
        let mut wallet = taproot_wallet().unwrap();
        let address = wallet.reveal_next_address(KeychainKind::External).address;
        debug!(%address, "wallet receiving address");
        let change_address = wallet.reveal_next_address(KeychainKind::Internal).address;
        debug!(%change_address, "wallet change address");

        // Get the recovery wallet.
        let musig_bridge_pk = parse_xonly_pk(MUSIG_BRIDGE_PK).unwrap();
        debug!(?musig_bridge_pk, "musig bridge pk");
        let mut recovery_wallet = recovery_wallet(musig_bridge_pk).unwrap();
        let recovery_address = recovery_wallet
            .reveal_next_address(KeychainKind::External)
            .address;
        debug!(%recovery_address, "recovery address");

        // Mine and get the last UTXO which should have 50 BTC.
        mine_blocks(&bitcoind, 101, Some(address)).unwrap();
        debug!("mined 101 blocks");

        // Mine one block to the recovery address so that it has fees for the recovery path.
        mine_blocks(&bitcoind, 1, Some(recovery_address)).unwrap();

        // Sleep for a while to let the transactions propagate.
        sleep(Duration::from_millis(200)).await;

        sync_wallet(&mut wallet, &wallet_client).unwrap();
        debug!("wallet synced with bitcoind");
        let wallet_utxos = wallet.list_unspent().collect::<Vec<LocalOutput>>();
        trace!(?wallet_utxos, "wallet utxos");
        let coinbase_utxo = wallet_utxos.first().unwrap();
        trace!(?coinbase_utxo, "coinbase utxo");
        let coinbase_outpoint = coinbase_utxo.outpoint.to_string();
        trace!(%coinbase_outpoint, "coinbase outpoint");

        let signed_tx =
            deposit_request_transaction_inner(EL_ADDRESS, MUSIG_BRIDGE_PK, &url, &user, &password)
                .unwrap();
        trace!(?signed_tx, "signed drt tx");

        let txid = client.send_raw_transaction(&signed_tx).await.unwrap();
        debug!(%txid, "sent drt tx");

        // Mine blocks enough for the spending policy (1008 blocks).
        // Need to break this into chunks to avoid bitcoind crashing.
        let blocks_for_maturity = RECOVER_DELAY;
        let chunks = 8u32;
        let chunk_size = blocks_for_maturity / chunks;
        for _ in 0..chunks {
            mine_blocks(&bitcoind, chunk_size as _, None).unwrap();
        }

        let recovery_tx = spend_recovery_path_inner(
            change_address.to_string().as_str(),
            MUSIG_BRIDGE_PK,
            &url,
            &user,
            &password,
        )
        .unwrap();
        let txid = client.send_raw_transaction(&recovery_tx).await.unwrap();
        assert_eq!(txid, recovery_tx.compute_txid());
    }

    #[test]
    fn recovery_wallet_address() {
        let musig_bridge_pk = parse_xonly_pk(MUSIG_BRIDGE_PK).unwrap();
        let mut wallet = recovery_wallet(musig_bridge_pk).unwrap();
        let address = wallet
            .reveal_next_address(KeychainKind::External)
            .address
            .to_string();
        let expected_address = "bcrt1pupc4tw9e2l7xlj7g5hg9587e78mcrfxkj23jklaf58jp2vwtuarq6eq4d9";
        assert_eq!(address, expected_address);
    }

    #[tokio::test]
    async fn get_balance() {
        init_logging("balance-tests");

        let bitcoind = Node::from_downloaded().unwrap();
        let url = bitcoind.rpc_url();
        let (user, password) = get_auth(&bitcoind);
        let client = Client::new(url.clone(), user.clone(), password.clone(), None, None).unwrap();
        let wallet_client = new_bitcoind_client(&url, None, Some(&user), Some(&password))
            .expect("valid wallet client");

        let mut wallet = super::taproot_wallet().unwrap();
        let address = wallet.reveal_next_address(KeychainKind::External).address;
        debug!(%address, "wallet receiving address");
        let change_address = wallet.reveal_next_address(KeychainKind::Internal).address;
        debug!(%change_address, "wallet change address");

        // Mine and get the last UTXO which should have 50 BTC.
        mine_blocks(&bitcoind, 1, Some(address.clone())).unwrap();
        mine_blocks(&bitcoind, 100, None).unwrap();
        debug!("mined 101 blocks");

        // Sleep for a while to let the transactions propagate.
        sleep(Duration::from_millis(200)).await;

        // Sync the wallet
        sync_wallet(&mut wallet, &wallet_client).unwrap();
        debug!("wallet synced with bitcoind");

        // Getting the balances
        let balance_address =
            super::get_balance_inner(&address.to_string(), &url, &user, &password)
                .expect("valid balance");
        info!(%balance_address, "before: balance address");
        let change_balance_address =
            super::get_balance_inner(&change_address.to_string(), &url, &user, &password)
                .expect("valid balance");
        info!(%change_balance_address, "before: change balance address");

        // Send 10 BTC to the change address
        let amount = Amount::from_btc(10.0).unwrap();
        let mut psbt = {
            let mut builder = wallet.build_tx();
            builder.add_recipient(change_address.script_pubkey(), amount);
            builder.fee_rate(FeeRate::from_sat_per_vb_unchecked(2));
            builder.finish().unwrap()
        };
        wallet.sign(&mut psbt, Default::default()).unwrap();
        let signed_tx = psbt.extract_tx().unwrap();
        trace!(?signed_tx, "signed drt tx");
        let txid = client.send_raw_transaction(&signed_tx).await.unwrap();
        debug!(%txid, "sent tx");

        // Mine the transaction
        mine_blocks(&bitcoind, 1, None).unwrap();

        // Sleep for a while to let the transactions propagate.
        sleep(Duration::from_millis(200)).await;

        // Getting the balances
        let balance_address =
            super::get_balance_inner(&address.to_string(), &url, &user, &password)
                .expect("valid balance");
        info!(%balance_address, "after: balance address");
        let change_balance_address =
            super::get_balance_inner(&change_address.to_string(), &url, &user, &password)
                .expect("valid balance");
        info!(%change_balance_address, "after: change balance address");

        assert!(balance_address < 50_000_000);
        assert!(change_balance_address > 10_000_000);
    }

    #[tokio::test]
    async fn get_balance_recovery() {
        init_logging("recovery-balance-tests");

        let bitcoind = Node::from_downloaded().unwrap();
        let url = bitcoind.rpc_url();
        let (user, password) = get_auth(&bitcoind);
        let client = Client::new(url.clone(), user.clone(), password.clone(), None, None).unwrap();
        let wallet_client = new_bitcoind_client(&url, None, Some(&user), Some(&password))
            .expect("valid wallet client");

        // Get the taproot wallet.
        let mut wallet = taproot_wallet().unwrap();
        let address = wallet.reveal_next_address(KeychainKind::External).address;
        debug!(%address, "wallet receiving address");
        let change_address = wallet.reveal_next_address(KeychainKind::Internal).address;
        debug!(%change_address, "wallet change address");

        // Get the recovery wallet.
        let musig_bridge_pk = parse_xonly_pk(MUSIG_BRIDGE_PK).unwrap();
        debug!(?musig_bridge_pk, "musig bridge pk");
        let mut recovery_wallet = recovery_wallet(musig_bridge_pk).unwrap();
        let recovery_address = recovery_wallet
            .reveal_next_address(KeychainKind::External)
            .address;
        debug!(%recovery_address, "recovery address");

        // Mine and get the last UTXO which should have 50 BTC.
        mine_blocks(&bitcoind, 101, Some(address)).unwrap();
        debug!("mined 101 blocks");

        // Mine one block to the recovery address so that it has fees for the recovery path.
        mine_blocks(&bitcoind, 1, Some(recovery_address.clone())).unwrap();

        // Sleep for a while to let the transactions propagate.
        sleep(Duration::from_millis(200)).await;

        sync_wallet(&mut wallet, &wallet_client).unwrap();
        debug!("wallet synced with bitcoind");
        let wallet_utxos = wallet.list_unspent().collect::<Vec<LocalOutput>>();
        trace!(?wallet_utxos, "wallet utxos");
        let coinbase_utxo = wallet_utxos.first().unwrap();
        trace!(?coinbase_utxo, "coinbase utxo");
        let coinbase_outpoint = coinbase_utxo.outpoint.to_string();
        trace!(%coinbase_outpoint, "coinbase outpoint");

        let signed_tx =
            deposit_request_transaction_inner(EL_ADDRESS, MUSIG_BRIDGE_PK, &url, &user, &password)
                .unwrap();
        trace!(?signed_tx, "signed drt tx");

        // Getting the balance pre-DRT
        let balance_recovery_address = super::get_balance_recovery_inner(
            &recovery_address.to_string(),
            &musig_bridge_pk.to_string(),
            &url,
            &user,
            &password,
        )
        .expect("valid balance");
        info!(%balance_recovery_address, "before: balance address");
        assert_eq!(
            balance_recovery_address,
            Amount::from_btc(50.0).unwrap().to_sat()
        );

        let txid = client.send_raw_transaction(&signed_tx).await.unwrap();
        debug!(%txid, "sent drt tx");

        // Mine blocks enough for the spending policy (1008 blocks).
        // Need to break this into chunks to avoid bitcoind crashing.
        let blocks_for_maturity = RECOVER_DELAY;
        let chunks = 8u32;
        let chunk_size = blocks_for_maturity / chunks;
        for _ in 0..chunks {
            mine_blocks(&bitcoind, chunk_size as _, None).unwrap();
        }

        let recovery_tx = spend_recovery_path_inner(
            change_address.to_string().as_str(),
            MUSIG_BRIDGE_PK,
            &url,
            &user,
            &password,
        )
        .unwrap();
        let txid = client.send_raw_transaction(&recovery_tx).await.unwrap();
        debug!(%txid, "sent recovery tx");

        // Mine the transaction
        mine_blocks(&bitcoind, 1, None).unwrap();

        // Sleep for a while to let the transactions propagate.
        sleep(Duration::from_millis(200)).await;

        // Getting the balance post-DRT
        let balance_recovery_address = super::get_balance_recovery_inner(
            &recovery_address.to_string(),
            &musig_bridge_pk.to_string(),
            &url,
            &user,
            &password,
        )
        .expect("valid balance");
        info!(%balance_recovery_address, "after: balance address");
        assert!(balance_recovery_address < Amount::from_btc(50.0).unwrap().to_sat());
    }
}
