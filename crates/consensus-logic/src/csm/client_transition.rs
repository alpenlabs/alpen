//! Core state transition function.

use std::cmp::Ordering;

use bitcoin::Transaction;
use strata_primitives::{
    batch::verify_signed_checkpoint_sig,
    l1::{L1BlockCommitment, L1BlockId},
    prelude::*,
};
use strata_state::{client_state::*, operation::*};
use strata_storage::NodeStorage;
use tracing::*;

use crate::{checkpoint_verification::verify_checkpoint, errors::*};

/// Interface for external context necessary specifically for transitioning.
pub trait EventContext {
    fn get_l1_block_manifest(&self, blockid: &L1BlockId) -> Result<L1BlockManifest, Error>;
    fn get_l1_block_manifest_at_height(&self, height: u64) -> Result<L1BlockManifest, Error>;
    fn get_client_state(&self, blockid: &L1BlockCommitment) -> Result<ClientState, Error>;
}

/// Event context using the main node storage interfaace.
#[derive(Clone)]
#[expect(missing_debug_implementations)]
pub struct StorageEventContext<'c> {
    storage: &'c NodeStorage,
}

impl<'c> StorageEventContext<'c> {
    pub fn new(storage: &'c NodeStorage) -> Self {
        Self { storage }
    }
}

impl EventContext for StorageEventContext<'_> {
    fn get_l1_block_manifest(&self, blockid: &L1BlockId) -> Result<L1BlockManifest, Error> {
        self.storage
            .l1()
            .get_block_manifest(blockid)?
            .ok_or(Error::MissingL1Block(*blockid))
    }

    fn get_l1_block_manifest_at_height(&self, height: u64) -> Result<L1BlockManifest, Error> {
        self.storage
            .l1()
            .get_block_manifest_at_height(height)?
            .ok_or(Error::MissingL1BlockHeight(height))
    }

    fn get_client_state(&self, blockid: &L1BlockCommitment) -> Result<ClientState, Error> {
        self.storage
            .client_state()
            ._get_state_blocking(*blockid)?
            .ok_or(Error::MissingClientState(0))
    }
}

/// Processes the block given the current consensus state, producing some
/// output.  This can return database errors.
pub fn process_block(
    state: &mut ClientStateMut,
    block: &L1BlockCommitment,
    context: &impl EventContext,
    params: &Params,
) -> Result<(), Error> {
    let height = block.height();

    // If the block is before genesis we don't care about it.
    // TODO maybe put back pre-genesis tracking?
    let genesis_trigger = params.rollup().genesis_l1_height;
    if height < genesis_trigger {
        #[cfg(test)]
        eprintln!(
                    "early L1 block at h={height} (gt={genesis_trigger}) you may have set up the test env wrong"
                );

        warn!(%height, "ignoring unexpected L1Block event before horizon");
        return Ok(());
    }

    // This doesn't do any SPV checks to make sure we only go to a
    // a longer chain, it just does it unconditionally.  This is fine,
    // since we'll be refactoring this more deeply soonish.
    let block_mf = context.get_l1_block_manifest(block.blkid())?;
    handle_block(state, &block_mf, context, params)?;
    Ok(())
}

