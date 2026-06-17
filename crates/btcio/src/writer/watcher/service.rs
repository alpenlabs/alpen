//! Watcher service for the btcio L1 writer.
//!
//! Drives the [`L1BundleStatus`] state machine for the current payload entry
//! on each timer tick.

use std::{
    collections::HashMap,
    future::Future,
    marker::PhantomData,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use bitcoin::secp256k1::XOnlyPublicKey;
use bitcoind_async_client::traits::{Reader, Signer, Wallet};
use serde::Serialize;
use strata_btc_types::{Buf32BitcoinExt, TxidExt};
use strata_db_types::types::{BundledPayloadEntry, L1BundleStatus, L1TxEntry, L1TxId, L1TxStatus};
use strata_primitives::buf::Buf32;
use strata_service::{AsyncService, Response, Service, ServiceState};
use strata_status::StatusChannel;
use strata_storage::ops::writer::EnvelopeDataOps;
use tracing::*;

use crate::{
    broadcaster::L1BroadcastHandle,
    rpc_error::{is_retryable_envelope_error, retryable_reason},
    status::{apply_status_updates, L1StatusUpdate},
    writer::{
        builder::{EnvelopeData, EnvelopeError},
        context::{EnvelopeSigningMode, WriterContext},
        signer::{
            complete_reveal_and_broadcast, create_payload_envelopes,
            sign_and_broadcast_payload_envelopes,
        },
    },
};

fn to_l1_txid(txid: bitcoin::Txid) -> L1TxId {
    L1TxId::from(txid.to_buf32().0)
}

fn to_raw_buf32(txid: L1TxId) -> Buf32 {
    Buf32(txid.0)
}

/// Abstracts the external dependencies of the watcher so that `process_input` can be
/// tested without a real Bitcoin node, database, or broadcast infrastructure.
pub(crate) trait WatcherServiceContext: Send + Sync + 'static {
    fn get_payload_entry(
        &self,
        idx: u64,
    ) -> impl Future<Output = anyhow::Result<Option<BundledPayloadEntry>>> + Send;

    fn put_payload_entry(
        &self,
        idx: u64,
        entry: BundledPayloadEntry,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    /// Returns the current envelope signing mode.
    fn signing_mode(&self) -> anyhow::Result<EnvelopeSigningMode>;

    fn create_envelopes(
        &self,
        idx: u64,
        entry: &BundledPayloadEntry,
        envelope_pubkey: XOnlyPublicKey,
    ) -> impl Future<Output = Result<EnvelopeData, EnvelopeError>> + Send;

    fn sign_and_broadcast(
        &self,
        idx: u64,
        entry: &BundledPayloadEntry,
    ) -> impl Future<Output = Result<(L1TxId, L1TxId), EnvelopeError>> + Send;
    fn complete_reveal_and_broadcast(
        &self,
        idx: u64,
        envelope: &EnvelopeData,
        sig: &[u8; 64],
    ) -> impl Future<Output = anyhow::Result<L1TxId>> + Send;
    fn get_tx_status(
        &self,
        txid: L1TxId,
    ) -> impl Future<Output = anyhow::Result<Option<L1TxEntry>>> + Send;

    fn report_status(
        &self,
        entry: &BundledPayloadEntry,
        status: &L1BundleStatus,
    ) -> impl Future<Output = ()> + Send;

    fn report_rpc_error(&self, reason: String) -> impl Future<Output = ()> + Send;
}

pub(crate) struct WatcherContextImpl<R: Reader + Signer + Wallet + Send + Sync + 'static> {
    context: Arc<WriterContext<R>>,
    ops: Arc<EnvelopeDataOps>,
    broadcast_handle: Arc<L1BroadcastHandle>,
}

impl<R: Reader + Signer + Wallet + Send + Sync + 'static> WatcherContextImpl<R> {
    pub(crate) fn new(
        context: Arc<WriterContext<R>>,
        ops: Arc<EnvelopeDataOps>,
        broadcast_handle: Arc<L1BroadcastHandle>,
    ) -> Self {
        Self {
            context,
            ops,
            broadcast_handle,
        }
    }
}

