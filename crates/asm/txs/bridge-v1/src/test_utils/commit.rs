use bitcoin::Transaction;
use strata_l1_txfmt::ParseConfig;

use crate::{
    commit::CommitInfo,
    test_utils::{TEST_MAGIC_BYTES, create_dummy_tx},
};

/// Creates a commit transaction for testing purposes.
pub fn create_test_commit_tx(commit_info: &CommitInfo) -> Transaction {
    // Create a dummy tx with one input and two outputs
    let mut tx = create_dummy_tx(1, 2);

    // Encode auxiliary data and construct SPS 50 op_return script
    let td = commit_info
        .header_aux()
        .build_tag_data()
        .expect("commit header must serialize");
    let sps_50_script = ParseConfig::new(*TEST_MAGIC_BYTES)
        .encode_script_buf(&td.as_ref())
        .expect("encoding SPS50 script must succeed");

    // The first output is SPS 50 header
    tx.output[0].script_pubkey = sps_50_script;

    // The second output is the payout script
    tx.output[1].script_pubkey = commit_info.second_output_script().clone();

    // The first input is the stake connector
    tx.input[0].previous_output = *commit_info.first_input_outpoint().outpoint();

    tx
}
