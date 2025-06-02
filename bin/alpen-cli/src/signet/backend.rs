use std::{
    collections::BTreeSet,
    fmt::Debug,
    marker::Send,
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use bdk_bitcoind_rpc::{
    bitcoincore_rpc::{self, json::EstimateMode, RpcApi},
    BlockEvent, Emitter,
};
use bdk_esplora::EsploraAsyncExt;
use bdk_wallet::{
    bitcoin::{consensus::encode, Block, FeeRate, Transaction},
    chain::{
        spk_client::{FullScanRequestBuilder, FullScanResponse, SyncRequestBuilder, SyncResponse},
        CheckPoint,
    },
    KeychainKind,
};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use terrors::OneOf;
use tokio::sync::mpsc::UnboundedSender;

use super::EsploraClient;

#[macro_export]
macro_rules! boxed_err {
    ($name:ident) => {
        impl $name {
            pub fn from_err<E>(err: E) -> Self
            where
                E: std::error::Error + Send + Sync + 'static,
            {
                Self::from(Box::new(err) as $crate::errors::BoxedErr)
            }
        }

        impl std::ops::Deref for $name {
            type Target = BoxedInner;
            fn deref(&self) -> &Self::Target {
                self.0.as_ref()
            }
        }

        impl From<BoxedErr> for $name {
            fn from(err: BoxedErr) -> Self {
                Self(err)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                std::fmt::Display::fmt(&self.0, f)
            }
        }

        impl std::error::Error for $name {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                self.0.source()
            }
        }
    };
}

use crate::errors::{BoxedErr, BoxedInner};

#[derive(Debug)]
pub struct UpdateError(BoxedErr);
boxed_err!(UpdateError);

#[derive(Debug)]
pub struct SyncError(BoxedErr);
boxed_err!(SyncError);

#[derive(Debug)]
pub struct ScanError(BoxedErr);
boxed_err!(ScanError);

#[derive(Debug)]
pub struct BroadcastTxError(BoxedErr);
boxed_err!(BroadcastTxError);

#[derive(Debug)]
pub struct GetFeeRateError(BoxedErr);
boxed_err!(GetFeeRateError);

pub enum WalletUpdate {
    SpkSync(SyncResponse),
    SpkScan(FullScanResponse<KeychainKind>),
    NewBlock(BlockEvent<Block>),
    MempoolTxs(Vec<(Transaction, u64)>),
}

pub type UpdateSender = UnboundedSender<WalletUpdate>;

#[async_trait]
pub trait SignetBackend: Debug + Send + Sync {
    async fn sync_wallet(
        &self,
        req: SyncRequestBuilder<(KeychainKind, u32)>,
        last_cp: CheckPoint,
        send_update: UpdateSender,
    ) -> Result<(), SyncError>;
    async fn scan_wallet(
        &self,
        req: FullScanRequestBuilder<KeychainKind>,
        last_cp: CheckPoint,
        send_update: UpdateSender,
    ) -> Result<(), ScanError>;
    async fn broadcast_tx(&self, tx: &Transaction) -> Result<(), BroadcastTxError>;
    async fn get_fee_rate(
        &self,
        target: u16,
    ) -> Result<Option<FeeRate>, OneOf<(InvalidFee, GetFeeRateError)>>;
}

#[async_trait]
impl SignetBackend for EsploraClient {
    async fn sync_wallet(
        &self,
        req: SyncRequestBuilder<(KeychainKind, u32)>,
        _last_cp: CheckPoint,
        send_update: UpdateSender,
    ) -> Result<(), SyncError> {
        println!("Syncing wallet...");
        let sty = ProgressStyle::with_template(
            "[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}",
        )
        .unwrap()
        .progress_chars("##-");

        let bar = MultiProgress::new();

        let ops = bar.add(ProgressBar::new(1));
        ops.set_style(sty.clone());
        ops.set_message("outpoints");
        let ops2 = ops.clone();

        let spks = bar.add(ProgressBar::new(1));
        spks.set_style(sty.clone());
        spks.set_message("script public keys");
        let spks2 = spks.clone();

        let txids = bar.add(ProgressBar::new(1));
        txids.set_style(sty.clone());
        txids.set_message("transactions");
        let txids2 = txids.clone();
        let req = req
            .inspect(move |item, progress| {
                let _ = bar.println(format!("{item}"));
                ops.set_length(progress.total_outpoints() as u64);
                ops.set_position(progress.outpoints_consumed as u64);
                spks.set_length(progress.total_spks() as u64);
                spks.set_position(progress.spks_consumed as u64);
                txids.set_length(progress.total_txids() as u64);
                txids.set_length(progress.txids_consumed as u64);
            })
            .build();

        let update = self
            .sync(req, 3)
            .await
            .map_err(|e| Box::new(e) as BoxedErr)?;
        ops2.finish();
        spks2.finish();
        txids2.finish();
        println!("Updating wallet");
        send_update.send(WalletUpdate::SpkSync(update)).unwrap();
        println!("Wallet synced");
        Ok(())
    }

    async fn scan_wallet(
        &self,
        req: FullScanRequestBuilder<KeychainKind>,
        _last_cp: CheckPoint,
        send_update: UpdateSender,
    ) -> Result<(), ScanError> {
        let bar = ProgressBar::new_spinner();
        bar.enable_steady_tick(Duration::from_millis(100));
        let bar2 = bar.clone();
        let req = req
            .inspect({
                let mut once = BTreeSet::<KeychainKind>::new();
                move |keychain, spk_i, script| {
                    if once.insert(keychain) {
                        bar2.println(format!("\nScanning keychain [{keychain:?}]"));
                    }
                    bar2.println(format!("- idx {spk_i}: {script}"));
                }
            })
            .build();

        let update = self
            .full_scan(req, 5, 3)
            .await
            .map_err(|e| Box::new(e) as BoxedErr)?;
        bar.set_message("Persisting updates");
        send_update.send(WalletUpdate::SpkScan(update)).unwrap();
        bar.finish_with_message("Scan complete");
        Ok(())
    }

