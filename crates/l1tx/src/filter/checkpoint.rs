use bitcoin::Transaction;
use strata_primitives::{batch::SignedCheckpoint, l1::payload::L1PayloadType};
use strata_state::{batch::verify_signed_checkpoint_sig, chain_state::Chainstate};
use tracing::warn;

use super::TxFilterConfig;
use crate::envelope::parser::parse_envelope_payloads;

/// Parses envelope from the given transaction. Currently, the only envelope recognizable is
/// the checkpoint envelope.
// TODO: we need to change envelope structure and possibly have envelopes for checkpoints and
// DA separately
pub(crate) fn parse_valid_checkpoint_envelopes<'a>(
    tx: &'a Transaction,
    filter_conf: &'a TxFilterConfig,
) -> impl Iterator<Item = SignedCheckpoint> + 'a {
    tx.input.iter().flat_map(move |inp| {
        inp.witness
            .taproot_leaf_script()
            .and_then(|scr| parse_envelope_payloads(&scr.script.into(), filter_conf).ok())
            .map(|items| {
                items
                    .into_iter()
                    .filter_map(|item| match *item.payload_type() {
                        L1PayloadType::Checkpoint => {
                            parse_and_validate_checkpoint(item.data(), filter_conf)
                        }
                        L1PayloadType::Da => {
                            warn!("Da parsing is not supported yet");
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    })
}

fn parse_and_validate_checkpoint(
    data: &[u8],
    filter_conf: &TxFilterConfig,
) -> Option<SignedCheckpoint> {
    // Parse
    let signed_checkpoint = borsh::from_slice::<SignedCheckpoint>(data).ok()?;

    validate_checkpoint(signed_checkpoint, filter_conf)
}

fn validate_checkpoint(
    signed_checkpoint: SignedCheckpoint,
    filter_conf: &TxFilterConfig,
) -> Option<SignedCheckpoint> {
    if !verify_signed_checkpoint_sig(&signed_checkpoint, &filter_conf.sequencer_cred_rule) {
        warn!("invalid checkpoint signature");
        return None;
    }

    if let Err(err) =
        borsh::from_slice::<Chainstate>(signed_checkpoint.checkpoint().sidecar().chainstate())
    {
        warn!(?err, "invalid chainstate in checkpoint");
        return None;
    }

    Some(signed_checkpoint)
}

#[cfg(test)]
mod test {
    use strata_btcio::test_utils::create_checkpoint_envelope_tx;
    use strata_primitives::{l1::payload::L1Payload, params::Params};
    use strata_state::{
        batch::{Checkpoint, CheckpointSidecar, SignedCheckpoint},
        chain_state::Chainstate,
    };
    use strata_test_utils::{l2::gen_params, ArbitraryGenerator};

    use super::TxFilterConfig;
    use crate::filter::parse_valid_checkpoint_envelopes;

    const TEST_ADDR: &str = "bcrt1q6u6qyya3sryhh42lahtnz2m7zuufe7dlt8j0j5";

    /// Helper function to create filter config
    fn create_tx_filter_config(params: &Params) -> TxFilterConfig {
        TxFilterConfig::derive_from(params.rollup()).expect("can't get filter config")
    }

    #[test]
    fn test_parse_envelopes() {
        // Test with valid name
        let mut params: Params = gen_params();
        let filter_config = create_tx_filter_config(&params);

        // Testing multiple envelopes are parsed
        let num_envelopes = 2;
        let l1_payloads: Vec<_> = (0..num_envelopes)
            .map(|_| {
                let mut gen = ArbitraryGenerator::new();
                let chainstate: Chainstate = gen.generate();
                let signed_checkpoint = SignedCheckpoint::new(
                    Checkpoint::new(
                        gen.generate(),
                        gen.generate(),
                        gen.generate(),
                        CheckpointSidecar::new(borsh::to_vec(&chainstate).unwrap()),
                    ),
                    gen.generate(),
                );
                L1Payload::new_checkpoint(borsh::to_vec(&signed_checkpoint).unwrap())
            })
            .collect();
        let tx = create_checkpoint_envelope_tx(&params, TEST_ADDR, l1_payloads.clone());
        let checkpoints: Vec<_> = parse_valid_checkpoint_envelopes(&tx, &filter_config).collect();

        assert_eq!(checkpoints.len(), 2, "Should filter relevant envelopes");

        // Test with invalid checkpoint tag
        params.rollup.checkpoint_tag = "invalid_checkpoint_tag".to_string();

        let tx = create_checkpoint_envelope_tx(&params, TEST_ADDR, l1_payloads);
        let checkpoints: Vec<_> = parse_valid_checkpoint_envelopes(&tx, &filter_config).collect();
        assert!(checkpoints.is_empty(), "There should be no envelopes");
    }

    #[test]
    fn test_parse_envelopes_invalid_chainstate() {
        // Test with valid name
        let params: Params = gen_params();
        let filter_config = create_tx_filter_config(&params);

        // Testing multiple envelopes are parsed
        let num_envelopes = 2;
        let l1_payloads: Vec<_> = (0..num_envelopes)
            .map(|_| {
                let mut gen = ArbitraryGenerator::new();
                let invalid_chainstate: [u8; 100] = gen.generate();
                let signed_checkpoint = SignedCheckpoint::new(
                    Checkpoint::new(
                        gen.generate(),
                        gen.generate(),
                        gen.generate(),
                        CheckpointSidecar::new(borsh::to_vec(&invalid_chainstate).unwrap()),
                    ),
                    gen.generate(),
                );
                L1Payload::new_checkpoint(borsh::to_vec(&signed_checkpoint).unwrap())
            })
            .collect();
        let tx = create_checkpoint_envelope_tx(&params, TEST_ADDR, l1_payloads.clone());
        let checkpoints: Vec<_> = parse_valid_checkpoint_envelopes(&tx, &filter_config).collect();

        assert!(checkpoints.is_empty(), "There should be no envelopes");
    }
}