fn handle_block(
    state: &mut ClientStateMut,
    block_mf: &L1BlockManifest,
    context: &impl EventContext,
    params: &Params,
) -> Result<(), Error> {
    let block: L1BlockCommitment = block_mf.into();
    let height = block.height();
    let rparams = params.rollup();

    // Skip blocks before the genesis height.
    if height < rparams.genesis_l1_height {
        return Ok(());
    }

    // Handle genesis height.
    if height == rparams.genesis_l1_height {
        // No checkpoints is expected at the genesis L1 height.
        state.accept_l1_block_state(&block, None, None);
        state.push_action(SyncAction::L2Genesis(*block.blkid()));
        return Ok(());
    }

    // Actualize the previous state to handle the reorg.
    match height.cmp(&state.state().next_exp_l1_block()) {
        Ordering::Less => {
            // Canonical chain reorg case: switch to the canonical block right before the chain
            // fork.
            // Unconditionally take the new client state, even though it comes
            // from the fork and the height less than expected.
            // The reason for that is btcio specific - we receive blocks with less height only
            // if btcio sees a longer fork of bitcoin.
            // So eventually, we receive a longer chain.
            state.take_client_state(context.get_client_state(&L1BlockCommitment::new(
                height - 1,
                block_mf.get_prev_blockid(),
            ))?);
        }
        Ordering::Equal => {
            // Canonical chain extension block, nothing to actualize.
        }
        Ordering::Greater => {
            // Indicates an error in the bookkeping, seems we didn't follow all the blocks.
            panic!("consensus L1 block following skipped blocks?");
        }
    }

    // Push actions from processing l1 block if any
    let (recent_checkpoint, actions) = process_l1_block(
        height,
        state.state().get_last_checkpoint(),
        block_mf,
        params.rollup(),
    )?;
    state.push_actions(actions.into_iter());

    // Structly speaking, here we rely on the fact that depth looks
    // at the canonical chain (and not on the fork).
    // Otherwise, we are screwed (because we query buried_block by height).
    let depth = rparams.l1_reorg_safe_depth as u64;
    let buried_height = height
        .checked_sub(depth)
        .expect("we should never see height less than depth");
    let last_finalized_checkpoint = fetch_last_finalized_checkpoint(buried_height, context);

    // Set the new client state.
    state.accept_l1_block_state(&block, last_finalized_checkpoint, recent_checkpoint);

    // Finalize the new epoch after the state transition (if any).
    if let Some(decl_epoch) = state.state().get_declared_final_epoch() {
        state.push_action(SyncAction::FinalizeEpoch(decl_epoch));
    }

    Ok(())
}

fn fetch_last_finalized_checkpoint(
    buried_height: u64,
    context: &impl EventContext,
) -> Option<L1Checkpoint> {
    let block = context.get_l1_block_manifest_at_height(buried_height).ok();
    // TODO(QQ): a bit ugly, unwind it somehow.
    if let Some(b) = block {
        if let Ok(cs) = context.get_client_state(&b.into()) {
            cs.get_last_checkpoint()
        } else {
            None
        }
    } else {
        None
    }
}

fn process_l1_block(
    height: u64,
    prev_checkpoint: Option<L1Checkpoint>,
    block_mf: &L1BlockManifest,
    params: &RollupParams,
) -> Result<(Option<L1Checkpoint>, Vec<SyncAction>), Error> {
    let mut sync_actions = Vec::new();
    let mut new_checkpoint = prev_checkpoint.clone();

    // Iterate through all of the protocol operations in all of the txs.
    // TODO split out each proto op handling into a separate function
    for tx in block_mf.txs() {
        for op in tx.protocol_ops() {
            if let ProtocolOperation::Checkpoint(signed_ckpt) = op {
                debug!(%height, "Obtained checkpoint in l1_block");
                // Before we do anything, check its signature.
                if !verify_signed_checkpoint_sig(signed_ckpt, &params.cred_rule) {
                    warn!(%height, "ignoring checkpointing with invalid signature");
                    continue;
                }

                let ckpt = signed_ckpt.checkpoint();

                // Now do the more thorough checks
                if verify_checkpoint(ckpt, prev_checkpoint.as_ref(), params).is_err() {
                    // If it's invalid then just print a warning and move on.
                    warn!(%height, "ignoring invalid checkpoint in L1 block");
                    continue;
                }

                let ckpt_ref = get_l1_reference(tx, *block_mf.blkid(), height)?;

                // Construct the state bookkeeping entry for the checkpoint.
                let l1ckpt = L1Checkpoint::new(
                    ckpt.batch_info().clone(),
                    *ckpt.batch_transition(),
                    ckpt_ref.clone(),
                );

                new_checkpoint = Some(l1ckpt);

                // Emit a sync action to update checkpoint entry in db
                sync_actions.push(SyncAction::UpdateCheckpointInclusion {
                    checkpoint: signed_ckpt.clone().into(),
                    l1_reference: ckpt_ref,
                });
            }
        }
    }

    Ok((new_checkpoint, sync_actions))
}

