//! Per-duty signing handlers dispatched by the signer service.

use std::sync::Arc;

use bitcoin::{
    key::UntweakedKeypair,
    secp256k1::{Message, SECP256K1, SecretKey},
};
use jsonrpsee::core::ClientError;
use rand::{RngCore, rngs::OsRng};
use strata_common::ws_client::ManagedWsClient;
use strata_ol_rpc_api::OLSequencerRpcClient;
use strata_ol_sequencer::{
    BlockCompletionData, BlockSigningDuty, CheckpointSigningDuty, Duty, PayloadSigningDuty,
    sign_checkpoint, sign_header,
};
use strata_primitives::{HexBytes32, HexBytes64, buf::Buf32};
use thiserror::Error;
use tokio::{sync::mpsc, time};
use tracing::{debug, error, info};

use crate::helpers::SequencerSk;

#[derive(Debug, Error)]
#[expect(clippy::enum_variant_names, reason = "pre-existing naming convention")]
enum DutyExecError {
    #[error("failed completing block template: {0}")]
    CompleteTemplate(#[source] ClientError),

    #[error("failed submitting checkpoint signature: {0}")]
    CompleteCheckpoint(#[source] ClientError),

    #[error("failed submitting payload signature: {0}")]
    CompletePayload(#[source] ClientError),
}

/// Dispatches a duty to the appropriate signing handler and reports failures.
pub(crate) async fn handle_duty(
    rpc: Arc<ManagedWsClient>,
    duty: Duty,
    sk: SequencerSk,
    failed_tx: mpsc::Sender<Buf32>,
) {
    let duty_id = duty.generate_id();
    debug!(%duty_id, %duty, "handling duty");

    // Borrow the key bytes for signing. `Buf32` is a brief stack copy scoped
    // to this function; the authoritative key lives in the `Arc` allocation.
    let sk_buf = Buf32(**sk);
    let result = match duty {
        Duty::SignBlock(block_duty) => handle_sign_block(&rpc, block_duty, &sk_buf).await,
        Duty::SignCheckpoint(cp_duty) => handle_sign_checkpoint(&rpc, cp_duty, &sk_buf).await,
        Duty::SignPayload(payload_duty) => handle_sign_payload(&rpc, payload_duty, &sk_buf).await,
    };

    if let Err(err) = result {
        error!(%duty_id, %err, "duty failed");
        let _ = failed_tx.send(duty_id).await;
    }
}

async fn handle_sign_block(
    rpc: &ManagedWsClient,
    duty: BlockSigningDuty,
    sk: &Buf32,
) -> Result<(), DutyExecError> {
    // TODO: recheck this logic
    if let Some(wait) = duty.wait_duration() {
        debug!(wait_ms = %wait.as_millis(), "waiting for block target time");
        time::sleep(wait).await;
    }

    let template_id = duty.template_id();
    let sig = sign_header(duty.template.header(), sk);
    let completion = BlockCompletionData::from_signature(sig);

    rpc.complete_block_template(template_id, completion)
        .await
        .map_err(DutyExecError::CompleteTemplate)?;

    info!(%template_id, "block signed and submitted");
    Ok(())
}

async fn handle_sign_checkpoint(
    rpc: &ManagedWsClient,
    duty: CheckpointSigningDuty,
    sk: &Buf32,
) -> Result<(), DutyExecError> {
    let epoch = duty.epoch();
    let sig = sign_checkpoint(duty.checkpoint(), sk);

    debug!(%epoch, %sig, "signed checkpoint");

    rpc.complete_checkpoint_signature(epoch, HexBytes64(sig.0))
        .await
        .map_err(DutyExecError::CompleteCheckpoint)?;

    info!(%epoch, "checkpoint signature submitted");
    Ok(())
}

async fn handle_sign_payload(
    rpc: &ManagedWsClient,
    duty: PayloadSigningDuty,
    sk: &Buf32,
) -> Result<(), DutyExecError> {
    let secret_key = SecretKey::from_slice(&sk.0).expect("valid secret key");
    let keypair = UntweakedKeypair::from_secret_key(SECP256K1, &secret_key);

    let msg = Message::from_digest_slice(duty.sighash.as_ref()).expect("sighash is valid 32 bytes");

    let mut aux_rand = [0u8; 32];
    OsRng.fill_bytes(&mut aux_rand);
    let sig = SECP256K1.sign_schnorr_with_aux_rand(&msg, &keypair, &aux_rand);

    let payload_idx = duty.payload_idx;
    debug!(%payload_idx, "signed payload envelope sighash");

    rpc.complete_payload_signature(
        payload_idx,
        HexBytes32(duty.sighash.0),
        HexBytes64(sig.serialize()),
    )
    .await
    .map_err(DutyExecError::CompletePayload)?;

    info!(%payload_idx, "payload signature submitted");
    Ok(())
}
