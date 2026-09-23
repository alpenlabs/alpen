use strata_acct_types::{BitcoinAmount, BRIDGE_GATEWAY_ACCT_ID};
use strata_identifiers::SubjectId;
use strata_ol_params::OLRuntimeParams;
use strata_ol_state_support_types::DaAccumulatingState;
use strata_ol_stf_v1::{
    execute_block_batch_predrain,
    test_utils::{
        make_account_id, make_deposit_manifest_for_account,
        make_deposit_manifest_with_destination_bytes, make_empty_manifest,
        make_p2wpkh_bosd_descriptor, make_withdrawal_payload, snark_inbox_msg, to_ol_block,
        InboxMmrTracker, OLStfFixture, TEST_RECIPIENT_ID, TEST_SNARK_ACCOUNT_ID,
    },
};
use strata_predicate::PredicateKey;
use strata_proofimpl_checkpoint::program::CheckpointProverInput;
use strata_proofimpl_predicate_keys::{PredicateKeyProvider, Sp1Groth16PredicateKey};

pub(super) fn stored_predicate() -> PredicateKey {
    // Use the production verifier encoding and key size, with a synthetic program
    // ID. These accounts exercise state storage/hashing, not Groth16 verification.
    Sp1Groth16PredicateKey::new([0x42; 32])
        .predicate_key()
        .expect("SP1 predicate key")
}

pub(super) fn prepare_checkpoint_input(predicate: &PredicateKey) -> CheckpointProverInput {
    let sender = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient = make_account_id(TEST_RECIPIENT_ID);
    let inbox_recipient = make_account_id(1_000);
    let mut builder = OLStfFixture::builder()
        .with_genesis_snark_account(sender, |account| {
            account.with_balance(BitcoinAmount::try_from(300_000_000).unwrap())
        })
        .with_genesis_empty_account(recipient)
        .with_genesis_manifest(make_empty_manifest(1, 0));
    for index in 1_000..1_062 {
        builder = builder.with_genesis_snark_account(make_account_id(index), |account| {
            account.with_update_vk(predicate.clone())
        });
    }
    let mut fixture = builder.execute_genesis();
    let parent = fixture.parent_header().clone();
    let start_state = fixture.state().clone();
    let sender_serial = fixture.account_serial(sender);
    let mut blocks = Vec::with_capacity(5);

    let message = snark_inbox_msg();
    let mut inbox = InboxMmrTracker::new();
    let proof = inbox.add_message(&message);
    let block = fixture
        .child_block()
        .with_gam(sender, |gam| {
            gam.with_payload(message.payload().data().to_vec())
        })
        .execute();
    blocks.push(to_ol_block(block.completed_block()));

    let block = fixture
        .child_block()
        .with_sau(sender, |update| {
            update
                .with_processed_messages(vec![message], vec![proof])
                .transfer(recipient, BitcoinAmount::try_from(1_000_000).unwrap())
                .output_message(
                    inbox_recipient,
                    BitcoinAmount::try_from(2_000_000).unwrap(),
                    b"account message".to_vec(),
                )
                .output_message(
                    BRIDGE_GATEWAY_ACCT_ID,
                    BitcoinAmount::try_from(100_000_000).unwrap(),
                    make_withdrawal_payload(make_p2wpkh_bosd_descriptor(0x14)),
                )
        })
        .execute();
    blocks.push(to_ol_block(block.completed_block()));

    // Distribute manifests across the epoch to exercise buffering as well as
    // terminal draining; a terminal-only manifest collector would miss these.
    let block = fixture
        .child_block()
        .with_manifest(make_deposit_manifest_for_account(
            2,
            1,
            sender_serial,
            SubjectId::from([42; 32]),
            BitcoinAmount::try_from(75_000_000).unwrap(),
        ))
        .execute();
    blocks.push(to_ol_block(block.completed_block()));

    let block = fixture
        .child_block()
        .with_sau(sender, |update| {
            update.with_new_predicate(predicate.clone())
        })
        .with_manifest(make_deposit_manifest_with_destination_bytes(
            3,
            2,
            Vec::new(),
            BitcoinAmount::try_from(25_000_000).unwrap(),
        ))
        .execute();
    blocks.push(to_ol_block(block.completed_block()));

    let block = fixture
        .child_block()
        .with_manifest(make_empty_manifest(4, 3))
        .terminal()
        .execute();
    blocks.push(to_ol_block(block.completed_block()));

    // Match checkpoint construction: collect transaction effects before the
    // terminal drain, which the verifier reconstructs from the ASM manifests.
    let mut accumulated = DaAccumulatingState::new(start_state.clone());
    execute_block_batch_predrain(
        &mut accumulated,
        &blocks,
        &parent,
        &OLRuntimeParams::test_default(),
    )
    .expect("replay benchmark epoch");
    let da_state_diff_bytes = accumulated
        .take_completed_epoch_da_blob()
        .expect("finalize benchmark DA")
        .expect("terminal epoch produces DA");
    CheckpointProverInput {
        start_state: start_state.state().clone(),
        blocks,
        parent,
        da_state_diff_bytes,
    }
}