    async fn broadcast_tx(&self, tx: &Transaction) -> Result<(), BroadcastTxError> {
        self.broadcast(tx)
            .await
            .map_err(|e| (Box::new(e) as BoxedErr).into())
    }

    async fn get_fee_rate(
        &self,
        target: u16,
    ) -> Result<Option<FeeRate>, OneOf<(InvalidFee, GetFeeRateError)>> {
        match self
            .get_fee_estimates()
            .await
            .map_err(|e| GetFeeRateError(Box::new(e) as BoxedErr))
            .map_err(OneOf::new)?
            .get(&target)
            .cloned()
        {
            Some(fr) => Ok(Some(
                FeeRate::from_sat_per_vb(fr as u64).ok_or(OneOf::new(InvalidFee))?,
            )),
            None => Ok(None),
        }
    }
}

#[async_trait]
impl SignetBackend for Arc<bitcoincore_rpc::Client> {
    async fn sync_wallet(
        &self,
        _req: SyncRequestBuilder<(KeychainKind, u32)>,
        last_cp: CheckPoint,
        send_update: UpdateSender,
    ) -> Result<(), SyncError> {
        sync_wallet_with_core(self.clone(), last_cp, false, send_update)
            .await
            .map_err(|e| (Box::new(e) as BoxedErr).into())
    }

    async fn scan_wallet(
        &self,
        _req: FullScanRequestBuilder<KeychainKind>,
        last_cp: CheckPoint,
        send_update: UpdateSender,
    ) -> Result<(), ScanError> {
        sync_wallet_with_core(self.clone(), last_cp, true, send_update)
            .await
            .map_err(|e| (Box::new(e) as BoxedErr).into())
    }

    async fn broadcast_tx(&self, tx: &Transaction) -> Result<(), BroadcastTxError> {
        let hex = encode::serialize_hex(tx);

        spawn_bitcoin_core(self.clone(), move |c| c.send_raw_transaction(hex))
            .await
            .map_err(|e| BroadcastTxError(Box::new(e) as BoxedErr))?;
        Ok(())
    }

    async fn get_fee_rate(
        &self,
        target: u16,
    ) -> Result<Option<FeeRate>, OneOf<(InvalidFee, GetFeeRateError)>> {
        let res = spawn_bitcoin_core(self.clone(), move |c| {
            c.estimate_smart_fee(target, Some(EstimateMode::Conservative))
        })
        .await
        .map_err(|e| GetFeeRateError(Box::new(e) as BoxedErr))
        .map_err(OneOf::new)?;

        match res.fee_rate {
            Some(per_kw) => Ok(Some(
                FeeRate::from_sat_per_vb((per_kw / 1000).to_sat()).ok_or(OneOf::new(InvalidFee))?,
            )),
            None => Ok(None),
        }
    }
}

async fn spawn_bitcoin_core<T, F>(
    client: Arc<bitcoincore_rpc::Client>,
    func: F,
) -> Result<T, bitcoincore_rpc::Error>
where
    T: Send + 'static,
    F: FnOnce(&bitcoincore_rpc::Client) -> Result<T, bitcoincore_rpc::Error> + Send + 'static,
{
    let handle = tokio::task::spawn_blocking(move || func(&client));
    handle.await.expect("thread should be fine")
}

async fn sync_wallet_with_core(
    client: Arc<bitcoincore_rpc::Client>,
    last_cp: CheckPoint,
    should_scan: bool,
    send_update: UpdateSender,
) -> Result<(), bitcoincore_rpc::Error> {
    let bar = ProgressBar::new_spinner()
        .with_style(ProgressStyle::with_template("{spinner} [{elapsed_precise}] {msg}").unwrap());
    bar.enable_steady_tick(Duration::from_millis(100));
    let bar2 = bar.clone();

    let start_height = match should_scan {
        true => 0,
        false => last_cp.height(),
    };

    let mut blocks_scanned = 0;

    spawn_bitcoin_core(client.clone(), move |client| {
        let mut emitter = Emitter::new(client, last_cp, start_height);
        while let Some(ev) = emitter.next_block().unwrap() {
            blocks_scanned += 1;
            let height = ev.block_height();
            let hash = ev.block_hash();
            let start_apply_block = Instant::now();
            send_update.send(WalletUpdate::NewBlock(ev)).unwrap();
            let elapsed = start_apply_block.elapsed();
            bar2.println(format!(
                "Applied block {hash} at height {height} in {elapsed:?}"
            ));
            bar2.set_message(format!(
                "Current height: {height}, scanned {blocks_scanned} blocks"
            ));
        }
        bar2.println("Scanning mempool");
        let mempool = emitter.mempool().unwrap();
        let txs_len = mempool.len();
        let apply_start = Instant::now();
        send_update.send(WalletUpdate::MempoolTxs(mempool)).unwrap();
        let elapsed = apply_start.elapsed();
        bar.println(format!(
            "Applied {txs_len} unconfirmed transactions in {elapsed:?}"
        ));
        Ok(())
    })
    .await
}

#[derive(Debug)]
pub struct InvalidFee;