fn get_l1_reference(tx: &L1Tx, blockid: L1BlockId, height: u64) -> Result<CheckpointL1Ref, Error> {
    let btx: Transaction = tx.tx_data().try_into().map_err(|_e| {
        warn!(%height, "Invalid bitcoin transaction data in L1Tx");
        let msg = format!("Invalid bitcoin transaction data in L1Tx at height {height}");
        Error::Other(msg)
    })?;

    let txid = btx.compute_txid().into();
    let wtxid = btx.compute_wtxid().into();
    let l1_comm = L1BlockCommitment::new(height, blockid);
    Ok(CheckpointL1Ref::new(l1_comm, txid, wtxid))
}

#[cfg(test)]
mod tests {
    use bitcoin::BlockHash;
    use strata_primitives::l1::L1BlockManifest;
    use strata_state::l1::L1BlockId;
    use strata_test_utils_btc::segment::BtcChainSegment;
    use strata_test_utils_l2::{gen_client_state, gen_params};

    use super::*;
    use crate::genesis;

    #[derive(Debug)]
    pub(crate) struct DummyEventContext {
        chainseg: BtcChainSegment,
    }

    impl DummyEventContext {
        pub(crate) fn new() -> Self {
            Self {
                chainseg: BtcChainSegment::load(),
            }
        }
    }

    impl EventContext for DummyEventContext {
        fn get_l1_block_manifest(&self, blockid: &L1BlockId) -> Result<L1BlockManifest, Error> {
            let blockhash: BlockHash = (*blockid).into();
            Ok(self
                .chainseg
                .get_block_manifest_by_blockhash(&blockhash)
                .unwrap())
        }

        fn get_l1_block_manifest_at_height(&self, height: u64) -> Result<L1BlockManifest, Error> {
            let rec = self.chainseg.get_header_record(height).unwrap();
            Ok(L1BlockManifest::new(rec, None, Vec::new(), 0, height))
        }

        fn get_client_state(&self, _blockid: &L1BlockCommitment) -> Result<ClientState, Error> {
            // TODO(QQ): populate with test data.
            todo!()
        }
    }

