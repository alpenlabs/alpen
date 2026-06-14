//! Perf inputs for the checkpoint SP1 guest.

use std::{env, time::Instant};

use ssz::Encode;
use strata_acct_types::{BitcoinAmount, BRIDGE_GATEWAY_ACCT_ID};
use strata_bridge_params::BridgeParams;
use strata_identifiers::{AccountSerial, Buf32, Buf64, L1BlockId, L1Height, SubjectId, WtxidsRoot};
use strata_ledger_types::IStateAccessor;
use strata_ol_chain_types_new::{
    AsmManifest, OLBlock, SignedOLBlockHeader, MAX_SEALING_MANIFEST_COUNT,
};
use strata_ol_state_support_types::{DaAccumulatingState, MemoryStateBaseLayer};
use strata_ol_stf::{
    execute_block_batch_predrain,
    test_utils::{
        make_account_id, make_deposit_log_for_account, make_empty_manifest,
        make_withdrawal_payload, OLStfFixture, TEST_RECIPIENT_ID, TEST_SNARK_ACCOUNT_ID,
    },
};
use strata_proofimpl_checkpoint::program::{CheckpointProgram, CheckpointProverInput};
use tracing::info;
use zkaleido::{ExecutionSummary, ZkVmHost, ZkVmProgram};

use super::ProofReport;

const PROOF_BLOCKS: usize = 64;
const DEFAULT_MANIFEST_COUNT: usize = 256;
const TRANSFER_AMOUNT_SATS: u64 = 1;
const WITHDRAWAL_AMOUNT_SATS: u64 = 100_000_000;
// Mirrors `MAX_LOGS_PER_MANIFEST` from the ASM manifest SSZ type.
const MAX_ASM_LOGS_PER_MANIFEST: usize = 1024;
// Mirrors `TxEffects.MAX_MESSAGES` from `crates/acct-types/ssz/effects.ssz`.
const TX_EFFECTS_MAX_MESSAGES: usize = 255;
const MAX_OL_LOGS_PER_CAPACITY_TX: usize = TX_EFFECTS_MAX_MESSAGES + 1;
const CAPACITY_MANIFEST_COUNTS_ENV: &str = "CHECKPOINT_CAPACITY_MANIFEST_COUNTS";
const CAPACITY_ASM_LOGS_PER_MANIFEST_ENV: &str = "CHECKPOINT_CAPACITY_ASM_LOGS_PER_MANIFEST";
const CAPACITY_OL_LOG_TARGET_ENV: &str = "CHECKPOINT_CAPACITY_OL_LOG_TARGET";

fn candidate_manifest_counts() -> Vec<usize> {
    if let Ok(raw_counts) = env::var(CAPACITY_MANIFEST_COUNTS_ENV) {
        let mut counts: Vec<usize> = raw_counts
            .split(',')
            .map(str::trim)
            .filter(|count| !count.is_empty())
            .map(|count| {
                count.parse::<usize>().unwrap_or_else(|e| {
                    panic!("{CAPACITY_MANIFEST_COUNTS_ENV} contains invalid count {count:?}: {e}")
                })
            })
            .collect();
        assert!(
            !counts.is_empty(),
            "{CAPACITY_MANIFEST_COUNTS_ENV} must include at least one count"
        );
        counts.retain(|count| *count <= MAX_SEALING_MANIFEST_COUNT as usize);
        assert!(
            !counts.is_empty(),
            "{CAPACITY_MANIFEST_COUNTS_ENV} contains no count <= MAX_SEALING_MANIFEST_COUNT"
        );
        counts.sort_unstable();
        counts.dedup();
        return counts;
    }

    let mut candidates = vec![256, 512, 1024, MAX_SEALING_MANIFEST_COUNT as usize];
    candidates.retain(|count| *count <= MAX_SEALING_MANIFEST_COUNT as usize);
    candidates.sort_unstable();
    candidates.dedup();
    candidates
}

