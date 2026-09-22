//! Checkpoint guest execution benchmark using a deterministic mixed epoch.

mod fixture;

use std::{env, fs, process::Command};

use anyhow::{Context, Result};
use fixture::{prepare_checkpoint_input, stored_predicate};
use strata_identifiers::Buf32;
use strata_ol_params::OLRuntimeParams;
use strata_proofimpl_checkpoint::program::{CheckpointProgram, CheckpointProverInput};
#[cfg(feature = "sp1")]
use zkaleido::{ExecutionSummary, ZkVmHost, ZkVmProgram};

fn prepare_report() -> CheckpointProverInput {
    print_environment();
    let predicate = stored_predicate();
    let input = prepare_checkpoint_input(&predicate);
    let transaction_count: usize = input
        .blocks
        .iter()
        .filter_map(|block| block.body().tx_segment())
        .map(|segment| segment.txs().len())
        .sum();
    println!(
        "Checkpoint workload: mixed-epoch-v1; deterministic, no random inputs\n\
         accounts={}; blocks={}; transactions={}; da_bytes={}\n\
         stored_predicate=Sp1Groth16; condition_bytes={}; synthetic program ID\n\
         active_update_predicate=AlwaysAccept; Groth16 proof verification is not measured\n\
         runtime_params_hash={:?}\n\
         parent_state_root={:?}; terminal_state_root={:?}",
        input.start_state.iter_account_states().count(),
        input.blocks.len(),
        transaction_count,
        input.da_state_diff_bytes.len(),
        predicate.condition().len(),
        Buf32::from(OLRuntimeParams::test_default().hash()),
        input.parent.state_root(),
        input
            .blocks
            .last()
            .expect("nonempty benchmark epoch")
            .header()
            .state_root(),
    );
    input
}

fn print_environment() {
    println!(
        "Benchmark platform: {} / {}",
        env::consts::OS,
        env::consts::ARCH
    );
    if let Ok(output) = Command::new("rustc").arg("--version").output() {
        println!(
            "Rust toolchain: {}",
            String::from_utf8_lossy(&output.stdout).trim()
        );
    }
    if let Ok(output) = Command::new("cargo").args(["prove", "--version"]).output() {
        if output.status.success() {
            println!(
                "SP1 toolchain: {}",
                String::from_utf8_lossy(&output.stdout).trim()
            );
        }
    }
    for name in [
        "GITHUB_SHA",
        "GITHUB_HEAD_REF",
        "GITHUB_BASE_REF",
        "RUNNER_NAME",
        "SP1_PROVER",
    ] {
        if let Ok(value) = env::var(name) {
            println!("{name}={value}");
        }
    }
    if let Ok(path) = env::var("GITHUB_EVENT_PATH") {
        match read_github_head_sha(&path) {
            Ok(Some(head)) => println!("PR head SHA: {head}"),
            Ok(None) => {}
            Err(error) => eprintln!("GitHub PR metadata unavailable: {error:#}"),
        }
    }
}

fn read_github_head_sha(path: &str) -> Result<Option<String>> {
    let file = fs::File::open(path).context("open GitHub event metadata")?;
    let event: serde_json::Value =
        serde_json::from_reader(file).context("parse GitHub event metadata")?;
    Ok(event
        .pointer("/pull_request/head/sha")
        .and_then(|value| value.as_str())
        .map(str::to_owned))
}

#[cfg(feature = "sp1")]
pub(crate) fn gen_perf_report(host: &impl ZkVmHost) -> ExecutionSummary {
    let input = prepare_report();
    <CheckpointProgram as ZkVmProgram>::execute(&input, host).expect("checkpoint execution")
}

#[cfg(test)]
mod tests {
    use strata_acct_types::BitcoinAmount;
    use strata_codec::decode_buf_exact;
    use strata_ol_da_types_v1::{OLDaPayloadV1, OLDaSchemeV1};
    use strata_ol_state_support_types::MemoryStateBaseLayer;
    use strata_ol_state_types::{IAccountState, ISnarkAccountState, IStateAccessor, OLSpecId};
    use strata_ol_stf_v1::{
        test_utils::{make_account_id, TEST_RECIPIENT_ID, TEST_SNARK_ACCOUNT_ID},
        verify_block, verify_epoch_with_diff, BlockInfo, EpochExecExpectations, EpochInfo,
    };

