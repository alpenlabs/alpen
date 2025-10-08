use bitcoin::Transaction;
use strata_checkpoint_types::{verify_signed_checkpoint_sig, SignedCheckpoint};
use strata_ol_chainstate_types::Chainstate;
use strata_primitives::l1::payload::L1PayloadType;
use tracing::warn;

use super::TxFilterConfig;
use crate::envelope::parser::parse_envelope_payload;

/// Parses envelope from the given transaction. Currently, the only envelope recognizable is
/// the checkpoint envelope.
// TODO: we need to change envelope structure and possibly have envelopes for checkpoints and
// DA separately
pub fn parse_valid_checkpoint_envelope(
    tx: &Transaction,
    filter_conf: &TxFilterConfig,
) -> Option<SignedCheckpoint> {
    tx.input.iter().find_map(|inp| {
        inp.witness
            .taproot_leaf_script()
            .and_then(|scr| parse_envelope_payload(&scr.script.into()).ok())
            .and_then(|data| parse_and_validate_checkpoint(&data, filter_conf))
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
    use strata_checkpoint_types::{Checkpoint, CheckpointSidecar, SignedCheckpoint};
    use strata_ol_chainstate_types::Chainstate;
    use strata_primitives::{l1::payload::L1Payload, params::Params};
    use strata_test_utils::ArbitraryGenerator;
    use strata_test_utils_l2::gen_params;

    use super::TxFilterConfig;
    use crate::filter::parse_valid_checkpoint_envelope;

    const TEST_ADDR: &str = "bcrt1q6u6qyya3sryhh42lahtnz2m7zuufe7dlt8j0j5";

    /// Helper function to create filter config
    fn create_tx_filter_config(params: &Params) -> TxFilterConfig {
        TxFilterConfig::derive_from(params.rollup()).expect("can't get filter config")
    }

    #[test]
    fn test_parse_envelope() {
        // Test with valid name
        let params: Params = gen_params();
        let filter_config = create_tx_filter_config(&params);

        // Testing envelope is parsed
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
        let l1_payload = L1Payload::new_checkpoint(borsh::to_vec(&signed_checkpoint).unwrap());

        let tx = create_checkpoint_envelope_tx(TEST_ADDR, l1_payload.clone());
        let checkpoint = parse_valid_checkpoint_envelope(&tx, &filter_config).unwrap();

        assert_eq!(signed_checkpoint, checkpoint);
    }

    #[test]
    fn test_parse_envelopes_invalid_chainstate() {
        // Test with valid name
        let params: Params = gen_params();
        let filter_config = create_tx_filter_config(&params);

        // Testing envelope is parsed
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
        let l1_payload = L1Payload::new_checkpoint(borsh::to_vec(&signed_checkpoint).unwrap());
        let tx = create_checkpoint_envelope_tx(TEST_ADDR, l1_payload);
        let res = parse_valid_checkpoint_envelope(&tx, &filter_config);

        assert!(res.is_none(), "There should be no envelopes");
    }
}