fn capacity_ol_log_target() -> Option<usize> {
    let raw_target = env::var(CAPACITY_OL_LOG_TARGET_ENV).ok()?;
    let target = raw_target.trim();
    if target.is_empty() {
        return None;
    }

    let target = target.parse::<usize>().unwrap_or_else(|e| {
        panic!("{CAPACITY_OL_LOG_TARGET_ENV} contains invalid target {target:?}: {e}")
    });
    let max_representable = PROOF_BLOCKS * MAX_OL_LOGS_PER_CAPACITY_TX;
    assert!(
        target <= max_representable,
        "{CAPACITY_OL_LOG_TARGET_ENV}={target} exceeds max representable OL logs {max_representable}"
    );
    Some(target)
}

fn capacity_asm_logs_per_manifest() -> usize {
    let Some(raw_count) = env::var(CAPACITY_ASM_LOGS_PER_MANIFEST_ENV).ok() else {
        return 1;
    };
    let count = raw_count.trim();
    if count.is_empty() {
        return 1;
    }

    let count = count.parse::<usize>().unwrap_or_else(|e| {
        panic!("{CAPACITY_ASM_LOGS_PER_MANIFEST_ENV} contains invalid count {count:?}: {e}")
    });
    assert!(
        count > 0,
        "{CAPACITY_ASM_LOGS_PER_MANIFEST_ENV} must be greater than zero"
    );
    assert!(
        count <= MAX_ASM_LOGS_PER_MANIFEST,
        "{CAPACITY_ASM_LOGS_PER_MANIFEST_ENV}={count} exceeds MAX_LOGS_PER_MANIFEST={MAX_ASM_LOGS_PER_MANIFEST}"
    );
    count
}

fn withdrawal_counts_by_block(ol_log_target: Option<usize>) -> Vec<Option<usize>> {
    (1..=PROOF_BLOCKS)
        .map(|block_idx| match ol_log_target {
            Some(target) => {
                let prior_capacity = (block_idx - 1) * MAX_OL_LOGS_PER_CAPACITY_TX;
                if prior_capacity >= target {
                    None
                } else {
                    let logs_this_tx = (target - prior_capacity).min(MAX_OL_LOGS_PER_CAPACITY_TX);
                    Some(logs_this_tx.saturating_sub(1))
                }
            }
            None => Some(1),
        })
        .collect()
}

fn checkpoint_report_name(
    manifest_count: usize,
    asm_logs_per_manifest: usize,
    ol_log_target: Option<usize>,
) -> String {
    let mut name = format!("{}-{manifest_count}", CheckpointProgram::name());
    if asm_logs_per_manifest != 1 {
        name.push_str(&format!("-asm-logs-{asm_logs_per_manifest}"));
    }
    if let Some(target) = ol_log_target {
        name.push_str(&format!("-ol-logs-{target}"));
    }
    name
}

fn buf32_from_l1_height(height: L1Height, domain: u8) -> Buf32 {
    let mut bytes = [domain; 32];
    let height_bytes = height.to_le_bytes();
    bytes[..height_bytes.len()].copy_from_slice(&height_bytes);
    Buf32::from(bytes)
}

fn make_deposit_manifest(
    height: L1Height,
    account_serial: AccountSerial,
    dest_subject: SubjectId,
    amount: BitcoinAmount,
    log_count: usize,
) -> AsmManifest {
    assert!(
        log_count <= MAX_ASM_LOGS_PER_MANIFEST,
        "deposit manifest log count exceeds MAX_LOGS_PER_MANIFEST"
    );
    let deposit_log = make_deposit_log_for_account(account_serial, dest_subject, amount);
    AsmManifest::new(
        height,
        L1BlockId::from(buf32_from_l1_height(height, 0xa1)),
        WtxidsRoot::from(buf32_from_l1_height(height, 0xb2)),
        vec![deposit_log; log_count],
    )
    .expect("deposit manifest should be valid")
}