    use super::*;

    #[test]
    fn test_mixed_checkpoint_matches_block_verification_and_da_reconstruction() {
        let predicate = stored_predicate();
        let input = prepare_checkpoint_input(&predicate);
        assert_eq!(input.start_state.iter_account_states().count(), 64);
        assert_eq!(input.blocks.len(), 5);
        let runtime_params = OLRuntimeParams::test_default();
        let ci_params: OLRuntimeParams = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../.github/fixtures/checkpoint-runtime-params.json"
        )))
        .unwrap();
        assert_eq!(runtime_params, ci_params);

        let mut verified = MemoryStateBaseLayer::new(input.start_state.clone());
        let mut parent = &input.parent;
        let mut manifests = Vec::new();
        for block in &input.blocks {
            verify_block(
                &mut verified,
                block.header(),
                Some(parent),
                block.body(),
                &runtime_params,
            )
            .unwrap();
            if let Some(container) = block.body().manifests() {
                manifests.extend_from_slice(container.manifests());
            }
            parent = block.header();
        }
        let terminal = input.blocks.last().unwrap().header();
        assert_eq!(
            verified.compute_state_root().unwrap(),
            *terminal.state_root()
        );

        // Assert the effects, so a regression to an empty workload cannot pass by
        // making both execution and reconstruction do the same trivial work.
        let sender = verified
            .get_account_state(make_account_id(TEST_SNARK_ACCOUNT_ID))
            .unwrap()
            .unwrap();
        assert_eq!(sender.balance().to_sat(), 272_000_000);
        let snark = sender.as_snark_account().unwrap();
        assert_eq!(*snark.seqno().inner(), 2);
        assert_eq!(snark.next_inbox_msg_idx(), 1);
        assert_eq!(snark.inbox_mmr().entries, 2);
        assert_eq!(snark.update_vk(), &predicate);
        let inbox_recipient = verified
            .get_account_state(make_account_id(1_000))
            .unwrap()
            .unwrap();
        assert_eq!(inbox_recipient.balance().to_sat(), 2_000_000);
        assert_eq!(
            inbox_recipient
                .as_snark_account()
                .unwrap()
                .inbox_mmr()
                .entries,
            1
        );
        assert_eq!(
            verified
                .get_account_state(make_account_id(TEST_RECIPIENT_ID))
                .unwrap()
                .unwrap()
                .balance()
                .to_sat(),
            1_000_000
        );
        assert_eq!(
            verified.limbo_funds(),
            BitcoinAmount::try_from(25_000_000).unwrap()
        );
        assert_eq!(verified.cur_epoch(), 2);
        assert_eq!(verified.active_version(), OLSpecId::V1);
        assert_eq!(verified.expected_version(), OLSpecId::V1);

        let payload: OLDaPayloadV1 = decode_buf_exact(&input.da_state_diff_bytes).unwrap();
        let mut reconstructed = MemoryStateBaseLayer::new(input.start_state.clone());
        verify_epoch_with_diff::<_, OLDaSchemeV1>(
            &mut reconstructed,
            &EpochInfo::new(
                BlockInfo::from_header(terminal),
                input.parent.compute_block_commitment(),
            ),
            payload,
            &manifests,
            &EpochExecExpectations::new(*terminal.state_root()),
            &runtime_params,
        )
        .unwrap();
        assert_eq!(reconstructed.state(), verified.state());

        // The native program runs the same checkpoint logic and SSZ input path
        // as the guest, including authenticated DA-witness verification.
        let claim = CheckpointProgram::execute(&input, runtime_params).unwrap();
        assert_eq!(
            claim.l2_range().start(),
            &input.parent.compute_block_commitment()
        );
        assert_eq!(claim.l2_range().end(), &terminal.compute_block_commitment());
    }
}
