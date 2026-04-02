//! Per-duty signing handlers dispatched by the signer service.

use std::{sync::Arc, thread};

use jsonrpsee::core::ClientError;
use strata_common::ws_client::ManagedWsClient;
use strata_ol_rpc_api::OLSequencerRpcClient;
use strata_ol_sequencer::{
    BlockCompletionData, Duty, sign_checkpoint, sign_header, sign_reveal_tx,
};
use strata_primitives::{
    HexBytes32, HexBytes64,
    buf::{Buf32, Buf64},
};
use thiserror::Error;
use tokio::sync::mpsc;
use tracing::{debug, error, info};
use zeroize::Zeroize;

use crate::helpers::SequencerSk;

#[derive(Debug, Error)]
#[expect(clippy::enum_variant_names, reason = "pre-existing naming convention")]
enum DutyExecError {
    #[error("failed completing block template: {0}")]
    CompleteTemplate(#[source] ClientError),

    #[error("failed submitting checkpoint signature: {0}")]
    CompleteCheckpoint(#[source] ClientError),

    #[error("failed submitting reveal tx signature: {0}")]
    CompleteRevealTx(#[source] ClientError),
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

    // Sign synchronously so key bytes never enter the async state machine.
    let signed = sign_duty(&duty, &sk);

    let result = match signed {
        SignedDuty::Block {
            template_id,
            completion,
        } => complete_block_duty(&rpc, template_id, completion).await,
        SignedDuty::Checkpoint { epoch, sig } => complete_checkpoint_duty(&rpc, epoch, sig).await,
        SignedDuty::RevealTx {
            payload_idx,
            sighash,
            sig,
        } => complete_reveal_tx_duty(&rpc, payload_idx, sighash, sig).await,
    };

    if let Err(err) = result {
        error!(%duty_id, %err, "duty failed");
        let _ = failed_tx.send(duty_id).await;
    }
}

/// Outcome of synchronous signing — holds only the data needed for async submission.
enum SignedDuty {
    Block {
        template_id: strata_primitives::OLBlockId,
        completion: BlockCompletionData,
    },
    Checkpoint {
        epoch: u32,
        sig: Buf64,
    },
    RevealTx {
        payload_idx: u64,
        sighash: Buf32,
        sig: Buf64,
    },
}

/// Signs a duty synchronously. Key bytes stay on the sync stack and are zeroized before returning.
fn sign_duty(duty: &Duty, sk: &SequencerSk) -> SignedDuty {
    let mut sk_buf = Buf32(***sk);
    let result = match duty {
        Duty::SignBlock(duty) => {
            if let Some(wait) = duty.wait_duration() {
                debug!(wait_ms = %wait.as_millis(), "waiting for block target time");
                thread::sleep(wait);
            }
            let sig = sign_header(duty.template.header(), &sk_buf);
            SignedDuty::Block {
                template_id: duty.template_id(),
                completion: BlockCompletionData::from_signature(sig),
            }
        }
        Duty::SignCheckpoint(duty) => {
            let sig = sign_checkpoint(duty.checkpoint(), &sk_buf);
            debug!(epoch = %duty.epoch(), %sig, "signed checkpoint");
            SignedDuty::Checkpoint {
                epoch: duty.epoch(),
                sig,
            }
        }
        Duty::SignRevealTx(duty) => {
            let sig = sign_reveal_tx(&duty.sighash, &sk_buf);
            debug!(payload_idx = %duty.payload_idx, "signed payload envelope sighash");
            SignedDuty::RevealTx {
                payload_idx: duty.payload_idx,
                sighash: duty.sighash,
                sig,
            }
        }
    };
    sk_buf.zeroize();
    result
}

async fn complete_block_duty(
    rpc: &ManagedWsClient,
    template_id: strata_primitives::OLBlockId,
    completion: BlockCompletionData,
) -> Result<(), DutyExecError> {
    rpc.complete_block_template(template_id, completion)
        .await
        .map_err(DutyExecError::CompleteTemplate)?;
    info!(%template_id, "block signed and submitted");
    Ok(())
}

async fn complete_checkpoint_duty(
    rpc: &ManagedWsClient,
    epoch: u32,
    sig: Buf64,
) -> Result<(), DutyExecError> {
    rpc.complete_checkpoint_signature(epoch, HexBytes64(sig.0))
        .await
        .map_err(DutyExecError::CompleteCheckpoint)?;
    info!(%epoch, "checkpoint signature submitted");
    Ok(())
}

async fn complete_reveal_tx_duty(
    rpc: &ManagedWsClient,
    payload_idx: u64,
    sighash: Buf32,
    sig: Buf64,
) -> Result<(), DutyExecError> {
    rpc.complete_payload_signature(payload_idx, HexBytes32(sighash.0), HexBytes64(sig.0))
        .await
        .map_err(DutyExecError::CompleteRevealTx)?;
    info!(%payload_idx, "payload signature submitted");
    Ok(())
}