fn prepare_checkpoint_input(
    manifest_count: usize,
    ol_log_target: Option<usize>,
    asm_logs_per_manifest: usize,
) -> CheckpointProverInput {
    assert!(
        manifest_count <= MAX_SEALING_MANIFEST_COUNT as usize,
        "manifest count exceeds MAX_SEALING_MANIFEST_COUNT"
    );

    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID);
    let dest_subject = SubjectId::from([42u8; 32]);
    let deposit_amount = BitcoinAmount::from_sat(WITHDRAWAL_AMOUNT_SATS);
    assert!(
        asm_logs_per_manifest > 0,
        "asm logs per manifest must be greater than zero"
    );
    assert!(
        asm_logs_per_manifest <= MAX_ASM_LOGS_PER_MANIFEST,
        "asm logs per manifest exceeds MAX_LOGS_PER_MANIFEST"
    );
    let withdrawal_counts = withdrawal_counts_by_block(ol_log_target);
    let withdrawal_count = withdrawal_counts
        .iter()
        .map(|maybe_count| maybe_count.unwrap_or_default())
        .sum::<usize>();
    let transfer_count = match ol_log_target {
        Some(_) => withdrawal_counts
            .iter()
            .filter(|maybe_count| matches!(maybe_count, Some(0)))
            .count(),
        None => PROOF_BLOCKS,
    };
    let total_withdrawal_sats = (withdrawal_count as u64)
        .checked_mul(WITHDRAWAL_AMOUNT_SATS)
        .expect("withdrawal capacity balance should fit in u64");
    let total_transfer_sats = (transfer_count as u64)
        .checked_mul(TRANSFER_AMOUNT_SATS)
        .expect("transfer capacity balance should fit in u64");
    let fixture_builder = OLStfFixture::builder();
    let snark_acct_serial = fixture_builder.next_account_serial();
    let initial_balance = total_withdrawal_sats
        .checked_add(total_transfer_sats)
        .expect("capacity balance should fit in u64");
    let mut fixture = fixture_builder
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(initial_balance))
        })
        .with_genesis_empty_account(recipient_id)
        .with_genesis_manifest(make_empty_manifest(1, 0))
        .execute_genesis();

    let parent = fixture.last_completed_block().header().clone();
    let start_state = fixture.state().state().clone();
    let mut blocks = Vec::with_capacity(PROOF_BLOCKS);

    for block_idx in 1..=PROOF_BLOCKS {
        let manifests = if block_idx == PROOF_BLOCKS {
            let first_manifest_height = fixture.state().last_l1_height() + 1;
            (0..manifest_count)
                .map(|offset| {
                    make_deposit_manifest(
                        first_manifest_height + offset as L1Height,
                        snark_acct_serial,
                        dest_subject,
                        deposit_amount,
                        asm_logs_per_manifest,
                    )
                })
                .collect()
        } else {
            Vec::new()
        };

        let mut block_builder = fixture.child_block();
        if let Some(withdrawal_count) = withdrawal_counts[block_idx - 1] {
            block_builder = block_builder.with_sau(snark_acct_id, |mut sau| {
                if ol_log_target.is_none() || withdrawal_count == 0 {
                    sau = sau.transfer(recipient_id, BitcoinAmount::from_sat(TRANSFER_AMOUNT_SATS));
                }

                for msg_idx in 0..withdrawal_count {
                    let withdrawal_dest = match ol_log_target {
                        Some(_) => format!("bc1qcapacity{block_idx:02}{msg_idx:03}").into_bytes(),
                        None => format!("bc1qcapacity{block_idx:02}").into_bytes(),
                    };
                    sau = sau.output_message(
                        BRIDGE_GATEWAY_ACCT_ID,
                        BitcoinAmount::from_sat(WITHDRAWAL_AMOUNT_SATS),
                        make_withdrawal_payload(withdrawal_dest),
                    );
                }

                sau
            });
        }
        if !manifests.is_empty() {
            block_builder = block_builder.with_manifests(manifests);
        }
        if block_idx == PROOF_BLOCKS {
            block_builder = block_builder.terminal();
        }

        let block = block_builder.execute().completed_block().clone();
        blocks.push(OLBlock::new(
            SignedOLBlockHeader::new(block.header().clone(), Buf64::zero()),
            block.body().clone(),
        ));
    }

    let mut da_state = DaAccumulatingState::new(MemoryStateBaseLayer::new(start_state.clone()));
    execute_block_batch_predrain(&mut da_state, &blocks, &parent, BridgeParams::default())
        .expect("checkpoint input should replay for DA predrain");
    let da_state_diff_bytes = da_state
        .take_completed_epoch_da_blob()
        .expect("DA blob extraction should succeed")
        .expect("terminal checkpoint input should produce a DA blob");

    CheckpointProverInput {
        start_state,
        blocks,
        parent,
        da_state_diff_bytes,
        bridge_params: BridgeParams::default(),
    }
}