impl<R: Reader + Signer + Wallet + Send + Sync + 'static> WatcherServiceContext
    for WatcherContextImpl<R>
{
    async fn get_payload_entry(&self, idx: u64) -> anyhow::Result<Option<BundledPayloadEntry>> {
        self.ops
            .get_payload_entry_by_idx_async(idx)
            .await
            .map_err(Into::into)
    }

    async fn put_payload_entry(&self, idx: u64, entry: BundledPayloadEntry) -> anyhow::Result<()> {
        self.ops
            .put_payload_entry_async(idx, entry)
            .await
            .map_err(Into::into)
    }

    fn signing_mode(&self) -> anyhow::Result<EnvelopeSigningMode> {
        self.context.signing_mode()
    }

    async fn create_envelopes(
        &self,
        idx: u64,
        entry: &BundledPayloadEntry,
        envelope_pubkey: XOnlyPublicKey,
    ) -> Result<EnvelopeData, EnvelopeError> {
        create_payload_envelopes(idx, entry, self.context.clone(), envelope_pubkey).await
    }

    async fn sign_and_broadcast(
        &self,
        idx: u64,
        entry: &BundledPayloadEntry,
    ) -> Result<(L1TxId, L1TxId), EnvelopeError> {
        sign_and_broadcast_payload_envelopes(
            idx,
            entry,
            self.context.clone(),
            &self.broadcast_handle,
        )
        .await
    }

    async fn complete_reveal_and_broadcast(
        &self,
        idx: u64,
        envelope: &EnvelopeData,
        sig: &[u8; 64],
    ) -> anyhow::Result<L1TxId> {
        complete_reveal_and_broadcast(idx, envelope, sig, &self.broadcast_handle)
            .await
            .map_err(Into::into)
    }

    async fn get_tx_status(&self, txid: L1TxId) -> anyhow::Result<Option<L1TxEntry>> {
        self.broadcast_handle
            .get_tx_entry_by_id_async(Buf32(txid.0))
            .await
            .map_err(Into::into)
    }

    async fn report_status(&self, entry: &BundledPayloadEntry, status: &L1BundleStatus) {
        update_l1_status(entry, status, &self.context.status_channel).await;
    }

    async fn report_rpc_error(&self, reason: String) {
        let status_updates = [
            L1StatusUpdate::RpcConnected(false),
            L1StatusUpdate::RpcError(reason),
            L1StatusUpdate::LastUpdate(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64,
            ),
        ];
        apply_status_updates(&status_updates, &self.context.status_channel).await;
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct WatcherStatus {
    pub(crate) current_payload_idx: u64,
    pub(crate) cache_size: usize,
}

pub(crate) struct WatcherState<C: WatcherServiceContext> {
    pub(crate) ctx: C,
    pub(crate) envelope_cache: HashMap<u64, EnvelopeData>,
    pub(crate) curr_payloadidx: u64,
}

impl<C: WatcherServiceContext> WatcherState<C> {
    pub(crate) fn new(ctx: C, curr_payloadidx: u64) -> Self {
        Self {
            ctx,
            envelope_cache: HashMap::new(),
            curr_payloadidx,
        }
    }
}

impl<C: WatcherServiceContext> ServiceState for WatcherState<C> {
    fn name(&self) -> &str {
        "btcio_watcher"
    }
}

pub(crate) struct WatcherService<C>(PhantomData<C>);

impl<C: WatcherServiceContext> Service for WatcherService<C> {
    type State = WatcherState<C>;
    type Msg = ();
    type Status = WatcherStatus;

    fn get_status(state: &Self::State) -> Self::Status {
        WatcherStatus {
            current_payload_idx: state.curr_payloadidx,
            cache_size: state.envelope_cache.len(),
        }
    }
}

impl<C: WatcherServiceContext> AsyncService for WatcherService<C> {
    async fn process_input(state: &mut Self::State, _: Self::Msg) -> anyhow::Result<Response> {
        let dspan = debug_span!("process payload", idx=%state.curr_payloadidx);
        let _ = dspan.enter();

        if let Some(payloadentry) = state.ctx.get_payload_entry(state.curr_payloadidx).await? {
            match payloadentry.status {
                // If unsigned or needs resign, build envelope txs, sign commit with
                // wallet, and transition to PendingRevealTxSign awaiting the external
                // signer's Schnorr signature on the reveal tx.
                L1BundleStatus::Unsigned | L1BundleStatus::NeedsResign => {
                    state.handle_unsigned_or_needs_resign(payloadentry).await?;
                }

                // Waiting for the external signer to provide the reveal signature.
                // When the signature arrives (via RPC), complete the reveal tx and
                // transition to Unpublished.
                L1BundleStatus::PendingRevealTxSign(_) => {
                    state.handle_pending_reveal_tx_sign(payloadentry).await?;
                }

                // If finalized, nothing to do, move on to process next entry
                L1BundleStatus::Finalized => {
                    state.curr_payloadidx += 1;
                }

                // If entry is signed but not finalized or excluded yet, check broadcast txs status
                L1BundleStatus::Published
                | L1BundleStatus::Confirmed
                | L1BundleStatus::Unpublished => {
                    state.handle_broadcast_status(payloadentry).await?;
                }
            }
        } else {
            // No payload exists, just continue the loop to wait for payload's presence in db
            debug!("Waiting for payloadentry to be present in db");
        }

        Ok(Response::Continue)
    }
}

impl<C: WatcherServiceContext> WatcherState<C> {
    /// Resolves the current envelope signing mode, deferring on failure.
    ///
    /// The signing mode is derived from dynamic ASM state, so a transient
    /// failure (e.g. a `DbError`) or a currently non-signable predicate (e.g.
    /// `NeverAccept`/`Sp1Groth16` after a rotation) must not be fatal: the
    /// writer service treats a `process_input` error as terminal. Returns
    /// `None` after logging so callers can defer and let the tick loop
    /// re-evaluate on the next pass instead of permanently killing the writer.
    fn resolve_signing_mode(&self) -> Option<EnvelopeSigningMode> {
        match self.ctx.signing_mode() {
            Ok(mode) => Some(mode),
            Err(err) => {
                warn!(%err, "could not resolve envelope signing mode; deferring to next tick");
                None
            }
        }
    }

    /// Builds envelope txs and transitions to `PendingRevealTxSign` or `Unpublished`.
    ///
    /// When an external signer is configured, signs the commit tx
    /// via wallet, caches the envelope, and waits for the reveal signature via RPC.
    /// When no external signer is needed, signs both in-process
    /// and transitions directly to `Unpublished`.
    async fn handle_unsigned_or_needs_resign(
        &mut self,
        payloadentry: BundledPayloadEntry,
    ) -> anyhow::Result<()> {
        debug!(current_status=?payloadentry.status);

        let Some(signing_mode) = self.resolve_signing_mode() else {
            return Ok(());
        };
        match signing_mode {
            EnvelopeSigningMode::External { pubkey } => match self
                .ctx
                .create_envelopes(self.curr_payloadidx, &payloadentry, pubkey)
                .await
            {
                Ok(envelope) => {
                    let cid = to_l1_txid(envelope.commit_tx.compute_txid());
                    let rid = to_l1_txid(envelope.reveal_tx.compute_txid());
                    let sighash = envelope.sighash;

                    let mut updated_entry = payloadentry.clone();
                    updated_entry.commit_txid = cid;
                    updated_entry.reveal_txid = rid;
                    updated_entry.payload_signature = None;
                    updated_entry.status = L1BundleStatus::PendingRevealTxSign(sighash);
                    self.ctx
                        .put_payload_entry(self.curr_payloadidx, updated_entry)
                        .await?;

                    self.envelope_cache.insert(self.curr_payloadidx, envelope);

                    debug!(%sighash, "envelope built, awaiting signer");
                }
                Err(EnvelopeError::NotEnoughUtxos(required, available)) => {
                    warn!(%required, %available, "waiting for sufficient utxos to create commit/reveal transaction");
                }
                Err(err) if is_retryable_envelope_error(&err) => {
                    let reason = retryable_reason(&err);
                    warn!(%reason, "retrying envelope creation after Bitcoin RPC error");
                    self.ctx.report_rpc_error(reason).await;
                }
                Err(err) => {
                    return Err(err.into());
                }
            },
            EnvelopeSigningMode::InProcess => match self
                .ctx
                .sign_and_broadcast(self.curr_payloadidx, &payloadentry)
                .await
            {
                Ok((cid, rid)) => {
                    let mut updated_entry = payloadentry.clone();
                    updated_entry.commit_txid = cid;
                    updated_entry.reveal_txid = rid;
                    updated_entry.status = L1BundleStatus::Unpublished;
                    self.ctx
                        .put_payload_entry(self.curr_payloadidx, updated_entry)
                        .await?;

                    debug!(?cid, reveal_txid = ?rid, "envelope signed and queued for broadcast");
                }
                Err(EnvelopeError::NotEnoughUtxos(required, available)) => {
                    warn!(%required, %available, "waiting for sufficient utxos to create commit/reveal transaction");
                }
                Err(err) if is_retryable_envelope_error(&err) => {
                    let reason = retryable_reason(&err);
                    warn!(%reason, "retrying envelope signing after Bitcoin RPC error");
                    self.ctx.report_rpc_error(reason).await;
                }
                Err(err) => {
                    return Err(err.into());
                }
            },
        }

        Ok(())
    }

    /// Completes the reveal tx and broadcasts both txs once the external sig arrives.
    ///
    /// On cache miss (e.g. restart), resets to `Unsigned` — safe because nothing
    /// has been broadcast yet.
    async fn handle_pending_reveal_tx_sign(
        &mut self,
        payloadentry: BundledPayloadEntry,
    ) -> anyhow::Result<()> {
        let Some(sig) = &payloadentry.payload_signature else {
            let Some(envelope) = self.envelope_cache.get(&self.curr_payloadidx) else {
                // Cache miss (e.g. restart) — reset to Unsigned to rebuild
                // envelope from scratch (new UTXOs, new sighash).
                // Safe: nothing has been broadcast yet.
                warn!("envelope not in cache, resetting to Unsigned");
                let mut updated_entry = payloadentry.clone();
                updated_entry.payload_signature = None;
                updated_entry.status = L1BundleStatus::Unsigned;
                self.ctx
                    .put_payload_entry(self.curr_payloadidx, updated_entry)
                    .await?;
                return Ok(());
            };

            let Some(signing_mode) = self.resolve_signing_mode() else {
                return Ok(());
            };
            match signing_mode {
                EnvelopeSigningMode::External { pubkey } if pubkey == envelope.envelope_pubkey => {}
                _ => {
                    warn!("envelope signing mode changed, resetting to Unsigned");
                    let mut updated_entry = payloadentry.clone();
                    updated_entry.payload_signature = None;
                    updated_entry.status = L1BundleStatus::Unsigned;
                    self.ctx
                        .put_payload_entry(self.curr_payloadidx, updated_entry)
                        .await?;
                    self.envelope_cache.remove(&self.curr_payloadidx);
                    return Ok(());
                }
            }

            trace!("waiting for signer to provide reveal signature");
            return Ok(());
        };
        let Some(envelope) = self.envelope_cache.remove(&self.curr_payloadidx) else {
            // Cache miss (e.g. restart) — reset to Unsigned to rebuild
            // envelope from scratch (new UTXOs, new sighash).
            // Safe: nothing has been broadcast yet.
            warn!("envelope not in cache, resetting to Unsigned");
            let mut updated_entry = payloadentry.clone();
            updated_entry.payload_signature = None;
            updated_entry.status = L1BundleStatus::Unsigned;
            self.ctx
                .put_payload_entry(self.curr_payloadidx, updated_entry)
                .await?;
            return Ok(());
        };
        let Some(signing_mode) = self.resolve_signing_mode() else {
            // Preserve the cached envelope so a transient failure does not force
            // a needless rebuild + external re-sign on the next tick.
            self.envelope_cache.insert(self.curr_payloadidx, envelope);
            return Ok(());
        };
        match signing_mode {
            EnvelopeSigningMode::External { pubkey } if pubkey == envelope.envelope_pubkey => {}
            _ => {
                warn!("envelope signing mode changed, resetting to Unsigned");
                let mut updated_entry = payloadentry.clone();
                updated_entry.payload_signature = None;
                updated_entry.status = L1BundleStatus::Unsigned;
                self.ctx
                    .put_payload_entry(self.curr_payloadidx, updated_entry)
                    .await?;
                return Ok(());
            }
        }
        match self
            .ctx
            .complete_reveal_and_broadcast(self.curr_payloadidx, &envelope, sig.as_ref())
            .await
        {
            Ok(_rid) => {
                let mut updated_entry = payloadentry.clone();
                updated_entry.status = L1BundleStatus::Unpublished;
                self.ctx
                    .put_payload_entry(self.curr_payloadidx, updated_entry)
                    .await?;
                debug!("reveal signed and stored for broadcast");
            }
            Err(e) => {
                error!(%e, "failed to attach reveal signature");
            }
        }
        Ok(())
    }

    /// Checks broadcast tx statuses and advances the payload state machine.
    async fn handle_broadcast_status(
        &mut self,
        payloadentry: BundledPayloadEntry,
    ) -> anyhow::Result<()> {
        trace!("Checking payloadentry's broadcast status");
        let commit_tx = self.ctx.get_tx_status(payloadentry.commit_txid).await?;
        let reveal_tx = self.ctx.get_tx_status(payloadentry.reveal_txid).await?;

        match (commit_tx, reveal_tx) {
            (Some(ctx), Some(rtx)) => {
                let new_status = determine_payload_next_status(&ctx.status, &rtx.status);
                debug!(?new_status, "The next status for payload");
                if matches!(
                    new_status,
                    L1BundleStatus::Confirmed | L1BundleStatus::Finalized
                ) {
                    debug!(
                        component = "btcio_writer",
                        payload_idx = self.curr_payloadidx,
                        commit_txid = ?payloadentry.commit_txid,
                        reveal_txid = ?payloadentry.reveal_txid,
                        payload_status = ?new_status,
                        commit_l1_status = ?ctx.status,
                        reveal_l1_status = ?rtx.status,
                        "payload advanced on L1"
                    );
                }

                self.ctx.report_status(&payloadentry, &new_status).await;

                // Update payloadentry with new status
                let mut updated_entry = payloadentry.clone();
                updated_entry.status = new_status.clone();
                self.ctx
                    .put_payload_entry(self.curr_payloadidx, updated_entry)
                    .await?;

                if new_status == L1BundleStatus::Finalized {
                    self.curr_payloadidx += 1;
                }
            }
            _ => {
                warn!("Corresponding commit/reveal entry for payloadentry not found in broadcast db. Sign and create transactions again.");
                let mut updated_entry = payloadentry.clone();
                updated_entry.payload_signature = None;
                updated_entry.status = L1BundleStatus::Unsigned;
                self.ctx
                    .put_payload_entry(self.curr_payloadidx, updated_entry)
                    .await?;
            }
        }
        Ok(())
    }
}

async fn update_l1_status(
    payloadentry: &BundledPayloadEntry,
    new_status: &L1BundleStatus,
    status_channel: &StatusChannel,
) {
    // Update L1 status. Since we are processing one payloadentry at a time, if the entry is
    // finalized/confirmed, then it means it is published as well
    if *new_status == L1BundleStatus::Published
        || *new_status == L1BundleStatus::Confirmed
        || *new_status == L1BundleStatus::Finalized
    {
        let status_updates = [
            L1StatusUpdate::LastPublishedTxid(to_raw_buf32(payloadentry.reveal_txid).to_txid()),
            L1StatusUpdate::IncrementPublishedRevealCount,
        ];
        apply_status_updates(&status_updates, status_channel).await;
    }
}

/// Determine the status of the `PayloadEntry` based on the status of its commit and reveal
/// transactions in bitcoin.
pub(crate) fn determine_payload_next_status(
    commit_status: &L1TxStatus,
    reveal_status: &L1TxStatus,
) -> L1BundleStatus {
    match (&commit_status, &reveal_status) {
        // If reveal is finalized, both are finalized
        (_, L1TxStatus::Finalized { .. }) => L1BundleStatus::Finalized,
        // If reveal is confirmed, both are confirmed
        (_, L1TxStatus::Confirmed { .. }) => L1BundleStatus::Confirmed,
        // If reveal is published regardless of commit, the payload is published
        (_, L1TxStatus::Published) => L1BundleStatus::Published,
        // if commit has invalid inputs, needs resign
        (L1TxStatus::InvalidInputs, _) => L1BundleStatus::NeedsResign,
        // If commit is unpublished, both are upublished
        (L1TxStatus::Unpublished, _) => L1BundleStatus::Unpublished,
        // If commit is published but not reveal, the payload is unpublished
        (_, L1TxStatus::Unpublished) => L1BundleStatus::Unpublished,
        // If reveal has invalid inputs, these need resign because we can do nothing with just
        // commit tx confirmed. This should not occur in practice
        (_, L1TxStatus::InvalidInputs) => L1BundleStatus::NeedsResign,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use anyhow::anyhow;
    use bitcoin::{
        absolute::LockTime,
        blockdata::{opcodes, script::Builder as ScriptBuilder},
        hashes::Hash,
        key::UntweakedKeypair,
        secp256k1::{XOnlyPublicKey, SECP256K1},
        taproot::TaprootBuilder,
        transaction::Version,
        Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid, Witness,
    };
    use bitcoind_async_client::error::ClientError;
    use strata_csm_types::L1Payload;
    use strata_db_types::types::{BundledPayloadEntry, L1BundleStatus, L1TxEntry, L1TxId};
    use strata_l1_txfmt::TagData;
    use strata_primitives::buf::{Buf32, Buf64};

    use super::*;
    use crate::{
        broadcaster::L1BroadcastHandle,
        writer::{
            builder::{EnvelopeData, EnvelopeError},
            signer::complete_reveal_and_broadcast,
            test_utils::get_broadcast_handle,
        },
    };

    const TEST_REQUIRED_SATS: u64 = 4096;
    const TEST_AVAILABLE_SATS: u64 = 2658;

    #[derive(Clone, Copy)]
    enum MockEnvelopeFailure {
        NotEnoughUtxos,
        PrereqFetch,
        SignRawTransaction,
        Other,
    }

    impl MockEnvelopeFailure {
        fn into_error(self) -> EnvelopeError {
            match self {
                Self::NotEnoughUtxos => {
                    EnvelopeError::NotEnoughUtxos(TEST_REQUIRED_SATS, TEST_AVAILABLE_SATS)
                }
                Self::PrereqFetch => EnvelopeError::PrereqFetch(anyhow::Error::from(
                    ClientError::Connection("mock connection failure".to_string()),
                )),
                Self::SignRawTransaction => EnvelopeError::SignRawTransaction(
                    ClientError::Connection("mock signing failure".to_string()),
                ),
                Self::Other => EnvelopeError::Other(anyhow!("mock storage failure")),
            }
        }
    }

    fn minimal_envelope_data() -> EnvelopeData {
        let keypair =
            UntweakedKeypair::from_seckey_slice(SECP256K1, &[1u8; 32]).expect("valid key");
        let pubkey = XOnlyPublicKey::from_keypair(&keypair).0;
        // A single OP_TRUE leaf so control_block lookup succeeds in attach_reveal_signature
        let reveal_script = ScriptBuilder::new()
            .push_opcode(opcodes::OP_TRUE)
            .into_script();
        let taproot_spend_info = TaprootBuilder::new()
            .add_leaf(0, reveal_script.clone())
            .expect("valid leaf")
            .finalize(SECP256K1, pubkey)
            .expect("valid taproot");
        let dummy_input = TxIn {
            previous_output: OutPoint {
                txid: Txid::all_zeros(),
                vout: 0,
            },
            script_sig: ScriptBuf::new(),
            sequence: Sequence::MAX,
            witness: Witness::new(),
        };
        let commit_tx = Transaction {
            version: Version(2),
            lock_time: LockTime::ZERO,
            input: vec![dummy_input.clone()],
            output: vec![TxOut {
                value: Amount::from_sat(1000),
                script_pubkey: ScriptBuf::new(),
            }],
        };
        let reveal_tx = Transaction {
            version: Version(2),
            lock_time: LockTime::ZERO,
            input: vec![dummy_input],
            output: vec![TxOut {
                value: Amount::from_sat(546),
                script_pubkey: ScriptBuf::new(),
            }],
        };
        EnvelopeData::new(
            commit_tx,
            reveal_tx,
            Buf32([42u8; 32]),
            reveal_script,
            taproot_spend_info,
            pubkey,
        )
    }

    struct MockWatcherContext {
        stored: Mutex<HashMap<u64, BundledPayloadEntry>>,
        broadcast_handle: Arc<L1BroadcastHandle>,
        signing_mode: Mutex<EnvelopeSigningMode>,
        signing_mode_fails: Mutex<bool>,
        create_failure: Option<MockEnvelopeFailure>,
        sign_failure: Option<MockEnvelopeFailure>,
        rpc_errors: Mutex<Vec<String>>,
    }

    impl MockWatcherContext {
        fn new(external_signing: bool) -> Self {
            let signing_mode = if external_signing {
                EnvelopeSigningMode::External {
                    pubkey: minimal_envelope_data().envelope_pubkey,
                }
            } else {
                EnvelopeSigningMode::InProcess
            };
            Self {
                stored: Mutex::new(HashMap::new()),
                broadcast_handle: get_broadcast_handle(),
                signing_mode: Mutex::new(signing_mode),
                signing_mode_fails: Mutex::new(false),
                create_failure: None,
                sign_failure: None,
                rpc_errors: Mutex::new(Vec::new()),
            }
        }

        fn with_create_not_enough_utxos(mut self) -> Self {
            self.create_failure = Some(MockEnvelopeFailure::NotEnoughUtxos);
            self
        }

        fn with_sign_not_enough_utxos(mut self) -> Self {
            self.sign_failure = Some(MockEnvelopeFailure::NotEnoughUtxos);
            self
        }

        fn with_create_prereq_fetch(mut self) -> Self {
            self.create_failure = Some(MockEnvelopeFailure::PrereqFetch);
            self
        }

        fn with_sign_raw_transaction_failure(mut self) -> Self {
            self.sign_failure = Some(MockEnvelopeFailure::SignRawTransaction);
            self
        }

        fn with_sign_other_failure(mut self) -> Self {
            self.sign_failure = Some(MockEnvelopeFailure::Other);
            self
        }

        fn get_stored(&self, idx: u64) -> Option<BundledPayloadEntry> {
            self.stored.lock().unwrap().get(&idx).cloned()
        }

        fn rpc_error_count(&self) -> usize {
            self.rpc_errors.lock().unwrap().len()
        }

        fn set_signing_mode(&self, signing_mode: EnvelopeSigningMode) {
            *self.signing_mode.lock().unwrap() = signing_mode;
        }

        fn set_signing_mode_failure(&self, fails: bool) {
            *self.signing_mode_fails.lock().unwrap() = fails;
        }
    }

    impl WatcherServiceContext for MockWatcherContext {
        async fn get_payload_entry(&self, idx: u64) -> anyhow::Result<Option<BundledPayloadEntry>> {
            Ok(self.stored.lock().unwrap().get(&idx).cloned())
        }

        async fn put_payload_entry(
            &self,
            idx: u64,
            entry: BundledPayloadEntry,
        ) -> anyhow::Result<()> {
            self.stored.lock().unwrap().insert(idx, entry);
            Ok(())
        }

        fn signing_mode(&self) -> anyhow::Result<EnvelopeSigningMode> {
            if *self.signing_mode_fails.lock().unwrap() {
                anyhow::bail!("mock signing mode failure");
            }
            Ok(*self.signing_mode.lock().unwrap())
        }

        async fn create_envelopes(
            &self,
            _idx: u64,
            _entry: &BundledPayloadEntry,
            _envelope_pubkey: XOnlyPublicKey,
        ) -> Result<EnvelopeData, EnvelopeError> {
            if let Some(failure) = self.create_failure {
                return Err(failure.into_error());
            }
            Ok(minimal_envelope_data())
        }

        async fn sign_and_broadcast(
            &self,
            _idx: u64,
            _entry: &BundledPayloadEntry,
        ) -> Result<(L1TxId, L1TxId), EnvelopeError> {
            if let Some(failure) = self.sign_failure {
                return Err(failure.into_error());
            }
            Ok((L1TxId::from([1u8; 32]), L1TxId::from([2u8; 32])))
        }

        async fn complete_reveal_and_broadcast(
            &self,
            idx: u64,
            envelope: &EnvelopeData,
            sig: &[u8; 64],
        ) -> anyhow::Result<L1TxId> {
            complete_reveal_and_broadcast(idx, envelope, sig, &self.broadcast_handle)
                .await
                .map_err(Into::into)
        }

        async fn get_tx_status(&self, _txid: L1TxId) -> anyhow::Result<Option<L1TxEntry>> {
            Ok(None)
        }

        async fn report_status(&self, _entry: &BundledPayloadEntry, _status: &L1BundleStatus) {}

        async fn report_rpc_error(&self, reason: String) {
            self.rpc_errors.lock().unwrap().push(reason);
        }
    }

    fn test_unsigned_entry() -> BundledPayloadEntry {
        let tag = TagData::new(1, 1, vec![]).unwrap();
        let payload = L1Payload::new(vec![vec![1; 150]; 1], tag).unwrap();
        BundledPayloadEntry::new_unsigned(payload)
    }

    #[tokio::test]
    async fn test_unchecked_transitions_to_unpublished() {
        let ctx = MockWatcherContext::new(false);
        let entry = test_unsigned_entry();
        ctx.stored.lock().unwrap().insert(0, entry.clone());

        let mut state = WatcherState::new(ctx, 0);
        state.handle_unsigned_or_needs_resign(entry).await.unwrap();

        let stored = state.ctx.get_stored(0).unwrap();
        assert_eq!(stored.status, L1BundleStatus::Unpublished);
        assert_eq!(stored.commit_txid, L1TxId::from([1u8; 32]));
        assert_eq!(stored.reveal_txid, L1TxId::from([2u8; 32]));
        // No cache entry — ephemeral path does not use the envelope cache
        assert!(state.envelope_cache.is_empty());
    }

    #[tokio::test]
    async fn test_unchecked_not_enough_utxos_keeps_unsigned_for_retry() {
        let ctx = MockWatcherContext::new(false).with_sign_not_enough_utxos();
        let entry = test_unsigned_entry();
        ctx.stored.lock().unwrap().insert(0, entry.clone());

        let mut state = WatcherState::new(ctx, 0);
        state.handle_unsigned_or_needs_resign(entry).await.unwrap();

        let stored = state.ctx.get_stored(0).unwrap();
        assert_eq!(stored.status, L1BundleStatus::Unsigned);
        // Unsigned entries use zero txids as sentinels because no txs have been built yet.
        assert_eq!(stored.commit_txid, L1TxId::zero());
        assert_eq!(stored.reveal_txid, L1TxId::zero());
        assert_eq!(state.curr_payloadidx, 0);
        assert!(state.envelope_cache.is_empty());
    }

    #[tokio::test]
    async fn test_schnorr_key_transitions_to_pending_reveal_sign() {
        let ctx = MockWatcherContext::new(true);
        let entry = test_unsigned_entry();
        ctx.stored.lock().unwrap().insert(0, entry.clone());

        let mut state = WatcherState::new(ctx, 0);
        state.handle_unsigned_or_needs_resign(entry).await.unwrap();

        let stored = state.ctx.get_stored(0).unwrap();
        // Status should carry the sighash from minimal_envelope_data
        assert!(
            matches!(stored.status, L1BundleStatus::PendingRevealTxSign(s) if s == Buf32([42u8; 32]))
        );
        // Envelope is cached for the reveal sig step
        assert!(state.envelope_cache.contains_key(&0));
    }

    #[tokio::test]
    async fn test_schnorr_key_not_enough_utxos_keeps_unsigned_for_retry() {
        let ctx = MockWatcherContext::new(true).with_create_not_enough_utxos();
        let entry = test_unsigned_entry();
        ctx.stored.lock().unwrap().insert(0, entry.clone());

        let mut state = WatcherState::new(ctx, 0);
        state.handle_unsigned_or_needs_resign(entry).await.unwrap();

        let stored = state.ctx.get_stored(0).unwrap();
        assert_eq!(stored.status, L1BundleStatus::Unsigned);
        // Unsigned entries use zero txids as sentinels because no txs have been built yet.
        assert_eq!(stored.commit_txid, L1TxId::zero());
        assert_eq!(stored.reveal_txid, L1TxId::zero());
        assert_eq!(state.curr_payloadidx, 0);
        assert!(state.envelope_cache.is_empty());
    }

    #[tokio::test]
    async fn test_schnorr_key_prereq_fetch_keeps_unsigned_for_retry() {
        let ctx = MockWatcherContext::new(true).with_create_prereq_fetch();
        let entry = test_unsigned_entry();
        ctx.stored.lock().unwrap().insert(0, entry);

        let mut state = WatcherState::new(ctx, 0);
        let response = WatcherService::<MockWatcherContext>::process_input(&mut state, ())
            .await
            .unwrap();

        let stored = state.ctx.get_stored(0).unwrap();
        assert!(matches!(response, Response::Continue));
        assert_eq!(stored.status, L1BundleStatus::Unsigned);
        assert_eq!(state.curr_payloadidx, 0);
        assert!(state.envelope_cache.is_empty());
        assert_eq!(state.ctx.rpc_error_count(), 1);
    }

    #[tokio::test]
    async fn test_unchecked_sign_raw_transaction_keeps_unsigned_for_retry() {
        let ctx = MockWatcherContext::new(false).with_sign_raw_transaction_failure();
        let entry = test_unsigned_entry();
        ctx.stored.lock().unwrap().insert(0, entry);

        let mut state = WatcherState::new(ctx, 0);
        let response = WatcherService::<MockWatcherContext>::process_input(&mut state, ())
            .await
            .unwrap();

        let stored = state.ctx.get_stored(0).unwrap();
        assert!(matches!(response, Response::Continue));
        assert_eq!(stored.status, L1BundleStatus::Unsigned);
        assert_eq!(state.curr_payloadidx, 0);
        assert!(state.envelope_cache.is_empty());
        assert_eq!(state.ctx.rpc_error_count(), 1);
    }

    #[tokio::test]
    async fn test_unchecked_other_error_exits_watcher() {
        let ctx = MockWatcherContext::new(false).with_sign_other_failure();
        let entry = test_unsigned_entry();
        ctx.stored.lock().unwrap().insert(0, entry);

        let mut state = WatcherState::new(ctx, 0);
        let err = WatcherService::<MockWatcherContext>::process_input(&mut state, ())
            .await
            .unwrap_err();

        assert!(err.to_string().contains("mock storage failure"));
        assert_eq!(
            state.ctx.get_stored(0).unwrap().status,
            L1BundleStatus::Unsigned
        );
        assert_eq!(state.ctx.rpc_error_count(), 0);
    }

    #[tokio::test]
    async fn test_signing_mode_error_defers_then_recovers() {
        let ctx = MockWatcherContext::new(true);
        ctx.set_signing_mode_failure(true);
        let entry = test_unsigned_entry();
        ctx.stored.lock().unwrap().insert(0, entry);

        let mut state = WatcherState::new(ctx, 0);

        // An unresolvable signing mode (transient DB error, or a non-signable
        // predicate after a rotation) must defer to the next tick, not kill the
        // writer. The service treats a `process_input` error as terminal.
        let response = WatcherService::<MockWatcherContext>::process_input(&mut state, ())
            .await
            .unwrap();
        assert!(matches!(response, Response::Continue));
        assert_eq!(
            state.ctx.get_stored(0).unwrap().status,
            L1BundleStatus::Unsigned
        );
        assert!(state.envelope_cache.is_empty());
        // A signing-mode failure is not a Bitcoin RPC error, so none is reported.
        assert_eq!(state.ctx.rpc_error_count(), 0);

        // Once the signing mode resolves again, the tick loop makes progress.
        state.ctx.set_signing_mode_failure(false);
        let response = WatcherService::<MockWatcherContext>::process_input(&mut state, ())
            .await
            .unwrap();
        assert!(matches!(response, Response::Continue));
        assert!(matches!(
            state.ctx.get_stored(0).unwrap().status,
            L1BundleStatus::PendingRevealTxSign(_)
        ));
    }

    #[tokio::test]
    async fn test_pending_reveal_resets_when_signing_mode_changes_without_sig() {
        let ctx = MockWatcherContext::new(true);

        let envelope = minimal_envelope_data();
        let mut entry = test_unsigned_entry();
        entry.status = L1BundleStatus::PendingRevealTxSign(Buf32([42u8; 32]));
        ctx.stored.lock().unwrap().insert(0, entry.clone());

        let mut state = WatcherState::new(ctx, 0);
        state.envelope_cache.insert(0, envelope);
        state.ctx.set_signing_mode(EnvelopeSigningMode::InProcess);

        state.handle_pending_reveal_tx_sign(entry).await.unwrap();

        let stored = state.ctx.get_stored(0).unwrap();
        assert_eq!(stored.status, L1BundleStatus::Unsigned);
        assert!(!state.envelope_cache.contains_key(&0));
    }

    #[tokio::test]
    async fn test_schnorr_key_reveal_sig_transitions_to_unpublished() {
        let ctx = MockWatcherContext::new(true);
        let bcast_handle = ctx.broadcast_handle.clone();

        let envelope = minimal_envelope_data();
        let commit_txid: Buf32 = envelope.commit_tx.compute_txid().to_buf32();
        let reveal_txid: Buf32 = envelope.reveal_tx.compute_txid().to_buf32();

        // Set up entry already in PendingRevealTxSign with a signature present
        let mut entry = test_unsigned_entry();
        entry.status = L1BundleStatus::PendingRevealTxSign(Buf32([42u8; 32]));
        entry.payload_signature = Some(Buf64([1u8; 64]));
        ctx.stored.lock().unwrap().insert(0, entry.clone());

        let mut state = WatcherState::new(ctx, 0);
        state.envelope_cache.insert(0, envelope);

        state.handle_pending_reveal_tx_sign(entry).await.unwrap();

        let stored = state.ctx.get_stored(0).unwrap();
        assert_eq!(stored.status, L1BundleStatus::Unpublished);
        // Cache entry consumed
        assert!(!state.envelope_cache.contains_key(&0));
        // Both txs stored in broadcaster DB
        assert!(bcast_handle
            .get_tx_entry_by_id_async(commit_txid)
            .await
            .unwrap()
            .is_some());
        assert!(bcast_handle
            .get_tx_entry_by_id_async(reveal_txid)
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn test_pending_reveal_resets_when_signing_mode_changes_with_sig() {
        let ctx = MockWatcherContext::new(true);

        let envelope = minimal_envelope_data();
        let commit_txid: Buf32 = envelope.commit_tx.compute_txid().to_buf32();
        let mut entry = test_unsigned_entry();
        entry.status = L1BundleStatus::PendingRevealTxSign(Buf32([42u8; 32]));
        entry.payload_signature = Some(Buf64([1u8; 64]));
        ctx.stored.lock().unwrap().insert(0, entry.clone());

        let mut state = WatcherState::new(ctx, 0);
        state.envelope_cache.insert(0, envelope);
        state.ctx.set_signing_mode(EnvelopeSigningMode::InProcess);

        state.handle_pending_reveal_tx_sign(entry).await.unwrap();

        let stored = state.ctx.get_stored(0).unwrap();
        assert_eq!(stored.status, L1BundleStatus::Unsigned);
        assert!(!state.envelope_cache.contains_key(&0));
        assert!(state
            .ctx
            .broadcast_handle
            .get_tx_entry_by_id_async(commit_txid)
            .await
            .unwrap()
            .is_none());
    }
}
