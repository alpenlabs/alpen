//! Bitcoin funding and broadcasting for admin transactions.

use std::{collections::HashMap, fmt, thread, time::Duration};

use anyhow::{bail, ensure, Context};
use bdk_bitcoind_rpc::bitcoincore_rpc::{json::FundRawTransactionOptions, Auth, Client, RpcApi};
use bdk_wallet::{
    bitcoin::{consensus::serialize, Address, Amount, FeeRate, Network, Transaction},
    KeychainKind, TxOrdering,
};
use serde_json::Value;

use crate::taproot::{sync_wallet, taproot_wallet};

const DEFAULT_RETRY_COUNT: usize = 5;
const RETRY_SLEEP_MS: u64 = 200;

/// Bitcoin RPC and transaction funding configuration.
pub(crate) struct BitcoinFundingConfig {
    pub(crate) rpc_url: String,
    pub(crate) rpc_user: String,
    pub(crate) rpc_password: String,
    pub(crate) wallet: Option<String>,
    pub(crate) fee_rate_sat_vb: u64,
    pub(crate) commit_output_sats: u64,
}

impl fmt::Debug for BitcoinFundingConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BitcoinFundingConfig")
            .field("wallet", &self.wallet)
            .field("fee_rate_sat_vb", &self.fee_rate_sat_vb)
            .field("commit_output_sats", &self.commit_output_sats)
            .finish_non_exhaustive()
    }
}

/// Bitcoin RPC clients and funding operations.
pub(crate) struct BitcoinFunding {
    rpc: Client,
    wallet_rpc: Option<Client>,
    network: Network,
    fee_rate_sat_vb: u32,
    commit_output_sats: u64,
}

impl fmt::Debug for BitcoinFunding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BitcoinFunding")
            .field("network", &self.network)
            .field("fee_rate_sat_vb", &self.fee_rate_sat_vb)
            .field("commit_output_sats", &self.commit_output_sats)
            .finish_non_exhaustive()
    }
}

impl BitcoinFunding {
    pub(crate) fn connect(config: BitcoinFundingConfig, network: Network) -> anyhow::Result<Self> {
        let fee_rate_sat_vb =
            u32::try_from(config.fee_rate_sat_vb).context("fee rate exceeds u32 range")?;
        let rpc = Client::new(
            &config.rpc_url,
            Auth::UserPass(config.rpc_user.clone(), config.rpc_password.clone()),
        )
        .context("create bitcoind RPC client")?;
        let wallet_rpc = match config.wallet {
            Some(wallet) => {
                let url = format!("{}/wallet/{wallet}", config.rpc_url.trim_end_matches('/'));
                Some(
                    Client::new(&url, Auth::UserPass(config.rpc_user, config.rpc_password))
                        .context("create bitcoind funding-wallet client")?,
                )
            }
            None => {
                ensure!(
                    network == Network::Regtest,
                    "--btc-wallet is required on {network} (built-in test wallet is regtest-only)"
                );
                None
            }
        };
        Ok(Self {
            rpc,
            wallet_rpc,
            network,
            fee_rate_sat_vb,
            commit_output_sats: config.commit_output_sats,
        })
    }

    pub(crate) fn network(&self) -> Network {
        self.network
    }

    pub(crate) fn fee_rate_sat_vb(&self) -> u64 {
        u64::from(self.fee_rate_sat_vb)
    }

    pub(crate) fn commit_output_sats(&self) -> u64 {
        self.commit_output_sats
    }

    pub(crate) fn fund_commit(&self, address: &Address) -> anyhow::Result<(Transaction, Address)> {
        match &self.wallet_rpc {
            Some(client) => self.fund_commit_with_bitcoind_wallet(client, address),
            None => self.fund_commit_with_test_wallet(address),
        }
    }