fn gen_perf_report_for_count(
    host: &impl ZkVmHost,
    manifest_count: usize,
) -> (String, ExecutionSummary) {
    let ol_log_target = capacity_ol_log_target();
    let asm_logs_per_manifest = capacity_asm_logs_per_manifest();
    info!(
        manifest_count,
        asm_logs_per_manifest,
        ?ol_log_target,
        "generating execution summary for checkpoint"
    );
    let input = prepare_checkpoint_input(manifest_count, ol_log_target, asm_logs_per_manifest);
    let encoded_blocks_len = input.blocks.as_ssz_bytes().len();
    info!(
        manifest_count,
        asm_logs_per_manifest,
        ?ol_log_target,
        encoded_blocks_len,
        "prepared checkpoint perf input"
    );
    let summary =
        <CheckpointProgram as ZkVmProgram>::execute(&input, host).expect("checkpoint execution");
    (
        checkpoint_report_name(manifest_count, asm_logs_per_manifest, ol_log_target),
        summary,
    )
}

pub(crate) fn gen_perf_report(host: &impl ZkVmHost) -> (String, ExecutionSummary) {
    gen_perf_report_for_count(host, DEFAULT_MANIFEST_COUNT)
}

pub(crate) fn gen_capacity_perf_reports(host: &impl ZkVmHost) -> Vec<(String, ExecutionSummary)> {
    candidate_manifest_counts()
        .into_iter()
        .map(|manifest_count| gen_perf_report_for_count(host, manifest_count))
        .collect()
}

fn prove_report_for_count(host: &impl ZkVmHost, manifest_count: usize) -> (String, ProofReport) {
    let ol_log_target = capacity_ol_log_target();
    let asm_logs_per_manifest = capacity_asm_logs_per_manifest();
    info!(
        manifest_count,
        asm_logs_per_manifest,
        ?ol_log_target,
        "generating SP1 proof for checkpoint"
    );
    let input = prepare_checkpoint_input(manifest_count, ol_log_target, asm_logs_per_manifest);
    let encoded_blocks_len = input.blocks.as_ssz_bytes().len();
    info!(
        manifest_count,
        asm_logs_per_manifest,
        ?ol_log_target,
        encoded_blocks_len,
        "prepared checkpoint proof input"
    );
    let started_at = Instant::now();
    let receipt =
        <CheckpointProgram as ZkVmProgram>::prove(&input, host).expect("checkpoint proof");
    let elapsed = started_at.elapsed();
    (
        checkpoint_report_name(manifest_count, asm_logs_per_manifest, ol_log_target),
        ProofReport { receipt, elapsed },
    )
}

pub(crate) fn prove_perf_report(host: &impl ZkVmHost) -> (String, ProofReport) {
    prove_report_for_count(host, DEFAULT_MANIFEST_COUNT)
}

pub(crate) fn prove_capacity_reports(host: &impl ZkVmHost) -> Vec<(String, ProofReport)> {
    candidate_manifest_counts()
        .into_iter()
        .map(|manifest_count| prove_report_for_count(host, manifest_count))
        .collect()
}

#[cfg(test)]
mod tests {
    use strata_proofimpl_checkpoint::program::CheckpointProgram;

    use super::*;

    #[test]
    fn test_checkpoint_native_execution_for_candidate_counts() {
        for manifest_count in candidate_manifest_counts() {
            let input = prepare_checkpoint_input(
                manifest_count,
                capacity_ol_log_target(),
                capacity_asm_logs_per_manifest(),
            );
            let output = CheckpointProgram::execute(&input).unwrap();
            assert_eq!(
                *output.l2_range().end().blkid(),
                input.blocks.last().unwrap().header().compute_blkid()
            );
        }
    }
}
