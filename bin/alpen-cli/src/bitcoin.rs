use colored::Colorize;
pub mod backend;
pub mod persist;

use std::{
    fmt::Debug,
    io::{self},
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    sync::Arc,
};

use backend::{BitcoinBackend, ScanError, SyncError, UpdateError, WalletUpdate};
use bdk_esplora::esplora_client::{self, AsyncClient};
use bdk_wallet::{
    bitcoin::{FeeRate, Network},
    rusqlite::{self, Connection},
    PersistedWallet, Wallet,
};
use persist::Persister;
use terrors::OneOf;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

use crate::seed::Seed;

pub fn log_fee_rate(fr: &FeeRate) {
    println!(
        "Using {} as feerate",
        format!("{} sat/vb", fr.to_sat_per_vb_ceil()).green(),
    )
}

pub async fn get_fee_rate(
    user_provided_sats_per_vb: Option<u64>,
    bitcoin_backend: &dyn BitcoinBackend,
) -> FeeRate {
    let fee_rate = match user_provided_sats_per_vb {
        Some(fr) => FeeRate::from_sat_per_vb(fr).expect("valid fee rate"),
        None => bitcoin_backend
            .get_fee_rate(1)
            .await
            .expect("valid fee rate")
            .unwrap_or(FeeRate::BROADCAST_MIN),
    };

    fee_rate.max(FeeRate::BROADCAST_MIN)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use async_trait::async_trait;
    use bdk_wallet::{
        bitcoin::{FeeRate, Network, Transaction},
        chain::{
            spk_client::{FullScanRequestBuilder, SyncRequestBuilder},
            CheckPoint,
        },
        KeychainKind,
    };
    use terrors::OneOf;

    use super::{
        backend::{BroadcastTxError, GetFeeRateError, InvalidFee, ScanError, UpdateSender},
        get_fee_rate, BitcoinBackend, BitcoinWallet, SyncError,
    };

    #[derive(Debug)]
    struct TestBitcoinBackend {
        fee_rate: Option<FeeRate>,
    }

    #[async_trait]
    impl BitcoinBackend for TestBitcoinBackend {
        async fn sync_wallet(
            &self,
            _req: SyncRequestBuilder<(KeychainKind, u32)>,
            _last_cp: CheckPoint,
            _send_update: UpdateSender,
        ) -> Result<(), SyncError> {
            unimplemented!("not needed for fee rate tests")
        }

        async fn scan_wallet(
            &self,
            _req: FullScanRequestBuilder<KeychainKind>,
            _last_cp: CheckPoint,
            _send_update: UpdateSender,
        ) -> Result<(), ScanError> {
            unimplemented!("not needed for fee rate tests")
        }

        async fn broadcast_tx(&self, _tx: &Transaction) -> Result<(), BroadcastTxError> {
            unimplemented!("not needed for fee rate tests")
        }

        async fn get_fee_rate(
            &self,
            _target: u16,
        ) -> Result<Option<FeeRate>, OneOf<(InvalidFee, GetFeeRateError)>> {
            Ok(self.fee_rate)
        }
    }

    #[tokio::test]
    async fn test_get_fee_rate_clamps_backend_zero_to_broadcast_minimum() {
        let backend = TestBitcoinBackend {
            fee_rate: Some(FeeRate::ZERO),
        };

        let fee_rate = get_fee_rate(None, &backend).await;

        assert_eq!(fee_rate, FeeRate::BROADCAST_MIN);
    }

    #[tokio::test]
    async fn test_get_fee_rate_uses_broadcast_minimum_when_backend_has_no_estimate() {
        let backend = TestBitcoinBackend { fee_rate: None };

        let fee_rate = get_fee_rate(None, &backend).await;

        assert_eq!(fee_rate, FeeRate::BROADCAST_MIN);
    }

    #[tokio::test]
    async fn test_get_fee_rate_clamps_user_zero_to_broadcast_minimum() {
        let backend = TestBitcoinBackend { fee_rate: None };

        let fee_rate = get_fee_rate(Some(0), &backend).await;

        assert_eq!(fee_rate, FeeRate::BROADCAST_MIN);
    }

    #[test]
    fn uses_distinct_mainnet_database_path() {
        let data_dir = Path::new("wallet-data");
        assert_eq!(
            BitcoinWallet::db_path("default", data_dir, Network::Bitcoin),
            data_dir.join("default-bitcoin.sqlite")
        );
        assert_eq!(
            BitcoinWallet::db_path("default", data_dir, Network::Signet),
            data_dir.join("default.sqlite")
        );
    }
}

#[derive(Clone, Debug)]
pub struct EsploraClient(AsyncClient);

