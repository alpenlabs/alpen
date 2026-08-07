//! Checkpoint publication decisions derived from existing payload and ASM state.

use std::{fmt, sync::Arc};

use bitcoin::Transaction;
use strata_asm_checkpoint_types::CheckpointPayload;
use strata_asm_common::{SectionStateExt, Subprotocol, TxInputRef};
use strata_asm_proto_checkpoint::CheckpointSubprotocol;
use strata_asm_proto_checkpoint_txs::{OL_STF_CHECKPOINT_TX_TAG, extract_checkpoint_from_envelope};
use strata_btcio::broadcaster::{BroadcasterError, PublishDecision, PublishPolicy};
use strata_codec::decode_buf_exact;
use strata_codec_utils::CodecSsz;
use strata_csm_types::L1Payload;
use strata_identifiers::Epoch;
use strata_l1_txfmt::{MagicBytes, ParseConfig};
use strata_storage::NodeStorage;
use tracing::warn;

fn is_accepted_checkpoint(checkpoint: &CheckpointPayload, verified_epoch: Epoch) -> bool {
    checkpoint.new_tip().epoch <= verified_epoch
}

fn checkpoint_decision(
    checkpoint: &CheckpointPayload,
    verified_epoch: Option<Epoch>,
) -> PublishDecision {
    match verified_epoch {
        Some(epoch) if is_accepted_checkpoint(checkpoint, epoch) => PublishDecision::Abandon,
        Some(_) => PublishDecision::Publish,
        None => PublishDecision::Defer,
    }
}

/// Strata's checkpoint-aware decision policy for the generic L1 broadcaster.
pub(crate) struct CheckpointPublishPolicy {
    storage: Arc<NodeStorage>,
    magic_bytes: MagicBytes,
}

impl fmt::Debug for CheckpointPublishPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CheckpointPublishPolicy")
            .finish_non_exhaustive()
    }
}

impl CheckpointPublishPolicy {
    pub(crate) fn new(storage: Arc<NodeStorage>, magic_bytes: MagicBytes) -> Self {
        Self {
            storage,
            magic_bytes,
        }
    }

    fn decide_checkpoint(&self, checkpoint: &CheckpointPayload) -> PublishDecision {
        let asm_state = match self.storage.fetch_canonical_asm_state_blocking() {
            Ok(Some((_, state))) => state,
            Ok(None) => return checkpoint_decision(checkpoint, None),
            Err(err) => {
                warn!(%err, "could not read canonical ASM state; deferring checkpoint publication");
                return PublishDecision::Defer;
            }
        };
        let Some(section) = asm_state
            .state()
            .find_section(<CheckpointSubprotocol as Subprotocol>::ID)
        else {
            warn!("ASM checkpoint section is missing; deferring checkpoint publication");
            return PublishDecision::Defer;
        };
        let state = match section.try_to_state::<CheckpointSubprotocol>() {
            Ok(state) => state,
            Err(err) => {
                warn!(%err, "could not decode ASM checkpoint section; deferring checkpoint publication");
                return PublishDecision::Defer;
            }
        };
        checkpoint_decision(checkpoint, Some(state.verified_tip().epoch))
    }
}

impl PublishPolicy for CheckpointPublishPolicy {
    fn decide(&self, tx: &Transaction) -> Result<PublishDecision, BroadcasterError> {
        let Ok(tag) = ParseConfig::new(self.magic_bytes).try_parse_tx(tx) else {
            return Ok(PublishDecision::Publish);
        };
        if tag.subproto_id() != OL_STF_CHECKPOINT_TX_TAG.subproto_id()
            || tag.tx_type() != OL_STF_CHECKPOINT_TX_TAG.tx_type()
            || tag.aux_data() != OL_STF_CHECKPOINT_TX_TAG.aux_data()
        {
            return Ok(PublishDecision::Publish);
        }
        let Ok(envelope) = extract_checkpoint_from_envelope(&TxInputRef::new(tx, tag)) else {
            return Ok(PublishDecision::Publish);
        };
        Ok(self.decide_checkpoint(&envelope.payload))
    }

    fn decide_payload(&self, payload: &L1Payload) -> PublishDecision {
        let tag = OL_STF_CHECKPOINT_TX_TAG.as_ref();
        if payload.tag().subproto_id() != tag.subproto_id()
            || payload.tag().tx_type() != tag.tx_type()
            || payload.tag().aux_data() != tag.aux_data()
        {
            return PublishDecision::Publish;
        }
        let Ok(checkpoint) =
            decode_buf_exact::<CodecSsz<CheckpointPayload>>(&payload.data().concat())
        else {
            return PublishDecision::Publish;
        };
        self.decide_checkpoint(&checkpoint.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use strata_asm_checkpoint_types::test_utils::create_test_checkpoint_payload;

    use super::*;

    #[test]
    fn checkpoint_decision_requires_a_known_tip_and_abandons_accepted_epochs() {
        let checkpoint = create_test_checkpoint_payload(14);
        assert_eq!(
            checkpoint_decision(&checkpoint, None),
            PublishDecision::Defer
        );
        assert_eq!(
            checkpoint_decision(&checkpoint, Some(Epoch::from(13u32))),
            PublishDecision::Publish
        );
        assert_eq!(
            checkpoint_decision(&checkpoint, Some(Epoch::from(14u32))),
            PublishDecision::Abandon
        );
        assert_eq!(
            checkpoint_decision(&checkpoint, Some(Epoch::from(28u32))),
            PublishDecision::Abandon
        );
    }
}