    struct TestEvent<'a> {
        event: SyncEvent,
        expected_actions: &'a [SyncAction],
    }

    struct TestCase<'a> {
        description: &'static str,
        events: &'a [TestEvent<'a>], // List of events to process
        state_assertions: Box<dyn Fn(&ClientState)>, // Closure to verify state after all events
    }

    fn run_test_cases(test_cases: &[TestCase<'_>], state: &mut ClientState, params: &Params) {
        let context = DummyEventContext::new();

        for case in test_cases {
            println!("Running test case: {}", case.description);

            let mut outputs = Vec::new();
            for (i, test_event) in case.events.iter().enumerate() {
                let mut state_mut = ClientStateMut::new(state.clone());
                let event = &test_event.event;
                eprintln!("giving sync event {event}");
                // TODO(QQ): adjust
                //process_event(&mut state_mut, event, &context, params).unwrap();
                let output = state_mut.into_update();
                outputs.push(output.clone());

                assert_eq!(
                    output.actions(),
                    test_event.expected_actions,
                    "Failed on actions for event {} in test case: {}",
                    i + 1,
                    case.description
                );

                *state = output.into_state();
            }

            // Run the state assertions after all events
            (case.state_assertions)(state);
        }
    }

    #[test]
    fn test_genesis() {
        let params = gen_params();
        let mut state = gen_client_state(Some(&params));

        let horizon = params.rollup().horizon_l1_height as u64;
        let genesis = params.rollup().genesis_l1_height as u64;
        let reorg_safe_depth = params.rollup().l1_reorg_safe_depth;

        let chain = BtcChainSegment::load();
        let _l1_verification_state = chain
            .get_verification_state(genesis + 1, reorg_safe_depth)
            .unwrap();

        let l1_chain = chain.get_header_records(horizon, 10).unwrap();

        let pregenesis_mfs = chain.get_block_manifests(genesis, 1).unwrap();
        let (genesis_block, _) = genesis::make_l2_genesis(&params, pregenesis_mfs);
        let _genesis_blockid = genesis_block.header().get_blockid();

        let l1_blocks = l1_chain
            .iter()
            .enumerate()
            .map(|(i, block)| L1BlockCommitment::new(horizon + i as u64, *block.blkid()))
            .collect::<Vec<_>>();

        let _blkids: Vec<L1BlockId> = l1_chain.iter().map(|b| *b.blkid()).collect();

        let test_cases = [
            // These are kinda weird out because we got rid of pre-genesis
            // tracking and just discard these L1 blocks that are before
            // genesis.  We might re-add this later if the project demands it.
            TestCase {
                description: "At horizon block",
                events: &[TestEvent {
                    event: SyncEvent::L1Block(l1_blocks[0]),
                    expected_actions: &[],
                }],
                state_assertions: Box::new(move |state| {
                    assert!(!state.has_genesis_occurred());
                }),
            },
            TestCase {
                description: "At horizon block + 1",
                events: &[TestEvent {
                    event: SyncEvent::L1Block(l1_blocks[1]),
                    expected_actions: &[],
                }],
                state_assertions: Box::new(move |state| {
                    assert!(!state.has_genesis_occurred());
                    /*assert_eq!(
                        state.most_recent_l1_block(),
                        Some(&l1_chain[1].blkid())
                    );*/
                    // Because values for horizon is 40318, genesis is 40320
                    assert_eq!(state.next_exp_l1_block(), genesis);
                }),
            },
            TestCase {
                // We're assuming no rollback here.
                description: "At L2 genesis trigger L1 block reached we lock in",
                events: &[TestEvent {
                    event: SyncEvent::L1Block(l1_blocks[2]),
                    expected_actions: &[SyncAction::L2Genesis(*l1_blocks[2].blkid())],
                }],
                state_assertions: Box::new(move |state| {
                    assert!(state.has_genesis_occurred());
                    assert_eq!(state.next_exp_l1_block(), genesis + 1);
                }),
            },
            TestCase {
                description: "At genesis + 1",
                events: &[TestEvent {
                    event: SyncEvent::L1Block(l1_blocks[3]),
                    expected_actions: &[],
                }],
                state_assertions: Box::new({
                    let l1_chain = l1_chain.clone();
                    move |state| {
                        assert!(state.has_genesis_occurred());
                        assert_eq!(
                            state.most_recent_l1_block(),
                            Some(l1_chain[(genesis + 1 - horizon) as usize].blkid(),)
                        );
                        assert_eq!(state.next_exp_l1_block(), genesis + 2);
                    }
                }),
            },
            TestCase {
                description: "At genesis + 2",
                events: &[TestEvent {
                    event: SyncEvent::L1Block(l1_blocks[4]),
                    expected_actions: &[],
                }],
                state_assertions: Box::new({
                    let l1_chain = l1_chain.clone();
                    move |state| {
                        assert!(state.has_genesis_occurred());
                        assert_eq!(
                            state.most_recent_l1_block(),
                            Some(l1_chain[(genesis + 2 - horizon) as usize].blkid())
                        );
                        assert_eq!(state.next_exp_l1_block(), genesis + 3);
                    }
                }),
            },
            TestCase {
                description: "At genesis + 3, lock in genesis",
                events: &[TestEvent {
                    event: SyncEvent::L1Block(l1_blocks[5]),
                    expected_actions: &[],
                }],
                state_assertions: Box::new(move |state| {
                    assert!(state.has_genesis_occurred());
                    assert_eq!(state.next_exp_l1_block(), genesis + 4);
                }),
            },
            TestCase {
                description: "Rollback to genesis height",
                events: &[TestEvent {
                    event: SyncEvent::L1Revert(l1_blocks[4]),
                    expected_actions: &[],
                }],
                state_assertions: Box::new(move |_state| {}),
            },
        ];

        run_test_cases(&test_cases, &mut state, &params);
    }
}