impl DerefMut for EsploraClient {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Deref for EsploraClient {
    type Target = AsyncClient;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl EsploraClient {
    pub fn new(esplora_url: &str) -> Result<Self, esplora_client::Error> {
        Ok(Self(
            esplora_client::Builder::new(esplora_url).build_async()?,
        ))
    }
}

#[derive(Debug)]
/// A wrapper around BDK's wallet with some custom logic
pub struct BitcoinWallet {
    wallet: PersistedWallet<Persister>,
    sync_backend: Arc<dyn BitcoinBackend>,
}

impl BitcoinWallet {
    fn db_path(wallet: &str, data_dir: &Path, network: Network) -> PathBuf {
        let wallet = match network {
            Network::Bitcoin => format!("{wallet}-bitcoin"),
            _ => wallet.to_string(),
        };
        data_dir.join(wallet).with_extension("sqlite")
    }

    pub fn persister(data_dir: &Path, network: Network) -> Result<Connection, rusqlite::Error> {
        Connection::open(Self::db_path("default", data_dir, network))
    }

    pub fn new(
        seed: &Seed,
        network: Network,
        sync_backend: Arc<dyn BitcoinBackend>,
    ) -> io::Result<Self> {
        let (load, create) = seed.bitcoin_wallet(network).split();
        Ok(Self {
            wallet: load
                .check_network(network)
                .load_wallet(&mut Persister)
                .expect("should be able to load wallet")
                .unwrap_or_else(|| {
                    create
                        .network(network)
                        .create_wallet(&mut Persister)
                        .expect("wallet creation to succeed")
                }),
            sync_backend,
        })
    }

    pub async fn sync(&mut self) -> Result<(), OneOf<(UpdateError, SyncError, rusqlite::Error)>> {
        sync_wallet(&mut self.wallet, self.sync_backend.clone()).await?;
        self.persist().map_err(OneOf::new)?;
        Ok(())
    }

    pub async fn scan(&mut self) -> Result<(), OneOf<(UpdateError, ScanError, rusqlite::Error)>> {
        scan_wallet(&mut self.wallet, self.sync_backend.clone()).await?;
        self.persist().map_err(OneOf::new)?;
        Ok(())
    }

    pub fn persist(&mut self) -> Result<bool, rusqlite::Error> {
        self.wallet.persist(&mut Persister)
    }
}

pub async fn scan_wallet(
    wallet: &mut Wallet,
    sync_backend: Arc<dyn BitcoinBackend>,
) -> Result<(), OneOf<(UpdateError, ScanError, rusqlite::Error)>> {
    let req = wallet.start_full_scan();
    let last_cp = wallet.latest_checkpoint();
    let (tx, rx) = unbounded_channel();

    let handle = tokio::spawn(async move { sync_backend.scan_wallet(req, last_cp, tx).await });

    apply_update_stream(wallet, rx).await.map_err(OneOf::new)?;

    handle
        .await
        .expect("thread to be fine")
        .map_err(OneOf::new)?;

    Ok(())
}

pub async fn sync_wallet(
    wallet: &mut Wallet,
    sync_backend: Arc<dyn BitcoinBackend>,
) -> Result<(), OneOf<(UpdateError, SyncError, rusqlite::Error)>> {
    let req = wallet.start_sync_with_revealed_spks();
    let last_cp = wallet.latest_checkpoint();
    let (tx, rx) = unbounded_channel();

    let handle = tokio::spawn(async move { sync_backend.sync_wallet(req, last_cp, tx).await });

    apply_update_stream(wallet, rx).await.map_err(OneOf::new)?;

    handle
        .await
        .expect("thread to be fine")
        .map_err(OneOf::new)?;

    Ok(())
}

async fn apply_update_stream(
    wallet: &mut Wallet,
    mut rx: UnboundedReceiver<WalletUpdate>,
) -> Result<(), UpdateError> {
    while let Some(update) = rx.recv().await {
        match update {
            WalletUpdate::SpkSync(update) => {
                wallet.apply_update(update).map_err(UpdateError::from_err)?
            }
            WalletUpdate::SpkScan(update) => {
                wallet.apply_update(update).map_err(UpdateError::from_err)?
            }
            WalletUpdate::NewBlock(ev) => {
                let height = ev.block_height();
                let connected_to = ev.connected_to();
                wallet
                    .apply_block_connected_to(&ev.block, height, connected_to)
                    .map_err(UpdateError::from_err)?
            }
            WalletUpdate::MempoolTxs(txs) => wallet.apply_unconfirmed_txs(txs),
        }
    }

    Ok(())
}

impl Deref for BitcoinWallet {
    type Target = PersistedWallet<Persister>;

    fn deref(&self) -> &Self::Target {
        &self.wallet
    }
}

impl DerefMut for BitcoinWallet {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.wallet
    }
}