    /// Funds and signs the commit tx with the hardcoded regtest test wallet (functional tests).
    fn fund_commit_with_test_wallet(
        &self,
        reveal_address: &Address,
    ) -> anyhow::Result<(Transaction, Address)> {
        let mut wallet = taproot_wallet()?;
        sync_wallet(&mut wallet, &self.rpc)?;

        let fee_rate = FeeRate::from_sat_per_vb_u32(self.fee_rate_sat_vb);
        let mut psbt = {
            let mut builder = wallet.build_tx();
            builder.ordering(TxOrdering::Untouched);
            builder.add_recipient(
                reveal_address.script_pubkey(),
                Amount::from_sat(self.commit_output_sats),
            );
            builder.fee_rate(fee_rate);
            builder.finish().context("failed to build commit tx")?
        };

        let finalized = wallet
            .sign(&mut psbt, Default::default())
            .context("failed to sign commit tx")?;
        ensure!(finalized, "test wallet could not finalize commit tx");

        let commit_tx = psbt.extract_tx().context("failed to finalize commit tx")?;
        let recipient = wallet.peek_address(KeychainKind::External, 0).address;
        Ok((commit_tx, recipient))
    }

    /// Funds and signs the commit tx with a `bitcoind` wallet
    /// (`createrawtransaction` -> `fundrawtransaction` -> `signrawtransactionwithwallet`).
    /// Nothing is broadcast and no UTXOs are locked.
    fn fund_commit_with_bitcoind_wallet(
        &self,
        client: &Client,
        reveal_address: &Address,
    ) -> anyhow::Result<(Transaction, Address)> {
        let chain = client
            .get_blockchain_info()
            .context("getblockchaininfo failed")?
            .chain;
        ensure!(
            chain == self.network,
            "bitcoind is on {chain}, but --network is {}",
            self.network
        );

        let outs = HashMap::from([(
            reveal_address.to_string(),
            Amount::from_sat(self.commit_output_sats),
        )]);
        let raw = client
            .create_raw_transaction_hex(&[], &outs, None, Some(true))
            .context("createrawtransaction failed")?;

        // `feeRate` is BTC per kvB: sat/vB * 1000 = sat/kvB.
        let fee_rate_per_kvb = u64::from(self.fee_rate_sat_vb)
            .checked_mul(1000)
            .context("fee rate overflow")?;
        let options = FundRawTransactionOptions {
            fee_rate: Some(Amount::from_sat(fee_rate_per_kvb)),
            replaceable: Some(true),
            ..Default::default()
        };
        // Zero-input txs are ambiguous with the segwit marker; createrawtransaction emits legacy.
        let funded = client
            .fund_raw_transaction(raw, Some(&options), Some(false))
            .context("fundrawtransaction failed")?;

        let signed = client
            .sign_raw_transaction_with_wallet(&funded.hex, None, None)
            .context("signrawtransactionwithwallet failed")?;
        if !signed.complete {
            bail!(
                "wallet could not fully sign commit tx: {:?}",
                signed.errors.unwrap_or_default()
            );
        }
        let commit_tx = signed
            .transaction()
            .context("failed to decode signed commit tx")?;

        let recipient = client
            .get_raw_change_address(None)
            .context("getrawchangeaddress failed")?
            .require_network(self.network)
            .context("wallet change address is on the wrong network")?;

        Ok((commit_tx, recipient))
    }

    pub(crate) fn broadcast_tx(&self, tx: &Transaction) -> anyhow::Result<String> {
        let raw_hex = hex::encode(serialize(tx));
        self.rpc
            .call("sendrawtransaction", &[Value::String(raw_hex)])
            .context("failed to broadcast transaction")
    }

    pub(crate) fn broadcast_reveal_with_retry(
        &self,
        reveal_tx: &Transaction,
    ) -> anyhow::Result<String> {
        for attempt in 0..DEFAULT_RETRY_COUNT {
            match self.broadcast_tx(reveal_tx) {
                Ok(txid) => return Ok(txid),
                Err(err) => {
                    let msg = err.to_string().to_lowercase();
                    let should_retry = msg.contains("missing") || msg.contains("invalid input");
                    if should_retry && attempt + 1 < DEFAULT_RETRY_COUNT {
                        thread::sleep(Duration::from_millis(RETRY_SLEEP_MS));
                        continue;
                    }
                    return Err(err);
                }
            }
        }

        bail!("exhausted reveal broadcast retries")
    }
}
